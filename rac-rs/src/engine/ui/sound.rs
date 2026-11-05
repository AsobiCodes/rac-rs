//! Menu sounds of the modern UI (route C of the S3 plan): short procedural ticks played on the engine mixer. The game pack's
//! `Pack::ui_sound` points at `request`; a pack with no modern menu keeps the default no-op. `request` may be called from any
//! thread; the display frame step calls `drain` with the engine `Audio` and plays everything that was asked for since the last frame.
use super::rows::UiSound;
use crate::audio::{Audio, Clip};
use std::sync::{Arc, Mutex, OnceLock};

/// Sounds asked for and not yet played. Capped so a silent device (no `Audio`) cannot grow it.
static PENDING: Mutex<Vec<UiSound>> = Mutex::new(Vec::new());
/// The three clips, built on first use.
static CLIPS: OnceLock<[Clip; 3]> = OnceLock::new();

const RATE: u32 = 48_000;
const PENDING_CAP: usize = 8;
/// Playback volume of every tick (the engine master volume is applied on top).
const VOLUME: f32 = 0.35;

/// `Pack::ui_sound` for a game with a modern menu.
pub fn request(s: UiSound) {
	if let Ok(mut p) = PENDING.lock() {
		if p.len() < PENDING_CAP {
			p.push(s);
		}
	}
}

/// Takes the queued sounds (oldest first).
pub fn take_pending() -> Vec<UiSound> {
	PENDING.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default()
}

/// Plays and clears the queue; call once per display frame. A burst of identical sounds in one frame (a held repeat) plays once.
pub fn drain(audio: &mut Audio) {
	let mut played = [false; 3];
	for s in take_pending() {
		let i = index(s);
		if std::mem::replace(&mut played[i], true) {
			continue;
		}
		audio.play(&clips()[i], VOLUME, 0.0);
	}
}

fn index(s: UiSound) -> usize {
	match s {
		UiSound::Move => 0,
		UiSound::Select => 1,
		UiSound::Back => 2,
	}
}

fn clips() -> &'static [Clip; 3] {
	CLIPS.get_or_init(|| [UiSound::Move, UiSound::Select, UiSound::Back].map(|s| Clip { rate: RATE, pcm: Arc::new(synth(s)), loop_start: None, reverb: false }))
}

/// One tone segment: `ms` long at `hz`, a 2 ms attack and an exponential-looking decay so it ticks instead of clicking.
fn tone(out: &mut Vec<i16>, hz: f32, ms: f32, amp: f32) {
	let n = (RATE as f32 * ms / 1000.0) as usize;
	let attack = (RATE as f32 * 0.002) as usize;
	for i in 0..n {
		let t = i as f32 / RATE as f32;
		let env_in = if i < attack { i as f32 / attack as f32 } else { 1.0 };
		let k = i as f32 / n as f32;
		let env_out = (1.0 - k) * (1.0 - k);
		let v = (t * hz * std::f32::consts::TAU).sin() * env_in * env_out * amp;
		out.push((v * i16::MAX as f32) as i16);
	}
}

/// The PCM of one menu sound: Move = 25 ms at 1.2 kHz, Select = two steps (900 Hz, 1.35 kHz) of 22 ms, Back = 35 ms at 700 Hz.
pub fn synth(s: UiSound) -> Vec<i16> {
	let mut out = Vec::new();
	match s {
		UiSound::Move => tone(&mut out, 1200.0, 25.0, 0.8),
		UiSound::Select => {
			tone(&mut out, 900.0, 22.0, 0.8);
			tone(&mut out, 1350.0, 23.0, 0.8);
		}
		UiSound::Back => tone(&mut out, 700.0, 35.0, 0.8),
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn synth_lengths_and_levels() {
		let m = synth(UiSound::Move);
		let s = synth(UiSound::Select);
		let b = synth(UiSound::Back);
		assert_eq!(m.len(), 1200);
		assert_eq!(s.len(), 1056 + 1104);
		assert_eq!(b.len(), 1680);
		for pcm in [&m, &s, &b] {
			let peak = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap();
			assert!(peak > 3000 && peak < 30000, "peak {peak}");
			assert_eq!(pcm[0], 0);
		}
	}

	#[test]
	fn request_queues_and_take_clears() {
		take_pending();
		request(UiSound::Move);
		request(UiSound::Back);
		assert_eq!(take_pending(), vec![UiSound::Move, UiSound::Back]);
		assert!(take_pending().is_empty());
		for _ in 0..PENDING_CAP + 5 {
			request(UiSound::Select);
		}
		assert_eq!(take_pending().len(), PENDING_CAP);
	}
}
