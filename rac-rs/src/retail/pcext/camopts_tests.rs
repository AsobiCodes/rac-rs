//! The Camera Options rows' tests (Lock-on, Camera speed, the camera rows of every level; moved out of pcext.rs 2026-10-23, lane ingame: a pure move, `use super::*` reaches the same private items) and the two layouts of the Options list (9 retail rows, 10 with Mods).
use super::*;

/// The tests below set process-wide state (CAMSPEED, CAMSHOW ..): they take turns.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
	TURN.lock().unwrap_or_else(|e| e.into_inner())
}

fn reset_statics() {
	CAMSPEED.store(0, Ordering::Relaxed);
	CAMSHOW.store(50, Ordering::Relaxed);
	LOCKON_MODE.store(0, Ordering::Relaxed);
	CAMSEL_SEEN.store(0xff, Ordering::Relaxed);
	RATE_AT.store(0, Ordering::Relaxed);
	RATE_SET.store(0, Ordering::Relaxed);
	mousespeed::reset_statics();
}

/// The retail Camera Options records at the table: Left / Right, Up / Down, Slow / Medium / Fast.
const RETAIL_RECS: [[u32; 6]; 3] = [[0x4fa2, 0x15ede0, 0x4fa5, 0x4fa4, 0, 0], [0x4fa3, 0x15eddc, 0x4fa5, 0x4fa4, 0, 0], [0x521b, 0x15ede4, 0x521a, 0x5219, 0x5218, 0]];

fn rec(mem: &mut Mem, i: u32) -> [u32; 6] {
	let mut r = [0u32; 6];
	for (k, w) in r.iter_mut().enumerate() {
		*w = mem.rd32(TAB + 24 * i + 4 * k as u32);
	}
	r
}

/// The row table of every level of both discs: the retail records, the Camera speed row in place of Slow / Medium / Fast, the
/// Mouse speed row, the three PC rows, Lock-on, and the zero record; patched once; 9 records (0xd8 bytes) at TAB, inside the
/// camopts block.
#[test]
fn camera_rows_of_every_level_pal_and_usa() {
	assert_eq!(9 * 24, 0xd8, "8 rows and the zero record");
	assert!(TAB >= BASE + 0x1000 + 0x40 * (ID_MPCT - ID_LOCKON + 1) && TAB + 9 * 24 <= CO_POOL_END, "the copy sits after the text slots, inside the camopts block");
	for (name, tables) in [("PAL", &TABLES), ("USA", &TABLES_USA)] {
		for (lv, &t) in tables.iter().enumerate() {
			let mut mem = Mem::new();
			for (i, r) in RETAIL_RECS.iter().enumerate() {
				for (k, w) in r.iter().enumerate() {
					mem.wr32(t + 24 * i as u32 + 4 * k as u32, *w);
				}
			}
			assert!(!patch_rows_at(&mut mem, t), "{name} level {lv}: no item pointer yet");
			mem.wr32(t + PTR_OFF, t);
			assert!(patch_rows_at(&mut mem, t), "{name} level {lv}");
			assert_eq!(mem.rd32(t + PTR_OFF), TAB, "{name} level {lv}: item repointed");
			assert_eq!(rec(&mut mem, 0), RETAIL_RECS[0], "{name} level {lv}: Left / Right as retail");
			assert_eq!(rec(&mut mem, 1), RETAIL_RECS[1], "{name} level {lv}: Up / Down as retail");
			assert_eq!(rec(&mut mem, 2), [0x521b, CAMSEL, ID_PCT, ID_PCT, 0, 0], "{name} level {lv}: camera speed row (retail's label)");
			assert_eq!(rec(&mut mem, 3), [ID_MOUSE, MOUSESEL, ID_MPCT, ID_MPCT, 0, 0], "{name} level {lv}: mouse speed row right below camera speed");
			assert_eq!(rec(&mut mem, 4), [ID_CAMERA, CAM, ID_CLASSIC, ID_STATIC, ID_SMOOTH, 0], "{name} level {lv}");
			assert_eq!(rec(&mut mem, 5), [ID_STRAFE, STRAFE, ID_OFF, ID_ON, 0, 0], "{name} level {lv}");
			assert_eq!(rec(&mut mem, 6), [ID_BUYALL, BUYALL, ID_OFF, ID_ON, 0, 0], "{name} level {lv}");
			assert_eq!(rec(&mut mem, 7), [ID_LOCKON, LOCKON, ID_AUTO, ID_ON, ID_OFF, 0], "{name} level {lv}: lock-on row");
			assert_eq!(rec(&mut mem, 8), [0; 6], "{name} level {lv}: zero record");
			// The retail table itself is left alone, and a patched item is not patched again.
			assert_eq!(mem.rd32(t + 4), 0x15ede0);
			assert!(!patch_rows_at(&mut mem, t), "{name} level {lv}: idempotent");
		}
	}
	// The bytes and the text ids do not collide with the other rows or the pools.
	let rows = [CAM, STRAFE, BUYALL, LOCKON, CAMSEL, MOUSESEL].into_iter().chain(GROWS);
	let mut v: Vec<u32> = rows.collect();
	v.sort();
	v.dedup();
	assert_eq!(v.len(), 6 + GROWS.len(), "row bytes are distinct");
	assert!((ID_LOCKON..=ID_MPCT).all(|id| !NEW_IDS.contains(&id)));
	assert!(CO_POOL >= POOL2 + 0x100 && CO_POOL + 0x40 * (ID_MPCT - ID_LOCKON + 1) <= TAB && TAB + 9 * 24 <= CO_POOL_END, "text slots, then the row copy, inside the block");
}

