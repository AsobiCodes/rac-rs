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
	seen: usize,
}

/// The pending list of `SceneCtx`: (bind group made with the original, key, authored chain length, wrap).
pub(in crate::viewer) type PendRaw = (wgpu::BindGroup, u32, Option<usize>, u32);

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
			let ms = std::env::var("RAC_TEXPACK_WAIT_MS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(1500);
			r.wait(std::time::Duration::from_millis(ms));
		}
		self.orig_on.set(r.is_some() && crate::retail::splitview::keep_originals()); // [PC] split view: keep the originals (split_orig.rs)
		*self.extra_origs.borrow_mut() = crate::texpack::extra_hd::take_extra_originals();
		*self.repl.borrow_mut() = r;
		self.repl_pending.borrow_mut().clear();
		self.repl_origs.borrow_mut().clear();
	}

	/// End of `build_scene`: the slots still waiting, as indexes into the final `bind` list (slots of a rebuilt prop set drop out).
	pub(in crate::viewer) fn repl_end(&self, bind: &[wgpu::BindGroup], views: Option<Views>) -> Option<Box<ReplScene>> {
		let repl = self.repl.borrow_mut().take()?;
		let at: HashMap<&wgpu::BindGroup, usize> = bind.iter().enumerate().map(|(i, b)| (b, i)).collect();
		let pend: Vec<Pend> = self.repl_pending.borrow_mut().drain(..).filter_map(|(b, key, chain, wrap)| Some(Pend { idx: *at.get(&b)?, key, chain, wrap })).collect();
		(!pend.is_empty()).then(|| Box::new(ReplScene { repl, pend, views, seen: 0 }))
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
		if let State::Ready(t) = &state {
			let r = self.repl.borrow();
			if let Some(e) = r.as_ref().and_then(|r| r.entry(key)) {
				match self.repl_bind(e, t, chain.map(<[_]>::len), sampler) {
					Ok((b, view)) => {
						if let Some(log) = self.tex_log.borrow_mut().as_mut() {
							log.insert(b.clone(), view);
						}
						self.orig_note(&b, orig, chain, sampler); // [PC] split view
						return b;
					}
					Err(m) => eprintln!("texpack: key {key:#x}: {m}"),
				}
			}
		}
		let b = match chain {
			Some(c) => self.upload_chain(c, sampler),
			None => self.upload(orig, sampler),
		};
		self.orig_extra(&b, key, sampler); // [PC] split view
		if matches!(state, State::Pending) {
			self.repl_pending.borrow_mut().push((b.clone(), key, chain.map(<[_]>::len), wrap));
		}
		b
	}

	/// A bind group (and its view) over the replacement of `e`: the authored-chain texture when the call has a chain of the original's
	/// length, else the plain texture with a generated full chain. The GPU textures are cached in the entry.
	pub(in crate::viewer) fn repl_bind(&self, e: &Entry, t: &Arc<ReplTex>, chain: Option<usize>, sampler: &wgpu::Sampler) -> Result<(wgpu::BindGroup, wgpu::TextureView), String> {
		let authored = chain == Some(e.n) && e.n >= 2;
		let mut g = e.gpu.lock().unwrap();
		let slot = &mut g[!authored as usize];
		if slot.is_none() {
			*slot = Some(if authored { self.repl_texture(t)? } else { self.repl_texture(&t.plain()?)? });
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
		Ok((bind, view))
	}

	/// The GPU texture of a chain: BC formats as they are when the device has `TEXTURE_COMPRESSION_BC`, else decoded to RGBA8.
	fn repl_texture(&self, t: &ReplTex) -> Result<wgpu::Texture, String> {
		let bc_ok = self.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
		let decoded;
		let t = if t.fmt.compressed() && !bc_ok {
			decoded = t.to_rgba8()?;
			&decoded
		} else {
			t
		};
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

impl Gpu {
	/// Top of `draw`: swaps replacements that finished loading since the scene was built into their slots (about 16 MB and 64
	/// textures per frame), then rebuilds the bindless texture array when anything changed. Nothing without pending slots.
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
			match ctx.repl_bind(e, &t, p.chain, sampler) {
				Ok((b, view)) => {
					bytes += t.bytes();
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
				scene.tex_array = super::bindless::array_group(&bl.layout, &self.device, &scene.bind_groups, v, &self.sampler, &self.wrap_samplers);
			}
		}
		if rs.pend.is_empty() {
			scene.repl = None;
		}
	}
}
