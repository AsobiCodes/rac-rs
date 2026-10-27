# Linux groundwork gate: `cargo check` of the launcher for x86_64-unknown-linux-gnu (check only, no linking).
# Needs the rust target (added here) and a C compiler for Linux, only because the `ring` crate (rustls) compiles C in its build script:
# `python -m pip install ziglang` provides one (`zig cc`); the two small wrappers below are made on first use. On a real Linux PC gcc does it and this script is not needed.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$tools = Join-Path $here 'linux_tools'
New-Item -ItemType Directory -Force -Path $tools | Out-Null
$zig = & python -c "import ziglang,os;print(os.path.join(os.path.dirname(ziglang.__file__),'zig.exe'))"
if (-not (Test-Path -LiteralPath $zig)) { throw 'zig not found: run  python -m pip install ziglang' }
# cc-rs adds --target=x86_64-unknown-linux-gnu, which zig cannot parse: the python shim drops it and passes zig's own -target
$py = @"
import subprocess, sys
args = [a for a in sys.argv[1:] if not a.startswith('--target=')]
sys.exit(subprocess.call([r'$zig', 'cc', '-target', 'x86_64-linux-gnu'] + args))
"@
Set-Content -LiteralPath (Join-Path $tools 'zigcc.py') -Encoding ascii -Value $py
Set-Content -LiteralPath (Join-Path $tools 'cc.cmd') -Encoding ascii -Value "@python `"%~dp0zigcc.py`" %*"
Set-Content -LiteralPath (Join-Path $tools 'ar.cmd') -Encoding ascii -Value "@`"$zig`" ar %*"
& rustup target add x86_64-unknown-linux-gnu
$env:RUSTC_WRAPPER = ''
$env:CC_x86_64_unknown_linux_gnu = Join-Path $tools 'cc.cmd'
$env:AR_x86_64_unknown_linux_gnu = Join-Path $tools 'ar.cmd'
Push-Location (Join-Path $here 'launcher')
try {
    & cargo check --target x86_64-unknown-linux-gnu --target-dir (Join-Path $here 'target')
    $code = $LASTEXITCODE
} finally { Pop-Location }
if ($code -ne 0) { Write-Host 'LINUX CHECK FAILED'; exit $code }
Write-Host 'LINUX CHECK OK'
