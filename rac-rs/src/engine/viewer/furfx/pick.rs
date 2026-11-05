//! Which marker draws are fur (texture content hash against the pack's table), the weld table of the shared shell normals, and the
//! two steps of the build: `pending` at scene build (copies only), `realise` on the first frame fur is drawn (everything on the GPU).

use super::*;
use std::collections::BTreeMap;
use super::super::presenter::GatedQueue;
use crate::furspec::{texhash, FurRow};
use wgpu::util::DeviceExt;

/// For each vertex the first vertex at the same model-space position: the shells of both copies of a seam vertex then move along the
/// same normal and stay closed.
pub(super) fn weld_table(model: &[Vertex]) -> Vec<u32> {
	let mut seen: HashMap<[i32; 3], u32> = HashMap::new();
	model.iter().enumerate().map(|(i, v)| *seen.entry(v.pos.map(|c| (c * 4096.0).round() as i32)).or_insert(i as u32)).collect()
}

/// Per marker draw, how many of its indices come before the afterimage tail (vertices `>= ghost_from`). The marker batch groups the
/// hero mesh's faces by full texture key, ascending, each group in face order (`scene_ctx.rs` `batch`, unordered), and the ghost
/// copies are appended to the mesh last (`rac1/host/app.rs` `combined_hero`, whole faces with their own index base), so in every
/// draw the tail faces are its last indices and the fur draw stops short of them: a hidden copy (all its vertices collapsed to the
/// hero's feet by `hero_pose::hide`, normals up) would otherwise grow real fur slivers, an opaque one for any colour alpha. The
/// tinted class tail and every other part of the hero keep their shells.
pub(super) fn ghost_cut(faces: &[(u32, [u32; 3])], ghost_from: usize) -> Vec<u32> {
	let mut groups: BTreeMap<u32, (u32, bool)> = BTreeMap::new();
	for (key, f) in faces {
		let g = groups.entry(*key).or_default();
		if f.iter().any(|&i| i as usize >= ghost_from) {
			g.1 = true;
		} else if !g.1 {
			g.0 += 3;
		}
	}
	groups.into_values().map(|g| g.0).collect()
}

/// The indices of draw `di` that get shells: `cut[di]` (never more than the draw has), all of them when `cut` is empty.
pub(super) fn fur_count(cut: &[u32], di: usize, count: u32) -> u32 {
	cut.get(di).map_or(count, |&c| c.min(count))
}

/// The texture of a marker draw: its decoded texels, found through the slot table (bind index back to texture key).
fn texture_of<'a>(d: &Draw, slots: &HashMap<u32, usize>, textures: &'a HashMap<u32, crate::level::Texture>) -> Option<(u32, &'a crate::level::Texture)> {
	let key = slots.iter().filter(|(_, i)| **i == d.texture).map(|(&k, _)| k).min()?;
	textures.get(&(key & mobies::KEY_MASK)).map(|t| (key, t))
}

/// RAC_FUR_DUMP=dir (dev): every marker draw's texture as raw RGBA plus its hash, to decide the pack's fur table.
fn dump(draws: &[Draw], slots: &HashMap<u32, usize>, textures: &HashMap<u32, crate::level::Texture>) {
	let Ok(dir) = std::env::var("RAC_FUR_DUMP") else { return };
	let _ = std::fs::create_dir_all(&dir);
	for (di, d) in draws.iter().enumerate() {
		let Some((key, t)) = texture_of(d, slots, textures) else { continue };
		let h = texhash(t.w, t.h, &t.rgba);
		let raw: Vec<u8> = t.rgba.iter().flatten().copied().collect();
		let _ = std::fs::write(format!("{dir}/d{di}_{}x{}_{h:016x}.rgba", t.w, t.h), raw);
		eprintln!("furdump: draw {di} key {key:#x} start {} count {} tex {}x{} hash {h:016x}", d.start, d.count, t.w, t.h);
	}
}

