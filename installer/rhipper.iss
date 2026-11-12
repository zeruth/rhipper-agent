; The Windows installer of Rhipper Agent: rc.exe and rc-gui.exe where `rc install` puts them,
; %LOCALAPPDATA%\Programs\rhipper, on the user's PATH, with a Start Menu entry for the window.
; Per user, no elevation. The release workflow passes AppVersion, BinDir and OutDir.

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif

[Setup]
AppId={{7B2F0E4C-9A4D-4D7E-9C1B-RHIPPERAGENT}
AppName=Rhipper Agent
AppVersion={#AppVersion}
AppVerName=Rhipper Agent {#AppVersion}
AppPublisher=zeruth
AppPublisherURL=https://github.com/zeruth/rhipper-agent
DefaultDirName={localappdata}\Programs\rhipper
DisableDirPage=no
DefaultGroupName=Rhipper Agent
PrivilegesRequired=lowest
OutputDir={#OutDir}
OutputBaseFilename=rhipper-agent-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesEnvironment=yes
UninstallDisplayIcon={app}\rc-gui.exe
WizardStyle=modern

[Files]
Source: "{#BinDir}\rc.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\rc-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\scripts\shot-window.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
Source: "..\.claude\skills\rhipper-pair\SKILL.md"; DestDir: "{app}\skills\rhipper-pair"; Flags: ignoreversion

[Icons]
Name: "{group}\Rhipper Agent"; Filename: "{app}\rc-gui.exe"
Name: "{group}\Rhipper Agent on GitHub"; Filename: "https://github.com/zeruth/rhipper-agent"
Name: "{userdesktop}\Rhipper Agent"; Filename: "{app}\rc-gui.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "A desktop shortcut to the window"; GroupDescription: "Shortcuts:"

[Registry]
; the install directory on the user's PATH, once; `rc` and `rc-gui` answer in a new terminal
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\rc-gui.exe"; Description: "Open the window"; Flags: nowait postinstall skipifsilent

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Lowercase(Param) + ';', ';' + Lowercase(OrigPath) + ';') = 0;
end;
