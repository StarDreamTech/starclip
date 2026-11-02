param(
  [Parameter(Mandatory)][string]$BaselineInstaller,
  [Parameter(Mandatory)][string]$FixtureDirectory,
  [Parameter(Mandatory)][string]$RepositoryRoot,
  [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{40}$')][string]$SourceHeadSha,
  [Parameter(Mandatory)][ValidatePattern('^[1-9][0-9]*$')][string]$ReleaseRunId
)

# Genuine public 0.2.2 -> exact signed draft target on a disposable Windows VM.
# The private HTTPS fixture retains production URLs and signature verification.
# Never run on a user PC; neither package nor product configuration is modified.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -ne 'cpython666/starclip') { throw 'A fresh GitHub-hosted Windows runner is required.' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'The disposable runner must already be elevated; this script never requests UAC elevation.'
}
$RepositoryRoot = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$FixtureDirectory = (Resolve-Path -LiteralPath $FixtureDirectory).Path
$BaselineInstaller = (Resolve-Path -LiteralPath $BaselineInstaller).Path
if (!$FixtureDirectory.StartsWith([IO.Path]::GetFullPath($env:RUNNER_TEMP) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'The HTTPS fixture must belong to this disposable runner.'
}
$helper = Join-Path $RepositoryRoot 'scripts/release-updater-fixture.py'
$fixture = Get-Content -LiteralPath (Join-Path $FixtureDirectory 'fixture.json') -Raw | ConvertFrom-Json
if ($fixture.platform -ne 'windows-x86_64' -or $fixture.sourceHeadSha -ne $SourceHeadSha -or
    [string]$fixture.releaseRunId -ne $ReleaseRunId -or $fixture.sourceRef -ne "v$($fixture.targetVersion)" -or
    [version]$fixture.targetVersion -le [version]'0.2.2') { throw 'The signed fixture provenance or version does not match this run.' }
$targetVersion = [string]$fixture.targetVersion
$baselineSchema = 2 # Immutable genuine public 0.2.2.
$targetSchema = [int]$fixture.targetSchema
if ($targetSchema -lt 1) { throw 'The signed target schema is missing or invalid.' }
$endpoint = 'https://github.com/cpython666/starclip/releases/latest/download/latest.json'
$targetUrl = "https://github.com/cpython666/starclip/releases/download/v$targetVersion/Starclip_${targetVersion}_x64-setup.exe"
if ($fixture.targetUrl -ne $targetUrl -or $fixture.targetFilename -ne "Starclip_${targetVersion}_x64-setup.exe") {
  throw 'Unexpected production update package URL or filename.'
}
$baselineInstallerHash = '5974a9d4584045c571f51e08657cfd228fd38bee87a9a7169392d924f1309bdd'
if ((Get-Item -LiteralPath $BaselineInstaller).Length -ne 5139609 -or
    (Get-FileHash -LiteralPath $BaselineInstaller -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baselineInstallerHash -or
    [Diagnostics.FileVersionInfo]::GetVersionInfo($BaselineInstaller).ProductVersion -ne '0.2.2') {
  throw 'The baseline must be the exact genuine public 0.2.2 installer.'
}
$dataDir = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'com.starclip.desktop'
$webviewDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'com.starclip.desktop'
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
$installDir = Join-Path $env:RUNNER_TEMP "Starclip Updater Acceptance $env:GITHUB_RUN_ID $env:GITHUB_RUN_ATTEMPT"
$evidence = Join-Path $env:RUNNER_TEMP 'starclip-windows-updater-evidence'
if (Test-Path -LiteralPath $installDir) { throw 'The acceptance installation already exists.' }
$null = New-Item -ItemType Directory -Path $installDir, $evidence
$exe = Join-Path $installDir 'starclip.exe'
$database = Join-Path $dataDir 'starclip.sqlite3'
$script:appProcess = $null
$server = $null
$trustedCertificate = $null
$hostsOriginal = $null
$hostsPath = Join-Path $env:SystemRoot 'System32/drivers/etc/hosts'
$proxyVariables = @{}
$stage = 'preflight'
function Set-AcceptanceStage([string]$Name) {
  $script:stage = $Name
  $progress = @{ stage=$Name; at=[DateTime]::UtcNow.ToString('o') }
  Write-Host (ConvertTo-Json $progress -Compress)
  $progress | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'progress.json') -Encoding utf8
}
Set-AcceptanceStage 'preflight'
$baselineHash = ''
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

function Stop-OwnedApplication {
  if ($null -ne $script:appProcess) {
    $script:appProcess.Refresh()
    if (!$script:appProcess.HasExited) {
      & taskkill.exe /PID $script:appProcess.Id /T /F | Out-Null
      if (!$script:appProcess.WaitForExit(10000)) { throw 'The owned application did not stop.' }
    }
    $script:appProcess = $null
  }
}

function Set-FixedClipboard { Set-Clipboard -Value 'Starclip CI original clipboard canary: an untouched orbit.' }

function Invoke-CertificateTool([string[]]$Arguments, [string]$DiagnosticName) {
  $process = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32/certutil.exe') -ArgumentList $Arguments -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $FixtureDirectory "$DiagnosticName.stdout") -RedirectStandardError (Join-Path $FixtureDirectory "$DiagnosticName.stderr")
  if (!$process.WaitForExit(30000)) {
    Stop-Process -Id $process.Id -Force
    throw "The temporary certificate operation timed out: $DiagnosticName"
  }
  if ($process.ExitCode -ne 0) { throw "The temporary certificate operation failed: $DiagnosticName ($($process.ExitCode))" }
}

function Wait-ForPausedWindow {
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  do {
    $script:appProcess.Refresh()
    if ($script:appProcess.HasExited) { throw 'The installed application exited before its paused UI appeared.' }
    if ($script:appProcess.MainWindowHandle -ne [IntPtr]::Zero) {
      try {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($script:appProcess.MainWindowHandle)
        $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        foreach ($node in $nodes) {
          if (!$node.Current.IsOffscreen -and $node.Current.IsEnabled -and $node.Current.Name -match '^(?:跳过教程|Skip tour)$') {
            $pattern = $null
            if ($node.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) { $pattern.Invoke() }
          }
          if (!$node.Current.IsOffscreen -and $node.Current.Name -match '^(?:记录已暂停|Recording paused)(?:\s|/|$)' -and
              (Test-Path -LiteralPath $database -PathType Leaf)) { return }
        }
      } catch [System.Windows.Automation.ElementNotAvailableException] { }
    }
    Start-Sleep -Milliseconds 500
  } while ([DateTime]::UtcNow -lt $deadline)
  throw 'The rendered pause status did not appear within one minute.'
}

function Start-And-AssertPaused {
  Set-FixedClipboard
  $script:appProcess = Start-Process -FilePath $exe -PassThru
  Wait-ForPausedWindow
}

function Get-MainRoot {
  $script:appProcess.Refresh()
  if ($script:appProcess.HasExited) { throw 'Application exited before installation confirmation.' }
  return [System.Windows.Automation.AutomationElement]::FromHandle($script:appProcess.MainWindowHandle)
}

function Find-Ui([string]$NamePattern, [bool]$Action = $true, $Scope = $null) {
  if ($null -eq $Scope) { $Scope = Get-MainRoot }
  $nodes = $Scope.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  $candidates = @($nodes | Where-Object {
    $_.Current.Name -match $NamePattern -and (!$Action -or ($_.Current.IsEnabled -and
      $_.Current.ControlType -in @([System.Windows.Automation.ControlType]::Button, [System.Windows.Automation.ControlType]::TabItem)))
  })
  $visible = @($candidates | Where-Object { !$_.Current.IsOffscreen })
  if ($visible.Count -gt 1 -and $Action) { throw "Ambiguous visible UI action: $NamePattern" }
  if ($visible.Count) { return $visible[0] }
  if ($Action -and $candidates.Count -eq 1) { return $candidates[0] }
  return $null
}

function Wait-Ui([string]$NamePattern, [bool]$Action = $true, [int]$Seconds = 60) {
  $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
  do {
    try { $node = Find-Ui $NamePattern $Action; if ($null -ne $node) { return $node } }
    catch [System.Windows.Automation.ElementNotAvailableException] { }
    Start-Sleep -Milliseconds 500
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Expected UI was not ready: $NamePattern"
}

function Invoke-Ui($Node) {
  if ($Node.Current.IsOffscreen) {
    $scroll = $null
    if ($Node.TryGetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern, [ref]$scroll)) { $scroll.ScrollIntoView() }
    $Node.SetFocus()
  }
  $pattern = $null
  if ($Node.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) { $pattern.Invoke(); return }
  if ($Node.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) { $pattern.Select(); return }
  throw 'The visible control exposes no native accessibility action.'
}

function Assert-PinnedManifest {
  $path = Join-Path $evidence 'latest.json'
  # Normal Windows certificate/hostname validation remains enabled.
  Invoke-WebRequest -Uri $endpoint -OutFile $path -TimeoutSec 30
  if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $fixture.latestManifestSha256) {
    throw 'The HTTPS response differs from the exact signed-release manifest.'
  }
  $manifest = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
  if ($manifest.version -ne $targetVersion -or $manifest.platforms.'windows-x86_64'.url -ne $targetUrl -or
      !$manifest.platforms.'windows-x86_64'.signature) { throw 'Unexpected target version, URL or missing signature.' }
}