/// The fur map of a fur texture as an RGBA8 texture (`FurMask::map`: fur, length scale, halo scale, tone), sampled by uv in vs_fur
/// and fs_fur.
fn mask_texture(device: &wgpu::Device, queue: &GatedQueue, row: &FurRow, t: &crate::level::Texture) -> wgpu::Texture {
	let (w, h) = (t.w as u32, t.h as u32);
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("fur mask"),
		size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let texels = row.mask.map(t.w, t.h, &t.rgba);
	queue.write_texture(tex.as_image_copy(), bytemuck::cast_slice(&texels), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
	tex
}

/// Scene build: what the lazy fur build needs, or None (the game has no fur spec: R&C2, or no pack). Only copies texels; no GPU
/// object, no hashing. `faces` = the hero mesh's faces, `ghost_from` = the first vertex of its afterimage copies (the tail).
pub(crate) fn pending(marker: &Batch, faces: &[(u32, [u32; 3])], ghost_from: usize, slots: &HashMap<u32, usize>, textures: &HashMap<u32, crate::level::Texture>) -> Option<FurScene> {
	dump(&marker.draws, slots, textures);
	spec()?;
	let draws = marker.draws.iter().map(|d| texture_of(d, slots, textures).map(|(key, t)| (key & mobies::KEY_MASK, crate::level::Texture { w: t.w, h: t.h, rgba: t.rgba.clone() }))).collect();
	let mut cut = ghost_cut(faces, ghost_from);
	if cut.len() != marker.draws.len() {
		eprintln!("fur: {} face groups for {} marker draws; no fur (cannot tell the afterimage copies apart)", cut.len(), marker.draws.len());
		cut = vec![0; marker.draws.len()];
	}
	Some(FurScene { draws, cut, gpu: OnceLock::new() })
}

/// The fur of a scene's marker, built on the first frame fur is drawn: None when none of the marker's textures is fur.
pub(super) fn realise(device: &wgpu::Device, queue: &GatedQueue, fs: &FurScene, marker: &Batch, model: &[Vertex], layout: &wgpu::BindGroupLayout) -> Option<FurMarker> {
	let spec = spec()?;
	let rows: Vec<Option<(&FurRow, &crate::level::Texture)>> = fs.draws.iter().map(|d| d.as_ref().and_then(|(_, t)| spec.rows.iter().find(|r| r.hash == texhash(t.w, t.h, &t.rgba)).map(|r| (r, t)))).collect();
	if rows.iter().all(Option::is_none) {
		return None;
	}
	let part: Vec<u8> = rows.iter().map(|r| r.map_or(0, |(r, _)| r.part + 1)).collect();
	let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("fur parts"), size: (spec.parts.len() as u64 * params::SLOT as u64).max(params::SLOT as u64), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
	let weld = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("fur weld"), contents: bytemuck::cast_slice(&weld_table(model)), usage: wgpu::BufferUsages::STORAGE });
	// RAC_HERO_GPU=0: the marker's vertex buffer has no STORAGE usage; the shells then use their own vertex normals (no seam welding).
	let weld_on = marker.vbuf.usage().contains(wgpu::BufferUsages::STORAGE);
	let dummy = (!weld_on).then(|| device.create_buffer(&wgpu::BufferDescriptor { label: Some("fur no weld"), size: 64, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false }));
	let vb_entry = dummy.as_ref().map_or_else(|| marker.vbuf.as_entire_binding(), |d| d.as_entire_binding());
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("fur mask"), address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, anisotropy_clamp: 8, ..Default::default() });
	let noise = noise::texture(device, queue).create_view(&Default::default());
	let masks: Vec<Option<wgpu::TextureView>> = rows.iter().map(|r| r.map(|(row, t)| mask_texture(device, queue, row, t).create_view(&Default::default()))).collect();
	let bind = masks
		.iter()
		.map(|m| {
			let m = m.as_ref()?;
			Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("fur"),
				layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &params, offset: 0, size: wgpu::BufferSize::new(params::SIZE) }) },
					wgpu::BindGroupEntry { binding: 1, resource: vb_entry.clone() },
					wgpu::BindGroupEntry { binding: 2, resource: weld.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(m) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&noise) },
				],
			}))
		})
		.collect();
	Some(FurMarker { part, params, bind, weld: weld_on, written: AtomicU16::new(0), _keep: std::iter::once(weld).chain(dummy).collect() })
}
