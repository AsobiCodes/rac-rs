//! The Gadgetron vendor (class 11, FUN_002baa58) and its shop screen (decomp/notes/vendor.md, owner of
//! every number here). 50 Hz PAL ticks are seconds * 50. Tags: [P] = placeholder for something not decoded.

use super::Event;
use super::Game;
use super::gadget_scene::{GadgetRun, GadgetScene};
use crate::physics::{add, V3};
use crate::particles::WorldQuad;
use std::collections::HashMap;

pub(super) const VENDOR: u32 = 11;
pub(super) const ORB: u32 = 1143;
/// UI-5d (vendor.md): the purchase prompt model class 0x471 (FUN_00263020 OV:111023); the viewer reads it for BRIDGE-36 (c1).
/// (Named `PROMPT_CLASS` because `PROMPT` is the world prompt text id.)
pub const PROMPT_CLASS: u32 = 1137;
/// UI-5d: the bolt model slot offset in the vendor frame, gp-0x5bc0 = 0x161040 (vendor.md UI-5d).
const BOLT_SLOT: V3 = [-9.5, -5.85, 0.85];
/// NOV-30d: the panel-1 bolt icon (slot 0, class 13) offset in the vendor frame, gp-0x5bd0 = 0x161030 (vendor.md NOV-30d).
const ICON_SLOT: V3 = [-10.2, -4.25, 0.0];
/// NOV-30d: the ammo-row label drawn in panel 1 at (24, 24) (FUN_002b1838).
pub const TEXT_AMMO_LABEL: u32 = 0x4f5d;
pub const TEXT_BUY: u32 = 0x5234;
pub const TEXT_EXIT: u32 = 0x4ee0;
/// NOV-30h: the Exit panel's " No" while the popup offers an affordable weapon (FUN_002b1fb8).
pub const TEXT_NO: u32 = 0x524b;

const TICK: f32 = crate::gameplay::PAL_DT;
const OPEN_FADE: f32 = 0.34;

/// Glow layer 1 (FUN_002ba2f0, fx 0x18): the beam 9 vertices (y, z, u, v, alpha) from `0x1d76e0`, `0x1d7770`,
/// `0x1d77b8`, and its 4 quads from `0x1d77e0`.
const BEAM_V: [[f32; 5]; 9] = [
	[-1.0, 2.6, 0.0, 0.0, 0.125], [0.0, 2.6, 0.5, 0.0, 0.125], [1.0, 2.6, 1.0, 0.0, 0.125],
	[-0.3, 1.4, 0.0, 0.5, 0.875], [0.0, 1.6, 0.5, 0.5, 0.875], [0.3, 1.4, 1.0, 0.5, 0.875],
	[-0.05, 1.1, 0.0, 1.0, 0.875], [0.0, 1.1, 0.5, 1.0, 0.875], [0.05, 1.1, 1.0, 1.0, 0.875],
];
const BEAM_Q: [[usize; 4]; 4] = [[0, 1, 3, 4], [1, 2, 4, 5], [3, 4, 6, 7], [4, 5, 7, 8]];
const BEAM_TEX: usize = 0x18;
const RING_TEX: usize = 0x1b;

/// Skeletal play state of one model (FUN_0026c140 / FUN_0026c1f8): the joint spins and whether the sequence holds at its
/// end; the blend and clock live in the prop's `PropAnim` (decomp/notes/vendor.md "Vendor animation").
#[derive(Clone, Debug, Default)]
pub struct PoseFx {
	/// Joint z-spins (joint, radians) of the orb two joint overrides.
	pub spins: Vec<(usize, f32)>,
}

#[derive(Clone, Debug, Default)]
pub struct VendorFx {
	pub poses: HashMap<usize, PoseFx>,
}


/// Ticker text per item (class strings 0x525b / 0x5260).
pub fn item_text(item: u8) -> u32 {
	if item == 16 { 0x5260 } else { 0x525b }
}

/// One shop row: item id and whether it sells ammo (entry kind 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
	pub item: u8,
	pub ammo: bool,
}

