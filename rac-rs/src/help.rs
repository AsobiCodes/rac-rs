//! The HelpDesk box and the Help Log (decomp/notes/ingame_menu_leftovers.md P1-a): the 8-state
//! timeline of the help state machine FUN_00203808, the box layout of FUN_002036d0, and the log
//! history Help_Request adds to (FUN_002046a8).

use crate::hud::Glyph;
use crate::overlay::{self, Quad};

/// Help Log table {helpId, log title text id}, 148 retail rows in row order (decomp/notes/savegame.md SAV-2;
/// row i title = 21103 + i for i >= 4). A saved log stores row indices.
const LOG_TABLE: [(u32, u32); 148] = [
	(3, 0x5272),
	(0, 0x526f),
	(1, 0x5270),
	(2, 0x5271),
	(1000, 0x5273),
	(1001, 0x5274),
	(1002, 0x5275),
	(1003, 0x5276),
	(4, 0x5277),
	(5, 0x5278),
	(2000, 0x5279),
	(2001, 0x527a),
	(2002, 0x527b),
	(2003, 0x527c),
	(2004, 0x527d),
	(2005, 0x527e),
	(2006, 0x527f),
	(2007, 0x5280),
	(2008, 0x5281),
	(2009, 0x5282),
	(3000, 0x5283),
	(3001, 0x5284),
	(3002, 0x5285),
	(3003, 0x5286),
	(3004, 0x5287),
	(3005, 0x5288),
	(3006, 0x5289),
	(3007, 0x528a),
	(4000, 0x528b),
	(4001, 0x528c),
	(4002, 0x528d),
	(4003, 0x528e),
	(4004, 0x528f),
	(5000, 0x5290),
	(5001, 0x5291),
	(5002, 0x5292),
	(5003, 0x5293),
	(5004, 0x5294),
	(5005, 0x5295),
	(6000, 0x5296),
	(6001, 0x5297),
	(6002, 0x5298),
	(6003, 0x5299),
	(6004, 0x529a),
	(6005, 0x529b),
	(6006, 0x529c),
	(6007, 0x529d),
	(6008, 0x529e),
	(8000, 0x529f),
	(8001, 0x52a0),
	(8002, 0x52a1),
	(8003, 0x52a2),
	(9000, 0x52a3),
	(10000, 0x52a4),
	(10001, 0x52a5),
	(10002, 0x52a6),
	(10003, 0x52a7),
	(10004, 0x52a8),
	(11000, 0x52a9),
	(11001, 0x52aa),
	(11002, 0x52ab),
	(11003, 0x52ac),
	(12000, 0x52ad),
	(12001, 0x52ae),
	(1004, 0x52af),
	(1005, 0x52b0),
	(1006, 0x52b1),
	(1007, 0x52b2),
	(1008, 0x52b3),
	(2010, 0x52b4),
	(2011, 0x52b5),
	(2012, 0x52b6),
	(2013, 0x52b7),
	(4005, 0x52b8),
	(20000, 0x52b9),
	(20001, 0x52ba),
	(20002, 0x52bb),
	(20003, 0x52bc),
	(20004, 0x52bd),
	(20005, 0x52be),
	(20006, 0x52bf),
	(20007, 0x52c0),
	(6009, 0x52c1),
	(6010, 0x52c2),
	(3008, 0x52c3),
	(11004, 0x52c4),
	(11005, 0x52c5),
	(11006, 0x52c6),
	(11007, 0x52c7),
	(11008, 0x52c8),
	(8004, 0x52c9),
	(9001, 0x52ca),
	(10005, 0x52cb),
	(12002, 0x52cc),
	(5006, 0x52cd),
	(5007, 0x52ce),
	(3009, 0x52cf),
	(5008, 0x52d0),
	(3010, 0x52d1),
	(3011, 0x52d2),
	(5009, 0x52d3),
	(3012, 0x52d4),
	(5010, 0x52d5),
	(5011, 0x52d6),
	(3013, 0x52d7),
	(10006, 0x52d8),
	(10007, 0x52d9),
	(10008, 0x52da),
	(11009, 0x52db),
	(12003, 0x52dc),
	(12004, 0x52dd),
	(13000, 0x52de),
	(8005, 0x52df),
	(11010, 0x52e0),
	(20008, 0x52e1),
	(20009, 0x52e2),
	(20010, 0x52e3),
	(20011, 0x52e4),
	(20012, 0x52e5),
	(20013, 0x52e6),
	(20014, 0x52e7),
	(20015, 0x52e8),
	(14000, 0x52e9),
	(15000, 0x52ea),
	(15001, 0x52eb),
	(15002, 0x52ec),
	(16000, 0x52ed),
	(15003, 0x52ee),
	(15004, 0x52ef),
	(20016, 0x52f0),
	(15005, 0x52f1),
	(16001, 0x52f2),
	(16002, 0x52f3),
	(18000, 0x52f4),
	(11011, 0x52f5),
	(16003, 0x52f6),
	(15006, 0x52f7),
	(15007, 0x52f8),
	(15008, 0x52f9),
	(17000, 0x52fa),
	(18001, 0x52fb),
	(18002, 0x52fc),
	(8006, 0x52fd),
	(16004, 0x52fe),
	(2014, 0x52ff),
	(20017, 0x5300),
	(20018, 0x5301),
	(20019, 0x5302),
];
/// History capacity (the table has 150 rows).
const LOG_MAX: usize = 150;
pub const WELCOME: u32 = 3;

