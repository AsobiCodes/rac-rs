#!/bin/sh
# setup.sh - builds rac-rs (a PC port of Ratchet & Clank, PS2) on Linux from your own disc image.
#
# The Linux counterpart of setup.ps1. It needs NOTHING installed on your PC: no compiler, no Rust, no
# -dev packages, no sudo. Step (a) downloads a private, pinned copy of Rust, Python and a C linker (zig)
# INTO this folder (./toolchain, about 1 GB on disk, about 250 MB to download; nothing outside the
# folder is touched) and the build uses only that. With --system-toolchain the old way is used instead:
# your own Rust (1.90 or newer), Python 3, gcc, pkg-config and the libasound/libudev -dev packages.
#
# Steps, in order:
#   (a)  the tools: download and unpack Rust, Python and the C linker (once; skipped when they are there)
#   (b)  check that the .iso is Ratchet & Clank PAL/EU v1.00 (SCES-50916) or USA v1.00 (SCUS-97199)
#   (c)  generate the game code from the disc with tools/recgen_iso.py (boot + 19 levels)
#   (c2) generate the VU microcode translations with tools/vugen_iso.py
#   (d)  build the game with cargo (release) into ./build
#   (e)  write generated/play.cfg, so ./build/release/rac-rs can start on its own
#
# Everything it writes stays inside this folder: toolchain/, oracle/rec, oracle/game, the generated .rs
# files named by .gitignore, generated/, build/, and player/ once you play. It writes nothing outside
# this folder at all (the temp folder, the cargo and zig caches are under toolchain/ too). The
# application menu entry is a separate script, ./install-desktop-entry.sh, so that it stays that way.
#
# This is plain POSIX shell, so it runs under whatever /bin/sh happens to be: bash on Arch and Fedora,
# dash on Debian and Ubuntu, ash on busybox. Nothing else is needed but tar, a downloader (curl, wget or
# python3) and the run-time libraries the game itself loads (see README.md, "Linux").
#
# Usage:
#   ./setup.sh                       # the disc in the rom folder, or the one from the last run
#   ./setup.sh --iso "<path to your .iso>"
#   ./setup.sh --check-only          # just verify the disc
#
# Options:
#   --iso PATH     the disc image (or pass it as the first plain argument)
#   -j, --jobs N   level generators in parallel (default 4)
#   --cargo-jobs N  compiler processes in parallel (default: from the cores and the free memory)
#   --check-only   do (b) only
#   --skip-hash    size check only, skip the SHA-1
#   --regenerate   redo (c) and (c2) even if the stamps say they are current
#   --no-build     stop after (c2)
#   --no-pause     do not wait for a key at the end (the default waits, so a run
#                  started from a file manager keeps its window open)
#   --system-toolchain  use your own Rust, Python, gcc and -dev packages (the old way; remembered
#                  for later runs and updates until you pass --pinned-toolchain)
#   --pinned-toolchain  use the private downloaded toolchain (the default)
#   --apply-update --new-version X  download and apply release X over this folder, then rebuild
#                  (used by the launcher; --finish-only skips the download, --test-update-base D
#                  takes the update from a folder or URL for tests)
#   --add-to-steam      put the game into Steam's list of non-Steam games (for the Steam Deck and Big Picture), then exit.
#                  Steam must be closed; shortcuts.vdf is copied to shortcuts.vdf.bak first (tools/steam_shortcut.py)
#   --remove-from-steam  take the game out of Steam's list again, then exit
#   --steam-status print "found=1 has=0" (Steam found / the game is in its list), then exit
#   -h, --help     this text
#
# The environment:
#   CARGO_BUILD_JOBS=N       how many cores the compile uses (the same as --cargo-jobs)
#   RAC_NO_PAUSE=1           the same as --no-pause
#   RAC_SYSTEM_TOOLCHAIN=1   the same as --system-toolchain
#
# Exit codes: 0 done, 1 failed (an "ERROR:" line says why), 3 the update could not be downloaded.
# With --add-to-steam / --remove-from-steam (the same codes as setup.ps1): 0 done, 1 error, 2 Steam not found, 3 Steam is running.
#

# Steam (Game Mode) preloads its overlay into everything it starts; a 32-bit one makes every program print an ld.so "ERROR:"
# line. Cleared first, so that not even the programs started below (dirname, ...) see it.
unset LD_PRELOAD

# Where the user ran this from: a relative --iso path is relative to that folder, not to the game folder.
callerdir=$(pwd) || exit 1
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 1
cd "$root" || exit 1

# Piped output (the launcher reads it line by line): no buffering, no colours, no progress bars.
PYTHONUNBUFFERED=1
CARGO_TERM_COLOR=never
CARGO_TERM_PROGRESS_WHEN=never
export PYTHONUNBUFFERED CARGO_TERM_COLOR CARGO_TERM_PROGRESS_WHEN
# Steam (Game Mode on the Deck) preloads its overlay into everything it starts. The 32-bit one makes every program print an ld.so
# "ERROR:" line, which the launcher would show as the failure, and none of it helps the build.
unset LD_PRELOAD

# The exit code Fail uses: 1, but 3 while the update's Python download runs (the launcher keys its text on 3).
fail_rc=1
Fail() {
    printf 'ERROR: %s\n' "$1" >&2
    exit "$fail_rc"
}

Step() {
    printf '\n== %s\n' "$1"
}

