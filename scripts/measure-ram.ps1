<#
.SYNOPSIS
Sums the private working set of an app and every process it started.

.DESCRIPTION
Waits, then walks the process tree from the target (Win32_Process ParentProcessId), so every
msedgewebview2, yt-dlp and deno child is counted and other apps' WebView2 processes are not.
Memory is WorkingSetPrivate from Win32_PerfFormattedData_PerfProc_Process, matched by IDProcess.
MB means 1,048,576 bytes, as in Task Manager.

By default the result is one snapshot and can undercount:
- A process started between the two CIM queries (process list, then counters) is omitted without a
  warning.
- A descendant whose intermediate parent has exited can no longer be reached by ParentProcessId, so it
  drops out of the tree.
- Short-lived peaks, such as a yt-dlp or deno resolve, are not captured.

With -SampleSeconds it samples the tree for that long instead, and reports the peak total, the
processes at the peak, the time above -ThresholdMB, the achieved interval, when each process name was
seen and a timeline of totals. Sampling reads Win32_PerfRawData_PerfProc_Process (the same
WorkingSetPrivate value, several times faster to query) and keeps counting a descendant it has seen
even after the parent between them exits. A peak shorter than the interval, or a process that starts
and exits between two samples, can still be missed, so the peak is a lower bound.

.PARAMETER ProcessName
Name of the app process, with or without .exe. Exactly one instance may be running.

.PARAMETER ProcessId
Id of the app process, instead of a name.

.PARAMETER WaitSeconds
Seconds to wait before measuring, so the app can settle. Default 30.

.PARAMETER SampleSeconds
Sample for this many seconds, after -WaitSeconds, instead of measuring once. It ends early if the
app exits.

.PARAMETER IntervalMs
Milliseconds from the start of one sample to the start of the next. Default 250; the achieved
interval is longer when the queries take longer.

.PARAMETER ThresholdMB
Sampling reports how long the total was above this. Default 150, the playing budget.

.PARAMETER AsJson
Write one JSON object instead of a table. Warnings then go to stderr, so stdout holds only the JSON.

.EXAMPLE
pwsh -File scripts/measure-ram.ps1 -ProcessName lumen

.EXAMPLE
pwsh -File scripts/measure-ram.ps1 -ProcessId 1234 -WaitSeconds 0 -AsJson

.EXAMPLE
pwsh -File scripts/measure-ram.ps1 -ProcessName lumen -WaitSeconds 0 -SampleSeconds 60
#>
[CmdletBinding()]
param(
    [string]$ProcessName,
    [int]$ProcessId,
    [ValidateRange(0, 3600)]
    [int]$WaitSeconds = 30,
    [ValidateRange(0, 3600)]
    [int]$SampleSeconds = 0,
    [ValidateRange(50, 500)]
    [int]$IntervalMs = 250,
    [ValidateRange(1, 100000)]
    [int]$ThresholdMB = 150,
    [switch]$AsJson
)

# Returns the root process and all its descendants.
function Get-ProcessTree {
    param(
        [Parameter(Mandatory)] [object[]]$Processes,
        [Parameter(Mandatory)] [int]$RootId
    )

    $children = @{}
    $root = $null
    foreach ($p in $Processes) {
        if ([int]$p.ProcessId -eq $RootId) { $root = $p }
        $parentId = [int]$p.ParentProcessId
        if (-not $children.ContainsKey($parentId)) { $children[$parentId] = New-Object System.Collections.ArrayList }
        [void]$children[$parentId].Add($p)
    }
    if ($null -eq $root) { return }

    $seen = @{ $RootId = $true }
    $queue = New-Object System.Collections.Queue
    $queue.Enqueue($root)
    while ($queue.Count -gt 0) {
        $parent = $queue.Dequeue()
        $parent
        foreach ($child in $children[[int]$parent.ProcessId]) {
            $id = [int]$child.ProcessId
            if ($seen.ContainsKey($id)) { continue }
            # Windows reuses process ids. A "child" older than its parent belonged to an earlier owner of that id.
            if ($child.CreationDate -and $parent.CreationDate -and $child.CreationDate -lt $parent.CreationDate) { continue }
            $seen[$id] = $true
            $queue.Enqueue($child)
        }
    }
}

