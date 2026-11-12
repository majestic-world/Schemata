; Standard Windows installer. Paths are relative to this script, not the compiler's working directory.
#ifndef ProjectDir
  #define ProjectDir SourcePath
#endif
#define MyAppName "Schemata"
#define MyAppPublisher "Majestic World Studio"
#define MyAppExeName "schematadb.exe"
#define AppBinary ProjectDir + "\target\package-native\win32\release\" + MyAppExeName
#define MyAppVersion GetStringFileInfo(AppBinary, "ProductVersion")
#if MyAppVersion == ""
  #error Build the release executable with make build before compiling this installer.
#endif

[Setup]
AppId={{95189A8A-FB7C-4DDE-A6E0-708C649084B2}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
VersionInfoVersion={#GetVersionNumbersString(AppBinary)}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExeName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
PrivilegesRequired=admin
OutputDir={#ProjectDir}\package\windows
OutputBaseFilename=Schemata_{#MyAppVersion}_x64-setup
SetupIconFile={#ProjectDir}\src-tauri\icons\icon.ico
LicenseFile={#ProjectDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern windows11
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The frontend and SQLite are embedded in the executable; no build DLLs, PDBs or source files ship.
Source: "{#AppBinary}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectDir}\THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[CustomMessages]
english.WebViewRequired=Microsoft Edge WebView2 Runtime is required. Check your internet connection and run Setup again.
brazilianportuguese.WebViewRequired=O Microsoft Edge WebView2 Runtime é necessário. Verifique sua conexão com a internet e execute o instalador novamente.

[Code]
function HasWebView2: Boolean;
var
  Version: String;
begin
  Result := (RegQueryStringValue(HKLM32,
    'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKCU,
    'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if HasWebView2 then
    Exit;
  try
    DownloadTemporaryFile('https://go.microsoft.com/fwlink/p/?LinkId=2124703',
      'MicrosoftEdgeWebview2Setup.exe', '', nil);
    if not Exec(ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'),
      '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    begin
      Result := CustomMessage('WebViewRequired');
      Exit;
    end;
    if (ExitCode <> 0) or not HasWebView2 then
      Result := CustomMessage('WebViewRequired');
  except
    Log(GetExceptionMessage);
    Result := CustomMessage('WebViewRequired');
  end;
end;
