//! Key names in the HelpDesk tutorial boxes (issue #36: "Mouse and Keyboard Buttons").
//!
//! The text lookup (`prompts::lookup`) answers key names for every string the game fetches by id. The HelpDesk popups ("To use
//! your Bomb Glove, press [Circle] to equip it ...") do not use it: the help director (Veldin FUN_002036d0 at message start,
//! FUN_002042f8 each frame, [C] decomp/notes/ingame_menu_leftovers.md P1-a) reads the string pointer straight from the help
//! table (`*(DAT_0015f640 + entry * 16)`) and hands it to the box text wrapper FUN_001fc058 (area, colour, str, len = -1), which
//! measures and draws. So those popups kept the pad glyphs on a keyboard.
//!
//! Fix: a relocatable hand on that 27-instruction wrapper (found in every level overlay and on the USA disc by its code words, the
//! relocated fields masked: `hs_reloc.rs`; spec from tools/mkreloc.py, checked against all 19 levels). With the keyboard as the
//! device and a glyph in the string it points a2 at a pool copy with the glyphs spelled out (`prompts::spell_arg`, the same
//! substitution and pool as the lookup), then declines: retail's own wrapper measures, wraps and draws the copy, so the box is
//! sized for the key names. A gamepad under a remap (v0.5.1): the same copy with the glyph codes / names of the buttons the actions are
//! on now (`prompt_pad`); an unmapped gamepad: nothing is touched. The wrapper is also called with strings that came out of the lookup:
//! those hold no glyph any more, so nothing changes for them.
//! Registered relocatable hands (PC hooks) are installed whatever `RAC_HS` says: it switches only the built-in speed hands (oracle `rec::Table::build`).

use ee_oracle::cpu::Cpu;
use ee_oracle::hs_reloc::{Reloc, RelocHand};
use std::sync::OnceLock;

/// FUN_001fc058 of Veldin (0x1fc058, 0x6c bytes); the masks are its `jal` targets (0x28, 0x48) and the font table address (0x4c).
const SPEC: &str = "reloc glyph_box entry 0x1fc058 len 0x6c\nmhash 0xdb86dd9fc4948660\nmask 0x28 0x3ffffff\nmask 0x48 0x3ffffff\nmask 0x4c 0xffff\n";

fn reloc() -> &'static Reloc {
	static R: OnceLock<Reloc> = OnceLock::new();
	R.get_or_init(|| Reloc::parse(SPEC).expect("the box text wrapper spec parses"))
}

/// a2 = the string, a3 = its length (-1: up to the NUL, the only form the help director uses).
fn box_text(c: &mut Cpu) -> bool {
	box_text_for(c, super::prompts::keyboard_now())
}

/// `box_text` with the device given (tests): always declines; on the keyboard with a3 = -1 a2 may point at the pool copy.
fn box_text_for(c: &mut Cpu, keyboard: bool) -> bool {
	// A keyboard: key names. A controller under a remap: the glyph codes / names of the buttons the actions are on now (B1, v0.5.1).
	if c.g(7) as u32 == u32::MAX {
		super::prompts::spell_arg_for(c, 6, keyboard);
	}
	false
}

