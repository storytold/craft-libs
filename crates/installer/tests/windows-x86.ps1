<#
.SYNOPSIS
  Exercise fixture MSI options on a clean, disposable Windows desktop.
.EXAMPLE
  .\windows-x86.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled
#>
param(
 [Parameter(Mandatory)] [switch] $DisposableMachine,
 [Parameter(Mandatory)] [string] $Packages,
 [string] $Results = (Join-Path $Packages 'x86')
)
if (-not $DisposableMachine) { throw 'An explicitly disposable Windows machine is required.' }
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'windows-ui.ps1') -DisposableMachine -Packages $Packages -Results $Results -DefineOnly
Assert-CleanFixtures x86
foreach($app in 'cadcraft','deckcraft','designcraft','effectcraft','figcraft','filmcraft','gridcraft','lightcraft','pdfcraft','photocraft','soundcraft','vectorcraft','wordcraft','craft-launcher') {
 $msi=Join-Path $Packages "$app-x86.msi"
 $db=$engine.OpenDatabase($msi,0)
 $code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
 $exe=if($app -eq 'craft-launcher'){'artcraft-launcher.exe'}else{"$app.exe"}
 $hive=if($app -eq 'craft-launcher'){[Microsoft.Win32.RegistryHive]::CurrentUser}else{[Microsoft.Win32.RegistryHive]::LocalMachine}
 $p=Start-Msi $msi '/i' "$app-x86-install" '/qn';Assert ($p.WaitForExit(300000)) "$app x86 install hung";Assert ($p.ExitCode -eq 0) "$app x86 install failed $($p.ExitCode)"
 Assert ($engine.ProductState($code) -eq 5) "$app x86 not registered"
 $root=[Microsoft.Win32.RegistryKey]::OpenBaseKey($hive,[Microsoft.Win32.RegistryView]::Registry32)
 try{$key=$root.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\App Paths\$exe");Assert ($null -ne $key) "$app x86 App Paths missing";$path=$key.GetValue('');$key.Close();Assert (Test-Path -LiteralPath $path) "$app x86 file missing"}finally{$root.Close()}
 $p=Start-Msi $code '/x' "$app-x86-remove" '/qn';Assert ($p.WaitForExit(300000)) "$app x86 removal hung";Assert ($p.ExitCode -eq 0) "$app x86 removal failed $($p.ExitCode)"
 Assert ($engine.ProductState($code) -eq -1) "$app x86 still registered"
 Assert (-not (Test-Path -LiteralPath $path)) "$app x86 executable remains"
 "ok $app x86 silent install and cached-package removal"
}
