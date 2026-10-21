// rac-rs: a from-scratch Rust reimplementation of Ratchet & Clank (PS2) that runs on the user's own
// disc image. This first layer reads the disc the way the game does: by sector, through the table
// of contents, with Insomniac's WAD decompression.
// This file is the root of the library crate rac_game (src/lib.rs includes it); the exe is exe/src/main.rs, which
// sets the no-console subsystem (feature `windows_gui`) and calls `main` below.

mod ambience;
mod anim;
mod audio;
mod gauss;
mod bytes;
mod camera;
mod camgrid;
mod collision;
mod devtune;
mod dynlight;
mod flow;
mod fmv;
mod game;
mod frontend;
mod gsfull;
mod gadgets;
mod gameplay;
mod help;
mod hud;
mod inventory;
mod iso;
mod level;
mod loadlog;
mod map;
mod mesh;
mod mobies;
mod particles;
mod music;
mod occlusion;
mod overlay;
mod physics;
mod player;
mod render;
mod retail;
mod save;
mod scene;
mod settings;
mod fire;
mod shadows;
mod shipgloss;
mod shop;
mod shrubs;
mod sky;
mod strips;
mod sound;
mod spumix;
mod spurev;
mod terrain;
mod ties;
mod toc;
mod viewer;
mod ocean;
mod water;
mod wad;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage:
  rac-rs levels <disc.iso>                     list the 19 levels and their sector ranges
  rac-rs view   <disc.iso> <level> [--play] [--shot out.png] [--cam x y z yaw pitch] [--give bomb-glove[=N]]
                                               fly around a level (right mouse look, WASD, PgUp/PgDn)
  rac-rs terrain <disc.iso> <level|all> <lod 0|2> <out.obj|dir>  decode level terrain to OBJ
  rac-rs ties   <disc.iso> <level|all>          decode tie (prop) classes and placements
  rac-rs scan  <disc.iso>                    decompress every sector-aligned WAD blob
  rac-rs verify <disc.iso> <file>...           check Wrench-extracted files appear in decompressed blobs
  rac-rs dump   <disc.iso> <sector> <out>      decompress the WAD starting at <sector> into <out>
  rac-rs warm-gpu                              build the GPU capture's kernels once (the driver keeps them), then exit";

/// Without a console (feature `windows_gui`) stderr goes nowhere, so panics and the final error are appended to `<player dir>/crash.log`.
#[cfg(feature = "windows_gui")]
fn crash_log(text: &str) {
	use std::io::Write;
	let dir = retail::memcard::player_dir();
	let _ = std::fs::create_dir_all(&dir);
	let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
	if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
		let _ = writeln!(f, "[unix time {secs}] {text}");
	}
}

/// A plain Windows message box (no console needed); on other platforms the text goes to stderr.
fn message_box(text: &str) {
	#[cfg(windows)]
	{
		#[link(name = "user32")]
		unsafe extern "system" {
			fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
		}
		let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
		// MB_OK | MB_ICONINFORMATION
		unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("Ratchet & Clank (rac-rs)").as_ptr(), 0x40) };
	}
	#[cfg(not(windows))]
	eprintln!("{text}");
}

/// The install root: the first folder from the exe's folder upwards that holds `generated/play.cfg`. Err is the text for a message box.
fn install_root() -> Result<std::path::PathBuf, String> {
	let exe = std::env::current_exe().map_err(|e| format!("Cannot find the game folder ({e}). Run Setup.cmd first."))?;
	exe.parent()
		.and_then(|d| d.ancestors().find(|d| d.join("generated").join("play.cfg").is_file()))
		.map(|d| d.to_path_buf())
		.ok_or_else(|| "Run Setup.cmd first: the game has not been set up yet (generated/play.cfg not found).".to_string())
}

/// [PC] The window title: "rac-rs v0.2.1" from the install root's `generated/built_version.txt` (setup.ps1 writes it after building
/// this exe, so an update whose files arrived but whose build has not finished still shows the old version), else its VERSION file
/// (the release the player downloaded), else "rac-rs" (dev and test runs outside an install).
pub fn app_name() -> &'static str {
	static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
	NAME.get_or_init(|| {
		let read = |r: &std::path::Path, f: &str| std::fs::read_to_string(r.join(f)).ok().filter(|v| !v.trim().is_empty());
		let ver = install_root().ok().and_then(|r| read(&r, "generated/built_version.txt").or_else(|| read(&r, "VERSION"))).unwrap_or_default();
		let ver = ver.trim_start_matches('\u{feff}').trim();
		if ver.is_empty() || ver.len() > 24 || !ver.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) { "rac-rs".into() } else { format!("rac-rs v{ver}") }
	})
}

