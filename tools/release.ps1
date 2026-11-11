# builds a release-candidate zip in dist\ (not committed, not published).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\release.ps1 [-SkipSmoke]
param([switch] $SkipSmoke)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
Write-Host "kadr $version"

cargo test --release
if ($LASTEXITCODE) { throw 'unit tests failed' }
cargo build --release
if ($LASTEXITCODE) { throw 'build failed' }
if (-not $SkipSmoke) {
  powershell -NoProfile -ExecutionPolicy Bypass -File tests\smoke.ps1
  if ($LASTEXITCODE) { throw 'smoke tests failed' }
}

$name = "kadr-$version-windows-x64"
$stage = Join-Path $root "dist\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item target\release\kadr.exe, README.md, CHANGELOG.md $stage
$zip = Join-Path $root "dist\$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $name.zip" | Set-Content "$zip.sha256" -Encoding ascii
Write-Host ("{0}  {1:N0} bytes  kadr.exe {2:N0} bytes" -f $zip, (Get-Item $zip).Length, (Get-Item target\release\kadr.exe).Length)
Write-Host "sha256 $hash"

# the installer is the same exe under a name containing "setup" (main.rs)
$setup = Join-Path $root "dist\kadr-$version-setup.exe"
Copy-Item target\release\kadr.exe $setup -Force
$sh = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
"$sh  kadr-$version-setup.exe" | Set-Content "$setup.sha256" -Encoding ascii
Write-Host "$setup  sha256 $sh"
