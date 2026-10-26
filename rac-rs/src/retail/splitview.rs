//! [PC] Showcase for videos (user 2026-10-25), Debug mode only: a split view of one frame, Classic on the left and Enhanced on
//! the right of a movable divider, and Hide HUD for clean recordings. Render only: no game state is read or written, so the
//! retail code and its RAM hash do not move. The drawing is `viewer/gpu/draw/split.rs`.
//!
//! Keys (while Debug mode is On, `devtools::NOTICE` lists them):
//! - F10 split view on / off. F11 hide the HUD on / off. F12 the CLASSIC / ENHANCED labels on / off.
//! - `[` `]` hold: move the divider left / right (Shift = 4x faster). Home: divider back to the centre. `\`: automatic wipe.
//!
//! The Classic half is the picture the Graphics page's Classic preset gives (Lighting Classic, draw distance Low = retail's), but
//! with the player's Frame smoothing and Culling rows kept. It is rendered by a full second pass of the frame into an offscreen
//! target (`classic()` is true on the render thread meanwhile, `pass` turns `pcext::graphics()` into the Classic one), then the
//! Enhanced pass draws the real frame and puts the offscreen picture over the left part.
//!
//! Texture packs: the Classic half shows the ORIGINAL disc textures, the Enhanced half the pack's (`keep_originals`, viewer/gpu/split_orig.rs).
//!
//! Test hooks (documented in ENV.md): RAC_SPLIT=0..1 (split on, divider at that fraction), RAC_SPLIT_LABELS=0, RAC_HIDE_HUD=1.

use crate::overlay::{rect, text, text_width, Quad};
use crate::retail::pcext::Graphics;
use std::cell::Cell;
use std::sync::{Mutex, Once};
use std::time::Instant;
use winit::keyboard::KeyCode as K;

/// `Quad.tex` of the Classic picture (viewer/targets.rs has the other special ids, down to MAX - 4).
pub const SPLIT_TEX: usize = usize::MAX - 8;
/// Divider speed in screen widths per second, the Shift factor, and the automatic wipe's seconds for one way.
const SPEED: f32 = 0.30;
const FAST: f32 = 4.0;
const WIPE_SECS: f32 = 6.0;
const MARGIN: f32 = 0.04;

struct St {
	split: bool,
	hide: bool,
	labels: bool,
	auto: bool,
	pos: f32,
	/// Held: `[` , `]`, Shift.
	held: [bool; 3],
	last: Option<Instant>,
	/// The auto wipe's phase in seconds.
	phase: f32,
}

static ST: Mutex<St> = Mutex::new(St { split: false, hide: false, labels: true, auto: false, pos: 0.5, held: [false; 3], last: None, phase: 0.0 });

thread_local! {
	/// True on the render thread while the Classic pass of a split frame is recorded.
	static CLASSIC: Cell<bool> = const { Cell::new(false) };
	/// True while a split frame re-runs `Gpu::set_props` for one of its passes, and the length of `moby_dyn` before the frame's first run.
	static REDO: Cell<(bool, usize)> = const { Cell::new((false, 0)) };
}

fn st() -> std::sync::MutexGuard<'static, St> {
	static ENV: Once = Once::new();
	ENV.call_once(|| {
		let mut s = ST.lock().unwrap_or_else(|e| e.into_inner());
		if let Some(p) = std::env::var("RAC_SPLIT").ok().and_then(|v| v.parse::<f32>().ok()) {
			(s.split, s.pos) = (true, p.clamp(0.0, 1.0));
		}
		s.labels = std::env::var("RAC_SPLIT_LABELS").map_or(true, |v| v != "0");
		s.hide = std::env::var("RAC_HIDE_HUD").is_ok_and(|v| v == "1");
	});
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The split view is on.
pub fn active() -> bool {
	st().split
}

/// True while the Classic pass of a split frame is recorded.
pub fn classic() -> bool {
	CLASSIC.with(Cell::get)
}

pub fn set_classic(on: bool) {
	CLASSIC.with(|c| c.set(on));
}

/// True while a split frame re-runs `set_props` (props draw-distance pick of the Classic half, then the real one again).
pub fn redo() -> bool {
	REDO.with(|r| r.get().0)
}

pub fn set_redo(on: bool) {
	REDO.with(|r| r.set((on, r.get().1)));
}

/// `set_props`: the `moby_dyn` length a re-run truncates back to (a normal run records its start length, a re-run reads it).
pub fn dyn_base(len: usize) -> usize {
	REDO.with(|r| {
		let (on, base) = r.get();
		if !on {
			r.set((false, len));
		}
		if on { base } else { len }
	})
}

/// The Graphics of the Classic half: Classic with the player's Frame smoothing and Culling rows (they steer the handler and the
/// culling, not the look).
pub fn classic_of(g: Graphics) -> Graphics {
	Graphics { smoothing: g.smoothing, culling: g.culling, ..Graphics::CLASSIC }
}

/// `pcext::graphics()` as the running pass sees it.
pub fn pass(g: Graphics) -> Graphics {
	if classic() { classic_of(g) } else { g }
}

