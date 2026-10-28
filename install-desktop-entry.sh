#!/bin/sh
# install-desktop-entry.sh - adds "Ratchet & Clank (rac-rs)" to your
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

# The paths go in as arguments to printf, never as text the shell reads.
# Exec is quoted as the Desktop Entry specification wants: inside the double
# quotes a ", a `, a $ and a backslash get a backslash in front (and the file
# format reads a backslash once more, so each of those is doubled), and a %
# is written %%. Path= and Icon= are plain strings: only the backslash is doubled (the file format reads one as an escape).
exec_path=$(printf '%s' "$root/build/release/rac-rs" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\&/g' -e 's/%/%%/g')
path_val=$(printf '%s' "$root" | sed -e 's/\\/\\&/g')
icon_val=$(printf '%s' "$icon" | sed -e 's/\\/\\&/g')
{
	printf '[Desktop Entry]\n'
	printf 'Type=Application\n'
	printf 'Name=Ratchet & Clank (rac-rs)\n'
	printf 'GenericName=Ratchet & Clank\n'
	printf 'Comment=Play Ratchet & Clank on Linux, from your own disc\n'
	printf 'Exec="%s"\n' "$exec_path"
	printf 'Path=%s\n' "$path_val"
	printf 'Terminal=false\n'
	printf 'Categories=Game;\n'
	printf 'Keywords=Ratchet;Clank;\n'
	printf 'Icon=%s\n' "$icon_val"
} >"$target"

if command -v desktop-file-validate >/dev/null 2>&1; then
	desktop-file-validate "$target" || true
fi

echo "Added a menu entry: $target"
echo "To remove it again: rm '$target'"