/// NOV-30f (decomp/notes/vendor.md): shop HUD slot ids. Ammo slot ids are the item id; `BOLTS` is the bolt counter
/// element (slot 2) and `NORMAL` the on-foot ammo request (FUN_0024f530) that hands slot 0 back to the viewer ammo HUD.
pub const BOLTS: u16 = 0xfffe;
pub const NORMAL: u16 = 0xffff;

/// One HUD slot of the master (FUN_0024a900 / FUN_0024abf8 / FUN_0024f3f0): the committed element, the pending
/// request, the hold flag (0x10), lifetime, slide, fade bytes and the rolling display value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudSlot {
	pub active: Option<u16>,
	pub pending: Option<(u16, bool)>,
	pub hold: bool,
	pub req: Option<(u16, bool)>,
	pub lifetime: i32,
	pub slide: i32,
	pub f1: u8,
	pub f2: u8,
	pub acc: u8,
	pub display: i32,
}

impl Default for HudSlot {
	fn default() -> HudSlot {
		HudSlot { active: None, pending: None, hold: false, req: None, lifetime: 0, slide: -6, f1: 0, f2: 0, acc: 0, display: 0 }
	}
}

impl HudSlot {
	/// FUN_0024a900 with the hold flag (0x10).
	pub fn request(&mut self, id: u16) {
		self.request_flags(id, true);
	}

	/// FUN_0024a900: an identical request is a no-op; any other zeroes lifetime and the fade word and goes pending.
	pub fn request_flags(&mut self, id: u16, flag: bool) {
		if self.req == Some((id, flag)) {
			return;
		}
		self.req = Some((id, flag));
		self.pending = Some((id, flag));
		self.lifetime = 0;
		self.f1 = 0;
		self.f2 = 0;
		self.acc = 0;
	}

	/// FUN_0024abf8: drop the hold flag of the pending request, or of the active element when nothing is pending.
	pub fn release(&mut self) {
		self.req = self.req.map(|(i, _)| (i, false));
		match &mut self.pending {
			Some(p) => p.1 = false,
			None => self.hold = false,
		}
	}





	/// The slot's element is a shop counter (not the NORMAL handover).
	pub fn shop_owned(&self) -> bool {
		self.active.is_some_and(|a| a != NORMAL) || self.pending.is_some_and(|p| p.0 != NORMAL)
	}
}

/// NOV-30f: the shop's HUD slots 0 (ammo counter of the selected ammo row) and 2 (bolt counter), plus the ammo-HUD
/// suppress counter `0x15f940` and the bolt spin counter `0x15fa70`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShopHud {
	pub suppress: bool,
	pub ammo: HudSlot,
	pub bolts: HudSlot,
	pub spin: u8,
}


/// Novalis price rows (0x1c4430 + id*0x18): item price, ammo price, pickup/start/max ammo.
pub struct Price {
	pub item: u32,
	/// NOV-13.4: the alternate item price (column +4), used while `cRam0013d4e3` (= `Inventory.have[35]`) is set.
	pub alt: u32,
	pub ammo: u32,
	pub max: u16,
}

pub fn price(item: u8) -> Option<Price> {
	match item {
		10 => Some(Price { item: 0, alt: 0, ammo: 5,  max: 40 }),
		16 => Some(Price { item: 2500, alt: 1250, ammo: 1,  max: 240 }),
		// WPN-6.1e3: rows 0x1c4568 (13 Visibomb) and 0x1c4598 (15 Blaster), +0 / +4 / +8.
		13 => Some(Price { item: 15000, alt: 10000, ammo: 100,  max: 20 }),
		15 => Some(Price { item: 2500, alt: 1250, ammo: 1,  max: 200 }),
		_ => None,
	}
}

impl Price {
	/// NOV-13.4: the item price, column +4 when the alternate-price flag is set.
	pub fn item_cost(&self, alt: bool) -> u32 {
		if alt { self.alt } else { self.item }
	}
}



#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShopKey {
	Left,
	Right,
	/// Cross.
	Buy,
	/// Triangle.
	Exit,
}

