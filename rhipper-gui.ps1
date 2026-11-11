# Builds, installs and opens the Rhipper Agent window on this repo, from the repo root.
# The build is installed (`rc install`) into a directory on the path and the window runs from
# there, so a running window never locks target/ against the next build. One window hosts every
# project as a tab and remembers them: `rc restart` brings the running window up again on the
# new build with the same tabs, and `rc open .` adds this project when it is not one of them.

Set-Location $PSScriptRoot
[Environment]::SetEnvironmentVariable('NO_COLOR', $null)

cargo build -q -p rc -p rc-gui
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& .\target\debug\rc.exe install
$rc = Join-Path $env:LOCALAPPDATA 'Programs\rhipper\rc.exe'
& $rc restart
& $rc open .
