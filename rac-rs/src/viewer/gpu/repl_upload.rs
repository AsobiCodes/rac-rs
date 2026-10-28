//! [PC] Texture packs (lane tex): the texture slots of a scene take the pack's replacement when it is loaded in time
//! (`SceneCtx::upload_slot`) and are swapped in by `Gpu::swap_repl` when it is not. Without a textures mod `SceneCtx::repl` is None
//! and `upload_slot` is exactly the `upload` / `upload_chain` call of the draw it replaced.

use super::*;
use crate::texpack::chain::ReplTex;
use crate::texpack::dds::Fmt;
use crate::texpack::repl::{self, Cpu, Entry, LevelRepl};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

/// A slot that was uploaded with the original and waits for its replacement.
pub(in crate::viewer) struct Pend {
	idx: usize,
	key: u32,
	/// Length of the authored chain the slot was made with.
	chain: Option<usize>,
	wrap: u32,
}

type Views = HashMap<wgpu::BindGroup, wgpu::TextureView>;

/// What `Scene` keeps while replacements are still loading.
pub(in crate::viewer) struct ReplScene {
	repl: Arc<LevelRepl>,
	pend: Vec<Pend>,
	views: Option<Views>,
	/// Number of texture slots the scene's `tex_array` was made of (the slots of the build). Debris and warp actor classes append
	/// slots after the build (`load_debris`, `load_warp_actors`): they have no view in `views` and no tie or shrub draw uses them.
	array_len: usize,
	seen: usize,
}

/// A scene that goes away (travel, Load Game) while slots still wait ends the level's plain chains and rebuilds at once: the pinned
/// chains go back to the budget, the rebuild threads give up and the level drops (and, when it is still loading, its loader threads stop).
impl Drop for ReplScene {
	fn drop(&mut self) {
		self.repl.scene_dropped();
	}
}

/// The pending list of `SceneCtx`: (bind group made with the original, key, authored chain length, wrap).
pub(in crate::viewer) type PendRaw = (wgpu::BindGroup, u32, Option<usize>, u32);

/// What one scene build may bind of the replacements that are already Ready: bytes uploaded and slots bound (cached GPU textures count as
/// slots, uploads as bytes). Past it a Ready entry is treated like a Pending one (the original is shown, `swap_repl` binds it within its
/// per-frame cap), so a pack that finished loading inside the wait does not make the level-start frame one giant upload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::viewer) struct BuildCap {
	bytes_left: usize,
	slots_left: usize,
}

impl BuildCap {
	/// Bytes a scene build may upload before the rest of the Ready replacements are deferred (`RAC_TEXPACK_BUILD_MB`, default 128).
	pub(in crate::viewer) const BYTES: usize = 128 << 20;
	/// Slots a scene build may bind with a replacement.
	pub(in crate::viewer) const SLOTS: usize = 1024;

	pub(in crate::viewer) fn new(bytes: usize, slots: usize) -> BuildCap {
		BuildCap { bytes_left: bytes, slots_left: slots }
	}

	/// The default cap, `RAC_TEXPACK_BUILD_MB` in MB (read once; 0 = defer every Ready replacement to `swap_repl`).
	fn from_env() -> BuildCap {
		static MB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
		let bytes = *MB.get_or_init(|| std::env::var("RAC_TEXPACK_BUILD_MB").ok().and_then(|v| v.parse::<usize>().ok()).map_or(Self::BYTES, |mb| mb.saturating_mul(1 << 20)));
		BuildCap::new(bytes, Self::SLOTS)
	}

	/// Whether the cap is used up (checked before a bind, charged after it: one texture may overshoot, like `swap_repl`'s 16 MB).
	pub(in crate::viewer) fn spent(self) -> bool {
		self.bytes_left == 0 || self.slots_left == 0
	}

