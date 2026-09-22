using System.ComponentModel;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text.Json;
using System.Windows;
using System.Windows.Threading;
using NAudio.CoreAudioApi;

namespace ToasterMic.Windows;

public partial class MainWindow : Window
{
    private Receiver? receiver;
    private bool busy, closeRequested, canClose;
    private List<OutputDevice> devices = [];
    private Settings settings = new("192.168.1.100", "8125", 100, 1, null);
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromMilliseconds(150) };
    private static readonly string SettingsPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "ToasterMic", "settings.json");
    public MainWindow()
    {
        InitializeComponent();
        try { settings = JsonSerializer.Deserialize<Settings>(File.ReadAllText(SettingsPath)) ?? settings; }
        catch (Exception e) when (e is IOException or JsonException or UnauthorizedAccessException) { }
        HostBox.Text = settings.Host; GainSlider.Value = double.IsFinite(settings.Gain) ? settings.Gain : 100;
        timer.Tick += (_, _) =>
        {
            if (receiver == null) return;
            StatusText.Text = receiver.Status; StatusText.ToolTip = receiver.Status;
            var s = receiver.Buffer.Stats;
            StatsText.Text = $"Puffer {s.BufferedMs} ms · Ziel {receiver.Buffer.TargetMs} ms";
            StatsText.ToolTip = $"Pakete {s.Received} · Fehlend {s.Lost} · Zu spät {s.Late} · Neusynchronisiert {s.Resyncs} · In Sprechpausen gekürzt {receiver.Buffer.TrimmedPackets * 5} ms";
            Meter.Value = s.Peak;
        };
        Loaded += async (_, _) => { await RefreshDevices(); if (!closeRequested && !canClose) timer.Start(); };
    }
    private async Task RefreshDevices()
    {
        SetBusy(true);
        try
        {
            devices = await Task.Run(() =>
            {
                using var enumerator = new MMDeviceEnumerator();
                var found = new List<OutputDevice>();
                foreach (var device in enumerator.EnumerateAudioEndPoints(DataFlow.Render, DeviceState.Active))
                    using (device) found.Add(new(device.FriendlyName, device.ID));
                return found;
            });
            var selected = devices.FirstOrDefault(d => d.Id == settings.Output)
                ?? devices.FirstOrDefault(d => d.Name.Contains("CABLE Input", StringComparison.OrdinalIgnoreCase));
            settings = settings with { Output = selected?.Id };
            OutputText.Text = selected?.Name ?? "Audioziel in den Einstellungen auswählen.";
            OutputText.ToolTip = OutputText.Text;
        }
        catch (Exception e) { StatusText.Text = "Audiogeräte: " + e.Message; }
        finally { SetBusy(false); }
    }
    private void SetBusy(bool value)
    {
        busy = value;
        ConnectButton.IsEnabled = !value;
        SettingsButton.IsEnabled = ConnectionSettings.IsEnabled = !value && receiver == null;
        if (!value && closeRequested) Dispatcher.BeginInvoke(new Action(Close));
    }
    private async void ConnectClick(object sender, RoutedEventArgs e)
    {
        if (busy) return;
        if (receiver != null) { await Disconnect(); return; }
        SetBusy(true);
        try
        {
            if (!IPAddress.TryParse(HostBox.Text.Trim(), out var host) || host.AddressFamily != AddressFamily.InterNetwork || host.Equals(IPAddress.Any) || host.Equals(IPAddress.Broadcast)) throw new ArgumentException("Gültige Handy-IP eingeben.");
            if (!int.TryParse(settings.Port, out int port) || port is < 1024 or > 65535) throw new ArgumentException("Port muss zwischen 1024 und 65535 liegen.");
            var device = devices.FirstOrDefault(d => d.Id == settings.Output) ?? throw new ArgumentException("In den Einstellungen CABLE Input auswählen.");
            int packets = new[] { 1, 3, 6, 12 }[Math.Clamp(settings.Buffer, 0, 3)];
            StatusText.Text = "Verbinde …";
            receiver = await Task.Run(() => new Receiver(host, port, device.Id, packets));
            receiver.Buffer.Gain = (float)GainSlider.Value / 100; receiver.Buffer.Muted = MuteBox.IsChecked == true;
            ConnectButton.Content = "Trennen";
            SaveSettings();
        }
        catch (Exception ex) { StatusText.Text = ex.Message; }
        finally { SetBusy(false); }
    }
    private async Task Disconnect()
    {
        var previous = receiver; receiver = null;
        SetBusy(true); StatusText.Text = "Trenne …";
        try { if (previous != null) await previous.DisposeAsync(); StatusText.Text = "Getrennt"; }
        catch (Exception e) { StatusText.Text = "Audioausgabe beendet: " + e.Message; }
        finally
        {
            ConnectButton.Content = "Verbinden"; StatsText.Text = "Keine Übertragung"; StatsText.ToolTip = null; Meter.Value = 0;
            SetBusy(false);
        }
    }
    private void SaveSettings()
    {
        settings = settings with { Host = HostBox.Text.Trim(), Gain = GainSlider.Value };
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(SettingsPath)!);
            File.WriteAllText(SettingsPath, JsonSerializer.Serialize(settings));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { }
    }
    private void OnGainChanged(object sender, RoutedPropertyChangedEventArgs<double> e)
    {
        if (GainText != null) GainText.Text = e.NewValue == 0 ? "0 %" : $"{e.NewValue:0} % · {20 * Math.Log10(e.NewValue / 100):+0.0;-0.0;0} dB";
        if (receiver != null) receiver.Buffer.Gain = (float)e.NewValue / 100;
    }
    private void OnMuteChanged(object sender, RoutedEventArgs e) { if (receiver != null) receiver.Buffer.Muted = MuteBox.IsChecked == true; }
    private async void SettingsClick(object sender, RoutedEventArgs e)
    {
        if (busy || receiver != null) return;
        await RefreshDevices();
        if (closeRequested || canClose) return;
        var dialog = new SettingsWindow(devices, settings.Port, settings.Buffer, settings.Output) { Owner = this };
        if (dialog.ShowDialog() == true)
        {
            settings = settings with { Port = dialog.Port, Buffer = dialog.BufferIndex, Output = dialog.Output?.Id };
            OutputText.Text = dialog.Output?.Name ?? "Audioziel auswählen"; OutputText.ToolTip = OutputText.Text;
            SaveSettings();
        }
    }
    private async void OnClosing(object? sender, CancelEventArgs e)
    {
        if (canClose) return;
        e.Cancel = true; closeRequested = true;
        if (busy) return;
        timer.Stop(); SaveSettings();
        closeRequested = false;
        await Disconnect();
        canClose = true; Close();
    }
    public sealed record OutputDevice(string Name, string Id);
    private sealed record Settings(string Host, string Port, double Gain, int Buffer, string? Output);
}