#[test]
fn camera_speed_steps_and_ends() {
	use Step::*;
	assert_eq!(camspeed_step(50, Right), 60);
	assert_eq!(camspeed_step(50, Left), 40);
	assert_eq!(camspeed_step(10, Left), 10, "Left stops at 10");
	assert_eq!(camspeed_step(200, Right), 200, "Right stops at 200");
	assert_eq!(camspeed_step(190, Right), 200);
	assert_eq!(camspeed_step(50, Cross), 60);
	assert_eq!(camspeed_step(200, Cross), 10, "Cross wraps");
	assert_eq!(camspeed_step(10, Cross), 20);
	// 20 presses of Cross visit every value once and come back.
	let mut v = 10;
	let mut seen = Vec::new();
	for _ in 0..20 {
		seen.push(v);
		v = camspeed_step(v, Cross);
	}
	assert_eq!((v, seen.len(), seen.first(), seen.last()), (10, 20, Some(&10), Some(&200)));
	// Retail's three presets show as 30 / 40 / 50 and a percent maps back to the nearest one.
	assert_eq!([0u8, 1, 2, 7].map(preset_pct), [30, 40, 50, 50]);
	assert_eq!([10u8, 30, 35, 36, 40, 45, 46, 50, 200].map(preset_byte), [0, 0, 0, 1, 1, 1, 2, 2, 2]);
	for b in 0..3u8 {
		assert_eq!(preset_byte(preset_pct(b)), b, "the shown value of a preset maps back to it");
	}
	// Auto-repeat: the press, then from the 13th field every 4th.
	let due: Vec<u32> = (0..30).filter(|&h| repeat_due(h)).collect();
	assert_eq!(due, [1, 13, 17, 21, 25, 29]);
}

#[test]
fn camera_speed_rate_maths() {
	assert_eq!(camspeed_rate_bits(50), 0x3ce4_c388, "50 % is retail's Fast exactly");
	assert_eq!(f32::from_bits(camspeed_rate_bits(100)), 2.0 * f32::from_bits(0x3ce4_c388), "100 % doubles the turn per tick");
	assert_eq!(f32::from_bits(camspeed_rate_bits(10)), f32::from_bits(0x3ce4_c388) * (10.0 / 50.0));
	let rates: Vec<f32> = (1..=20).map(|k| f32::from_bits(camspeed_rate_bits(10 * k))).collect();
	assert!(rates.windows(2).all(|w| w[1] > w[0]), "monotonic");
	assert_eq!(camspeed_factor(0), 1.0, "no key: the Modern camera is untouched");
	assert_eq!(camspeed_factor(50), 1.0);
	assert_eq!(camspeed_factor(100), 2.0);
	assert_eq!(camspeed_factor(10), 0.2);
	// Retail's three rates are 1.0, 1.3 and 1.6 degrees.
	let deg: Vec<f32> = RATE_RETAIL.iter().map(|&b| f32::from_bits(b).to_degrees()).collect();
	assert!((deg[0] - 1.0).abs() < 1e-3 && (deg[1] - 1.3).abs() < 1e-3 && (deg[2] - 1.6).abs() < 1e-3, "{deg:?}");
}

