//! Which retail disc runs (PAL SCES-50916 or USA SCUS-97199, both v1.00; ee_oracle::ver) and what differs for rac-rs.
//! The USA disc is the same program with its level code and data moved (tools/vermap.py writes the PAL -> USA maps) and the
//! game itself runs in NTSC mode (60 fields a second) when SYSTEM.CNF says so. PAL keeps every value it had.
//! Port extras that read or write retail RAM at PAL addresses (modern camera, aim, strafe, vendor screen, prompts, PC
//! options, widescreen hands, the front-end mirror) stay off on USA until their addresses go through the address tables.
use super::fmv_bridge::{Player, BOOT, FRONT};
use std::sync::atomic::{AtomicU8, Ordering};

/// The USA disc runs.
#[inline]
pub fn usa() -> bool {
	ee_oracle::ver::disc() == ee_oracle::ver::Disc::Usa
}

/// The port extras (modern camera, aim, strafe, vendor screen, prompts, PC options, widescreen hands, native effects,
/// front-end mirror) are off: the USA disc with RAC_USA_EXTRAS=0 only (a test switch; their addresses go through `va`).
pub fn extras_off() -> bool {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	usa() && *OFF.get_or_init(|| std::env::var("RAC_USA_EXTRAS").is_ok_and(|v| v == "0"))
}

/// One retail field in seconds (0.02 PAL; NTSC 1001 / 60000).
#[inline]
pub fn field_dt() -> f32 {
	if ee_oracle::ver::ntsc() { 1001.0 / 60000.0 } else { 0.02 }
}

/// Two fields (the catch-up cap of the viewer's step loops).
#[inline]
pub fn field_dt2() -> f32 {
	if ee_oracle::ver::ntsc() { 2.0 * 1001.0 / 60000.0 } else { 0.04 }
}

/// Logs the disc once at start-up.
pub fn announce() {
	eprintln!("retail: disc {} ({}), {} fields a second", ee_oracle::ver::disc().serial(), if ee_oracle::ver::ntsc() { "NTSC" } else { "PAL" }, if ee_oracle::ver::ntsc() { "59.94" } else { "50" });
}

/// The retail globals rac-rs reads in the boot / front-end flow, USA address of the PAL one. They sit at the head of the
/// overlay's small data (0x15ef00..), where every level and the front end have the same layout (vermap segments: one shift
/// for all scopes). Others are returned unchanged.
pub fn g(pal: u32) -> u32 {
	if !usa() {
		return pal;
	}
	match pal {
		0x15f5a0..=0x15f5e7 => pal + 0x20,
		_ => pal,
	}
}

/// The overlay in RAM (0..18 a level, `SCOPE_FRONT` the front end, `SCOPE_BOOT` the boot ELF's own overlay part); kept
/// for the USA disc only (`rescope`).
static SCOPE: AtomicU8 = AtomicU8::new(SCOPE_BOOT);
pub const SCOPE_FRONT: u8 = 19;
pub const SCOPE_BOOT: u8 = 20;

/// The overlay `va` maps for.
pub fn scope() -> u8 {
	SCOPE.load(Ordering::Relaxed)
}

/// Finds the overlay in RAM by its code signature (disc_hands::SCOPE_SIG: 8 code words at one address inside the code of
/// every overlay); keeps the last one while none matches (an overlay being loaded). USA only; cheap (one hash per scope).
pub fn rescope(ram: &[u8]) {
	// Every table build: the built-in hands under the omni hook entries may be another level's (omni/layer.rs).
	crate::omni::layer_rebuilt();
	if !usa() {
		return;
	}
	for &(sc, a, h) in super::disc_hands::SCOPE_SIG {
		let w = &ram[a as usize..a as usize + 32];
		let mut x = 0xcbf2_9ce4_8422_2325u64;
		for k in 0..8 {
			x = (x ^ u32::from_le_bytes(w[4 * k..4 * k + 4].try_into().unwrap()) as u64).wrapping_mul(0x100_0000_01b3);
		}
		if x == h {
			if SCOPE.swap(sc, Ordering::Relaxed) != sc && std::env::var_os("RAC_SCOPE_LOG").is_some() {
				eprintln!("retail: disc scope {sc}");
			}
			return;
		}
	}
}

