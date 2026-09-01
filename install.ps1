Write-Host "Installing starcode-cli..."
cargo install --path .
if ($?) {
    Write-Host "Installation successful!" -ForegroundColor Green
    Write-Host "You can now run 'starcode-cli' from anywhere."
} else {
    Write-Host "Installation failed." -ForegroundColor Red
    exit 1
}
