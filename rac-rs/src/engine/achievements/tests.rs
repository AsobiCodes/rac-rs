use super::store::DEBOUNCE;
use super::*;
use std::collections::VecDeque;

const fn d(id: &'static str, grade: Grade, max: u32, platinum: bool) -> Def {
	Def { id, name: id, desc: "a \"quoted\" description", grade, hidden: false, max, platinum }
}

static DEFS: [Def; 4] = [d("a", Grade::Bronze, 0, false), d("b", Grade::Silver, 10, false), d("c", Grade::Gold, 0, false), d("p", Grade::Platinum, 0, true)];

fn pack() -> AchPack {
	AchPack { game: "test", defs: &DEFS, read: |_| None, icons: None }
}

fn met(a: bool, b: u32, c: bool) -> Vec<Reading> {
	vec![Reading { met: a, ..Reading::default() }, Reading { met: b >= 10, progress: b, epoch: None }, Reading { met: c, ..Reading::default() }, Reading::default()]
}

fn tmp(name: &str) -> std::path::PathBuf {
	let p = std::env::temp_dir().join(format!("rac_ach_test_{}_{name}", std::process::id()));
	let _ = std::fs::create_dir_all(&p);
	p.join("achievements.json")
}

/// Waits for the writer thread to drain (true = it did within 10 s).
fn settle() -> bool {
	super::writer::wait_idle(std::time::Duration::from_secs(10))
}

fn run(s: &mut Store, r: &[Reading], fields: u16, now: u64) -> Vec<usize> {
	let mut all = Vec::new();
	for _ in 0..fields {
		all.extend(s.apply(r, now).0);
	}
	all
}

#[test]
fn unlock_needs_a_steady_condition() {
	let mut s = Store::new(&pack());
	assert!(run(&mut s, &met(true, 0, false), DEBOUNCE - 1, 100).is_empty());
	// A dropout resets the count.
	run(&mut s, &met(false, 0, false), 1, 100);
	assert!(run(&mut s, &met(true, 0, false), DEBOUNCE - 1, 100).is_empty());
	assert_eq!(run(&mut s, &met(true, 0, false), 1, 100), vec![0]);
	assert!(s.entries[0].unlocked && s.entries[0].at == 100);
	// Unlocked stays unlocked and is reported once.
	assert!(run(&mut s, &met(false, 0, false), 50, 200).is_empty());
	assert!(s.entries[0].unlocked && s.entries[0].at == 100);
}

#[test]
fn progress_only_grows_and_caps() {
	let mut s = Store::new(&pack());
	run(&mut s, &met(false, 4, false), DEBOUNCE, 1);
	run(&mut s, &met(false, 2, false), DEBOUNCE, 2);
	assert_eq!(s.entries[1].progress, 4);
	run(&mut s, &met(false, 99, false), DEBOUNCE, 3);
	assert_eq!(s.entries[1].progress, 10);
}

/// m4: progress is stored only after the same (or a higher) value held `DEBOUNCE` fields; a one-field spike changes nothing, and a
/// value that keeps rising is stored at the lowest value of each run.
#[test]
fn progress_needs_a_steady_value() {
	let mut s = Store::new(&pack());
	run(&mut s, &met(false, 4, false), DEBOUNCE - 1, 1);
	assert_eq!(s.entries[1].progress, 0, "one field short: not stored");
	assert!(s.apply(&met(false, 4, false), 1).1, "the DEBOUNCE-th field stores it and asks for a save");
	assert_eq!(s.entries[1].progress, 4);
	// A one-field spike (the half-built RAM of a load) does not raise it; the steady value after it ends the run.
	assert!(!s.apply(&met(false, 9, false), 2).1);
	run(&mut s, &met(false, 4, false), 5 * DEBOUNCE, 3);
	assert_eq!(s.entries[1].progress, 4, "the spike is gone");
	// A spike inside a run of 20 fields above the stored value: the run is stored at its lowest value.
	run(&mut s, &met(false, 6, false), DEBOUNCE / 2 - 1, 4);
	s.apply(&met(false, 9, false), 5);
	run(&mut s, &met(false, 6, false), DEBOUNCE / 2 - 1, 6);
	assert_eq!(s.entries[1].progress, 4, "19 fields of the run: not yet");
	s.apply(&met(false, 6, false), 7);
	assert_eq!(s.entries[1].progress, 6, "20 fields all above 4: stored at the run's lowest value, not at 9");
	// A rising counter ends at its final value: 1..=10 (one field each), then 10 held.
	let mut r = Store::new(&pack());
	for v in 1..=10 {
		r.apply(&met(false, v, false), 8);
	}
	run(&mut r, &met(false, 10, false), 10, 8);
	assert_eq!(r.entries[1].progress, 1, "the first 20 fields were all above 0: stored at their lowest value");
	run(&mut r, &met(false, 10, false), DEBOUNCE, 9);
	assert_eq!(r.entries[1].progress, 10);
}

