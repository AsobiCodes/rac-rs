//! [PC] Ko-fi drawings: felt-marker "children's drawings", one per Ko-fi supporter (`roster.rs`: Scrolt, korixx, reTokyo, khalifax10),
//! taped up round Ratchet's garage on Veldin. They are made from code (no image files): `scenes.rs` / `achieve.rs` draw a sheet
//! with the hand-drawn pen of `hand.rs` / `canvas.rs`, the stroke font `font.rs`, the shapes and people of `shapes.rs` /
//! `people.rs`. The "kofi art" thread paints them once (about 1 s) when they are first wanted; `world_art`
//! (the pack hook) hands them to the engine, whose `engine/viewer/gpu/draw/pass_art.rs` uploads them as textures and
//! draws crooked quads in the level pass (depth-tested, tinted to the garage light). Renderer only: the game's RAM is read (`note`, from `retail/worker/field_snap.rs`), never written.
//! OFF by default (user 2026-11-04): the Graphics row "Supporter drawings" (`retail/pcsupp.rs`, key `supporters=on`) shows them on EVERY
//! visit to the Veldin garage (level 0), with any Lighting. `RAC_KOFI=0` switches them off whatever the row says.

mod canvas;
mod font;
mod hand;
mod people;
mod achieve;
mod desynkro;
mod fans_a;
mod fans_b;
mod walls;
mod rng;
mod roster;
mod scenes;
mod shapes;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub use roster::ROSTER;
pub use walls::fit_walls;
pub use scenes::Drawing;

/// True while the drawings are wanted here: the row is On, in the Veldin garage (set by `note` every field).
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The Graphics row "Supporter drawings" (pcsupp.rs); Off until the player switches it on.
static SHOWN: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// The painted drawings, until `world_art` moves their pixels into the world sheets (one copy of the pixels, not two).
static WORLD: OnceLock<crate::pack::WorldArt> = OnceLock::new();
static ART: Mutex<Option<Vec<Drawing>>> = Mutex::new(None);

/// `RAC_KOFI=0` turns the drawings off.
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_KOFI").map(|v| v.trim() != "0").unwrap_or(true))
}

/// The row's setting (`retail/pcsupp.rs`).
pub fn set_shown(on: bool) {
	SHOWN.store(on, Ordering::Relaxed);
}

pub fn shown() -> bool {
	SHOWN.load(Ordering::Relaxed)
}

/// The garage of Ratchet's house on Veldin: level 0, every visit (the row decides whether the sheets show).
pub fn gate(level: u32) -> bool {
	level == 0
}

/// Called by the snapshot step every field with what the game's RAM says now (`playing` = not the front end, not booting).
pub fn note(playing: bool, level: u32) {
	let on = playing && enabled() && shown() && gate(level);
	ACTIVE.store(on, Ordering::Relaxed);
	if on {
		start_art_thread();
	}
}

/// True while the sheets are wanted here (see `note`).
pub fn active() -> bool {
	ACTIVE.load(Ordering::Relaxed)
}

/// Paints the sheets on their own thread (once).
fn start_art_thread() {
	if STARTED.swap(true, Ordering::Relaxed) {
		return;
	}
	let _ = std::thread::Builder::new().name("kofi art".into()).spawn(|| {
		crate::retail::hot::idle(); // as the vegetation replan thread: the sheets must not take a core from the game on a 4-core PC
		let v = (0..ROSTER.len()).map(scenes::draw).collect();
		if let Ok(mut a) = ART.lock() {
			*a = Some(v);
		}
	});
}

/// Garage light on the paper: the garage's fill light is dim and warm, so the white sheets are tinted to match
/// (`RAC_TUNE` `kofi_tint` scales all three channels).
const TINT: [f32; 3] = [0.64, 0.6, 0.54];
/// How far the sheet floats off the wall (metres), against depth fighting.
const LIFT: f32 = 0.03;
/// The extra lift of each later sheet (m).
const LIFT_STEP: f32 = 0.0015;

