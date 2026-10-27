#!/bin/sh
# install-desktop-launcher.sh - adds "Ratchet & Clank (rac-rs)" to your
# application menu.
#
# It writes one small file into your home folder and touches nothing else.
# Run it once the game has been built, however you built it.

set -e

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
icon="$root/rac-rs/assets/icon/rac-rs-256.png"
dest="$HOME/.local/share/applications"
target="$dest/rac-rs.desktop"

if [ ! -f "$root/build/release/rac-rs" ]; then
	echo "The game is not built yet. Run ./setup.sh, or 'sudo docker compose up', first." >&2
	exit 1
fi

mkdir -p "$dest"

# The two paths go in as arguments to printf, never as text the shell reads,
# so a folder name with a space, an & or a $ in it cannot break this.
{
	printf '[Desktop Entry]\n'
	printf 'Type=Application\n'
	printf 'Name=Ratchet & Clank (rac-rs)\n'
	printf 'GenericName=Ratchet & Clank\n'
	printf 'Comment=Play Ratchet & Clank on Linux, from your own disc\n'
	printf 'Exec="%s/build/release/rac-rs"\n' "$root"
	printf 'Path=%s\n' "$root"
	printf 'Terminal=false\n'
	printf 'Categories=Game;\n'
	printf 'Keywords=Ratchet;Clank;\n'
	printf 'Icon=%s\n' "$icon"
} >"$target"

if command -v desktop-file-validate >/dev/null 2>&1; then
	desktop-file-validate "$target" || true
fi

echo "Added a menu entry: $target"
echo "To remove it again: rm '$target'"
