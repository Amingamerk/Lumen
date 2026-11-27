# Offline tests for the pinned binary downloader. Run: pwsh -NoProfile -File scripts/fetch-pinned-binaries.test.ps1
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
. (Join-Path $PSScriptRoot 'fetch-pinned-binaries.ps1')
$script:failed = 0
$script:passed = 0
function Test-Case([string]$Name, [scriptblock]$Body) { try { & $Body; $script:passed++; Write-Host "ok   $Name" } catch { $script:failed++; Write-Host "FAIL $Name"; Write-Host "     $($_.Exception.Message)" } }
function Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Assert-Throws([scriptblock]$Body) { try { & $Body | Out-Null } catch { return }; throw 'expected an error' }
function New-TestManifest([string]$Source, [string]$Hash) { @{ artifacts = @(@{ name = 'sample'; version = '1'; url = ([uri]$Source).AbsoluteUri; sha256 = $Hash; file = 'sample.bin' }) } | ConvertTo-Json -Depth 4 }
function New-TestDir { $d = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Path $d | Out-Null; $d }

Test-Case 'clean local fetch verifies the download' {
    $d = New-TestDir; try { $source = Join-Path $d 'source.bin'; [IO.File]::WriteAllText($source, 'good'); $hash = (Get-FileHash $source -Algorithm SHA256).Hash; $manifest = Join-Path $d 'manifest.json'; New-TestManifest $source $hash | Set-Content $manifest; $dest = Join-Path $d 'out'; & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest; Assert (Test-Path "$dest/sample.bin") 'download missing' } finally { Remove-Item $d -Recurse -Force }
}
Test-Case 'second run is idempotent and does not download' {
    $d = New-TestDir; try { $source = Join-Path $d 'source.bin'; [IO.File]::WriteAllText($source, 'good'); $hash = (Get-FileHash $source -Algorithm SHA256).Hash; $manifest = Join-Path $d 'manifest.json'; New-TestManifest $source $hash | Set-Content $manifest; $dest = Join-Path $d 'out'; & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest; Assert (Test-Path "$dest/.pinned-hashes.json") 'stamp missing'; Remove-Item $source -Force; & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest } finally { Remove-Item $d -Recurse -Force }
}
Test-Case 'bad download hash removes every usable output' {
    $d = New-TestDir; try { $source = Join-Path $d 'source.bin'; [IO.File]::WriteAllText($source, 'bad'); $manifest = Join-Path $d 'manifest.json'; New-TestManifest $source ('0' * 64) | Set-Content $manifest; $dest = Join-Path $d 'out'; New-Item -ItemType Directory $dest | Out-Null; [IO.File]::WriteAllText((Join-Path $dest 'mpv.lib'), 'derived'); Assert-Throws { & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest }; Assert (-not (Test-Path "$dest/sample.bin")) 'bad archive survived'; Assert (-not (Test-Path "$dest/mpv.lib")) 'derived output survived' } finally { Remove-Item $d -Recurse -Force }
}
Test-Case 'tampered cached file fails and cleans up' {
    $d = New-TestDir; try { $source = Join-Path $d 'source.bin'; [IO.File]::WriteAllText($source, 'good'); $hash = (Get-FileHash $source -Algorithm SHA256).Hash; $manifest = Join-Path $d 'manifest.json'; New-TestManifest $source $hash | Set-Content $manifest; $dest = Join-Path $d 'out'; & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest; [IO.File]::WriteAllText((Join-Path $dest 'sample.bin'), 'tampered'); Assert-Throws { & $PSScriptRoot/fetch-pinned-binaries.ps1 -ManifestPath $manifest -DestDir $dest -VerifyOnly }; Assert (-not (Test-Path "$dest/sample.bin")) 'tampered cache survived' } finally { Remove-Item $d -Recurse -Force }
}
Test-Case 'mpv export parsing keeps only exports once' {
    $exports = @(Get-MpvExports @('  1    0 00001000 mpv_create', '  2    1 00001020 mpv_create', 'noise mpv_get_time_us extra'))
    Assert (($exports -join ',') -eq 'mpv_create,mpv_get_time_us') 'exports were not parsed'
}
Write-Host "$script:passed passed, $script:failed failed"
if ($script:failed -gt 0) { exit 1 }
