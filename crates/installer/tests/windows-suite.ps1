<#
.SYNOPSIS
  Compile every app's actual WiX sources around fixture files and check the MSI UI tables.
.DESCRIPTION
  Windows only. Requires WiX 5.0.2 and its UI/Util extensions. Creates MSI fixtures in a
  temporary directory, without installing, launching or uninstalling anything. This checks
  linking, architecture-specific actions, sequence ordering and the final MSI tables; it does
  not replace interactive Windows acceptance testing with real app binaries.
.EXAMPLE
  pwsh crates/installer/tests/windows-suite.ps1 -Repositories C:\work\CraftApps
#>
param(
  [Parameter(Mandatory)] [string] $Repositories,
  [string] $StartAt,
  [string] $Only,
  [string] $ModelsDirectory,
  [ValidateSet('x64', 'x86', 'arm64')] [string[]] $ValidationArchitectures = @('x64', 'x86', 'arm64'),
  [string] $Work = (Join-Path ([IO.Path]::GetTempPath()) ("craft-installer-test-" + [guid]::NewGuid()))
)
$ErrorActionPreference = 'Stop'
trap { Write-Output "FAILED: $_"; exit 1 }
$Apps = 'cadcraft', 'deckcraft', 'designcraft', 'effectcraft', 'figcraft', 'filmcraft',
        'gridcraft', 'lightcraft', 'pdfcraft', 'photocraft', 'soundcraft', 'vectorcraft',
        'wordcraft', 'craft-launcher'
if ($Only) {
  if ($StartAt) { throw 'Only and StartAt cannot be combined.' }
  if ($Apps -notcontains $Only) { throw "Unknown app: $Only" }
  $Apps = @($Only)
}
if ($StartAt -and $Apps -notcontains $StartAt) { throw "Unknown app: $StartAt" }
$Started = -not $StartAt
$Folders = 'effectcraft', 'figcraft', 'lightcraft', 'photocraft', 'vectorcraft', 'craft-launcher'
$Desktop = @{ cadcraft = 'DESKTOPSHORTCUT'; effectcraft = 'DESKTOPSHORTCUT'; gridcraft = 'DESKTOPSHORTCUT';
              pdfcraft = 'INSTALLDESKTOPSHORTCUT'; vectorcraft = 'INSTALLDESKTOPSHORTCUT'; 'craft-launcher' = 'INSTALLDESKTOPSHORTCUT' }
$Launch = @{ figcraft = 'LaunchFigcraft'; lightcraft = 'LaunchLightcraft'; photocraft = 'LaunchPhotocraft';
             vectorcraft = 'LaunchVectorcraft'; wordcraft = 'LaunchWordcraft'; 'craft-launcher' = 'LaunchArtCraftLauncher' }
$Installer = New-Object -ComObject WindowsInstaller.Installer
$Installer.UILevel = 2
New-Item -ItemType Directory -Force $Work | Out-Null
$License = Join-Path $Work 'license.rtf'
[IO.File]::WriteAllText($License, '{\rtf1\ansi Synthetic test fixture license.}')

function Read-Value([object] $Database, [string] $Sql) {
  $view = $Database.OpenView($Sql)
  try {
    [void] $view.Execute()
    $row = $view.Fetch()
    if ($null -eq $row) { return $null }
    return $row.StringData(1)
  } finally { [void] $view.Close() }
}
function Assert-Equal($Actual, $Expected, [string] $What) {
  if ($Actual -ne $Expected) { throw "$What`: expected '$Expected', got '$Actual'" }
}

