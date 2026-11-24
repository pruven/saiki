param(
    [ValidateSet('x86_64', 'arm64')]
    [string]$Arch = 'x86_64',
    [string]$Compiler = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
)
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if (!(Test-Path build/saiki.exe)) { throw 'Run bin/build.ps1 first.' }
    if (!(Test-Path $Compiler)) { throw 'Install Inno Setup 6 or pass -Compiler with the path to ISCC.exe.' }
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
    & $Compiler "/DAppVersion=$version" "/DAppArch=$Arch" packaging/windows/saiki.iss
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
} finally {
    Pop-Location
}
