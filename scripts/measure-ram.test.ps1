# Tests the pure process-tree and summing logic of measure-ram.ps1 with mocked process lists.
# Run: pwsh -NoProfile -File scripts/measure-ram.test.ps1
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

. (Join-Path $PSScriptRoot 'measure-ram.ps1')

$script:failed = 0
$script:passed = 0

function Test-Case([string]$Name, [scriptblock]$Body) {
    try {
        & $Body
        $script:passed++
        Write-Host "ok   $Name"
    } catch {
        $script:failed++
        Write-Host "FAIL $Name"
        Write-Host "     $($_.Exception.Message)"
    }
}

function Assert-Equal($Expected, $Actual, [string]$What) {
    # [string] formats numbers the same way in every locale.
    $e = @($Expected | ForEach-Object { [string]$_ }) -join ','
    $a = @($Actual | ForEach-Object { [string]$_ }) -join ','
    if ($e -cne $a) { throw "$What - expected '$e', got '$a'" }
}

function Assert-Throws([scriptblock]$Body, [string]$Pattern) {
    try { & $Body | Out-Null } catch {
        if ($_.Exception.Message -notmatch $Pattern) { throw "wrong error: $($_.Exception.Message)" }
        return
    }
    throw "expected an error matching '$Pattern'"
}

$t0 = Get-Date '2026-01-01T12:00:00'

function New-Proc([int]$Id, [int]$ParentId, [string]$Name, [int]$Second = 0, [string]$CommandLine = '') {
    [pscustomobject]@{
        ProcessId       = $Id
        ParentProcessId = $ParentId
        Name            = $Name
        CreationDate    = $t0.AddSeconds($Second)
        CommandLine     = $CommandLine
    }
}

function New-Perf([int]$Id, [long]$Bytes) {
    [pscustomobject]@{ IDProcess = $Id; WorkingSetPrivate = $Bytes }
}

# explorer -> lumen -> webview2 browser -> renderer + gpu, plus yt-dlp -> deno.
# Teams and Edge run their own, unrelated webview2 and msedge processes.
$processes = @(
    New-Proc 10 1 'explorer.exe' 0
    New-Proc 100 10 'lumen.exe' 10
    New-Proc 101 100 'msedgewebview2.exe' 11 '"msedgewebview2.exe" --embedded-browser-webview=1'
    New-Proc 102 101 'msedgewebview2.exe' 12 'msedgewebview2.exe --type=renderer --lang=en'
    New-Proc 103 101 'msedgewebview2.exe' 12 'msedgewebview2.exe --type=gpu-process'
    New-Proc 104 100 'yt-dlp.exe' 20
    New-Proc 105 104 'deno.exe' 21
    New-Proc 200 10 'ms-teams.exe' 5
    New-Proc 201 200 'msedgewebview2.exe' 6
    New-Proc 202 201 'msedgewebview2.exe' 7 'msedgewebview2.exe --type=renderer'
    New-Proc 300 10 'msedge.exe' 5
)

Test-Case 'tree holds the root, every child and every grandchild' {
    $tree = @(Get-ProcessTree -Processes $processes -RootId 100)
    Assert-Equal @(100, 101, 102, 103, 104, 105) @($tree.ProcessId | Sort-Object) 'tree ids'
}

Test-Case 'unrelated msedgewebview2 and msedge processes are excluded' {
    $ids = @(Get-ProcessTree -Processes $processes -RootId 100).ProcessId
    foreach ($other in 10, 200, 201, 202, 300) {
        if ($ids -contains $other) { throw "unrelated process $other is in the tree" }
    }
}

Test-Case 'a process older than its listed parent is a reused PID, not a child' {
    $reused = @($processes) + (New-Proc 400 100 'msedgewebview2.exe' 2) + (New-Proc 401 400 'msedgewebview2.exe' 3)
    $ids = @(Get-ProcessTree -Processes $reused -RootId 100).ProcessId
    if ($ids -contains 400 -or $ids -contains 401) { throw 'stale parent id was followed' }
}

Test-Case 'a parent loop does not hang or duplicate processes' {
    $loop = @((New-Proc 1 2 'a.exe' 0), (New-Proc 2 1 'b.exe' 0))
    $tree = @(Get-ProcessTree -Processes $loop -RootId 1)
    Assert-Equal @(1, 2) @($tree.ProcessId | Sort-Object) 'loop ids'
}

