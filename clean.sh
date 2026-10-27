#!/bin/sh
# clean.sh - removes what the build left behind, and keeps what the game needs.
#
# The setup generates the game code from your disc and compiles it. That
# leaves a couple of gigabytes behind that the game does not need in order to
# run. This removes them:
#
#   oracle/rec, oracle/game        the generated game code
#   oracle/src/vu0_prog*.rs        the generated VU microcode
#   rac-rs/src/gsfull/vu1_prog*.rs
#   rac-rs/src/particles/orb_tables.rs
#   build/                         the compiler's working files
#   generated/                     the build log and the stamps
#
# It keeps:
#
#   build/release/rac-rs           the game
#   generated/play.cfg             the path to your disc
#
# Your disc, your saves and the source code are not touched. Run ./setup.sh
# again and it generates what it needs, if you want to build once more.

set -e

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

if [ ! -f build/release/rac-rs ]; then
	echo "The game has not been built yet, so there is nothing to clean." >&2
	exit 1
fi

before=$(du -sk . 2>/dev/null | cut -f1)

# The cargo target directory. Keep the game, drop the compiler's working files.
echo "  removing the compiler's working files"
find build -mindepth 1 -maxdepth 1 ! -name release -exec rm -rf {} +
find build/release -mindepth 1 -maxdepth 1 ! -name rac-rs -exec rm -rf {} +

# The game code the setup generated from your disc.
echo "  removing the game code generated from your disc"
rm -rf oracle/rec oracle/game
rm -f oracle/src/vu0_prog0.rs oracle/src/vu0_prog1.rs oracle/src/vu0_prog9hi.rs
rm -f oracle/src/skinff_ee.rs oracle/src/skinff_vu.rs
rm -f rac-rs/src/gsfull/vu1_prog*.rs rac-rs/src/gsfull/vu1aot_gen.rs
rm -f rac-rs/src/particles/orb_tables.rs

# The build log and the stamps. play.cfg is what the game reads to find your
# disc, so it stays. The version the window title shows falls back to the
# VERSION file when built_version.txt is gone.
echo "  removing the build log"
if [ -d generated ]; then
	find generated -mindepth 1 -maxdepth 1 ! -name play.cfg -exec rm -rf {} +
fi

after=$(du -sk . 2>/dev/null | cut -f1)
freed=$((before - after))

echo "  kept build/release/rac-rs and generated/play.cfg, so the game still runs"
awk -v k="$freed" 'BEGIN {
	if (k >= 1048576) printf "  freed %.1f GB\n", k / 1048576
	else if (k >= 1024) printf "  freed %.1f MB\n", k / 1024
	else printf "  freed %d KB\n", k
}'
echo
echo "The Docker container and image are still there. Remove them with:"
echo "  sudo docker compose down --rmi all --volumes"