/// THE switch of every Enhanced-only effect for the split view: `on` as the caller decided it, but false while the Classic pass of
/// a split frame is recorded. The Classic half is the Classic preset's picture, so no Enhanced effect may draw there, whatever
/// per-frame state it kept from the last Enhanced pass (bug 2026-10-25: the animated Gemlik star decided its slot in a step the
/// Classic pass skips, so the Classic half drew it). A feature calls it at its draw gate, or its module doc says which gate it hangs
/// on ("Split view:"); `tests::every_enhanced_module_obeys_the_split_switch` fails for a new Enhanced module that does neither.
/// Off the split view (and in the Enhanced pass) it returns `on` unchanged.
pub fn fx(on: bool) -> bool {
	on && !classic()
}

/// Texture packs: a scene (or the fx table) being built keeps the original disc textures next to the pack's, for the Classic half of the
/// split view. Only while the split view could be used (Debug mode or the split view on), so a normal game pays nothing.
pub fn keep_originals() -> bool {
	active() || super::devtools::enabled() || super::debugkeys::enabled()
}

/// The two halves differ: with the modern renderer, Lighting Enhanced or a draw distance above Low.
pub fn differs(g: Graphics, modern_ok: bool) -> bool {
	modern_ok && (g.enhanced() || g.draw_q() != 0)
}

/// Debug mode was switched Off: everything back to normal.
pub fn menu_off() {
	let mut s = st();
	(s.split, s.hide, s.auto, s.held) = (false, false, false, [false; 3]);
}

/// Hide HUD is on.
pub fn hidden() -> bool {
	st().hide
}

/// Window thread: a key event. True = taken (only while Debug mode is On).
pub fn key(k: K, pressed: bool, repeat: bool) -> bool {
	let on = super::devtools::enabled() || super::debugkeys::enabled();
	let mut s = st();
	match k {
		K::ShiftLeft | K::ShiftRight => {
			s.held[2] = pressed;
			return false;
		}
		K::BracketLeft | K::BracketRight if on => {
			s.held[(k == K::BracketRight) as usize] = pressed;
			if pressed {
				s.auto = false;
			}
		}
		K::F10 | K::F11 | K::F12 | K::Home | K::Backslash if on => {
			if pressed && !repeat {
				match k {
					K::F10 => s.split = !s.split,
					K::F11 => s.hide = !s.hide,
					K::F12 => s.labels = !s.labels,
					K::Home => (s.pos, s.auto) = (0.5, false),
					_ => (s.auto, s.split) = (!s.auto, true),
				}
			}
		}
		_ => return false,
	}
	true
}

/// The divider's pixel column for a `sw` wide frame, moved by the held keys / the auto wipe since the last call.
fn divider(s: &mut St, sw: f32) -> f32 {
	let now = Instant::now();
	let dt = s.last.replace(now).map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.1));
	let dir = s.held[1] as i32 - s.held[0] as i32;
	if dir != 0 {
		s.pos += dir as f32 * SPEED * dt * if s.held[2] { FAST } else { 1.0 };
	}
	if s.auto {
		s.phase += dt;
		let t = (s.phase / WIPE_SECS) % 2.0;
		s.pos = MARGIN + (1.0 - 2.0 * MARGIN) * if t < 1.0 { t } else { 2.0 - t };
	}
	s.pos = s.pos.clamp(0.0, 1.0);
	(s.pos * sw).round().clamp(0.0, sw)
}

