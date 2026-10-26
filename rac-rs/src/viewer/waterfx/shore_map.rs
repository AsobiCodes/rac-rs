//! The geometric sea's shore map: its compute shader (WGSL), the uniform block and the map's size constants.

use super::*;

/// [PC] Geometric sea waves: the shore map. Every SHORE_STRIDE-th pixel of the water bed's depth (mirror.rs `draw_bed`: the
/// level without its water) as a world point; a point at most SHORE_CAP above the highest sea plane raises its cell of a
/// camera-centred SHORE_N x SHORE_N grid of SHORE_CELL units to its height (ordered key, atomic max; 0 = not seen). So a cell
/// holds the sea bed, or the sand, wall or rock that comes up to or through the surface there (`calm`). [PC] The buffer's
/// second quarter keeps per cell the highest ground from SHORE_NEAR under the highest sea plane to SHORE_HIGH above it, the
/// third and fourth make two distances (`cs_shore_h`, `cs_shore_v`; `shore_dist`): to any such ground (a dock's rim, a wall:
/// `WALL_D0..WALL_D1` calm, waves come up to it) and to low shore only (a beach's waterline band: `SHORE_D0..SHORE_D1`).
/// [PC] The fifth to eighth quarters keep the first two per world cell over frames (`cs_shore_keep`: no flicker at a beach).
pub(super) const SHORE_SHADER: &str = r#"
struct SU { ivp: mat4x4<f32>, vp: mat4x4<f32>, map: vec4<f32>, size: vec4<f32> };
@group(0) @binding(0) var<uniform> s: SU;
@group(0) @binding(1) var bed: texture_depth_multisampled_2d;
@group(0) @binding(2) var<storage, read_write> shore: array<atomic<u32>>;
fn okey(z: f32) -> u32 {
	let b = bitcast<u32>(z);
	return select(b | 0x80000000u, ~b, (b & 0x80000000u) != 0u);
}
@compute @workgroup_size(8, 8) fn cs_shore(@builtin(global_invocation_id) id: vec3<u32>) {
	let dim = textureDimensions(bed);
	let px = id.xy * u32(s.size.x);
	if (any(px >= dim)) { return; }
	let d = textureLoad(bed, vec2<i32>(px), 0);
	if (d >= 0.99999) { return; }
	let uv = (vec2<f32>(px) + 0.5) / vec2<f32>(dim);
	let q = s.ivp * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
	let p = q.xyz / q.w;
	let n = i32(s.size.y);
	let c = vec2<i32>(floor((p.xy - s.map.xy) / s.map.z));
	if (any(c < vec2<i32>(0)) || any(c >= vec2<i32>(n))) { return; }
	let i = c.y * n + c.x;
	// [PC] Ground at the surface (s.size.z..w: a little under the highest sea plane to well above it, a bank seen from above
	// with its waterline hidden under its edge) marks its cell for the distance map (`cs_shore_h`).
	let k = okey(p.z);
	if (p.z >= s.size.z && p.z <= s.size.w && atomicLoad(&shore[n * n + i]) < k) { atomicMax(&shore[n * n + i], k); }
	if (p.z > s.map.w) { return; }
	// (Most pixels of a near cell hold no new maximum: a plain read first keeps them off the atomic.)
	if (atomicLoad(&shore[i]) >= k) { return; }
	atomicMax(&shore[i], k);
}
// [PC] The map kept over frames (`cs_shore_keep`, after `cs_shore`, before the distance map). The map is made from what the camera
// sees of the bed this frame, every SHORE_STRIDE-th pixel: a cell far off, at a grazing angle or behind a bank's lip got a sample
// in one frame and none in the next, so its mark and the shore distances round it came and went as the camera or the hero moved
// and the calm at a beach flickered instead of fading (user report 2026-10-24, Pokitaru, hero 488.8 478.5). Per world cell (a
// slot of the buffer's fifth to eighth quarters, addressed by the cell's world index mod SHORE_N, so it stays with the ground
// while the camera-centred window slides; a tag tells the cell that last used the slot) the last seen keys of the first two
// quarters are kept: a cell seen now takes what it shows (a door that moved), a cell not seen now keeps its keys while it holds the
// slot (was 60 frames: when they ran out a patch of calm switched in one frame ~1 s after a pan, user 2026-10-25; still ground only).
const SHORE_KEEP: u32 = 4000000000u;
@compute @workgroup_size(8, 8) fn cs_shore_keep(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = i32(s.size.y);
	let c = vec2<i32>(id.xy);
	if (any(c >= vec2<i32>(n))) { return; }
	let wc = vec2<i32>(round(s.map.xy / s.map.z)) + c;
	let sl = ((wc % n) + n) % n;
	let nn = n * n;
	let j = sl.y * n + sl.x;
	let i = c.y * n + c.x;
	let tag = (u32(wc.x) & 0x7fffu) | ((u32(wc.y) & 0x7fffu) << 15u) | 0x40000000u;
	let k0 = atomicLoad(&shore[i]);
	let k1 = atomicLoad(&shore[nn + i]);
	if (atomicLoad(&shore[4 * nn + j]) != tag || k0 != 0u || k1 != 0u) {
		atomicStore(&shore[4 * nn + j], tag);
		atomicStore(&shore[5 * nn + j], k0);
		atomicStore(&shore[6 * nn + j], k1);
		atomicStore(&shore[7 * nn + j], select(SHORE_KEEP, 0u, k0 == 0u && k1 == 0u));
		return;
	}
	let left = atomicLoad(&shore[7 * nn + j]);
	if (left == 0u) { return; }
	atomicStore(&shore[7 * nn + j], left - 1u);
	atomicStore(&shore[i], atomicLoad(&shore[5 * nn + j]));
	atomicStore(&shore[nn + i], atomicLoad(&shore[6 * nn + j]));
}
// [PC] The distance map, an exact distance transform over SHORE_R cells in two passes: per cell the distances (cells) along its
// row to the nearest marked cell and the nearest low one (SHORE_R + 1 = none; u16 | u16), then the nearest over its column
// (centre to centre), stored + 1 as two f16 (pack2x16float; 0 = not made).
const SHORE_R: i32 = 10;
// [PC] Low shore: no ground within 2 cells higher than SHORE_LOW under the height cap (s.map.w = the highest plane + SHORE_CAP),
// all of them in view (`shore_seen`).
const SHORE_LOW: f32 = 0.4;
// [PC] Low shore is claimed only where the camera sees all the ground that could deny it: the 5 x 5 cells round the mark (the
// low test's reach), from a little under the highest sea plane to the height cap (s.size.z .. s.map.w), lie inside the view.
// On a fast turn a dock's foot at the waterline came into view at the screen's edge a frame before the dock beside it, so for
// that frame it was low shore and calmed the sea SHORE_D1 round it: a big glassy patch over open water between frames with
// waves (user report 2026-10-25, Pokitaru's cove, "glassy vs bigger waves" while turning). Partly off the view a mark counts
// as any ground (`shore_dist` x: WALL_D0..WALL_D1).
fn shore_seen(c: vec2<i32>) -> bool {
	for (var k = 0; k < 8; k++) {
		let xy = s.map.xy + vec2<f32>(c + vec2<i32>(select(-2, 3, (k & 1) != 0), select(-2, 3, (k & 2) != 0))) * s.map.z;
		let q = s.vp * vec4<f32>(xy, select(s.size.z, s.map.w, (k & 4) != 0), 1.0);
		if (q.w <= 0.0 || abs(q.x) > q.w || abs(q.y) > q.w) { return false; }
	}
	return true;
}
@compute @workgroup_size(8, 8) fn cs_shore_h(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = i32(s.size.y);
	let c = vec2<i32>(id.xy);
	if (any(c >= vec2<i32>(n))) { return; }
	// The highest mark over 5 rows per column of the window (2 cells round it each side).
	var cm: array<u32, 25>;
	for (var k = 0; k < 2 * SHORE_R + 5; k++) {
		let x = c.x + k - SHORE_R - 2;
		var m = 0u;
		if (x >= 0 && x < n) {
			for (var j = max(c.y - 2, 0); j <= min(c.y + 2, n - 1); j++) { m = max(m, atomicLoad(&shore[n * n + j * n + x])); }
		}
		cm[k] = m;
	}
	let lowk = okey(s.map.w - SHORE_LOW);
	var best = SHORE_R + 1;
	var low = SHORE_R + 1;
	for (var k = -SHORE_R; k <= SHORE_R; k++) {
		let x = c.x + k;
		if (x >= 0 && x < n && atomicLoad(&shore[n * n + c.y * n + x]) != 0u) {
			best = min(best, abs(k));
			let q = k + SHORE_R + 2;
			if (max(max(max(cm[q - 2], cm[q - 1]), max(cm[q], cm[q + 1])), cm[q + 2]) <= lowk && abs(k) < low && shore_seen(vec2<i32>(x, c.y))) { low = min(low, abs(k)); }
		}
	}
	atomicStore(&shore[2 * n * n + c.y * n + c.x], u32(best) | (u32(low) << 16u));
}
@compute @workgroup_size(8, 8) fn cs_shore_v(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = i32(s.size.y);
	let c = vec2<i32>(id.xy);
	if (any(c >= vec2<i32>(n))) { return; }
	var d2 = vec2<f32>(f32((SHORE_R + 1) * (SHORE_R + 1)));
	for (var k = -SHORE_R; k <= SHORE_R; k++) {
		let y = c.y + k;
		if (y >= 0 && y < n) {
			let r = atomicLoad(&shore[2 * n * n + y * n + c.x]);
			let h = vec2<f32>(f32(r & 0xffffu), f32(r >> 16u));
			d2 = min(d2, h * h + f32(k * k));
		}
	}
	atomicStore(&shore[3 * n * n + c.y * n + c.x], pack2x16float(sqrt(d2) + 1.0));
}
"#;

