//! Eudora acceptance test (headless): drives the rac-rs retail path (retail::Retail::load / tick, exactly as the exe does) with the
//! recorded Eudora play windows, per window against the oracle runner. See <dev>/scratch_play/handoff/eudora_acceptance.md.
#![allow(warnings)]
mod ambience;
mod anim;
mod audio;
mod gauss;
mod bytes;
mod camera;
mod camgrid;
mod collision;
mod dynlight;
mod flow;
mod fmv;
mod game;
mod frontend;
#[allow(dead_code)]
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
mod missions;
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

/// The window title main.rs gives the viewer (the acceptance runs have no install root).
pub fn app_name() -> &'static str {
	"rac-rs"
}

// The recorded-pad feed shared with the oracle runners (movetest / fndiff): pad.raw + pad.pokes of a play window.
#[path = "../../oracle/pcsx2rec/playfeed.rs"]
mod playfeed;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const PLAY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/play");
const D1: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scratch_emufix/deploy/runs/D1/verify.tsv");
const TRAIL_EVERY: u32 = 50;
const USAGE: &str = "eudora_accept: headless acceptance test of the rac-rs retail path on the recorded Eudora play windows
  eudora_accept [--windows 0000,0001|all] [--fields CAP] [-j N] [--timeout SECS] [--stall SECS] [--out DIR]
                [--nohud] [--gsfull] [--rebaseline] [--no-baseline] [--allow-feed-gap]
  (internal) eudora_accept --child STATE_DIR FIELDS [--gsfull] | --oracle STATE_DIR FIELDS OUT_FILE
exit code 0 = every window ok";

