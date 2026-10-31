//! [PC] Linux: a message window for things the player must read when the game was started from the applications menu, Steam or the
//! launcher (no terminal). A start that failed ("your disc image moved", "no graphics driver") used to show nothing at all: the window
//! never opened and the text went to a stderr nobody reads. This asks the helpers a desktop usually has, in the order that fits the
//! desktop (zenity on GNOME, kdialog on KDE and the Steam Deck's Desktop Mode, then xmessage, then notify-send), and does not wait on
//! more than one. No new library: the helper is a spawned program, found on PATH. Nothing found, or no desktop: the text stays in
//! stderr (and so in player/log.txt) as before.
//!
//! The logic (which program, which arguments, was it shown) is plain functions that compile and are tested on every system; only
//! `show` spawns programs. Windows has its own MessageBox and never calls this.
#![cfg_attr(windows, allow(dead_code))]

use std::path::PathBuf;

/// How serious the text is (Windows MessageBox icons: error 0x10, warning 0x30, information 0x40).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
	Info,
	Warning,
	Error,
}

/// The kind of a Windows `MB_*` flag word (`gscap_guard::message_box` passes those on every system).
pub(crate) fn kind_from_flags(flags: u32) -> Kind {
	match flags & 0xF0 {
		0x10 => Kind::Error,
		0x30 => Kind::Warning,
		_ => Kind::Info,
	}
}

/// The helpers to try, best first for this desktop (`XDG_CURRENT_DESKTOP`, e.g. "KDE" or "ubuntu:GNOME").
pub(crate) fn tool_order(desktop: &str) -> [&'static str; 4] {
	if desktop.to_ascii_uppercase().contains("KDE") {
		["kdialog", "zenity", "xmessage", "notify-send"]
	} else {
		["zenity", "kdialog", "xmessage", "notify-send"]
	}
}

/// The program and its arguments that show `text` with helper `tool`; None for an unknown helper.
pub(crate) fn command(tool: &str, title: &str, text: &str, kind: Kind) -> Option<Vec<String>> {
	let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
	let mut a = match tool {
		"zenity" => s(&[tool, match kind {
			Kind::Info => "--info",
			Kind::Warning => "--warning",
			Kind::Error => "--error",
		}, "--no-markup", "--width=520", "--title"]),
		"kdialog" => s(&[tool, match kind {
			Kind::Info => "--msgbox",
			Kind::Warning => "--sorry",
			Kind::Error => "--error",
		}]),
		"xmessage" => s(&[tool, "-center", "-buttons", "OK:0", "-default", "OK", "-title"]),
		"notify-send" => s(&[tool, "-a", "rac-rs", "-u", if kind == Kind::Error { "critical" } else { "normal" }]),
		_ => return None,
	};
	match tool {
		"zenity" => {
			a.push(title.to_string());
			a.push("--text".to_string());
			a.push(text.to_string());
		}
		"kdialog" => {
			a.push(text.to_string());
			a.push("--title".to_string());
			a.push(title.to_string());
		}
		"xmessage" => {
			a.push(title.to_string());
			a.push(text.to_string());
		}
		_ => {
			a.push(title.to_string());
			a.push(text.to_string());
		}
	}
	Some(a)
}

/// Did the helper really show a window? 0 = the player pressed OK; 1 = zenity / kdialog report a closed window like that, but the
/// same code comes back at once when the helper could not open a display, so a quick 1 does not count. Anything else = it failed.
pub(crate) fn shown(code: Option<i32>, elapsed_ms: u128) -> bool {
	match code {
		Some(0) => true,
		Some(1) => elapsed_ms >= 400,
		_ => false,
	}
}

/// `tool` in the folders of `path` (the PATH variable, split the way this system splits it), when it is a file there.
pub(crate) fn find_in_path(tool: &str, path: &str) -> Option<PathBuf> {
	std::env::split_paths(path).filter(|d| !d.as_os_str().is_empty()).map(|d| d.join(tool)).find(|p| p.is_file())
}

