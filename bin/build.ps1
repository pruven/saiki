$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo build --locked --release
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed' }
    New-Item -ItemType Directory -Force build | Out-Null
    Copy-Item target/release/saiki.exe build/saiki.exe -Force
} finally {
    Pop-Location
}
