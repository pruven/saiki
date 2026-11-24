#ifndef AppVersion
  #error AppVersion must be supplied by the build script
#endif
#ifndef AppArch
  #error AppArch must be supplied by the build script
#endif

[Setup]
AppId=Saiki
AppName=Saiki
AppVersion={#AppVersion}
AppPublisher=Saiki
AppPublisherURL=https://github.com/pruven/saiki
DefaultDirName={localappdata}\Programs\Saiki
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\saiki.exe
OutputDir=..\..\release
OutputBaseFilename=saiki-windows-{#AppArch}-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=saiki.exe
RestartApplications=no
#if AppArch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
MinVersion=10.0.22000
#else
ArchitecturesAllowed=x64compatible and not arm64
ArchitecturesInstallIn64BitMode=x64compatible and not arm64
MinVersion=10.0.14393
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\build\saiki.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Saiki"; Filename: "{app}\saiki.exe"
Name: "{autodesktop}\Saiki"; Filename: "{app}\saiki.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\saiki.exe"; Description: "{cm:LaunchProgram,Saiki}"; Flags: nowait postinstall skipifsilent

