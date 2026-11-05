//! Tests of the upscaler inputs (upscale.rs, motion.wgsl): the env switch, the jitter sequence and matrix, the cut test, and a
//! CPU twin of the shader's reprojection checked against the true motion of world points.

use super::*;

type V4 = [f32; 4];

fn mul(m: &M4, v: V4) -> V4 {
	std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2] + m[3][r] * v[3])
}

fn uv_of(c: V4) -> [f32; 2] {
	[0.5 + 0.5 * c[0] / c[3], 0.5 - 0.5 * c[1] / c[3]]
}

/// CPU twin of motion.wgsl `motion_of`.
fn motion_of(u: &MotionUniforms, uv: [f32; 2], d: f32, sky: bool) -> [f32; 2] {
	let (inv, cur, prev) = if sky { (&u.sky_inv_cur, &u.sky_cur, &u.sky_prev) } else { (&u.inv_cur, &u.cur, &u.prev) };
	let h = mul(inv, [uv[0] * 2.0 - 1.0, 1.0 - uv[1] * 2.0, d, 1.0]);
	let p = [h[0] / h[3], h[1] / h[3], h[2] / h[3], 1.0];
	let (c, q) = (mul(cur, p), mul(prev, p));
	if q[3] <= 1e-6 || c[3] <= 1e-6 {
		return [0.0; 2];
	}
	let (a, b) = (uv_of(q), uv_of(c));
	[a[0] - b[0], a[1] - b[1]]
}

fn cam(pos: [f32; 3], yaw: f32, pitch: f32) -> Camera {
	Camera { pos, yaw, pitch, tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] }
}

fn mats(c: &Camera, aspect: f32) -> Mats {
	let eye = Camera { pos: [0.0; 3], ..*c };
	Mats { vp: view_proj(c, aspect, RETAIL_NEAR, RETAIL_FAR), sky: view_proj(&eye, aspect, RETAIL_NEAR, RETAIL_FAR) }
}

fn jittered(m: &Mats, j: [f32; 2], size: (u32, u32)) -> Mats {
	Mats { vp: jitter_matrix(&m.vp, j, size), sky: jitter_matrix(&m.sky, j, size) }
}

const SIZE: (u32, u32) = (1280, 720);

/// World points in front of camera `c` (yaw 0 looks along +x): near, mid and far, off centre.
fn points() -> Vec<[f32; 3]> {
	vec![[1.5, 0.3, 0.2], [6.0, -1.0, 0.8], [20.0, 4.0, -2.0], [80.0, -15.0, 6.0], [300.0, 40.0, 20.0]]
}

/// For each point: where the jittered frame drew it (uv, depth), what the shader twin says, and the true motion
/// (previous uv - current plain uv).
fn check(prev_cam: &Camera, cur_cam: &Camera, j: [f32; 2], tol: f32) {
	let aspect = SIZE.0 as f32 / SIZE.1 as f32;
	let (cur, prev) = (mats(cur_cam, aspect), mats(prev_cam, aspect));
	let jit = jittered(&cur, j, SIZE);
	let u = motion_uniforms(&cur, &prev, &jit, SIZE, j, false);
	for p in points() {
		let w = [p[0] + cur_cam.pos[0], p[1] + cur_cam.pos[1], p[2] + cur_cam.pos[2], 1.0];
		let drawn = mul(&jit.vp, w);
		let (uv, d) = (uv_of(drawn), drawn[2] / drawn[3]);
		let got = motion_of(&u, uv, d, false);
		let (a, b) = (uv_of(mul(&prev.vp, w)), uv_of(mul(&cur.vp, w)));
		let want = [a[0] - b[0], a[1] - b[1]];
		let err = ((got[0] - want[0]) * SIZE.0 as f32).abs().max(((got[1] - want[1]) * SIZE.1 as f32).abs());
		assert!(err < tol, "point {p:?}: got {got:?} want {want:?} ({err} px)");
	}
}

#[test]
fn the_env_switch() {
	assert_eq!(parse_mode(None), Mode::Off);
	assert_eq!(parse_mode(Some("")), Mode::Off);
	assert_eq!(parse_mode(Some("0")), Mode::Off);
	assert_eq!(parse_mode(Some("off")), Mode::Off);
	assert_eq!(parse_mode(Some("1")), Mode::On);
	assert_eq!(parse_mode(Some(" Show ")), Mode::Show);
	assert_eq!(parse_mode(Some("still")), Mode::Still);
}

