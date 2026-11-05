//! Render-only pose overrides of the hero skeleton (OmniLoader "pose" block, stage A; spec
//! scratch_omni/private/arsenal-mastery/research/POSE_OVERRIDE.md). `JointFx` is the joint override record the renderer's own
//! evaluator (`Animated::local_fx_parts`) applies after the animation: retail's +0x64 records, the port's blink / Clank /
//! strafe records, and since this file the MIRROR op: a joint takes the mirror image of another joint's final local pose.
//! Nothing here touches game RAM: ops travel in `HeroParts.pose_ops` (empty until a host store fills them) and are applied on
//! the main thread, so the RAM hash is unaffected by construction.

use crate::anim::Animated;

/// One joint's local pose: (rotation quaternion x y z w, per-lane scale, raw-unit translation) as `local_fx_parts` returns it.
pub type Part = ([f32; 4], [f32; 3], [f32; 3]);

/// The `mirror` marker of a rotation SET op (`JointFx::set_rot`): past every skeleton, so `apply_mirrors` can tell it from a source joint.
pub const SET_ROT: u16 = 0xfffe;

/// One joint override record of the retail evaluator FUN_00268a80 (moby +0x64 list): `rot` (port quaternion sign),
/// per-lane `scale` and raw-unit `off`; `blend` None = mode 0 (multiply / scale / add), Some(w) = mode 1 (weighted
/// blend with weight w); `reg` orders the list (newest registration first = descending).
/// `mirror` Some(src) makes it a MIRROR op instead (rot / scale / off unused): after all records, the joint becomes the
/// mirror image of joint `src`'s final local pose (see `apply_mirrors`), `blend` None = weight 1, Some(w) = weight w.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointFx {
	pub joint: usize,
	pub rot: [f32; 4],
	pub scale: [f32; 3],
	pub off: [f32; 3],
	pub blend: Option<f32>,
	pub reg: u32,
	pub mirror: Option<u16>,
}

impl JointFx {
	/// A rotation-only mode-0 record.
	pub fn rot(joint: usize, rot: [f32; 4]) -> JointFx {
		JointFx { joint, rot, scale: [1.0; 3], off: [0.0; 3], blend: None, reg: 0, mirror: None }
	}

	/// A rotation SET op (pose overrides, `set_rotation`): after all records the joint's rotation moves the fraction `weight` of the
	/// way to `rot` (an absolute rotation in the parent's frame); scale and translation stay. Marked by `mirror == Some(SET_ROT)`.
	pub fn set_rot(dest: usize, rot: [f32; 4], weight: f32) -> JointFx {
		JointFx { mirror: Some(SET_ROT), blend: Some(weight), ..JointFx::rot(dest, rot) }
	}

	/// A mirror op: joint `dest` copies the mirror image of joint `src` with `weight` (1 = fully, 0 = not at all).
	pub fn mirror_from(dest: usize, src: usize, weight: f32) -> JointFx {
		JointFx { mirror: Some(src as u16), blend: Some(weight), ..JointFx::rot(dest, [0.0, 0.0, 0.0, 1.0]) }
	}
}

/// The mirror of a local pose across the parent's Y plane: with M = diag(1, -1, 1), a local transform [R(q) S | t] becomes
/// M [R S | t] M = [R(q') S | M t] with q' = (-x, y, -z, w) (an axial vector flips as (ax, ay, az) -> (-ax, ay, -az), the angle
/// stays) and the scale unchanged. Exact across the plane through the parent frame's origin with normal = its local Y.
pub fn mirrored(p: &Part) -> Part {
	let (q, s, t) = *p;
	([-q[0], q[1], -q[2], q[3]], s, [t[0], -t[1], t[2]])
}

/// `dest` moved the fraction `w` of the way to `m`: the rotation by a shortest-way nlerp, scale and translation linear.
fn towards(dest: &Part, m: &Part, w: f32) -> Part {
	let u = 1.0 - w;
	let (a, b) = (dest.0.map(|c| c * u), m.0.map(|c| c * w));
	let d = ((a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]) + a[3] * b[3];
	let v = if d < 0.0 { [0, 1, 2, 3].map(|i| a[i] - b[i]) } else { [0, 1, 2, 3].map(|i| a[i] + b[i]) };
	let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
	let q = if n > 0.0 { v.map(|c| c / n) } else { dest.0 };
	(q, [0, 1, 2].map(|i| dest.1[i] * u + m.1[i] * w), [0, 1, 2].map(|i| dest.2[i] * u + m.2[i] * w))
}

