<#
.SYNOPSIS
Runs the end-of-milestone RAM check against the release lumen.exe and prints a summary table.

.DESCRIPTION
Launches the release app fresh -Runs times (default 3) and drives it over Windows UI Automation. Each
launch measures, in this order: idle, the 5,000-row playlist, one search and one resolve (sampled peak),
playing, a sampled steady peak, CPU share while minimized, and idle after the track ended. With -Login it
also does one launch per run that opens the login window (never signs in) and closes it again.
Memory is the summed private working set from scripts/measure-ram.ps1. The table has the 3-run medians.

Needs an interactive desktop: keep the window visible and untouched, at 100% display scaling. Searches
and resolves are paced: one resolve and at most two Enter presses for the one search per launch, 20 s between launches, no retry loops.
Close lumen.exe first; the script refuses to start while one is running.

.PARAMETER Runs
Number of fresh launches, 1 to 3. Default 3.

.PARAMETER ExePath
Release exe. Default: src-tauri/target/release/lumen.exe under the repo root.

.PARAMETER OutDir
Where per-launch logs and summary.md / summary.json go. Default: src-tauri/target/ram-run/<timestamp>.

.PARAMETER Login
Also measure the login window (one extra launch per run).

.PARAMETER Query
Search text. Default 'lofi hip hop'.

.EXAMPLE
pwsh -File scripts/ram-run.ps1
#>
[CmdletBinding()]
param(
    [int]$Runs = 3,
    [string]$ExePath,
    [string]$OutDir,
    [switch]$Login,
    [string]$Query = 'lofi hip hop'
)