#[test]
fn lockon_truth_table() {
	// Auto follows the device (pad: on, keyboard: off); On and Off do not care.
	assert!(lockon_decide(0, false));
	assert!(!lockon_decide(0, true));
	assert!(lockon_decide(1, true) && lockon_decide(1, false));
	assert!(!lockon_decide(2, true) && !lockon_decide(2, false));
	assert!(lockon_decide(9, false) && !lockon_decide(9, true), "a stray byte acts as Auto");
}

#[test]
fn camera_option_texts() {
	for lang in 0..5 {
		for id in ID_LOCKON..=ID_MPCT {
			let s = camopt_string(id, lang, 70).unwrap();
			assert!(!s.is_empty() && s.len() < 0x3f && s.len() <= 21, "{id:#x} {lang}: fits the row");
			assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || [0x84, 0x85, 0x9c].contains(&b)), "printable ASCII (and the a-acute 0x84, a-grave 0x85, o-acute 0x9c) in the font: {s:?}");
		}
	}
	assert_eq!(camopt_string(ID_LOCKON, 0, 0).unwrap(), b"Lock-on");
	assert_eq!(camopt_string(ID_LOCKON, 2, 0).unwrap(), b"Zielerfassung");
	assert_eq!(camopt_string(ID_AUTO, 3, 0).unwrap(), b"Auto");
	assert_eq!(camopt_string(ID_PCT, 1, 50).unwrap(), b"< 50 >");
	assert_eq!(camopt_string(ID_PCT, 4, 200).unwrap(), b"< 200 >");
	assert_eq!(camopt_string(ID_STATIC, 0, 0).unwrap(), b"Static");
	assert_eq!(camopt_string(ID_SMOOTH, 0, 0).unwrap(), b"Smooth");
	assert_eq!(camopt_string(ID_STATIC, 1, 0).unwrap(), b"Statique");
	assert_eq!(camopt_string(ID_SMOOTH, 1, 0).unwrap(), b"Fluide");
	assert_eq!(camopt_string(ID_STATIC, 2, 0).unwrap(), b"Statisch");
	assert_eq!(camopt_string(ID_SMOOTH, 2, 0).unwrap(), b"Weich");
	assert_eq!(camopt_string(ID_STATIC, 3, 0).unwrap(), b"Est\x84tica");
	assert_eq!(camopt_string(ID_SMOOTH, 3, 0).unwrap(), b"Suave");
	assert_eq!(camopt_string(ID_STATIC, 4, 0).unwrap(), b"Statica");
	assert_eq!(camopt_string(ID_SMOOTH, 4, 0).unwrap(), b"Fluida");
	// The Mouse speed row: label in the five languages (font bytes), value text from the percent.
	let label = |lang| camopt_string(ID_MOUSE, lang, 0).unwrap();
	assert_eq!([0, 1, 2, 3, 4].map(label), [b"Mouse speed".to_vec(), b"Vitesse souris".to_vec(), b"Mausgeschwindigkeit".to_vec(), b"Velocidad del rat\x9cn".to_vec(), b"Velocit\x85 mouse".to_vec()]);
	assert_eq!(camopt_string(ID_MPCT, 0, 50).unwrap(), b"< 50 >");
	assert_eq!(camopt_string(ID_MPCT, 2, 200).unwrap(), b"< 200 >");
	assert_eq!(camopt_string(ID_MPCT + 1, 0, 50), None);
	assert_eq!(camopt_string(ID_LOCKON - 1, 0, 50), None);
}