# Trim the spaces off both ends of a string.
trim() {
    printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

# Join words with a separator. The words are split apart first, so this is for lists of
# plain words, never for paths.
join_words() {
    sep=$1
    shift
    out=''
    for w in "$@"; do
        if [ -z "$out" ]; then out=$w; else out="$out$sep$w"; fi
    done
    printf '%s' "$out"
}

# A path from the command line made absolute against the folder this was started from.
CallerPath() {
    case $1 in
        /*) printf '%s' "$1" ;;
        *) printf '%s/%s' "$callerdir" "$1" ;;
    esac
}

Have() {
    command -v "$1" >/dev/null 2>&1
}

# ---------------------------------------------------------------- stopping
# Descendants of a process (children, grandchildren, ...), one pid per line. Uses ps (procps and busybox have it).
Descendants() {
    { ps -A -o pid= -o ppid= 2>/dev/null || ps -o pid= -o ppid= 2>/dev/null; } | awk -v root="$1" '
        { kid[NR] = $1; par[NR] = $2; n = NR }
        END {
            want[root] = 1
            do {
                found = 0
                for (i = 1; i <= n; i++) if ((par[i] in want) && !(kid[i] in want)) { want[kid[i]] = 1; found = 1 }
            } while (found)
            for (p in want) if (p != root) print p
        }'
}

# Stops everything below this shell (the level generators, the build and what they started): TERM, a second later KILL.
KillKids() {
    kk_list=$(Descendants "$$")
    [ -n "$kk_list" ] || return 0
    for p in $kk_list; do kill -TERM "$p" 2>/dev/null; done
    sleep 1
    for p in $(Descendants "$$"); do kill -KILL "$p" 2>/dev/null; done
}

# The launcher kills the whole process group on Cancel. This is for a plain "kill <pid of setup.sh>": the
# background jobs (level generators, the build) and everything below them are stopped too.
StopAll() {
    trap '' TERM HUP INT
    KillKids
    printf 'ERROR: stopped\n' >&2
    exit 130
}
trap StopAll TERM HUP INT
# Also when this script ends early (a failed step): no generator may go on in the background.
trap KillKids EXIT

# ---------------------------------------------------------------- arguments
iso=''
jobs=4
cargo_jobs=''
skip_hash=0
check_only=0
regenerate=0
no_build=0
no_pause=0
tc_mode=''            # pinned | system ('' = the default, or what the marker says)
apply_update=0
new_version=''
finish_only=0
test_update_base=''
expect_compiles=''
steam_mode=''         # add | remove | status ('' = a normal run)

while [ $# -gt 0 ]; do
    case $1 in
        --iso)
            [ $# -ge 2 ] || Fail '--iso needs a path'
            iso=$(CallerPath "$2")
            shift 2 ;;
        -j|--jobs)
            [ $# -ge 2 ] || Fail '--jobs needs a number'
            jobs=$2
            case $jobs in
                ''|*[!0-9]*) Fail '--jobs needs a number of 1 or more' ;;
            esac
            [ "$jobs" -ge 1 ] || Fail '--jobs needs a number of 1 or more'
            shift 2 ;;
        --cargo-jobs)
            [ $# -ge 2 ] || Fail '--cargo-jobs needs a number'
            cargo_jobs=$2
            case $cargo_jobs in
                ''|*[!0-9]*) Fail '--cargo-jobs needs a number of 1 or more' ;;
            esac
            [ "$cargo_jobs" -ge 1 ] || Fail '--cargo-jobs needs a number of 1 or more'
            shift 2 ;;
        --skip-hash)
            skip_hash=1
            shift ;;
        --check-only)
            check_only=1
            shift ;;
        --regenerate)
            regenerate=1
            shift ;;
        --no-build)
            no_build=1
            shift ;;
        --no-pause)
            no_pause=1
            shift ;;
        --system-toolchain)
            tc_mode=system
            shift ;;
        --pinned-toolchain)
            tc_mode=pinned
            shift ;;
        --apply-update)
            apply_update=1
            shift ;;
        --new-version)
            [ $# -ge 2 ] || Fail '--new-version needs a version like 0.4.2'
            new_version=$2
            shift 2 ;;
        --finish-only)
            finish_only=1
            shift ;;
        --test-update-base)
            [ $# -ge 2 ] || Fail '--test-update-base needs a folder or URL'
            test_update_base=$2
            case $test_update_base in
                http://*|https://*|file://*) ;;
                *) test_update_base=$(CallerPath "$test_update_base") ;;
            esac
            shift 2 ;;
        --expect-compiles)
            [ $# -ge 2 ] || Fail '--expect-compiles needs a number'
            expect_compiles=$2
            shift 2 ;;
        --add-to-steam)
            steam_mode=add
            shift ;;
        --remove-from-steam)
            steam_mode=remove
            shift ;;
        --steam-status)
            steam_mode=status
            shift ;;
        --skip-unblock|--no-shortcut|--no-pgo)
            # Windows-only switches the launcher passes to both scripts; nothing to do here.
            shift ;;
        -h|--help)
            last=$(grep -n '^#   -h, --help' "$root/setup.sh" | cut -d: -f1)
            head -n "$last" "$root/setup.sh" | tail -n +2
            exit 0 ;;
        *)
            [ -z "$iso" ] || Fail 'the disc image was given twice'
            iso=$(CallerPath "$1")
            shift ;;
    esac
done

# The folder the setup writes its logs, stamps and the play configuration to.
gen=generated
# The folder a disc image is dropped into.
romdir=rom
# The private tools (step a).
tc=$root/toolchain

# "Add to Steam" / "Remove from Steam" / status: tools/steam_shortcut.py does the work, with the private Python when it is there, else the
# system's. It needs no set-up step. Its output and exit code (0 done, 1 error, 2 Steam not found, 3 Steam running) are passed on.
SteamShortcut() {
    if [ -x "$tc/python/bin/python3" ]; then
        ss_py=$tc/python/bin/python3
    elif Have python3; then
        ss_py=$(command -v python3)
    else
        Fail 'python3 not found (the Steam shortcut tool needs it)'
    fi
    [ -f "$root/tools/steam_shortcut.py" ] || Fail 'tools/steam_shortcut.py is missing from this folder'
    unset PYTHONHOME PYTHONPATH PYTHONSTARTUP PYTHONPYCACHEPREFIX PYTHONUSERBASE
    case ${LD_LIBRARY_PATH:-} in
        *[Ss]team*) unset LD_LIBRARY_PATH ;;
    esac
    "$ss_py" -I "$root/tools/steam_shortcut.py" "$1" --root "$root"
    ss_rc=$?
    trap - EXIT
    exit "$ss_rc"
}

# ---------------------------------------------------------------- Steam shortcut (only this, then exit)
if [ -n "$steam_mode" ]; then
    SteamShortcut "$steam_mode"
fi

# ---------------------------------------------------------------- toolchain mode
# pinned (default) or system. The choice is remembered in toolchain/system.txt, so an update or a repair
# keeps it; --pinned-toolchain clears it.
if [ -z "$tc_mode" ] && [ -n "$RAC_SYSTEM_TOOLCHAIN" ] && [ "$RAC_SYSTEM_TOOLCHAIN" != 0 ]; then
    tc_mode=system
fi
if [ -z "$tc_mode" ]; then
    if [ -f "$tc/system.txt" ]; then tc_mode=system; else tc_mode=pinned; fi
fi
if [ "$tc_mode" = system ]; then
    mkdir -p "$tc" && printf 'system toolchain: set by setup.sh --system-toolchain; remove with --pinned-toolchain\n' >"$tc/system.txt"
else
    rm -f "$tc/system.txt"
fi

if [ "$tc_mode" = pinned ]; then
    # The private Python must not see the player's Python settings: a PYTHONHOME left by an IDE stops it from starting, a
    # PYTHONPYCACHEPREFIX writes outside this folder, a PYTHONPATH or user site folder can shadow modules.
    unset PYTHONHOME PYTHONPATH PYTHONSTARTUP PYTHONPYCACHEPREFIX PYTHONUSERBASE
    PYTHONNOUSERSITE=1
    export PYTHONNOUSERSITE
    # The Steam runtime's library folders (Game Mode) are for Steam games; the private tools bring their own.
    case ${LD_LIBRARY_PATH:-} in
        *[Ss]team*) unset LD_LIBRARY_PATH ;;
    esac
fi

# ---------------------------------------------------------------- the pins (step a)
# Rust 1.99.0 (the same as on Windows): the three packages of x86_64-unknown-linux-gnu, their SHA-256 from
# https://static.rust-lang.org/dist/channel-rust-1.99.0.toml (whose own SHA-256 is
# ce6dddc886364f8d786514771212cebe9b731ba82d6b859951c6b0ccc516b6a2, the value setup.ps1 pins).
RUST_VER=1.99.0
RUST_BASE=https://static.rust-lang.org/dist/2026-10-01
RUSTC_SHA=77171ba2a0345fdf2abc4fedda55d6de078dae7a68527c28be8c77dcc9604bd5
CARGO_SHA=d7674918d28093097614cd9728b6ca60db9ea3038f640f0bd1e9a4188c7568ce
RUSTSTD_SHA=3e58dff2d0b72196b5ea4e90536e174d400de88564a52694686b81e091169933
# Python 3.14.8 (the same as on Windows), python-build-standalone, SHA-256 from the release's SHA256SUMS.
PY_VER=3.14.8
PY_TAG=20261013
PY_FILE=cpython-3.14.8+20261013-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz
PY_URL=https://github.com/astral-sh/python-build-standalone/releases/download/20261013/cpython-3.14.8%2B20261013-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz
PY_SHA=d8faa1b588506b9e3e5a3f57807bb056d534a1641a6b10d15b6ea3f0b413b960
# zig = the C linker driver (it bundles clang's driver, lld and the glibc stubs). SHA-256 from
# https://ziglang.org/download/index.json. The program is linked against glibc 2.28, so it runs on
# every distribution since 2019 (Debian 10, Ubuntu 18.10, SteamOS 3, Fedora 29, ...).
ZIG_VER=0.14.1
ZIG_URL=https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz
ZIG_SHA=24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c
ZIG_GLIBC=2.28

# The Python that runs the generators and the helper tools: the private one, or the system's.
PyExe() {
    if [ "$tc_mode" = pinned ]; then
        printf '%s' "$tc/python/bin/python3"
    else
        command -v python3
    fi
}

# ---------------------------------------------------------------- downloading
# SHA-256 of a file, lower case.
Sha256() {
    if Have sha256sum; then
        sha256sum <"$1" | cut -d' ' -f1
    elif Have shasum; then
        shasum -a 256 <"$1" | cut -d' ' -f1
    elif Have openssl; then
        openssl dgst -sha256 <"$1" | sed -e 's/^.*= *//'
    else
        _py=$(AnyPython) || return 1
        "$_py" -I -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"
    fi | tr '[:upper:]' '[:lower:]'
}

# Some python: the private one if it is already there, else the system's.
AnyPython() {
    if [ -x "$tc/python/bin/python3" ]; then printf '%s' "$tc/python/bin/python3"; return 0; fi
    if Have python3; then command -v python3; return 0; fi
    return 1
}

# Download one URL to one file: curl, else wget, else python's urllib.
Fetch() {
    if Have curl; then
        # -q first: no ~/.curlrc (it can switch off the certificate check, add a proxy or an output folder)
        curl -q -fsSL --connect-timeout 30 --max-time 3600 -o "$2" "$1"
    elif Have wget; then
        # GNU wget: no ~/.wgetrc, and no ~/.wget-hsts (wget writes it on every https download: a file outside this folder).
        # busybox wget has neither option nor file.
        _wh=$(wget --help 2>&1)
        _wo=''
        case $_wh in *--no-config*) _wo=--no-config ;; esac
        case $_wh in *--no-hsts*) _wo="$_wo --no-hsts" ;; esac
        wget $_wo -q -T 60 -O "$2" "$1"
    elif _py=$(AnyPython) && [ -f "$root/tools/update_apply.py" ]; then
        "$_py" -I "$root/tools/update_apply.py" fetch "$1" "$2"
    else
        printf '  no downloader found (curl, wget or python3 is needed)\n' >&2
        return 1
    fi
}

# Download $1 (url) to $2 (file) and check its SHA-256 $3; up to 3 tries (port of Get-Pinned in setup.ps1).
Pinned() {
    p_url=$1
    p_file=$2
    p_sha=$3
    if [ -f "$p_file" ] && [ "$(Sha256 "$p_file")" = "$p_sha" ]; then return 0; fi
    mkdir -p "$(dirname "$p_file")"
    p_try=1
    while [ "$p_try" -le 3 ]; do
        rm -f "$p_file.part"
        if Fetch "$p_url" "$p_file.part"; then
            p_h=$(Sha256 "$p_file.part")
            if [ "$p_h" = "$p_sha" ]; then
                mv -f "$p_file.part" "$p_file"
                return 0
            fi
            printf '  download of %s has the wrong SHA-256 (%s), retrying\n' "$p_url" "$p_h"
        else
            printf '  download of %s failed, retrying\n' "$p_url"
        fi
        rm -f "$p_file.part"
        [ "$p_try" -lt 3 ] && sleep $((5 * p_try))
        p_try=$((p_try + 1))
    done
    Fail "could not download $p_url (check the internet connection, then run setup.sh again)"
}

# Unpack a .tar.xz into an existing folder: tar when xz is there, else python's tarfile.
UnXz() {
    if Have xz && [ -z "$RAC_NO_XZ" ] && tar -xJf "$1" -C "$2" 2>/dev/null; then
        return 0
    fi
    _py=$(AnyPython) || return 1
    "$_py" -I "$root/tools/update_apply.py" untar "$1" "$2"
}