fn hash_ram(ram: &[u8]) -> u64 {
	let r = &ram[0x80000..0x1f00000.min(ram.len())];
	r.chunks_exact(8).fold(0xcbf2_9ce4_8422_2325u64, |h, w| (h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(0x0100_0000_01b3).rotate_left(23))
}

fn big_stack(f: impl FnOnce() -> ExitCode + Send + 'static) -> ExitCode {
	std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap_or(ExitCode::from(4))
}

fn main() -> ExitCode {
	let a: Vec<String> = std::env::args().skip(1).collect();
	match a.first().map(String::as_str) {
		Some("--child") if a.len() >= 3 => child(&a[1], a[2].parse().unwrap_or(0), a.iter().any(|x| x == "--gsfull")),
		Some("--oracle") if a.len() >= 4 => oracle(&a[1], a[2].parse().unwrap_or(0), &a[3]),
		Some("-h" | "--help") => {
			println!("{USAGE}");
			ExitCode::SUCCESS
		}
		_ => parent(&a),
	}
}

// ---------------------------------------------------------------------------------------------------------------- pad feed

/// pad.raw as pads[field] (a field without a line repeats the last report; start report before the first line).
fn read_pads(dir: &str) -> Vec<[u8; 18]> {
	let text = std::fs::read_to_string(format!("{dir}/pad.raw")).unwrap_or_default();
	let mut pads: Vec<[u8; 18]> = Vec::new();
	let mut last = [0xff, 0xff, 0x7f, 0x7f, 0x7f, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
	for l in text.lines() {
		let Some((f, h)) = l.split_once('\t') else { continue };
		let (Ok(f), true) = (f.parse::<usize>(), h.len() == 36) else { continue };
		while pads.len() < f {
			pads.push(last);
		}
		for k in 0..18 {
			last[k] = u8::from_str_radix(&h[2 * k..2 * k + 2], 16).unwrap_or(0);
		}
		pads.push(last);
	}
	pads
}

/// The pad report `Machine::set_pad(buttons, sticks)` builds (oracle/src/lib.rs), to measure what the retail path's
/// (buttons, sticks) interface loses against the recorded 18-byte report.
fn synth(buttons: u16, sticks: [u8; 4]) -> [u8; 18] {
	let b = !buttons;
	let mut r = [0u8; 18];
	r[0] = (b >> 8) as u8;
	r[1] = b as u8;
	r[2..6].copy_from_slice(&sticks);
	for (k, m) in [0x2000u16, 0x8000, 0x1000, 0x4000, 0x10, 0x20, 0x40, 0x80, 4, 8, 1, 2].iter().enumerate() {
		r[6 + k] = if buttons & m != 0 { 0xff } else { 0 };
	}
	r
}

fn pokes_of(dir: &str) -> Vec<u32> {
	std::fs::read_to_string(format!("{dir}/pad.pokes")).unwrap_or_default().lines().filter_map(|l| l.split('\t').next()?.parse().ok()).collect()
}

// ---------------------------------------------------------------------------------------------------------------- child

static SENT: AtomicU32 = AtomicU32::new(0);

/// One window through the retail path: `Retail::load` + `tick(buttons, sticks)` per field, as viewer/handler.rs does with
/// the keyboard's pad. RAC_RETAIL_SMOKE=FIELDS (set by the parent) makes the EE thread print speed lines and the RAM hash
/// after the last field and exit; RAC_RETAIL_REC / _REC_TRAIL write the pad actually fed and the RAM trail.
fn child(dir: &str, fields: u32, gsfull: bool) -> ExitCode {
	std::panic::set_hook(Box::new(|i| {
		println!("ACC panic sent {} {}", SENT.load(Ordering::SeqCst), i.to_string().replace('\n', " | "));
		std::process::exit(4);
	}));
	let pads = read_pads(dir);
	let mut diff = 0u32;
	let mut buf = Vec::new();
	for f in 0..fields as usize {
		let p = pads.get(f).or(pads.last()).copied().unwrap_or([0xff, 0xff, 0x7f, 0x7f, 0x7f, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
		let b = !(((p[0] as u16) << 8) | p[1] as u16);
		buf.push((b, [p[2], p[3], p[4], p[5]]));
		if synth(b, [p[2], p[3], p[4], p[5]]) != p {
			diff += 1;
		}
	}
	println!("ACC pad fields {} recorded {} report_differs {diff} pokes {}", fields, pads.len(), pokes_of(dir).len());
	if let Err(e) = retail::enable(dir) {
		println!("ACC stop load {e}");
		return ExitCode::from(5);
	}
	if gsfull {
		retail::enable_gsfull();
	}
	let Some(mut r) = retail::Retail::load() else {
		println!("ACC stop load Retail::load returned None");
		return ExitCode::from(5);
	};
	let (mut snd, mut ovl, mut gsf) = (0usize, 0usize, 0usize);
	for f in 0..=fields {
		let (b, s) = buf.get(f as usize).copied().unwrap_or((0, [0x80; 4]));
		SENT.store(f + 1, Ordering::SeqCst);
		r.tick(b, s);
		snd += r.take_sound().ev.len();
		ovl += r.take_overlay().is_some() as usize;
		gsf += r.take_gsfull_frame().is_some() as usize;
		if let Some(sw) = r.switch() {
			println!("ACC travel field {f} level {}", sw.0);
		}
		if let Some(e) = r.take_stop() {
			println!("ACC stop field {} {}", f.saturating_sub(1), e.replace('\n', " | "));
			return ExitCode::from(3);
		}
		if f % 100 == 0 {
			println!("ACC prog {f} snd {snd} overlays {ovl} gsfull_frames {gsf}");
		}
	}
	println!("ACC end-without-smoke");
	ExitCode::from(6)
}

// ---------------------------------------------------------------------------------------------------------------- oracle

/// The oracle runner's trail for a window (movetest's loop: playfeed apply, trail hash every 50 fields, frame), in this
/// build, so the retail path is compared with the same recompiled code. Lines `trail F HASH` and `end N HASH`.
fn oracle(dir: &str, fields: u32, out: &str) -> ExitCode {
	let (dir, out) = (dir.to_string(), out.to_string());
	big_stack(move || {
		let mut m = ee_oracle::Machine::load(&dir);
		ee_game::register();
		m.enable_rec();
		let feed = playfeed::open(Some(playfeed::MARK), false, &dir, 0, "ACC_ORACLE_NO_TRAIL", playfeed::code_hash(include_str!("../../oracle/pcsx2rec/playfeed.rs"))).expect("feed");
		let mut o = String::new();
		for f in 0..fields {
			feed.apply(&mut m.cpu, f);
			if f % TRAIL_EVERY == 0 {
				o += &format!("trail {f} {:016x}\n", hash_ram(&m.cpu.mem.ram[..]));
			}
			if let Err(e) = m.frame() {
				o += &format!("stop {f} {}\n", e.replace('\n', " | "));
				std::fs::write(&out, o).ok();
				return ExitCode::from(3);
			}
		}
		o += &format!("end {fields} {:016x}\n", hash_ram(&m.cpu.mem.ram[..]));
		std::fs::write(&out, o).ok();
		ExitCode::SUCCESS
	})
}

// ---------------------------------------------------------------------------------------------------------------- parent

#[derive(Clone)]
struct Opts {
	out: String,
	cap: u32,
	timeout: u64,
	stall: u64,
	hud: bool,
	gsfull: bool,
	rebase: bool,
	baseline: bool,
}

#[derive(Default, Clone)]
struct Res {
	win: String,
	asked: u32,
	verdict: String,
	detail: String,
	fields_done: u32,
	ee_ms: f64,
	worker_fps: f64,
	ram: String,
	pokes: usize,
	first_poke: Option<u32>,
	pad_diff: u32,
	hooked: bool,
	trail_ok: String,
	oracle_end: String,
	snd: String,
	d1: String,
}

fn run_with_limits(mut cmd: std::process::Command, log: &str, o: &Opts, tag: &str) -> (Option<i32>, String) {
	let f = std::fs::File::create(log).expect("log");
	cmd.stdout(f.try_clone().unwrap()).stderr(f);
	let mut ch = match cmd.spawn() {
		Ok(c) => c,
		Err(e) => return (None, format!("spawn: {e}")),
	};
	let (t0, mut last_change, mut last_len) = (Instant::now(), Instant::now(), 0u64);
	loop {
		match ch.try_wait() {
			Ok(Some(st)) => return (st.code(), String::new()),
			Ok(None) => {}
			Err(e) => return (None, format!("wait: {e}")),
		}
		let len = std::fs::metadata(log).map(|m| m.len()).unwrap_or(0);
		if len != last_len {
			(last_len, last_change) = (len, Instant::now());
		}
		let why = if t0.elapsed().as_secs() > o.timeout {
			Some(format!("{tag}: wall timeout {}s", o.timeout))
		} else if last_change.elapsed().as_secs() > o.stall {
			Some(format!("{tag}: no output for {}s", o.stall))
		} else {
			None
		};
		if let Some(w) = why {
			let _ = ch.kill(); // exact PID of our own child
			let _ = ch.wait();
			return (None, w);
		}
		std::thread::sleep(Duration::from_millis(250));
	}
}

fn parse_trail(text: &str) -> (BTreeMap<u32, String>, Option<(u32, String)>, Option<String>) {
	let (mut t, mut end, mut stop) = (BTreeMap::new(), None, None);
	for l in text.lines() {
		let p: Vec<&str> = l.split_whitespace().collect();
		match p.as_slice() {
			["trail", f, h] => {
				if let Ok(f) = f.parse() {
					t.insert(f, h.to_string());
				}
			}
			["end", n, h] => end = n.parse().ok().map(|n| (n, h.to_string())),
			["stop", ..] => stop = Some(l.to_string()),
			_ => {}
		}
	}
	(t, end, stop)
}

fn one_window(win: &str, o: &Opts, d1: &BTreeMap<String, String>) -> Res {
	let dir = format!("{PLAY}/eudora1_{win}");
	let exe = std::env::current_exe().unwrap();
	let recorded: u32 = std::fs::read_to_string(format!("{dir}/origin.txt")).unwrap_or_default().lines().find_map(|l| l.strip_prefix("fields\t")?.trim().parse().ok()).unwrap_or(0);
	let n = if o.cap > 0 { o.cap.min(recorded) } else { recorded };
	let mut r = Res { win: win.into(), asked: n, d1: d1.get(win).cloned().unwrap_or_else(|| "?".into()), ..Default::default() };
	let pokes = pokes_of(&dir);
	r.pokes = pokes.len();
	r.first_poke = pokes.iter().copied().min();
	// 1. oracle baseline (cached by window + field count)
	let base_path = format!("{}/baseline/{win}.{n}.trail", o.out);
	let mut base = (BTreeMap::new(), None, None);
	if o.baseline {
		if o.rebase || !std::fs::read_to_string(&base_path).is_ok_and(|t| t.contains("end ") || t.contains("stop ")) {
			let mut c = std::process::Command::new(&exe);
			c.args(["--oracle", &dir, &n.to_string(), &base_path]);
			let (code, why) = run_with_limits(c, &format!("{}/{win}.oracle.log", o.out), o, "oracle");
			if !why.is_empty() || !std::fs::read_to_string(&base_path).is_ok_and(|t| t.contains("end ") || t.contains("stop ")) {
				r.detail = format!("baseline failed (exit {code:?} {why})");
			}
		}
		base = parse_trail(&std::fs::read_to_string(&base_path).unwrap_or_default());
	}
	// 2. the retail path
	let rec = format!("{}/{win}.rec", o.out);
	let _ = std::fs::remove_file(format!("{rec}.trail"));
	let mut c = std::process::Command::new(&exe);
	c.args(["--child", &dir, &n.to_string()]);
	if o.gsfull {
		c.arg("--gsfull");
	}
	c.env("RAC_RETAIL_SMOKE", n.to_string()).env("RAC_RETAIL_REC", &rec).env("RAC_RETAIL_REC_TRAIL", TRAIL_EVERY.to_string());
	for k in ["RAC_RETAIL_PAD", "RAC_RETAIL_TRAVEL", "RAC_RETAIL_REPLAY", "RAC_GSFULL"] {
		c.env_remove(k);
	}
	// Opt-in hook (documented in the handoff): retail.rs applies the recorded raw report and pokes when this is set.
	c.env("RAC_RETAIL_PLAY", "@play");
	if !o.hud {
		c.env("RAC_RETAIL_HUD", "0");
	}
	let log = format!("{}/{win}.log", o.out);
	let (code, why) = run_with_limits(c, &log, o, "retail");
	let text = std::fs::read_to_string(&log).unwrap_or_default();
	let (mut done, mut lastprog, mut stop, mut panic) = (None, 0u32, None, None);
	for l in text.lines() {
		if let Some(s) = l.strip_prefix("smoke: done ") {
			let nums: Vec<&str> = s.split_whitespace().collect();
			// "N fields | ee X ms/field | worker Y fields/s | ram H"
			done = Some((nums[0].parse::<u32>().unwrap_or(0), nums.get(4).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0), nums.get(8).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0), nums.last().unwrap_or(&"").to_string()));
		} else if let Some(s) = l.strip_prefix("smoke: field ") {
			lastprog = s.split_whitespace().next().and_then(|x| x.parse().ok()).unwrap_or(lastprog);
		} else if let Some(s) = l.strip_prefix("ACC stop ") {
			stop = Some(s.to_string());
		} else if let Some(s) = l.strip_prefix("ACC panic ") {
			panic = Some(s.to_string());
		} else if let Some(s) = l.strip_prefix("ACC pad ") {
			r.pad_diff = s.split("report_differs ").nth(1).and_then(|x| x.split_whitespace().next()?.parse().ok()).unwrap_or(0);
		} else if let Some(s) = l.strip_prefix("ACC prog ") {
			r.snd = s.split_once(' ').map(|x| x.1.to_string()).unwrap_or_default();
		} else if l.starts_with("# playfeed") {
			r.hooked = true;
		}
	}
	let (rtrail, _, _) = parse_trail(&std::fs::read_to_string(format!("{rec}.trail")).unwrap_or_default());
	let rtrail: BTreeMap<u32, String> = rtrail.into_iter().map(|(k, v)| (k, v.trim_start_matches("0x").to_string())).collect();
	if let Some((nd, ee, wfps, ram)) = &done {
		r.fields_done = *nd;
		r.ee_ms = *ee;
		r.worker_fps = *wfps;
		r.ram = ram.clone();
	} else {
		r.fields_done = lastprog;
	}
	if !why.is_empty() {
		r.verdict = "hung".into();
		r.detail = format!("{why}; last progress field {lastprog}");
		return r;
	}
	if let Some(p) = panic {
		r.verdict = "stopped".into();
		r.detail = format!("panic {p}");
		return r;
	}
	if let Some(s) = stop {
		r.verdict = "stopped".into();
		r.detail = s;
		return r;
	}
	let Some((_, _, _, ram)) = done else {
		r.verdict = "stopped".into();
		r.detail = format!("exit {code:?} without a smoke:done line (last progress field {lastprog})");
		return r;
	};
	// 3. compare with the oracle runner
	let (btrail, bend, bstop) = base;
	if let Some(s) = bstop {
		r.verdict = "no-baseline".into();
		r.detail = format!("oracle runner stopped: {s}");
		return r;
	}
	if !o.baseline || btrail.is_empty() {
		r.verdict = "ok-unchecked".into();
		r.detail = "no oracle baseline".into();
		return r;
	}
	let mut first_div = None;
	let mut matched = 0;
	for (f, h) in &rtrail {
		match btrail.get(f) {
			Some(b) if b == h => matched += 1,
			Some(_) => {
				first_div = Some(*f);
				break;
			}
			None => {}
		}
	}
	let end_ok = bend.as_ref().is_some_and(|(_, h)| *h == ram);
	r.oracle_end = bend.map(|x| x.1).unwrap_or_default();
	r.trail_ok = format!("{matched}/{} checkpoints, end {}", btrail.len(), if end_ok { "same" } else { "differs" });
	if first_div.is_none() && !end_ok {
		first_div = Some(n);
	}
	match first_div {
		None => r.verdict = "ok".into(),
		Some(f) => {
			let gap = !r.hooked && r.first_poke.is_some_and(|p| p <= f);
			r.verdict = if gap { "feed-gap" } else { "diverged" }.into();
			r.detail = format!("first differing checkpoint field {f}{}", if gap { format!(" (poke at field {} not fed: retail.rs hook missing)", r.first_poke.unwrap()) } else { String::new() });
		}
	}
	r
}

fn parent(a: &[String]) -> ExitCode {
	let mut o = Opts { out: concat!(env!("CARGO_MANIFEST_DIR"), "/../scratch_play_e2e/accept").into(), cap: 0, timeout: 1800, stall: 150, hud: true, gsfull: false, rebase: false, baseline: true };
	let (mut sel, mut jobs, mut allow_gap) = ("all".to_string(), 2usize, false);
	let mut i = 0;
	while i < a.len() {
		let nx = |i: usize| a.get(i + 1).cloned().unwrap_or_default();
		match a[i].as_str() {
			"--windows" => (sel, i) = (nx(i), i + 1),
			"--fields" => (o.cap, i) = (nx(i).parse().unwrap_or(0), i + 1),
			"-j" => (jobs, i) = (nx(i).parse().unwrap_or(2), i + 1),
			"--timeout" => (o.timeout, i) = (nx(i).parse().unwrap_or(1800), i + 1),
			"--stall" => (o.stall, i) = (nx(i).parse().unwrap_or(150), i + 1),
			"--out" => (o.out, i) = (nx(i), i + 1),
			"--nohud" => o.hud = false,
			"--gsfull" => o.gsfull = true,
			"--rebaseline" => o.rebase = true,
			"--no-baseline" => o.baseline = false,
			"--allow-feed-gap" => allow_gap = true,
			_ => {
				eprintln!("{USAGE}");
				return ExitCode::from(2);
			}
		}
		i += 1;
	}
	std::fs::create_dir_all(format!("{}/baseline", o.out)).ok();
	let mut wins: Vec<String> = std::fs::read_dir(PLAY).map(|d| d.filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().strip_prefix("eudora1_")?.to_string())).collect()).unwrap_or_default();
	wins.sort();
	if sel != "all" {
		wins.retain(|w| sel.split(',').any(|s| s == w));
	}
	if wins.is_empty() {
		eprintln!("no windows selected under {PLAY}");
		return ExitCode::from(2);
	}
	let d1: BTreeMap<String, String> = std::fs::read_to_string(D1).unwrap_or_default().lines().filter_map(|l| {
		let p: Vec<&str> = l.split('\t').collect();
		Some((p.first()?.to_string(), p.get(2)?.to_string()))
	}).collect();
	println!("# eudora_accept: {} windows, cap {}, {} job(s), hud {}, gsfull {}, out {}", wins.len(), if o.cap > 0 { o.cap.to_string() } else { "none".into() }, jobs, o.hud, o.gsfull, o.out);
	let queue = std::sync::Arc::new(std::sync::Mutex::new(wins.clone()));
	let results = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Res>::new()));
	let hs: Vec<_> = (0..jobs.max(1))
		.map(|_| {
			let (q, rs, o, d1) = (queue.clone(), results.clone(), o.clone(), d1.clone());
			std::thread::spawn(move || {
				loop {
					let w = {
						let mut q = q.lock().unwrap();
						if q.is_empty() {
							break;
						}
						q.remove(0)
					};
					let r = one_window(&w, &o, &d1);
					println!("{}", line(&r));
					rs.lock().unwrap().push(r);
				}
			})
		})
		.collect();
	for h in hs {
		let _ = h.join();
	}
	let mut rs = results.lock().unwrap().clone();
	rs.sort_by(|a, b| a.win.cmp(&b.win));
	let mut tsv = String::from("window\tverdict\tfields_done\tfields_asked\tee_ms_per_field\tworker_fields_per_s\tpokes\tpad_report_differs\thooked\ttrail\tram_hash\toracle_end_hash\td1_verdict\tdetail\n");
	for r in &rs {
		tsv += &format!("{}\t{}\t{}\t{}\t{:.2}\t{:.1}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", r.win, r.verdict, r.fields_done, r.asked, r.ee_ms, r.worker_fps, r.pokes, r.pad_diff, r.hooked, r.trail_ok, r.ram, r.oracle_end, r.d1, r.detail);
	}
	std::fs::write(format!("{}/report.tsv", o.out), tsv).ok();
	let c = |v: &str| rs.iter().filter(|r| r.verdict == v).count();
	let ran: Vec<&Res> = rs.iter().filter(|r| r.ram.len() > 0).collect();
	let mean = |f: &dyn Fn(&Res) -> f64| if ran.is_empty() { 0.0 } else { ran.iter().map(|r| f(r)).sum::<f64>() / ran.len() as f64 };
	let ee_fps = mean(&|r| if r.ee_ms > 0.0 { 1000.0 / r.ee_ms } else { 0.0 });
	let d1c = |v: &str| rs.iter().filter(|r| r.d1.starts_with(v)).count();
	let ok = c("ok");
	println!(
		"SUMMARY windows {}: ok {} (identical to oracle runner), stopped {}, hung {}, diverged {}, feed-gap {}, other {} | oracle-runner verdicts of these windows: none {} small {} desync {} | mean fields/s: EE thread {:.1}, worker wall {:.1} | report {}/report.tsv",
		rs.len(), ok, c("stopped"), c("hung"), c("diverged"), c("feed-gap"), rs.len() - ok - c("stopped") - c("hung") - c("diverged") - c("feed-gap"), d1c("none"), d1c("small"), d1c("desync"), ee_fps, mean(&|r| r.worker_fps), o.out
	);
	let fail = rs.len() - ok - if allow_gap { c("feed-gap") } else { 0 };
	std::io::stdout().flush().ok();
	if fail == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

fn line(r: &Res) -> String {
	format!(
		"{} {:<9} fields {}/{} ee {:.1} ms/f  worker {:.1} f/s  oracle-runner {}  pokes {} pad-report-differs {}  {}{}{}",
		r.win, r.verdict, r.fields_done, r.asked, r.ee_ms, r.worker_fps, r.d1, r.pokes, r.pad_diff, r.trail_ok, if r.detail.is_empty() { "" } else { "  | " }, r.detail
	)
}
