//! [S3] DLSS frame generation, render thread (RAC_FG=dlss2..4, sl.rs): the frame's Streamline inputs, set at the end of
//! `upscale_encode` (frame.rs `fg_encode`), before the HUD pass. The same inputs as FSR frame generation (frame.rs): the upscaler
//! inputs' depth and motion vectors (render size; with no upscaler they run as Mode::Still, no jitter) and the hudless picture (the
//! scene before the HUD at the window size). DirectX 12: tagged as the native ID3D12Resources in the shader-resource state wgpu
//! leaves them in after our transition to RESOURCE (PIXEL | NON_PIXEL_SHADER_RESOURCE), valid until the present, no command list.
//! Vulkan (sl_vk.rs `resource`): VkImage + VkImageView + the layout of RESOURCE, size, format and usage; the hudless picture's
//! view is made once per hudless texture (sl.rs `hudless_resource`; the one before lives until the next size change).
//! The camera constants come from the same eye / forward / up / fov / far as FSR's (`sl_math`, pure, row-major, row vectors as
//! SL's headers say; clipToPrevClip through the PREVIOUS frame's view and projection; a fov jump counts as a cut). A frame
//! without inputs clears its tags and goes without generation (sl.rs `present_begin`).

use super::sl::Sl;
use super::sl_abi as abi;
use crate::viewer::Gpu;

/// D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE (wgpu's RESOURCE state).
const DX12_SRV: u32 = 0xC0;

impl Gpu {
	/// [S3] This frame's Streamline constants and tags (nothing unless DLSS frame generation runs on our swapchain).
	pub(super) fn sl_encode(&mut self, enc: &mut wgpu::CommandEncoder, sl: &'static Sl) {
		let Some(fr) = self.screen.sl_frame() else { return };
		if !sl.usable() {
			return;
		}
		// The back buffer keeps the window extent even when cleared (SL guide DLSS-G 5.2, as below): no "Invalid backbuffer
		// resource extent" at the first paused frame.
		let win = self.win_size();
		let clear = |sl: &Sl| {
			let ext = |t| if t == abi::BUFFER_BACKBUFFER { abi::Extent { top: 0, left: 0, width: win.0, height: win.1 } } else { abi::Extent::default() };
			let tags = [abi::BUFFER_DEPTH, abi::BUFFER_MVEC, abi::BUFFER_HUDLESS, abi::BUFFER_BACKBUFFER].map(|t| abi::ResourceTag::new(std::ptr::null_mut(), t, ext(t)));
			sl.set_tags(&fr, &tags);
		};
		// Paused (frame.rs `fg_paused`: a picture box, or the value in force is Off: follow.rs): no tags, the frame stays
		// unprepared (DLSS-G off for it: today's real frames). Its camera history is dropped and the first resumed frame sends a
		// reset (frame.rs `fg_resume_reset`).
		if self.fg_paused(super::follow::Maker::Dlss) {
			*sl.prev.lock().unwrap_or_else(|e| e.into_inner()) = None;
			clear(sl);
			return;
		}
		// Lane fglive: the menu's DLSS 2x/3x/4x applies at once (sl.rs `want`: the next DLSS-G options carry it).
		if let Some(v) = super::follow::dlss_value() {
			sl.want(v as u32 - 1);
		}
		let (Some((motion, depth, jitter, reset)), Some((eye, fwd, up))) = (self.upscale.outputs(), self.upscale.fg_view()) else {
			clear(sl);
			return;
		};
		let (mv, dv) = (motion.clone(), depth.clone());
		let (motion, depth) = (motion.texture().clone(), depth.texture().clone());
		let display = self.win_size();
		let render = (motion.width(), motion.height());
		if render.0 > display.0 || render.1 > display.1 {
			clear(sl);
			return;
		}
		let hudless = self.fg_hudless(enc, display);
		let mut list = vec![&motion, &depth];
		list.extend(hudless.iter());
		enc.transition_resources(std::iter::empty(), list.iter().map(|&texture| wgpu::TextureTransition { texture, selector: None, state: wgpu::TextureUses::RESOURCE }));
		let res = |t: &wgpu::Texture| unsafe { t.as_hal::<wgpu::hal::api::Dx12>() }.map(|h| unsafe { h.raw_resource() }.clone());
		let (mut rm, mut rd, mut rh) = if let (Some(m), Some(d)) = (res(&motion), res(&depth)) {
			use windows::core::Interface;
			let h = hudless.as_ref().and_then(res);
			(abi::Resource::tex2d(m.as_raw(), DX12_SRV), abi::Resource::tex2d(d.as_raw(), DX12_SRV), h.map(|h| abi::Resource::tex2d(h.as_raw(), DX12_SRV)))
		} else {
			use super::sl_vk::resource;
			let (Some(m), Some(d)) = (resource(&motion, &mv), resource(&depth, &dv)) else {
				clear(sl);
				return;
			};
			(m, d, hudless.as_ref().and_then(|t| sl.hudless_resource(t)))
		};
		let (far, fov_y) = self.upscale.fg_cam();
		let reset = self.fg_resume_reset() || reset;
		let cam = sl_math::Cam { eye, fwd, up, fov_y, near: crate::viewer::projection::RETAIL_NEAR, far, aspect: render.0 as f32 / render.1.max(1) as f32, jitter, reset };
		let mut prev = sl.prev.lock().unwrap_or_else(|e| e.into_inner());
		let (c, now) = sl_math::constants(&cam, *prev);
		*prev = Some(now);
		drop(prev);
		let ext = abi::Extent { top: 0, left: 0, width: render.0, height: render.1 };
		let full = abi::Extent { top: 0, left: 0, width: display.0, height: display.1 };
		let tags = [
			abi::ResourceTag::new(&mut rd, abi::BUFFER_DEPTH, ext),
			abi::ResourceTag::new(&mut rm, abi::BUFFER_MVEC, ext),
			match rh.as_mut() {
				Some(r) => abi::ResourceTag::new(r, abi::BUFFER_HUDLESS, full),
				None => abi::ResourceTag::new(std::ptr::null_mut(), abi::BUFFER_HUDLESS, abi::Extent::default()),
			},
			// Null resource + the window extent (SL guide DLSS-G 5.2): silences "Invalid backbuffer resource extent".
			abi::ResourceTag::new(std::ptr::null_mut(), abi::BUFFER_BACKBUFFER, full),
		];
		let ok = sl.set_constants(&fr, &c) && sl.set_tags(&fr, &tags);
		super::trace(format_args!("sl encode frame {} render {render:?} hudless {} ok {ok}", fr.index, hudless.is_some()));
		// The resources stay alive with wgpu's textures (the upscaler's and the screen's) until the present.
		self.screen.sl_prepared(ok);
	}
}

