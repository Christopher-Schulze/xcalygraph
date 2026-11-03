# Install xcalygraph from a GitHub release on Windows: verified binary in ~\.local\bin, agent skill
# in ~\.agents\skills\xcalygraph, linked for Claude Code when ~\.claude exists.
# Usage (attached to every release, never fetched from main):
#   irm https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.ps1 | iex
#   XCALYGRAPH_VERSION=X.Y.Z     a specific version (default: latest)
#   XCALYGRAPH_INSTALL_DIR=DIR   install directory (default ~\.local\bin)
#   XCALYGRAPH_RELEASE_BASE=URL  release download base (default GitHub; file:// for tests)
# Verifies the archive against SHA256SUMS and smoke-tests the binary before it replaces an
# existing one. PATH is never edited: a missing entry is reported.
$ErrorActionPreference = 'Stop'

$repository = 'Christopher-Schulze/xcalygraph'
$base = if ($env:XCALYGRAPH_RELEASE_BASE) { $env:XCALYGRAPH_RELEASE_BASE } else { "https://github.com/$repository/releases/download" }
$binDir = if ($env:XCALYGRAPH_INSTALL_DIR) { $env:XCALYGRAPH_INSTALL_DIR } else { Join-Path $HOME '.local\bin' }

$version = $env:XCALYGRAPH_VERSION
if (-not $version) {
    Write-Host '>> resolving the latest release'
    $version = (Invoke-RestMethod "https://api.github.com/repos/$repository/releases/latest").tag_name
    if (-not $version) { throw 'install.ps1: no published release found' }
}
$version = $version.TrimStart('v')
if ($version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$') { throw "install.ps1: invalid version $version" }

# Windows on Arm runs the x86-64 binary through emulation.
$name = "xcalygraph-$version-windows-amd64"
$work = Join-Path ([IO.Path]::GetTempPath()) ("xcalygraph-install-" + [IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host ">> downloading $name.tar.gz"
    $archive = Join-Path $work "$name.tar.gz"
    $sums = Join-Path $work 'SHA256SUMS'
    curl.exe -fsSL -o $archive "$base/v$version/$name.tar.gz"
    if ($LASTEXITCODE -ne 0) { throw "install.ps1: download failed: $base/v$version/$name.tar.gz" }
    curl.exe -fsSL -o $sums "$base/v$version/SHA256SUMS"
    if ($LASTEXITCODE -ne 0) { throw "install.ps1: download failed: $base/v$version/SHA256SUMS" }

    Write-Host '>> verifying SHA-256'
    $expected = (Get-Content $sums | Where-Object { $_ -match "\s\*?$([regex]::Escape("$name.tar.gz"))$" } | ForEach-Object { ($_ -split '\s+')[0] }) | Select-Object -First 1
    if (-not $expected) { throw "install.ps1: SHA256SUMS lists no $name.tar.gz" }
    $actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
    if ($expected -ne $actual) { throw "install.ps1: checksum mismatch for $name.tar.gz" }

    tar.exe -xzf $archive -C $work
    if ($LASTEXITCODE -ne 0) { throw 'install.ps1: cannot unpack the archive' }
    $candidate = Join-Path $work "$name\xcalygraph.exe"
    if (-not (Test-Path $candidate)) { throw 'install.ps1: archive contains no xcalygraph binary' }

    Write-Host '>> smoke test'
    $reported = (& $candidate version | Select-Object -First 1)
    if ($reported -ne "xcalygraph $version") { throw "install.ps1: downloaded binary failed its smoke test (reported '$reported'); nothing was replaced" }

    # The binary installs itself, the skill and the Claude Code link, and reports PATH problems.
    $env:XCALYGRAPH_INSTALL_DIR = $binDir
    & $candidate install
    if ($LASTEXITCODE -ne 0) { throw 'install.ps1: installation failed' }
    Write-Host ">> xcalygraph $version installed"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
