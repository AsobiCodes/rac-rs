# Linux parity with the Windows build

This page lists where this fork differs from the upstream v0.4.0 Windows
build, and why. The rule is: if a feature is not platform-locked it runs here
too, and if it is, the game behaves as if the feature were not there rather
than showing a row that does nothing.

## Not on Linux

| Windows feature | Why | What happens here |
| --- | --- | --- |
| DirectX 12 renderer | wgpu's DX12 backend and its shader compiler (DXC) are Windows-only | Vulkan only; the Renderer row is not shown (`pcctl` hides it, `wgpu` backend list is clamped to Vulkan) |
| ReShade | A Windows DLL injected into the swapchain | Not built; the ReShade startup branch in `main.rs` is `cfg(windows)` |
| RTSS overlay | Windows shared memory between RTSS and the game | Not built; `rtss_*` modules stay `cfg(windows)` |
| RAC-RS Launcher, Steam shortcut | Windows apps | Not part of this fork; every option is in the in-game menus |
| OmniLoader script and native mods | Upstream ships `omniloader.dll` (Windows) and does not publish the loader's source or a Linux `.so` | Texture packs work. The Mods page hides script and native rows while `omni::load::loader_present()` is false. Dropping a compatible `libomniloader.so` next to the exe turns them back on |
| `Setup.cmd`, `setup.ps1`, `release_files.txt`, `pgo/`, `tools/mingw_as.py`, `tools/rust_unpack.py` | Windows installer and Windows toolchain helpers | Archived under `windows/`; `setup.sh` and the Linux build ignore them |
| Windows PGO profiles | Recorded with the Windows toolchain | The Linux setup builds its own profile (`generated/build_profile.toml`) |
| Windows HID pad reader (`retail/pspad.rs`) | Raw HID input is Windows-specific | Replaced by `retail/lxpad.rs` (evdev through `gilrs`), same `XRaw` reports |
| Large pages (`oracle/src/mem.rs`) | Windows API | Standard allocations; `make_large_pages` stays `cfg(windows)` |
| No-console window and its error box | Windows subsystem details | Plain Linux process; errors still reach `player/log.txt` and `player/crash.log` |

## Differences that are visible

- **Graphics rows:** the title page and the pause page share the same section
  definitions (`pcctl::gfx_sections`), so the rows that v0.4.0 added (Preset,
  HUD, Skip intro, Button glyphs) show in both places. The Renderer row is the
  one row removed on Linux.
- **Title page input:** the title's Graphics page is the game's own Video
  Options page and steps a row with Cross / A. The pause page also takes
  Left / Right. On a keyboard, Cross is the confirm key (Enter/Space by
  default) on that page.
- **In-level button prompts** (`prompts.rs`, the GS sheet) take a glyph-style
  or rebind change at the next level load, because the fork skips the GPU
  font repatch (see below). The HUD and menus update immediately.
- **HUD Sharp** uses the GPU capture on Vulkan when the card supports it
  (v0.4.0's capture no longer waits for the GPU). If a start freezes while
  the capture opens, the guard writes `gscap_gpu=off` to
  `player/pc_options.txt` and the processor draws the layer from then on.
  `gscap_gpu=auto` tries the GPU again.
- **Skip intro** works from the row (title or pause), and Start / Escape /
  Space still end the logos on the first frame they are pressed.
- **Save clock:** the emulated console answers the CDVD clock command with the
  PC's local time (`oracle/src/hle.rs` -> `oracle/src/clock.rs`), plus the
  save date and memory-card stamps.
- **Vulkan extensions:** `wgpu` picks the same feature set on Linux; the
  Windows-only `VK_KHR_*` workarounds behind `cfg(windows)` are inert.

## Why the font repatch skips the GPU path

`gscap::patch_font` / `repatch_font` rewrite the GS font sheet in guest
memory. On v0.3.1 that happened inside a GS job and the fork's CPU capture
coped with it. v0.4.0 builds one transfer stream per job for the GPU capture,
so the synthetic write is not seen and the pause menu froze on the first frame
after a controller rebind. The fork patches the CPU copy and skips the GPU
re-capture (`retail/gscap/font.rs`); the next level load rebuilds the sheet,
which is why in-level prompts pick changes up then.

## Keeping parity

When upstream adds something, the Windows-only parts go to `windows/` and the
cross-platform parts stay. Files with `cfg(windows)` branches are listed in
`windows/README.md`; they are kept in the tree so a rebase shows upstream's
change instead of a conflict.
