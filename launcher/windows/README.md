# The launcher's Windows files, archived

This folder holds the parts of the upstream RAC-RS Launcher (v0.1.1,
`AsobiCodes/rac-rs-launcher`) that only make sense on Windows. This fork's
launcher builds and runs on Linux. Nothing here is compiled, installed or run,
and the Linux launcher does not read it.

Kept for reference, and to make the next upstream sync a reviewable copy rather
than a rewrite. Provenance and the sync steps are in `../UPSTREAM.md`.

## The layout

- `build.rs` - embeds the icon, version info and manifest into the Windows exe.
- `assets/rac-rs.ico`, `assets/launcher.rc`, `assets/launcher.manifest` - the
  icon, resource script and manifest those embed, plus the DPI/long-path settings.
- `cargo-config.toml` - was `.cargo/config.toml`: static VC++ runtime for
  `x86_64-pc-windows-msvc`.
- `build_release.ps1` - builds the Windows release exe and its SHA-256.
- `check_linux.ps1` - a Windows-hosted `cargo check` for a Linux target
  (`zig cc` for the `ring` build script). On a real Linux PC the normal
  `cargo build` in `launcher/` replaces it.
- `upstream-README.md`, `upstream-CHANGELOG.md` - the upstream documentation
  this tree started from; the fork's own `../README.md` supersedes the README.

## Still in `launcher/src` at the import commit

The PowerShell setup worker (`engine.rs`, `setup_parse.rs`), the Windows
platform calls and the Windows release naming stay in `src/` until the Linux
port replaces their callers. The port removes or moves them here, so the
working crate ends up Linux-only.

## Licence

GPL-3.0-or-later, same as the rest of the launcher and the game. The upstream
copyright and third-party notices are in `../LICENSE` and
`../THIRD_PARTY.md`.
