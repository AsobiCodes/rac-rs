# RAC-RS Launcher

A small desktop app (Rust, egui) that installs, updates, configures and starts the PC port of Ratchet & Clank from
[AsobiCodes/rac-rs](https://github.com/AsobiCodes/rac-rs). Bring your own disc: the game is built on your PC from your own disc image; nothing of the game is shipped here.

The game itself (download, changelog, bug reports): https://github.com/AsobiCodes/rac-rs
Project site: https://asobicodes.dev

## Download
Get `rac-rs-launcher-windows-x64.exe` from the [Releases](../../releases) page (the release shows its SHA-256) and double-click it.
The launcher checks for a new launcher and a new game version and asks before it installs either. It updates itself only from this repo.

## Install a game
1. Library, "New install": pick a folder and a game version (default: the newest; older ones from 0.2.1 are installed side by side and stay on that version).
2. The launcher downloads the release from GitHub (checksum checked), then asks for your disc image (.iso, PAL or USA v1.00) and builds the game (about 15 minutes, once).
3. Play.

## Updating and backups
When a new game version is out, the Play page offers it. "Back up your current version first" is ticked by default: the old version is copied next to your install (`<install> (backup vX)`) and listed in the Library as a "Backup". It plays at once, keeps its own saves and settings and is never updated. Big texture packs are shared with the backup by hard links, not copied again (same drive only); if the drive cannot do that the pack is left out. Not enough free space greys the tick out and the update runs without a backup. Delete a backup in the Library ("Delete backup", asks twice, Recycle Bin). If an update fails, the error screen offers "Play the backup".

## Profiles, mods, texture packs
Each install has profiles (the game's options and the active mods). Mods and texture packs are "bring your own pack": add a folder or .zip, or import from your PCSX2 texture folder.
Script mods (for example Arsenal Mastery) are folders with an `omni-mod.json` in them. The format is the one of the game's mod loader OmniLoader, which follows the MIT-licensed mod SDK of [OpenRAC](https://github.com/OpenRAC/OpenRAC); a folder with OpenRAC's `openrac-mod.json` instead is listed too (credit in THIRD_PARTY.md). Put the folder into the install's `mods` folder ("Open mods folder" on the Mods page) and it appears under "Script mods": a switch turns it on or off, and "Mod settings" opens its settings (the mod's own choices, switches, sliders, text fields and key bindings, preset buttons, "Reset to defaults"). The "Load mods" master switch holds all script mods back without losing the per-mod choices. Where the files go: on/off and order are in the profile's `active.txt` (the launcher copies it to `mods\active.txt` when you press Play; a script mod is listed there by the `id` in its omni-mod.json, which may differ from the folder name; a script mod with no line is off); the settings are in `mods\settings.json` (keyed by that id, shared by all profiles of the install). Changes apply the next time the game starts. Needs game version 0.4.0 or newer (older versions grey out the "Load mods" switch with the reason).
Saves: "Open saves folder" opens the install's saves folder (copy `card.bin` between installs, game closed).

## Reporting a problem
"Open log folder" (Game page, sidebar, or Settings > Advanced) opens the folder with `log.txt` selected: attach it to an issue on [AsobiCodes/rac-rs](https://github.com/AsobiCodes/rac-rs/issues). If you already restarted the game, attach `log.prev.txt` too.

## Build from source
`cd launcher; cargo build --release` (Rust stable). `build_release.ps1` makes `dist\rac-rs-launcher-windows-x64.exe` and its SHA-256. `check_linux.ps1` checks the Linux build.

## License
GPL-3.0-or-later, see LICENSE. The libraries built into the exe and their licences are listed in THIRD_PARTY.md.
