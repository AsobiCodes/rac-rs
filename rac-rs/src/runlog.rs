//! [PC] The player's run log, `<player dir>/log.txt` (the run before it: `log.prev.txt`). The public exe has no console, so all the
//! game wrote to stderr (the `gpu:` / `retail:` lines, panics) went nowhere and a freeze left no trace (GitHub issue #12: a white
//! "Not Responding" window at start, no log). With the run log, stderr goes to that file, the load steps of `loadlog::say` are
//! written too, and a watchdog thread notes when the window stops answering (the test behind Windows' "Not Responding"), for how
//! long, the last load step, whether the main thread is busy or blocked meanwhile and the named waits of every thread
//! (`waiting`), with the call stacks of the main thread and of the waiting threads (the game's own functions by name, from the
//! exe's symbol table); a Windows-level crash leaves its exception, module and call stack. RAC_TEST_CRASH=1 (tests only) crashes
//! the watchdog thread once the window is open, to check that last part.
//! Only when stderr goes nowhere (the double-clicked game, Setup's Play): a console or a redirected stderr keeps it, so dev and test
//! runs are unchanged. RAC_RUN_LOG=1 forces it (tests), RAC_RUN_LOG=0 turns it off. Windows (the whole log) and Linux (stderr into the file only: runlog_linux.rs); elsewhere `start` does nothing.
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The log stops growing at this size (a runaway message must not fill the disk); a last line says so.
const MAX_BYTES: u64 = 16 << 20;
/// The watchdog's notes: when the window has not answered for each of these many seconds.
const STALL_NOTES: [u64; 7] = [10, 30, 60, 120, 300, 600, 1800];

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The game window's HWND (0 until `set_window`).
static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// The last `loadlog::say` line.
static STEP: Mutex<String> = Mutex::new(String::new());
/// The waits in progress (`waiting`): id, thread, what, the thread's Windows id (its stack in the notes).
static WAITS: Mutex<Vec<(u64, Arc<str>, &'static str, u32)>> = Mutex::new(Vec::new());
static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);
/// The main thread's Windows thread id (its CPU time tells busy from blocked).
static MAIN_TID: AtomicU32 = AtomicU32::new(0);

thread_local! {
	/// This thread's name in the notes.
	static LABEL: std::cell::OnceCell<Arc<str>> = const { std::cell::OnceCell::new() };
}

/// True once `start` sent stderr into the run log.
pub fn active() -> bool {
	ACTIVE.load(Ordering::Relaxed)
}

/// Starts the run log in `dir` when stderr goes nowhere (see the module doc). Called once on the main thread, after `launch_args`.
pub fn start(dir: &std::path::Path) -> bool {
	#[cfg(target_os = "linux")]
	{
		if !crate::runlog_linux::redirect(dir) {
			return false;
		}
		LABEL.with(|l| {
			let _ = l.set("main".into());
		});
		ACTIVE.store(true, Ordering::Release);
		eprintln!("rac-rs run log: {} started (Linux, no watchdog)", crate::app_name());
		return true;
	}
	#[cfg(all(not(windows), not(target_os = "linux")))]
	{
		let _ = dir;
		return false;
	}
	#[cfg(windows)]
	{
		let force = std::env::var("RAC_RUN_LOG").ok();
		if force.as_deref() == Some("0") || (force.as_deref() != Some("1") && win::has_stderr()) {
			return false;
		}
		let _ = std::fs::create_dir_all(dir);
		let path = dir.join("log.txt");
		let _ = std::fs::rename(&path, dir.join("log.prev.txt"));
		let Ok(f) = std::fs::File::create(&path) else { return false };
		if !win::set_stderr(f) {
			return false;
		}
		LABEL.with(|l| {
			let _ = l.set("main".into());
		});
		MAIN_TID.store(win::thread_id(), Ordering::Relaxed);
		ACTIVE.store(true, Ordering::Release);
		eprintln!("rac-rs run log: {} started {} (local time)", crate::app_name(), win::local_time());
		eprintln!("system: {}", win::system());
		eprintln!("exe: {}", std::env::current_exe().map_or_else(|e| e.to_string(), |p| p.display().to_string()));
		win::catch_crashes();
		let _ = std::thread::Builder::new().name("runlog".into()).spawn(move || win::watch(path));
		true
	}
}

