# The upstream Windows build, archived

This folder holds the parts of [rac-rs](https://github.com/AsobiCodes/rac-rs) (v0.4.0) that only make sense on
Windows. This fork builds and plays on Linux. Nothing here is compiled, installed or run by the Linux build or by
`setup.sh`, and the folder is not a cargo workspace member.

Windows players should use Asobi's project itself.

## The layout

It mirrors the upstream tree, so a file here sits where upstream keeps it:

- `Setup.cmd`, `setup.ps1`, `release_files.txt` - the Windows setup (PowerShell 5.1) and its release file list.
- `pgo/` - the speed profiles (`<disc serial>.profdata.gz`) the Windows setup builds with. They were recorded with
  the Windows toolchain, so they do not apply to a Linux build; a Linux profile would have to be recorded again.
- `tools/mingw_as.py`, `tools/rust_unpack.py` - Windows setup helpers (the bundled GNU assembler wrapper and the
  embedded Rust toolchain unpacker). `tools/` still holds the cross-platform generators the Linux setup runs.
- `bundled/omniloader.dll` - OmniLoader, the mod loader, as the Windows DLL. The Linux side of the game looks for
  `libomniloader.so` instead; upstream has not published one, so script mods wait for it. Texture packs do not need
  the loader and work on Linux. See `rac-rs/src/omni/load.rs`.
- `src/rac-rs/exe/rac-rs.manifest` - the Windows segment heap manifest the upstream exe embeds.
- `src/rac-rs/src/retail/pspad.rs` - the DualShock 4 / DualSense reader over Windows HID. On Linux the same job is
  done by `rac-rs/src/retail/lxpad.rs` (evdev through `gilrs`), which produces the same `XRaw` report.

## Still in the tree

Some upstream code is shared but keeps a `cfg(windows)` branch: `main.rs` (no-console build, ReShade and RTSS
startup), `audio.rs`, `runlog.rs`, `gscap_guard.rs`, `viewer/handler.rs`, `retail/prompts.rs` and `oracle/src/mem.rs`
(Windows large pages). Those branches do not compile on Linux and are kept for reference while the Linux parity work
goes through them.

## Parity

Everything here that has a Linux counterpart gets one where it is useful (`hot.rs` thread priority, `mem.rs` large
pages, the capture guard's message box). The rest is platform-locked and is documented in `docs/linux-parity.md`.