/// The Mouse speed row's steps use the same grid as Camera speed (10..200 by 10, Cross wraps, Left / Right stop at the ends).
#[test]
fn mouse_speed_steps_like_camera_speed() {
	assert_eq!(MOUSE_ROW, SPEED_ROW + 1, "right below Camera speed");
	assert!(matches!(step_of(true, [false, false]), Step::Cross));
	assert!(matches!(step_of(false, [true, false]), Step::Left));
	assert!(matches!(step_of(false, [false, true]), Step::Right));
	let mut v = 50;
	for want in [60, 70, 80] {
		v = camspeed_step(v, step_of(false, [false, true]));
		assert_eq!(v, want);
	}
	assert_eq!(camspeed_step(10, step_of(false, [true, false])), 10, "Left stops at 10");
	assert_eq!(camspeed_step(200, step_of(false, [false, true])), 200, "Right stops at 200");
	assert_eq!(camspeed_step(200, step_of(true, [false, false])), 10, "Cross wraps");
	assert_eq!(camspeed_step(50, step_of(true, [false, false])), 60);
}

/// No `mousespeed` key: the row shows 50, nothing is written (no option, no file, no RAM byte), the mouse factor is exactly 1.0.
/// The two speeds are independent: setting one leaves the other's factor alone.
#[test]
fn mouse_speed_no_key_no_write_and_independent_of_camera_speed() {
	let _t = turn();
	let _turn = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
	reset_statics();
	let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
	let s: &mut St = &mut guard;
	let file = std::env::temp_dir().join(format!("pcext_mousespeed_{}.txt", std::process::id()));
	let _ = std::fs::remove_file(&file);
	s.opts = PcOptions::default();
	s.path = Some(file.clone());
	let mut mem = Mem::new();
	camopts_setup(&mut mem, s);
	assert_eq!(mousespeed_scale().to_bits(), 1.0f32.to_bits(), "no key: exactly 1.0");
	assert_eq!(mousespeed::shown(), 50);
	// Fields with nothing pressed (the cursor is not on the row either): nothing is written.
	let before = mem.ram.clone();
	mousespeed::field(&mut mem, 4, [false, false], s);
	mousespeed::field(&mut mem, 4, [true, true], s);
	assert!(mem.ram == before, "no key, no press: RAM untouched");
	assert!(s.opts.mousespeed.is_none() && !file.exists(), "nothing remembered until the player changes the row");
	assert_eq!((mousespeed_scale().to_bits(), mousespeed::shown()), (1.0f32.to_bits(), 50));
	// The two speeds are independent.
	CAMSPEED.store(100, Ordering::Relaxed);
	assert_eq!(camspeed_scale(), 2.0);
	assert_eq!(mousespeed_scale().to_bits(), 1.0f32.to_bits(), "Camera speed 100 does not move the mouse");
	mousespeed::set_for_test(150);
	assert_eq!((mousespeed_scale(), camspeed_scale()), (3.0, 2.0));
	CAMSPEED.store(0, Ordering::Relaxed);
	assert_eq!((mousespeed_scale(), camspeed_scale().to_bits()), (3.0, 1.0f32.to_bits()), "Mouse speed 150 does not move the stick");
	// `load` takes the options file's key; no key again -> back to exactly 1.0.
	let mut o = PcOptions::default();
	o.mousespeed = Some(30);
	mousespeed::load(&o);
	assert_eq!((mousespeed_scale(), mousespeed::shown()), (camspeed_factor(30), 30));
	mousespeed::load(&PcOptions::default());
	assert_eq!((mousespeed_scale().to_bits(), mousespeed::shown()), (1.0f32.to_bits(), 50));
	let _ = std::fs::remove_file(&file);
	s.path = None;
	s.opts = PcOptions::default();
	reset_statics();
}