Test-Case 'an unknown root id gives an empty tree' {
    Assert-Equal 0 @(Get-ProcessTree -Processes $processes -RootId 999).Count 'count'
}

Test-Case 'root is found by name, with or without .exe, ignoring case' {
    foreach ($name in 'lumen', 'lumen.exe', 'LUMEN') {
        Assert-Equal 100 (Find-RootProcess -Processes $processes -ProcessName $name).ProcessId $name
    }
}

Test-Case 'root is found by id' {
    Assert-Equal 'lumen.exe' (Find-RootProcess -Processes $processes -ProcessId 100).Name 'name'
}

Test-Case 'a child with the same name as its parent is not a second root' {
    $self = @($processes) + (New-Proc 106 100 'lumen.exe' 30)
    Assert-Equal 100 (Find-RootProcess -Processes $self -ProcessName 'lumen').ProcessId 'root'
}

Test-Case 'a missing target is an error' {
    Assert-Throws { Find-RootProcess -Processes $processes -ProcessName 'nothing' } 'No process'
    Assert-Throws { Find-RootProcess -Processes $processes -ProcessId 999 } 'No process'
}

Test-Case 'two separate instances are an error instead of a silent mixed sum' {
    $two = @($processes) + (New-Proc 500 10 'lumen.exe' 40)
    Assert-Throws { Find-RootProcess -Processes $two -ProcessName 'lumen' } '100, 500'
}

$perf = @(
    New-Perf 100 10485760   # 10 MB
    New-Perf 101 20971520   # 20 MB
    New-Perf 102 31457280   # 30 MB
    New-Perf 103 5242880    #  5 MB
    New-Perf 104 1048576    #  1 MB
    New-Perf 105 524288     #  0.5 MB
    New-Perf 201 104857600  # unrelated
    New-Perf 202 104857600  # unrelated
    New-Perf 0 999999999    # Idle / _Total
)

Test-Case 'total sums the tree only' {
    $tree = @(Get-ProcessTree -Processes $processes -RootId 100)
    $report = Get-RamReport -Tree $tree -PerfRecords $perf
    Assert-Equal 69730304 $report.TotalBytes 'bytes'
    Assert-Equal 66.5 $report.TotalMB 'MB'
    Assert-Equal 6 @($report.Processes).Count 'rows'
}

Test-Case 'rows carry name, id, role and MB, largest first' {
    $tree = @(Get-ProcessTree -Processes $processes -RootId 100)
    $first = @((Get-RamReport -Tree $tree -PerfRecords $perf).Processes)[0]
    Assert-Equal 'msedgewebview2.exe' $first.Name 'name'
    Assert-Equal 102 $first.PID 'pid'
    Assert-Equal 'renderer' $first.Role 'role'
    Assert-Equal 30 $first.PrivateWS_MB 'MB'
}

Test-Case 'total is rounded once, from bytes' {
    $tree = @((New-Proc 1 0 'a.exe'), (New-Proc 2 1 'b.exe'))
    $report = Get-RamReport -Tree $tree -PerfRecords @((New-Perf 1 1090000), (New-Perf 2 1090000))
    Assert-Equal 2.1 $report.TotalMB 'MB'
}

Test-Case 'a process without a counter is reported, not silently counted as zero' {
    $tree = @(Get-ProcessTree -Processes $processes -RootId 100)
    $report = Get-RamReport -Tree $tree -PerfRecords @($perf | Where-Object IDProcess -ne 103)
    Assert-Equal 103 $report.MissingIds 'missing'
    Assert-Equal 5 @($report.Processes).Count 'rows'
}

Test-Case 'webview2 roles come from the command line' {
    Assert-Equal 'browser' (Get-ProcessRole -Name 'msedgewebview2.exe' -CommandLine 'x --embedded-browser-webview=1') 'browser'
    Assert-Equal 'gpu-process' (Get-ProcessRole -Name 'msedgewebview2.exe' -CommandLine 'x --type=gpu-process') 'gpu'
    Assert-Equal 'utility (network.mojom.NetworkService)' (Get-ProcessRole -Name 'msedgewebview2.exe' -CommandLine 'x --type=utility --utility-sub-type=network.mojom.NetworkService') 'utility'
    Assert-Equal '' (Get-ProcessRole -Name 'lumen.exe' -CommandLine 'lumen.exe --list') 'app'
}

