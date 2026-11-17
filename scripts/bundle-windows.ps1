[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$BuildProfile = 'debug',
    [string]$Target = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }

$Candidates = if ($Target) {
    @(Join-Path $TargetDir "$Target\$BuildProfile\riseonly.exe")
} else {
    @(
        (Join-Path $TargetDir "$BuildProfile\riseonly.exe"),
        (Join-Path $TargetDir "x86_64-pc-windows-msvc\$BuildProfile\riseonly.exe"),
        (Join-Path $TargetDir "aarch64-pc-windows-msvc\$BuildProfile\riseonly.exe")
    )
}
$Bin = $Candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $Bin) {
    throw "missing binary, looked at: $($Candidates -join ', '). Run cargo build -p riseonly first."
}

$Info = (Get-Item -LiteralPath $Bin).VersionInfo
$AppName = $Info.ProductName
$Version = $Info.ProductVersion
if ([string]::IsNullOrWhiteSpace($AppName) -or [string]::IsNullOrWhiteSpace($Version)) {
    throw "$Bin carries no VERSIONINFO: it was not built by app/build.rs for a windows target."
}

$ExeBytes = [System.IO.File]::ReadAllBytes($Bin)
$ExeText = [System.Text.Encoding]::ASCII.GetString($ExeBytes)
$FfmpegPrefix = $null
if ($ExeText -match '(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale)-[0-9]+\.dll') {
    $PeOffset = [System.BitConverter]::ToInt32($ExeBytes, 0x3C)
    $Machine = [System.BitConverter]::ToUInt16($ExeBytes, $PeOffset + 4)
    $Arch = switch ($Machine) {
        0x8664 { 'x86_64' }
        0xAA64 { 'arm64' }
        default { throw "$Bin has PE machine 0x$('{0:X4}' -f $Machine), which has no FFmpeg prefix." }
    }
    $FfmpegPrefix = Join-Path $Root "vendor\ffmpeg\windows-$Arch"
    if (-not (Test-Path -LiteralPath (Join-Path $FfmpegPrefix 'bin') -PathType Container)) {
        throw "$Bin imports $($Matches[0]), but $FfmpegPrefix is missing. Run scripts/build-ffmpeg-lgpl.sh for windows-$Arch."
    }
}

$ProfileDir = Join-Path $Root "build\windows\$BuildProfile"
$Out = Join-Path $ProfileDir $AppName
if (Test-Path -LiteralPath $Out) {
    Remove-Item -LiteralPath $Out -Recurse -Force
}
New-Item -ItemType Directory -Path $Out -Force | Out-Null

Copy-Item -LiteralPath $Bin -Destination (Join-Path $Out 'riseonly.exe')

$Assets = Join-Path $Root 'assets'
if (Test-Path -LiteralPath $Assets -PathType Container) {
    Copy-Item -LiteralPath $Assets -Destination (Join-Path $Out 'assets') -Recurse
}

$Python = Get-Command python3, python -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
$RlottieRoot = Join-Path $Root 'vendor\rlottie'
$Trees = if (Test-Path -LiteralPath $RlottieRoot) {
    @(Get-ChildItem -LiteralPath $RlottieRoot -Directory -Filter 'rlottie-*')
} else {
    @()
}
foreach ($Tree in $Trees) {
    if (-not $Python) {
        throw "python is needed to list the rlottie MPL-2.0 sources of $($Tree.FullName)."
    }
    $Report = @(& $Python.Source (Join-Path $Root 'scripts\check-rlottie-license.py') $Tree.FullName)
    if ($LASTEXITCODE -ne 0) {
        throw ($Report -join [Environment]::NewLine)
    }
    $Licenses = Join-Path $Out "licenses\$($Tree.Name)"
    New-Item -ItemType Directory -Path $Licenses -Force | Out-Null
    Copy-Item -Path (Join-Path $Tree.FullName 'licenses\COPYING.*') -Destination $Licenses
    foreach ($Line in $Report) {
        if ($Line -match '^  MPL-2\.0, source must ship with the release: (.+)$') {
            $File = $Matches[1]
            $Destination = Join-Path $Licenses "source\$File"
            New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
            Copy-Item -LiteralPath (Join-Path $Tree.FullName $File) -Destination $Destination
        }
    }
}

if ($FfmpegPrefix) {
    if (-not $Python) {
        throw "python is needed to re-derive the FFmpeg licence of $FfmpegPrefix."
    }
    $Report = @(& $Python.Source (Join-Path $Root 'scripts\check-ffmpeg-license.py') $FfmpegPrefix)
    if ($LASTEXITCODE -ne 0) {
        throw ($Report -join [Environment]::NewLine)
    }
    $Dlls = @(Get-ChildItem -LiteralPath (Join-Path $FfmpegPrefix 'bin') -File -Filter '*.dll' |
        Where-Object { $_.Name -match '^(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale)-[0-9]+\.dll$' })
    if ($Dlls.Count -eq 0) {
        throw "$FfmpegPrefix\bin holds no FFmpeg DLL."
    }
    foreach ($Dll in $Dlls) {
        Copy-Item -LiteralPath $Dll.FullName -Destination (Join-Path $Out $Dll.Name)
    }
    $BuildInfo = Join-Path $FfmpegPrefix 'BUILD_INFO.txt'
    $TarballLine = if (Test-Path -LiteralPath $BuildInfo) {
        Select-String -LiteralPath $BuildInfo -Pattern '^source_tarball=(.+)$' | Select-Object -First 1
    } else {
        $null
    }
    $Tarball = if ($TarballLine) { Join-Path $Root "vendor\ffmpeg\$($TarballLine.Matches[0].Groups[1].Value)" } else { $null }
    if (-not $Tarball -or -not (Test-Path -LiteralPath $Tarball -PathType Leaf)) {
        throw "FFmpeg source tarball for $FfmpegPrefix not found; LGPL needs it shipped."
    }
    $Licenses = Join-Path $Out 'licenses\ffmpeg'
    New-Item -ItemType Directory -Path $Licenses -Force | Out-Null
    Copy-Item -LiteralPath $Tarball -Destination $Licenses
    Copy-Item -LiteralPath $BuildInfo -Destination (Join-Path $Licenses 'BUILD_INFO.txt')
    Write-Host "ffmpeg: $(($Dlls | ForEach-Object { $_.Name }) -join ' ')"
}

$Zip = Join-Path $ProfileDir "$AppName $Version.zip"
Compress-Archive -Path $Out -DestinationPath $Zip -Force

Write-Host "bundled: $Out ($AppName $Version)"
Write-Host "zip: $Zip"
