# 只安装一个二进制，`sc` 和 `starcode` 做成指向它的链接。
# 声明三个 [[bin]] 也行，但一个 release 二进制约 55 MB，那样要链三遍、占三份磁盘。
$BinName = "starcode-cli"
$Aliases = @("sc", "starcode")

Write-Host "Installing $BinName..."
cargo install --path .
if (-not $?) {
    Write-Host "Installation failed." -ForegroundColor Red
    exit 1
}

$InstallDir = Join-Path $env:USERPROFILE ".cargo\bin"
$Target = Join-Path $InstallDir "$BinName.exe"

if (-not (Test-Path $Target)) {
    Write-Host "Installed, but could not find $Target to alias." -ForegroundColor Yellow
    Write-Host "You can still run '$BinName'." -ForegroundColor Yellow
    exit 0
}

# 每次安装都重建别名：cargo install 覆盖二进制时会换掉文件本体，
# 上一轮建的硬链接会指向旧内容。
foreach ($alias in $Aliases) {
    $Link = Join-Path $InstallDir "$alias.exe"
    Remove-Item $Link -Force -ErrorAction SilentlyContinue
    try {
        # 硬链接不额外占磁盘；Windows 上的符号链接要管理员权限或开发者模式，所以不用它
        New-Item -ItemType HardLink -Path $Link -Target $Target -ErrorAction Stop | Out-Null
        Write-Host "  $alias -> $BinName"
    } catch {
        Copy-Item $Target $Link -Force
        Write-Host "  $alias (copy — hard link not available)"
    }
}

Write-Host "Installation successful!" -ForegroundColor Green
Write-Host "Run 'sc' to start (or 'starcode' / '$BinName')."