foreach ($app in $Apps) {
  if (-not $Started) {
    if ($app -ne $StartAt) { continue }
    $Started = $true
  }
  $name = if ($app -eq 'craft-launcher') { 'artcraft-launcher' } else { $app }
  $repo = Join-Path $Repositories $app
  $packaging = Join-Path $repo 'packaging/windows'
  $source = Join-Path $packaging "$name.wxs"
  $ui = Join-Path $packaging 'installer-ui.wxs'
  $bin = Join-Path $Work "$app-bin"
  New-Item -ItemType Directory -Force $bin | Out-Null
  # Fixtures only supply PE bytes; none is ever run. Include conditional CLI files too.
  foreach ($match in [regex]::Matches((Get-Content -Raw $source), 'Source="\$\(var\.BinDir\)\\([^"/]+)"')) {
    Copy-Item (Join-Path $env:SystemRoot 'System32/notepad.exe') (Join-Path $bin $match.Groups[1].Value)
  }
  $icon = Join-Path $repo "assets/app-icon/$name.ico"
  # Match the release package.ps1 extension sets; extra extensions can mask missing dependencies.
  $extensions = switch ($app) {
    { $_ -in 'figcraft','lightcraft','photocraft','vectorcraft','craft-launcher' } { '-ext'; 'WixToolset.UI.wixext'; '-ext'; 'WixToolset.Util.wixext'; break }
    'wordcraft' { '-ext'; 'WixToolset.Util.wixext'; break }
    default { @() }
  }
  foreach ($arch in 'x64', 'x86', 'arm64') {
    $msi = Join-Path $Work "$app-$arch.msi"
    $modelArgs = @()
    if ($app -eq 'pdfcraft') {
      if (-not $ModelsDirectory -or -not (Test-Path (Join-Path $ModelsDirectory 'ATTRIBUTION.txt'))) {
        throw 'PdfCraft fixtures require -ModelsDirectory with the qualified OCR models and licences.'
      }
      $modelArgs = @('-d', "ModelsDir=$ModelsDirectory")
    }
    & wix build $source $ui -arch $arch @extensions @modelArgs `
      -d Version=90.0.0 -d DisplayVersion=90.0.0 -d "BinDir=$bin" -d "IconPath=$icon" `
      -d HasCli=1 -d "LicenseRtf=$License" -o $msi
    if ($LASTEXITCODE -ne 0) { throw "$app $arch compilation/linking failed" }
    if ($ValidationArchitectures -contains $arch) {
      & wix msi validate $msi
      if ($LASTEXITCODE -ne 0) { throw "$app $arch MSI validation failed" }
    } else {
      Write-Host "Skipped ICE validation for $app $arch; compilation and UI table checks still run."
    }
    $db = $Installer.OpenDatabase($msi, 0)
    $allUsers = Read-Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ALLUSERS'''
    if ($app -eq 'craft-launcher') {
      if ($allUsers) { throw 'Launcher must remain per-user' }
    } else {
      Assert-Equal $allUsers '1' "$app per-machine scope"
    }
    Assert-Equal (Read-Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ARPSYSTEMCOMPONENT''') '1' "$app native ARP entry hidden"
    $settingsKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Craft.' + $app
    $settingsWhere = ' WHERE `Key` = ''' + $settingsKey + ''''
    $root = if ($app -eq 'craft-launcher') { '1' } else { '2' }
    Assert-Equal (Read-Value $db ('SELECT `Root` FROM `Registry`' + $settingsWhere + ' AND `Name` = ''DisplayName''')) $root "$app Settings registration scope"
    Assert-Equal (Read-Value $db ('SELECT `Value` FROM `Registry`' + $settingsWhere + ' AND `Name` = ''DisplayName''')) '[ProductName] (Craft Apps)' "$app legacy Launcher compatibility"
    $noModify = if ($app -in 'cadcraft','lightcraft','figcraft','photocraft','vectorcraft','wordcraft','craft-launcher') { '1' } else { $null }
    Assert-Equal (Read-Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ARPNOMODIFY''') $noModify "$app original Modify policy"
    $systemFolder = if ($arch -eq 'x86') { 'SystemFolder' } else { 'System64Folder' }
    $command = '"[' + $systemFolder + ']msiexec.exe" /x [ProductCode] '
    Assert-Equal (Read-Value $db ('SELECT `Value` FROM `Registry`' + $settingsWhere + ' AND `Name` = ''UninstallString''')) ($command + '/qf /norestart') "$app interactive Settings removal"
    Assert-Equal (Read-Value $db ('SELECT `Value` FROM `Registry`' + $settingsWhere + ' AND `Name` = ''QuietUninstallString''')) ($command + '/qn /norestart') "$app quiet Settings removal"
    if (Read-Value $db ('SELECT `Value` FROM `Registry`' + $settingsWhere + ' AND `Name` = ''WindowsInstaller''')) { throw "$app Settings must invoke its command, not MSI's basic UI API" }
    Assert-Equal (Read-Value $db 'SELECT `Feature_` FROM `FeatureComponents` WHERE `Component_` = ''CraftInstallerSettingsEntry''') 'Main' "$app Settings component selected"
    Assert-Equal (Read-Value $db 'SELECT `Condition` FROM `Component` WHERE `Component` = ''CraftInstallerSettingsNoModify''') 'ARPNOMODIFY = "1"' "$app change-button policy"
    Assert-Equal (Read-Value $db 'SELECT `Control_First` FROM `Dialog` WHERE `Dialog` = ''InstallComplete''') 'Finish' "$app completion focus"
    if ($Folders -contains $app) {
      $first = if ($app -eq 'effectcraft') { 'Install' } else { 'Folder' }
      Assert-Equal (Read-Value $db 'SELECT `Control_First` FROM `Dialog` WHERE `Dialog` = ''InstallFolder''') $first "$app enabled folder-dialog focus"
    }
    Assert-Equal (Read-Value $db 'SELECT `Name` FROM `Binary` WHERE `Name` = ''CraftInstallerErrorIcon''') 'CraftInstallerErrorIcon' "$app warning icon binary linked"
    Assert-Equal (Read-Value $db 'SELECT `Type` FROM `Control` WHERE `Dialog_` = ''CraftInstallerErrorDlg'' AND `Control` = ''ErrorIcon''') 'Icon' "$app native warning icon"
    foreach ($sequence in 'InstallUISequence', 'AdminUISequence') {
      foreach ($exit in @(@('InstallComplete', '-1'), @('InstallCancelled', '-2'), @('InstallFailed', '-3'))) {
        $where = ' WHERE `Action` = ''' + $exit[0] + ''''
        Assert-Equal (Read-Value $db ('SELECT `Condition` FROM `' + $sequence + '`' + $where)) 'UILevel = 5' "$app $sequence exit UI level"
        Assert-Equal (Read-Value $db ('SELECT `Sequence` FROM `' + $sequence + '`' + $where)) $exit[1] "$app $sequence exit sequence"
      }
    }
    foreach ($ending in 'InstallComplete', 'InstallCancelled', 'InstallFailed') {
      Assert-Equal (Read-Value $db ('SELECT `Argument` FROM `ControlEvent` WHERE `Dialog_` = ''' + $ending + ''' AND `Control_` = ''Finish'' AND `Event` = ''EndDialog''')) 'Return' "$app $ending Finish"
    }
    $removed = Read-Value $db 'SELECT `Text` FROM `Control` WHERE `Dialog_` = ''InstallComplete'' AND `Control` = ''Removed'''
    if ($removed -notmatch 'was uninstalled') { throw "$app lost its removal message" }
    $welcome = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''InstallWelcome''')
    $cost = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''CostInitialize''')
    if ($welcome -ge $cost) { throw "$app shortcut choice must precede costing" }
    $maintenance = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''InstallMaintenance''')
    $finalize = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''CostFinalize''')
    $execute = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''ExecuteAction''')
    if ($maintenance -le $finalize -or $maintenance -ge $execute) { throw "$app maintenance must follow costing before execution (MSI error 2731)" }
    foreach ($choice in @(@('Repair', 'repair'), @('Remove', 'remove'))) {
      Assert-Equal (Read-Value $db ('SELECT `Argument` FROM `ControlEvent` WHERE `Dialog_` = ''InstallMaintenance'' AND `Control_` = ''' + $choice[0] + ''' AND `Event` = ''[CraftInstallerOperation]''')) $choice[1] "$app client maintenance outcome"
    }
    if ($Folders -contains $app) {
      $folder = [int](Read-Value $db 'SELECT `Sequence` FROM `InstallUISequence` WHERE `Action` = ''InstallFolder''')
      if ($folder -le $finalize) { throw "$app folder dialog must follow CostFinalize" }
    }
    if ($Desktop.ContainsKey($app)) {
      $property = $Desktop[$app]
      if ($app -ne 'craft-launcher') {
        $secure = Read-Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''SecureCustomProperties'''
        if (($secure -split ';') -notcontains $property) { throw "$app shortcut choice is not secure" }
      }
      foreach ($value in '1', '0') {
        $condition = Read-Value $db ('SELECT `Condition` FROM `ControlEvent` WHERE `Dialog_` = ''InstallWelcome'' AND `Control_` = ''Install'' AND `Event` = ''[' + $property + ']'' AND `Argument` = ''' + $value + '''')
        if (-not $condition) { throw "$app missing explicit shortcut value $value" }
      }
    }
    if ($Launch.ContainsKey($app)) {
      $condition = Read-Value $db ('SELECT `Condition` FROM `ControlEvent` WHERE `Dialog_` = ''InstallComplete'' AND `Control_` = ''Finish'' AND `Event` = ''DoAction'' AND `Argument` = ''' + $Launch[$app] + '''')
      foreach ($guard in 'NOT Installed', 'NOT REMOVE', 'NOT REINSTALL') {
        if (-not $condition.Contains($guard)) { throw "$app launch is missing $guard" }
      }
      Assert-Equal (Read-Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''WIXUI_EXITDIALOGOPTIONALCHECKBOX''') '1' "$app launch default"
    }
    Write-Output "ok $app $arch compiled MSI UI contract"
  }
}
Write-Output "MSI fixtures retained in $Work"
