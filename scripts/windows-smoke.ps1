param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
  [Parameter(Mandatory)][ValidateSet('fresh', 'upgrade')][string]$Scenario,
  [string]$BaselineInstaller
)

# Only disposable GitHub-hosted runners. Windows Known Folders do not follow an
# APPDATA environment override, so refuse any pre-existing app state instead.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Windows installation smoke tests require a fresh GitHub-hosted runner.'
}
if (!$env:RUNNER_TEMP -or !(Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)) {
  throw 'Runner temporary directory is unavailable.'
}
$dataDir = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'com.starclip.desktop'
$webviewDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'com.starclip.desktop'
$startupId = 'com.starclip.desktop'
$runKeyPath = 'Software\Microsoft\Windows\CurrentVersion\Run'
$approvedKeyPath = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
function Read-StartupValue([string]$Path, [Microsoft.Win32.RegistryKey]$Hive = [Microsoft.Win32.Registry]::CurrentUser) {
  $key = $Hive.OpenSubKey($Path)
  if ($null -eq $key) { return $null }
  try {
    $value = $key.GetValue($startupId, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($null -ne $value) { return ,$value } # Preserve binary values as one byte array.
    return $null
  }
  finally { $key.Dispose() }
}
$uninstallKeys = @(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Starclip',
  'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Starclip',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Starclip',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Starclip'
)
foreach ($path in @($dataDir, $webviewDir) + $uninstallKeys) {
  if (Test-Path -LiteralPath $path) { throw 'Existing Starclip data or installation found; refusing to run.' }
}
if (Get-Process -Name starclip -ErrorAction SilentlyContinue) { throw 'Starclip is already running.' }
if ($null -ne (Read-StartupValue $runKeyPath) -or $null -ne (Read-StartupValue $approvedKeyPath) -or
    $null -ne (Read-StartupValue $runKeyPath ([Microsoft.Win32.Registry]::LocalMachine))) {
  throw 'Existing Starclip login registration found; refusing to run.'
}
$Installer = (Resolve-Path -LiteralPath $Installer).Path
if ($Scenario -eq 'upgrade') {
  if (!$BaselineInstaller) { throw 'A verified 0.1.0 installer is required for a real upgrade test.' }
  $BaselineInstaller = (Resolve-Path -LiteralPath $BaselineInstaller).Path
  if ([version]$Version -le [version]'0.1.0') { throw 'Upgrade target must be newer than 0.1.0.' }
}
$installDir = Join-Path $env:RUNNER_TEMP "Starclip Startup Smoke $Scenario $env:GITHUB_RUN_ID $env:GITHUB_RUN_ATTEMPT"
if (Test-Path -LiteralPath $installDir) { throw 'Smoke installation directory already exists.' }
$null = New-Item -ItemType Directory -Path $installDir
$exe = Join-Path $installDir 'starclip.exe'
$database = Join-Path $dataDir 'starclip.sqlite3'
$script:appProcess = $null
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class StarclipSmokeWindow {
  [DllImport("user32.dll")]
  [return: MarshalAs(UnmanagedType.Bool)]
  public static extern bool IsWindow(IntPtr handle);
  private delegate bool EnumWindowCallback(IntPtr handle, IntPtr data);
  [DllImport("user32.dll")]
  private static extern bool EnumWindows(EnumWindowCallback callback, IntPtr data);
  [DllImport("user32.dll")]
  private static extern uint GetWindowThreadProcessId(IntPtr handle, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  private static extern int GetWindowText(IntPtr handle, StringBuilder text, int length);
  public static bool HasMainWindow(uint pid) {
    bool found = false;
    EnumWindows((handle, data) => {
      uint owner; GetWindowThreadProcessId(handle, out owner);
      if (owner != pid) return true;
      var title = new StringBuilder(256); GetWindowText(handle, title, title.Capacity);
      // Enumerate hidden HWNDs too; MainWindowHandle alone only sees visible UI.
      found = title.ToString() == "Starclip" || title.ToString() == "星笺 Starclip";
      return !found;
    }, IntPtr.Zero);
    return found;
  }
}
'@

# This script never reads the clipboard. A fixed canary replaces runner contents
# before any application launch, making a pause regression safe and detectable.
function Set-FixedClipboard([bool]$AfterStartup = $false) {
  $canary = if ($AfterStartup) { 'Starclip CI original clipboard canary: a second orbit.' } else { 'Starclip CI original clipboard canary: a quiet orbit.' }
  Set-Clipboard -Value $canary
}

function Install-Package([string]$Path, [string]$ExpectedVersion, [bool]$Update) {
  if ([Diagnostics.FileVersionInfo]::GetVersionInfo($Path).ProductVersion -ne $ExpectedVersion) {
    throw 'Installer version does not match the expected version.'
  }
  # NSIS /D must be last and unquoted, including when the directory has spaces.
  $arguments = if ($Update) { "/S /UPDATE /NS /D=$installDir" } else { "/S /NS /D=$installDir" }
  $process = Start-Process -FilePath $Path -ArgumentList $arguments -PassThru
  if (!$process.WaitForExit(180000)) { throw 'Installer did not finish within three minutes.' }
  if ($process.ExitCode -ne 0) { throw "Installer failed with exit code $($process.ExitCode)." }
  if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Installed executable is missing.' }
  if ([Diagnostics.FileVersionInfo]::GetVersionInfo($exe).ProductVersion -ne $ExpectedVersion) {
    throw 'Installed executable version does not match the installer.'
  }
  $entries = @($uninstallKeys | Where-Object { Test-Path -LiteralPath $_ } | ForEach-Object { Get-ItemProperty -LiteralPath $_ })
  if ($entries.Count -ne 1 -or $entries[0].DisplayVersion -ne $ExpectedVersion) {
    throw 'Installed application registration has an unexpected version or duplicate entries.'
  }
  Write-Host "NSIS installation verified: $ExpectedVersion"
}

function Start-And-AssertPaused {
  Set-FixedClipboard
  $script:appProcess = Start-Process -FilePath $exe -PassThru
  Wait-ForPausedWindow
}

function Wait-ForPausedWindow {
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  do {
    $script:appProcess.Refresh()
    if ($script:appProcess.HasExited) { throw 'Installed application exited during startup.' }
    $handle = $script:appProcess.MainWindowHandle
    if ($handle -ne [IntPtr]::Zero) {
      try {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
        # The historical baseline is Chinese; the candidate follows the runner's
        # system language. Match the shipped labels without changing preferences.
        $skip = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
          [System.Windows.Automation.OrCondition]::new(
            [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, '跳过教程'),
            [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, 'Skip tour')))
        if ($skip -and !$skip.Current.IsOffscreen -and $skip.Current.IsEnabled) {
          $pattern = $null
          if ($skip.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
            $pattern.Invoke() # Never click the capture toggle.
          }
        }
        $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($node in $nodes) {
          if (!$node.Current.IsOffscreen -and $node.Current.Name -match '^(?:记录已暂停|Recording paused)(?:\s|/|$)') {
            if (Test-Path -LiteralPath $database -PathType Leaf) {
              Write-Host 'Native window rendered and reports recording paused.'
              return
            }
          }
        }
      } catch [System.Windows.Automation.ElementNotAvailableException] {
        # The tutorial can remove its accessibility nodes while we inspect them.
      }
    }
    Start-Sleep -Milliseconds 500
  } while ([DateTime]::UtcNow -lt $deadline)
  throw 'Native window, rendered pause status or database was not ready within one minute.'
}

function Find-NativeControl([string]$Name, [bool]$Switch = $false) {
  $script:appProcess.Refresh()
  if ($script:appProcess.HasExited) { throw 'Candidate exited while inspecting its settings.' }
  if ($script:appProcess.MainWindowHandle -eq [IntPtr]::Zero) { return $null }
  try {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:appProcess.MainWindowHandle)
    $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($node in $nodes) {
      if (!$node.Current.IsEnabled -or $node.Current.Name -notmatch $Name) { continue }
      if ($node.Current.IsOffscreen) {
        $scroll = $null
        if ($node.TryGetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern, [ref]$scroll)) {
          $scroll.ScrollIntoView()
        }
        continue
      }
      $patterns = if ($Switch) { @([System.Windows.Automation.TogglePattern]::Pattern) } else {
        @([System.Windows.Automation.InvokePattern]::Pattern, [System.Windows.Automation.SelectionItemPattern]::Pattern)
      }
      foreach ($id in $patterns) {
        $pattern = $null
        if ($node.TryGetCurrentPattern($id, [ref]$pattern)) { return $pattern }
      }
    }
  } catch [System.Windows.Automation.ElementNotAvailableException] {
    # Re-read after the WebView replaces nodes during a successful save.
  }
  return $null
}

