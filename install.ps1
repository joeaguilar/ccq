<#
.SYNOPSIS
    Install or update the ccq CLI on Windows from the latest GitHub Release.

.DESCRIPTION
    Downloads a prebuilt ccq binary matching the host architecture
    (x86_64 or arm64), verifies its SHA256 checksum, and installs it
    into a directory on PATH.

    When -Update is supplied (or the positional `update` argument), the
    installer prefers an existing ccq.exe already on PATH and replaces it
    in-place rather than always writing to $InstallDir. This mirrors the
    `install.sh --update` behavior on Unix.

.PARAMETER Version
    Pin a specific release tag (e.g. v0.1.0). Defaults to the latest.

.PARAMETER InstallDir
    Install location. Defaults to $env:LOCALAPPDATA\Programs\ccq. When
    -Update is set and an existing ccq.exe is found on PATH, that location
    wins over the default so the shell keeps resolving the same binary.

.PARAMETER Repo
    GitHub repo slug. Defaults to joeaguilar/ccq.

.PARAMETER Update
    Update an existing ccq install if found on PATH; otherwise install it.
    The positional value `update` (or `install`) is also accepted, e.g.
    `.\install.ps1 update`.

.EXAMPLE
    iwr -useb https://raw.githubusercontent.com/joeaguilar/ccq/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version v0.1.0 -InstallDir C:\tools\ccq

.EXAMPLE
    .\install.ps1 -Update

.EXAMPLE
    .\install.ps1 update

.NOTES
    Manual checksum verification:
    - In Windows PowerShell 5.1 and PowerShell 7, run against a test release
      that contains the ccq zip asset but no .sha256 asset. The installer should
      warn "Checksum file not available" and continue.
    - Run against a test release with an incorrect .sha256 asset. The installer
      should fail with "Checksum mismatch" before extraction or installation.
#>

