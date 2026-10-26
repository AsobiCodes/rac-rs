//! The moby as the interpreter left it (`Moby`), the 4:3 side-test and cull inputs (`SideOnly`, `Culls`, `DrawView`),
//! the animation clock, the hero's attached parts, and the readers of the hero's joints, layers and draw basis.

use super::*;

/// One moby as the interpreter left it after a field.
pub struct Moby {
	pub addr: u32,
	pub class: u32,
	pub pos: [f32; 3],
	pub rot: [f32; 3],
	/// The draw basis is mirrored in model Y (+0x34 bit 0x8000 negates basis row 1): `rot` is the plain rotation (`basis_pose`).
	pub mirror: bool,
	/// Scale over the class header's base scale.
	pub scale: f32,
	pub shown: bool,
	/// Retail culled it only at the 4:3 side plane (`DrawView::culls`): rac-rs shows it when its wider plane keeps it.
	pub side_only: Option<SideOnly>,
	/// [PC] Why retail did not draw it this field when it is still in the world (`Culls::why`): `View` = the game camera alone, so
	/// it keeps casting its sun shadow (viewer/sunshadow.rs) and Culling Off draws it; `Far` = past its draw distance from the
	/// game camera, drawn (and casting) with Culling Off only.
	pub view_culled: Culled,
	/// The 4:3 draw tests as rac-rs reproduces them (tests compare them with `+0x31`).
	#[cfg(test)]
	pub culls: Option<Culls>,
	/// (+0x34 flags, +0x32 draw distance, draw-distance cap read apart from `DrawView`), for the tests' own recheck of the rule.
	#[cfg(test)]
	pub raw: (u16, i16, i32),
	pub rgb: [u8; 3],
	pub light: u32,
	pub clock: Clock,
	/// Its +0x64 joint override records (`joint_records`, empty when none): the joint evaluator FUN_00268a80 applies every
	/// evaluated moby's list (hero_movement.md HERO-17b) [C]; e.g. Clank's (class 601, `*0x1404d4`) blink records.
	pub fx: Vec<crate::anim::JointFx>,
	/// Draw alpha byte +0x23 (moby header colour word = min(fade, 0x80) * +0x23 >> 7, moby_format.md CLASS-tint-alpha). [C]
	pub alpha: u8,
	/// Draw distance +0x32 (whole metres; the draw fade of the header alpha). [C]
	pub draw: i16,
	/// The tail-packet colour +0x90 while flag +0x34 & 0x10 is set (FUN_00263020 init = class +0x40; overrides by actors). [C]
	pub ovr: Option<u32>,
}

/// The inputs of the 4:3 side tests of a moby that retail's draw rejected for a side plane (x or y) and nothing else: view-space
/// x, y and depth of its bounding sphere, its radius (all 1/1024 m) and the vertical tangent rac-rs renders with (`tan_v`: the retail
/// horizontal scalar times `PROJ_Y_RATIO`, as `self.cam.tan_half`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideOnly {
	pub(super) vx: f32,
	vy: f32,
	pub(super) vz: f32,
	pub(super) w: f32,
	pub(super) tan_v: f32,
}

impl SideOnly {
	/// Retail's horizontal reject (`tx * z - (|x| - sx * w)` with its sign bit set) with the plane of a window `aspect` wide:
	/// the same vertical tangent, so the horizontal one is `tan_v * aspect`. REN-MOBY-DRAWN keeps the near / far / vertical tests.
	pub fn inside(&self, aspect: f32) -> bool {
		self.inside_lag(aspect, ViewLag::NONE)
	}

	/// [PC] `inside` for a drawn view that trails the tick's: both side planes turned out by `lag.ang` radians and the sphere
	/// grown by `lag.grow` (1/1024 m). A smoothed frame's camera lies between the last two ticks (`Retail::camera`) while
	/// retail's verdict is the newest tick's, so without it a moby at the trailing screen edge vanishes while still on screen.
	pub fn inside_lag(&self, aspect: f32, lag: ViewLag) -> bool {
		let turn = |t: f32| if lag.ang > 0.0 { (t.atan() + lag.ang).min(1.45).tan() } else { t };
		let (tx, ty) = (turn(self.tan_v * aspect), turn(self.tan_v));
		let (sx, sy) = ((1.0 + tx * tx).sqrt(), (1.0 + ty * ty).sqrt());
		let w = self.w + lag.grow;
		!(tx * self.vz - (self.vx.abs() - sx * w)).is_sign_negative() && !(ty * self.vz - (self.vy.abs() - sy * w)).is_sign_negative()
	}
}

