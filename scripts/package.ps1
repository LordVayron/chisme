# Build Chisme for distribution and make the installer in ..\dist.
#   powershell -ExecutionPolicy Bypass -File scripts\package.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$env:Path += ";$env:USERPROFILE\.cargo\bin"

Push-Location $root
try {
    cargo build --profile dist --bins
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = $meta.packages[0].version
    $bin = Join-Path $meta.target_directory "dist"

    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw "Inno Setup 6 not found: winget install JRSoftware.InnoSetup" }
    & $iscc /Q "/DBin=$bin" "/DAppVersion=$version" installer\chisme.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
    Get-Item "dist\Chisme-Setup-$version.exe"
} finally {
    Pop-Location
}