/// m4: a spike never reaches `carry`: the next cycle starts from the debounced total.
#[test]
fn a_spike_is_not_carried_into_the_next_cycle() {
	let rd = |n: u32, ep: u32| vec![Reading::default(), Reading { met: false, progress: n, epoch: Some(ep) }, Reading::default(), Reading::default()];
	let mut s = Store::new(&pack());
	run(&mut s, &rd(3, 0), DEBOUNCE, 1);
	s.apply(&rd(900_000, 0), 2);
	s.apply(&rd(0, 1), 3);
	assert_eq!((s.entries[1].carry, s.entries[1].epoch, s.entries[1].progress), (3, 1, 3), "carry 3, not the spike (it would have been the goal, 10)");
	run(&mut s, &rd(2, 1), DEBOUNCE, 4);
	assert_eq!(s.entries[1].progress, 5, "3 + 2");
}

#[test]
fn platinum_follows_the_others() {
	let mut s = Store::new(&pack());
	let all = run(&mut s, &met(true, 10, true), DEBOUNCE, 7);
	assert_eq!(all, vec![0, 1, 2, 3], "platinum comes last, in the same field");
	assert_eq!(s.counts(), (4, 4));
}

#[test]
fn file_round_trip() {
	let mut s = Store::new(&pack());
	run(&mut s, &met(true, 6, false), DEBOUNCE, 1_762_084_800);
	let path = tmp("round");
	s.save(&path, 1_762_084_900).unwrap();
	assert!(!super::writer::tmp_path(&path).exists(), "the temp file is renamed away");
	let text = std::fs::read_to_string(&path).unwrap();
	assert!(text.contains("\"unlocked_at\": \"2025-11-02T12:00:00Z\""), "{text}");
	assert!(text.contains("\"progress\": 6, \"progress_max\": 10"));
	let mut t = Store::new(&pack());
	t.load(&path);
	assert_eq!(t.entries, s.entries);
	// A second save replaces the file.
	s.save(&path, 1_762_084_901).unwrap();
}

#[test]
fn broken_and_foreign_files_are_safe() {
	let path = tmp("broken");
	for bad in ["", "{", "[1,2]", "{\"version\":2,\"game\":\"test\",\"achievements\":[]}", "{\"version\":1,\"game\":\"other\",\"achievements\":[]}", "\u{0}\u{1}garbage"] {
		std::fs::write(&path, bad).unwrap();
		let mut s = Store::new(&pack());
		s.load(&path);
		assert_eq!(s.counts().0, 0, "{bad:?}");
		assert!(path.with_extension("json.broken").exists());
		let _ = std::fs::remove_file(path.with_extension("json.broken"));
	}
	// Missing file: nothing, no panic.
	let mut s = Store::new(&pack());
	s.load(&tmp("missing"));
	assert_eq!(s.counts(), (0, 4));
	// Unknown ids and extra keys are ignored, a bad date keeps the unlock.
	let mut s = Store::new(&pack());
	assert!(s.load_text("{\"version\":1,\"game\":\"test\",\"x\":5,\"achievements\":[{\"id\":\"zz\",\"unlocked\":true},{\"id\":\"c\",\"unlocked\":true,\"unlocked_at\":\"soon\"},{\"id\":\"b\",\"progress\":3}]}"));
	assert!(s.entries[2].unlocked && s.entries[2].at == 0);
	assert_eq!(s.entries[1].progress, 3);
}

