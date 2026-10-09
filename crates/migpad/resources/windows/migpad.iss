; The installer of MigPad for Windows, made by Inno Setup 6 (script/bundle-windows.ps1 runs it):
;
;   ISCC /DVersion=0.1.0 /DArch=x64 /DSource=<folder of migpad.exe> /DOutput=<folder> migpad.iss
;
; Arch is x64 or arm64. It installs for the user, without administrator rights, or for all users;
; offers MigPad in Open With for texts and among the default apps; and as chosen adds the command
; migpad to PATH, Open in MigPad to the context menu of Explorer, and puts MigPad in place of
; Notepad (for all users only). Uninstalling takes all of it back; the data in .migpad stays.

#ifndef Version
  #error Version is not defined
#endif
#ifndef Arch
  #error Arch is not defined
#endif
#ifndef Source
  #error Source is not defined
#endif
#ifndef Output
  #define Output "."
#endif
; Numbers only: 0.1.0 of 0.1.0-dev.
#define NumericVersion Copy(Version, 1, Pos("-", Version + "-") - 1)
#if Arch == "arm64"
  #define Architectures "arm64"
#else
  #define Architectures "x64compatible"
#endif

[Setup]
AppId={{EEB2E3F7-6E3D-4C7A-B372-67315FC34AB3}
AppName=MigPad
AppVersion={#Version}
AppVerName=MigPad {#Version}
AppPublisher=Iaroslav Vorobev
AppPublisherURL=https://migpad.com
AppSupportURL=https://github.com/migpad/migpad/issues
AppUpdatesURL=https://migpad.com
AppCopyright=Copyright (c) 2026 Iaroslav Vorobev
VersionInfoVersion={#NumericVersion}
VersionInfoProductName=MigPad
DefaultDirName={autopf}\MigPad
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed={#Architectures}
ArchitecturesInstallIn64BitMode={#Architectures}
; Windows 10 version 1903 and later.
MinVersion=10.0.18362
OutputDir={#Output}
OutputBaseFilename=MigPad-{#Arch}-setup
SetupIconFile=migpad.ico
UninstallDisplayIcon={app}\migpad.exe
UninstallDisplayName=MigPad
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ChangesAssociations=yes
ChangesEnvironment=yes
ShowLanguageDialog=auto

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
english.TaskPath=Add the migpad command to PATH
russian.TaskPath=Добавить команду migpad в PATH
english.TaskExplorer=Add Open in MigPad to the context menu of files in Explorer
russian.TaskExplorer=Добавить «Открыть в MigPad» в контекстное меню файлов в Проводнике
english.TaskNotepad=Open MigPad in place of Notepad (notepad.exe)
russian.TaskNotepad=Открывать MigPad вместо Блокнота (notepad.exe)
english.OpenInMigPad=Open in MigPad
russian.OpenInMigPad=Открыть в MigPad
english.TextDocument=Text Document
russian.TextDocument=Текстовый документ
english.Description=A fast text editor
russian.Description=Быстрый текстовый редактор

[Tasks]
Name: "path"; Description: "{cm:TaskPath}"
Name: "explorer"; Description: "{cm:TaskExplorer}"
Name: "notepad"; Description: "{cm:TaskNotepad}"; Flags: unchecked; Check: IsAdminInstallMode
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\migpad.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Source}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\MigPad"; Filename: "{app}\migpad.exe"
Name: "{autodesktop}\MigPad"; Filename: "{app}\migpad.exe"; Tasks: desktopicon

[Registry]
; Win+R knows migpad.
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\migpad.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\migpad.exe"; Flags: uninsdeletekey
; The application in Open With; its types are added below, in [Code].
Root: HKA; Subkey: "Software\Classes\Applications\migpad.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "MigPad"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\migpad.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\migpad.exe"",0"
Root: HKA; Subkey: "Software\Classes\Applications\migpad.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\migpad.exe"" ""%1"""
; The type of the documents MigPad opens.
Root: HKA; Subkey: "Software\Classes\MigPad.Text"; ValueType: string; ValueName: ""; ValueData: "{cm:TextDocument}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\MigPad.Text\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\migpad.exe"",0"
Root: HKA; Subkey: "Software\Classes\MigPad.Text\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\migpad.exe"" ""%1"""
; Among the default apps of Settings. The key of MigPad goes before its subkey, which is taken back
; first: then it is empty.
Root: HKA; Subkey: "Software\MigPad"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\MigPad\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "MigPad"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\MigPad\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "{cm:Description}"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "MigPad"; ValueData: "Software\MigPad\Capabilities"; Flags: uninsdeletevalue
; Open in MigPad, PATH and the place of Notepad follow their tasks in [Code], also when a task is
; unchecked on installing again.

[Run]
Filename: "{app}\migpad.exe"; Description: "{cm:LaunchProgram,MigPad}"; Flags: nowait postinstall skipifsilent

[Code]
const
  // The types of files MigPad offers itself for in Open With and among the default apps.
  Extensions = '.txt .text .log .ini .inf .cfg .conf .config .properties .toml .yaml .yml .json .jsonc .xml .csv .tsv .md .markdown .nfo .diz .srt .reg .bat .cmd .ps1 .sh .sql .c .h .cpp .hpp .cs .java .js .ts .py .rs .go .css .html .htm';

// The registry the installation writes to: of the machine for all users, of the user otherwise.
function Root: Integer;
begin
  if IsAdminInstallMode then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function EnvironmentKey: String;
begin
  if IsAdminInstallMode then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

// What starts in place of Notepad: Windows starts it with the command line of Notepad after it.
const
  NotepadKey = 'SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\notepad.exe';

function NotepadDebugger: String;
begin
  Result := '"' + ExpandConstant('{app}') + '\migpad.exe" --notepad';
end;

// Puts MigPad in place of Notepad, keeping what was there — another editor — to put it back.
procedure ReplaceNotepad;
var
  Before: String;
begin
  if RegQueryStringValue(HKLM, NotepadKey, 'Debugger', Before) then
    if Before <> NotepadDebugger then
      RegWriteStringValue(HKLM, 'Software\MigPad', 'NotepadBefore', Before);
  RegWriteStringValue(HKLM, NotepadKey, 'Debugger', NotepadDebugger);
end;

// Takes MigPad out of the place of Notepad, if it is there — another editor put there since is left
// alone — and puts back what was there before it.
procedure RestoreNotepad;
var
  Current, Before: String;
begin
  if RegQueryStringValue(HKLM, NotepadKey, 'Debugger', Current) then
    if Current = NotepadDebugger then
    begin
      if RegQueryStringValue(HKLM, 'Software\MigPad', 'NotepadBefore', Before) then
        RegWriteStringValue(HKLM, NotepadKey, 'Debugger', Before)
      else
      begin
        RegDeleteValue(HKLM, NotepadKey, 'Debugger');
        RegDeleteKeyIfEmpty(HKLM, NotepadKey);
      end;
    end;
  RegDeleteValue(HKLM, 'Software\MigPad', 'NotepadBefore');
end;

// Open in MigPad in the context menu of any file in Explorer.
procedure SetExplorerVerb(Install: Boolean);
var
  Key, Exe: String;
begin
  Key := 'Software\Classes\*\shell\MigPad';
  Exe := ExpandConstant('{app}') + '\migpad.exe';
  if Install then
  begin
    RegWriteStringValue(Root, Key, '', CustomMessage('OpenInMigPad'));
    RegWriteStringValue(Root, Key, 'Icon', '"' + Exe + '",0');
    RegWriteStringValue(Root, Key + '\command', '', '"' + Exe + '" "%1"');
  end
  else
    RegDeleteKeyIncludingSubkeys(Root, Key);
end;

// Writes the types when Install is true, takes them back otherwise.
procedure SetTypes(Install: Boolean);
var
  Rest, Extension: String;
  Space: Integer;
begin
  Rest := Extensions + ' ';
  while Rest <> '' do
  begin
    Space := Pos(' ', Rest);
    Extension := Copy(Rest, 1, Space - 1);
    Delete(Rest, 1, Space);
    if Extension = '' then
      continue;
    if Install then
    begin
      RegWriteStringValue(Root, 'Software\Classes\Applications\migpad.exe\SupportedTypes', Extension, '');
      RegWriteStringValue(Root, 'Software\Classes\' + Extension + '\OpenWithProgids', 'MigPad.Text', '');
      RegWriteStringValue(Root, 'Software\MigPad\Capabilities\FileAssociations', Extension, 'MigPad.Text');
    end
    else
    begin
      RegDeleteValue(Root, 'Software\Classes\' + Extension + '\OpenWithProgids', 'MigPad.Text');
      // Keys made for MigPad alone: of types Windows did not know.
      RegDeleteKeyIfEmpty(Root, 'Software\Classes\' + Extension + '\OpenWithProgids');
      RegDeleteKeyIfEmpty(Root, 'Software\Classes\' + Extension);
    end;
  end;
end;

procedure AddToPath(Folder: String);
var
  Paths: String;
begin
  if not RegQueryStringValue(Root, EnvironmentKey, 'Path', Paths) then
    Paths := '';
  if Pos(';' + Uppercase(Folder) + ';', ';' + Uppercase(Paths) + ';') > 0 then
    exit;
  if (Paths <> '') and (Copy(Paths, Length(Paths), 1) <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(Root, EnvironmentKey, 'Path', Paths + Folder);
end;

procedure RemoveFromPath(Folder: String);
var
  Paths: String;
  At: Integer;
begin
  if not RegQueryStringValue(Root, EnvironmentKey, 'Path', Paths) then
    exit;
  Paths := ';' + Paths + ';';
  At := Pos(';' + Uppercase(Folder) + ';', Uppercase(Paths));
  if At = 0 then
    exit;
  Delete(Paths, At, Length(Folder) + 1);
  RegWriteExpandStringValue(Root, EnvironmentKey, 'Path', Copy(Paths, 2, Length(Paths) - 2));
end;

// The tasks, each way: installing again with a task unchecked takes it back.
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    SetTypes(True);
    if WizardIsTaskSelected('path') then
      AddToPath(ExpandConstant('{app}'))
    else
      RemoveFromPath(ExpandConstant('{app}'));
    SetExplorerVerb(WizardIsTaskSelected('explorer'));
    if IsAdminInstallMode then
      if WizardIsTaskSelected('notepad') then
        ReplaceNotepad
      else
        RestoreNotepad;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  // Before the registry of the installation is taken back: the keys of MigPad are empty then.
  if CurUninstallStep = usUninstall then
  begin
    if IsAdminInstallMode then
      RestoreNotepad;
    SetExplorerVerb(False);
  end;
  if CurUninstallStep = usPostUninstall then
  begin
    SetTypes(False);
    RemoveFromPath(ExpandConstant('{app}'));
  end;
end;
