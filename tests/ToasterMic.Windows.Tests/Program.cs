using System.Net;
using System.Net.Sockets;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using System.IO;
using ToasterMic.Core;
using ToasterMic.Windows;

internal static class Program
{
    private static int passed;
    [STAThread]
    public static int Main()
    {
        int result = 0;
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext());
        Dispatcher.CurrentDispatcher.BeginInvoke(new Action(async () =>
        {
            try { await Run(); Console.WriteLine($"{passed} Windows checks passed."); }
            catch (Exception e) { Console.Error.WriteLine(e); result = 1; }
            finally { Dispatcher.CurrentDispatcher.InvokeShutdown(); }
        }));
        Dispatcher.Run();
        return result;
    }
    private static void Check(bool ok, string message)
    {
        if (!ok) throw new Exception(message);
        passed++; Console.WriteLine("PASS " + message);
    }
    private static async Task Run()
    {
        using var server = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0));
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var slow = new SlowOutput();
        var receiver = new Receiver(IPAddress.Loopback, ((IPEndPoint)server.Client.LocalEndPoint!).Port, 1, (_, _) => slow);
        var offer = await server.ReceiveAsync(timeout.Token);
        long session = BitConverter.ToInt64(offer.Buffer, 8);
        Check(KeyExchange.IsOffer(offer.Buffer, 1, session), "receiver starts its real network loop from a UI context");
        using var phone = new KeyExchange();
        byte[] key = phone.Derive(offer.Buffer);
        await server.SendAsync(phone.Offer(2, session), offer.RemoteEndPoint);
        var hello = await server.ReceiveAsync(timeout.Token);
        Check(Wire.Decode(hello.Buffer, key)?.Type == Wire.Hello, "receiver completes encrypted handshake");
        await server.SendAsync(Wire.Encode(Wire.Audio, 1, 0, session, 0, new byte[Wire.Frames * 2], key), offer.RemoteEndPoint);
        while (!receiver.Connected) await Task.Delay(5, timeout.Token);
        Check(receiver.Buffer.Stats.Received == 1, "receiver accepts phone audio");
        var dispose = receiver.DisposeAsync().AsTask();
        var repeated = receiver.DisposeAsync().AsTask();
        await slow.Started.Task.WaitAsync(timeout.Token);
        bool responsive = false;
        await Dispatcher.CurrentDispatcher.InvokeAsync(() => responsive = true);
        Check(responsive && !dispose.IsCompleted, "UI remains responsive while an audio driver is still stopping");
        slow.Release.Set();
        await Task.WhenAll(dispose, repeated).WaitAsync(timeout.Token);
        Check(slow.Disposals == 1, "repeated disconnect disposes the audio output exactly once");
        bool stopped = false;
        while (!stopped)
        {
            var packet = await server.ReceiveAsync(timeout.Token);
            stopped = Wire.Decode(packet.Buffer, key)?.Type == Wire.Stop;
        }
        Check(stopped, "disconnect sends STOP to the phone");
        for (int i = 0; i < 20; i++)
        {
            await using var idle = new Receiver(IPAddress.Loopback, ((IPEndPoint)server.Client.LocalEndPoint!).Port, 1, (_, _) => new EmptyOutput());
        }
        Check(true, "20 immediate connect/disconnect cycles complete without deadlock");

        var app = new App(); app.InitializeComponent();
        var main = new MainWindow();
        var settings = new SettingsWindow([new("CABLE Input (VB-Audio Virtual Cable)", "test")], "8125", 0, "test");
        Render(main, "windows-main", 504, 561);
        Render(main, "windows-main-small", 444, 541);
        Render(settings, "windows-settings", 454, 381);
        Check(!ContainsScroll((DependencyObject)main.Content), "native main window has no scroll container");
        Check(((Slider)main.FindName("GainSlider")).Maximum == 1000, "output boost is available in the native UI");
    }
    private static bool ContainsScroll(DependencyObject item)
    {
        if (item is ScrollViewer scroll) return scroll.ComputedVerticalScrollBarVisibility == Visibility.Visible || scroll.ComputedHorizontalScrollBarVisibility == Visibility.Visible;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(item); i++)
            if (ContainsScroll(VisualTreeHelper.GetChild(item, i))) return true;
        return false;
    }
    private static void Render(Window window, string name, int width, int height)
    {
        var content = (FrameworkElement)window.Content;
        var surface = new Border { Background = window.Background };
        TextElement.SetForeground(surface, window.Foreground);
        TextElement.SetFontFamily(surface, window.FontFamily);
        window.Content = null;
        surface.Child = content;
        surface.Measure(new Size(width, height));
        surface.Arrange(new Rect(0, 0, width, height)); surface.UpdateLayout();
        var bitmap = new RenderTargetBitmap(width, height, 96, 96, PixelFormats.Pbgra32); bitmap.Render(surface);
        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap));
        Directory.CreateDirectory("artifacts/tests");
        using (var file = File.Create($"artifacts/tests/{name}.png")) encoder.Save(file);
        surface.Child = null; window.Content = content;
    }
    private sealed class EmptyOutput : IDisposable { public void Dispose() { } }
    private sealed class SlowOutput : IDisposable
    {
        public readonly TaskCompletionSource Started = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public readonly ManualResetEventSlim Release = new();
        public int Disposals;
        public void Dispose() { Disposals++; Started.TrySetResult(); Release.Wait(TimeSpan.FromSeconds(5)); }
    }
}
