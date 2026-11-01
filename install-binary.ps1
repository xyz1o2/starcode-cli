# Downloads and installs a prebuilt StarCode CLI binary (Windows).
#
# Usage (the recommended entry point from the README):
#   irm https://raw.githubusercontent.com/xyz1o2/starcode-cli/dev-m/install-binary.ps1 | iex
#
# To build from source instead, use .\install.ps1.
#
# Notes:
# - This file is intentionally pure ASCII (no CJK comments): Windows
#   PowerShell 5.1 decodes BOM-less UTF-8 files with the system ANSI codepage,
#   which can corrupt multi-byte characters and even swallow line breaks,
#   breaking the whole script. Keep it ASCII-only.
# - Never use `exit` here: when the script runs through `irm | iex`, `exit`
#   would close the user's PowerShell window. Abort with `throw` instead.

$ErrorActionPreference = "Stop"

$Repo = "xyz1o2/starcode-cli"
$BinName = "starcode-cli"
$Aliases = @("sc", "starcode")

# ---- platform detection --------------------------------------------------
$Arch = $env:PROCESSOR_ARCHITECTURE
switch ($Arch) {
    "AMD64" { $Targets = @("x86_64-pc-windows-msvc") }
    "ARM64" { $Targets = @("aarch64-pc-windows-msvc") }
    default { throw "unsupported Windows arch: $Arch" }
}

# ---- fetch the latest release asset list ---------------------------------
try {
    $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
        -Headers @{ "User-Agent" = "starcode-cli-installer" }
} catch {
    throw "failed to fetch release info: $_"
}

# Asset names look like starcode-cli-<version>-<target>.zip (the version part
# may be empty), so match by the "-<target>.zip" suffix to cover both namings.
$Asset = $null
foreach ($t in $Targets) {
    $Asset = $Release.assets | Where-Object { $_.name -like "*-$t.zip" } | Select-Object -First 1
    if ($Asset) { break }
}
if (-not $Asset) { throw "latest release has no archive for $Arch" }

Write-Host "Latest release: $($Asset.name)"

# ---- download -------------------------------------------------------------
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) "starcode-install-$(Get-Random)"
New-Item -ItemType Directory -Path $Tmp -Force | Out-Null

try {
    $Zip = Join-Path $Tmp $Asset.name
    Write-Host "Downloading ..."
    Invoke-WebRequest -Uri $Asset.browser_download_url -OutFile $Zip

    # ---- verify SHA256 against SHA256SUMS.txt -----------------------------
    $SumsAsset = $Release.assets | Where-Object { $_.name -eq "SHA256SUMS.txt" } | Select-Object -First 1
    if ($SumsAsset) {
        try {
            $SumsFile = Join-Path $Tmp "SHA256SUMS.txt"
            Invoke-WebRequest -Uri $SumsAsset.browser_download_url -OutFile $SumsFile
            $line = Get-Content $SumsFile | Where-Object { $_ -like "*$($Asset.name)" } | Select-Object -First 1
            if ($line) {
                $expected = ($line -split '\s+')[0].ToLower()
                $actual = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLower()
                if ($actual -ne $expected) {
                    throw "sha256 mismatch for $($Asset.name) (expected $expected, got $actual)"
                }
                Write-Host "  checksum verified (sha256)"
            }
        } catch {
            # A failed/partial checksum step must not block the install.
            Write-Host "  [!] checksum verification skipped: $_" -ForegroundColor Yellow
        }
    }

    # ---- extract ------------------------------------------------------------
    Expand-Archive -Path $Zip -DestinationPath $Tmp -Force
    $Exe = Get-ChildItem -Path $Tmp -Recurse -Filter "$BinName.exe" -File | Select-Object -First 1
    if (-not $Exe) { throw "$BinName.exe not found in the archive" }

    # ---- install (same convention as install.ps1) ---------------------------
    $InstallDir = Join-Path $env:USERPROFILE ".cargo\bin"
    if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir | Out-Null }

    $TargetExe = Join-Path $InstallDir "$BinName.exe"
    if (Test-Path $TargetExe) { Remove-Item $TargetExe -Force }
    Copy-Item $Exe.FullName $TargetExe -Force
    Write-Host "  $BinName -> $InstallDir"

    # Hard links cost no extra disk; rebuild them every install so they never
    # point at a previous binary.
    foreach ($alias in $Aliases) {
        $Link = Join-Path $InstallDir "$alias.exe"
        Remove-Item $Link -Force -ErrorAction SilentlyContinue
        try {
            New-Item -ItemType HardLink -Path $Link -Target $TargetExe -ErrorAction Stop | Out-Null
            Write-Host "  $alias -> $BinName"
        } catch {
            Copy-Item $TargetExe $Link -Force
            Write-Host "  $alias (copy - hard link not available)"
        }
    }

    Write-Host ""
    Write-Host "Installation successful!" -ForegroundColor Green
    Write-Host "Installed to: $InstallDir"
    if (($env:PATH -split ';') -notcontains $InstallDir) {
        Write-Host ""
        Write-Host "Note: '$InstallDir' is not on your PATH yet. Add it with:"
        Write-Host "  [Environment]::SetEnvironmentVariable('Path', '$InstallDir;' + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
    }
    Write-Host "Run 'sc' to start (or 'starcode' / '$BinName')."
}
finally {
    Remove-Item -Path $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
