[CmdletBinding()]
param(
    [ValidateSet('check', 'test', 'build', 'fmt', 'fmt-check')]
    [string]$Mode = 'check',
    [switch]$Clean,
    [switch]$Offline,
    [string]$Filter
)

$ErrorActionPreference = 'Stop'

$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$HerdrRoot = Join-Path $ProjectRoot 'herdr'
$CacheRoot = Join-Path $ProjectRoot '.cache'
$CargoHome = Join-Path $CacheRoot 'cargo-home'
$RustupHome = Join-Path $CacheRoot 'rustup-home'
$TempRoot = Join-Path $CacheRoot 'tmp'
$TargetRoot = Join-Path $HerdrRoot 'target'
$CargoExe = Join-Path $CargoHome 'bin\cargo.exe'
$ZigExe = Join-Path $ProjectRoot '.tools\zig-x86_64-windows-0.16.0\zig.exe'

foreach ($requiredPath in @($HerdrRoot, $CargoExe, $RustupHome, $ZigExe)) {
    if (-not (Test-Path -LiteralPath $requiredPath)) {
        throw "Required build path is missing: $requiredPath"
    }
}

New-Item -ItemType Directory -Force -Path $TempRoot | Out-Null

# Keep all build state on E: and keep the compiler's peak memory bounded.
$env:CARGO_HOME = $CargoHome
$env:RUSTUP_HOME = $RustupHome
$env:CARGO_TARGET_DIR = $TargetRoot
$env:TEMP = $TempRoot
$env:TMP = $TempRoot
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:RUSTFLAGS = '-C debuginfo=0'
$env:ZIG = $ZigExe
$env:Path = "$CargoHome\bin;$env:Path"

if ($Offline) {
    $env:CARGO_NET_OFFLINE = 'true'
}

$manifest = Join-Path $HerdrRoot 'Cargo.toml'
if ($Clean) {
    Write-Host "Cleaning generated artifacts under $TargetRoot"
    & $CargoExe clean --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$common = @('--locked', '-p', 'herdr', '--bin', 'herdr')
switch ($Mode) {
    'check' {
        $arguments = @('check') + $common + @('--tests')
    }
    'test' {
        $arguments = @('test') + $common
        if ($Filter) {
            $arguments += $Filter
        }
        $arguments += @('--', '--nocapture')
    }
    'build' {
        $arguments = @('build') + $common
    }
    'fmt' {
        $arguments = @('fmt', '--all')
    }
    'fmt-check' {
        $arguments = @('fmt', '--all', '--', '--check')
    }
}

Push-Location $HerdrRoot
try {
    Write-Host "Running Cargo $Mode (Clean=$Clean, Offline=$Offline)"
    & $CargoExe @arguments
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
