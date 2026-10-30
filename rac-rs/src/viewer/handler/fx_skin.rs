//! [PC] perf: the skinning of the props with joint records (`frame_props.rs` `fx_jobs`: vendor and orb spins, props fading from a
//! snapshot, joint-override records) kept from frame to frame. Skinning is a pure function of the class and the joint matrices
//! (`anim::skin_all`), so a prop whose class and matrices are, bit for bit, last frame's keeps last frame's pose. With frame
//! smoothing a game tick spans several display frames, and the matrices only change in a tick (or when the animation moves them).

use super::*;
use std::sync::Arc;

/// One prop's last skinning: what it was made from, and the pose.
struct Kept {
	/// The class's address. A class lives as long as its level's `prop_anims`, which is replaced whole (`app.rs`, the level load),
	/// and `FxSkins::forget` runs there, so an address is never another class's here; the matrices below must match too.
	class: usize,
	mats: Vec<[f32; 12]>,
	/// Shared with `fx_posed` and the pose history (`smooth.rs`): a kept pose is not copied, only counted.
	posed: Arc<Posed>,
	/// [PC] perf: the pose's token (`pose_born.rs`).
	born: u64,
}

/// The last frame's skinnings by prop slot.
#[derive(Default)]
pub(in crate::viewer) struct FxSkins {
	kept: HashMap<usize, Kept>,
	/// [PC] perf: the key poses' sources and this frame's pose tokens (`pose_born.rs`).
	pub(in crate::viewer) keys: super::pose_born::KeySkins,
	pub(in crate::viewer) born: super::pose_born::PoseBorn,
}

/// The matrices as bytes: equal bit for bit (a `-0.0` or a NaN payload is not the same pose input).
pub(super) fn same_mats(a: &[[f32; 12]], b: &[[f32; 12]]) -> bool {
	a.len() == b.len() && bytemuck::cast_slice::<_, u8>(a) == bytemuck::cast_slice::<_, u8>(b)
}

impl FxSkins {
	/// Forgets every kept skinning: the level's classes are being replaced (`app.rs`, next to the `prop_anims` swap), and a new class
	/// may be allocated at a dropped one's address.
	pub(in crate::viewer) fn forget(&mut self) {
		self.kept.clear();
		self.keys.forget();
	}

	/// Skins `jobs` (prop slot, class, joint matrices) into `out`: a job equal to last frame's takes its pose again, the others are
	/// skinned on the worker pool (serially below three, as before). Slots not in `jobs` are forgotten. Their tokens go to
	/// `born.fx` (a kept pose keeps its token).
	pub(in crate::viewer) fn skin(&mut self, jobs: Vec<(usize, &mobies::MobyClass, Vec<[f32; 12]>)>, out: &mut HashMap<usize, Arc<Posed>>) {
		use rayon::prelude::*;
		let mut old = std::mem::take(&mut self.kept);
		let mut kept = HashMap::with_capacity(jobs.len());
		let mut fresh = Vec::new();
		self.born.fx.clear();
		for (pi, class, mats) in jobs {
			let id = class as *const mobies::MobyClass as usize;
			match old.remove(&pi) {
				Some(k) if k.class == id && same_mats(&k.mats, &mats) => {
					out.insert(pi, k.posed.clone());
					self.born.fx.insert(pi, k.born);
					kept.insert(pi, k);
				}
				_ => fresh.push((pi, id, class, mats)),
			}
		}
		let skin = |(pi, class, c, mats): (usize, usize, &mobies::MobyClass, Vec<[f32; 12]>)| {
			let posed = Arc::new(anim::skin_all(c, &mats));
			(pi, Kept { class, mats, posed, born: super::pose_born::new_token() })
		};
		let done: Vec<(usize, Kept)> = if fresh.len() < 3 { fresh.into_iter().map(skin).collect() } else { fresh.into_par_iter().map(skin).collect() };
		for (pi, k) in done {
			out.insert(pi, k.posed.clone());
			self.born.fx.insert(pi, k.born);
			kept.insert(pi, k);
		}
		self.kept = kept;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mats_compare_bit_for_bit() {
		let a = [[0.5f32; 12]];
		assert!(same_mats(&a, &[[0.5; 12]]));
		let mut z = [[0.0f32; 12]];
		let mut n = [[0.0f32; 12]];
		n[0][3] = -0.0;
		assert!(!same_mats(&z, &n), "-0.0 is another bit pattern");
		z[0][3] = -0.0;
		assert!(same_mats(&z, &n));
		assert!(!same_mats(&a, &[[0.5; 12], [0.5; 12]]));
		let nan = [[f32::NAN; 12]];
		assert!(same_mats(&nan, &nan), "the same NaN bits are the same input");
	}

	#[test]
	fn forget_drops_the_kept_poses() {
		let mut s = FxSkins::default();
		s.kept.insert(3, Kept { class: 0x1000, mats: vec![[0.0; 12]], posed: Arc::new((Vec::new(), Vec::new())), born: 1 });
		s.forget();
		assert!(s.kept.is_empty());
	}
}
