//! [Fix #62] The 989snd driver's random stream on a cold boot.
//!
//! `worker/machine.rs` `register_hooks` attaches the sound mixer (`spumix::attach_state`) before the first field, so on a cold
//! boot `Snd989::ensure` builds the driver while the IOP RAM is still empty: `rng_from_iop` finds no R250 table and the driver
//! falls back to an all-zero one, which XORs to 0 forever. Every random grain then draws 0 (Kerwan sound 47, Ratchet's hit-3
//! voice: never the 1-in-3 silence, the variants only alternate, always the lowest random bend). Retail's table is the module's
//! .data, never reseeded [C decomp/notes/sound_grains.md "RNG"], and a cold boot loads the module fresh from the disc. So right
//! after the attach, before the first field (no draw made yet), the driver gets the initial state of the player's own disc
//! module (`Rng250::from_irx`; the table is read at run time, never shipped). Nothing is taken from IOP RAM: the boot's copy of
//! the module only reaches it later (found there at driver field 2 in every measured cold run, PAL and USA) and a pattern match
//! there could hit another copy. Savestate runs adopt the live table and are left alone.

use ee_oracle::snd989::irx::{Irx, MODULE_989SND};
use ee_oracle::snd989::rng::Rng250;

/// ToC slot (sector, sector count) of the IOP module pack `globals/irx.bin` (WAD-compressed; 989snd is its last module). Same
/// slot on both discs [C: PAL sector 14344, USA 14345, 162 sectors each]; the module name is checked before use.
const IRX_PACK: usize = 0x12c0;

/// The initial R250 state (table and index word of the .data) of the 989snd module on the disc at `iso`.
pub fn disc_rng(iso: &str) -> Result<Rng250, String> {
	let mut d = crate::iso::Disc::open(std::path::Path::new(iso)).map_err(|e| e.to_string())?;
	let t = crate::toc::read_toc(&mut d).map_err(|e| e.to_string())?;
	let (sector, sectors) = (crate::bytes::u32_at(&t, IRX_PACK)? as u64, crate::bytes::u32_at(&t, IRX_PACK + 4)? as u64);
	let raw = d.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let pack = crate::wad::decompress(&raw).map_err(|e| e.to_string())?;
	let elf = Irx::find_in_blob(&pack, MODULE_989SND).ok_or("no 989snd module in the ToC 0x12c0 pack")?;
	Rng250::from_irx(&Irx::parse(elf)?)
}

/// What `seed_fresh` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Seed {
	/// The driver had the zero fallback; it has the module's state now.
	Zero,
	/// The driver had another table (not the disc module's); it has the module's state now.
	Other,
	/// The driver already had exactly the module's state.
	Same,
	/// Refused: the driver already ran (clock or index not 0), so the draws its table made are not known.
	Ran,
}

/// Gives a driver that has not run yet (clock `vb` 0, R250 index 0: no draw made, the driver draws only inside a field) the
/// module's initial state `r`.
pub fn seed_fresh(rng: &mut Rng250, vb: u32, r: Rng250) -> Seed {
	let (t, ix) = rng.state();
	if vb != 0 || ix != 0 {
		return Seed::Ran;
	}
	let s = if *rng == r {
		Seed::Same
	} else if t.iter().all(|&v| v == 0) {
		Seed::Zero
	} else {
		Seed::Other
	};
	*rng = r;
	s
}

/// Cold boots only, once, right after `spumix::attach_state` made the driver (`worker/machine.rs register_hooks`, before the
/// first field). `iso` = the disc the machine boots from. One log line says what happened.
pub fn seed_cold_driver(m: &mut ee_oracle::Machine, iso: &str) {
	let c = &mut **m.cpu;
	if c.cold.is_none() {
		return;
	}
	let Some(i) = c.snd.inner.as_mut() else {
		eprintln!("snd989: cold boot: no sound driver at attach; the R250 table was not seeded (#62)");
		return;
	};
	let r = match disc_rng(iso) {
		Ok(r) => r,
		Err(e) => {
			eprintln!("snd989: cold boot: no R250 table from the disc ({e}); random sounds draw 0 (#62)");
			return;
		}
	};
	match seed_fresh(&mut i.drv.core.rng, i.vb, r) {
		Seed::Ran => eprintln!("snd989: cold boot: the driver already ran (field {}); R250 seed refused (#62)", i.vb),
		s => eprintln!("snd989: cold boot: R250 table taken from the disc's 989snd module before the first field ({s:?})"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ee_oracle::snd989::rng::LEN;

	const PAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
	const USA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso");

	fn table(seed: u16) -> Rng250 {
		Rng250::from_state(std::array::from_fn(|k| seed.wrapping_add(k as u16) | 1), 0).unwrap()
	}

	#[test]
	fn a_fresh_driver_takes_the_module_state_one_that_ran_is_refused() {
		let m = table(7);
		let mut z = Rng250::from_state([0; LEN], 0).unwrap();
		assert_eq!(seed_fresh(&mut z, 0, m.clone()), Seed::Zero);
		assert_eq!(z, m);
		assert_eq!(seed_fresh(&mut z, 0, m.clone()), Seed::Same);
		let mut o = table(9);
		assert_eq!(seed_fresh(&mut o, 0, m.clone()), Seed::Other);
		assert_eq!(o, m);
		// a driver that ran a field, or whose zero table already made draws, keeps what it has
		let mut ran = Rng250::from_state([0; LEN], 0).unwrap();
		assert_eq!(seed_fresh(&mut ran, 1, m.clone()), Seed::Ran);
		assert_eq!(ran.state().0, &[0; LEN]);
		let mut drew = Rng250::from_state([0; LEN], 3).unwrap();
		assert_eq!(seed_fresh(&mut drew, 0, m), Seed::Ran);
		assert_eq!(drew.state(), (&[0; LEN], 3));
	}

	/// The disc module's initial stream (sound_grains.md "[O] 2.": 1173, 53403, 52352, 35341, 27497, 54011); USA has the same
	/// module state. Skipped without the discs.
	#[test]
	fn disc_module_state_pal_and_usa() {
		let first = |r: &Rng250| {
			let mut r = r.clone();
			[(); 6].map(|_| r.next())
		};
		if std::path::Path::new(PAL).exists() {
			let p = disc_rng(PAL).expect("PAL disc 989snd module");
			assert_eq!(p.state().1, 0);
			assert_eq!(first(&p), [1173, 53403, 52352, 35341, 27497, 54011]);
			if std::path::Path::new(USA).exists() {
				assert_eq!(disc_rng(USA).expect("USA disc 989snd module"), p);
			}
		}
		assert!(disc_rng("no_such_disc.iso").is_err());
	}
}
