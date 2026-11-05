//! [PC] Metal On: contact reflections for the planar floor mirror (metalplanar.rs; a child of metalenv.rs). Painted and rough metal
//! reflects only what is near it: a tall window post in Blarg's hangar was mirrored as one long dark band down the whole floor, and
//! the far ceiling came and went in the floor (user 2026-11-03); the hangar's far window bars and rail posts, 5-15 m away but standing
//! on the floor, were mirrored as radial bands / long streaks that shimmered in motion (user v13). After the mirror pass one
//! full-screen pass reads its depth, rebuilds each pixel's reflected point O and the floor point F that reflects it (the pixel's ray on
//! the mirror plane: a point on the plane is its own mirror image) and writes the REFLECTION DISTANCE |F - O| (= the eye's distance to
//! O's mirror image minus its distance to F along the view ray) and writes its contact weight 1 - smoothstep(`metal_contact_near`,
//! `metal_contact_far`, d) into the ALPHA of the planar colour (level 0) and multiplies the colour by it (blend state: premultiplied),
//! so the mip passes average only what is reflected: a thin near object (Ratchet) blurred with the far background keeps its own
//! colour and only its coverage softens (agent 20: storing the plain colour let the faded background dilute his reflection away, and
//! a stored distance averaged with it read as far); nothing drawn = 0. metalfx.wgsl `metal_planar` divides the weight back out,
//! blends the mirror by it over the sky / surroundings and blurs the faded part by up to `metal_contact_blur` mips per unit of
//! (far - near). The water mirror and bed never run this (mirror.rs keeps their depth discarded as before).

use super::*;

/// The full-screen contact pass: the mirror pass's MSAA depth (nearest sample) through the inverse of the mirrored view-projection.
pub(super) const CONTACT: &str = r#"
struct C { inv: mat4x4<f32>, k: vec4<f32> };
@group(0) @binding(0) var dep: texture_depth_multisampled_2d;
@group(0) @binding(1) var<uniform> c: C;
struct VO { @builtin(position) pos: vec4<f32> };
@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	var o: VO;
	o.pos = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
	return o;
}
// k = (mirror plane height, near, far, 0). The blend state keeps the mirror's colour times this alpha (premultiplied) and stores the alpha.
@fragment fn fs_contact(v: VO) -> @location(0) vec4<f32> {
	let px = vec2<i32>(v.pos.xy);
	var z = 1.0;
	for (var s = 0; s < i32(textureNumSamples(dep)); s++) { z = min(z, textureLoad(dep, px, s)); }
	if (z >= 1.0) { return vec4<f32>(0.0); }
	let dim = vec2<f32>(textureDimensions(dep));
	let ndc = vec2<f32>(v.pos.x / dim.x * 2.0 - 1.0, 1.0 - v.pos.y / dim.y * 2.0);
	// O: the reflected point (the inverse mirrored view-projection gives the real point); q: a second point on the same pixel ray
	// (on the clip plane); F: where that ray meets the mirror plane.
	let wo = c.inv * vec4<f32>(ndc, z, 1.0);
	let wq = c.inv * vec4<f32>(ndc, 0.0, 1.0);
	let o = wo.xyz / wo.w;
	let q = wq.xyz / wq.w;
	let dz = q.z - o.z;
	var f = o;
	if (abs(dz) > 1e-6) { f = o + (q - o) * ((c.k.x - o.z) / dz); }
	return vec4<f32>(0.0, 0.0, 0.0, 1.0 - smoothstep(c.k.y, max(c.k.z, c.k.y + 1e-3), distance(f, o)));
}
"#;

/// RAC_TUNE `metal_contact_near` / `metal_contact_far` (reflection distance in world units, the floor point to the reflected point:
/// full reflection up to near, none from far) and `metal_contact_blur` (extra blur of the faded part, mips per unit of far - near): u.metal[13]
/// (metalenv.rs `knobs`); near / far also fade the screen walk by its hit distance (metalfx.wgsl `metal_ssr`).
pub(super) fn knobs() -> [f32; 4] {
	let near = crate::devtune::f32("metal_contact_near", 1.0);
	[near, crate::devtune::f32("metal_contact_far", 4.0).max(near + 0.01), crate::devtune::f32("metal_contact_blur", 0.6), 0.0]
}