/// The help history (0x141e08, count 0x15ee30): help ids in the order last seen.
#[derive(Clone, Debug, PartialEq)]
pub struct HelpLog(pub Vec<u32>);

impl Default for HelpLog {
	/// Boot default: count 1 with entry 0 = 'Welcome!'.
	fn default() -> Self {
		HelpLog(vec![WELCOME])
	}
}

impl HelpLog {
	/// Logs a help id if the table has it; a repeat moves to the end.
	pub fn add(&mut self, id: u32) {
		if title(id).is_none() {
			return;
		}
		self.0.retain(|&h| h != id);
		self.0.push(id);
		if self.0.len() > LOG_MAX {
			self.0.remove(0);
		}
	}

}

pub fn title(help_id: u32) -> Option<u32> {
	LOG_TABLE.iter().find(|e| e.0 == help_id).map(|e| e.1)
}

const FADE: u32 = 4;
/// Box alpha 0x60 of 0x80.
const BOX_ALPHA: f32 = 0x60 as f32 / 128.0;
/// Text colour 0x80ffa888, bytes / 128 (font texels 0x80, `hud::ps2_rgba`).
const TEXT: [f32; 3] = [0x88 as f32 / 128.0, 0xa8 as f32 / 128.0, 0xff as f32 / 128.0];
/// Wrap width 424 (clip x 44..468), line height 16.
const WRAP: f32 = 424.0;
const LINE: f32 = 16.0;

/// The help box: state (DAT_00179410, 0 = idle) and frames in it (DAT_00179414).
#[derive(Clone, Debug, Default)]
pub struct HelpBox {
	pub state: u8,
	pub t: u32,
	pub id: u32,
	pub text: String,
}

impl HelpBox {
	pub fn busy(&self) -> bool {
		self.state != 0
	}



	/// The box at the bottom centre of the 512x448 screen, scaled into a w x h window.
	/// `icon` = the HUD slot of fx texture 4 (64x64).
	pub fn draw(&self, out: &mut Vec<Quad>, font: &(usize, Vec<Glyph>), icon: Option<usize>, w: f32, h: f32, text_on: bool) {
		if self.state == 0 || !text_on {
			return;
		}
		let k = h / 448.0;
		let ox = (w - 512.0 * k) / 2.0;
		let lines = wrap(font, &self.text, WRAP);
		let tw = lines.iter().map(|l| width(font, l)).fold(0.0, f32::max);
		let (fw, fh) = (tw / 2.0 + 10.0, lines.len() as f32 * LINE / 2.0 + 5.0);
		let screen_h = 448.0;
		let mut cy = screen_h - 60.0;
		if cy + fh > screen_h - 12.0 {
			cy = screen_h - (fh + 17.0);
		}
		let cx = 256.0;
		let t = self.t as f32;
		let ((hw, hh), a, text_a) = match self.state {
			1 => ((4.0 * t + 8.0, 4.0 * t + 8.0), BOX_ALPHA, 0.0),
			2 => ((32.0, 32.0), BOX_ALPHA, 0.0),
			3 => ((32.0 + (fw - 32.0) * t / 8.0, 32.0 + (fh - 32.0) * t / 8.0), BOX_ALPHA, 0.0),
			4 => ((fw, fh), BOX_ALPHA, t * 0x20 as f32 / 128.0),
			5 => ((fw, fh), BOX_ALPHA, 1.0),
			6 => ((fw, fh), BOX_ALPHA, (FADE as f32 - t).max(0.0) * 0x20 as f32 / 128.0),
			_ => ((fw + (8.0 - fw) * t / 8.0, fh + (8.0 - fh) * t / 8.0), (8.0 - t).max(0.0) * 12.0 / 128.0, 0.0),
		};
		// FUN_001facd8: a rect with 4 px stepped top and bottom rows (RGB 4,4,4).
		let c = [4.0 / 255.0, 4.0 / 255.0, 4.0 / 255.0, a];
		let px = |x: f32, y: f32, rw: f32, rh: f32| [ox + x * k, y * k, rw * k, rh * k];
		overlay::rect(out, px(cx - hw, cy - hh + 4.0, 2.0 * hw, (2.0 * hh - 8.0).max(0.0)), c);
		overlay::rect(out, px(cx - hw + 4.0, cy - hh, (2.0 * hw - 8.0).max(0.0), 4.0), c);
		overlay::rect(out, px(cx - hw + 4.0, cy + hh - 4.0, (2.0 * hw - 8.0).max(0.0), 4.0), c);
		// The help icon in states 2-3: FUN_001f9218(4) = fx texture record 4, the Gadgetron logo,
		// 64x64 centred on the box (ingame_menu_leftovers.md P1-a).
		if let (true, Some(tex)) = (matches!(self.state, 2 | 3), icon) {
			out.push(Quad { tex, rect: px(cx - 32.0, cy - 32.0, 64.0, 64.0), uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, 1.0], angle: 0.0, corners: None });
		}
		if text_a > 0.0 {
			let top = cy - lines.len() as f32 * LINE / 2.0;
			for (i, l) in lines.iter().enumerate() {
				let x = cx - width(font, l) / 2.0;
				text_line(out, font, l, ox + x * k, (top + i as f32 * LINE) * k, k, [TEXT[0], TEXT[1], TEXT[2], text_a]);
			}
		}
	}
}