/// The pack's `world_art` hook (pack_bind.rs): the sheets on the garage walls while `active` (the row is On and this is the Veldin
/// garage) and the art thread is done; built once (the engine uploads it once; it is simply not asked for while the row is Off).
/// The sheets show with Lighting Classic too, on purpose (user decision 2026-10-27: "Keep the Ko-fi drawings visible in Classic too"):
/// only the Classic half of the split view, the row and `RAC_KOFI=0` hide them. Not a bug, not to be gated by the Lighting row.
pub fn world_art(level: usize) -> Option<&'static crate::pack::WorldArt> {
	if level != 0 || !active() {
		return None;
	}
	// Built: nothing else to do (no lock, no clone, no allocation per frame).
	if let Some(w) = WORLD.get() {
		return Some(w);
	}
	let mut slot = ART.lock().ok()?;
	let art = slot.as_mut()?;
	// Not before the walls are fitted (the Veldin load does it before the first frame there).
	let fits = walls::fits()?;
	let k = crate::devtune::f32("kofi_tint", 1.0);
	let places = placements();
	// On the wall found at the Veldin load (kofi/walls.rs); each sheet a little further out than the one before, so sheets
	// that overlap never fight in depth. The pixels move out of `ART` (the only copy).
	let sheets = art
		.iter_mut()
		.zip(&places)
		.enumerate()
		.map(|(i, (d, place))| {
			let lift = LIFT + LIFT_STEP * i as f32;
			let quad = match fits.get(i).copied().flatten() {
				Some(f) => corners_n(f.c, f.n, place.w, place.tilt, d.w, d.h, lift),
				None => corners(place, d.w, d.h, lift),
			};
			(crate::level::Texture { w: d.w, h: d.h, rgba: std::mem::take(&mut d.rgba) }, quad)
		})
		.collect();
	*slot = None;
	let _ = WORLD.set(crate::pack::WorldArt { sheets, tint: [TINT[0] * k, TINT[1] * k, TINT[2] * k, 1.0] });
	WORLD.get()
}

/// Where a sheet hangs: the centre of its texture, the horizontal direction the wall faces (radians; 0 = the wall looks toward +x,
/// pi / 2 = toward +y), the width in metres and a tilt in radians (crooked paper).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
	pub c: [f32; 3],
	pub facing: f32,
	pub w: f32,
	pub tilt: f32,
}

/// The sheets' places, one per roster entry (`roster.rs`). `RAC_TUNE` `kofiN_x`, `_y`, `_z`, `_facing`, `_w`, `_tilt` override
/// sheet N (0 = first) for picture checks without a rebuild.
pub fn placements() -> Vec<Placement> {
	ROSTER
		.iter()
		.enumerate()
		.map(|(i, e)| {
			let (p, f) = (e.place, |k: &str, d: f32| crate::devtune::f32(&format!("kofi{i}_{k}"), d));
			Placement { c: [f("x", p.c[0]), f("y", p.c[1]), f("z", p.c[2])], facing: f("facing", p.facing), w: f("w", p.w), tilt: f("tilt", p.tilt) }
		})
		.collect()
}

/// Each drawing with its placement; the shorter list bounds the loop (a drawing without a placement is not hung).
pub fn sheets<'a>(art: &'a [Drawing], places: &'a [Placement]) -> impl Iterator<Item = (&'a Drawing, &'a Placement)> {
	art.iter().zip(places)
}

/// The four corners (top-left, top-right, bottom-right, bottom-left, seen from the room) of a sheet with the texture aspect
/// `tex_w / tex_h`, lifted `lift` off the wall toward the room.
pub fn corners(p: &Placement, tex_w: usize, tex_h: usize, lift: f32) -> [[f32; 3]; 4] {
	corners_n(p.c, [rng::cos(p.facing), rng::sin(p.facing), 0.0], p.w, p.tilt, tex_w, tex_h, lift)
}

/// `corners` for a sheet centred at `c` on a wall with the unit normal `n` toward the room (it may lean: the sheet's right
/// edge stays level, its up axis follows the wall).
pub fn corners_n(c: [f32; 3], n: [f32; 3], w: f32, tilt: f32, tex_w: usize, tex_h: usize, lift: f32) -> [[f32; 3]; 4] {
	// Seen from the room (looking along -normal, z up) the right-hand direction is (-ny, nx) on the level; up = n x right.
	let l = (n[0] * n[0] + n[1] * n[1]).sqrt().max(1e-6);
	let r = [-n[1] / l, n[0] / l, 0.0];
	let up = [n[1] * r[2] - n[2] * r[1], n[2] * r[0] - n[0] * r[2], n[0] * r[1] - n[1] * r[0]];
	let (ct, st) = (rng::cos(tilt), rng::sin(tilt));
	// Right and up axes of the sheet after the tilt (turned around the wall normal).
	let u = [0, 1, 2].map(|k| r[k] * ct + up[k] * st);
	let v = [0, 1, 2].map(|k| -r[k] * st + up[k] * ct);
	let (hw, hh) = (w * 0.5, w * 0.5 * tex_h as f32 / tex_w as f32);
	let c = [0, 1, 2].map(|k| c[k] + n[k] * lift);
	let at = |a: f32, b: f32| [0, 1, 2].map(|k| c[k] + u[k] * a * hw + v[k] * b * hh);
	[at(-1.0, 1.0), at(1.0, 1.0), at(1.0, -1.0), at(-1.0, -1.0)]
}

/// The size of every finished sheet in texels (the drawing and its margin).
pub fn sheet_size() -> (usize, usize) {
	(scenes::W + 2 * scenes::MARGIN, scenes::H + 2 * scenes::MARGIN)
}