/// [PC] How far the drawn view can trail the newest tick's (`SideOnly::inside_lag`, `Retail::view_lag`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewLag {
	/// Radians the side planes turn out.
	pub ang: f32,
	/// Growth of the sphere radius, 1/1024 m.
	pub grow: f32,
}

impl ViewLag {
	pub const NONE: ViewLag = ViewLag { ang: 0.0, grow: 0.0 };
}

/// [PC] Why retail's draw left out a moby that is not hidden (`Moby::view_culled`, `Culls::why`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Culled {
	/// Drawn, hidden (+0x34 & 0x81), or not tested.
	#[default]
	No,
	/// Only by a camera test (occlusion mask, near or side plane).
	View,
	/// By its draw distance (+0x32 under the cap) from the game camera, maybe a camera test too.
	Far,
}

/// What the retail moby draw (`moby_draw_build` FUN_0026a338 and its overlay twins, REN-MOBY-DRAWN in decomp/notes) tests, one
/// flag per criterion, true = rejects. `+0x31` is 1 when none of them does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Culls {
	occ: bool,
	pub(super) hidden: bool,
	pub(super) dist: bool,
	/// [PC] The draw distance (`+0x32` under the cap) is 0 or less: nothing in front of the camera is inside it, so retail draws the
	/// moby only while the camera sits in its bounding sphere (an invisible helper, e.g. Veldin's class 1964, the grey walls of the
	/// first landing pad). Not a reject itself; see `why`.
	pub(super) never: bool,
	pub(super) near: bool,
	side_x: bool,
	side_y: bool,
}

impl Culls {
	#[cfg(test)]
	pub(super) fn any(&self) -> bool {
		self.occ || self.hidden || self.dist || self.near || self.side_x || self.side_y
	}

	/// [PC] Rejected only by a camera test (occlusion mask, near or side plane), not hidden (+0x34 & 0x81) and not past its draw distance.
	pub(super) fn view_only(&self) -> bool {
		(self.occ || self.near || self.side_x || self.side_y) && !(self.hidden || self.dist)
	}

	/// [PC] `Culled` of a moby retail did not draw: `View` when `view_only`, `Far` when past its draw distance and not hidden.
	pub(super) fn why(&self) -> Culled {
		// A moby with no draw distance is hidden by design, whatever way the camera turns (it shows no side / near verdict that
		// Culling Off may overrule).
		if self.never {
			return Culled::No;
		}
		match (self.view_only(), self.dist && !self.hidden) {
			(true, _) => Culled::View,
			(false, true) => Culled::Far,
			_ => Culled::No,
		}
	}

	/// Rejected by a side plane (horizontal or vertical) and by nothing else.
	pub(super) fn side_only(&self) -> bool {
		(self.side_x || self.side_y) && !(self.occ || self.hidden || self.dist || self.near)
	}
}

/// The camera side of the retail moby draw, from the overlay's globals: eye, the world-to-view rows at 0x167080, the
/// projection scalars 0x16cef0 / 0x16cef4 and the 128-byte occlusion mask at 0x174100 (Novalis addresses plus the level shift).
pub(super) struct DrawView {
	pub(super) eye: [f32; 3],
	pub(super) rows: [[f32; 3]; 3],
	pub(super) tx: f32,
	pub(super) ty: f32,
	pub(super) near: f32,
	/// Stored `sqrt(1 + t^2)` of each tangent (vf22 of the retail draw, 0x16d020 / 0x16d024 plus the level shift).
	pub(super) sx: f32,
	pub(super) sy: f32,
	/// The draw-distance cap in metres, `DAT_0015ff70` (`DAT_0015ffb0` in levels 0, 5, 14, 16): 500 at level setup, 0x90 during
	/// Visibomb flight, 0x40 with the debug toggle. Applied with a signed `pminw` to `+0x32`.
	pub(super) cap: i32,
	pub(super) mask: [u8; 128],
}

