; SPDX-License-Identifier: GPL-3.0-or-later
; Copyright (C) 2026 The Photonation contributors
;
; Inno Setup 6 script for the Windows installer. Build it with
; packaging/windows/build-installer.ps1, which compiles the release binaries
; and passes the workspace version as AppVersion.
;
; Installs per user by default (no administrator rights needed); the first
; page offers an install for all users instead.

#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=x.y.z photonation.iss
#endif

#define AppName "Photonation"
#define AppExe "photonation-desktop.exe"
#define CliExe "photonation.exe"
#define Root "..\.."
#define Binaries Root + "\target\release"

[Setup]
; The AppId identifies the installation for upgrades and uninstall. Never change it.
AppId={{EC6B459F-FA84-4948-934F-373E79D99E23}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=The Photonation contributors
AppPublisherURL=https://github.com/DeBlo12/photonation
AppSupportURL=https://github.com/DeBlo12/photonation/issues
AppUpdatesURL=https://github.com/DeBlo12/photonation/releases
AppCopyright=Copyright (C) 2026 The Photonation contributors
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809 or later (wgpu needs DirectX 12 or Vulkan).
MinVersion=10.0.17763
LicenseFile={#Root}\LICENSE
SetupIconFile={#Root}\assets\icon\photonation.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
ChangesEnvironment=yes
CloseApplications=yes
OutputDir={#Root}\target\installer
OutputBaseFilename=photonation-{#AppVersion}-windows-x64-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "addtopath"; Description: "Add the photonation command-line tool to PATH"; GroupDescription: "Command line:"

[Files]
Source: "{#Binaries}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Binaries}\{#CliExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "{#Root}\NOTICE"; DestDir: "{app}"; DestName: "NOTICE.txt"
Source: "{#Root}\assets\fonts\OFL-Inter.txt"; DestDir: "{app}\licenses"
Source: "{#Root}\assets\fonts\OFL-Sora.txt"; DestDir: "{app}\licenses"
Source: "{#Root}\README.md"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// PATH is the user's for a per-user install and the machine's for an
// all-users install. Uninstall removes the entry if it is there.

function EnvironmentRoot: Integer;
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

// The 1-based position of Directory in Paths, or 0.
function PathEntryPosition(Paths, Directory: String): Integer;
begin
  Result := Pos(';' + Uppercase(Directory) + ';', ';' + Uppercase(Paths) + ';');
end;

procedure AddToPath(Directory: String);
var
  Paths: String;
begin
  if not RegQueryStringValue(EnvironmentRoot, EnvironmentKey, 'Path', Paths) then
    Paths := '';
  if PathEntryPosition(Paths, Directory) > 0 then
    exit;
  if (Paths <> '') and (Paths[Length(Paths)] <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(EnvironmentRoot, EnvironmentKey, 'Path', Paths + Directory);
end;

procedure RemoveFromPath(Directory: String);
var
  Paths: String;
  Position: Integer;
begin
  if not RegQueryStringValue(EnvironmentRoot, EnvironmentKey, 'Path', Paths) then
    exit;
  Position := PathEntryPosition(Paths, Directory);
  if Position = 0 then
    exit;
  // Drop the entry with the separator before it, or after it when it is first.
  if Position > 1 then
    Delete(Paths, Position - 1, Length(Directory) + 1)
  else
    Delete(Paths, 1, Length(Directory) + 1);
  RegWriteExpandStringValue(EnvironmentRoot, EnvironmentKey, 'Path', Paths);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
