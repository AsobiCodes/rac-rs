# RTX Video — by Agers

Optional NVIDIA RTX Video Super Resolution for pre-rendered movies in RAC-RS. Independent of translations and fonts.

Author: **Agers** · https://komunitni-preklady.org/ · agers@komunitni-preklady.org

Requires Windows, a compatible NVIDIA RTX GPU/driver, the companion movie-filter host integration, and the complete player package containing `rac-rtx-video-worker.exe`, `nvngx_vsr.dll` and NVIDIA's license. The source-only folder is not a complete player package. Players using a complete package do not need the SDK.

Extract the mod folder into `mods`, enable it in Options → Mods and restart. Adjust Upscale factor and RTX quality (1–4) there; changes take effect at the next movie. Performance is 2×/quality 1, Balanced 3×/quality 2, Quality 4×/quality 4. Missing/unsupported helpers fall back to original video. Audio, timing and aspect ratio remain unchanged. First model startup adds loading time; weak GPUs may not keep up.

Own helper code: MIT (`LICENSE`). NVIDIA runtime: separate `NVIDIA_RTX_Video_SDK_License.pdf` included in a complete release ZIP. Companion host: GPL-3.0-only. Unofficial, not affiliated with NVIDIA, Sony Interactive Entertainment or Insomniac Games.

Full build, architecture and validation notes: `docs/RTX_VIDEO.md` in the source repository. Build the standalone package with `tools/rtx-video/package.ps1 -SdkDir ...`.
