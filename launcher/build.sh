#!/bin/sh
# Builds the Linux launcher. Run it from anywhere; the game itself is built by ../setup.sh.
#
#   ./build.sh              release build in launcher/target/release/rac-rs-launcher
#   ./build.sh --offline    any other argument goes to cargo
#
# To start it: ../launcher.sh. For an application-menu entry:
# ./install-desktop-entry.sh.
set -eu
cd "$(dirname "$0")"
cargo build --release "$@"
printf 'built: %s\n' "$PWD/target/release/rac-rs-launcher"