/// Public Windows build only: before the game starts, ask `setup.ps1 -Launch` (a hidden PowerShell) whether an update is waiting.
/// Returns true when the update window took over (PowerShell exit code 10): the game must not start, this exe must exit so that the
/// update can replace it. Every other outcome (no setup.ps1, offline, up to date, "Not now", spawn failure, a hang) returns false = start the game.
/// The script writes `generated/update_window.flag` when it shows a window: until then it gets 15 s; once the flag exists the exe waits for the
/// answer without a time limit. A flag file (not a pipe) keeps this single-threaded, as `launch_args` needs (`set_var`).
/// Skipped with RAC_NO_UPDATE_CHECK (Play old version), and in the automated test runs (RAC_TEST_WINDOW / RAC_RETAIL_SMOKE) unless RAC_TEST_UPDATE_BASE is set.
/// Test hooks: RAC_TEST_UPDATE_BASE -> `-TestUpdateBase`, RAC_TEST_UPDATE_CHOICE=update|notnow -> `-TestNoShow -TestChoice`, RAC_TEST_UPDATE_SHOT -> `-TestShot`.
#[cfg(all(windows, feature = "windows_gui"))]
fn update_check() -> bool {
	use std::os::windows::process::CommandExt;
	use std::process::{Command, Stdio};
	use std::time::{Duration, Instant};
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	let has = |k: &str| std::env::var_os(k).is_some();
	if has("RAC_NO_UPDATE_CHECK") || ((has("RAC_TEST_WINDOW") || has("RAC_RETAIL_SMOKE")) && !has("RAC_TEST_UPDATE_BASE")) {
		return false;
	}
	let Ok(root) = install_root() else { return false };
	let setup = root.join("setup.ps1");
	if !setup.is_file() {
		return false;
	}
	let flag = root.join("generated").join("update_window.flag");
	let _ = std::fs::remove_file(&flag);
	let sysroot = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
	let mut cmd = Command::new(sysroot.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
	cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&setup);
	cmd.args(["-Launch", "-ExePid"]).arg(std::process::id().to_string()).arg("-WindowFlag").arg(&flag);
	if let Some(v) = std::env::var_os("RAC_TEST_UPDATE_BASE") {
		cmd.arg("-TestUpdateBase").arg(v);
	}
	if let Ok(c) = std::env::var("RAC_TEST_UPDATE_CHOICE") {
		if c == "update" || c == "notnow" {
			cmd.args(["-TestNoShow", "-TestChoice"]).arg(c);
			if let Some(v) = std::env::var_os("RAC_TEST_UPDATE_SHOT") {
				cmd.arg("-TestShot").arg(v);
			}
		}
	}
	cmd.current_dir(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW);
	let Ok(mut child) = cmd.spawn() else { return false };
	let t0 = Instant::now();
	loop {
		match child.try_wait() {
			Ok(Some(status)) => return status.code() == Some(10),
			Ok(None) => {}
			Err(_) => return false,
		}
		if t0.elapsed() > Duration::from_secs(15) && !flag.is_file() {
			let _ = child.kill();
			let _ = child.wait();
			return false;
		}
		std::thread::sleep(Duration::from_millis(40));
	}
}