fn state_dir(name: &str) -> Option<String> {
	[concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/")].iter().map(|b| format!("{b}{name}")).find(|p| std::path::Path::new(p).join("eeMemory.bin").exists())
}

/// On the loaded Eudora state: the rate table is found by value; without the key nothing at all is written; with the key all
/// three entries take the rate, again after a level reload; the texts come out of the new pool through the real lookup.
#[test]
fn eudora_rate_table_texts_and_no_key_no_write() {
	let _t = turn();
	let _turn = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner()); // pcext's tests reset LOCKON_MODE through `sync`
	let Some(dir) = state_dir("fg1/4_0159") else { return };
	let mut m = ee_oracle::Machine::load(&dir);
	ee_game::register();
	register_hand(crate::retail::disc::hands(&HANDS));
	m.enable_rec();
	assert_eq!(m.level(), 4);
	reset_statics();
	assert_eq!(find_rate_table(&mut m.cpu.mem), Some(0x161ea0), "the Eudora overlay's table, found by its three values");
	let mut guard = ST.lock().unwrap_or_else(|e| e.into_inner());
	let s: &mut St = &mut guard;
	let file = std::env::temp_dir().join(format!("pcext_camopts_{}.txt", std::process::id()));
	let _ = std::fs::remove_file(&file);
	s.opts = PcOptions::default();
	s.path = Some(file.clone());
	// No key: setup, then a field with nothing pressed changes no byte of RAM at all.
	camopts_setup(&mut m.cpu.mem, s);
	let before = m.cpu.mem.ram.clone();
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	camopts_field(&mut m.cpu.mem, 4, 0x2000, s);
	assert!(m.cpu.mem.ram == before, "no camspeed key: the field writes nothing");
	assert!(!file.exists(), "nothing is remembered until the player changes a row");
	assert_eq!(CAMSHOW.load(Ordering::Relaxed), preset_pct(m.cpu.mem.rd8(RETAIL_SPEED)), "the row shows retail's own preset");
	// Lock-on: the retail update cycles the byte, the host applies and remembers it.
	m.cpu.mem.wr8(LOCKON, 2);
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	assert_eq!(LOCKON_MODE.load(Ordering::Relaxed), 2);
	assert!(!lockon_decide(LOCKON_MODE.load(Ordering::Relaxed), false));
	assert_eq!(PcOptions::load(&file).lockon, Some(2));
	// Key 100: all three entries take the doubled Fast rate; an unchanged table is not rewritten; a level reload (retail data
	// again) is written again; the retail byte is not touched by the key alone.
	let at = 0x161ea0;
	let byte = m.cpu.mem.rd8(RETAIL_SPEED);
	CAMSPEED.store(100, Ordering::Relaxed);
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	let w = |m: &mut ee_oracle::Machine| [m.cpu.mem.rd32(at), m.cpu.mem.rd32(at + 4), m.cpu.mem.rd32(at + 8)];
	assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3]);
	assert_eq!(f32::from_bits(w(&mut m)[0]), 2.0 * f32::from_bits(0x3ce4_c388));
	assert_eq!(m.cpu.mem.rd8(RETAIL_SPEED), byte);
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3]);
	for (k, v) in RATE_RETAIL.iter().enumerate() {
		m.cpu.mem.wr32(at + 4 * k as u32, *v);
	}
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	assert_eq!(w(&mut m), [camspeed_rate_bits(100); 3], "rewritten after a level reload");
	// A new value: the doubled rate gives way to the new one.
	CAMSPEED.store(50, Ordering::Relaxed);
	camopts_field(&mut m.cpu.mem, 4, 0, s);
	assert_eq!(w(&mut m), [0x3ce4_c388; 3], "50 % writes retail's Fast into all three entries");
	// The row's step: the player sets 120: remembered, retail's byte follows with the nearest preset (Fast).
	set_camspeed(&mut m.cpu.mem, s, 120);
	assert_eq!((CAMSPEED.load(Ordering::Relaxed), CAMSHOW.load(Ordering::Relaxed), m.cpu.mem.rd8(RETAIL_SPEED)), (120, 120, 2));
	assert_eq!(PcOptions::load(&file).camspeed, Some(120));
	set_camspeed(&mut m.cpu.mem, s, 30);
	assert_eq!(m.cpu.mem.rd8(RETAIL_SPEED), 0);
	// The texts through the real lookup: the language word decides, the percent row shows the current value.
	let look = |m: &mut ee_oracle::Machine, id: u32, word: u32| -> (u32, String) {
		m.cpu.mem.wr32(LANG_WORD, word);
		m.cpu.s(4, id as u64);
		m.cpu.call(0x1f6b80);
		let p = m.cpu.g(2) as u32;
		let mut out = Vec::new();
		let mut a = p;
		while m.cpu.mem.rd8(a) != 0 {
			out.push(m.cpu.mem.rd8(a));
			a += 1;
		}
		(p, out.iter().map(|&b| b as char).collect())
	};
	let (p, text) = look(&mut m, ID_LOCKON, 2);
	assert_eq!((p, text.as_str()), (CO_POOL, "Verrouillage"));
	let (p, text) = look(&mut m, ID_LOCKON, 3);
	assert_eq!((p, text.as_str()), (CO_POOL, "Zielerfassung"));
	let (p, text) = look(&mut m, ID_AUTO, 5);
	assert_eq!((p, text.as_str()), (CO_POOL + 0x40, "Auto"));
	CAMSHOW.store(170, Ordering::Relaxed);
	let (p, text) = look(&mut m, ID_PCT, 0);
	assert_eq!((p, text.as_str()), (CO_POOL + 0x80, "< 170 >"));
	// A retail id still declines to retail's lookup (German state: 'Kamera').
	assert_eq!(look(&mut m, ID_CAMERA, 3).1, "Kamera");
	let _ = std::fs::remove_file(&file);
	s.path = None;
	s.opts = PcOptions::default();
	reset_statics();
}

