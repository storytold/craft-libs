<#
.SYNOPSIS
  Exercise fixture MSI options on a clean, disposable Windows desktop.
.EXAMPLE
  .\windows-options.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled -Repositories C:\CraftInstallerTest\repos
#>
param(
 [Parameter(Mandatory)] [switch] $DisposableMachine,
 [Parameter(Mandatory)] [string] $Packages,
 [string] $Results = (Join-Path $Packages 'options'),
 [Parameter(Mandatory)] [string] $Repositories,
 [string] $ModelsDirectory
)
if (-not $DisposableMachine) { throw 'An explicitly disposable Windows machine is required.' }
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'windows-ui.ps1') -DisposableMachine -Packages $Packages -Results $Results -DefineOnly
Assert-CleanFixtures x64
$Repos=$Repositories
function MainPath($app) {
 $exe=if($app -eq 'craft-launcher'){'artcraft-launcher.exe'}else{"$app.exe"}
 $scope=if($app -eq 'craft-launcher'){'HKCU:'}else{'HKLM:'}
 (Get-Item "$scope\Software\Microsoft\Windows\CurrentVersion\App Paths\$exe").GetValue('')
}
function Assert-SettingsProduct($app,$msi) {
 $scope=if($app -eq 'craft-launcher'){'HKCU:'}else{'HKLM:'}
 $key=Get-Item "$scope\Software\Microsoft\Windows\CurrentVersion\Uninstall\Craft.$app"
 $db=$engine.OpenDatabase($msi,0)
 $code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
 $version=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductVersion'''
 Assert ($key.GetValue('DisplayVersion') -eq $version -and $key.GetValue('UninstallString') -like "*/x $code /qf /norestart") "$app Settings registration did not follow repair/upgrade"
}
function UpgradePackage($app,$bin='') {
 $name=if($app -eq 'craft-launcher'){'artcraft-launcher'}else{$app}
 $pack=Join-Path $Repos "$app\packaging\windows"
 if(-not $bin){$bin=Join-Path $Packages "$app-bin"}
 $msi=Join-Path $Packages "$app-upgrade.msi"
 $modelArgs=@();if($app -eq 'pdfcraft'){if(-not $ModelsDirectory -or -not(Test-Path (Join-Path $ModelsDirectory 'ATTRIBUTION.txt'))){throw 'PdfCraft option fixtures require qualified -ModelsDirectory'};$modelArgs=@('-d',"ModelsDir=$ModelsDirectory")}
 & wix build "$pack\$name.wxs" "$pack\installer-ui.wxs" -arch x64 @modelArgs -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext -d Version=90.0.1 -d DisplayVersion=90.0.1 -d "BinDir=$bin" -d "IconPath=$Repos\$app\assets\app-icon\$name.ico" -d HasCli=1 -d "LicenseRtf=$Packages\license.rtf" -o $msi | Out-Host
 Assert ($LASTEXITCODE -eq 0) "$app upgrade fixture failed to build"
 return $msi
}
# Upgrade every folder profile, keeping a custom directory selected through the full UI.
foreach($app in 'effectcraft','figcraft','lightcraft','photocraft','vectorcraft','craft-launcher') {
 $old=Join-Path $Packages "$app-x64.msi";$new=UpgradePackage $app
 $folder="C:\CraftInstallerTest\Upgrade folder $([char]0xe9) $app"
 $p=Start-Msi $old '/i' "$app-folder-first";Complete $p 'install' "$app-folder-first" $folder
 $before=MainPath $app
 $p=Start-Msi $new '/i' "$app-folder-upgrade";Complete $p 'install' "$app-folder-upgrade"
 Assert ((MainPath $app) -eq $before) "$app upgrade lost the custom directory"
 Assert-SettingsProduct $app $new
 if($app -eq 'figcraft'){$p=Start-Msi $old '/i' 'figcraft-downgrade';Complete $p 'fail' 'figcraft-downgrade';Assert ((MainPath $app) -eq $before) 'Failed downgrade changed the installation'}
 $p=Start-Msi $new '/x' "$app-folder-remove";Complete $p 'remove' "$app-folder-remove"
}
# Every optional desktop shortcut can be enabled through the full UI and cleaned up.
foreach($app in 'cadcraft','effectcraft','gridcraft','pdfcraft','vectorcraft','craft-launcher') {
 $msi=Join-Path $Packages "$app-x64.msi"
 $db=$engine.OpenDatabase($msi,0);$product=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductName'''
 $desktop=Join-Path ([Environment]::GetFolderPath($(if($app -eq 'craft-launcher'){'DesktopDirectory'}else{'CommonDesktopDirectory'}))) "$product.lnk"
 $p=Start-Msi $msi '/i' "$app-desktop-on";Complete $p 'install' "$app-desktop-on" '' $true
 Assert (Test-Path -LiteralPath $desktop) "$app desktop opt-in ignored"
 Remove-Item -LiteralPath $desktop
 $scope=if($app -eq 'craft-launcher'){'HKCU:'}else{'HKLM:'}
 Remove-Item "$scope\Software\Microsoft\Windows\CurrentVersion\Uninstall\Craft.$app" -Recurse
 $p=Start-Msi $msi '/i' "$app-desktop-repair";Complete $p 'repair' "$app-desktop-repair"
 Assert (Test-Path -LiteralPath $desktop) "$app repair lost desktop opt-in"
 Assert-SettingsProduct $app $msi
 $new=UpgradePackage $app
 $p=Start-Msi $new '/i' "$app-desktop-upgrade-off";Complete $p 'install' "$app-desktop-upgrade-off" '' $false $false $true
 Assert (-not (Test-Path -LiteralPath $desktop)) "$app upgrade ignored desktop opt-out"
 Assert-SettingsProduct $app $new
 $p=Start-Msi $new '/x' "$app-desktop-remove";Complete $p 'remove' "$app-desktop-remove"
 Assert (-not (Test-Path -LiteralPath $desktop)) "$app left desktop shortcut behind"
 if($app -eq 'gridcraft') {
  $p=Start-Msi $msi '/i' 'gridcraft-system-first';Complete $p 'install' 'gridcraft-system-first' '' $true
  $db=$engine.OpenDatabase($msi,0);$code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
  $task='CraftInstallerTest-'+[guid]::NewGuid().ToString('N')
  $log=Join-Path $Results 'gridcraft-system-remove.log'
  $action=New-ScheduledTaskAction -Execute "$env:WINDIR\System32\msiexec.exe" -Argument "/x $code /qn /norestart /l*v `"$log`""
  $principal=New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
  try {
   Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings (New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 5))|Out-Null
   $started=Get-Date;Start-ScheduledTask $task;$deadline=$started.AddMinutes(5)
   do {Start-Sleep -Milliseconds 500;$info=Get-ScheduledTaskInfo $task;$state=(Get-ScheduledTask $task).State} while(($info.LastRunTime -lt $started.AddSeconds(-1) -or $state -eq 'Running') -and (Get-Date) -lt $deadline)
   Assert ($info.LastTaskResult -eq 0) "SYSTEM removal failed: $($info.LastTaskResult)"
   Assert ($engine.ProductState($code) -eq -1) 'SYSTEM left GridCraft registered'
   Assert (-not (Test-Path $desktop)) 'SYSTEM left the Public desktop shortcut'
   $key=Get-Item HKLM:\Software\GridCraft\Installer -ErrorAction SilentlyContinue
   Assert ($null -eq $key -or $null -eq $key.GetValue('DesktopShortcutComponent')) 'SYSTEM left the shortcut marker'
   $key=Get-Item HKCU:\Software\GridCraft\Installer -ErrorAction SilentlyContinue
   Assert ($null -eq $key -or $null -eq $key.GetValue('DesktopShortcutComponent')) 'Shortcut marker leaked into HKCU'
   'ok gridcraft SYSTEM uninstall cleaned up the machine shortcut and marker'
  } finally {Stop-ScheduledTask $task -ErrorAction SilentlyContinue;Unregister-ScheduledTask $task -Confirm:$false -ErrorAction SilentlyContinue}
 }
}
# EffectCraft's unattended upgrades retain the saved shortcut choice without wizard input.
$app='effectcraft';$old=Join-Path $Packages 'effectcraft-x64.msi';$new=UpgradePackage $app
$desktop=Join-Path ([Environment]::GetFolderPath('CommonDesktopDirectory')) 'EffectCraft.lnk'
foreach($enabled in @($false,$true)) {
 $tag='effectcraft-quiet-retain-'+$(if($enabled){'on'}else{'off'})
 $choice=if($enabled){'1'}else{'0'}
 $p=Start-Msi $old '/i' "$tag-install" "/qn DESKTOPSHORTCUT=$choice"
 Assert ($p.WaitForExit(300000) -and $p.ExitCode -eq 0) "$tag initial install failed or waited for input"
 $p=Start-Msi $new '/i' "$tag-upgrade" '/qn'
 Assert ($p.WaitForExit(300000) -and $p.ExitCode -eq 0) "$tag upgrade failed or waited for input"
 Assert ((Test-Path $desktop) -eq $enabled) "$tag upgrade changed the saved choice"
 Assert-SettingsProduct $app $new
 $p=Start-Msi $new '/x' "$tag-remove" '/qn'
 Assert ($p.WaitForExit(300000) -and $p.ExitCode -eq 0) "$tag removal failed or waited for input"
 Assert (-not(Test-Path $desktop)) "$tag removal left the shortcut"
}
# PDF context-menu choices survive repair and can be changed during a major upgrade.
$app='pdfcraft'
$old=Join-Path $Packages 'pdfcraft-x64.msi';$new=UpgradePackage $app
$menu='HKLM:\Software\Classes\SystemFileAssociations\.pdf\shell\PdfCraft.Edit'
foreach($enabled in @($false,$true)) {
 $tag='pdfcraft-context-'+$(if($enabled){'on'}else{'off'})
 $p=Start-Msi $old '/i' "$tag-install"
 Complete $p 'install' "$tag-install" '' $null $false $null $enabled
 Assert ((Test-Path $menu) -eq $enabled) "$tag initial choice ignored"
 $p=Start-Msi $old '/i' "$tag-repair";Complete $p 'repair' "$tag-repair"
 Assert ((Test-Path $menu) -eq $enabled) "$tag repair changed the choice"
 $p=Start-Msi $new '/i' "$tag-upgrade"
 Complete $p 'install' "$tag-upgrade" '' $null $false $null (-not $enabled)
 Assert ((Test-Path $menu) -eq (-not $enabled)) "$tag upgrade ignored the new choice"
 $p=Start-Msi $new '/i' "$tag-upgraded-repair";Complete $p 'repair' "$tag-upgraded-repair"
 Assert ((Test-Path $menu) -eq (-not $enabled)) "$tag upgraded repair changed the choice"
 $p=Start-Msi $new '/x' "$tag-remove";Complete $p 'remove' "$tag-remove"
 Assert (-not(Test-Path $menu)) "$tag removal left the context menu"
}
# Record launches with an original GUI fixture that exits immediately.
$stub=Join-Path $Results 'LaunchFixture.cs'
@'
using System;using System.IO;using System.Diagnostics;using System.Security.Principal;
[assembly:System.Reflection.AssemblyFileVersion("90.0.1.0")]
public static class Program {public static void Main(){var folder=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"CraftInstallerTest");Directory.CreateDirectory(folder);var id=WindowsIdentity.GetCurrent();File.AppendAllText(Path.Combine(folder,"launches.log"),Process.GetCurrentProcess().MainModule.FileName+"|"+id.User.Value+Environment.NewLine);}}
'@ | Set-Content $stub -Encoding UTF8
$fixture=Join-Path $Results 'LaunchFixture.exe'
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /platform:anycpu /target:winexe "/out:$fixture" $stub
Assert ($LASTEXITCODE -eq 0) 'Launch fixture compilation failed'
$marker=Join-Path $env:LOCALAPPDATA 'CraftInstallerTest\launches.log'
function Launches {if(Test-Path $marker){@(Get-Content $marker)}else{@()}}
foreach($app in 'figcraft','lightcraft','photocraft','vectorcraft','wordcraft','craft-launcher') {
 $bin=Join-Path $Results "$app-launch-bin";New-Item -ItemType Directory -Force $bin|Out-Null
 foreach($f in (Get-ChildItem (Join-Path $Packages "$app-bin") -Filter *.exe)){Copy-Item $fixture (Join-Path $bin $f.Name)}
 $msi=UpgradePackage $app $bin
 $count=@(Launches).Count
 $p=Start-Msi $msi '/i' "$app-launch-on";Complete $p 'install' "$app-launch-on" '' $null $true
 $deadline=(Get-Date).AddMinutes(2)
 while(@(Launches).Count -eq $count -and (Get-Date) -lt $deadline){Start-Sleep -Milliseconds 500}
 $lines=@(Launches);Assert ($lines.Count -eq $count+1) "$app Finish did not launch exactly once"
 $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
 Assert ($lines[-1].EndsWith("|$sid")) "$app launched as a different account"
 $p=Start-Msi $msi '/i' "$app-launch-repair";Complete $p 'repair' "$app-launch-repair"
 Assert (@(Launches).Count -eq $count+1) "$app repair launched the app"
 $p=Start-Msi $msi '/x' "$app-launch-remove";Complete $p 'remove' "$app-launch-remove"
 Assert (@(Launches).Count -eq $count+1) "$app removal launched the app"
 $p=Start-Msi $msi '/i' "$app-launch-off";Complete $p 'install' "$app-launch-off"
 Assert (@(Launches).Count -eq $count+1) "$app unchecked Finish launched the app"
 $p=Start-Msi $msi '/x' "$app-launch-off-remove";Complete $p 'remove' "$app-launch-off-remove"
}