#[test]
fn halton_values() {
	let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
	assert!(near(halton(1, 2), 0.5) && near(halton(2, 2), 0.25) && near(halton(3, 2), 0.75) && near(halton(4, 2), 0.125));
	assert!(near(halton(1, 3), 1.0 / 3.0) && near(halton(2, 3), 2.0 / 3.0) && near(halton(3, 3), 1.0 / 9.0));
	assert_eq!(halton(0, 2), 0.0);
}

#[test]
fn phases_follow_the_fsr_rule() {
	assert_eq!(phase_count(1920, 1920), 8);
	assert_eq!(phase_count(1280, 1920), 18, "1.5x: ceil(8 * 2.25)");
	assert_eq!(phase_count(960, 1920), 32, "2x");
	assert_eq!(phase_count(2560, 1920), MIN_PHASES, "supersampled: never below 8");
	assert_eq!(phase_count(0, 0), 8, "no zero division");
}

#[test]
fn jitter_stays_inside_the_pixel_and_averages_out() {
	for phases in [8, 18, 32] {
		let js: Vec<[f32; 2]> = (0..phases).map(|f| jitter_px(f, phases)).collect();
		assert!(js.iter().all(|j| j[0] > -0.5 && j[0] < 0.5 && j[1] > -0.5 && j[1] < 0.5));
		let mean = js.iter().fold([0.0f32; 2], |a, j| [a[0] + j[0], a[1] + j[1]]).map(|s| s / phases as f32);
		assert!(mean[0].abs() < 0.07 && mean[1].abs() < 0.07, "{phases}: mean {mean:?}");
		assert_eq!(jitter_px(phases, phases), jitter_px(0, phases), "period");
	}
}

#[test]
fn jitter_matrix_moves_every_point_by_the_jitter() {
	let c = cam([10.0, -4.0, 2.0], 0.4, -0.2);
	let m = view_proj(&c, SIZE.0 as f32 / SIZE.1 as f32, RETAIL_NEAR, RETAIL_FAR);
	let j = [0.3, -0.45];
	let jm = jitter_matrix(&m, j, SIZE);
	for p in points() {
		let w = [p[0] * 0.4f32.cos() + c.pos[0], p[0] * 0.4f32.sin() + p[1] + c.pos[1], p[2] + c.pos[2], 1.0];
		let (a, b) = (mul(&m, w), mul(&jm, w));
		if a[3] <= 0.0 {
			continue;
		}
		let (pa, pb) = (uv_of(a), uv_of(b));
		let moved = [(pb[0] - pa[0]) * SIZE.0 as f32, (pb[1] - pa[1]) * SIZE.1 as f32];
		assert!((moved[0] - j[0]).abs() < 1e-3 && (moved[1] - j[1]).abs() < 1e-3, "{p:?}: moved {moved:?}");
		assert!((a[2] / a[3] - b[2] / b[3]).abs() < 1e-7, "depth unchanged");
	}
	assert_eq!(jitter_matrix(&m, [0.0; 2], SIZE), m, "no jitter = the same matrix");
}

#[test]
fn a_still_camera_has_no_motion_even_with_jitter() {
	let c = cam([3.0, 7.0, 1.5], 0.0, 0.1);
	check(&c, &c, [0.0; 2], 2e-3);
	check(&c, &c, [0.37, -0.21], 2e-3);
}

#[test]
fn camera_moves_and_turns_give_the_true_motion() {
	let cur = cam([0.0, 0.0, 1.5], 0.0, 0.0);
	check(&cam([-0.3, 0.1, 1.5], 0.0, 0.0), &cur, [0.2, 0.4], 0.05);
	check(&cam([0.0, 0.0, 1.5], 0.05, -0.02), &cur, [-0.25, 0.1], 0.05);
	check(&cam([0.2, -0.2, 1.4], -0.08, 0.03), &cur, [0.0, 0.0], 0.05);
}

