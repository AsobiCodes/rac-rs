#!/bin/sh
# Builds the Linux launcher. Run it from anywhere; the game itself is built by ../setup.sh.
#
#   ./build.sh                    release build in launcher/target/release/rac-rs-launcher
#   ./build.sh --install-desktop  also add "RAC-RS Launcher" to the application menu
#
# Any other argument goes to cargo (for example --offline).
set -eu
cd "$(dirname "$0")"
install=0
args=""
for a in "$@"; do
    case "$a" in
        --install-desktop) install=1 ;;
        *) args="$args $a" ;;
    esac
done
# shellcheck disable=SC2086
cargo build --release $args
exe="$PWD/target/release/rac-rs-launcher"
printf 'built: %s\n' "$exe"
if [ "$install" = 1 ]; then
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$apps"
    entry="$apps/rac-rs-launcher.desktop"
    cat >"$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=RAC-RS Launcher
Comment=Install, play and configure the Ratchet & Clank PC port
Exec=$exe
Icon=$PWD/assets/rac-rs-256.png
Terminal=false
Categories=Game;
DESKTOP
    chmod 755 "$entry"
    printf 'desktop entry: %s\n' "$entry"
fi
