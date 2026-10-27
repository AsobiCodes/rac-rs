#!/bin/sh
# play.sh - starts Ratchet & Clank (rac-rs) from your disc image.
# The game reads generated/play.cfg itself (the disc path, the save folder, the
# language), so there are no arguments to pass. Run setup.sh first.
#
# Plain shell on purpose: the game needs nothing from your system, so there is
# nothing to install to play.
d=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 1
cd "$d" || exit 1
exec "$d/build/release/rac-rs" "$@"
