#Requires -Version 7

param(
    [string]$CargoProfile = 'dist'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Find-InnoSetupCompiler {
    $command = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA\Programs")) {
        if (-not $base) { continue }
        $candidate = Join-Path $base 'Inno Setup 6/ISCC.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    throw 'Inno Setup 6 (ISCC.exe) is required to build the installer: winget install JRSoftware.InnoSetup'
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq 'zcord').version
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }

cargo build --locked --profile $CargoProfile
if ($LASTEXITCODE) { throw "cargo build failed ($LASTEXITCODE)" }

$package = Join-Path $root 'target/package'
$name = "zcord-$version-windows-$arch"
$stage = Join-Path $package $name
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'licenses') | Out-Null

Copy-Item -LiteralPath "target/$CargoProfile/zcord.exe" -Destination $stage
Copy-Item -LiteralPath 'LICENSE' -Destination $stage
Copy-Item -LiteralPath 'assets/fonts/OFL.txt' -Destination (Join-Path $stage 'licenses/NotoSans-OFL.txt')
Copy-Item -LiteralPath 'assets/fonts/display/OFL.txt' -Destination (Join-Path $stage 'licenses/display-fonts-OFL.txt')
Copy-Item -LiteralPath 'assets/fonts/display/NOTICE.md' -Destination (Join-Path $stage 'licenses/display-fonts-NOTICE.md')
Copy-Item -LiteralPath 'assets/TWEMOJI-LICENSE.md' -Destination (Join-Path $stage 'licenses')

$zip = Join-Path $package "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force

$iscc = Find-InnoSetupCompiler
& $iscc /Qp "/DAppVersion=$version" "/DArch=$arch" "/DPackageDir=$stage" "/DOutputDir=$package" `
    (Join-Path $root 'dist/windows/zcord.iss')
if ($LASTEXITCODE) { throw "ISCC failed ($LASTEXITCODE)" }

Get-ChildItem -LiteralPath $package -File | Format-Table Name, Length
