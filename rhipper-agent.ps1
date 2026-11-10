# Builds and launches the Rhipper Agent console from the repo root.

Set-Location $PSScriptRoot
cargo run -p rc -- console