/// The patched Options list (record ids, description ids, pictures) of a loaded Eudora state: Graphics and Controls before Quit
/// Game, and Mods between them and Quit Game only when `mods` (a mod is switched on at the build, `pcmods::load_store`).
pub(super) fn options_records_hold_the_new_rows(mem: &mut Mem, mods: bool) {
	let (pcctl, pcmods) = (super::super::pcctl::ID_CONTROLS, super::super::pcmods::ID_MODS);
	let (mut ids, mut desc) = (vec![0x4f1e, 0x4f1f, 0x4f20, 0x4f22, 0x4f23, 0x4f24, ID_GRAPHICS, pcctl], vec![0x4f31, 0x4f32, 0x4f33, 0x4f35, 0x4f36, 0x4f37, ID_GRAPHICS_DESC, super::super::pcctl::ID_DESC]);
	if mods {
		(ids.push(pcmods), desc.push(super::super::pcmods::ID_DESC));
	}
	let quit = ids.len() as u32;
	(ids.extend([0x4f26, 0]), desc.extend([0x4f39, 0]));
	let got: Vec<u32> = (0..ids.len() as u32).map(|i| mem.rd32(OPT_RECS + 12 * i) & 0xffff).collect();
	assert_eq!(got, ids);
	let got: Vec<u32> = (0..desc.len() as u32).map(|i| mem.rd32(OPT_DESC + 4 * i)).collect();
	assert_eq!(got, desc);
	let go = |mem: &mut Mem, row: u32| (mem.rd32(OPT_RECS + 12 * row) >> 16, mem.rd32(OPT_RECS + 12 * row + 4));
	assert_eq!(go(mem, 6), (3, GPAGE), "type 3: go to the Graphics page");
	assert_eq!(go(mem, 7), (3, super::super::pcctl::PAGE_AT), "type 3: go to the Controls page");
	if mods {
		assert_eq!(go(mem, 8), (3, super::super::pcmods::PAGE_AT), "type 3: go to the Mods page");
	}
	// Pictures: the retail seven around help_ss 5 (the record just below options_ss 0 is help_ss 6) and the Camera row's picture for Controls.
	let pic = |mem: &mut Mem, i: u32| (mem.rd32(OPT_PICS + 8 * i), mem.rd32(OPT_PICS + 8 * i + 4));
	assert_eq!(mem.rd32(OPT_PICS + 8 * quit), mem.rd32(0x138040 + 8 * 6), "Quit Game keeps the retail picture, wherever it sits");
	assert_eq!(pic(mem, 7), pic(mem, 4));
	if mods {
		assert_eq!(pic(mem, 8), pic(mem, 6), "Mods wears the planet picture of Graphics");
	}
	assert_eq!(mem.rd32(OPT_PICS + 8 * 6), mem.rd32(0x138040 - 0x10));
	assert_ne!(mem.rd32(OPT_PICS + 8 * 6 + 4), 0);
}

