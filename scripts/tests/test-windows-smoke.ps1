# Exercise a Windows release build on real work: queries, refresh after an edit, a local index,
# clean, and install.ps1 against a local release. Run from the repository root by CI.
# Usage: scripts/tests/test-windows-smoke.ps1
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$binary = (Resolve-Path 'target\release\xcalygraph.exe').Path
$work = Join-Path ([IO.Path]::GetTempPath()) ("xcalygraph-smoke-" + [IO.Path]::GetRandomFileName())
$root = Join-Path $work 'repo'
$index = Join-Path $work 'index'
New-Item -ItemType Directory -Path $root | Out-Null
Set-Content (Join-Path $root 'go.mod') "module example.com/app`n`ngo 1.23`n"
Set-Content (Join-Path $root 'main.go') "package main`n`nfunc Helper() {}`n`nfunc main() { Helper() }`n"

function Query {
    param([Parameter(ValueFromRemainingArguments)] [string[]] $Arguments)
    $text = (& $binary @Arguments -C $root --index-dir $index | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "query failed: $Arguments" }
    return $text
}
function Expect([string] $Text, [string] $Pattern) {
    if ($Text -notmatch $Pattern) { throw "expected '$Pattern' in:`n$Text" }
}

Expect (Query map) 'main\.go:5'
Expect (Query find Helper) 'main\.go'
Expect (Query show Helper) '< .*main\.go:5'
Expect (Query impact Helper) '< 1 '
# Forward slashes in every path, whatever the platform.
Set-Content (Join-Path $root 'sub.go') "package main`n`nfunc Added() { Helper() }`n"
Expect (Query find Added) 'sub\.go'
Expect (Query show Helper) 'sub\.go:3'
Expect (& $binary clean $root --index-dir $index | Out-String) 'deleted'

# install.ps1 against a local release, into a private HOME.
$version = ((& $binary version | Select-Object -First 1) -split ' ')[1]
$name = "xcalygraph-$version-windows-amd64"
$release = Join-Path $work "release\v$version"
$package = Join-Path $work $name
New-Item -ItemType Directory -Path $release, $package | Out-Null
Copy-Item $binary $package
tar.exe -czf (Join-Path $release "$name.tar.gz") -C $work $name
$sum = (Get-FileHash -Algorithm SHA256 (Join-Path $release "$name.tar.gz")).Hash.ToLowerInvariant()
Set-Content (Join-Path $release 'SHA256SUMS') "$sum  $name.tar.gz"
$homeDir = Join-Path $work 'home'
New-Item -ItemType Directory -Path $homeDir | Out-Null
$env:HOME = $homeDir
$env:XCALYGRAPH_VERSION = $version
$env:XCALYGRAPH_RELEASE_BASE = 'file:///' + ((Join-Path $work 'release') -replace '\\', '/')
$env:XCALYGRAPH_INSTALL_DIR = Join-Path $homeDir 'bin'
& "$PSScriptRoot\..\utils\install.ps1"
if (-not (Test-Path (Join-Path $homeDir 'bin\xcalygraph.exe'))) { throw 'binary not installed' }
if (-not (Test-Path (Join-Path $homeDir '.agents\skills\xcalygraph\SKILL.md'))) { throw 'skill not installed' }
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Write-Host 'PASS: Windows queries, refresh, clean and installer'