/// Registers the hand (before the recompiled table is built: `prompts::init`).
pub fn register() {
	static HAND: RelocHand = RelocHand { name: "prompts_box", reloc, est: ee_oracle::hs::EXACT, f: box_text };
	ee_oracle::rec::register_reloc_hand(&HAND);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The hand's behaviour: the keyboard + a glyph string + a3 = -1 points a2 at a pool copy with key names; an unmapped gamepad, another a3, a string
	/// without a glyph or one outside RAM change nothing; the hand always declines (retail measures and draws).
	#[test]
	fn the_hand_copies_for_the_keyboard_and_declines() {
		use ee_oracle::mem::Mem;
		let _g = crate::retail::pcopts::globals_guard(); // the pad-remap tests share the kbm / device globals
		let at = 0x0030_0000u32;
		let put = |c: &mut Cpu, a: u32, s: &[u8]| {
			for (i, b) in s.iter().chain(&[0u8]).enumerate() {
				c.mem.wr8(a + i as u32, *b);
			}
		};
		let call = |c: &mut Cpu, keyboard: bool, a2: u32, a3: u64| {
			c.s(6, a2 as u64);
			c.s(7, a3);
			let declined = !box_text_for(c, keyboard);
			(declined, c.g(6) as u32)
		};
		let mut c = Cpu::new(Mem::new());
		put(&mut c, at, b"Press  to fire");
		let (declined, a2) = call(&mut c, true, at, u64::MAX);
		assert!(declined && a2 != at, "keyboard: a2 now points at the pool copy");
		let mut copy = Vec::new();
		while c.mem.rd8(a2 + copy.len() as u32) != 0 {
			copy.push(c.mem.rd8(a2 + copy.len() as u32));
		}
		assert_eq!(copy, b"Press LMB to fire");
		assert_eq!(c.mem.rd8(at + 6), 0x11, "the retail string is untouched");
		assert_eq!(call(&mut c, false, at, u64::MAX), (true, at), "gamepad: nothing");
		// A controller under a remap (Jump on B): the Cross glyph becomes the Circle cell in the copy.
		{
			use rac_engine::input::kbm;
			kbm::init(None, None, None, None);
			crate::kbm_actions::install();
			let jump = kbm::table().index("jump").unwrap();
			kbm::assign_pad(jump, 0, 1);
			put(&mut c, at + 0x200, b"Press \x10 to jump");
			let (declined, a2) = call(&mut c, false, at + 0x200, u64::MAX);
			assert!(declined && a2 != at + 0x200, "remapped pad: a pool copy");
			let mut copy = Vec::new();
			while c.mem.rd8(a2 + copy.len() as u32) != 0 {
				copy.push(c.mem.rd8(a2 + copy.len() as u32));
			}
			assert_eq!(copy, b"Press \x11 to jump");
			kbm::init(None, None, None, None);
		}
		assert_eq!(call(&mut c, true, at, 20), (true, at), "a3 is a length: nothing");
		put(&mut c, at + 0x100, b"No glyph here");
		assert_eq!(call(&mut c, true, at + 0x100, u64::MAX), (true, at + 0x100), "no glyph: nothing");
		assert_eq!(call(&mut c, true, 0x1000, u64::MAX), (true, 0x1000), "not in RAM: nothing");
	}

	/// TB2-B1 / S1: a string the lookup answered under a pad remap is a pool copy; both wrapper hooks (box text and font line) leave it
	/// alone, so a swap (Jump on B, Fire on A) or a 3-cycle shows each button once-rewritten, not permuted twice. A string in the pool that
	/// the lookup kept standard (a menu id) stays standard too.
	#[test]
	fn pool_copies_are_not_rewritten_again_on_either_hook() {
		use ee_oracle::mem::Mem;
		use rac_engine::input::kbm;
		let _g = crate::retail::pcopts::globals_guard();
		crate::retail::prompts::test_set_pad_device(true);
		let read = |c: &mut Cpu, a: u32| {
			let mut v = Vec::new();
			while c.mem.rd8(a + v.len() as u32) != 0 {
				v.push(c.mem.rd8(a + v.len() as u32));
			}
			v
		};
		let put = |c: &mut Cpu, a: u32, s: &[u8]| {
			for (i, b) in s.iter().chain(&[0u8]).enumerate() {
				c.mem.wr8(a + i as u32, *b);
			}
		};
		// (assignments as (action, pad slot-0 PHYS index)), the retail string, what ONE rewrite gives.
		let cases: [(&[(&str, usize)], &[u8], &[u8]); 2] = [
			(&[("jump", 1), ("fire", 0)], b"\x10 jump \x11 fire", b"\x11 jump \x10 fire"),
			(&[("jump", 1), ("fire", 2), ("wrench", 0)], b"\x10 jump \x11 fire \x13 swing", b"\x11 jump \x13 fire \x10 swing"),
		];
		for (assigns, retail, once) in cases {
			kbm::init(None, None, None, None);
			crate::kbm_actions::install();
			for (name, p) in assigns {
				let a = kbm::table().index(name).unwrap();
				kbm::assign_pad(a, 0, *p);
			}
			let mut c = Cpu::new(Mem::new());
			let (src, copy) = (0x0030_0000u32, crate::retail::prompts::POOL + 0x400 * 3);
			put(&mut c, src, retail);
			// The retail string through each hook once: the rewrite.
			c.s(6, src as u64);
			c.s(7, u64::MAX);
			assert!(!box_text_for(&mut c, false));
			let at = c.g(6) as u32;
			assert_eq!(read(&mut c, at), once, "box hook: one rewrite");
			c.s(7, src as u64);
			assert!(!crate::retail::prompts::glyph_draw_hook(&mut c));
			let at7 = c.g(7) as u32;
			assert_eq!(read(&mut c, at7), once, "font hook: one rewrite");
			// The lookup's pool copy (made by the same rewrite) through each hook: unchanged, same pointer.
			put(&mut c, copy, once);
			c.s(6, copy as u64);
			c.s(7, u64::MAX);
			box_text_for(&mut c, false);
			assert_eq!((c.g(6) as u32, read(&mut c, copy)), (copy, once.to_vec()), "box hook: pool copy untouched");
			c.s(7, copy as u64);
			crate::retail::prompts::glyph_draw_hook(&mut c);
			assert_eq!((c.g(7) as u32, read(&mut c, copy)), (copy, once.to_vec()), "font hook: pool copy untouched");
			// A menu text the lookup kept standard sits in the pool with its standard glyphs: the hooks do not apply the gameplay layout.
			put(&mut c, copy, retail);
			c.s(6, copy as u64);
			c.s(7, u64::MAX);
			box_text_for(&mut c, false);
			c.s(7, copy as u64);
			crate::retail::prompts::glyph_draw_hook(&mut c);
			assert_eq!((c.g(6) as u32, c.g(7) as u32, read(&mut c, copy)), (copy, copy, retail.to_vec()), "standard pool copy stays standard");
		}
		kbm::init(None, None, None, None);
		crate::retail::prompts::test_set_pad_device(false);
	}

	/// The 27 code words of Veldin's FUN_001fc058 and of Novalis' twin (0x21dbd8): different `jal` targets and font table address,
	/// the same masked hash; any other code gives another one.
	#[test]
	fn the_spec_matches_the_wrapper_of_every_level() {
		const VELDIN: [u32; 27] = [
			0x27bdffb0, 0x7fb00000, 0x7fb30030, 0x0080802d, 0x7fb20020, 0x00e0982d, 0x7fb10010, 0x00c0902d, 0x7fbf0040, 0x00a0882d, 0x0c07e486, 0x24040002,
			0x3c09001c, 0x0200202d, 0x0220282d, 0x0240302d, 0x0260382d, 0x0040402d, 0x0c07eebe, 0x252934f0, 0x7bbf0040, 0x7bb30030, 0x7bb20020, 0x7bb10010,
			0x7bb00000, 0x03e00008, 0x27bd0050,
		];
		let r = reloc();
		assert_eq!(r.len, 4 * VELDIN.len() as u32);
		let hash = |w: &[u32; 27]| r.masked_hash_by(0x1fc058, |a| w[((a - 0x1fc058) / 4) as usize]);
		assert_eq!(hash(&VELDIN), r.mhash);
		let mut nov = VELDIN;
		nov[10] = 0x0c086b66;
		nov[18] = 0x0c08759e;
		nov[19] = 0x252938f0;
		assert_eq!(hash(&nov), r.mhash);
		let mut other = VELDIN;
		other[2] = 0;
		assert_ne!(hash(&other), r.mhash);
	}
}
