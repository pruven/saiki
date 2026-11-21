$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo test --locked @args
    if ($LASTEXITCODE -ne 0) { throw 'Cargo tests failed' }
} finally {
    Pop-Location
}