# Returns the single process to measure, found by id or by name.
function Find-RootProcess {
    param(
        [Parameter(Mandatory)] [object[]]$Processes,
        [string]$ProcessName,
        [int]$ProcessId
    )

    if ($ProcessId) {
        $found = @($Processes | Where-Object { [int]$_.ProcessId -eq $ProcessId })
        if ($found.Count -eq 0) { throw "No process with id $ProcessId." }
        return $found[0]
    }

    $wanted = $ProcessName -replace '\.exe$', ''
    $found = @($Processes | Where-Object { ($_.Name -replace '\.exe$', '') -eq $wanted })
    if ($found.Count -eq 0) { throw "No process named '$ProcessName'." }

    # A match that descends from another match is part of that instance.
    $nested = @{}
    foreach ($match in $found) {
        foreach ($member in @(Get-ProcessTree -Processes $Processes -RootId $match.ProcessId)) {
            if ([int]$member.ProcessId -ne [int]$match.ProcessId) { $nested[[int]$member.ProcessId] = $true }
        }
    }
    $roots = @($found | Where-Object { -not $nested.ContainsKey([int]$_.ProcessId) })
    if ($roots.Count -ne 1) {
        $ids = @($roots | ForEach-Object { [int]$_.ProcessId } | Sort-Object) -join ', '
        throw "Expected one instance of '$ProcessName' but found ids: $ids. Close the extras or use -ProcessId."
    }
    return $roots[0]
}

# WebView2 starts several processes with the same name; the command line tells them apart.
# Only the role is returned, because command lines contain user paths.
function Get-ProcessRole {
    param(
        [string]$Name,
        [string]$CommandLine
    )

    if ($Name -ne 'msedgewebview2.exe') { return '' }
    if ($CommandLine -notmatch '--type=([\w-]+)') { return 'browser' }
    $type = $Matches[1]
    if ($CommandLine -match '--utility-sub-type=([\w.]+)') { return "$type ($($Matches[1]))" }
    return $type
}

# Joins the process tree with the performance counters and sums the private working set.
function Get-RamReport {
    param(
        [Parameter(Mandatory)] [object[]]$Tree,
        [Parameter(Mandatory)] [AllowEmptyCollection()] [object[]]$PerfRecords
    )

    $bytesById = @{}
    foreach ($record in $PerfRecords) { $bytesById[[int]$record.IDProcess] = [long]$record.WorkingSetPrivate }

    $total = [long]0
    $missing = @()
    $rows = @(
        foreach ($p in $Tree) {
            $id = [int]$p.ProcessId
            if (-not $bytesById.ContainsKey($id)) { $missing += $id; continue }
            $bytes = $bytesById[$id]
            $total += $bytes
            [pscustomobject]@{
                Name           = $p.Name
                PID            = $id
                Role           = Get-ProcessRole -Name $p.Name -CommandLine $p.CommandLine
                PrivateWS_MB   = [math]::Round($bytes / 1MB, 1)
                PrivateWSBytes = $bytes
            }
        }
    )

    [pscustomobject]@{
        TotalMB    = [math]::Round($total / 1MB, 1)
        TotalBytes = $total
        Processes  = @($rows | Sort-Object PrivateWSBytes -Descending)
        MissingIds = $missing
    }
}

# The root's tree, plus the trees of processes seen earlier that are still alive but have lost the
# parent that linked them to the root. $Known maps process id to creation date and is updated.
# Gives nothing when the root is gone.
function Get-TrackedTree {
    param(
        [Parameter(Mandatory)] [object[]]$Processes,
        [Parameter(Mandatory)] [int]$RootId,
        [Parameter(Mandatory)] [hashtable]$Known
    )

    $alive = @{}
    foreach ($p in $Processes) {
        $id = [int]$p.ProcessId
        # A different creation date means Windows gave the id to a new process.
        if ($Known.ContainsKey($id) -and $Known[$id] -eq $p.CreationDate) { $alive[$id] = $p }
    }
    if ($Known.ContainsKey($RootId) -and -not $alive.ContainsKey($RootId)) { return }

    $tree = [ordered]@{}
    foreach ($start in @($RootId) + @($alive.Keys)) {
        if ($tree.Contains([string]$start)) { continue }
        foreach ($member in @(Get-ProcessTree -Processes $Processes -RootId $start)) {
            $tree[[string]$member.ProcessId] = $member
        }
    }
    if (-not $tree.Contains([string]$RootId)) { return }
    foreach ($member in $tree.Values) { $Known[[int]$member.ProcessId] = $member.CreationDate }
    $tree.Values
}

