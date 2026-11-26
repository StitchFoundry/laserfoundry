; Inno Setup script for the LaserFoundry Windows installer (https://jrsoftware.org/ishelp/).
; Built by packaging/windows/build.ps1, which passes /DAppVersion=... /DBinDir=... /DOutDir=... on the command line.
; Installs per user by default (no administrator rights); the user may choose all users in the dialog.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "bin"
#endif
#ifndef OutDir
  #define OutDir "."
#endif

[Setup]
AppId={{6C1F5B1E-4D0B-4E52-9B0F-3A1C7E5D2F10}
AppName=LaserFoundry
AppVersion={#AppVersion}
AppPublisher=StitchFoundry
AppPublisherURL=https://github.com/StitchFoundry/laserfoundry
DefaultDirName={autopf}\LaserFoundry
DefaultGroupName=LaserFoundry
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=laserfoundry-windows-x64-setup
SetupIconFile={#BinDir}\laserfoundry.ico
UninstallDisplayIcon={app}\laserfoundry.exe
LicenseFile={#BinDir}\LICENSE-MIT
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "addtopath"; Description: "Add the command-line tool (laserfoundry-cli) to &PATH"; Flags: unchecked

[Files]
Source: "{#BinDir}\laserfoundry.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\laserfoundry-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\LaserFoundry"; Filename: "{app}\laserfoundry.exe"
Name: "{group}\Uninstall LaserFoundry"; Filename: "{uninstallexe}"
Name: "{autodesktop}\LaserFoundry"; Filename: "{app}\laserfoundry.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; \
  Tasks: addtopath; Check: PerUserInstall and NeedsPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\laserfoundry.exe"; Description: "Launch LaserFoundry"; Flags: nowait postinstall skipifsilent

[Code]
function PerUserInstall: Boolean;
begin
  Result := not IsAdminInstallMode;
end;

function NeedsPath(Dir: string): Boolean;
var
  Existing: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Existing) then
    Existing := '';
  Result := Pos(';' + Lowercase(Dir) + ';', ';' + Lowercase(Existing) + ';') = 0;
end;