fn glyph(font: &(usize, Vec<Glyph>), c: char) -> Option<Glyph> {
	font.1.get(c as usize).copied().filter(|g| g.adv != 0)
}

/// Width at scale 1 (control bytes like the 0x0c/0x08 highlight marks have no glyph).
pub fn width(font: &(usize, Vec<Glyph>), s: &str) -> f32 {
	s.chars().filter_map(|c| glyph(font, c)).map(|g| g.adv as f32).sum()
}

/// Splits at 0x01 newlines and wraps words to `max` pixels.
pub fn wrap(font: &(usize, Vec<Glyph>), s: &str, max: f32) -> Vec<String> {
	let mut out = Vec::new();
	for para in s.split('\u{1}') {
		let mut line = String::new();
		for word in para.split(' ').filter(|w| !w.is_empty()) {
			let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
			if !line.is_empty() && width(font, &cand) > max {
				out.push(std::mem::replace(&mut line, word.to_string()));
			} else {
				line = cand;
			}
		}
		out.push(line);
	}
	out
}

/// One line of the game font at (x, y) window pixels, scale k, with no shadow.
pub fn text_line(out: &mut Vec<Quad>, font: &(usize, Vec<Glyph>), s: &str, x: f32, y: f32, k: f32, colour: [f32; 4]) {
	text_line_xy(out, font, s, x, y, k, k, colour)
}

/// `text_line` with separate horizontal and vertical scales (cells stay 16x16 texels, advances scale by `kx`).
pub fn text_line_xy(out: &mut Vec<Quad>, font: &(usize, Vec<Glyph>), s: &str, x: f32, y: f32, kx: f32, ky: f32, colour: [f32; 4]) {
	let mut cx = x;
	for c in s.chars() {
		let Some(g) = glyph(font, c) else { continue };
		if c != ' ' {
			let (u, v) = (g.u as f32, g.v as f32);
			out.push(Quad { tex: font.0, rect: [cx, y + g.dy as f32 * ky, 16.0 * kx, 16.0 * ky], uv: [u / 256.0, v / 128.0, (u + 16.0) / 256.0, (v + 16.0) / 128.0], colour, angle: 0.0, corners: None });
		}
		cx += g.adv as f32 * kx;
	}
}

#[cfg(test)]
mod tests {
	// UI-TEXT-1 test (2): the help text colour is 0x80ffa888 as bytes / 128.

	use super::*;

	#[test]
	fn log_order() {
		let mut log = HelpLog::default();
		log.add(5);
		log.add(4);
		log.add(999); // not in the table: not logged
		log.add(5); // a repeat moves to the end
		assert_eq!(log.0, [3, 4, 5]);
		assert_eq!([5, 4, 3].map(title), [Some(0x5278), Some(0x5277), Some(0x5272)]);
	}

}
