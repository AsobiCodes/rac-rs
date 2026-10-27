# Environment switches

The engine's development and test switches, set as environment variables (`RAC_NAME=value ./play.sh`). This catalog is kept in
step with the source by `rac-rs/src/env_doc_tests.rs`: every `"RAC_..."` string literal in `rac-rs/src` needs a row here, and every
row here must still exist in the source. Add the row in the same change as the switch.

Most are for development: the game runs without any of them set.

| Switch | What it does |
|---|---|
| `RAC_ADAPTER` | Part of a GPU adapter's name: runs on that adapter. |
| `RAC_AIM_FAR` | Debug or compatibility switch of aiming (see the run log and the source). |
| `RAC_AIM_LOG` | Debug or compatibility switch of aiming (see the run log and the source). |
| `RAC_AIM_MASK` | Debug or compatibility switch of aiming (see the run log and the source). |
| `RAC_AIM_OFF` | Debug or compatibility switch of aiming (see the run log and the source). |
| `RAC_AIM_R` | Debug or compatibility switch of aiming (see the run log and the source). |
| `RAC_ASSET_LOG` | Debug switch (see the run log and the source). |
| `RAC_ATTACK_TICKS` | Debug switch (see the run log and the source). |
| `RAC_AUDIO_NO_PROMOTE` | Debug switch (see the run log and the source). |
| `RAC_AUDIO_STATS` | Debug switch (see the run log and the source). |
| `RAC_BINDLESS` | Debug switch (see the run log and the source). |
| `RAC_BIS` | Debug switch (see the run log and the source). |
| `RAC_BOOT_TITLE` | Uses the boot title path instead of the level start (dev). |
| `RAC_CAM` | Debug switch (see the run log and the source). |
| `RAC_CLOUD_DIV` | Debug or compatibility switch of clouds (see the run log and the source). |
| `RAC_CLOUD_REUSE` | Debug or compatibility switch of clouds (see the run log and the source). |
| `RAC_CULL_CHECK` | Debug switch (see the run log and the source). |
| `RAC_CURSOR_DEV` | Debug switch (see the run log and the source). |
| `RAC_DBG_CLASS` | Debug switch (see the run log and the source). |
| `RAC_DEBUGKEYS_FEED` | Debug switch (see the run log and the source). |
| `RAC_DEBUGKEYS_TRACE` | Debug switch (see the run log and the source). |
| `RAC_DEBUG_KEYS` | Enables the debug keys without the Debug mode row. |
| `RAC_DEVTOOLS_FEED` | Debug switch (see the run log and the source). |
| `RAC_DEV_MIRROR_DELAY_MS` | Debug switch (see the run log and the source). |
| `RAC_DEV_TOOLS` | Enables the development tools (planet list, free camera). |
| `RAC_DRAWSKIP` | Debug switch (see the run log and the source). |
| `RAC_EFX_DRAW` | Debug or compatibility switch of effects (see the run log and the source). |
| `RAC_EFX_DUMP` | Debug or compatibility switch of effects (see the run log and the source). |
| `RAC_EFX_DUMPFILE` | Debug or compatibility switch of effects (see the run log and the source). |
| `RAC_EFX_LOG` | Debug or compatibility switch of effects (see the run log and the source). |
| `RAC_EFX_VU_FULL` | Debug or compatibility switch of effects (see the run log and the source). |
| `RAC_ENC_SPLIT` | Debug switch (see the run log and the source). |
| `RAC_FALL_LOG` | Debug switch (see the run log and the source). |
| `RAC_FAR_DRAW` | Debug switch (see the run log and the source). |
| `RAC_FLEET_WATER_OFF` | Debug switch (see the run log and the source). |
| `RAC_FOG_DIV` | Debug or compatibility switch of fog (see the run log and the source). |
| `RAC_FOG_LOG` | Debug or compatibility switch of fog (see the run log and the source). |
| `RAC_FOLLOW_LOG` | Debug switch (see the run log and the source). |
| `RAC_FOLLOW_ROOT` | Debug switch (see the run log and the source). |
| `RAC_FPS_CAP` | Debug switch (see the run log and the source). |
| `RAC_FPS_METER` | Debug switch (see the run log and the source). |
| `RAC_FRAME_LATENCY` | Overrides the swap chain's frame latency. |
| `RAC_FRAME_PROFILE` | Debug or compatibility switch of frame pacing (see the run log and the source). |
| `RAC_FRONT_DLG` | Debug or compatibility switch of the front end (see the run log and the source). |
| `RAC_FRONT_DUMP` | Dumps the front end's page, items and text ids (debug). |
| `RAC_FRONT_GS` | Debug or compatibility switch of the front end (see the run log and the source). |
| `RAC_GIVE` | Debug switch (see the run log and the source). |
| `RAC_GLYPH_DEBUG` | Logs the font sheet uploads and the button-glyph patches. |
| `RAC_GPU_CULL` | Debug or compatibility switch of GPU culling and passes (see the run log and the source). |
| `RAC_GPU_CULL_CHECK` | Debug or compatibility switch of GPU culling and passes (see the run log and the source). |
| `RAC_GPU_CULL_STATS` | Debug or compatibility switch of GPU culling and passes (see the run log and the source). |
| `RAC_GPU_PASSES` | Debug or compatibility switch of GPU culling and passes (see the run log and the source). |
| `RAC_GSCAP_CHECK` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_GPU` | `1` forces the GPU GS capture; `0` the CPU capture. |
| `RAC_GSCAP_OVHASH` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_OVHASH8` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_PMEMO` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_PROF` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_THREADS` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_TRACE` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSCAP_WAIT` | Debug or compatibility switch of the GS capture (see the run log and the source). |
| `RAC_GSDISP` | Debug switch (see the run log and the source). |
| `RAC_GSFULL` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_BUFDUMP` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_DROP` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_DUMP` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_LOG` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_QDUMP` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_QUEUE` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_SPIN_US` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_STATE` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSFULL_THREADS` | Debug or compatibility switch of the GS-full (GPU) picture (see the run log and the source). |
| `RAC_GSGPU_BUILDERS` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_BUILDLOG` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_CACHEDIR` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_NOCACHE` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_SELFTEST` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_SOFT` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_TIME` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSGPU_WGSL` | Debug or compatibility switch of the GPU GS capture (see the run log and the source). |
| `RAC_GSPIX` | Debug or compatibility switch of the GS pixel path (see the run log and the source). |
| `RAC_GSPIX_COLS` | Debug or compatibility switch of the GS pixel path (see the run log and the source). |
| `RAC_GSSTATE_LOG` | Debug or compatibility switch of the GS state (see the run log and the source). |
| `RAC_GS_ASYNC` | Debug switch (see the run log and the source). |
| `RAC_HEALTH` | Debug switch (see the run log and the source). |
| `RAC_HERO` | Debug or compatibility switch of the hero's GPU skinning (see the run log and the source). |
| `RAC_HERO_CHECK` | Debug or compatibility switch of the hero's GPU skinning (see the run log and the source). |
| `RAC_HERO_GPU` | Debug or compatibility switch of the hero's GPU skinning (see the run log and the source). |
| `RAC_HERO_GPU_CHECK` | Debug or compatibility switch of the hero's GPU skinning (see the run log and the source). |
| `RAC_HERO_GPU_FAULT` | Debug or compatibility switch of the hero's GPU skinning (see the run log and the source). |
| `RAC_HIDE_HUD` | Debug switch (see the run log and the source). |
| `RAC_HUD` | `sharp` or `original`: the HUD style for this run. |
| `RAC_HUD_ALL` | Debug or compatibility switch of the HUD (see the run log and the source). |
| `RAC_KOFI` | Debug switch (see the run log and the source). |
| `RAC_LANG` | `en`, `fr`, `de`, `es` or `it`: the title language for this run. |
| `RAC_LOAD_LOG` | Logs the level load timings. |
| `RAC_LOAD_STOP` | Debug switch (see the run log and the source). |
| `RAC_LOCKON_AB` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_LOCKON_CLASSIC` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_LOCKON_CONE` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_LOCKON_NEAREST` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_LOCKON_RANGE` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_LOCKON_SNAP` | Debug or compatibility switch of lock-on (see the run log and the source). |
| `RAC_MARKER_CHECK` | Debug switch (see the run log and the source). |
| `RAC_MENU_LOG` | Debug or compatibility switch of the menus (see the run log and the source). |
| `RAC_MODCAM_DYN` | Debug or compatibility switch of the modern camera (see the run log and the source). |
| `RAC_MODCAM_FEED` | Debug or compatibility switch of the modern camera (see the run log and the source). |
| `RAC_MODCAM_LOG` | Debug or compatibility switch of the modern camera (see the run log and the source). |
| `RAC_MODCAM_SENS` | Debug or compatibility switch of the modern camera (see the run log and the source). |
| `RAC_MODCAM_STRAFE` | Debug or compatibility switch of the modern camera (see the run log and the source). |
| `RAC_MODERN_CAM` | Debug switch (see the run log and the source). |
| `RAC_MODS_DIR` | The mods folder (texture packs, script mods). |
| `RAC_MOVE_CLASSIC` | Debug or compatibility switch of movement (see the run log and the source). |
| `RAC_MUTE` | Silences the audio (automated runs). |
| `RAC_NO_HELP` | Hides the HelpDesk prompts (screenshots). |
| `RAC_NO_UPDATE_CHECK` | Skips the launcher's update check (Windows). |
| `RAC_OMNI` | `0` turns the mods (OmniLoader) off. |
| `RAC_OMNI_DISC` | `pal` or `ntsc`: the disc string native mods are told. |
| `RAC_OMNI_DLL` | Path of the mod loader library (`libomniloader.so` next to the exe by default). |
| `RAC_OMNI_DRAW_DEMO` | Debug or compatibility switch of the mods (OmniLoader) (see the run log and the source). |
| `RAC_OMNI_LAYER` | Debug or compatibility switch of the mods (OmniLoader) (see the run log and the source). |
| `RAC_OMNI_TRACE` | Debug or compatibility switch of the mods (OmniLoader) (see the run log and the source). |
| `RAC_OV_SYNC` | Debug switch (see the run log and the source). |
| `RAC_P1_ZERO` | Debug switch (see the run log and the source). |
| `RAC_PC_EXT` | `0` turns the PC options extension off (the bit-exact retail path). |
| `RAC_PC_SAMPLE` | Debug or compatibility switch of the PC options extension (see the run log and the source). |
| `RAC_PERF_ALL` | Debug switch (see the run log and the source). |
| `RAC_PERF_LOG` | Writes the performance counters to the run log. |
| `RAC_PERF_VU` | Debug switch (see the run log and the source). |
| `RAC_PLAYER_CARD` | Debug switch (see the run log and the source). |
| `RAC_PLAYER_DIR` | The player data folder (saves, options, log). |
| `RAC_POOL_ROOM` | Debug switch (see the run log and the source). |
| `RAC_POST_VIEW` | Debug switch (see the run log and the source). |
| `RAC_PRESENT` | Debug or compatibility switch of presentation (see the run log and the source). |
| `RAC_PRESENT_LOG` | Debug or compatibility switch of presentation (see the run log and the source). |
| `RAC_PRESENT_THREAD` | Debug or compatibility switch of presentation (see the run log and the source). |
| `RAC_PROMPT_CANARY` | Debug or compatibility switch of the button prompts (see the run log and the source). |
| `RAC_PROMPT_DEV` | Debug or compatibility switch of the button prompts (see the run log and the source). |
| `RAC_PROMPT_LOG` | Debug or compatibility switch of the button prompts (see the run log and the source). |
| `RAC_QUEUE_LOG` | Debug switch (see the run log and the source). |
| `RAC_RESHADE` | `0` disables the ReShade add-on; `dry` runs its passes without ReShade (Windows only). |
| `RAC_RETAIL_FMV` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_GS_LAG` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_HOLD` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_HUD` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_KEYTAP` | Scripted keyboard taps for tests: `KEY@WHEN+N;...`. |
| `RAC_RETAIL_NATIVE_DELAY` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_OVDUMP` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_OVLOG` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_OV_DELAY` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_PAD` | Scripted pad reports for tests: `from-to:buttons:lx,ly[:rx,ry]`. |
| `RAC_RETAIL_PLAY` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_PLAYTRAIL` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_REC` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_REC_TRAIL` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_REPLAY` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_SMOKE` | Retail smoke run: hidden window, silent audio, no real pads. |
| `RAC_RETAIL_SMOKE_RAM` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_SNDLOG` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RETAIL_TRAVEL` | Debug or compatibility switch of retail mode (see the run log and the source). |
| `RAC_RTSS_LAYER` | `1` keeps RivaTuner's Vulkan layer (Windows only). |
| `RAC_RUN_AHEAD` | Debug switch (see the run log and the source). |
| `RAC_RUN_LOG` | Forces the run log on or off. |
| `RAC_SCENE_LOG` | Debug switch (see the run log and the source). |
| `RAC_SCOPE_LOG` | Debug switch (see the run log and the source). |
| `RAC_SHOP_DEBUG` | Debug switch (see the run log and the source). |
| `RAC_SHOT_FIELD` | Debug switch (see the run log and the source). |
| `RAC_SHOT_FRAMES` | Debug switch (see the run log and the source). |
| `RAC_SHOT_SEQ` | Debug switch (see the run log and the source). |
| `RAC_SHOT_STEP` | Debug switch (see the run log and the source). |
| `RAC_SHOT_TICK` | Debug switch (see the run log and the source). |
| `RAC_SIZE_CAPS_WRITE` | `1` lowers `SIZE_CAPS.txt` to the current sizes (dev; it never raises a cap). |
| `RAC_SKY_DUMP` | Debug or compatibility switch of the sky (see the run log and the source). |
| `RAC_SMOOTH_KEY` | Debug or compatibility switch of frame smoothing (see the run log and the source). |
| `RAC_SMOOTH_LOG` | Debug or compatibility switch of frame smoothing (see the run log and the source). |
| `RAC_SMOOTH_SHOT` | Debug or compatibility switch of frame smoothing (see the run log and the source). |
| `RAC_SNAP_CHECK` | Debug switch (see the run log and the source). |
| `RAC_SNAP_PAR` | Debug switch (see the run log and the source). |
| `RAC_SNAP_PIPE` | Debug switch (see the run log and the source). |
| `RAC_SND9_COLD` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_FIELDS` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_KEYS` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_NOFX` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_ONLY` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_OUT` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_REVERB` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_STATE` | Debug or compatibility switch of the 989snd driver (see the run log and the source). |
| `RAC_SND9_TRACE` | Logs the 989snd driver's commands. |
| `RAC_SND_PAD` | Debug switch (see the run log and the source). |
| `RAC_SPLIT` | Debug or compatibility switch of the F10 split view (see the run log and the source). |
| `RAC_SPLIT_LABELS` | Debug or compatibility switch of the F10 split view (see the run log and the source). |
| `RAC_STARS` | Debug or compatibility switch of the starry skies (see the run log and the source). |
| `RAC_STAR_DBG` | Debug or compatibility switch of the starry skies (see the run log and the source). |
| `RAC_STRAFE_LOG` | Debug or compatibility switch of strafe aiming (see the run log and the source). |
| `RAC_SUBMIT_DEFER` | Debug switch (see the run log and the source). |
| `RAC_SUN_NODYN` | Debug or compatibility switch of sunlight and shadows (see the run log and the source). |
| `RAC_SUN_PLAIN` | Debug or compatibility switch of sunlight and shadows (see the run log and the source). |
| `RAC_TEST_CLOCK` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_CRASH` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_EE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_FIELDS` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_GAME_VERSION` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_GSCAP_WAIT` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_GSGPU_CRASH` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_GSGPU_DELAY` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_IDLE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_PACK_B` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_PAD` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_RTSS_ANSWER` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_SIZE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_SLOW_EE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_SLOW_RENDER` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_STATE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_SUB` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_TRAVEL` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_UNPACED` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_UPDATE_BASE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_UPDATE_CHOICE` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_UPDATE_SHOT` | Debug or compatibility switch of automated tests (see the run log and the source). |
| `RAC_TEST_WINDOW` | `hidden` puts the game window off-screen and silences the audio/pads for automated runs. |
| `RAC_TEXDUMP_ISO` | Debug or compatibility switch of texture dumps (see the run log and the source). |
| `RAC_TEXDUMP_LEVEL` | Debug or compatibility switch of texture dumps (see the run log and the source). |
| `RAC_TEXDUMP_OUT` | Debug or compatibility switch of texture dumps (see the run log and the source). |
| `RAC_TEXDUMP_RAW` | Debug or compatibility switch of texture dumps (see the run log and the source). |
| `RAC_TEXDUMP_STATE` | Debug or compatibility switch of texture dumps (see the run log and the source). |
| `RAC_TEXNAMES` | Debug or compatibility switch of texture names (see the run log and the source). |
| `RAC_TEXNAMES_RAW` | Debug or compatibility switch of texture names (see the run log and the source). |
| `RAC_TEXPACK_MB` | Debug or compatibility switch of texture packs (see the run log and the source). |
| `RAC_TEXPACK_THREADS` | Debug or compatibility switch of texture packs (see the run log and the source). |
| `RAC_TEXPACK_WAIT_MS` | Milliseconds to wait for a texture pack's build jobs. |
| `RAC_TEXW_CENSUS` | Debug or compatibility switch of the texture walker (see the run log and the source). |
| `RAC_TEXW_CENSUS_LIST` | Debug or compatibility switch of the texture walker (see the run log and the source). |
| `RAC_TEXW_PACK` | Debug or compatibility switch of the texture walker (see the run log and the source). |
| `RAC_TFRAG_INFO` | Debug or compatibility switch of texture fragments (see the run log and the source). |
| `RAC_TFRAG_SKIP` | Debug or compatibility switch of texture fragments (see the run log and the source). |
| `RAC_TFRAG_SKIPBIT` | Debug or compatibility switch of texture fragments (see the run log and the source). |
| `RAC_TFRAG_STATS` | Debug or compatibility switch of texture fragments (see the run log and the source). |
| `RAC_THREADS` | Debug switch (see the run log and the source). |
| `RAC_TITLE_PILOT` | Scripted front-end player for the headless proofs (`start,item:HEX,cross,down:N,wait:N`). |
| `RAC_TITLE_TRACE` | Logs the boot-to-title steering and the front-end pilot. |
| `RAC_TUNE` | Debug switch (see the run log and the source). |
| `RAC_UNDER_LOG` | Debug or compatibility switch of the underwater pass (see the run log and the source). |
| `RAC_UNDER_TUNE` | Debug or compatibility switch of the underwater pass (see the run log and the source). |
| `RAC_USA_EXTRAS` | Debug switch (see the run log and the source). |
| `RAC_VEG` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_CAM` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_DEBUG` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_KIND` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_LEAFLOG` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_LOG` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_PROBE_COL` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VEG_TREES` | Debug or compatibility switch of vegetation (see the run log and the source). |
| `RAC_VENDOR_OFF` | Debug switch (see the run log and the source). |
| `RAC_VIF_FEED_CHECK` | Debug or compatibility switch of the VIF FIFO (see the run log and the source). |
| `RAC_VIF_LAZY` | Debug or compatibility switch of the VIF FIFO (see the run log and the source). |
| `RAC_VSYNC` | Overrides the vsync mode. |
| `RAC_VU1_PROF` | Debug or compatibility switch of the VU1 profiler (see the run log and the source). |
| `RAC_WATER_CLASS` | Debug or compatibility switch of water (see the run log and the source). |
| `RAC_WATER_DBG` | Debug or compatibility switch of water (see the run log and the source). |
| `RAC_WATER_LOG` | Debug or compatibility switch of water (see the run log and the source). |
| `RAC_WATER_MIRRORS` | Debug or compatibility switch of water (see the run log and the source). |
| `RAC_WATER_TIME` | Debug or compatibility switch of water (see the run log and the source). |
| `RAC_WAVES` | Debug or compatibility switch of waves (see the run log and the source). |
| `RAC_WAVE_CAPT` | Debug or compatibility switch of waves (see the run log and the source). |
| `RAC_WGSL_DIR` | Debug switch (see the run log and the source). |
| `RAC_WIDE` | Debug or compatibility switch of widescreen (see the run log and the source). |
| `RAC_WIDE_LOG` | Debug or compatibility switch of widescreen (see the run log and the source). |
| `RAC_WIDE_OFF` | Debug or compatibility switch of widescreen (see the run log and the source). |