/// An integer type `va` maps (the literal's own type stays).
pub trait Addr: Copy {
	fn to_u32(self) -> u32;
	fn from_u32(v: u32, like: Self) -> Self;
}
macro_rules! addr_impl {
	($($t:ty),*) => {$(
		impl Addr for $t {
			#[inline]
			fn to_u32(self) -> u32 {
				self as u32
			}
			#[inline]
			fn from_u32(v: u32, _: Self) -> Self {
				v as $t
			}
		}
	)*};
}
addr_impl!(u32, usize, u64, i32, i64);

/// The address of PAL retail address `pal` on the disc that runs: PAL returns it unchanged; USA maps it in the overlay in
/// RAM (`scope`) with the vermap address map (disc_map.rs). Core addresses (below 0x15ef00) are the same on both discs;
/// an address outside every mapped segment stays as it is.
#[inline]
pub fn va<T: Addr>(pal: T) -> T {
	if !usa() {
		return pal;
	}
	T::from_u32(va_usa(pal.to_u32()), pal)
}

fn va_usa(pal: u32) -> u32 {
	let segs: &[super::disc_map::Seg] = match scope() {
		SCOPE_FRONT => super::disc_map::FRONT,
		SCOPE_BOOT => super::disc_map::BOOTOV,
		n => super::disc_map::LEVELS.get(n as usize).copied().unwrap_or(super::disc_map::BOOTOV),
	};
	map_in(segs, pal)
}

/// PAL retail address `pal` in level `level`'s overlay on the disc that runs, whatever overlay is in RAM now (for the
/// native readers of a level's overlay image, `Level::sections[0]`). PAL returns it unchanged.
pub fn va_level(level: usize, pal: u32) -> u32 {
	if !usa() {
		return pal;
	}
	map_in(super::disc_map::LEVELS.get(level).copied().unwrap_or(super::disc_map::BOOTOV), pal)
}

/// USA: the `len` bytes at overlay address `va` (an address on the disc that runs) of a level overlay image
/// (`Level::sections[0]`: records of {load address, size, flags, entry} and `size` data bytes). The PAL readers keep their
/// fixed image bases.
pub fn ov_bytes(img: &[u8], va: usize, len: usize) -> Option<&[u8]> {
	let mut p = 0;
	while p + 16 <= img.len() {
		let w = |o: usize| u32::from_le_bytes(img[p + o..p + o + 4].try_into().unwrap()) as usize;
		let (load, size) = (w(0), w(4));
		if (load..load + size).contains(&va) {
			return img.get(p + 16 + va - load..)?.get(..len);
		}
		p += 16 + size;
	}
	None
}

fn map_in(segs: &[super::disc_map::Seg], pal: u32) -> u32 {
	let a = pal & 0x01ff_ffff;
	if !(0x15_ef00..0x200_0000).contains(&a) {
		return pal;
	}
	let i = segs.partition_point(|s| s.lo <= a);
	if i == 0 || a >= segs[i - 1].hi {
		return pal;
	}
	(pal & !0x01ff_ffff) | (a as i64 + segs[i - 1].d as i64) as u32
}

/// The PAL address of `a`, an address on the disc that runs (the inverse of `va`; for matching RAM pointers against PAL
/// constants in patterns). PAL returns it unchanged.
pub fn unva(a: u32) -> u32 {
	if !usa() {
		return a;
	}
	let x = a & 0x01ff_ffff;
	let segs: &[super::disc_map::Seg] = match scope() {
		SCOPE_FRONT => super::disc_map::FRONT,
		SCOPE_BOOT => super::disc_map::BOOTOV,
		n => super::disc_map::LEVELS.get(n as usize).copied().unwrap_or(super::disc_map::BOOTOV),
	};
	for s in segs {
		let (lo, hi) = ((s.lo as i64 + s.d as i64) as u32, (s.hi as i64 + s.d as i64) as u32);
		if (lo..hi).contains(&x) {
			return (a & !0x01ff_ffff) | (x as i64 - s.d as i64) as u32;
		}
	}
	a
}