/// B14: a damaged file is kept as `.broken` and never overwritten; later damaged files get numbers; the same bytes are not kept twice.
#[test]
fn broken_copies_are_kept_and_never_overwritten() {
	let path = tmp("keep");
	let kept = |n: &str| std::fs::read_to_string(path.with_extension(format!("json.broken{n}"))).ok();
	for (bad, first, second, third) in [("AAA", Some("AAA"), None, None), ("BBB", Some("AAA"), Some("BBB"), None), ("BBB", Some("AAA"), Some("BBB"), None), ("AAA", Some("AAA"), Some("BBB"), None), ("CCC", Some("AAA"), Some("BBB"), Some("CCC"))] {
		std::fs::write(&path, bad).unwrap();
		let mut s = Store::new(&pack());
		s.load(&path);
		assert_eq!(s.counts().0, 0, "the list starts empty");
		assert_eq!((kept("").as_deref(), kept(".2").as_deref(), kept(".3").as_deref()), (first, second, third), "after loading {bad}");
	}
	assert_eq!(std::fs::read_to_string(&path).unwrap(), "CCC", "loading does not touch the damaged file itself (the next write replaces it)");
}

/// m5: the writer thread keeps the latest text per path, writes through a per-process tmp file (none left behind), and a path it cannot
/// write is reported once so the hub writes the whole state again.
#[test]
fn the_writer_thread_writes_the_latest_text_atomically() {
	let path = tmp("writer");
	let tmp_name = super::writer::tmp_path(&path).file_name().unwrap().to_string_lossy().into_owned();
	assert_eq!(tmp_name, format!("achievements.json.{}.tmp", std::process::id()), "unique per process");
	for i in 0..50 {
		super::writer::submit(path.clone(), format!("text {i}"));
	}
	assert!(settle());
	assert_eq!(std::fs::read_to_string(&path).unwrap(), "text 49", "the latest text wins");
	let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
	assert!(leftovers.is_empty(), "no tmp file stays: {leftovers:?}");
	assert!(!super::writer::take_failed(&path));
	// A folder that cannot be made (a plain file is in the way): the write fails once, is reported once.
	let blocker = tmp("blocker");
	std::fs::write(blocker.parent().unwrap().join("dir"), "a file, not a folder").unwrap();
	let bad = blocker.parent().unwrap().join("dir").join("achievements.json");
	super::writer::submit(bad.clone(), "x".into());
	assert!(settle());
	assert!(super::writer::take_failed(&bad) && !super::writer::take_failed(&bad), "reported once");
	// The hub then writes the whole state again at its next flush: dirty stays set through the 10 s throttle, and the write happens.
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let mut h = Hub { pack: p, store: Store::new(p), path: bad.clone(), queue: VecDeque::new(), last_save: None, dirty: false, unreadable: false };
	h.flush(true);
	assert!(settle());
	h.observe(&met(false, 0, false), 1);
	assert!(h.dirty, "the failed write marks the hub dirty again");
	std::fs::remove_file(blocker.parent().unwrap().join("dir")).unwrap();
	h.last_save = None;
	h.observe(&met(false, 0, false), 2);
	assert!(settle());
	assert!(!h.dirty && bad.exists(), "the repeat write went through");
}

/// Data safety: only NotFound means "start a new file". A file that exists but cannot be read is tried again, and if it still fails
/// the hub never writes over it (read seam: `Store::load_via`).
#[test]
fn an_unreadable_file_is_never_overwritten() {
	use std::io::{Error, ErrorKind};
	let none = std::time::Duration::ZERO;
	let path = tmp("locked");
	std::fs::write(&path, "KEEP ME: a good file another program holds").unwrap();
	// Every read fails with a sharing violation / permission error: 4 tries, then Unreadable, list empty.
	let mut calls = 0;
	let mut s = Store::new(&pack());
	let got = s.load_via(&path, &mut |_| { calls += 1; Err(Error::new(ErrorKind::PermissionDenied, "locked")) }, 4, none);
	assert_eq!((got, calls, s.counts().0), (Loaded::Unreadable, 4, 0));
	// A read that works on the second try is a normal load (the first error was only a moment).
	let mut good = Store::new(&pack());
	good.entries[0].unlocked = true;
	let j = good.to_json(1);
	let mut calls = 0;
	let mut s = Store::new(&pack());
	let got = s.load_via(&path, &mut |_| { calls += 1; if calls < 2 { Err(Error::new(ErrorKind::PermissionDenied, "busy")) } else { Ok(j.clone().into_bytes()) } }, 4, none);
	assert_eq!((got, calls), (Loaded::Read, 2));
	assert_eq!(s.counts().0, 1, "the retried read loaded the entries");
	// NotFound is the only "new file": no retry.
	let mut calls = 0;
	let mut s = Store::new(&pack());
	let got = s.load_via(&path, &mut |_| { calls += 1; Err(Error::new(ErrorKind::NotFound, "gone")) }, 4, none);
	assert_eq!((got, calls), (Loaded::New, 1));
	// The hub of an unreadable file writes nothing, however often it is flushed or an unlock happens; the file keeps its bytes.
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let mut h = Hub { pack: p, store: Store::new(p), path: path.clone(), queue: VecDeque::new(), last_save: None, dirty: true, unreadable: true };
	h.flush(true);
	for _ in 0..DEBOUNCE + 1 {
		h.observe(&met(true, 0, false), 5);
	}
	assert!(!h.queue.is_empty(), "the pop-up still shows");
	assert!(settle());
	assert_eq!(std::fs::read_to_string(&path).unwrap(), "KEEP ME: a good file another program holds");
	assert!(!super::writer::tmp_path(&path).exists() && !h.dirty);
}

