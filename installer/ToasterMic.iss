; Inno Setup script for the Windows app (per-user install, no administrator rights).
; Build: iscc /DAppVersion=0.5.0.42 /DSourceDir=..\artifacts\windows /DOutputDir=..\artifacts installer\ToasterMic.iss

#ifndef AppVersion
  #define AppVersion "0.5.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\artifacts\windows"
#endif
#ifndef OutputDir
  #define OutputDir "..\artifacts"
#endif

[Setup]
AppId={{6B0E2F0B-7C1D-4E55-9B1A-2E4C6F7D9A10}
AppName=Toaster Mic
AppVersion={#AppVersion}
AppVerName=Toaster Mic {#AppVersion}
AppPublisher=Toaster Mic
AppPublisherURL=https://github.com/Toaster187/Toaster-Mic
AppSupportURL=https://github.com/Toaster187/Toaster-Mic/issues
DefaultDirName={localappdata}\Programs\Toaster Mic
DefaultGroupName=Toaster Mic
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=ToasterMic-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\ToasterMic.exe
UninstallDisplayName=Toaster Mic
; Close a running instance (tray) before files are replaced.
AppMutex=Local\ToasterMic.Instance
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\ToasterMic.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\toaster-probe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs
Source: "{#SourceDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\Toaster Mic"; Filename: "{app}\ToasterMic.exe"
Name: "{group}\Toaster Mic deinstallieren"; Filename: "{uninstallexe}"
Name: "{userdesktop}\Toaster Mic"; Filename: "{app}\ToasterMic.exe"; Tasks: desktopicon

[Registry]
; The app writes this value itself ("Mit Windows starten"); remove it on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "ToasterMic"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\ToasterMic.exe"; Description: "{cm:LaunchProgram,Toaster Mic}"; Flags: nowait postinstall skipifsilent