# Takes a RAM report every interval until the duration has passed or the app is gone. The snapshot,
# clock (milliseconds) and sleep are passed in, so tests can fake them.
function Invoke-Sampling {
    param(
        [Parameter(Mandatory)] [int]$RootId,
        [Parameter(Mandatory)] [int]$DurationMs,
        [Parameter(Mandatory)] [int]$IntervalMs,
        [Parameter(Mandatory)] [scriptblock]$Snapshot,
        [Parameter(Mandatory)] [scriptblock]$Clock,
        [Parameter(Mandatory)] [scriptblock]$Sleep
    )

    $known = @{}
    $start = & $Clock
    while ($true) {
        $began = & $Clock
        $taken = & $Snapshot
        $tree = @(Get-TrackedTree -Processes $taken.Processes -RootId $RootId -Known $known)
        if ($tree.Count -eq 0) { return }
        [pscustomobject]@{
            ElapsedMs = [long]($began - $start)
            Report    = Get-RamReport -Tree $tree -PerfRecords $taken.PerfRecords
        }
        if ($began - $start -ge $DurationMs) { return }
        $rest = $began + $IntervalMs - (& $Clock)
        if ($rest -gt 0) { & $Sleep ([int]$rest) }
    }
}

# Peak, time above the threshold and achieved interval of a series of samples.
function Get-SampleSummary {
    param(
        [Parameter(Mandatory)] [object[]]$Samples,
        [Parameter(Mandatory)] [int]$ThresholdMB
    )

    $limit = [long]$ThresholdMB * 1MB
    $peak = $Samples[0]
    $above = [long]0
    $worst = [long]0
    $incomplete = 0
    $seen = [ordered]@{}
    for ($i = 0; $i -lt $Samples.Count; $i++) {
        $sample = $Samples[$i]
        if ($sample.Report.TotalBytes -gt $peak.Report.TotalBytes) { $peak = $sample }
        if ($i + 1 -lt $Samples.Count) {
            $gap = $Samples[$i + 1].ElapsedMs - $sample.ElapsedMs
            if ($gap -gt $worst) { $worst = $gap }
            if ($sample.Report.TotalBytes -gt $limit) { $above += $gap }
        }
        if (@($sample.Report.MissingIds).Count -gt 0) { $incomplete++ }
        foreach ($name in @($sample.Report.Processes | ForEach-Object { $_.Name } | Sort-Object -Unique)) {
            if (-not $seen.Contains($name)) {
                $seen[$name] = [pscustomobject]@{ Name = $name; FirstMs = $sample.ElapsedMs; LastMs = $sample.ElapsedMs; Samples = 0 }
            }
            $seen[$name].LastMs = $sample.ElapsedMs
            $seen[$name].Samples++
        }
    }

    $duration = $Samples[-1].ElapsedMs - $Samples[0].ElapsedMs
    $mean = if ($Samples.Count -gt 1) { [int][math]::Round($duration / ($Samples.Count - 1)) } else { 0 }

    [pscustomobject]@{
        SampleCount                = $Samples.Count
        DurationMs                 = $duration
        MeanIntervalMs             = $mean
        MaxIntervalMs              = $worst
        PeakMB                     = $peak.Report.TotalMB
        PeakBytes                  = $peak.Report.TotalBytes
        PeakAtMs                   = $peak.ElapsedMs
        PeakProcesses              = @($peak.Report.Processes)
        ThresholdMB                = $ThresholdMB
        MsAboveThreshold           = $above
        SamplesWithMissingCounters = $incomplete
        Seen                       = @($seen.Values)
        Timeline                   = @($Samples | ForEach-Object {
                [pscustomobject]@{ ElapsedMs = $_.ElapsedMs; TotalMB = $_.Report.TotalMB; ProcessCount = @($_.Report.Processes).Count }
            })
        Note                       = "Samples were $mean ms apart on average, $worst ms at worst. A shorter peak can be missed, so the figures can undercount."
    }
}

