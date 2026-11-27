# Tests the pure helpers of ram-run.ps1 (median, verdict, summary table, option handling, track choice).
# Run: pwsh -NoProfile -File scripts/ram-run.test.ps1
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

. (Join-Path $PSScriptRoot 'ram-run.ps1')

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

Test-Case 'median of an odd count is the middle value' {
    Assert-Equal 5 (Get-Median @(9, 1, 5)) 'median'
}
Test-Case 'median of an even count is the mean of the middle two' {
    Assert-Equal 3.5 (Get-Median @(4, 1, 3, 9)) 'median'
}
Test-Case 'median of one value is that value' {
    Assert-Equal 7.2 (Get-Median @(7.2)) 'median'
}
Test-Case 'median of nothing is an error' {
    Assert-Throws { Get-Median @() } 'no values'
}

Test-Case 'verdict passes at the budget and fails above it' {
    Assert-Equal pass (Get-Verdict 100 100) 'at budget'
    Assert-Equal fail (Get-Verdict 100.1 100) 'above budget'
}

Test-Case 'summary row holds median, headroom and verdict' {
    $row = New-SummaryRow -Scenario 'Idle' -Values @(95.1, 94.3, 97.0) -BudgetMB 100
    Assert-Equal 95.1 $row.Median 'median'
    Assert-Equal 4.9 $row.Headroom 'headroom'
    Assert-Equal pass $row.Verdict 'verdict'
}

Test-Case 'summary row without a budget has no verdict' {
    $row = New-SummaryRow -Scenario 'CPU' -Values @(0.2) -BudgetMB 0
    Assert-Equal 'n/a' $row.Verdict 'verdict'
}

Test-Case 'summary table is markdown with one cell per run' {
    $rows = @(
        New-SummaryRow -Scenario 'Idle' -Values @(95.1, 94.3, 97.0) -BudgetMB 100
        New-SummaryRow -Scenario 'Playing' -Values @(151, 140) -BudgetMB 150
    )
    $lines = @(Format-SummaryTable $rows -Runs 3)
    Assert-Equal '| Scenario | Run 1 | Run 2 | Run 3 | Median | Budget | Headroom | Verdict |' $lines[0] 'header'
    Assert-Equal '| Idle | 95.1 MB | 94.3 MB | 97.0 MB | 95.1 MB | 100 MB | 4.9 MB | pass |' $lines[2] 'idle row'
    Assert-Equal '| Playing | 151.0 MB | 140.0 MB | - | 145.5 MB | 150 MB | 4.5 MB | pass |' $lines[3] 'missing run'
}

Test-Case 'options default the exe and output below the repo root' {
    $root = Join-Path ([IO.Path]::GetTempPath()) ("ramrun-" + [guid]::NewGuid().ToString('N'))
    $exe = Join-Path $root 'src-tauri/target/release/lumen.exe'
    New-Item -ItemType Directory -Force (Split-Path $exe) | Out-Null
    Set-Content $exe ''
    try {
        $o = Resolve-RamRunOptions -RepoRoot $root -Runs 2
        Assert-Equal $exe $o.ExePath 'exe'
        if (-not $o.OutDir.StartsWith((Join-Path $root 'src-tauri/target/ram-run'))) { throw "out dir $($o.OutDir)" }
        Assert-Equal 2 $o.Runs 'runs'
    } finally { Remove-Item -LiteralPath $root -Recurse -Force }
}

Test-Case 'options reject a missing exe' {
    Assert-Throws { Resolve-RamRunOptions -RepoRoot ([IO.Path]::GetTempPath()) -ExePath 'nope/lumen.exe' } 'pnpm tauri build'
}

Test-Case 'options reject a run count outside 1 to 3' {
    Assert-Throws { Resolve-RamRunOptions -RepoRoot ([IO.Path]::GetTempPath()) -Runs 4 } 'Runs'
    Assert-Throws { Resolve-RamRunOptions -RepoRoot ([IO.Path]::GetTempPath()) -Runs 0 } 'Runs'
}

Test-Case 'track choice prefers a long mix and varies by run' {
    $names = @('Play Song A by X', 'Play 1 hour lofi by Y', 'Play Song B by X', 'Play 2 hours rain by Z')
    Assert-Equal 'Play 1 hour lofi by Y' (Select-Track $names -Run 1) 'run 1'
    Assert-Equal 'Play 2 hours rain by Z' (Select-Track $names -Run 2) 'run 2'
    Assert-Equal 'Play 1 hour lofi by Y' (Select-Track $names -Run 3) 'run 3 wraps'
}

Test-Case 'track choice falls back to the first ordinary song' {
    Assert-Equal 'Play Song A by X' (Select-Track @('Play Song A by X', 'Play Song B by X') -Run 1) 'fallback'
}

Test-Case 'track choice with no results is an error' {
    Assert-Throws { Select-Track @() -Run 1 } 'no results'
}

Write-Host "$script:passed passed, $script:failed failed"
if ($script:failed -gt 0) { exit 1 }
exit 0
