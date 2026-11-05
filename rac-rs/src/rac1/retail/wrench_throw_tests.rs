use super::*;
use ee_oracle::mem::Mem;

/// Classic is the default; the row's setter applies at once, writes the file only on a change, and a saved Moving comes back.
#[test]
fn the_option_defaults_to_classic_and_persists() {
	let _g = crate::retail::pcopts::globals_guard();
	let mut opts = PcOptions::new();
	init(&opts);
	assert!(!moving(), "default: Classic (retail)");
	assert!(!set_from_ui(&mut opts, false), "no change, no save");
	assert!(set_from_ui(&mut opts, true));
	assert!(moving());
	assert!(!set_from_ui(&mut opts, true));
	let saved = opts.to_text();
	assert_eq!(PcOptions::parse(&saved).extra_get(KEY), Some("moving"));
	init(&PcOptions::new());
	assert!(!moving());
	init(&PcOptions::parse(&saved));
	assert!(moving(), "the file brings Moving back");
	assert!(set_from_ui(&mut opts, false));
	assert_eq!(PcOptions::parse(&opts.to_text()).extra_get(KEY), Some("classic"));
	init(&PcOptions::new());
}

/// A throw in flight past the release with the stick pushed: the RAM state `exit_wanted` looks at.
fn throwing() -> Cpu {
	let mut c = Cpu::new(Mem::new());
	let w = 0x0020_0000u32;
	c.mem.wr32(HERO_STATE, THROW);
	c.mem.wr32(WRENCH_MOBY, w);
	c.mem.wr8(w + 0x20, 10);
	c.mem.wr32(ANIM_T, 40f32.to_bits());
	c.mem.wr32(STICK_AMT, 1f32.to_bits());
	c
}

/// The exit is wanted only in the throw, with the wrench out, past the release and with the stick pushed or Cross held.
#[test]
fn the_throw_ends_only_when_wanted() {
	assert!(exit_wanted(&mut throwing()));
	let mut c = throwing();
	c.mem.wr32(STICK_AMT, 0.1f32.to_bits());
	assert!(!exit_wanted(&mut c), "stick at rest: Ratchet keeps retail's throw pose");
	c.mem.wr32(PAD_HELD, CROSS);
	assert!(exit_wanted(&mut c), "Cross held: the throw ends (the jump follows in state 0)");
	let mut c = throwing();
	c.mem.wr32(HERO_STATE, 0x13);
	assert!(!exit_wanted(&mut c), "a swing is not the throw");
	let mut c = throwing();
	c.mem.wr8(0x0020_0020, 0);
	assert!(!exit_wanted(&mut c), "wrench still in the hand");
	let mut c = throwing();
	c.mem.wr32(WRENCH_MOBY, 0);
	assert!(!exit_wanted(&mut c), "no wrench moby");
	let mut c = throwing();
	c.mem.wr32(ANIM_T, 34f32.to_bits());
	assert!(!exit_wanted(&mut c), "just after the release: the follow-through plays");
}

/// Classic: the hand declines and writes nothing, whatever the RAM says.
#[test]
fn classic_declines_and_writes_nothing() {
	let _g = crate::retail::pcopts::globals_guard();
	init(&PcOptions::new());
	let mut c = throwing();
	let before = c.mem.ram.clone();
	let regs = (c.r, c.f, c.pc);
	assert!(!slew(&mut c));
	assert!(c.mem.ram == before, "no RAM byte changed");
	assert!((c.r, c.f, c.pc) == regs, "no register changed");
}

/// The 20 code words of Novalis' hero_speed_slew (0x2321b0); a twin with other `jal` targets has the same masked hash, other code not.
#[test]
fn the_spec_matches_the_slew_of_every_level() {
	const NOVALIS: [u32; 20] = [
		0x27bdfff0, 0x3c020014, 0x7fbf0000, 0x2442f350, 0x46006046, 0xc44c0190, 0xc4400194, 0x460c0034, 0x00000000, 0x45000005, 0x24440194, 0x0c09c0b0,
		0x46000b46, 0x10000004, 0x7bbf0000, 0x0c09c0b0, 0x24440194, 0x7bbf0000, 0x03e00008, 0x27bd0010,
	];
	let r = reloc();
	assert_eq!((r.entry, r.len), (0x2321b0, 4 * NOVALIS.len() as u32));
	let hash = |w: &[u32; 20]| r.masked_hash_by(0x2321b0, |a| w[((a - 0x2321b0) / 4) as usize]);
	assert_eq!(hash(&NOVALIS), r.mhash);
	let mut twin = NOVALIS;
	twin[11] = 0x0c099999;
	twin[15] = 0x0c099999;
	assert_eq!(hash(&twin), r.mhash, "another level: only the jal targets differ");
	let mut other = NOVALIS;
	other[5] = 0xc44c0198;
	assert_ne!(hash(&other), r.mhash, "another field: another function");
}