function Check-UpdaterFixture([string]$Mode) {
  if ($Mode -eq 'seed') {
    & python $helper seed-data --database $database --expected-schema $baselineSchema
    if ($LASTEXITCODE -ne 0) { throw 'Fixed data seed failed.' }
  }
  $expectedSchema = if ($Mode -eq 'after-install') { $targetSchema } else { $baselineSchema }
  & python $helper verify-data --database $database --expected-schema $expectedSchema
  if ($LASTEXITCODE -ne 0) { throw 'Fixed data verification failed.' }
  $points = @(Get-ChildItem -LiteralPath (Join-Path $dataDir 'recovery') -Filter snapshot.json -Recurse -File -ErrorAction SilentlyContinue | Where-Object {
    (Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json).reason -eq 'before-update'
  })
  if ($Mode -eq 'after-install') {
    if ($points.Count -ne 1) { throw 'Expected exactly one before-update recovery point.' }
    $snapshot = Get-Content -LiteralPath $points[0].FullName -Raw | ConvertFrom-Json
    if ($snapshot.schemaVersion -ne $baselineSchema) { throw 'Unexpected recovery schema.' }
    foreach ($entry in $snapshot.files.PSObject.Properties) {
      $saved = Join-Path $points[0].DirectoryName $entry.Name
      if ((Get-FileHash -LiteralPath $saved -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.Value.sha256) { throw 'Recovery file digest mismatch.' }
    }
    & python $helper verify-data --database (Join-Path $points[0].DirectoryName 'starclip.sqlite3') --expected-schema $baselineSchema
    if ($LASTEXITCODE -ne 0) { throw 'The before-update snapshot lost fixed data.' }
    $oldApp = Join-Path $dataDir "application-backups/$($snapshot.id)/starclip.exe"
    if ((Get-FileHash -LiteralPath $oldApp -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baselineHash) { throw 'The old application backup differs from genuine 0.2.2.' }
  } elseif ($points.Count) { throw 'A before-update recovery point was created before explicit confirmation.' }
  @{ stage=$Mode; paused=$true; recoveryPoints=$points.Count; fixedDataVerified=$true } | ConvertTo-Json |
    Set-Content -LiteralPath (Join-Path $evidence "$Mode.json") -Encoding utf8
}

try {
  Set-AcceptanceStage 'inspect-signed-target'
  $targetInstaller = Join-Path $fixture.assetDirectory $fixture.targetFilename
  if ((Get-FileHash -LiteralPath $targetInstaller -Algorithm SHA256).Hash.ToLowerInvariant() -ne $fixture.targetSha256 -or
      [Diagnostics.FileVersionInfo]::GetVersionInfo($targetInstaller).ProductVersion -ne $targetVersion) { throw 'The signed target package changed.' }
  $unpacked = Join-Path $FixtureDirectory 'target-unpacked'
  & 7z x $targetInstaller "-o$unpacked" -y | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the signed NSIS payload.' }
  $targetExecutables = @(Get-ChildItem -LiteralPath $unpacked -Filter starclip.exe -File -Recurse)
  if ($targetExecutables.Count -ne 1) { throw 'The signed installer does not contain exactly one main executable.' }
  $expectedInstalledHash = (Get-FileHash -LiteralPath $targetExecutables[0].FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  Set-AcceptanceStage 'install-genuine-baseline'
  Set-FixedClipboard
  $installation = Start-Process -FilePath $BaselineInstaller -ArgumentList "/S /NS /D=$installDir" -PassThru
  if (!$installation.WaitForExit(180000) -or $installation.ExitCode -ne 0) { throw 'The genuine baseline installation failed.' }
  if ([Diagnostics.FileVersionInfo]::GetVersionInfo($exe).ProductVersion -ne '0.2.2') { throw 'The installed baseline is not genuine 0.2.2.' }
  $baselineHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
  Start-And-AssertPaused
  Set-FixedClipboard
  & python (Join-Path $RepositoryRoot 'scripts/installer-fixture.py') $database fresh $baselineSchema
  if ($LASTEXITCODE -ne 0) { throw 'The genuine baseline captured the fixed clipboard canary.' }
  Stop-OwnedApplication
  Check-UpdaterFixture 'seed'

  Set-AcceptanceStage 'isolate-trusted-https'
  # All tools and artifacts have already been downloaded. No credential is
  # passed to the fixture. Trust is scoped to a fresh runner and removed below.
  # Avoid unbounded CIM queries in the test harness. Claim/release only the
  # loopback port the fixture will use; no external connection is made.
  $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 443)
  try { $probe.Start() } finally { $probe.Stop() }
  $caPath = Join-Path $FixtureDirectory 'ca.crt'
  $ca = [Security.Cryptography.X509Certificates.X509Certificate2]::new($caPath)
  if (Test-Path -LiteralPath "Cert:\LocalMachine\Root\$($ca.Thumbprint)") { throw 'The temporary CA is unexpectedly already trusted.' }
  Set-AcceptanceStage 'trust-temporary-ca'
  # The current-user protected Root store can still show a confirmation UI even
  # with certutil -f. Use the already-elevated disposable VM's machine store;
  # no UAC prompt or user trust settings are touched, and delete this exact CA.
  $trustedCertificate = $ca
  Invoke-CertificateTool @('-f', '-addstore', 'Root', ('"' + $caPath + '"')) 'ca-import'
  if (!(Test-Path -LiteralPath "Cert:\LocalMachine\Root\$($ca.Thumbprint)")) { throw 'The temporary CA did not enter the disposable machine root store.' }
  Set-AcceptanceStage 'map-update-host'
  $hostsOriginal = [IO.File]::ReadAllBytes($hostsPath)
  if ([Text.Encoding]::UTF8.GetString($hostsOriginal) -match '(?im)^\s*[^#\r\n]+\s+github\.com(?:\s|$)') { throw 'An existing github.com hosts mapping would make isolation ambiguous.' }
  [IO.File]::AppendAllText($hostsPath, "`r`n127.0.0.1 github.com # Starclip disposable updater acceptance`r`n", [Text.UTF8Encoding]::new($false))
  Clear-DnsClientCache
  foreach ($name in @('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','NO_PROXY')) {
    $proxyVariables[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
  }
  $serverArgs = @(('"' + $helper + '"'), 'serve', '--directory', ('"' + $FixtureDirectory + '"'))
  Set-AcceptanceStage 'launch-https-server'
  $server = Start-Process -FilePath (Get-Command python).Source -ArgumentList $serverArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $FixtureDirectory 'server.stdout') -RedirectStandardError (Join-Path $FixtureDirectory 'server.stderr')
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  do {
    if ($server.HasExited) { throw 'The private HTTPS fixture server exited.' }
    $connected = $false
    $client = [Net.Sockets.TcpClient]::new()
    try { $connected = $client.ConnectAsync('127.0.0.1', 443).Wait(500) -and $client.Connected }
    catch [AggregateException] { }
    finally { $client.Dispose() }
    if ($connected) { break }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  Set-AcceptanceStage 'probe-trusted-https'
  Assert-PinnedManifest
  Copy-Item -LiteralPath (Join-Path $FixtureDirectory 'fixture.json') -Destination (Join-Path $evidence 'fixture.json')

  Set-AcceptanceStage 'check'
  Start-And-AssertPaused
  $originalPid = $script:appProcess.Id
  Invoke-Ui (Wait-Ui '^(?:偏好设置|Preferences)(?:\s|$)')
  Invoke-Ui (Wait-Ui '^(?:帮助|Help)$')
  Invoke-Ui (Wait-Ui '^(?:检查更新|Check for updates)$')
  $null = Wait-Ui ('^(?:新版本|Version)\s*' + [regex]::Escape($targetVersion) + '(?:\s|$)') $false
  Assert-PinnedManifest
  Set-AcceptanceStage 'download-and-verify'
  Invoke-Ui (Wait-Ui '^(?:下载更新|Download update)$')
  $later = Wait-Ui '^(?:稍后再说|Later)$' $true 180
  # The real updater reaches this dialog only after official-key/signature and
  # signed-version checks. No IPC, modified package or injected download data.
  Set-AcceptanceStage 'later-keeps-current-app'
  Invoke-Ui $later
  Start-Sleep -Seconds 3
  if (Find-Ui '^(?:稍后再说|Later)$') { throw 'Later did not dismiss the dialog.' }
  if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne $baselineHash -or
      $script:appProcess.Id -ne $originalPid -or $script:appProcess.HasExited) { throw 'Later changed or restarted the baseline.' }
  Check-UpdaterFixture 'before-install'
  Set-AcceptanceStage 'confirm-install'
  Assert-PinnedManifest
  Invoke-Ui (Wait-Ui '^(?:安装并重启|Install and restart)$')
  $later = Wait-Ui '^(?:稍后再说|Later)$'
  $row = [System.Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($later)
  $confirm = Find-Ui '^(?:安装并重启|Install and restart)$' $true $row
  if ($null -eq $confirm) { throw 'The explicit confirmation action could not be identified.' }
  Invoke-Ui $confirm
  Set-AcceptanceStage 'automatic-restart'
  $deadline = [DateTime]::UtcNow.AddSeconds(180)
  do {
    $script:appProcess.Refresh()
    $restarted = @(Get-Process -Name starclip -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $originalPid -and $_.Path -eq $exe })
    if ($script:appProcess.HasExited -and $restarted.Count -eq 1) { break }
    Start-Sleep -Milliseconds 500
  } while ([DateTime]::UtcNow -lt $deadline)
  if (!$script:appProcess.HasExited -or $restarted.Count -ne 1) { throw 'No unique automatic restart at the original path was observed.' }
  $script:appProcess = $restarted[0]
  $installedHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($installedHash -ne $expectedInstalledHash -or $installedHash -eq $baselineHash -or
      [Diagnostics.FileVersionInfo]::GetVersionInfo($exe).ProductVersion -ne $targetVersion) { throw 'The restarted executable differs from the exact signed target.' }
  $entries = @($uninstallKeys | Where-Object { Test-Path -LiteralPath $_ } | ForEach-Object { Get-ItemProperty -LiteralPath $_ })
  if ($entries.Count -ne 1 -or $entries[0].DisplayVersion -ne $targetVersion) { throw 'Updated application registration has an unexpected version.' }
  Set-AcceptanceStage 'restarted-paused-with-fixed-data'
  Wait-ForPausedWindow
  Set-FixedClipboard
  for ($i = 0; $i -lt 3; $i++) { Start-Sleep -Seconds 1; Check-UpdaterFixture 'after-install' }
  Stop-OwnedApplication
  Check-UpdaterFixture 'after-install'
  @{ result='passed'; sourceHeadSha=$SourceHeadSha; releaseRunId=$ReleaseRunId; baselineVersion='0.2.2'; targetVersion=$targetVersion;
    baselineInstallerSha256=$baselineInstallerHash; targetInstallerSha256=$fixture.targetSha256; manifestSha256=$fixture.latestManifestSha256;
    originalPid=$originalPid; originalExeSha256=$baselineHash; installedExeSha256=$installedHash; restartMode='automatic';
    transport='runner-only trusted HTTPS fixture retaining production URLs and signed bytes'; publicDownloadVerified=$false } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
  '- Windows genuine public 0.2.2 -> exact signed target passed: native check, verified download, Later, explicit confirmation, automatic restart, paused fixed data and recovery copies.' >> $env:GITHUB_STEP_SUMMARY
  '- Transport used a disposable runner HTTPS fixture with a temporary trusted CA and unchanged production URLs/packages. This is pre-publication package acceptance, not a public GitHub/CDN download test.' >> $env:GITHUB_STEP_SUMMARY
} catch {
  @{ result='failed'; stage=$stage; message=$_.Exception.Message } | ConvertTo-Json |
    Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding utf8
  try {
    (Get-MainRoot).FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
      ForEach-Object { '{0} | enabled={1} | offscreen={2} | {3}' -f $_.Current.ControlType.ProgrammaticName, $_.Current.IsEnabled, $_.Current.IsOffscreen, $_.Current.Name } |
      Set-Content -LiteralPath (Join-Path $evidence 'owned-window-uia.txt') -Encoding utf8
  } catch { Write-Host 'The owned window was unavailable for UIA failure evidence.' }
  throw
} finally {
  # Remove interception first so Actions uploads use real GitHub. Private CA
  # keys, auth tokens, profile/database and application backups are never uploaded.
  $cleanupErrors = @()
  foreach ($name in @('server.stdout', 'server.stderr', 'ca-import.stdout', 'ca-import.stderr')) {
    $diagnostic = Join-Path $FixtureDirectory $name
    if (Test-Path -LiteralPath $diagnostic -PathType Leaf) {
      Get-Content -LiteralPath $diagnostic -Tail 100 -ErrorAction SilentlyContinue |
        Set-Content -LiteralPath (Join-Path $evidence $name) -Encoding utf8
    }
  }
  try {
    if ($null -ne $hostsOriginal) {
      [IO.File]::WriteAllBytes($hostsPath, $hostsOriginal)
      Clear-DnsClientCache
      if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($hostsPath)) -ne [Convert]::ToBase64String($hostsOriginal)) { throw 'Hosts restoration mismatch.' }
    }
  } catch { $cleanupErrors += 'hosts restoration failed' }
  try {
    if ($null -ne $trustedCertificate) {
      $certificatePath = "Cert:\LocalMachine\Root\$($trustedCertificate.Thumbprint)"
      if (Test-Path -LiteralPath $certificatePath) {
        Invoke-CertificateTool @('-delstore', 'Root', $trustedCertificate.Thumbprint) 'ca-remove'
      }
      if (Test-Path -LiteralPath $certificatePath) { throw 'Temporary CA removal failed.' }
    }
  } catch { $cleanupErrors += 'temporary CA removal failed' }
  foreach ($name in $proxyVariables.Keys) { [Environment]::SetEnvironmentVariable($name, $proxyVariables[$name], 'Process') }
  try { if ($null -ne $server -and !$server.HasExited) { Stop-Process -Id $server.Id -Force } }
  catch { $cleanupErrors += 'fixture server stop failed' }
  try { Stop-OwnedApplication } catch { $cleanupErrors += 'owned app stop failed' }
  @{ restored=$cleanupErrors.Count -eq 0; errors=$cleanupErrors } | ConvertTo-Json |
    Set-Content -LiteralPath (Join-Path $evidence 'cleanup.json') -Encoding utf8
  if ($cleanupErrors.Count) { throw ($cleanupErrors -join '; ') }
}