# ---------------------------------------------------------------- the machine
# Which distribution packages hold the libraries, as one line for the player.
PkgHint() {
    # $1 = apt name, $2 = dnf name, $3 = pacman name
    _id=''
    _like=''
    if [ -r /etc/os-release ]; then
        _id=$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"' | head -n 1)
        _like=$(sed -n 's/^ID_LIKE=//p' /etc/os-release | tr -d '"' | head -n 1)
    fi
    case " $_id $_like " in
        # SteamOS ships these libraries and its system folder is read-only: no package command, a way to the right place instead
        *" steamos "*) printf 'nothing (SteamOS ships it: use Desktop Mode and the latest SteamOS update, and report it at github.com/AsobiCodes/rac-rs if it is still missing)' ;;
        *" arch "*|*" manjaro "*) printf 'sudo pacman -S %s' "$3" ;;
        *" fedora "*|*" rhel "*|*" centos "*) printf 'sudo dnf install %s' "$2" ;;
        *" debian "*|*" ubuntu "*) printf 'sudo apt install %s' "$1" ;;
        *) printf 'install %s (Debian/Ubuntu), %s (Fedora) or %s (Arch)' "$1" "$2" "$3" ;;
    esac
}

# ldconfig lives in /sbin, which a normal user may not have on PATH.
LdConfig() {
    if Have ldconfig; then command -v ldconfig; return 0; fi
    for _l in /sbin/ldconfig /usr/sbin/ldconfig; do
        if [ -x "$_l" ]; then printf '%s' "$_l"; return 0; fi
    done
    return 1
}

# Is this file a 64-bit ELF file? (a 32-bit library in /usr/lib on Fedora/openSUSE must never be linked or reported as found)
Is64() {
    Have od || return 0
    [ "$(od -An -tx1 -j4 -N1 "$1" 2>/dev/null | tr -d ' \n')" = 02 ]
}

# The folder that holds a shared library (its file name, like libasound.so.2); empty and status 1 when the system has none.
FindLibDir() {
    if _ldc=$(LdConfig); then
        _p=$("$_ldc" -p 2>/dev/null | awk -v n="$1" '$1 == n && /x86-64/ { print $NF; exit }')
        if [ -n "$_p" ] && [ -e "$_p" ]; then printf '%s' "${_p%/*}"; return 0; fi
    fi
    for _d in /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu /usr/lib64 /lib64 /usr/lib /lib; do
        if [ -e "$_d/$1" ] && Is64 "$_d/$1"; then printf '%s' "$_d"; return 0; fi
    done
    return 1
}

# One library entry 'soname|apt package|dnf package|pacman package|what it is for'. When the library is missing: prints $2
# (ERROR or WARNING) with the install line for this distribution and returns 1.
CheckLib() {
    cl_so=${1%%|*}
    cl_r=${1#*|}
    cl_apt=${cl_r%%|*}; cl_r=${cl_r#*|}
    cl_dnf=${cl_r%%|*}; cl_r=${cl_r#*|}
    cl_pac=${cl_r%%|*}; cl_what=${cl_r#*|}
    if FindLibDir "$cl_so" >/dev/null; then return 0; fi
    if [ "$2" = ERROR ]; then
        printf 'ERROR: %s (%s) is missing, and the game cannot be built without it. Install it with: %s\n' \
            "$cl_so" "$cl_what" "$(PkgHint "$cl_apt" "$cl_dnf" "$cl_pac")" >&2
    else
        printf 'WARNING: %s (%s) is missing; the game will not start without it. Install it with: %s\n' \
            "$cl_so" "$cl_what" "$(PkgHint "$cl_apt" "$cl_dnf" "$cl_pac")"
    fi
    return 1
}

# Checks before the long part. Hard errors only where the build would fail anyway (after 20 minutes).
Preflight() {
    [ "$(uname -s)" = Linux ] || Fail "this setup is for Linux (uname says $(uname -s))"
    [ "$(uname -m)" = x86_64 ] || Fail "this setup is for 64-bit Intel/AMD PCs (x86_64); this machine is $(uname -m)"
    # Programs must be able to run from this folder, and links must work (an SD card formatted as exFAT/FAT can do neither).
    mkdir -p "$tc/tmp" || Fail "cannot create $tc/tmp (is this folder writable?)"
    pf_t="$tc/tmp/exec_test.sh"
    printf '#!/bin/sh\nexit 0\n' >"$pf_t" && chmod +x "$pf_t" 2>/dev/null
    if ! "$pf_t" 2>/dev/null; then
        rm -f "$pf_t"
        Fail "programs cannot run from this folder ($root): the drive is mounted without permission to run programs, or it is a FAT/exFAT drive. Use a folder on your Linux drive (for the Steam Deck: the internal drive, or an SD card formatted as ext4)."
    fi
    rm -f "$pf_t"
    rm -f "$tc/tmp/link_test"
    if ! ln -s exec_test_target "$tc/tmp/link_test" 2>/dev/null; then
        rm -f "$tc/tmp/link_test"
        Fail "this folder's drive cannot make symbolic links ($root; FAT/exFAT?). Use a folder on your Linux drive (for the Steam Deck: the internal drive, or an SD card formatted as ext4)."
    fi
    rm -f "$tc/tmp/link_test"
    # Disk space (the whole install is about 7 GB before ./clean.sh).
    pf_free=$(df -Pk . 2>/dev/null | awk 'NR == 2 { print $4 }')
    case $pf_free in
        ''|*[!0-9]*) ;;
        *) if [ "$pf_free" -lt 7340032 ]; then
               printf 'WARNING: only %d MB free in this folder; the install needs about 7 GB (it falls to 0.3 GB after ./clean.sh)\n' $((pf_free / 1024))
           fi ;;
    esac
    # Libraries. The game links to libasound and libudev, so the build cannot finish without them: an error now
    # beats one after the compile. The rest are loaded when the game starts: only a warning.
    pf_bad=0
    CheckLib 'libasound.so.2|libasound2t64 (libasound2 on older Debian/Ubuntu)|alsa-lib|alsa-lib|sound' ERROR || pf_bad=1
    CheckLib 'libudev.so.1|libudev1|systemd-libs|systemd-libs|controllers' ERROR || pf_bad=1
    [ "$pf_bad" -eq 0 ] || exit 1
    CheckLib 'libvulkan.so.1|libvulkan1|vulkan-loader|vulkan-icd-loader|graphics (the Vulkan loader; your graphics driver brings it)' WARNING
    CheckLib 'libxkbcommon.so.0|libxkbcommon0|libxkbcommon|libxkbcommon|the window (keyboard)' WARNING
    # The window library (winit) opens these with dlopen when the game starts (x11-dl: libX11, libX11-xcb, libXcursor, libXi, all four
    # needed on X11; xkbcommon-dl: the two above; wayland-sys: libwayland-client). A missing one stops the game with "Failed to
    # load one of xlib's shared libraries". Minimal installs have libX11 (Xvfb, Xwayland) but not the other X11 libraries.
    if FindLibDir libX11.so.6 >/dev/null; then
        CheckLib 'libX11-xcb.so.1|libx11-xcb1|libX11-xcb|libx11|the window (X11)' WARNING
        CheckLib 'libXcursor.so.1|libxcursor1|libXcursor|libxcursor|the window (X11, mouse pointer)' WARNING
        CheckLib 'libXi.so.6|libxi6|libXi|libxi|the window (X11, mouse and keyboard input)' WARNING
        # winit loads libxkbcommon-x11 for X11 only: a Wayland-only PC without it must not be told it is missing
        CheckLib 'libxkbcommon-x11.so.0|libxkbcommon-x11-0|libxkbcommon-x11|libxkbcommon-x11|the window (keyboard)' WARNING
    elif ! FindLibDir libwayland-client.so.0 >/dev/null; then
        printf 'WARNING: neither the X11 window libraries nor libwayland-client.so.0 are installed; the game needs a desktop. Install: %s\n' \
            "$(PkgHint 'libx11-6 libx11-xcb1 libxcursor1 libxi6 (or libwayland-client0)' 'libX11 libX11-xcb libXcursor libXi (or libwayland-client)' 'libx11 libxcursor libxi (or wayland)')"
    fi
}

# How many compiler processes: the cores, but about 2.3 GB of memory for each (the generated crates are big).
AutoCargoJobs() {
    ac_cores=$(nproc 2>/dev/null || grep -c '^processor' /proc/cpuinfo 2>/dev/null || echo 4)
    ac_kb=$(awk '/^MemAvailable:/ { print $2; exit }' /proc/meminfo 2>/dev/null)
    case $ac_kb in ''|*[!0-9]*) ac_kb=8000000 ;; esac
    ac_by_mem=$((ac_kb / 2300000))
    [ "$ac_by_mem" -ge 1 ] || ac_by_mem=1
    ac_j=$ac_cores
    [ "$ac_j" -le "$ac_by_mem" ] || ac_j=$ac_by_mem
    [ "$ac_j" -ge 1 ] || ac_j=1
    printf '%s' "$ac_j"
}

# One line about how long the compile takes: measured 15 minutes with 5 compiler processes, about half an hour with 4 (the same numbers
# as the launcher's install page: launcher/src/linuxenv.rs build_minutes). $1 = compiler processes. Whole numbers only (no bc/awk float).
BuildEstimate() {
    be_j=$1
    case $be_j in ''|*[!0-9]*) return 0 ;; esac
    [ "$be_j" -ge 1 ] || return 0
    be_est=$((20 + 650 / be_j))          # tenths of a minute: 2 + 65 / jobs
    be_lo=$(((be_est + 25) / 50 * 5))
    be_hi=$(((be_est * 16 / 10 + 25) / 50 * 5))
    [ "$be_lo" -ge 5 ] || be_lo=5
    [ "$be_hi" -gt "$be_lo" ] || be_hi=$((be_lo + 5))
    printf '  This takes about %s to %s minutes on this PC. Keep it plugged in and awake until it is done (sleep would stop the build).
' "$be_lo" "$be_hi"
}

