using System.Windows;

namespace ToasterMic.Windows;

public partial class SettingsWindow : Window
{
    public string Port => PortBox.Text.Trim();
    public int BufferIndex => BufferBox.SelectedIndex;
    public MainWindow.OutputDevice? Output => OutputBox.SelectedItem as MainWindow.OutputDevice;
    public SettingsWindow(List<MainWindow.OutputDevice> devices, string port, int buffer, string? output)
    {
        InitializeComponent();
        OutputBox.ItemsSource = devices;
        OutputBox.SelectedItem = devices.FirstOrDefault(d => d.Id == output);
        PortBox.Text = port; BufferBox.SelectedIndex = Math.Clamp(buffer, 0, 3);
    }
    private void SaveClick(object sender, RoutedEventArgs e)
    {
        if (!int.TryParse(Port, out int port) || port is < 1024 or > 65535) { ErrorText.Text = "Port: 1024–65535"; return; }
        if (Output == null) { ErrorText.Text = "Audioziel auswählen."; return; }
        DialogResult = true;
    }
}