function Get-Median([double[]]$Values) {
    if (-not $Values -or $Values.Count -eq 0) { throw 'no values for a median' }
    $s = @($Values | Sort-Object)
    $n = $s.Count
    if ($n % 2 -eq 1) { return $s[($n - 1) / 2] }
    ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

function Get-Verdict([double]$Value, [double]$BudgetMB) {
    if ($BudgetMB -le 0) { return 'n/a' }
    if ($Value -le $BudgetMB) { 'pass' } else { 'fail' }
}

function New-SummaryRow([string]$Scenario, [double[]]$Values, [double]$BudgetMB, [string]$Unit = 'MB') {
    $median = [math]::Round((Get-Median $Values), 1)
    [pscustomobject]@{
        Scenario = $Scenario
        Values   = @($Values)
        Median   = $median
        Budget   = $BudgetMB
        Headroom = if ($BudgetMB -gt 0) { [math]::Round($BudgetMB - $median, 1) } else { $null }
        Verdict  = Get-Verdict $median $BudgetMB
        Unit     = $Unit
    }
}

function Format-SummaryTable([object[]]$Rows, [int]$Runs = 3) {
    $inv = [cultureinfo]::InvariantCulture
    $num = { param($v, $unit) [string]::Format($inv, '{0:0.0} {1}', $v, $unit) }
    $head = @('Scenario') + @(1..$Runs | ForEach-Object { "Run $_" }) + @('Median', 'Budget', 'Headroom', 'Verdict')
    '| ' + ($head -join ' | ') + ' |'
    '|' + (($head | ForEach-Object { '---' }) -join '|') + '|'
    foreach ($r in $Rows) {
        $cells = @($r.Scenario)
        foreach ($i in 0..($Runs - 1)) {
            $cells += if ($i -lt @($r.Values).Count) { & $num @($r.Values)[$i] $r.Unit } else { '-' }
        }
        $cells += & $num $r.Median $r.Unit
        $cells += if ($r.Budget -gt 0) { [string]::Format($inv, '{0:0.##} {1}', $r.Budget, $r.Unit) } else { '-' }
        $cells += if ($null -ne $r.Headroom) { & $num $r.Headroom $r.Unit } else { '-' }
        $cells += $r.Verdict
        '| ' + ($cells -join ' | ') + ' |'
    }
}

function Resolve-RamRunOptions([string]$RepoRoot, [int]$Runs = 3, [string]$ExePath, [string]$OutDir) {
    if ($Runs -lt 1 -or $Runs -gt 3) { throw "Runs must be 1 to 3, got $Runs" }
    if (-not $ExePath) { $ExePath = Join-Path $RepoRoot 'src-tauri/target/release/lumen.exe' }
    if (-not (Test-Path -LiteralPath $ExePath)) { throw "release exe not found: $ExePath. Build it with: pnpm tauri build --no-bundle" }
    if (-not $OutDir) { $OutDir = Join-Path $RepoRoot ('src-tauri/target/ram-run/' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
    [pscustomobject]@{ RepoRoot = $RepoRoot; Runs = $Runs; ExePath = (Resolve-Path -LiteralPath $ExePath).Path; OutDir = $OutDir }
}

# Picks the track to play from the "Play <title> by <artist>" button names: a long mix (so steady playback,
# the CPU reading and the seek fit inside it), varying by run; else the first ordinary song.
function Select-Track([string[]]$Names, [int]$Run) {
    $names = @($Names | Where-Object { $_ -match '^Play .+ by ' })
    if ($names.Count -eq 0) { throw 'no results to play' }
    $long = @($names | Where-Object { $_ -match '(?i)hour' })
    if ($long.Count) { return $long[($Run - 1) % $long.Count] }
    $short = @($names | Where-Object { $_ -notmatch '(?i)loop|mix|session' })
    if ($short.Count) { $short[0] } else { $names[0] }
}

if ($MyInvocation.InvocationName -eq '.') { return }

# ---- Driver (needs a desktop, UI Automation and the release build) ----
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$opts = Resolve-RamRunOptions -RepoRoot $repoRoot -Runs $Runs -ExePath $ExePath -OutDir $OutDir
. (Join-Path $PSScriptRoot 'ram-run-ui.ps1')
Add-Type -Namespace N -Name F -MemberDefinition '[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid); [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach); [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h); [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();'
New-Item -ItemType Directory -Force $opts.OutDir | Out-Null

function Write-Log([string]$Dir, [string]$Message) {
    $line = "$(Get-Date -f HH:mm:ss) $Message"
    Add-Content (Join-Path $Dir 'log.txt') $line
    Write-Host $line
}

# One measured launch. Returns an ordered map of scenario -> value.
function Invoke-Launch([int]$Run, [string]$Dir) {
    function Log($m) { Write-Log $Dir $m }
    function Nav($pattern) { Invoke-Element (Find-Element $app $pattern 'Hyperlink') }
    function Position { (Get-Range (Get-Slider $app '^Seek$')).Current.Value }
    function Read-Sample($file) { Get-Content $file -Raw | ConvertFrom-Json }
    $m = [ordered]@{}
    # --list opens a throwaway library with the 5,000-song playlist, so the user's own library is never touched.
    $app = Start-App $opts.ExePath @('--list')
    try {
        Log "launched run $Run"
        Nav '^Home$'
        $m['Idle, home screen'] = (Measure-Once $app 30).TotalMB
        Nav '^The long haul'
        $m['5,000-row playlist open'] = (Measure-Once $app 30).TotalMB

        (Get-Range (Get-Slider $app '^Volume$')).SetValue(0)
        Nav '^Search$'; Start-Sleep 1
        $box = Find-Element $app 'Search for songs' 'Edit'
        $box.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Query)
        # Posted keys reach the web view only while its window has the focus. Windows may refuse
        # SetForegroundWindow, so share the foreground thread's input queue. Only the focus is retried;
        # the search Enter is sent at most twice.
        $prev = [N.F]::GetForegroundWindow()
        $w = [IntPtr][long](([Win]::Children($app.Hwnd) | Where-Object { $_ -match 'Chrome_RenderWidgetHostHWND' } | Select-Object -First 1) -split '\|')[0]
        $focused = $false
        foreach ($try in 1..10) {
            $fg = [N.F]::GetWindowThreadProcessId([N.F]::GetForegroundWindow(), [IntPtr]::Zero)
            $me = [N.F]::GetCurrentThreadId()
            [void][N.F]::AttachThreadInput($me, $fg, $true)
            [void][N.F]::BringWindowToTop($app.Hwnd); [void][N.F]::SetForegroundWindow($app.Hwnd)
            [void][N.F]::AttachThreadInput($me, $fg, $false)
            Start-Sleep -Milliseconds 400
            $focused = [N.F]::GetForegroundWindow() -eq $app.Hwnd
            if ($focused) { break }
            Start-Sleep 3
        }
        if (-not $focused) { throw 'could not bring the window to the foreground' }
        # A posted Enter is sometimes lost, so it is sent at most twice, 10 s apart, and only if no result showed.
        $found = $false
        foreach ($send in 1..2) {
            [void][N.F]::SetForegroundWindow($app.Hwnd); Start-Sleep -Milliseconds 400
            $box.SetFocus(); Start-Sleep -Milliseconds 400
            [void][Win]::PostMessage($w, 0x100, [IntPtr]13, [IntPtr]0x001C0001)
            [void][Win]::PostMessage($w, 0x102, [IntPtr]13, [IntPtr]0x001C0001)
            [void][Win]::PostMessage($w, 0x101, [IntPtr]13, [IntPtr]0xC01C0001)
            Start-Sleep 1
            try { $null = Find-Element $app '^Play .+ by ' 'Button' 10; $found = $true; break } catch { Log "no results after send $send" }
        }
        [void][N.F]::SetForegroundWindow($prev)
        if (-not $found) { throw 'search was not sent' }
        Start-Sleep 3
        $names = @(Find-All $app 'Button' | ForEach-Object { $_.Current.Name })
        $song = Select-Track $names -Run $Run
        Log "$(@($names | Where-Object { $_ -match '^Play .+ by ' }).Count) results, picked one"

        # One resolve per launch.
        $button = Find-Element $app "^$([regex]::Escape($song))$" 'Button'
        $sample = Join-Path $Dir 'resolve.sample.json'
        $sampler = Start-Sampler $app 35 $sample
        Start-Sleep 4
        $watch = [Diagnostics.Stopwatch]::StartNew()
        Invoke-Element $button
        Start-Sleep -Milliseconds 500
        $seen = $false
        while ($watch.Elapsed.TotalSeconds -lt 25) {
            if ((Position) -gt 0) { $seen = $true; break }
            Start-Sleep -Milliseconds 300
        }
        if (-not $seen) { throw 'the track did not start playing' }
        Log "playing after $([math]::Round($watch.Elapsed.TotalSeconds, 1)) s (includes UIA polling)"
        $sampler.WaitForExit()
        $m['Search + resolve, sampled peak'] = (Read-Sample $sample).PeakMB
        Start-Sleep 5

        Nav '^The long haul'
        $p0 = Position
        $m['Playing, 5,000-row playlist open'] = (Measure-Once $app 30).TotalMB
        $sample = Join-Path $Dir 'steady.sample.json'
        $sampler = Start-Sampler $app 30 $sample; $sampler.WaitForExit()
        $m['Playing, sampled peak over 30 s'] = (Read-Sample $sample).PeakMB
        if ((Position) -le $p0) { throw 'playback was not advancing' }

        [void][Win]::ShowWindow($app.Hwnd, 6); Start-Sleep 10
        if (-not [Win]::IsIconic($app.Hwnd)) { throw 'window is not minimized' }
        function CpuSeconds {
            $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, CreationDate, KernelModeTime, UserModeTime)
            . $script:Measure
            $sum = 0.0
            foreach ($p in Get-ProcessTree -Processes $all -RootId $app.Id) { $sum += ($p.KernelModeTime + $p.UserModeTime) / 1e7 }
            $sum
        }
        $a = CpuSeconds; $clock = [Diagnostics.Stopwatch]::StartNew(); Start-Sleep 60
        $b = CpuSeconds; $el = $clock.Elapsed.TotalSeconds
        $m['CPU while minimized and playing'] = [math]::Round(($b - $a) / $el / [Environment]::ProcessorCount * 100, 3)
        [void][Win]::ShowWindow($app.Hwnd, 9); Start-Sleep 3

        $seek = Get-Range (Get-Slider $app '^Seek$')
        $max = $seek.Current.Maximum
        $seek.SetValue($max - ($max / 100))
        $deadline = (Get-Date).AddSeconds(200)
        while ($true) {
            if (@(Find-All $app 'Button' | ForEach-Object { $_.Current.Name }) -ccontains 'Play') { break }
            if ((Get-Date) -gt $deadline) { throw 'the track did not end' }
            Start-Sleep 2
        }
        Nav '^Home$'
        $m['Idle after playback'] = (Measure-Once $app 30).TotalMB
    } finally {
        Stop-App $app
        Log "closed; leftover processes: $(@(Get-Process lumen, yt-dlp*, deno* -ErrorAction SilentlyContinue).Count)"
    }
    $m
}

# Opens the login window and closes it again. Never signs in or disconnects.
function Invoke-LoginLaunch([int]$Run, [string]$Dir) {
    function Log($m) { Write-Log $Dir $m }
    $title = 'Sign in to YouTube Music'
    $app = Start-App $opts.ExePath
    try {
        Start-Sleep 30
        $button = Find-Element $app '^Sign in again$' 'Button'
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $sample = Join-Path $Dir 'login.sample.json'
        $sampler = Start-Sampler $app 170 $sample
        Start-Sleep 8
        $clicked = $watch.ElapsedMilliseconds
        Invoke-Element $button
        $deadline = (Get-Date).AddSeconds(15)
        while (-not ($h = Get-Hwnd $app.Id 'Tauri Window' $title)) { if ((Get-Date) -gt $deadline) { throw 'no login window' }; Start-Sleep -Milliseconds 200 }
        Start-Sleep 30
        $closed = $watch.ElapsedMilliseconds
        [void][Win]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep 2
        $sampler.WaitForExit()
        $s = Get-Content $sample -Raw | ConvertFrom-Json
        $base = ($s.Timeline | Select-Object -First 1).ProcessCount
        $up = $s.Timeline | Where-Object { $_.ProcessCount -gt $base } | Select-Object -First 1
        $closeAt = $closed + $(if ($up) { $up.ElapsedMs - $clicked } else { 0 })
        $after = @($s.Timeline | Where-Object { $_.ElapsedMs -ge $closeAt })
        $at60 = $after | Where-Object { $_.ElapsedMs -ge $closeAt + 60000 } | Select-Object -First 1
        Log "login window closed, peak $($s.PeakMB) MB"
        [ordered]@{
            'Login window open, peak'    = $s.PeakMB
            'Login window, 60 s after close' = $(if ($at60) { $at60.TotalMB } else { $after[-1].TotalMB })
        }
    } finally {
        Stop-App $app
        Log "closed; leftover processes: $(@(Get-Process lumen, yt-dlp*, deno* -ErrorAction SilentlyContinue).Count)"
    }
}

$budgets = @{
    'Idle, home screen' = 200; '5,000-row playlist open' = 200; 'Search + resolve, sampled peak' = 0
    'Playing, 5,000-row playlist open' = 200; 'Playing, sampled peak over 30 s' = 200
    'Idle after playback' = 200; 'Login window open, peak' = 0; 'Login window, 60 s after close' = 200
}
$collected = [ordered]@{}
function Add-Metrics($metrics) { foreach ($k in $metrics.Keys) { if (-not $collected.Contains($k)) { $collected[$k] = @() }; $collected[$k] += [double]$metrics[$k] } }

$failedRun = 0
foreach ($run in 1..$opts.Runs) {
    $dir = Join-Path $opts.OutDir "run$run"
    New-Item -ItemType Directory -Force $dir | Out-Null
    try {
        Add-Metrics (Invoke-Launch $run $dir)
        if ($Login) { Start-Sleep 20; Add-Metrics (Invoke-LoginLaunch $run (Join-Path $opts.OutDir "login$run")) }
    } catch {
        # Keep what the finished launches measured; a failed launch is not retried.
        Write-Warning "run $run failed: $($_.Exception.Message)"
        $failedRun = $run
        break
    }
    if ($run -lt $opts.Runs) { Start-Sleep 20 }
}

$rows = foreach ($k in $collected.Keys) {
    if ($k -like 'CPU*') { New-SummaryRow $k $collected[$k] 2 '%' }
    else { New-SummaryRow $k $collected[$k] $(if ($budgets.ContainsKey($k)) { $budgets[$k] } else { 0 }) }
}
$table = Format-SummaryTable @($rows) -Runs $opts.Runs
$table | Set-Content (Join-Path $opts.OutDir 'summary.md')
$rows | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $opts.OutDir 'summary.json')
$table
Write-Host "Results in $($opts.OutDir)"
if ($failedRun) { Write-Warning "run $failedRun failed; the table has the earlier runs only"; exit 1 }