# ---------------------------------------------------------------- the update (-ApplyUpdate of setup.ps1)
if [ "$apply_update" -eq 1 ]; then
    Step "(u) updating to v$new_version"
    case $new_version in
        [0-9]*.[0-9]*.[0-9]*) ;;
        *) Fail "--apply-update needs --new-version X.Y.Z" ;;
    esac
    # The helper is Python (standard library only): the private Python, downloaded first when the folder has none yet
    # (a ./clean.sh removes it), else the system's.
    [ -f "$root/tools/update_apply.py" ] || Fail 'tools/update_apply.py is missing (the update tool); download the release again'
    if [ "$tc_mode" = pinned ] && [ ! -x "$tc/python/bin/python3" ]; then
        printf '  downloading Python %s (python-build-standalone, about 36 MB) ...\n' "$PY_VER"
        mkdir -p "$tc/tmp"
        fail_rc=3                 # a failed download is an update that could not be downloaded
        Pinned "$PY_URL" "$tc/downloads/$PY_FILE" "$PY_SHA"
        fail_rc=1
        rm -rf "$tc/pyx" && mkdir -p "$tc/pyx" && tar -xzf "$tc/downloads/$PY_FILE" -C "$tc/pyx" || Fail 'could not unpack Python'
        rm -rf "$tc/python" && mv "$tc/pyx/python" "$tc/python" || Fail 'could not unpack Python'
        rm -rf "$tc/pyx" "$tc/downloads"
        "$tc/python/bin/python3" -I -c 'import sys, lzma, ssl, hashlib' || Fail 'the downloaded Python does not run on this system'
        printf '%s\n' "$PY_VER $PY_TAG $PY_SHA" >"$tc/python/.pin"
    fi
    upy=$(PyExe) || Fail 'python3 not found (the update tool needs it)'
    set -- "$upy" -I "$root/tools/update_apply.py" apply --root "$root" --version "$new_version"
    [ "$finish_only" -eq 0 ] || set -- "$@" --finish-only
    [ -z "$test_update_base" ] || set -- "$@" --test-base "$test_update_base"
    "$@"
    rc=$?
    case $rc in
        0) ;;
        3) exit 3 ;;
        *) exit 1 ;;
    esac
    # The rest runs as the NEW setup.sh in a fresh process: the file was replaced by a rename, and the
    # shell that is running this one still reads the old file.
    ex=$(sed -n 's/^expect-compiles: //p' "$gen/update_expect.txt" 2>/dev/null | head -n 1)
    set -- sh "$root/setup.sh" --skip-hash --no-pause
    [ -z "$iso" ] || set -- "$@" --iso "$iso"
    [ -z "$ex" ] || set -- "$@" --expect-compiles "$ex"
    set -- "$@" --jobs "$jobs"
    [ -z "$cargo_jobs" ] || set -- "$@" --cargo-jobs "$cargo_jobs"
    [ "$tc_mode" != system ] || set -- "$@" --system-toolchain
    [ "$regenerate" -eq 0 ] || set -- "$@" --regenerate
    exec "$@"
fi

# ---------------------------------------------------------------- (a) tools
Step '(a) tools'
if [ "$check_only" -eq 1 ]; then
    printf '  not needed for --check-only\n'