/// The last line of a run that ended on its own (a log without it: the game was closed from outside or crashed hard).
pub fn end() {
	if active() {
		eprintln!("runlog: +{:.3} the game ended", crate::loadlog::secs());
	}
}

/// The calling thread's calls into the run log (the panic hook: where the game panicked).
pub fn note_calls() {
	#[cfg(windows)]
	if active() {
		eprint!("runlog: calls of thread {}:\n{}", std::thread::current().name().unwrap_or("unnamed"), win::own_calls());
	}
}

/// Remembers a load step (`loadlog::say`) for the watchdog's notes.
pub fn note_step(line: &str) {
	if active() {
		if let Ok(mut s) = STEP.lock() {
			s.clear();
			s.push_str(line);
		}
	}
}

/// The game window, for the watchdog's "does it answer" test.
pub fn set_window(w: &winit::window::Window) {
	#[cfg(windows)]
	{
		use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
		if let Ok(h) = w.window_handle() {
			if let RawWindowHandle::Win32(h) = h.as_raw() {
				WINDOW.store(h.hwnd.get() as usize, Ordering::Relaxed);
			}
		}
	}
	#[cfg(not(windows))]
	let _ = w;
}

/// While the guard lives, the watchdog's notes list `what` as a wait of this thread. Only for waits that can last (a frame's
/// vsync, another thread's result), not for spin loops: it takes a lock.
pub fn waiting(what: &'static str) -> Waiting {
	// Only the Windows watchdog reads the list; elsewhere a wait would take the lock for nothing, every frame.
	#[cfg(windows)]
	let off = !active();
	#[cfg(not(windows))]
	let off = true;
	if off {
		return Waiting(0);
	}
	let thread = LABEL.with(|l| l.get_or_init(|| std::thread::current().name().unwrap_or("unnamed").into()).clone());
	let id = NEXT_WAIT.fetch_add(1, Ordering::Relaxed);
	#[cfg(windows)]
	let tid = win::thread_id();
	#[cfg(not(windows))]
	let tid = 0;
	if let Ok(mut w) = WAITS.lock() {
		w.push((id, thread, what, tid));
	}
	Waiting(id)
}

pub struct Waiting(u64);

impl Drop for Waiting {
	fn drop(&mut self) {
		if self.0 != 0 {
			if let Ok(mut w) = WAITS.lock() {
				w.retain(|e| e.0 != self.0);
			}
		}
	}
}

/// "main: the present thread ...; gs-gpu: ..." (every wait in progress), or "none named".
fn waits_now() -> String {
	let w = WAITS.lock().map(|w| w.iter().map(|(_, t, what, _)| format!("{t}: {what}")).collect::<Vec<_>>()).unwrap_or_default();
	if w.is_empty() { "none named".into() } else { w.join("; ") }
}

#[cfg(windows)]
mod win {
	use super::*;
	use std::ffi::c_void;
	use std::time::{Duration, Instant};

	const STD_ERROR_HANDLE: u32 = -12i32 as u32;
	const WM_NULL: u32 = 0;
	const SMTO_ABORTIFHUNG: u32 = 0x2;

	#[repr(C)]
	struct OsVersion {
		size: u32,
		major: u32,
		minor: u32,
		build: u32,
		platform: u32,
		csd: [u16; 128],
	}

	#[repr(C)]
	struct MemStatus {
		len: u32,
		load: u32,
		total: u64,
		avail: u64,
		page_total: u64,
		page_avail: u64,
		virt_total: u64,
		virt_avail: u64,
		ext_avail: u64,
	}

	#[repr(C)]
	struct ExceptionRecord {
		code: u32,
		flags: u32,
		next: *mut c_void,
		address: *mut c_void,
		nparams: u32,
		info: [usize; 15],
	}

	#[repr(C)]
	struct ExceptionPointers {
		record: *const ExceptionRecord,
		context: *mut c_void,
	}

