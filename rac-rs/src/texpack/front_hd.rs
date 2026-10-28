//! [PC] Texture packs for the pictures the mirror draws itself (stage 3 of lane v041_hud): the front end (title logo, Press Start, the
//! menu panel frame, the title level's HUD icons) is not drawn from GS memory but from textures decoded off the disc
//! (`viewer/front_mirror.rs`, `frontend.rs`), so the GPU capture's run-time naming (`gsname`, `hudrepl`) never sees them. Here they get
//! the same PCSX2 names the GS would give them and the same lookup (`hudrepl::resolve`: index first, then the PAL <-> USA twins):
//! - fx records (Press Start, panel noise / scanlines / bevel) and HUD sprites (panel underlay, icons) are PSMT8 + CT32 palette,
//!   named like every other PSMT8 texture (`extra::ExtraSource::name`, `hud::Sprite::name`);
//! - the logo is a 256x128 PSMCT32 picture: laid out in a scratch GS memory at the position a GS upload puts it and named by
//!   `gsname::name_for_draw`, the function the capture uses.
//! The decoded pack image replaces the `Texture` itself (engine alpha scale), exactly like `extra_hd::apply_fx` does for the world's
//! fx textures; the quads use normalised UVs, so the layout does not change. With no textures mod enabled (or `RAC_TEXPACK_HUD=0`)
//! every function returns at its first line and the pictures are the disc's.

use super::name::TexName;
use super::{extra, extra_hd, gsname, hudrepl, index::PackFile, packs, xmap};
use crate::bytes::{span, u32_at};
use crate::level::{Level, Texture};

/// Whether front-end pictures take pack images: a textures mod is enabled and `RAC_TEXPACK_HUD` is not `0`.
pub fn active() -> bool {
	hudrepl::enabled()
}

/// The PCSX2 name of fx record `k` of the level (index +0x58 table: palette, pixels, w, h; PSMT8 + CT32 palette). None without a pack.
pub fn fx_name(l: &Level, k: usize) -> Option<TexName> {
	if !active() {
		return None;
	}
	let (count, table, base) = (u32_at(&l.index, 0x58).ok()? as usize, u32_at(&l.index, 0x5c).ok()? as usize, u32_at(&l.index, 0x68).ok()? as usize);
	if k >= count {
		return None;
	}
	let r = span(&l.index, table + k * 16, 16).ok()?;
	let (pal, pix, w, h) = (u32_at(r, 0).ok()? as usize, u32_at(r, 4).ok()? as usize, u32_at(r, 8).ok()? as usize, u32_at(r, 12).ok()? as usize);
	let (pal, px) = (span(&l.core, base + pal, 1024).ok()?, span(&l.core, base + pix, w * h).ok()?);
	(w > 0 && h > 0).then(|| extra::ExtraSource { kind: extra::Kind::Fx, key: extra::FX_KEY | k as u32, w, h, pal, px }.name())
}

/// The PCSX2 name of the title logo (`frontend::logo`: 256x128 PSMCT32 at core + index[0x84], alpha 0x80 = opaque). The picture is
/// written into a scratch local memory the way a GS upload lays out a 256-wide CT32 image (TBW 4) and named like a draw that samples
/// it. None without a pack.
pub fn logo_name(l: &Level) -> Option<TexName> {
	if !active() {
		return None;
	}
	let at = u32_at(&l.index, 0x84).ok()? as usize;
	ct32_name(span(&l.core, at, 256 * 128 * 4).ok()?, 256, 128)
}

/// The name of a PSMCT32 picture of `w` x `h` (powers of two, `w` a multiple of 64; `px` = w * h RGBA words as the GS holds them):
/// uploaded at TBP 0 with TBW = w / 64 into a scratch local memory, named by `gsname::name_for_draw`.
pub fn ct32_name(px: &[u8], w: u32, h: u32) -> Option<TexName> {
	if !w.is_power_of_two() || !h.is_power_of_two() || w < 64 || px.len() != (w * h * 4) as usize {
		return None;
	}
	let lut = crate::gsfull::gs_mem::luts();
	let mut vram = vec![0u8; crate::gsfull::gs_mem::VRAM_SIZE];
	for y in 0..h {
		for x in 0..w {
			let a = crate::gsfull::gs_mem::a32(lut, 0, w / 64, x, y);
			vram[a..a + 4].copy_from_slice(&px[((y * w + x) * 4) as usize..][..4]);
		}
	}
	let tex0 = u64::from(w / 64) << 14 | u64::from(w.trailing_zeros()) << 26 | u64::from(h.trailing_zeros()) << 30;
	gsname::name_for_draw(&vram, tex0, 0, 0, 0, false).map(|n| n.name)
}

/// The files the active packs have for `named` ((index into the caller's list, name)).
fn files(named: &[(usize, TexName)]) -> Vec<(usize, PackFile)> {
	let Some(p) = packs() else { return Vec::new() };
	named.iter().filter_map(|(i, n)| hudrepl::resolve(&p.index, xmap::shipped(), n).map(|f| (*i, f))).collect()
}

/// Replaces `tex[i]` by the pack's image for each `(i, name)` the packs have a decodable file for (decoded in parallel on the loader
/// threads); returns how many were replaced. The log line counts the asked, found and replaced names.
pub fn apply(tex: &mut [Texture], named: &[(usize, TexName)]) -> usize {
	if !active() || named.is_empty() {
		return 0;
	}
	let t0 = std::time::Instant::now();
	let want = files(named);
	let found = want.len();
	let got = extra_hd::decode_all(&want);
	let n = got.len();
	for (i, t) in got {
		if let Some(slot) = tex.get_mut(i) {
			*slot = t;
		}
	}
	eprintln!("texpack: front end art: {} named, {found} in the packs, {n} replaced in {:.0} ms", named.len(), t0.elapsed().as_secs_f64() * 1000.0);
	n
}

/// `t` or the pack's image for `name` (one picture: the title logo, a Press Start caption).
pub fn replace(name: Option<TexName>, t: Texture) -> Texture {
	let Some(name) = name else { return t };
	let mut one = [t];
	apply(&mut one, &[(0, name)]);
	let [t] = one;
	t
}

/// The title logo, replaced by the pack's picture when it has one (`frontend::logo` result in, texture out).
pub fn logo(l: &Level, t: Texture) -> Texture {
	replace(logo_name(l), t)
}

/// Press Start caption `i` (fx record 4 + i), replaced by the pack's picture when it has one.
pub fn press_start(l: &Level, i: usize, t: Texture) -> Texture {
	replace(fx_name(l, 4 + i), t)
}

#[cfg(test)]
#[path = "front_hd_tests.rs"]
mod tests;
