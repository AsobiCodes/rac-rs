//! Viewer sound state: the level / global sample banks (`Sfx`), voice bookkeeping, the retail start gate and the
//! spatial gain / pan / pitch of voices and live one-shots; with the start-gate test.

use super::*;

/// The level's decoded samples and which one the game plays for each event (sound::events, read
/// from the game's play calls and animation triggers; decomp/notes/sound.md "Event table").
#[derive(Default)]
pub(super) struct Sfx {
	pub(super) clips: Vec<Option<audio::Clip>>,
	pub(super) ev: sound::Events,
	/// False until the ambient loop's tone format is decoded.
	pub(super) ambient_ok: bool,
}

/// Mix level of the music: the settings default 0x2cc of the 0x400 full scale (music.md; the
/// Settings gain scales it from there).
pub(super) const MUSIC_VOLUME: f32 = 0x2cc as f32 / 1024.0;

/// A cue as (bank sound, volume) for the mixer.
pub(super) fn cue(c: Option<&sound::Cue>) -> (Option<usize>, f32) {
	c.map_or((None, 0.0), |c| (Some(c.sound), c.volume))
}

pub(super) fn load_sfx(l: &Level) -> Sfx {
	let clips = match sound::level_sounds(l) {
		Ok(samples) => samples
			.into_iter()
			.map(|s| (!s.pcm.is_empty()).then(|| audio::Clip { rate: s.rate, pcm: Arc::new(s.pcm), loop_start: s.loop_start, reverb: s.reverb }))
			.collect(),
		Err(e) => {
			eprintln!("sounds: {e}");
			return Sfx::default();
		}
	};
	let ev = sound::events(l).unwrap_or_else(|e| {
		eprintln!("sound events: {e}");
		sound::Events::default()
	});
	Sfx { clips, ev, ambient_ok: true }
}

/// The resident global bank (front-end and menu sounds), without level events.
pub(super) fn global_sfx(disc: &mut iso::Disc) -> Sfx {
	match sound::global_sounds(disc) {
		Ok(samples) => Sfx {
			clips: samples.into_iter().map(|s| (!s.pcm.is_empty()).then(|| audio::Clip { rate: s.rate, pcm: Arc::new(s.pcm), loop_start: s.loop_start, reverb: s.reverb })).collect(),
			ev: sound::Events::default(),
			ambient_ok: true,
		},
		Err(e) => {
			eprintln!("global sounds: {e}");
			Sfx::default()
		}
	}
}

/// RNG-4b: which voice a request-point draw belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VoiceKey {
	/// A wanted loop (`loop_want` owner).
	Loop(usize),
	/// An animation cue a prop's clock fired: (prop index, ordinal of the cue within the frame's list).
	EnemyCue(usize, usize),
	/// The looping sound of a prop's sequence.
	EnemyLoop(usize),
	Suck,
	Pyro,
}

/// RNG-4b / BRIDGE-24: the def's pitch draw of a voice is made when the voice is requested (in the tick, FUN_002a0cd0)
/// and kept in `pending` until the post-loop drain starts it; `rand` keeps the draw of a running voice for its updates.
#[derive(Default)]
pub(super) struct VoiceBook {
	pub(super) pending: HashMap<VoiceKey, i32>,
	pub(super) rand: HashMap<VoiceKey, i32>,
}

impl VoiceBook {
	/// The draw a starting voice uses: the one made at its request point, else (no tick ran) drawn now.
	pub(super) fn start(&mut self, key: VoiceKey, c: &sound::Cue) -> i32 {
		let r = self.pending.remove(&key).unwrap_or_else(|| c.random_shift(|n| particles::Rng::shared().small(n)));
		self.rand.insert(key, r);
		r
	}

	/// The draw of the running voice (updates draw nothing).
	pub(super) fn current(&self, key: VoiceKey) -> i32 {
		self.rand.get(&key).copied().unwrap_or(0)
	}
}

/// Animation-driven sound state of one enemy prop: sequence and the loop voice (id, class sound).
pub(super) struct EnemySfx {
	pub(super) seq: usize,
	pub(super) voice: Option<(u64, usize)>,
}

/// Retail start gate (FUN_002a0cd0) and out-of-range loop stop (FUN_0029ff68): a voice is audible while its
/// def's distance gain is at least 0x20 (decomp/notes/sound.md AUD-11h).
pub(super) fn audible(c: &sound::Cue, dist: f32) -> bool {
	c.fall.level(dist) != 0
}

/// Position of a voice owned by a moby: its position raised one unit (OV:101080 / OV:100699).
pub(super) fn voice_at(pos: [f32; 3]) -> [f32; 3] {
	[pos[0], pos[1], pos[2] + 1.0]
}

