# Installs the crewai standalone binary on Windows.
#
#   powershell -c "irm https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.ps1 | iex"
#
# Optional environment variables (set before running):
#   CREWAI_VERSION      release tag to install, for example crewai-v0.1.0 (default: newest crewai-v* release)
#   CREWAI_INSTALL_DIR  where to put crewai.exe (default: %LOCALAPPDATA%\Programs\crewai)
#   CREWAI_NO_PATH      set to 1 to leave your PATH unchanged
#   CREWAI_BASE_URL     folder URL holding the binary and SHA256SUMS (testing or mirrors)

function Install-Crewai {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

    $repo = 'Hemant9785/crewAI'
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($env:PROCESSOR_ARCHITEW6432) { $arch = $env:PROCESSOR_ARCHITEW6432 }
    if ($arch -ne 'AMD64') { throw "crewai has no Windows build for $arch yet. Only 64-bit Intel/AMD (x64) is supported." }
    $file = 'crewai-windows-x64.exe'

    $base = $env:CREWAI_BASE_URL
    if (-not $base) {
        $tag = $env:CREWAI_VERSION
        if (-not $tag) {
            Write-Host 'Looking up the latest crewai release...'
            $releases = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$repo/releases?per_page=30" -Headers @{ 'User-Agent' = 'crewai-installer' }
            $latest = $releases | Where-Object { $_.tag_name -like 'crewai-v*' -and -not $_.draft -and -not $_.prerelease } | Select-Object -First 1
            if (-not $latest) { throw "No crewai release was found at https://github.com/$repo/releases yet." }
            $tag = $latest.tag_name
        }
        $base = "https://github.com/$repo/releases/download/$tag"
    }
    $base = $base.TrimEnd('/')

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("crewai-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "Downloading $file ..."
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$file" -OutFile (Join-Path $tmp $file)
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')

        $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match ([regex]::Escape($file) + '\s*$') } | Select-Object -First 1
        if (-not $line) { throw "SHA256SUMS has no entry for $file." }
        $expected = ($line -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path (Join-Path $tmp $file)).Hash.ToLowerInvariant()
        if ($expected -ne $actual) { throw "Checksum mismatch for $file. Expected $expected but got $actual. Nothing was installed." }
        Write-Host 'Checksum verified.'

        $dir = $env:CREWAI_INSTALL_DIR
        if (-not $dir) { $dir = Join-Path $env:LOCALAPPDATA 'Programs\crewai' }
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        $dest = Join-Path $dir 'crewai.exe'
        try {
            Move-Item -Force -Path (Join-Path $tmp $file) -Destination $dest
        } catch {
            throw "Could not write $dest. If crewai is running, close it and run this again."
        }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }

    if ($env:CREWAI_NO_PATH -ne '1') {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $parts = @()
        if ($userPath) { $parts = $userPath -split ';' | Where-Object { $_ } }
        if ($parts -notcontains $dir) {
            [Environment]::SetEnvironmentVariable('Path', (($parts + $dir) -join ';'), 'User')
            Write-Host "Added $dir to your PATH."
        }
        if (($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }
    }

    $version = & $dest --version
    Write-Host ''
    Write-Host "crewai $version is installed at $dest"
    Write-Host 'Open a project folder and run: crewai'
    Write-Host 'To look around first: crewai --demo'
}

Install-Crewai