impl DrawView {
	pub(super) fn read(m: &ee_oracle::Machine, level: u32) -> Option<DrawView> {
		let d = *LEVEL_SHIFT.get(level as usize)?;
		let at = |a: u32| va(a.wrapping_add_signed(d));
		Some(DrawView {
			eye: m.rdv3(at(0x1671c0)),
			rows: [m.rdv3(at(0x167080)), m.rdv3(at(0x167090)), m.rdv3(at(0x1670a0))],
			tx: m.rdf(at(0x16cef0)),
			ty: m.rdf(at(0x16cef4)),
			near: m.rdf(at(0x16cee0)),
			sx: m.rdf(at(0x16d020)),
			sy: m.rdf(at(0x16d024)),
			cap: m.rd32(if matches!(level, 0 | 5 | 14 | 16) { va(0x15ffb0) } else { va(0x15ff70) }) as i32,
			mask: std::array::from_fn(|i| m.rd8(at(0x174100) + i as u32)),
		})
	}

	/// The retail draw's tests of the moby at `a` (asm 0x26a6c4-0x26a7b8 in Novalis): occlusion `(+0x36 & mask[+0x37])`, hidden
	/// `+0x34 & 0x81`, draw distance `((min(+0x32, cap) << 10) - w) - (z - w)`, near `32 - (w + z)`, side `t * z - (|v| - s * w)`.
	/// Returns them with the sphere's view-space (x, z) and radius.
	pub(super) fn culls(&self, m: &ee_oracle::Machine, a: u32) -> (Culls, SideOnly) {
		let (c, w) = (m.rdv3(a), m.rdf(a + 0xc));
		let d: [f32; 3] = std::array::from_fn(|k| c[k] - self.eye[k] * 1024.0);
		// vf25..vf27 are the columns of the world-to-view matrix (VMULAbc / VMADDAbc / VMADDbc by d.x, d.y, d.z).
		let dot = |k: usize| (d[0] * self.rows[0][k] + d[1] * self.rows[1][k]) + d[2] * self.rows[2][k];
		let (vx, vy, vz) = (dot(0), dot(1), dot(2));
		let (tx, ty) = (self.tx, self.ty);
		let (sx, sy) = (self.sx, self.sy);
		let limit = ((m.rd16(a + 0x32) as i16 as i32).min(self.cap) << 10) as f32;
		let culls = Culls {
			occ: m.rd8(a + 0x36) & self.mask[m.rd8(a + 0x37) as usize & 0x7f] == 0,
			hidden: m.rd16(a + 0x34) & 0x81 != 0,
			dist: ((limit - w) - (vz - w)).is_sign_negative(),
			never: limit <= 0.0,
			near: !(self.near - (w + vz)).is_sign_negative(),
			side_x: (tx * vz - (vx.abs() - sx * w)).is_sign_negative(),
			side_y: (ty * vz - (vy.abs() - sy * w)).is_sign_negative(),
		};
		(culls, SideOnly { vx, vy, vz, w, tan_v: tx * crate::viewer::PROJ_Y_RATIO })
	}
}

/// Moby `+0x50..+0x5f`: cur, next, playing / requested sequence, progress, multiplier, rate (`PropAnim::mirror`).
#[derive(Clone, Copy, Default, Debug)]
pub struct Clock {
	pub cur: u8,
	pub next: u8,
	pub old_seq: u8,
	pub new_seq: u8,
	pub p: f32,
	pub m: f32,
	pub r: f32,
}

impl Clock {
	pub(super) fn read(m: &ee_oracle::Machine, a: u32) -> Clock {
		Clock { cur: m.rd8(a + 0x50), next: m.rd8(a + 0x51), old_seq: m.rd8(a + 0x52), new_seq: m.rd8(a + 0x53), p: m.rdf(a + 0x54), m: m.rdf(a + 0x58), r: m.rdf(a + 0x5c) }
	}

	pub fn apply(&self, a: &mut crate::anim::PropAnim) {
		a.mirror(self.cur, self.next, self.old_seq, self.new_seq, self.p, self.m, self.r);
	}
}

/// Retail's held wrench (class 71 at `HELD_MOBY`; the crank 0x2997a0 and the held-object mirror 0x224fb8 write its clock):
/// its moby, its clock, and whether it sits in the hand (+0x34 bit 2 set and the slot-0 detached byte 0x1403fa clear; the
/// Comet-Strike throw, hero state 0x15, clears both while it flies) [C: hero_anims.md, dump w0006 f714-757].
#[derive(Clone, Copy)]
pub struct HeldItem {
	pub addr: u32,
	pub clock: Clock,
	pub attached: bool,
}

