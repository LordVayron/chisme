; Chisme installer (Inno Setup 6). Build with scripts\package.ps1, which
; passes Bin (the folder with the executables) and AppVersion.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef Bin
  #error Pass /DBin=<folder with chisme.exe and chisme-app.exe>
#endif

[Setup]
AppId={{6B3C2E71-9F4A-4C5D-8E1B-2A7D5C9F0E13}
AppName=Chisme
AppVersion={#AppVersion}
AppVerName=Chisme {#AppVersion}
AppPublisher=079 AI Labs
AppPublisherURL=https://github.com/LordVayron/chisme
AppSupportURL=https://github.com/LordVayron/chisme/issues
AppCopyright=Copyright (c) 2026 Jorge (LordVayron) and 079 AI Labs
AppComments=Graba y transmite tu pantalla, cámara y audio en Windows.
DefaultDirName={autopf}\Chisme
DefaultGroupName=Chisme
DisableProgramGroupPage=yes
; Per-user by default (no admin prompt); "all users" is offered.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=Chisme-Setup-{#AppVersion}
SetupIconFile={#Bin}\chisme.ico
UninstallDisplayIcon={app}\chisme-app.exe
UninstallDisplayName=Chisme
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 2004 or later (window capture without the yellow border, WDA_EXCLUDEFROMCAPTURE).
MinVersion=10.0.19041
ChangesEnvironment=yes
CloseApplications=yes

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
spanish.AddToPath=Agregar el comando "chisme" a la terminal (PATH)
english.AddToPath=Add the "chisme" command to the terminal (PATH)
spanish.Terminal=Terminal:
english.Terminal=Terminal:

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "addtopath"; Description: "{cm:AddToPath}"; GroupDescription: "{cm:Terminal}"; Flags: unchecked

[Files]
Source: "{#Bin}\chisme-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\chisme.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\NOTICE.md"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\assets\fonts\*-OFL.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\Chisme"; Filename: "{app}\chisme-app.exe"; Comment: "Graba y transmite tu pantalla"
Name: "{autodesktop}\Chisme"; Filename: "{app}\chisme-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\chisme-app.exe"; Description: "{cm:LaunchProgram,Chisme}"; Flags: nowait postinstall skipifsilent
; The in-app update installs silently with /relaunch=1 and reopens Chisme.
Filename: "{app}\chisme-app.exe"; Flags: nowait runasoriginaluser; Check: RelaunchAfterUpdate

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM chisme-app.exe /F"; Flags: runhidden; RunOnceId: "StopChisme"

[Code]
function RelaunchAfterUpdate: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:relaunch|0}') = '1');
end;

function NeedsAddPath(Dir: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    Path := '';
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: string;
  P: Integer;
begin
  // Take Chisme back out of the user's PATH.
  if (CurUninstallStep = usPostUninstall) and RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
  begin
    Dir := ExpandConstant('{app}');
    P := Pos(';' + Uppercase(Dir), Uppercase(Path));
    if P > 0 then
    begin
      Delete(Path, P, Length(Dir) + 1);
      RegWriteExpandStringValue(HKCU, 'Environment', 'Path', Path);
    end;
  end;
end;
