using System.Buffers.Binary;
using System.Diagnostics;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text.Json;
using ToasterMic.Core;

if (args.Length != 1) throw new ArgumentException("Usage: PhoneProbe <phone IPv4>");
using var socket = new UdpClient(); socket.Connect(args[0], 18125);
using var exchange = new KeyExchange();
long session = BitConverter.ToInt64(RandomNumberGenerator.GetBytes(8));
byte[]? key = null;
using var limit = new CancellationTokenSource(TimeSpan.FromSeconds(15));
var watch = Stopwatch.StartNew(); long lastHello = -1000;
int packets = 0, frames = 0, different = 0;
double leftEnergy = 0, rightEnergy = 0, cross = 0;
try
{
    while (packets < 600)
    {
        limit.Token.ThrowIfCancellationRequested();
        if (watch.ElapsedMilliseconds - lastHello >= 1000)
        {
            await socket.SendAsync(key == null ? exchange.Offer(1, session) : Wire.Encode(Wire.Hello, 0, 0, session, 0, [], key));
            lastHello = watch.ElapsedMilliseconds;
        }
        using var receiveTimeout = CancellationTokenSource.CreateLinkedTokenSource(limit.Token); receiveTimeout.CancelAfter(500);
        UdpReceiveResult received;
        try { received = await socket.ReceiveAsync(receiveTimeout.Token); } catch (OperationCanceledException) { continue; }
        if (KeyExchange.IsOffer(received.Buffer, 2, session))
        {
            key = exchange.Derive(received.Buffer);
            await socket.SendAsync(Wire.Encode(Wire.Hello, 0, 0, session, 0, [], key)); continue;
        }
        if (key == null) continue;
        var p = Wire.Decode(received.Buffer, key);
        if (p == null || p.Type != Wire.Audio || p.Session != session) continue;
        if (p.Channels != 2) throw new Exception("Phone sent mono");
        for (int i = 0; i < p.Audio.Length; i += 4)
        {
            double l = BinaryPrimitives.ReadInt16LittleEndian(p.Audio.AsSpan(i)), r = BinaryPrimitives.ReadInt16LittleEndian(p.Audio.AsSpan(i + 2));
            leftEnergy += l*l; rightEnergy += r*r; cross += l*r; frames++; if (l != r) different++;
        }
        packets++;
    }
    var report = new { packets, frames, different, rmsLeft = Math.Sqrt(leftEnergy/frames), rmsRight = Math.Sqrt(rightEnergy/frames), correlation = cross / Math.Max(1, Math.Sqrt(leftEnergy*rightEnergy)) };
    string json = JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true });
    Directory.CreateDirectory("artifacts/tests"); File.WriteAllText("artifacts/tests/pixel-network-stereo.json", json); Console.WriteLine(json);
    if (leftEnergy == 0 || rightEnergy == 0 || different < 100) throw new Exception("Stereo separation not demonstrated by received samples");
    Console.WriteLine("PASS Pixel 9 encrypted WLAN transmission: two nonidentical live audio channels");
}
finally { if (key != null) await socket.SendAsync(Wire.Encode(Wire.Stop, 0, 0, session, 0, [], key)); }
