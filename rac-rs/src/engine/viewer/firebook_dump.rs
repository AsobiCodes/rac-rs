//! [PC] Dev dump of the flipbook fire (lane nfire, Firebook H): with `RAC_TUNE=fb_dump=1` (off by default) every display frame writes to
//! stderr the level's tongue list (`FBD` header with the cap truncation, one `FBT` row per tongue after `tongues_of`, `m` = its quads),
//! the ground-fire quads (`FBQ`) and the sprite list
//! after `firebook::sort` (one `FBS` row per sprite, `o` = its draw order, 0 = the farthest, drawn first). b\fbdump.py diffs consecutive
//! frames: sprites and tongues that appear or vanish, draw-order swaps of sprites near each other, per-frame changes of a tongue's
//! height / width / fade / foot (does the game feed them per field, 50 Hz, while the display runs faster?).

use super::firebook::Spr;
use super::firefx::tune;
use super::firefx_ground::Tongue;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

/// One write for a whole block of lines (stderr is unbuffered: an `eprintln!` per row was thousands of tiny, slow writes a frame).
fn emit(text: &str) {
	let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

static FRAME: AtomicU64 = AtomicU64::new(0);

/// Whether the dump is on (RAC_TUNE fb_dump).
pub(super) fn on() -> bool {
	tune("fb_dump", 0.0) > 0.5
}

/// The tongues of this display frame (`gather`): `all` before the cap, `kept` after it (the volume cap `room` included), the camera.
pub(super) fn tongues(all: &[Tongue], kept: &[Tongue], cap: usize, room: usize, eye: [f32; 3], yaw: f32, pitch: f32) {
	let f = FRAME.fetch_add(1, Ordering::Relaxed);
	let mut o = String::new();
	let _ = writeln!(o, "FBD f={f} eye={:.4},{:.4},{:.4} yaw={yaw:.5} pitch={pitch:.5} all={} kept={} cap={cap} room={room}", eye[0], eye[1], eye[2], all.len(), kept.len());
	for (i, t) in kept.iter().enumerate() {
		let (b, e) = (t.emit.base, &t.emit);
		let m: Vec<String> = t.members.iter().map(|k| k.to_string()).collect();
		let _ = writeln!(o, "FBT f={f} i={i} seed={:.5} foot={:.4},{:.4},{:.4} h={:.4} w={:.4} fade={:.4} m={}", e.seed, b[0], b[1], b[2], e.h, e.w, t.fade, m.join(","));
	}
	emit(&o);
}

/// The sprites after `sort` (`FireBook::draw`, the same display frame as the last `tongues`).
pub(super) fn sprites(sorted: &[Spr]) {
	let f = FRAME.load(Ordering::Relaxed).saturating_sub(1);
	let mut out = String::new();
	let _ = writeln!(out, "FBN f={f} n={}", sorted.len());
	for (o, s) in sorted.iter().enumerate() {
		let _ = writeln!(out, "FBS f={f} o={o} seed={:.5} foot={:.4},{:.4},{:.4} h={:.4} w={:.4} off={:.2} mir={} fade={:.4}", s.c[3], s.a[0], s.a[1], s.a[2], s.a[3], s.b[0], s.b[1], s.b[2], s.b[3]);
	}
	emit(&out);
}

/// The raw ground-fire quads of this display frame (`FBQ`: quad index, the 4 corners, the 4 vertex alphas; a repeated last corner = a
/// triangle, firefx_ground.rs `is_tri`).
pub(super) fn quads(quads: &[crate::prims::WorldQuad], ids: &[usize]) {
	let f = FRAME.load(Ordering::Relaxed).saturating_sub(1);
	let mut o = String::new();
	for &i in ids {
		let q = &quads[i];
		let p = q.pos;
		let _ = writeln!(
			o,
			"FBQ f={f} i={i} p={:.3},{:.3},{:.3};{:.3},{:.3},{:.3};{:.3},{:.3},{:.3};{:.3},{:.3},{:.3} a={:.3},{:.3},{:.3},{:.3}",
			p[0][0], p[0][1], p[0][2], p[1][0], p[1][1], p[1][2], p[2][0], p[2][1], p[2][2], p[3][0], p[3][1], p[3][2],
			q.colour[0][3], q.colour[1][3], q.colour[2][3], q.colour[3][3]
		);
	}
	emit(&o);
}