#[derive(Clone, Debug)]
pub struct Shop {
	prop: usize,
	/// 0 opening, 1 browsing, 2 closed (the screen is gone next tick).
	pub state: u8,
	pub timer: f32,
	pub rows: Vec<Row>,
	pub sel: usize,
	/// The first Cross was pressed (DAT_001ca89c).
	pub confirm: bool,
	/// Ammo quantity for an ammo row.
	pub qty: u16,
	/// Last result text and seconds left.
	pub msg: Option<(u32, f32)>,
	/// Seconds since opening (turntable).
	pub clock: f32,
	/// Seconds since Triangle (state 1 until the fold starts after 8 ticks, then state 2 until the vendor has folded).
	pub leave: Option<f32>,
	/// UI-5b2: selection frame pulse counter DAT_001ca844 (+1 per tick that starts in state 1 or 2, 0 on 1 -> 2; vendor.md UI-5b2).
	pub pulse: u32,
	/// NOV-13.2: left / right new presses of this tick (pad `cb04` 0x8000 / 0x2000), cleared by `update_shop`.
	pub edge: [bool; 2],
	/// NOV-13.3: turntable yaw DAT_00162224 (pal_scale_f(0.05) a PAL tick, wrapped to [-pi, pi)).
	pub yaw: f32,
	/// NOV-13.4: purchase stage DAT_001ca89c (0 browse, 2 prompt, 3 prompt animation, resolved the next tick).
	pub stage: u8,
	/// UI-5a: the portrait actor (spawned in browse state once the pack is loaded, `Game.portrait_timing`).
	pub portrait: Option<Portrait>,
	/// UI-5d: the prompt prop index (class 1137) and the bolt slot rotation (+0x500, vendor.md UI-5d); stage 4 is the
	/// prompt model's backwards close.
	pub prompt: Option<usize>,
	pub bolt_rot: V3,
	/// UI-5d: a press this tick (retail steps the prompt clock only in the no-press branch, OV:111139).
	pressed: bool,
	/// NOV-13.4: the stage-3 press was Triangle (`b60c == 0`).
	pub cancel: bool,
	/// UI-5e: the ticker box text id (the 18 leading spaces are implied; None = empty box) and its counter DAT_001ca884.
	pub ticker: Option<u32>,
	pub ticker_x: u32,
	/// UI-5e: local frame counter for the LED blink (phase [I], vendor.md UI-5e).
	pub frame: u32,
	pub noise_uv: [[f32; 2]; 7],
	pub noise_fix: [Option<u8>; 7],
	pub sweep_a: [Option<(u8, u16)>; 7],
	/// NOV-30c: icon bar joint offset (7 - n) * 0x681 raw units for n < 8 rows at open (FUN_002adad0), else 0; a purchase
	/// that rebuilds `rows` does not rewrite it.
	pub bar_off: f32,
}

/// UI-5a (vendor.md "UI-5a port spec"): the shop portrait actor (class 12, FUN_002ae750 state machine). Owned by `Shop`;
/// the viewer poses `anim` at `pos` / `rot` into the portrait target (BRIDGE-37).
#[derive(Clone, Debug)]
pub struct Portrait {
	pub anim: crate::anim::PropAnim,
	pub pos: V3,
	pub rot: V3,
	state: u8,
	r: u8,
	idle: u32,
	talk_pending: bool,
	close_pending: bool,
	last_browse: u32,
}

/// The portrait class (FUN_002afce8 OV:111595).
pub const PORTRAIT_CLASS: u32 = 12;


impl Portrait {


	/// FUN_002aeb78: `ev` 1 spawn voice, 0 browse step, 2 / 3 bought. `lang` = DAT_001ca878.
	fn event(&mut self, ev: u8, lang: u32, ticks: u32, ended: &mut bool, events: &mut Vec<Event>) {
		let mut rng = crate::particles::Rng::shared();
		let mut say = |id: u32, events: &mut Vec<Event>| {
			*ended = false;
			events.push(Event::Voice(id as u16));
		};
		match ev {
			1 => {
				self.r = rng.small(2) as u8;
				self.close_pending = false;
				say(10000 + 18 * self.r as u32 + lang, events);
				self.talk_pending = true;
				if self.anim.seq != 1 {
					self.anim.blend(1, 0, 10);
				}
				self.state = 10;
			}
			0 if self.state == 2 || self.state == 0 => {
				if rng.small(4) == 0 && self.last_browse + 300 < ticks {
					self.last_browse = ticks;
					self.r = rng.small(2) as u8;
					self.talk_pending = true;
					self.close_pending = false;
					say(10000 + (3 * self.r as u32 + 2) * 6 + lang, events);
					self.state = 0xb;
				}
			}
			2 | 3 if self.state == 2 || self.state == 0 => self.idle = 0,
			_ => {}
		}
	}

}