/// The no-argument start (what Play.cmd did): find the install root (the first folder from the exe's folder upwards that holds
/// `generated/play.cfg`), read `iso=<path>` and optional `lang=en|fr|de|es|it` from it, set the environment Play.cmd set, rotate the
/// card safety copies and return the arguments of `view <iso> 0 --retail-cold --modern-cam`. Err is the text for a message box.
/// Runs first thing in `main`, before any thread starts (`set_var` is only sound then).
fn launch_args() -> Result<Vec<String>, String> {
	let root = install_root()?;
	let cfg = std::fs::read(root.join("generated").join("play.cfg")).map_err(|e| format!("Cannot read generated/play.cfg ({e}). Run Setup.cmd again."))?;
	let cfg = String::from_utf8_lossy(&cfg);
	let (mut iso, mut lang) = (None, None);
	for line in cfg.trim_start_matches('\u{feff}').lines() {
		if let Some((k, v)) = line.split_once('=') {
			match k.trim() {
				"iso" => iso = Some(v.trim().to_string()),
				"lang" => lang = Some(v.trim().to_ascii_lowercase()),
				_ => {}
			}
		}
	}
	let iso = iso.filter(|i| !i.is_empty()).ok_or("Run Setup.cmd again: generated/play.cfg has no disc image path.")?;
	if !std::path::Path::new(&iso).is_file() {
		return Err(format!("Your disc image moved or is gone:\n{iso}\n\nRun Setup.cmd again."));
	}
	let player = root.join("player");
	let _ = std::fs::create_dir_all(&player);
	// SAFETY: still single-threaded (called first in main, after the load-log clock which starts no thread).
	unsafe {
		std::env::set_var("ORACLE_ISO", &iso);
		std::env::set_var("ORACLE_STUBS", root.join("oracle").join("stubs.txt"));
		std::env::set_var("RAC_PLAYER_DIR", &player);
		if std::env::var_os("COLD_LANG").is_none() {
			let lang = lang.filter(|l| ["en", "fr", "de", "es", "it"].contains(&l.as_str())).unwrap_or_else(|| "en".to_string());
			std::env::set_var("COLD_LANG", lang);
		}
	}
	// Safety copies of the save card before each start (the newest two starts).
	let (card, c1, c2) = (player.join("card.bin"), player.join("card.start1.bin"), player.join("card.start2.bin"));
	if c1.is_file() {
		let _ = std::fs::copy(&c1, &c2);
	}
	if card.is_file() {
		let _ = std::fs::copy(&card, &c1);
	}
	Ok(["view", iso.as_str(), "0", "--retail-cold", "--modern-cam"].map(String::from).to_vec())
}

pub fn main() -> ExitCode {
	loadlog::init();
	#[cfg(feature = "windows_gui")]
	std::panic::set_hook(Box::new(|info| {
		let thread = std::thread::current();
		crash_log(&format!("panic in thread {}: {info}", thread.name().unwrap_or("?")));
	}));
	let mut args: Vec<String> = std::env::args().skip(1).collect();
	// Started by the player: double-clicked (no arguments: the desktop shortcut, Setup's Play) or by Play.cmd (the same cold boot with
	// its arguments spelled out). Both ask about updates first; Play.cmd skipped the question up to v0.1.2.
	#[cfg(all(windows, feature = "windows_gui"))]
	if (args.is_empty() || args.iter().any(|a| a == "--retail-cold")) && update_check() {
		return ExitCode::SUCCESS;
	}
	// Double-clicked (no arguments): start the game the way Play.cmd does, from generated/play.cfg written by setup.
	if args.is_empty() {
		match launch_args() {
			Ok(a) => args = a,
			Err(msg) => {
				message_box(&msg);
				return ExitCode::from(2);
			}
		}
	}
	let result = match args.first().map(String::as_str) {
		Some("levels") if args.len() == 2 => cmd_levels(&args[1]),
		Some("view") if args.len() >= 3 => cmd_view(&args[1..]),
		Some("terrain") if args.len() == 5 => cmd_terrain(&args[1], &args[2], &args[3], &args[4]),
		Some("ties") if args.len() == 3 => cmd_ties(&args[1], &args[2]),
		Some("scan") if args.len() == 2 => cmd_scan(&args[1]).map(|_| ()),
		Some("verify") if args.len() >= 3 => cmd_verify(&args[1], &args[2..]),
		Some("dump") if args.len() == 4 => cmd_dump(&args[1], &args[2], &args[3]),
		Some("warm-gpu") => retail::warm_gpu().map_err(Into::into),
		_ => {
			eprintln!("{USAGE}");
			return ExitCode::from(2);
		}
	};
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			#[cfg(feature = "windows_gui")]
			{
				crash_log(&format!("error: {e}"));
				message_box(&format!("The game stopped: {e}\n\nDetails are in player/crash.log."));
			}
			ExitCode::FAILURE
		}
	}
}

type Res<T> = Result<T, Box<dyn std::error::Error>>;


