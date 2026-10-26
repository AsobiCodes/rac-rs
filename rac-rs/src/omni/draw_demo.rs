//! `RAC_OMNI_DRAW_DEMO=1`: a fixed draw list the host submits itself each tick (as the loader would), for checking the draw pass without a mod DLL
//! that speaks ABI 1.2: text in the three alignments, a rect, two bars, an outline of the 512 x 448 space and a marker on the hero's world
//! position through `world_to_screen`. Runs only when mods are loaded (`omni::tick`) and the switch is set.

use super::cabi::OmniDrawCmd;
use super::draw_quads::{KIND_BAR, KIND_RECT, KIND_TEXT};
use ee_oracle::cpu::Cpu;
use std::sync::OnceLock;

/// The hero's position (gHeroPos) on the disc layout both our discs have (rac1.json: base hero 0x13F350 + 0x80).
const HERO_POS: usize = 0x13_F3D0;

fn enabled() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_OMNI_DRAW_DEMO").is_ok_and(|v| v != "0" && !v.is_empty()))
}

struct List {
	cmds: Vec<OmniDrawCmd>,
	pool: Vec<u8>,
}

impl List {
	fn text(&mut self, x: f32, y: f32, size: f32, align: u32, color: u32, shadow: u32, layer: u32, s: &str) {
		let off = self.pool.len() as u32;
		self.pool.extend_from_slice(s.as_bytes());
		self.cmds.push(OmniDrawCmd { kind: KIND_TEXT, layer, align, x, y, w: size, color, color2: shadow, text_off: off, text_len: s.len() as u32, ..Default::default() });
	}
	fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
		self.cmds.push(OmniDrawCmd { kind: KIND_RECT, x, y, w, h, color, ..Default::default() });
	}
	fn bar(&mut self, x: f32, y: f32, w: f32, h: f32, value: f32, color: u32, back: u32) {
		self.cmds.push(OmniDrawCmd { kind: KIND_BAR, x, y, w, h, value, color, color2: back, ..Default::default() });
	}
}

/// The demo list; `hero` is the hero's game-space spot (None when there is no 3D view).
fn list(hero: Option<([f32; 2], bool)>) -> List {
	let mut l = List { cmds: Vec::new(), pool: Vec::new() };
	// The 512 x 448 outline and its corner marks.
	for (x, y, w, h) in [(0.0, 0.0, 512.0, 2.0), (0.0, 446.0, 512.0, 2.0), (0.0, 0.0, 2.0, 448.0), (510.0, 0.0, 2.0, 448.0)] {
		l.rect(x, y, w, h, 0x00FFFFC0);
	}
	l.text(256.0, 12.0, 1.5, 1, 0xFFFFFFFF, 0x000000FF, 1, "OMNI DRAW TEST");
	l.text(16.0, 40.0, 1.0, 0, 0xFFFF40FF, 0, 0, "Left text, size 1");
	l.text(496.0, 40.0, 1.0, 2, 0x80FF80FF, 0x000000FF, 0, "Right text 0123456789");
	l.text(256.0, 64.0, 0.6, 1, 0xFFFFFFFF, 0, 1, "centre, size 0.6: ()[]+-/%&!?:.");
	l.rect(16.0, 90.0, 40.0, 16.0, 0xFF3030C0);
	l.bar(16.0, 116.0, 200.0, 12.0, 0.5, 0x40FF40FF, 0x203020FF);
	l.bar(16.0, 134.0, 200.0, 12.0, 0.1, 0xFF4040FF, 0x302020FF);
	l.text(16.0, 150.0, 0.75, 0, 0xFFFFFFFF, 0x000000FF, 0, "bars at 50 and 10 percent");
	// Icons of the game's HUD art (the table of draw_art.rs): the bolt, the nanotech orb, three weapons and a tinted one.
	for (i, name) in ["bolt", "nanotech", "weapon.blaster", "weapon.pyrocitor", "weapon.devastator", "weapon.blaster"].iter().enumerate() {
		if let Some(idx) = super::draw_art::index_of(name) {
			let color = if i == 5 { 0xFF4040FF } else { 0xFFFFFFFF };
			l.cmds.push(OmniDrawCmd { kind: super::draw_quads::KIND_ICON, x: 300.0 + 36.0 * i as f32, y: 90.0, w: 32.0, h: 32.0, color, icon: idx as u32, ..Default::default() });
		}
	}
	if let Some((g, front)) = hero {
		let c = if front { 0xFFE020FF } else { 0xFF8020FF };
		l.rect(g[0] - 4.0, g[1] - 4.0, 8.0, 8.0, c);
		l.rect(g[0] - 14.0, g[1] - 0.5, 28.0, 1.0, c);
		l.rect(g[0] - 0.5, g[1] - 14.0, 1.0, 28.0, c);
		l.text(g[0], g[1] + 12.0, 0.75, 1, 0xFFFFFFFF, 0x000000FF, 0, if front { "HERO" } else { "HERO (behind)" });
	}
	l
}

/// After the mods' tick on the worker thread: submits the demo list when RAC_OMNI_DRAW_DEMO is set.
pub fn tick(c: &Cpu) {
	if !enabled() {
		return;
	}
	let mut p = [0f32; 3];
	for (i, v) in p.iter_mut().enumerate() {
		let a = HERO_POS + 4 * i;
		*v = c.mem.ram.get(a..a + 4).map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	}
	let hero = super::draw_w2s::screen_of(c, p);
	let l = list(hero);
	super::draw::submit(&l.cmds, &l.pool);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_demo_list_is_well_formed() {
		let l = list(Some(([256.0, 224.0], true)));
		assert!(l.cmds.len() > 10);
		for c in l.cmds.iter().filter(|c| c.kind == KIND_TEXT) {
			assert!((c.text_off + c.text_len) as usize <= l.pool.len());
		}
		assert!(list(None).cmds.len() < l.cmds.len());
	}
}