impl Shop {

	pub fn row(&self) -> Row {
		self.rows[self.sel]
	}
	/// Black screen fade alpha: 1.0 at the open, down 0.34 a tick in state 0, then 0.
	pub fn fade(&self) -> f32 {
		if self.state == 0 {
			(1.0 - OPEN_FADE * self.timer / TICK).max(0.0)
		} else {
			0.0
		}
	}
}

/// NOV-13.3: one shop model slot: class, world position and Euler rotation (x, y, z; applied x, then y, then z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPose {
	pub class: u32,
	pub pos: V3,
	pub rot: V3,
	pub dest: ItemDest,
}

/// REN-9: where a shop model is drawn. `World` = the B (ammo row) or C (weapon row) model in the world pass; `Panel`
/// = the A model, rendered into the offscreen item-panel target and shown on panel 1 (BRIDGE-19), never a world moby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemDest {
	World,
	Panel,
}

/// Local offsets gp-0x5bf0 (static slot +0x100) and gp-0x5be0 (spinning slot +0x300).
const BASE_A: V3 = [0.0, -1.6, 1.5];
const BASE_B: V3 = [-1.0, 0.05, 1.3];

/// Shop pose rows by item (overlay tables 0x1c8c90 A and 0x1c9cb0 C, stride 0x30): A rotation, A vec3, A z add,
/// C vec3 +0x00, C rot x / y, C vec3 +0x20, and the ammo-row B table (0x1c9380, stride 0x40): rot x / y, vec3 +0x10,
/// ammo class +0x20.
struct PoseRow {
	b_rot: [f32; 2],
	b_vec: V3,
	b_class: u32,
	a_rot: V3,
	a_vec: V3,
	a_z: f32,
	c_off: V3,
	c_rot: [f32; 2],
	c_vec: V3,
}

fn pose_row(item: u8) -> Option<PoseRow> {
	match item {
		10 => Some(PoseRow { b_rot: [0.0; 2], b_vec: [0.0, 0.0, -0.2], b_class: 226, a_rot: [0.0, 0.0, 2.8], a_vec: [0.5, -0.05, -0.02], a_z: 0.06, c_off: [0.0; 3], c_rot: [0.5235988, 0.0], c_vec: [1.0, -0.05, 0.2] }),
		16 => Some(PoseRow { b_rot: [0.0; 2], b_vec: [0.0, 0.0, -0.2], b_class: 214, a_rot: [0.0, -1.57, -3.0], a_vec: [-3.4, -2.15, -0.15], a_z: 0.12, c_off: [0.0, 0.4, 0.0], c_rot: [0.0, -std::f32::consts::FRAC_PI_2], c_vec: [-0.3, 0.0, 0.05] }),
		_ => None,
	}
}

/// Rotation by Euler angles x, then y, then z (the moby matrix order).
fn rotate_xyz(v: V3, r: V3) -> V3 {
	let (sx, cx) = r[0].sin_cos();
	let (sy, cy) = r[1].sin_cos();
	let (y, z) = (v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx);
	let (x, z) = (v[0] * cy + z * sy, -v[0] * sy + z * cy);
	rotate_z([x, y, z], r[2])
}

