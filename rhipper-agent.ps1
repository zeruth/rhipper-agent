# Builds and launches the Rhipper Agent console from the repo root. A console that is still
# running is told to quit first (through var/console.quit) and stopped when it does not, so the
# build never finds the binary locked; no model sees any of this.

Set-Location $PSScriptRoot
$pidFile = "var\console.pid"
$quitFile = "var\console.quit"
if (Test-Path $pidFile) {
    $running = Get-Content $pidFile -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force var | Out-Null
    Set-Content -Path $quitFile -Value "" -Encoding ascii
    $deadline = (Get-Date).AddSeconds(3)
    while ((Test-Path $pidFile) -and ((Get-Date) -lt $deadline)) { Start-Sleep -Milliseconds 100 }
    if (Test-Path $pidFile) {
        if ($running) { Stop-Process -Id $running -Force -ErrorAction SilentlyContinue }
        Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $quitFile -Force -ErrorAction SilentlyContinue
}
cargo run -p rc -- console --replace
