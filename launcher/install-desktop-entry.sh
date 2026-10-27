#!/bin/sh
# install-desktop-entry.sh - adds "RAC-RS Launcher" to your application menu.
#
# It writes one small file into your home folder and touches nothing else.
# Build the launcher first: ../launcher.sh (or ./build.sh).

set -e

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exe="$root/launcher/target/release/rac-rs-launcher"
icon="$root/launcher/assets/rac-rs-256.png"
dest="$HOME/.local/share/applications"
target="$dest/rac-rs-launcher.desktop"

if [ ! -x "$exe" ]; then
	echo "The launcher is not built yet. Run ./launcher.sh (or ./launcher/build.sh) first." >&2
	exit 1
fi

mkdir -p "$dest"

# The paths go in as arguments to printf, never as text the shell reads, so a
# folder name with a space, an & or a $ in it cannot break this.
{
	printf '[Desktop Entry]\n'
	printf 'Type=Application\n'
	printf 'Version=1.0\n'
	printf 'Name=RAC-RS Launcher\n'
	printf 'GenericName=Ratchet & Clank\n'
	printf 'Comment=Install, play and configure Ratchet & Clank on Linux\n'
	printf 'Exec="%s"\n' "$exe"
	printf 'Path=%s\n' "$root"
	printf 'Terminal=false\n'
	printf 'Categories=Game;\n'
	printf 'Keywords=Ratchet;Clank;Launcher;\n'
	printf 'Icon=%s\n' "$icon"
} >"$target"

chmod 755 "$target"

if command -v desktop-file-validate >/dev/null 2>&1; then
	desktop-file-validate "$target" || true
fi

echo "Added a menu entry: $target"
echo "To remove it again: rm '$target'"
