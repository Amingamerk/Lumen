<#
.SYNOPSIS
Fetches the pinned libmpv, yt-dlp and Deno binaries into src-tauri/binaries and verifies them.

.DESCRIPTION
Reads scripts/pinned-binaries.json, downloads each artifact that is missing, checks its SHA-256
(also for files already present, such as a CI cache restore), unpacks deno.exe, extracts
libmpv-2.dll with the tar.exe that ships with Windows and generates mpv.lib from the DLL exports.
A second run downloads nothing. A hash mismatch deletes the bad file and every file derived from
it, then fails.

.PARAMETER VerifyOnly
Check the files that are present against the manifest; fail on a missing or wrong file. No downloads.

.EXAMPLE
./scripts/fetch-pinned-binaries.ps1
#>
param(
    [string]$ManifestPath = (Join-Path $PSScriptRoot 'pinned-binaries.json'),
    [string]$DestDir = (Join-Path (Split-Path -Parent $PSScriptRoot) 'src-tauri/binaries'),
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'

# Files produced from the artifacts; removed whenever an artifact fails its hash check.
$script:DerivedFiles = @(
    'libmpv-2.dll', 'libmpv.dll.a', 'mpv.def', 'mpv.exp', 'mpv.lib', 'include',
    'deno-x86_64-pc-windows-msvc.exe', 'deno-unpack', '.pinned-hashes.json'
)

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Remove-PinnedOutputs([string]$Dir, [string]$File) {
    $names = @($script:DerivedFiles)
    if ($File) { $names = @($File) + $names }
    foreach ($name in $names) {
        $p = Join-Path $Dir $name
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }
    }
}

function Assert-PinnedHash([string]$Path, $Item, [string]$Dir) {
    $actual = Get-Sha256 $Path
    if ($actual -ne $Item.sha256.ToLowerInvariant()) {
        Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
        Remove-PinnedOutputs $Dir $Item.file
        throw "SHA-256 mismatch for $($Item.name): expected $($Item.sha256), got $actual. Removed it and its derived files."
    }
}

function Save-PinnedArtifact($Item, [string]$Target) {
    $uri = [uri]$Item.url
    if ($uri.IsFile) { Copy-Item -LiteralPath $uri.LocalPath -Destination $Target -Force }
    else { Invoke-WebRequest -Uri $Item.url -OutFile $Target -UseBasicParsing }
}

function Get-MpvExports([string[]]$DumpbinLines) {
    $seen = [System.Collections.Generic.List[string]]::new()
    foreach ($line in $DumpbinLines) {
        if ($line -match '\b(mpv_[A-Za-z0-9_]+)\b' -and -not $seen.Contains($Matches[1])) { $seen.Add($Matches[1]) }
    }
    $seen.ToArray()
}

function Get-PinnedStampLines($Artifacts) {
    @($Artifacts | ForEach-Object { "$($_.name):$($_.sha256.ToLowerInvariant())" } | Sort-Object)
}

function Test-PinnedStamp([string]$Dir, $Artifacts) {
    $stamp = Join-Path $Dir '.pinned-hashes.json'
    if (-not (Test-Path -LiteralPath $stamp)) { return $false }
    try {
        $saved = @(Get-Content -LiteralPath $stamp -Raw | ConvertFrom-Json).artifacts
        return ((@($saved | Sort-Object) -join "`n") -eq ((Get-PinnedStampLines $Artifacts) -join "`n"))
    } catch { return $false }
}

function Write-PinnedStamp([string]$Dir, $Artifacts) {
    [pscustomobject]@{ artifacts = @(Get-PinnedStampLines $Artifacts) } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Dir '.pinned-hashes.json') -Encoding ascii
}