/// Distance from the listener (the camera) to `at`.
pub(super) fn listener_dist(cam: &Camera, at: [f32; 3]) -> f32 {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Volume scale (relative to the cue's own volume), pan and pitch shift (1/128 semitones) of a sound at `at`
/// heard from the camera: the def's retail falloff (FUN_0029fc10 via `Cue::level`), the retail pan curve
/// (FUN_0029fd48), Doppler, underwater and occlusion effects and the def's random pitch (sound.md "Voice
/// update", FUN_0029ff68 / FUN_002a0cd0).
pub(super) fn spatial(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity) -> (f32, f32, i32) {
	spatial_for(cam, at, cue, ln, None)
}

/// `spatial` for a voice owned by prop `owner`, whose own solid its occlusion line skips (FUN_0029fb70).
pub(super) fn spatial_for(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity, owner: Option<usize>) -> (f32, f32, i32) {
	spatial_rand(cam, at, cue, ln, owner, None)
}

/// `spatial_for` with the def's random pitch already drawn at the request point (BRIDGE-24, `Game.sound_requests`);
/// `None` draws it here.
pub(super) fn spatial_rand(cam: &Camera, at: [f32; 3], cue: Option<&sound::Cue>, ln: &sound::ListenerVelocity, owner: Option<usize>, drawn: Option<i32>) -> (f32, f32, i32) {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
	let right = [cam.yaw.sin(), -cam.yaw.cos()];
	let fwd = [cam.yaw.cos(), cam.yaw.sin()];
	let pan = sound::pan_of(d[0] * right[0] + d[1] * right[1], d[0] * fwd[0] + d[1] * fwd[1]);
	// Only the pan is wanted of a def-less call (ambience emitters, which do their own gain).
	let Some(c) = cue else { return (1.0, pan, 0) };
	let scale = if c.volume > 0.0 { c.level(l) / c.volume } else { 0.0 };
	let (vf, shift) = ln.effects(cam.pos, at, [0.0; 3], c.fall.flags);
	// Start-time occlusion test (six probes, def flag 2 exempts) when the def attenuates by distance.
	let occ = if scale > 0.0 { ln.occlusion(None, cam.pos, at, c.fall.flags, c.looped, owner) } else { 1.0 };
	let random = drawn.unwrap_or_else(|| c.random_shift(|n| particles::Rng::shared().small(n)));
	(scale * vf * occ, pan, shift + random)
}

/// An event-site one-shot that is re-evaluated every frame with the live listener (gain, pan, Doppler and
/// the occlusion history; FUN_0029ff68 re-runs every voice each update), decomp/notes/sound.md.
pub(super) struct LiveShot {
	pub(super) id: u64,
	pub(super) at: [f32; 3],
	pub(super) cue: sound::Cue,
	pub(super) owner: Option<usize>,
	/// The def's random pitch drawn at the start (1/128 semitones).
	pub(super) random: i32,
	/// ARI-27b: a flags-0x10 voice's fixed level (1/1024): no falloff, no occlusion.
	pub(super) level: Option<i32>,
}

/// Per-frame gain, pan and pitch shift of a running one-shot (`random` is added to the Doppler shift).
pub(super) fn live_shot(cam: &Camera, s: &LiveShot, at: [f32; 3], ln: &sound::ListenerVelocity) -> (f32, f32, i32) {
	let d = [at[0] - cam.pos[0], at[1] - cam.pos[1], at[2] - cam.pos[2]];
	let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
	let right = [cam.yaw.sin(), -cam.yaw.cos()];
	let fwd = [cam.yaw.cos(), cam.yaw.sin()];
	let pan = sound::pan_of(d[0] * right[0] + d[1] * right[1], d[0] * fwd[0] + d[1] * fwd[1]);
	let c = &s.cue;
	let (vf, shift) = ln.effects(cam.pos, at, [0.0; 3], c.fall.flags);
	if let Some(level) = s.level {
		return (level as f32 / 1024.0 * vf, pan, shift + s.random);
	}
	let scale = if c.volume > 0.0 { c.level(l) / c.volume } else { 0.0 };
	let occ = if scale > 0.0 { ln.occlusion(Some(s.id as u32), cam.pos, at, c.fall.flags, false, s.owner) } else { 1.0 };
	(c.volume * scale * vf * occ, pan, shift + s.random)
}

#[cfg(test)]
mod audible_tests {
	use super::*;

	// AUD-11h tests 1 and 2: the start gate edge and the owner voice offset.
	#[test]
	fn start_gate_and_voice_at() {
		let c = sound::Cue { fall: sound::Falloff { min_range: 0.0, max_range: 100.0, min_vol: 0, max_vol: 1228, min_pitch: 0, max_pitch: 0, flags: 0 }, sound: 0, volume: 1.0, looped: true, bend: (0, 0) };
		assert_eq!(c.fall.level(97.0), 36);
		assert!(audible(&c, 97.0));
		assert_eq!(c.fall.level(97.39), 32);
		assert!(audible(&c, 97.39));
		assert!(!audible(&c, 97.4));
		assert!(!audible(&c, 98.0));
		assert_eq!(c.fall.level(50.0), 614);
		assert!(audible(&c, 50.0));
		assert_eq!(voice_at([1.0, 2.0, 3.0]), [1.0, 2.0, 4.0]);
	}
}