# Dot-sourcing (the tests) loads the functions without measuring anything.
if ($MyInvocation.InvocationName -eq '.') { return }

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

if ([bool]$ProcessName -eq [bool]$ProcessId) {
    [Console]::Error.WriteLine('Give exactly one of -ProcessName or -ProcessId.')
    exit 1
}

try {
    if ($WaitSeconds -gt 0) {
        Write-Verbose "Waiting $WaitSeconds s"
        Start-Sleep -Seconds $WaitSeconds
    }

    $processes = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, CreationDate, CommandLine)
    if ($SampleSeconds -gt 0) {
        $root = Find-RootProcess -Processes $processes -ProcessName $ProcessName -ProcessId $ProcessId
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $samples = @(Invoke-Sampling -RootId $root.ProcessId -DurationMs ($SampleSeconds * 1000) -IntervalMs $IntervalMs `
                -Snapshot {
                    [pscustomobject]@{
                        Processes   = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, CreationDate, CommandLine)
                        PerfRecords = @(Get-CimInstance Win32_PerfRawData_PerfProc_Process -Property IDProcess, WorkingSetPrivate)
                    }
                } `
                -Clock { $watch.ElapsedMilliseconds } `
                -Sleep { param($Ms) Start-Sleep -Milliseconds $Ms })
        if ($samples.Count -eq 0) { throw "$($root.Name) exited before the first sample." }
        $summary = Get-SampleSummary -Samples $samples -ThresholdMB $ThresholdMB
        if ($AsJson) {
            $summary | ConvertTo-Json -Depth 4
        } else {
            $summary.PeakProcesses | Format-Table Name, PID, Role, PrivateWS_MB -AutoSize | Out-String -Width 160 | ForEach-Object { $_.TrimEnd() }
            ''
            "PeakMB: $($summary.PeakMB) at $($summary.PeakAtMs) ms (root $($root.Name) pid $($root.ProcessId))"
            "Above $ThresholdMB MB: $($summary.MsAboveThreshold) ms of $($summary.DurationMs) ms"
            "Samples: $($summary.SampleCount), mean interval $($summary.MeanIntervalMs) ms, max $($summary.MaxIntervalMs) ms"
            $summary.Seen | Format-Table Name, FirstMs, LastMs, Samples -AutoSize | Out-String -Width 160 | ForEach-Object { $_.TrimEnd() }
            $summary.Note
        }
        exit 0
    }
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Property IDProcess, WorkingSetPrivate)

    $root = Find-RootProcess -Processes $processes -ProcessName $ProcessName -ProcessId $ProcessId
    $tree = @(Get-ProcessTree -Processes $processes -RootId $root.ProcessId)
    $report = Get-RamReport -Tree $tree -PerfRecords $perf
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}

if (@($report.MissingIds).Count -gt 0) {
    $warning = "No counter for process id(s) $($report.MissingIds -join ', '); they exited during the measurement and are not in the total."
    # A child shell prints Write-Warning on stdout, which would break the JSON.
    if ($AsJson) { [Console]::Error.WriteLine("WARNING: $warning") } else { Write-Warning $warning }
}

if ($AsJson) {
    [pscustomobject]@{
        Target     = $root.Name
        RootPID    = [int]$root.ProcessId
        MeasuredAt = (Get-Date).ToString('s')
        TotalMB    = $report.TotalMB
        TotalBytes = $report.TotalBytes
        Processes  = $report.Processes
        MissingIds = @($report.MissingIds)
    } | ConvertTo-Json -Depth 4
} else {
    $report.Processes | Format-Table Name, PID, Role, PrivateWS_MB -AutoSize | Out-String -Width 160 | ForEach-Object { $_.TrimEnd() }
    ''
    "TotalMB: $($report.TotalMB) ($(@($report.Processes).Count) processes, root $($root.Name) pid $($root.ProcessId))"
}