fn cmd_levels(path: &str) -> Res<()> {
	let mut disc = iso::Disc::open(&PathBuf::from(path))?;
	for l in toc::read_levels(&mut disc)? {
		// The data range starts with a table of (offset, size) pairs; the first points at the
		// level's code and data block.
		let head = disc.read_bytes(l.data.sector, 16)?;
		let (off, size) = (iso::u32_le(&head, 0) as u64, iso::u32_le(&head, 4) as u64);
		let at = disc.read_bytes(l.data.sector + off / iso::SECTOR_SIZE, 16)?;
		let kind = if off % iso::SECTOR_SIZE == 0 && wad::is_wad(&at) { "WAD" } else { "raw" };
		println!(
			"{:>2} {:<27} header {:>7}  data {:>7}+{:<6} gameplay {:>7}+{:<4} occl {:>7}+{:<4} first block +{:#x} {:>8} B {kind}",
			l.id,
			toc::LEVEL_NAMES[l.id],
			l.header_sector,
			l.data.sector,
			l.data.sectors,
			l.gameplay_pal.sector,
			l.gameplay_pal.sectors,
			l.occlusion.sector,
			l.occlusion.sectors,
			off,
			size
		);
	}
	Ok(())
}

/// `--give bomb-glove[=N]`, `helmet` or an item number (or RAC_GIVE): the item id and its ammo (None = the start ammo).
fn parse_give(s: &str) -> Option<(u8, Option<u16>)> {
	let (name, n) = s.split_once('=').map_or((s, None), |(a, b)| (a, b.parse().ok()));
	match name {
		"bomb-glove" | "glove" => Some((inventory::BOMB_GLOVE, n)),
		// The Pilot's Helmet (item 7): not placed on Veldin, but the one Veldin pickup popup.
		"helmet" => Some((7, n)),
		id => id.parse().ok().map(|id| (id, n)),
	}
}

/// view <iso> <level|title> [--play] [--shot out.png] [--cam x y z yaw pitch] [--give bomb-glove[=N]]
fn cmd_view(a: &[String]) -> Res<()> {
	// "title" boots the front end (title scene and menu) instead of a level.
	let front = a[1] == "title";
	let mut level: usize = if front { 0 } else { a[1].parse()? };
	let (mut shot, mut cam, mut play) = (None, None, false);
	let mut give = std::env::var("RAC_GIVE").ok().and_then(|g| parse_give(&g));
	let mut i = 2;
	while i < a.len() {
		match a[i].as_str() {
			"--shot" if i + 1 < a.len() => {
				shot = Some(a[i + 1].clone());
				i += 2;
			}
			"--play" => {
				play = true;
				i += 1;
			}
			"--cam" if i + 5 < a.len() => {
				let v: Vec<f32> = a[i + 1..i + 6].iter().map(|x| x.parse()).collect::<Result<_, _>>()?;
				cam = Some([v[0], v[1], v[2], v[3], v[4]]);
				i += 6;
			}
			// Retail mode: the game's own code runs the level from a savestate (p2s.py output dir), retail.rs.
			"--retail" if i + 1 < a.len() => {
				retail::enable(&a[i + 1])?;
				play = true;
				i += 2;
			}
			// Retail mode from a cold boot (no savestate): the retail game starts from the disc (logos, title, New Game), oracle Machine::cold.
			"--retail-cold" => {
				retail::enable_cold(&a[0])?;
				play = true;
				i += 1;
			}
			// Retail mode, opt-in: show the game's own complete picture (GS output, gsfull/) instead of the mirror renderer.
			"--gsfull" => {
				retail::enable_gsfull();
				i += 1;
			}
			// Retail mode: start from the game's own title screen (it walks itself there through Quit Game), retail/title_boot.rs.
			"--title" => {
				retail::title_boot::enable();
				i += 1;
			}
			// With --gsfull: 16:9 view (wider field of view, HUD squeezed back, square pixels); gsfull/wide.rs.
			// Modern camera + RMB aim in retail mode (retail/modcam.rs); F7 toggles in game.
			"--modern-cam" => {
				retail::modcam::enable();
				i += 1;
			}
			"--wide" => {
				gsfull::wide::enable();
				i += 1;
			}
			"--give" if i + 1 < a.len() => {
				give = Some(parse_give(&a[i + 1]).ok_or_else(|| format!("unknown item {}", a[i + 1]))?);
				i += 2;
			}
			x => return Err(format!("unknown view option {x}").into()),
		}
	}
	if !retail::active() {
		return Err("view needs --retail STATE_DIR: the hand-written game is archived".into());
	}
	// Retail mode: rac-rs loads the savestate's own level (assets, moby pools); a different LEVEL argument is overridden.
	if let Some(l) = retail::start_level().map(|l| l as usize).filter(|&l| !front && l != level) {
		eprintln!("retail: the savestate is level {l}, not {level}: loading level {l}");
		level = l;
	}
	viewer::run(&a[0], level, shot, cam, play, front, give)
}

