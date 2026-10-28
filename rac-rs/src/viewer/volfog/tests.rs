use super::*;

/// The fog shader compiles (naga) with its three entry pairs and validates.
#[test]
fn volfog_shader_parses() {
	let m = naga::front::wgsl::parse_str(&VolFog::source()).expect("volfog shader");
	let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
	for n in ["vs_full", "fs_march", "fs_resolve", "fs_apply"] {
		assert!(names.contains(&n), "{n}");
	}
	let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
	v.validate(&m).expect("volfog shader validates");
}

/// The Rust uniform is the WGSL block: 16-byte multiples, 16 members of the sizes the block lists.
#[test]
fn uniform_size_matches_block() {
	assert_eq!(std::mem::size_of::<Uniform>(), 4 * 16 * 2 + 3 * 64 + 17 * 16);
	assert_eq!(std::mem::size_of::<Uniform>() % 16, 0);
}

#[test]
fn quality_levels() {
	assert_eq!(quality(0), None);
	assert_eq!(quality(1).map(|q| (q.div, q.shadows)), Some((2, false)));
	assert!(quality(3).unwrap().steps > quality(2).unwrap().steps);
	// What runs at High: Medium's grid with more steps. (RAC_FOG_DIV, gfxq.rs, overrides the divisor at run time in `run`, not in
	// `quality`, so this holds whatever the environment says.)
	assert_eq!((quality(2).unwrap().div, quality(3).unwrap().div), (2, 2));
}

/// Retail's own fog is faded (scale < 1) exactly while the pass draws, decided from the frame's inputs alone (`Plan::new`, the code
/// prepare.rs runs before it writes the world uniforms): switching Enhanced -> Classic, going under water or into a level with no
/// volume gives 1.0 in the same frame, never a stale value.
#[test]
fn retail_fog_scale_follows_the_pass() {
	use crate::retail::pcext::Graphics;
	let on = Graphics { lighting: 1, fog_q: 3, ..Graphics::CLASSIC };
	let scale = |g: Graphics, level: usize, fog: [f32; 4], active: bool, ocean: &[(f32, usize)]| frame::retail_fog_scale(frame::Plan::new(&g, level, fog, active, ocean));
	let veldin = [0.0, 235.0, 255.0, 53.55];
	let faded = scale(on, 0, veldin, true, &[]);
	assert!(faded < 1.0 && faded > 0.0, "{faded}");
	// Enhanced -> Classic (the old build kept `faded` here), Fog Classic with Lighting Enhanced, and a frame the pass skips (active false).
	assert_eq!(scale(Graphics { lighting: 0, ..on }, 0, veldin, true, &[]), 1.0);
	assert_eq!(scale(Graphics { fog_q: 0, ..on }, 0, veldin, true, &[]), 1.0);
	assert_eq!(scale(on, 0, veldin, false, &[]), 1.0);
	// Back to Enhanced: the same scale as before the toggle (= a fresh Enhanced start).
	assert_eq!(scale(on, 0, veldin, true, &[]), faded);
	// A level whose retail fog ends at nothing and has no sheet, and a space level (no air haze, no water): nothing draws, retail's fog stays.
	assert_eq!(scale(on, 5, [0.0, 900.0, 255.0, 255.0], true, &[]), 1.0);
	assert_eq!(scale(on, 16, veldin, true, &[]), 1.0);
	// Kerwan's fog sheet stands on the whole ocean's height: the plan is the same wherever the camera is; no ocean, no sheet, retail's fog.
	let kerwan = [0.0, 900.0, 255.0, 255.0];
	let ocean = [(-24.0, 300usize)];
	assert_eq!(frame::Plan::new(&on, 3, kerwan, true, &ocean).map(|p| p.sheet_z), Some(Some(-24.0)));
	assert_eq!(scale(on, 3, kerwan, true, &ocean), faded);
	assert_eq!(scale(on, 3, kerwan, true, &[]), 1.0);
}

/// The sheet is the mean height of all patches, the water mist lies on the height a quarter of the patches are below.
#[test]
fn ocean_heights_to_sheet_and_mist() {
	assert_eq!(frame::Plan::new(&crate::retail::pcext::Graphics { lighting: 1, fog_q: 3, ..crate::retail::pcext::Graphics::CLASSIC }, 3, [0.0, 900.0, 255.0, 255.0], true, &[(-20.0, 100), (-30.0, 300)]).and_then(|p| p.sheet_z), Some(-27.5));
	let novalis = [0.0, 240.0, 255.0, 102.0];
	let gfx = crate::retail::pcext::Graphics { lighting: 1, fog_q: 3, ..crate::retail::pcext::Graphics::CLASSIC };
	let mist = |h: &[(f32, usize)]| frame::Plan::new(&gfx, 1, novalis, true, h).and_then(|p| p.mist_z);
	// A high pond (20 patches) does not lift the mist off the main water (10 + 10 patches): the quarter of 40 patches is the 10th.
	assert_eq!(mist(&[(9.0, 20), (1.0, 10), (5.0, 10)]), Some(5.0));
	assert_eq!(mist(&[]), None);
}

/// The wiring of the fog decision in `frame_uniforms` (viewer/gpu/draw/prepare.rs), checked on its source: `Plan::new` and the
/// `set_gs_scale` write run EVERY frame at the top level of the function (not inside `if post_on`, which was the bug: Enhanced ->
/// Classic kept retail's fog faded), in that order, before the world uniform write, and a frame without a plan lets the bind-group
/// cache go (`idle`).
#[test]
fn prepare_decides_the_fog_every_frame() {
	let src = include_str!("../gpu/draw/prepare.rs").replace("\r\n", "\n");
	let line_start = |at: usize| src[..at].rfind('\n').map_or(0, |n| n + 1);
	let find = |needle: &str| {
		assert_eq!(src.matches(needle).count(), 1, "prepare.rs: `{needle}` exactly once");
		src.find(needle).unwrap()
	};
	// The text in front of the call on its line, split into the indentation and the rest.
	let before = |at: usize| {
		let pre = &src[line_start(at)..at];
		let rest = pre.trim_start_matches('\t');
		(pre.len() - rest.len(), rest.to_string())
	};
	let (plan, scale) = (find("volfog::Plan::new("), find("fogopt::set_gs_scale("));
	let write = find("write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&super::super::volfog::fade_retail_fog(u)))");
	let idle = find("self.volfog.idle()");
	// Two tabs = the function body itself: no enclosing `if post_on { .. }`.
	assert_eq!(before(plan), (2, "let fog = super::super::".to_string()), "Plan::new is a plain statement of the function");
	assert_eq!(before(scale), (2, "crate::retail::pcext::".to_string()), "set_gs_scale is a plain statement of the function");
	assert!(plan < scale && scale < write, "plan, then the scale, then the uniform upload");
	assert!(plan < idle && idle < write && src[plan..idle].contains("if fog.is_none() {"), "a frame with no plan drops the cached bind groups");
	assert_eq!(before(idle).0, 3, "idle is the body of the `if fog.is_none()`");
	// The pass is on exactly when `Plan::new` says so: the activity test is the one shared by the plan (no separate flag).
	assert!(src.contains("let fog_active = post_on && !scene.warp_shown && !self.ship_fight"), "fog_active keeps its terms");
}