/// Stale `achievements.json.<pid>.tmp` of another process are removed at init; nothing else in the folder is touched.
#[test]
fn stale_tmp_files_of_other_processes_are_removed() {
	let path = tmp("stale");
	let dir = path.parent().unwrap().to_path_buf();
	for f in std::fs::read_dir(&dir).unwrap().flatten() {
		let _ = std::fs::remove_file(f.path());
	}
	let go = [("achievements.json.1.tmp", true), ("achievements.json.99999.tmp", true), ("achievements.json.4242.tmp", false), ("achievements.json.abc.tmp", false), ("achievements.json..tmp", false), ("achievements.json.12x.tmp", false), ("other.json.5.tmp", false), ("achievements.json", false), ("achievements.json.broken", false), ("achievements.json.5.tmp.bak", false), ("achievements.json.5", false)];
	for (name, _) in go {
		std::fs::write(dir.join(name), "x").unwrap();
	}
	std::fs::create_dir_all(dir.join("achievements.json.7.tmp")).unwrap();
	assert_eq!(super::writer::remove_stale_tmp(&path, 4242), 2);
	for (name, removed) in go {
		assert_eq!(dir.join(name).exists(), !removed, "{name}");
	}
	assert!(dir.join("achievements.json.7.tmp").is_dir(), "a folder with that name stays");
	assert_eq!(super::writer::remove_stale_tmp(&dir.join("missing_folder").join("achievements.json"), 4242), 0);
}

#[test]
fn dates_round_trip() {
	for t in [0u64, 86399, 951_782_400, 1_709_209_800, 1_762_084_800, 4_102_444_799] {
		assert_eq!(iso_to_unix(&unix_to_iso(t)), Some(t), "{t}");
	}
	assert_eq!(unix_to_iso(1_709_164_800), "2024-02-29T00:00:00Z");
	assert_eq!(iso_to_unix("2026-13-01T00:00:00Z"), None);
	assert_eq!(iso_to_unix("nope"), None);
}

#[test]
fn toast_queue_and_phases() {
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let mut h = Hub { pack: p, store: Store::new(p), path: tmp("hub"), queue: VecDeque::new(), last_save: None, dirty: false, unreadable: false };
	for _ in 0..DEBOUNCE {
		h.observe(&met(true, 0, true), 5);
	}
	assert_eq!(h.queue.iter().copied().collect::<Vec<_>>(), vec![0, 2], "each unlock queues once, in order");
	assert!(settle(), "the writer thread finished");
	assert!(h.path.exists(), "an unlock is written at once (by the writer thread)");
	let (a0, s0) = toast::phase(0.0).unwrap();
	assert!(a0 < 0.01 && s0 < 0.01);
	let (a1, s1) = toast::phase(2.5).unwrap();
	assert!(a1 > 0.99 && s1 > 0.99);
	assert!(toast::phase(4.9).unwrap().0 < 0.3);
	assert!(toast::phase(5.0).is_none() && toast::phase(-1.0).is_none());
	assert!(toast::fit("These are not the droids you're looking for...", 300.0, 20.0) < 10.0);
}