type M3 = [[f32; 3]; 3];

fn mm(a: &M3, b: &M3) -> M3 {
	[0, 1, 2].map(|r| [0, 1, 2].map(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

fn mv(a: &M3, v: [f32; 3]) -> [f32; 3] {
	[0, 1, 2].map(|r| a[r][0] * v[0] + a[r][1] * v[1] + a[r][2] * v[2])
}

/// The inverse, or None for a (nearly) singular matrix (a parent chain with a zero scale, e.g. a hidden joint: dividing by the
/// determinant would put infinity or NaN into the pose).
fn inverse(a: &M3) -> Option<M3> {
	let c = |r: usize, k: usize| a[(r + 1) % 3][(k + 1) % 3] * a[(r + 2) % 3][(k + 2) % 3] - a[(r + 1) % 3][(k + 2) % 3] * a[(r + 2) % 3][(k + 1) % 3];
	let det = a[0][0] * c(0, 0) + a[0][1] * c(0, 1) + a[0][2] * c(0, 2);
	if !(det.abs() >= 1e-8) {
		return None;
	}
	Some([0, 1, 2].map(|r| [0, 1, 2].map(|k| c(k, r) / det)))
}

/// Rotation matrix -> unit quaternion (x, y, z, w).
fn mat_quat(m: &M3) -> [f32; 4] {
	let tr = m[0][0] + m[1][1] + m[2][2];
	if tr > 0.0 {
		let s = (tr + 1.0).sqrt() * 2.0;
		[(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
	} else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
		let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
		[0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
	} else if m[1][1] > m[2][2] {
		let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
		[(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
	} else {
		let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
		[(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
	}
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
	let ([ax, ay, az, aw], [bx, by, bz, bw]) = (a, b);
	[aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

/// Model-space placement of joint `j` from the local parts (as `Animated::local_fx` builds matrices: R(q) S | t, composed down the
/// parents): (linear part, origin) and the rotation-only product of the normalised quaternions. None (the model frame) = identity.
fn world_of(parts: &[Part], parents: &[Option<usize>], j: Option<usize>) -> (M3, [f32; 3], M3) {
	let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
	let mut chain = Vec::new();
	let mut at = j;
	while let Some(k) = at.filter(|&k| k < parts.len() && chain.len() <= parts.len()) {
		chain.push(k);
		at = parents.get(k).copied().flatten();
	}
	let (mut lin, mut org, mut rot) = (id, [0.0; 3], id);
	for &k in chain.iter().rev() {
		let (q, s, t) = parts[k];
		let r = crate::anim::quat_mat(q);
		let l = [0, 1, 2].map(|i| [0, 1, 2].map(|c| r[i][c] * s[c]));
		let o = mv(&lin, t);
		org = [0, 1, 2].map(|i| org[i] + o[i]);
		lin = mm(&lin, &l);
		let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
		rot = mm(&rot, &crate::anim::quat_mat(if n > 0.0 { q.map(|c| c / n) } else { [0.0, 0.0, 0.0, 1.0] }));
	}
	(lin, org, rot)
}

/// The mirror image of `src` for joint `dest` across the BODY plane (model space Y = 0, the moby's left axis: the plane the hero is
/// symmetric about and the one a mod mirrors a held gun across): world(dest) = B world(src) M, B = M = diag(1, -1, 1), so
/// local(dest) = world(parent dest)^-1 B world(parent src) local(src) M. The rotation goes through the parents' rotation-only frames
/// (G_d^T B G_s M is a rotation, times the plain mirror `mirrored(q)`), the origin through the full placements. With both parents on
/// a symmetric pose this is `mirrored`; a twisted chest (the one-handed firing pose turns it) no longer swings the arm out sideways
/// (2026-10-25: Dual Blasters, the left gun pointed to the left while standing and shooting). A singular destination parent
/// chain (zero scale) has no inverse: the joint keeps its own pose (the op is skipped).
fn mirrored_body(parts: &[Part], parents: &[Option<usize>], dest: usize, src: usize) -> Part {
	let (ls, os, gs) = world_of(parts, parents, parents.get(src).copied().flatten());
	let (ld, od, gd) = world_of(parts, parents, parents.get(dest).copied().flatten());
	let b = |m: &M3| [m[0], m[1].map(|v| -v), m[2]];
	let gdt = [0, 1, 2].map(|r| [0, 1, 2].map(|c| gd[c][r]));
	let n = mm(&gdt, &b(&gs));
	let n = [0, 1, 2].map(|r| [n[r][0], -n[r][1], n[r][2]]);
	let (q, s, _) = mirrored(&parts[src]);
	let x = mv(&ls, parts[src].2);
	let x = [os[0] + x[0], -(os[1] + x[1]), os[2] + x[2]];
	let Some(inv) = inverse(&ld) else { return parts[dest] };
	let t2 = mv(&inv, [x[0] - od[0], x[1] - od[1], x[2] - od[2]]);
	(qmul(mat_quat(&n), q), s, t2)
}

/// The mirror ops of `fx`, in list order, applied to the per-joint local poses AFTER the record loop of `local_fx_parts`
/// (so a source joint is read with its records, layers and fades already in). An op naming a joint outside `parts` is
/// ignored; weight <= 0 changes nothing; weight >= 1 (or `blend` None) copies the mirrored source (no renormalisation
/// below a chain root: a same-sequence lerp leaves the source quaternion unnormalised and so does its mirror).
/// A chain root (a mirrored joint whose parent is not itself mirrored by an op of `fx`, e.g. the left shoulder under the chest)
/// is mirrored across the body plane (`mirrored_body`); the joints below it copy their source's local pose mirrored, so the whole
/// limb is the body-plane mirror image of the source limb.
pub(crate) fn apply_mirrors(parts: &mut [Part], fx: &[JointFx], parents: &[Option<usize>]) {
	let dest_of = |j: usize| fx.iter().any(|f| f.joint == j && f.mirror.is_some_and(|m| m != SET_ROT));
	for f in fx {
		let Some(src) = f.mirror.map(usize::from) else { continue };
		let w = f.blend.unwrap_or(1.0);
		if src == usize::from(SET_ROT) {
			if f.joint < parts.len() && w > 0.0 {
				let q = if w >= 1.0 { f.rot } else { towards(&parts[f.joint], &(f.rot, [1.0; 3], [0.0; 3]), w).0 };
				parts[f.joint].0 = q;
			}
			continue;
		}
		if f.joint >= parts.len() || src >= parts.len() {
			continue;
		}
		if w <= 0.0 {
			continue;
		}
		let root = !parents.get(f.joint).copied().flatten().is_some_and(dest_of);
		let m = if root { mirrored_body(parts, parents, f.joint, src) } else { mirrored(&parts[src]) };
		parts[f.joint] = if w >= 1.0 { m } else { towards(&parts[f.joint], &m, w) };
	}
}

/// A content hash of an op list for the hero pose cache key (viewer/herocache.rs): 0 for no ops, else never 0. Equal lists
/// hash equal, so a display frame without a tick reuses the posed bundle exactly while the ops stay the same.
pub fn ops_hash(ops: &[JointFx]) -> u64 {
	if ops.is_empty() {
		return 0;
	}
	let mut h = 0xcbf2_9ce4_8422_2325u64;
	let mut eat = |v: u32| {
		for b in v.to_le_bytes() {
			h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
		}
	};
	for f in ops {
		eat(f.joint as u32);
		f.rot.iter().chain(&f.scale).chain(&f.off).for_each(|c| eat(c.to_bits()));
		eat(f.blend.map_or(0x7fc0_1234, f32::to_bits));
		eat(f.reg);
		eat(f.mirror.map_or(0xffff_ffff, u32::from));
	}
	h | 1
}

/// True when the op set appears or disappears AT ONCE (the side that has ops weighs more than half): the smoothed hero then
/// has to draw this tick unblended (`Smooth::hero_cut`). A faded set (small weights ramping up or down) needs no cut.
pub fn ops_need_cut(old: &[JointFx], new: &[JointFx]) -> bool {
	let strong = |ops: &[JointFx]| ops.iter().any(|f| f.blend.unwrap_or(1.0) > 0.5);
	match (old.is_empty(), new.is_empty()) {
		(true, false) => strong(new),
		(false, true) => strong(old),
		_ => false,
	}
}

impl Animated {
	/// The bind-symmetry pairing of the joints: `Some(k)` = joint `j` is the mirror image of joint `k` (`k == j`: on the centre
	/// line), `None` = no partner. The plane is y = the bind y of joint 6 (the hero's chest, parent of both shoulders), the
	/// bind world position of a joint is the sum of its local bind translations along its parents; a joint pairs with the
	/// NEAREST joint to its reflected position and only when that choice is mutual. Computed from the loaded skeleton, so
	/// no region data in code.
	pub fn partner_table(&self) -> Vec<Option<usize>> {
		let sk = &self.skeleton;
		let n = sk.parents.len();
		if n <= 6 {
			return vec![None; n];
		}
		let mut world: Vec<[f32; 3]> = Vec::with_capacity(n);
		for j in 0..n {
			let t = sk.bind_translation[j];
			world.push(match sk.parents[j].filter(|&p| p < j) {
				Some(p) => [0, 1, 2].map(|i| world[p][i] + t[i]),
				None => t,
			});
		}
		let y0 = world[6][1];
		let dist2 = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>();
		let nearest: Vec<usize> = (0..n)
			.map(|j| {
				let m = [world[j][0], 2.0 * y0 - world[j][1], world[j][2]];
				(0..n).min_by(|&a, &b| dist2(world[a], m).total_cmp(&dist2(world[b], m))).unwrap_or(j)
			})
			.collect();
		(0..n).map(|j| Some(nearest[j]).filter(|&k| nearest[k] == j)).collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn a_zero_scale_parent_skips_the_mirror_op() {
		let id = [0.0, 0.0, 0.0, 1.0];
		let parts: Vec<Part> = vec![
			(id, [0.0, 0.0, 0.0], [0.0; 3]), // 0: zero-scale root, parent of the destination
			(id, [1.0; 3], [1.0, 2.0, 3.0]), // 1: destination
			(id, [1.0; 3], [0.0; 3]),        // 2: source root
			(id, [1.0; 3], [4.0, 5.0, 6.0]), // 3: source
		];
		let parents = [None, Some(0), None, Some(2)];
		let m = mirrored_body(&parts, &parents, 1, 3);
		assert_eq!(m, parts[1]);
		let mut p = parts.clone();
		let fx = [JointFx { joint: 1, rot: id, scale: [1.0; 3], off: [0.0; 3], blend: None, reg: 0, mirror: Some(3) }];
		apply_mirrors(&mut p, &fx, &parents);
		assert_eq!(p[1], parts[1]);
		assert!(p.iter().all(|x| x.0.iter().chain(&x.1).chain(&x.2).all(|v| v.is_finite())));
		assert!(inverse(&[[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]]).is_some());
	}

	// Hash and cut helpers: empty = 0, equal lists equal, any field change differs; a set that appears or vanishes at full weight cuts.
	#[test]
	fn ops_hash_and_cut_rules() {
		assert_eq!(ops_hash(&[]), 0);
		let a = [JointFx::mirror_from(72, 52, 1.0)];
		let b = [JointFx::mirror_from(72, 52, 0.9)];
		assert_ne!(ops_hash(&a), 0);
		assert_eq!(ops_hash(&a), ops_hash(&a.clone()));
		assert_ne!(ops_hash(&a), ops_hash(&b));
		assert_ne!(ops_hash(&a), ops_hash(&[JointFx::mirror_from(73, 53, 1.0)]));
		let faint = [JointFx::mirror_from(72, 52, 0.1)];
		assert!(ops_need_cut(&[], &a) && ops_need_cut(&a, &[]));
		assert!(!ops_need_cut(&[], &faint) && !ops_need_cut(&faint, &[]) && !ops_need_cut(&a, &b) && !ops_need_cut(&[], &[]));
	}
}
