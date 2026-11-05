; FireCraft Windows installer (Inno Setup 6). Built by .github/workflows/release.yml:
;   set FIRECRAFT_VERSION=0.4.0 && iscc packaging\windows\firecraft.iss
; Per-user install (no admin prompt), Start-menu + desktop shortcut, opens .fcproj files.

#define AppVersion GetEnv("FIRECRAFT_VERSION")
#if AppVersion == ""
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{5E1B6C1E-7F3A-4C1B-9B57-2B7C1F0E9A11}
AppName=FireCraft
AppVersion={#AppVersion}
AppVerName=FireCraft {#AppVersion}
AppPublisher=Circuit Biscuit
DefaultDirName={autopf}\FireCraft
DefaultGroupName=FireCraft
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=FireCraft-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\logo.ico
UninstallDisplayIcon={app}\FireCraft.exe
UninstallDisplayName=FireCraft
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\..\target\release\firecraft.exe"; DestDir: "{app}"; DestName: "FireCraft.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\FireCraft"; Filename: "{app}\FireCraft.exe"
Name: "{autodesktop}\FireCraft"; Filename: "{app}\FireCraft.exe"; Tasks: desktopicon

[Registry]
; double-click a .fcproj project to open it in FireCraft
Root: HKA; Subkey: "Software\Classes\.fcproj"; ValueType: string; ValueName: ""; ValueData: "FireCraft.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\FireCraft.Project"; ValueType: string; ValueName: ""; ValueData: "FireCraft Project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\FireCraft.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\FireCraft.exe,0"
Root: HKA; Subkey: "Software\Classes\FireCraft.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\FireCraft.exe"" ""%1"""

[Run]
Filename: "{app}\FireCraft.exe"; Description: "Launch FireCraft"; Flags: nowait postinstall skipifsilent
