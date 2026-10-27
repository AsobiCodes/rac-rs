# Where this launcher came from

This folder is a copy of the **RAC-RS Launcher** by
[AsobiCodes](https://github.com/AsobiCodes/rac-rs-launcher), which installs,
configures and starts the upstream Windows port. Here it is being turned into a
launcher for this Linux fork.

- Source: https://github.com/AsobiCodes/rac-rs-launcher
- Commit: `67747ca44189e20ede422c3b118c044519533798` (2026-10-26, one docs
  commit after tag `v0.1.1`)
- Upstream version: 0.1.1
- Licence: GPL-3.0-or-later (see `LICENSE`); third-party crates and their
  licences are in `THIRD_PARTY.md`

## What is here

The upstream `launcher/` crate (its `Cargo.toml`, `Cargo.lock`, `src/`,
`tests/`, and `assets/rac-rs-256.png`), unchanged except for the Linux port
that follows this import.

## What changed at import

- The standalone Windows-only files moved to `windows/` and are not built or
  run: `build.rs`, the resource script and icon, the MSVC cargo config,
  `build_release.ps1`, `check_linux.ps1`, and the upstream README and
  changelog.
- The Linux port (the commit after the import) removed the Windows code from
  the working crate. The PowerShell spawner is in `windows/engine_windows.rs`,
  the Windows platform calls in `windows/platform_windows.rs`, the Windows
  start-up in `windows/play_windows.rs`; the Windows-only tests are in
  `windows/tests/`. `src/setup_parse.rs` stays because the Linux
  `setup.sh --progress=launcher` mode is meant to print the same markers.
- `src/engine.rs` now holds the cross-platform process reader and a
  placeholder `start` that says disc installs come in a later version;
  `installer.rs` is the Linux backend.
- The `windows-sys`, `mslnk` and `embed-resource` dependencies were removed.
- This crate is its own Cargo workspace, and the game's root `Cargo.toml`
  excludes `launcher`, so the two builds cannot see each other's dependencies.

## What the Linux port removed

The update, self-update and release-download modules (`releases.rs`,
`game_dl.rs`, `update.rs`, `selfupdate.rs`, `ui/page_net.rs`,
`ui/state_net.rs`) were removed from the Linux crate: this is a fork, so a new
version is a new download or `git pull`, and the launcher only builds and
repairs the local source. Their upstream versions are in the git history and in
`windows/` where they describe Windows flows. The launcher's own version is the
fork's `VERSION`, used for display only.

## Re-syncing with upstream

1. Fetch the new upstream commit: `git clone https://github.com/AsobiCodes/rac-rs-launcher`.
2. Diff `launcher/` against the commit recorded here, not against this tree,
   so the Linux port does not hide upstream changes.
3. Copy the changed files into `launcher/src` and re-apply the Linux changes
   listed in `README.md` (platform, install contract, gating).
4. Move any new Windows-only files to `launcher/windows/`.
5. Update the commit hash and version above.

The Linux port is deliberately kept in a small set of files and `cfg(not(windows))`
branches so this stays a reviewable diff, not a rewrite.