/// A modern page holds the pop-up: nothing shows, the queue waits, and the pop-up that was up starts over when the page closes.
#[test]
fn a_held_toast_waits_and_starts_over() {
	let t0 = std::time::Instant::now();
	let at = |s: f32| t0 + std::time::Duration::from_secs_f32(s);
	let mut q: VecDeque<usize> = VecDeque::from([3, 4]);
	let mut cur = None;
	let mut step = |held: bool, s: f32| toast::tick(&mut cur, held, at(s), || q.pop_front()).map(|(i, age)| (i, (age * 10.0).round() / 10.0));
	assert_eq!(step(false, 0.0), Some((3, 0.0)));
	assert_eq!(step(false, 4.0), Some((3, 4.0)));
	assert_eq!(step(true, 4.5), None, "a page opens: nothing shows");
	assert_eq!(step(true, 30.0), None, "the queue waits however long the page stays");
	assert_eq!(step(false, 31.0), Some((3, 1.0)), "the same pop-up again, from its start");
	assert_eq!(step(false, 36.5), Some((4, 0.0)), "then the next one");
	assert_eq!(step(false, 42.0), None);
}

#[test]
fn cycle_counters_carry_the_finished_cycles() {
	let rd = |n: u32, ep: u32| vec![Reading::default(), Reading { met: false, progress: n, epoch: Some(ep) }, Reading::default(), Reading::default()];
	let mut s = Store::new(&pack());
	run(&mut s, &rd(4, 0), DEBOUNCE, 1);
	assert_eq!(s.entries[1].progress, 4);
	// New cycle: the game's counter restarts at 0, the total carries on.
	s.apply(&rd(0, 1), 2);
	assert_eq!(s.entries[1].progress, 4);
	run(&mut s, &rd(3, 1), DEBOUNCE, 3);
	assert_eq!(s.entries[1].progress, 7);
	// An earlier save (older cycle) neither lowers nor inflates it.
	run(&mut s, &rd(9, 0), DEBOUNCE, 4);
	assert_eq!(s.entries[1].progress, 7);
	// The file keeps the carry: a restart in the same cycle gives the same total, the next cycle adds to it.
	let path = tmp("cycles");
	s.save(&path, 5).unwrap();
	let mut t = Store::new(&pack());
	t.load(&path);
	assert_eq!((t.entries[1].carry, t.entries[1].epoch, t.entries[1].progress), (4, 1, 7));
	run(&mut t, &rd(5, 1), DEBOUNCE, 6);
	assert_eq!(t.entries[1].progress, 9);
	// The goal is reached over cycles: unlocks after the debounce although each cycle alone is short (carry 9 + 1).
	let all = run(&mut t, &rd(1, 2), DEBOUNCE, 9);
	assert_eq!((all, t.entries[1].carry, t.entries[1].progress), (vec![1], 9, 10));
}

#[test]
fn pad_scrolls_pages_and_closes() {
	use super::screen::{pad_step, PadStep};
	const UP: u16 = 0x1000;
	const DOWN: u16 = 0x4000;
	let idle = [0x80; 4];
	let mut st = PadStep { now: 0, hold: 0, close: false, rows: 0, pages: 0 };
	let mut rows = Vec::new();
	// Hold Down: one row at once, then nothing for 20 fields, then every 4th.
	for _ in 0..30 {
		st = pad_step(st.now, DOWN, idle, st.hold);
		rows.push(st.rows);
	}
	assert_eq!(rows[0], 1);
	assert!(rows[1..20].iter().all(|&r| r == 0));
	assert_eq!(rows.iter().filter(|&&r| r == 1).count(), 3, "the press, then fields 24 and 28");
	// The left stick pushed up scrolls up; centre stops it; a stick inside the dead zone does nothing.
	st = pad_step(0, 0, [0x80, 0x80, 0x80, 0x20], 0);
	assert_eq!(st.rows, -1);
	assert_eq!(pad_step(st.now, 0, [0x80, 0x80, 0x80, 0x90], st.hold).rows, 0);
	// L1 / R1: a page on the press only.
	assert_eq!(pad_step(0, 0x08, idle, 0).pages, 1);
	assert_eq!(pad_step(0x08, 0x08, idle, 0).pages, 0);
	assert_eq!(pad_step(0, 0x04, idle, 0).pages, -1);
	// Circle or Triangle close, only as a new press; Cross and the others do not.
	assert!(pad_step(0, 0x20, idle, 0).close && pad_step(0, 0x10, idle, 0).close);
	assert!(!pad_step(0x20, 0x20, idle, 0).close && !pad_step(0, 0x40 | UP, idle, 0).close);
}