	type Filter = unsafe extern "system" fn(*const ExceptionPointers) -> i32;

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetStdHandle(id: u32) -> *mut c_void;
		fn SetStdHandle(id: u32, h: *mut c_void) -> i32;
		fn GetLocalTime(t: *mut [u16; 8]);
		fn GlobalMemoryStatusEx(m: *mut MemStatus) -> i32;
		fn SetUnhandledExceptionFilter(f: Option<Filter>) -> Option<Filter>;
		fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
		fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, len: u32) -> u32;
		fn GetCurrentThreadId() -> u32;
		fn OpenThread(access: u32, inherit: i32, id: u32) -> *mut c_void;
		fn GetThreadTimes(h: *mut c_void, created: *mut u64, exited: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
		fn WriteFile(h: *mut c_void, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
		fn GetModuleHandleW(name: *const u16) -> *mut c_void;
		fn SuspendThread(h: *mut c_void) -> u32;
		fn ResumeThread(h: *mut c_void) -> u32;
		fn GetThreadContext(h: *mut c_void, ctx: *mut c_void) -> i32;
		fn CloseHandle(h: *mut c_void) -> i32;
	}

	#[cfg(target_arch = "x86_64")]
	#[link(name = "ntdll")]
	unsafe extern "system" {
		fn RtlCaptureContext(ctx: *mut c_void);
		fn RtlLookupFunctionEntry(pc: u64, base: *mut u64, history: *mut c_void) -> *mut c_void;
		fn RtlVirtualUnwind(kind: u32, base: u64, pc: u64, f: *mut c_void, ctx: *mut c_void, data: *mut *mut c_void, frame: *mut u64, ptrs: *mut c_void) -> *mut c_void;
		fn NtQueryInformationThread(h: *mut c_void, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
	}

	#[link(name = "user32")]
	unsafe extern "system" {
		fn SendMessageTimeoutW(hwnd: *mut c_void, msg: u32, wp: usize, lp: isize, flags: u32, timeout_ms: u32, result: *mut usize) -> isize;
		fn IsWindow(hwnd: *mut c_void) -> i32;
	}

	#[link(name = "ntdll")]
	unsafe extern "system" {
		fn RtlGetVersion(v: *mut OsVersion) -> i32;
	}

	pub(super) fn has_stderr() -> bool {
		// SAFETY: GetStdHandle only reads the process's standard handle table.
		let h = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
		!h.is_null() && h as isize != -1
	}

	/// Makes `f` the process's stderr for the rest of the run (Rust's `eprintln!` looks the handle up at each write).
	pub(super) fn set_stderr(f: std::fs::File) -> bool {
		use std::os::windows::io::IntoRawHandle;
		// SAFETY: the handle is an open file that is never closed while the game runs.
		unsafe { SetStdHandle(STD_ERROR_HANDLE, f.into_raw_handle()) != 0 }
	}

	pub(super) fn thread_id() -> u32 {
		// SAFETY: plain thread id query.
		unsafe { GetCurrentThreadId() }
	}

	/// CPU seconds the thread `h` has used so far (None: no handle or the query failed).
	fn cpu_secs(h: *mut c_void) -> Option<f64> {
		if h.is_null() {
			return None;
		}
		let (mut c, mut x, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
		// SAFETY: four FILETIMEs (u64 in 100 ns units) and a thread handle opened with query rights.
		(unsafe { GetThreadTimes(h, &mut c, &mut x, &mut k, &mut u) } != 0).then(|| (k + u) as f64 / 1e7)
	}

	pub(super) fn local_time() -> String {
		let mut t = [0u16; 8];
		// SAFETY: SYSTEMTIME is eight u16 fields.
		unsafe { GetLocalTime(&mut t) };
		format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t[0], t[1], t[3], t[4], t[5], t[6])
	}

	/// Windows version, CPU, logical CPUs and RAM (what a bug report asks for first).
	pub(super) fn system() -> String {
		let mut v = OsVersion { size: std::mem::size_of::<OsVersion>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
		// SAFETY: `v` is an OSVERSIONINFOW with its size set.
		unsafe { RtlGetVersion(&mut v) };
		let mut m = MemStatus { len: std::mem::size_of::<MemStatus>() as u32, load: 0, total: 0, avail: 0, page_total: 0, page_avail: 0, virt_total: 0, virt_avail: 0, ext_avail: 0 };
		// SAFETY: `m` is a MEMORYSTATUSEX with its length set.
		unsafe { GlobalMemoryStatusEx(&mut m) };
		let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
		let cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
		format!("Windows {}.{}.{}, {}, {cpus} logical CPUs, {:.1} GB RAM ({:.1} GB free)", v.major, v.minor, v.build, cpu_name(), gb(m.total), gb(m.avail))
	}

	fn cpu_name() -> String {
		#[cfg(target_arch = "x86_64")]
		{
			let mut b = Vec::with_capacity(48);
			for leaf in 0x8000_0002u32..=0x8000_0004 {
				// SAFETY: every x86-64 CPU has CPUID and these brand-string leaves.
				#[allow(unused_unsafe)]
				let r = unsafe { std::arch::x86_64::__cpuid(leaf) };
				for w in [r.eax, r.ebx, r.ecx, r.edx] {
					b.extend_from_slice(&w.to_le_bytes());
				}
			}
			String::from_utf8_lossy(&b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
		}
		#[cfg(not(target_arch = "x86_64"))]
		{
			std::env::consts::ARCH.to_string()
		}
	}

	/// "file.dll + 0x1234" for a code address (a crash in a graphics driver shows its DLL).
	fn module_of(addr: usize) -> String {
		match module_at(addr) {
			Some((base, file)) => format!("{file} + {:#x}", addr.wrapping_sub(base)),
			None => "in no module".into(),
		}
	}

	/// The module holding `addr`: its base and file name.
	fn module_at(addr: usize) -> Option<(usize, String)> {
		let mut m = std::ptr::null_mut();
		// GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT.
		// SAFETY: with FROM_ADDRESS the name argument is only used as an address; `m` receives the module or stays null.
		if unsafe { GetModuleHandleExW(0x4 | 0x2, addr as *const u16, &mut m) } == 0 || m.is_null() {
			return None;
		}
		let mut buf = [0u16; 260];
		// SAFETY: `buf` holds `buf.len()` u16s.
		let n = unsafe { GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) } as usize;
		let path = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
		Some((m as usize, path.rsplit(['\\', '/']).next().unwrap_or("?").to_string()))
	}

	/// "rac-rs.exe + 0x1234 (function + 0x56)": `module_of`, with the function's name inside the game's exe (its symbol name as the
	/// compiler wrote it; rustfilt or rustc-demangle reads it).
	fn where_is(addr: usize, syms: Option<&Syms>) -> String {
		let Some((base, file)) = module_at(addr) else { return format!("{addr:#x} in no module") };
		let mut s = format!("{file} + {:#x}", addr.wrapping_sub(base));
		// SAFETY: a null name asks for the exe's own module.
		let exe = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
		if let (true, Some((name, off))) = (base == exe, syms.and_then(|y| y.at(addr.wrapping_sub(base) as u32))) {
			s += &format!(" ({} + {off:#x})", &name[..name.len().min(220)]);
		}
		s
	}

	/// The game exe's function names: its COFF symbol table, read from the file when a note first needs them.
	struct Syms {
		/// (RVA, offset of the name in `names`), by RVA.
		funcs: Vec<(u32, u32)>,
		/// The symbol string table (offsets as in the file, its 4 size bytes first), then the short names.
		names: Vec<u8>,
	}

	impl Syms {
		fn load() -> Option<Syms> {
			use std::io::{Read, Seek, SeekFrom};
			let mut f = std::fs::File::open(std::env::current_exe().ok()?).ok()?;
			let mut h = vec![0u8; 4096];
			f.read_exact(&mut h).ok()?;
			let u16_at = |b: &[u8], o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
			let u32_at = |b: &[u8], o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
			let pe = u32_at(&h, 0x3c)? as usize;
			let (nsec, symptr, nsym, opt) = (u16_at(&h, pe + 6)? as usize, u32_at(&h, pe + 12)? as u64, u32_at(&h, pe + 16)? as usize, u16_at(&h, pe + 20)? as usize);
			if symptr == 0 || nsym == 0 || nsym > 8 << 20 {
				return None;
			}
			// Each section's RVA when it holds code (IMAGE_SCN_CNT_CODE).
			let mut code = Vec::with_capacity(nsec);
			for i in 0..nsec {
				let s = pe + 24 + opt + 40 * i;
				code.push(if u32_at(&h, s + 36)? & 0x20 != 0 { Some(u32_at(&h, s + 12)?) } else { None });
			}
			f.seek(SeekFrom::Start(symptr)).ok()?;
			let mut st = vec![0u8; nsym * 18];
			f.read_exact(&mut st).ok()?;
			let mut size = [0u8; 4];
			f.read_exact(&mut size).ok()?;
			let size = u32::from_le_bytes(size) as usize;
			if !(4..=256 << 20).contains(&size) {
				return None;
			}
			let mut names = vec![0u8; size];
			names[..4].copy_from_slice(&size.to_le_bytes()[..4]);
			f.read_exact(&mut names[4..]).ok()?;
			let mut funcs = Vec::new();
			let mut i = 0;
			while i < nsym {
				let e = &st[i * 18..i * 18 + 18];
				let (value, sec, kind, class, aux) = (u32_at(e, 8)?, i16::from_le_bytes([e[12], e[13]]), u16_at(e, 14)?, e[16], e[17] as usize);
				// Functions (type 0x20) in a code section, external or static.
				if let (0x20, 2 | 3, 1..) = (kind, class, sec) {
					if let Some(&Some(va)) = code.get(sec as usize - 1) {
						// A name of up to 8 bytes sits in the entry itself; a longer one is an offset into the string table.
						let name = if e[..4] == [0; 4] {
							u32_at(e, 4)?
						} else {
							let at = names.len() as u32;
							names.extend(e[..8].iter().take_while(|&&b| b != 0));
							names.push(0);
							at
						};
						funcs.push((va.wrapping_add(value), name));
					}
				}
				i += 1 + aux;
			}
			funcs.sort_unstable_by_key(|f| f.0);
			Some(Syms { funcs, names })
		}

		/// The function holding `rva`: its name and the offset into it.
		fn at(&self, rva: u32) -> Option<(&str, u32)> {
			let i = self.funcs.partition_point(|f| f.0 <= rva).checked_sub(1)?;
			let (start, name) = self.funcs[i];
			let s = self.names.get(name as usize..)?;
			std::str::from_utf8(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]).ok().map(|n| (n, rva - start))
		}
	}

	/// An x64 CONTEXT (1232 bytes, 16-byte aligned), read and written at the offsets the unwinder needs.
	#[cfg(target_arch = "x86_64")]
	#[repr(C, align(16))]
	struct Context([u8; 1232]);

	#[cfg(target_arch = "x86_64")]
	impl Context {
		const RSP: usize = 0x98;
		const RIP: usize = 0xf8;

		/// ContextFlags = CONTEXT_FULL (AMD64 | CONTROL | INTEGER | FLOATING_POINT).
		fn full() -> Context {
			let mut c = Context([0; 1232]);
			c.0[0x30..0x34].copy_from_slice(&0x0010_000bu32.to_le_bytes());
			c
		}

		fn get(&self, off: usize) -> u64 {
			u64::from_le_bytes(self.0[off..off + 8].try_into().unwrap_or([0; 8]))
		}

		fn set(&mut self, off: usize, v: u64) {
			self.0[off..off + 8].copy_from_slice(&v.to_le_bytes());
		}
	}

	/// The return addresses from `ctx` outwards into `out` (innermost first, the first one the current instruction), while the stack
	/// pointer stays in `lo..hi`; the count. It neither allocates nor locks the heap: the caller may have another thread suspended.
	#[cfg(target_arch = "x86_64")]
	fn unwind(ctx: &mut Context, lo: u64, hi: u64, out: &mut [usize]) -> usize {
		let mut n = 0;
		while n < out.len() {
			let (rip, rsp) = (ctx.get(Context::RIP), ctx.get(Context::RSP));
			if rip == 0 || rsp < lo || rsp + 8 > hi {
				break;
			}
			out[n] = rip as usize;
			n += 1;
			let mut base = 0u64;
			// SAFETY: a lookup in the loaded modules' unwind tables.
			let f = unsafe { RtlLookupFunctionEntry(rip, &mut base, std::ptr::null_mut()) };
			if f.is_null() {
				// A leaf function (no unwind data): its return address is at the stack pointer, inside the stack (checked above).
				// SAFETY: `rsp..rsp + 8` lies in the thread's stack.
				ctx.set(Context::RIP, unsafe { *(rsp as *const u64) });
				ctx.set(Context::RSP, rsp + 8);
			} else {
				let (mut data, mut frame) = (std::ptr::null_mut(), 0u64);
				// SAFETY: UNW_FLAG_NHANDLER with the function entry and module base just looked up for `rip`; `ctx` is a full CONTEXT.
				unsafe { RtlVirtualUnwind(0, base, rip, f, ctx.0.as_mut_ptr().cast(), &mut data, &mut frame, std::ptr::null_mut()) };
			}
			// No progress (a damaged stack): stop instead of looping.
			if ctx.get(Context::RSP) <= rsp {
				break;
			}
		}
		n
	}

	/// The calling thread's stack bounds (StackLimit, StackBase of its TEB).
	#[cfg(target_arch = "x86_64")]
	fn own_stack() -> (u64, u64) {
		let teb: *const u64;
		// SAFETY: on x64 Windows gs:[0x30] holds the TEB's own address; its NT_TIB starts ExceptionList, StackBase, StackLimit.
		unsafe {
			std::arch::asm!("mov {}, qword ptr gs:[0x30]", out(reg) teb, options(nostack, readonly, preserves_flags));
			(*teb.add(2), *teb.add(1))
		}
	}

	/// Thread `tid`'s return addresses into `out` (see `unwind`); the count, 0 when the thread cannot be read. The thread is
	/// suspended only while its registers and stack are read.
	#[cfg(target_arch = "x86_64")]
	fn stack_of(tid: u32, out: &mut [usize]) -> usize {
		if tid == 0 || tid == thread_id() {
			return 0;
		}
		// THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION.
		// SAFETY: OpenThread returns null on failure.
		let h = unsafe { OpenThread(0x2 | 0x8 | 0x40, 0, tid) };
		if h.is_null() {
			return 0;
		}
		let mut n = 0;
		// THREAD_BASIC_INFORMATION: exit status, TEB address, client id, affinity, priorities.
		let mut tbi = [0u64; 6];
		// SAFETY: `tbi` is 48 bytes; the TEB of a suspended thread stays mapped; `ctx` is a 16-byte aligned full CONTEXT.
		unsafe {
			if NtQueryInformationThread(h, 0, tbi.as_mut_ptr().cast(), 48, std::ptr::null_mut()) >= 0 && tbi[1] != 0 && SuspendThread(h) != u32::MAX {
				let mut ctx = Context::full();
				if GetThreadContext(h, ctx.0.as_mut_ptr().cast()) != 0 {
					let teb = tbi[1] as *const u64;
					n = unwind(&mut ctx, *teb.add(2), *teb.add(1), out);
				}
				ResumeThread(h);
			}
			CloseHandle(h);
		}
		n
	}

	#[cfg(not(target_arch = "x86_64"))]
	fn stack_of(_tid: u32, _out: &mut [usize]) -> usize {
		0
	}

	/// The lines of a call stack (`where_is` per address; a return address is looked up one byte back, inside its call).
	fn stack_lines(pcs: &[usize], syms: Option<&Syms>) -> String {
		pcs.iter().enumerate().map(|(i, &pc)| format!("  {}\n", where_is(if i == 0 { pc } else { pc.wrapping_sub(1) }, syms))).collect()
	}

	/// The watchdog's stacks: the main thread and each thread with a named wait.
	fn note_stacks(syms: &mut Option<Option<Syms>>) {
		let mut threads = vec![("main".to_string(), MAIN_TID.load(Ordering::Relaxed))];
		if let Ok(w) = WAITS.lock() {
			for (_, t, _, tid) in w.iter() {
				if *tid != 0 && threads.iter().all(|x| x.1 != *tid) {
					threads.push((t.to_string(), *tid));
				}
			}
		}
		threads.truncate(8);
		let syms = syms.get_or_insert_with(Syms::load).as_ref();
		for (name, tid) in threads {
			let mut pcs = [0usize; 48];
			let n = stack_of(tid, &mut pcs);
			eprint!("runlog: calls of thread {name} ({tid}){}\n{}", if n == 0 { ": not readable" } else { ":" }, stack_lines(&pcs[..n], syms));
		}
	}

	/// The calling thread's calls (a panic, `note_stack`).
	pub(super) fn own_calls() -> String {
		#[cfg(target_arch = "x86_64")]
		{
			let mut ctx = Context::full();
			// SAFETY: `ctx` is a 16-byte aligned CONTEXT.
			unsafe { RtlCaptureContext(ctx.0.as_mut_ptr().cast()) };
			let (lo, hi) = own_stack();
			let mut pcs = [0usize; 48];
			let n = unwind(&mut ctx, lo, hi, &mut pcs);
			stack_lines(&pcs[..n], Syms::load().as_ref())
		}
		#[cfg(not(target_arch = "x86_64"))]
		String::new()
	}

	/// `text` straight to the log file, not through `eprintln!`: another thread may hold Rust's stderr lock.
	fn write_raw(text: &str) {
		let mut n = 0u32;
		// SAFETY: `text` is valid for its length; a null or closed handle only makes WriteFile fail.
		unsafe { WriteFile(GetStdHandle(STD_ERROR_HANDLE), text.as_ptr(), text.len() as u32, &mut n, std::ptr::null_mut()) };
	}

	/// The filter that was installed before `crashed` (0: none).
	static PREV: AtomicUsize = AtomicUsize::new(0);

	pub(super) fn catch_crashes() {
		// SAFETY: a process-wide filter; `crashed` only writes a line and then hands over to the filter it replaced.
		let prev = unsafe { SetUnhandledExceptionFilter(Some(crashed)) };
		PREV.store(prev.map_or(0, |f| f as usize), Ordering::Relaxed);
	}

	unsafe extern "system" fn crashed(e: *const ExceptionPointers) -> i32 {
		// SAFETY: Windows calls the filter with valid exception pointers.
		let r = unsafe { &*(*e).record };
		let at = r.address as usize;
		// The mods' log lines still queued come first (omni/logq.rs; never waits for a lock).
		crate::omni::drain_log_raw(write_raw);
		// EXCEPTION_ACCESS_VIOLATION: what was touched.
		let access = match (r.code, r.nparams) {
			(0xC000_0005, 2..) => format!(", {} {:#x}", ["reading", "writing"].get(r.info[0]).copied().unwrap_or("executing"), r.info[1]),
			_ => String::new(),
		};
		let tid = thread_id();
		let line =format!("runlog: +{:.3} the game crashed: exception {:#010x} at {at:#x} ({}){access}, thread {tid} ({})\n", crate::loadlog::secs(), r.code, module_of(at), std::thread::current().name().unwrap_or("unnamed"));
		write_raw(&line);
		// The crashed thread's calls, from the exception's registers (the filter runs on that thread). A stack overflow
		// (EXCEPTION_STACK_OVERFLOW) leaves too little stack for it.
		#[cfg(target_arch = "x86_64")]
		if r.code != 0xC000_00FD {
			let mut ctx = Context::full();
			// SAFETY: the exception's ContextRecord is a full x64 CONTEXT of `ctx`'s size.
			unsafe { std::ptr::copy_nonoverlapping((*e).context as *const u8, ctx.0.as_mut_ptr(), ctx.0.len()) };
			let (lo, hi) = own_stack();
			let mut pcs = [0usize; 48];
			let n = unwind(&mut ctx, lo, hi, &mut pcs);
			write_raw(&format!("runlog: calls of the crashed thread:\n{}", stack_lines(&pcs[..n], Syms::load().as_ref())));
		}
		// RivaTuner running with Vulkan and the crash in its hook or the NVIDIA driver: the box that offers DirectX 12 (rtss_crash.rs;
		// the filter hands on after it).
		crate::rtss_crash::on_crash(r.code, &module_at(at).map_or_else(String::new, |(_, file)| file), write_raw);
		match PREV.load(Ordering::Relaxed) {
			// EXCEPTION_CONTINUE_SEARCH: Windows ends the game as without the log.
			0 => 0,
			f => {
				// SAFETY: `f` is the filter SetUnhandledExceptionFilter returned in `catch_crashes`.
				let f: Filter = unsafe { std::mem::transmute::<usize, Filter>(f) };
				unsafe { f(e) }
			}
		}
	}

	/// The watchdog: once a second, does the window answer a no-op message (2 s, and at once when Windows already calls it hung)?
	/// A window that has not answered for `STALL_NOTES` seconds gets a note with the last load step, the main thread's CPU time
	/// since it last answered (near the wall time: busy; near 0: blocked) and the named waits.
	pub(super) fn watch(path: std::path::PathBuf) {
		let secs = crate::loadlog::secs;
		// THREAD_QUERY_LIMITED_INFORMATION; never closed (one handle for the run).
		// SAFETY: OpenThread returns null on failure.
		let main = unsafe { OpenThread(0x800, 0, MAIN_TID.load(Ordering::Relaxed)) };
		let mut cpu0 = cpu_secs(main);
		let mut answered = Instant::now();
		let mut had_window = false;
		let mut notes = 0usize;
		let mut cut = false;
		// The game exe's function names, read at the first note with stacks.
		let mut syms = None;
		loop {
			std::thread::sleep(Duration::from_secs(1));
			if !cut && std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
				eprintln!("runlog: +{:.3} the log reached {} MB: the rest of this run is not written", secs(), MAX_BYTES >> 20);
				// SAFETY: with no stderr handle Rust's later stderr writes do nothing.
				unsafe { SetStdHandle(STD_ERROR_HANDLE, std::ptr::null_mut()) };
				cut = true;
			}
			let hwnd = WINDOW.load(Ordering::Relaxed) as *mut c_void;
			if hwnd.is_null() {
				// Before the window: "no game window yet".
			} else if !had_window {
				had_window = true;
				if notes > 0 {
					eprintln!("runlog: +{:.3} the window opened after {:.1} s", secs(), answered.elapsed().as_secs_f64());
				}
				(answered, notes, cpu0) = (Instant::now(), 0, cpu_secs(main));
				// RAC_TEST_CRASH (tests only): =1 an access violation here (the crash note), =panic a panic (the panic hook's calls).
				match std::env::var("RAC_TEST_CRASH").as_deref() {
					// SAFETY: none: a deliberate write to address 8.
					Ok("1") => unsafe { std::ptr::write_volatile(8 as *mut u8, 0) },
					// =rtss: the same crash, as if RivaTuner's hook were loaded (rtss_crash.rs).
					Ok("rtss") => {
						crate::rtss_crash::pretend_loaded();
						// SAFETY: as for =1.
						unsafe { std::ptr::write_volatile(8 as *mut u8, 0) }
					}
					Ok("panic") => panic!("RAC_TEST_CRASH=panic"),
					_ => {}
				}
				continue;
			// SAFETY: IsWindow accepts any handle value.
			} else if unsafe { IsWindow(hwnd) } == 0 {
				eprintln!("runlog: +{:.3} the window closed", secs());
				return;
			} else {
				let mut r = 0usize;
				// SAFETY: WM_NULL does nothing in the window; the call returns after at most 2 s.
				if unsafe { SendMessageTimeoutW(hwnd, WM_NULL, 0, 0, SMTO_ABORTIFHUNG, 2000, &mut r) } != 0 {
					if notes > 0 {
						eprintln!("runlog: +{:.3} the window answers again after {:.1} s", secs(), answered.elapsed().as_secs_f64());
					}
					(answered, notes, cpu0) = (Instant::now(), 0, cpu_secs(main));
					continue;
				}
			}
			let s = answered.elapsed().as_secs();
			if notes < STALL_NOTES.len() && s >= STALL_NOTES[notes] {
				notes += 1;
				let step = STEP.lock().map(|g| g.clone()).unwrap_or_default();
				let cpu = match (cpu0, cpu_secs(main)) {
					(Some(a), Some(b)) => format!("{:.1} s", b - a),
					_ => "unknown".into(),
				};
				eprintln!(
					"runlog: +{:.3} {} for {s} s; last load step: {}; main thread CPU meanwhile: {cpu}; waits: {}",
					secs(),
					if had_window { "the window has not answered" } else { "no game window yet" },
					if step.is_empty() { "none" } else { step.as_str() },
					waits_now()
				);
				// Where the threads are, at the first note and again at the minute (moved: slow; the same: stuck).
				if notes == 1 || STALL_NOTES[notes - 1] == 60 {
					note_stacks(&mut syms);
				}
				// A minute stuck with RivaTuner's hook and Vulkan: the DirectX 12 box, once, and only for an open window (rtss_crash.rs).
				if STALL_NOTES[notes - 1] == 60 {
					crate::rtss_crash::on_freeze(had_window, cpu_secs(main).zip(cpu0).map(|(b, a)| b - a), write_raw);
				}
			}
		}
	}
}