/// [PC] `shore_dist` and `shore_calm` (WGSL, appended to `SHADER` in `WaterFx::new`): the water shader's read of the distance map
/// and the shore calm made from it.
pub(super) const SHORE_DIST_WGSL: &str = r#"
// [PC] At world XY: the distances (world units, bilinear) to the nearest cell with ground at the surface (x: any ground, a dock
// rim or wall too; y: low shore only, a beach's waterline band) (`SHORE_SHADER` `cs_shore_v`; SHORE_FAR = none near).
// Where no ground is seen under the water (shallows over a level with no floor there, a bank seen from above: Pokitaru's
// beaches) this is all that tells the shore is near.
const SHORE_FAR: f32 = 11.0;
fn shore_dist(xy: vec2<f32>) -> vec2<f32> {
	let n = i32(w.gmap.w);
	let f = (xy - w.gmap.xy) / w.gmap.z - 0.5;
	let i = vec2<i32>(floor(f));
	let fr = f - floor(f);
	var d: array<vec2<f32>, 4>;
	for (var k = 0; k < 4; k++) {
		let c = i + vec2<i32>(k & 1, k >> 1);
		d[k] = vec2<f32>(SHORE_FAR);
		if (all(c >= vec2<i32>(0)) && all(c < vec2<i32>(n))) {
			let key = shore[3 * n * n + c.y * n + c.x];
			if (key != 0u) { d[k] = unpack2x16float(key) - 1.0; }
		}
	}
	return mix(mix(d[0], d[1], fr.x), mix(d[2], d[3], fr.x), fr.y) * w.gmap.z;
}

