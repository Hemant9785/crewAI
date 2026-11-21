$ErrorActionPreference = 'Stop'

$repository = 'Hemant9785/crewAI'
$releaseUrl = "https://github.com/$repository/releases/latest/download"
$asset = 'team-herdr-windows-x86_64.zip'
$installDir = if ($env:TEAM_HERDR_INSTALL_DIR) {
    $env:TEAM_HERDR_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\TeamHerdr'
}
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString('N'))

try {
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
    $archivePath = Join-Path $tempDir $asset
    $checksumsPath = Join-Path $tempDir 'SHA256SUMS'
    Invoke-WebRequest -Uri "$releaseUrl/$asset" -OutFile $archivePath
    Invoke-WebRequest -Uri "$releaseUrl/SHA256SUMS" -OutFile $checksumsPath

    $checksumLine = Get-Content -LiteralPath $checksumsPath | Where-Object { $_ -match "\s+$([regex]::Escape($asset))$" } | Select-Object -First 1
    if (-not $checksumLine -or $checksumLine -notmatch '^([0-9a-fA-F]{64})\s+') {
        throw 'The release checksum entry is missing or malformed.'
    }
    $expectedHash = $Matches[1].ToLowerInvariant()
    $actualHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw 'The downloaded release archive failed SHA-256 verification.'
    }

    $extractDir = Join-Path $tempDir 'unpacked'
    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir
    if (-not (Test-Path -LiteralPath (Join-Path $extractDir 'herdr.exe') -PathType Leaf)) {
        throw 'The release archive does not contain herdr.exe.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $extractDir 'LICENSE') -PathType Leaf)) {
        throw 'The release archive does not contain the license.'
    }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Copy-Item -Path (Join-Path $extractDir '*') -Destination $installDir -Recurse -Force
    Write-Host "Installed Team Herdr to $installDir"
    Write-Host "Launch it with: `"$(Join-Path $installDir 'herdr.exe')`""
} finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force
    }
}
