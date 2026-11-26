<#
.SYNOPSIS
Sums the private working set of an app and every process it started.

.DESCRIPTION
Waits, then walks the process tree from the target (Win32_Process ParentProcessId), so every
msedgewebview2, yt-dlp and deno child is counted and other apps' WebView2 processes are not.
Memory is WorkingSetPrivate from Win32_PerfFormattedData_PerfProc_Process, matched by IDProcess.
MB means 1,048,576 bytes, as in Task Manager.

.PARAMETER ProcessName
Name of the app process, with or without .exe. Exactly one instance may be running.

.PARAMETER ProcessId
Id of the app process, instead of a name.

.PARAMETER WaitSeconds
Seconds to wait before measuring, so the app can settle. Default 30.

.PARAMETER AsJson
Write one JSON object instead of a table.

.EXAMPLE
pwsh -File scripts/measure-ram.ps1 -ProcessName lumen

.EXAMPLE
pwsh -File scripts/measure-ram.ps1 -ProcessId 1234 -WaitSeconds 0 -AsJson
#>
[CmdletBinding()]
param(
    [string]$ProcessName,
    [int]$ProcessId,
    [ValidateRange(0, 3600)]
    [int]$WaitSeconds = 30,
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
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Property IDProcess, WorkingSetPrivate)

    $root = Find-RootProcess -Processes $processes -ProcessName $ProcessName -ProcessId $ProcessId
    $tree = @(Get-ProcessTree -Processes $processes -RootId $root.ProcessId)
    $report = Get-RamReport -Tree $tree -PerfRecords $perf
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}

if (@($report.MissingIds).Count -gt 0) {
    Write-Warning "No counter for process id(s) $($report.MissingIds -join ', '); they exited during the measurement and are not in the total."
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
