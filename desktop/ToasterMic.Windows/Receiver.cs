using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using NAudio.CoreAudioApi;
using NAudio.Wave;
using ToasterMic.Core;

namespace ToasterMic.Windows;

public sealed class Receiver : IAsyncDisposable
{
    private readonly UdpClient socket = new(AddressFamily.InterNetwork);
    private readonly CancellationTokenSource cancel = new();
    private volatile byte[]? key;
    private readonly KeyExchange exchange = new();
    private readonly long session = BitConverter.ToInt64(RandomNumberGenerator.GetBytes(8));
    private readonly IDisposable output;
    private readonly object disposeGate = new();
    private Task? disposal;
    private readonly ConnectionLease lease = new(Stopwatch.Frequency * 3);
    private Task? loop;
    private long lastAudio;
    private volatile string? fault;
    public JitterBuffer Buffer { get; }
    public int Channels { get; private set; }
    public bool PhoneMuted { get; private set; }
    public bool Connected => fault == null && lease.IsActive(Stopwatch.GetTimestamp());
    public string Status => fault ?? (Interlocked.Read(ref lastAudio) == 0 ? "Warte auf Handy …" : !Connected ? "Verbinde erneut …" : PhoneMuted ? "Verbunden · Handy stumm" : $"Verbunden · {(Channels == 2 ? "Stereo" : "Mono")}");
    public Receiver(IPAddress host, int port, string deviceId, int packets)
        : this(host, port, packets, (buffer, onError) => new Playback(deviceId, buffer, onError)) { }

    internal Receiver(IPAddress host, int port, int packets, Func<JitterBuffer, Action<Exception>, IDisposable> createOutput)
    {
        Buffer = new(packets, adaptive: true);
        try
        {
            socket.Connect(host, port);
            socket.Client.ReceiveBufferSize = 64 * 1024;
            output = createOutput(Buffer, e => fault = "Audioausgabe: " + e.Message);
            // Never bind the network loops to a caller's WPF synchronization context.
            loop = Task.Run(() => Task.WhenAll(Receive(), Heartbeat()));
        }
        catch { output?.Dispose(); socket.Dispose(); cancel.Dispose(); exchange.Dispose(); throw; }
    }
    private async Task Heartbeat()
    {
        var offer = exchange.Offer(1, session);
        try
        {
            while (!cancel.IsCancellationRequested)
            {
                try {
                    if (!Connected) await socket.SendAsync(offer, cancel.Token);
                    var currentKey = key;
                    if (currentKey != null) await socket.SendAsync(Wire.Encode(Wire.Hello, 0, 0, session, 0, [], currentKey), cancel.Token);
                }
                catch (SocketException) { }
                await Task.Delay(1000, cancel.Token);
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (cancel.IsCancellationRequested) { }
        catch (Exception e) { fault = "Netzwerk: " + e.Message; }
    }
    private async Task Receive()
    {
        try
        {
            while (!cancel.IsCancellationRequested)
            {
                UdpReceiveResult data;
                try { data = await socket.ReceiveAsync(cancel.Token); }
                catch (SocketException) { await Task.Delay(100, cancel.Token); continue; }
                if (KeyExchange.IsOffer(data.Buffer, 2, session))
                {
                    if (Connected) continue;
                    try {
                        var nextKey = exchange.Derive(data.Buffer);
                        if (key == null || !key.AsSpan().SequenceEqual(nextKey)) { Buffer.Reset(); key = nextKey; }
                        await socket.SendAsync(Wire.Encode(Wire.Hello, 0, 0, session, 0, [], nextKey), cancel.Token);
                    }
                    catch (CryptographicException) { }
                    catch (SocketException) { }
                    continue;
                }
                var currentKey = key;
                if (currentKey == null) continue;
                var p = Wire.Decode(data.Buffer, currentKey);
                if (p == null || p.Type != Wire.Audio || p.Session != session) continue;
                Channels = p.Channels; PhoneMuted = (p.Flags & 1) != 0;
                Interlocked.Exchange(ref lastAudio, Stopwatch.GetTimestamp());
                lease.Observe(Stopwatch.GetTimestamp());
                Buffer.Push(p);
            }
        }
        catch (OperationCanceledException) { }
        catch (ObjectDisposedException) when (cancel.IsCancellationRequested) { }
        catch (Exception e) { fault = "Netzwerk: " + e.Message; }
    }
    public ValueTask DisposeAsync()
    {
        lock (disposeGate) return new ValueTask(disposal ??= Task.Run(Shutdown));
    }
    private async Task Shutdown()
    {
        Buffer.Muted = true;
        lease.Clear(); cancel.Cancel();
        try
        {
            var currentKey = key;
            if (currentKey != null)
            {
                using var timeout = new CancellationTokenSource(200);
                await socket.SendAsync(Wire.Encode(Wire.Stop, 0, 0, session, 0, [], currentKey), timeout.Token).ConfigureAwait(false);
            }
        }
        catch (Exception e) when (e is SocketException or OperationCanceledException or ObjectDisposedException) { }
        finally
        {
            socket.Dispose();
            try { if (loop != null) await loop.ConfigureAwait(false); }
            finally
            {
                try { output.Dispose(); }
                finally { cancel.Dispose(); exchange.Dispose(); }
            }
        }
    }
    private sealed class Playback : IDisposable
    {
        private readonly MMDevice device;
        private readonly WasapiOut output;
        public Playback(string deviceId, JitterBuffer buffer, Action<Exception> onError)
        {
            using var enumerator = new MMDeviceEnumerator();
            device = enumerator.GetDevice(deviceId);
            try
            {
                output = new WasapiOut(device, AudioClientShareMode.Shared, true, 10);
                output.Init(new Provider(buffer));
                output.PlaybackStopped += (_, e) => { if (e.Exception != null) onError(e.Exception); };
                output.Play();
            }
            catch { output?.Dispose(); device.Dispose(); throw; }
        }
        public void Dispose()
        {
            try { output.Dispose(); }
            finally { device.Dispose(); }
        }
    }
    private sealed class Provider(JitterBuffer buffer) : IWaveProvider
    {
        public WaveFormat WaveFormat { get; } = new(48000, 16, 2);
        public int Read(byte[] data, int offset, int count) { buffer.Read(data.AsSpan(offset, count)); return count; }
    }
}