/// The quads the real pass adds over the HUD (window pixels): the Classic picture over [0, x), the divider, the labels. Nothing
/// when the split view is off. `classic_ready`: the Classic pass was recorded this frame.
pub fn extras(out: &mut Vec<Quad>, sw: f32, sh: f32, classic_ready: bool) {
	let mut s = st();
	if !s.split {
		return;
	}
	let x = divider(&mut s, sw);
	let labels = s.labels;
	drop(s);
	if classic_ready && x > 0.0 {
		out.push(Quad { tex: SPLIT_TEX, rect: [0.0, 0.0, x, sh], uv: [0.0, 0.0, x / sw, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
	}
	if x > 1.0 && x < sw - 1.0 {
		rect(out, [x - 1.0, 0.0, 2.0, sh], [1.0, 1.0, 1.0, 0.85]);
	}
	if labels && classic_ready {
		let (k, size) = ((sh / 720.0).max(0.6), (sh / 720.0).max(0.6) * 18.0);
		for (name, left) in [("CLASSIC", true), ("ENHANCED", false)] {
			let w = text_width(name, size);
			let (room, edge) = if left { (x, 14.0 * k) } else { (sw - x, 14.0 * k) };
			if room < w + 2.0 * edge {
				continue;
			}
			let lx = if left { edge } else { sw - w - edge };
			let ly = sh - size - 16.0 * k;
			rect(out, [lx - 6.0 * k, ly - 4.0 * k, w + 12.0 * k, size + 10.0 * k], [0.0, 0.0, 0.0, 0.55]);
			text(out, name, lx, ly, size, [1.0, 1.0, 1.0, 1.0]);
		}
	}
}

/// Hide HUD, before the frame is drawn: in plain gameplay (no menu, no shop) the HUD quads go. `gameplay`: the caller's verdict.
pub fn strip_hud(hud: &mut Vec<Quad>, hud_split: &mut Option<usize>, gameplay: bool) {
	if gameplay && hidden() {
		hud.clear();
		*hud_split = None;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classic_pass_keeps_smoothing_and_culling_only() {
		let real = Graphics { lighting: 1, smoothing: true, culling: false, draw: 2, water: true, ..Graphics::CLASSIC };
		let c = classic_of(real);
		assert!(c.smoothing && !c.culling);
		assert_eq!((c.lighting, c.draw, c.water), (0, 0, false));
		assert!(differs(real, true) && !differs(real, false));
		assert!(differs(Graphics { lighting: 1, ..Graphics::CLASSIC }, true) && !differs(Graphics { lighting: 1, ..Graphics::CLASSIC }, false));
		assert!(!differs(Graphics::CLASSIC, true));
	}

	#[test]
	fn pass_follows_the_flag() {
		let real = Graphics { lighting: 1, ..Graphics::CLASSIC };
		assert_eq!(pass(real), real);
		set_classic(true);
		assert_eq!(pass(real).lighting, 0);
		set_classic(false);
	}

	#[test]
	fn fx_is_off_in_the_classic_pass_only() {
		assert!(fx(true) && !fx(false));
		set_classic(true);
		assert!(!fx(true) && !fx(false));
		set_classic(false);
		assert!(fx(true));
	}

	/// The guard of the class "an Enhanced effect shows in the Classic half": every viewer module whose doc calls it Enhanced either
	/// gates its draw with `splitview::fx` or names its gate in a "Split view:" doc line; and `classic()` itself is only read where a
	/// pass must keep the Enhanced pass's history or target (a feature uses `fx`).
	#[test]
	fn every_enhanced_module_obeys_the_split_switch() {
		const CLASSIC_READERS: [&str; 4] = ["gpu/draw/passes.rs", "gpu/draw/prepare.rs", "volclouds.rs", "volfog/frame.rs"];
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/viewer");
		let (mut files, mut todo, mut bad) = (Vec::new(), vec![root.clone()], Vec::new());
		while let Some(d) = todo.pop() {
			for e in std::fs::read_dir(&d).unwrap().flatten() {
				let p = e.path();
				if p.is_dir() { todo.push(p) } else if p.extension().is_some_and(|x| x == "rs") { files.push(p) }
			}
		}
		assert!(files.len() > 50, "viewer sources not found under {root:?}");
		for p in files {
			let (src, rel) = (std::fs::read_to_string(&p).unwrap(), p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
			let doc: String = src.lines().take_while(|l| l.starts_with("//!")).collect::<Vec<_>>().join("\n");
			if doc.contains("Enhanced") && !src.contains("splitview::fx(") && !doc.contains("Split view:") {
				bad.push(format!("{rel}: Enhanced module without splitview::fx or a \"Split view:\" doc line"));
			}
			if src.contains("splitview::classic()") && !CLASSIC_READERS.contains(&rel.as_str()) {
				bad.push(format!("{rel}: reads splitview::classic(); an effect gate uses splitview::fx"));
			}
		}
		assert!(bad.is_empty(), "{}", bad.join("\n"));
	}

	#[test]
	fn divider_moves_and_clamps() {
		let mut s = St { split: true, hide: false, labels: true, auto: false, pos: 0.5, held: [false, true, true], last: None, phase: 0.0 };
		assert_eq!(divider(&mut s, 1000.0), 500.0);
		s.last = Some(Instant::now() - std::time::Duration::from_millis(100));
		let x = divider(&mut s, 1000.0);
		assert!(x > 500.0 && x < 700.0, "{x}");
		s.pos = 7.0;
		assert_eq!(divider(&mut s, 1000.0), 1000.0);
	}

	#[test]
	fn extras_geometry() {
		let mut s = ST.lock().unwrap();
		(s.split, s.pos, s.labels, s.held, s.last, s.auto) = (true, 0.5, true, [false; 3], None, false);
		drop(s);
		let mut out = Vec::new();
		extras(&mut out, 1280.0, 720.0, true);
		assert_eq!(out[0].tex, SPLIT_TEX);
		assert_eq!(out[0].rect, [0.0, 0.0, 640.0, 720.0]);
		assert_eq!(out[0].uv, [0.0, 0.0, 0.5, 1.0]);
		assert_eq!(out[1].rect, [639.0, 0.0, 2.0, 720.0]);
		assert!(out.len() > 4);
		let mut none = Vec::new();
		extras(&mut none, 1280.0, 720.0, false);
		assert_eq!(none.len(), 1);
		ST.lock().unwrap().split = false;
		let mut off = Vec::new();
		extras(&mut off, 1280.0, 720.0, true);
		assert!(off.is_empty());
	}
}
