# Builds the Windows setup program laserfoundry-windows-x64-setup.exe with Inno Setup (preinstalled on GitHub's
# windows runners; locally: https://jrsoftware.org/isinfo.php).
#
#   packaging/windows/build.ps1 -Bin <dir> -Out <dir>
#
# <dir> for -Bin holds laserfoundry.exe and laserfoundry-cli.exe (already signed when the release job signs them).
# The setup program itself is not signed here: sign it in a later step when signing is available.
param(
    [Parameter(Mandatory = $true)][string]$Bin,
    [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$Bin = (Resolve-Path $Bin).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

$match = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)' | Select-Object -First 1
if (-not $match) { throw 'cannot read the version from Cargo.toml' }
$version = $match.Matches[0].Groups[1].Value

# Stage the files the script installs next to the binaries. The icon is drawn in code (crates/icon).
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("laserfoundry-setup-" + [guid]::NewGuid())
New-Item -ItemType Directory $stage | Out-Null
try {
    foreach ($f in 'laserfoundry.exe', 'laserfoundry-cli.exe') { Copy-Item (Join-Path $Bin $f) $stage }
    foreach ($f in 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE') { Copy-Item (Join-Path $root $f) $stage }
    Push-Location $root
    try { cargo run --release --locked -q -p laserfoundry-icon -- ico (Join-Path $stage 'laserfoundry.ico') } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'could not draw the icon' }

    $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
    if (-not $iscc) { $iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
    if (-not (Test-Path $iscc)) { throw 'Inno Setup (ISCC.exe) is not installed' }
    & $iscc "/DAppVersion=$version" "/DBinDir=$stage" "/DOutDir=$Out" (Join-Path $PSScriptRoot 'laserfoundry.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
} finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
Get-Item (Join-Path $Out 'laserfoundry-windows-x64-setup.exe')