function Wait-NativeControl([string]$Name, [bool]$Switch = $false) {
  $deadline = [DateTime]::UtcNow.AddSeconds(25)
  do {
    $pattern = Find-NativeControl $Name $Switch
    if ($null -ne $pattern) { return $pattern }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Native control unavailable or still disabled: $Name"
}

function Invoke-NativeControl([string]$Name) {
  $pattern = Wait-NativeControl $Name
  if ($pattern -is [System.Windows.Automation.SelectionItemPattern]) { $pattern.Select() }
  else { $pattern.Invoke() }
}

function Assert-NativeSwitch([string]$Name, [bool]$Enabled) {
  $expected = if ($Enabled) { [System.Windows.Automation.ToggleState]::On } else { [System.Windows.Automation.ToggleState]::Off }
  $deadline = [DateTime]::UtcNow.AddSeconds(25)
  do {
    $pattern = Find-NativeControl $Name $true
    try {
      if ($null -ne $pattern -and $pattern.Current.ToggleState -eq $expected) { return }
    } catch [System.Windows.Automation.ElementNotAvailableException] {
      # A completed save can replace the control just before the state read.
    }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Native switch did not report the saved state ${Enabled}: $Name"
}

function Set-NativeSwitch([string]$Name, [bool]$Enabled) {
  $pattern = Wait-NativeControl $Name $true
  $expected = if ($Enabled) { [System.Windows.Automation.ToggleState]::On } else { [System.Windows.Automation.ToggleState]::Off }
  if ($pattern.Current.ToggleState -ne $expected) { $pattern.Toggle() }
  Assert-NativeSwitch $Name $Enabled
}

$loginSwitch = '^(?:登录系统后自动启动|Launch automatically at login)$'
$hiddenSwitch = '^(?:启动时收起主窗口|Start with the main window hidden)$'
$traySwitch = '^(?:显示菜单栏 / 托盘图标|Show menu bar / tray icon)$'
function Open-StartupSettings {
  Invoke-NativeControl '^(?:偏好设置|Preferences)(?:\s*v\d[^ ]*)?$'
  Invoke-NativeControl '^(?:使用习惯|Habits)$'
  $null = Wait-NativeControl $loginSwitch $true
}
function Close-StartupSettings {
  Invoke-NativeControl '^(?:关闭弹窗|Close dialog)$'
  Wait-ForPausedWindow
}
function Quit-FromSettings {
  Invoke-NativeControl '^(?:退出星笺|Quit Starclip)$'
  if (!$script:appProcess.WaitForExit(15000)) { throw 'Quit from settings did not end the candidate process.' }
  if ($script:appProcess.ExitCode -ne 0) { throw 'Candidate exited with an unexpected error.' }
  $script:appProcess = $null
}

function Check-StartupDatabase([bool]$Hidden, [bool]$Tray) {
  @'
import json, pathlib, sqlite3, sys
path = pathlib.Path(sys.argv[1])
with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
    row = db.execute("SELECT value FROM preferences WHERE key='window_startup_v1'").fetchone()
    saved = json.loads(row[0]) if row else dict(startHidden=False, showTrayIcon=True)
    assert saved == dict(startHidden=sys.argv[2] == 'True', showTrayIcon=sys.argv[3] == 'True'), 'Unexpected persisted window startup preferences'
    settings = db.execute("SELECT value FROM preferences WHERE key='settings'").fetchone()
    assert settings is None or json.loads(settings[0])['paused'] is True, 'Startup preferences changed recording state'
'@ | python - $database ($Hidden.ToString()) ($Tray.ToString())
  if ($LASTEXITCODE -ne 0) { throw 'Startup settings persistence check failed.' }
  Check-Database $Scenario $targetSchema
}

function Assert-LoginRegistered {
  if ((Read-StartupValue $runKeyPath) -ne ('"' + $exe + '"')) { throw 'Run registration did not quote the full executable path containing spaces.' }
  $approved = Read-StartupValue $approvedKeyPath
  if ($approved -isnot [byte[]] -or $approved.Length -ne 12 -or $approved[0] -notin @(2, 6) -or
      $approved[1] -ne 0 -or $approved[2] -ne 0 -or $approved[3] -ne 0) {
    throw 'StartupApproved does not report the explicitly enabled login registration.'
  }
  if ($null -ne (Read-StartupValue $runKeyPath ([Microsoft.Win32.Registry]::LocalMachine))) {
    throw 'Login startup unexpectedly wrote a machine-wide registration.'
  }
}

function Check-StartupPreferences {
  Open-StartupSettings
  Assert-NativeSwitch $loginSwitch $false
  Assert-NativeSwitch $hiddenSwitch $false
  Assert-NativeSwitch $traySwitch $true
  Check-StartupDatabase $false $true

  Set-NativeSwitch $loginSwitch $true
  Assert-LoginRegistered
  # Simulate the OS startup-app toggle in this disposable user's own value.
  $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($approvedKeyPath, $true)
  try { $key.SetValue($startupId, [byte[]]@(3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0), [Microsoft.Win32.RegistryValueKind]::Binary) }
  finally { $key.Dispose() }
  Close-StartupSettings
  Open-StartupSettings
  Assert-NativeSwitch $loginSwitch $false
  if ((Read-StartupValue $approvedKeyPath)[0] -ne 3) { throw 'Reading preferences silently re-enabled the externally disabled login item.' }
  Set-NativeSwitch $loginSwitch $true
  Assert-LoginRegistered
  Set-NativeSwitch $loginSwitch $false
  if ($null -ne (Read-StartupValue $runKeyPath)) { throw 'Disabling login startup left the Run entry registered.' }

  Set-NativeSwitch $traySwitch $false
  Set-NativeSwitch $hiddenSwitch $true
  Check-StartupDatabase $true $false
  Quit-FromSettings
  Set-FixedClipboard
  $script:appProcess = Start-Process -FilePath $exe -PassThru
  if (!$script:appProcess.WaitForInputIdle(30000)) { throw 'Hidden startup did not reach its native event loop.' }
  $deadline = [DateTime]::UtcNow.AddSeconds(7)
  do {
    $script:appProcess.Refresh()
    if ($script:appProcess.HasExited) { throw 'Candidate exited instead of starting in the background.' }
    if ([StarclipSmokeWindow]::HasMainWindow($script:appProcess.Id)) { throw 'Hidden startup created a main HWND.' }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $deadline)
  Check-StartupDatabase $true $false
  $second = Start-Process -FilePath $exe -PassThru
  if (!$second.WaitForExit(10000)) {
    & taskkill.exe /PID $second.Id /T /F | Out-Null
    throw 'Second launch did not hand off after hidden startup.'
  }
  if ($second.ExitCode -ne 0) { throw 'Second launch failed after hidden startup.' }
  Wait-ForPausedWindow
  Open-StartupSettings
  Assert-NativeSwitch $loginSwitch $false
  Assert-NativeSwitch $hiddenSwitch $true
  Assert-NativeSwitch $traySwitch $false
  Set-NativeSwitch $hiddenSwitch $false
  Set-NativeSwitch $traySwitch $true
  Check-StartupDatabase $false $true
  Quit-FromSettings
  Write-Host 'Native startup settings passed: current-user login enable/disable and external disable, hidden cold start, second-launch recovery, persisted tray choice and explicit quit.'
}

function Check-WindowRecreation {
  # Only the fixed runner-owned database. An enabled pet changes the expected
  # window lifecycle, so verify the fixture before requesting a normal close.
  @'
import json, pathlib, sqlite3, sys
path = pathlib.Path(sys.argv[1])
with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as db:
    saved = db.execute("SELECT value FROM preferences WHERE key='starpet_v1'").fetchone()
    assert saved is None or json.loads(saved[0])['enabled'] is False, 'Window fixture must have the pet disabled'
'@ | python - $database
  if ($LASTEXITCODE -ne 0) { throw 'Window lifecycle fixture is not isolated or has an enabled pet.' }
  $script:appProcess.Refresh()
  $previous = $script:appProcess.MainWindowHandle
  if ($previous -eq [IntPtr]::Zero -or !$script:appProcess.CloseMainWindow()) {
    throw 'Could not request a normal close of the candidate main window.'
  }
  $deadline = [DateTime]::UtcNow.AddSeconds(30)
  while ([StarclipSmokeWindow]::IsWindow($previous)) {
    $script:appProcess.Refresh()
    if ($script:appProcess.HasExited) { throw 'Closing the main window exited the application.' }
    if ([DateTime]::UtcNow -ge $deadline) { throw 'The main window was hidden but its native handle was not destroyed.' }
    Start-Sleep -Milliseconds 200
  }
  $script:appProcess.Refresh()
  if ($script:appProcess.HasExited) { throw 'The background application exited after releasing its window.' }
  # A second launch uses the shipped single-instance callback to rebuild/show
  # the original process's window. No JavaScript injection or product test hook.
  $second = Start-Process -FilePath $exe -PassThru
  if (!$second.WaitForExit(10000)) {
    & taskkill.exe /PID $second.Id /T /F | Out-Null
    throw 'The second instance did not hand control back to the background application.'
  }
  if ($second.ExitCode -ne 0) { throw 'The second instance failed to reopen the main window.' }
  Wait-ForPausedWindow
  Check-Database $Scenario $targetSchema
  Write-Host 'Native main window was destroyed and recreated; recording remains paused and fixed data is intact.'
}

function Stop-OwnedApplication {
  if ($null -ne $script:appProcess) {
    $script:appProcess.Refresh()
    if (!$script:appProcess.HasExited) {
      # A native close keeps the app in the tray. Stop only our process tree.
      & taskkill.exe /PID $script:appProcess.Id /T /F | Out-Null
      if (!$script:appProcess.WaitForExit(10000)) { throw 'Smoke application did not stop.' }
    }
    $script:appProcess = $null
  }
}

# SQLite access is limited to the new runner-owned app directory checked above.
# Python's sqlite3 is standard library; no driver, GUI framework or app hook.
$targetSchemaText = & python (Join-Path $PSScriptRoot 'fixture_schema.py') (Split-Path $PSScriptRoot -Parent)
if ($LASTEXITCODE -ne 0 -or $targetSchemaText -notmatch '^[1-9][0-9]*$') { throw 'Could not determine the exact candidate source schema.' }
$targetSchema = [int]$targetSchemaText

function Check-Database([string]$Mode, [int]$Schema) {
  if ($null -ne $script:appProcess) {
    $script:appProcess.Refresh()
    if ($script:appProcess.HasExited) { throw 'Installed application exited during observation.' }
  }
  if ($Mode -eq 'upgrade' -and (Test-Path (Join-Path $PSScriptRoot '../src-tauri/src/storage/recovery_cleanup.rs'))) { $Mode = 'upgrade-cleaned' }
  & python (Join-Path $PSScriptRoot 'installer-fixture.py') $database $Mode $Schema
  if ($LASTEXITCODE -ne 0) { throw "Fixed database verification failed: $Mode" }
}

try {
  Set-FixedClipboard
  if ($Scenario -eq 'upgrade') {
    Install-Package $BaselineInstaller '0.1.0' $false
    Start-And-AssertPaused
    Set-FixedClipboard $true
    for ($i = 0; $i -lt 3; $i++) { Start-Sleep -Seconds 1; Check-Database 'fresh' 0 }
    Stop-OwnedApplication
    Check-Database 'seed' 0
    Install-Package $Installer $Version $true
  } else {
    Install-Package $Installer $Version $false
  }
  Start-And-AssertPaused
  Set-FixedClipboard $true
  for ($i = 0; $i -lt 3; $i++) { Start-Sleep -Seconds 1; Check-Database $Scenario $targetSchema }
  Check-WindowRecreation
  Check-StartupPreferences
  Check-Database $Scenario $targetSchema
  if ($env:GITHUB_STEP_SUMMARY) {
    "- Windows native $Scenario smoke passed: NSIS $Version, rendered pause status, fixed database checks. No capture toggle was enabled." >> $env:GITHUB_STEP_SUMMARY
    '- Candidate main-window close destroyed the native HWND while the process stayed alive; a second launch recreated the window, restored paused UI and preserved fixed data.' >> $env:GITHUB_STEP_SUMMARY
    '- Startup preferences passed through native UI: quoted current-user Run registration, enable/disable, external StartupApproved disable readback, hidden cold start without a main HWND, second-launch recovery, persisted tray preference and explicit settings quit.' >> $env:GITHUB_STEP_SUMMARY
    if ($Scenario -eq 'upgrade') { '- Real 0.1.0 → candidate upgrade preserved the fixed record, group and preferences; automatic migration backups followed the candidate source retention policy.' >> $env:GITHUB_STEP_SUMMARY }
    '- This does not verify an actual logout/login cycle, Explorer tray rendering, the in-app update download/dialog flow, SmartScreen, or every Windows environment.' >> $env:GITHUB_STEP_SUMMARY
  }
} finally {
  try { Stop-OwnedApplication } finally {
    # The entry was proven absent before this test. Delete only our own values;
    # preserve registry keys and unrelated applications' startup settings.
    foreach ($path in @($runKeyPath, $approvedKeyPath)) {
      $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($path, $true)
      if ($null -ne $key) {
        try { $key.DeleteValue($startupId, $false) } finally { $key.Dispose() }
      }
    }
  }
  # Keep the synthetic state on this disposable VM for failure diagnosis. No
  # databases, clipboard data, screenshots or profile files are uploaded.
}
