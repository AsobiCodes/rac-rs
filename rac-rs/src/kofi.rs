//! [PC] Ko-fi drawings: three felt-marker "children's drawings" (one per Ko-fi supporter: Scrolt, korixx, reTokyo) taped to the wall
//! of Ratchet's garage on Veldin, only during the first visit. They are made from code (no image files): `scenes.rs` draws a sheet
//! with the hand-drawn pen of `hand.rs` / `canvas.rs`, the stroke font `font.rs`, the shapes and people of `shapes.rs` /
//! `people.rs`. The "kofi art" thread paints them once (about 1 s) when the first visit starts; the GPU side
//! (`viewer/gpu/draw/pass_kofi.rs`) uploads them as three textures and draws three crooked quads in the level pass (depth-tested,
//! tinted to the garage light). Renderer only: the game's RAM is read (`note`, from `retail/worker/field_snap.rs`), never written.
//! `RAC_KOFI=0` switches them off.

mod canvas;
mod font;
mod hand;
mod people;
mod rng;
mod scenes;
mod shapes;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

pub use scenes::{Drawing, SUPPORTERS};

/// True during the first Veldin visit (set by `note` every field).
static FIRST_VISIT: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static ART: OnceLock<Vec<Drawing>> = OnceLock::new();

/// `RAC_KOFI=0` turns the drawings off.
pub fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_KOFI").map(|v| v.trim() != "0").unwrap_or(true))
}

/// The first Veldin visit: level 0 (Veldin) and Novalis not yet unlocked. Retail sets the unlock byte (0x13dd40 + level, level 1 =
/// Novalis) in its silent unlock when the game enters Novalis for the first time, so it is 0 for the whole first stay on Veldin
/// (also across saves and loads before that) and 1 after it, also when the player goes back to Veldin.
pub fn gate(level: u32, novalis_unlocked: u8) -> bool {
	level == 0 && novalis_unlocked == 0
}

/// Core RAM address of the Novalis unlock byte (the same on PAL and USA: below the overlay at 0x15ef00).
pub const NOVALIS_UNLOCK: usize = 0x13dd41;

/// Called by the snapshot step every field with what the game's RAM says now (`playing` = not the front end, not booting).
pub fn note(playing: bool, level: u32, novalis_unlocked: u8) {
	let on = playing && enabled() && gate(level, novalis_unlocked);
	FIRST_VISIT.store(on, Ordering::Relaxed);
	if on {
		start_art_thread();
	}
}

pub fn first_visit() -> bool {
	FIRST_VISIT.load(Ordering::Relaxed)
}

/// Paints the three sheets on their own thread (once).
fn start_art_thread() {
	if STARTED.swap(true, Ordering::Relaxed) {
		return;
	}
	let _ = std::thread::Builder::new().name("kofi art".into()).spawn(|| {
		crate::retail::hot::idle(); // as the vegetation replan thread: the sheets must not take a core from the game on a 4-core PC
		let v = (0..SUPPORTERS.len()).map(scenes::draw).collect();
		let _ = ART.set(v);
	});
}

/// The finished drawings (None while the thread is still painting).
pub fn art() -> Option<&'static [Drawing]> {
	ART.get().map(|v| &v[..])
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

/// The three sheets on the garage walls (Scrolt, korixx, reTokyo).
pub fn placements() -> [Placement; 3] {
	// The flat green panel of the -x garage wall, right of the turbine: left to right Scrolt, korixx, reTokyo, just below eye height (z 34.1; above ~35 the arch leans into the room and would cut the top of a sheet). The wall
	// slants (its plane moves about 0.25 m toward +x per metre of y), so the sheets face -0.25 rad off the x axis and each has its own x.
	[
		Placement { c: [139.90, 97.8, 34.1], facing: -0.25, w: 1.2, tilt: 0.05 },
		Placement { c: [140.26, 99.3, 34.1], facing: -0.25, w: 1.2, tilt: -0.04 },
		Placement { c: [140.70, 100.8, 34.1], facing: -0.25, w: 1.2, tilt: 0.03 },
	]
}

/// Each drawing with its placement; the shorter list bounds the loop (a drawing without a placement is not hung).
pub fn sheets<'a>(art: &'a [Drawing], places: &'a [Placement]) -> impl Iterator<Item = (&'a Drawing, &'a Placement)> {
	art.iter().zip(places)
}

/// The four corners (top-left, top-right, bottom-right, bottom-left, seen from the room) of a sheet with the texture aspect
/// `tex_w / tex_h`, lifted `lift` off the wall toward the room.
pub fn corners(p: &Placement, tex_w: usize, tex_h: usize, lift: f32) -> [[f32; 3]; 4] {
	let (nx, ny) = (rng::cos(p.facing), rng::sin(p.facing));
	// Seen from the room (looking along -normal, z up) the right-hand direction is (-ny, nx).
	let (rx, ry) = (-ny, nx);
	let (ct, st) = (rng::cos(p.tilt), rng::sin(p.tilt));
	// Right and up axes of the sheet after the tilt (turned around the wall normal).
	let u = [rx * ct, ry * ct, st];
	let v = [-rx * st, -ry * st, ct];
	let (hw, hh) = (p.w * 0.5, p.w * 0.5 * tex_h as f32 / tex_w as f32);
	let c = [p.c[0] + nx * lift, p.c[1] + ny * lift, p.c[2]];
	let at = |a: f32, b: f32| [c[0] + u[0] * a * hw + v[0] * b * hh, c[1] + u[1] * a * hw + v[1] * b * hh, c[2] + u[2] * a * hw + v[2] * b * hh];
	[at(-1.0, 1.0), at(1.0, 1.0), at(1.0, -1.0), at(-1.0, -1.0)]
}
