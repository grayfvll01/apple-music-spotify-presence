; Installer for Apple Music & Spotify Presence (Inno Setup 6).
; Built by CI:  iscc /DAppVersion=1.2.3 installer\setup.iss
; Per-user install (no admin prompt): %LOCALAPPDATA%\Programs, Start menu,
; optional desktop icon and "Start with Windows". Also used for updates:
; the app downloads the new setup and runs it with /SILENT.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppName "Apple Music & Spotify Presence"
#define AppExe "AppleMusicSpotifyPresence.exe"
#define AppId "AppleMusicSpotifyPresence"
#define Repo "https://github.com/grayfvll01/apple-music-spotify-presence"
; The name up to 1.1.x: an update from it moves everything over.
#define OldName "Apple Music Discord Presence"
#define OldExe "AppleMusicDiscordPresence.exe"
#define OldId "AppleMusicDiscordPresence"

[Setup]
AppId={{8C3B6E2A-5D1F-4B7E-9A64-2F0D7C1E5B93}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=grayfvll01
AppPublisherURL={#Repo}
AppSupportURL={#Repo}/issues
AppUpdatesURL={#Repo}/releases
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=yes
DisableReadyPage=yes
; Always the folder named after the app (older versions used another name).
UsePreviousAppDir=no
OutputDir=..\target\installer
OutputBaseFilename=AppleMusicSpotifyPresence-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
; Close a running copy (it clears its Discord status on the way out).
CloseApplications=force
; Leave a log in %TEMP% ("Setup Log <date>.txt") for troubleshooting updates.
SetupLogging=yes
RestartApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

[Tasks]
; Offered on the first install only; updates keep the choices.
Name: "desktopicon"; Description: "Add a desktop icon"; Check: not IsUpgrade
Name: "startup"; Description: "Start with Windows"; Check: not IsUpgrade

[InstallDelete]
; The app under its old name: its program folder and shortcuts.
Type: files; Name: "{autopf}\{#OldName}\{#OldExe}"
Type: files; Name: "{autopf}\{#OldName}\unins000.exe"
Type: files; Name: "{autopf}\{#OldName}\unins000.dat"
Type: dirifempty; Name: "{autopf}\{#OldName}"
Type: files; Name: "{autoprograms}\{#OldName}.lnk"
Type: files; Name: "{autodesktop}\{#OldName}.lnk"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Show your Apple Music and Spotify songs on Discord"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Check: WantDesktopIcon

[Registry]
; Same entry the app's own "Start with Windows" switch uses, so they agree.
; (An entry under the old name is moved over in [Code].)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; \
  ValueName: "{#AppId}"; ValueData: """{app}\{#AppExe}"""; Tasks: startup

[Run]
; Also runs after silent updates, so the new version comes straight back.
; ("&&": a check box caption would take a single "&" as a shortcut key.)
Filename: "{app}\{#AppExe}"; Description: "Start {#StringChange(AppName, "&", "&&")} now"; Flags: nowait postinstall

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';

var
  HadOldDesktopIcon: Boolean;

function IsUpgrade: Boolean;
begin
  Result := RegValueExists(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{8C3B6E2A-5D1F-4B7E-9A64-2F0D7C1E5B93}_is1', 'UninstallString');
end;

{ Asks a running copy to quit (it clears its Discord status), then makes sure. }
procedure StopExe(Exe: String);
var
  Code: Integer;
begin
  if Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM ' + Exe, '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0) then
  begin
    Sleep(1500);
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ' + Exe, '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  { A copy under the old name isn't one of the files being replaced, so
    closing it is up to us; and its desktop icon comes back renamed. }
  StopExe('{#OldExe}');
  HadOldDesktopIcon := FileExists(ExpandConstant('{autodesktop}\{#OldName}.lnk'));
  Result := '';
end;

function WantDesktopIcon: Boolean;
begin
  if HadOldDesktopIcon then
    Result := True
  else if IsUpgrade then
    Result := False
  else
    Result := WizardIsTaskSelected('desktopicon');
end;

{ "Start with Windows" under the old name moves to the new exe (even if the
  app isn't started after this install), still off if it was switched off
  in Task Manager. }
procedure MoveStartupEntry;
var
  Cmd: String;
  Flag: AnsiString;
begin
  if RegQueryStringValue(HKCU, RunKey, '{#OldId}', Cmd) then
  begin
    RegWriteStringValue(HKCU, RunKey, '{#AppId}', '"' + ExpandConstant('{app}\{#AppExe}') + '"');
    if RegQueryBinaryValue(HKCU, ApprovedKey, '{#OldId}', Flag) then
      RegWriteBinaryValue(HKCU, ApprovedKey, '{#AppId}', Flag);
    RegDeleteValue(HKCU, RunKey, '{#OldId}');
    RegDeleteValue(HKCU, ApprovedKey, '{#OldId}');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    MoveStartupEntry;
end;

function InitializeUninstall: Boolean;
begin
  StopExe('{#AppExe}');
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKCU, RunKey, '{#AppId}');
    RegDeleteValue(HKCU, ApprovedKey, '{#AppId}');
    RegDeleteValue(HKCU, RunKey, '{#OldId}');
    RegDeleteValue(HKCU, ApprovedKey, '{#OldId}');
  end;
end;
