//! The blend from the modern camera's last pose back to retail's (modcam.rs `step`, after a hand-back; moved out of it 2026-11-05).

use super::*;

/// One field of the blend-out (`st.blend` > 0): the pose written is the last modern one mixed towards retail's by a smoothstep.
pub(super) fn blend_back(c: &mut Cpu, rec: u32, st: &mut St) {
	// Blend from the last modern pose to retail's.
	st.blend -= 1;
	let t = 1.0 - st.blend as f32 / BLEND as f32;
	let t = t * t * (3.0 - 2.0 * t);
	let re = rv(c, rec + 0x30);
	let (e0, y0, p0) = st.last;
	let e = [e0[0] + (re[0] - e0[0]) * t, e0[1] + (re[1] - e0[1]) * t, e0[2] + (re[2] - e0[2]) * t];
	let rr = [rv(c, rec), rv(c, rec + 0x10), rv(c, rec + 0x20)];
	if st.fr.flat() && rr[1][2].abs() < 1e-4 {
		let (ry, rp) = rec_angles(c, rec);
		let y = y0 + wrap(ry - y0) * t;
		let p = p0 + (rp - p0) * t;
		st.last_rows = write_pose(c, rec, e, &FLAT, y, p);
		st.last = (e, y, p);
	} else {
		// A rolled view (magnetic wall) on either end: blend the rows themselves and keep them orthonormal.
		let l = st.last_rows;
		let mix = |i: usize| [l[i][0] + (rr[i][0] - l[i][0]) * t, l[i][1] + (rr[i][1] - l[i][1]) * t, l[i][2] + (rr[i][2] - l[i][2]) * t];
		let f = unit3(mix(0));
		let m2 = mix(2);
		let d = dot3(m2, f);
		let u = unit3([m2[0] - f[0] * d, m2[1] - f[1] * d, m2[2] - f[2] * d]);
		let rows3 = [f, cross3(u, f), u];
		write_rows(c, rec, e, &rows3);
		st.last_rows = rows3;
		st.last = (e, f[1].fatan2(f[0]), (-f[2]).clamp(-1.0, 1.0).fasin());
	}
}
