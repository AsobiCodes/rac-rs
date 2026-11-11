//! The per-part uniform block of the fur shader (`FurPart` in fur.wgsl): one `SLOT` bytes per part, picked by a dynamic offset.

use crate::furspec::{shells, FurSpec};

/// Bytes per part slot (the dynamic-offset alignment every backend allows).
pub(super) const SLOT: u32 = 256;

/// Bytes of one part's values (`FurPart` in fur.wgsl: five vec4).
pub(super) const SIZE: u64 = 80;

/// The values of one part for level `q` (1..=3) and style `fluffy`: five vec4 as `FurPart` in fur.wgsl reads them.
pub(super) fn part_values(spec: &FurSpec, part: usize, q: u8, fluffy: bool, weld: bool) -> [[f32; 4]; 5] {
	let p = &spec.parts[part];
	let s = fluffy as usize;
	[
		[p.len[s], p.density, p.comb[s], shells(q, fluffy) as f32],
		[p.tip[0], p.tip[1], p.tip[2], p.root],
		[p.thick[s], p.sheen[s], s as f32, weld as u32 as f32],
		[p.halo[s], p.occ, p.spec, p.spec_exp],
		[p.lift, p.sharp, p.strand, 0.0],
	]
}

/// All part slots, ready for one `write_buffer`.
pub(super) fn block(spec: &FurSpec, q: u8, fluffy: bool, weld: bool) -> Vec<u8> {
	let mut out = vec![0u8; spec.parts.len() * SLOT as usize];
	for i in 0..spec.parts.len() {
		let v = part_values(spec, i, q, fluffy, weld);
		out[i * SLOT as usize..][..SIZE as usize].copy_from_slice(bytemuck::cast_slice(&v));
	}
	out
}
