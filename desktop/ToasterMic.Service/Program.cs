using System.Diagnostics;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
using System.ServiceProcess;
using ToasterMic.Service;
using ToasterMic.Core;
using Microsoft.Win32.SafeHandles;

if (args.Length != 0)
{
    try
    {
        if (args.Length == 2 && args[0] == "--install-driver") VirtualDevice.CreateRootDevice(Path.GetFullPath(args[1]));
        else if (args.Length == 1 && args[0] == "--disable-device") VirtualDevice.SetEnabled(false);
        else if (args.Length == 1 && args[0] == "--verify-installation") VirtualDevice.VerifyInstallation();
        else if (args.Length == 1 && args[0] == "--diagnose") Console.WriteLine($"Toaster Mic devices: {VirtualDevice.Find().Count}");
        else throw new ArgumentException("Unbekanntes Setup-Kommando.");
    }
    catch (Exception e) { Console.Error.WriteLine(e.Message); Environment.ExitCode = 1; }
    return;
}
ServiceBase.Run(new MicService());

sealed class MicService : ServiceBase
{
    private readonly CancellationTokenSource stop = new();
    private Task? worker;
    public MicService() { ServiceName = "ToasterMicBridge"; CanShutdown = true; AutoLog = false; }
    protected override void OnStart(string[] args) { worker = Task.Run(Run); }
    protected override void OnStop() { stop.Cancel(); worker?.GetAwaiter().GetResult(); }
    protected override void OnShutdown() => OnStop();
    private async Task Disable()
    {
        // Fail closed: do not accept another client until Windows confirms disable.
        for (int i = 0; i < 10; i++)
        {
            try { VirtualDevice.SetEnabled(false); return; }
            catch (Exception e) { Log(e.Message); await Task.Delay(250); }
        }
        throw new InvalidOperationException("Windows verweigert die Deaktivierung des Toaster-Mic-Geräts.");
    }
    private static NamedPipeServerStream Pipe()
    {
        var security = new PipeSecurity();
        security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.NetworkSid, null), PipeAccessRights.FullControl, AccessControlType.Deny));
        security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.LocalSystemSid, null), PipeAccessRights.FullControl, AccessControlType.Allow));
        security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, null), PipeAccessRights.FullControl, AccessControlType.Allow));
        security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.InteractiveSid, null), PipeAccessRights.ReadWrite, AccessControlType.Allow));
        return NamedPipeServerStreamAcl.Create("ToasterMic.Audio.v1", PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous, 4096, 4096, security);
    }
    private async Task Run()
    {
        try
        {
            await Disable(); // Includes reboot and service crash recovery.
            while (!stop.IsCancellationRequested)
            {
                using var pipe = Pipe();
                try
                {
                    await pipe.WaitForConnectionAsync(stop.Token);
                    await MicrophoneBridgeSession.RunAsync(pipe, new DeviceAdapter(Disable), stop.Token);
                }
                catch (Exception e) when (e is IOException or OperationCanceledException or InvalidOperationException or System.ComponentModel.Win32Exception) { if (!stop.IsCancellationRequested) Log(e.Message); }
                finally { await Disable(); }
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception e) { Log(e.ToString()); Environment.Exit(1); } // SCM recovery restarts, then disables first.
        finally { try { await Disable(); } catch (Exception e) { Log(e.Message); } }
    }
    private static void Log(string text)
    {
        try { EventLog.WriteEntry("ToasterMicBridge", text, EventLogEntryType.Error); } catch { }
    }
    private sealed class DeviceAdapter(Func<Task> disable) : IVirtualMicrophoneDevice
    {
        private SafeFileHandle? handle;
        public async Task EnableAsync(CancellationToken token) { VirtualDevice.SetEnabled(true); handle = await VirtualDevice.OpenFeed(token); }
        public void Feed(byte[] pcm) { VirtualDevice.Feed(handle ?? throw new InvalidOperationException("Kein Treiberkanal"), pcm); }
        public async Task DisableAsync() { handle?.Dispose(); handle = null; await disable(); }
    }
}
