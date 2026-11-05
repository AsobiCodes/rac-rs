//! The mods' draw list (ABI 1.2 `draw_submit`): the list the loader hands over once per tick is copied into a shared slot on the retail worker
//! thread; the render thread (MAIN) takes the newest one each frame (`current`) and draws it as the last 2D layer
//! (viewer/gpu/draw/pass_omni.rs, quads from draw_quads.rs, space mapping draw_map.rs).
//!
//! Nothing is created or recorded until a mod has submitted a non-empty list: `current` answers from one atomic load (`EVER`) before that.
//! A list stays valid for `FRESH` after its submit (the loader sends one per game tick and an empty one when a mod stops drawing); when the game
//! stops ticking (a load) the picture does not hang on screen for ever.

use super::cabi::{Host, OmniDrawCmd};
use super::draw_map::Layout;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a submitted list stays on screen without a newer one.
const FRESH: Duration = Duration::from_millis(1000);
/// Commands kept per list (a mod drawing more than this is cut, not trusted).
pub const MAX_CMDS: usize = 20_000;
/// Text bytes kept per list.
pub const MAX_POOL: usize = 1 << 20;

/// One tick's list.
pub struct Frame {
	pub cmds: Vec<OmniDrawCmd>,
	pub pool: Vec<u8>,
	at: Instant,
}

impl Frame {
	/// The UTF-8 text of a text command (empty when its range is outside the pool or not valid UTF-8).
	pub fn text(&self, c: &OmniDrawCmd) -> &str {
		let (a, n) = (c.text_off as usize, c.text_len as usize);
		a.checked_add(n).and_then(|e| self.pool.get(a..e)).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
	}
}

/// The newest list (WK writes, MAIN reads) and whether a mod ever drew.
pub struct Slot {
	/// A mod has submitted a non-empty list at least once: until then the renderer does no work at all.
	ever: AtomicBool,
	/// `None` = nothing to draw.
	cur: Mutex<Option<Arc<Frame>>>,
}

impl Slot {
	pub const fn new() -> Slot {
		Slot { ever: AtomicBool::new(false), cur: Mutex::new(None) }
	}

	/// Copies the loader's list into the slot. `n == 0` clears it.
	pub fn submit(&self, cmds: &[OmniDrawCmd], pool: &[u8]) {
		if cmds.is_empty() {
			*self.cur.lock().unwrap_or_else(|e| e.into_inner()) = None;
			return;
		}
		let n = cmds.len().min(MAX_CMDS);
		let p = pool.len().min(MAX_POOL);
		let frame = Arc::new(Frame { cmds: cmds[..n].to_vec(), pool: pool[..p].to_vec(), at: Instant::now() });
		*self.cur.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
		self.ever.store(true, Ordering::Release);
	}

	/// The list to draw this frame, if a fresh one exists. Before the first submit: one atomic load.
	pub fn current(&self) -> Option<Arc<Frame>> {
		if !self.ever.load(Ordering::Acquire) {
			return None;
		}
		let f = self.cur.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
		(f.at.elapsed() < FRESH).then_some(f)
	}

	pub fn ever(&self) -> bool {
		self.ever.load(Ordering::Acquire)
	}
}

static SLOT: Slot = Slot::new();

pub fn submit(cmds: &[OmniDrawCmd], pool: &[u8]) {
	SLOT.submit(cmds, pool);
}

/// The debug dots of `pose_debug` (pose_capture.rs): a list of its own, so the mods' list is never mixed with it by the worker.
static DEBUG_SLOT: Slot = Slot::new();

pub fn submit_debug(cmds: &[OmniDrawCmd], pool: &[u8]) {
	DEBUG_SLOT.submit(cmds, pool);
}

/// The list the renderer draws this frame: the mods' list, then the debug dots (text offsets of the second moved past the first's pool).
pub fn current() -> Option<Arc<Frame>> {
	match (SLOT.current(), DEBUG_SLOT.current()) {
		(a, None) => a,
		(None, b) => b,
		(Some(a), Some(b)) => Some(Arc::new(merge(&a, &b))),
	}
}

/// `a`'s commands then `b`'s; the text offsets of `b` move past `a`'s pool.
fn merge(a: &Frame, b: &Frame) -> Frame {
	let shift = a.pool.len() as u32;
	let mut cmds = a.cmds.clone();
	cmds.extend(b.cmds.iter().map(|c| OmniDrawCmd { text_off: c.text_off + shift, ..*c }));
	Frame { cmds, pool: [a.pool.as_slice(), b.pool.as_slice()].concat(), at: Instant::now() }
}

/// The window and the overlay's place in it, from the last frame the renderer prepared (MAIN writes, WK reads for `world_to_screen`).
static VIEW: Mutex<Option<(f32, f32, Layout)>> = Mutex::new(None);

