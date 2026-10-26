//! [PC] Split view (retail/splitview.rs), props: the draw-distance pick of the props (`Gpu::set_props` with the clip view) is
//! made by the handler with the player's setting before `Gpu::draw`; the Classic half must pick with the Classic one, so it is
//! re-run here around the Classic pass (`Gpu::split_classic` calls it with the Classic flag up, then once more for the real pick).

use super::*;

/// Draws the Classic half of a split frame (nothing when the split view is off or both halves would be equal).
pub(super) fn run(g: &mut Gpu, game: Option<&Game>, smooth: &smooth::Smooth, debris: &[particles::Debris], f: &Frame, cam: &Camera) {
	g.split_classic(cam, &mut |g: &mut Gpu| {
		let Some(game) = game else { return };
		g.redo_clip_view(cam);
		let (centres, radii) = (game.fade_centres(), game.fade_radii());
		g.set_props(Some(&game.props), smooth.posed(), &f.fx_posed, &f.actors, debris, &[], &centres, &radii);
	});
}