function Sync-PinnedArtifacts([string]$Manifest, [string]$Dir, [switch]$Verify) {
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    $artifacts = (Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json).artifacts
    $stampMatches = Test-PinnedStamp $Dir $artifacts
    if (-not $stampMatches -and -not $Verify) { Remove-PinnedOutputs $Dir '' }
    foreach ($item in $artifacts) {
        $path = Join-Path $Dir $item.file
        if (Test-Path -LiteralPath $path) { Assert-PinnedHash $path $item $Dir; continue }
        if ($Verify) { throw "Missing $($item.name): $path" }
        $tmp = "$path.download"
        try {
            Write-Host "Downloading $($item.name) $($item.version)"
            Save-PinnedArtifact $item $tmp
            Assert-PinnedHash $tmp $item $Dir
            Move-Item -LiteralPath $tmp -Destination $path -Force
        } finally { Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue }
    }
    $names = @($artifacts | ForEach-Object { $_.name })
    if (-not $Verify) {
        if ($names -contains 'deno') { Expand-PinnedDeno $Dir }
        if ($names -contains 'libmpv') { Expand-PinnedLibmpv $Dir }
        Write-PinnedStamp $Dir $artifacts
    }
}

function Expand-PinnedDeno([string]$Dir) {
    $exe = Join-Path $Dir 'deno-x86_64-pc-windows-msvc.exe'
    if ((Test-Path -LiteralPath $exe) -and (Get-Item -LiteralPath $exe).Length -gt 0) { return }
    $unpack = Join-Path $Dir 'deno-unpack'
    try {
        Expand-Archive -LiteralPath (Join-Path $Dir 'deno.zip') -DestinationPath $unpack -Force
        Copy-Item -LiteralPath (Join-Path $unpack 'deno.exe') -Destination $exe -Force
    } finally { Remove-Item -LiteralPath $unpack -Recurse -Force -ErrorAction SilentlyContinue }
}

function Expand-PinnedLibmpv([string]$Dir) {
    $dll = Join-Path $Dir 'libmpv-2.dll'
    $lib = Join-Path $Dir 'mpv.lib'
    if (-not (Test-Path -LiteralPath $dll)) {
        # bsdtar (System32\tar.exe, Windows 10 1803+) reads 7z archives; no 7-Zip install needed.
        $tar = Join-Path $env:SystemRoot 'System32\tar.exe'
        if (-not (Test-Path -LiteralPath $tar)) { $tar = (Get-Command tar.exe -ErrorAction SilentlyContinue).Source }
        if ($tar) { & $tar -xf (Join-Path $Dir 'libmpv.7z') -C $Dir }
        else {
            $seven = Get-Command 7z.exe -ErrorAction SilentlyContinue
            if (-not $seven) { throw 'tar.exe and 7z.exe are both unavailable.' }
            & $seven.Source x (Join-Path $Dir 'libmpv.7z') "-o$Dir" -y
        }
        if ($LASTEXITCODE -ne 0) { throw 'tar.exe failed to extract the libmpv archive.' }
    }
    if (Test-Path -LiteralPath $lib) { return }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere.exe not found; install the Visual Studio C++ build tools.' }
    $dumpbin = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe' | Select-Object -First 1
    if (-not $dumpbin) { throw 'dumpbin.exe not found via vswhere.' }
    $libExe = Join-Path (Split-Path -Parent $dumpbin) 'lib.exe'
    $exports = @(Get-MpvExports (& $dumpbin /nologo /exports $dll))
    if ($exports.Count -eq 0) { throw 'No mpv_* exports found in libmpv-2.dll.' }
    $def = Join-Path $Dir 'mpv.def'
    @('LIBRARY libmpv-2.dll', 'EXPORTS') + $exports | Set-Content -LiteralPath $def -Encoding ascii
    & $libExe /nologo "/def:$def" /machine:x64 "/out:$lib" | Out-Null
    if ($LASTEXITCODE -ne 0) { Remove-Item -LiteralPath $lib -Force -ErrorAction SilentlyContinue; throw 'lib.exe failed to generate mpv.lib.' }
    Write-Host "Generated mpv.lib ($($exports.Count) exports)"
}

# Dot-sourcing (the tests) only loads the functions.
if ($MyInvocation.InvocationName -ne '.') {
    Sync-PinnedArtifacts -Manifest $ManifestPath -Dir $DestDir -Verify:$VerifyOnly
}