elif [ "$tc_mode" = system ]; then
    printf '  using your own Rust, Python and C compiler (--system-toolchain)\n'
    Have cargo || Fail 'cargo not found (install the rust toolchain: https://rustup.rs)'
    Have python3 || Fail 'python3 not found (the generators need it)'
    Have rustc || Fail 'rustc not found (install the rust toolchain: https://rustup.rs)'
    Have cc || Have gcc || Fail 'no C compiler found (cc/gcc): install gcc, pkg-config and the libasound2-dev / libudev-dev packages, or drop --system-toolchain'
    # The Rust libraries the game uses declare Rust 1.90 as their minimum (ordered-float 5.5; wgpu needs 1.87), and cargo refuses an older
    # compiler only after the ~20 minute code generation. The game itself is built and tested with Rust 1.99 (what setup.ps1 installs).
    rust_ver=$(rustc --version 2>/dev/null | sed -n 's/^rustc \([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')
    [ -n "$rust_ver" ] || Fail 'cannot read the Rust version from "rustc --version"'
    rust_major=${rust_ver%%.*}
    rust_minor=${rust_ver#*.}
    rust_minor=${rust_minor%%.*}
    if [ "$rust_major" -lt 1 ] || { [ "$rust_major" -eq 1 ] && [ "$rust_minor" -lt 90 ]; }; then
        Fail "Rust $rust_ver is too old (1.90 or newer is needed, 1.99 is what the game is tested with). Install a current Rust with rustup: https://rustup.rs"
    fi
    if [ "$rust_major" -eq 1 ] && [ "$rust_minor" -lt 99 ]; then
        printf '  note: Rust %s found; the game is built and tested with 1.99, newer is safest (rustup update)\n' "$rust_ver"
    fi
    printf '  cargo %s and python3 found\n' "$rust_ver"
else
    Preflight
    mkdir -p "$tc/downloads" "$tc/tmp" "$tc/bin"
    t_start=$(date +%s)
    # Which of the three are missing or stale (a stamp file in each folder holds the pins it was unpacked from).
    rust_pin="$RUST_VER $RUSTC_SHA $CARGO_SHA $RUSTSTD_SHA"
    py_pin="$PY_VER $PY_TAG $PY_SHA"
    zig_pin="$ZIG_VER $ZIG_SHA"
    need_rust=1; need_py=1; need_zig=1
    [ -x "$tc/rust/bin/rustc" ] && [ -x "$tc/rust/bin/cargo" ] && [ "$(cat "$tc/rust/.pin" 2>/dev/null)" = "$rust_pin" ] && need_rust=0
    [ -x "$tc/python/bin/python3" ] && [ "$(cat "$tc/python/.pin" 2>/dev/null)" = "$py_pin" ] && need_py=0
    [ -x "$tc/zig/zig" ] && [ "$(cat "$tc/zig/.pin" 2>/dev/null)" = "$zig_pin" ] && need_zig=0
    if [ "$need_rust" -eq 0 ] && [ "$need_py" -eq 0 ] && [ "$need_zig" -eq 0 ]; then
        printf '  Rust %s, Python %s and the C linker (zig %s) are already in toolchain/\n' "$RUST_VER" "$PY_VER" "$ZIG_VER"
    else
        # The three downloads first (the progress lines are in the order Rust, Python, C linker); the unpacking needs Python.
        if [ "$need_rust" -eq 1 ]; then
            printf '  downloading Rust %s (x86_64-unknown-linux-gnu, about 127 MB) ...\n' "$RUST_VER"
            Pinned "$RUST_BASE/rustc-$RUST_VER-x86_64-unknown-linux-gnu.tar.xz" "$tc/downloads/rustc-$RUST_VER.tar.xz" "$RUSTC_SHA"
            Pinned "$RUST_BASE/cargo-$RUST_VER-x86_64-unknown-linux-gnu.tar.xz" "$tc/downloads/cargo-$RUST_VER.tar.xz" "$CARGO_SHA"
            Pinned "$RUST_BASE/rust-std-$RUST_VER-x86_64-unknown-linux-gnu.tar.xz" "$tc/downloads/rust-std-$RUST_VER.tar.xz" "$RUSTSTD_SHA"
        fi
        if [ "$need_py" -eq 1 ]; then
            printf '  downloading Python %s (python-build-standalone, about 36 MB) ...\n' "$PY_VER"
            Pinned "$PY_URL" "$tc/downloads/$PY_FILE" "$PY_SHA"
        fi
        if [ "$need_zig" -eq 1 ]; then
            printf '  downloading the C linker (zig %s, about 49 MB) ...\n' "$ZIG_VER"
            Pinned "$ZIG_URL" "$tc/downloads/zig-$ZIG_VER.tar.xz" "$ZIG_SHA"
        fi
        if [ "$need_py" -eq 1 ]; then
            printf '  unpacking Python ...\n'
            rm -rf "$tc/pyx" "$tc/python"
            mkdir -p "$tc/pyx" && tar -xzf "$tc/downloads/$PY_FILE" -C "$tc/pyx" || Fail 'could not unpack Python'
            mv "$tc/pyx/python" "$tc/python" || Fail 'could not unpack Python (unexpected layout)'
            rm -rf "$tc/pyx"
            "$tc/python/bin/python3" -I -c 'import sys, lzma, ssl, hashlib' || Fail 'the downloaded Python does not run on this system'
            printf '%s\n' "$py_pin" >"$tc/python/.pin"
        fi
        if [ "$need_rust" -eq 1 ]; then
            printf '  unpacking Rust ...\n'
            rm -rf "$tc/rust"
            "$tc/python/bin/python3" -I "$root/tools/rust_unpack.py" "$tc/rust" "$tc/downloads/rustc-$RUST_VER.tar.xz" \
                "$tc/downloads/cargo-$RUST_VER.tar.xz" "$tc/downloads/rust-std-$RUST_VER.tar.xz" || Fail 'could not unpack Rust'
            "$tc/rust/bin/rustc" --version >/dev/null 2>&1 || Fail 'the downloaded Rust does not run on this system'
            printf '%s\n' "$rust_pin" >"$tc/rust/.pin"
        fi
        if [ "$need_zig" -eq 1 ]; then
            printf '  unpacking the C linker ...\n'
            rm -rf "$tc/zigx" "$tc/zig"
            mkdir -p "$tc/zigx" && UnXz "$tc/downloads/zig-$ZIG_VER.tar.xz" "$tc/zigx" || Fail 'could not unpack the C linker (zig)'
            for zd in "$tc"/zigx/*/; do
                [ -x "${zd}zig" ] && mv "${zd%/}" "$tc/zig"
            done
            rm -rf "$tc/zigx"
            [ -x "$tc/zig/zig" ] || Fail 'could not unpack the C linker (zig): unexpected layout'
            [ "$("$tc/zig/zig" version 2>/dev/null)" = "$ZIG_VER" ] || Fail 'the downloaded C linker (zig) does not run on this system'
            printf '%s\n' "$zig_pin" >"$tc/zig/.pin"
        fi
        # The archives are only needed for the unpacking (setup.ps1 does the same).
        rm -rf "$tc/downloads"
        t_used=$(( $(date +%s) - t_start ))
        printf '  tools ready in %d s\n' "$t_used"
    fi
    # The C linker as "cc": rustc calls it to link. It finds zig next to itself, so the folder can be moved.
    # (Linking against glibc 2.28 works on newer systems without --allow-shlib-undefined: tested on Ubuntu 24.04 and Arch.)
    cat >"$tc/bin/cc" <<'CCEOF'
#!/bin/sh
d=${0%/*}
exec "$d/../zig/zig" cc -target x86_64-linux-gnu.2.28 "$@"
CCEOF
    chmod +x "$tc/bin/cc"
fi

# Everything below uses only the private tools (or, with --system-toolchain, the system's).
if [ "$tc_mode" = pinned ] && [ "$check_only" -eq 0 ]; then
    PATH="$tc/bin:$tc/rust/bin:$tc/python/bin:$PATH"
    CARGO_HOME=$tc/cargo
    ZIG_GLOBAL_CACHE_DIR=$tc/zigcache/global
    ZIG_LOCAL_CACHE_DIR=$tc/zigcache/local
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$tc/bin/cc
    CC=$tc/bin/cc
    TMPDIR=$tc/tmp
    export PATH CARGO_HOME ZIG_GLOBAL_CACHE_DIR ZIG_LOCAL_CACHE_DIR CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER CC TMPDIR
    mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR" "$CARGO_HOME" "$TMPDIR"
fi

# From here on the ISO is a plain path. It may contain spaces and '&' (for example
# ".../Ratchet & Clank/Roms/Ratchet & Clank.iso"), so it is only ever passed as one
# element of a command list, never interpolated into a command string.
# Without --iso the disc of the last run is used (generated/play.cfg), so a rebuild
# needs no arguments at all.
if [ -z "$iso" ] && [ -f "$gen/play.cfg" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        case $line in
            iso=*) iso=$(trim "$(printf '%s' "$line" | sed -e 's/^iso=//')") ;;
        esac
    done <"$gen/play.cfg"
    # A relative path in the config is relative to this folder.
    if [ -n "$iso" ]; then
        case $iso in
            /*) ;;
            *) iso="$root/$iso" ;;
        esac
    fi
    if [ -n "$iso" ]; then
        printf '  disc image: %s (from generated/play.cfg)\n' "$iso"
    fi
fi

# Still no disc: look in the rom folder.
if [ -z "$iso" ]; then
    # The .iso files in the rom folder. The list is kept in the positional parameters,
    # because the paths may contain spaces.
    set --
    for f in "$romdir"/*.iso; do
        test -f "$f" || continue
        set -- "$@" "$f"
    done
    n_found=$#
    if [ "$n_found" -eq 0 ]; then
        printf '%s\n' \
            'ERROR: no disc image found: put your Ratchet & Clank disc image (any name ending in .iso) in the rom folder, or pass it with --iso' \
            '' \
            '  Put your Ratchet & Clank disc image in the rom folder' \
            '  (any name ending in .iso), or pass it:' \
            '' \
            '      ./setup.sh --iso "<path to your disc>.iso"' \
            '' \
            '  The supported discs are PAL / Europe v1.00 (SCES-50916) and' \
            '  USA v1.00 (SCUS-97199).' >&2
        exit 1
    fi
    if [ "$n_found" -eq 1 ]; then
        iso=$1
        printf '  disc image: %s (from the rom folder)\n' "$iso"
    elif [ -t 0 ]; then
        # More than one and a terminal to ask on: let the player pick.
        printf '\nMore than one disc image in the rom folder:\n'
        n=0
        for f in "$@"; do
            n=$((n + 1))
            printf '  %d) %s\n' "$n" "$f"
        done
        printf '\nWhich one? [1-%d] ' "$n_found"
        pick=''
        read pick || true
        case $pick in
            ''|*[!0-9]*) Fail 'no disc chosen' ;;
        esac
        [ "$pick" -ge 1 ] && [ "$pick" -le "$n_found" ] || Fail 'no disc chosen'
        n=0
        for f in "$@"; do
            n=$((n + 1))
            if [ "$n" -eq "$pick" ]; then
                iso=$f
                break
            fi
        done
        printf '  disc image: %s\n' "$iso"
    else
        # No terminal to ask on: take the newest and say which. The mtime is the entry's own (stat without -L), so a linked
        # ISO is dated by when it was linked in, not by the file it points at.
        newest=''
        newest_t=0
        for f in "$@"; do
            t=$(stat -c %Y -- "$f")
            if [ "$t" -gt "$newest_t" ]; then
                newest=$f
                newest_t=$t
            fi
        done
        iso=$newest
        printf '  disc image: %s (the newest in the rom folder)\n' "$iso"
    fi
fi

[ -n "$iso" ] || Fail 'no disc image given (use --iso "<path to your .iso>")'
[ -f "$iso" ] || Fail "disc image not found: $iso"

# The version this tree is, from the VERSION file. Written into
# generated/built_version.txt after a build; the game puts it in the window title.
build_version=$(trim "$(cat VERSION)")

# Rust crates are 97% of the compile work; they build at opt-level 3 in one codegen
# unit. Passed to cargo as a config file so rac-rs/Cargo.toml stays unchanged. (Step d
# in setup.ps1 does the same.)
# Only this toolchain is used and no build settings from your shell or your cargo config can change the build or its speed.
for v in RUSTC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS \
         CARGO_BUILD_TARGET CARGO_BUILD_RUSTFLAGS CARGO_BUILD_RUSTC_WRAPPER CARGO_TARGET_DIR \
         CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER \
         CARGO_PROFILE_RELEASE_OPT_LEVEL CARGO_PROFILE_RELEASE_LTO RUSTUP_DIST_SERVER \
         RUSTUP_TOOLCHAIN RUSTUP_HOME
do
    unset "$v"
done
# Cargo also reads .cargo/config.toml in every parent folder of this one (~/.cargo/config.toml counts when the game is below your
# home folder): a compiler wrapper such as sccache, linker flags. An EMPTY value in the environment beats all of them.
# "-A warnings": the generated crates and the game print thousands of harmless compiler warnings; errors are still printed.
RUSTC_WRAPPER=
RUSTC_WORKSPACE_WRAPPER=
RUSTFLAGS='-A warnings'
export RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS
if [ "$tc_mode" = pinned ]; then
    RUSTC=$tc/rust/bin/rustc
    export RUSTC
fi

# The generator flags, as in setup.ps1. --shared-slow writes the memory-access slow paths
# as shared helpers instead of inlining them.
gen_flags='--no-comments --compact --shared-slow --outline-mem --direct-mem --skin-ff --vu-guard'

# ---------------------------------------------------------------- (b) disc image
Step '(b) disc image'

# The two supported discs, v1.00 of each. They differ in size; the SHA-1 then proves the copy.
disc_size=0
disc_sha1=''
disc_name=''
# -L: follow a symlink, so an ISO linked into the rom folder is measured by its size and not the link's.
len=$(stat -L -c %s -- "$iso")
case $len in
    4213997568)
        disc_size=4213997568
        disc_sha1=B0DFBD8EAC0D84F173B6FC25D8A02A5FF22C652F
        disc_name='PAL / Europe v1.00 (SCES-50916)' ;;
    4214095872)
        disc_size=4214095872
        disc_sha1=AEE9275384F739C9B24CD5DE6F7E200292DB7B01
        disc_name='USA v1.00 (SCUS-97199)' ;;
    *)
        Fail "wrong size: $len bytes. Expected 4213997568 (Ratchet & Clank PAL/EU v1.00, SCES-50916) or 4214095872 (USA v1.00, SCUS-97199), both En/Fr/De/Es/It. Other regions and versions are not supported." ;;
esac
printf '  %s (%d bytes)\n' "$iso" "$len"
printf '  disc: Ratchet & Clank %s (by size)\n' "$disc_name"
sha1=$disc_sha1
if [ "$skip_hash" -eq 0 ]; then
    printf '  computing SHA-1 (about half a minute) ...\n'
    sha1=$(sha1sum <"$iso" | cut -d' ' -f1 | tr '[:lower:]' '[:upper:]')
    if [ "$sha1" != "$disc_sha1" ]; then
        Fail "SHA-1 mismatch: $sha1 (expected $disc_sha1). This is not the $disc_name disc image (re-dump, or another version)."
    fi
    printf '  SHA-1 ok: Ratchet & Clank %s\n' "$disc_name"
else
    printf '  SHA-1 skipped (--skip-hash); size matches\n'
fi
if [ "$check_only" -eq 1 ]; then
    printf 'Checks passed.\n'
    exit 0
fi

mkdir -p "$gen"

# The stamp is the disc, the generator flags and a hash of the generator files: if any of
# those change, the generated code is redone. (setup.ps1 keeps a list of older generator
# hashes that produce identical output, so that an update which only changed how the files
# are written does not regenerate. Nothing to keep compatible here.)
Stamp() {
    cat -- "$@" | sha256sum | cut -d' ' -f1
}

# Blocks until a level's .exit file shows up, or about 30 minutes, whichever is first.
# The wait is bounded so a generator killed outright cannot hang setup forever.
WaitExit() {
    spins=3600
    while [ ! -f "$1" ]; do
        sleep 0.5
        spins=$((spins - 1))
        if [ "$spins" -le 0 ]; then
            return 1
        fi
    done
    return 0
}

# Waits for level $1, prints its result line, and adds it to $failed when it did not succeed (or never reported: then the
# generator may still be running; the EXIT trap stops it).
ReapLevel() {
    rl=$1
    if WaitExit "$gen/logs/level_$rl.exit"; then
        rl_rc=$(trim "$(cat "$gen/logs/level_$rl.exit")")
    else
        rl_rc=timeout
    fi
    if [ "$rl_rc" = 0 ]; then
        printf '  level %s done: %s\n' "$rl" "$(tail -n 1 "$gen/logs/level_$rl.log")"
    else
        failed="$failed, $rl"
        printf '  level %s FAILED (see generated/logs/level_%s.log.err)\n' "$rl" "$rl" >&2
    fi
}

# ---------------------------------------------------------------- (c) generate
Step '(c) generating game code from the disc image'
# A new release unzipped by hand over an older install carries the ZIP's file times, so cargo could keep the old game: set the
# time of the release files to now, once (the launcher's update does this itself and leaves generated/synced_<version>).
if [ -f "$gen/built_version.txt" ] && [ -f "$root/tools/update_apply.py" ] && [ ! -f "$gen/synced_$build_version" ] \
   && [ "$(trim "$(cat "$gen/built_version.txt")")" != "$build_version" ]; then
    "$(PyExe)" -I "$root/tools/update_apply.py" stamp --root "$root" || Fail 'could not set the file times of the new release (tools/update_apply.py stamp)'
fi
orc=oracle
stamp=$gen/done.txt
gen_files='tools/recgen_iso.py tools/share_iso.py tools/fn_bounds.tsv oracle/stubs.txt'
if [ "$sha1" = AEE9275384F739C9B24CD5DE6F7E200292DB7B01 ]; then
    gen_files="$gen_files tools/fn_bounds_usa.tsv"
fi
gen_hash=$(Stamp $gen_files)
want="$sha1 $disc_size flags=$(join_words ' ' $gen_flags) gen=$gen_hash"
# A stamp that is empty means something upstream failed to compute. Two empty stamps would
# compare equal and silently skip the generation, so refuse to continue instead.
[ ${#want} -gt 40 ] || Fail "internal error: could not build the generator stamp"
have=''
if [ -f "$stamp" ]; then have=$(trim "$(cat "$stamp")"); fi
[ -f "$orc/game/src/lib.rs" ] || have=''
[ -f "$orc/rec/l18_00/Cargo.toml" ] || have=''

if [ "$regenerate" -eq 1 ] || [ "$have" != "$want" ]; then
    # The generators write in place and only touch files whose content differs, so
    # cargo rebuilds by file mtime and an unchanged regenerate costs nothing extra.
    rm -f "$stamp"
    rm -rf "$gen/logs"
    mkdir -p "$gen/logs"
    # Upstream's sharing pass, step one of three. share_iso.py groups the functions that every
    # level repeats into shared crates (rec/sh_NN), and each copy becomes a one-line wrapper.
    # This first step records the content and time of every generated file, before the levels
    # are generated, so the last step can put the times back and cargo rebuilds only what
    # really changed.
    python3 tools/share_iso.py --rec "$orc/rec" --snapshot "$gen/logs/gen_files_before.txt"
    [ $? -eq 0 ] || Fail 'listing the generated files failed (tools/share_iso.py --snapshot)'
    # The shared templates of the previous generation go (setup.ps1 does the same): the sharing pass below refuses to run on
    # a rec/ that already has sh_NN crates, and it rebuilds them from the new level crates, which recgen_iso.py writes in full.
    for sh_d in "$root/$orc"/rec/sh_*; do
        [ -d "$sh_d" ] && rm -rf "$sh_d"
    done
    # Levels 0..18, --jobs of them at a time. The levels are started in order and reaped in
    # order, so the oldest running one is always the one to reap: two counters are enough
    # for the queue, and no list is needed.
    # Each level writes its own exit status to generated/logs/level_N.exit, and that file
    # appearing is what marks the level finished. The block writes it whether the generator
    # succeeded or not, so it always appears.
    started=0
    reaped=0
    failed=''
    while [ "$started" -le 18 ]; do
        while [ $((started - reaped)) -ge "$jobs" ]; do
            l=$reaped
            reaped=$((reaped + 1))
            ReapLevel "$l"
        done
        set -- python3 -I tools/recgen_iso.py --iso "$iso" --level "$started" --out "$orc" --oracle "$orc"
        if [ "$started" -gt 0 ]; then set -- "$@" --keep-boot; fi
        set -- "$@" $gen_flags
        {
            "$@" >"$gen/logs/level_$started.log" 2>"$gen/logs/level_$started.log.err"
            echo $? >"$gen/logs/level_$started.exit"
        } &
        started=$((started + 1))
    done
    while [ "$reaped" -lt "$started" ]; do
        l=$reaped
        reaped=$((reaped + 1))
        ReapLevel "$l"
    done
    [ -z "$failed" ] || Fail "generation failed for level(s) $failed; logs are in generated/logs"
    # The parallel generators race on game/src/lib.rs: rewrite it once from what is on disk.
    python3 tools/recgen_iso.py --facade-only --out "$orc" --oracle "$orc"
    [ $? -eq 0 ] || Fail 'facade rewrite failed'
    # The sharing pass itself: the shared crates appear and every level crate holds wrappers.
    python3 tools/share_iso.py --rec "$orc/rec"
    [ $? -eq 0 ] || Fail 'sharing pass failed (tools/share_iso.py)'
    # The pass rewrote the crates, so the facade is rewritten again to match.
    python3 tools/recgen_iso.py --facade-only --out "$orc" --oracle "$orc"
    [ $? -eq 0 ] || Fail 'facade rewrite failed'
    # And the file times go back, so cargo does not rebuild every crate after a regeneration.
    python3 tools/share_iso.py --rec "$orc/rec" --keep-times "$gen/logs/gen_files_before.txt"
    [ $? -eq 0 ] || Fail 'restoring file times failed (tools/share_iso.py --keep-times)'
    # Every crate must be in the facade (the concurrent generators rewrite it).
    n_crates=$(find "$orc/rec" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
    n_reg=$(grep -c '::FNS)' "$orc/game/src/lib.rs")
    [ "$n_crates" -eq "$n_reg" ] || Fail "facade lists $n_reg of $n_crates crates; rerun with --regenerate --jobs 1"
    printf '%s\n' "$want" >"$stamp"
    printf '  generated %d crates\n' "$n_crates"
else
    printf '  already generated for this disc image (use --regenerate to redo)\n'
fi

# ---------------------------------------------------------------- (c2) VU microcode translations
# The VU0/VU1 microprograms (graphics/maths code of the game) are cut out of YOUR disc image
# and translated to Rust. None of that is shipped. (Same as setup.ps1 step c2.)
Step '(c2) generating the VU microcode translations from the disc image'
vu_stamp=$gen/vu_done.txt
# Every file of tools/vu (as setup.ps1 does), so a file added there changes the stamp too; sorted, so the stamp does not depend on the locale.
vu_files="tools/vugen_iso.py tools/skinff_ee.py tools/skinff_gen.py $(find tools/vu -maxdepth 1 -type f | LC_ALL=C sort | tr '\n' ' ')"
vu_want="$sha1 vu1 gen=$(Stamp $vu_files)"
vu_have=''
if [ -f "$vu_stamp" ]; then vu_have=$(trim "$(cat "$vu_stamp")"); fi
[ -f "$orc/src/vu0_prog0.rs" ] || vu_have=''
[ -f "rac-rs/src/gsfull/vu1aot_gen.rs" ] || vu_have=''
[ -f "rac-rs/src/particles/orb_tables.rs" ] || vu_have=''

if [ "$regenerate" -eq 1 ] || [ "$vu_have" != "$vu_want" ]; then
    # Not -I: vugen_iso.py puts tools/vu on sys.path itself.
    python3 tools/vugen_iso.py --iso "$iso" --tree "$root"
    [ $? -eq 0 ] || Fail 'VU microcode generation failed (tools/vugen_iso.py)'
    printf '%s\n' "$vu_want" >"$vu_stamp"
    printf '  done\n'
else
    printf '  already generated (use --regenerate to redo)\n'
fi

if [ "$no_build" -eq 1 ]; then
    printf 'Generation done (--no-build).\n'
    exit 0
fi

# The generated crates are what the build needs; if step (c) was skipped or half-done,
# cargo would fail with a confusing "no matching package named ee-game".
[ -f "$orc/game/src/lib.rs" ] || Fail 'the generated game code is missing; rerun with --regenerate'
[ -d "$orc/rec" ] || Fail 'the generated game code is missing; rerun with --regenerate'

# ---------------------------------------------------------------- (d) build
Step '(d) building the game (release)'
# The launcher's progress bar follows cargo's "Compiling" lines. Generated crates plus the
# downloaded libraries are what a first build compiles; a rebuild needs fewer (an update tells how many).
n_gen=$(find "$orc/rec" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
if [ -n "$expect_compiles" ]; then
    printf 'expect-compiles: %d\n' "$expect_compiles"
else
    printf 'expect-compiles: %d\n' $((n_gen + 150))
fi
# This takes a while: the 427 generated crates are 97% of the compile work. Cargo prints
# every crate as it goes, so the output below is the progress. Set CARGO_BUILD_JOBS (or --cargo-jobs) to
# use fewer cores than the machine has.
prof=$gen/build_profile.toml
printf '%s\n' '# written by setup.sh (step d): build profile of the generated crates' \
    '[profile.release.package.ee-game]' 'opt-level = 3' 'codegen-units = 1' >"$prof"
# One opt-level 3 / codegen-units 1 table per generated crate. The crate directory names
# (l0_00, boot_00, ...) become package names with underscores turned into dashes.
for p in $(find "$orc/rec" -mindepth 1 -maxdepth 1 -type d); do
    printf '\n[profile.release.package.rec-%s]\nopt-level = 3\ncodegen-units = 1\n' \
        "$(basename "$p" | tr '_' '-')" >>"$prof"
done
# The private toolchain links alsa and udev by file name instead of by pkg-config: the config tells cargo that the two
# "links" libraries (alsa-sys, libudev-sys) are already described, so their build scripts (which need pkg-config and the
# -dev packages) are not run. The libraries are found by ldconfig; the game loads the same files at run time.
linkcfg=$gen/linux_link.toml
if [ "$tc_mode" = pinned ]; then
    alsa_dir=$(FindLibDir libasound.so.2) || Fail 'libasound.so.2 is missing (install the ALSA library, e.g. libasound2t64 / alsa-lib)'
    udev_dir=$(FindLibDir libudev.so.1) || Fail 'libudev.so.1 is missing (install libudev1 / systemd-libs)'
    # zig does not accept "-l:libasound.so.2" (tested), so a private folder gets the two libraries under their plain names
    # (libasound.so, libudev.so: symbolic links to the system files) and is the only search folder.
    sysl=$tc/syslibs
    rm -rf "$sysl" && mkdir -p "$sysl" || Fail "cannot create $sysl"
    ln -s "$alsa_dir/libasound.so.2" "$sysl/libasound.so" && ln -s "$udev_dir/libudev.so.1" "$sysl/libudev.so" || Fail "cannot link the system libraries into $sysl"
    case $sysl in
        *"'"*) Fail "the folder name contains a quote character, which the build settings cannot hold: $root" ;;
    esac
    {
        printf '# written by setup.sh (step d): link settings for the private toolchain\n'
        printf '[target.x86_64-unknown-linux-gnu.alsa]\n'
        printf "rustc-link-search = ['%s']\n" "$sysl"
        printf 'rustc-link-lib = ["asound"]\n\n'
        printf '[target.x86_64-unknown-linux-gnu.libudev]\n'
        printf "rustc-link-search = ['%s']\n" "$sysl"
        printf 'rustc-link-lib = ["udev"]\n'
    } >"$linkcfg"
    # A first, small link test: finds a broken zig in seconds instead of after the compile.
    printf 'fn main() { println!("ok"); }\n' >"$tc/tmp/linktest.rs"
    if ! lt_out=$(rustc -O -C linker="$tc/bin/cc" -o "$tc/tmp/linktest" "$tc/tmp/linktest.rs" 2>&1) || [ "$("$tc/tmp/linktest" 2>/dev/null)" != ok ]; then
        printf 'ERROR: the C linker (zig) failed a test link on this system:\n%s\n' "$lt_out" >&2
        exit 1
    fi
    rm -f "$tc/tmp/linktest" "$tc/tmp/linktest.rs"
fi
# nice: the build yields to whatever else the machine is doing (the desktop stays usable).
# Differences from setup.ps1 step d: no PGO (the speed profiles in pgo/ are recorded with the Windows toolchain), so the Linux build
# can be slower than the Windows player build.
# --features rac-rs-exe/windows_gui is the public build: it also writes panics and the final error to player/crash.log
# (the feature name is historical; on Linux it also turns on the update notice window and the crash/panic message windows).
if [ -z "$cargo_jobs" ] && [ -z "$CARGO_BUILD_JOBS" ]; then
    cargo_jobs=$(AutoCargoJobs)
    printf '  compiling with %s parallel jobs (the cores and the free memory decide; --cargo-jobs N changes it)\n' "$cargo_jobs"
fi
BuildEstimate "${cargo_jobs:-$CARGO_BUILD_JOBS}"
set -- cargo build --release --manifest-path rac-rs/Cargo.toml --target-dir build \
    -p rac-rs-exe --bin rac-rs --features rac-rs-exe/windows_gui --config "$prof"
[ "$tc_mode" != pinned ] || set -- "$@" --config "$linkcfg"
[ -z "$cargo_jobs" ] || set -- "$@" --jobs "$cargo_jobs"
# The exit status of cargo is captured inside the braces, because a pipeline's status is
# the last command's (tee), which would hide a failed build. The build runs in the background and is waited
# for, so that a "kill" of this script (the trap above) can stop it.
rm -f "$gen/.build_rc"
{ { nice -n 10 "$@" 2>&1; echo $? >"$gen/.build_rc"; } | tee "$gen/build.log"; } &
wait $!
build_rc=$(cat "$gen/.build_rc" 2>/dev/null || echo 1)
rm -f "$gen/.build_rc"
exe=build/release/rac-rs
[ "$build_rc" -eq 0 ] || Fail "cargo build failed (exit $build_rc; the log is in generated/build.log)"
[ -f "$exe" ] || Fail "build finished but $exe is missing"
printf '  built %s\n' "$exe"
printf '%s\n' "$build_version" >"$gen/built_version.txt"

# ---------------------------------------------------------------- (d2) the bundled mod loader and mods
# The Linux counterpart of setup.ps1 Install-Bundled: bundled/libomniloader.so goes next to the game (build/release, where
# the game looks for it), bundled/mods/<id> go into mods/. A loader or a mod with a known version that is the same or newer
# stays, an older one is replaced; an installed one whose version cannot be known is replaced too, the old one kept
# (loader: libomniloader.so.prev, mod: mod_backups/). A bundled version that cannot be read changes nothing. mods/active.txt and
# mods/settings.json (which mods are on, and their settings) are never touched. Bundled mods start switched off.
# One rule for the loader, the same as setup.ps1 (Install-BundledLoader) and the launcher's loader update:
#   known version = MAJOR.MINOR.PATCH[-pre] AND a sidecar <file>.json with a sha256 equal to the file's (a missing sidecar, a
#   sidecar of another file or an unreadable version = unknown). 1 not installed: install. 2 same SHA-256: nothing (the bundled
#   sidecar is written when the installed one is missing or stale). 3 bundled version unknown: leave. 4 installed known:
#   same or newer stays, older is replaced. 5 installed unknown: replaced. 6 every replacement keeps the old file as .prev, and
#   its sidecar as .json.prev only when the sidecar belonged to it (else a leftover .json.prev is deleted).
# Any trouble here is only said: the game itself is built and runs without mods.

# First string value of a key in a small JSON file ("" when the file or the key is missing).
JsonStr() {
    [ -f "$1" ] || return 0
    grep -o "\"$2\"[[:space:]]*:[[:space:]]*\"[^\"]*\"" "$1" 2>/dev/null | head -n 1 | cut -d'"' -f4
}

# VerClean V prints V without surrounding blanks and without a leading v.
VerClean() {
    printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e 's/^[vV]//'
}

# VerIsNum S: success when S is 0 or a number without leading zeros of at most 18 digits.
VerIsNum() {
    case $1 in
        0) return 0 ;;
        '' | *[!0-9]* | 0*) return 1 ;;
    esac
    [ "${#1}" -le 18 ]
}

# VerOk V: success when V is a version: MAJOR.MINOR.PATCH (numbers as VerIsNum) with an optional -pre of letters, digits, dots
# and dashes without empty dot parts. A leading v and surrounding blanks are ignored.
VerOk() (
    set -f
    v=$(VerClean "$1")
    core=${v%%-*}
    case $v in
        *-*)
            case ${v#*-} in
                '' | *[!A-Za-z0-9.-]* | .* | *. | *..*) exit 1 ;;
            esac
            ;;
    esac
    case $core in
        '' | *[!0-9.]* | .* | *. | *..* | *.*.*.*) exit 1 ;;
        *.*.*) ;;
        *) exit 1 ;;
    esac
    IFS=.
    set -- $core
    for p in "$@"; do
        VerIsNum "$p" || exit 1
    done
    exit 0
)

# VerCmp A B prints -1, 0 or 1 when version A is below, equal to or above B, and x when either is not a version (VerOk). A pre-release
# is below the plain version; pre parts are compared as numbers when both are numbers (rc.2 < rc.10), a number is below a word, words
# compare as bytes. The same rules as setup.ps1 (Compare-ModVersion) and the launcher.
VerCmp() (
    LC_ALL=C
    export LC_ALL
    set -f
    if ! VerOk "$1" || ! VerOk "$2"; then printf 'x'; exit 0; fi
    a=$(VerClean "$1")
    b=$(VerClean "$2")
    pa=
    pb=
    case $a in *-*) pa=${a#*-}; a=${a%%-*} ;; esac
    case $b in *-*) pb=${b#*-}; b=${b%%-*} ;; esac
    IFS=.
    set -- $a
    a1=$1; a2=$2; a3=$3
    set -- $b
    b1=$1; b2=$2; b3=$3
    unset IFS
    for pair in "$a1 $b1" "$a2 $b2" "$a3 $b3"; do
        set -- $pair
        if [ "$1" -lt "$2" ]; then printf -- '-1'; exit 0; fi
        if [ "$1" -gt "$2" ]; then printf '1'; exit 0; fi
    done
    if [ -z "$pa" ] && [ -z "$pb" ]; then printf '0'; exit 0; fi
    if [ -z "$pa" ]; then printf '1'; exit 0; fi
    if [ -z "$pb" ]; then printf -- '-1'; exit 0; fi
    while :; do
        if [ -z "$pa" ] && [ -z "$pb" ]; then printf '0'; exit 0; fi
        if [ -z "$pa" ]; then printf -- '-1'; exit 0; fi
        if [ -z "$pb" ]; then printf '1'; exit 0; fi
        ia=${pa%%.*}
        ib=${pb%%.*}
        case $pa in *.*) pa=${pa#*.} ;; *) pa= ;; esac
        case $pb in *.*) pb=${pb#*.} ;; *) pb= ;; esac
        na=0
        nb=0
        if VerIsNum "$ia"; then na=1; fi
        if VerIsNum "$ib"; then nb=1; fi
        if [ "$na" = 1 ] && [ "$nb" = 1 ]; then
            if [ "$ia" -lt "$ib" ]; then printf -- '-1'; exit 0; fi
            if [ "$ia" -gt "$ib" ]; then printf '1'; exit 0; fi
        elif [ "$na" = 1 ]; then
            printf -- '-1'; exit 0
        elif [ "$nb" = 1 ]; then
            printf '1'; exit 0
        elif [ "$ia" != "$ib" ]; then
            if [ "x$ia" \< "x$ib" ]; then printf -- '-1'; exit 0; fi
            printf '1'; exit 0
        fi
    done
)

# SidecarOwns FILE: success when FILE.json holds a sha256 equal to the SHA-256 of FILE (a sidecar of another file, or one without
# a sha256, is stale and is not believed).
SidecarOwns() {
    [ -f "$1" ] && [ -f "$1.json" ] || return 1
    _sc=$(JsonStr "$1.json" sha256 | tr '[:upper:]' '[:lower:]')
    [ -n "$_sc" ] && [ "$_sc" = "$(Sha256 "$1")" ]
}

# LoaderVersion FILE prints the known version of a loader file (its sidecar belongs to it and the version is readable), else nothing.
LoaderVersion() {
    SidecarOwns "$1" || return 0
    _lv=$(JsonStr "$1.json" version)
    if VerOk "$_lv"; then printf '%s' "$_lv"; fi
    return 0
}

# Copy FILE to DEST through a temporary name and a rename, so a game that is running keeps the file it has mapped.
SafeCopy() {
    cp -f "$1" "$2.new" && mv -f "$2.new" "$2"
}

InstallBundledLoader() {
    src=$root/bundled/libomniloader.so
    if [ ! -f "$src" ]; then
        printf '  mod loader: this release has no libomniloader.so (script mods stay off)\n'
        return 0
    fi
    dst=$root/build/release/libomniloader.so
    mkdir -p "$root/build/release" || return 1
    if [ ! -f "$dst" ]; then
        SafeCopy "$src" "$dst" || return 1
        [ ! -f "$src.json" ] || SafeCopy "$src.json" "$dst.json"
        printf '  mod loader: libomniloader.so installed next to the game\n'
        return 0
    fi
    # rule 2: the same file; the bundled sidecar is written when the installed one is missing or names another file
    if [ "$(Sha256 "$src")" = "$(Sha256 "$dst")" ]; then
        if [ -f "$src.json" ] && ! SidecarOwns "$dst"; then SafeCopy "$src.json" "$dst.json"; fi
        printf '  mod loader: libomniloader.so is already there\n'
        return 0
    fi
    have_v=$(LoaderVersion "$dst")
    new_v=$(LoaderVersion "$src")
    # rule 3: the bundled version cannot be known, so nothing can be compared
    if [ -z "$new_v" ]; then
        printf '  mod loader: libomniloader.so left as it is (the bundled version "%s" cannot be compared)\n' "$(JsonStr "$src.json" version)"
        return 0
    fi
    # rule 4: a known installed version that is the same or newer stays (rule 5: an unknown one is replaced)
    if [ -n "$have_v" ] && [ "$(VerCmp "$have_v" "$new_v")" != '-1' ]; then
        printf '  mod loader: libomniloader.so %s not copied (the installed one is %s, the same or newer)\n' "$new_v" "$have_v"
        return 0
    fi
    # rule 6: keep the old file as .prev (the launcher can put it back), its sidecar as .json.prev only when it belongs to it
    SafeCopy "$dst" "$dst.prev" || return 1
    if SidecarOwns "$dst"; then
        SafeCopy "$dst.json" "$dst.json.prev" || return 1
    else
        rm -f "$dst.json.prev"
    fi
    SafeCopy "$src" "$dst" || return 1
    if [ -f "$src.json" ]; then SafeCopy "$src.json" "$dst.json"; else rm -f "$dst.json"; fi
    if [ -n "$have_v" ]; then
        printf '  mod loader: libomniloader.so %s replaces %s (the old one is kept as libomniloader.so.prev)\n' "$new_v" "$have_v"
    else
        printf '  mod loader: libomniloader.so %s replaces a loader of unknown version (the old one is kept as libomniloader.so.prev)\n' "$new_v"
    fi
    return 0
}

# The manifest file of a mod folder: omni-mod.json, else openrac-mod.json ("" when there is none).
ModManifest() {
    if [ -f "$1/omni-mod.json" ]; then printf '%s' "$1/omni-mod.json"
    elif [ -f "$1/openrac-mod.json" ]; then printf '%s' "$1/openrac-mod.json"
    fi
}

InstallBundledMod() {
    m=$1
    name=$(basename "$m")
    mods=$root/mods
    new_mf=$(ModManifest "$m")
    new_id=$(JsonStr "$new_mf" id)
    new_v=$(JsonStr "$new_mf" version)
    # the installed copy: the folder of the same name, unless it holds another mod; else the folder with the same id
    have=
    have_id=
    have_v=
    if [ -d "$mods/$name" ]; then
        have=$mods/$name
        hm=$(ModManifest "$have")
        have_id=$(JsonStr "$hm" id)
        have_v=$(JsonStr "$hm" version)
    fi
    if [ -n "$new_id" ] && [ "$have_id" != "$new_id" ] && [ -d "$mods" ]; then
        for d in "$mods"/*/; do
            [ -d "$d" ] || continue
            d=${d%/}
            [ "$d" != "$have" ] || continue
            hm=$(ModManifest "$d")
            if [ -n "$hm" ] && [ "$(JsonStr "$hm" id)" = "$new_id" ]; then
                have=$d
                have_id=$new_id
                have_v=$(JsonStr "$hm" version)
                break
            fi
        done
    fi
    if [ -z "$have" ]; then
        mkdir -p "$mods" && cp -R "$m" "$mods/$name" || return 1
        printf '  bundled mod: %s in mods/%s (off until you turn it on)\n' "$name" "$name"
        return 0
    fi
    if [ -n "$new_id" ] && [ -n "$have_id" ] && [ "$have_id" != "$new_id" ]; then
        printf '  bundled mod: %s not copied (mods/%s holds another mod, %s)\n' "$name" "$(basename "$have")" "$have_id"
        return 0
    fi
    # never replace what is not clearly older: an equal or higher version, or a bundled version that cannot be read; an installed
    # version that cannot be read (and a readable bundled one) is replaced, the old folder is kept in mod_backups
    c=$(VerCmp "$have_v" "$new_v")
    if [ "$c" = x ]; then
        if ! VerOk "$new_v"; then
            printf '  bundled mod: %s not copied (its version "%s" cannot be read, so mods/%s is left as it is)\n' "$name" "$new_v" "$(basename "$have")"
            return 0
        fi
    elif [ "$c" != '-1' ]; then
        printf '  bundled mod: %s %s not copied (mods/%s has %s, which is the same or newer)\n' "$name" "$new_v" "$(basename "$have")" "$have_v"
        return 0
    fi
    # older (or an installed version that cannot be read): move the installed folder whole to mod_backups, then copy the bundled one
    tag=$(printf '%s' "$have_v" | tr -c 'A-Za-z0-9._-' '_' | cut -c1-40)
    [ -n "$tag" ] || tag=unknown
    bk=$root/mod_backups/$(basename "$have")-$tag
    n=2
    while [ -e "$bk" ]; do bk=$root/mod_backups/$(basename "$have")-$tag-$n; n=$((n + 1)); done
    mkdir -p "$root/mod_backups" && mv "$have" "$bk" || return 1
    if ! cp -R "$m" "$have"; then
        rm -rf "$have"
        mv "$bk" "$have"
        return 1
    fi
    printf '  bundled mod: %s %s replaces %s in mods/%s; the old one is kept in mod_backups/%s\n' "$name" "$new_v" "${have_v:-an unreadable version}" "$(basename "$have")" "$(basename "$bk")"
    return 0
}

