# Builds the release launcher and copies it as "RAC-RS Launcher.exe" (rustc does not allow a space in a binary name).
# Output: dist\RAC-RS Launcher.exe next to this script (also passed through as the script result).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File build_release.ps1
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$crate = Join-Path $here 'launcher'
$target = Join-Path $here 'target'
$env:RUSTC_WRAPPER = ''
# These flags replace the rustflags of launcher\.cargo\config.toml, so crt-static is repeated here; the remaps keep this PC's
# folders (user name, checkout folder) out of the panic messages in the exe. Encoded form (0x1F between flags): paths may hold spaces.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$env:RUSTFLAGS = ''
$env:CARGO_ENCODED_RUSTFLAGS = @('-C', 'target-feature=+crt-static', "--remap-path-prefix=$cargoHome=cargo", "--remap-path-prefix=$here=rac-rs-launcher") -join [char]0x1f
Push-Location $crate
try {
    cargo build --release --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
} finally { Pop-Location }
$built = Join-Path $here 'target\release\rac-rs-launcher.exe'
if (-not (Test-Path -LiteralPath $built)) { throw "missing $built" }
$dist = Join-Path $here 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
$out = Join-Path $dist 'RAC-RS Launcher.exe'
Copy-Item -LiteralPath $built -Destination $out -Force
"{0}  {1:N0} bytes" -f $out, (Get-Item -LiteralPath $out).Length
# Release asset name used by the launcher's self-update and by Setup.cmd / setup.ps1 -GetLauncher, plus its SHA-256 (GitHub shows the same digest).
$asset = Join-Path $dist 'rac-rs-launcher-windows-x64.exe'
Copy-Item -LiteralPath $built -Destination $asset -Force
$hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLower()
Set-Content -LiteralPath ($asset + '.sha256') -Value "$hash  rac-rs-launcher-windows-x64.exe" -Encoding ascii
"{0}`nsha256 {1}" -f $asset, $hash
