# WinVerifyTrust verifies the file digest as well as the Authenticode signature.
# PowerShell reports CERT_E_UNTRUSTEDROOT as UnknownError on some Windows builds.
if (-not ('ToasterMic.BuildSignature' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace ToasterMic {
    public static class BuildSignature {
        [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
        private struct FileInfo {
            public uint Size;
            [MarshalAs(UnmanagedType.LPWStr)] public string Path;
            public IntPtr Handle, Subject;
        }
        [StructLayout(LayoutKind.Sequential)]
        private struct TrustData {
            public uint Size;
            public IntPtr Policy, Sip;
            public uint Ui, Revocation, Choice;
            public IntPtr File;
            public uint StateAction;
            public IntPtr StateData, Url;
            public uint Flags, Context;
            public IntPtr Settings;
        }
        [DllImport("wintrust.dll", ExactSpelling=true)]
        private static extern uint WinVerifyTrust(IntPtr window, ref Guid action, ref TrustData data);
        public static uint Verify(string path) {
            var file = new FileInfo { Size=(uint)Marshal.SizeOf(typeof(FileInfo)), Path=path };
            IntPtr memory = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(FileInfo)));
            try {
                Marshal.StructureToPtr(file, memory, false);
                var data = new TrustData { Size=(uint)Marshal.SizeOf(typeof(TrustData)), Ui=2, Choice=1, File=memory, Flags=0x1010 };
                var action = new Guid("00AAC56B-CD44-11d0-8CC2-00C04FC295EE");
                return WinVerifyTrust(new IntPtr(-1), ref action, ref data);
            } finally { Marshal.DestroyStructure(memory, typeof(FileInfo)); Marshal.FreeHGlobal(memory); }
        }
    }
}
'@
}
