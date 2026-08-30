$ErrorActionPreference = 'Stop'

$BaseUrl = "https://starcode.help"
$Asset = "starcode-cli-windows-x86_64.zip"
$InstallDir = Join-Path $env:LOCALAPPDATA "starcode-cli"
$DownloadUrl = "$BaseUrl/dist/$Asset"

$Version = try { (Invoke-RestMethod "$BaseUrl/dist/version.txt").Trim() } catch { "latest" }
$DownloadUrlWithTimestamp = "$DownloadUrl`?v=$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())"

Write-Host "Downloading starcode-cli $Version for Windows..."
$TmpDir = Join-Path $env:TEMP ("starcode-cli-" + [System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $TmpDir | Out-Null
$ZipPath = Join-Path $TmpDir $Asset

Invoke-WebRequest -Uri $DownloadUrlWithTimestamp -OutFile $ZipPath -UseBasicParsing
Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir | Out-Null
}
Copy-Item (Join-Path $TmpDir "starcode-cli.exe") -Destination $InstallDir -Force
Remove-Item $TmpDir -Recurse -Force

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    Write-Host "Added $InstallDir to your PATH."
}
if ($env:Path -notlike "*$InstallDir*") {
    $env:Path = "$env:Path;$InstallDir"
}

Write-Host ""
Write-Host "starcode-cli $Version installed to $InstallDir" -ForegroundColor Green
Write-Host "Run: starcode-cli --help"
