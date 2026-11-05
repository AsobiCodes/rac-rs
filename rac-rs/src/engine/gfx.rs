//! [PC] What the game's Graphics page asks of the renderer (engine split stage E2 batch 4a, 2026-10-26: moved out of
//! rac1/retail/pcext.rs): `Graphics` (lighting, the effect rows and their levels, vegetation, culling, draw distance, fog,
//! clouds); `graphics()`, the verdict the renderer reads every frame (the game pack's answer, `pack::Pack::graphics`, through
//! the split view); the scale of the retail fog while the volumetric fog draws (`gs_scale`).

use std::sync::atomic::Ordering;

/// What the Graphics page asks of the renderer (viewer/post.rs). `lighting` 0 = Classic: the retail picture, nothing added; the
/// other rows only act with Lighting Enhanced, except frame smoothing, culling and the draw distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Graphics {
	/// 0 Classic (the retail picture), 1 Enhanced.
	pub lighting: u8,
	/// Ambient occlusion on (`ao_q` > 0).
	pub ao: bool,
	pub shadows: bool,
	/// Water Enhanced (viewer/waterfx.rs; `water_q` > 0).
	pub water: bool,
	/// Particles Enhanced (`particles_q` > 0).
	pub particles: bool,
	pub sun_rays: bool,
	/// Frame smoothing On (rac1/interp.rs; works with Lighting Classic too).
	pub smoothing: bool,
	/// Vegetation: 0 Off, 1 Moderate, 2 Lush (viewer/vegfx.rs).
	pub veg: u8,
	/// Culling On (the default): the renderer leaves out what is hidden or out of view. Off draws it (viewer/gpu.rs `culling`).
	pub culling: bool,
	/// The quality level of each effect row: 0 Off (Classic), 1 Low, 2 Medium, 3 High. High is the full Enhanced picture; the
	/// renderer reads these to pick the cheaper settings of Low and Medium.
	pub ao_q: u8,
	pub shadows_q: u8,
	pub water_q: u8,
	pub particles_q: u8,
	pub sun_q: u8,
	/// Global illumination (viewer/post.rs `fs_gi`): 0 Off, 1 Low, 2 Medium, 3 High.
	pub gi_q: u8,
	/// Waves On (the default): the sea of Water Medium / High rolls in geometric waves (viewer/waterfx.rs `geo_look`).
	pub waves: bool,
	/// Draw distance (viewer/fardraw.rs, with either Lighting): 0 Low (retail's distances and PS2 far LODs), 1 Medium (the
	/// default), 2 High (`draw_q`).
	pub draw: u8,
	/// Fog (viewer/volfog.rs, with Lighting Enhanced): 0 Classic (the retail fog only), 1 Low, 2 Medium, 3 High (retail/pcext/fogopt.rs).
	pub fog_q: u8,
	/// Volumetric clouds (viewer/volclouds.rs, with Lighting Enhanced): 0 Classic (the retail sky only), 1 Low, 2 Medium, 3 High (retail/pcext/cloudopt.rs).
	pub clouds_q: u8,
	/// Fur on the hero (viewer/furfx.rs, with Lighting Enhanced): 0 Off (the retail picture), 1 Low, 2 Medium, 3 High.
	pub fur_q: u8,
	/// Fur style: false Soft (short fluff), true Long (strands past the outline).
	pub fur_long: bool,
}

impl Graphics {
	pub const CLASSIC: Graphics = Graphics { lighting: 0, ao: false, shadows: false, water: false, particles: false, sun_rays: false, smoothing: false, veg: 0, culling: true, ao_q: 0, shadows_q: 0, water_q: 0, particles_q: 0, sun_q: 0, gi_q: 0, waves: true, draw: 0, fog_q: 0, clouds_q: 0, fur_q: 0, fur_long: false };

	/// Enhanced: the post-processing runs.
	pub fn enhanced(self) -> bool {
		self.lighting != 0
	}

	/// The draw distance level: 0 Low (retail), 1 Medium, 2 High.
	pub fn draw_q(self) -> u8 {
		self.draw.min(2)
	}
}

/// The scale of retail's own fog (f32 bits; 1.0 = the retail picture): the volumetric fog (viewer/gpu/draw/prepare.rs, from `volfog::Plan`)
/// lowers it while it draws, because retail's fog would paint a flat opaque floor under the haze. The world uniform upload
/// (viewer/gpu/draw/prepare.rs -> volfog `fade_retail_fog`) multiplies the fog's share (1 - f) by it. The GS capture is not
/// involved: the world's fog is drawn by the native shaders (viewer.rs), not by captured GS prims.
static GS_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// The viewer's per-frame verdict: the retail fog's scale (1.0 when no volumetric fog is drawn).
pub fn set_gs_scale(s: f32) {
	GS_SCALE.store(s.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// The retail GS fog's scale for the capture (exactly 1.0 = unchanged maths).
pub fn gs_scale() -> f32 {
	f32::from_bits(GS_SCALE.load(Ordering::Relaxed))
}

/// The Graphics page's verdict as the running pass sees it (render thread, every frame): the game pack's answer
/// (`pack::Pack::graphics`; Classic without a game) through the split view (`splitview::pass`).
pub fn graphics() -> Graphics {
	crate::splitview::pass((crate::pack::get().graphics)())
}
