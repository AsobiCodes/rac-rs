//! The engine unit tests that need the R&C1 disc or the R&C1 pack (engine split stage E2, 2026-10-26; tool
//! tools\e2_tests_move.py). One file per engine file, one `mod` per source test module.

mod fmv_adpcm_stream;
mod fmv_mpeg2;
mod texpack_tests;
mod texpack_dump_tests;
mod texpack_extra;
mod fire_defs_tests;
mod hero_fur_discs;
mod planet_names_discs;

mod omni_data_map;
mod omni_window;
mod omni_events;
mod omni_frame_wait;
mod omni_newgame;
mod omni_draw_art;
mod omni_raycast;
mod omni_pvars;
mod omni_tests;
mod omni_counts;
mod omni_native_tests;
mod omni_mobys_tests;
mod omni_pose;
mod omni_pose_read;
mod omni_return_stop;
mod omni_clash;
mod omni_input;
mod omni_draw_demo;

mod awake_src;
