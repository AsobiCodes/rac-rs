//! BC7 replacement files and the PS2 alpha scale. A pack file's alpha is on the PS2 scale (0x80 = opaque); the engine's is
//! `min(a * 2, 255)` (`dds::engine_alpha`). BC2 / BC3 get their alpha blocks re-coded (`bc`); a BC7 block cannot be re-coded, so `dds::load`
//! hands every BC7 file whose DX10 alpha mode is not "opaque" (3) to `apply`: PCSX2 ignores that field, so unknown / straight /
//! premultiplied / custom all mean "PS2 scale". If every texel of every stored level decodes to alpha 255 the file stays BC7 for the GPU, bytes
//! unchanged (the doubling would change nothing); otherwise every level is decoded (`bc7dec`) to RGBA8 with the alpha doubled, exactly what an
//! uncompressed DDS or a PNG gives, and the file goes on through the chain / budget / BC3 path like those.
//! Files from texconv / Compressonator rarely have exactly 255 everywhere (their BC7 encoders leave 251..254 in some texels), so most real
//! BC7 packs take the RGBA8 route.

use super::bc7dec;
use super::dds::{engine_alpha, Fmt, Image};

/// The DX10 header's alpha mode (DDS_ALPHA_MODE_MASK) of a DDS file, if the header is a DX10 one for BC7 (DXGI 98 or 99).
fn bc7_alpha_mode(header: &[u8]) -> Option<u32> {
	let u32_at = |at: usize| header.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
	if header.get(..4)? != b"DDS " || header.get(84..88)? != b"DX10" || !matches!(u32_at(128)?, 98 | 99) {
		return None;
	}
	Some(u32_at(144)? & 7)
}

/// From the first 148 bytes of a DDS file alone: whether the loader may turn this BC7 file into RGBA8 (every BC7 file except alpha mode 3
/// "opaque", which is always kept as BC7). false for anything that is not a DX10 BC7 header. A planner that sizes a file before decoding it
/// takes true as "RGBA8 sized" (the conservative choice; the loader keeps it BC7 when all alpha is 255).
pub fn bc7_may_decode(header: &[u8]) -> bool {
	bc7_alpha_mode(header).is_some_and(|m| m != 3)
}

/// Applies the rule above to a loaded BC7 `img` (alpha mode other than 3). Err if a stored level is short.
pub fn apply(img: Image) -> Result<Image, String> {
	let sizes = |k: usize| ((img.w >> k).max(1), (img.h >> k).max(1));
	let mut opaque = true;
	for (k, l) in img.levels.iter().enumerate() {
		let (w, h) = sizes(k);
		opaque = opaque && bc7dec::level_is_opaque(w, h, l)?;
		if !opaque {
			break;
		}
	}
	if opaque {
		return Ok(img);
	}
	let mut levels = Vec::with_capacity(img.levels.len());
	for (k, l) in img.levels.iter().enumerate() {
		let (w, h) = sizes(k);
		let mut px = bc7dec::decode_level(w, h, l)?;
		px.chunks_exact_mut(4).for_each(|p| p[3] = engine_alpha(p[3]));
		levels.push(px);
	}
	Ok(Image { fmt: Fmt::Rgba8, w: img.w, h: img.h, levels })
}
