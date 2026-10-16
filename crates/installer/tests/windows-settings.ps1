<#
.SYNOPSIS
  Verify Settings registration and interactive/quiet removal on a disposable desktop.
.DESCRIPTION
  Uses only version 90.0.0 fixtures from windows-suite.ps1. The actual Settings button
  workflow must additionally be exercised on the visible desktop; these checks invoke
  the exact registered commands for every profile and verify their outcomes.
#>
param(
 [Parameter(Mandatory)] [switch] $DisposableMachine,
 [Parameter(Mandatory)] [string] $Packages,
 [ValidateSet('x64','x86')] [string] $Architecture = 'x64',
 [string] $Results = (Join-Path $Packages 'settings')
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'windows-ui.ps1') -DisposableMachine:$DisposableMachine -Packages $Packages -Results $Results -DefineOnly
Assert-CleanFixtures $Architecture

function Settings-Values($app,$keyName,$native=$false) {
 if($native -and $app -eq 'craft-launcher'){
  # An elevated per-user MSI can register its hidden native entry in HKLM.
  # Search the same hives as Launcher; the visible entry must still be HKCU.
  $values=Settings-Values 'machine' $keyName
  if($null -ne $values){return $values}
 }
 $hive=if($app -eq 'craft-launcher'){[Microsoft.Win32.RegistryHive]::CurrentUser}else{[Microsoft.Win32.RegistryHive]::LocalMachine}
 $view=if($Architecture -eq 'x86'){[Microsoft.Win32.RegistryView]::Registry32}else{[Microsoft.Win32.RegistryView]::Registry64}
 $root=[Microsoft.Win32.RegistryKey]::OpenBaseKey($hive,$view)
 try {
  $key=$root.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\Uninstall\$keyName")
  if($null -eq $key){return $null}
  try {
   $values=@{}
   foreach($name in $key.GetValueNames()){$values[$name]=$key.GetValue($name)}
   return $values
  } finally {$key.Close()}
 } finally {$root.Close()}
}
function Registered-Removal($command,$code,$quiet,$tag) {
 $level=if($quiet){'qn'}else{'qf'}
 Assert ($command -match '^"([^"\r\n]+)" /x (\{[0-9A-Fa-f-]{36}\}) /(qf|qn) /norestart$') "$tag unexpected registered command"
 $exe=$Matches[1];$product=$Matches[2];$ui=$Matches[3]
 Assert ($product -eq $code -and $ui -eq $level) "$tag stale product or incorrect UI level"
 Assert ((Split-Path $exe -Leaf) -eq 'msiexec.exe' -and (Split-Path (Split-Path $exe)) -eq $env:WINDIR) "$tag must use the system installer"
 $log=Join-Path $Results "$tag.log"
 $p=Start-Process $exe -ArgumentList "/x $code /$level /norestart /l*v `"$log`"" -PassThru
 [void]$p.Handle
 return $p
}
foreach($app in 'cadcraft','deckcraft','designcraft','effectcraft','figcraft','filmcraft','gridcraft','lightcraft','pdfcraft','photocraft','soundcraft','vectorcraft','wordcraft','craft-launcher') {
 $msi=Join-Path $Packages "$app-$Architecture.msi"
 $db=$engine.OpenDatabase($msi,0)
 $code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
 $name=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductName'''
 foreach($quiet in $false,$true) {
  $tag="$app-settings-"+$(if($quiet){'quiet'}else{'interactive'})
  $p=Start-Msi $msi '/i' "$tag-install" '/qn'
  Assert ($p.WaitForExit(300000) -and $p.ExitCode -eq 0) "$tag silent install failed"
  $entry=Settings-Values $app "Craft.$app"
  Assert ($null -ne $entry) "$tag visible entry missing"
  Assert ($entry.DisplayName -eq "$name (Craft Apps)" -and $entry.DisplayVersion -eq '90.0.0') "$tag display metadata changed"
  Assert ($entry.Publisher -eq 'Learning Machines LLC') "$tag publisher changed"
  Assert (-not $entry.ContainsKey('WindowsInstaller') -and -not $entry.ContainsKey('SystemComponent')) "$tag must be a visible command entry"
  Assert (($entry.NoModify -eq 1) -eq ($app -in 'cadcraft','lightcraft','figcraft','photocraft','vectorcraft','wordcraft','craft-launcher')) "$tag changed the Modify policy"
  $native=Settings-Values $app $code $true
  Assert ($null -ne $native -and $native.SystemComponent -eq 1) "$tag native MSI entry not hidden"
  Assert ($native.WindowsInstaller -eq 1 -and $native.DisplayName -eq $name -and $native.Publisher -eq $entry.Publisher) "$tag native entry must remain discoverable by Launcher"
  Assert ($engine.ProductState($code) -eq 5) "$tag MSI registration missing"
  $command=if($quiet){$entry.QuietUninstallString}else{$entry.UninstallString}
  $p=Registered-Removal $command $code $quiet $tag
  if($quiet){
   Assert ($p.WaitForExit(300000) -and $p.ExitCode -eq 0) "$tag quiet removal waited for UI or failed"
   "ok $tag unattended exit 0"
  }else{Complete $p 'remove' $tag}
  Assert ($engine.ProductState($code) -eq -1) "$tag product still installed"
  Assert ($null -eq (Settings-Values $app "Craft.$app")) "$tag left its visible entry behind"
  Assert ($null -eq (Settings-Values $app $code $true)) "$tag left its native entry behind"
 }
}
