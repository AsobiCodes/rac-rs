//! [PC] Free camera (devtools F2, or the test camera RAC_VEG_CAM): Hoven's abyss water seen from any view.
//!
//! Retail draws the wave plane in the queue-C callback FUN_0030b798 -> FUN_0030b6c0 (12_hoven overlay.elf.c 165911-166004;
//! decomp/notes/draw_producers.md "Hoven 0x30b798"): five static triangle strips (counts 0x205980, xyz f32 tables 0x208998,
//! RGBA tables 0x2089e0, UV tables 0x2089b0 plus the UV scroll at 0x162078 + 8 * layer, FUN_0026646c) go through FUN_00220898
//! to the VU1 program (MSCAL 0xe), which projects and clips them for the GAME camera. The effects layer (efx_draw.rs) un-projects
//! what that program kicked, so the strips only hold what the game camera has in view [C: 0600 game camera on a door 403
//! vertices, turned towards the abyss 1173]. A free camera elsewhere saw a patch or nothing ("the water just disappeared and the
//! fog remained", user 2026-10-25). With the free camera on, the two layers (tex 0x2f FIX 0x7f lerp, then tex 0x30 additive
//! with FIX = DAT_00162054) are built here from those tables in world space and replace the clipped ones.
//! Off (the game camera) nothing runs: pictures unchanged. RAM is only read; the PAL code words are checked (`SIG`), so another
//! build of the overlay keeps the old behaviour. No retail fog on these (the VU1 fog byte belongs to the game camera).
use super::efx_draw;
use super::efx_own::Own;
use crate::particles::WorldStrip;
use crate::retail::disc::{usa, va};
use ee_oracle::Machine;

const LEVEL: u32 = 12;
/// The callback that draws the abyss water (the owner must have run this field).
const OWNER: u32 = 0x30b798;
/// FUN_0030b6c0's table loads (addiu words): uv table 0x2089b0, positions 0x208998, colours 0x2089e0, counts 0x205980,
/// scroll 0x162078.
const SIG: [(u32, u32); 5] = [(0x30b6fc, 0x2454_89e0), (0x30b704, 0x2473_8998), (0x30b70c, 0x24d2_89b0), (0x30b710, 0x24b0_5980), (0x30b718, 0x26f7_2078)];
const COUNTS: u32 = 0x205980;
const POS: u32 = 0x208998;
const COL: u32 = 0x2089e0;
const UV: u32 = 0x2089b0;
const SCROLL: u32 = 0x162078;
const FIX_ADD: u32 = 0x162054;
const PASSES: u32 = 5;
/// Sanity cap per pass (retail: 12, 40, 105, 148, 33).
const MAX_VERTS: u32 = 1024;
const TEX: [usize; 2] = [0x2f, 0x30];

fn veg_cam() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var_os("RAC_VEG_CAM").is_some())
}

/// The drawn view is not the game camera.
pub fn active() -> bool {
	super::devtools::free_on() || veg_cam()
}

/// FUN_0030b6c0's table loads are the code `SIG` names. USA: the same instructions, but the addiu immediates are the USA
/// table addresses, so only opcode and registers (the high half) are compared.
fn sig_ok(m: &Machine) -> bool {
	SIG.iter().all(|&(a, w)| {
		let x = m.rd32(va(a));
		if usa() { x >> 16 == w >> 16 } else { x == w }
	})
}

