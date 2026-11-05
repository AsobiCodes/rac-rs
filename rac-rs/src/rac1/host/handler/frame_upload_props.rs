//! RedrawRequested, upload phase: the props' draw inputs that are CPU work on game state only (per prop tint, class UV
//! scrolls, draw-distance fade spheres). [PC] perf: `frame_upload` builds them before its first queue use, which waits for the
//! present thread's submit of the last frame (presenter.rs `GatedQueue`), so this work overlaps that wait.

use super::*;

/// Fills `PROP_TINTS` and `g.class_uv` for this frame (`Gpu::set_props` reads them) and returns the fade spheres
/// (`Game::fade_spheres`: centres, radii). PROP_TINTS and class_uv keep their buffers between frames (the sphere Vecs are new per call); the values are the per-frame rebuild's.
pub(super) fn prop_inputs(g: &mut Gpu, game: &game::Game, prop_anims: &HashMap<u32, (mobies::MobyClass, anim::Animated)>, retail: Option<&crate::retail::Retail>) -> (Vec<[f32; 3]>, Vec<Option<f32>>) {
	let _s = profile::span("u_tints");
	PROP_TINTS.with(|t| {
		let mut t = t.borrow_mut();
		t.clear();
		crate::retail::queue_q2::with_static_tints(|static_start| {
			// The last class's lookups kept (pool slots of one class are contiguous): the same answers, no rehash per prop.
			let mut memo: Option<(u32, Option<&(mobies::MobyClass, anim::Animated)>, Option<Option<usize>>)> = None;
			t.extend(game.props.iter().map(|p| {
				let m = memo.filter(|m| m.0 == p.class_id).unwrap_or_else(|| (p.class_id, prop_anims.get(&p.class_id), None));
				let Some((class, _)) = m.1 else {
					// Retail mode: a static class (no animation, so not in `prop_anims`) takes the tail colour too.
					let st = m.2.unwrap_or_else(|| static_start(p.class_id as u32));
					memo = Some((m.0, m.1, Some(st)));
					let start = st?;
					return p.retail_tint().flatten().map(|c| (c, start));
				};
				memo = Some(m);
				// Retail mode: the moby's own +0x90 word (class +0x40 at init, actor overrides) when +0x34 & 0x10.
				if let Some(t) = p.retail_tint() {
					return t.map(|c| (c, class.colour_start));
				}
				Some((mobies::glow_rgb(class.glow)?, class.colour_start))
			}));
		});
	});
	// The class UV scrolls retail holds in RAM (only looked up by class: the map's contents are what a fresh collect gives).
	g.class_uv.clear();
	if let Some(fx) = retail.and_then(|r| r.fx()) {
		g.class_uv.extend(fx.class_uv.iter().copied());
	}
	let _f = profile::span("u_fade");
	game.fade_spheres()
}
