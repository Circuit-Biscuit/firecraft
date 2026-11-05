# One-time setup: creates the private GitHub repository under your organization and pushes.
# GitHub Actions then builds the Windows, macOS and Linux installers and publishes release v<version>.
#
#   powershell -ExecutionPolicy Bypass -File scripts\publish.ps1
#
# Needs the GitHub CLI logged in (gh auth login). Change -Org / -Name if you like.
param(
    [string]$Org = "Circuit Biscuit",
    [string]$Name = "firecraft"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

gh auth status | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Log in first: gh auth login" }

# find the organization by its login or display name ("Circuit Biscuit" -> circuit-biscuit, CircuitBiscuit, ...)
$want = ($Org -replace '[^A-Za-z0-9]', '').ToLower()
$orgs = @(gh api user/orgs --paginate --jq '.[].login')
$login = $orgs | Where-Object { ($_ -replace '[^A-Za-z0-9]', '').ToLower() -eq $want } | Select-Object -First 1
if (-not $login) {
    foreach ($o in $orgs) {
        $display = gh api "orgs/$o" --jq '.name'
        if ((($display -replace '[^A-Za-z0-9]', '').ToLower()) -eq $want) { $login = $o; break }
    }
}
if (-not $login) { throw "Organization '$Org' not found. Your organizations: $($orgs -join ', ')" }

gh repo create "$login/$Name" --private --source . --remote origin --push `
    --description "Real-time fire, smoke and explosion simulator with a node graph. Rust + wgpu."
if ($LASTEXITCODE -ne 0) { throw "Creating the repository failed." }

Write-Host ""
Write-Host "Published: https://github.com/$login/$Name"
Write-Host "The first release (installers for Windows, macOS, Linux) builds in ~15 minutes:"
Write-Host "  https://github.com/$login/$Name/actions"
