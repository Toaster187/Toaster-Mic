using ToasterMic.Core;

internal static class BridgeTests
{
    public static async Task Run(Action<bool, string> check)
    {
        var lease = new ConnectionLease(100);
        check(!lease.IsActive(0), "virtual microphone starts inactive");
        lease.Observe(50); check(lease.IsActive(149) && !lease.IsActive(150), "network loss expires the device lease at the deadline");
        lease.Clear(); check(!lease.IsActive(60), "disconnect clears the device lease immediately");
        byte[] valid = [.."TMV1"u8, ..new byte[960]];
        var device = new FakeDevice();
        try { await MicrophoneBridgeSession.RunAsync(new Duplex(valid), device, default); } catch (EndOfStreamException) { }
        check(device.Events.SequenceEqual(new[]{"enable", "feed", "disable"}), "valid silent audio enables; client EOF disables (mute stays a valid stream)");
        foreach (byte[] bad in new[]{"TMV1"u8.ToArray(), "BAD!"u8.ToArray(), valid[..100]})
        {
            device = new FakeDevice();
            try { await MicrophoneBridgeSession.RunAsync(new Duplex(bad), device, default); } catch (IOException) { }
            check(device.Events.SequenceEqual(new[]{"disable"}), "incomplete or invalid connection never enables microphone");
        }
        device = new FakeDevice();
        try { await MicrophoneBridgeSession.RunAsync(new Duplex(valid, true), device, default, TimeSpan.FromMilliseconds(30)); } catch (OperationCanceledException) { }
        check(device.Events.Last() == "disable", "independent service watchdog disables a stalled producer");
        device = new FakeDevice { FailEnable = true };
        try { await MicrophoneBridgeSession.RunAsync(new Duplex(valid), device, default); } catch (InvalidOperationException) { }
        check(device.Events.SequenceEqual(new[]{"enable", "disable"}), "partial activation failure still disables device");
        device = new FakeDevice { FailFeed = true };
        try { await MicrophoneBridgeSession.RunAsync(new Duplex(valid), device, default); } catch (IOException) { }
        check(device.Events.Last() == "disable", "driver write error closes device lifetime");
        device = new FakeDevice(); using var stop = new CancellationTokenSource();
        Task run = MicrophoneBridgeSession.RunAsync(new Duplex(valid, true), device, stop.Token);
        await device.Fed.Task.WaitAsync(TimeSpan.FromSeconds(1)); stop.Cancel();
        try { await run; } catch (OperationCanceledException) { }
        check(device.Events.Last() == "disable", "service shutdown cancels pending read and disables microphone");
        device = new FakeDevice { FailDisable = true };
        bool failedClosed = false;
        try { await MicrophoneBridgeSession.RunAsync(new Duplex(valid), device, default); }
        catch (InvalidOperationException) { failedClosed = true; }
        check(failedClosed, "failed Windows disable is reported instead of claiming success");
    }
    private sealed class FakeDevice : IVirtualMicrophoneDevice
    {
        public List<string> Events = [];
        public bool FailEnable, FailFeed, FailDisable;
        public TaskCompletionSource Fed = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task EnableAsync(CancellationToken token) { Events.Add("enable"); if (FailEnable) throw new InvalidOperationException(); return Task.CompletedTask; }
        public void Feed(byte[] pcm) { Events.Add("feed"); if (FailFeed) throw new IOException(); Fed.TrySetResult(); }
        public Task DisableAsync() { Events.Add("disable"); if (FailDisable) throw new InvalidOperationException(); return Task.CompletedTask; }
    }
    private sealed class Duplex(byte[] input, bool hang = false) : Stream
    {
        private int position;
        public override bool CanRead => true; public override bool CanWrite => true; public override bool CanSeek => false;
        public override long Length => input.Length; public override long Position { get => position; set => throw new NotSupportedException(); }
        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();
        public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            if (position == input.Length) { if (hang) await Task.Delay(Timeout.Infinite, cancellationToken); return 0; }
            int count = Math.Min(73, Math.Min(buffer.Length, input.Length - position)); // Exercise split pipe reads.
            input.AsMemory(position, count).CopyTo(buffer); position += count; return count;
        }
        public override void Write(byte[] buffer, int offset, int count) { }
        public override ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken cancellationToken = default) => ValueTask.CompletedTask;
        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
    }
}