/// The table of the disc that runs: `pal` (PAL) or its USA twin (tools/racrs_va.py writes the twins).
#[inline]
pub fn pick<T: ?Sized>(pal: &'static T, usa_twin: &'static T) -> &'static T {
	if usa() { usa_twin } else { pal }
}

/// The hands of `set` on the disc that runs: PAL returns `set`; USA the same hands at their USA entry with the hash of
/// the USA code (disc_hands.rs: functions that exist with relocations only), the others left out (the retail code runs).
pub fn hands(set: &'static [ee_oracle::rec::Hand]) -> &'static [ee_oracle::rec::Hand] {
	if !usa() {
		return set;
	}
	let map = super::disc_hands::HANDS;
	let mut v = Vec::new();
	for h in set {
		let i = map.partition_point(|r| (r.0, r.1) < (h.entry, h.hash));
		for r in map[i..].iter().take_while(|r| r.0 == h.entry && r.1 == h.hash) {
			v.push(ee_oracle::rec::Hand { entry: r.2, end: r.3, hash: r.4, est: h.est, f: h.f });
		}
	}
	Box::leak(v.into_boxed_slice())
}

/// Frame rate of the PSS movie at `lba` on the disc (MPEG sequence header frame_rate_code 3 = 25, 5 = 29.97, 4 = 30 fps).
/// PAL keeps 25 (the PAL game asks for its 25 fps movies only).
pub fn movie_fps(lba: u32) -> f32 {
	if !usa() {
		return 25.0;
	}
	use std::io::{Read, Seek, SeekFrom};
	let iso = std::env::var("ORACLE_ISO").unwrap_or_default();
	let rate = (|| {
		let mut f = std::fs::File::open(&iso).ok()?;
		f.seek(SeekFrom::Start(lba as u64 * 2048)).ok()?;
		let mut b = vec![0u8; 0x10000];
		f.read_exact(&mut b).ok()?;
		let i = b.windows(4).position(|w| w == [0, 0, 1, 0xb3])?;
		b.get(i + 7).map(|x| x & 15)
	})();
	match rate {
		Some(3) => 25.0,
		Some(4) => 30.0,
		Some(5) => 30000.0 / 1001.0,
		_ => 30000.0 / 1001.0,
	}
}

/// fmv_bridge::PLAYERS of the USA disc: the same 42-word PSS player per level overlay, frontbin and boot ELF, at its USA
/// address with the hash of its USA code words. Generated by tools/players_usa.py (vermap function map; each PAL hash
/// re-checked against the PAL disc first).
pub const PLAYERS_USA: [Player; 21] = [
	Player { level: 0, name: "veldin_tutorial", entry: 0x2edd28, end: 0x2eddd0, hash: 0x9c1c_6907_3511_5802 },
	Player { level: 1, name: "novalis", entry: 0x31a260, end: 0x31a308, hash: 0xd6a4_14ca_fc97_5bb0 },
	Player { level: 2, name: "aridia", entry: 0x2fc988, end: 0x2fca30, hash: 0x28e4_f49b_05b5_ec5a },
	Player { level: 3, name: "kerwan", entry: 0x2eda98, end: 0x2edb40, hash: 0xbcea_4992_0adf_213e },
	Player { level: 4, name: "eudora", entry: 0x2f1ac0, end: 0x2f1b68, hash: 0xae83_6498_412f_4b88 },
	Player { level: 5, name: "rilgar", entry: 0x32bdb0, end: 0x32be58, hash: 0x09fe_00c7_4821_0404 },
	Player { level: 6, name: "nebula_g34", entry: 0x3169f0, end: 0x316a98, hash: 0x62d8_f375_e849_6eb4 },
	Player { level: 7, name: "umbris", entry: 0x3299d8, end: 0x329a80, hash: 0xdf61_f79c_fd12_a1ae },
	Player { level: 8, name: "batalia", entry: 0x317368, end: 0x317410, hash: 0x172d_251e_2876_fff2 },
	Player { level: 9, name: "gaspar", entry: 0x315de8, end: 0x315e90, hash: 0xa5fb_39d3_7a81_5652 },
	Player { level: 10, name: "orxon", entry: 0x2f6fb8, end: 0x2f7060, hash: 0x8a92_80de_6590_c386 },
	Player { level: 11, name: "pokitaru", entry: 0x328fa0, end: 0x329048, hash: 0x2fcb_e50c_9dd6_b760 },
	Player { level: 12, name: "hoven", entry: 0x316d68, end: 0x316e10, hash: 0xbc6a_0e57_b632_1172 },
	Player { level: 13, name: "oltanis_orbit", entry: 0x318700, end: 0x3187a8, hash: 0xc076_2877_7335_2bf8 },
	Player { level: 14, name: "oltanis", entry: 0x317250, end: 0x3172f8, hash: 0x8cc4_381a_8b9d_7d6c },
	Player { level: 15, name: "quartu", entry: 0x2f9440, end: 0x2f94e8, hash: 0xa130_c48d_4abc_51e8 },
	Player { level: 16, name: "kalebo_iii", entry: 0x2f5ed8, end: 0x2f5f80, hash: 0xe3d1_09f9_c041_536e },
	Player { level: 17, name: "veldin_orbit", entry: 0x300f20, end: 0x300fc8, hash: 0x6171_b2e6_f4c2_9bc0 },
	Player { level: 18, name: "veldin_finale", entry: 0x307118, end: 0x3071c0, hash: 0xd8cc_1537_ce5a_559e },
	Player { level: FRONT, name: "frontbin", entry: 0x239d70, end: 0x239e18, hash: 0x356d_e756_ae3b_5214 },
	Player { level: BOOT, name: "boot", entry: 0x23a3b8, end: 0x23a460, hash: 0x00ae_5136_0805_9d46 },
];

/// The movie hooks of the USA players (as fmv_bridge's HOOKS for PAL).
pub static HOOKS_USA: [ee_oracle::rec::Hand; PLAYERS_USA.len()] = {
	const NONE: ee_oracle::rec::Hand = ee_oracle::rec::Hand { entry: 0, end: 0, hash: 0, est: 0, f: super::fmv_bridge::movie_call };
	let mut v = [NONE; PLAYERS_USA.len()];
	let mut i = 0;
	while i < PLAYERS_USA.len() {
		v[i] = ee_oracle::rec::Hand { entry: PLAYERS_USA[i].entry, end: PLAYERS_USA[i].end, hash: PLAYERS_USA[i].hash, est: 0, f: super::fmv_bridge::movie_call };
		i += 1;
	}
	v
};

#[cfg(test)]
mod tests {
	use super::*;

	/// PAL values are the ones rac-rs always used.
	#[test]
	fn pal_unchanged() {
		ee_oracle::ver::set_disc(ee_oracle::ver::Disc::Pal);
		ee_oracle::ver::set_ntsc(false);
		assert_eq!(field_dt().to_bits(), 0.02f32.to_bits());
		assert_eq!(field_dt2().to_bits(), 0.04f32.to_bits());
		assert_eq!(g(0x15f5e4), 0x15f5e4);
		assert_eq!(movie_fps(0), 25.0);
	}

	/// The USA globals of the front-end flow (vermap: +0x20 in every scope).
	#[test]
	fn usa_globals() {
		ee_oracle::ver::set_disc(ee_oracle::ver::Disc::Usa);
		assert_eq!((g(0x15f5e4), g(0x15f5a0), g(0x15f5b8), g(0x15f5a4)), (0x15f604, 0x15f5c0, 0x15f5d8, 0x15f5c4));
		ee_oracle::ver::set_disc(ee_oracle::ver::Disc::Pal);
	}
}