#[test]
fn the_sky_moves_with_the_turn_only() {
	let aspect = SIZE.0 as f32 / SIZE.1 as f32;
	let (cur, moved) = (mats(&cam([0.0, 0.0, 1.5], 0.0, 0.0), aspect), mats(&cam([5.0, 2.0, 1.5], 0.0, 0.0), aspect));
	let u = motion_uniforms(&cur, &moved, &cur, SIZE, [0.0; 2], false);
	for uv in [[0.5, 0.5], [0.1, 0.2], [0.9, 0.7]] {
		let m = motion_of(&u, uv, 1.0, true);
		assert!(m[0].abs() < 1e-5 && m[1].abs() < 1e-5, "a move without a turn leaves the sky still: {m:?}");
	}
	let turned = mats(&cam([0.0, 0.0, 1.5], 0.05, 0.0), aspect);
	let u = motion_uniforms(&cur, &turned, &cur, SIZE, [0.0; 2], false);
	let m = motion_of(&u, [0.5, 0.5], 1.0, true);
	// Last frame looked further left (yaw + 0.05 turns towards +y, the view's left): the point now in the centre was right of it.
	assert!(m[0] > 0.01 && m[1].abs() < 1e-3, "a turn: {m:?}");
}

#[test]
fn a_reset_frame_has_no_motion() {
	let aspect = SIZE.0 as f32 / SIZE.1 as f32;
	let cur = mats(&cam([0.0; 3], 0.3, 0.0), aspect);
	let u = motion_uniforms(&cur, &cur, &cur, SIZE, [0.0; 2], true);
	assert_eq!(u.misc[1], 1.0);
	assert_eq!(u.prev, u.cur);
	assert_eq!(std::mem::size_of::<MotionUniforms>(), 416, "motion.wgsl U is 6 mat4 + 2 vec4");
}

#[test]
fn cuts() {
	let v = View { level: 3, render: (1280, 720), display: (1920, 1080), eye: [0.0; 3], fwd: [1.0, 0.0, 0.0] };
	assert!(is_cut(None, &v), "first frame");
	assert!(!is_cut(Some(&v), &v));
	assert!(!is_cut(Some(&v), &View { eye: [1.0, 1.0, 0.5], ..v }), "a normal move");
	assert!(is_cut(Some(&v), &View { eye: [CUT_DIST + 0.1, 0.0, 0.0], ..v }), "a jump");
	assert!(is_cut(Some(&v), &View { fwd: [0.0, 1.0, 0.0], ..v }), "a 90 degree snap");
	assert!(!is_cut(Some(&v), &View { fwd: [0.9, 0.43, 0.0], ..v }), "a fast turn");
	assert!(is_cut(Some(&v), &View { level: 4, ..v }));
	assert!(is_cut(Some(&v), &View { render: (960, 540), ..v }));
	assert!(is_cut(Some(&v), &View { display: (1280, 720), ..v }));
}

#[test]
fn shader_validates() {
	let m = naga::front::wgsl::parse_str(motion::SHADER).expect("wgsl parse");
	naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
}

/// DX12 parity: the shader also goes through naga's HLSL writer (as volsmoke / metalenv tests do; FXC / DXC run on a DX12 run).
#[cfg(windows)]
#[test]
fn shader_writes_hlsl() {
	let m = naga::front::wgsl::parse_str(motion::SHADER).expect("wgsl parse");
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	let (options, pipeline) = (naga::back::hlsl::Options::default(), naga::back::hlsl::PipelineOptions::default());
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &options, &pipeline).write(&m, &info, None).expect("HLSL writes");
	assert!(out.contains("fs_motion") && out.contains("fs_show"));
}

/// R9-2: the Classic half of a split view draws with the upscaler state paused (split.rs), and the Enhanced frame's comes back.
#[test]
fn the_split_view_classic_pass_pauses_the_upscaler() {
	let mut p = Prep { on: true, slot_on: true, ..Default::default() };
	let was = p.pause();
	assert!(!p.on && !p.slot_on && !p.active() && p.scene_view().is_none() && p.hud_targets().is_none());
	assert_eq!(p.hud_rect([1.0, 2.0, 3.0, 4.0]), [1.0, 2.0, 3.0, 4.0]);
	p.resume(was);
	assert!(p.on && p.slot_on);
}
