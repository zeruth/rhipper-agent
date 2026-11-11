# Builds, installs and launches the Rhipper Agent workspace, the window, from the repo root.
# The build is installed (`rc install`) into a directory on the path and the window runs from
# there, so a running instance never locks target/ against the next build. The window is another
# instance of the agent: a console or an older window that is still running is told to quit
# first (through var/console.quit) and stopped when it does not; no model sees any of this.

Set-Location $PSScriptRoot
[Environment]::SetEnvironmentVariable('NO_COLOR', $null)

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

cargo build -q -p rc -p rc-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& .\target\debug\rc.exe install
$installed = Join-Path $env:LOCALAPPDATA 'Programs\rhipper\rc-gui.exe'
Start-Process -FilePath $installed -WorkingDirectory $PSScriptRoot