impl Shop {
	/// REN-9 (FUN_002aed20 + the per-tick turntable): the A model (item's held class, `ItemDest::Panel`) and the
	/// world model of the selected row: B at the ammo class (+0x20) on an ammo row, else the spinning C model of the held
	/// class. `alt` = the alternate-price byte (`Inventory.have[35]`).
	pub fn item_poses(&self, vendor_pos: V3, vendor_yaw: f32, alt: bool) -> Vec<ItemPose> {
		let row = self.row();
		let (Some(class), Some(t)) = (crate::inventory::info(row.item).map(|i| i.held_class), pose_row(row.item)) else { return Vec::new() };
		let mut a = add(BASE_A, t.a_vec);
		if !row.ammo {
			a[2] += t.a_z;
			if alt {
				a[2] += t.a_z;
			}
		}
		let main = ItemPose { class, pos: add(vendor_pos, rotate_z(a, vendor_yaw)), rot: [t.a_rot[0], t.a_rot[1], t.a_rot[2] + vendor_yaw], dest: ItemDest::Panel };
		let bob = add(BASE_B, [0.0, 0.0, self.yaw.sin() / 20.0]);
		if row.ammo {
			// B: rotation (B x, B y, yaw), position vendor + matrix * (base + bob + B vec); nothing when the class is 0.
			if t.b_class == 0 {
				return vec![main];
			}
			let world = ItemPose { class: t.b_class, pos: add(vendor_pos, rotate_z(add(bob, t.b_vec), vendor_yaw)), rot: [t.b_rot[0], t.b_rot[1], self.yaw], dest: ItemDest::World };
			return vec![main, world];
		}
		let rot = [t.c_rot[0], t.c_rot[1], self.yaw];
		let c = add(bob, t.c_vec);
		let spin = ItemPose { class, pos: add(add(vendor_pos, rotate_z(c, vendor_yaw)), rotate_xyz(t.c_off, rot)), rot, dest: ItemDest::World };
		vec![main, spin]
	}
}