	/// This cap with the slots of `before`: a prop pass the build throws away (`bind.truncate`) gives back the slots it took, not the bytes
	/// (its uploads stay in `Entry.gpu`, so the redo binds them again for 0 bytes and the frame's upload stays within the cap).
	pub(in crate::viewer) fn with_slots_of(self, before: BuildCap) -> BuildCap {
		BuildCap { bytes_left: self.bytes_left, slots_left: before.slots_left }
	}

	/// The cap after one bind that uploaded `bytes`.
	pub(in crate::viewer) fn charge(self, bytes: usize) -> BuildCap {
		BuildCap { bytes_left: self.bytes_left.saturating_sub(bytes), slots_left: self.slots_left.saturating_sub(1) }
	}
}

impl Default for BuildCap {
	fn default() -> BuildCap {
		BuildCap::new(Self::BYTES, Self::SLOTS)
	}
}

enum State {
	No,
	Pending,
	Ready(Arc<ReplTex>),
}

fn gpu_format(f: Fmt) -> wgpu::TextureFormat {
	match f {
		Fmt::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
		Fmt::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
		Fmt::Bc2 => wgpu::TextureFormat::Bc2RgbaUnorm,
		Fmt::Bc3 => wgpu::TextureFormat::Bc3RgbaUnorm,
		Fmt::Bc7 => wgpu::TextureFormat::Bc7RgbaUnorm,
	}
}

