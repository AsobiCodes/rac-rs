# RAC-RS Launcher for Linux

An optional desktop app (Rust, egui) that keeps a list of Ratchet & Clank
installs, starts the game, edits its options, switches profiles and installs
texture packs. It is the launcher for this Linux fork; the game works the same
without it.

It started as [AsobiCodes' RAC-RS Launcher](https://github.com/AsobiCodes/rac-rs-launcher),
the Windows launcher for the upstream port, and lives in this repository so
there is one version scheme, one release tag and one place to work. The exact
upstream commit and the re-sync steps are in [`UPSTREAM.md`](UPSTREAM.md); the
Windows-only files are archived in [`windows/`](windows/) and are not built.

## Status

Working now:

- **Library:** "Add existing install..." for a game you set up with `setup.sh`,
  several installs side by side, open, remove from the list (files untouched).
- **Play:** starts `build/release/rac-rs` from the install root, with the save
  card safety copies; notices a game started another way; copies the options
  back into the profile when it exits.
- **Settings:** the game's own options (`player/pc_options.txt`), profiles,
  the graphics presets, Skip intro, Mouse speed, Camera speed and the fork's
  Button glyphs row. Vulkan is the only renderer here, so the renderer row is
  not shown.
- **Mods:** texture packs from a `.zip` or folder, drag and drop, or imported
  from PCSX2's own texture folders; per-pack switches, priority order and the
  "Load texture packs" master tick. Same `mods/` layout and `active.txt` the
  game reads.
- **Folders:** open the saves and the run log; the log opens with `log.txt`
  selected.
- **Desktop entry:** `./install-desktop-entry.sh` adds "RAC-RS Launcher" to the
  application menu.

Not yet (next phases):

- Installing the game from a disc (the `setup.sh` engine), Repair / rebuild.
- Updates, the backup-before-update step and the launcher's self-update.
- Script and native mods: they need an OmniLoader library built for Linux, and
  upstream only publishes the Windows DLL. The Mods page hides them until one
  exists; texture packs never need it.

## Requirements and build

The game must be built first (`../setup.sh`). To build the launcher you need the
Rust toolchain (the same one the game uses), a C compiler and the usual X11 or
Wayland development headers for egui; file dialogs use the XDG desktop portal.

```sh
../launcher.sh        # builds it if needed, then starts it
./build.sh            # build only (launcher/target/release/rac-rs-launcher)
./install-desktop-entry.sh   # app-menu entry (build first)
```

`../launcher.sh` is the simple way in: it runs `build.sh` once and passes any
arguments to the launcher. `./build.sh` takes cargo flags such as `--offline`.
The launcher reports the fork's version from `../VERSION`.

## Where it keeps things

- `$XDG_DATA_HOME/RAC-RS Launcher/` (usually `~/.local/share/RAC-RS Launcher/`):
  `library.json` (the installs) and `launcher.cfg` (window, last page, last
  disc folder, backup tick).
- Next to the launcher binary instead, when it sits inside a game folder
  (`VERSION` + `setup.sh`) or when a `portable.txt` is next to it.
- Each install keeps its own saves (`player/`), options, profiles and mods; the
  launcher only stores paths. Removing an install from the list never deletes
  files.
- `--root DIR` opens one install directly (handy for a desktop entry);
  `RAC_GAME_RELEASES` / `RAC_LAUNCHER_RELEASES` point the release checks at a
  test folder.

## How it reads a game folder

A Linux install of this fork is the tree `setup.sh` builds, so the launcher
looks for:

- `VERSION` and `setup.sh` (a release tree),
- `build/release/rac-rs` and `play.sh` plus `generated/play.cfg` (a built game),
- `iso=` in `generated/play.cfg` for the disc image,
- `player/` for saves, `log.txt` and `pc_options.txt`,
- `mods/` for texture packs and `active.txt`.

Texture packs are copied into `mods/<id>/textures/` with a `mod.json`, exactly
what the game's own Mods page reads. PCSX2 packs are found under
`~/.config/PCSX2/textures` (Flatpak: `~/.var/app/net.pcsx2.PCSX2/config/PCSX2/textures`).

## Code map

| File | What it does |
| --- | --- |
| `src/main.rs` | window, icon, single-instance guard |
| `src/platform.rs` | Linux calls: flock, `statvfs`, XDG folders, `.desktop`, xdg-open, trash |
| `src/games.rs` | the game table, install detection, `play.cfg` / version reading |
| `src/library.rs` | `library.json`, add/remove installs, folder compare |
| `src/play.rs` | start the game, save-card copies, saves/log folders |
| `src/profiles.rs` | profiles: options, mods list, copy in/out on play |
| `src/pcopts.rs` | `player/pc_options.txt` parser/writer (mirrors the game) |
| `src/mods/` | mod store, `active.txt`, zip/folder/PCSX2 import, settings, script manifests |
| `src/gating.rs` | which game version knows which option |
| `src/engine.rs` | the process reader the Linux `setup.sh` engine will use; `start` says "later version" for now |
| `src/installer.rs` | Linux install backend (disabled until the engine lands) |
| `src/backup.rs` | playable copy for updates (compiled, not offered yet) |
| `src/releases.rs`, `src/game_dl.rs`, `src/update.rs`, `src/selfupdate.rs` | releases and update plumbing (the Windows releases are not used; the fork's own channel comes with Phase 2) |
| `src/ui/` | egui pages: library, game (play/settings/mods), profiles, network/update screens |

## Updating from upstream

See [`UPSTREAM.md`](UPSTREAM.md). In short: diff against the recorded commit,
copy the changed files, then re-apply the Linux changes (this crate is
Linux-only; the Windows files live in `windows/`). Keep `src/setup_parse.rs`:
the Linux `setup.sh --progress=launcher` mode is meant to print the same markers
so the existing progress screen works unchanged.

## Planned

1. `setup.sh --progress=launcher` and the Linux install engine: New install
   and Repair from a disc.
2. Update + backup (binary, `player/`, `mods/` with hard-linked packs) and
   launcher self-update from the same release tag.
3. Packaging (tarball or AppImage) built against Debian 12 so it runs on older
   systems too.

## Licence and credits

GPL-3.0-or-later, see [`LICENSE`](LICENSE); the game and this fork use the same
licence. Third-party crates and their licences are in
[`THIRD_PARTY.md`](THIRD_PARTY.md). The original launcher is by
[AsobiCodes](https://github.com/AsobiCodes); the Linux port is part of this
fork. *Ratchet & Clank* is a trademark of Sony Interactive Entertainment; this
project is not affiliated with or endorsed by Sony or Insomniac Games.