/// M2 / m6: once cheating tools were on, the latch stays on for the session whatever the switch does afterwards; F6 follows the
/// debug keys live.
#[test]
fn the_cheat_latch_stays_on_and_f6_follows_the_switch() {
	let l = Latch::new();
	assert!(!l.cheated() && !l.owns_f6());
	l.note(false, false);
	assert!(!l.cheated());
	l.note(true, true);
	assert!(l.cheated() && l.owns_f6());
	// Debug mode switched Off again: F6 is the list's again, but nothing unlocks for the rest of the session.
	l.note(false, false);
	assert!(l.cheated(), "latched");
	assert!(!l.owns_f6());
	// Dev tools without the debug keys: latched, F6 stays the list's.
	let t = Latch::new();
	t.note(true, false);
	assert!(t.cheated() && !t.owns_f6());
}

#[test]
fn the_hint_names_f6_only_while_it_opens_the_list_and_says_when_cheated() {
	use super::screen::{hint_text, CHEAT_LINE};
	assert_eq!(hint_text(3, 35, false, false), "F6  ACHIEVEMENTS  3/35");
	assert_eq!(hint_text(3, 35, true, false), "ACHIEVEMENTS  3/35");
	assert!(hint_text(3, 35, false, true).ends_with("OFF: DEBUG MODE"));
	assert!(CHEAT_LINE.is_ascii() && CHEAT_LINE.contains("OFF") && CHEAT_LINE.contains("DEBUG"));
}

/// B16: the longest name wraps to two lines at a space, readable (floor 14 px per 1080 lines), instead of shrinking to 9 px.
#[test]
fn a_long_pop_up_name_wraps_at_a_space_and_stays_readable() {
	use super::toast::{fit, name_lines};
	let name = "These are not the droids you're looking for...";
	assert_eq!(name.chars().count(), 46);
	for k in [0.6f32, 1.0, 2.0, 3.0] {
		let avail = 326.0 * k;
		assert!(fit(name, avail, 20.0 * k) < 14.0 * k, "one line would be unreadable at scale {k}");
		let (lines, size) = name_lines(name, avail, 20.0 * k, 14.0 * k, 15.0 * k);
		assert_eq!(lines, ["These are not the droids", "you're looking for..."], "scale {k}");
		assert!(size >= 14.0 * k - 1e-3 && size <= 15.0 * k + 1e-3, "size {size} at scale {k}");
		assert!(lines.iter().all(|l| crate::overlay::text_width(l, size) <= avail + 1e-2), "both lines fit");
	}
	// Names that fit stay on one line at today's size; a name without a space stays on one line.
	let (l, s) = name_lines("Skillful", 326.0, 20.0, 14.0, 15.0);
	assert_eq!((l, s), (vec!["Skillful"], 20.0));
	let (l, _) = name_lines("Supercalifragilisticexpialidocious_trophy_name", 326.0, 20.0, 14.0, 15.0);
	assert_eq!(l.len(), 1);
}