/// A temporary mods folder: every `(id, on)` is an installed textures mod, switched on in `active.txt` when `on`.
fn mods_folder(name: &str, mods: &[(&str, bool)]) -> PathBuf {
	let root = std::env::temp_dir().join(format!("pcext_rows_{}_{name}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let mut active = String::from("# Enabled mods, one id per line, highest priority first. Written by the RAC-RS Launcher.\n");
	for (id, on) in mods {
		std::fs::create_dir_all(root.join(id)).unwrap();
		std::fs::write(root.join(id).join("mod.json"), format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{id}\",\"kind\":\"textures\",\"game\":\"rac1\"}}")).unwrap();
		if *on {
			active += &format!("{id}\n");
		}
	}
	std::fs::create_dir_all(&root).unwrap();
	std::fs::write(root.join("active.txt"), active).unwrap();
	root
}

/// A fresh load of the Eudora state (a level load: retail's Options records again), then one `sync` with the mods folder `root`.
fn eudora_options_list(dir: &str, root: &std::path::Path) -> ee_oracle::Machine {
	let mut m = ee_oracle::Machine::load(dir);
	super::super::pcmods::test_root(Some(root));
	let mut s = St { opts: PcOptions::default(), path: None, last: None, glast: None, lang_seen: None, warned: false };
	sync(&mut m.cpu.mem, 4, false, &mut s);
	assert_eq!(m.cpu.mem.rd32(0x1b4bf8 + 0x34), OPT_RECS, "the Eudora Options list now reads the patched records");
	m
}

/// The Options list shows the Mods row only when at least one installed mod is switched on in `active.txt`: without any mod on
/// (no mods folder content, or mods installed and all off) the list is the retail 7 rows + Graphics + Controls = 9, one mod on gives
/// 10 rows with Mods before Quit Game. Switching the last mod off in the game keeps the row until the next level load.
#[test]
fn the_options_list_shows_mods_only_while_a_mod_is_on() {
	let _t = turn(); // sync sets the camera statics the other tests of this file read
	let _turn = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
	let Some(dir) = state_dir("fg1/4_0159") else { return };
	let (empty, off, on) = (mods_folder("empty", &[]), mods_folder("off", &[("pack_a", false), ("pack_b", false)]), mods_folder("on", &[("pack_a", false), ("pack_b", true)]));
	for (root, label) in [(&empty, "no mod installed"), (&off, "mods installed, all off")] {
		let mut m = eudora_options_list(&dir, root);
		options_records_hold_the_new_rows(&mut m.cpu.mem, false);
		eprintln!("{label}: the 9 retail rows");
	}
	let mut m = eudora_options_list(&dir, &on);
	options_records_hold_the_new_rows(&mut m.cpu.mem, true);
	// The last mod goes off in the game (active.txt rewritten): the patched list is left alone (idempotent), so the row stays.
	std::fs::write(on.join("active.txt"), "# nothing on\n").unwrap();
	let mut s = St { opts: PcOptions::default(), path: None, last: None, glast: None, lang_seen: None, warned: false };
	sync(&mut m.cpu.mem, 4, false, &mut s);
	options_records_hold_the_new_rows(&mut m.cpu.mem, true);
	// The next level load rebuilds the list from the folder as it is now: no row.
	let mut m = eudora_options_list(&dir, &on);
	options_records_hold_the_new_rows(&mut m.cpu.mem, false);
	super::super::pcmods::test_root(None);
	for r in [empty, off, on] {
		let _ = std::fs::remove_dir_all(r);
	}
}
