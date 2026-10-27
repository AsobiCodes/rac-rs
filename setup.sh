#!/bin/sh
# setup.sh - builds rac-rs (a PC port of Ratchet & Clank, PS2) on Linux from your own disc image.
#
# A Linux stand-in for the upstream setup.ps1 (which is Windows PowerShell 5.1). It runs the
# project's own generators and cargo, and nothing else. The Windows tools it replaces are not
# used here: no rustup, no embeddable Python, no MSYS2 GNU assembler. Your system Rust and
# Python are used as they are, so the pinned-toolchain download (~1.2 GB) does not happen.
#
# Steps, in order:
#   (b)  check that the .iso is Ratchet & Clank PAL/EU v1.00 (SCES-50916) or USA v1.00 (SCUS-97199)
#   (c)  generate the game code from the disc with tools/recgen_iso.py (boot + 19 levels)
#   (c2) generate the VU microcode translations with tools/vugen_iso.py
#   (d)  build the game with cargo (release) into ./build
#   (e)  write generated/play.cfg, so ./build/release/rac-rs can start on its own
#
# Everything it writes stays inside this folder: oracle/rec, oracle/game, the generated .rs
# files named by .gitignore, generated/, build/, and player/ once you play. It writes nothing
# outside this folder at all. The application menu entry is a separate script,
# ./install-desktop-launcher.sh, so that it stays that way.
#
# This is plain POSIX shell, so it runs under whatever /bin/sh happens to be: bash on Arch
# and Fedora, dash on Debian and Ubuntu. Nothing else is needed.
#
# Usage:
#   ./setup.sh                       # the disc in the rom folder, or the one from the last run
#   ./setup.sh --iso "<path to your .iso>"
#   ./setup.sh --check-only          # just verify the disc
#
# Options:
#   --iso PATH     the disc image (or pass it as the first plain argument)
#   -j, --jobs N   level generators in parallel (default 4)
#   --check-only   do (b) only
#   --skip-hash    size check only, skip the SHA-1
#   --regenerate   redo (c) and (c2) even if the stamps say they are current
#   --no-build     stop after (c2)
#   --no-pause     do not wait for a key at the end (the default waits, so a run
#                  started from a file manager keeps its window open)
#   -h, --help     this text
#
# The environment:
#   CARGO_BUILD_JOBS=N   how many cores the compile uses (cargo takes them all otherwise)
#   CARGO_NO_PAUSE=1     the same as --no-pause
#

root=$(realpath "$(dirname "$0")") || exit 1
cd "$root" || exit 1

Fail() {
    printf 'ERROR: %s\n' "$1" >&2
    exit 1
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

# ---------------------------------------------------------------- arguments
iso=''
jobs=4
skip_hash=0
check_only=0
regenerate=0
no_build=0
no_pause=0

while [ $# -gt 0 ]; do
    case $1 in
        --iso)
            [ $# -ge 2 ] || Fail '--iso needs a path'
            iso=$2
            shift 2 ;;
        -j|--jobs)
            [ $# -ge 2 ] || Fail '--jobs needs a number'
            jobs=$2
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
        -h|--help)
            last=$(grep -n '^#   -h, --help' "$0" | cut -d: -f1)
            head -n "$last" "$0" | tail -n +2
            exit 0 ;;
        *)
            [ -z "$iso" ] || Fail 'the disc image was given twice'
            iso=$1
            shift ;;
    esac
done

# The folder the setup writes its logs, stamps and the play configuration to.
gen=generated
# The folder a disc image is dropped into.
romdir=rom

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
            'No disc image found.' \
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

# The version this tree is, from the VERSION file (0.2.2). Written into
# generated/built_version.txt after a build; the game puts it in the window title.
build_version=$(trim "$(cat VERSION)")

# Rust crates are 97% of the compile work; they build at opt-level 3 in one codegen
# unit. Passed to cargo as a config file so rac-rs/Cargo.toml stays unchanged. (Step d
# in setup.ps1 does the same.)
# Only this toolchain is used and no build settings from your shell config can change
# the build or its speed.
for v in RUSTC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER RUSTFLAGS CARGO_ENCODED_RUSTFLAGS \
         CARGO_BUILD_TARGET CARGO_BUILD_RUSTFLAGS CARGO_BUILD_RUSTC_WRAPPER CARGO_TARGET_DIR \
         CARGO_PROFILE_RELEASE_OPT_LEVEL CARGO_PROFILE_RELEASE_LTO RUSTUP_DIST_SERVER
do
    unset "$v"