fn cmd_terrain(path: &str, which: &str, lod: &str, out: &str) -> Res<()> {
	let mut disc = iso::Disc::open(&PathBuf::from(path))?;
	let levels = toc::read_levels(&mut disc)?;
	let lod: u8 = lod.parse()?;
	let ids: Vec<usize> = if which == "all" { (0..levels.len()).collect() } else { vec![which.parse()?] };
	let mut failed = 0;
	for id in ids {
		let info = levels.get(id).ok_or("no such level")?;
		let result = level::Level::load(&mut disc, info)
			.and_then(|l| Ok((terrain::terrain(l.terrain_block()?, lod)?, l.all_textures()?)));
		match result {
			Ok((meshes, textures)) => {
				let tris: usize = meshes.iter().map(|m| m.faces.len()).sum();
				let verts: usize = meshes.iter().map(|m| m.positions.len()).sum();
				let file = if which == "all" {
					std::fs::create_dir_all(out)?;
					format!("{out}/level_{id:02}_lod{lod}.obj")
				} else {
					out.to_string()
				};
				std::fs::write(&file, mesh::to_obj(&meshes))?;
				let img = render::render(&meshes, &textures, 1280, 720, 225.0, 40.0);
				std::fs::write(PathBuf::from(&file).with_extension("png"), render::png(&img))?;
				println!("{id:>2} {:<27} {:>4} fragments {:>7} vertices {:>7} triangles -> {file}", toc::LEVEL_NAMES[id], meshes.len(), verts, tris);
			}
			Err(e) => {
				failed += 1;
				println!("{id:>2} {:<27} FAILED: {e}", toc::LEVEL_NAMES[id]);
			}
		}
	}
	if failed > 0 {
		return Err(format!("{failed} level(s) failed").into());
	}
	Ok(())
}

fn cmd_ties(path: &str, which: &str) -> Res<()> {
	let mut disc = iso::Disc::open(&PathBuf::from(path))?;
	let levels = toc::read_levels(&mut disc)?;
	let ids: Vec<usize> = if which == "all" { (0..levels.len()).collect() } else { vec![which.parse()?] };
	let mut failed = 0;
	for id in ids {
		let info = levels.get(id).ok_or("no such level")?;
		let result = level::Level::load(&mut disc, info).and_then(|l| {
			let classes = ties::tie_classes(&l)?;
			let instances = ties::tie_instances(&l, &classes)?;
			let textures = l.all_textures()?;
			let missing = classes.values().flat_map(|m| m.faces.iter()).filter(|f| !textures.contains_key(&(f.0 & mobies::KEY_MASK))).count();
			Ok((classes, instances, missing))
		});
		match result {
			Ok((classes, instances, missing)) => {
				let class_tris: usize = classes.values().map(|m| m.faces.len()).sum();
				let placed: usize = instances.iter().map(|i| classes[&i.class_id].faces.len()).sum();
				println!("{id:>2} {:<27} {:>3} classes {:>6} class tris {:>5} instances {:>8} placed tris, {missing} faces without texture", toc::LEVEL_NAMES[id], classes.len(), class_tris, instances.len(), placed);
			}
			Err(e) => {
				failed += 1;
				println!("{id:>2} {:<27} FAILED: {e}", toc::LEVEL_NAMES[id]);
			}
		}
	}
	if failed > 0 {
		return Err(format!("{failed} level(s) failed").into());
	}
	Ok(())
}

pub struct Blob {
	pub sector: u64,
	pub data: Vec<u8>,
}