/// The render thread tells where the overlay sits in the window this frame (called from `Gpu::frame_buffers`). It does nothing unless mods run.
pub fn note_view(hud: &[crate::overlay::Quad], ov_tex: usize, sw: f32, sh: f32) {
	if super::get().is_none() {
		return;
	}
	*VIEW.lock().unwrap_or_else(|e| e.into_inner()) = Some((sw, sh, Layout::from_hud(hud, ov_tex, sw, sh)));
}

/// Window size and overlay layout of the last frame; `None` before the first frame.
pub fn view() -> Option<(f32, f32, Layout)> {
	*VIEW.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C" fn draw_submit_cb(_user: *mut c_void, cmds: *const OmniDrawCmd, n: u32, text_pool: *const c_char, text_len: u32) {
	let list: &[OmniDrawCmd] = if cmds.is_null() || n == 0 {
		&[]
	} else {
		// SAFETY: the loader passes `n` commands that stay valid for the call; they are copied before it returns.
		unsafe { std::slice::from_raw_parts(cmds, n as usize) }
	};
	let pool: &[u8] = if text_pool.is_null() || text_len == 0 {
		&[]
	} else {
		// SAFETY: `text_len` bytes at `text_pool`, valid for the call.
		unsafe { std::slice::from_raw_parts(text_pool as *const u8, text_len as usize) }
	};
	submit(list, pool);
}

/// Sets the five ABI 1.2 entries this stage owns (`raycast` is the host stage's).
pub fn fill(h: &mut Host) {
	h.draw_submit = Some(draw_submit_cb);
	super::draw_w2s::fill(h);
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Mutex as TestLock;

	/// The slot is process-wide: tests that use it take turns.
	static TURN: TestLock<()> = TestLock::new(());

	fn cmd(kind: u32, x: f32) -> OmniDrawCmd {
		OmniDrawCmd { kind, x, ..Default::default() }
	}

	#[test]
	fn the_list_and_the_text_pool_are_copied() {
		let _t = TURN.lock().unwrap_or_else(|e| e.into_inner());
		let mut t = cmd(1, 10.0);
		t.text_off = 3;
		t.text_len = 5;
		let list = [cmd(2, 1.0), t];
		let pool = *b"abcHELLOxyz";
		// SAFETY: valid pointers and lengths for the call.
		unsafe { draw_submit_cb(std::ptr::null_mut(), list.as_ptr(), 2, pool.as_ptr() as *const c_char, pool.len() as u32) };
		let f = current().expect("a fresh list");
		assert_eq!(f.cmds.len(), 2);
		assert_eq!(f.cmds[0].kind, 2);
		assert_eq!(f.text(&f.cmds[1]), "HELLO");
		// A text range outside the pool draws nothing.
		let mut bad = cmd(1, 0.0);
		bad.text_off = 8;
		bad.text_len = 40;
		assert_eq!(f.text(&bad), "");
		// n = 0 clears.
		unsafe { draw_submit_cb(std::ptr::null_mut(), std::ptr::null(), 0, std::ptr::null(), 0) };
		assert!(current().is_none());
		assert!(SLOT.ever());
	}

	#[test]
	fn no_submit_means_no_list_and_no_work() {
		let s = Slot::new();
		assert!(!s.ever());
		assert!(s.current().is_none());
		// An empty submit is not a draw either.
		s.submit(&[], &[]);
		assert!(!s.ever() && s.current().is_none());
		s.submit(&[cmd(2, 0.0)], &[]);
		assert!(s.ever() && s.current().is_some());
	}

	#[test]
	fn a_huge_list_is_cut() {
		let _t = TURN.lock().unwrap_or_else(|e| e.into_inner());
		let list = vec![cmd(2, 0.0); MAX_CMDS + 5];
		submit(&list, &[]);
		assert_eq!(current().expect("list").cmds.len(), MAX_CMDS);
		submit(&[], &[]);
	}
}

#[cfg(test)]
mod merge_tests {
	use super::*;

	fn frame(texts: &[&str]) -> Frame {
		let mut pool = Vec::new();
		let cmds = texts
			.iter()
			.map(|t| {
				let c = OmniDrawCmd { kind: 1, text_off: pool.len() as u32, text_len: t.len() as u32, ..OmniDrawCmd::default() };
				pool.extend_from_slice(t.as_bytes());
				c
			})
			.collect();
		Frame { cmds, pool, at: Instant::now() }
	}

	// The debug list is appended to the mods' list and its text offsets stay valid in the joined pool.
	#[test]
	fn the_debug_list_is_appended_with_its_texts_moved() {
		let (a, b) = (frame(&["ab", "cde"]), frame(&["xy"]));
		let m = merge(&a, &b);
		assert_eq!(m.cmds.len(), 3);
		assert_eq!((m.text(&m.cmds[0]), m.text(&m.cmds[1]), m.text(&m.cmds[2])), ("ab", "cde", "xy"));
	}
}
