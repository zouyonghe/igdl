$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo = 'zouyonghe/igdl'
$LatestReleaseApi = "https://api.github.com/repos/$Repo/releases/latest"
$InstallDir = if ($env:IGDL_INSTALL_DIR) {
    $env:IGDL_INSTALL_DIR
} elseif ($env:LOCALAPPDATA) {
    Join-Path $env:LOCALAPPDATA 'Programs\igdl'
} else {
    Join-Path $HOME 'AppData\Local\Programs\igdl'
}

function Write-Info {
    param([string] $Message)

    Write-Host $Message
}

function Fail {
    param([string] $Message)

    throw $Message
}

function Get-RequestHeaders {
    @{
        'Accept' = 'application/vnd.github+json'
        'User-Agent' = 'igdl-install-script'
    }
}

function Get-ArchiveSuffix {
    $architecture = if ($env:PROCESSOR_ARCHITEW6432) {
        $env:PROCESSOR_ARCHITEW6432
    } else {
        $env:PROCESSOR_ARCHITECTURE
    }

    switch ($architecture.ToUpperInvariant()) {
        'AMD64' { return 'windows-x86_64.zip' }
        default { Fail "unsupported Windows architecture: $architecture. Only x86_64 is currently supported." }
    }
}

function Get-LatestRelease {
    $release = Invoke-RestMethod -Headers (Get-RequestHeaders) -Uri $LatestReleaseApi

    if (-not $release.tag_name) {
        Fail 'failed to resolve the latest release tag from GitHub.'
    }

    return $release
}

function Get-ReleaseAsset {
    param(
        [Parameter(Mandatory = $true)]
        $Release,

        [Parameter(Mandatory = $true)]
        [string] $AssetName
    )

    $asset = $Release.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1

    if (-not $asset) {
        Fail "release asset not found: $AssetName"
    }

    return $asset
}

function Get-ExpectedChecksum {
    param(
        [Parameter(Mandatory = $true)]
        [string] $ChecksumPath,

        [Parameter(Mandatory = $true)]
        [string] $ArchiveName
    )

    foreach ($line in Get-Content -Path $ChecksumPath) {
        if ($line -match '^([0-9A-Fa-f]{64})\s+\*?(.+)$' -and $Matches[2] -eq $ArchiveName) {
            return $Matches[1].ToLowerInvariant()
        }
    }

    Fail "missing checksum for $ArchiveName"
}

function Test-ArchiveChecksum {
    param(
        [Parameter(Mandatory = $true)]
        [string] $ArchivePath,

        [Parameter(Mandatory = $true)]
        [string] $ChecksumPath,

        [Parameter(Mandatory = $true)]
        [string] $ArchiveName
    )

    $expected = Get-ExpectedChecksum -ChecksumPath $ChecksumPath -ArchiveName $ArchiveName
    $actual = (Get-FileHash -Path $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()

    if ($expected -ne $actual) {
        Fail "checksum verification failed for $ArchiveName"
    }
}

function Ensure-UserPathContains {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Directory
    )

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathEntries = @()

    if ($userPath) {
        $pathEntries = $userPath.Split(';', [System.StringSplitOptions]::RemoveEmptyEntries)
    }

    foreach ($entry in $pathEntries) {
        if ($entry.TrimEnd('\\') -ieq $Directory.TrimEnd('\\')) {
            Write-Info "$Directory is already on your user PATH."
            return
        }
    }

    $updatedPath = if ($userPath) {
        "$userPath;$Directory"
    } else {
        $Directory
    }

    try {
        [Environment]::SetEnvironmentVariable('Path', $updatedPath, 'User')

        if ($env:Path) {
            $env:Path = "$Directory;$env:Path"
        } else {
            $env:Path = $Directory
        }

        Write-Info "Added $Directory to your user PATH. Restart PowerShell if the command is not available yet."
    } catch {
        Write-Info "Add $Directory to your user PATH if needed."
    }
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("igdl-install-" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempDir | Out-Null

try {
    $archiveSuffix = Get-ArchiveSuffix
    $release = Get-LatestRelease
    $tag = [string] $release.tag_name
    $archiveName = "igdl-$tag-$archiveSuffix"
    $archiveAsset = Get-ReleaseAsset -Release $release -AssetName $archiveName
    $checksumAsset = Get-ReleaseAsset -Release $release -AssetName 'SHA256SUMS.txt'
    $archivePath = Join-Path $tempDir $archiveName
    $checksumPath = Join-Path $tempDir 'SHA256SUMS.txt'
    $extractDir = Join-Path $tempDir 'extract'
    $binaryPath = Join-Path $extractDir 'igdl.exe'
    $installedBinaryPath = Join-Path $InstallDir 'igdl.exe'

    Write-Info "Downloading $archiveName..."
    Invoke-WebRequest -Headers (Get-RequestHeaders) -Uri $archiveAsset.browser_download_url -OutFile $archivePath

    Write-Info 'Downloading SHA256SUMS.txt...'
    Invoke-WebRequest -Headers (Get-RequestHeaders) -Uri $checksumAsset.browser_download_url -OutFile $checksumPath

    Test-ArchiveChecksum -ArchivePath $archivePath -ChecksumPath $checksumPath -ArchiveName $archiveName
    Write-Info "Verified checksum for $archiveName."

    New-Item -ItemType Directory -Path $extractDir | Out-Null
    Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

    if (-not (Test-Path -LiteralPath $binaryPath)) {
        Fail 'igdl.exe was not found in the downloaded archive.'
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -LiteralPath $binaryPath -Destination $installedBinaryPath -Force
    Ensure-UserPathContains -Directory $InstallDir

    Write-Info "Installed igdl $tag to $installedBinaryPath"
    Write-Info 'Usage: igdl <instagram-url>'
    Write-Info 'Example: igdl "https://www.instagram.com/reel/abc123/" --browser chrome'
} finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force
    }
}