/// The contact weight at reflection distance `d` (what the pass stores; `k` = `knobs()`).
#[cfg(test)]
fn weight(d: f32, k: [f32; 4]) -> f32 {
	let t = ((d - k[0]) / (k[1].max(k[0] + 1e-3) - k[0])).clamp(0.0, 1.0);
	1.0 - t * t * (3.0 - 2.0 * t)
}

/// The contact pass's blend: colour = the mirror's colour times the weight (premultiplied), alpha = the weight.
const PREMULTIPLY: wgpu::BlendState = wgpu::BlendState {
	color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
	alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
};

/// The contact pass's pipeline, uniforms and bind group (over the planar MSAA depth).
pub(super) struct Contact {
	pipe: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	bind: wgpu::BindGroup,
}

impl Contact {
	/// `depth`: the planar MSAA depth texture (made with TEXTURE_BINDING); `format`: the planar colour's.
	pub(super) fn new(device: &wgpu::Device, depth: &wgpu::Texture, format: wgpu::TextureFormat) -> Contact {
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("metal contact"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("metal contact", CONTACT.into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("metal contact"),
			entries: &[
				entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }),
				entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
			],
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("metal contact"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let dview = depth.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("metal contact"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&dview) }, wgpu::BindGroupEntry { binding: 1, resource: ubuf.as_entire_binding() }] });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("metal contact"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("metal contact"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_contact"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: Some(PREMULTIPLY), write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		Contact { pipe, ubuf, bind }
	}

	/// The contact weights (by reflection distance) into the alpha of `target` (planar level 0, already resolved); `mvp`: the mirror pass's
	/// view-projection (mirror.rs `mirrored_clip`), `h`: the mirror plane, `k`: `knobs()`.
	pub(super) fn draw(&self, queue: &super::super::presenter::GatedQueue, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, mvp: &[[f32; 4]; 4], h: f32, k: [f32; 4]) {
		let Some(inv) = super::super::mirror::inverse(mvp) else { return };
		let mut data = [[0.0f32; 4]; 5];
		data[..4].copy_from_slice(&inv);
		data[4] = [h, k[0], k[1], 0.0];
		queue.write_buffer(&self.ubuf, 0, bytemuck::cast_slice(&data));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("metal contact"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.pipe);
		pass.set_bind_group(0, &self.bind, &[]);
		pass.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn contact_shader_validates() {
		let m = naga::front::wgsl::parse_str(CONTACT).unwrap_or_else(|e| panic!("{}", e.emit_to_string(CONTACT)));
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validates");
	}

	/// The shader's maths on the CPU: the reflection distance |F - O| at the pixel where `p` shows in the mirror pass `mvp`.
	fn reflect_dist(mvp: &[[f32; 4]; 4], p: [f32; 3], h: f32) -> f32 {
		let inv = super::super::mirror::inverse(mvp).unwrap();
		let ap = |m: &[[f32; 4]; 4], p: [f32; 4]| -> [f32; 4] { std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum()) };
		let c = ap(mvp, [p[0], p[1], p[2], 1.0]);
		let (x, y, z) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
		let wo = ap(&inv, [x, y, z, 1.0]);
		let wq = ap(&inv, [x, y, 0.0, 1.0]);
		let o: [f32; 3] = std::array::from_fn(|i| wo[i] / wo[3]);
		let q: [f32; 3] = std::array::from_fn(|i| wq[i] / wq[3]);
		let s = (h - o[2]) / (q[2] - o[2]);
		let f: [f32; 3] = std::array::from_fn(|i| o[i] + (q[i] - o[i]) * s);
		(0..3).map(|i| (f[i] - o[i]).powi(2)).sum::<f32>().sqrt()
	}

	/// A camera 1.5 m above a floor at z = 1 looking down it, and its metal planar mirror pass.
	fn hangar() -> ([[f32; 4]; 4], f32) {
		let cam = Camera { pos: [0.0, -6.0, 2.5], yaw: std::f32::consts::FRAC_PI_2, pitch: -0.4, tan_half: 0.6, up: [0.0, 0.0, 1.0] };
		let vp = super::super::view_proj(&cam, 1.6, 0.5, 2000.0);
		(super::super::mirror::mirrored_clip(&vp, 1.0, 1.15), 1.0)
	}

	/// The pass measures the true reflection distance: F lies on the eye's ray to O's mirror image O', so
	/// |F - O| = |eye - O'| - |eye - F|.
	#[test]
	fn contact_measures_the_reflection_distance() {
		let (mvp, h) = hangar();
		let eye = [0.0f32, -6.0, 2.5];
		let o = [0.5f32, 4.0, 2.2];
		let om = [o[0], o[1], 2.0 * h - o[2]];
		let len = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
		let s = (h - eye[2]) / (om[2] - eye[2]);
		let f: [f32; 3] = std::array::from_fn(|i| eye[i] + (om[i] - eye[i]) * s);
		let want = len(eye, om) - len(eye, f);
		let got = reflect_dist(&mvp, o, h);
		assert!((got - want).abs() < 2e-2 && (got - len(f, o)).abs() < 2e-2, "{got} vs {want}");
	}

	/// Blarg's hangar (user v13): a window bar 16 m from the eye, its point 0.6 m up (full weight under the old height rule) is
	/// mirrored at a floor point metres nearer the eye and is not reflected (nor from 2 m up, what streaked the floor near the eye);
	/// Ratchet's ankle 0.1 m up, 8 m out, reflects in full, his knee 0.4 m up mostly (a grazing view stretches the
	/// reflection distance: near 2 / far 6 let the posts streak again at a low eye, agent 20).
	#[test]
	fn far_objects_at_floor_height_fade_feet_stay() {
		let (mvp, h) = hangar();
		let k = knobs();
		let d_bar = reflect_dist(&mvp, [0.0, 10.0, h + 0.6], h);
		assert!(d_bar > 4.0 && weight(d_bar, k) == 0.0, "bar {d_bar}");
		let d_top = reflect_dist(&mvp, [0.0, 10.0, h + 2.0], h);
		assert!(weight(d_top, k) == 0.0, "bar top {d_top}");
		let d_feet = reflect_dist(&mvp, [0.0, 2.0, h + 0.1], h);
		assert!(d_feet < 0.6 && weight(d_feet, k) == 1.0, "feet {d_feet}");
		let d_knee = reflect_dist(&mvp, [0.0, 2.0, h + 0.4], h);
		assert!(weight(d_knee, k) > 0.8, "knee {d_knee}");
		assert!(k[0] < k[1]);
	}

	/// Premultiplied (the contact blend, `PREMULTIPLY`): Ratchet's reflection (weight 1) averaged by the mips with the faded background
	/// (weight 0) keeps his colour once `metal_planar` divides the weight out; only his coverage halves (agent 20: the plain colour
	/// times the averaged weight lost him).
	#[test]
	fn contact_mips_keep_a_thin_near_object() {
		assert_eq!((PREMULTIPLY.color.src_factor, PREMULTIPLY.color.dst_factor), (wgpu::BlendFactor::Zero, wgpu::BlendFactor::SrcAlpha));
		assert_eq!((PREMULTIPLY.alpha.src_factor, PREMULTIPLY.alpha.dst_factor), (wgpu::BlendFactor::One, wgpu::BlendFactor::Zero));
		let (hero, back) = ([0.8f32, 0.6, 0.1], [0.1f32, 0.1, 0.2]);
		let k = knobs();
		let (wh, wb) = (weight(0.8, k), weight(12.0, k));
		assert!(wh == 1.0 && wb == 0.0);
		let mip: [f32; 3] = std::array::from_fn(|i| 0.5 * (hero[i] * wh + back[i] * wb));
		let a = 0.5 * (wh + wb);
		let got: [f32; 3] = std::array::from_fn(|i| mip[i] / a);
		assert!((0..3).all(|i| (got[i] - hero[i]).abs() < 1e-6) && a == 0.5, "{got:?} {a}");
	}

	/// A point drawn through the mirrored view-projection comes back at its real height.
	#[test]
	fn contact_rebuilds_the_real_height() {
		let (mvp, _) = hangar();
		let inv = super::super::mirror::inverse(&mvp).unwrap();
		let ap = |m: &[[f32; 4]; 4], p: [f32; 4]| -> [f32; 4] { std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum()) };
		let p = [0.5, 4.0, 2.2, 1.0];
		let c = ap(&mvp, p);
		let back = ap(&inv, [c[0] / c[3], c[1] / c[3], c[2] / c[3], 1.0]);
		assert!((back[2] / back[3] - 2.2).abs() < 1e-3, "{back:?}");
	}
}