/// Linux: the helper window ends when the game does (`PR_SET_PDEATHSIG` = SIGTERM, set between fork and exec). Without it a zenity /
/// kdialog window outlives a game that quit or crashed, and under desktop Steam the game stays "Running" until the window is closed.
/// The signal follows the thread that spawned the helper; every caller blocks that thread in `status()` until the helper ends, so it
/// fires when the process (or the helper's own thread) ends.
#[cfg(target_os = "linux")]
pub(crate) fn die_with_parent(cmd: &mut std::process::Command) {
	use std::os::unix::process::CommandExt;
	// SAFETY: the closure runs in the forked child before exec and only calls prctl(2), which is async-signal-safe.
	unsafe {
		cmd.pre_exec(|| {
			libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong);
			Ok(())
		});
	}
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn die_with_parent(_cmd: &mut std::process::Command) {}

/// Shows the text and waits until it is closed (notify-send does not wait). True when a helper showed it.
/// `path` and `desktop` are PATH and XDG_CURRENT_DESKTOP (parameters so the tests can use their own).
pub(crate) fn show_with(path: &str, desktop: &str, title: &str, text: &str, kind: Kind) -> bool {
	for tool in tool_order(desktop) {
		let (Some(exe), Some(argv)) = (find_in_path(tool, path), command(tool, title, text, kind)) else { continue };
		let t = std::time::Instant::now();
		let mut cmd = std::process::Command::new(exe);
		cmd.args(&argv[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
		die_with_parent(&mut cmd);
		if let Ok(st) = cmd.status() {
			if shown(st.code(), t.elapsed().as_millis()) {
				return true;
			}
		}
	}
	false
}

/// `show_with` for this process's environment.
pub(crate) fn show(title: &str, text: &str, kind: Kind) -> bool {
	// [PC] The text is a run log line too (runlog_policy.rs), whether or not a helper can show it.
	crate::runlog_policy::note_message(text);
	let path = std::env::var("PATH").unwrap_or_default();
	let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
	show_with(&path, &desktop, title, text, kind)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A helper started with `die_with_parent` gets SIGTERM when the thread that started it ends (the process ending is the same case).
	#[cfg(target_os = "linux")]
	#[test]
	fn a_helper_ends_when_its_parent_thread_ends() {
		let (tx, rx) = std::sync::mpsc::channel();
		let t = std::thread::spawn(move || {
			let mut cmd = std::process::Command::new("sleep");
			cmd.arg("60");
			die_with_parent(&mut cmd);
			let Ok(child) = cmd.spawn() else { return tx.send(0).unwrap() };
			tx.send(child.id() as i32).unwrap();
		});
		let pid = rx.recv().unwrap();
		t.join().unwrap();
		if pid == 0 {
			return; // no `sleep` here
		}
		let mut status = 0;
		let mut got = 0;
		for _ in 0..100 {
			// SAFETY: waitpid on a child of this process, with a valid status pointer.
			got = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
			if got != 0 {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}
		if got == 0 {
			// SAFETY: a plain kill(2) of the helper this test started.
			unsafe { libc::kill(pid, libc::SIGKILL) };
			let _ = unsafe { libc::waitpid(pid, &mut status, 0) };
			panic!("the helper outlived the thread that started it");
		}
		assert_eq!(got, pid);
		assert!(libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGTERM, "status {status:#x}");
	}

	#[test]
	fn windows_flags_map_to_a_kind() {
		assert_eq!(kind_from_flags(0x30 | 0x1_0000 | 0x4_0000), Kind::Warning);
		assert_eq!(kind_from_flags(0x40 | 0x4_0000), Kind::Info);
		assert_eq!(kind_from_flags(0x10), Kind::Error);
		assert_eq!(kind_from_flags(0), Kind::Info);
	}

	#[test]
	fn the_desktop_picks_the_first_helper() {
		assert_eq!(tool_order("KDE")[0], "kdialog");
		assert_eq!(tool_order("ubuntu:GNOME")[0], "zenity");
		assert_eq!(tool_order("")[0], "zenity");
		assert_eq!(tool_order("")[3], "notify-send");
	}

	#[test]
	fn each_helper_gets_its_own_arguments_and_the_text_is_one_argument() {
		let text = "Your disc image moved or is gone:\n/home/p/R&C \"x\".iso\n\nRun setup.sh again.";
		let z = command("zenity", "Ratchet & Clank (rac-rs)", text, Kind::Error).unwrap();
		assert_eq!(z[..4], ["zenity", "--error", "--no-markup", "--width=520"]);
		assert_eq!(z[z.len() - 4..], ["--title", "Ratchet & Clank (rac-rs)", "--text", text], "markup off: the & in the text is literal");
		let k = command("kdialog", "T", text, Kind::Warning).unwrap();
		assert_eq!(k, ["kdialog", "--sorry", text, "--title", "T"]);
		assert_eq!(command("kdialog", "T", "x", Kind::Error).unwrap()[1], "--error");
		assert_eq!(command("kdialog", "T", "x", Kind::Info).unwrap()[1], "--msgbox");
		let x = command("xmessage", "T", text, Kind::Info).unwrap();
		assert_eq!(x.last().unwrap(), text);
		assert_eq!(x[x.len() - 2], "T");
		let n = command("notify-send", "T", "body", Kind::Error).unwrap();
		assert_eq!(n[..5], ["notify-send", "-a", "rac-rs", "-u", "critical"]);
		assert_eq!(n[n.len() - 2..], ["T", "body"]);
		assert!(command("bash", "T", "x", Kind::Info).is_none(), "only the known helpers are ever run");
	}

	#[test]
	fn a_quick_failure_is_not_a_shown_window() {
		assert!(shown(Some(0), 5));
		assert!(shown(Some(1), 3000), "closed with the window's X after a while");
		assert!(!shown(Some(1), 40), "no display: zenity says 1 at once");
		assert!(!shown(Some(255), 3000));
		assert!(!shown(None, 3000), "killed by a signal");
	}

	#[test]
	fn the_path_search_takes_files_only() {
		let d = std::env::temp_dir().join(format!("racdlg_find_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(d.join("a").join("zenity")).unwrap();
		std::fs::create_dir_all(d.join("b")).unwrap();
		std::fs::write(d.join("b").join("zenity"), "x").unwrap();
		let sep = if cfg!(windows) { ";;" } else { "::" };
		let path = format!("{}{sep}{}", d.join("a").display(), d.join("b").display());
		assert_eq!(find_in_path("zenity", &path), Some(d.join("b").join("zenity")), "a folder named zenity is skipped");
		assert_eq!(find_in_path("kdialog", &path), None);
		let _ = std::fs::remove_dir_all(&d);
	}

	/// A fake `zenity` in a private PATH: it records its arguments and exits 0 (a Linux shell script).
	#[cfg(unix)]
	#[test]
	fn show_runs_the_first_helper_that_works_and_falls_through_a_failing_one() {
		use std::os::unix::fs::PermissionsExt;
		let d = std::env::temp_dir().join(format!("racdlg_show_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		let out = d.join("args.txt");
		let script = |name: &str, body: &str| {
			let p = d.join(name);
			std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
			std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
		};
		let path = d.display().to_string();
		// nothing installed: false, no panic
		assert!(!show_with(&path, "", "T", "text", Kind::Error));
		// zenity fails at once (no display), kdialog works
		script("zenity", "exit 255");
		script("kdialog", &format!("printf '%s|' \"$@\" > '{}'; exit 0", out.display()));
		assert!(show_with(&path, "", "Title", "Hello & goodbye", Kind::Error));
		assert_eq!(std::fs::read_to_string(&out).unwrap(), "--error|Hello & goodbye|--title|Title|");
		let _ = std::fs::remove_dir_all(&d);
	}
}