Test-Case '-AsJson keeps stdout pure JSON when a counter is missing' {
    # Runs the script in a child shell, so stdout and stderr are the real streams.
    # The fake Get-CimInstance lists two processes and a counter for only one of them.
    $harness = @'
param([string]$Script)
function Get-CimInstance {
    param([Parameter(Position = 0)] [string]$ClassName, [string[]]$Property)
    if ($ClassName -eq 'Win32_Process') {
        [pscustomobject]@{ ProcessId = 100; ParentProcessId = 1; Name = 'lumen.exe'; CreationDate = $null; CommandLine = '' }
        [pscustomobject]@{ ProcessId = 101; ParentProcessId = 100; Name = 'yt-dlp.exe'; CreationDate = $null; CommandLine = '' }
    } else {
        [pscustomobject]@{ IDProcess = 100; WorkingSetPrivate = 10485760 }
    }
}
& $Script -ProcessId 100 -WaitSeconds 0 -AsJson
'@
    $dir = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $dir | Out-Null
    try {
        $harnessPath = Join-Path $dir 'harness.ps1'
        $outPath = Join-Path $dir 'stdout.txt'
        $errPath = Join-Path $dir 'stderr.txt'
        Set-Content -LiteralPath $harnessPath -Value $harness
        $exe = if ($PSVersionTable.PSVersion.Major -ge 6) { 'pwsh.exe' } else { 'powershell.exe' }
        $script = Join-Path $PSScriptRoot 'measure-ram.ps1'
        $child = Start-Process -FilePath (Join-Path $PSHOME $exe) -NoNewWindow -Wait -PassThru `
            -ArgumentList '-NoProfile', '-File', "`"$harnessPath`"", "`"$script`"" `
            -RedirectStandardOutput $outPath -RedirectStandardError $errPath
        $stdout = [string](Get-Content -LiteralPath $outPath -Raw)
        $stderr = [string](Get-Content -LiteralPath $errPath -Raw)

        Assert-Equal 0 $child.ExitCode "exit code (stderr: $stderr)"
        try { $json = $stdout | ConvertFrom-Json } catch { throw "stdout is not pure JSON: $stdout" }
        Assert-Equal 101 $json.MissingIds 'MissingIds'
        Assert-Equal 10 $json.TotalMB 'TotalMB'
        if ($stderr -notmatch 'No counter for process id\(s\) 101') { throw "no warning on stderr: '$stderr'" }
    } finally {
        Remove-Item -LiteralPath $dir -Recurse -Force
    }
}

# --- Sampling mode ---

# A fake machine: each snapshot is a process list and the counters that go with it.
function New-Snapshot([object[]]$Processes, [hashtable]$MB) {
    [pscustomobject]@{
        Processes   = $Processes
        PerfRecords = @($MB.GetEnumerator() | ForEach-Object { New-Perf $_.Key ([long]($_.Value * 1MB)) })
    }
}

$app = New-Proc 100 10 'lumen.exe' 10
$view = New-Proc 101 100 'msedgewebview2.exe' 11
$ytdlp = New-Proc 104 100 'yt-dlp.exe' 20
$deno = New-Proc 105 104 'deno.exe' 21
$stranger = New-Proc 201 200 'msedgewebview2.exe' 6

$quiet = New-Snapshot @($app, $view, $stranger) @{ 100 = 10; 101 = 80; 201 = 500 }
$resolving = New-Snapshot @($app, $view, $ytdlp) @{ 100 = 10; 101 = 80; 104 = 70 }
$peaking = New-Snapshot @($app, $view, $ytdlp, $deno) @{ 100 = 10; 101 = 80; 104 = 70; 105 = 120 }

# Runs the sampling loop on a fake clock. Each snapshot takes $CostMs; sleeping advances the clock.
function Invoke-FakeSampling([object[]]$Snapshots, [int]$DurationMs, [int]$IntervalMs = 250, [int]$CostMs = 100) {
    $state = @{ Now = 0; Next = 0; Slept = New-Object System.Collections.ArrayList }
    $samples = Invoke-Sampling -RootId 100 -DurationMs $DurationMs -IntervalMs $IntervalMs `
        -Snapshot {
            $snapshot = $Snapshots[[math]::Min($state.Next, $Snapshots.Count - 1)]
            $state.Next++
            $state.Now += $CostMs
            $snapshot
        }.GetNewClosure() `
        -Clock { $state.Now }.GetNewClosure() `
        -Sleep { param($Ms) [void]$state.Slept.Add($Ms); $state.Now += $Ms }.GetNewClosure()
    [pscustomobject]@{ Samples = @($samples); Slept = @($state.Slept) }
}

Test-Case 'sampling starts a snapshot every interval, less the time the snapshot took' {
    $run = Invoke-FakeSampling @($quiet) -DurationMs 1000
    Assert-Equal @(0, 250, 500, 750, 1000) $run.Samples.ElapsedMs 'sample times'
    Assert-Equal @(150, 150, 150, 150) $run.Slept 'sleeps'
}

Test-Case 'a snapshot slower than the interval is followed at once' {
    $run = Invoke-FakeSampling @($quiet) -DurationMs 800 -IntervalMs 250 -CostMs 400
    Assert-Equal @(0, 400, 800) $run.Samples.ElapsedMs 'sample times'
    Assert-Equal 0 $run.Slept.Count 'sleeps'
}

Test-Case 'sampling stops when the app is gone and keeps the samples it has' {
    $gone = New-Snapshot @($stranger) @{ 201 = 500 }
    $run = Invoke-FakeSampling @($quiet, $quiet, $gone, $quiet) -DurationMs 5000
    Assert-Equal 2 $run.Samples.Count 'samples'
}

Test-Case 'a child that outlives its parent stays counted' {
    $orphaned = New-Snapshot @($app, $view, $deno) @{ 100 = 10; 101 = 80; 105 = 120 }
    $run = Invoke-FakeSampling @($peaking, $orphaned) -DurationMs 250
    Assert-Equal 210 $run.Samples[1].Report.TotalMB 'orphaned deno is still in the total'
}

Test-Case 'a later owner of a tracked process id is not counted' {
    $reused = New-Proc 105 200 'other.exe' 40
    $after = New-Snapshot @($app, $view, $reused) @{ 100 = 10; 101 = 80; 105 = 900 }
    $run = Invoke-FakeSampling @($peaking, $after) -DurationMs 250
    Assert-Equal 90 $run.Samples[1].Report.TotalMB 'reused pid'
}

Test-Case 'summary reports the peak total of the whole tree and its breakdown' {
    $run = Invoke-FakeSampling @($quiet, $resolving, $peaking, $quiet) -DurationMs 750
    $summary = Get-SampleSummary -Samples $run.Samples -ThresholdMB 150
    Assert-Equal 280 $summary.PeakMB 'peak'
    Assert-Equal 500 $summary.PeakAtMs 'peak time'
    Assert-Equal @('deno.exe', 'msedgewebview2.exe', 'yt-dlp.exe', 'lumen.exe') $summary.PeakProcesses.Name 'breakdown, largest first'
    Assert-Equal @(120, 80, 70, 10) $summary.PeakProcesses.PrivateWS_MB 'breakdown MB'
}

Test-Case 'time above the threshold adds the gaps that follow the samples over it' {
    # 90, 160, 280, 90 MB at 0, 250, 500, 750 ms.
    $run = Invoke-FakeSampling @($quiet, $resolving, $peaking, $quiet) -DurationMs 750
    Assert-Equal 500 (Get-SampleSummary -Samples $run.Samples -ThresholdMB 150).MsAboveThreshold 'over 150'
    Assert-Equal 250 (Get-SampleSummary -Samples $run.Samples -ThresholdMB 160).MsAboveThreshold 'exactly 160 is not above'
}

Test-Case 'summary reports the achieved interval, mean and worst' {
    $samples = foreach ($ms in 0, 200, 500, 1200) {
        [pscustomobject]@{ ElapsedMs = $ms; Report = (Invoke-FakeSampling @($quiet) -DurationMs 0).Samples[0].Report }
    }
    $summary = Get-SampleSummary -Samples @($samples) -ThresholdMB 150
    Assert-Equal 4 $summary.SampleCount 'count'
    Assert-Equal 1200 $summary.DurationMs 'duration'
    Assert-Equal 400 $summary.MeanIntervalMs 'mean'
    Assert-Equal 700 $summary.MaxIntervalMs 'max'
}

Test-Case 'summary says when each process name was seen, and that peaks can be undercounted' {
    $run = Invoke-FakeSampling @($quiet, $resolving, $peaking, $quiet) -DurationMs 750
    $summary = Get-SampleSummary -Samples $run.Samples -ThresholdMB 150
    $seen = @($summary.Seen | Where-Object Name -eq 'yt-dlp.exe')[0]
    Assert-Equal @(250, 500, 2) @($seen.FirstMs, $seen.LastMs, $seen.Samples) 'yt-dlp'
    Assert-Equal 1 @($summary.Seen | Where-Object Name -eq 'deno.exe')[0].Samples 'deno'
    if ($summary.Note -notmatch 'undercount' -or $summary.Note -notmatch '250') { throw "note: $($summary.Note)" }
}

Test-Case 'summary keeps a timeline of totals' {
    $run = Invoke-FakeSampling @($quiet, $peaking) -DurationMs 250
    $timeline = @((Get-SampleSummary -Samples $run.Samples -ThresholdMB 150).Timeline)
    Assert-Equal @(0, 250) $timeline.ElapsedMs 'times'
    Assert-Equal @(90, 280) $timeline.TotalMB 'totals'
}

Test-Case '-SampleSeconds writes the summary as pure JSON' {
    $harness = @'
param([string]$Script)
function Get-CimInstance {
    param([Parameter(Position = 0)] [string]$ClassName, [string[]]$Property)
    if ($ClassName -eq 'Win32_Process') {
        [pscustomobject]@{ ProcessId = 100; ParentProcessId = 1; Name = 'lumen.exe'; CreationDate = $null; CommandLine = '' }
    } elseif ($ClassName -eq 'Win32_PerfRawData_PerfProc_Process') {
        [pscustomobject]@{ IDProcess = 100; WorkingSetPrivate = 10485760 }
    }
}
& $Script -ProcessId 100 -WaitSeconds 0 -SampleSeconds 1 -IntervalMs 200 -AsJson
'@
    $dir = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $dir | Out-Null
    try {
        $harnessPath = Join-Path $dir 'harness.ps1'
        $outPath = Join-Path $dir 'stdout.txt'
        $errPath = Join-Path $dir 'stderr.txt'
        Set-Content -LiteralPath $harnessPath -Value $harness
        $exe = if ($PSVersionTable.PSVersion.Major -ge 6) { 'pwsh.exe' } else { 'powershell.exe' }
        $script = Join-Path $PSScriptRoot 'measure-ram.ps1'
        $child = Start-Process -FilePath (Join-Path $PSHOME $exe) -NoNewWindow -Wait -PassThru `
            -ArgumentList '-NoProfile', '-File', "`"$harnessPath`"", "`"$script`"" `
            -RedirectStandardOutput $outPath -RedirectStandardError $errPath
        $stdout = [string](Get-Content -LiteralPath $outPath -Raw)
        $stderr = [string](Get-Content -LiteralPath $errPath -Raw)

        Assert-Equal 0 $child.ExitCode "exit code (stderr: $stderr)"
        try { $json = $stdout | ConvertFrom-Json } catch { throw "stdout is not pure JSON: $stdout" }
        Assert-Equal 10 $json.PeakMB 'PeakMB'
        Assert-Equal 0 $json.MsAboveThreshold 'MsAboveThreshold'
        if ($json.SampleCount -lt 2) { throw "only $($json.SampleCount) sample(s)" }
        Assert-Equal $json.SampleCount @($json.Timeline).Count 'timeline rows'
    } finally {
        Remove-Item -LiteralPath $dir -Recurse -Force
    }
}

Write-Host "$script:passed passed, $script:failed failed"
if ($script:failed -gt 0) { exit 1 }
exit 0
