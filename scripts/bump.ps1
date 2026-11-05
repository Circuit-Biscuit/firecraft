# Ship a new version: sets the version, commits and pushes. The Release workflow then builds
# and publishes installers for every platform automatically.
#
#   powershell -ExecutionPolicy Bypass -File scripts\bump.ps1 0.5.0
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 0.5.0" }

$toml = Get-Content Cargo.toml -Raw
$toml = ([regex]'(?m)^version = "[^"]+"').Replace($toml, "version = `"$Version`"", 1)  # first one = the package version
[IO.File]::WriteAllText((Join-Path (Get-Location) 'Cargo.toml'), $toml)  # UTF-8 without BOM
cargo check --quiet   # refreshes Cargo.lock with the new version
git add Cargo.toml Cargo.lock
git commit -m "Release v$Version"
git push
Write-Host "Pushed v$Version - the release appears under Releases in ~15 minutes."