// [PC] The shore calm (`calm`): sd = `shore_dist`, hc = the calm by the depth of the seen ground under the point, seen = how much
// of it was seen (`shore_at` y). Seen ground gives the calm by its depth, unseen the distance fades (SHORE_D0..D1 to low shore,
// WALL_D0..WALL_D1 to any ground), and the low shore's distance fade holds over seen ground too: where a beach drops steeply
// the depth calm ended in a hard edge (full waves next to none) that moved with the seen cells as the camera or the hero
// moved, so the waves at Pokitaru's beaches looked choppy and flickered instead of fading out (user report 2026-10-24).
fn shore_calm(sd: vec2<f32>, hc: f32, seen: f32) -> f32 {
	let low = smoothstep(SHORE_D0, SHORE_D1, sd.y);
	return min(low, mix(min(low, smoothstep(WALL_D0, WALL_D1, sd.x)), hc, smoothstep(0.0, 0.25, seen)));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct ShoreU {
	pub(super) ivp: [[f32; 4]; 4],
	/// The view-projection `ivp` inverts (`shore_seen`).
	pub(super) vp: [[f32; 4]; 4],
	/// Origin x, y, cell size, highest height kept.
	pub(super) map: [f32; 4],
	/// Stride (bed pixels), cells across.
	pub(super) size: [f32; 4],
}

/// [PC] Geometric sea waves: sea planes drawn at most; the shore map's cells across, cell size (units), bed pixel stride and how
/// far above the sea a point still counts as ground at the surface.
pub(super) const GEO_PLANES: usize = 4;
pub(super) const SHORE_N: u32 = 256;
pub(super) const SHORE_CELL: f32 = 1.0;
pub(super) const SHORE_STRIDE: u32 = 2;
pub(super) const SHORE_CAP: f32 = 1.0;
pub(super) const SHORE_NEAR: f32 = 0.3;
pub(super) const SHORE_HIGH: f32 = 3.5;
/// A moby that moved (units, or turned: radians) within SHORE_STILL frames (about a second) stays out of the shore map.
pub(super) const SHORE_MOVED: f32 = 1e-3;
pub(super) const SHORE_STILL: u32 = 50;
/// The projected grid's margins beyond the screen (NDC: sides, bottom, top): crests rising in from below the screen's edge.
pub(super) const GRID_MARGIN: [f32; 3] = [0.06, 0.35, 0.12];

#[cfg(test)]
mod tests {
	/// The shore map's compute shader parses and validates, with its four passes.
	#[test]
	fn shore_shader_parses() {
		let m = naga::front::wgsl::parse_str(super::SHORE_SHADER).expect("shore shader");
		for n in ["cs_shore", "cs_shore_keep", "cs_shore_h", "cs_shore_v"] {
			assert!(m.entry_points.iter().any(|e| e.name == n), "{n}");
		}
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("shore shader validates");
	}
}
