param(
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string] $Target
)

$ErrorActionPreference = 'Stop'
$previousFxcPath = $env:GPUI_FXC_PATH
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    # GPUI's native ARM64 lookup can miss SDKs that only ship the x64 shader compiler.
    # Windows 11 on ARM can run this build tool while producing shaders for the ARM64 executable.
    if (-not $env:GPUI_FXC_PATH -and -not (Get-Command fxc.exe -ErrorAction SilentlyContinue)) {
        $sdk = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Microsoft SDKs\Windows\v10.0' -ErrorAction SilentlyContinue
        if ($sdk.InstallationFolder) {
            $compilers = Get-ChildItem (Join-Path $sdk.InstallationFolder 'bin') -Directory -Filter '10.*' |
                Sort-Object { [version]$_.Name } -Descending |
                ForEach-Object { Join-Path $_.FullName 'x64/fxc.exe' } |
                Where-Object { Test-Path $_ -PathType Leaf }
            $env:GPUI_FXC_PATH = $compilers | Select-Object -First 1
        }
    }
    $targetArgs = @()
    $binary = 'target/release/saiki.exe'
    if ($Target) {
        $targetArgs = @('--target', $Target)
        $binary = "target/$Target/release/saiki.exe"
    }
    cargo build --locked --release @targetArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed' }
    New-Item -ItemType Directory -Force build | Out-Null
    Copy-Item $binary build/saiki.exe -Force
} finally {
    $env:GPUI_FXC_PATH = $previousFxcPath
    Pop-Location
}