InstallBundled() {
    InstallBundledLoader || printf '  mod loader: could not be installed; the game still runs, without mod support\n'
    [ -d "$root/bundled/mods" ] || return 0
    for md in "$root/bundled/mods"/*/; do
        [ -d "$md" ] || continue
        InstallBundledMod "${md%/}" || printf '  bundled mod %s: could not be installed; the game still runs, without that mod\n' "$(basename "$md")"
    done
    return 0
}

Step '(d2) mod loader and bundled mods'
InstallBundled

# ---------------------------------------------------------------- (e) play
Step '(e) play.cfg'
# generated/play.cfg: key=value, UTF-8, LF. The game, started with no arguments, reads it to
# find the disc image; it then sets the environment itself, so play.sh needs no arguments.
# The disc path is relative to this folder when the disc is inside it, so the whole folder can
# be moved (or copied to another machine) and still find the disc. The path is used as it was
# given, not resolved, so a linked disc stays a link.
stored=$iso
for prefix in "$root/" "$(pwd -P)/"; do
    case $iso in
        "$prefix"*) stored=${iso#"$prefix"}; break ;;
    esac
done
printf 'iso=%s\n' "$stored" >"$gen/play.cfg"
printf '  wrote generated/play.cfg\n'
# The marker of an update that put its files in place (see update_apply.py) has done its job.
rm -f "$gen"/synced_* "$gen/update_expect.txt"

Step 'done'
printf 'To play:  ./play.sh\n'
printf 'For a menu entry:  ./install-desktop-entry.sh\n'

# ---------------------------------------------------------------- pause
# Started from a file manager, the window would close at once and the log would be gone
# with it. So wait for a key. --no-pause (or RAC_NO_PAUSE=1) skips it.
if [ "$no_pause" -eq 0 ] && [ -z "$RAC_NO_PAUSE" ] && [ -t 0 ]; then
    printf '\nPress enter to close...'
    read -r _ || true
fi
