#!/bin/sh
# launcher.sh - starts the RAC-RS Launcher.
# The launcher builds itself first if it has not been built yet. The game is
# started by play.sh, or by the launcher's Play button.
d=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 1
exe="$d/launcher/target/release/rac-rs-launcher"
if [ ! -x "$exe" ]; then
	echo "The launcher is not built yet; building it once..."
	"$d/launcher/build.sh" || exit 1
fi
exec "$exe" "$@"