/// Walk the whole disc in large chunks and decompress every WAD that starts on a sector boundary.
fn cmd_scan(path: &str) -> Res<Vec<Blob>> {
	let mut disc = iso::Disc::open(&PathBuf::from(path))?;
	let chunk = 4096u64; // sectors (8 MB)
	let mut blobs = Vec::new();
	let (mut ok, mut failed, mut in_bytes, mut out_bytes) = (0usize, 0usize, 0usize, 0usize);
	let mut sector = 0u64;
	while sector < disc.sector_count {
		let n = chunk.min(disc.sector_count - sector);
		let buf = disc.read_sectors(sector, n)?;
		for i in 0..n {
			let off = (i * iso::SECTOR_SIZE) as usize;
			let head = &buf[off..off + 16];
			if !wad::is_wad(head) {
				continue;
			}
			let size = wad::compressed_size(head);
			if size < 0x20 || size > 64 << 20 {
				continue;
			}
			let s = sector + i;
			let data = disc.read_bytes(s, size.min(((disc.sector_count - s) * iso::SECTOR_SIZE) as usize))?;
			match wad::decompress(&data) {
				Ok(out) => {
					ok += 1;
					in_bytes += size;
					out_bytes += out.len();
					blobs.push(Blob { sector: s, data: out });
				}
				Err(e) => {
					failed += 1;
					if failed <= 10 {
						eprintln!("sector {s}: {e}");
					}
				}
			}
		}
		sector += n;
	}
	println!(
		"WAD blobs: {ok} decompressed, {failed} failed; {:.1} MB -> {:.1} MB",
		in_bytes as f64 / 1e6,
		out_bytes as f64 / 1e6
	);
	Ok(blobs)
}

/// Oracle: Wrench independently extracted the disc's assets. If our decompressor is right, a file
/// Wrench pulled out of a compressed WAD must appear byte-for-byte inside one of our blobs.
/// For ELF files only the .text section is compared.
fn cmd_verify(path: &str, files: &[String]) -> Res<()> {
	let blobs = cmd_scan(path)?;
	let (mut full, mut partial, mut missing) = (0, 0, 0);
	for file_path in files {
		let raw = std::fs::read(file_path)?;
		let content: &[u8] = if raw.starts_with(b"\x7fELF") {
			elf_section(&raw, ".text").ok_or("no .text section")?
		} else {
			&raw
		};
		if content.len() < 64 {
			continue;
		}
		// Search with a window from the middle: file starts are often shared headers.
		let mid = content.len() / 2 - 32;
		let needle = &content[mid..mid + 64];
		let hit = blobs.iter().find_map(|b| find(&b.data, needle).map(|off| (b, off)));
		match hit {
			Some((blob, off)) if off >= mid => {
				let start = off - mid;
				let whole = blob.data.get(start..start + content.len()) == Some(content);
				if whole {
					full += 1;
				} else {
					partial += 1;
					println!("{file_path}: window found @ sector {} but full content differs", blob.sector);
				}
			}
			_ => {
				missing += 1;
				println!("{file_path}: not in any decompressed blob");
			}
		}
	}
	println!("verify: {full} full matches, {partial} partial, {missing} not found (of {} files)", files.len());
	Ok(())
}

fn cmd_dump(path: &str, sector: &str, out: &str) -> Res<()> {
	let mut disc = iso::Disc::open(&PathBuf::from(path))?;
	let sector: u64 = sector.parse()?;
	let head = disc.read_bytes(sector, 16)?;
	if !wad::is_wad(&head) {
		return Err(format!("no WAD header at sector {sector}").into());
	}
	let data = disc.read_bytes(sector, wad::compressed_size(&head))?;
	let raw = wad::decompress(&data)?;
	std::fs::write(out, &raw)?;
	println!("sector {sector}: {} B compressed -> {} B written to {out}", data.len(), raw.len());
	Ok(())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
	hay.windows(needle.len()).position(|w| w == needle)
}

/// Minimal ELF32 little-endian section lookup.
fn elf_section<'a>(elf: &'a [u8], name: &str) -> Option<&'a [u8]> {
	let u16_at = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
	let shoff = iso::u32_le(elf, 0x20) as usize;
	let shentsize = u16_at(0x2e);
	let shnum = u16_at(0x30);
	let shstrndx = u16_at(0x32);
	let sh = |i: usize| shoff + i * shentsize;
	let strtab = iso::u32_le(elf, sh(shstrndx) + 0x10) as usize;
	for i in 0..shnum {
		let h = sh(i);
		let name_off = strtab + iso::u32_le(elf, h) as usize;
		let end = elf[name_off..].iter().position(|&c| c == 0)? + name_off;
		if &elf[name_off..end] == name.as_bytes() {
			let off = iso::u32_le(elf, h + 0x10) as usize;
			let size = iso::u32_le(elf, h + 0x14) as usize;
			return elf.get(off..off + size);
		}
	}
	None
}
