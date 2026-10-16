<#
.SYNOPSIS
  Exercise full MSI install, repair and cached-product removal on a disposable Windows desktop.
.DESCRIPTION
  Use only the version 90.0.0 fixtures built by windows-suite.ps1. Requires an interactive,
  administrator desktop with no existing suite installations. Never run against a developer's
  normal Windows installation: fixture packages share the real products' UpgradeCodes.
.EXAMPLE
  .\windows-ui.ps1 -DisposableMachine -Packages C:\CraftInstallerTest\compiled
#>
param(
 [Parameter(Mandatory)] [switch] $DisposableMachine,
 [Parameter(Mandatory)] [string] $Packages,
 [string] $Results = (Join-Path $Packages 'interactive'),
 [switch] $DefineOnly,
 [string] $StartAt
)
if (-not $DisposableMachine) { throw 'An explicitly disposable Windows machine is required.' }
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Force $Results | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class MsiWindow {
 public IntPtr Handle; public string Text; public string Class; public bool Enabled; public long Style;
}
public static class MsiDesktop {
 [DllImport("user32.dll")] public static extern int GetSystemMetrics(int n);
 public delegate bool Callback(IntPtr h,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumWindows(Callback c,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,Callback c,IntPtr p);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr h);
 [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h,int n);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,string l);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 static MsiWindow Read(IntPtr h) {var t=new StringBuilder(4096); var c=new StringBuilder(256);GetWindowText(h,t,t.Capacity);GetClassName(h,c,c.Capacity);return new MsiWindow{Handle=h,Text=t.ToString(),Class=c.ToString(),Enabled=IsWindowEnabled(h),Style=GetWindowLong(h,-16)};}
 public static MsiWindow[] Windows() {var a=new List<MsiWindow>();EnumWindows((h,p)=>{if(IsWindowVisible(h))a.Add(Read(h));return true;},IntPtr.Zero);return a.ToArray();}
 public static MsiWindow[] Controls(IntPtr root) {var a=new List<MsiWindow>();EnumChildWindows(root,(h,p)=>{if(IsWindowVisible(h))a.Add(Read(h));return true;},IntPtr.Zero);return a.ToArray();}
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 public static void Click(IntPtr h){SendMessage(h,0xF5,IntPtr.Zero,IntPtr.Zero);}
 [StructLayout(LayoutKind.Sequential)] struct GuiThreadInfo {
  public uint Size,Flags; public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret;
  public int Left,Top,Right,Bottom;
 }
 [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref GuiThreadInfo info);
 public static IntPtr Focused(){var g=new GuiThreadInfo{Size=(uint)Marshal.SizeOf(typeof(GuiThreadInfo))};return GetGUIThreadInfo(0,ref g)?g.Focus:IntPtr.Zero;}
 public static int Checked(IntPtr h){return SendMessage(h,0xF0,IntPtr.Zero,IntPtr.Zero).ToInt32();}
 public static void SetText(IntPtr h,string s){SendMessage(h,0xC,IntPtr.Zero,s);}
}
'@
$engine=New-Object -ComObject WindowsInstaller.Installer
$keyboard=New-Object -ComObject WScript.Shell
$defaultDesktop=@{cadcraft=$false;effectcraft=$true;gridcraft=$false;pdfcraft=$true;vectorcraft=$true;'craft-launcher'=$true}
function Value($db,$sql) { $v=$db.OpenView($sql);try{[void]$v.Execute();$r=$v.Fetch();if($r){$r.StringData(1)}}finally{[void]$v.Close()} }
function Assert($yes,$message){if(-not $yes){throw $message}}
function Start-Msi($msi,$operation,$tag,$extra='/qf') {
 Write-Host "starting $tag"
 $log=Join-Path $Results "$tag.log"
 $p=Start-Process msiexec.exe -ArgumentList "$operation `"$msi`" /norestart /l*v `"$log`" $extra" -PassThru
 [void]$p.Handle
 return $p
}
function Save-Desktop($tag) {
 $bitmap = New-Object Drawing.Bitmap ([MsiDesktop]::GetSystemMetrics(78)), ([MsiDesktop]::GetSystemMetrics(79))
 $graphics = [Drawing.Graphics]::FromImage($bitmap)
 try {
  $graphics.CopyFromScreen([MsiDesktop]::GetSystemMetrics(76), [MsiDesktop]::GetSystemMetrics(77), 0, 0, $bitmap.Size)
  $bitmap.Save((Join-Path $Results "$tag.png"), [Drawing.Imaging.ImageFormat]::Png)
 } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Dialog {
 @([MsiDesktop]::Windows() | Where-Object { $_.Class -in '#32770','MsiDialogCloseClass' -and ($_.Text -like '* Setup' -or $_.Text -in 'Cancel setup','Windows Installer') })
}
function Controls($d){@([MsiDesktop]::Controls($d.Handle))}
function Button($c,$text){ @($c | Where-Object { $_.Class -eq 'Button' -and ($_.Text -replace '&','') -eq $text -and $_.Enabled }) | Select-Object -First 1 }
function Click($c,$text){$b=Button $c $text;Assert ($null -ne $b) "Missing enabled button $text";[MsiDesktop]::Click($b.Handle)}
function Complete($p,$mode,$tag,$folder='',$desktop=$null,$launchChecked=$false,$desktopInitial=$null,$contextMenu=$null,$contextMenuInitial=$true) {
 $deadline=(Get-Date).AddMinutes(12);$done=$false;$folderSeen=$false;$welcome=$false;$browsePending=$false;$browseDone=$false
 while((Get-Date) -lt $deadline) {
  $p.Refresh();if($p.HasExited){throw "$tag exited before completion, code $($p.ExitCode)"}
  foreach($d in (Dialog)) {
   [void][MsiDesktop]::SetForegroundWindow($d.Handle)
   $c=Controls $d;$text=($c.Text -join "`n")
   if($mode -eq 'remove' -and $d.Text -eq 'Windows Installer' -and $text -match 'Are you sure you want to uninstall this product') {Click $c 'Yes';break}
   if($text -match 'Setup completed successfully|Uninstall completed successfully|Setup cancelled|Setup failed') {
    if(-not (Button $c 'Finish')){continue}
    switch($mode){
     'remove' {Assert ($text -match 'Uninstall completed successfully' -and $text -match 'was uninstalled') "$tag missing removal confirmation"}
     'repair' {Assert ($text -match 'was repaired') "$tag missing repair confirmation"}
     'cancel' {Assert ($text -match 'Setup cancelled') "$tag missing cancellation confirmation"}
     'fail' {Assert ($text -match 'Setup failed') "$tag missing failure confirmation"}
     default {Assert ($text -match 'is installed') "$tag missing installation confirmation"}
    }
    $launch=@($c | Where-Object {$_.Class -eq 'Button' -and $_.Text -like 'Launch*'})
    if($mode -in 'remove','repair','cancel','fail'){Assert ($launch.Count -eq 0) "$tag must not offer launch"}
    foreach($l in $launch){Assert ([MsiDesktop]::Checked($l.Handle) -eq 1) "$tag launch default changed";if(-not $launchChecked){[MsiDesktop]::Click($l.Handle)}}
    $summary=@{tag=$tag;mode=$mode;title=$d.Text;controls=@($c | ForEach-Object {@{text=$_.Text;class=$_.Class;enabled=$_.Enabled}})}
    $summary|ConvertTo-Json -Depth 8|Set-Content (Join-Path $Results "$tag.json") -Encoding UTF8
    Start-Sleep -Seconds 2;$p.Refresh();Assert (-not $p.HasExited) "$tag completion did not wait for Finish"
    Save-Desktop $tag
    Click $c 'Finish';$done=$true;break
   }
   if($text -match 'setup could not be completed' -and $mode -ne 'fail'){throw "$tag failed: $text"}
   if(Button $c 'Repair') {Click $c $(if($mode -eq 'repair'){'Repair'}else{'Remove'});break}
   if($text -match 'Browse for a folder') {Click $c 'Cancel';$browsePending=$false;$browseDone=$true;break}
   if($text -match 'Choose the destination folder') {
    $folderSeen=$true
    if($browsePending){continue}
    if($tag -eq 'figcraft-install' -and -not $browseDone){$b=Button $c 'Browse...';Assert ($null -ne $b) 'Browse option missing';[void][MsiDesktop]::PostMessage($b.Handle,0xF5,[IntPtr]::Zero,[IntPtr]::Zero);$browsePending=$true;break}
    if($folder){
     $edit=@($c|Where-Object {$_.Class -in 'Edit','RichEdit20W'})|Select-Object -First 1
     $install=Button $c 'Install'
     if($null -eq $edit -or $null -eq $install){continue}
     # PathEdit commits its property on focus loss; BM_CLICK alone leaves it uncommitted.
     if(-not $keyboard.AppActivate($p.Id)){continue}
     # Do not assume the dialog initially focuses PathEdit: EffectCraft must focus an
     # enabled Install button when its folder is locked during upgrades.
     for($focusStep=0;$focusStep -lt 8 -and [MsiDesktop]::Focused() -ne $edit.Handle;$focusStep++){
      $keyboard.SendKeys('{TAB}');Start-Sleep -Milliseconds 250
     }
     Assert ([MsiDesktop]::Focused() -eq $edit.Handle) "$tag could not focus the destination field"
     [MsiDesktop]::SetText($edit.Handle,$folder)
     $keyboard.SendKeys('{TAB}');Start-Sleep -Milliseconds 250
    }
    Click $c 'Install';break
   }
   if($text -match 'accept the license terms') {
    $check=@($c|Where-Object {$_.Class -eq 'Button' -and $_.Text -match 'accept'})|Select-Object -First 1
    if($check -and [MsiDesktop]::Checked($check.Handle) -ne 1){[MsiDesktop]::Click($check.Handle)}
    $c=Controls $d;if(-not (Button $c 'Install')){continue};Click $c 'Install';break
   }
   if($text -match 'Setup will install') {
    $welcome=$true
    if($mode -eq 'cancel'){Click $c 'Cancel';break}
    if($null -ne $desktop){$check=@($c|Where-Object {$_.Class -eq 'Button' -and $_.Text -match 'desktop'})|Select-Object -First 1;if($null -eq $check){continue};Assert (([MsiDesktop]::Checked($check.Handle) -eq 1) -eq $(if($null -eq $desktopInitial){$defaultDesktop[$app]}else{$desktopInitial})) "$tag changed the fresh shortcut default";if(([MsiDesktop]::Checked($check.Handle) -eq 1) -ne $desktop){[MsiDesktop]::Click($check.Handle)}}
    if($null -ne $contextMenu){
     $check=@($c|Where-Object {$_.Class -eq 'Button' -and $_.Text -match 'Edit with PdfCraft'})|Select-Object -First 1
     if($null -eq $check){continue}
     Assert (([MsiDesktop]::Checked($check.Handle) -eq 1) -eq $contextMenuInitial) "$tag changed the fresh PDF context-menu default"
     if(([MsiDesktop]::Checked($check.Handle) -eq 1) -ne $contextMenu){[MsiDesktop]::Click($check.Handle)}
    }
    if(Button $c 'Next'){Click $c 'Next'}else{Click $c 'Install'};break
   }
   if($mode -eq 'fail' -and (Button $c 'OK')){Click $c 'OK';break}
  }
  if($done){break};Start-Sleep -Milliseconds 500
 }
 Assert $done "$tag timed out waiting for completion"
 Assert ($p.WaitForExit(120000)) "$tag did not exit after Finish"
 $expected=switch($mode){'cancel'{1602};'fail'{1603};default{0}}
 Assert ($p.ExitCode -eq $expected) "$tag exit code $($p.ExitCode), expected $expected"
 "ok $tag completion and Finish exit $($p.ExitCode)"
}
function Assert-CleanFixtures($architecture) {
 foreach ($name in 'cadcraft','deckcraft','designcraft','effectcraft','figcraft','filmcraft','gridcraft','lightcraft','pdfcraft','photocraft','soundcraft','vectorcraft','wordcraft','craft-launcher') {
  $db = $engine.OpenDatabase((Join-Path $Packages "$name-$architecture.msi"), 0)
  Assert ((Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductVersion''') -eq '90.0.0') "$name must be a generated test fixture"
  $upgrade = Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''UpgradeCode'''
  foreach ($existing in $engine.RelatedProducts($upgrade)) { throw "$name has an existing installation ($existing); use a clean disposable machine." }
 }
}
if($DefineOnly){return}
Assert-CleanFixtures x64
$apps='cadcraft','deckcraft','designcraft','effectcraft','figcraft','filmcraft','gridcraft','lightcraft','pdfcraft','photocraft','soundcraft','vectorcraft','wordcraft','craft-launcher'
$folders='effectcraft','figcraft','lightcraft','photocraft','vectorcraft','craft-launcher'
$desktopApps='cadcraft','effectcraft','gridcraft','pdfcraft','vectorcraft','craft-launcher'
$first=0
if($StartAt){$first=[Array]::IndexOf($apps,$StartAt);Assert ($first -ge 0) "Unknown start profile $StartAt"}
foreach($app in $apps[$first..($apps.Count-1)]) {
 $msi=Join-Path $Packages "$app-x64.msi"
 $db=$engine.OpenDatabase($msi,0);$code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
 $product=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductName'''
 $scope=if($app -eq 'craft-launcher'){'HKCU:'}else{'HKLM:'}
 $exe=if($app -eq 'craft-launcher'){'artcraft-launcher.exe'}else{"$app.exe"}
 $pathKey="$scope\Software\Microsoft\Windows\CurrentVersion\App Paths\$exe"
 $start=Join-Path ([Environment]::GetFolderPath($(if($app -eq 'craft-launcher'){'Programs'}else{'CommonPrograms'}))) "$product.lnk"
 $desktopLink=Join-Path ([Environment]::GetFolderPath($(if($app -eq 'craft-launcher'){'DesktopDirectory'}else{'CommonDesktopDirectory'}))) "$product.lnk"
 $folder=if($folders -contains $app){"C:\CraftInstallerTest\Custom folder $([char]0xe9) $app"}else{''}
 $desktop=if($desktopApps -contains $app){$false}else{$null}
 $p=Start-Msi $msi '/i' "$app-install";Complete $p 'install' "$app-install" $folder $desktop
 Assert ($engine.ProductState($code) -eq 5) "$app not registered installed"
 $installedExe=(Get-Item $pathKey).GetValue('')
 Assert (Test-Path -LiteralPath $installedExe) "$app executable missing"
 if($folder){Assert ((Split-Path $installedExe).TrimEnd('\') -eq $folder.TrimEnd('\')) "$app ignored folder selection"}
 Assert (Test-Path -LiteralPath $start) "$app Start menu shortcut missing"
 if($desktopApps -contains $app){Assert (-not (Test-Path -LiteralPath $desktopLink)) "$app ignored desktop opt-out"}
 Remove-Item -LiteralPath $installedExe
 $p=Start-Msi $msi '/i' "$app-repair";Complete $p 'repair' "$app-repair"
 Assert (Test-Path -LiteralPath $installedExe) "$app repair did not restore the executable"
 if($desktopApps -contains $app){Assert (-not (Test-Path -LiteralPath $desktopLink)) "$app repair changed desktop opt-out"}
 $sentinel=Join-Path $env:LOCALAPPDATA "CraftInstallerTest\$app-user-document.txt";New-Item -ItemType Directory -Force (Split-Path $sentinel)|Out-Null;Set-Content $sentinel 'keep me'
 $p=Start-Msi $code '/x' "$app-uninstall";Complete $p 'remove' "$app-uninstall"
 Assert ($engine.ProductState($code) -eq -1) "$app still registered installed"
 Assert (-not (Test-Path -LiteralPath $installedExe)) "$app executable survived removal"
 Assert (-not (Test-Path -LiteralPath $start)) "$app Start menu shortcut survived removal"
 Assert (-not (Test-Path -LiteralPath $desktopLink)) "$app desktop shortcut survived removal"
 Assert ((Get-Content $sentinel) -eq 'keep me') "$app removed user document"
}
$msi=Join-Path $Packages 'figcraft-x64.msi'
$p=Start-Msi $msi '/i' 'figcraft-cancel';Complete $p 'cancel' 'figcraft-cancel'
$p=Start-Msi $msi '/i' 'figcraft-maintenance-first' '/qn';Assert ($p.WaitForExit(180000) -and $p.ExitCode -eq 0) 'FigCraft maintenance fixture install failed'
$p=Start-Msi $msi '/i' 'figcraft-maintenance-remove';Complete $p 'remove' 'figcraft-maintenance-remove'
$db=$engine.OpenDatabase($msi,0);$code=Value $db 'SELECT `Value` FROM `Property` WHERE `Property` = ''ProductCode'''
Assert ($engine.ProductState($code) -eq -1) 'Maintenance Remove left FigCraft installed'
foreach($level in '/qn','/qb','/passive') {
 $tag='figcraft-'+($level -replace '/','')
 $p=Start-Msi $msi '/i' "$tag-install" $level;Assert ($p.WaitForExit(180000)) "$tag install hung";Assert ($p.ExitCode -eq 0) "$tag install failed $($p.ExitCode)"
 $p=Start-Msi $msi '/x' "$tag-uninstall" $level;Assert ($p.WaitForExit(180000)) "$tag uninstall hung";Assert ($p.ExitCode -eq 0) "$tag uninstall failed $($p.ExitCode)"
 "ok $tag noninteractive install/uninstall"
}