/// A class-71 moby retail draws away from the hand: the thrown wrench (the detached held moby) and its two afterimage
/// copies (+0x34 0x080a). World vertex = `pos + scale * (x rows[0] + y rows[1] + z rows[2])` from +0x10 and the rows
/// +0xc0..+0xe0, `scale` = +0x2c over the class base; `alpha` = the draw alpha byte +0x23 (moby_format.md: vertex alpha
/// `min(fade, 0x80) * +0x23 >> 7`).
#[derive(Clone, Copy)]
pub struct LooseWrench {
	pub addr: u32,
	pub pos: [f32; 3],
	pub rows: [[f32; 3]; 3],
	pub scale: f32,
	pub alpha: u8,
	pub clock: Clock,
	pub(super) shown: bool,
	pub(super) side_only: Option<SideOnly>,
}

/// The hero's attached parts as retail poses them in one field: the held wrench, the back objects d0 `*0x1404d0`
/// (class 607) and d4 `*0x1404d4` (Clank, 601), the +0x64 joint override records of the hero and of d4, and the loose
/// class-71 mobys.
#[derive(Clone, Default)]
pub struct HeroParts {
	pub wrench: Option<HeldItem>,
	pub d0: Option<Clock>,
	pub d4: Option<Clock>,
	pub hero_fx: Vec<crate::anim::JointFx>,
	/// Render-only pose overrides of the hero (anim_pose.rs: mirror ops, ...), applied after `hero_fx` and the strafe legs; empty
	/// until a host store fills them (OmniLoader pose block, stage B). Never written to RAM.
	pub pose_ops: Vec<crate::anim::JointFx>,
	/// Follow poses of this field (omni/pose_follow.rs): props the renderer draws at a hero joint (viewer/follow.rs). Picture only.
	pub follow: Vec<crate::omni::FollowRec>,
	/// Modern-controls torso twist / backpedal of this field (strafe.rs; default = none).
	pub strafe: strafe::Draw,
	/// The hero +0x60 weighted layers (gadget / weapon hold arms) in chain order (`layer_list`).
	pub layers: Vec<crate::anim::ListLayer>,
	pub clank_fx: Vec<crate::anim::JointFx>,
	pub loose: Vec<LooseWrench>,
	/// The hero's FROM frame while its clock fades from a snapshot (+0x52 == 0xff): the frame-format bytes at +0x68, the
	/// snapshot slot `0x18f0c0 + (+0x50) * 0x800` that hero SetAnim filled through FUN_00266660 (moby_bind_frames
	/// FUN_002632b0, NOV OV:54124-54152) [C]; empty otherwise.
	pub from_frame: Vec<u8>,
}

/// M3a: retail evaluates the hero's joints (the draw matrices and the held-item attach 0x13fe40) BEFORE the hero update
/// steps the +0x60 layer list (hold clock FUN_0022e128, layer pose rebuild FUN_00267308), so the field's picture uses the
/// layer list as the previous field left it [C measured, window eudora1_0002 + w0002.racpad, 331 fields: wrist joint 56
/// vs 0x13fe40 p90 0.033 with this field's list, 0.0001 with the previous field's; the previous field's weights alone 0.032,
/// t alone 0.020]. Swaps `s.parts.layers` for the list kept from the previous snap (empty after a level change).
pub(super) fn lag_layers(s: &mut Snap, used: &mut (u32, Vec<crate::anim::ListLayer>)) {
	let now = std::mem::take(&mut s.parts.layers);
	let prev = std::mem::replace(used, (s.level, now));
	if prev.0 == s.level {
		s.parts.layers = prev.1;
	}
}

/// The joint override records of a moby's +0x64 chain (next +8, capped at 64) in chain order, the order retail applies
/// them [C: hero_anims.md "+0x64 record layout", dump w0002b]: +3 mode (0 compose, 1 weighted), +4 dest = scratchpad
/// joint matrix 0x70000000 + joint * 0x40 (the skeleton joint itself), +0xc weight (mode 1), +0x10 quaternion xyzw (the
/// port's `JointFx.rot` is its conjugate, headpose.rs `block_quat`), +0x20 scale, +0x30 offset. `reg` keeps chain order
/// under the descending sort.
pub(super) fn joint_records(m: &ee_oracle::Machine, moby: u32) -> Vec<crate::anim::JointFx> {
	let mut v = Vec::new();
	let mut r = m.rd32(moby + 0x64) & 0x01ff_ffff;
	while r != 0 && v.len() < 64 {
		let q = m.rdv3(r + 0x10);
		v.push(crate::anim::JointFx {
			joint: ((m.rd32(r + 4) & 0xffff) / 0x40) as usize,
			rot: [-q[0], -q[1], -q[2], m.rdf(r + 0x1c)],
			scale: m.rdv3(r + 0x20),
			off: m.rdv3(r + 0x30),
			blend: (m.rd8(r + 3) != 0).then(|| m.rdf(r + 0xc)),
			reg: 0,
			mirror: None,
		});
		r = m.rd32(r + 8) & 0x01ff_ffff;
	}
	let n = v.len() as u32;
	for (i, f) in v.iter_mut().enumerate() {
		f.reg = n - i as u32;
	}
	v
}

