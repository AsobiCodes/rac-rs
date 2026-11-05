//! RedrawRequested, upload phase: while the modern vendor page is up, the shop moby's six screens (frames, black plates, hubs) are not drawn
//! (`modern_menu/vendor_cut.rs` says why and how: the dark plates behind the page are 3D geometry of the class-11 moby). The
//! vendor's pose (`fx_posed` by prop slot, else the key pose) is copied with the screen vertices collapsed and handed to `set_props`
//! in place of the real map; `f.fx_posed` and `smooth.posed()` keep the real poses (the Classic half of a split view re-runs
//! `set_props` with them, `frame_split.rs`). The collapsed pose has no token (`pose_born.rs`: 0 = always written), so the GPU's
//! cached copy of the real pose is never reused for it, and the real one is written again when the page goes away.

use super::fx_skin::FxSkins;
use super::*;
use crate::modern_menu::vendor_cut;
use std::sync::Arc;

/// `fx_posed` with every vendor prop's pose replaced by its screen-less copy; None = nothing to change (the modern vendor page is not
/// up, no vendor prop posed, or the class has no screen vertex): then `fx_posed` itself goes to `set_props`.
pub(super) fn cut(
	props: &[Prop],
	prop_anims: &HashMap<u32, (mobies::MobyClass, anim::Animated)>,
	posed: &HashMap<PoseKey, Posed>,
	fx_posed: &HashMap<usize, Arc<Posed>>,
	skins: &mut FxSkins,
) -> Option<HashMap<usize, Arc<Posed>>> {
	if !crate::modern_menu::vendor_host::up() {
		return None;
	}
	let (class, _) = prop_anims.get(&vendor_cut::VENDOR_CLASS)?;
	let mask = skins.vendor_mask.of(class);
	let mut out: Option<HashMap<usize, Arc<Posed>>> = None;
	for (pi, p) in props.iter().enumerate().filter(|(_, p)| p.class_id == vendor_cut::VENDOR_CLASS) {
		let pose = fx_posed.get(&pi).map(|a| &**a).or_else(|| posed.get(&pose_key(p)));
		let Some(cut) = pose.and_then(|pose| vendor_cut::collapse(pose, mask)) else { continue };
		out.get_or_insert_with(|| fx_posed.clone()).insert(pi, Arc::new(cut));
		skins.born.fx.remove(&pi);
	}
	out
}

/// [PC] perf: the vendor class's screen-vertex mask (`vendor_cut::frame_vertices`, a pure function of the class), kept by the class's
/// address instead of rebuilt every frame the page is up. Held in `FxSkins` and dropped by `FxSkins::forget` with the level's classes
/// (`app.rs`, the `prop_anims` swap), so an address is never another class's here.
#[derive(Default)]
pub(super) struct ScreenMask(Option<(usize, Vec<bool>)>);

impl ScreenMask {
	/// The mask of `class`, built on the first call for it.
	fn of(&mut self, class: &mobies::MobyClass) -> &[bool] {
		let id = class as *const mobies::MobyClass as usize;
		if self.0.as_ref().map(|(k, _)| *k) != Some(id) {
			self.0 = Some((id, vendor_cut::frame_vertices(class)));
		}
		self.0.as_ref().map_or(&[], |(_, m)| m)
	}

	pub(super) fn forget(&mut self) {
		self.0 = None;
	}
}
