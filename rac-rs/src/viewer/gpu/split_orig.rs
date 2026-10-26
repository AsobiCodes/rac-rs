//! [PC] Split view with a texture pack (lane splittp): the Classic half shows the ORIGINAL disc textures, the Enhanced half the pack's.
//! Split view: this module only supplies the originals; the Classic pass (draw/split.rs) swaps them in around its recording.
//!
//! A scene slot with a replacement is uploaded as the pack's texture and the disc's original is dropped (repl_upload.rs). While the
//! split view could be used (`splitview::keep_originals`: Debug mode or the split view on) a build also uploads the original of every
//! replaced slot (`SceneCtx::orig_note`) and keeps, next to `Scene::bind_groups` and `Scene::tex_array`, the same lists made of the
//! originals (`OrigSet`, `SceneCtx::orig_end`). `Gpu::swap_originals` exchanges the two sets by index for the Classic pass and back
//! after it, so every draw site of the pass reads the originals without knowing; a slot that is still waiting for its replacement
//! already holds the original in both sets. The fx / particle table does the same with `GpuOrig::fx`. With the split view
//! unusable nothing is kept and the swap does nothing: the normal game path is unchanged.
//!
//! Sky shells (a CPU texture replaced by a load hook) are noted by `extra_hd::note_orig` and uploaded by `orig_extra`; the chrome and glass of
//! the metal packets (`GpuOrig::metal`) and the particle tints (`PartFx::tints_orig`) are swapped like the fx table.
//!
//! Not covered (these show the pack's picture in both halves): the ship and warp bundles' textures and the grass colour.

use super::*;

/// The disc's textures outside the scene (`Gpu::orig_gpu`): the fx / particle table, the particle tints and the metal packets' chrome and glass.
#[derive(Default)]
pub(in crate::viewer) struct GpuOrig {
	fx: HashMap<usize, wgpu::BindGroup>,
	pub(super) metal: Option<[wgpu::BindGroup; 2]>,
}

/// The scene's texture slots as the disc has them: `Scene::bind_groups` and `Scene::tex_array` with the replaced slots original.
pub(in crate::viewer) struct OrigSet {
	bind: Vec<wgpu::BindGroup>,
	array: Option<wgpu::BindGroup>,
}

impl SceneCtx {
	/// `upload_slot`, a slot uploaded with its replacement `replaced`: uploads the original too (same chain and sampler) and notes the pair.
	pub(super) fn orig_note(&self, replaced: &wgpu::BindGroup, orig: Option<&Texture>, chain: Option<&[Texture]>, sampler: &wgpu::Sampler) {
		if !self.orig_on.get() {
			return;
		}
		let o = match chain {
			Some(c) => self.upload_chain(c, sampler),
			None => self.upload(orig, sampler),
		};
		self.repl_origs.borrow_mut().push((replaced.clone(), o));
	}

	/// `upload_slot`, a slot with no pack texture of the world tables whose CPU texture a load hook replaced (`extra_origs`: the sky shells):
	/// uploads the disc's original of it too and notes the pair.
	pub(super) fn orig_extra(&self, uploaded: &wgpu::BindGroup, key: u32, sampler: &wgpu::Sampler) {
		if !self.orig_on.get() {
			return;
		}
		if let Some(t) = self.extra_origs.borrow().get(&key) {
			let o = self.upload(Some(t), sampler);
			self.repl_origs.borrow_mut().push((uploaded.clone(), o));
		}
	}

	/// End of `build_scene`, before `repl_end`: the original set of the final `bind` list (None when the build kept no originals or
	/// no slot has a pack texture). `views` = the views `tex_log` collected (the originals' are in it).
	pub(in crate::viewer) fn orig_end(&self, bind: &[wgpu::BindGroup], views: Option<&HashMap<wgpu::BindGroup, wgpu::TextureView>>) -> Option<Box<OrigSet>> {
		let origs = std::mem::take(&mut *self.repl_origs.borrow_mut());
		if !self.orig_on.replace(false) || (origs.is_empty() && self.repl_pending.borrow().is_empty()) {
			return None;
		}
		let at: HashMap<&wgpu::BindGroup, usize> = bind.iter().enumerate().map(|(i, b)| (b, i)).collect();
		let mut set = bind.to_vec();
		for (replaced, original) in origs {
			if let Some(&i) = at.get(&replaced) {
				set[i] = original;
			}
		}
		let array = self.bindless_layout.as_ref().zip(views).and_then(|(l, v)| super::bindless::array_group(l, &self.device, &set, v, &self.sampler, &self.wrap_samplers));
		Some(Box::new(OrigSet { bind: set, array }))
	}
}

impl Gpu {
	/// The split view has original textures to show (the Classic half would differ from the Enhanced one by them alone).
	pub(in crate::viewer) fn has_originals(&self) -> bool {
		self.scene.as_ref().is_some_and(|s| s.orig.is_some()) || !self.orig_gpu.fx.is_empty() || self.orig_gpu.metal.is_some()
	}

	/// Exchanges the scene's and the fx table's texture bind groups with the originals (call again to swap back). Slots the scene added
	/// after its build (debris, warp actors) have no pack texture and stay as they are.
	pub(in crate::viewer) fn swap_originals(&mut self) {
		if let Some(scene) = self.scene.as_mut() {
			if let Some(o) = scene.orig.as_mut() {
				for (a, b) in scene.bind_groups.iter_mut().zip(o.bind.iter_mut()) {
					std::mem::swap(a, b);
				}
				std::mem::swap(&mut scene.tex_array, &mut o.array);
			}
		}
		if let Some(o) = self.orig_gpu.metal.as_mut() {
			if let Some(cur) = self.metal.tex.as_mut() {
				std::mem::swap(cur, o);
			}
		}
		for (k, c) in self.partfx.tints_orig.iter_mut() {
			if let Some(cur) = self.partfx.tints.get_mut(k) {
				std::mem::swap(cur, c);
			}
		}
		for (k, b) in self.orig_gpu.fx.iter_mut() {
			if let Some(cur) = self.fx_tex.get_mut(k) {
				std::mem::swap(cur, b);
			}
		}
	}

	/// Level load, after `fx_tex` is filled: the originals (`extra_hd::take_fx_originals`) of the fx / particle textures the pack replaced,
	/// uploaded under the same keys and in the same variants as `fx_tex` has them (clamped, no-mip glass).
	pub(in crate::viewer) fn set_fx_originals(&mut self, orig: &[(usize, Texture)]) {
		let nomip = self.device.create_sampler(&wgpu::SamplerDescriptor { address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }); // as app.rs `nomip`
		let mut m = HashMap::new();
		for (k, t) in orig {
			m.insert(*k, self.upload_texture(Some(t)));
			if crate::retail::queue_b::FX_CLAMPED.contains(k) || crate::retail::queue_q2::FX_CLAMPED.contains(k) || *k < crate::particles::FX_TEX as usize {
				m.insert(k | crate::particles::FX_CLAMP, self.upload(Some(t), &self.clamp_sampler));
			}
			if crate::retail::queue_q1::GLASS_TEX.contains(k) {
				m.insert(k | crate::particles::FX_NOMIP, self.upload(Some(t), &nomip));
			}
		}
		self.partfx.tints_orig = orig.iter().map(|(k, t)| (*k as u32, super::super::partfx::tint_of(t))).collect();
		self.orig_gpu.fx = m;
	}
}