/// M3a F5: the hero's +0x60 weighted layer objects (next +0x1c, at most 6: the pool of anim_layer_alloc FUN_002639a0) in
/// chain order, the order the joint evaluator applies them [C: hero_movement.md HERO-24 "Draw"; layout verified on the
/// r1250 RAM image: jtab 12 object +2 = 20, +0xc = the jtab-12 joint list 52..71]. A frame flagged +6 / +7 comes from
/// the hold set of the hold slot `*(0x140050 + 4i)` (i = 0 class 1, i = 1 class 2) whose object it is, or whose hold
/// model +0x18 it shares [I: a flagged non-slot object was not seen].
/// `HeroParts::from_frame`: header (16 bytes, body size u16 +6 in quadwords) plus body, capped at the 0x800-byte slot.
pub(super) fn snapshot_frame(m: &ee_oracle::Machine, moby: u32) -> Vec<u8> {
	let f = m.rd32(moby + 0x68) & 0x01ff_ffff;
	if m.rd8(moby + 0x52) != 0xff || f == 0 {
		return Vec::new();
	}
	let n = (16 + m.rd16(f + 6) as u32 * 16).min(0x800);
	(0..n).map(|i| m.rd8(f + i)).collect()
}

pub(super) fn layer_list(m: &ee_oracle::Machine, hero: u32) -> Vec<crate::anim::ListLayer> {
	let slots = [0u32, 1].map(|i| m.rd32(0x140050 + 4 * i) & 0x01ff_ffff);
	let mut v = Vec::new();
	let mut o = m.rd32(hero + 0x60) & 0x01ff_ffff;
	while o != 0 && v.len() < 6 {
		let set = slots.iter().position(|&s| s == o).or_else(|| {
			let model = m.rd32(o + 0x18);
			slots.iter().position(|&s| s != 0 && model != 0 && m.rd32(s + 0x18) == model)
		});
		let list = m.rd32(o + 0xc) & 0x01ff_ffff;
		let n = m.rd16(o + 2).min(128) as u32;
		v.push(crate::anim::ListLayer {
			pose: crate::anim::LayerPose {
				seq: m.rd8(o + 0x23) as usize,
				frame: m.rd8(o + 0x21) as usize,
				from_seq: m.rd8(o + 0x22) as usize,
				from_frame: m.rd8(o + 0x20) as usize,
				t: m.rdf(o + 0x24),
				weight: m.rdf(o + 8),
			},
			joints: (0..n).map(|k| m.rd8(list + k) as usize).collect(),
			from_hold: set.filter(|_| m.rd8(o + 6) != 0),
			to_hold: set.filter(|_| m.rd8(o + 7) != 0),
		});
		o = m.rd32(o + 0x1c) & 0x01ff_ffff;
	}
	v
}

/// rac-rs Euler angles (viewer `place`) of the draw basis rows the update leaves at moby `+0xc0..+0xef` (actor_transform_bsphere
/// FUN_00265770: Euler of `+0x40`, or the rows themselves under `+0x34` bit 0x100, as for held items on a hero joint).
pub(super) fn basis_euler(m: &ee_oracle::Machine, a: u32) -> [f32; 3] {
	basis_pose(m, a, false).0
}

/// `basis_euler` plus the mirror of the draw basis (`unmirror`): +0x34 bit 0x8000 makes the standard transform negate basis row 1
/// (actor_transform_bsphere OV:56461-56467, collision_and_gameplay.md), a reflection in model Y that Euler angles cannot carry (the
/// class-331 right track of the Veldin return tank is the left track's model so drawn). Then the angles come from the rows with
/// row 1 negated back, i.e. the plain rotation, and the caller applies the mirror on top (`Prop::mirror`, viewer `place_mirrored`).
pub(super) fn basis_pose(m: &ee_oracle::Machine, a: u32, unmirror: bool) -> ([f32; 3], bool) {
	let n = |v: [f32; 3]| {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		if l > 0.0 { v.map(|x| x / l) } else { v }
	};
	let (r0, mut r1, r2) = (n(m.rdv3(a + 0xc0)), n(m.rdv3(a + 0xd0)), n(m.rdv3(a + 0xe0)));
	if r0 == [0.0; 3] {
		return (m.rdv3(a + 0x40), false);
	}
	// Left-handed rows (r0 . r1 x r2 < 0) under the flag: the update really negated row 1 (not the 0x100 supplied-rows path).
	let det = r0[0] * (r1[1] * r2[2] - r1[2] * r2[1]) - r0[1] * (r1[0] * r2[2] - r1[2] * r2[0]) + r0[2] * (r1[0] * r2[1] - r1[1] * r2[0]);
	let mirror = unmirror && m.rd16(a + 0x34) & 0x8000 != 0 && det < -0.5;
	if mirror {
		r1 = r1.map(|x| -x);
	}
	(euler_from_rows(r0, r1, r2), mirror)
}

