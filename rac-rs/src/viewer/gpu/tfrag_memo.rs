//! [PC] perf (lane ncull, 2026-10-29): `Gpu::cull_tfrags` poses a fragment (`terrain::pose`) only when its result can differ from what
//! the fragment's vertex range already holds.
//!
//! `pose` reads the camera only through the view depth `z` in the morph bands (codes 8, 0xa, 0xe, 0x10) and the straddle collapse
//! (codes 8, 0xe). The far mesh (code 6) and the plain near mesh (code 0x14) are the fragment's static positions with its current
//! colours (`tfrag_cols[i]`), whatever the camera or the band distances are. So a fragment that is drawn with code 6 or 0x14 again, into
//! a range that last got the same code, with colours unchanged since, would be posed to bytes equal to the range's: `cull_tfrags`'
//! compare finds no change and writes nothing. This skips that pose (the allocation of three vectors per fragment and the compare)
//! and the join of the frame worker pool when no job is left. Same vertices, same writes, same picture.
//!
//! `memo[i][lod]` = the code last posed into fragment i's LOD range (6 or 0x14), else 0. Relight (`Gpu::relight`) clears the fragment's
//! entries when it replaces its colours. RAC_TFRAG_MEMO=0 poses every drawn fragment as before; RAC_CULL_CHECK=1 poses them too and
//! counts any skipped one whose pose would have changed the range (`ctf_memo_bad` span, must stay 0).

/// The codes whose pose does not depend on the camera or on the band distances (`terrain::pose`).
pub(super) fn camera_free(code: u8) -> bool {
	matches!(code, 6 | 0x14)
}

/// Whether the pose of fragment `i` with `code` into LOD range `lod` can be skipped (see the module doc).
pub(super) fn skip(memo: &[[u8; 3]], i: usize, lod: usize, code: u8) -> bool {
	camera_free(code) && memo.get(i).is_some_and(|m| m[lod] == code)
}

/// After the pose of `code` into LOD range `lod` of fragment `i` (or its skip).
pub(super) fn record(memo: &mut [[u8; 3]], i: usize, lod: usize, code: u8) {
	memo[i][lod] = if camera_free(code) { code } else { 0 };
}

/// A relight replaced fragment `i`'s colours: nothing in its ranges is known any more.
pub(super) fn forget(memo: &mut [[u8; 3]], i: usize) {
	if let Some(m) = memo.get_mut(i) {
		*m = [0; 3];
	}
}

/// RAC_TFRAG_MEMO=0 turns the skip off (A/B and fault isolation); on by default.
pub(super) fn on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_TFRAG_MEMO").map_or(true, |v| v != "0"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::terrain::{pose, TfragCull, TfragLods};

	/// A fragment with morph and snap tables on both stages, so that the morphing codes would differ from the plain ones.
	fn frag() -> TfragLods {
		let mut mesh = crate::mesh::Mesh::new("t".into());
		mesh.positions = vec![[0.0; 3]; 4];
		mesh.uvs = vec![[0.0, 0.0], [0.25, 0.5], [0.75, 0.75], [0.5, 0.5]];
		TfragLods {
			materials: Vec::new(),
			face_mat: Default::default(),
			far: mesh.clone(),
			mid: mesh.clone(),
			near: mesh,
			pos_world: vec![[0.0; 3], [4.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
			vert_pos: vec![0, 1, 2, 3],
			lit: crate::mesh::Mesh::new("l".into()),
			morph: [vec![[2, 0, 1]], vec![[3, 0, 2]]],
			snap: [vec![[2, 0, 1]], vec![[3, 0, 1]]],
			cull: TfragCull { centre: [0.0; 3], radius: 1.0, corners: [[0.0; 3]; 8], force_far: false, occ: (0x80, 0x7f), id: 0 },
		}
	}

	fn bytes(p: &crate::terrain::Posed) -> Vec<u8> {
		[bytemuck::cast_slice::<_, u8>(&p.pos), bytemuck::cast_slice(&p.col), bytemuck::cast_slice(&p.uv)].concat()
	}

	/// The claim the skip rests on: for codes 6 and 0x14 the pose does not read the view depth, the band distances or the clip-w scale
	/// (and for every morphing code it does, so those never skip).
	#[test]
	fn plain_codes_ignore_the_camera_and_the_bands() {
		let l = frag();
		let cols = [[1.0, 2.0, 3.0, 128.0], [40.0, 0.0, 200.0, 128.0], [100.0, 50.0, 0.0, 128.0], [7.0; 4]];
		let far = |p: [f32; 3]| 1000.0 + p[0];
		let near = |p: [f32; 3]| 5.0 - p[0];
		for code in [6u8, 0x14] {
			let a = pose(&l, code, [60.0, 40.0, 20.0], -0.857234, &cols, &far);
			let b = pose(&l, code, [90.0, 70.0, 30.0], -0.5, &cols, &near);
			assert_eq!(bytes(&a), bytes(&b), "code {code:#x}");
			assert!(camera_free(code));
		}
		for code in [8u8, 0xa, 0xe, 0x10] {
			let a = pose(&l, code, [60.0, 40.0, 20.0], -0.857234, &cols, &|_| 50.0);
			let b = pose(&l, code, [60.0, 40.0, 20.0], -0.857234, &cols, &|_| 30.0);
			assert_ne!(bytes(&a), bytes(&b), "code {code:#x} must read the depth");
			assert!(!camera_free(code));
		}
	}

	/// The skip needs the same plain code in the same LOD range as the last pose; a morphing code in between, a relight or a new
	/// fragment poses again.
	#[test]
	fn skip_only_after_the_same_plain_code() {
		let mut memo = vec![[0u8; 3]; 2];
		assert!(!skip(&memo, 0, 2, 0x14));
		record(&mut memo, 0, 2, 0x14);
		assert!(skip(&memo, 0, 2, 0x14));
		assert!(!skip(&memo, 0, 1, 0x14), "another LOD range");
		assert!(!skip(&memo, 1, 2, 0x14), "another fragment");
		assert!(!skip(&memo, 0, 2, 6), "another code");
		record(&mut memo, 0, 2, 0x10);
		assert!(!skip(&memo, 0, 2, 0x14), "a morphing pose was written in between");
		assert!(!skip(&memo, 0, 2, 0x10), "morphing codes never skip");
		record(&mut memo, 0, 0, 6);
		assert!(skip(&memo, 0, 0, 6));
		forget(&mut memo, 0);
		assert!(!skip(&memo, 0, 0, 6) && !skip(&memo, 0, 2, 0x14), "relight clears the fragment");
		assert!(!skip(&memo, 9, 0, 6), "no entry: pose");
	}
}
