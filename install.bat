@echo off
setlocal EnableExtensions EnableDelayedExpansion

echo Installing starcode-cli...

:: Switch to the script's directory so relative paths resolve correctly.
cd /d "%~dp0"

:: Check that cargo is available.
where cargo >nul 2>&1
if !ERRORLEVEL! NEQ 0 (
    echo Error: cargo not found. Please install Rust from https://rustup.rs
    exit /b 1
)

if not defined CARGO_REGISTRIES_CRATES_IO_PROTOCOL set "CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse"
if not defined CARGO_HTTP_TIMEOUT set "CARGO_HTTP_TIMEOUT=120"
if not defined CARGO_NET_RETRY set "CARGO_NET_RETRY=10"
if not defined CARGO_TERM_COLOR set "CARGO_TERM_COLOR=always"
if not defined CARGO_TARGET_DIR set "CARGO_TARGET_DIR=%~dp0target"
:: Limit parallel compilation to avoid CPU overload (default: half of logical cores)
if not defined CARGO_BUILD_JOBS (
    set /a "HALF_CORES=%NUMBER_OF_PROCESSORS%/2" 2>nul
    if !HALF_CORES! LSS 2 set HALF_CORES=2
    set "CARGO_BUILD_JOBS=!HALF_CORES!"
)

echo   protocol=%CARGO_REGISTRIES_CRATES_IO_PROTOCOL%
echo   timeout=%CARGO_HTTP_TIMEOUT%s
echo   retry=%CARGO_NET_RETRY%
echo   target=%CARGO_TARGET_DIR%
echo   jobs=%CARGO_BUILD_JOBS%
echo.

:: Step 1: Build release with CPU-friendly settings
echo [1/2] Building release...
cargo build --release --bin starcode-cli ^
    --config "profile.release.opt-level=2" ^
    --config "profile.release.lto=\"off\"" ^
    --config "profile.release.codegen-units=32"
if !ERRORLEVEL! NEQ 0 (
    echo Build failed.
    exit /b 1
)

:: Step 2: Install from already-built binary (--offline skips rebuild)
echo.
echo [2/2] Installing binary...
cargo install --path . --bin starcode-cli --force --offline
if !ERRORLEVEL! EQU 0 goto install_success

:: Fallback: if offline install fails, try online (rare, registry index may be stale)
echo Offline install failed, trying online install...
cargo install --path . --bin starcode-cli --force
if !ERRORLEVEL! EQU 0 goto install_success

goto install_failed

:install_success
echo.
echo Installation successful!
echo You can now run 'starcode-cli' from anywhere.
exit /b 0

:install_failed
echo.
echo Installation failed.
echo Try running manually:
echo   cargo build --release
echo   cargo install --path . --force
exit /b 1