done
command -v cargo >/dev/null 2>&1 || Fail 'cargo not found (install the rust toolchain: https://rustup.rs)'
command -v python3 >/dev/null 2>&1 || Fail 'python3 not found (the generators need it)'

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
    sha1=$(sha1sum -- "$iso" | cut -d' ' -f1 | tr '[:lower:]' '[:upper:]')
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

# ---------------------------------------------------------------- (c) generate
Step '(c) generating game code from the disc image'
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
            WaitExit "$gen/logs/level_$l.exit"
            rc=$(trim "$(cat "$gen/logs/level_$l.exit")")
            if [ "$rc" = 0 ]; then
                printf '  level %s done: %s\n' "$l" "$(tail -n 1 "$gen/logs/level_$l.log")"
            else
                failed="$failed, $l"
                printf '  level %s FAILED (see generated/logs/level_%s.log.err)\n' "$l" "$l" >&2
            fi
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
        WaitExit "$gen/logs/level_$l.exit"
        rc=$(trim "$(cat "$gen/logs/level_$l.exit")")
        if [ "$rc" = 0 ]; then
            printf '  level %s done: %s\n' "$l" "$(tail -n 1 "$gen/logs/level_$l.log")"
        else
            failed="$failed, $l"
            printf '  level %s FAILED (see generated/logs/level_%s.log.err)\n' "$l" "$l" >&2
        fi
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
vu_files='tools/vugen_iso.py tools/skinff_ee.py tools/skinff_gen.py tools/vu/profile.txt tools/vu/vu0gen.py tools/vu/vu1gen.py tools/vu/vu_disasm.py tools/vu/vuclean.py tools/vu/vudec.py tools/vu/vugraph.py'
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
# This takes a while: the 427 generated crates are 97% of the compile work. Cargo prints
# every crate as it goes, so the output below is the progress. Set CARGO_BUILD_JOBS to
# use fewer cores than the machine has (cargo takes them all otherwise).
prof=$gen/build_profile.toml
printf '%s\n' '# written by setup.sh (step d): build profile of the generated crates' \
    '[profile.release.package.ee-game]' 'opt-level = 3' 'codegen-units = 1' >"$prof"
# One opt-level 3 / codegen-units 1 table per generated crate. The crate directory names
# (l0_00, boot_00, ...) become package names with underscores turned into dashes.
for p in $(find "$orc/rec" -mindepth 1 -maxdepth 1 -type d); do
    printf '\n[profile.release.package.rec-%s]\nopt-level = 3\ncodegen-units = 1\n' \
        "$(basename "$p" | tr '_' '-')" >>"$prof"
done
# nice: the build yields to whatever else the machine is doing (the desktop stays usable).
# The exit status of cargo is captured inside the braces, because a pipeline's status is
# the last command's (tee), which would hide a failed build.
{ nice -n 10 cargo build --release --manifest-path rac-rs/Cargo.toml --target-dir build \
    -p rac-rs-exe --bin rac-rs --config "$prof" 2>&1; echo $? >"$gen/.build_rc"; } | tee "$gen/build.log"
build_rc=$(cat "$gen/.build_rc")
rm -f "$gen/.build_rc"
exe=build/release/rac-rs
[ "$build_rc" -eq 0 ] || Fail "cargo build failed (exit $build_rc; the log is in generated/build.log)"
[ -f "$exe" ] || Fail "build finished but $exe is missing"
printf '  built %s\n' "$exe"
printf '%s\n' "$build_version" >"$gen/built_version.txt"

# ---------------------------------------------------------------- (e) play
# generated/play.cfg: key=value, UTF-8, LF. The game, started with no arguments, reads it to
# find the disc image; it then sets the environment itself, so play.sh needs no arguments.
# The disc path is relative to this folder when the disc is inside it, so the whole folder can
# be moved (or copied to another machine) and still find the disc. The path is used as it was
# given, not resolved, so a linked disc stays a link.
stored=$iso
prefix="$root/"
case $iso in
    "$prefix"*) stored=${iso#"$prefix"} ;;
esac
printf 'iso=%s\n' "$stored" >"$gen/play.cfg"
printf '  wrote generated/play.cfg\n'

Step 'done'
printf 'To play:  ./play.sh\n'
printf 'For a menu entry:  ./install-desktop-launcher.sh\n'

# ---------------------------------------------------------------- pause
# Started from a file manager, the window would close at once and the log would be gone
# with it. So wait for a key. --no-pause (or RAC_NO_PAUSE=1) skips it.
if [ "$no_pause" -eq 0 ] && [ -z "$RAC_NO_PAUSE" ] && [ -t 0 ]; then
    printf '\nPress enter to close...'
    read -r _ || true
fi
