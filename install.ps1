#Requires -Version 5.1
<#
.SYNOPSIS
    agent-switch (ags) Windows Installer and Uninstaller

.DESCRIPTION
    Installs or uninstalls the agent-switch (ags) CLI tool for Windows x64.
    Installs into $env:LOCALAPPDATA\Programs\agent-switch and configures User PATH.

.EXAMPLE
    irm https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.ps1 | iex
    .\install.ps1 -Uninstall
    .\install.ps1 -Uninstall -CleanData
    .\install.ps1 -Version 0.1.0
#>

[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$CleanData,
    [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$Repo = "Skyxim/agent-switch"
$BinName = "ags.exe"
$TargetTriple = "x86_64-pc-windows-msvc"
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\agent-switch"
$TargetExe = Join-Path $InstallDir $BinName

# Ensure TLS 1.2 is enabled
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Write-Banner {
    Write-Host @"
    ___                    __     ______          _ __       __  
   /   | ____ ____  ____  / /_   / ___/      __  (_) /______/ /_ 
  / /| |/ __ `/ _ \/ __ \/ __/   \__ \ | | /| / / / __/ ___/ __ \
 / ___ / /_/ /  __/ / / / /_    ___/ / | |/ |/ / / /_/ /__/ / / /
/_/  |_\__, /\___/_/ /_/\__/   /____/  |__/|__/_/\__/\___/_/ /_/ 
      /____/                                                     
"@ -ForegroundColor Cyan
    Write-Host "Fast, encrypted CLI switcher for AI agent credentials`n" -ForegroundColor DarkGray
}

function Write-Step([string]$Message) {
    Write-Host "==> " -ForegroundColor Blue -NoNewline
    Write-Host $Message -ForegroundColor White
}

function Write-Success([string]$Message) {
    Write-Host "success: " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Info([string]$Message) {
    Write-Host "info: " -ForegroundColor Cyan -NoNewline
    Write-Host $Message
}

function Write-Warn([string]$Message) {
    Write-Host "warning: " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

# Uninstall workflow
function Do-Uninstall {
    Write-Step "Uninstalling agent-switch ($BinName)..."
    $removed = $false

    if (Test-Path -LiteralPath $TargetExe) {
        Remove-Item -LiteralPath $TargetExe -Force
        Write-Info "Removed binary: $TargetExe"
        $removed = $true
    }

    if (Test-Path -LiteralPath $InstallDir) {
        # Check if directory has other files; if not, remove directory
        $remaining = Get-ChildItem -LiteralPath $InstallDir -Force -ErrorAction SilentlyContinue
        if (-not $remaining) {
            Remove-Item -LiteralPath $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
            Write-Info "Removed directory: $InstallDir"
        }
    }

    # Clean User PATH
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath) {
        $normalizedInstall = $InstallDir.TrimEnd('\')
        $parts = $userPath -split ';' | Where-Object {
            $_.Trim() -ne "" -and $_.Trim().TrimEnd('\') -ine $normalizedInstall
        }
        $newUserPath = $parts -join ';'
        if ($newUserPath -ne $userPath) {
            [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
            Write-Info "Removed $InstallDir from User PATH."
        }
    }

    # Update current session PATH
    $sessionParts = $env:Path -split ';' | Where-Object {
        $_.Trim() -ne "" -and $_.Trim().TrimEnd('\') -ine $InstallDir.TrimEnd('\')
    }
    $env:Path = $sessionParts -join ';'

    # Handle data directory
    $dataDir = if ($env:AGS_HOME) { $env:AGS_HOME } else { Join-Path $env:USERPROFILE ".config\agent-switch" }
    if ($CleanData) {
        if (Test-Path -LiteralPath $dataDir) {
            Remove-Item -LiteralPath $dataDir -Recurse -Force
            Write-Info "Removed configuration & data directory: $dataDir"
        } else {
            Write-Info "Data directory $dataDir does not exist."
        }
    } else {
        if (Test-Path -LiteralPath $dataDir) {
            Write-Info "Configuration & data directory preserved: $dataDir"
            Write-Info "Use -CleanData to remove it."
        }
    }

    if ($removed) {
        Write-Success "agent-switch has been successfully uninstalled."
    } else {
        Write-Warn "No installed binary found at $TargetExe."
    }
}

# Resolve latest version from GitHub
function Get-LatestVersion {
    # 1. Try GitHub Releases API
    try {
        $headers = @{ "User-Agent" = "agent-switch-installer" }
        $apiUrl = "https://api.github.com/repos/$Repo/releases/latest"
        $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers -Method Get -TimeoutSec 10 -ErrorAction Stop
        if ($release -and $release.tag_name) {
            return $release.tag_name
        }
    } catch {
        # API might fail or rate limit, fall back to redirect check
    }

    # 2. Try redirect header from releases/latest
    try {
        $req = [System.Net.WebRequest]::Create("https://github.com/$Repo/releases/latest")
        $req.Method = "HEAD"
        $req.AllowAutoRedirect = $false
        $req.UserAgent = "agent-switch-installer"
        $response = $req.GetResponse()
        $location = $response.GetResponseHeader("Location")
        $response.Close()

        if ($location -match '/releases/tag/(.+)$') {
            return $Matches[1]
        }
    } catch {
        # Fall through
    }

    return $null
}

Write-Banner

if ($Uninstall) {
    Do-Uninstall
    exit 0
}

# 1. Architecture Check
Write-Step "Checking system architecture..."
if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Host "error: agent-switch requires a 64-bit Windows operating system (x64)." -ForegroundColor Red
    exit 1
}
Write-Info "Detected Windows x64 ($TargetTriple)."

# 2. Resolve Version
Write-Step "Resolving agent-switch release version..."
$Tag = ""
$CleanVer = ""

if ($Version) {
    $CleanVer = $Version.TrimStart('v')
    $Tag = "v$CleanVer"
    Write-Info "Using specified version: $Tag"
} else {
    $Tag = Get-LatestVersion
    if (-not $Tag) {
        Write-Host "error: Failed to retrieve the latest version of $Repo from GitHub." -ForegroundColor Red
        Write-Info "You can specify a version manually: .\install.ps1 -Version <VER>"
        exit 1
    }
    $CleanVer = $Tag.TrimStart('v')
    Write-Info "Latest release version: $Tag"
}

# 3. Download Release Asset
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("ags_install_" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

try {
    Write-Step "Downloading release asset..."
    $CandidateUrls = @(
        "https://github.com/$Repo/releases/download/$Tag/ags-$CleanVer-$TargetTriple.zip",
        "https://github.com/$Repo/releases/download/$Tag/ags-$Tag-$TargetTriple.zip",
        "https://github.com/$Repo/releases/download/$Tag/ags-$TargetTriple.zip"
    )

    $DownloadedFile = Join-Path $TempDir "asset_download"
    $DownloadedSuccess = $false

    foreach ($url in $CandidateUrls) {
        Write-Info "Trying asset: $url"
        try {
            Invoke-WebRequest -Uri $url -OutFile $DownloadedFile -UseBasicParsing -TimeoutSec 30 -ErrorAction Stop
            if ((Test-Path -LiteralPath $DownloadedFile) -and ((Get-Item -LiteralPath $DownloadedFile).Length -gt 0)) {
                $DownloadedSuccess = $true
                Write-Success "Successfully downloaded asset from: $url"
                break
            }
        } catch {
            # Try next candidate
        }
    }

    if (-not $DownloadedSuccess) {
        Write-Host "error: Failed to download release asset for $TargetTriple ($Tag)." -ForegroundColor Red
        Write-Info "Please verify that a release exists at: https://github.com/$Repo/releases"
        exit 1
    }

    # 4. Extract Binary
    Write-Step "Extracting binary..."
    $ExtractDir = Join-Path $TempDir "extracted"
    Expand-Archive -LiteralPath $DownloadedFile -DestinationPath $ExtractDir -Force
    $FoundItems = Get-ChildItem -Path $ExtractDir -Filter $BinName -Recurse -File
    $SourceExe = if ($FoundItems.Count -gt 0) { $FoundItems[0].FullName } else { $null }

    if (-not $SourceExe -or -not (Test-Path -LiteralPath $SourceExe)) {
        Write-Host "error: Could not find '$BinName' in downloaded asset." -ForegroundColor Red
        exit 1
    }

    # 5. Install to LocalAppData
    Write-Step "Installing binary..."
    if (-not (Test-Path -LiteralPath $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    Copy-Item -LiteralPath $SourceExe -Destination $TargetExe -Force
    Write-Success "Installed $BinName to $TargetExe"

    # 6. Configure User PATH
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = if ($userPath) { $userPath -split ';' | Where-Object { $_.Trim() -ne "" } } else { @() }
    $normalizedInstall = $InstallDir.TrimEnd('\')
    $alreadyInPath = $false

    foreach ($p in $pathParts) {
        if ($p.TrimEnd('\') -ieq $normalizedInstall) {
            $alreadyInPath = $true
            break
        }
    }

    if (-not $alreadyInPath) {
        $newUserPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
        Write-Success "Added $InstallDir to User PATH environment variable."
    }

    # Update current PowerShell process PATH
    $sessionParts = $env:Path -split ';' | Where-Object { $_.Trim() -ne "" }
    $alreadyInSession = $false
    foreach ($p in $sessionParts) {
        if ($p.TrimEnd('\') -ieq $normalizedInstall) {
            $alreadyInSession = $true
            break
        }
    }
    if (-not $alreadyInSession) {
        $env:Path = "$InstallDir;$env:Path"
    }

    Write-Host ""
    Write-Success "Installation complete! 'ags' is now available in your terminal."
    Write-Host @"

Quick Start:
  ags info                 # View environment and accounts status
  ags codex add [alias]    # Add current Codex account
  ags agy add [alias]      # Add current Antigravity account
  ags --help               # View all available commands

To uninstall:
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/$Repo/main/install.ps1))) -Uninstall
  # or if cloned locally:
  .\install.ps1 -Uninstall
"@ -ForegroundColor Gray

} finally {
    if (Test-Path -LiteralPath $TempDir) {
        Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