impl SceneCtx {
	/// Start of `build_scene`: takes the level's replacements (left by `texpack::level_loaded` on this thread) and gives the loader
	/// threads a head start (`RAC_TEXPACK_WAIT_MS`, default 1500; 0 = never wait, swap in later).
	pub(in crate::viewer) fn repl_begin(&self) {
		let r = repl::take_thread_repl();
		if let Some(r) = &r {
			static WAIT_MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
			let ms = *WAIT_MS.get_or_init(|| std::env::var("RAC_TEXPACK_WAIT_MS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(1500));
			r.wait(std::time::Duration::from_millis(ms));
		}
		self.orig_on.set(r.is_some() && crate::retail::splitview::keep_originals()); // [PC] split view: keep the originals (split_orig.rs)
		*self.extra_origs.borrow_mut() = crate::texpack::extra_hd::take_extra_originals();
		*self.repl.borrow_mut() = r;
		self.repl_pending.borrow_mut().clear();
		self.repl_origs.borrow_mut().clear();
		self.repl_cap.set(BuildCap::from_env());
	}

	/// End of `build_scene`: the slots still waiting, as indexes into the final `bind` list (slots of a rebuilt prop set drop out).
	pub(in crate::viewer) fn repl_end(&self, bind: &[wgpu::BindGroup], views: Option<Views>) -> Option<Box<ReplScene>> {
		let repl = self.repl.borrow_mut().take()?;
		let at: HashMap<&wgpu::BindGroup, usize> = bind.iter().enumerate().map(|(i, b)| (b, i)).collect();
		let asked: Vec<PendRaw> = self.repl_pending.borrow_mut().drain(..).collect();
		let pend: Vec<Pend> = asked.iter().filter_map(|(b, key, chain, wrap)| Some(Pend { idx: *at.get(b)?, key: *key, chain: *chain, wrap: *wrap })).collect();
		// Keys only slots of a thrown-away prop pass asked for: their rebuilt chains would never be uploaded.
		let kept: std::collections::HashSet<u32> = pend.iter().map(|p| p.key).collect();
		asked.iter().filter(|a| !kept.contains(&a.1)).for_each(|a| repl.unwant(a.1));
		if pend.is_empty() {
			repl.scene_dropped(); // every slot is bound: no slot waits for the keys still loading, the plain chains held are of no use (as when `swap_repl` ends the ReplScene)
			return None;
		}
		Some(Box::new(ReplScene { repl, pend, views, array_len: bind.len(), seen: 0 }))
	}

	fn repl_state(&self, key: u32, orig: Option<&Texture>) -> State {
		let r = self.repl.borrow();
		let (Some(e), Some(o)) = (r.as_ref().and_then(|r| r.entry(key)), orig) else { return State::No };
		if !e.fp.is(o) {
			return State::No;
		}
		match &*e.cpu.lock().unwrap() {
			Cpu::Ready(t) => State::Ready(t.clone()),
			Cpu::Pending => State::Pending,
			Cpu::Failed => State::No,
		}
	}

	/// The bind group of one texture slot: the original (`chain` = its authored mip chain if it has one), or the pack's replacement
	/// when `allow` and it is loaded. A replacement still loading is recorded and swapped in later.
	pub(in crate::viewer) fn upload_slot(&self, key: u32, orig: Option<&Texture>, chain: Option<&[Texture]>, wrap: u32, allow: bool) -> wgpu::BindGroup {
		let sampler = if wrap != 0 { &self.wrap_samplers[wrap as usize - 1] } else { &self.sampler };
		let state = if allow { self.repl_state(key, orig) } else { State::No };
		// A Ready replacement past the build's cap is deferred like a Pending one (see `BuildCap`).
		let mut waits = matches!(state, State::Ready(_)) && self.repl_cap.get().spent();
		if let (State::Ready(t), false) = (&state, waits) {
			let r = self.repl.borrow();
			if let Some((r, e)) = r.as_ref().and_then(|r| r.entry(key).map(|e| (r, e))) {
				match self.repl_bind(r, e, t, chain.map(<[_]>::len), sampler) {
					Ok(Some((b, view, uploaded))) => {
						self.repl_cap.set(self.repl_cap.get().charge(uploaded));
						if let Some(log) = self.tex_log.borrow_mut().as_mut() {
							log.insert(b.clone(), view);
						}
						self.orig_note(&b, orig, chain, sampler); // [PC] split view
						return b;
					}
					// The upload chain is being rebuilt by a loader thread: the slot shows the original until it is swapped in.
					Ok(None) => waits = true,
					Err(m) => eprintln!("texpack: key {key:#x}: {m}"),
				}
			}
		}
		let b = match chain {
			Some(c) => self.upload_chain(c, sampler),
			None => self.upload(orig, sampler),
		};
		self.orig_extra(&b, key, sampler); // [PC] split view
		if waits || matches!(state, State::Pending) {
			self.repl_pending.borrow_mut().push((b.clone(), key, chain.map(<[_]>::len), wrap));
		}
		b
	}

	/// A bind group (and its view) over the replacement of `e`, and the bytes uploaded for it (0 when the GPU texture was cached): the
	/// authored-chain texture when the call has a chain of the original's length, else the plain texture with a full chain down to 1x1.
	/// The plain chain was made by the loader from the level-0 file (`Entry::plain_for_bind`, dropped once uploaded). If it is not held
	/// (the budget had no room for it) nothing is built here: `Ok(None)`, a loader thread rebuilds it and the caller keeps the slot's
	/// picture and asks again (`Gpu::swap_repl`). The GPU textures are cached in the entry.
	pub(in crate::viewer) fn repl_bind(&self, repl: &Arc<LevelRepl>, e: &Entry, t: &Arc<ReplTex>, chain: Option<usize>, sampler: &wgpu::Sampler) -> Result<Option<(wgpu::BindGroup, wgpu::TextureView, usize)>, String> {
		let authored = chain == Some(e.n) && e.n >= 2;
		let mut g = e.gpu.lock().unwrap();
		let slot = &mut g[!authored as usize];
		let mut uploaded = 0;
		if slot.is_none() {
			let t = if authored {
				t.clone()
			} else {
				match e.plain_for_bind(repl)? {
					Some(p) => p,
					None => return Ok(None),
				}
			};
			*slot = Some(self.repl_texture(&t)?);
			uploaded = t.bytes();
			if !authored {
				e.release_plain();
			}
		}
		let view = slot.as_ref().unwrap().create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: None,
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
			],
		});
		Ok(Some((bind, view, uploaded)))
	}

	/// The GPU texture of a chain, uploaded as it is. A BC chain needs `TEXTURE_COMPRESSION_BC`; without it the loader has decoded the
	/// chain already (`repl::set_bc_native`), so a BC chain here means the two disagree and the slot keeps the original.
	fn repl_texture(&self, t: &ReplTex) -> Result<wgpu::Texture, String> {
		if t.fmt.compressed() && !self.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
			return Err("the device has no BC texture support".into());
		}
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: None,
			size: wgpu::Extent3d { width: t.mips[0].w, height: t.mips[0].h, depth_or_array_layers: 1 },
			mip_level_count: t.mips.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: gpu_format(t.fmt),
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		for (k, m) in t.mips.iter().enumerate() {
			// Compressed levels are written in whole 4x4 blocks (a level under 4 texels still occupies one block).
			let (rows, ext_w, ext_h, row) = if t.fmt.compressed() {
				let (bw, bh) = (m.w.div_ceil(4), m.h.div_ceil(4));
				(bh, bw * 4, bh * 4, bw * t.fmt.block_bytes() as u32)
			} else {
				(m.h, m.w, m.h, m.w * 4)
			};
			self.queue.write_texture(
				wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: k as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
				&m.data,
				wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
				wgpu::Extent3d { width: ext_w, height: ext_h, depth_or_array_layers: 1 },
			);
		}
		Ok(tex)
	}
}

