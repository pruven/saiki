param(
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string] $Target
)

$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    $targetArgs = @()
    if ($Target) { $targetArgs = @('--target', $Target) }
    cargo test --locked @targetArgs @args
    if ($LASTEXITCODE -ne 0) { throw 'Cargo tests failed' }
} finally {
    Pop-Location
}
