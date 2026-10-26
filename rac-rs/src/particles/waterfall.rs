//! Region-5 waterfall particles (types 0x38 mist and 0x39 flow): their state structs and `waterfall_sprites`.
//! Leftover from the retired hand port; still compiled because retail mode and the renderer still use parts of `particles` (`Rng`, sprite and quad types). Do not extend.
// NOV-20.4: region-5 waterfall particles (types 0x38 and 0x39).
use super::*;


#[derive(Clone, Debug)]
pub(super) struct Mist {
	pos: [f32; 3],
	size: f32,
	colour: u32,
	rot: u8,
}


#[derive(Clone, Debug)]
pub(super) struct Flow {
	pos: [f32; 3],
	size: f32,
	colour: u32,
	rot: u8,
}


impl Particles {

	pub(super) fn waterfall_sprites(e: &Effect, out: &mut Vec<Sprite>) {
		// NOV-22.12: the 0x39 flow is a mode-1 flat sprite, the 0x38 mist a billboard.
		let (pos, size, rot, colour, tex, pool, flat) = match e {
			Effect::WaterfallMist(p) => (p.pos, p.size, p.rot, p.colour, 0x38, 0x24, false),
			Effect::WaterfallFlow(p) => (p.pos, p.size, p.rot, p.colour, 0x39, 0x2c, true),
			_ => return,
		};
		out.push(Sprite { pos, size: size * SIZE_SCALE, rot: rot as f32 * TAU / 256.0,
			colour: tint(word(colour)), tex, additive: true, pool: Some(pool), flat });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

}
