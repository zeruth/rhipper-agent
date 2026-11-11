# Builds, installs and launches the Rhipper Agent console from the repo root.
#
# The console paints in 24-bit color, which the legacy console host cannot show, so outside
# Windows Terminal the script reopens itself in a Windows Terminal tab when one is installed.
# The build is installed (`rc install`) into a directory on the path and the console runs from
# there, so a running instance never locks target/ against the next build. A console that is
# still running is told to quit first (through var/console.quit) and stopped when it does not;
# no model sees any of this.

Set-Location $PSScriptRoot
# a launcher's NO_COLOR would strip the theme; the console is color or nothing
[Environment]::SetEnvironmentVariable('NO_COLOR', $null)
if (-not $env:WT_SESSION -and (Get-Command wt -ErrorAction SilentlyContinue)) {
    Start-Process wt -ArgumentList @('-d', "`"$PSScriptRoot`"", '--title', '"Rhipper Agent"', 'powershell', '-NoLogo', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSScriptRoot\rhipper-agent.ps1`"")
    exit
}

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
$installed = Join-Path $env:LOCALAPPDATA 'Programs\rhipper\rc.exe'
& $installed console --replace
