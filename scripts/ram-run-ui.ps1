# UIA helpers for driving the release app without a DevTools port. Dot-sourced by ram-run.ps1.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Win {
  public delegate bool Proc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc p, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, Proc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  static string Cls(IntPtr h){ var s=new StringBuilder(256); GetClassName(h,s,256); return s.ToString(); }
  static string Txt(IntPtr h){ var s=new StringBuilder(256); GetWindowText(h,s,256); return s.ToString(); }
  public static List<string> Top(uint pid){ var r=new List<string>(); EnumWindows((h,l)=>{uint p; GetWindowThreadProcessId(h,out p); if(p==pid) r.Add(h.ToInt64()+"|"+Cls(h)+"|"+IsWindowVisible(h)+"|"+Txt(h)); return true;},IntPtr.Zero); return r; }
  public static List<string> Children(IntPtr parent){ var r=new List<string>(); EnumChildWindows(parent,(h,l)=>{r.Add(h.ToInt64()+"|"+Cls(h)); return true;},IntPtr.Zero); return r; }
}
'@
$script:AE = [System.Windows.Automation.AutomationElement]
$script:Measure = Join-Path $PSScriptRoot 'measure-ram.ps1'

function Get-Hwnd([int]$ProcessId, [string]$Class = 'Tauri Window', [string]$Title) {
    foreach ($w in [Win]::Top($ProcessId)) {
        $h, $c, $v, $t = $w -split '\|', 4
        if ($c -eq $Class -and $v -eq 'True' -and (-not $Title -or $t -eq $Title)) { return [IntPtr][long]$h }
    }
}
function Start-App([Parameter(Mandatory)][string]$Exe, [string[]]$Arguments = @()) {
    if (Get-Process lumen -ErrorAction SilentlyContinue) { throw 'lumen already running' }
    $p = if ($Arguments) { Start-Process $Exe -ArgumentList $Arguments -PassThru } else { Start-Process $Exe -PassThru }
    $deadline = (Get-Date).AddSeconds(15)
    while (-not ($h = Get-Hwnd $p.Id)) { if ((Get-Date) -gt $deadline) { throw 'no window' }; Start-Sleep -Milliseconds 200 }
    [pscustomobject]@{ Id = $p.Id; Hwnd = $h; Process = $p }
}
function Stop-App($App) {
    $ids = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($App.Id)" | ForEach-Object ProcessId)
    [void][Win]::PostMessage($App.Hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
    $deadline = (Get-Date).AddSeconds(20)
    while (Get-Process -Id (@($App.Id) + $ids) -ErrorAction SilentlyContinue) {
        if ((Get-Date) -gt $deadline) { Stop-Process -Id $App.Id -Force -Confirm:$false; throw 'app did not exit' }
        Start-Sleep -Milliseconds 250
    }
}
function Find-All($App, [string]$TypeName) {
    $root = $script:AE::FromHandle($App.Hwnd)
    $cond = [System.Windows.Automation.Condition]::TrueCondition
    if ($TypeName) {
        $type = [System.Windows.Automation.ControlType]::$TypeName
        $cond = [System.Windows.Automation.PropertyCondition]::new($script:AE::ControlTypeProperty, $type)
    }
    $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
}
function Find-Element($App, [string]$Pattern, [string]$TypeName, [int]$TimeoutSeconds = 15) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        foreach ($e in (Find-All $App $TypeName)) { if ($e.Current.Name -match $Pattern) { return $e } }
        if ((Get-Date) -gt $deadline) { throw "no element '$Pattern' $TypeName" }
        Start-Sleep -Milliseconds 500
    }
}
function Invoke-Element($Element) { $Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
function Get-Slider($App, [string]$Pattern) { Find-Element $App $Pattern 'Slider' }
function Get-Range($Element) { $Element.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern) }
function Show-Tree($App) {
    foreach ($e in (Find-All $App)) {
        $n = $e.Current.Name
        $t = $e.Current.ControlType.ProgrammaticName -replace 'ControlType.', ''
        if ($n -and $t -in 'Button', 'Hyperlink', 'Edit', 'Slider', 'RadioButton') { "$t | $n" }
    }
}
function Measure-Once($App, [int]$Wait = 30) {
    (& pwsh -NoProfile -File $script:Measure -ProcessId $App.Id -WaitSeconds $Wait -AsJson) | ConvertFrom-Json
}
function Start-Sampler($App, [int]$Seconds, [string]$Out) {
    Start-Process pwsh -NoNewWindow -PassThru -RedirectStandardOutput $Out -RedirectStandardError "$Out.err" -ArgumentList @(
        '-NoProfile', '-File', "`"$script:Measure`"", '-ProcessId', $App.Id, '-WaitSeconds', 0, '-SampleSeconds', $Seconds, '-AsJson')
}
