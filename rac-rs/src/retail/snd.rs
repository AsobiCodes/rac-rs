//! Retail sound in rac-rs: the exact 989snd driver (ee-oracle `snd989`, a Rust port of the IOP module) plays the game's
//! sounds, rac-rs only mixes what it writes to the SPU2.
//!
//! Each field the oracle thread hands over a `Batch` (spumix.rs `drain`): the driver's SPU2 register writes in order
//! (voice pitch / volume / ADSR / start address, key-on / key-off, master and effect registers), the bank or stream sample
//! data the voice plays (`Ev.bank` / `Ev.stream`), the core mix image, and (once) the state of an adopted savestate. `field`
//! hands them to the SPU2 voice mixer in audio.rs / spumix.rs, which plays them 1:1 with the hardware: bank sounds, music and
//! speech streams (the VAG payload is read from the player's disc at run time), group and master volumes, reverb, pause.
//! Nothing of the game's sound is decided on this side; the volume options reach the driver as the game's own commands
//! (cid 9, music = group 1).

use crate::audio::Audio;
use super::SndBatch;

/// One field of the retail driver's sound: attach the SPU2 voice mixer on the first batch (the state an adopted savestate
/// carries) and feed it the field's register writes.
pub fn field(a: &mut Audio, ev: SndBatch) {
	if let Some(att) = &ev.attach {
		a.spu_attach(att.tick, Some(att), true);
	} else if !a.spu_on() {
		a.spu_attach(0, None, true);
	}
	a.spu_push(ev.ev, Some(&ev.core));
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write_wav(path: &str, rate: u32, ch: u16, s: &[f32]) {
		let mut f = Vec::with_capacity(44 + s.len() * 2);
		let n = (s.len() * 2) as u32;
		f.extend_from_slice(b"RIFF");
		f.extend_from_slice(&(36 + n).to_le_bytes());
		f.extend_from_slice(b"WAVEfmt ");
		f.extend_from_slice(&16u32.to_le_bytes());
		f.extend_from_slice(&1u16.to_le_bytes());
		f.extend_from_slice(&ch.to_le_bytes());
		f.extend_from_slice(&rate.to_le_bytes());
		f.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
		f.extend_from_slice(&(ch * 2).to_le_bytes());
		f.extend_from_slice(&16u16.to_le_bytes());
		f.extend_from_slice(b"data");
		f.extend_from_slice(&n.to_le_bytes());
		for x in s {
			f.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
		}
		std::fs::write(path, f).unwrap();
	}

	/// Stage 9 (manual): RAC_SND9_STATE (a savestate dir) or RAC_SND9_COLD (the ISO: a cold boot), the oracle's
	/// SPU2 writes of RAC_SND9_FIELDS (1500) fields replayed on the SPU2 voice mixer. Checks: at every field the mixer's ENVX of
	/// all 48 voices equals the oracle driver's (tick-exact render); writes RAC_SND9_OUT (WAV, 48 kHz stereo through audio.rs
	/// `mixer`, reverb off) and the key-on log RAC_SND9_KEYS (field, tick, core, voices, bank sample starts).
	#[test]
	#[ignore]
	fn snd9_wav() {
		use crate::spumix::SpuVoices;
		use ee_oracle::snd989::voices::Reg;
		let n: u32 = std::env::var("RAC_SND9_FIELDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
		let out = std::env::var("RAC_SND9_OUT").unwrap_or_else(|_| "snd9.wav".into());
		let keys_path = std::env::var("RAC_SND9_KEYS").unwrap_or_else(|_| "snd9_keys.txt".into());
		match (std::env::var("RAC_SND9_STATE"), std::env::var("RAC_SND9_COLD")) {
			(Ok(dir), _) => crate::retail::enable(&dir).unwrap(),
			(_, Ok(iso)) => crate::retail::enable_cold(&iso).unwrap(),
			_ => panic!("set RAC_SND9_STATE or RAC_SND9_COLD"),
		}
		let mut r = crate::retail::Retail::load().expect("load");
		crate::retail::fmv_bridge::set_serviced(true);
		let (mut audio, mut mix) = Audio::offline(48000.0, 2);
		// RAC_SND9_REVERB=mode,depth: the effect type the game asked for (cid 80), as `Bridge::reverb` reports it
		if let Some((m, d)) = std::env::var("RAC_SND9_REVERB").ok().and_then(|v| v.split_once(',').map(|(a, b)| (a.to_string(), b.to_string()))) {
			audio.set_reverb(m.parse().unwrap(), d.parse().unwrap());
		}
		let mut check: Option<SpuVoices> = None;
		let (mut wav, mut keys) = (Vec::<f32>::new(), String::new());
		let (mut last_tick, mut bad, mut checked, mut kons) = (0u32, 0u32, 0u32, 0u32);
		let mut first_bad = String::new();
		let trace: Option<usize> = std::env::var("RAC_SND9_TRACE").ok().and_then(|v| v.parse().ok());
		let (mut t0, mut want_kons) = (0u32, Vec::<(i64, usize)>::new());
		// RAC_SND_PAD (hex buttons, `from-to:buttons[:lx,ly]` entries): presses the buttons for those fields (help voice, dialogue).
		let pad = pad_script();
		for f in 0..n {
			let on = |p: &&(u32, u32, u16, Option<[u8; 2]>)| (p.0..p.1).contains(&f);
			let st = pad.iter().filter(on).find_map(|p| p.3).unwrap_or([0x80; 2]);
			r.tick(pad.iter().filter(on).fold(0, |a, p| a | p.2), [0x80, 0x80, st[0], st[1]]);
			if r.take_movie().is_some() {
				r.end_movie();
			}
			let b = r.take_sound();
			let nofx = std::env::var("RAC_SND9_NOFX").is_ok(); // stage 10e: drop the driver's reverb return (EVOL / EFFECT_ENABLE) for a dry render
			if let Some(att) = &b.attach {
				let mut att = att.clone();
				if nofx {
					att.fx.clear();
				}
				let att = &att;
				for o in &att.fx {
					eprintln!("snd9 attach fx {:?} idx {} {:#x}", o.reg, o.index, o.value);
				}
				audio.spu_attach(att.tick, Some(att), false);
				let mut c = SpuVoices::new(att.tick, false);
				c.attach(att);
				c.log_kons();
				check = Some(c);
				last_tick = att.tick;
				t0 = att.tick;
			}
			let Some(c) = check.as_mut() else { continue };
			// stream voices: the host event (voice, stream id), key-offs and the driver's stream start / next part / end ticks
			for e in b.ev.iter().filter(|e| matches!(e.w.reg, Reg::StreamSrc | Reg::MvolL | Reg::MvolR) || (e.w.reg == Reg::Koff && e.w.value != 0)) {
				keys.push_str(&format!("field {f} tick {} {:?} idx {} value {:#x}
", e.w.tick, e.w.reg, e.w.index, e.w.value));
			}
			for s in &b.streams {
				keys.push_str(&format!("field {f} stream {s:?}
"));
			}
			for e in b.ev.iter().filter(|e| e.w.reg == Reg::Kon && e.w.value != 0) {
				kons += 1;
				let vs: Vec<usize> = (0..24).filter(|i| e.w.value & (1 << i) != 0).map(|i| e.w.index as usize * 24 + i).collect();
				let at = 200 * (e.w.tick as i64 - t0 as i64);
				want_kons.extend(vs.iter().map(|&v| (at, v)));
				keys.push_str(&format!("field {f} tick {} core {} voices {:?}
", e.w.tick, e.w.index, vs));
			}
			// A field the oracle thread has not handed over yet (the first take) carries no driver data.
			if b.envx.is_empty() {
				assert!(b.ev.is_empty(), "events without driver state at field {f}");
				continue;
			}
			if f < 3 || b.tick < last_tick {
				eprintln!("snd9: field {f} attach {:?} batch tick {} last {last_tick} events {} envx {}", b.attach.as_ref().map(|a| a.tick), b.tick, b.ev.len(), b.envx.len());
			}
			assert!(b.tick >= last_tick, "driver tick went back");
			let samples = 200 * (b.tick - last_tick) as usize;
			last_tick = b.tick;
			if let Some(tv) = trace {
				for e in &b.ev {
					let hit = if e.w.reg.per_voice() { e.w.index as usize == tv } else { e.w.index as usize == tv / 24 && e.w.value & (1 << (tv % 24)) != 0 && matches!(e.w.reg, Reg::Kon | Reg::Koff) };
					if hit {
						eprintln!("trace f{f} tick {} {:?} {:#x}", e.w.tick, e.w.reg, e.w.value);
					}
				}
			}
			for e in b.ev.iter().filter(|e| matches!(e.w.reg, Reg::EvolL | Reg::EvolR | Reg::FxOn | Reg::Eea | Reg::FxAttr | Reg::FxClear)) {
				eprintln!("snd9 fx f{f} tick {} {:?} idx {} {:#x}", e.w.tick, e.w.reg, e.w.index, e.w.value);
			}
			let fx_off = |e: &crate::spumix::Ev| nofx && matches!(e.w.reg, Reg::EvolL | Reg::EvolR | Reg::FxOn);
			// RAC_SND9_ONLY=24,25: render only these voices (isolates the music from the sound effects in the WAV; the ENVX check
			// below still uses every event)
			let only: Option<Vec<usize>> = std::env::var("RAC_SND9_ONLY").ok().map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect());
			let keep = |e: &crate::spumix::Ev| -> Option<crate::spumix::Ev> {
				let Some(vs) = &only else { return Some(e.clone()) };
				let mut e = e.clone();
				if e.w.reg.per_voice() || e.w.reg == Reg::StreamSrc {
					return vs.contains(&(e.w.index as usize)).then_some(e);
				}
				if matches!(e.w.reg, Reg::Kon | Reg::Koff) {
					let mask: u32 = vs.iter().filter(|&&v| v / 24 == e.w.index as usize).fold(0, |m, &v| m | 1 << (v % 24));
					e.w.value &= mask;
					return (e.w.value != 0).then_some(e);
				}
				Some(e)
			};
			audio.spu_push(b.ev.iter().filter(|e| !fx_off(e)).filter_map(keep).collect(), Some(&b.core));
			let mut buf = vec![0f32; samples * 2];
			mix(&mut buf);
			wav.extend_from_slice(&buf);
			c.push(b.ev);
			c.render(samples);
			c.flush_due();
			let mine: Vec<u16> = (0..crate::spumix::VOICES).map(|v| c.envx(v)).collect();
			checked += 1;
			let want = b.envx.clone();
			if let Some(tv) = trace {
				if mine[tv] != 0 || want[tv] != 0 {
					eprintln!("trace f{f} tick {} envx mixer {} oracle {}", b.tick, mine[tv], want[tv]);
				}
			}
			if mine != want {
				bad += 1;
				if first_bad.is_empty() {
					let d: Vec<_> = (0..mine.len()).filter(|&v| mine[v] != want[v]).map(|v| (v, mine[v], want[v])).collect();
					first_bad = format!("field {f} tick {} (voice, mixer envx, oracle envx) {d:?}", b.tick);
				}
			}
			assert!(r.take_stop().is_none(), "oracle stopped at field {f}");
		}
		if let Some(c) = check.as_ref() {
			let (esa, eea, evol) = c.effect_regs();
			eprintln!("snd9: effect unit at the end: ESA {esa:#x} EEA {eea:#x} EVOL {evol:?}");
		}
		let got_kons = check.as_ref().map(|c| c.kon_log().to_vec()).unwrap_or_default();
		let kon_first_diff = want_kons.iter().zip(got_kons.iter()).position(|(a, b)| a != b);
		eprintln!("snd9: driver key-ons (voice starts) {}, mixer key-ons {}, first difference {kon_first_diff:?}", want_kons.len(), got_kons.len());
		assert!(want_kons == got_kons, "mixer key-ons differ from the driver's");
		write_wav(&out, 48000, 2, &wav);
		std::fs::write(&keys_path, &keys).unwrap();
		let peak = wav.iter().fold(0f32, |m, x| m.max(x.abs()));
		let rms = (wav.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / wav.len().max(1) as f64).sqrt();
		eprintln!("snd9: {n} fields, {} samples, key-ons {kons}, peak {peak:.4} rms {rms:.5}, envx checked {checked} fields, mismatching {bad}; first {first_bad}", wav.len() / 2);
		assert!(bad == 0, "mixer envelopes differ from the oracle's: {first_bad}");
		assert!(peak > 0.0, "silent");
	}

	/// RAC_SND_PAD: `from-to:buttons[:lx,ly]` (hex) entries separated by `;`, held for fields `from..to`.
	fn pad_script() -> Vec<(u32, u32, u16, Option<[u8; 2]>)> {
		std::env::var("RAC_SND_PAD")
			.unwrap_or_default()
			.split(';')
			.filter_map(|p| {
				let mut it = p.split(':');
				let (x, y) = it.next()?.split_once('-')?;
				let b = u16::from_str_radix(it.next()?, 16).ok()?;
				let st = it.next().and_then(|s| {
					let (lx, ly) = s.split_once(',')?;
					Some([u8::from_str_radix(lx, 16).ok()?, u8::from_str_radix(ly, 16).ok()?])
				});
				Some((x.parse().ok()?, y.parse().ok()?, b, st))
			})
			.collect()
	}
}
