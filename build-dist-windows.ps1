# Build Windows release binary and package it into cloudflare-web/dist/
# Run this script from the repo root (starcode-cli-main/)

$ErrorActionPreference = 'Stop'
$Root = $PSScriptRoot

Write-Host "Building starcode-cli for Windows (release)..."
Push-Location (Join-Path $Root "starcode-cli")
cargo build --release --locked
Pop-Location

# Extract version from Cargo.toml (single source of truth)
$CargoToml = Join-Path $Root "starcode-cli\Cargo.toml"
$VersionLine = Select-String -Path $CargoToml -Pattern '^version' | Select-Object -First 1
$Version = ($VersionLine.Line -split '"')[1]
$DistDir = Join-Path $Root "cloudflare-web\dist"
"v$Version" | Out-File -FilePath (Join-Path $DistDir "version.txt") -Encoding ascii -NoNewline
Write-Host "Version: v$Version"

$BinSrc = Join-Path $Root "starcode-cli\target\release\starcode-cli.exe"
$BinDst  = Join-Path $DistDir "starcode-cli.exe"
$Archive = Join-Path $DistDir "starcode-cli-windows-x86_64.zip"

Copy-Item $BinSrc -Destination $BinDst -Force

if (Test-Path $Archive) { Remove-Item $Archive -Force }
Compress-Archive -Path $BinDst -DestinationPath $Archive
Remove-Item $BinDst -Force

Write-Host ""
Write-Host "Done: $Archive (v$Version)" -ForegroundColor Green