[CmdletBinding()]
param(
    [string]$Version = $env:CCQ_VERSION,
    [string]$InstallDir = $env:CCQ_INSTALL_DIR,
    [string]$Repo = $(if ($env:CCQ_REPO) { $env:CCQ_REPO } else { 'joeaguilar/ccq' }),
    [switch]$Update,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Action
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Windows PowerShell 5.1 still negotiates TLS 1.0 on some hosts and GitHub
# requires 1.2+. No-op on PowerShell 7+, which already defaults higher.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

function Write-Info    { param([string]$m) Write-Host "i $m" -ForegroundColor Blue }
function Write-Ok      { param([string]$m) Write-Host "+ $m" -ForegroundColor Green }
function Write-Warn    { param([string]$m) Write-Host "! $m" -ForegroundColor Yellow }
function Write-Err     { param([string]$m) Write-Host "x $m" -ForegroundColor Red }

function Get-Target {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if (-not $arch) { $arch = (Get-CimInstance Win32_Processor).Architecture }
    switch -Regex ($arch) {
        '^(AMD64|x86_64|9)$' { return 'x86_64-pc-windows-msvc' }
        '^(ARM64|12)$'       { return 'aarch64-pc-windows-msvc' }
        default {
            throw "Unsupported architecture: $arch"
        }
    }
}

function Resolve-LatestTag {
    param([string]$Repo)
    # Follow the /releases/latest redirect to avoid the API rate limit.
    $url = "https://github.com/$Repo/releases/latest"
    $tag = $null

    # HttpWebRequest with redirects disabled behaves identically on Windows
    # PowerShell 5.1 and PowerShell 7+, and never engages 5.1's IE-based
    # parser (which fails outright in non-interactive sessions).
    try {
        $req = [System.Net.HttpWebRequest]::Create($url)
        $req.AllowAutoRedirect = $false
        $req.UserAgent = 'ccq-installer'
        $resp = $req.GetResponse()
        try {
            $location = $resp.Headers['Location']
            if ($location) { $tag = ($location -split '/')[-1] }
        } finally {
            $resp.Close()
        }
    } catch {
        $tag = $null
    }

    # Fallback: follow the redirect and read the final URI. BaseResponse is an
    # HttpWebResponse on 5.1 (ResponseUri) but an HttpResponseMessage on 7+
    # (RequestMessage.RequestUri), so probe for both instead of assuming -- a
    # bare property access on the wrong one is fatal under Set-StrictMode.
    if (-not $tag) {
        $resp = Invoke-WebRequest -Uri $url -UseBasicParsing
        $final = $null
        $baseProp = $resp.PSObject.Properties['BaseResponse']
        if ($baseProp -and $baseProp.Value) {
            $baseResp = $baseProp.Value
            $reqMsg = $baseResp.PSObject.Properties['RequestMessage']
            $respUri = $baseResp.PSObject.Properties['ResponseUri']
            if ($reqMsg -and $reqMsg.Value) {
                $final = $reqMsg.Value.RequestUri.AbsoluteUri
            } elseif ($respUri -and $respUri.Value) {
                $final = $respUri.Value.AbsoluteUri
            }
        }
        if (-not $final) {
            throw "Could not resolve latest release tag from $url"
        }
        $tag = ($final -split '/')[-1]
    }
    # When a repo has no published releases, GitHub redirects /releases/latest
    # to /releases, so the last URL segment is the literal string "releases"
    # rather than a real tag. Detect that and fail loudly instead of building
    # a bogus download URL.
    if (-not $tag -or $tag -eq 'releases') {
        throw "No published releases found at https://github.com/$Repo/releases. Publish a release (e.g. v0.1.0) with the Windows zip + sha256 assets, or pin a tag with -Version."
    }
    return $tag
}

function Get-PathEntries {
    param([string]$Scope)
    $v = [Environment]::GetEnvironmentVariable('Path', $Scope)
    if (-not $v) { return @() }
    return @($v -split ';' | Where-Object { $_ -ne '' })
}

# Compare against both User and Machine PATH so a directory already on the
# system PATH is never duplicated into the user scope. Entries are matched
# case-insensitively and ignoring a trailing separator.
function Test-OnPersistentPath {
    param([string]$Dir)
    $target = $Dir.TrimEnd('\')
    foreach ($e in (@(Get-PathEntries 'User') + @(Get-PathEntries 'Machine'))) {
        if ($e.TrimEnd('\') -ieq $target) { return $true }
    }
    return $false
}

function Add-ToUserPath {
    param([string]$Dir)
    if (Test-OnPersistentPath $Dir) { return $false }
    [Environment]::SetEnvironmentVariable(
        'Path', ((@($Dir) + (Get-PathEntries 'User')) -join ';'), 'User')
    return $true
}

# Make ccq resolvable for the rest of this session as well.
function Add-ToSessionPath {
    param([string]$Dir)
    $target = $Dir.TrimEnd('\')
    foreach ($e in ($env:Path -split ';' | Where-Object { $_ -ne '' })) {
        if ($e.TrimEnd('\') -ieq $target) { return }
    }
    $env:Path = "$Dir;$env:Path"
}

function Get-ExistingCcqPath {
    # Resolve the ccq.exe that the current shell would invoke, if any.
    $cmd = Get-Command ccq -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Path -and (Test-Path $cmd.Path)) {
        return $cmd.Path
    }
    return $null
}

function Get-ExistingCcqInDir {
    param([string]$Dir)
    if (-not $Dir) { return $null }
    $candidate = Join-Path $Dir 'ccq.exe'
    if (Test-Path $candidate) { return $candidate }
    return Get-ExistingCcqPath
}

function Show-ExistingVersion {
    param([string]$BinPath)
    if (-not $BinPath) { return }
    try {
        $ver = & $BinPath --version 2>$null
        if ($ver) {
            Write-Info "Current install: $ver ($BinPath)"
        } else {
            Write-Info "Current install: $BinPath"
        }
    } catch {
        Write-Info "Current install: $BinPath"
    }
}

# ---- Argument normalization ------------------------------------------------

# Accept both `-Update` switch and a positional `update`/`install` token to
# mirror install.sh's `--update` / `update` / `--install` / `install` parsing.
$ActionMode = if ($Update) { 'update' } else { 'install' }
if ($Action) {
    foreach ($arg in $Action) {
        switch -Regex ($arg) {
            '^(--update|update)$'   { $ActionMode = 'update' }
            '^(--install|install)$' { $ActionMode = 'install' }
            '^(-h|--help)$' {
                $scriptPath = $MyInvocation.MyCommand.Path
                if ($scriptPath -and (Test-Path $scriptPath)) {
                    Get-Help $scriptPath -Detailed
                } else {
                    Write-Host 'Usage:'
                    Write-Host '  .\install.ps1 [-Update] [-Version <tag>] [-InstallDir <path>] [-Repo <slug>]'
                    Write-Host '  .\install.ps1 update      # positional form, mirrors install.sh'
                    Write-Host ''
                    Write-Host 'Environment overrides:'
                    Write-Host '  CCQ_VERSION       Pin a specific release tag (defaults to latest).'
                    Write-Host '  CCQ_INSTALL_DIR   Override the install directory.'
                    Write-Host '  CCQ_REPO          Override the GitHub repo slug.'
                }
                exit 0
            }
            default {
                Write-Err "Unknown argument: $arg"
                exit 1
            }
        }
    }
}

# ---- Main ------------------------------------------------------------------

Write-Host ''
if ($ActionMode -eq 'update') {
    Write-Info 'Updating ccq - the read-only Claude Code transcript query CLI'
} else {
    Write-Info 'Installing ccq - the read-only Claude Code transcript query CLI'
}
Write-Host ''

$target = Get-Target
Write-Info "Detected target: $target"

if (-not $Version) {
    $Version = Resolve-LatestTag -Repo $Repo
}
Write-Info "Release: $Version"

# Install-dir selection mirrors install.sh::choose_install_dir:
#   1. Explicit -InstallDir / $env:CCQ_INSTALL_DIR wins.
#   2. Otherwise, if an existing ccq.exe is on PATH, replace it in-place so
#      the shell keeps resolving the same binary (especially for updates,
#      but also for plain installs where the user already has ccq).
#   3. Otherwise fall back to $env:LOCALAPPDATA\Programs\ccq.
$existingCcq = Get-ExistingCcqPath
if (-not $InstallDir) {
    if ($existingCcq) {
        $InstallDir = Split-Path -Parent $existingCcq
        if ($ActionMode -eq 'install') {
            Write-Info "Existing ccq.exe found on PATH - installing alongside it at $InstallDir"
        }
    } else {
        $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\ccq'
    }
}
$InstallDir = [Environment]::ExpandEnvironmentVariables($InstallDir)

$assetBase = "ccq-$Version-$target"
$zipUrl    = "https://github.com/$Repo/releases/download/$Version/$assetBase.zip"
$sumUrl    = "$zipUrl.sha256"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

try {
    $zipPath = Join-Path $tmp "$assetBase.zip"
    $sumPath = Join-Path $tmp "$assetBase.zip.sha256"

    Write-Info "Downloading $assetBase.zip"
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing

    $hasChecksum = $true
    try {
        Invoke-WebRequest -Uri $sumUrl -OutFile $sumPath -UseBasicParsing -ErrorAction Stop
    } catch {
        $statusCode = $null
        if ($_.Exception.Response -and $_.Exception.Response.StatusCode) {
            $statusCode = [int]$_.Exception.Response.StatusCode
        }
        if ($statusCode -eq 404) {
            $hasChecksum = $false
            Write-Warn "Checksum file not available (HTTP 404) - skipping verification."
        } else {
            throw
        }
    }

    if ($hasChecksum) {
        $expected = (Get-Content $sumPath -Raw).Trim().Split()[0].ToLower()
        $actual   = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
        if ($expected -ne $actual) {
            throw "Checksum mismatch: expected $expected, got $actual"
        }
        Write-Ok 'Checksum verified.'
    }

    Write-Info 'Extracting...'
    Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

    $binSrc = Join-Path $tmp "$assetBase\ccq.exe"
    if (-not (Test-Path $binSrc)) {
        throw "Extracted archive is missing ccq.exe"
    }

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    }
    $binDst = Join-Path $InstallDir 'ccq.exe'

    $existingBefore = Get-ExistingCcqInDir -Dir $InstallDir
    Show-ExistingVersion -BinPath $existingBefore

    if ($existingBefore) {
        Write-Info "Updating $binDst"
    } else {
        Write-Info "Installing to $InstallDir"
    }
    Copy-Item -Force $binSrc $binDst
    if ($existingBefore) {
        Write-Ok "Updated $binDst"
    } else {
        Write-Ok "Installed $binDst"
    }

    if (Add-ToUserPath -Dir $InstallDir) {
        Write-Ok "Added $InstallDir to your User PATH (restart your shell to pick it up)."
    }
    Add-ToSessionPath -Dir $InstallDir

    Write-Host ''
    try { & $binDst --version } catch { }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

Write-Host ''
Write-Ok 'Done.'
Write-Host ''
Write-Info 'Quick start:'
Write-Host '  ccq projects                         # list transcript projects'
Write-Host '  ccq stats --since 7d                 # corpus rollup, last week'
Write-Host '  ccq bash -p . --count --by command   # top commands in this project'
Write-Host '  ccq --help                           # all commands'
Write-Host ''