/// The camera matrices of Streamline's constants (pure): row-major, row vectors (`v * M`), left-handed view (x right, y up, z
/// forward), clip depth 0..1 (not inverted), no jitter in the matrices.
pub(super) mod sl_math {
	use super::abi::{self, Mat};

	pub(in crate::viewer::fgen) struct Cam {
		pub eye: [f32; 3],
		pub fwd: [f32; 3],
		pub up: [f32; 3],
		pub fov_y: f32,
		pub near: f32,
		pub far: f32,
		pub aspect: f32,
		pub jitter: [f32; 2],
		pub reset: bool,
	}

	pub(in crate::viewer::fgen) const IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

	/// A change of the vertical fov above this from one frame to the next is a cut (radians, ~2.9 degrees: a camera switch;
	/// the game's zooms move by far less per frame).
	pub(in crate::viewer::fgen) const FOV_CUT: f32 = 0.05;

	/// The previous frame's camera: world -> view, view -> clip and its vertical fov.
	#[derive(Clone, Copy, Debug)]
	pub(in crate::viewer::fgen) struct Prev {
		pub view: Mat,
		pub proj: Mat,
		pub fov_y: f32,
	}

	fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
		a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
	}
	fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
		[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
	}
	fn norm(a: [f32; 3]) -> [f32; 3] {
		let l = dot(a, a).sqrt();
		if l > 0.0 { [a[0] / l, a[1] / l, a[2] / l] } else { a }
	}

	/// The camera basis (right, up, forward), as FSR's (ffx_fg.rs `prepare_desc`).
	pub(in crate::viewer::fgen) fn basis(fwd: [f32; 3], up: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
		let right = norm(cross(fwd, up));
		(right, norm(cross(right, fwd)), norm(fwd))
	}

	/// World -> view.
	pub(in crate::viewer::fgen) fn view(eye: [f32; 3], fwd: [f32; 3], up: [f32; 3]) -> Mat {
		let (r, u, f) = basis(fwd, up);
		[[r[0], u[0], f[0], 0.0], [r[1], u[1], f[1], 0.0], [r[2], u[2], f[2], 0.0], [-dot(r, eye), -dot(u, eye), -dot(f, eye), 1.0]]
	}

	/// View -> clip, depth 0 (near) .. 1 (far).
	pub(in crate::viewer::fgen) fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat {
		let ys = 1.0 / (fov_y * 0.5).tan();
		let xs = ys / aspect.max(1e-6);
		let q = far / (far - near);
		[[xs, 0.0, 0.0, 0.0], [0.0, ys, 0.0, 0.0], [0.0, 0.0, q, 1.0], [0.0, 0.0, -near * q, 0.0]]
	}

	pub(in crate::viewer::fgen) fn mul(a: &Mat, b: &Mat) -> Mat {
		let mut o = [[0.0f32; 4]; 4];
		for i in 0..4 {
			for j in 0..4 {
				o[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
			}
		}
		o
	}

	/// The inverse (Gauss-Jordan with partial pivoting, f64); a singular matrix gives the identity.
	pub(in crate::viewer::fgen) fn inverse(m: &Mat) -> Mat {
		let mut a = [[0.0f64; 8]; 4];
		for i in 0..4 {
			for j in 0..4 {
				a[i][j] = m[i][j] as f64;
			}
			a[i][4 + i] = 1.0;
		}
		for c in 0..4 {
			let p = (c..4).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs())).unwrap_or(c);
			if a[p][c].abs() < 1e-12 {
				return IDENTITY;
			}
			a.swap(c, p);
			let d = a[c][c];
			for v in a[c].iter_mut() {
				*v /= d;
			}
			for r in 0..4 {
				if r != c {
					let f = a[r][c];
					for k in 0..8 {
						a[r][k] -= f * a[c][k];
					}
				}
			}
		}
		let mut o = [[0.0f32; 4]; 4];
		for i in 0..4 {
			for j in 0..4 {
				o[i][j] = a[i][4 + j] as f32;
			}
		}
		o
	}

	/// The frame's constants and its camera (kept for the next frame's `prev`). clipToPrevClip = clip -> view -> world (this
	/// frame's inverses) -> previous view -> previous clip (the previous frame's matrices: a zoom changes the projection). With no
	/// previous camera, a cut or a fov jump (`FOV_CUT`, also sent as reset) it is the identity.
	pub(in crate::viewer::fgen) fn constants(c: &Cam, prev: Option<Prev>) -> (abi::Constants, Prev) {
		let v = view(c.eye, c.fwd, c.up);
		let p = perspective(c.fov_y, c.aspect, c.near, c.far);
		let p_inv = inverse(&p);
		let reset = c.reset || prev.is_some_and(|q| (q.fov_y - c.fov_y).abs() > FOV_CUT);
		let to_prev = match prev.filter(|_| !reset) {
			Some(q) => mul(&mul(&mul(&p_inv, &inverse(&v)), &q.view), &q.proj),
			None => IDENTITY,
		};
		let (r, u, f) = basis(c.fwd, c.up);
		let k = abi::Constants {
			header: abi::Header::of(abi::T_CONSTANTS, 2),
			camera_view_to_clip: p,
			clip_to_camera_view: p_inv,
			clip_to_lens_clip: IDENTITY,
			clip_to_prev_clip: to_prev,
			prev_clip_to_clip: inverse(&to_prev),
			jitter_offset: c.jitter,
			// Our motion vectors are in UV units (FSR scales them by the render size).
			mvec_scale: [1.0, 1.0],
			camera_pinhole_offset: [0.0, 0.0],
			camera_pos: c.eye,
			camera_up: u,
			camera_right: r,
			camera_fwd: f,
			camera_near: c.near,
			camera_far: c.far,
			camera_fov: c.fov_y,
			camera_aspect_ratio: c.aspect,
			motion_vectors_invalid_value: abi::INVALID_FLOAT,
			depth_inverted: abi::FALSE,
			camera_motion_included: abi::TRUE,
			motion_vectors_3d: abi::FALSE,
			reset: if reset { abi::TRUE } else { abi::FALSE },
			orthographic_projection: abi::FALSE,
			motion_vectors_dilated: abi::FALSE,
			motion_vectors_jittered: abi::FALSE,
			min_relative_linear_depth_object_separation: 40.0,
		};
		(k, Prev { view: v, proj: p, fov_y: c.fov_y })
	}
}
