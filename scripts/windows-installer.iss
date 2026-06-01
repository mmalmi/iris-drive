#define AppVersion GetEnv("IRIS_DRIVE_RELEASE_VERSION")
#define SourceDir GetEnv("IRIS_DRIVE_WINDOWS_PUBLISH_DIR")
#define OutputDir GetEnv("IRIS_DRIVE_WINDOWS_INSTALLER_OUTPUT_DIR")
#define OutputBaseName GetEnv("IRIS_DRIVE_WINDOWS_INSTALLER_BASENAME")
#define ProjectRoot GetEnv("IRIS_DRIVE_PROJECT_ROOT")

[Setup]
AppId={{7EB7D560-CFAE-4D0F-9F07-4095C078D7CC}
AppName=Iris Drive
AppVersion={#AppVersion}
AppPublisher=Iris
DefaultDirName={localappdata}\Programs\Iris Drive
DefaultGroupName=Iris Drive
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
PrivilegesRequired=lowest
SetupIconFile={#ProjectRoot}\windows\IrisDrive.ico
UninstallDisplayIcon={app}\IrisDrive.exe

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Iris Drive"; Filename: "{app}\IrisDrive.exe"; IconFilename: "{app}\IrisDrive.ico"
Name: "{autodesktop}\Iris Drive"; Filename: "{app}\IrisDrive.exe"; IconFilename: "{app}\IrisDrive.ico"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\IrisDrive.exe"; Description: "Launch Iris Drive"; Flags: nowait postinstall skipifsilent
