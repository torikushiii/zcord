; Built by scripts/package-windows.ps1. With PrivilegesRequired=lowest, {autopf} resolves to
; %LOCALAPPDATA%\Programs, so installing needs no elevation.

#ifndef AppVersion
  #error AppVersion must be defined (/DAppVersion=x.y.z)
#endif
#ifndef Arch
  #error Arch must be defined (/DArch=x86_64 or /DArch=aarch64)
#endif
#ifndef PackageDir
  #error PackageDir must be defined (/DPackageDir=<staged package directory>)
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

#if Arch == "aarch64"
  #define ArchAllowed "arm64"
#else
  #define ArchAllowed "x64compatible"
#endif

[Setup]
; Never change AppId: it identifies the installation across upgrades.
AppId={{756456F8-EFB7-4F9C-90AF-7A0DA1F66902}
AppName=zcord
AppVersion={#AppVersion}
AppVerName=zcord {#AppVersion}
AppPublisher=zcord
AppPublisherURL=https://github.com/torikushiii/zcord
AppSupportURL=https://github.com/torikushiii/zcord/issues
AppUpdatesURL=https://github.com/torikushiii/zcord/releases
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\zcord
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchAllowed}
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=zcord-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile=..\..\assets\zcord.ico
UninstallDisplayIcon={app}\zcord.exe
UninstallDisplayName=zcord
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PackageDir}\zcord.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\zcord"; Filename: "{app}\zcord.exe"
Name: "{autodesktop}\zcord"; Filename: "{app}\zcord.exe"; Tasks: desktopicon

[Registry]
; The launch-on-startup setting (src/startup.rs) writes this value; remove it on uninstall
; without creating it on install.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "zcord"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\zcord.exe"; Description: "{cm:LaunchProgram,zcord}"; Flags: nowait postinstall skipifsilent
