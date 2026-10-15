use super::*;

impl Particles {




















	/// The live shard and fireball mobys.
	pub fn effect_mobys(&self) -> Vec<EffectMoby> {
		let mut out: Vec<EffectMoby> = self.shards.iter().map(|s| EffectMoby { class: SHARD_CLASS, pos: s.pos, rot: [s.rot[0], s.rot[1], 0.0], scale: s.scale, alpha: 128.0, tint: [127.0 / 128.0; 3], draw: Some(255) }).collect();
		out.extend(self.fireballs.iter().map(|f| EffectMoby { class: f.class, pos: f.pos, rot: f.rot, scale: f.scale, alpha: f.alpha, tint: f.rgb.map(|c| c / 128.0), draw: Some(255) }));
		out
	}





	/// Draw state of the Novalis effect types.
	pub(super) fn blast_sprites(&self, e: &Effect, out: &mut Vec<Sprite>) {
		match e {
			Effect::BlastEmber(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(k.colour), tex: k.tex, additive: k.additive, pool: Some(B9_FAR), flat: false }),
			// WPN-6.4e2 type 0x40: mode 0 billboard, colour fixed at c0.
			Effect::WaterDebris(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(word(k.c0)), tex: k.tex, additive: k.additive, pool: Some(k.b9), flat: false }),
			Effect::BlastSpark(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(k.colour), tex: TEX_SPARK, additive: k.additive, pool: Some(B9_FAR), flat: false }),
			Effect::Flame(k) => out.push(Sprite {
				pos: k.pos,
				size: k.size * SIZE_SCALE,
				rot: k.rot as f32 * TAU / 256.0,
				colour: tint(k.colour),
				tex: FX_TEX + PART_TEX_BASE + self.flame_frames[k.frame.min(9)] as u32,
				additive: true,
				pool: Some(B9_FAR),
				flat: false,
			}),
			Effect::Smoke(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(k.colour), tex: TEX_SMOKE4, additive: k.additive, pool: Some(B9_FAR), flat: false }),
			// NOV-9.3: blend byte 0x44 (source-alpha-over), +9 0x74.
			// Type 0x10: blend byte 0x44 (source-alpha-over), near/far 0x44; the def's frame for its mode.
			Effect::Puff10(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(k.colour), tex: FX_TEX + PART_TEX_BASE + k.tex as u32, additive: false, pool: Some(0x44), flat: false }),
			Effect::SmokeLoft(k) => out.push(Sprite { pos: k.pos, size: k.size * SIZE_SCALE, rot: k.rot as f32 * TAU / 256.0, colour: tint(k.colour), tex: TEX_SMOKE_LOFT, additive: false, pool: Some(B9_PUFF), flat: false }),
			_ => {}
		}
	}
}