/// The quit path (`quit::quit_app`, Alt+F4 too): a dirty hub is written at once although the 10 s debounce would skip it, a clean hub
/// or no hub is left alone, and a worker that holds the hub (it is writing the file) is waited for, but only for a bounded time.
#[test]
fn the_quit_flush_writes_at_once_and_waits_for_a_busy_writer_for_a_bounded_time() {
	use std::sync::Arc;
	use std::time::Duration;
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let path = tmp("quit_flush");
	let _ = std::fs::remove_file(&path);
	let mut h = Hub { pack: p, store: Store::new(p), path: path.clone(), queue: VecDeque::new(), last_save: Some(Instant::now()), dirty: true, unreadable: false };
	run(&mut h.store, &met(false, 4, false), DEBOUNCE, 1);
	h.flush(false);
	assert!(settle() && !path.exists(), "the debounce skips an ordinary flush");
	let hub = Arc::new(Mutex::new(Some(h)));
	assert!(flush_in(&hub, Duration::from_millis(100)) && settle());
	let mut back = Store::new(p);
	back.load(&path);
	assert_eq!(back.entries[1].progress, 4, "the counter is on disk");
	assert!(!hub.lock().unwrap().as_ref().unwrap().dirty);
	// Nothing changed since: no second write.
	let _ = std::fs::remove_file(&path);
	assert!(flush_in(&hub, Duration::from_millis(100)) && settle() && !path.exists(), "a clean hub is not rewritten");
	assert!(flush_in(&Mutex::new(None), Duration::from_millis(10)), "no hub: nothing to flush, quit goes on");
	// The worker is inside a write: the quit waits for it, then flushes.
	hub.lock().unwrap().as_mut().unwrap().dirty = true;
	let busy = |ms: u64| {
		let (tx, rx) = std::sync::mpsc::channel();
		let h2 = hub.clone();
		let t = std::thread::spawn(move || {
			let g = h2.lock().unwrap();
			tx.send(()).unwrap();
			std::thread::sleep(Duration::from_millis(ms));
			drop(g);
		});
		rx.recv().unwrap();
		t
	};
	let t = busy(150);
	let t0 = Instant::now();
	assert!(flush_in(&hub, Duration::from_secs(5)));
	assert!(t0.elapsed() >= Duration::from_millis(100) && settle() && path.exists(), "it waited for the worker, then wrote");
	t.join().unwrap();
	// A hub that stays busy does not hang the quit.
	hub.lock().unwrap().as_mut().unwrap().dirty = true;
	let t = busy(500);
	let t0 = Instant::now();
	assert!(!flush_in(&hub, Duration::from_millis(40)), "gave up");
	assert!(t0.elapsed() < Duration::from_millis(400), "the wait is bounded");
	t.join().unwrap();
}

/// R6-4: a write that failed just before the quit (the launcher or a virus scanner held the file) is written again by the quit, which
/// the worker's next field never does; a folder that stays unwritable answers false after a few bounded tries.
#[test]
fn the_quit_flush_writes_again_after_a_failed_write() {
	use std::time::Duration;
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let base = tmp("quit_retry");
	let held = base.parent().unwrap().join("held");
	let _ = std::fs::remove_dir_all(&held);
	std::fs::write(&held, "a file where the folder should be").unwrap();
	let path = held.join("achievements.json");
	let hub = Mutex::new(Some(Hub { pack: p, store: Store::new(p), path: path.clone(), queue: VecDeque::new(), last_save: None, dirty: true, unreadable: false }));
	let t0 = Instant::now();
	assert!(!flush_retrying(&hub, Duration::from_secs(5)), "every try fails: false");
	assert!(t0.elapsed() < Duration::from_secs(2), "a few short tries, not the whole wait");
	assert!(!hub.lock().unwrap().as_ref().unwrap().dirty && super::writer::failed(&path), "the hub is clean, only the writer knows");
	std::fs::remove_file(&held).unwrap();
	assert!(flush_retrying(&hub, Duration::from_secs(5)) && path.exists(), "the failed write is handed over again and lands");
	assert!(!super::writer::failed(&path));
	assert!(flush_retrying(&Mutex::new(None), Duration::from_millis(10)), "no hub: nothing to flush");
}

/// R6-2: with the debug keys on, F6 in the pause menu is theirs (infinite ammo), not the list's, as the hint line already says.
#[test]
fn f6_opens_the_list_only_in_the_pause_menu_and_never_with_the_debug_keys() {
	assert!(super::screen::f6_opens_list(true, false));
	assert!(!super::screen::f6_opens_list(true, true), "debug keys on: F6 is theirs");
	assert!(!super::screen::f6_opens_list(false, false), "not paused: the game's");
}

/// The list's statics (OPEN, HOSTED, the pad's EAT / LAST) and the session hub (HUB) are shared: the tests that use them take this.
static SCREEN: Mutex<()> = Mutex::new(());