fn rotate_z(v: V3, a: f32) -> V3 {
	let (s, c) = a.sin_cos();
	[c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

/// The two scrolling glow layers of the vendor callback (FUN_002ba2f0) around a vendor at `pos`: the beam cone (fx 0x18, V
/// scroll `s0`) and the rising ring (fx 0x1b, rise `s1`), both scaled by the ramp `g` (pvar +0x90) and turned about z by the
/// camera yaw.
pub fn vendor_layers(pos: V3, g: f32, (s0, s1): (f32, f32), cam_yaw: f32) -> Vec<WorldQuad> {
	let mut quads = Vec::new();
	let (sn, cs) = cam_yaw.sin_cos();
	let rot = |x: f32, y: f32| [cs * x - sn * y, sn * x + cs * y];
	let at = |x: f32, y: f32, z: f32| [pos[0] + x, pos[1] + y, pos[2] + z];
	for q in BEAM_Q {
		let c = |k: usize| {
			let v = BEAM_V[q[k]];
			let r = rot(0.0, v[0]);
			(at(r[0] * g, r[1] * g, v[1]), [v[2], v[3] + s0], [1.0, 1.0, 1.0, v[4]])
		};
		let (a, b, c2, d) = (c(0), c(1), c(2), c(3));
		quads.push(WorldQuad { pos: [a.0, b.0, d.0, c2.0], uv: [a.1, b.1, d.1, c2.1], colour: [a.2, b.2, d.2, c2.2], tex: BEAM_TEX, additive: false });
	}
	let (e, z) = (1.2 * s1 * g, 1.4 * s1 + 1.1);
	let corner = |x: f32, y: f32, z: f32| {
		let r = rot(x, y);
		at(r[0], r[1], z)
	};
	let c = [1.0, 1.0, 1.0, 1.0 - s1];
	quads.push(WorldQuad {
		pos: [corner(-e, -e, z), corner(e, -e, z - 0.2), corner(e, e, z - 0.2), corner(-e, e, z)],
		uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
		colour: [c; 4],
		tex: RING_TEX,
		additive: false,
	});
	quads
}

impl Game {










	/// NOV-13.3: the shop item models (spawned at the end of the opening fade, state 0 -> 1).
	pub fn shop_item_poses(&self) -> Vec<ItemPose> {
		let Some(s) = self.shop.as_ref().filter(|s| s.state != 0) else { return Vec::new() };
		let p = &self.props[s.prop];
		s.item_poses(p.pos, p.rot[2], self.inv.have[35])
	}

	/// NOV-30c: the icon bar joint records of prop `pi` while its shop is open (empty otherwise).
	pub fn shop_bar_fx(&self, pi: usize, slot_map: &[Option<usize>]) -> Vec<crate::anim::JointFx> {
		match &self.shop {
			Some(shop) if shop.prop == pi => bar_fx(shop.bar_off, slot_map),
			_ => Vec::new(),
		}
	}

	/// NOV-30d: the slot-0 class-13 bolt icon drawn into the item target after the item (FUN_002b1838): vendor pos plus
	/// (-10.2, -4.25, 0) turned by the vendor yaw, sharing the popup bolt's rotation; whenever the panels draw (state 1).
	pub fn shop_icon_bolt(&self) -> Option<(V3, V3)> {
		let s = self.shop.as_ref().filter(|s| s.state == 1)?;
		let v = &self.props[s.prop];
		Some((add(v.pos, rotate_z(ICON_SLOT, v.rot[2])), s.bolt_rot))
	}

	/// UI-5d: the bolt model of the quantity prompt (class 13 at the bolt slot, vendor.md UI-5d): position and rotation
	/// while an affordable, not-full ammo row is being bought (stages 2..4).
	pub fn shop_bolt(&self) -> Option<(V3, V3)> {
		let s = self.shop.as_ref().filter(|s| s.state == 1 && s.stage != 0)?;
		let row = s.row();
		let p = price(row.item).filter(|p| row.ammo && self.inv.ammo[row.item as usize] < p.max && p.ammo <= self.bolts)?;
		let _ = p;
		let v = &self.props[s.prop];
		Some((add(v.pos, rotate_z(BOLT_SLOT, v.rot[2])), s.bolt_rot))
	}

	/// NOV-35b / BRIDGE-45: the class-13 draw spheres (centre and radius, 1/1024 m) of the icon bolt and the popup bolt,
	/// the BRIDGE-9 sphere of class 13's first sequence at the bolt's pose and scale 1, used for the metal alpha depth and the
	/// metal env matrix (moby_format.md section 4.1). None per slot when that bolt is not posed or class 13 has no sphere.
	pub fn shop_bolt_spheres(&self) -> [Option<(V3, f32)>; 2] {
		let sphere = |pose: Option<(V3, V3)>| pose.and_then(|(pos, rot)| self.bolt_sphere(pos, rot));
		[sphere(self.shop_icon_bolt()), sphere(self.shop_bolt())]
	}

	/// The draw sphere of a class-13 bolt placed at `pos` / `rot` (its seq-0 sphere, class scale), in 1/1024 units.
	pub fn bolt_sphere(&self, pos: V3, rot: V3) -> Option<(V3, f32)> {
		let (seqs, cs) = self.seq_spheres.get(&13)?;
		let sp = seqs.first()?;
		let g = cs / 1024.0;
		let m = super::moby_axes(rot);
		let k: V3 = std::array::from_fn(|a| (pos[a] + m[0][a] * sp[0] * g + m[1][a] * sp[1] * g + m[2][a] * sp[2] * g) * 1024.0);
		Some((k, sp[3] * g * 1024.0))
	}






	/// NOV-13.5: reads the pending cinematic's pack (TOC +0x15a0 + 8 * entry on PAL) and builds the scene;
	/// `joints` = the joint counts of the hero and the gadget model.
	pub fn load_gadget_pack(&mut self, disc: &mut crate::iso::Disc, pal: bool, joints: [usize; 2]) -> crate::bytes::R<()> {
		let Some(GadgetRun::Fanfare { entry, origin, yaw, scene, .. }) = &mut self.gadget else { return Ok(()) };
		if scene.is_some() {
			return Ok(());
		}
		let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
		let o = super::gadget_scene::pack_toc_offset(*entry, pal);
		let (lba, n) = (crate::bytes::u32_at(&toc, o)?, crate::bytes::u32_at(&toc, o + 4)?);
		let pack = disc.read_sectors(lba as u64, n as u64).map_err(|e| e.to_string())?;
		*scene = Some(GadgetScene::new(&pack, pal, *origin, *yaw, joints)?);
		Ok(())
	}


	/// Shop input (edge presses), OV:111287: browse in stage 0, the purchase prompt in stage 2 (NOV-13.4).
	pub fn shop_press(&mut self, key: ShopKey) {
		let (bolts, alt) = (self.bolts, self.inv.have[35]);
		let Some(s) = &mut self.shop else { return };
		if s.state != 1 || s.leave.is_some() {
			return;
		}
		let (prop, row) = (s.prop, s.row());
		let pos = self.props[prop].pos;
		let sound = |cs| Event::Sound { class: VENDOR, cs, pos };
		match (key, s.stage) {
			(ShopKey::Exit, 0) => {
				s.leave = Some(0.0);
				self.events.push(sound(5));
			}
			// Triangle in the prompt: purchase cancel (b60c = 0).
			(ShopKey::Exit, 2) => {
				s.stage = 3;
				s.cancel = true;
				if let Some(pi) = s.prompt {
					// FUN_0026c1f8(prompt, 0, n0 - 1, 1): the close plays seq 0 from its last frame.
					let a = &mut self.props[pi].anim;
					let n0 = a.frame_count_of(0);
					a.blend(0, n0.saturating_sub(1), 1);
					s.pressed = true;
				}
			}
			// NOV-13.2: the prompt records the new press; `update_shop` steps the quantity.
			(ShopKey::Left | ShopKey::Right, 2) => {
				if row.ammo {
					s.edge[(key == ShopKey::Right) as usize] = true;
				}
			}
			(ShopKey::Left | ShopKey::Right, 0) => {
				let n = s.rows.len();
				let moved = match key {
					ShopKey::Left if s.sel > 0 => Some(s.sel - 1),
					ShopKey::Right if s.sel + 1 < n => Some(s.sel + 1),
					_ => None,
				};
				if let Some(m) = moved {
					s.sel = m;
					// NOV-30f (FUN_002afce8 OV:111710-111735): release the ammo counter; re-request on an ammo row.
					self.shop_hud.ammo.release();
					if s.rows[m].ammo {
						self.shop_hud.ammo.request(s.rows[m].item as u16);
					}
					s.confirm = false;
					s.qty = 1;
					s.msg = None;
					s.ticker = Some(item_text(s.rows[m].item));
					s.ticker_x = 0;
					self.events.push(sound(1));
					if let Some(p) = &mut s.portrait {
						p.event(0, self.voice_lang as u32, self.frames, &mut self.voice_ended, &mut self.events);
					}
				}
			}
			(ShopKey::Buy, 0) => {
				// NOV-13.2: cs 0 on the browse Cross; stage 1 runs the same tick and ends in stage 2.
				self.events.push(sound(0));
				let p = price(row.item).unwrap();
				s.qty = 1;
				s.cancel = false;
				s.confirm = true;
				s.stage = 2;
				if let Some(pi) = s.prompt {
					// UI-5d (A): spawn the prompt at the vendor, sequence 0 from frame 0; nothing steps this tick.
					let (vp, vr) = (self.props[prop].pos, self.props[prop].rot);
					let p = &mut self.props[pi];
					p.pos = vp;
					p.rot = vr;
					p.shown = true;
					p.anim.set(0, 0);
					if p.anim.frame_count_of(0) < 2 {
						p.anim.clock.m = 0.0;
					}
					s.pressed = true;
				}
				let owned = self.inv.ammo[row.item as usize];
				if !row.ammo && bolts < p.item_cost(alt) || row.ammo && (bolts < p.ammo || owned >= p.max) {
					self.events.push(sound(2));
				}
			}
			(ShopKey::Buy, 2) => {
				s.stage = 3;
				s.cancel = false;
				if let Some(pi) = s.prompt {
					let a = &mut self.props[pi].anim;
					let n0 = a.frame_count_of(0);
					a.blend(0, n0.saturating_sub(1), 8);
					s.pressed = true;
				}
			}
			_ => {}
		}
	}
}


/// NOV-30c: the four mode-0 joint records of attach slots 20..23 (joints 3, 2, 5, 4), local y offset +off, -off, +off,
/// -off raw units; an unmapped slot is skipped.
pub fn bar_fx(off: f32, slot_map: &[Option<usize>]) -> Vec<crate::anim::JointFx> {
	(0..4)
		.filter_map(|i| {
			let joint = slot_map.get(20 + i).copied().flatten()?;
			let y = if i % 2 == 0 { off } else { -off };
			Some(crate::anim::JointFx { joint, rot: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], off: [0.0, y, 0.0], blend: None, reg: 0 })
		})
		.collect()
}


