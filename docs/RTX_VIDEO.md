# Optional RTX Video movie enhancement

By **Agers** ([Agers01](https://github.com/Agers01)) Â· https://komunitni-preklady.org/ Â· agers@komunitni-preklady.org

This optional Windows mod applies NVIDIA RTX Video Super Resolution to the original pre-rendered PSS movies while they play. It is independent of translations and texture packs. Scenes rendered by the game engine are unchanged. It is an unofficial community contribution, not affiliated with NVIDIA, Sony Interactive Entertainment or Insomniac Games.

## Players

The companion host integration and a complete `rac-rtx-video` mod ZIP are both required. Stock 0.5.0 cannot load this movie filter by dropping in its script alone. The complete player package contains the helper executable, NVIDIA release runtime and its license: players do **not** need to obtain the SDK.

Extract the complete mod folder into `mods`, enable **RTX Video â€” by Agers** in Options â†’ Mods and restart. The ordinary **Load mods** switch also disables it. A folder in `mods` is off until enabled. On unsupported systems, or if the helper is absent, fails or times out, playback continues with the original decoded video.

The Mods page exposes:

| Setting | Range | Meaning |
|---|---|---|
| Upscale factor | 1â€“4 | Output width/height multiplier; a 512Ă—416 source at 4Ă— becomes 2048Ă—1664. |
| RTX quality | 1â€“4 | NVIDIA processing quality/cost; 1 is faster, 4 is highest quality. |

Changes to these values apply at the next movie. `mods/settings.json` takes precedence over the fallback `rtx-video.ini`. Try Performance (2Ă—, quality 1), Balanced (3Ă—, quality 2), or Quality (4Ă—, quality 4). This is not frame interpolation, HDR, a sharpening-strength slider or a guarantee of recovered detail. Original audio, PTS, frame rate, aspect ratio and language-track selection are retained.

Successful activation appears in `player/log.txt` as `rtx-video: active ...`. NVIDIA Control Panel's browser-video status does not report this independent SDK integration.

## Architecture and limitations

The decoder worker submits planar YUV420 frames through a bounded pipe to an independent D3D11 helper process. The helper uses NVIDIA RTX Video SDK 1.1, converts to/from RGB and returns enlarged planes. Only the frame's dimensions and planes change; its presentation metadata is preserved. The game itself is not linked against SDK libraries and does not load the NVIDIA VSR DLL. Its renderer can use DX12 or Vulkan; the helper uses its own D3D11 device.

Processing is sequential and each decoded frame is submitted once. The audio clock waits for 12 enhanced frames before starting (3 for original/fallback video). Model startup may take several seconds. Steady-state processing must keep up with the movie; slow hardware can still stutter. CPU conversion, GPU readback and pipe copies add overhead. A helper is initialized for each movie. The initial implementation is Windows-only and does not adapt quality automatically.

Cancellation is checked while waiting for results. Startup has a 30-second timeout, subsequent frames 3 seconds. Malformed dimensions, plane lengths, replies and oversized helper error messages are rejected. A failed filter is disabled for the remainder of that movie.

## Build and package

The standalone MIT helper source and scripts are in [`tools/rtx-video`](../tools/rtx-video). Host integration follows the project's GPL-3.0 license. NVIDIA components retain NVIDIA's separate SDK/runtime license; the SDK itself is not included in this repository. The helper/runtime binaries belong in release assets, not in Git history.

On Windows with Visual Studio C++ Build Tools and a separately obtained official RTX Video SDK 1.1:

```powershell
.\tools\rtx-video\package.ps1 -SdkDir 'C:\path\to\RTXVideoSDK'
```

The output ZIP includes only the mod, compiled helper, required release `nvngx_vsr.dll` and license notices. Build the game normally to include the companion host integration. The packaging script does not patch a player's source, modify an ISO, activate the mod or redistribute game data.

## Diagnostics and validation

| Variable | Meaning |
|---|---|
| `RAC_RTX_VIDEO` | Unset: follow the mod switch. `0`: force off. `1`: force on for diagnostics. Windows only; `RAC_OMNI=0` still disables processing. |
| `RAC_RTX_TRACE` | When set, log dimensions and pipe progress for the first four frames. |

Local runtime test: Windows, RTX 4090, PAL SCES-50916, DX12 game renderer, 4Ă— / quality 4. A complete 363-frame movie was processed at 512Ă—416 â†’ 2048Ă—1664 without fallback; mean host round trip 13.837 ms, maximum 20.110 ms excluding model startup. This measured the preceding local integration; the upstream proposal adds master-switch/input checks and bounded helper reaping. New tests cover those boundaries. Other GPUs, Linux, NTSC, every movie and extended play sessions have not been validated.

Current upstream-proposal checks: `cargo test --offline --locked -p rac-engine --lib fmv::` passed all 15 tests. The standalone helper was rebuilt with MSVC and SDK 1.1, packaged with its runtime/license, then processed three real 512×416 frames to 2048×1664 (quality 4, exit 0). No full-game runtime test of this exact upstream branch has been performed.
