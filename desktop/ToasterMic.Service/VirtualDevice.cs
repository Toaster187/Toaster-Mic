using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace ToasterMic.Service;

internal static class VirtualDevice
{
    public const string HardwareId = @"ROOT\TOASTERMIC";
    private static Guid mediaClass = new("4d36e96c-e325-11ce-bfc1-08002be10318");
    [StructLayout(LayoutKind.Sequential)] private struct DeviceInfo { public int Size; public Guid ClassGuid; public uint DevInst; public UIntPtr Reserved; }
    public static IReadOnlyList<uint> Find()
    {
        var result = new List<uint>();
        IntPtr list = SetupDiGetClassDevs(ref mediaClass, null, IntPtr.Zero, 2);
        if (list == new IntPtr(-1)) throw new Win32Exception();
        try
        {
            for (uint i = 0; ; i++)
            {
                var info = new DeviceInfo { Size = Marshal.SizeOf<DeviceInfo>() };
                if (!SetupDiEnumDeviceInfo(list, i, ref info)) { if (Marshal.GetLastWin32Error() != 259) throw new Win32Exception(); break; }
                byte[] ids = new byte[4096];
                if (!SetupDiGetDeviceRegistryProperty(list, ref info, 1, out _, ids, (uint)ids.Length, out uint size)) continue;
                if (Encoding.Unicode.GetString(ids, 0, (int)size).Split('\0', StringSplitOptions.RemoveEmptyEntries).Any(x => x.Equals(HardwareId, StringComparison.OrdinalIgnoreCase))) result.Add(info.DevInst);
            }
        }
        finally { SetupDiDestroyDeviceInfoList(list); }
        return result;
    }
    public static void SetEnabled(bool enabled)
    {
        var devices = Find();
        if (enabled && devices.Count != 1) throw new InvalidOperationException("Toaster-Mic-Treiber fehlt oder ist mehrfach installiert.");
        foreach (uint node in devices)
        {
            uint error = enabled ? CM_Enable_DevNode(node, 0) : CM_Disable_DevNode(node, 0x1 | 0x4 | 0x8); // ABSOLUTE | UI_NOT_OK | PERSIST, never HARDWARE
            if (error != 0) throw new InvalidOperationException($"Toaster-Mic-Gerät konnte nicht {(enabled ? "aktiviert" : "deaktiviert")} werden (CM 0x{error:X}).");
            if (!enabled)
            {
                error = CM_Get_DevNode_Status(out _, out uint problem, node, 0);
                if (error != 0 || problem != 22) throw new InvalidOperationException("Windows hat die Deaktivierung des Toaster-Mic-Geräts nicht bestätigt.");
            }
        }
    }
    public static async Task<SafeFileHandle> OpenFeed(CancellationToken token)
    {
        for (int i = 0; i < 40; i++)
        {
            var handle = CreateFile(@"\\.\ToasterMicFeed", 0x40000000, 0, IntPtr.Zero, 3, 0, IntPtr.Zero);
            if (!handle.IsInvalid) return handle;
            handle.Dispose(); await Task.Delay(50, token);
        }
        throw new Win32Exception(Marshal.GetLastWin32Error(), "Treiber-Audiokanal nicht verfügbar.");
    }
    public static void VerifyInstallation()
    {
        var nodes = Find();
        if (nodes.Count != 1) throw new InvalidOperationException("Es muss genau ein Toaster-Mic-Gerät installiert sein.");
        uint error = CM_Get_DevNode_Status(out _, out uint problem, nodes[0], 0);
        if (error != 0 || problem != 22) throw new InvalidOperationException("Das installierte Mikrofon ist nicht bestätigt deaktiviert.");
        using var pipe = new System.IO.Pipes.NamedPipeClientStream(".", "ToasterMic.Audio.v1", System.IO.Pipes.PipeDirection.InOut);
        pipe.Connect(5000); // The bridge is ready. No audio/handshake -> device stays disabled.
    }
    public static void Feed(SafeFileHandle handle, byte[] pcm)
    {
        if (pcm.Length != 960) throw new ArgumentException("Expected 5 ms stereo PCM16.");
        if (!DeviceIoControl(handle, 0x22A004, pcm, 960, IntPtr.Zero, 0, out _, IntPtr.Zero)) throw new Win32Exception();
    }
    public static void CreateRootDevice(string inf)
    {
        if (Find().Count == 0)
        {
            IntPtr list = SetupDiCreateDeviceInfoList(ref mediaClass, IntPtr.Zero);
            if (list == new IntPtr(-1)) throw new Win32Exception();
            try
            {
                var info = new DeviceInfo { Size = Marshal.SizeOf<DeviceInfo>() };
                if (!SetupDiCreateDeviceInfo(list, "ToasterMic", ref mediaClass, "Toaster Mic Virtual Microphone", IntPtr.Zero, 1, ref info)) throw new Win32Exception();
                byte[] hardware = Encoding.Unicode.GetBytes(HardwareId + "\0\0");
                if (!SetupDiSetDeviceRegistryProperty(list, ref info, 1, hardware, (uint)hardware.Length) || !SetupDiCallClassInstaller(0x19, list, ref info)) throw new Win32Exception();
            }
            finally { SetupDiDestroyDeviceInfoList(list); }
        }
        try
        {
            if (!UpdateDriverForPlugAndPlayDevices(IntPtr.Zero, HardwareId, inf, 1, out bool reboot)) throw new Win32Exception();
            if (reboot) throw new InvalidOperationException("Treiber installiert. Windows-Neustart erforderlich; Setup danach erneut ausführen.");
            SetEnabled(true);
            // Validate that Windows actually loaded the driver, not merely copied its INF.
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            using var feed = OpenFeed(timeout.Token).GetAwaiter().GetResult();
        }
        finally { SetEnabled(false); }
    }
    [DllImport("setupapi.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr SetupDiGetClassDevs(ref Guid guid, string? enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError=true)] private static extern bool SetupDiEnumDeviceInfo(IntPtr list, uint index, ref DeviceInfo info);
    [DllImport("setupapi.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool SetupDiGetDeviceRegistryProperty(IntPtr list, ref DeviceInfo info, uint property, out uint type, byte[] buffer, uint size, out uint required);
    [DllImport("setupapi.dll")] private static extern bool SetupDiDestroyDeviceInfoList(IntPtr list);
    [DllImport("cfgmgr32.dll")] private static extern uint CM_Enable_DevNode(uint node, uint flags);
    [DllImport("cfgmgr32.dll")] private static extern uint CM_Disable_DevNode(uint node, uint flags);
    [DllImport("cfgmgr32.dll")] private static extern uint CM_Get_DevNode_Status(out uint status, out uint problem, uint node, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern SafeFileHandle CreateFile(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern bool DeviceIoControl(SafeFileHandle handle, uint code, byte[] input, uint inputSize, IntPtr output, uint outputSize, out uint bytes, IntPtr overlapped);
    [DllImport("setupapi.dll", SetLastError=true)] private static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid guid, IntPtr parent);
    [DllImport("setupapi.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool SetupDiCreateDeviceInfo(IntPtr list, string name, ref Guid guid, string description, IntPtr parent, uint flags, ref DeviceInfo info);
    [DllImport("setupapi.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool SetupDiSetDeviceRegistryProperty(IntPtr list, ref DeviceInfo info, uint property, byte[] data, uint size);
    [DllImport("setupapi.dll", SetLastError=true)] private static extern bool SetupDiCallClassInstaller(uint function, IntPtr list, ref DeviceInfo info);
    [DllImport("newdev.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool UpdateDriverForPlugAndPlayDevices(IntPtr parent, string hardwareId, string inf, uint flags, out bool reboot);
}