/// Euler angles `[x, y, z]` of the unit basis rows (row 0 = `[cy cz, cy sz, -sy]`, row 1 z = `sx cy`, row 2 z = `cx cy`). At the
/// gimbal lock (`cy` ~ 0: a moby rolled to stand on its side, e.g. the Hoven wall plates with row 0 = `[0, 0, -1]`) the rows
/// carry only `x - z` (`y > 0`) or `x + z` (`y < 0`): the old atan2 of two zeros gave x = z = 0 and turned the moby 90 degrees. There x
/// = 0 and z comes from row 1, the same matrix.
fn euler_from_rows(r0: [f32; 3], r1: [f32; 3], r2: [f32; 3]) -> [f32; 3] {
	let y = (-r0[2]).clamp(-1.0, 1.0).asin();
	if r0[0].hypot(r0[1]) < 1e-4 {
		return [0.0, y, -r1[0].atan2(r1[1])];
	}
	[r1[2].atan2(r2[2]), y, r0[1].atan2(r0[0])]
}

#[cfg(test)]
mod euler_tests {
	use super::euler_from_rows;

	/// The rows the viewer's Euler order builds: row 0 `[cy cz, cy sz, -sy]`, row 1 `[sx sy cz - cx sz, sx sy sz + cx cz, sx cy]`,
	/// row 2 `[cx sy cz + sx sz, cx sy sz - sx cz, cx cy]`.
	fn rows(x: f32, y: f32, z: f32) -> [[f32; 3]; 3] {
		let (sx, cx, sy, cy, sz, cz) = (x.sin(), x.cos(), y.sin(), y.cos(), z.sin(), z.cos());
		[[cy * cz, cy * sz, -sy], [sx * sy * cz - cx * sz, sx * sy * sz + cx * cz, sx * cy], [cx * sy * cz + sx * sz, cx * sy * sz - sx * cz, cx * cy]]
	}

	#[test]
	fn euler_round_trip_keeps_the_matrix_including_gimbal_lock() {
		let half = std::f32::consts::FRAC_PI_2;
		for (x, y, z) in [(0.3, 0.2, -1.1), (0.0, half, 1.982), (0.0, -half, -2.4), (0.7, half, 0.4), (-1.2, -half, 2.9), (0.5, half - 1e-5, 0.1)] {
			let r = rows(x, y, z);
			let e = euler_from_rows(r[0], r[1], r[2]);
			let b = rows(e[0], e[1], e[2]);
			for i in 0..3 {
				for j in 0..3 {
					assert!((r[i][j] - b[i][j]).abs() < 2e-3, "({x},{y},{z}) -> {e:?} row {i}[{j}]: {} vs {}", r[i][j], b[i][j]);
				}
			}
		}
	}
}

#[cfg(test)]
mod culled_tests {
	use super::*;

	/// A moby with no draw distance (`never`) is hidden by design: its side / near verdict is no camera-only reason, so Culling Off
	/// does not draw it (Veldin's class 1964, the grey walls of the first landing pad). The others keep their verdicts.
	#[test]
	fn a_moby_with_no_draw_distance_is_never_a_view_cull() {
		let side = Culls { side_x: true, ..Culls::default() };
		assert_eq!(side.why(), Culled::View);
		assert_eq!(Culls { never: true, ..side }.why(), Culled::No);
		assert_eq!(Culls { dist: true, ..Culls::default() }.why(), Culled::Far);
		assert_eq!(Culls { dist: true, never: true, ..Culls::default() }.why(), Culled::No);
		assert_eq!(Culls { hidden: true, side_x: true, ..Culls::default() }.why(), Culled::No);
	}
}