/// Every level's group-stop twin lies at its table address, the PAL one being the address the decomp names (FUN_00227e70 on Novalis).
#[test]
fn the_group_stop_tables_are_twinned() {
	assert_eq!(GROUP_STOP[1], 0x227e70);
	assert_eq!(GROUP_STOP_USA.len(), GROUP_STOP.len());
	assert!(GROUP_STOP.iter().zip(&GROUP_STOP_USA).all(|(p, u)| p != u && (p & 3) == 0 && (u & 3) == 0));
}

fn iso(rels: &[&str]) -> Option<std::path::PathBuf> {
	std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().flat_map(|a| rels.iter().map(move |r| a.join(r))).find(|p| p.is_file())
}

/// The overlay images of the 19 levels of a disc.
fn overlays(path: &std::path::Path) -> Vec<Vec<u8>> {
	let mut disc = crate::iso::Disc::open(path).expect("open the ISO");
	let levels = crate::toc::read_levels(&mut disc).expect("read the level table");
	(0..19).map(|n| crate::level::Level::load(&mut disc, &levels[n]).ok().and_then(|l| l.sections.first().cloned().flatten()).expect("overlay image")).collect()
}

/// The addresses of every copy of the slew in an overlay image (its records: load address, size, 8 bytes, data).
fn slew_copies(img: &[u8]) -> Vec<u32> {
	let r = reloc();
	let (mut out, mut p) = (Vec::new(), 0usize);
	while p + 16 <= img.len() {
		let w = |o: usize| u32::from_le_bytes(img[o..o + 4].try_into().unwrap());
		let (load, size) = (w(p), w(p + 4) as usize);
		let data = &img[p + 16..(p + 16 + size).min(img.len())];
		let word = |a: u32| data.get((a - load) as usize..(a - load) as usize + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
		for off in (0..data.len().saturating_sub(r.len as usize)).step_by(4) {
			let a = load + off as u32;
			if word(a) == 0x27bdfff0 && r.masked_hash_by(a, word) == r.mhash {
				out.push(a);
			}
		}
		p += 16 + size;
	}
	out
}

/// With both ISOs on disk: exactly one slew copy in each of the 19 overlays of each disc (PAL at the decomp's twin addresses), and
/// each USA group-stop address holds the PAL function (code words equal, `jal` targets apart). Prints SKIPPED without the ISOs.
#[test]
fn both_discs_have_one_slew_per_level() {
	let pal = iso(&["Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"]);
	let usa = iso(&["Ratchet & Clank (USA) (En,Fr,De,Es,It).iso", "Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"]);
	let (Some(pal), Some(usa)) = (pal, usa) else {
		eprintln!("both_discs_have_one_slew_per_level: SKIPPED: ISO missing");
		return;
	};
	const PAL_SLEW: [u32; 19] = [
		0x211bd0, 0x2321b0, 0x220a90, 0x209fa8, 0x205130, 0x23f718, 0x228a08, 0x2399b8, 0x223498, 0x231830, 0x2051b8, 0x241bb8, 0x233270, 0x21d638, 0x2236a0, 0x209b60,
		0x20f328, 0x20e960, 0x219848,
	];
	let (po, uo) = (overlays(&pal), overlays(&usa));
	let code = |img: &[u8], a: u32| super::super::disc::ov_bytes(img, a as usize, 48).map(|b| b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<u32>>());
	let same = |p: &[u32], u: &[u32]| p.iter().zip(u).all(|(&p, &u)| if p >> 26 == 3 { u >> 26 == 3 } else { p == u });
	for l in 0..19 {
		assert_eq!(slew_copies(&po[l]), vec![PAL_SLEW[l]], "PAL level {l}");
		assert_eq!(slew_copies(&uo[l]).len(), 1, "USA level {l}");
		let (p, u) = (code(&po[l], GROUP_STOP[l]).expect("PAL words"), code(&uo[l], GROUP_STOP_USA[l]).expect("USA words"));
		assert_eq!(p[0], 0x27bdff70, "level {l}: the PAL group stop starts its frame");
		assert!(same(&p, &u), "level {l}: USA {:#x} is not the group stop: {u:x?}", GROUP_STOP_USA[l]);
	}
}