/// The two wave-plane layers in world space for this field (empty unless the free camera is on and Hoven drew its water).
pub fn capture(m: &Machine, level: u32, own: &[Own]) -> Vec<WorldStrip> {
	if level != LEVEL || !active() || !own.iter().any(|o| o.fnp == va(OWNER)) || !sig_ok(m) {
		return Vec::new();
	}
	// Every address below is a PAL overlay address of Hoven's: `va` gives the disc that runs its own.
	let (counts, pos_t, col_t, uv_t, scroll, fix_add) = (va(COUNTS), va(POS), va(COL), va(UV), va(SCROLL), va(FIX_ADD));
	let f = |a: u32| f32::from_bits(m.rd32(a));
	(0..2u32)
		.map(|layer| {
			let sc = [f(scroll + 8 * layer), f(scroll + 8 * layer + 4)];
			let (fix, add) = if layer == 0 { (0x7f, false) } else { (m.rd32(fix_add) as u8, true) };
			let mut s = WorldStrip { pos: Vec::new(), uv: Vec::new(), colour: Vec::new(), tex: efx_draw::fog_off(TEX[layer as usize]), fix, add };
			for p in 0..PASSES {
				let n = m.rd32(counts + 4 * p).min(MAX_VERTS);
				let (pa, ca, ua) = (m.rd32(pos_t + 4 * p), m.rd32(col_t + 4 * p), m.rd32(uv_t + 4 * p));
				for j in 0..n {
					let pos = [f(pa + 12 * j), f(pa + 12 * j + 4), f(pa + 12 * j + 8)];
					let uv = [f(ua + 8 * j) + sc[0], f(ua + 8 * j + 4) + sc[1]];
					let c = m.rd32(ca + 4 * j).to_le_bytes().map(|x| x as f32 / 128.0);
					if j == 0 && !s.pos.is_empty() {
						// Strips join through degenerate triangles (repeat the last and the first vertex), as efx_draw's runs.
						let (lp, lu, lc) = (*s.pos.last().unwrap(), *s.uv.last().unwrap(), *s.colour.last().unwrap());
						s.pos.extend([lp, pos]);
						s.uv.extend([lu, uv]);
						s.colour.extend([lc, c]);
					}
					s.pos.push(pos);
					s.uv.push(uv);
					s.colour.push(c);
				}
			}
			s
		})
		.filter(|s| s.pos.len() >= 3)
		.collect()
}

/// Puts `extra` (from `capture`) in place of retail's clipped wave-plane strips in `pools` (at the first one's index, so the
/// strip order the water pass keys its flows on is kept).
pub fn apply(extra: &[WorldStrip], pools: &mut Vec<WorldStrip>) {
	if extra.is_empty() {
		return;
	}
	let ours = |s: &WorldStrip| {
		let t = efx_draw::plain_tex(s.tex);
		t == TEX[0] && !s.add && s.fix == 0x7f || t == TEX[1] && s.add
	};
	let at = pools.iter().position(ours).unwrap_or(pools.len());
	pools.retain(|s| !ours(s));
	pools.splice(at..at, extra.iter().cloned());
}

#[cfg(test)]
mod tests {
	use super::*;

	fn strip(tex: usize, fix: u8, add: bool) -> WorldStrip {
		WorldStrip { pos: vec![[0.0; 3]; 3], uv: vec![[0.0; 2]; 3], colour: vec![[1.0; 4]; 3], tex, fix, add }
	}

	/// `apply` swaps both clipped layers for the built ones at the first layer's index and leaves other strips alone.
	#[test]
	fn apply_replaces_the_clipped_layers_in_place() {
		let mut pools = vec![strip(0x2c, 0x7f, false), strip(0x2f | efx_draw::FOG_ON, 0x7f, false), strip(0x30 | efx_draw::FOG_ON, 120, true), strip(0x41, 0x40, false)];
		let extra = vec![strip(efx_draw::fog_off(0x2f), 0x7f, false), strip(efx_draw::fog_off(0x30), 120, true)];
		apply(&extra, &mut pools);
		let t: Vec<usize> = pools.iter().map(|s| s.tex).collect();
		assert_eq!(t, vec![0x2c, efx_draw::fog_off(0x2f), efx_draw::fog_off(0x30), 0x41]);
		let mut same = pools.clone();
		apply(&[], &mut same);
		assert_eq!(same, pools);
	}
}