/// The slots `Scene::tex_array` is made of: the first `array_len` of the scene's bind groups (the rest were appended after the build).
fn array_slots<T>(all: &[T], array_len: usize) -> &[T] {
	&all[..array_len.min(all.len())]
}

impl Gpu {
	/// Top of `draw`: swaps replacements that finished loading since the scene was built into their slots (about 16 MB actually
	/// uploaded and 64 textures per frame), then rebuilds the bindless texture array when anything changed. Nothing without pending slots.
	pub(in crate::viewer) fn swap_repl(&mut self) {
		let ctx = self.ctx();
		let Some(scene) = self.scene.as_mut() else { return };
		let Some(rs) = scene.repl.as_mut() else { return };
		let done = rs.repl.done.load(Relaxed);
		if done == rs.seen {
			return;
		}
		let (mut bytes, mut count, mut changed, mut left) = (0usize, 0usize, false, false);
		let mut i = 0;
		while i < rs.pend.len() {
			let p = &rs.pend[i];
			let ready = match rs.repl.entry(p.key).map(|e| (e, e.cpu.lock().unwrap())) {
				Some((e, c)) => match &*c {
					Cpu::Ready(t) => Some((e, t.clone())),
					Cpu::Pending => {
						left = true;
						i += 1;
						continue;
					}
					Cpu::Failed => None,
				},
				None => None,
			};
			let Some((e, t)) = ready else {
				rs.pend.swap_remove(i);
				continue;
			};
			if bytes >= 16 << 20 || count >= 64 {
				left = true;
				i += 1;
				continue;
			}
			let sampler = if p.wrap != 0 { &self.wrap_samplers[p.wrap as usize - 1] } else { &self.sampler };
			match ctx.repl_bind(&rs.repl, e, &t, p.chain, sampler) {
				Ok(None) => {
					// The upload chain is being rebuilt: the slot keeps its picture and comes back when it is ready.
					left = true;
					i += 1;
					continue;
				}
				Ok(Some((b, view, uploaded))) => {
					bytes += uploaded;
					count += 1;
					changed = true;
					if let Some(v) = rs.views.as_mut() {
						v.remove(&scene.bind_groups[p.idx]);
						v.insert(b.clone(), view);
					}
					scene.bind_groups[p.idx] = b;
				}
				Err(m) => eprintln!("texpack: key {:#x}: {m}", p.key),
			}
			rs.pend.swap_remove(i);
		}
		if !left {
			rs.seen = done;
		}
		if changed {
			eprintln!("texpack: swapped in {count} replacements ({:.1} MB), {} slots still wait", bytes as f64 / 1048576.0, rs.pend.len());
		}
		if changed && scene.tex_array.is_some() {
			if let (Some(bl), Some(v)) = (self.bindless.as_ref(), rs.views.as_ref()) {
				// The array of the build's slots, as the build made it. Over all `bind_groups` it came out None as soon as debris / warp
				// slots had been appended (no view for them): every bindless draw fell back to per-draw binds and the sun shadows lost
				// the shrubs and the texel-alpha cut of terrain and ties (`sunshadow::encode`) for the rest of the visit.
				let slots = array_slots(&scene.bind_groups, rs.array_len);
				scene.tex_array = super::bindless::array_group(&bl.layout, &self.device, slots, v, &self.sampler, &self.wrap_samplers);
			}
		}
		if rs.pend.is_empty() {
			rs.repl.release_plains();
			scene.repl = None;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::texpack::repl::Job;

	/// R2: dropping the scene releases the level's plain chains and ends its rebuild threads (nobody else calls `release_plains` when a
	/// scene is replaced by travel or Load Game).
	#[test]
	fn dropping_the_scene_releases_the_levels_plain_chains() {
		let job = Job { key: 1, fp: crate::texpack::repl::Fp { w: 1, h: 1, hash: 0 }, n: 1, files: [None, None, None, None] };
		let r = LevelRepl::with_bc(5, vec![job], true);
		let scene = ReplScene { repl: r.clone(), pend: Vec::new(), views: None, array_len: 0, seen: 0 };
		assert!(!r.is_released() && !r.is_gone());
		drop(scene);
		assert!(r.is_released() && r.is_gone());
	}

	#[test]
	fn a_late_swap_rebuilds_the_array_from_the_build_slots_not_the_appended_ones() {
		// 3 slots at the build, then 2 debris slots appended (no view logged for them, `load_debris`).
		let all = [1u32, 2, 3, 100, 101];
		let views: std::collections::HashMap<u32, &str> = [(1, "a"), (2, "b"), (3, "c")].into();
		assert!(crate::viewer::bindless::array_views(&all, &views).is_none(), "over every bind group the rebuild came out None: no array for the rest of the visit");
		let built = crate::viewer::bindless::array_views(array_slots(&all, 3), &views).expect("the build's slots all have views");
		assert_eq!(built, [&"a", &"b", &"c"]);
		assert_eq!(array_slots(&all, 99), &all[..], "a longer array_len is clamped");
	}

	#[test]
	fn the_build_cap_is_spent_by_bytes_or_by_slots_and_never_goes_below_zero() {
		let c = BuildCap::new(100, 5);
		assert!(!c.spent());
		// One bind may overshoot the bytes (checked before, charged after), then the cap is spent.
		let c = c.charge(60);
		assert!(!c.spent());
		let c = c.charge(60);
		assert!(c.spent());
		assert_eq!(c.charge(10), BuildCap::new(0, 2), "saturates at 0");
		// Cached GPU textures upload 0 bytes but still count as slots.
		let mut c = BuildCap::new(1 << 30, 3);
		for _ in 0..3 {
			assert!(!c.spent());
			c = c.charge(0);
		}
		assert!(c.spent());
		assert!(BuildCap::new(0, 5).spent(), "RAC_TEXPACK_BUILD_MB=0 defers everything");
		let d = BuildCap::default();
		assert_eq!((d.bytes_left, d.slots_left), (128 << 20, 1024));
		assert!(!d.spent());
		// A thrown-away prop pass gives its slots back, not its bytes (the redo binds its uploads again for 0 bytes).
		assert_eq!(BuildCap::new(10, 1).with_slots_of(BuildCap::new(100, 5)), BuildCap::new(10, 5));
	}
}
