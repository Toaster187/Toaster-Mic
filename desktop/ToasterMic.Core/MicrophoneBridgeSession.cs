namespace ToasterMic.Core;

public interface IVirtualMicrophoneDevice
{
    Task EnableAsync(CancellationToken cancellation);
    void Feed(byte[] pcm);
    Task DisableAsync();
}

// Shared by the Windows service and fault-injection tests. Device lifetime is
// scoped to live audio, not to the desktop window or an installation flag.
public static class MicrophoneBridgeSession
{
    public static async Task RunAsync(Stream pipe, IVirtualMicrophoneDevice device, CancellationToken stop, TimeSpan? frameTimeout = null)
    {
        async Task Read(byte[] data)
        {
            using var deadline = CancellationTokenSource.CreateLinkedTokenSource(stop);
            deadline.CancelAfter(frameTimeout ?? TimeSpan.FromSeconds(1));
            await pipe.ReadExactlyAsync(data, deadline.Token);
        }
        try
        {
            byte[] hello = new byte[4]; await Read(hello);
            if (!hello.AsSpan().SequenceEqual("TMV1"u8)) throw new IOException("Unbekanntes Audio-Protokoll.");
            byte[] pcm = new byte[960]; await Read(pcm);
            await device.EnableAsync(stop);
            device.Feed(pcm);
            using (var ackDeadline = CancellationTokenSource.CreateLinkedTokenSource(stop))
            { ackDeadline.CancelAfter(TimeSpan.FromSeconds(1)); await pipe.WriteAsync(new byte[]{1}, ackDeadline.Token); }
            while (!stop.IsCancellationRequested) { await Read(pcm); device.Feed(pcm); }
        }
        finally { await device.DisableAsync(); }
    }
}