/// Review R22 m5: while a game's page hosts the open list (`set_hosted`), `ui_key` keeps only F6 (it closes the list), `ui_pad`
/// leaves the pad to the page's host and `draw` draws neither the list nor the hint; not hosted, the HUD list has all three again.
#[test]
fn a_hosted_list_leaves_keys_pad_and_picture_to_the_page() {
	use super::screen::{close, draw_in, is_open, key_in, open, pad_in};
	use winit::keyboard::KeyCode as K;
	let _g = SCREEN.lock().unwrap_or_else(|e| e.into_inner());
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	*hub() = Some(Hub { pack: p, store: Store::new(p), path: tmp("hosted"), queue: VecDeque::new(), last_save: None, dirty: false, unreadable: true });
	let quads = || {
		let mut out = Vec::new();
		draw_in(&mut out, 1280.0, 720.0, true);
		out.len()
	};
	let idle = [0x80; 4];
	open();
	set_hosted(true);
	assert!(!key_in(K::ArrowDown, true, false, true) && !key_in(K::Escape, true, false, true), "the page's keys");
	assert!(is_open(), "Esc is the page's Back, not this list's");
	assert!(!pad_in(0x20, idle) && is_open(), "Circle: the page's host has the pad");
	assert!(!pad_in(0, idle));
	assert_eq!(quads(), 0, "neither the list nor the hint under the page");
	assert!(key_in(K::F6, true, false, true) && !is_open(), "F6 still closes the list");
	assert_eq!(quads(), 0, "no hint while hosted");
	// The HUD list (the Classic style, the page given back): its picture, its keys, its pad.
	set_hosted(false);
	assert!(quads() > 0, "the hint");
	open();
	assert!(quads() > 0, "the list");
	assert!(key_in(K::ArrowDown, true, false, true), "the list's key");
	assert!(pad_in(0x20, idle) && !is_open(), "Circle closes the HUD list");
	assert!(!pad_in(0, idle), "released");
	close();
	*hub() = None;
}

/// Review R22 m1 / m5: a session cheated from its first field loads the hub (the list shows what was earned before) and writes
/// nothing: no new file or folder, the file keeps its bytes and time, a broken file is neither replaced nor copied aside, no write
/// at the quit either. Not cheated, the first field makes a missing file at once (as before).
#[test]
fn a_cheated_field_loads_the_hub_and_writes_nothing() {
	use std::time::Duration;
	let p: &'static AchPack = Box::leak(Box::new(pack()));
	let ram = [0u8; 4];
	let flushed = |hub: &Mutex<Option<Hub>>| flush_in(hub, Duration::from_millis(100)) && settle();
	// No file, in a folder that is not there.
	let dir = tmp("cheated_new").parent().unwrap().join("sub");
	let _ = std::fs::remove_dir_all(&dir);
	let path = dir.join("achievements.json");
	let hub = Mutex::new(None);
	assert!(field_in(p, &hub, || path.clone(), true, &ram), "loaded");
	assert!(!field_in(p, &hub, || path.clone(), true, &ram), "once");
	assert_eq!(hub.lock().unwrap().as_ref().map(|h| h.store.counts().0), Some(0));
	assert!(flushed(&hub) && !dir.exists(), "no folder, no file, not even at the quit");
	// An earned entry on disk: it shows; the file keeps its bytes and its time.
	let path = tmp("cheated_old");
	let mut s = Store::new(p);
	run(&mut s, &met(true, 0, false), DEBOUNCE, 1);
	s.save(&path, 1).unwrap();
	let look = |path: &std::path::Path| (std::fs::read(path).unwrap(), std::fs::metadata(path).unwrap().modified().unwrap());
	let before = look(&path);
	let hub = Mutex::new(None);
	assert!(field_in(p, &hub, || path.clone(), true, &ram));
	assert_eq!(hub.lock().unwrap().as_ref().map(|h| h.store.counts().0), Some(1), "the earned one shows");
	assert!(flushed(&hub));
	assert!(look(&path) == before, "untouched");
	// A broken file: the list starts empty, the file stays as it is, no copy is made.
	let path = tmp("cheated_broken");
	let copy = path.with_extension("json.broken");
	let _ = std::fs::remove_file(&copy);
	std::fs::write(&path, b"{ not json").unwrap();
	let hub = Mutex::new(None);
	assert!(field_in(p, &hub, || path.clone(), true, &ram) && flushed(&hub));
	assert_eq!(hub.lock().unwrap().as_ref().map(|h| h.store.counts().0), Some(0));
	assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
	assert!(!copy.exists(), "no broken copy");
	// Not cheated: the missing file is made at the first field.
	let hub = Mutex::new(None);
	assert!(field_in(p, &hub, || dir.join("achievements.json"), false, &ram) && settle());
	assert!(dir.join("achievements.json").exists());
	let _ = std::fs::remove_dir_all(&dir);
}
