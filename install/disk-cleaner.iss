; disk-cleaner-rs 安装脚本（Inno Setup 6）
; 版本号与产物名后缀由构建时经 ISCC /D 传入：
;   /DMyAppVersion=1.2.3   /DSetupSuffix=v1.2.3
; 未传入时使用下方兜底值（仅演练产物可能出现）。

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef SetupSuffix
  #define SetupSuffix "rehearsal"
#endif

#define MyAppName "disk-cleaner-rs"
#define MyAppExeName "disk-app.exe"

[Setup]
AppId={{B74A9D3C-2E6F-4F8A-9C1D-5B7E0A2F4D68}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=DeepLearning666888
DefaultDirName={localappdata}\Programs\disk-cleaner-rs
DefaultGroupName=disk-cleaner-rs
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputDir=.
OutputBaseFilename=disk-cleaner-rs-setup-{#SetupSuffix}-x86_64
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\disk-app.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\disk-cleaner-rs"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\disk-cleaner-rs"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,disk-cleaner-rs}"; Flags: nowait postinstall skipifsilent