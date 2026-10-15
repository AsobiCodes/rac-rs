//! Hand-written Rust replacements of retail functions: the readable, moddable rewrite of the game, one function at
//! a time. Each entry names the retail code it replaces (address range and hash, so it applies only where RAM holds
//! that code); `hand/fninfo.py` prints a function's disassembly and its entry, and the ee-game example `fndiff`
//! checks a rewrite call by call against the interpreter (memory, results and saved registers bit-exact).
mod actor;
mod actor_near_walk;
mod actor_sway;
mod actor_tick;
mod aim_steer;
mod ammo_flags;
mod angle_mix;
mod approach_glue;
mod atan_swap;
mod attached_flags;
mod aux_slot_step;
mod axis_probe;
mod ballistic_kit;
mod beam_fan_reset;
mod blast_impulse;
mod blend_scene;
mod blocks;
mod bolt_prompts;
mod box_quad_pkt;
mod bubble_bounce;
mod buildtag;
mod callback_tables;
mod calls;
mod cam17;
mod cam_latch;
mod cam_latch_step;
mod cam_look;
mod cam_params;
mod cam_pick;
mod cam_polar_hero;
mod cam_rig_setters;
mod cam_slot_copy;
mod cam_slot_fill;
mod cam_table_switch;
mod cam_vec_pair;
mod camera;
mod camera_claim;
mod camera_frame;
mod camera_pre;
mod camera_tick;
mod camhook;
mod camobj;
mod camrig_spring;
mod camswitch;
mod cancel_save;
mod cell_row_emit;
mod cells;
mod chunk_emit;
mod chunk_latch;
mod chunk_pass;
mod claim_table;
mod clear_rgbaq_setup;
mod collision;
mod colour_flag;
mod comet_guard;
mod cone_probe;
mod counter_hud;
mod counter_setup;
mod counters_portrait;
mod ctx_slots;
mod cue_tables;
mod cursor_tags;
pub mod cut;
mod cutf_2713b0;
mod cutf_2a40b8;
mod debris_step;
mod dma_ring;
mod dma_setup;
mod dma_tag_link;
mod draw_queue;
mod draw_run_text;
mod ember_spawn;
mod emit_record;
mod entry_patch;
mod euler_basis;
mod fade_sprite;
mod feet_ik;
mod flag_dispatch;
mod flash5_update;
mod fluff_ctor;
mod fmt_template;
mod follow_ground_point;
mod font2_text;
mod font_line;
mod font_tex;
mod foot_probe;
mod frame;
mod frame_dispatch;
mod frame_mark;
mod frame_tail;
mod frustum_step;
mod fx_frame;
mod fx_push;
mod fx_sway;
mod fxpkt_steps;
mod fxtex;
mod gate_frames;
mod gather;
mod geom;
mod gif_quad_pkt;
mod gif_queue;
mod gif_rect_pkt;
mod gifpkt;
mod glove_aim;
mod glove_gate;
mod glove_slot;
mod glow;
mod glyph_vu;
mod god_capsule;
mod grid_fanout;
mod grid_offset;
mod grid_probe_pair;
mod grind_rail;
mod ground_probe;
mod gs_append_pkt;
mod gs_packet;
mod gs_pkt_misc;
mod gs_queue_gate;
mod gs_tag_loop;
mod h;
mod held_layers;
mod held_ribbon;
mod hero_anim;
mod hero_aux;
mod hero_blend;
mod hero_blend_pose;
mod hero_damage_reset;
mod hero_edge_probe;
mod hero_front_probe;
mod hero_hand;
mod hero_head_blink;
mod hero_items;
mod hero_jump_land;
mod hero_lock;
mod hero_pad_walk;
mod hero_probe_sweep;
mod hero_recover;
mod hero_rig;
mod hero_seek;
mod hero_slide;
mod hero_slot_sweep;
mod hero_snap;
mod hero_space;
mod hero_state;
mod hero_tilt;
mod hero_turn;
mod hero_water;
mod hero_weapon_anim;
mod hero_wish_yaw;
mod hit_point_write;
mod hit_ring;
mod homing_ray_spawn;
mod hud_ammo;
mod hud_bolts;
mod hud_counter;
mod hud_fx_pool;
mod hud_pool;
mod hud_quad_gif;
mod hud_slot_init;
mod hud_slot_ops;
mod hud_slot_rows;
mod hud_slots;
mod hud_snap_counter;
mod hud_sprites;
mod hud_suppress;
mod hud_widget;
mod inherit_pose_rig;
mod init_chain;
mod inset_boxes;
mod joint_block_pose;
mod jump_ground;
mod key_slots;
mod keyframe_moby;
mod l4_boot;
mod latch_step;
mod launch_block;
mod layer_bake;
mod layer_masks;
mod layer_scale;
mod layers;
mod lerp_pair;
mod level_flags;
mod level_motion;
mod limb_pulse;
mod lock_scan;
mod look_spring;
mod marker_draw;
mod marker_spawn;
mod mask_prims;
mod mat4_rle;
mod mathx;
mod menu_moby;
mod menu_scroll;
mod misc_leaf;
mod moby;
mod moby_ad;
mod moby_anim_bsphere;
mod moby_anim_hooks;
mod moby_cull;
mod moby_init;
mod moby_release;
mod moby_slot_reset;
mod moby_slots;
mod motion;
mod mouse_burst;
mod nibblepack;
mod obj_slot_attach;
mod objects;
mod orb_fx;
mod orb_gates;
mod orbit_place;
mod orient_step;
mod packed_vu;
mod pair_scale;
mod pair_update;
mod particle_spawn;
mod particles;
mod path;
mod path_walk;
mod pause_hud;
mod pause_slots;
mod pitch_point;
mod plane_hits;
mod planet_gold;
mod point_cull;
mod pool_alloc;
mod pool_record;
mod pose_anim_hud;
mod pose_chain_glow;
mod pose_setup;
mod pose_spring;
mod probe_cast;
mod proj_init;
mod proj_setup;
pub mod ps2;
mod puff_spawn;
mod quad_block;
mod quad_pair;
mod rand;
mod rcall;
mod rec_emit;
mod rec_hands;
mod rec_reset;
mod record_list_walk;
mod record_pool;
mod record_stream;
mod ref_table;
mod registry_restore;
mod ring_tags;
mod ripple_items;
mod row_records;
mod sample_ring;
mod scene;
mod seq_lookup;
mod shadow;
mod shadow_slot_queue;
mod shark_suck;
mod shop_frame_setup;
mod shop_ticker;
mod sky_xform;
mod slot_alloc;
mod slot_anim;
mod slot_attach;
mod slot_bits;
mod slot_cam;
mod slot_claim;
mod slot_clear;
mod slot_dispatch;
mod slot_fixup;
mod slot_flags;
mod slot_gather;
mod slot_jitter_step;
mod slot_latch;
mod slot_pick;
mod slot_probe;
mod slot_ranges;
mod slot_rec_fill;
mod slot_refresh;
mod slot_registry;
mod slot_setup;
mod slot_spawn;
mod slot_start;
mod slot_table;
mod slot_tables;
mod slot_turn;
mod slots;
mod smoke_loft;
mod smoke_spawn;
mod sndcache;
mod spark_motion;
mod spawn_curve;
mod spawn_rec;
mod spheres;
mod spr_chain;
mod spr_stream;
mod sprite_queue;
mod sprite_text;
mod stage_dispatch;
mod stage_pack;
mod state_dispatch;
mod state_tick;
mod streak_spawn;
mod stream_ctl;
mod suck_cannon;
mod suck_puff;
mod surface_probe;
mod surface_push;
mod sweep;
mod swim_queue;
mod swing_cam;
mod swing_particle;
mod table_fill;
mod talk_pad_scan;
mod targets;
mod tex_blit;
mod tex_strip_pkt;
mod texrec_class;
mod tfrag_lod_morph;
mod tfrag_texture;
mod trail;
mod trail_ctor;
mod travel_moby;
mod trig;
mod vecmath;
mod veldin_cam;
mod view_track;
mod vif_emit;
mod vif_timer;
mod visibomb_fx;
mod voice_level;
mod voice_slot;
mod voice_slots;
mod vu_blend_pack;
mod vu_geom;
mod vu_probes;
mod vu_vec;
mod walker_orbit;
mod walker_turn;
mod wallop_cues;
mod warp_camera_key;
mod warp_quads;
mod water_occ;
mod weapon_slots;
mod wrench_moby;
mod xy_packet;
mod zone_view_matrices;

pub use ee_oracle;
use ee_oracle::rec::Hand;

/// Eudora (level 4) overlay functions.
static EUDORA: &[Hand] = &[
    Hand { entry: 0x1dd878, end: 0x1dd8b0, hash: 0xfceb509a6ca9d085, est: 14, f: camswitch::spill_cam_args },
    Hand { entry: 0x1dd8b0, end: 0x1dd8c8, hash: 0xf6b5d7c1f1ebe84d, est: 6, f: cam_params::cam_params_zero },
    Hand { entry: 0x1dd8c8, end: 0x1dd924, hash: 0x847d0f1689e08aeb, est: 23, f: cam_params::cam_params_copy },
    Hand { entry: 0x1dd928, end: 0x1dd984, hash: 0x8d3d4927319fad63, est: 23, f: camswitch::copy_cam_tables },
    Hand { entry: 0x1dd988, end: 0x1dd9f0, hash: 0x8ae3e1017a5d9236, est: 18, f: hero_blend_pose::frame_list_push },
    Hand { entry: 0x1dd9f0, end: 0x1dda5c, hash: 0xeeee01d4cfbe1b6d, est: 15, f: camobj::run_deferred_callbacks },
    Hand { entry: 0x1dda60, end: 0x1dda94, hash: 0x559cb7eb675ca61a, est: 26, f: camswitch::find_cam_slot },
    Hand { entry: 0x1ddaa8, end: 0x1ddb8c, hash: 0x51cad64575047c08, est: 39, f: camobj::spring_follow },
    Hand { entry: 0x1ddb90, end: 0x1ddcac, hash: 0xf8fee043f690f876, est: 52, f: camobj::angle_spring_follow },
    Hand { entry: 0x1ddcb0, end: 0x1ddcd0, hash: 0x2a77d349dbb36cc5, est: 8, f: camswitch::clear_cur_cam_flags },
    Hand { entry: 0x1ddcd0, end: 0x1ddd30, hash: 0xc1bbb74b412fa4db, est: 13, f: camobj::camera_obj_sync },
    Hand { entry: 0x1ddd30, end: 0x1ddd74, hash: 0xc66ec637fb3efbc9, est: 17, f: camhook::dispatch_cam_hook },
    Hand { entry: 0x1de078, end: 0x1de23c, hash: 0x411a1762dc4a7d10, est: 48, f: camobj::camera_record_accept },
    Hand { entry: 0x1de240, end: 0x1de284, hash: 0xd54d24c287e1ea1, est: 17, f: camobj::camera_leave },
    Hand { entry: 0x1de288, end: 0x1de398, hash: 0xbface1b2adc7f2a3, est: 374, f: camobj::camera_select },
    Hand { entry: 0x1de578, end: 0x1de654, hash: 0x96dfb197b565c15b, est: 55, f: hero_blend_pose::hero_blend_encode },
    Hand { entry: 0x1de6d0, end: 0x1de708, hash: 0xf77d4c01e11ee249, est: 7, f: hero_blend_pose::blend_mode0_from_mode2 },
    Hand { entry: 0x1df118, end: 0x1df1c8, hash: 0xd390612f9fb1bf13, est: 20, f: cam_pick::cam_blend_done },
    Hand { entry: 0x1df1c8, end: 0x1df2d4, hash: 0xa2e89212b9c28070, est: 22, f: camera_frame::camera_shake_step },
    Hand { entry: 0x1df2d8, end: 0x1df654, hash: 0xb27cdde6022a34aa, est: 191, f: camera_pre::camera_pre_update },
    Hand { entry: 0x1df658, end: 0x1df7a0, hash: 0xdc52389c25ca6837, est: 46, f: camera_frame::camera_clip_check },
    Hand { entry: 0x1df7a8, end: 0x1df8c8, hash: 0xda8fb1317b6fef9a, est: 54, f: camera_frame::camera_flags_init },
    Hand { entry: 0x1df8c8, end: 0x1df910, hash: 0x36e6f705d74d6df1, est: 10, f: camera_frame::camera_boost_tick },
    Hand { entry: 0x1dfa98, end: 0x1dfae4, hash: 0x8e22f58ca2072571, est: 36, f: cam_pick::camera_class_index },
    Hand { entry: 0x1dfde8, end: 0x1dfe98, hash: 0xfbf96ddb19207e2f, est: 44, f: cam_polar_hero::cam_polar_eye },
    Hand { entry: 0x1dfe98, end: 0x1dff10, hash: 0xf029c1454f0d1940, est: 30, f: cam_polar_hero::cam_eye_to_polar },
    Hand { entry: 0x1e00e8, end: 0x1e049c, hash: 0x9f19299aa10ab1c6, est: 45, f: sprite_queue::track_sprites },
    Hand { entry: 0x1e07c8, end: 0x1e08a4, hash: 0xf333e65382538e17, est: 46, f: fx_push::push },
    Hand { entry: 0x1e08a8, end: 0x1e0a14, hash: 0x8da7c24d6ed8d6ef, est: 32, f: sprite_queue::draw_sprite_queue },
    Hand { entry: 0x1e0a18, end: 0x1e0b7c, hash: 0x833096ca5c8a0429, est: 52, f: camera_tick::grid_find_entry },
    Hand { entry: 0x1e2438, end: 0x1e24d8, hash: 0x25a164b4bebc5ee5, est: 38, f: collision::grid_cell_reg },
    Hand { entry: 0x1e5e40, end: 0x1e5e70, hash: 0xafb27ad78f12a5e8, est: 10, f: collision::hit_surface_class },
    Hand { entry: 0x1e5e70, end: 0x1e5ea0, hash: 0xa129c4fc278d6db2, est: 10, f: collision::hit_surface_group },
    Hand { entry: 0x1e5f60, end: 0x1e5fd4, hash: 0xc2f6f5417fb09022, est: 16, f: collision::hit_queue_flush },
    Hand { entry: 0x1e9488, end: 0x1e9490, hash: 0xc84dc27f29a28935, est: 2, f: collision::empty_hook },
    Hand { entry: 0x1e9490, end: 0x1e9498, hash: 0xc84dc27f29a28935, est: 2, f: cam_params::empty_hook_tail },
    Hand { entry: 0x1e95d8, end: 0x1e95f4, hash: 0x2c581c12232b5016, est: 7, f: collision::move_pending_flag },
    Hand { entry: 0x1e9600, end: 0x1e9608, hash: 0xc84dc27f29a28935, est: 2, f: collision::empty_hook },
    Hand { entry: 0x1e9608, end: 0x1e9610, hash: 0xc84dc27f29a28935, est: 2, f: collision::empty_hook },
    Hand { entry: 0x1e99d8, end: 0x1e9adc, hash: 0xa3cd85bdb8d2b023, est: 39, f: water_occ::water_view_swap },
    Hand { entry: 0x1e9bb8, end: 0x1e9c70, hash: 0x5899433ca1e3e5c7, est: 37, f: cam_pick::occ_nearer_order },
    Hand { entry: 0x1ea060, end: 0x1ea0b0, hash: 0xe96c1ae6d45cbaa3, est: 13, f: water_occ::occ_frame },
    Hand { entry: 0x1ea0b0, end: 0x1ea1e4, hash: 0xd570923fadebbb24, est: 77, f: proj_setup::proj_setup },
    Hand { entry: 0x1ea808, end: 0x1ea938, hash: 0x1bcd593574f36ba1, est: 76, f: proj_init::offscreen_setup },
    Hand { entry: 0x1eac38, end: 0x1eacb4, hash: 0x8da5ecf38446f9c9, est: 31, f: proj_init::clear_globals },
    Hand { entry: 0x1eacb8, end: 0x1eada4, hash: 0xdfe52803c3197465, est: 59, f: gs_queue_gate::gs_queue_write },
    Hand { entry: 0x1eada8, end: 0x1eae20, hash: 0x5078db820a9b7ca9, est: 30, f: gs_packet::gs_init_regs },
    Hand { entry: 0x1eb8c8, end: 0x1eb9dc, hash: 0xadbb8cecae0720bb, est: 2193, f: fxtex::fx_frame_reset },
    Hand { entry: 0x1eb9e0, end: 0x1ebafc, hash: 0xd40b869ae17cc2c9, est: 71, f: fxtex::fx_pkt_tags },
    Hand { entry: 0x1ebb00, end: 0x1ebc48, hash: 0xe47654060167c0c8, est: 39, f: fxtex::fx_tex_upload },
    Hand { entry: 0x1ebc48, end: 0x1ebc98, hash: 0xb5b5f3137247cc61, est: 20, f: draw_queue::draw_queue_add },
    Hand { entry: 0x1ebe00, end: 0x1ebe50, hash: 0x9bfcfc0e2d299401, est: 20, f: draw_queue::draw_queue_add_alt },
    Hand { entry: 0x1ebec8, end: 0x1ec0a0, hash: 0xead9a7158d23ea27, est: 87, f: gif_queue::shadow_queue_flush },
    Hand { entry: 0x1ec440, end: 0x1ec660, hash: 0x59d2cfb90174a3d, est: 10, f: gif_queue::queue_emit },
    Hand { entry: 0x1ec660, end: 0x1ec7e0, hash: 0x1f97f13bacd50726, est: 3376, f: cell_row_emit::cell_row_emit },
    Hand { entry: 0x1ec8b8, end: 0x1ec948, hash: 0x19e4aded260110a3, est: 36, f: l4_boot::pack_submit },
    Hand { entry: 0x1ecaf8, end: 0x1ecc7c, hash: 0x8f7b0ad25121859a, est: 97, f: sprite_text::sprite_blit_int },
    Hand { entry: 0x1ed5b8, end: 0x1ed6bc, hash: 0x6834a9d3e27f4a0c, est: 64, f: tex_strip_pkt::emit_textured_strip },
    Hand { entry: 0x1ed6c0, end: 0x1ed804, hash: 0xac555afe0a655614, est: 81, f: inset_boxes::draw_inset_boxes },
    Hand { entry: 0x1ed808, end: 0x1ed98c, hash: 0x2cc4b57e12cc2d7, est: 97, f: grid_fanout::grid_fanout },
    Hand { entry: 0x1ed990, end: 0x1ed99c, hash: 0x9989fd575d807350, est: 3, f: l4_boot::set_flag_one },
    Hand { entry: 0x1ed9a0, end: 0x1ed9a8, hash: 0x47d5755858562011, est: 2, f: l4_boot::clear_flag },
    Hand { entry: 0x1ed9a8, end: 0x1ed9f8, hash: 0x2f4f618007104710, est: 217, f: sprite_text::text_width_n },
    Hand { entry: 0x1ed9f8, end: 0x1eda18, hash: 0x8e27dc6cfb6220fb, est: 8, f: sprite_text::text_width_1c3250 },
    Hand { entry: 0x1eda18, end: 0x1eda38, hash: 0xe8d44c9536a3685b, est: 8, f: font_tex::text_width_35f0 },
    Hand { entry: 0x1eda38, end: 0x1eda58, hash: 0xa22eabd4735c1a3b, est: 8, f: font_tex::text_width_3990 },
    Hand { entry: 0x1edce0, end: 0x1edd5c, hash: 0xb2b2af1fab8ae0bd, est: 31, f: font_tex::fx_upload_draw_slot1 },
    Hand { entry: 0x1edd60, end: 0x1edddc, hash: 0xf4b27619e8da8736, est: 31, f: font_tex::font2_upload_draw },
    Hand { entry: 0x1ee168, end: 0x1ee1f4, hash: 0xd42945e0c346c6f8, est: 35, f: draw_run_text::text_place_1ee168 },
    Hand { entry: 0x1ee1f8, end: 0x1ee284, hash: 0x5af47c8e69145fa3, est: 35, f: font2_text::font2_draw_right },
    Hand { entry: 0x1ee288, end: 0x1ee314, hash: 0x5fe7c9ab54b43efe, est: 35, f: glyph_vu::text_width_upload_draw },
    Hand { entry: 0x1ee318, end: 0x1ee3ac, hash: 0x4f1de759d075cb98, est: 37, f: draw_run_text::text_place_1ee318 },
    Hand { entry: 0x1ee3b0, end: 0x1ee444, hash: 0xba9497c2de2fc969, est: 37, f: font2_text::font2_draw_centred },
    Hand { entry: 0x1ee448, end: 0x1ee4dc, hash: 0x313022d5dd879ca2, est: 37, f: font2_text::font2_draw_centred_b },
    Hand { entry: 0x1ee9d0, end: 0x1eea3c, hash: 0xf7ab80cf6261dde3, est: 27, f: slot_dispatch::upload_then_fill_1 },
    Hand { entry: 0x1eea40, end: 0x1eeaac, hash: 0xd7ccd0458034e0b8, est: 27, f: slot_dispatch::upload_then_fill_2 },
    Hand { entry: 0x1eeab8, end: 0x1eeaf0, hash: 0x4dfff2b42d138715, est: 14, f: glyph_vu::fill_glyph_record },
    Hand { entry: 0x1eeec0, end: 0x1eefac, hash: 0x810d2818303ebc7b, est: 47, f: slot_dispatch::fill_pair_slots },
    Hand { entry: 0x1eefb0, end: 0x1eefdc, hash: 0xd9f5889fbcf0d1f5, est: 11, f: misc_leaf::flush_chain },
    Hand { entry: 0x1eefe0, end: 0x1ef068, hash: 0xc80642fdcc4c72f9, est: 8, f: blend_scene::scene_flush },
    Hand { entry: 0x1efd74, end: 0x1efdf4, hash: 0x466b288e4ec00292, est: 32, f: dma_ring::setup_quads },
    Hand { entry: 0x1efdf4, end: 0x1efe34, hash: 0x271ac261f139a8f1, est: 16, f: dma_ring::ring_entry },
    Hand { entry: 0x1f0430, end: 0x1f0624, hash: 0x290dcb3f65a260d, est: 2477, f: gifpkt::build_gif_packet },
    Hand { entry: 0x1f0f28, end: 0x1f0fac, hash: 0x84afa32f439b0106, est: 30, f: slots::slot_spawn },
    Hand { entry: 0x1f0fb0, end: 0x1f1070, hash: 0x83c8b2792e55fd4e, est: 108, f: slots::slot_update },
    Hand { entry: 0x1f14a8, end: 0x1f14b8, hash: 0xd20b152003028f7f, est: 4, f: mathx::scale_float },
    Hand { entry: 0x1f14b8, end: 0x1f14e0, hash: 0x2fcd69b1ccdeb15, est: 10, f: mathx::float_to_int_scaled },
    Hand { entry: 0x1f1500, end: 0x1f1530, hash: 0x65301d65797634d3, est: 7, f: mathx::tick_i32 },
    Hand { entry: 0x1f1560, end: 0x1f1590, hash: 0x8e7c3a4cf17634d3, est: 6, f: mathx::tick_u8 },
    Hand { entry: 0x1f1590, end: 0x1f15a0, hash: 0xf33181d61ba51ffd, est: 4, f: mathx::log2_floor },
    Hand { entry: 0x1f15b0, end: 0x1f15d8, hash: 0xe3ca5b5c337a4eee, est: 8, f: misc_leaf::countdown_by_two },
    Hand { entry: 0x1f15d8, end: 0x1f1600, hash: 0xd05491c69944e1b9, est: 8, f: mathx::fill_words },
    Hand { entry: 0x1f1600, end: 0x1f1628, hash: 0x9f171e2301411499, est: 8, f: mathx::zero_qwords },
    Hand { entry: 0x1f1628, end: 0x1f16c0, hash: 0x6409a2fb4705a6ce, est: 37, f: mathx::mem_copy },
    Hand { entry: 0x1f16c0, end: 0x1f16e8, hash: 0xf6c82f964d68e92c, est: 8, f: mathx::copy_qwords },
    Hand { entry: 0x1f16e8, end: 0x1f1718, hash: 0xe5bbbcbdbac348bb, est: 10, f: misc_leaf::or_copy_quads },
    Hand { entry: 0x1f1718, end: 0x1f1748, hash: 0x89da2458ada76dda, est: 12, f: gifpkt::gs_regs_write },
    Hand { entry: 0x1f1748, end: 0x1f1770, hash: 0xfab9ddb6c8e50f62, est: 10, f: vecmath::clip_flags },
    Hand { entry: 0x1f1778, end: 0x1f1798, hash: 0x502b7baf3417e7f5, est: 8, f: mathx::sqrt_q },
    Hand { entry: 0x1f1798, end: 0x1f17b0, hash: 0x94890b99bb389bc, est: 5, f: misc_leaf::iabs },
    Hand { entry: 0x1f17b0, end: 0x1f17b8, hash: 0xc82dbde1b76c20b6, est: 2, f: motion::fabs },
    Hand { entry: 0x1f17b8, end: 0x1f17c0, hash: 0xd32daae1ca1d006d, est: 2, f: mathx::fmax },
    Hand { entry: 0x1f17c0, end: 0x1f17c8, hash: 0xd32da9e1ca1cfeba, est: 2, f: mathx::fmin },
    Hand { entry: 0x1f17d8, end: 0x1f17e8, hash: 0x2e8f25e48568e1ea, est: 4, f: mathx::fclamp },
    Hand { entry: 0x1f17e8, end: 0x1f17f8, hash: 0x2d2ad6bcbad6ed15, est: 4, f: cell_row_emit::zero_record_12 },
    Hand { entry: 0x1f17f8, end: 0x1f1800, hash: 0x484dc2bb7c228935, est: 2, f: euler_basis::zero_vec4 },
    Hand { entry: 0x1f1828, end: 0x1f1840, hash: 0x43ab774f77d350d, est: 6, f: vecmath::vec_add_self },
    Hand { entry: 0x1f1858, end: 0x1f1870, hash: 0xc4500788b86642c9, est: 6, f: glyph_vu::vsub_xyz_inplace },
    Hand { entry: 0x1f1870, end: 0x1f1898, hash: 0x9aba52ea6b57e8d9, est: 10, f: vu_vec::vec_lerp_xyz },
    Hand { entry: 0x1f1898, end: 0x1f18b0, hash: 0xcbf2d873f030e55d, est: 6, f: vecmath::vec_scale },
    Hand { entry: 0x1f18b0, end: 0x1f18c8, hash: 0xebf2d8742690e55d, est: 6, f: vecmath::vec_scale_self },
    Hand { entry: 0x1f18c8, end: 0x1f18e0, hash: 0xebf231c04590e55d, est: 6, f: vecmath::vec4_scale },
    Hand { entry: 0x1f18e0, end: 0x1f18f8, hash: 0xe43025280808ae2f, est: 6, f: glyph_vu::vmul_xyz_store },
    Hand { entry: 0x1f18f8, end: 0x1f1910, hash: 0xe53ae55ecae8ae2f, est: 6, f: vecmath::vec4_mul },
    Hand { entry: 0x1f1910, end: 0x1f1930, hash: 0xa05b57bd4b985b7a, est: 8, f: vecmath::vec_div_scalar },
    Hand { entry: 0x1f1930, end: 0x1f1958, hash: 0x8a509f8f0c80779f, est: 10, f: vecmath::dot3 },
    Hand { entry: 0x1f1958, end: 0x1f1970, hash: 0xa6e82c8f523fbfd5, est: 6, f: vecmath::cross3 },
    Hand { entry: 0x1f1970, end: 0x1f19a0, hash: 0x8d7d3c7ffd6629df, est: 12, f: vecmath::length3 },
    Hand { entry: 0x1f19a0, end: 0x1f19c8, hash: 0x30b01f5738a39d92, est: 10, f: vecmath::length_xy },
    Hand { entry: 0x1f19c8, end: 0x1f19e8, hash: 0x21fa9a4f238ec507, est: 8, f: mat4_rle::vec3_len_sq },
    Hand { entry: 0x1f19e8, end: 0x1f1a20, hash: 0x5dbfbfc7d59061a9, est: 14, f: vu_vec::dist3 },
    Hand { entry: 0x1f1a20, end: 0x1f1a50, hash: 0x4a3dc79cd443b6ce, est: 12, f: vecmath::dist_xy },
    Hand { entry: 0x1f1a50, end: 0x1f1a98, hash: 0x41079e8c7b55efff, est: 16, f: vu_probes::diff_dot_sign },
    Hand { entry: 0x1f1a98, end: 0x1f1ae8, hash: 0xee34454c38ca84cb, est: 16, f: vu_vec::vec_set_length },
    Hand { entry: 0x1f1ae8, end: 0x1f1b30, hash: 0xda101bd28a4f58fe, est: 14, f: vecmath::vec_set_length_xy },
    Hand { entry: 0x1f1b30, end: 0x1f1b98, hash: 0x20fc0ba0b9cdc257, est: 20, f: vecmath::vec_clamp_length },
    Hand { entry: 0x1f1b98, end: 0x1f1bf8, hash: 0x600a921aaabe8736, est: 17, f: vu_probes::vec_cap_length_xy },
    Hand { entry: 0x1f1bf8, end: 0x1f1c68, hash: 0x2203b193f12b4eae, est: 24, f: vu_probes::reflect_vec },
    Hand { entry: 0x1f1c68, end: 0x1f1c90, hash: 0x99ed4473b3d6759d, est: 10, f: vecmath::vec3_mul_mat3 },
    Hand { entry: 0x1f1c90, end: 0x1f1cc0, hash: 0xfdc86182d3c84b85, est: 12, f: vu_vec::vec4_mul_mat4 },
    Hand { entry: 0x1f1cc0, end: 0x1f1cd8, hash: 0x9b92d604babad3c1, est: 6, f: vu_probes::pack_bytes_f32 },
    Hand { entry: 0x1f1cd8, end: 0x1f1d00, hash: 0x6b749a523099c439, est: 10, f: vu_blend_pack::pack_vec4_bytes },
    Hand { entry: 0x1f1d00, end: 0x1f1d48, hash: 0x4b66efc01bfc8b94, est: 18, f: vecmath::vec4_to_i16x4 },
    Hand { entry: 0x1f1d50, end: 0x1f1d80, hash: 0xa235b251a6f1b481, est: 148, f: mathx::pair_byte_sum },
    Hand { entry: 0x1f1d80, end: 0x1f1d98, hash: 0xbbddbff4e4c217b2, est: 6, f: trig::fast_cos },
    Hand { entry: 0x1f1d98, end: 0x1f1db0, hash: 0x9ad7d0ab4d291f32, est: 6, f: trig::fast_sin },
    Hand { entry: 0x1f1db0, end: 0x1f1e48, hash: 0x35896bb343619c50, est: 34, f: mathx::acos_approx },
    Hand { entry: 0x1f1e48, end: 0x1f1f58, hash: 0xe727bef8271e5e1b, est: 65, f: vu_geom::fast_atan2 },
    Hand { entry: 0x1f1f80, end: 0x1f1fb0, hash: 0x61b9f866ef1a0eaa, est: 12, f: vecmath::mat4_identity },
    Hand { entry: 0x1f1fb0, end: 0x1f1fe8, hash: 0xfbad05a8ee9885a9, est: 14, f: vecmath::mat4_scale },
    Hand { entry: 0x1f2008, end: 0x1f2028, hash: 0xa12b08c12316e44c, est: 8, f: euler_basis::euler_matrix },
    Hand { entry: 0x1f2250, end: 0x1f2270, hash: 0x85fd0ca08db2aeed, est: 8, f: vecmath::mat3_copy_w1 },
    Hand { entry: 0x1f2270, end: 0x1f2290, hash: 0x5fd1dbedf32ca1d, est: 8, f: vecmath::mat3_copy },
    Hand { entry: 0x1f2290, end: 0x1f22e0, hash: 0xce5335c0449e8ee1, est: 20, f: vecmath::mat3_transpose },
    Hand { entry: 0x1f22e0, end: 0x1f2320, hash: 0xf7a69fa3e6d4747d, est: 16, f: mat4_rle::mat4_scale },
    Hand { entry: 0x1f2320, end: 0x1f2370, hash: 0x7f35a2e6ba55fee2, est: 20, f: vecmath::mat3_mul },
    Hand { entry: 0x1f2370, end: 0x1f23b8, hash: 0x59bff30fabfe91ba, est: 48, f: vu_geom::mat4_mul_rows },
    Hand { entry: 0x1f23b8, end: 0x1f2400, hash: 0x4d7b66782a2db2f5, est: 18, f: mat4_rle::mat4_add },
    Hand { entry: 0x1f2440, end: 0x1f24c0, hash: 0x900969955297ff56, est: 28, f: vu_blend_pack::quat_nlerp },
    Hand { entry: 0x1f24c0, end: 0x1f2580, hash: 0x6ec267474ccc6edb, est: 37, f: trig::axis_quat },
    Hand { entry: 0x1f2580, end: 0x1f25f8, hash: 0x78882e74cb62f7ff, est: 30, f: vecmath::quat_to_mat3 },
    Hand { entry: 0x1f25f8, end: 0x1f2680, hash: 0x84463b7496c93de5, est: 34, f: vu_blend_pack::basis_from_vec },
    Hand { entry: 0x1f2680, end: 0x1f26c8, hash: 0xa2eb4278a3a4afbf, est: 14, f: vecmath::angle_add },
    Hand { entry: 0x1f26c8, end: 0x1f2710, hash: 0x32ed3edf10328d2e, est: 14, f: vecmath::angle_sub },
    Hand { entry: 0x1f2710, end: 0x1f2788, hash: 0x8effe7f9b9292765, est: 19, f: mathx::angle_wrap },
    Hand { entry: 0x1f2788, end: 0x1f27c0, hash: 0x29d01eb7f2cf3e47, est: 12, f: mathx::angle_dist },
    Hand { entry: 0x1f27c0, end: 0x1f27d8, hash: 0xa2c7bac03b82ece6, est: 6, f: mathx::split_frac },
    Hand { entry: 0x1f27d8, end: 0x1f27e8, hash: 0xc98cf94020dad00d, est: 4, f: mathx::int_to_float },
    Hand { entry: 0x1f27e8, end: 0x1f27f8, hash: 0xd1a19cad48f98349, est: 4, f: mathx::float_to_int },
    Hand { entry: 0x1f27f8, end: 0x1f2830, hash: 0xaca0af62beaea282, est: 14, f: mathx::fmod_q },
    Hand { entry: 0x1f2830, end: 0x1f2878, hash: 0xa34e07384df1fba0, est: 18, f: vecmath::color_lerp },
    Hand { entry: 0x1f2970, end: 0x1f29ac, hash: 0x40a4ebe375e6d409, est: 13, f: vu_blend_pack::sphere_fade_store },
    Hand { entry: 0x1f29b0, end: 0x1f2aa8, hash: 0x9affe1421a5414c5, est: 851710, f: mat4_rle::rle_bits_pack },
    Hand { entry: 0x1f2aa8, end: 0x1f2b60, hash: 0x5bb8eaf67cf446c4, est: 41, f: approach_glue::approach_angle },
    Hand { entry: 0x1f4000, end: 0x1f4028, hash: 0xe0743eb7c5b76c19, est: 10, f: gs_pkt_misc::set_clear_rgbaq },
    Hand { entry: 0x1f4028, end: 0x1f404c, hash: 0x5f5bbba80d1d91a9, est: 9, f: approach_glue::forward_word },
    Hand { entry: 0x1f4050, end: 0x1f40e4, hash: 0x4340a73a97ea974c, est: 33, f: dma_setup::tag_emit_or_flush },
    Hand { entry: 0x1f40e8, end: 0x1f4150, hash: 0xa52691b63f6f268d, est: 26, f: gs_pkt_misc::append_tag_pair },
    Hand { entry: 0x1f4150, end: 0x1f41bc, hash: 0x15371dffbb7c4f91, est: 27, f: dma_setup::tag_emit },
    Hand { entry: 0x1f4400, end: 0x1f445c, hash: 0xeb0852bf99dd719e, est: 23, f: dma_setup::tag_emit_gs },
    Hand { entry: 0x1f4460, end: 0x1f44bc, hash: 0xe6a618be84be80a0, est: 23, f: ref_table::append_vif_ref },
    Hand { entry: 0x1f4830, end: 0x1f483c, hash: 0x7233d8e59e7cd30a, est: 3, f: approach_glue::level_word },
    Hand { entry: 0x1f6928, end: 0x1f69b0, hash: 0x1238d9ba18c0db8, est: 16, f: cancel_save::help_state_step },
    Hand { entry: 0x1f69b0, end: 0x1f6a44, hash: 0xa538b95a76e35fd9, est: 15, f: approach_glue::claim_slot_try },
    Hand { entry: 0x1f6a48, end: 0x1f6ad0, hash: 0x1d6b84d5d11a5cdb, est: 7, f: gs_pkt_misc::clear_help_slot },
    Hand { entry: 0x1f6ad0, end: 0x1f6b0c, hash: 0x94a228bee1c730a5, est: 1044, f: gs_pkt_misc::clear_flag_bits },
    Hand { entry: 0x1f6bc0, end: 0x1f6c20, hash: 0x9ddea4abeb561c79, est: 13, f: cancel_save::help_graceful_cancel },
    Hand { entry: 0x1f6c20, end: 0x1f6c2c, hash: 0xb3dd95da50f677af, est: 3, f: cancel_save::help_chain_clear },
    Hand { entry: 0x1f6c30, end: 0x1f6d64, hash: 0x4fed07d6636eb4e7, est: 70, f: slot_rec_fill::fill_slot_record },
    Hand { entry: 0x1f7770, end: 0x1f7854, hash: 0x354e618c29dee4f2, est: 54, f: tex_blit::tex_blit_slot },
    Hand { entry: 0x1f7858, end: 0x1f7b9c, hash: 0x6b959ad6b37e58d9, est: 16, f: hero_rig::rig_update },
    Hand { entry: 0x1f7ba0, end: 0x1f7c04, hash: 0x1ad95dd55139d56b, est: 389, f: tex_blit::find_tex_slot },
    Hand { entry: 0x1f7c08, end: 0x1f7d08, hash: 0xa1dac611634b0354, est: 80, f: swim_queue::queue_touch },
    Hand { entry: 0x1f8028, end: 0x1f80a8, hash: 0x15dbe35f6677c893, est: 32, f: cam_polar_hero::hero_hand_init },
    Hand { entry: 0x1f80a8, end: 0x1f8160, hash: 0x835331af0109a025, est: 24, f: hero_rig::hero_idle_seq },
    Hand { entry: 0x1f8160, end: 0x1f8168, hash: 0x485da57e503d87ee, est: 2, f: hero_rig::seq_identity },
    Hand { entry: 0x1f8168, end: 0x1f81b4, hash: 0xd234d14b7b1debf6, est: 18, f: hero_damage_reset::hero_take_damage },
    Hand { entry: 0x1f81b8, end: 0x1f8210, hash: 0xa7e8c15476f06c21, est: 7, f: joint_block_pose::joint_state_slot_hand },
    Hand { entry: 0x1f85e0, end: 0x1f86a4, hash: 0x1b025074b5230701, est: 529, f: hero_damage_reset::hero_joint_blocks_reset },
    Hand { entry: 0x1f86a8, end: 0x1f8750, hash: 0x6b18cd0125e7a8ee, est: 24, f: hero_head_blink::hero_blink_schedule },
    Hand { entry: 0x1f8f50, end: 0x1f9008, hash: 0xba52003a90f99ef4, est: 11, f: hero_head_blink::marker_prompt_finalize },
    Hand { entry: 0x1f9128, end: 0x1f9294, hash: 0x5de84612bc7ba815, est: 30, f: foot_probe::hero_footsteps },
    Hand { entry: 0x1f9cb0, end: 0x1f9da0, hash: 0x61a09e711a3ddf7b, est: 119, f: foot_probe::slot_release },
    Hand { entry: 0x1f9da0, end: 0x1f9ea8, hash: 0x306f9e8b25659349, est: 23, f: frame_dispatch::dispatch_gate },
    Hand { entry: 0x1fa750, end: 0x1fa950, hash: 0x4be7375cf2a8f951, est: 44, f: fx_sway::fx_tint_update },
    Hand { entry: 0x1fa950, end: 0x1faa8c, hash: 0x7e0df8350f413b99, est: 16, f: fx_sway::fx_sway_matrix },
    Hand { entry: 0x1faa90, end: 0x1fab00, hash: 0x60c090c1fe6633eb, est: 10, f: fx_sway::fx_sway_yaw },
    Hand { entry: 0x1fab00, end: 0x1facf0, hash: 0x9eb3bf1cbc2b7207, est: 34, f: limb_pulse::limb_pulse },
    Hand { entry: 0x1facf0, end: 0x1fada4, hash: 0x3b5f9dd32f8ab173, est: 9, f: limb_pulse::pulse_timer_step },
    Hand { entry: 0x1fada8, end: 0x1fade8, hash: 0x1a7d4745ed3d5bd4, est: 16, f: limb_pulse::pulse_init_once },
    Hand { entry: 0x1fb8c8, end: 0x1fbab4, hash: 0x46bf62b2d8bb212c, est: 111, f: hero_state::hero_pose_sync },
    Hand { entry: 0x1fbfc8, end: 0x1fc0c0, hash: 0x5c5eeff9f86b5b7b, est: 35, f: hero_hand::hand_rate_slew },
    Hand { entry: 0x1fc0c0, end: 0x1fc174, hash: 0xdd6f0e7a1931f029, est: 44, f: swim_queue::table_lerp },
    Hand { entry: 0x1fc280, end: 0x1fc2e4, hash: 0x832275e3a5c1850e, est: 25, f: seq_lookup::euler_pose_vec4 },
    Hand { entry: 0x1fc2e8, end: 0x1fc36c, hash: 0x72fe668990126824, est: 33, f: hero_hand::hand_pose_chain },
    Hand { entry: 0x1fc370, end: 0x1fc5e8, hash: 0xd2c6dd3c1665b8eb, est: 872, f: pose_spring::build_pose_copy_frame },
    Hand { entry: 0x1fc5e8, end: 0x1fc6dc, hash: 0x6f6d97951c72d4dc, est: 23, f: ripple_items::ripple_tick },
    Hand { entry: 0x1fd288, end: 0x1fd29c, hash: 0x74dede00d5ff80ab, est: 5, f: pose_spring::set_spring_pair_a },
    Hand { entry: 0x1fd2a0, end: 0x1fd2b4, hash: 0x39af7df70a692c6b, est: 5, f: pose_spring::set_spring_pair_b },
    Hand { entry: 0x1fd2b8, end: 0x1fd2cc, hash: 0xbffe60d94b5f08eb, est: 5, f: hero_front_probe::hero_block2_gain },
    Hand { entry: 0x1fd2d0, end: 0x1fd2e4, hash: 0x9c7f7df73950472b, est: 5, f: hero_front_probe::hero_block3_gain },
    Hand { entry: 0x1fd2e8, end: 0x1fd3bc, hash: 0x173f6c3ef3538c02, est: 47, f: hero_front_probe::hero_front_probe },
    Hand { entry: 0x1fd3c0, end: 0x1fd5a4, hash: 0x49dd3028a45ad881, est: 14, f: glove_aim::glove_aim_block },
    Hand { entry: 0x1fd5a8, end: 0x1fd5e4, hash: 0x91099a66b9a2d66a, est: 36, f: glove_aim::clear_aim_slots },
    Hand { entry: 0x1fda90, end: 0x1fdd3c, hash: 0xb72163bffb04c6e9, est: 29, f: glove_aim::aim_jitter_tick },
    Hand { entry: 0x1fe280, end: 0x1fe660, hash: 0x885f83e5dc346ca3, est: 79, f: feet_ik::foot_ik_blocks },
    Hand { entry: 0x1fe660, end: 0x1fe9d8, hash: 0xefb48201b9608279, est: 65, f: hero_seek::aim_step },
    Hand { entry: 0x200000, end: 0x2003a0, hash: 0x1f526c8e36c2758c, est: 14, f: grind_rail::grind_zone_tick },
    Hand { entry: 0x2003a0, end: 0x2003d8, hash: 0xd4c93dc67ff07c26, est: 14, f: grind_rail::grind_probe_point },
    Hand { entry: 0x2003d8, end: 0x200504, hash: 0xeb4b3102a5e1d6c0, est: 23, f: grind_rail::grind_rail_probe },
    Hand { entry: 0x200508, end: 0x20053c, hash: 0x947532a9aa97b728, est: 11, f: glove_slot::slot_value_a_fn },
    Hand { entry: 0x200540, end: 0x200574, hash: 0xb61236ebb17a2d86, est: 11, f: glove_slot::slot_value_b_fn },
    Hand { entry: 0x200578, end: 0x2005c0, hash: 0x307ff92e4099f8e3, est: 17, f: glove_slot::item_is_glove },
    Hand { entry: 0x2005c0, end: 0x2005e4, hash: 0x988a4988c51a7eb7, est: 9, f: glove_gate::slot_b_in_5_7 },
    Hand { entry: 0x2005e8, end: 0x20060c, hash: 0x8938b3804d0f041, est: 9, f: glove_gate::slot_b_in_1c_1d },
    Hand { entry: 0x200610, end: 0x200660, hash: 0x1bf97a7b6c79433b, est: 18, f: glove_gate::hero_skip_gate },
    Hand { entry: 0x200660, end: 0x200674, hash: 0x9ef21b153320fad7, est: 4, f: seq_lookup::hold_seq_map },
    Hand { entry: 0x200678, end: 0x200700, hash: 0x854181374aa822d7, est: 6, f: seq_lookup::code_store },
    Hand { entry: 0x200700, end: 0x200758, hash: 0x6529f56ca4954365, est: 22, f: ripple_items::item_frac },
    Hand { entry: 0x200ab8, end: 0x200b10, hash: 0xd1fb612d572f91d2, est: 10, f: seq_lookup::code_to_id },
    Hand { entry: 0x200b10, end: 0x200dc8, hash: 0xe72c0f5365d95028, est: 53, f: hero_lock::hero_lock_fx_update },
    Hand { entry: 0x201408, end: 0x20156c, hash: 0xa769c1c825cdfb69, est: 24, f: held_layers::hero_item_layers_on_idle_exit },
    Hand { entry: 0x201740, end: 0x2017d0, hash: 0x9ab9027cdbb8cac4, est: 16, f: hero_aux::hero_weapon_release },
    Hand { entry: 0x201af8, end: 0x201b24, hash: 0xd51c1cf5e0ee8085, est: 11, f: gate_frames::held_layers_frame },
    Hand { entry: 0x202160, end: 0x202208, hash: 0x655d5a889beec7d9, est: 129, f: hero_aux::abs_diff_sum3 },
    Hand { entry: 0x202d50, end: 0x202e24, hash: 0xadaeb3829e5c4550, est: 34, f: slot_tables::slot_release },
    Hand { entry: 0x202e28, end: 0x202e88, hash: 0x8f4f44ed148460ce, est: 10, f: slot_tables::mode_table_a },
    Hand { entry: 0x202e88, end: 0x202ed4, hash: 0xd6ad905bb000bbc9, est: 8, f: slot_tables::mode_table_b },
    Hand { entry: 0x202ed8, end: 0x202f44, hash: 0x67c59a5f6821d613, est: 25, f: ripple_items::item_use_allowed },
    Hand { entry: 0x2039d0, end: 0x203aac, hash: 0x693c5e14cb00db62, est: 49, f: gate_frames::hero_gate_item_frame },
    Hand { entry: 0x203bd8, end: 0x203d24, hash: 0xec2c7506d98bc6b5, est: 55, f: hero_slot_sweep::slot_sweep_tick },
    Hand { entry: 0x203d28, end: 0x203d90, hash: 0x58dac877188e4436, est: 16, f: hero_aux::knock_vec },
    Hand { entry: 0x204408, end: 0x204428, hash: 0xd8a245f2ec9b8433, est: 6, f: hero_items::actor_kind_is_47 },
    Hand { entry: 0x204718, end: 0x2048d8, hash: 0x3e0d7b3e8ff6445f, est: 93, f: view_track::view_frame },
    Hand { entry: 0x204918, end: 0x204958, hash: 0x7534b867986a6009, est: 11, f: hero_wish_yaw::hero_frame_crossed },
    Hand { entry: 0x204970, end: 0x204c90, hash: 0x5104b043f830451e, est: 84, f: hero_wish_yaw::hero_stick_wish_yaw },
    Hand { entry: 0x204c90, end: 0x204d9c, hash: 0x38f42fa5fe7623fd, est: 36, f: hero_wish_yaw::hero_stick_magnitude },
    Hand { entry: 0x204e20, end: 0x204ed4, hash: 0x668da31f0663b4c1, est: 45, f: slot_turn::aim_turn_step },
    Hand { entry: 0x204ed8, end: 0x204f40, hash: 0xfde6c3b2d78b210e, est: 22, f: hero_turn::hero_yaw_turn },
    Hand { entry: 0x204f40, end: 0x204fdc, hash: 0xdfa2fc4618f65f7c, est: 26, f: slot_turn::yaw_turn_by_slot_b },
    Hand { entry: 0x204fe0, end: 0x205130, hash: 0xae66479ebfbd5acc, est: 52, f: hero_turn::hero_gait0_turn },
    Hand { entry: 0x205130, end: 0x205180, hash: 0x29570145a5e21c6a, est: 16, f: hero_turn::hero_speed_slew },
    Hand { entry: 0x205180, end: 0x205264, hash: 0x39f799bdb84d6224, est: 32, f: hero_slide::hero_move_dir_vec },
    Hand { entry: 0x205268, end: 0x2053c0, hash: 0x8c0e071e6909dbf7, est: 27, f: surface_push::surface_push },
    Hand { entry: 0x2053c0, end: 0x205760, hash: 0x66b9e53a0d006946, est: 133, f: hero_slide::hero_ground_probe_slide },
    Hand { entry: 0x205760, end: 0x205860, hash: 0x93b6e328fb015746, est: 37, f: hero_slide::hero_push_pull },
    Hand { entry: 0x206360, end: 0x2063f8, hash: 0xbafefbd4f4095713, est: 33, f: hero_snap::hero_push_decay },
    Hand { entry: 0x2063f8, end: 0x20644c, hash: 0xe5172261793bea80, est: 21, f: hero_snap::hero_forward_dot },
    Hand { entry: 0x206850, end: 0x206924, hash: 0xd5bc301666ca8911, est: 29, f: aim_steer::ring_slot_copy },
    Hand { entry: 0x206e90, end: 0x206f0c, hash: 0x9e989cb1fa05c63c, est: 31, f: surface_push::scale_block },
    Hand { entry: 0x206f10, end: 0x2071b0, hash: 0x483de8b88036d23b, est: 120, f: aim_steer::aim_vector },
    Hand { entry: 0x208018, end: 0x208128, hash: 0x884c406ccef2fb0f, est: 52, f: aim_steer::aim_lock },
    Hand { entry: 0x208128, end: 0x20847c, hash: 0x6c115e1742decf8d, est: 14, f: hero_tilt::hero_magnet_tilt_blocks },
    Hand { entry: 0x209038, end: 0x2091b8, hash: 0xa9dcd27019cf4a94, est: 86, f: hero_tilt::hero_tilt_gain_blocks },
    Hand { entry: 0x2096f8, end: 0x209850, hash: 0xc6efe9f2cda5a4c0, est: 79, f: hero_recover::hero_tilt_recover },
    Hand { entry: 0x209850, end: 0x20990c, hash: 0x5d6f5aaa5cbeb10a, est: 35, f: hero_recover::hero_carry_decay },
    Hand { entry: 0x209f88, end: 0x209ffc, hash: 0x1160534ec9cf5274, est: 29, f: slot_turn::set_slot_pair },
    Hand { entry: 0x20a050, end: 0x20a0cc, hash: 0xd921fd51cc2f1695, est: 83, f: comet_guard::tick_slots },
    Hand { entry: 0x20a258, end: 0x20a4a4, hash: 0x4c2114d97cef189f, est: 66, f: comet_guard::hero_edge_guard },
    Hand { entry: 0x20a4a8, end: 0x20a58c, hash: 0x5a56bddec440f432, est: 29, f: wallop_cues::wallop_feedback_scan },
    Hand { entry: 0x214450, end: 0x214708, hash: 0x75a603e90a67541e, est: 61, f: hero_probe_sweep::hero_probe_update },
    Hand { entry: 0x214ca8, end: 0x214ed4, hash: 0xe4cd8c6814e1625, est: 26, f: hero_probe_sweep::hero_tilt_settle },
    Hand { entry: 0x214f38, end: 0x21501c, hash: 0xce6505aeb1d5c4a8, est: 52, f: hero_water::heading_bucket },
    Hand { entry: 0x21a778, end: 0x21a800, hash: 0x5d6d1e9d773db116, est: 19, f: wallop_cues::ledge_mode_cue },
    Hand { entry: 0x21a800, end: 0x21a8b4, hash: 0xe1b9419d52b405b2, est: 15, f: hero_water::water_exit_gate },
    Hand { entry: 0x21a8b8, end: 0x21aaec, hash: 0x59a85087fa3fbc14, est: 27, f: hero_water::breath_water_reaction },
    Hand { entry: 0x21aaf0, end: 0x21ae7c, hash: 0xd7ade10afd27382, est: 62, f: hero_edge_probe::edge_probe_step },
    Hand { entry: 0x21bcb8, end: 0x21bd54, hash: 0x87731a4978079f20, est: 21, f: hero_edge_probe::combo_input_fire },
    Hand { entry: 0x21bd58, end: 0x21c0e8, hash: 0x4498d88f6675cd76, est: 69, f: hero_blend::hero_blend_update },
    Hand { entry: 0x21c0e8, end: 0x21c150, hash: 0x9ff7231f995be457, est: 21, f: wallop_cues::release_held_moby },
    Hand { entry: 0x21c150, end: 0x21c4e4, hash: 0x75b3b532cf675d81, est: 246, f: slot_pick::weighted_pick },
    Hand { entry: 0x21c4e8, end: 0x21c7a0, hash: 0xdeae14f7a2243011, est: 57, f: hero_jump_land::landing_end },
    Hand { entry: 0x21ca50, end: 0x21cbd4, hash: 0xef46d6d5c16107c9, est: 80, f: cone_probe::cone_probe_hit },
    Hand { entry: 0x21cbd8, end: 0x21ccb0, hash: 0x290b67a615145e11, est: 14, f: hero_weapon_anim::hero_weapon_idle },
    Hand { entry: 0x224e48, end: 0x224ef0, hash: 0x215524041b53bbfa, est: 42, f: hero_weapon_anim::held_anim_pair },
    Hand { entry: 0x224ef0, end: 0x224fb8, hash: 0x9f09609a819b1f18, est: 61, f: hero_anim::held_seq_find_hand },
    Hand { entry: 0x224fb8, end: 0x225134, hash: 0x6f575002308f2de1, est: 55, f: hero_anim::hero_held_mirror },
    Hand { entry: 0x225138, end: 0x225268, hash: 0xb92a0d5886a92122, est: 69, f: hero_anim::held_seq_pick_hand },
    Hand { entry: 0x225268, end: 0x2252e8, hash: 0x5de39210735047e0, est: 21, f: hero_anim::held_speed_copy },
    Hand { entry: 0x2252e8, end: 0x2252fc, hash: 0x137c46bda9ab30c7, est: 5, f: hero_anim::held_extra_clear },
    Hand { entry: 0x225408, end: 0x225460, hash: 0x3cbc3f06a5c226aa, est: 11, f: hero_anim::held_extra_speed },
    Hand { entry: 0x225460, end: 0x2254f8, hash: 0xf658594e727a93f, est: 7, f: hero_anim::held_extra_sync },
    Hand { entry: 0x2254f8, end: 0x2256dc, hash: 0x273fac5c6e5fca8b, est: 99, f: hero_anim::hero_set_anim },
    Hand { entry: 0x2256e0, end: 0x225720, hash: 0xa1ed2af50488835e, est: 16, f: hero_weapon_anim::anim_track_setup },
    Hand { entry: 0x225720, end: 0x225768, hash: 0x4e3c705172a9d9ef, est: 17, f: hero_anim::hero_loop_range },
    Hand { entry: 0x225768, end: 0x225780, hash: 0x39bcd1a2c07bf6c4, est: 6, f: hero_anim::hero_loop_clear_hand },
    Hand { entry: 0x225780, end: 0x2257b0, hash: 0x7e0037e953e18fa0, est: 9, f: hero_anim::set_sound_voice },
    Hand { entry: 0x226128, end: 0x226210, hash: 0x9fa4685a5f167969, est: 83, f: ammo_flags::mark_pending_flags },
    Hand { entry: 0x226210, end: 0x22634c, hash: 0xfa67bc1db309e14f, est: 88, f: hero_anim::hero_items_reshow },
    Hand { entry: 0x226388, end: 0x2263c0, hash: 0x9f127aa47174f91d, est: 11, f: hero_pad_walk::clear_pad_flags },
    Hand { entry: 0x2263c8, end: 0x22640c, hash: 0x77c91d83335a1381, est: 17, f: hero_anim::foot_dust_set },
    Hand { entry: 0x226580, end: 0x226610, hash: 0xd8c41a704523e605, est: 23, f: hero_anim::wrench_swing_count },
    Hand { entry: 0x226610, end: 0x226704, hash: 0x4923589af69a1d8c, est: 24, f: hero_space::up_offset_hand },
    Hand { entry: 0x226708, end: 0x2267a0, hash: 0x862292cea3051505, est: 38, f: hero_space::polar_to_world },
    Hand { entry: 0x2267a0, end: 0x226824, hash: 0xeb7b6f4b2df9a4e7, est: 33, f: hero_space::point_from_offset },
    Hand { entry: 0x226828, end: 0x226844, hash: 0xdb51c5b7f8b0f644, est: 7, f: hero_space::up_offset_neg },
    Hand { entry: 0x226848, end: 0x226950, hash: 0xd184d14e6621c0fd, est: 39, f: hero_space::point_ahead },
    Hand { entry: 0x226950, end: 0x226a0c, hash: 0x3d347de85713ce9e, est: 28, f: hero_space::point_up },
    Hand { entry: 0x226a10, end: 0x226b0c, hash: 0xfcd8a18b723dd9dc, est: 26, f: hero_space::project_off_up },
    Hand { entry: 0x226b10, end: 0x226bb4, hash: 0xb89c06ad0b6eaf34, est: 19, f: hero_space::length_off_up },
    Hand { entry: 0x226bb8, end: 0x226c5c, hash: 0x47119ac2c0114959, est: 18, f: hero_space::up_component },
    Hand { entry: 0x226c60, end: 0x226ce0, hash: 0xfeaaf58e2fc83c11, est: 21, f: hero_space::set_up_component },
    Hand { entry: 0x226ce0, end: 0x226d6c, hash: 0x8c48fad7ae13d0fb, est: 20, f: hero_space::set_length_xy_up },
    Hand { entry: 0x226d70, end: 0x226e70, hash: 0x5c1168ed0f9ae265, est: 44, f: hero_space::fall_height },
    Hand { entry: 0x226e70, end: 0x226ef8, hash: 0xa56cc3957f80eeb3, est: 19, f: hero_space::pitch_of },
    Hand { entry: 0x226ef8, end: 0x226f7c, hash: 0xc9a00c4eb4505d55, est: 29, f: ammo_flags::consume_ammo },
    Hand { entry: 0x226f80, end: 0x226fd8, hash: 0x1b261223f12792e0, est: 20, f: ammo_flags::add_ammo },
    Hand { entry: 0x226fd8, end: 0x227028, hash: 0x1f56a23924713275, est: 18, f: actor_tick::slot_record },
    Hand { entry: 0x227028, end: 0x227074, hash: 0xc3d3836b588a8203, est: 19, f: hero_pad_walk::pad_walk_begin },
    Hand { entry: 0x227078, end: 0x2270c0, hash: 0x2de229288e8b5f72, est: 7, f: hero_pad_walk::pad_walk_status },
    Hand { entry: 0x227948, end: 0x227b90, hash: 0x35a0de5b9f995960, est: 23, f: glow::glow_pulse_update },
    Hand { entry: 0x227b90, end: 0x227c78, hash: 0x3f42441aefe545e2, est: 11, f: actor_tick::hero_frame_tick },
    Hand { entry: 0x227c78, end: 0x227db4, hash: 0x5f0fee5fd94d0b34, est: 33, f: layer_scale::layer_scale_update },
    Hand { entry: 0x227e80, end: 0x227f3c, hash: 0x430a233ec9df428e, est: 47, f: hero_pad_walk::pad_walk_tick },
    Hand { entry: 0x227f40, end: 0x227f84, hash: 0xdfd3f9d305b28328, est: 17, f: layer_scale::hud_slot_request_tail },
    Hand { entry: 0x227f88, end: 0x227fd4, hash: 0x861909394f85a246, est: 382, f: hud_slots::hud_icon_index },
    Hand { entry: 0x227fd8, end: 0x22810c, hash: 0xc711e10289e75fcb, est: 274, f: hud_slot_init::hud_slot_table_init },
    Hand { entry: 0x2282f0, end: 0x228450, hash: 0xb08a247bb6fd1e27, est: 935, f: slot_ranges::mark_slot_ranges },
    Hand { entry: 0x228450, end: 0x2285cc, hash: 0xa80650d534b794f2, est: 1102, f: slot_fixup::slot_range_fixup },
    Hand { entry: 0x2285d0, end: 0x228708, hash: 0x93ef7f79000ef942, est: 2073, f: layer_masks::build_layer_masks },
    Hand { entry: 0x228730, end: 0x2287a4, hash: 0x286d5ee8f58a7915, est: 27, f: hud_pool::pool_grab },
    Hand { entry: 0x2287a8, end: 0x22883c, hash: 0x20d0bc8f56f46449, est: 37, f: layer_masks::swap_camera_bank },
    Hand { entry: 0x228840, end: 0x22894c, hash: 0xb92700230cb954f1, est: 35, f: hud_slots::hud_slot_request },
    Hand { entry: 0x228950, end: 0x2289b8, hash: 0x4d0647952cd8d28f, est: 24, f: hud_slot_ops::hud_slot_commit },
    Hand { entry: 0x2289b8, end: 0x228a48, hash: 0xbefa17e33abed2b2, est: 240, f: hud_slot_ops::hud_slots_clear },
    Hand { entry: 0x228a48, end: 0x228ac8, hash: 0xc8acd824865d8784, est: 43, f: hud_slot_ops::hud_slot_close },
    Hand { entry: 0x228ac8, end: 0x228b30, hash: 0x405497474535caa6, est: 26, f: hud_slot_ops::hud_icon_bind },
    Hand { entry: 0x228b38, end: 0x228bac, hash: 0xfb7b5b46fcbce553, est: 20, f: hud_slots::hud_slot_release },
    Hand { entry: 0x228bb0, end: 0x228c18, hash: 0xc2d6486b04dda473, est: 18, f: hud_counter::counter_align },
    Hand { entry: 0x228dc0, end: 0x228ebc, hash: 0x23f1620fd8e7cf6e, est: 51, f: hud_counter::counter_size },
    Hand { entry: 0x228ec0, end: 0x228f00, hash: 0x8c8340724d5dbac9, est: 16, f: hud_pool::hud_slot_init },
    Hand { entry: 0x228f58, end: 0x228fe0, hash: 0x93bb9ef983b001a9, est: 108, f: hud_slots::hud_slot_extend },
    Hand { entry: 0x228fe0, end: 0x229264, hash: 0x726a949b97ae8aa, est: 42, f: hud_bolts::bolts_counter_update },
    Hand { entry: 0x229268, end: 0x22939c, hash: 0xbb41b6a0bf72990d, est: 46, f: hud_snap_counter::hud_counter_update_snap },
    Hand { entry: 0x229e40, end: 0x229f8c, hash: 0x74eaad805d090650, est: 1827, f: bubble_bounce::bubble_bounce_step },
    Hand { entry: 0x22a2d0, end: 0x22a31c, hash: 0x5e4d1fec708e839, est: 19, f: hud_counter::slot_init },
    Hand { entry: 0x22a320, end: 0x22a33c, hash: 0x2c5040ff2f207907, est: 7, f: hud_bolts::bolts_counter_update_tail },
    Hand { entry: 0x22a858, end: 0x22a8d8, hash: 0x7cd260a91538d5b8, est: 32, f: hud_bolts::hud_name_set },
    Hand { entry: 0x22a8d8, end: 0x22a8f4, hash: 0x51dbe3a4a97f00cf, est: 7, f: bubble_bounce::passthrough_228ec0 },
    Hand { entry: 0x22a8f8, end: 0x22a97c, hash: 0x2bf9631fbb3924a8, est: 22, f: counter_hud::counter_hud_tick },
    Hand { entry: 0x22ac28, end: 0x22ace0, hash: 0xcef9ca4dfca317f2, est: 116, f: counter_setup::table_init },
    Hand { entry: 0x22c6f0, end: 0x22c770, hash: 0xafddbc3f21a4e7da, est: 32, f: counter_setup::bar_init },
    Hand { entry: 0x22ce08, end: 0x22ce40, hash: 0x99979a3b5318aec6, est: 14, f: counter_setup::bolts_init },
    Hand { entry: 0x22ce40, end: 0x22cf38, hash: 0x929207496666183a, est: 39, f: hud_bolts::bolts_spin_update },
    Hand { entry: 0x22d8a8, end: 0x22d8ec, hash: 0x6f895eaf90fda4f1, est: 17, f: counter_setup::counter_init },
    Hand { entry: 0x22dbf0, end: 0x22ddc0, hash: 0xec36b9eeb2a85514, est: 33, f: hud_widget::icon_anim },
    Hand { entry: 0x22ddc0, end: 0x22defc, hash: 0x1bc1c8780c20cfc5, est: 308, f: hud_ammo::hud_slots_tick },
    Hand { entry: 0x22e040, end: 0x22e078, hash: 0x2f16bfd41fe4456f, est: 14, f: hud_suppress::hud_reset_suppress },
    Hand { entry: 0x22e078, end: 0x22e090, hash: 0x85038c3e8ff59030, est: 6, f: hud_suppress::hud_unsuppress },
    Hand { entry: 0x22e400, end: 0x22e620, hash: 0x23d6705e88bceb50, est: 93, f: quad_pair::index_pass },
    Hand { entry: 0x22e620, end: 0x22e808, hash: 0x54294727745c36be, est: 122, f: quad_pair::quad_pair },
    Hand { entry: 0x22f8f8, end: 0x22fa68, hash: 0x76616cd8cd8146dc, est: 86, f: hud_suppress::hud_packet_emit },
    Hand { entry: 0x22fa68, end: 0x22fbec, hash: 0x5e807046952f8ee, est: 70, f: gif_quad_pkt::gif_quad_pkt },
    Hand { entry: 0x22fbf0, end: 0x22fd74, hash: 0x282532e8a48faa4d, est: 70, f: gif_rect_pkt::gif_rect_pkt },
    Hand { entry: 0x22fd78, end: 0x22fef8, hash: 0xc397ac9797fde8eb, est: 72, f: xy_packet::gif_xy_packet },
    Hand { entry: 0x22fff0, end: 0x2300c8, hash: 0xf63a3dd6f22cb89c, est: 54, f: quad_block::quad_block_pass },
    Hand { entry: 0x2300c8, end: 0x230170, hash: 0x21ac1d1b43763bf, est: 42, f: quad_block::quad_label_draw },
    Hand { entry: 0x230668, end: 0x2306e0, hash: 0x987a4d6a4e3047fb, est: 30, f: slot_setup::dma_block_setup },
    Hand { entry: 0x230ab0, end: 0x230b2c, hash: 0x3fad114b19b1781c, est: 29, f: hud_quad_gif::gs_mode_setup },
    Hand { entry: 0x230b30, end: 0x230b38, hash: 0xc84dc27f29a28935, est: 2, f: slot_setup::empty_fn },
    Hand { entry: 0x230b38, end: 0x230c90, hash: 0x6bbf4e4dec0eeea9, est: 76, f: slot_setup::claim_slot_color },
    Hand { entry: 0x230c90, end: 0x230d8c, hash: 0xbb09c57092bc1255, est: 53, f: record_pool::alloc_record },
    Hand { entry: 0x230d90, end: 0x230de8, hash: 0x12daaff0e9a27230, est: 22, f: record_pool::release_record },
    Hand { entry: 0x230de8, end: 0x230f68, hash: 0x893070e79f0ce988, est: 124, f: hud_quad_gif::slot_ring_update },
    Hand { entry: 0x231318, end: 0x231344, hash: 0x65fc4df92e4bd583, est: 11, f: swing_particle::swing_cam_pair },
    Hand { entry: 0x231b88, end: 0x231cc0, hash: 0x7153aac12f2bc2d3, est: 824, f: swing_particle::particle_tex_install },
    Hand { entry: 0x231cc0, end: 0x231d9c, hash: 0x1c07219380badbca, est: 157, f: texrec_class::fx_tex_table_build },
    Hand { entry: 0x232bb0, end: 0x232c08, hash: 0xba1084cf5695e09e, est: 695, f: record_pool::rebase_record },
    Hand { entry: 0x232f10, end: 0x232ffc, hash: 0x3c55fc340fecea1d, est: 52, f: texrec_class::moby_class_register },
    Hand { entry: 0x237710, end: 0x2377c8, hash: 0x9a1dc5cb2c50a6ac, est: 36, f: chunk_latch::chunk_step },
    Hand { entry: 0x237c78, end: 0x237cc8, hash: 0x8b1ca36b88aae7ad, est: 20, f: chunk_latch::chunk_reload },
    Hand { entry: 0x237cc8, end: 0x237f74, hash: 0xbcbe1a37b60cc1fb, est: 321, f: sndcache::slot_bind },
    Hand { entry: 0x237f78, end: 0x2380b8, hash: 0x24205732988e6470, est: 80, f: sample_ring::ring_push },
    Hand { entry: 0x2380b8, end: 0x23817c, hash: 0x702a69e543ba54c6, est: 49, f: slot_alloc::log_append },
    Hand { entry: 0x238fb8, end: 0x23901c, hash: 0x5cd59c2d42ab3ec2, est: 65, f: slot_alloc::find_free_slot },
    Hand { entry: 0x239020, end: 0x2390c0, hash: 0x27140c6075d65896, est: 12, f: slot_alloc::alloc_slot },
    Hand { entry: 0x2391a8, end: 0x2392e0, hash: 0x1db37501ffd4ea07, est: 92, f: slot_probe::probe_slot_alt },
    Hand { entry: 0x2392e0, end: 0x239334, hash: 0x2176626cc7b1f09a, est: 39, f: slot_probe::find_slot_index },
    Hand { entry: 0x239338, end: 0x239498, hash: 0x72bc1073cfc12390, est: 69, f: ctx_slots::pick_slot },
    Hand { entry: 0x239498, end: 0x2394fc, hash: 0x34a7f3b6acfb5b62, est: 3188785, f: ctx_slots::mask_select_rows },
    Hand { entry: 0x23a7d0, end: 0x23a920, hash: 0x26251eea21079d68, est: 4225540, f: nibblepack::pack_nibble_bits },
    Hand { entry: 0x23c2d8, end: 0x23c3f8, hash: 0x3bce06c8b822d33e, est: 46, f: sndcache::cache_fetch },
    Hand { entry: 0x23c3f8, end: 0x23c4a0, hash: 0xb23c0f41e7ac19f0, est: 32, f: stage_pack::stage_pack_or_fill },
    Hand { entry: 0x23c4a0, end: 0x23c518, hash: 0x9e42a966770297b4, est: 27, f: stage_pack::stage_pack_entry },
    Hand { entry: 0x23cb70, end: 0x23ccec, hash: 0x65d6873edabcab6, est: 262, f: fmt_template::expand_text },
    Hand { entry: 0x23ccf0, end: 0x23cd3c, hash: 0x8abf0925e69e9503, est: 19, f: stage_dispatch::stage_slot_store },
    Hand { entry: 0x23cd40, end: 0x23ce38, hash: 0x95df16ab33db5307, est: 38, f: lerp_pair::blend_pair },
    Hand { entry: 0x23d0a8, end: 0x23d144, hash: 0xa8744e3720715b58, est: 62, f: row_records::pair_table_copy },
    Hand { entry: 0x23d148, end: 0x23d150, hash: 0xc84dc27f29a28935, est: 2, f: stage_dispatch::nop_hook },
    Hand { entry: 0x23d178, end: 0x23d1dc, hash: 0xb29bb6267d01da4b, est: 23, f: lerp_pair::run_callback },
    Hand { entry: 0x23d860, end: 0x23d8d8, hash: 0xdcc5be3505c00a10, est: 12, f: row_records::level_flag_gate },
    Hand { entry: 0x23f3e8, end: 0x23f490, hash: 0xf7b205481173a4eb, est: 194, f: cancel_save::save_record_pack },
    Hand { entry: 0x23f4c0, end: 0x23f4f8, hash: 0x91113bc288776df8, est: 382, f: row_records::record_list_bytes },
    Hand { entry: 0x23f4f8, end: 0x23f56c, hash: 0x25a5526636e33fd5, est: 248053, f: row_records::crc_bytes },
    Hand { entry: 0x23f5b0, end: 0x23f698, hash: 0x21e3131d46a62933, est: 702, f: chunk_emit::chunk_emit },
    Hand { entry: 0x23fc18, end: 0x23fc50, hash: 0x9397acb6a9970456, est: 12, f: spr_stream::wait_spr_to_idle },
    Hand { entry: 0x23fce0, end: 0x23fe50, hash: 0xa5961fd3e36019c3, est: 385, f: spr_stream::emit_spr_packets },
    Hand { entry: 0x23fe50, end: 0x24013c, hash: 0x3841c03178e97918, est: 692898, f: spr_stream::spr_lzo_unpack },
    Hand { entry: 0x240120, end: 0x240140, hash: 0xe4fb37ba2252a1cd, est: 8, f: moby_init::kick_block },
    Hand { entry: 0x240140, end: 0x240168, hash: 0x1016dcff02e952de, est: 8, f: moby_init::wait_block_idle },
    Hand { entry: 0x240328, end: 0x240438, hash: 0x950e88cb220f6a45, est: 22, f: stage_dispatch::dispatch_flag },
    Hand { entry: 0x240438, end: 0x240564, hash: 0xf8c45913b9d6ce8, est: 102, f: record_list_walk::walk_records },
    Hand { entry: 0x240d28, end: 0x240db4, hash: 0xe787b3334965e47e, est: 3504, f: actor::count_reclaimable_mobys },
    Hand { entry: 0x240db8, end: 0x240eb0, hash: 0xfb375cc211a7cd74, est: 412, f: actor::alloc_moby },
    Hand { entry: 0x240eb0, end: 0x2410e8, hash: 0x5781c90237429a2a, est: 118, f: moby_init::moby_init },
    Hand { entry: 0x2410e8, end: 0x241140, hash: 0xa88cd063c30414d9, est: 20, f: actor::free_moby },
    Hand { entry: 0x241140, end: 0x2411fc, hash: 0xfb5c3474251c248f, est: 36, f: actor::anim_load_tracks },
    Hand { entry: 0x241200, end: 0x241298, hash: 0xda4739bc40624198, est: 22, f: actor::actor_voice_sync },
    Hand { entry: 0x241298, end: 0x241340, hash: 0x110ddd0efee5ac8e, est: 64, f: actor::anim_find_key },
    Hand { entry: 0x241348, end: 0x24143c, hash: 0x16986ee839393567, est: 43, f: actor::anim_blend_value },
    Hand { entry: 0x241440, end: 0x24149c, hash: 0x96d5817ed694671d, est: 22, f: gate_frames::scale_table_value },
    Hand { entry: 0x2414a0, end: 0x24150c, hash: 0x47cb877573d1c9d7, est: 26, f: record_list_walk::slot_scaled },
    Hand { entry: 0x2417a0, end: 0x241878, hash: 0x299cab5401b79401, est: 54, f: actor::link_node_set },
    Hand { entry: 0x241878, end: 0x2418ec, hash: 0x309fa0e0a14f8705, est: 2325, f: layer_bake::shift_entry_offsets },
    Hand { entry: 0x2418f0, end: 0x2419b0, hash: 0x8160bb724d13f824, est: 39, f: actor::fx_slot_alloc },
    Hand { entry: 0x2419b0, end: 0x241a54, hash: 0xc1f328d4030df781, est: 21, f: actor::fx_slot_free },
    Hand { entry: 0x241ca8, end: 0x241d04, hash: 0x680e6f7b66613fcd, est: 23, f: actor::fx_slot_start },
    Hand { entry: 0x241d08, end: 0x241e58, hash: 0xa6cad00cffb2ef69, est: 254, f: layer_bake::bake_layer_frame },
    Hand { entry: 0x241e58, end: 0x241ed0, hash: 0x6a3e400a53ffb236, est: 30, f: actor::layer_attach },
    Hand { entry: 0x241ed0, end: 0x241f60, hash: 0x8a037453bbafb0d0, est: 33, f: actor::layer_detach },
    Hand { entry: 0x241f60, end: 0x241fa4, hash: 0x312783aed20b6bb4, est: 16, f: actor::active_add },
    Hand { entry: 0x241fa8, end: 0x241ff0, hash: 0xcc8f9d52f41f38dd, est: 109, f: actor::active_prune },
    Hand { entry: 0x241ff0, end: 0x242090, hash: 0x3168aa5e76148cc7, est: 40, f: actor::pose_node_world },
    Hand { entry: 0x242090, end: 0x242118, hash: 0xba865badeaa2ec0e, est: 34, f: actor::pose_node_pos },
    Hand { entry: 0x242118, end: 0x2421b8, hash: 0xd4d70d704e0dea9f, est: 115, f: actor::pose_chain_pos },
    Hand { entry: 0x2421b8, end: 0x24228c, hash: 0x31d0042dd9732906, est: 181, f: actor::pose_chain_world },
    Hand { entry: 0x242290, end: 0x242398, hash: 0x6574b16bc84e919e, est: 66, f: frame::frame_list_begin },
    Hand { entry: 0x242398, end: 0x242470, hash: 0x3813072594be3f69, est: 1404, f: actor::patch_draw_records },
    Hand { entry: 0x242470, end: 0x242500, hash: 0x05f93ead3301ad30, est: 36, f: actor::light_dir_update },
    Hand { entry: 0x242500, end: 0x242644, hash: 0x2c0aaa9cbdfbfc90, est: 67, f: frame::frame_list_second },
    Hand { entry: 0x242648, end: 0x24268c, hash: 0xa4cec867bb077950, est: 17, f: frame::spr_reload },
    Hand { entry: 0x242690, end: 0x2426b8, hash: 0xf4a764273e258773, est: 10, f: frame::spr_clear },
    Hand { entry: 0x2426b8, end: 0x2426e4, hash: 0xe199eaf8a5505aee, est: 11, f: menu_moby::moby_scratch_restore },
    Hand { entry: 0x2426e8, end: 0x242714, hash: 0x630875bcbcc85aee, est: 11, f: menu_moby::moby_scratch_save },
    Hand { entry: 0x242718, end: 0x2427d0, hash: 0xf1141edd498f3fad, est: 46, f: frame::frame_state_init },
    Hand { entry: 0x242850, end: 0x2428fc, hash: 0xf1a81c81a3fb5b0f, est: 42, f: frame::frame_lists_build },
    Hand { entry: 0x242900, end: 0x242980, hash: 0x20cddf1134e3bc6a, est: 29, f: frame::frame_setup },
    Hand { entry: 0x242b68, end: 0x242b94, hash: 0x88eb2da4dea8ede2, est: 5, f: menu_moby::set_slot_byte },
    Hand { entry: 0x242b98, end: 0x242bb8, hash: 0xfbada15ca3f88ac0, est: 8, f: packed_vu::pack_word_low },
    Hand { entry: 0x242bb8, end: 0x242be8, hash: 0xfa2283efbdd55848, est: 12, f: packed_vu::pack_word_high },
    Hand { entry: 0x242be8, end: 0x242c18, hash: 0x79e403e9b203b006, est: 12, f: packed_vu::unpack_bytes },
    Hand { entry: 0x242c18, end: 0x242c88, hash: 0x718a18aad2362540, est: 16, f: actor::pivot_blend_hand },
    Hand { entry: 0x242cf8, end: 0x242d48, hash: 0x42b53d30a3be276a, est: 20, f: packed_vu::vu_gap_len },
    Hand { entry: 0x243308, end: 0x243368, hash: 0x7746e8f09dc3dd41, est: 20, f: cells::free_blocks_hand },
    Hand { entry: 0x243368, end: 0x2433e8, hash: 0x66cd1c7e5d2f7e79, est: 145, f: cells::alloc_blocks_hand },
    Hand { entry: 0x2433e8, end: 0x2436c0, hash: 0xa637e77c0dc95a3c, est: 113, f: cells::actor_refile_cells },
    Hand { entry: 0x2436c0, end: 0x243860, hash: 0x1b13844937641088, est: 57, f: moby::actor_transform_bsphere },
    Hand { entry: 0x243860, end: 0x2438a8, hash: 0xb672dcac38a66f03, est: 16, f: moby_anim_bsphere::bsphere_stored_rows },
    Hand { entry: 0x246710, end: 0x246864, hash: 0x1f3da3543fa8a304, est: 653, f: gather::gather_part_rows },
    Hand { entry: 0x247e88, end: 0x248138, hash: 0xf396517d507e8224, est: 4378, f: blocks::build_lod_blocks },
    Hand { entry: 0x248138, end: 0x2481a8, hash: 0x545f72d80cc59b96, est: 1134, f: blocks::fill_slot_words },
    Hand { entry: 0x2496a8, end: 0x2497dc, hash: 0x67feb341bbcf9a18, est: 41, f: spheres::flat_zone_hit },
    Hand { entry: 0x2497dc, end: 0x2498ec, hash: 0xdff3d05d086fa788, est: 26, f: spheres::grid_zone_lookup },
    Hand { entry: 0x2498ec, end: 0x249adc, hash: 0xbcb3d5fbd00cea6f, est: 20, f: spheres::zone_tint },
    Hand { entry: 0x24a090, end: 0x24a148, hash: 0xf0ff07a39b1ed4f0, est: 45, f: moby_anim_bsphere::set_moby_animation },
    Hand { entry: 0x24a148, end: 0x24a290, hash: 0xc99e35a530b6901e, est: 68, f: objects::anim_start },
    Hand { entry: 0x24a290, end: 0x24a3e4, hash: 0x473562b88f855efe, est: 80, f: spawn_curve::spawn_init },
    Hand { entry: 0x24a418, end: 0x24a454, hash: 0xe9b0d90e2f21ca7c, est: 14, f: rand::rand_below },
    Hand { entry: 0x24a458, end: 0x24a4ac, hash: 0x4bef50b20308c0a8, est: 20, f: rand::rand_int },
    Hand { entry: 0x24a4b0, end: 0x24a50c, hash: 0xfb4623e0aa1f2d32, est: 23, f: rand::rand_float },
    Hand { entry: 0x24a510, end: 0x24a578, hash: 0x6df261642d0f3e4, est: 25, f: rand::rand_float_signed },
    Hand { entry: 0x24a578, end: 0x24a5c8, hash: 0xbee87aff80caf06f, est: 20, f: rand::rand_angle },
    Hand { entry: 0x24a5c8, end: 0x24a640, hash: 0x9046d5f1cd2e44bf, est: 30, f: rand::rand_polar_vec },
    Hand { entry: 0x24a640, end: 0x24a6e8, hash: 0x7270e3b763d7678c, est: 42, f: rand::rand_sphere_vec },
    Hand { entry: 0x24a6e8, end: 0x24a720, hash: 0x2b6d81adb9efe314, est: 14, f: spawn_curve::curve_eval },
    Hand { entry: 0x24a720, end: 0x24a7b4, hash: 0x75f51a0346e9dd13, est: 34, f: rand::ease_cos },
    Hand { entry: 0x24a7b8, end: 0x24a834, hash: 0x9ad7ed41f2b2189a, est: 31, f: angle_mix::angle_rot },
    Hand { entry: 0x24a838, end: 0x24a918, hash: 0xbb1c192d0e2e6fc4, est: 48, f: angle_mix::angle_blend },
    Hand { entry: 0x24a918, end: 0x24a9d8, hash: 0x94072e3f7cc50237, est: 48, f: walker_orbit::orbit_blend },
    Hand { entry: 0x24a9d8, end: 0x24ab40, hash: 0x82644a3c9fe5f49, est: 75, f: motion::angle_spring_to },
    Hand { entry: 0x24ab40, end: 0x24aca8, hash: 0x211c22e25669b8e4, est: 74, f: motion::angle_spring_field },
    Hand { entry: 0x24acb8, end: 0x24ad58, hash: 0x165bc0b293df9440, est: 37, f: grid_probe_pair::probe_clamped },
    Hand { entry: 0x24ad58, end: 0x24b0ec, hash: 0xb85e700ba5e61fa9, est: 136, f: sweep::ground_follow },
    Hand { entry: 0x24b398, end: 0x24b414, hash: 0x9e1036d257d39f14, est: 31, f: grid_probe_pair::probe_scaled_pair },
    Hand { entry: 0x24b418, end: 0x24b490, hash: 0x6bd8a7a66cd4b6eb, est: 30, f: walker_orbit::walker_init },
    Hand { entry: 0x24b968, end: 0x24badc, hash: 0x3133e16c9d5d23e, est: 87, f: walker_turn::walker_turn },
    Hand { entry: 0x24bb10, end: 0x24bbf8, hash: 0x1363db8d6b1aae04, est: 41, f: slot_table::lookup_slot },
    Hand { entry: 0x24bbf8, end: 0x24bd5c, hash: 0x1ea5dc3ad1bb704b, est: 38, f: record_stream::next_record },
    Hand { entry: 0x24bda8, end: 0x24be48, hash: 0x2a5453c94b011e28, est: 39, f: ballistic_kit::arc_apex_z },
    Hand { entry: 0x24be48, end: 0x24bee0, hash: 0x645d6a6f9607e5dc, est: 38, f: geom::ballistic_pos },
    Hand { entry: 0x24bee0, end: 0x24bfd4, hash: 0x13e57d759da9054b, est: 55, f: geom::solve_quadratic },
    Hand { entry: 0x24bfd8, end: 0x24c04c, hash: 0xf1f103b7f101938, est: 26, f: sweep::ground_height },
    Hand { entry: 0x24c050, end: 0x24c074, hash: 0xc0fe5f0874bbd651, est: 9, f: sweep::ground_height_here },
    Hand { entry: 0x24c120, end: 0x24c1ec, hash: 0xe0d200cb4a8db12b, est: 411, f: geom::point_in_polygon_xy },
    Hand { entry: 0x24c490, end: 0x24c4b4, hash: 0x37b7eca9b1ad912d, est: 8, f: objects::link_active },
    Hand { entry: 0x24c4b8, end: 0x24c4e4, hash: 0xfae71e61f5065358, est: 11, f: ballistic_kit::set_ballistic_fields },
    Hand { entry: 0x24c4e8, end: 0x24c50c, hash: 0x99b246eb1f97a534, est: 9, f: objects::probe_init },
    Hand { entry: 0x24c510, end: 0x24c644, hash: 0xfa91b598c44d0a19, est: 72, f: probe_cast::probe_cast },
    Hand { entry: 0x24c648, end: 0x24c784, hash: 0x44c2e4cb8712bde3, est: 66, f: hit_ring::hit_ring_push },
    Hand { entry: 0x24c788, end: 0x24c8c0, hash: 0xee2cc2c203f7be41, est: 65, f: hit_point_write::hit_write_point },
    Hand { entry: 0x24c8c8, end: 0x24ca14, hash: 0xe575d46ef71a0f92, est: 189, f: sweep::sight_sweep },
    Hand { entry: 0x24cb10, end: 0x24cba4, hash: 0xb17f0e454004047e, est: 37, f: geom::euler_to_quat },
    Hand { entry: 0x24cba8, end: 0x24ccd0, hash: 0xfdcda591cdeb85a3, est: 32, f: shadow_slot_queue::shadow_queue_push },
    Hand { entry: 0x24cce0, end: 0x24cda0, hash: 0x43f2c6f6f8e7a38e, est: 45, f: sweep::ledge_probe },
    Hand { entry: 0x24cfc0, end: 0x24cfdc, hash: 0x768f3999c765977b, est: 7, f: vecmath::color_lerp_tail },
    Hand { entry: 0x24cfe0, end: 0x24d038, hash: 0x898f4d04b7bb78b4, est: 6, f: objects::link_record },
    Hand { entry: 0x24d5b8, end: 0x24d704, hash: 0x4a9f8fa3fb5e3a55, est: 72, f: blast_impulse::blast_impulse_list },
    Hand { entry: 0x24d708, end: 0x24d7b0, hash: 0x400c7ca8a5e0c63, est: 37, f: atan_swap::angle_scale },
    Hand { entry: 0x24d7b0, end: 0x24d860, hash: 0x61f2cc9881368d83, est: 43, f: geom::ballistic_aim },
    Hand { entry: 0x24e268, end: 0x24e278, hash: 0xc7650cdb1d944372, est: 4, f: atan_swap::lerp_scalar },
    Hand { entry: 0x24e278, end: 0x24e310, hash: 0x86d445cb25e1f4bc, est: 38, f: atan_swap::lerp_vec4 },
    Hand { entry: 0x24e310, end: 0x24e354, hash: 0x1880b69a36763281, est: 17, f: shadow_slot_queue::angle_fold },
    Hand { entry: 0x24e358, end: 0x24e428, hash: 0xed5d23a7cc8a58ef, est: 34, f: motion::spring_vel_step },
    Hand { entry: 0x24e428, end: 0x24e47c, hash: 0x57d4efff7022879b, est: 19, f: motion::slew_toward },
    Hand { entry: 0x24e480, end: 0x24e52c, hash: 0x19214846c4d2fbc1, est: 40, f: motion::spring_settle_hand },
    Hand { entry: 0x24e6f8, end: 0x24e7c0, hash: 0x5ffc6d506b0133e8, est: 17, f: motion::angle_diff_dir_hand },
    Hand { entry: 0x24e7c0, end: 0x24e858, hash: 0xc117ba7db4cc8b83, est: 35, f: motion::angle_step_toward_hand },
    Hand { entry: 0x24e858, end: 0x24e9bc, hash: 0x671d3649c66e650d, est: 63, f: motion::turn_spring_angle },
    Hand { entry: 0x24e9c0, end: 0x24eb98, hash: 0x67e78a95b9a76c96, est: 72, f: motion::angle_brake_to },
    Hand { entry: 0x24eba0, end: 0x24ec44, hash: 0xffdb297063d421ea, est: 41, f: slot_registry::pack_rgba_f },
    Hand { entry: 0x24ec48, end: 0x24eca4, hash: 0x6169272325baa7c9, est: 9, f: atan_swap::swap_masked },
    Hand { entry: 0x24eca8, end: 0x24ed2c, hash: 0xdaf833e10c1e322, est: 10, f: colour_flag::colour_swap_channels },
    Hand { entry: 0x24ed30, end: 0x24edec, hash: 0x844293cbe15b19d0, est: 197, f: geom::mat3_normalize_columns },
    Hand { entry: 0x24eef8, end: 0x24ef28, hash: 0x769cdafe63fbdb1b, est: 10, f: objects::attached_value },
    Hand { entry: 0x24ef28, end: 0x24ef58, hash: 0x769ccafe63fbbfeb, est: 10, f: colour_flag::flag20_first_pvar },
    Hand { entry: 0x24f118, end: 0x24f254, hash: 0x72bd045e7666a879, est: 71, f: launch_block::launch_init },
    Hand { entry: 0x24fd28, end: 0x24fd74, hash: 0x8a7326c58e9c7f40, est: 19, f: launch_block::mat3_relay },
    Hand { entry: 0x24fd78, end: 0x24fd8c, hash: 0x54be956e4b4364b, est: 5, f: launch_block::copy_hero_tint },
    Hand { entry: 0x24fd90, end: 0x24fdf4, hash: 0xdb4c60a55501a341, est: 25, f: geom::axis_angle_quat },
    Hand { entry: 0x24fdf8, end: 0x24fef0, hash: 0x9684252c2bde6dd5, est: 79, f: geom::vec_pull_to_limit },
    Hand { entry: 0x24fef0, end: 0x250014, hash: 0x3d6a65aad2173083, est: 73, f: geom::mat4_to_euler },
    Hand { entry: 0x250018, end: 0x2500f8, hash: 0x1a54685f00ca6c2c, est: 31, f: pair_scale::scale_pair_step },
    Hand { entry: 0x2500f8, end: 0x250360, hash: 0x1c41f2566320e248, est: 19, f: path::colour_fade_step },
    Hand { entry: 0x250630, end: 0x2506c0, hash: 0xf06a34df4c692b6, est: 24, f: path::path_point_dist },
    Hand { entry: 0x250898, end: 0x250b28, hash: 0xae7b3e21860300e3, est: 143, f: path::path_segment_closest },
    Hand { entry: 0x250ed0, end: 0x250f24, hash: 0x5f5bbb8f532c68d0, est: 21, f: path::wrap_angle_pi },
    Hand { entry: 0x250f28, end: 0x250f74, hash: 0x45247d85b0dcf584, est: 15, f: pair_scale::class_flag },
    Hand { entry: 0x250f78, end: 0x250fb8, hash: 0xb0a50e0b4a159a9f, est: 14, f: path::is_class_500_540 },
    Hand { entry: 0x250fb8, end: 0x251010, hash: 0xd896e708e7f7b930, est: 20, f: path::is_special_class },
    Hand { entry: 0x2522f0, end: 0x25233c, hash: 0x4573ca1aa51e2da6, est: 19, f: path::vec_clamp_len },
    Hand { entry: 0x252340, end: 0x2523bc, hash: 0xea56feb1fcd57662, est: 23, f: slot_registry::rocket_registry_insert },
    Hand { entry: 0x2523c0, end: 0x252434, hash: 0xdae9942225207957, est: 21, f: slot_registry::rocket_registry_remove },
    Hand { entry: 0x252438, end: 0x252498, hash: 0xf2dc5a821b161da7, est: 10, f: orient_step::rocket_find },
    Hand { entry: 0x2524a0, end: 0x252520, hash: 0xfb7a586c8f06c435, est: 32, f: ballistic_kit::jitter_pos },
    Hand { entry: 0x252520, end: 0x2525f8, hash: 0x696195a2cb40f90d, est: 33, f: path::point_in_obb },
    Hand { entry: 0x252738, end: 0x2527c4, hash: 0x1b30723314d13e49, est: 35, f: slot_bits::rotate_vec_quat },
    Hand { entry: 0x252870, end: 0x2529a4, hash: 0x3f28317f1eace46a, est: 61, f: orient_step::orient_step },
    Hand { entry: 0x2529a8, end: 0x252c28, hash: 0xebf39ccd79365051, est: 192, f: targets::pick_lock_target },
    Hand { entry: 0x252c38, end: 0x252c68, hash: 0xac319422ac981a2e, est: 8, f: targets::actor_ground_height },
    Hand { entry: 0x252d58, end: 0x252ecc, hash: 0xb59f348cdc2190cd, est: 79, f: targets::attach_point_place },
    Hand { entry: 0x252fa0, end: 0x253034, hash: 0x16bfd8300f3ebbc9, est: 37, f: targets::compose_orientation },
    Hand { entry: 0x253038, end: 0x253078, hash: 0x68a83ebe3f05ff88, est: 16, f: targets::sphere_visible_scaled },
    Hand { entry: 0x253078, end: 0x253108, hash: 0xb9753b74dea36e70, est: 13, f: level_motion::unlock_planet },
    Hand { entry: 0x253d28, end: 0x253e50, hash: 0x2da7538a8d20629a, est: 72, f: axis_probe::axis_index_store },
    Hand { entry: 0x253e50, end: 0x253ed4, hash: 0xc6c1274481c29da8, est: 33, f: axis_probe::probe_scale_vec },
    Hand { entry: 0x254100, end: 0x25417c, hash: 0x2411237a4ec93bea, est: 52, f: layers::round_decimals_hand },
    Hand { entry: 0x254180, end: 0x25420c, hash: 0x2f8d8d2b6f52a794, est: 34, f: layers::anim_blend_not_past },
    Hand { entry: 0x2548a0, end: 0x25491c, hash: 0xd822adc5f0f25a05, est: 31, f: layers::euler_rotate },
    Hand { entry: 0x254920, end: 0x254948, hash: 0x6d5d03e906e4e6c, est: 10, f: trail::trail_start },
    Hand { entry: 0x254948, end: 0x254a28, hash: 0xe3a15b97d8296e, est: 55, f: trail::trail_add_ghost },
    Hand { entry: 0x254a28, end: 0x254c60, hash: 0xff43d490b1cf637b, est: 158, f: trail::trail_update },
    Hand { entry: 0x254c60, end: 0x254ce0, hash: 0x87c653bf1ea30ab8, est: 15, f: trail::trail_free },
    Hand { entry: 0x254ce0, end: 0x254d44, hash: 0x3c8b9579c3fe331f, est: 25, f: ballistic_kit::launch_block },
    Hand { entry: 0x254d58, end: 0x254f80, hash: 0x499c4404b1bac6fa, est: 83, f: layers::layer_spring_update },
    Hand { entry: 0x254f80, end: 0x254ffc, hash: 0xa0527ba5ea6529bb, est: 31, f: level_motion::bob_z },
    Hand { entry: 0x255000, end: 0x2550c4, hash: 0x2f5bea02ddb0c730, est: 49, f: level_motion::wobble_euler },
    Hand { entry: 0x2550d0, end: 0x25516c, hash: 0xead11cceb12668c, est: 39, f: scene::spherical_to_xyz },
    Hand { entry: 0x255f48, end: 0x255fb8, hash: 0xddf68bc49a365a30, est: 16, f: scene::layer_set_scale },
    Hand { entry: 0x255fb8, end: 0x255fe0, hash: 0xc1b1eb864a945a59, est: 8, f: scene::layer_scale_factor },
    Hand { entry: 0x255ff0, end: 0x256060, hash: 0x88ed186c395fbe19, est: 28, f: bolt_prompts::flower_near_bolt_drop },
    Hand { entry: 0x256060, end: 0x2560cc, hash: 0x6775970697d144ba, est: 26, f: bolt_prompts::timed_text_popup },
    Hand { entry: 0x2560d0, end: 0x25614c, hash: 0xc37affee7f9b6232, est: 31, f: cue_tables::store_slot },
    Hand { entry: 0x2566d8, end: 0x256774, hash: 0x70f8b8e5c4aa8b40, est: 15, f: scene::fx_request_tick },
    Hand { entry: 0x256778, end: 0x25681c, hash: 0xd0d34b06482243f1, est: 25, f: scene::fx_request },
    Hand { entry: 0x256820, end: 0x256890, hash: 0x431be0fa1a22758c, est: 16, f: bolt_prompts::force_prompt_slot },
    Hand { entry: 0x256890, end: 0x2568f0, hash: 0x10fd23d38b7f1cb5, est: 23, f: hud_fx_pool::hud_slot_gate },
    Hand { entry: 0x2568f0, end: 0x256938, hash: 0x8fcbc4178a5dba50, est: 18, f: bolt_prompts::spare_gold_bolts },
    Hand { entry: 0x256938, end: 0x2569a8, hash: 0x588c0f98a3df8b49, est: 732, f: planet_gold::gold_bolts_found },
    Hand { entry: 0x2569a8, end: 0x2569f0, hash: 0x441243feb4131553, est: 270, f: cue_tables::gold_bolts_spent },
    Hand { entry: 0x2569f0, end: 0x256a38, hash: 0xafc9851d2f2d20c6, est: 235, f: cue_tables::shift_count },
    Hand { entry: 0x256a38, end: 0x256aec, hash: 0x7722b00c20508471, est: 1210, f: planet_gold::record_link },
    Hand { entry: 0x256f08, end: 0x256fdc, hash: 0xc5b9e8079c03fd3f, est: 53, f: slot_spawn::spawn_slot_a },
    Hand { entry: 0x256fe0, end: 0x2570d8, hash: 0x9d336186a266195d, est: 61, f: slot_spawn::spawn_slot_b },
    Hand { entry: 0x257418, end: 0x257470, hash: 0x77f609fc15862aa8, est: 21, f: slot_claim::poll_slot },
    Hand { entry: 0x257548, end: 0x25761c, hash: 0x8d109f12f4c1480e, est: 53, f: slot_claim::claim_slot },
    Hand { entry: 0x257620, end: 0x257704, hash: 0xb989ef1cdc126b3f, est: 57, f: slot_start::slot_start },
    Hand { entry: 0x257708, end: 0x2577e8, hash: 0x4f0bcfb0dd43dcea, est: 55, f: stream_ctl::stream_start },
    Hand { entry: 0x2577e8, end: 0x25783c, hash: 0x5782f0bb7e890ee2, est: 12, f: stream_ctl::stream_slot_set },
    Hand { entry: 0x2579a8, end: 0x2579dc, hash: 0xb89ff54fc6789c1f, est: 9, f: rec_reset::set_pair_limits },
    Hand { entry: 0x2579e0, end: 0x2579fc, hash: 0xc77cdf7dcadba358, est: 7, f: rec_reset::set_four },
    Hand { entry: 0x257a00, end: 0x257a40, hash: 0xb7aada4f1a2676cf, est: 15, f: stream_ctl::stream_hold_timers_set },
    Hand { entry: 0x258080, end: 0x2580c0, hash: 0x996ef7a1f662ba79, est: 11, f: stream_ctl::stream_kick },
    Hand { entry: 0x2580c0, end: 0x258120, hash: 0x83db8ee90c4974d1, est: 24, f: emit_record::record_call },
    Hand { entry: 0x258120, end: 0x2581c0, hash: 0x4ac1252a93f53e34, est: 34, f: cam_latch::cam_latch_request },
    Hand { entry: 0x258240, end: 0x2582e8, hash: 0xa8a388d7daea0076, est: 18, f: cam_latch::cam_latch_drain },
    Hand { entry: 0x2582e8, end: 0x258328, hash: 0x439c63ba7e446b55, est: 15, f: slot_latch::probe_latch },
    Hand { entry: 0x258358, end: 0x2583b4, hash: 0xef6dcd91d501263b, est: 16, f: slot_latch::store_mark },
    Hand { entry: 0x258500, end: 0x258558, hash: 0x87539af700926c91, est: 15, f: particles::set_once },
    Hand { entry: 0x2585c8, end: 0x258950, hash: 0xc424b40ecdf0c9ec, est: 46, f: particles::npc_talk },
    Hand { entry: 0x258950, end: 0x2589d4, hash: 0x34a00324fb6db07e, est: 39, f: slot_latch::find_index },
    Hand { entry: 0x258a20, end: 0x258aec, hash: 0x68e5935f59e79161, est: 43, f: talk_pad_scan::npc_talk_init },
    Hand { entry: 0x258ec8, end: 0x258edc, hash: 0x463d694bcc429055, est: 5, f: talk_pad_scan::pad_history_reset },
    Hand { entry: 0x258ee0, end: 0x258f80, hash: 0x20f324143ba0aa9b, est: 157, f: particles::input_history_scan },
    Hand { entry: 0x259000, end: 0x259044, hash: 0x8b8a4370ef2a8e09, est: 16, f: hud_fx_pool::ring_sample_pick },
    Hand { entry: 0x2590d0, end: 0x2592c4, hash: 0x3bab310a0eaf3933, est: 33, f: particles::pad_update },
    Hand { entry: 0x259a18, end: 0x259a38, hash: 0x6feb54be066a44fd, est: 8, f: particles::pad_scan_default },
    Hand { entry: 0x259a38, end: 0x259a40, hash: 0xea7ff04f4a8a8fd, est: 2, f: talk_pad_scan::particle_alloc_mode0 },
    Hand { entry: 0x259a40, end: 0x259b2c, hash: 0x2b2396a90e4c820c, est: 103, f: pool_alloc::particle_alloc },
    Hand { entry: 0x259b88, end: 0x259c8c, hash: 0x6d031575b19b945a, est: 34, f: slot_bits::slot_release },
    Hand { entry: 0x25aa0c, end: 0x25aa7c, hash: 0xd7a7a3ba8d8a85dd, est: 28, f: particles::pool_tex_upload_tag },
    Hand { entry: 0x25bad8, end: 0x25bbec, hash: 0x91069ff4c26f6fa2, est: 69, f: smoke_spawn::part_type04_spawn },
    Hand { entry: 0x25bcf0, end: 0x25bdec, hash: 0x7223d4820b34e68a, est: 63, f: hud_fx_pool::particle_flash_spawn },
    Hand { entry: 0x25bdf0, end: 0x25bea0, hash: 0xdfa7d93191a068c2, est: 26, f: flash5_update::flash5_update },
    Hand { entry: 0x25c648, end: 0x25c734, hash: 0x497a807c0ba00872, est: 59, f: particle_spawn::particle_spawn },
    Hand { entry: 0x25da30, end: 0x25dbb0, hash: 0xbd34cea7cf0e474f, est: 94, f: fluff_ctor::fluff_ctor },
    Hand { entry: 0x25dbb0, end: 0x25dd08, hash: 0x37348c88c7dd30a4, est: 51, f: pair_update::pair_update },
    Hand { entry: 0x25de10, end: 0x25df44, hash: 0x4934eb0433ffff64, est: 69, f: ember_spawn::spawn_ember15 },
    Hand { entry: 0x25e670, end: 0x25e7f4, hash: 0x21dc2c899d6584a8, est: 97, f: puff_spawn::particle_spawn_puff },
    Hand { entry: 0x25e7f8, end: 0x25e984, hash: 0x97306567b6d1edbf, est: 84, f: suck_puff::puff_update },
    Hand { entry: 0x25e988, end: 0x25ea8c, hash: 0xb3cc0bd19f5be1b2, est: 65, f: streak_spawn::spawn_streak },
    Hand { entry: 0x25ea90, end: 0x25eafc, hash: 0x6a744b6bfb0bc963, est: 25, f: scene::particle_integrate },
    Hand { entry: 0x25ee50, end: 0x25ef50, hash: 0xbfa586a048d4eb96, est: 62, f: spawn_rec::spawn_record },
    Hand { entry: 0x25f170, end: 0x25f29c, hash: 0xbe45b41f5562dfc9, est: 75, f: smoke_loft::spawn_smoke_loft },
    Hand { entry: 0x25f838, end: 0x25f99c, hash: 0xe16e767d99e98156, est: 75, f: emit_record::spawn_particle },
    Hand { entry: 0x25fd40, end: 0x25fe80, hash: 0xa4f2c2dba8467c8a, est: 61, f: trail_ctor::trail_spawn },
    Hand { entry: 0x25fe80, end: 0x25ffbc, hash: 0xb4d41719ac959447, est: 39, f: slot_refresh::slot_refresh },
    Hand { entry: 0x25ffc0, end: 0x2600a0, hash: 0xaa085e9eff35f790, est: 56, f: fade_sprite::spawn_fade27 },
    Hand { entry: 0x2600a0, end: 0x260130, hash: 0xa7af45b66b318fca, est: 32, f: fade_sprite::fade_sprite_update },
    Hand { entry: 0x263ef0, end: 0x263fbc, hash: 0x144f24c03aa1b519, est: 51, f: spark_motion::spark_spawn },
    Hand { entry: 0x263fc0, end: 0x264058, hash: 0xae20877329208eaa, est: 36, f: spark_motion::spark_tick },
    Hand { entry: 0x264438, end: 0x2645c4, hash: 0x6aa2334997abac51, est: 94, f: mouse_burst::spawn_mouse_burst },
    Hand { entry: 0x2649b0, end: 0x264aac, hash: 0xe8b8965d3e2a46ca, est: 63, f: held_ribbon::held_ribbon_spawn },
    Hand { entry: 0x264ab0, end: 0x264c10, hash: 0xaa390fa4643bd042, est: 81, f: slot_jitter_step::slot_jitter_update },
    Hand { entry: 0x265340, end: 0x265460, hash: 0xd864daf3546e1990, est: 68, f: particles::spawn_particle_3b },
    Hand { entry: 0x265460, end: 0x2654b8, hash: 0x68f3369995731c54, est: 18, f: particles::particle_fall_update },
    Hand { entry: 0x265ae0, end: 0x265bd4, hash: 0x32e448b5a0b8bc01, est: 61, f: particles::spawn_orb_sparkle },
    Hand { entry: 0x265bd8, end: 0x265c80, hash: 0x1ffb47b62761c0d3, est: 29, f: rec_hands::blend_obj_step },
    Hand { entry: 0x267d20, end: 0x267eb0, hash: 0xb6b40c23b9420eb6, est: 91, f: homing_ray_spawn::spawn_ray_4e },
    Hand { entry: 0x268428, end: 0x268520, hash: 0xd5a499187a9f7b72, est: 2196, f: path_walk::path_init_lengths },
    Hand { entry: 0x268528, end: 0x2685f0, hash: 0x163aa8283c90d7f1, est: 1085, f: path_walk::path_nearest },
    Hand { entry: 0x268630, end: 0x2686a8, hash: 0x3798ce23cadffc00, est: 15, f: particles::ring_point },
    Hand { entry: 0x2686a8, end: 0x2686f8, hash: 0xa0b9e9cf88eb7424, est: 11, f: particles::ring_index },
    Hand { entry: 0x2686f8, end: 0x268744, hash: 0x283459362b2021ff, est: 14, f: particles::ring_advance },
    Hand { entry: 0x268748, end: 0x268900, hash: 0xc58b3302b47326dc, est: 105, f: particles::spline_point },
    Hand { entry: 0x268900, end: 0x268a0c, hash: 0xc65f89e96072ddfa, est: 67, f: particles::hermite_point },
    Hand { entry: 0x268d68, end: 0x268dd0, hash: 0xfde72d53dbf40b9, est: 26, f: init_chain::init_scale_block },
    Hand { entry: 0x26a698, end: 0x26a824, hash: 0xdfc46cbdf4ce8aa0, est: 50, f: menu_scroll::menu_scroll_cursor },
    Hand { entry: 0x26c628, end: 0x26c6e4, hash: 0x7887de7c4b17b38d, est: 24, f: init_chain::tick_pending },
    Hand { entry: 0x26cca8, end: 0x26cd44, hash: 0x2ef82b9387e75b57, est: 39, f: init_chain::build_record_tables },
    Hand { entry: 0x26d368, end: 0x26d388, hash: 0x937a3cc5280f9db5, est: 8, f: init_chain::run_init_step },
    Hand { entry: 0x26da80, end: 0x26db4c, hash: 0xe5b486e5d0a7afbe, est: 94, f: slot_flags::slot_flags_update },
    Hand { entry: 0x26db50, end: 0x26dbbc, hash: 0x9c4d352df64e0761, est: 21, f: slot_flags::slot_flags_finish },
    Hand { entry: 0x26dbc0, end: 0x26dd2c, hash: 0x5efab7ace6985f7c, est: 245, f: latch_step::latch_step },
    Hand { entry: 0x26e7e0, end: 0x26e81c, hash: 0xdf8ddc8d8a7ec554, est: 15, f: slot_clear::pause_slot_clear },
    Hand { entry: 0x26e820, end: 0x26e994, hash: 0x3169f8ef52448fcb, est: 37, f: obj_slot_attach::obj_slot_attach },
    Hand { entry: 0x26e998, end: 0x26ea7c, hash: 0x7f906c69b852b5e4, est: 30, f: fxpkt_steps::fx_pkt_submit },
    Hand { entry: 0x26ea80, end: 0x26eab8, hash: 0x3e8a542719c7960b, est: 14, f: fxpkt_steps::angle_nudge },
    Hand { entry: 0x26ee90, end: 0x26ef1c, hash: 0x475d4e6afd702eb7, est: 29, f: fxpkt_steps::fx_pkt_release },
    Hand { entry: 0x26ef20, end: 0x26f050, hash: 0x2c217e896a27b1cb, est: 69, f: orbit_place::orbit_place },
    Hand { entry: 0x26f2d0, end: 0x26f378, hash: 0x330ef0230e6498d6, est: 42, f: travel_moby::ship_travel_spawn },
    Hand { entry: 0x26f378, end: 0x26f3a8, hash: 0x8e579d19eea48eb, est: 12, f: travel_moby::ship_travel_release },
    Hand { entry: 0x26f9a8, end: 0x26f9e0, hash: 0x2706b61e798926ab, est: 14, f: travel_moby::ship_angle_step },
    Hand { entry: 0x26fbb8, end: 0x26fbf0, hash: 0x7eddeffe57dc559b, est: 14, f: orbit_place::orbit_setup },
    Hand { entry: 0x2704f8, end: 0x27056c, hash: 0xf12033a4d92e74ba, est: 28, f: marker_draw::sprite_gate },
    Hand { entry: 0x270608, end: 0x270654, hash: 0x553f64f1b585500a, est: 19, f: moby_release::obj_pair_fixup },
    Hand { entry: 0x270ed8, end: 0x27101c, hash: 0x146a2db1794caf90, est: 19, f: cam_latch_step::cam_latch_step },
    Hand { entry: 0x271020, end: 0x2710e0, hash: 0x1a113a6f67e0ad1c, est: 46, f: fx_frame::fx_frame_pass },
    Hand { entry: 0x2713b0, end: 0x2716b4, hash: 0x520e0d1887f21a62, est: 0, f: cutf_2713b0::f_2713b0 },
    Hand { entry: 0x271f50, end: 0x272084, hash: 0x9c7ec02b9f6158f, est: 326, f: hud_slot_rows::fill_hud_slot_rows },
    Hand { entry: 0x2721c0, end: 0x2721f8, hash: 0x3df7a0041bba6496, est: 14, f: hud_slot_rows::slot_pair_setup },
    Hand { entry: 0x2721f8, end: 0x2722d4, hash: 0x6f60d047e903d122, est: 20, f: flag_dispatch::dispatch_flags },
    Hand { entry: 0x272318, end: 0x272348, hash: 0x5338ac352e4b35c9, est: 12, f: flag_dispatch::release_link },
    Hand { entry: 0x2737e8, end: 0x273818, hash: 0xe49ebd58df905671, est: 12, f: pause_hud::release_store },
    Hand { entry: 0x274b48, end: 0x274bf8, hash: 0x4f3acf488cd813af, est: 219, f: moby_release::slot_table_refresh },
    Hand { entry: 0x275dc0, end: 0x275e04, hash: 0x35f4c36f28c01750, est: 13, f: moby_release::release_moby },
    Hand { entry: 0x276550, end: 0x2766a4, hash: 0xc0580312b35e0c8, est: 82, f: table_fill::fill_table },
    Hand { entry: 0x2766a8, end: 0x276764, hash: 0x9d6127a8e603f5ac, est: 59, f: claim_table::claim_slot },
    Hand { entry: 0x276768, end: 0x276818, hash: 0x9cf7ce82e7983cc8, est: 39, f: claim_table::release_record },
    Hand { entry: 0x276818, end: 0x276868, hash: 0x3feeaf46d3da3436, est: 20, f: claim_table::lookup_entry },
    Hand { entry: 0x276868, end: 0x2768ac, hash: 0x8c36c3a3149fc836, est: 14, f: key_slots::flag_set },
    Hand { entry: 0x2768b0, end: 0x2768fc, hash: 0x42afe308b2396cc5, est: 16, f: key_slots::flag_clear },
    Hand { entry: 0x277068, end: 0x2770fc, hash: 0x7e2ee2e0642ca419, est: 35, f: key_slots::queue_push },
    Hand { entry: 0x277100, end: 0x2771a4, hash: 0xb99652ccb34def68, est: 233, f: key_slots::queue_pop },
    Hand { entry: 0x2771a8, end: 0x277204, hash: 0xf7b696f6d977a5b1, est: 15, f: pause_slots::reset_pause_state },
    Hand { entry: 0x277208, end: 0x277248, hash: 0x952d4d1b30541745, est: 16, f: pause_slots::make_half_rect },
    Hand { entry: 0x277248, end: 0x2772d8, hash: 0x5afcd5f754edbb34, est: 20, f: pause_slots::clear_pending_slots },
    Hand { entry: 0x2772d8, end: 0x277450, hash: 0xfd94c31f82d7626d, est: 90, f: chunk_pass::chunk_pass },
    Hand { entry: 0x277898, end: 0x2778e8, hash: 0x6ce5185933fe6f46, est: 15, f: chunk_pass::frame_counter_tick },
    Hand { entry: 0x2778e8, end: 0x2779e8, hash: 0xb9699aad289631d0, est: 60, f: registry_restore::save_tier_slots },
    Hand { entry: 0x277e60, end: 0x277f14, hash: 0x156c0893d5e1253c, est: 19, f: registry_restore::restore_registry_records },
    Hand { entry: 0x277f18, end: 0x277f38, hash: 0x409b86548ba6c511, est: 8, f: registry_restore::restore_registry_boot },
    Hand { entry: 0x278058, end: 0x278188, hash: 0xe8ea8a462fa47aaf, est: 56, f: shadow::shadow_axis_setup },
    Hand { entry: 0x278188, end: 0x2781dc, hash: 0x10a0a025fa66b2f6, est: 17, f: shadow::shadow_build },
    Hand { entry: 0x2781e0, end: 0x278270, hash: 0x1eab44ebd385f8a1, est: 36, f: rec_hands::emit_gs_bucket },
    Hand { entry: 0x278270, end: 0x278418, hash: 0x86eb249f9f2ab28e, est: 271, f: shadow::shadow_rect_draw },
    Hand { entry: 0x278418, end: 0x2785e4, hash: 0x1e9bc9938c656beb, est: 280, f: shadow::shadow_rect_end },
    Hand { entry: 0x278e20, end: 0x278f6c, hash: 0x436487ca94881f88, est: 196, f: shadow::shadow_ring_build },
    Hand { entry: 0x2795c0, end: 0x279634, hash: 0x90b66b1ea518d03d, est: 214, f: shadow::shadow_clip_z },
    Hand { entry: 0x279990, end: 0x279acc, hash: 0x44776b6ba8bb9b69, est: 76, f: shadow::shadow_end_chain },
    Hand { entry: 0x279ad0, end: 0x279bd4, hash: 0xa575e17f195d129, est: 1578, f: shadow::shadow_patch_models },
    Hand { entry: 0x27c2f8, end: 0x27c384, hash: 0x8ac6ba4609395c13, est: 58, f: frame_mark::clear_entry_list },
    Hand { entry: 0x27c388, end: 0x27c4b4, hash: 0xf781ddc6414918f5, est: 75, f: spr_chain::spr_chain_flush },
    Hand { entry: 0x27c4c0, end: 0x27c518, hash: 0x6dfabf16f37bbc05, est: 18, f: spr_chain::spr_slot_dispatch },
    Hand { entry: 0x27c9d4, end: 0x27cb20, hash: 0xf4dd6f7a06bcc5ca, est: 607, f: sky_xform::sky_vertex_xform },
    Hand { entry: 0x27cb20, end: 0x27cc48, hash: 0x3b9a6792bb2c7f70, est: 1176, f: mask_prims::mask_prim_emit },
    Hand { entry: 0x27cfe8, end: 0x27d094, hash: 0x4d5268a2b7976b8, est: 33, f: point_cull::particle_jitter },
    Hand { entry: 0x27d138, end: 0x27d224, hash: 0x3c996e71db2d0b96, est: 32, f: voice_level::voice_level },
    Hand { entry: 0x27d228, end: 0x27d26c, hash: 0x6bd6135f8f251317, est: 17, f: voice_level::voice_distance_level },
    Hand { entry: 0x27d270, end: 0x27d310, hash: 0x52ffaf8317712a40, est: 40, f: voice_level::voice_bearing },
    Hand { entry: 0x27e148, end: 0x27e190, hash: 0x4c2e366b22e808ba, est: 8, f: voice_slot::voice_alive },
    Hand { entry: 0x27e1a0, end: 0x27e1f4, hash: 0x1097884ac568c7aa, est: 17, f: voice_slot::voice_release },
    Hand { entry: 0x27e470, end: 0x27e514, hash: 0x3443555aa1dd0139, est: 32, f: voice_slot::moby_cue },
    Hand { entry: 0x27e518, end: 0x27e5c8, hash: 0x765aeab3973671b2, est: 40, f: slot_attach::slot_attach_keyed },
    Hand { entry: 0x27e5c8, end: 0x27e654, hash: 0xab485581e7b8f577, est: 33, f: slot_attach::slot_attach_bounded },
    Hand { entry: 0x27e668, end: 0x27e72c, hash: 0xca1e3e9430d13fb7, est: 49, f: voice_slots::voice_pick_slot },
    Hand { entry: 0x27e738, end: 0x27e758, hash: 0x7b6abfaab58baf23, est: 8, f: voice_slots::voice_set_level },
    Hand { entry: 0x27e758, end: 0x27e778, hash: 0x7b6ac3aab58bb5ef, est: 8, f: voice_slots::voice_set_aux },
    Hand { entry: 0x27e778, end: 0x27e7e8, hash: 0x285648aa1ecda051, est: 17, f: voice_slots::voice_find_entry },
    Hand { entry: 0x27e7e8, end: 0x27e860, hash: 0x56840378be830579, est: 40, f: callback_tables::run_entry_callbacks },
    Hand { entry: 0x27e868, end: 0x27e910, hash: 0x570c5635efa75a68, est: 265, f: callback_tables::reset_tables },
    Hand { entry: 0x27ed60, end: 0x27ee90, hash: 0x8571a781ddda0765, est: 72, f: warp_camera_key::warp_camera_key },
    Hand { entry: 0x27ff00, end: 0x28003c, hash: 0x454e88467bf6edd1, est: 121, f: warp_quads::warp_big_quad },
    Hand { entry: 0x280040, end: 0x2800e0, hash: 0x139e314c15f72bf7, est: 40, f: warp_quads::warp_banner_quad },
    Hand { entry: 0x282d20, end: 0x282eac, hash: 0xd050e43fae9543f2, est: 99, f: box_quad_pkt::box_quad_pkt },
    Hand { entry: 0x284478, end: 0x284558, hash: 0x268e53e2d5b7ba24, est: 53, f: pool_record::push },
    Hand { entry: 0x2845d8, end: 0x2846f0, hash: 0x3492621838823a9, est: 66, f: rec_emit::rec_emit_loop },
    Hand { entry: 0x2846f0, end: 0x284718, hash: 0x5ea5834954a0c8b7, est: 8, f: rec_emit::rec_word_at },
    Hand { entry: 0x284720, end: 0x284878, hash: 0x7fd9c66c14c7fa31, est: 86, f: tfrag_lod_morph::lod_thresholds_morph },
    Hand { entry: 0x284878, end: 0x2849bc, hash: 0x6d5992ebd47eabc9, est: 78, f: tfrag_texture::tfrag_texture_chain },
    Hand { entry: 0x2849c0, end: 0x284a5c, hash: 0xb07d357b4a37454b, est: 1825, f: tfrag_texture::tfrag_texture_patch },
    Hand { entry: 0x286218, end: 0x286354, hash: 0xf08c1c80221c000b, est: 76, f: dma_tag_link::dma_tag_link },
    Hand { entry: 0x286358, end: 0x286418, hash: 0x46c201b28365ee60, est: 909, f: entry_patch::patch_entries },
    Hand { entry: 0x286418, end: 0x28646c, hash: 0x31e9329d1fbda2c2, est: 21, f: entry_patch::reload_patch_tables },
    Hand { entry: 0x286470, end: 0x28649c, hash: 0xea3d7f3d72d807ca, est: 11, f: entry_patch::call_pair },
    Hand { entry: 0x2864a0, end: 0x286568, hash: 0x948eb600682384c6, est: 50, f: frame_tail::frame_tail },
    Hand { entry: 0x288648, end: 0x288650, hash: 0xc84dc27f29a28935, est: 2, f: frame_tail::noop_hook },
    Hand { entry: 0x288650, end: 0x288690, hash: 0xafc0a39975bef9f1, est: 16, f: frame_tail::frame_latch_clear },
    Hand { entry: 0x288690, end: 0x288794, hash: 0xbdf456aeb7f05180, est: 23, f: counters_portrait::controller_tail_counters },
    Hand { entry: 0x28b890, end: 0x28b8cc, hash: 0x46e5d8a5d90ec7b, est: 15, f: shop_ticker::shop_ticker_set },
    Hand { entry: 0x28e4f8, end: 0x28e604, hash: 0xcde3715c55bdd055, est: 56, f: shop_ticker::shop_ticker_panel },
    Hand { entry: 0x28e608, end: 0x28e660, hash: 0x5404df4294e19f5d, est: 22, f: shop_ticker::shop_ticker_overlay },
    Hand { entry: 0x28faf0, end: 0x28fbac, hash: 0x21f2326dffe38454, est: 47, f: shop_frame_setup::offscreen_target_init },
    Hand { entry: 0x28fbb0, end: 0x28fbdc, hash: 0xd9f5889fbcf0d1f5, est: 11, f: shop_frame_setup::proj_dma_setup },
    Hand { entry: 0x290f20, end: 0x290fe8, hash: 0x3bc887ee5c9417fc, est: 33, f: vif_timer::vif0_chain_start },
    Hand { entry: 0x290fe8, end: 0x291014, hash: 0xc027ed7bf14998ef, est: 11, f: vif_timer::indexed_table_word },
    Hand { entry: 0x291018, end: 0x291074, hash: 0x765d1b1dd818a424, est: 23, f: vif_timer::timer_record_load },
    Hand { entry: 0x291078, end: 0x2910e8, hash: 0xc3200b96fab3b57a, est: 28, f: vif_timer::timer_ring_advance },
    Hand { entry: 0x291278, end: 0x2912c4, hash: 0x3fc6f7c72418187c, est: 19, f: cam_table_switch::cam_table_publish },
    Hand { entry: 0x2912d0, end: 0x291374, hash: 0xacbba4f8360e74a4, est: 41, f: vif_emit::emit_unpack_packet },
    Hand { entry: 0x291380, end: 0x2913c8, hash: 0x59317b867184bb23, est: 18, f: vif_emit::append_dma_tag },
    Hand { entry: 0x2913c8, end: 0x29147c, hash: 0xbc69b369fb1e5bf8, est: 45, f: vif_emit::gs_set_reg },
    Hand { entry: 0x291488, end: 0x2915a4, hash: 0xc3110a2539035fac, est: 71, f: gs_append_pkt::gs_scissor_pkt },
    Hand { entry: 0x2915b0, end: 0x29160c, hash: 0x26ce8b8952c48c93, est: 23, f: gs_append_pkt::dma_ref_1c3000 },
    Hand { entry: 0x291670, end: 0x2916cc, hash: 0x7b1182054d5d3623, est: 23, f: gs_append_pkt::dma_ref_1c25f0 },
    Hand { entry: 0x2916d8, end: 0x291734, hash: 0xbbeb84cfdc39620b, est: 23, f: ring_tags::ring_tag_pair },
    Hand { entry: 0x2917e8, end: 0x291944, hash: 0x29f35de158f6d4ea, est: 2637, f: cursor_tags::tag_ring_fill },
    Hand { entry: 0x291bc0, end: 0x291bf4, hash: 0xffb305ce8fa7df21, est: 13, f: cursor_tags::cursor_put_tag },
    Hand { entry: 0x291bf8, end: 0x291c94, hash: 0x2dd0c314262082ff, est: 35, f: ground_probe::probe_floor_height },
    Hand { entry: 0x291c98, end: 0x291cdc, hash: 0x9f79a8407c6fd57f, est: 17, f: ground_probe::joint_limits_default },
    Hand { entry: 0x291fb0, end: 0x2920f8, hash: 0xf21cfd551482e3b7, est: 74, f: plane_hits::plane_mask },
    Hand { entry: 0x2920f8, end: 0x292150, hash: 0xebc94ee6d85794a8, est: 104, f: plane_hits::plane_find },
    Hand { entry: 0x2925e0, end: 0x292670, hash: 0x42313c41e8efb0c6, est: 55, f: slot_anim::pose_slots_reset },
    Hand { entry: 0x2997a0, end: 0x2998bc, hash: 0xf9c13626403752b4, est: 17, f: slot_anim::anim_state_start },
    Hand { entry: 0x2998c0, end: 0x299910, hash: 0xb86145688b44cbdd, est: 17, f: attached_flags::attached_byte9_is_one },
    Hand { entry: 0x299910, end: 0x299950, hash: 0xa9543894faebde1, est: 11, f: attached_flags::attached_bit1_set },
    Hand { entry: 0x299df0, end: 0x299f48, hash: 0x36153dd84ac49413, est: 3843, f: actor_near_walk::proximity_walk },
    Hand { entry: 0x299f48, end: 0x299ff4, hash: 0x147dc2c706e21e50, est: 43, f: wrench_moby::wrench_aim_push },
    Hand { entry: 0x29b4e0, end: 0x29b5b4, hash: 0x1dc22b984857b82b, est: 53, f: wrench_moby::wrench_spawn },
    Hand { entry: 0x29b5d0, end: 0x29b6b0, hash: 0x5bc9a7bf603be736, est: 56, f: marker_spawn::alloc_marker_moby },
    Hand { entry: 0x29b6b0, end: 0x29b70c, hash: 0x540fdaae82531dac, est: 18, f: marker_spawn::marker_update },
    Hand { entry: 0x29edb0, end: 0x29eec4, hash: 0x85d5cdc4343a23b8, est: 61, f: moby_cull::orient_cull },
    Hand { entry: 0x2a40b8, end: 0x2a43d8, hash: 0xb7f654086a787658, est: 0, f: cutf_2a40b8::f_2a40b8 },
    Hand { entry: 0x2a7a78, end: 0x2a7b94, hash: 0x2c31812d6833ac9c, est: 71, f: clear_rgbaq_setup::setup_frame_clear },
    Hand { entry: 0x2a7b98, end: 0x2a7c7c, hash: 0x7c9e22394a87ab4d, est: 38, f: visibomb_fx::render_restore },
    Hand { entry: 0x2a7ec8, end: 0x2a7f48, hash: 0x3ef1c5ab06894d68, est: 32, f: visibomb_fx::cleanup },
    Hand { entry: 0x2aab40, end: 0x2aab88, hash: 0x539eb4b1e86d7e03, est: 11, f: moby_cull::claim_slot },
    Hand { entry: 0x2ad7a8, end: 0x2ad900, hash: 0xf493cce8d31f76de, est: 64, f: aux_slot_step::aux_slot_update },
    Hand { entry: 0x2adea8, end: 0x2adf70, hash: 0xc2e50e8d511ceb70, est: 251, f: beam_fan_reset::fan_emit_spawns },
    Hand { entry: 0x2af980, end: 0x2afa84, hash: 0xc0dd260d722c60fb, est: 17, f: beam_fan_reset::morph_beam_reset },
    Hand { entry: 0x2b1630, end: 0x2b1664, hash: 0x978732ce148ec0e6, est: 13, f: jump_ground::save_jump_pose },
    Hand { entry: 0x2b21f8, end: 0x2b2294, hash: 0xd6571e3df0f8a1d9, est: 102, f: jump_ground::scaled_step_chain },
    Hand { entry: 0x2b28b0, end: 0x2b2990, hash: 0x59fb2aea5366d792, est: 42, f: surface_probe::probe_surface_state },
    Hand { entry: 0x2b4100, end: 0x2b415c, hash: 0xdd8b760fdfaf4aee, est: 17, f: moby_anim_hooks::held_fire_latch },
    Hand { entry: 0x2b6d10, end: 0x2b6d64, hash: 0x5b813b2bed1ec2a4, est: 13, f: surface_probe::dispatch_surface_class },
    Hand { entry: 0x2b9d70, end: 0x2b9d8c, hash: 0xc282dc6e375fe36a, est: 7, f: moby_anim_hooks::moby_store_f58 },
    Hand { entry: 0x2b9d98, end: 0x2b9e48, hash: 0x5e60921bad492c7e, est: 44, f: moby_anim_hooks::held_pose_probe },
    Hand { entry: 0x2bc870, end: 0x2bc944, hash: 0x538dde5a94ba2334, est: 48, f: god_capsule::god_capsule_create },
    Hand { entry: 0x2bf5c8, end: 0x2bf654, hash: 0x6ddc2449481da5b3, est: 7, f: level_flags::update },
    Hand { entry: 0x2c1698, end: 0x2c17bc, hash: 0x261a5ee7ea2dbda5, est: 73, f: moby_slot_reset::moby_slot_reset },
    Hand { entry: 0x2c17c0, end: 0x2c18e0, hash: 0x4b135f934e58d251, est: 1648, f: lock_scan::lock_scan },
    Hand { entry: 0x2c3958, end: 0x2c39a4, hash: 0xf1ed9530cf8be9ad, est: 14, f: shark_suck::shark_suck_accept },
    Hand { entry: 0x2c39f8, end: 0x2c3aa8, hash: 0x6738712dcc9a918f, est: 34, f: shark_suck::suck_slot_latch },
    Hand { entry: 0x2c3aa8, end: 0x2c3ad4, hash: 0x818fb2fd77b82063, est: 11, f: shark_suck::shark_suck_cancel },
    Hand { entry: 0x2c3ff8, end: 0x2c412c, hash: 0xff6d570feab27a26, est: 77, f: moby_slots::slot_setup_all },
    Hand { entry: 0x2c4130, end: 0x2c4178, hash: 0xb4588849bef6fdd7, est: 16, f: moby_slots::actor_release },
    Hand { entry: 0x2c6038, end: 0x2c617c, hash: 0x2f5714363ec0b467, est: 67, f: debris_step::debris_update },
    Hand { entry: 0x2cd988, end: 0x2cd9a4, hash: 0xb70d97b6c3c42ffd, est: 7, f: moby_slots::free_moby_entry },
    Hand { entry: 0x2d0f30, end: 0x2d0fdc, hash: 0xf24340ea7bac71ac, est: 43, f: pose_anim_hud::pose_probe_call },
    Hand { entry: 0x2d41e8, end: 0x2d4248, hash: 0xa93353c1f177f763, est: 12, f: pose_anim_hud::anim_start_if_flag },
    Hand { entry: 0x2d67c0, end: 0x2d6890, hash: 0xef4c6220d8b9121d, est: 150, f: pose_anim_hud::trespass_hud_backdrop },
    Hand { entry: 0x2d6890, end: 0x2d69d4, hash: 0x74ccd1a8da329ea0, est: 81, f: keyframe_moby::keyframe_blend },
    Hand { entry: 0x2d9be0, end: 0x2d9bf0, hash: 0x1833b2c200656b95, est: 4, f: camera::actor_state_is_1 },
    Hand { entry: 0x2d9bf0, end: 0x2d9c64, hash: 0xdb679b8ae2f39420, est: 29, f: keyframe_moby::spawn_attached_moby },
    Hand { entry: 0x2d9ed0, end: 0x2d9fc8, hash: 0x156463f1f5fc8486, est: 62, f: orb_fx::orb_puff_burst },
    Hand { entry: 0x2da308, end: 0x2da3bc, hash: 0x1a7ed57a82c0dce3, est: 40, f: orb_fx::orb_sparkle_pos },
    Hand { entry: 0x2da3c0, end: 0x2da434, hash: 0x54c3991bf1a7293a, est: 120, f: orb_gates::orb_vectors_spin },
    Hand { entry: 0x2da438, end: 0x2da4a8, hash: 0xbf8d956989e5bf79, est: 28, f: orb_gates::orb_pickup_los },
    Hand { entry: 0x2dbc30, end: 0x2dbc88, hash: 0x5f26bc6682b4251f, est: 19, f: orb_gates::suck_hero_blocked },
    Hand { entry: 0x2dc318, end: 0x2dc444, hash: 0x44bb5b915a3ef2d9, est: 93, f: weapon_slots::weapon_slot_sweep },
    Hand { entry: 0x2dd548, end: 0x2dd5b0, hash: 0x2403257f19b0f745, est: 23, f: weapon_slots::comp_lookup },
    Hand { entry: 0x2ddad8, end: 0x2ddbdc, hash: 0x4514941720b0717c, est: 50, f: slot_cam::slot_state_step },
    Hand { entry: 0x2df958, end: 0x2df970, hash: 0xf191126ad2fb8f92, est: 6, f: weapon_slots::set_ready_flags },
    Hand { entry: 0x2e0c98, end: 0x2e0d00, hash: 0x25e4cc09a104b615, est: 18, f: suck_cannon::suck_tube_fade_step },
    Hand { entry: 0x2e1090, end: 0x2e1100, hash: 0xc9e200ed21fc7f0a, est: 12, f: suck_cannon::kind_scale_factor },
    Hand { entry: 0x2e1548, end: 0x2e1628, hash: 0x2d3713a08f691b9e, est: 104, f: suck_cannon::seed_jitter_block },
    Hand { entry: 0x2e1f40, end: 0x2e202c, hash: 0x1160556666dd54d7, est: 24, f: actor_sway::actor_sway },
    Hand { entry: 0x2e2030, end: 0x2e2110, hash: 0x421340ca6525ff3e, est: 104, f: actor_sway::actor_sway_seed },
    Hand { entry: 0x2e2740, end: 0x2e27dc, hash: 0xab764c99e891739c, est: 39, f: pose_chain_glow::pose_chain_lengths },
    Hand { entry: 0x2e28b8, end: 0x2e29a0, hash: 0xd881bc79b9d44013, est: 24, f: pose_chain_glow::glow_cheat_layers },
    Hand { entry: 0x2e36f8, end: 0x2e3750, hash: 0x7d7a9ba4f2100975, est: 12, f: pose_chain_glow::object_state_tick },
    Hand { entry: 0x2e4cc8, end: 0x2e4d64, hash: 0x29628d519556649c, est: 14, f: state_tick::slot_state_tick },
    Hand { entry: 0x2e72d0, end: 0x2e73c8, hash: 0x6ec744539082c634, est: 21, f: state_tick::link_state_tick },
    Hand { entry: 0x2e73d0, end: 0x2e73d8, hash: 0xc84dc27f29a28935, est: 2, f: camera::noop },
    Hand { entry: 0x2e73d8, end: 0x2e73f0, hash: 0x14b8b2f5b1ae8d21, est: 6, f: camera::follow_hero_pos },
    Hand { entry: 0x2e73f0, end: 0x2e7500, hash: 0x767821abdf1c2132, est: 28, f: grid_offset::cell_offset },
    Hand { entry: 0x2e7500, end: 0x2e752c, hash: 0x69cfd2280e2c7b03, est: 11, f: camera::cam_point_visible },
    Hand { entry: 0x2e7530, end: 0x2e7698, hash: 0x71aa1c3a33ee5829, est: 37, f: follow_ground_point::follow_ground_point },
    Hand { entry: 0x2e7698, end: 0x2e76a0, hash: 0xc84dc27f29a28935, est: 2, f: camera::noop },
    Hand { entry: 0x2e76a0, end: 0x2e77e4, hash: 0xe2fce18c43b47733, est: 81, f: pose_setup::pose_setup },
    Hand { entry: 0x2e77f0, end: 0x2e7924, hash: 0x7bb6b9de3f1eedd4, est: 74, f: pitch_point::follow_point_pitch_angle },
    Hand { entry: 0x2e7928, end: 0x2e7b0c, hash: 0xdc1a53a4789a1353, est: 68, f: camera::cam_hero_nudge },
    Hand { entry: 0x2e7b10, end: 0x2e7b9c, hash: 0xffc666c3ba59fe3d, est: 35, f: camera::follow_held_height },
    Hand { entry: 0x2e8828, end: 0x2e88e4, hash: 0x171d5a9c6cc04125, est: 47, f: camrig_spring::rig_spring_pos },
    Hand { entry: 0x2e88e8, end: 0x2e89a0, hash: 0x17730cb3a882f9b7, est: 26, f: camrig_spring::cam_offset_update },
    Hand { entry: 0x2e89a0, end: 0x2e8a9c, hash: 0xa986846c674ce82d, est: 50, f: look_spring::look_point_spring },
    Hand { entry: 0x2e90a0, end: 0x2e90a8, hash: 0xc84dc27f29a28935, est: 2, f: camera::noop },
    Hand { entry: 0x2e90a8, end: 0x2e926c, hash: 0x4dadf4285321bebb, est: 102, f: camera::cam_rig_reset },
    Hand { entry: 0x2e9e68, end: 0x2e9fd0, hash: 0xfe054cae1e2b964c, est: 90, f: inherit_pose_rig::follow_rig_from_inherited_pose },
    Hand { entry: 0x2ea790, end: 0x2ea7a0, hash: 0x5345b9a9494db00e, est: 4, f: camera::return_zero },
    Hand { entry: 0x2ea7a0, end: 0x2ea884, hash: 0xd5f04e881c3ddddb, est: 57, f: camera_claim::rig_offset_vectors },
    Hand { entry: 0x2eac38, end: 0x2eac9c, hash: 0xeef7593f95bd0236, est: 13, f: camera::cam_facing_away },
    Hand { entry: 0x2eb5e8, end: 0x2eb5f4, hash: 0x31d50a20f4dec2af, est: 3, f: camera::noop_frame },
    Hand { entry: 0x2eb5f8, end: 0x2eb630, hash: 0x4c458bd2e57e173c, est: 9, f: camera::cam_release_owner },
    Hand { entry: 0x2eb630, end: 0x2eb6c0, hash: 0xdab290fcf0c52a3b, est: 12, f: camera_claim::rig_claim_owner },
    Hand { entry: 0x2eb6c0, end: 0x2eb6fc, hash: 0xef079d762a1cf5e7, est: 14, f: camera::cam_set_override },
    Hand { entry: 0x2eb728, end: 0x2eb75c, hash: 0xa9bdb2e76619f955, est: 13, f: camera_claim::rig_set_pair },
    Hand { entry: 0x2eb760, end: 0x2eb7b0, hash: 0x506a2cb31f09d650, est: 14, f: cam_rig_setters::cam_set_look_target },
    Hand { entry: 0x2eb7d8, end: 0x2eb7fc, hash: 0x8079fdf20af419c, est: 9, f: cam_rig_setters::cam_set_auto_follow },
    Hand { entry: 0x2eb800, end: 0x2eb848, hash: 0xbd44b3c50605296, est: 18, f: cam_rig_setters::cam_set_target_xy_spring },
    Hand { entry: 0x2eb890, end: 0x2eb8b8, hash: 0x3aed7d0dc5ad424a, est: 10, f: cam_rig_setters::cam_no_retreat },
    Hand { entry: 0x2eb8b8, end: 0x2eb8e0, hash: 0xde572cfaa531015a, est: 10, f: cam_look::cam_set_look_blend_flag },
    Hand { entry: 0x2ebb88, end: 0x2ebbdc, hash: 0x9c0a6287e0824815, est: 21, f: cam_look::cam_set_look_float },
    Hand { entry: 0x2ebc20, end: 0x2ebe24, hash: 0xd6e7918342f3efb2, est: 62, f: camera::cam_input_axes },
    Hand { entry: 0x2ebe28, end: 0x2ec1b0, hash: 0xebffd30f2adbf15f, est: 147, f: camera::orbit_tick_hand },
    Hand { entry: 0x2ec1b0, end: 0x2ec27c, hash: 0xcf91a6d0778f14b3, est: 42, f: camera::cam_boom_follow },
    Hand { entry: 0x2ecd10, end: 0x2ece20, hash: 0x6d1273543fc5753f, est: 45591, f: camera::cam_volume_scan },
    Hand { entry: 0x2ed038, end: 0x2ed174, hash: 0xd7223c0818eb3cae, est: 79, f: cam_look::cam_orient_refresh },
    Hand { entry: 0x2ed248, end: 0x2ed264, hash: 0xb1bf837973d4c6bb, est: 7, f: cam_look::cam_update_wrapper },
    Hand { entry: 0x2ed268, end: 0x2ed284, hash: 0x3e5993c35f41de71, est: 7, f: cam_vec_pair::passthrough_2ed038 },
    Hand { entry: 0x2ed2f0, end: 0x2ed3fc, hash: 0x4cb462be7a4654ef, est: 41, f: cam_vec_pair::camera_vec_setup },
    Hand { entry: 0x2ed400, end: 0x2ed520, hash: 0x9d750a02a1dced73, est: 56, f: veldin_cam::veldin_basis },
    Hand { entry: 0x2ed520, end: 0x2ed568, hash: 0x4d3a7dd6d36fc7a0, est: 18, f: veldin_cam::veldin_reset },
    Hand { entry: 0x2eddd0, end: 0x2ede40, hash: 0xd945e52e5ff4444a, est: 28, f: veldin_cam::cam_mode_tick },
    Hand { entry: 0x2ee008, end: 0x2ee060, hash: 0xa08c2665aea74300, est: 18, f: cam_slot_copy::cam_store_slot_80 },
    Hand { entry: 0x2ee060, end: 0x2ee0b8, hash: 0xeab9b96d13746f90, est: 18, f: cam_slot_copy::cam_store_slot_90 },
    Hand { entry: 0x2ee0c0, end: 0x2ee124, hash: 0xf7adf73a78244586, est: 21, f: cam_slot_copy::cam_store_slot_float_e0 },
    Hand { entry: 0x2ee130, end: 0x2ee2a4, hash: 0xbed5b1ad8874d1e8, est: 90, f: cam_slot_fill::cam_slot_fill },
    Hand { entry: 0x2ee8a8, end: 0x2ee8fc, hash: 0x4075032d9e4ea824, est: 15, f: cam_slot_fill::cam_slot_refresh },
    Hand { entry: 0x2ee9b0, end: 0x2eea40, hash: 0xb0262b017892540d, est: 36, f: slot_cam::cam6_reseat },
    Hand { entry: 0x2efd08, end: 0x2efd34, hash: 0x1a9d1357006661e5, est: 11, f: swing_cam::swing_cam_reset },
    Hand { entry: 0x2efd38, end: 0x2efd6c, hash: 0x4d9f61d7b6152ef0, est: 13, f: swing_cam::swing_cam_init },
    Hand { entry: 0x2f0020, end: 0x2f010c, hash: 0xb2effb3a1a74a90d, est: 31, f: swing_cam::swing_cam_update },
    Hand { entry: 0x2f0b90, end: 0x2f0c90, hash: 0x969be24eaa60e06a, est: 33, f: cam17::cam17_gate },
    Hand { entry: 0x2f1210, end: 0x2f1294, hash: 0xdf702ba22a9ae0ad, est: 19, f: cam17::cam17_release },
];

/// Registers the hand-written functions with ee-oracle (before `Machine::enable_rec`).
/// ORACLE_HAND_RANGE=lo-hi keeps only the entries in [lo, hi) and ORACLE_HAND_SKIP=a,b,.. drops entries (hex;
/// bisecting a gate failure).
pub fn register() {
    buildtag::touch();
    let hex = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap();
    let range = std::env::var("ORACLE_HAND_RANGE").ok().map(|v| {
        let (a, b) = v.split_once('-').unwrap();
        (hex(a), hex(b))
    });
    let skip: Vec<u32> = std::env::var("ORACLE_HAND_SKIP").map(|v| v.split(',').map(hex).collect()).unwrap_or_default();
    if range.is_none() && skip.is_empty() {
        ee_oracle::rec::register_hand(EUDORA);
        return;
    }
    let keep = EUDORA
        .iter()
        .filter(|h| range.is_none_or(|(a, b)| h.entry >= a && h.entry < b) && !skip.contains(&h.entry))
        .map(|h| Hand { entry: h.entry, end: h.end, hash: h.hash, est: h.est, f: h.f })
        .collect::<Vec<_>>();
    ee_oracle::rec::register_hand(Box::leak(keep.into_boxed_slice()));
}
