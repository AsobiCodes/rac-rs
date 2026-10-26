//! Moby skeletons, animation sequences, posing and skinning.
//! Layout and conventions: decomp/notes/moby_animation.md (verified on our disc).
//!
//! Matrices are affine 3x4, row-major: `[r00 r01 r02 tx, r10 r11 r12 ty, r20 r21 r22 tz]`, column-vector
//! convention (`p' = R p + t`). Skinning matrices returned by `pose` act directly on the class's
//! `mesh.positions` (model space, game units): their translations are already scaled by the class unit.

use crate::bytes::{f32_at, span, u16_at, u32_at, R};
use crate::level::Level;
use crate::mobies::MobyClass;

type Mat = [[f32; 4]; 3];

const IDENTITY: Mat = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

fn mul(a: &Mat, b: &Mat) -> Mat {
	let mut o = [[0.0; 4]; 3];
	for i in 0..3 {
		for j in 0..4 {
			o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + if j == 3 { a[i][3] } else { 0.0 };
		}
	}
	o
}

fn flat(m: &Mat) -> [f32; 12] {
	[m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1], m[2][2], m[2][3]]
}

/// Ordinary right-handed rotation matrix of a unit quaternion (x, y, z, w).
pub(crate) fn quat_mat(q: [f32; 4]) -> [[f32; 3]; 3] {
	let [x, y, z, w] = q;
	[
		[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
		[2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
		[2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
	]
}

fn normalize4(q: [f32; 4]) -> [f32; 4] {
	let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
	if n > 0.0 {
		q.map(|x| x / n)
	} else {
		[0.0, 0.0, 0.0, 1.0]
	}
}

/// Retail's rotation step between consecutive frames of one sequence, shared by the moby joint evaluator (flag 0x400,
/// `blend_rotation`) and the hero +0x60 layer builder FUN_00267308 (`Animation::layer_frame`) [merge M8]: t == 0 (bit
/// pattern) keeps `qa` raw; otherwise `qa (1 - t) + qb t` in VU arithmetic (VMULA / VMADD), no sign test and no
/// normalisation.
fn lerp_consecutive(qa: [f32; 4], qb: [f32; 4], t: f32) -> [f32; 4] {
	use crate::physics::{vadd1, vmul, vsub1};
	if t.to_bits() == 0 {
		return qa;
	}
	let u = vsub1(1.0, t);
	[0, 1, 2, 3].map(|i| vadd1(vmul(qa[i], u), vmul(qb[i], t)))
}

/// One joint's rotation channel of the retail evaluator (FUN_00268a80 Novalis OV:60330-60580, Eudora 0x2469d0):
/// p == 0 (bits) reads frame A's raw quaternion (no blend, no normalisation); a same-sequence step (`direct`,
/// flag 0x400) blends `a (1-p) + b p` and uses it unnormalised (`lerp_consecutive`); otherwise the sign of `a . b`
/// picks sum or difference and the result is normalised (VU RSQRT).
fn blend_rotation(qa: [f32; 4], qb: [f32; 4], t: f32, direct: bool) -> [f32; 4] {
	if t.to_bits() == 0 || direct {
		return lerp_consecutive(qa, qb, t);
	}
	let qb = if qa.iter().zip(&qb).map(|(x, y)| x * y).sum::<f32>() < 0.0 { qb.map(|x| -x) } else { qb };
	normalize4([0, 1, 2, 3].map(|i| qa[i] * (1.0 - t) + qb[i] * t))
}

/// The non-inherited scale pass of the retail skin build 0x2438a8 (Eudora; list build 0x243ac0..0x243c18, p == 0 build
/// 0x243f68..0x243fc0, post pass 0x244400..0x2444c0) [C]: scale records with flags bit 0x80 clear are linked into a
/// second list, frame A's in record order, then (p != 0) frame B's, each B record appended even when its joint is
/// already listed (the check at 0x243bd4 reads B's own slot, empty until then). Every append writes the old tail's
/// next pointer, so re-appending a listed joint cuts off the nodes after it. Each joint the walk visits scales its
/// WORLD matrix columns by `a (1-p) + b p` (a side without a record = 1,1,1; p == 0: frame A's value) after the
/// parent chain, so the children never see it. Returns (joint, scale) in walk order.
fn post_scale_list(a: &Frame, b: &Frame, t: f32) -> Vec<(usize, [f32; 3])> {
	let raw = t.to_bits() == 0;
	let mut order: Vec<usize> = a.post_scale.iter().map(|e| e.0).collect();
	if !raw {
		let mut seen_b: Vec<usize> = Vec::new();
		for &(j, _) in &b.post_scale {
			if !seen_b.contains(&j) {
				seen_b.push(j);
				order.push(j);
			}
		}
	}
	let last = |f: &Frame, j: usize| f.post_scale.iter().rev().find(|e| e.0 == j).map_or([1.0; 3], |e| e.1);
	let mut out = Vec::new();
	let mut i = 0;
	while i < order.len() {
		let j = order[i];
		let sa = last(a, j);
		out.push((j, if raw { sa } else { let sb = last(b, j); [0, 1, 2].map(|k| sa[k] * (1.0 - t) + sb[k] * t) }));
		i = order.iter().rposition(|&x| x == j).unwrap() + 1;
	}
	out
}

#[derive(Clone)]
pub struct Skeleton {
	/// Parent joint of each joint; None for the model root (joint 0).
	pub parents: Vec<Option<usize>>,
	/// Local bind translations in raw s16 vertex units.
	pub bind_translation: Vec<[f32; 3]>,
	/// Inverse bind matrices (raw units), `[R^T | c]` from the skeleton table.
	inverse_bind: Vec<Mat>,
	/// Raw vertex units -> game units (class scale / 1024).
	pub unit: f32,
}

impl Skeleton {
	/// Bind-pose world matrix of a joint (game units): the inverse of its stored inverse bind matrix.
	pub fn bind_world(&self, joint: usize) -> Mat {
		let m = &self.inverse_bind[joint];
		let a = |r: usize, c: usize| m[r][c];
		let cof = |r: usize, c: usize| {
			let (r1, r2, c1, c2) = ((r + 1) % 3, (r + 2) % 3, (c + 1) % 3, (c + 2) % 3);
			a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)
		};
		let det = a(0, 0) * cof(0, 0) + a(0, 1) * cof(0, 1) + a(0, 2) * cof(0, 2);
		let inv_det = if det.abs() > 1e-12 { 1.0 / det } else { 0.0 };
		let mut o = IDENTITY;
		for r in 0..3 {
			for c in 0..3 {
				o[r][c] = cof(c, r) * inv_det;
			}
		}
		for r in 0..3 {
			o[r][3] = -(o[r][0] * m[0][3] + o[r][1] * m[1][3] + o[r][2] * m[2][3]) * self.unit;
		}
		o
	}
}

#[derive(Clone, Debug)]
pub struct Frame {
	/// Frames per game tick (1/60 s) for the span starting at this frame.
	pub speed: f32,
	/// The frame number the game's updaters read (header +4 / 16, FUN_002634b8).
	pub number: f32,
	/// Per-joint quaternion (x, y, z, w) as stored (unit up to s16 rounding, not normalised; the retail evaluator
	/// reads the raw value at t = 0), w already sign-corrected (the stored w is negated).
	pub rotation: Vec<[f32; 4]>,
	/// Joints with a scale record whose flags byte has bit 0x80 (default 1,1,1): part of the joint's local matrix, so
	/// the children inherit it.
	pub scale: Vec<(usize, [f32; 3])>,
	/// Scale records whose flags byte has bit 0x80 clear, in record order: the skin build 0x2438a8 scales the joint's
	/// world matrix with them after the parent chain, so the children do NOT inherit them (`post_scale_list`).
	pub post_scale: Vec<(usize, [f32; 3])>,
	/// Joints with a local translation record in raw units (default: bind translation).
	pub translation: Vec<(usize, [f32; 3])>,
}

#[derive(Clone, Debug)]
pub struct Sequence {
	pub frames: Vec<Frame>,
	/// Frames per tick (1/60 s) from the sequence header; 0 = use each frame's own speed.
	pub speed: f32,
	/// Bounding sphere (x, y, z, r) in raw units.
	pub sphere: [f32; 4],
	/// (sound index, time) trigger words, as stored: low u16 = sound, high u16 = time.
	pub triggers: Vec<u32>,
	/// Persistent loop sound (class sound index) from header byte 0x11; 0xff = none (FUN_0024e7d0 copies it
	/// to moby+0x7c, FUN_0024e890 keeps the voice alive, decomp/notes/sound.md "Animation sound cues").
	pub loop_sound: u8,
}

impl Sequence {


	/// Per-frame clock rates (frames per 50 Hz tick): the sequence rate if its bits are non-zero, else each frame's own.
	pub fn rates(&self) -> Vec<f32> {
		(0..self.frames.len()).map(|i| self.rate_of(i)).collect()
	}

	/// Length of one loop in 50 Hz ticks (sum of 1 / rate over the frames).
	pub fn tick_len(&self) -> f32 {
		self.rates().iter().map(|r| if *r > 0.0 { 1.0 / r } else { 1.0 }).sum()
	}

	/// First trigger id with `old < time <= new` (clock units), the 0x264df8 cue scan; none unless `new > old`.
	pub fn scan_cue(&self, old: u32, new: u32) -> Option<u16> {
		if new <= old {
			return None;
		}
		self.triggers.iter().find(|w| {
			let t = *w >> 16;
			old < t && t <= new
		}).map(|w| (*w & 0xffff) as u16)
	}

	/// Per-frame rate the clock uses: the sequence rate if its bits are non-zero, else the frame's own.
	fn rate_of(&self, frame: usize) -> f32 {
		ee(if self.speed.to_bits() != 0 { self.speed } else { self.frames.get(frame).map_or(0.0, |f| f.speed) })
	}
}

/// The EE FPU has no infinities or NaN: results saturate at +-f32::MAX (an infinite frame rate, p / r with both
/// saturated). Mirroring it is what makes the 0x264df8 advance loop terminate on such frames.
fn ee(x: f32) -> f32 {
	if x.is_nan() { f32::MAX } else { x.clamp(-f32::MAX, f32::MAX) }
}

const ONE_BITS: i32 = 0x3f80_0000;
/// Snap window around 1.0 (0.99609375 ..= 1.00390625) of the retail clock, decomp/notes/moby_animation.md.
const SNAP_LO: i32 = 0x3f7f_0000;
const SNAP_HI: i32 = 0x3f80_8000;
/// Progress threshold above which a sequence change snapshots the current pose (0x3ccccccd).
pub const BLEND_SNAPSHOT_MIN: f32 = 0.025;
pub const SNAPSHOT_SLOTS: usize = 16;

/// Result of one clock call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockStep {
	/// bit 0: a frame boundary was crossed, bit 1: the sequence wrapped.
	pub flags: u8,
	/// Trigger id fired by the cue scan.
	pub cue: Option<u16>,
}

/// Per-actor animation clock: the 0x264df8 state machine, one `step` per 50 Hz game tick (not per second).
/// Frame fields are indices into the current sequence; `old_seq == 0xff` means the "current" frame is a snapshot slot.
#[derive(Clone, Debug)]
pub struct AnimClock {
	pub p: f32,
	pub m: f32,
	pub r: f32,
	pub cur: i32,
	pub next: i32,
	pub old_seq: u8,
	pub new_seq: u8,
	/// Snapshot slot held while fading (valid when `old_seq == 0xff`).
	pub slot: Option<usize>,
	/// Retail +0x60 / +0x64 words (non-zero forces a snapshot on set/blend).
	pub x60: u32,
	pub x64: u32,
	pub prev_seq: u8,
	/// Loop-sound / cue scan disabled until the next set (+0x7c word == 0xffff).
	pub cues_off: bool,
	/// Hero ramp fade (row, next index) into `RAMP` while `old_seq != new_seq`.
	pub ramp: Option<(u8, u8)>,
	/// Hero loop range (from, to): after an advance with `next > to` the clock jumps back to `from`.
	pub hold: Option<(u8, u8)>,
}

/// Hero blend ramp weights per fade tick (3 rows; rows 0/1/2 hold 11/16/3 entries, the rest unused).
pub const RAMP: [[f32; 16]; 3] = [
	[0.15, 0.29, 0.42, 0.53, 0.63, 0.72, 0.8, 0.87, 0.93, 0.97, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
	[0.1, 0.2, 0.29, 0.38, 0.46, 0.54, 0.61, 0.68, 0.74, 0.8, 0.85, 0.9, 0.94, 0.97, 0.99, 1.0],
	[0.5, 0.75, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
];

impl AnimClock {
	/// A clock just set to `seq` at `frame` (set_moby_animation without blend).
	/// Retail's same-sequence blend flag 0x400 (FUN_00268a80 Novalis OV:60242-60250): set when
	/// `((+0x51 - +0x50) & 0xfffe) + (+0x52 - +0x53) == 0` (u32 bytes: next - cur, old_seq - new_seq), i.e. no fade
	/// and the next frame is the current one or the one after it (not the wrap to frame 0). Set = plain lerp of the two
	/// frames' quaternions without normalisation (`blend_rotation`).
	pub fn direct_blend(&self) -> bool {
		let d = (self.next as u8 as u32).wrapping_sub(self.cur as u8 as u32) & 0xfffe;
		d.wrapping_add((self.old_seq as u32).wrapping_sub(self.new_seq as u32)) == 0
	}

	pub fn new(seq: &Sequence, new_seq: u8, frame: usize) -> Self {
		let f = frame.min(seq.frames.len().saturating_sub(1));
		AnimClock { p: 0.0, m: 1.0, r: seq.rate_of(f), cur: f as i32, next: (f + 1).min(seq.frames.len().saturating_sub(1)) as i32, old_seq: new_seq, new_seq, slot: None, x60: 0, x64: 0, prev_seq: 0xff, cues_off: false, ramp: None, hold: None }
	}

	/// One tick of the retail clock for the sequence `seq` (= `new_seq`).
	pub fn step(&mut self, seq: &Sequence) -> ClockStep {
		// The cue gate is per tick (retail reloads it every call; only the tick the blend ends skips the scan).
		self.cues_off = false;
		let mut out = ClockStep::default();
		let count = seq.frames.len() as i32;
		if self.r.to_bits() == 0 || self.m.to_bits() == 0 {
			return out;
		}
		let (old_cur, old_p) = (self.cur, self.p);
		let same = self.old_seq == self.new_seq;
		let np = if same {
			ee(self.p + self.m * self.r)
		} else if let Some((row, i)) = self.ramp {
			self.ramp = Some((row, i + 1));
			RAMP[row as usize][(i as usize).min(15)]
		} else {
			ee(self.p + self.r)
		};
		let bits = np.to_bits() as i32;
		self.p = np;
		if (SNAP_LO..=SNAP_HI).contains(&bits) {
			self.p = 1.0;
			self.advance(seq, &mut out);
		} else if bits > ONE_BITS {
			self.advance(seq, &mut out);
		} else if bits < 0 {
			self.reverse(seq, &mut out);
		}
		if self.old_seq == self.new_seq && count > 0 && !self.cues_off {
			let newi = (self.cur * 16 + (self.p * 16.0) as i32) as u32;
			let oldi = (old_cur * 16 + (old_p * 16.0) as i32) as u32;
			out.cue = seq.scan_cue(oldi, newi);
		}
		out
	}

	fn advance(&mut self, seq: &Sequence, out: &mut ClockStep) {
		let count = seq.frames.len() as i32;
		loop {
			self.p -= 1.0;
			self.cur = self.next;
			self.next += 1;
			if self.old_seq != self.new_seq {
				self.old_seq = self.new_seq;
				self.cues_off = true;
				// The fade ended (moby +0x52 back to its sequence): FUN_00264058 frees the entry next frame.
				if let Some(s) = self.slot {
					SHARED.with(|t| t.borrow_mut().mark_ended(s));
				}
				self.slot = None;
				self.ramp = None;
			}
			if let Some((a, b)) = self.hold {
				if self.next > b as i32 {
					self.next = a as i32;
					self.p = 0.0;
					self.r = 0.33333334;
					out.flags |= 1;
					break;
				}
			}
			self.p = ee(self.p / self.r);
			out.flags |= 1;
			if count - self.next <= 0 {
				self.next = 0;
				out.flags |= 2;
			}
			self.r = seq.rate_of(self.cur.max(0) as usize);
			self.p = ee(self.p * self.r);
			if self.p.to_bits() as i32 <= ONE_BITS {
				break;
			}
		}
	}

	fn reverse(&mut self, seq: &Sequence, out: &mut ClockStep) {
		let count = seq.frames.len() as i32;
		loop {
			self.p = ee(self.p / self.r);
			self.next = self.cur;
			self.cur -= 1;
			if self.cur < 0 {
				self.cur = count - 1;
				out.flags |= 2;
			}
			self.r = seq.rate_of(self.cur.max(0) as usize);
			self.p = ee(self.p * self.r + 1.0);
			if (self.p.to_bits() as i32) >= 0 {
				break;
			}
		}
	}

	/// set/blend (0x26c140 / 0x26c1f8; `ex` = 0x26c340 blend_ex: no frame clamp, `flags` 4 forces a snapshot).
	/// Returns true if a snapshot of the current pose was taken (the caller stores the evaluated pose for `slot`).
	pub fn blend(&mut self, table: &mut SnapshotTable, owner: u32, seq: &Sequence, new_seq: u8, frame: i32, n: u32, ex: Option<u32>) -> bool {
		let s2 = match ex {
			Some(_) => frame,
			None => frame.min(seq.frames.len() as i32 - 1),
		};
		let force = ex.is_some_and(|f| f & 4 != 0);
		let mut snap = false;
		if force || self.p > BLEND_SNAPSHOT_MIN || self.x60 != 0 || self.x64 != 0 {
			if let Some(slot) = table.alloc(owner) {
				snap = true;
				if self.old_seq != 0xff {
					self.prev_seq = self.old_seq;
				}
				self.old_seq = 0xff;
				self.cur = slot as i32;
				self.next = s2;
				self.slot = Some(slot);
			} else {
				self.next = s2;
			}
		} else {
			self.next = s2;
		}
		self.new_seq = new_seq;
		self.m = 1.0;
		self.r = if n == 0 { f32::MAX } else { 1.0 / n as f32 };
		self.p = 0.0;
		self.cues_off = false;
		self.ramp = None;
		self.hold = None;
		snap
	}
}

/// The 16-slot pose snapshot table (retail 0x1970c0 / 0x264010): a slot is taken by the first free-or-owned entry.
#[derive(Clone, Debug, Default)]
pub struct SnapshotTable {
	owner: [Option<u32>; SNAPSHOT_SLOTS],
	/// The owner's fade ended or it was retired: freed by the next `begin_frame` (0x264058's two conditions).
	ended: [bool; SNAPSHOT_SLOTS],
}

thread_local! {
	/// The one table every moby, the hero and the held objects share (0x1970c0).
	static SHARED: std::cell::RefCell<SnapshotTable> = std::cell::RefCell::new(SnapshotTable::default());
	static NEXT_ID: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
}


impl SnapshotTable {
	/// FUN_00264010: the first empty entry, or the owner's own; None (-1) when all 16 are taken.
	pub fn alloc(&mut self, owner: u32) -> Option<usize> {
		let i = self.owner.iter().position(|o| o.is_none() || *o == Some(owner))?;
		self.owner[i] = Some(owner);
		self.ended[i] = false;
		Some(i)
	}

	/// Marks entry `slot` as ended; `begin_frame` frees it.
	pub fn mark_ended(&mut self, slot: usize) {
		if let Some(e) = self.ended.get_mut(slot) {
			*e = true;
		}
	}



}

/// Description of a pose snapshot (0x266660): the clock state it was taken from, plus the snapshot that clock was
/// itself fading from, so nested snapshots resolve to a `Frame` at draw time.
#[derive(Clone, Debug)]
struct Snap {
	clock: AnimClock,
	inner: Option<Box<Snap>>,
}

/// A 30-frame, one-frame-a-tick sequence standing in for classes without animation data.
fn fallback_sequence() -> &'static Sequence {
	static S: std::sync::OnceLock<Sequence> = std::sync::OnceLock::new();
	S.get_or_init(|| Sequence {
		frames: (0..30).map(|i| Frame { speed: 1.0, number: i as f32, rotation: Vec::new(), scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }).collect(),
		speed: 1.0,
		sphere: [0.0; 4],
		triggers: Vec::new(),
		loop_sound: 0xff,
	})
}

fn pick<'a>(seqs: Option<&'a Vec<Sequence>>, seq: usize) -> &'a Sequence {
	let found: Option<&'a Sequence> = seqs.and_then(|v| v.get(seq)).filter(|s| !s.frames.is_empty());
	match found {
		Some(s) => s,
		None => fallback_sequence(),
	}
}

impl Sequence {
	/// A timing-only sequence from (frame length in ticks, frame number) pairs: each frame's rate is 1 / length.
	pub fn from_timing(frames: &[(f32, f32)]) -> Sequence {
		Sequence {
			frames: frames.iter().map(|&(d, n)| Frame { speed: if d > 0.0 { 1.0 / d } else { 1.0 }, number: n, rotation: Vec::new(), scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }).collect(),
			speed: 0.0,
			sphere: [0.0; 4],
			triggers: Vec::new(),
			loop_sound: 0xff,
		}
	}

	/// The clock-relevant part of a sequence (frame count, numbers, rates, triggers) without joint data, for the game side.
	pub fn timing_only(&self) -> Sequence {
		Sequence {
			frames: self.frames.iter().map(|f| Frame { speed: f.speed, number: f.number, rotation: Vec::new(), scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }).collect(),
			speed: self.speed,
			sphere: self.sphere,
			triggers: self.triggers.clone(),
			loop_sound: self.loop_sound,
		}
	}
}

/// Timing-only sequences of one class, shared by every prop of it.
pub fn share_timing(seqs: &[Sequence]) -> std::rc::Rc<Vec<Sequence>> {
	std::rc::Rc::new(seqs.iter().map(Sequence::timing_only).collect())
}

/// The animation state of a non-hero actor: one retail `AnimClock` stepped once per 50 Hz game tick
/// (decomp/notes/moby_animation.md "Clock"), the pending pose snapshot of a blend, and the per-tick results the
/// game-side gates read.
#[derive(Clone)]
pub struct PropAnim {
	/// Current sequence id (the clock's `new_seq`).
	pub seq: usize,
	pub clock: AnimClock,
	seqs: Option<std::rc::Rc<Vec<Sequence>>>,
	snap: Option<Box<Snap>>,
	/// The owner id in the shared snapshot table (REN-4); clones keep it.
	id: u32,
	/// Steps since the last set/blend.
	pub age: u32,
	/// The sequence has wrapped since the last set/blend (the "animation finished" flag the gates read).
	pub done: bool,
	/// The last step wrapped.
	pub wrapped: bool,
	/// Cue ids fired by steps and not yet taken (`take_cues`), oldest first.
	cues: Vec<u16>,
	/// M3a F5, retail mode: retail's hero +0x60 layer list in chain order; when not empty `pose_held` / `local_held` use
	/// it in place of their weapon / hold layer arguments.
	pub layers: Vec<ListLayer>,
	/// Retail mode: retail's own snapshot FROM frame while the clock fades (`HeroParts::from_frame`); it replaces the port's
	/// snapshot, which is rebuilt from the last mirrored clock and so misses the in-field state SetAnim snapshotted.
	pub from_ram: Option<Frame>,
}

impl std::fmt::Debug for PropAnim {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("PropAnim").field("seq", &self.seq).field("clock", &self.clock).finish()
	}
}

impl PropAnim {
	pub fn new(seq: usize) -> Self {
		PropAnim { seq, clock: AnimClock::new(fallback_sequence(), seq as u8, 0), seqs: None, snap: None, id: NEXT_ID.with(|n| n.replace(n.get() + 1)), age: 0, done: false, wrapped: false, cues: Vec::new(), layers: Vec::new(), from_ram: None }
	}

	/// Give the actor its class's sequence timings; the clock is rebuilt at its current frame with the real rate.
	pub fn attach(&mut self, seqs: Option<std::rc::Rc<Vec<Sequence>>>) {
		self.seqs = seqs;
		let frame = self.clock.cur.max(0) as usize;
		self.set(self.seq, frame);
	}

	fn sq(&self) -> std::rc::Rc<Vec<Sequence>> {
		self.seqs.clone().unwrap_or_default()
	}

	/// Set the sequence at `frame` with no blend or snapshot (a fresh clock).
	pub fn set(&mut self, seq: usize, frame: usize) {
		let v = self.sq();
		let s = pick(Some(&*v), seq);
		self.clock = AnimClock::new(s, seq as u8, frame);
		self.seq = seq;
		self.snap = None;
		self.age = 0;
		self.done = false;
		self.wrapped = false;
	}






	/// UI-5d: the frame count of sequence `seq` (`frame_count` counts the current one).
	pub fn frame_count_of(&self, seq: usize) -> usize {
		let v = self.sq();
		pick(Some(&*v), seq).frames.len()
	}

	/// Start `seq` at `frame` fading in over `n` ticks (0x26c1f8): the new sequence holds frame `frame` while the old
	/// pose fades out, and plays when the fade ends; `n` = 0 is the plain set (0x26c140).
	pub fn blend(&mut self, seq: usize, frame: usize, n: u32) {
		let v = self.sq();
		let s = pick(Some(&*v), seq);
		let before = self.clock.clone();
		let snapped = SHARED.with(|t| self.clock.blend(&mut t.borrow_mut(), self.id, s, seq as u8, frame as i32, n, None));
		self.finish_blend(seq, snapped.then_some(before));
	}



	/// Held-weapon switch (0x26c340 blend_ex): `seq` at `frame` unclamped, fading over `n` ticks; `flags` 4 forces the snapshot.
	pub fn blend_ex(&mut self, seq: usize, frame: i32, n: u32, flags: u32) {
		let v = self.sq();
		let s = pick(Some(&*v), seq);
		let before = self.clock.clone();
		let snapped = SHARED.with(|t| self.clock.blend(&mut t.borrow_mut(), self.id, s, seq as u8, frame, n, Some(flags)));
		self.finish_blend(seq, snapped.then_some(before));
	}

	fn finish_blend(&mut self, seq: usize, before: Option<AnimClock>) {
		if let Some(before) = before {
			let mut node = Snap { clock: before, inner: self.snap.take() };
			// Bound the nesting of snapshots of snapshots (the retail table has 16 slots).
			let mut depth = 0;
			let mut at = &mut node;
			while at.inner.is_some() {
				depth += 1;
				if depth >= SNAPSHOT_SLOTS {
					at.inner = None;
					break;
				}
				at = at.inner.as_mut().unwrap();
			}
			self.snap = Some(Box::new(node));
		}
		self.seq = seq;
		self.age = 0;
		self.done = false;
		self.wrapped = false;
	}

	/// Whether the class sequence timings have been attached.
	pub fn attached(&self) -> bool {
		self.seqs.is_some()
	}




	/// Freeze on the last (`end`) or first frame with the clock stopped: a sequence played through and held.
	pub fn hold(&mut self, end: bool) {
		let v = self.sq();
		let n = pick(Some(&*v), self.seq).frames.len() as i32;
		self.clock.m = 0.0;
		self.clock.p = 0.0;
		if end {
			(self.clock.cur, self.clock.next) = (n - 1, 0);
		} else {
			(self.clock.cur, self.clock.next) = (0, 1.min(n - 1));
		}
	}


	/// Retail mode (retail.rs): the clock as the interpreter left it, moby `+0x50` cur, `+0x51` next, `+0x52` / `+0x53`
	/// playing / requested sequence, `+0x54` progress, `+0x58` multiplier, `+0x5c` rate (decomp/notes/moby_animation.md).
	/// A snapshot fade that starts this tick (`+0x52` turns 0xff, or its slot or target changes) keeps the previous
	/// clock as the fading pose, as `blend` does.
	pub fn mirror(&mut self, cur: u8, next: u8, old_seq: u8, new_seq: u8, p: f32, m: f32, r: f32) {
		let c = &self.clock;
		if old_seq == 0xff && (c.old_seq != 0xff || c.cur != cur as i32 || c.new_seq != new_seq) {
			let before = self.clock.clone();
			self.finish_blend(new_seq as usize, Some(before));
		} else if old_seq != 0xff {
			self.snap = None;
		}
		self.seq = new_seq as usize;
		let c = &mut self.clock;
		(c.cur, c.next, c.old_seq, c.new_seq, c.p, c.m, c.r) = (cur as i32, next as i32, old_seq, new_seq, p, m, r);
	}

	/// One 50 Hz tick of the clock.
	pub fn step(&mut self) -> ClockStep {
		let v = self.sq();
		let s = pick(Some(&*v), self.seq);
		let out = self.clock.step(s);
		self.age = self.age.saturating_add(1);
		self.wrapped = out.flags & 2 != 0;
		self.done |= self.wrapped;
		if let Some(c) = out.cue {
			if self.cues.len() >= 8 {
				self.cues.remove(0);
			}
			self.cues.push(c);
		}
		if self.clock.old_seq != 0xff {
			self.snap = None;
		}
		out
	}


	/// Cue ids fired since the last call.
	pub fn take_cues(&mut self) -> Vec<u16> {
		std::mem::take(&mut self.cues)
	}

	/// True while the clock fades from a snapshot into the new sequence.
	/// The FROM frame while fading: `from_ram` when set, else the port's snapshot resolved for `a`.
	fn fade_frame(&self, a: &Animated) -> Option<Frame> {
		if !self.fading() {
			return None;
		}
		self.from_ram.clone().or_else(|| self.snap.as_deref().and_then(|s| Self::resolve(a, s)))
	}

	pub fn fading(&self) -> bool {
		self.clock.old_seq == 0xff
	}






	fn resolve(a: &Animated, snap: &Snap) -> Option<Frame> {
		let s = a.sequences.get(snap.clock.new_seq as usize).filter(|s| !s.frames.is_empty())?;
		let last = s.frames.len() - 1;
		let at = |i: i32| &s.frames[(i.max(0) as usize).min(last)];
		let from = match (snap.clock.old_seq == 0xff, snap.inner.as_deref()) {
			(true, Some(inner)) => Self::resolve(a, inner)?,
			_ => at(snap.clock.cur).clone(),
		};
		Some(a.snapshot_frame(&from, at(snap.clock.next), snap.clock.p.clamp(0.0, 1.0)))
	}

	/// Model-space joint positions (game units) of the clock state as the retail joint evaluator computes them
	/// (`Animated::joint_positions`), with the frame selection of `pose` (no spins); None when the class has no frames.
	pub fn joint_positions(&self, a: &Animated) -> Option<Vec<[f32; 3]>> {
		let snapshot = self.fade_frame(a);
		if a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			a.joint_positions_clock(&self.clock, snapshot.as_ref())
		} else if let Some(s0) = a.sequences.first().filter(|s| !s.frames.is_empty()) {
			a.joint_positions_clock(&AnimClock::new(s0, 0, 0), None)
		} else {
			None
		}
	}

	/// Per joint local rotation, scale and raw translation of the clock state (frames chosen as `pose`), before the
	/// parent chain; empty when the class has no frames.
	pub fn local_trs(&self, a: &Animated) -> Vec<([f32; 4], [f32; 3], [f32; 3])> {
		let snapshot = self.fade_frame(a);
		let fresh;
		let clock = if a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			&self.clock
		} else if let Some(s0) = a.sequences.first().filter(|s| !s.frames.is_empty()) {
			fresh = AnimClock::new(s0, 0, 0);
			&fresh
		} else {
			return Vec::new();
		};
		let snapshot = if std::ptr::eq(clock, &self.clock) { snapshot } else { None };
		let s = &a.sequences[clock.new_seq as usize];
		let last = s.frames.len() - 1;
		let b = &s.frames[(clock.next.max(0) as usize).min(last)];
		let f = a.cur_frame(clock, s, snapshot.as_ref());
		a.local_parts(f, b, clock.p.clamp(0.0, 1.0), clock.direct_blend())
	}

	/// `pose` with joint scale overrides (joint, factor per axis) multiplied into the animated scale.
	pub fn pose_scaled(&self, a: &Animated, scales: &[(usize, [f32; 3])]) -> Vec<[f32; 12]> {
		if scales.is_empty() {
			return self.pose(a, &[]);
		}
		let snapshot = self.fade_frame(a);
		if a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			a.pose_clock_scaled(&self.clock, snapshot.as_ref(), scales)
		} else if let Some(s0) = a.sequences.first().filter(|s| !s.frames.is_empty()) {
			a.pose_clock_scaled(&AnimClock::new(s0, 0, 0), None, scales)
		} else {
			a.pose_clock_scaled(&self.clock, None, scales)
		}
	}

	/// Skin matrices of the clock state (blending from its snapshot while fading); `spins` are joint z-rotations.
	pub fn pose(&self, a: &Animated, spins: &[(usize, f32)]) -> Vec<[f32; 12]> {
		self.pose_fx(a, spins, &[])
	}

	/// `pose` with joint rotation overrides `joint_rots` (joint, quaternion `[x, y, z, w]` multiplied on the left of
	/// the animated rotation, NOV-14.8e) besides the z-spins.
	pub fn pose_fx(&self, a: &Animated, spins: &[(usize, f32)], joint_rots: &[(usize, [f32; 4])]) -> Vec<[f32; 12]> {
		let fx: Vec<JointFx> = joint_rots.iter().map(|&(joint, rot)| JointFx::rot(joint, rot)).collect();
		self.pose_fx_full(a, spins, &fx)
	}

	/// `pose_fx_full` with the hero layers blended in (BRIDGE-20 slot 0 `layer`, BRIDGE-41 slot 1 `layer1`); both None
	/// is exactly `pose_fx_full`.
	pub fn pose_layered(&self, a: &Animated, fx: &[JointFx], layer: Option<&LayerPose>, layer1: Option<&LayerPose>) -> Vec<[f32; 12]> {
		self.pose_held(a, fx, layer, layer1, &[None, None])
	}

	/// `pose_layered` with the HERO-24 hold layers (BRIDGE-58); all four None is exactly `pose_fx_full`.
	pub fn pose_held(&self, a: &Animated, fx: &[JointFx], layer: Option<&LayerPose>, layer1: Option<&LayerPose>, hold: &[Option<LayerPose>; 2]) -> Vec<[f32; 12]> {
		if !self.layers.is_empty() && a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			let snapshot = self.fade_frame(a);
			return match a.list_frame(&self.clock, snapshot.as_ref(), &self.layers) {
				Some(m) => a.pose_frames_fx(&m, &m, 0.0, fx),
				None => a.pose_clock_fx_full(&self.clock, snapshot.as_ref(), &[], fx),
			};
		}
		if (layer.is_none() && layer1.is_none() && hold.iter().all(Option::is_none)) || !a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			return self.pose_fx_full(a, &[], fx);
		}
		let snapshot = self.fade_frame(a);
		a.pose_held(&self.clock, snapshot.as_ref(), fx, layer, layer1, hold)
	}

	/// The local joint parts (rotation, scale, raw translation) of exactly the `pose_held` pose, frames chosen the same
	/// way: the hero input of the in-play hand-item pose copy (`hand_copy_pose`).
	pub fn local_held(&self, a: &Animated, fx: &[JointFx], layer: Option<&LayerPose>, layer1: Option<&LayerPose>, hold: &[Option<LayerPose>; 2]) -> Vec<([f32; 4], [f32; 3], [f32; 3])> {
		if a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			let snapshot = self.fade_frame(a);
			if !self.layers.is_empty() {
				if let Some(m) = a.list_frame(&self.clock, snapshot.as_ref(), &self.layers) {
					return a.local_fx_parts(&m, &m, 0.0, fx, false);
				}
				return a.local_held(&self.clock, snapshot.as_ref(), fx, None, None, &[None, None]);
			}
			a.local_held(&self.clock, snapshot.as_ref(), fx, layer, layer1, hold)
		} else if let Some(s0) = a.sequences.first().filter(|s| !s.frames.is_empty()) {
			a.local_held(&AnimClock::new(s0, 0, 0), None, fx, None, None, &[None, None])
		} else {
			Vec::new()
		}
	}

	/// `pose` with joint override records (BRIDGE-22 / HERO-17b) applied after the two-frame channel blend.
	pub fn pose_fx_full(&self, a: &Animated, spins: &[(usize, f32)], fx: &[JointFx]) -> Vec<[f32; 12]> {
		let snapshot = self.fade_frame(a);
		if a.sequences.get(self.clock.new_seq as usize).is_some_and(|s| !s.frames.is_empty()) {
			a.pose_clock_fx_full(&self.clock, snapshot.as_ref(), spins, fx)
		} else if let Some(s0) = a.sequences.first().filter(|s| !s.frames.is_empty()) {
			a.pose_clock_fx_full(&AnimClock::new(s0, 0, 0), None, spins, fx)
		} else {
			a.pose_clock_fx_full(&self.clock, None, spins, fx)
		}
	}
}

pub use crate::anim_pose::JointFx;

#[derive(Clone)]
pub struct Animated {
	pub skeleton: Skeleton,
	/// Indexed by sequence id; absent sequences have no frames.
	pub sequences: Vec<Sequence>,
	/// Joint of attach point 0 (class +0x1c table, FUN_00253bb0), the one FUN_00264140(moby, 0, out) reads.
	pub attach0: Option<usize>,
	/// Joints of attach points 1 and 2.
	pub attach12: [Option<usize>; 2],
	/// The joint of each attach point 0..31 (class +0x1c table, the last chain byte), the joint `FUN_00264140(moby, point, out)`
	/// reads: a `joint_world` argument is a point, not a bone index.
	pub attach_points: Vec<Option<usize>>,
	/// HERO-20: the skeleton joint of each attach slot 0..31 (`attach_slot_joint`), the joint a joint-override record
	/// registered on that slot drives (FUN_00263f08 record +4); None past the table or the joint count.
	pub slot_map: Vec<Option<usize>>,
	/// Joint subset of hero layer id 12 (the throw layer, class jtab entry 12; empty when absent).
	pub layer_joints: Vec<usize>,
	/// BRIDGE-41: joint subset of hero layer id 13 (weapon layer slot 1, class jtab entry 13; empty when absent).
	pub layer_joints13: Vec<usize>,
	/// HERO-24 / BRIDGE-58: the hero hold sets of slot 0 / 1 (classes 1 / 2 remapped to these joints by
	/// `hold_sequences`); empty for every other class.
	pub hold: [Vec<Sequence>; 2],
}

pub fn parse_frame(block: &[u8], at: usize, joints: usize) -> R<Frame> {
	let h = span(block, at, 16)?;
	let speed = f32_at(h, 0)?;
	let number = u16_at(h, 4)? as f32 / 16.0;
	let size = u16_at(h, 6)? as usize * 16;
	let (scale_off, scale_n, trans_off, trans_n) = (u16_at(h, 8)? as usize, u16_at(h, 10)? as usize, u16_at(h, 12)? as usize, u16_at(h, 14)? as usize);
	let body = span(block, at + 16, size)?;
	let mut rotation = Vec::with_capacity(joints);
	for j in 0..joints.min(scale_off / 8) {
		let b = span(body, j * 8, 8)?;
		let s = |i: usize| i16::from_le_bytes([b[i * 2], b[i * 2 + 1]]) as f32 / 32768.0;
		rotation.push([s(0), s(1), s(2), -s(3)]);
	}
	rotation.resize(joints, [0.0, 0.0, 0.0, 1.0]);
	let (mut scale, mut post_scale) = (Vec::new(), Vec::new());
	for i in 0..scale_n {
		let b = span(body, scale_off + i * 8, 8)?;
		let s = |k: usize| u16::from_le_bytes([b[k * 2], b[k * 2 + 1]]) as f32 / 4096.0;
		(if b[7] & 0x80 != 0 { &mut scale } else { &mut post_scale }).push((b[6] as usize, [s(0), s(1), s(2)]));
	}
	let mut translation = Vec::new();
	for i in 0..trans_n {
		let b = span(body, trans_off + i * 8, 8)?;
		let s = |k: usize| i16::from_le_bytes([b[k * 2], b[k * 2 + 1]]) as f32;
		translation.push((b[6] as usize, [s(0), s(1), s(2)]));
	}
	Ok(Frame { speed, number, rotation, scale, post_scale, translation })
}

/// Number of streamed hero menu clips (globals/ratchet_seqs/0..27).
pub const MENU_CLIPS: usize = 28;

/// The pause-menu hero clips (decomp/notes/ingame_pause_menu.md "Hero preview clips"): ToC (sector 1500) +0x18
/// holds 28 (sector, sectors) entries, each a WAD block holding one standard sequence (frame offsets relative to
/// its header). Retail streams clip N as Ratchet seq (class-0 seq count + N); the port preloads them.
pub fn menu_clips(disc: &mut crate::iso::Disc, joints: usize) -> R<Vec<Sequence>> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	(0..MENU_CLIPS)
		.map(|n| {
			let (sector, sectors) = (u32_at(&toc, 0x18 + n * 8)? as u64, u32_at(&toc, 0x1c + n * 8)? as u64);
			let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
			let file = crate::wad::decompress(&raw).map_err(|e| format!("menu clip {n}: {e}"))?;
			parse_sequence(&file, &file, joints).map_err(|e| format!("menu clip {n}: {e}"))
		})
		.collect()
}

/// Pause-menu auxiliary clip `k` (ToC +0xf8 + 8k, table 0x137c78; FUN_00275a08 reads them at pause open and
/// FUN_00284028 installs them in their class's sequence slot, decomp/notes/ingame_pause_menu.md "Hero preview
/// records").
pub fn menu_aux_clip(disc: &mut crate::iso::Disc, k: usize, joints: usize) -> R<Sequence> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&toc, 0xf8 + k * 8)? as u64, u32_at(&toc, 0xfc + k * 8)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let file = crate::wad::decompress(&raw).map_err(|e| format!("menu aux clip {k}: {e}"))?;
	parse_sequence(&file, &file, joints).map_err(|e| format!("menu aux clip {k}: {e}"))
}

/// UI-5a / BRIDGE-37 (c): the shop portrait's class-12 sequence pack (TOC +0x198 LBA / +0x19c sectors, a `WAD`): 16 u32 offsets
/// from the start of the decompressed buffer, nonzero word i = sequence i (header-relative frame and trigger offsets,
/// FUN_002541e0). Empty slots stay empty sequences (decomp/notes/vendor.md "UI-5a port spec").
pub fn portrait_seqs(disc: &mut crate::iso::Disc, joints: usize) -> R<Vec<Sequence>> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&toc, 0x198)? as u64, u32_at(&toc, 0x19c)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let buf = crate::wad::decompress(&raw).map_err(|e| format!("portrait pack: {e}"))?;
	(0..16)
		.map(|i| {
			let off = u32_at(&buf, 4 * i)? as usize;
			if off == 0 {
				return Ok(Sequence::from_timing(&[]));
			}
			let seq = buf.get(off..).ok_or_else(|| format!("portrait sequence {i}: offset {off:#x} out of range"))?;
			parse_sequence(seq, seq, joints).map_err(|e| format!("portrait sequence {i}: {e}"))
		})
		.collect()
}

/// `block` is the memory the frame offsets are relative to (the class block or the sequence block).
pub(crate) fn parse_sequence(header: &[u8], block: &[u8], joints: usize) -> R<Sequence> {
	parse_sequence_with(header, block, joints, true)
}

/// `parse_sequence` without the trigger words (scene actor records: no triggers read, count byte 0xff).
pub(crate) fn parse_sequence_frames(header: &[u8], block: &[u8], joints: usize) -> R<Sequence> {
	parse_sequence_with(header, block, joints, false)
}

fn parse_sequence_with(header: &[u8], block: &[u8], joints: usize, with_triggers: bool) -> R<Sequence> {
	let h = span(header, 0, 0x1c)?;
	let sphere = [f32_at(h, 0)?, f32_at(h, 4)?, f32_at(h, 8)?, f32_at(h, 12)?];
	let (count, trigger_count) = (h[0x10] as usize, h[0x12] as usize);
	let speed = f32_at(h, 0x18)?;
	let mut frames = Vec::with_capacity(count);
	for k in 0..count {
		let o = u32_at(header, 0x1c + 4 * k)?;
		if o >> 28 != 0 {
			return Err("compressed (special) frame format is not used by R&C1".into());
		}
		frames.push(parse_frame(block, o as usize, joints).map_err(|e| format!("frame {k}: {e}"))?);
	}
	let triggers = if with_triggers { (0..trigger_count).map(|k| u32_at(header, 0x1c + 4 * (count + k))).collect::<R<_>>()? } else { Vec::new() };
	Ok(Sequence { frames, speed, sphere, triggers, loop_sound: h[0x11] })
}

fn parse_skeleton(class: &[u8], joints: usize) -> R<Skeleton> {
	let skel = u32_at(class, 0x14)? as usize;
	let bones = u32_at(class, 0x18)? as usize;
	let unit = f32_at(class, 0x24)? / 1024.0;
	let (mut parents, mut bind, mut inv) = (Vec::new(), Vec::new(), Vec::new());
	for j in 0..joints {
		let b = span(class, bones + 16 * j, 16)?;
		bind.push([f32_at(b, 0)?, f32_at(b, 4)?, f32_at(b, 8)?]);
		let (parent, marker) = (u16_at(b, 12)? as usize / 64, u16_at(b, 14)?);
		parents.push(if marker == 0 || parent >= j { None } else { Some(parent) });
		let m = span(class, skel + 64 * j, 64)?;
		let f = |i: usize| f32_at(m, i * 4);
		// Retail inverse bind (FUN_00265958's final loop): S = W * [R^T | c] where qword k of the entry is column k of
		// the 3x3 R (so mat[r][k] = lane r of qword k) and qword 3 lanes xyz are c; lane w of every qword is never read.
		let mut mat = IDENTITY;
		for r in 0..3 {
			for c in 0..3 {
				mat[r][c] = f(c * 4 + r)?;
			}
			mat[r][3] = f(12 + r)?;
		}
		inv.push(mat);
	}
	Ok(Skeleton { parents, bind_translation: bind, inverse_bind: inv, unit })
}

/// Joint of attach point `point` of a class (FUN_00253bb0 via FUN_0024f338): class +0x1c -> table, entry
/// +4 + 4*point -> chain {u16 count, .., bytes at +4}; the last chain byte is the joint the matrix comes from.
pub fn attach_joint(level: &Level, class_id: u32, point: usize) -> Option<usize> {
	let entry = level.table(0x18, 32).ok()?.into_iter().find(|e| u32_at(e, 4).map_or(false, |id| id == class_id))?;
	let class = level.block(u32_at(entry, 0).ok()? as usize).ok()?;
	attach_joint_of(class, point)
}

/// `attach_joint` for a gadget class (the held weapons, e.g. the Pyrocitor's muzzle at point 0).
pub fn gadget_attach_joint(level: &Level, class_id: u32, point: usize) -> Option<usize> {
	let block = crate::mobies::gadget_block(level, class_id).ok()??;
	attach_joint_of(&block, point)
}

fn attach_joint_of(class: &[u8], point: usize) -> Option<usize> {
	let t = u32_at(class, 0x1c).ok()? as usize;
	if t == 0 {
		return None;
	}
	let c = u32_at(class, t + 4 + 4 * point).ok()? as usize;
	let n = u16_at(class, c).ok()? as usize;
	(n > 0).then(|| span(class, c + 4 + n - 1, 1).ok().map(|b| b[0] as usize)).flatten()
}

/// The joint a moby joint-override block `point` registers on (FUN_00263f08 OV:54779): the byte after the chain's
/// last joint byte, `class[c + 4 + class[c]]` with `c` the point's chain offset (NOV-14.8e).
fn attach_slot_joint(class: &[u8], point: usize) -> Option<usize> {
	let t = u32_at(class, 0x1c).ok()? as usize;
	if t == 0 {
		return None;
	}
	let c = u32_at(class, t + 4 + 4 * point).ok()? as usize;
	let n = *class.get(c)? as usize;
	class.get(c + 4 + n).map(|&b| b as usize)
}

/// HERO-20: the joint of attach slots 0..31 (`attach_slot_joint`), None past the table or the joint count.
fn slot_map_of(class: &[u8], joints: usize) -> Vec<Option<usize>> {
	(0..32).map(|k| attach_slot_joint(class, k).filter(|&j| j < joints)).collect()
}

/// The joints of layer `id` of a class (FUN_0024ec38): jtab entry `id` is a record whose first u16 is the offset
/// of the joint list past a 4-byte head and whose second u16 is the joint count.
pub fn layer_joints_of(class: &[u8], id: usize) -> Vec<usize> {
	let Some(t) = u32_at(class, 0x1c).ok().map(|t| t as usize).filter(|&t| t != 0) else { return Vec::new() };
	let Some(d) = u32_at(class, t + 4 + 4 * id).ok().map(|d| d as usize).filter(|&d| d != 0) else { return Vec::new() };
	let (Ok(off), Ok(n)) = (u16_at(class, d), u16_at(class, d + 2)) else { return Vec::new() };
	span(class, d + off as usize + 4, n as usize).map(|b| b.iter().map(|&j| j as usize).collect()).unwrap_or_default()
}

/// The hero throw layer as the draw pass sees it (BRIDGE-20): the layer sequence and target frame, the FROM pose
/// (sequence, frame), the layer clock `t` and the layer weight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerPose {
	pub seq: usize,
	pub frame: usize,
	pub from_seq: usize,
	pub from_frame: usize,
	pub t: f32,
	pub weight: f32,
}

/// Skeleton and sequences of a class; None when it has no joints or no sequences.
pub fn animation(level: &Level, class_id: u32) -> R<Option<Animated>> {
	let entry = level.table(0x18, 32)?.into_iter().find(|e| u32_at(e, 4).map_or(false, |id| id == class_id));
	let Some(entry) = entry else { return Ok(None) };
	let start = u32_at(entry, 0)? as usize;
	if start == 0 {
		return Ok(None);
	}
	animation_of_block(Some(level), level.block(start)?, class_id)
}

/// M3a F5: one retail hero +0x60 layer object (anim_layer_alloc FUN_002639a0: +0 jtab id, +2 joint count, +0xc joint list,
/// +8 weight, +0x20 / +0x21 from / to frame, +0x22 / +0x23 from / to seq, +0x24 t; +6 / +7 = from / to frame built from the
/// hold set, FUN_00263db8). `from_hold` / `to_hold` = the hold set index (`Animated.hold`) of a flagged frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLayer {
	pub pose: LayerPose,
	pub joints: Vec<usize>,
	pub from_hold: Option<usize>,
	pub to_hold: Option<usize>,
}

/// One weighted layer over `joints` (the `held_frame` blend): rotation nlerp toward the layer (shortest way), scale and
/// translation linear.
fn mix_layer(m: &mut Frame, lay: &Frame, w: f32, joints: &[usize], n: usize) {
	for &j in joints.iter().filter(|&&j| j < n) {
		let (bq, lq) = (m.rotation[j], lay.rotation[j]);
		let sg = if bq.iter().zip(&lq).map(|(x, y)| x * y).sum::<f32>() < 0.0 { -1.0 } else { 1.0 };
		m.rotation[j] = normalize4([0, 1, 2, 3].map(|i| bq[i] * (1.0 - w) + sg * w * lq[i]));
		let mix = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|i| x[i] * (1.0 - w) + y[i] * w);
		m.scale[j].1 = mix(m.scale[j].1, lay.scale[j].1);
		m.translation[j].1 = mix(m.translation[j].1, lay.translation[j].1);
	}
}

/// VU nlerp of the retail evaluators (FUN_00267b58 / FUN_00267308 / the layer mix): `qa (1 - t)` and `qb t`, their sum,
/// or their difference when the dot of the two parts has its sign bit set, times one VRSQRT of the squared length.
fn nlerp_vu(qa: [f32; 4], qb: [f32; 4], t: f32) -> [f32; 4] {
	use crate::physics::{vadd1, vmul, vrsqrt, vsub1};
	let u = vsub1(1.0, t);
	let (pa, pb) = (qa.map(|v| vmul(v, u)), qb.map(|v| vmul(v, t)));
	let p = [0, 1, 2, 3].map(|i| vmul(pa[i], pb[i]));
	let dot = vadd1(vadd1(vadd1(p[0], p[1]), p[2]), p[3]);
	let d = [0, 1, 2, 3].map(|i| if dot.is_sign_negative() { vsub1(pa[i], pb[i]) } else { vadd1(pa[i], pb[i]) });
	let sq = d.map(|v| vmul(v, v));
	let k = vrsqrt(1.0, vadd1(vadd1(vadd1(sq[0], sq[1]), sq[2]), sq[3]));
	d.map(|v| vmul(v, k))
}

/// The +0x60 layer mix of the joint evaluator [C: NOV OV:60595-60640]: per listed joint, base rotation `(1 - w)` and layer
/// rotation `w` combined by `nlerp_vu`'s rule (the layer rotation enters as built, unnormalised when lerped), scale and
/// translation `b (1 - w) + l w`.
fn mix_layer_vu(m: &mut Frame, lay: &Frame, w: f32, joints: &[usize], n: usize) {
	use crate::physics::{vadd1, vmul, vsub1};
	let u = vsub1(1.0, w);
	for &j in joints.iter().filter(|&&j| j < n) {
		m.rotation[j] = nlerp_vu(m.rotation[j], lay.rotation[j], w);
		let mix = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|i| vadd1(vmul(x[i], u), vmul(y[i], w)));
		m.scale[j].1 = mix(m.scale[j].1, lay.scale[j].1);
		m.translation[j].1 = mix(m.translation[j].1, lay.translation[j].1);
	}
}

/// HERO-24 / BRIDGE-58 (decomp/notes/hero_movement.md "HERO-24 port spec"): the sequences of the animation-only hold
/// set `class_id` (class 1 = slot 0, class 2 = slot 1) rebuilt for a `hero_joints` skeleton as the hold frame builder
/// does: every rotation starts cleared (0,0,0,0), hold joint k's rotation lands on hero joint `map[k]` (map = the hold
/// class bytes at its +0x1c table + 4), scale and translation entries copied unchanged. Empty when the level has no
/// such class.
pub fn hold_sequences(level: &Level, class_id: u32, hero_joints: usize) -> R<Vec<Sequence>> {
	let entry = level.table(0x18, 32)?.into_iter().find(|e| u32_at(e, 4).map_or(false, |id| id == class_id));
	let Some(entry) = entry else { return Ok(Vec::new()) };
	let start = u32_at(entry, 0)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let block = level.block(start)?;
	let Some(a) = animation_of_block(Some(level), block, class_id)? else { return Ok(Vec::new()) };
	let map = span(block, u32_at(block, 0x1c)? as usize + 4, span(block, 8, 1)?[0] as usize)?;
	Ok(a.sequences
		.into_iter()
		.map(|s| Sequence {
			frames: s
				.frames
				.into_iter()
				.map(|f| {
					let mut rotation = vec![[0.0; 4]; hero_joints];
					for (k, &m) in map.iter().enumerate() {
						if let (Some(q), true) = (f.rotation.get(k), (m as usize) < hero_joints) {
							rotation[m as usize] = *q;
						}
					}
					Frame { rotation, ..f }
				})
				.collect(),
			..s
		})
		.collect())
}

/// Skeleton and sequences of a gadget class (the held wrench and weapons); None when the level has no such
/// gadget or it has no joints or sequences. Same layout as an ordinary moby class (sequences listed at +0x48).
pub fn gadget_animation(level: &Level, class_id: u32) -> R<Option<Animated>> {
	let Some(block) = crate::mobies::gadget_block(level, class_id)? else { return Ok(None) };
	animation_of_block(Some(level), &block, class_id)
}

/// A class block that never needs the level (the warp bundle's ship classes 0x213..0x215, never class 0).
pub(crate) fn animation_of_class_block(class: &[u8], class_id: u32) -> R<Option<Animated>> {
	animation_of_block(None, class, class_id)
}

pub(crate) fn animation_of_block(level: Option<&Level>, class: &[u8], class_id: u32) -> R<Option<Animated>> {
	let (joints, seq_count) = (span(class, 8, 1)?[0] as usize, span(class, 0x0c, 1)?[0] as usize);
	if joints == 0 || seq_count == 0 {
		return Ok(None);
	}
	let wrap = |e: String| format!("class {class_id}: {e}");
	let skeleton = parse_skeleton(class, joints).map_err(wrap)?;
	let mut sequences = Vec::new();
	let empty = Sequence { frames: Vec::new(), speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff };
	if class_id == 0 && level.is_some_and(|l| l.title) {
		// The title has no Ratchet sequence table: its scene supplies his frames.
		sequences.push(empty);
	} else if class_id == 0 {
		let level = level.ok_or("class 0 needs the level")?;
		// Ratchet: 256 core offsets in the index; offsets inside a sequence are relative to its own header.
		let table = u32_at(&level.index, 0x78)? as usize;
		for i in 0..seq_count.max(1) {
			let o = u32_at(&level.index, table + i * 4)? as i32;
			if o <= 0 {
				sequences.push(empty.clone());
				continue;
			}
			let block = level.block(o as usize)?;
			sequences.push(parse_sequence(block, block, joints).map_err(|e| format!("class 0 sequence {i}: {e}"))?);
		}
	} else {
		for i in 0..seq_count {
			let o = u32_at(class, 0x48 + i * 4)? as i32;
			if o <= 0 {
				sequences.push(empty.clone());
				continue;
			}
			let header = span(class, o as usize, class.len() - o as usize)?;
			sequences.push(parse_sequence(header, class, joints).map_err(|e| format!("class {class_id} sequence {i}: {e}"))?);
		}
	}
	let attach0 = (|| {
		let t = u32_at(class, 0x1c).ok()? as usize;
		if t == 0 {
			return None;
		}
		let c = u32_at(class, t + 4).ok()? as usize;
		let n = u16_at(class, c).ok()? as usize;
		(n > 0).then(|| span(class, c + 4 + n - 1, 1).ok().map(|b| b[0] as usize)).flatten().filter(|&j| j < joints)
	})();
	// NOV-14.3: attach points 1 and 2 (the sand mouse's bolt joints).
	let attach12 = [1, 2].map(|k| attach_joint_of(class, k).filter(|&j| j < joints));
	let attach_points = (0..32).map(|k| attach_joint_of(class, k).filter(|&j| j < joints)).collect();
	let slot_joints = [0, 1].map(|k| attach_slot_joint(class, k).filter(|&j| j < joints));
	let slot_map = slot_map_of(class, joints);
	let layer_joints = layer_joints_of(class, 12);
	let layer_joints13 = layer_joints_of(class, 13);
	Ok(Some(Animated { skeleton, sequences, attach0, attach12, attach_points,  slot_map, layer_joints, layer_joints13, hold: [Vec::new(), Vec::new()] }))
}

impl Animated {
	/// Model-space position of attach point 0 in sequence `seq` at `ticks` (1/60 s) from its start: FUN_00264140
	/// before the moby rotation and position are applied.
	pub fn attach_pos(&self, seq: usize, ticks: f32) -> Option<[f32; 3]> {
		let j = self.attach0?;
		let w = self.joint_world(&self.pose_after(seq, ticks), j);
		Some([w[3], w[7], w[11]])
	}

	/// Model-space position of joint `j` in sequence `seq` at `ticks` (1/60 s) from its start.
	pub fn joint_pos(&self, seq: usize, ticks: f32, j: usize) -> Option<[f32; 3]> {
		if j >= self.skeleton.parents.len() {
			return None;
		}
		let w = self.joint_world(&self.pose_after(seq, ticks), j);
		Some([w[3], w[7], w[11]])
	}

	/// Per joint local (rotation quaternion `[x, y, z, w]`, scale, translation in raw units) of the pose `a`..`b` at
	/// `t`, before the parent chain: the values `local` turns into matrices.
	pub fn local_parts(&self, a: &Frame, b: &Frame, t: f32, direct: bool) -> Vec<([f32; 4], [f32; 3], [f32; 3])> {
		let n = self.skeleton.parents.len();
		let mut sa = vec![[1.0f32; 3]; n];
		let mut sb = sa.clone();
		let mut ta = self.skeleton.bind_translation.clone();
		let mut tb = ta.clone();
		for &(j, s) in &a.scale { if j < n { sa[j] = s; } }
		for &(j, s) in &b.scale { if j < n { sb[j] = s; } }
		for &(j, v) in &a.translation { if j < n { ta[j] = v; } }
		for &(j, v) in &b.translation { if j < n { tb[j] = v; } }
		(0..n).map(|j| {
			let q = blend_rotation(a.rotation[j], b.rotation[j], t, direct);
			let s = [0, 1, 2].map(|i| sa[j][i] + (sb[j][i] - sa[j][i]) * t);
			let tr = [0, 1, 2].map(|i| ta[j][i] + (tb[j][i] - ta[j][i]) * t);
			(q, s, tr)
		}).collect()
	}

	/// `pose_frames` of one frame whose joint list may be shorter than the skeleton (a pose-copy frame): the
	/// missing joints take the identity rotation, no scale and the bind translation.
	pub fn pose_single(&self, f: &Frame) -> Vec<[f32; 12]> {
		let mut f = f.clone();
		f.rotation.resize(self.skeleton.parents.len(), [0.0, 0.0, 0.0, 1.0]);
		self.pose_frames(&f, &f, 0.0)
	}


	/// `local` with the joint override records applied per joint right after the channel blend (FUN_00268a80, records in
	/// list order): mode 0 `q = r (x) q`, `s *= rs`, `tr += ro`; mode 1 (weight w) `u = 1 - w`, `a = q u`, `b = r w`,
	/// `s = s u + rs w`, `tr = tr u + ro w`, `v = a -+ b` by the sign of `a . b`, `q = v / |v|`.
	fn local_fx(&self, a: &Frame, b: &Frame, t: f32, fx: &[JointFx], direct: bool) -> Vec<Mat> {
		self.local_fx_parts(a, b, t, fx, direct).into_iter().map(|(q, s, tr)| {
			let r = quat_mat(q);
			let mut m = IDENTITY;
			for i in 0..3 {
				for c in 0..3 {
					m[i][c] = r[i][c] * s[c];
				}
				m[i][3] = tr[i];
			}
			m
		}).collect()
	}

	/// `local_fx` as per joint (rotation quaternion, scale, raw translation) before they become matrices.
	fn local_fx_parts(&self, a: &Frame, b: &Frame, t: f32, fx: &[JointFx], direct: bool) -> Vec<([f32; 4], [f32; 3], [f32; 3])> {
		let n = self.skeleton.parents.len();
		let mut sa = vec![[1.0f32; 3]; n];
		let mut sb = sa.clone();
		let mut ta = self.skeleton.bind_translation.clone();
		let mut tb = ta.clone();
		for &(j, s) in &a.scale { if j < n { sa[j] = s; } }
		for &(j, s) in &b.scale { if j < n { sb[j] = s; } }
		for &(j, v) in &a.translation { if j < n { ta[j] = v; } }
		for &(j, v) in &b.translation { if j < n { tb[j] = v; } }
		let mut parts: Vec<_> = (0..n).map(|j| {
			let mut q = blend_rotation(a.rotation[j], b.rotation[j], t, direct);
			let mut s = [0, 1, 2].map(|i| sa[j][i] + (sb[j][i] - sa[j][i]) * t);
			let mut tr = [0, 1, 2].map(|i| ta[j][i] + (tb[j][i] - ta[j][i]) * t);
			for f in fx.iter().filter(|f| f.joint == j && f.mirror.is_none()) {
				let [x, y, z, w] = q;
				let [rx, ry, rz, rw] = f.rot;
				match f.blend {
					None => {
						q = [rw * x + rx * w + ry * z - rz * y, rw * y - rx * z + ry * w + rz * x, rw * z + rx * y - ry * x + rz * w, rw * w - rx * x - ry * y - rz * z];
						s = [0, 1, 2].map(|i| s[i] * f.scale[i]);
						tr = [0, 1, 2].map(|i| tr[i] + f.off[i]);
					}
					Some(wt) => {
						let u = 1.0 - wt;
						let av = [x * u, y * u, z * u, w * u];
						let bv = [rx * wt, ry * wt, rz * wt, rw * wt];
						let d = ((av[1] * bv[1] + av[0] * bv[0]) + av[2] * bv[2]) + av[3] * bv[3];
						let v = if d.is_sign_negative() { [0, 1, 2, 3].map(|i| av[i] - bv[i]) } else { [0, 1, 2, 3].map(|i| av[i] + bv[i]) };
						let nn = ((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]) + v[3] * v[3];
						let inv = 1.0 / nn.sqrt();
						q = v.map(|c| c * inv);
						s = [0, 1, 2].map(|i| s[i] * u + f.scale[i] * wt);
						tr = [0, 1, 2].map(|i| tr[i] * u + f.off[i] * wt);
					}
				}
			}
			(q, s, tr)
		}).collect();
		crate::anim_pose::apply_mirrors(&mut parts, fx, &self.skeleton.parents);
		parts
	}

	/// Skin matrices `World_j * InvBind_j` (3x4 row-major, translation in game units) of sequence `seq` after `ticks`
	/// 50 Hz ticks of a fresh clock (it loops; a fractional tick advances the progress). An absent or empty sequence
	/// yields identity matrices (bind pose). Used where nothing owns a live clock (tests, attach-point lookups).
	pub fn pose_after(&self, seq: usize, ticks: f32) -> Vec<[f32; 12]> {
		let n = self.skeleton.parents.len();
		let Some(s) = self.sequences.get(seq).filter(|s| !s.frames.is_empty()) else { return vec![flat(&IDENTITY); n] };
		let total = s.tick_len().max(1.0);
		let t = ticks.max(0.0) % total;
		let mut c = AnimClock::new(s, seq as u8, 0);
		for _ in 0..t as u32 {
			c.step(s);
		}
		c.p = (c.p + t.fract() * c.r).min(0.999);
		self.pose_clock(&c, None)
	}

	/// World matrix of `joint` (3x4 row-major, translation in game units, Ratchet-model space): the posed
	/// joint frame, i.e. `skin matrix * bind world matrix`. Anything attached to a joint (the held wrench)
	/// has its vertices in this frame. `mats` is a `pose` result.
	pub fn joint_world(&self, mats: &[[f32; 12]], joint: usize) -> [f32; 12] {
		let skin = mats[joint];
		let skin = [[skin[0], skin[1], skin[2], skin[3]], [skin[4], skin[5], skin[6], skin[7]], [skin[8], skin[9], skin[10], skin[11]]];
		flat(&mul(&skin, &self.skeleton.bind_world(joint)))
	}

	/// Pose of exactly one frame (no interpolation), for tests and tools.
	pub fn pose_frame(&self, seq: usize, frame: usize) -> Vec<[f32; 12]> {
		let f = &self.sequences[seq].frames[frame];
		self.pose_frames(f, f, 0.0)
	}

	/// The blended joint state of `a`..`b` at `t` as a full `Frame`: the pose snapshot (0x266660 into the 0x18f0c0 table)
	/// that a later set/blend fades from. Snapshots of a snapshot nest because the result is itself a `Frame`.
	pub fn snapshot_frame(&self, a: &Frame, b: &Frame, t: f32) -> Frame {
		let n = self.skeleton.parents.len();
		let (mut sa, mut ta) = (vec![[1.0f32; 3]; n], self.skeleton.bind_translation.clone());
		let (mut sb, mut tb) = (sa.clone(), ta.clone());
		for &(j, s) in &a.scale { if j < n { sa[j] = s; } }
		for &(j, s) in &b.scale { if j < n { sb[j] = s; } }
		for &(j, v) in &a.translation { if j < n { ta[j] = v; } }
		for &(j, v) in &b.translation { if j < n { tb[j] = v; } }
		let rotation = (0..n).map(|j| {
			let (qa, mut qb) = (a.rotation[j], b.rotation[j]);
			if qa.iter().zip(&qb).map(|(x, y)| x * y).sum::<f32>() < 0.0 {
				qb = qb.map(|x| -x);
			}
			normalize4([0, 1, 2, 3].map(|i| qa[i] * (1.0 - t) + qb[i] * t))
		}).collect();
		let lerp = |x: [f32; 3], y: [f32; 3]| [0, 1, 2].map(|i| x[i] + (y[i] - x[i]) * t);
		Frame {
			speed: a.speed,
			number: a.number,
			rotation,
			scale: (0..n).map(|j| (j, lerp(sa[j], sb[j]))).collect(),
			// [?] what the snapshot 0x266660 keeps of the non-inherited scale records is not decoded; none kept.
			post_scale: Vec::new(),
			translation: (0..n).map(|j| (j, lerp(ta[j], tb[j]))).collect(),
		}
	}

	/// The FROM frame of a clock state, the one moby +0x68 holds (FUN_00241140, Novalis moby_bind_frames 0x2632b0): the
	/// snapshot while +0x52 is 0xff, else frame +0x50 of the PLAYING sequence +0x52, which differs from the requested
	/// sequence +0x53 (`s`) between a set/blend and the next advance, or for good when the clock multiplier is 0 (the
	/// Eudora bridges 0x41c, FUN_002c6180: seq 0 frame 0 blended toward seq 1 frame 0 by the build fraction).
	fn cur_frame<'f>(&'f self, clock: &AnimClock, s: &'f Sequence, snapshot: Option<&'f Frame>) -> &'f Frame {
		if let (true, Some(f)) = (clock.old_seq == 0xff, snapshot) {
			return f;
		}
		let so = match clock.old_seq {
			0xff => s,
			o => self.sequences.get(o as usize).filter(|q| !q.frames.is_empty()).unwrap_or(s),
		};
		&so.frames[(clock.cur.max(0) as usize).min(so.frames.len() - 1)]
	}

	/// Pose of a clock state: frame `cur` (or the snapshot while fading) to frame `next` at progress `p`.
	pub fn pose_clock(&self, clock: &AnimClock, snapshot: Option<&Frame>) -> Vec<[f32; 12]> {
		let n = self.skeleton.parents.len();
		let Some(s) = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty()) else { return vec![flat(&IDENTITY); n] };
		let last = s.frames.len() - 1;
		let b = &s.frames[(clock.next.max(0) as usize).min(last)];
		let a = self.cur_frame(clock, s, snapshot);
		self.pose_frames_direct(a, b, clock.p.clamp(0.0, 1.0), clock.direct_blend())
	}

	/// `pose_clock` with per-joint scale overrides multiplied into the frames' scale (the pause preview's hero joint
	/// scale records, FUN_0024f218): `scales` = (joint, factor per axis).
	pub fn pose_clock_scaled(&self, clock: &AnimClock, snapshot: Option<&Frame>, scales: &[(usize, [f32; 3])]) -> Vec<[f32; 12]> {
		let n = self.skeleton.parents.len();
		let Some(s) = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty()) else { return vec![flat(&IDENTITY); n] };
		let last = s.frames.len() - 1;
		let mut b = s.frames[(clock.next.max(0) as usize).min(last)].clone();
		let mut a = self.cur_frame(clock, s, snapshot).clone();
		for f in [&mut a, &mut b] {
			for &(j, k) in scales {
				match f.scale.iter_mut().find(|e| e.0 == j) {
					Some(e) => e.1 = [0, 1, 2].map(|i| e.1[i] * k[i]),
					None => f.scale.push((j, k)),
				}
			}
		}
		self.pose_frames_direct(&a, &b, clock.p.clamp(0.0, 1.0), clock.direct_blend())
	}


	/// `pose_clock_fx` with full joint override records applied after the blend (BRIDGE-22).
	pub fn pose_clock_fx_full(&self, clock: &AnimClock, snapshot: Option<&Frame>, spins: &[(usize, f32)], fx: &[JointFx]) -> Vec<[f32; 12]> {
		if spins.is_empty() && fx.is_empty() {
			return self.pose_clock(clock, snapshot);
		}
		let n = self.skeleton.parents.len();
		let Some(s) = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty()) else { return vec![flat(&IDENTITY); n] };
		let last = s.frames.len() - 1;
		let mut b = s.frames[(clock.next.max(0) as usize).min(last)].clone();
		let mut a = self.cur_frame(clock, s, snapshot).clone();
		// A constant spin times both quaternions commutes with their blend and normalisation.
		for f in [&mut a, &mut b] {
			for &(j, ang) in spins {
				if let Some(q) = f.rotation.get_mut(j) {
					let (sn, cs) = (ang * 0.5).sin_cos();
					let [x, y, z, w] = *q;
					*q = [cs * x - sn * y, cs * y + sn * x, cs * z + sn * w, cs * w - sn * z];
				}
			}
		}
		self.pose_frames_mode(&a, &b, clock.p.clamp(0.0, 1.0), fx, clock.direct_blend())
	}


	/// `pose_layered` with the HERO-24 hold layers (BRIDGE-58, decomp/notes/hero_movement.md "HERO-24 port spec"): hold
	/// slot i takes a frame from the hold set `hold[i]` when its sequence is below 23, else from the hero's own sequences
	/// (FROM and target picked independently). Application order: weapon `layer1` (jtab 13), weapon `layer` (jtab 12),
	/// hold 1 (jtab 13), hold 0 (jtab 12).
	pub fn pose_held(&self, clock: &AnimClock, snapshot: Option<&Frame>, fx: &[JointFx], layer: Option<&LayerPose>, layer1: Option<&LayerPose>, hold: &[Option<LayerPose>; 2]) -> Vec<[f32; 12]> {
		match self.held_frame(clock, snapshot, layer, layer1, hold) {
			Some(m) => self.pose_frames_fx(&m, &m, 0.0, fx),
			None => self.pose_clock_fx_full(clock, snapshot, &[], fx),
		}
	}

	/// The per joint local (rotation, scale, raw translation) of the `pose_held` pose before the parent chain: what the
	/// hero local evaluator FUN_00254768 leaves in the scratchpad for the pose copy FUN_00208e10 (base frames, the +0x60
	/// layer list, then the +0x64 override records; Veldin OV:66121-66811). Empty for an absent or empty sequence.
	pub fn local_held(&self, clock: &AnimClock, snapshot: Option<&Frame>, fx: &[JointFx], layer: Option<&LayerPose>, layer1: Option<&LayerPose>, hold: &[Option<LayerPose>; 2]) -> Vec<([f32; 4], [f32; 3], [f32; 3])> {
		if let Some(m) = self.held_frame(clock, snapshot, layer, layer1, hold) {
			return self.local_fx_parts(&m, &m, 0.0, fx, false);
		}
		let Some(s) = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty()) else { return Vec::new() };
		let last = s.frames.len() - 1;
		let b = &s.frames[(clock.next.max(0) as usize).min(last)];
		let a = self.cur_frame(clock, s, snapshot);
		self.local_fx_parts(a, b, clock.p.clamp(0.0, 1.0), fx, clock.direct_blend())
	}

	/// The base frame of `pose_held` with its weapon / hold layers blended in, as one frame; None when no layer applies.
	fn held_frame(&self, clock: &AnimClock, snapshot: Option<&Frame>, layer: Option<&LayerPose>, layer1: Option<&LayerPose>, hold: &[Option<LayerPose>; 2]) -> Option<Frame> {
		let n = self.skeleton.parents.len();
		fn pick_in(seqs: &[Sequence], seq: usize, frame: usize) -> Option<&Frame> {
			let s = seqs.get(seq).filter(|s| !s.frames.is_empty())?;
			Some(&s.frames[frame.min(s.frames.len() - 1)])
		}
		let pick = |seq: usize, frame: usize| pick_in(&self.sequences, seq, frame);
		let frames = |l: &LayerPose| pick(l.seq, l.frame).zip(pick(l.from_seq, l.from_frame)).map(|f| (*l, f));
		let hold_pick = |i: usize, seq: usize, frame: usize| if seq < 23 { pick_in(&self.hold[i], seq, frame) } else { pick(seq, frame) };
		let hold_frames = |i: usize, l: &LayerPose| hold_pick(i, l.seq, l.frame).zip(hold_pick(i, l.from_seq, l.from_frame)).map(|f| (*l, f));
		let l0 = layer.filter(|l| l.weight != 0.0 && !self.layer_joints.is_empty()).and_then(frames);
		let l1 = layer1.filter(|l| l.weight != 0.0 && !self.layer_joints13.is_empty()).and_then(frames);
		let h0 = hold[0].as_ref().filter(|l| l.weight != 0.0 && !self.layer_joints.is_empty()).and_then(|l| hold_frames(0, l));
		let h1 = hold[1].as_ref().filter(|l| l.weight != 0.0 && !self.layer_joints13.is_empty()).and_then(|l| hold_frames(1, l));
		let bs = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty());
		let bs = bs.filter(|_| l0.is_some() || l1.is_some() || h0.is_some() || h1.is_some())?;
		let last = bs.frames.len() - 1;
		let b1 = &bs.frames[(clock.next.max(0) as usize).min(last)];
		let b0 = self.cur_frame(clock, bs, snapshot);
		let mut m = self.snapshot_frame(b0, b1, clock.p.clamp(0.0, 1.0));
		for (layer, joints) in [(l1, &self.layer_joints13), (l0, &self.layer_joints), (h1, &self.layer_joints13), (h0, &self.layer_joints)] {
			let Some((l, (lt, lf))) = layer else { continue };
			let lay = self.snapshot_frame(lf, lt, l.t.clamp(0.0, 1.0));
			mix_layer(&mut m, &lay, l.weight, joints, n);
		}
		Some(m)
	}

	/// M3a F5: the base frame with retail's +0x60 layer list blended in, head first (the order `held_frame` uses), each over
	/// its own jtab joint list; a frame flagged `from_hold` / `to_hold` comes from that hold set (FUN_00263db8), otherwise from
	/// the hero's own sequences. None when no layer has weight and frames.
	pub fn list_frame(&self, clock: &AnimClock, snapshot: Option<&Frame>, layers: &[ListLayer]) -> Option<Frame> {
		let n = self.skeleton.parents.len();
		let pick = |hold: Option<usize>, seq: usize, frame: usize| {
			let seqs: &[Sequence] = match hold {
				Some(i) => self.hold.get(i).map_or(&[][..], |h| &h[..]),
				None => &self.sequences,
			};
			let s = seqs.get(seq).filter(|s| !s.frames.is_empty())?;
			Some(&s.frames[frame.min(s.frames.len() - 1)])
		};
		let live: Vec<_> = layers.iter().filter(|l| l.pose.weight != 0.0 && !l.joints.is_empty()).filter_map(|l| {
			let to = pick(l.to_hold, l.pose.seq, l.pose.frame)?;
			let from = pick(l.from_hold, l.pose.from_seq, l.pose.from_frame)?;
			Some((l, to, from))
		}).collect();
		if live.is_empty() {
			return None;
		}
		let bs = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty())?;
		let last = bs.frames.len() - 1;
		let b1 = &bs.frames[(clock.next.max(0) as usize).min(last)];
		let b0 = self.cur_frame(clock, bs, snapshot);
		let mut m = self.eval_frame(b0, b1, clock.p.clamp(0.0, 1.0));
		for (l, lt, lf) in live {
			let p = &l.pose;
			let lay = self.layer_frame(lf, lt, p.t, p.from_seq == p.seq && p.from_frame + 1 == p.frame);
			mix_layer_vu(&mut m, &lay, p.weight, &l.joints, n);
		}
		Some(m)
	}

	/// Local pose `a`..`b` at `t` as the retail joint evaluator FUN_00267b58 blends it before composing (the arithmetic of
	/// `joint_positions`, COL-6): t = 0 (bit pattern) keeps `a` raw; otherwise rotation nlerp in VU arithmetic (difference
	/// when the dot of the weighted parts is negative, one VRSQRT), scale and translation `x + (y - x) t`.
	pub fn eval_frame(&self, a: &Frame, b: &Frame, t: f32) -> Frame {
		use crate::physics::{vadd1, vmul, vsub1};
		let raw = t.to_bits() == 0;
		self.blend_frames(a, b, |qa, qb| if raw { qa } else { nlerp_vu(qa, qb, t) }, |x, y| if raw { x } else { vadd1(x, vmul(vsub1(y, x), t)) })
	}

	/// Layer pose of a +0x60 layer object as FUN_00267308 builds it [C: NOV OV:58337-58881] from its FROM frame (+0x38)
	/// and TO frame (+0x3c) at its raw t (+0x24, never clamped: the hold clock FUN_0022e128 leaves t >= 1 in place):
	/// t = 0 (bit pattern) copies FROM; FROM and TO consecutive frames of one sequence (+0x22 == +0x23 and +0x20 + 1 ==
	/// +0x21) lerp the rotation `qa (1 - t) + qb t` with no sign test and no normalisation (`lerp_consecutive`); otherwise the VU nlerp of
	/// `eval_frame`. Scale and translation are `x (1 - t) + y t` (VMULA / VMADD).
	pub fn layer_frame(&self, a: &Frame, b: &Frame, t: f32, consecutive: bool) -> Frame {
		use crate::physics::{vadd1, vmul, vsub1};
		let raw = t.to_bits() == 0;
		let u = vsub1(1.0, t);
		let rot = |qa: [f32; 4], qb: [f32; 4]| if raw || consecutive { lerp_consecutive(qa, qb, t) } else { nlerp_vu(qa, qb, t) };
		self.blend_frames(a, b, rot, |x, y| if raw { x } else { vadd1(vmul(x, u), vmul(y, t)) })
	}

	/// Every joint of `a` / `b` (scale 1 and the bind translation where a frame has no record) blended per component.
	fn blend_frames(&self, a: &Frame, b: &Frame, rot: impl Fn([f32; 4], [f32; 4]) -> [f32; 4], mix: impl Fn(f32, f32) -> f32) -> Frame {
		let n = self.skeleton.parents.len();
		let (mut sa, mut ta) = (vec![[1.0f32; 3]; n], self.skeleton.bind_translation.clone());
		let (mut sb, mut tb) = (sa.clone(), ta.clone());
		for &(j, s) in &a.scale { if j < n { sa[j] = s; } }
		for &(j, s) in &b.scale { if j < n { sb[j] = s; } }
		for &(j, v) in &a.translation { if j < n { ta[j] = v; } }
		for &(j, v) in &b.translation { if j < n { tb[j] = v; } }
		let q = |f: &Frame, j: usize| f.rotation.get(j).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]);
		Frame {
			speed: a.speed,
			number: a.number,
			rotation: (0..n).map(|j| rot(q(a, j), q(b, j))).collect(),
			scale: (0..n).map(|j| (j, [0, 1, 2].map(|k| mix(sa[j][k], sb[j][k])))).collect(),
			// [merge M8] The hero layer path keeps no non-inherited scale records (m3b part7 `post_scale_list` is the moby
			// evaluator's); before part7 the parser dropped them, so this path is unchanged.
			post_scale: Vec::new(),
			translation: (0..n).map(|j| (j, [0, 1, 2].map(|k| mix(ta[j][k], tb[j][k])))).collect(),
		}
	}

	/// `joint_positions` of a clock state, frames chosen as `pose_clock`; None for an absent or empty sequence.
	fn joint_positions_clock(&self, clock: &AnimClock, snapshot: Option<&Frame>) -> Option<Vec<[f32; 3]>> {
		let s = self.sequences.get(clock.new_seq as usize).filter(|s| !s.frames.is_empty())?;
		let last = s.frames.len() - 1;
		let b = &s.frames[(clock.next.max(0) as usize).min(last)];
		let a = self.cur_frame(clock, s, snapshot);
		Some(self.joint_positions(a, b, clock.p.clamp(0.0, 1.0)))
	}

	/// Model-space joint positions (game units) of the pose `a`..`b` at `t` as the retail joint evaluator FUN_00267b58
	/// composes them (collision_and_gameplay.md COL-6), VU arithmetic throughout: t = 0 (bit pattern) uses the stored
	/// quaternion unnormalised; otherwise nlerp of qa (1 - t) and qb t (difference when the dot's sign bit is set),
	/// normalised by one VRSQRT. Rotation columns from the doubled quaternion scaled by the joint scale, translation
	/// record or bind lerped, then composed with the parent columns and translation; output = translation * unit.
	pub fn joint_positions(&self, a: &Frame, b: &Frame, t: f32) -> Vec<[f32; 3]> {
		use crate::physics::{vadd1, vmul, vrsqrt, vsub1};
		let n = self.skeleton.parents.len();
		let raw = t.to_bits() == 0;
		let (mut sa, mut ta) = (vec![[1.0f32; 3]; n], self.skeleton.bind_translation.clone());
		let (mut sb, mut tb) = (sa.clone(), ta.clone());
		for &(j, s) in &a.scale { if j < n { sa[j] = s; } }
		for &(j, s) in &b.scale { if j < n { sb[j] = s; } }
		for &(j, v) in &a.translation { if j < n { ta[j] = v; } }
		for &(j, v) in &b.translation { if j < n { tb[j] = v; } }
		// The port stores w negated; the evaluator works on the stored sign.
		let stored = |f: &Frame, j: usize| f.rotation.get(j).map_or([0.0, 0.0, 0.0, -1.0], |q| [q[0], q[1], q[2], -q[3]]);
		let lerp = |x: f32, y: f32| if raw { x } else { vadd1(x, vmul(vsub1(y, x), t)) };
		let mut world: Vec<([[f32; 3]; 3], [f32; 3])> = Vec::with_capacity(n);
		for j in 0..n {
			let qa = stored(a, j);
			let q = if raw {
				qa
			} else {
				let (u, qb) = (vsub1(1.0, t), stored(b, j));
				let (pa, pb) = (qa.map(|v| vmul(v, u)), qb.map(|v| vmul(v, t)));
				let p = [0, 1, 2, 3].map(|i| vmul(pa[i], pb[i]));
				let dot = vadd1(vadd1(vadd1(p[0], p[1]), p[2]), p[3]);
				let d = [0, 1, 2, 3].map(|i| if dot.is_sign_negative() { vsub1(pa[i], pb[i]) } else { vadd1(pa[i], pb[i]) });
				let sq = d.map(|v| vmul(v, v));
				let k = vrsqrt(1.0, vadd1(vadd1(vadd1(sq[0], sq[1]), sq[2]), sq[3]));
				d.map(|v| vmul(v, k))
			};
			let [x, y, z, w] = q;
			let (x2, y2, z2) = (vadd1(x, x), vadd1(y, y), vadd1(z, z));
			let one = |p: f32, q: f32| vsub1(vsub1(1.0, p), q);
			let c = [
				[one(vmul(y2, y), vmul(z2, z)), vsub1(vmul(x2, y), vmul(z2, w)), vadd1(vmul(x2, z), vmul(y2, w))],
				[vadd1(vmul(x2, y), vmul(z2, w)), one(vmul(x2, x), vmul(z2, z)), vsub1(vmul(y2, z), vmul(x2, w))],
				[vsub1(vmul(x2, z), vmul(y2, w)), vadd1(vmul(y2, z), vmul(x2, w)), one(vmul(x2, x), vmul(y2, y))],
			];
			let sc = [0, 1, 2].map(|k| lerp(sa[j][k], sb[j][k]));
			let c = [0, 1, 2].map(|k| c[k].map(|v| vmul(v, sc[k])));
			let tr = [0, 1, 2].map(|i| lerp(ta[j][i], tb[j][i]));
			world.push(match self.skeleton.parents[j].filter(|&p| p < j) {
				Some(p) => {
					let (pc, pt) = world[p];
					let xf = |v: [f32; 3]| [0, 1, 2].map(|i| vadd1(vadd1(vmul(pc[0][i], v[0]), vmul(pc[1][i], v[1])), vmul(pc[2][i], v[2])));
					let r = xf(tr);
					(c.map(xf), [0, 1, 2].map(|i| vadd1(r[i], pt[i])))
				}
				None => (c, tr),
			});
		}
		world.iter().map(|(_, t)| t.map(|v| vmul(v, self.skeleton.unit))).collect()
	}

	pub fn pose_frames(&self, a: &Frame, b: &Frame, t: f32) -> Vec<[f32; 12]> {
		self.pose_frames_fx(a, b, t, &[])
	}

	/// `pose_frames` with retail's same-sequence blend (`AnimClock::direct_blend`): the rotations are lerped and used
	/// without the sign test and normalisation.
	pub fn pose_frames_direct(&self, a: &Frame, b: &Frame, t: f32, direct: bool) -> Vec<[f32; 12]> {
		self.pose_frames_mode(a, b, t, &[], direct)
	}

	/// `pose_frames` with the joint override records (`local_fx`).
	pub fn pose_frames_fx(&self, a: &Frame, b: &Frame, t: f32, fx: &[JointFx]) -> Vec<[f32; 12]> {
		self.pose_frames_mode(a, b, t, fx, false)
	}

	fn pose_frames_mode(&self, a: &Frame, b: &Frame, t: f32, fx: &[JointFx], direct: bool) -> Vec<[f32; 12]> {
		let local = self.local_fx(a, b, t, fx, direct);
		let mut world: Vec<Mat> = Vec::with_capacity(local.len());
		for (j, l) in local.iter().enumerate() {
			world.push(match self.skeleton.parents[j] {
				Some(p) => mul(&world[p], l),
				None => *l,
			});
		}
		for (j, s) in post_scale_list(a, b, t) {
			if let Some(w) = world.get_mut(j) {
				for r in 0..3 {
					for c in 0..3 {
						w[r][c] *= s[c];
					}
				}
			}
		}
		world.iter().zip(&self.skeleton.inverse_bind).map(|(w, inv)| {
			let mut m = mul(w, inv);
			for r in 0..3 {
				m[r][3] *= self.skeleton.unit;
			}
			flat(&m)
		}).collect()
	}
}

/// VU0 FTOI0 + PPACH: a skinned position is truncated to whole raw units (`unit` game units each) and kept as 16 bits.
pub fn vu0_quantise(p: [f32; 3], unit: f32) -> [f32; 3] {
	p.map(|x| ((x / unit) as i32 as i16) as f32 * unit)
}

/// Weighted skinning of the class's LOD 0 vertices; returns (positions, normals) in the space of
/// `class.mesh.positions`. Normals are transformed by the rotation part and renormalised.
pub fn skin(class: &MobyClass, mats: &[[f32; 12]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
	let mut positions = Vec::with_capacity(class.skin.len());
	let mut normals = Vec::with_capacity(class.skin.len());
	for ((p, n), s) in class.mesh.positions.iter().zip(&class.normals).zip(&class.skin) {
		let (mut pa, mut na) = ([0.0f32; 3], [0.0f32; 3]);
		for k in 0..3 {
			if s.weights[k] == 0.0 {
				continue;
			}
			let w = s.weights[k];
			let Some(m) = mats.get(s.joints[k] as usize) else { continue };
			for r in 0..3 {
				pa[r] += w * (m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]);
				na[r] += w * (m[r * 4] * n[0] + m[r * 4 + 1] * n[1] + m[r * 4 + 2] * n[2]);
			}
		}
		let len = (na[0] * na[0] + na[1] * na[1] + na[2] * na[2]).sqrt();
		positions.push(vu0_quantise(pa, class.scale / 1024.0));
		normals.push(if len > 0.0 { na.map(|x| x / len) } else { *n });
	}
	(positions, normals)
}

/// NOV-35b: the class's metal-packet vertices (packet order) skinned with the same joint matrices as the LOD packets
/// (FUN_00210358 raw 0x2113a8-0x211648, VU0 prog0 L_0990): M = sum w_i S_(j_i) (one joint: S_j0 unweighted), p' = FTOI0(M p + t)
/// (quantised like the LOD positions), n' = M3x3 n NOT renormalised (the sphere-map UV and the metal light use it as is).
pub fn skin_metal(class: &MobyClass, mats: &[[f32; 12]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
	let (mut positions, mut normals) = (Vec::new(), Vec::new());
	for pk in &class.metal {
		for ((p, n), s) in pk.positions.iter().zip(&pk.normals).zip(&pk.skin) {
			// Joint count 0 (classes without joints): the model-space vertex as stored [I].
			if s.weights == [0.0; 3] {
				positions.push(*p);
				normals.push(*n);
				continue;
			}
			let (mut pa, mut na) = ([0.0f32; 3], [0.0f32; 3]);
			for k in 0..3 {
				if s.weights[k] == 0.0 {
					continue;
				}
				let w = s.weights[k];
				let Some(m) = mats.get(s.joints[k] as usize) else { continue };
				for r in 0..3 {
					pa[r] += w * (m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]);
					na[r] += w * (m[r * 4] * n[0] + m[r * 4 + 1] * n[1] + m[r * 4 + 2] * n[2]);
				}
			}
			positions.push(vu0_quantise(pa, class.scale / 1024.0));
			normals.push(na);
		}
	}
	(positions, normals)
}

/// `skin` with the metal-packet vertices (`skin_metal`) appended after the LOD vertices: the prop poses carry both, the
/// props batch reads the first `mesh.positions.len()` and the metal batch (BRIDGE-45) the rest.
pub fn skin_all(class: &MobyClass, mats: &[[f32; 12]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
	let (mut ps, mut ns) = skin(class, mats);
	if !class.metal.is_empty() {
		let (mp, mn) = skin_metal(class, mats);
		ps.extend(mp);
		ns.extend(mn);
	}
	(ps, ns)
}

/// The pause preview's back item moby (FUN_002825a0): sequence 1 idles; class 607 (0x25f) opens with sequence 6
/// (10-tick blend) for 8 end-of-loop events, then fades to sequence 1.
#[derive(Clone)]
pub struct BackAnim {
	pub class: u32,
	pub pa: PropAnim,
	left: u32,
}

impl BackAnim {
	pub fn new(class: u32, seqs: std::rc::Rc<Vec<Sequence>>) -> Self {
		let mut pa = PropAnim::new(1);
		pa.attach(Some(seqs));
		let left = if class == 607 { 8 } else { 0 };
		if class == 607 {
			pa.blend(6, 0, 10);
		} else {
			pa.set(1, 0);
		}
		BackAnim { class, pa, left }
	}

	/// One 50 Hz tick; the end-of-loop flag drives the sequence changes.
	pub fn step(&mut self) {
		self.pa.step();
		if !self.pa.wrapped {
			return;
		}
		// The sequence last stepped (+0x52) is the current one at an end-of-loop event: the stepper writes it at the
		// first advance after a change, which is before the flag can be set.
		let cur = self.pa.seq;
		if self.class == 607 && cur == 6 {
			self.left = self.left.saturating_sub(1);
			if self.left == 0 {
				if cur != 1 {
					self.pa.blend(1, 0, 10);
				}
			}
		} else if cur != 1 {
			self.pa.blend(1, 0, 0);
		}
	}
}

/// Hero joints copied onto a hand item's joints 0..16 (list 0x17a5c0: item joint k = hero joint list[k]).
pub const HAND_COPY: [usize; 17] = [54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70];
/// Head class 433: item joints 1..15 = hero joints (list 0x17a608), then 6 identity joints.
pub const HEAD_COPY_433: [usize; 16] = [7, 8, 9, 10, 16, 17, 25, 26, 29, 30, 33, 34, 35, 36, 41, 44];
pub const HEAD_COPY_433_EXTRA: usize = 6;
/// The other head classes (1289, 1290): item joints 1, 2 = hero joints 8, 10 (list 0x15f688).
pub const HEAD_COPY: [usize; 3] = [7, 8, 10];

/// FUN_00208e10: the joint frame an item skeleton takes from the hero's live local joints. Joint 0 is the identity;
/// item joint k (>= 1) copies hero joint `list[k]` (`list[0]` is skipped) with the rotation quantised to 1/32768
/// (truncated, clamped), a scale record only when the scale quantised to 1/4096 differs from (1, 1, 1), and a
/// translation record only when the truncated raw translation differs from the truncated item bind translation;
/// `extra` identity joints follow. `hero` is `PropAnim::local_trs` of the hero.
pub fn pose_copy_frame(hero: &[([f32; 4], [f32; 3], [f32; 3])], list: &[usize], extra: usize, item_bind: &[[f32; 3]]) -> Frame {
	let q = |v: f32| ((v * 32768.0).trunc().clamp(-32768.0, 32767.0)) / 32768.0;
	let mut rotation = vec![[0.0, 0.0, 0.0, 1.0]];
	let (mut scale, mut translation) = (Vec::new(), Vec::new());
	for k in 1..list.len() {
		let Some(&(r, s, t)) = hero.get(list[k]) else {
			rotation.push([0.0, 0.0, 0.0, 1.0]);
			continue;
		};
		rotation.push(r.map(q));
		let qs = s.map(|v| (v * 4096.0).trunc());
		if qs != [4096.0; 3] {
			scale.push((k, qs.map(|v| v / 4096.0)));
		}
		let qt = t.map(f32::trunc);
		if item_bind.get(k).map(|b| b.map(f32::trunc)) != Some(qt) {
			translation.push((k, qt));
		}
	}
	for _ in 0..extra {
		rotation.push([0.0, 0.0, 0.0, 1.0]);
	}
	Frame { speed: 1.0, number: 0.0, rotation, scale, post_scale: Vec::new(), translation }
}

/// In-play pose of a hand-copy item (the gloves, `inventory::hand_pose_copy`): HeroItemsAttach FUN_0020f0c8 (Veldin
/// OV:26176-26261) skips the item's clock step and points both its frames at the pose-copy frame of the hero's live
/// local joints (list 0x17a5c0) every tick, so its own sequences never show (decomp/notes/bomb_glove.md "HELD-GLOVE").
/// `hero_local` is `PropAnim::local_held` of the same hero pose the hero is skinned with.
pub fn hand_copy_pose(item: &Animated, hero_local: &[([f32; 4], [f32; 3], [f32; 3])]) -> Vec<[f32; 12]> {
	item.pose_single(&pose_copy_frame(hero_local, &HAND_COPY, 0, &item.skeleton.bind_translation))
}

#[cfg(test)]
mod tests {
	// Every ramp row has a fade length (the HERO-HURT entry uses row 2): the tick where the row first reaches 1.0.
	/// Inverse-bind columns as the retail skeleton fill reads them (decomp/notes/moby_animation.md REN-5).
	#[test]
	fn inverse_bind_columns_retail() {
		let class = |q: [[f32; 4]; 4]| {
			let mut c = vec![0u8; 0x100];
			c[0x14..0x18].copy_from_slice(&0x40u32.to_le_bytes());
			c[0x18..0x1c].copy_from_slice(&0xc0u32.to_le_bytes());
			c[0x24..0x28].copy_from_slice(&1024.0f32.to_le_bytes());
			for (k, v) in q.iter().enumerate() {
				for (l, x) in v.iter().enumerate() {
					c[0x40 + 16 * k + 4 * l..0x40 + 16 * k + 4 * l + 4].copy_from_slice(&x.to_le_bytes());
				}
			}
			c
		};
		let w: Mat = [[1.0, 0.0, 0.0, 10.0], [0.0, 1.0, 0.0, 20.0], [0.0, 0.0, 1.0, 30.0]];
		let s = parse_skeleton(&class([[0.0, 1.0, 0.0, 9.0], [-1.0, 0.0, 0.0, 9.0], [0.0, 0.0, 1.0, 9.0], [5.0, 6.0, 7.0, 0.0]]), 1).unwrap();
		assert_eq!(mul(&w, &s.inverse_bind[0]), [[0.0, -1.0, 0.0, 15.0], [1.0, 0.0, 0.0, 26.0], [0.0, 0.0, 1.0, 37.0]]);
		// The w lanes change nothing.
		let s2 = parse_skeleton(&class([[0.0, 1.0, 0.0, 1.0], [-1.0, 0.0, 0.0, 2.0], [0.0, 0.0, 1.0, 3.0], [5.0, 6.0, 7.0, 4.0]]), 1).unwrap();
		assert_eq!(s.inverse_bind[0], s2.inverse_bind[0]);
		// W = 90 degrees about z, translation (1, 2, 3); R = identity, c = (-0.5, 0.25, -2).
		let w: Mat = [[0.0, -1.0, 0.0, 1.0], [1.0, 0.0, 0.0, 2.0], [0.0, 0.0, 1.0, 3.0]];
		let s3 = parse_skeleton(&class([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [-0.5, 0.25, -2.0, 0.0]]), 1).unwrap();
		assert_eq!(mul(&w, &s3.inverse_bind[0]), [[0.0, -1.0, 0.0, 0.75], [1.0, 0.0, 0.0, 1.5], [0.0, 0.0, 1.0, 1.0]]);
	}

	fn flat_seqs(n: usize) -> std::rc::Rc<Vec<Sequence>> {
		let f = || Frame { speed: 0.5, number: 0.0, rotation: vec![[0.0, 0.0, 0.0, 1.0]], scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() };
		std::rc::Rc::new((0..n).map(|_| Sequence { frames: vec![f(), f()], speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff }).collect())
	}

	#[test]
	fn pose_copy_quantises_rotation_scale_and_translation() {
		let mut hero = vec![([0.0, 0.0, 0.0, 1.0], [1.0; 3], [0.0; 3]); 71];
		hero[55] = ([0.70710677, -0.70710677, 0.0, 1.0], [1.0001; 3], [12.7, 1.0, 2.0]);
		hero[56] = ([0.0; 4], [1.0, 1.5, 1.0], [0.0; 3]);
		let bind = vec![[12.0, 1.0, 2.0]; 17];
		let f = pose_copy_frame(&hero, &HAND_COPY, 0, &bind);
		assert_eq!(f.rotation.len(), 17);
		assert_eq!(f.rotation[0], [0.0, 0.0, 0.0, 1.0]);
		assert_eq!(f.rotation[1], [23170.0 / 32768.0, -23170.0 / 32768.0, 0.0, 32767.0 / 32768.0]);
		// Scale 1.0001 quantises to 4096 (no record); 1.5 keeps its record. Translation 12.7 vs bind 12.0 both
		// truncate to 12 (no record).
		assert_eq!(f.scale, vec![(2, [1.0, 1.5, 1.0])]);
		assert!(f.translation.iter().all(|&(j, _)| j != 1));
		assert_eq!(f.translation.iter().map(|t| t.0).collect::<Vec<_>>(), (2..17).collect::<Vec<_>>());
		assert_eq!(pose_copy_frame(&hero, &HEAD_COPY_433, HEAD_COPY_433_EXTRA, &bind).rotation.len(), 22);
		assert_eq!(pose_copy_frame(&hero, &HEAD_COPY, 0, &bind).rotation.len(), 3);
	}

	use super::*;
	use crate::mobies::moby_classes;
	use crate::{iso, toc};
	use std::path::PathBuf;

	fn frame(rotation: Vec<[f32; 4]>) -> Frame {
		Frame { speed: 1.0, number: 0.0, rotation, scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }
	}

	/// HERO-10.1b test 1: the layer joint list of a synthetic class (jtab at 0x40, entry 12 -> record at 0x80).
	#[test]
	fn layer_joint_list() {
		let mut c = vec![0u8; 0x100];
		c[0x1c..0x20].copy_from_slice(&0x40u32.to_le_bytes());
		c[0x40 + 4 + 4 * 12..0x40 + 4 + 4 * 12 + 4].copy_from_slice(&0x80u32.to_le_bytes());
		c[0x80..0x89].copy_from_slice(&[2, 0, 3, 0, 9, 9, 5, 6, 7]);
		assert_eq!(layer_joints_of(&c, 12), vec![5, 6, 7]);
		assert!(layer_joints_of(&c, 13).is_empty());
	}

	/// HERO-20 test 4: the attach-slot map of a synthetic class (jtab at 0x40, entry 15 -> chain at 0xa0 with
	/// count 2, joints 7, 9 and the slot joint 23 at c + 4 + 2).
	#[test]
	fn attach_slot_map() {
		let mut c = vec![0u8; 0x100];
		c[0x1c..0x20].copy_from_slice(&0x40u32.to_le_bytes());
		c[0x40 + 4 + 4 * 15..0x40 + 4 + 4 * 15 + 4].copy_from_slice(&0xa0u32.to_le_bytes());
		c[0xa0..0xa7].copy_from_slice(&[2, 0, 0, 0, 7, 9, 23]);
		let map = slot_map_of(&c, 30);
		assert_eq!((map.len(), map[15]), (32, Some(23)));
		assert_eq!(slot_map_of(&c, 23)[15], None);
	}

	/// UI-3b test 1, 2, 4: the boots' attach points 2 / 3 are hero joints 99 / 93, the spawn scale 0x3c23d70a is
	/// 0.01, and owned extra items 35 / 33 resolve to classes 407 / 614.
	#[test]
	fn worn_attach_points_and_classes() {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		let l = crate::level::Level::load(&mut disc, &levels[0]).unwrap();
		assert_eq!((attach_joint(&l, 0, 2), attach_joint(&l, 0, 3)), (Some(99), Some(93)));
		assert_eq!(f32::from_bits(0x3c23d70a), 0.01f32);
		assert_eq!((crate::inventory::preview_class(35), crate::inventory::preview_class(33)), (Some(407), Some(614)));
	}

	#[test]
	fn menu_clips_load() {
		let (mut disc, _) = open();
		let c = menu_clips(&mut disc, 111).unwrap();
		assert_eq!(c.len(), MENU_CLIPS);
		assert_eq!((c[0].frames.len(), c[3].frames.len(), c[27].frames.len()), (146, 253, 72));
		assert_eq!((c[0].speed, c[3].speed), (0.5, 0.0));
	}

	fn open() -> (iso::Disc, Vec<toc::Level>) {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(&path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		(disc, levels)
	}

	fn bbox(p: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
		let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
		for v in p {
			for i in 0..3 {
				lo[i] = lo[i].min(v[i]);
				hi[i] = hi[i].max(v[i]);
			}
		}
		(lo, hi)
	}

	/// Mean |posed edge length - bind edge length| / bbox height over all faces.
	fn distortion(class: &MobyClass, posed: &[[f32; 3]]) -> f32 {
		let d = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
		let (mut sum, mut n) = (0.0, 0);
		for (_, f) in &class.mesh.faces {
			for e in 0..3 {
				let (a, b) = (f[e] as usize, f[(e + 1) % 3] as usize);
				sum += (d(posed[a], posed[b]) - d(class.mesh.positions[a], class.mesh.positions[b])).abs();
				n += 1;
			}
		}
		let (lo, hi) = bbox(&class.mesh.positions);
		sum / n as f32 / (hi[2] - lo[2])
	}

	// AUD-11d test 1: Novalis hero sequence triggers (cs, time) and no hero loop byte.
	#[test]
	fn novalis_hero_triggers() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let anim = animation(&level, 0).unwrap().unwrap();
		let trig = |n: usize| -> Vec<(u32, u32)> { anim.sequences[n].triggers.iter().map(|w| (w & 0xffff, w >> 16)).collect() };
		assert_eq!(trig(7), vec![(26, 8), (30, 160), (31, 208)]);
		assert_eq!(trig(16), vec![(33, 24), (39, 64), (31, 112)]);
		assert!(anim.sequences.iter().all(|s| s.loop_sound == 0xff));
	}

	// HELD-GLOVE (decomp/notes/bomb_glove.md): the Veldin gloves (192 Bomb, 190 Mine, 229 Doom) posed by the hero hand copy
	// at hero joint 54 put every glove joint k (1..16) on hero joint 54 + k: at idle the glove wrist (joint 2) is the hero
	// wrist 56 (-0.0112, -0.3130, 0.5244), also under a weapon layer and in seq 48 where the glove's own idle sequence 1
	// (the old draw) left the wrist 0.4 units off the hand.
	#[test]
	fn veldin_glove_hand_copy_follows_the_hand() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		assert!(!hero.layer_joints.is_empty());
		// FUN_0020d018: only the gloves; the Blaster, Pyrocitor, Suck Cannon and Taunter keep their own clocks.
		let copied: Vec<u8> = (0..crate::inventory::GADGETS as u8).filter(|&id| crate::inventory::hand_pose_copy(id)).collect();
		assert_eq!(copied, [10, 17, 20, 25]);
		let world =|m: &[f32; 12], v: [f32; 3]| [0, 1, 2].map(|r| m[r * 4] * v[0] + m[r * 4 + 1] * v[1] + m[r * 4 + 2] * v[2] + m[r * 4 + 3]);
		let dist = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
		let layer = LayerPose { seq: 44, frame: 11, from_seq: 44, from_frame: 11, t: 0.0, weight: 1.0 };
		for class in [192, 190, 229] {
			let glove = gadget_animation(&level, class).unwrap().unwrap();
			assert_eq!(glove.skeleton.parents.len(), HAND_COPY.len(), "class {class}");
			for (seq, frame, l) in [(0, 0, None), (48, 1, None), (0, 0, Some(&layer))] {
				let mut pa = PropAnim::new(seq);
				pa.attach(Some(share_timing(&hero.sequences)));
				pa.set(seq, frame);
				let mats = pa.pose_held(&hero, &[], l, None, &[None, None]);
				let gm = hand_copy_pose(&glove, &pa.local_held(&hero, &[], l, None, &[None, None]));
				let j54 = hero.joint_world(&mats, 54);
				let at = |k: usize| {
					let g = glove.joint_world(&gm, k);
					world(&j54, [g[3], g[7], g[11]])
				};
				for k in 1..HAND_COPY.len() {
					let h = hero.joint_world(&mats, 54 + k);
					assert!(dist(at(k), [h[3], h[7], h[11]]) < 1e-4, "class {class} seq {seq} joint {k}: {:?} vs {h:?}", at(k));
				}
				if (seq, l.is_none()) == (0, true) {
					assert!(dist(at(2), [-0.011197273, -0.31303102, 0.5243815]) < 1e-4, "{:?}", at(2));
				}
				if seq == 48 {
					let own = glove.joint_world(&glove.pose_frame(1, 0), 2);
					let h = hero.joint_world(&mats, 56);
					assert!(dist(world(&j54, [own[3], own[7], own[11]]), [h[3], h[7], h[11]]) > 0.3);
				}
			}
		}
	}

	// HERO-24 test 1: the hold sets (classes 1 / 2) load on Novalis and map onto the hero layer joints 12 / 13.
	#[test]
	fn novalis_hold_sets() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		let n = hero.skeleton.parents.len();
		for (class, joints) in [(1, &hero.layer_joints), (2, &hero.layer_joints13)] {
			let a = animation(&level, class).unwrap().unwrap();
			assert_eq!((a.skeleton.parents.len(), a.sequences.len()), (20, 23), "class {class}");
			let h = hold_sequences(&level, class, n).unwrap();
			assert_eq!(h.len(), 23);
			assert!(h.iter().flat_map(|s| &s.frames).all(|f| f.rotation.len() == n));
			let f = &h[0].frames[0];
			assert!(!joints.is_empty());
			assert!(joints.iter().all(|&j| f.rotation[j] != [0.0; 4]), "class {class}");
		}
	}

	// HERO-20: the hero (class 0) and Clank (class 601) attach-slot maps of Novalis from the class data.
	#[test]
	fn novalis_hero_clank_slot_maps() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let hero = animation(&level, 0).unwrap().unwrap();
		let want = [56, 55, 99, 93, 7, 5, 54, 98, 92, 0, 2, 6, 52, 72, 76, 23, 21, 24, 22, 50, 49, 18, 100, 94, 8, 104, 106, 108, 110, 3, 5];
		assert_eq!(hero.slot_map[..31], want.map(Some));
		assert_eq!(hero.slot_map[31], None);
		let clank = animation(&level, 601).unwrap().unwrap();
		// The four Clank blink records register on slots 2..5 (CLANK_JOINTS).
		assert_eq!(clank.slot_map[2..6], [Some(15), Some(17), Some(21), Some(23)]);
	}

	// NOV-35a test 2: the Novalis purchase prompt (class 1137) attach points 0 / 1 / 2 are joints 2 / 1 / 4.
	#[test]
	fn novalis_prompt_attach_points() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let a = animation(&level, 1137).unwrap().unwrap();
		assert_eq!(a.attach0, Some(2));
		assert_eq!(a.attach12, [Some(1), Some(4)]);
		let w = a.joint_world(&a.pose_frame(1, 0), 4);
		for (got, want) in [w[3], w[7], w[11]].iter().zip([0.4607226, -0.41478366, 0.9498151]) {
			assert!((got - want).abs() < 1e-4, "{w:?}");
		}
	}

	// M3b: the shop icon / popup bolt (Eudora class 13) at seq 0 frame 0 vs its stored (bind) vertices.
	#[test]
	fn eudora_bolt_pose_vs_bind() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[4]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let c = &classes[&13];
		let a = animation(&level, 13).unwrap().unwrap();
		let m = a.pose_frame(0, 0);
		// One joint, seq 0 has one frame and it is the bind pose (identity, translation < 1e-4): the shop icon / popup bolt
		// is the stored mesh, so its shape rides only on the slot matrix [C].
		assert_eq!((a.skeleton.parents.len(), a.sequences[0].frames.len(), c.mesh.positions.len()), (1, 1, 84));
		let id = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
		assert!(m[0].iter().zip(id).all(|(x, y)| (x - y).abs() < 1e-4), "{:?}", m[0]);
	}

	// Eudora class 186 (Glove of Doom bot), seq 0 frame 3 -> 4 at p = 1/6, no fade: retail's skin matrices S22 / S30
	// (columns, raw-unit translation) read from scratch 0x70000000 + j*0x40 at the return of the Eudora skin build
	// 0x2438a8 (interpreted e06_17 field 0, moby 0x1bd8f00) [C]. The same-sequence blend is unnormalised, so these leg
	// joints (frame-to-frame dot 0.07 / 0.44) shrink; the normalised blend is 0.3 off.
	#[test]
	fn eudora_doom_bot_same_seq_blend_unnormalised() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[4]).unwrap();
		let a = animation(&level, 186).unwrap().unwrap();
		let mut clock = AnimClock::new(&a.sequences[0], 0, 3);
		(clock.cur, clock.next, clock.old_seq, clock.new_seq, clock.p) = (3, 4, 0, 0, 1.0 / 6.0);
		assert!(clock.direct_blend());
		let m = a.pose_clock(&clock, None);
		let u = a.skeleton.unit;
		let retail: [(usize, [f32; 12]); 2] = [
			(22, [0.35086, -0.74263, -0.06451, -0.00712, 0.15316, -0.71372, 0.93759, 0.17134, 0.16590, -817.57257, 270.27353, 1063.25427]),
			(30, [0.66986, 0.06211, -0.51579, 0.04783, 1.02461, 0.13885, 0.55950, -0.14775, 0.60672, 441.55328, 98.64288, -9.95908]),
		];
		for (j, r) in retail {
			let s = &m[j];
			for k in 0..3 {
				for row in 0..3 {
					assert!((s[row * 4 + k] - r[k * 3 + row]).abs() < 2e-3, "S{j} col {k} row {row}: {} vs {}", s[row * 4 + k], r[k * 3 + row]);
				}
			}
			for row in 0..3 {
				assert!((s[row * 4 + 3] / u - r[9 + row]).abs() < 2.0, "S{j} t{row}: {} vs {}", s[row * 4 + 3] / u, r[9 + row]);
			}
		}
		// The flag: wrap to frame 0, a fade (old 0xff) or another old sequence clear it; a hold (next == cur) keeps it.
		for (cur, next, old, new, want) in [(4, 0, 0, 0, false), (3, 4, 0xff, 0, false), (3, 4, 1, 0, false), (3, 3, 0, 0, true), (2, 4, 0, 0, false)] {
			(clock.cur, clock.next, clock.old_seq, clock.new_seq) = (cur, next, old, new);
			assert_eq!(clock.direct_blend(), want, "{cur} {next} {old} {new}");
		}
		// Seq 7 (moby 0x1bdf200, 35-joint LOD record): scale records with flags 0 scale the joint's world matrix after the
		// chain (`post_scale_list`). F3 (7,0)->(7,1) p 0.5 direct: frame 1 re-appends 22, so 23 and 33 lose theirs, 26
		// (frame 1 only) blends from 1; F1 (7,3)->(7,0) p 0.5 normalised. Retail 0x2438a8 output, e06_17 [C].
		let check = |m: &[[f32; 12]], j: usize, r: [f32; 12]| {
			for k in 0..3 {
				for row in 0..3 {
					assert!((m[j][row * 4 + k] - r[k * 3 + row]).abs() < 2e-3, "S{j} col {k} row {row}: {} vs {}", m[j][row * 4 + k], r[k * 3 + row]);
				}
			}
			for row in 0..3 {
				assert!((m[j][row * 4 + 3] / u - r[9 + row]).abs() < 2.0, "S{j} t{row}: {} vs {}", m[j][row * 4 + 3] / u, r[9 + row]);
			}
		};
		let mut c7 = AnimClock::new(&a.sequences[7], 7, 0);
		(c7.cur, c7.next, c7.old_seq, c7.new_seq, c7.p) = (0, 1, 7, 7, 0.5);
		assert!(c7.direct_blend());
		let m = a.pose_clock(&c7, None);
		check(&m, 23, [0.60311, -0.00999, -0.66534, 0.03859, 0.99892, 0.01818, 0.72381, -0.03719, 0.60564, -1237.67493, 67.42755, -65.37485]);
		check(&m, 26, [0.56477, 0.03508, -0.66796, 0.01238, 0.99752, 0.04894, 0.73598, -0.03596, 0.57585, -1384.89209, 108.11121, -19.54456]);
		check(&m, 31, [0.75566, -0.00084, 0.38407, 0.00892, 0.99905, 0.00190, -0.44870, -0.01902, 0.87736, 737.49744, -9.77148, 118.93004]);
		check(&m, 33, [0.60408, 0.01443, -0.54783, 0.00892, 0.99905, 0.00190, 0.54734, -0.00632, 0.60197, 761.04291, -6.41254, -60.80092]);
		(c7.cur, c7.next) = (3, 0);
		assert!(!c7.direct_blend());
		let m = a.pose_clock(&c7, None);
		check(&m, 22, [0.45819, -0.05661, 0.87659, 0.07924, 0.99659, 0.02284, -1.14350, 0.07713, 0.60192, 1504.40845, 64.49805, 242.41187]);
		check(&m, 25, [0.69245, -0.00870, 0.72950, 0.05187, 0.99798, -0.03585, -0.72650, 0.06264, 0.68314, 1202.01172, 108.12280, -37.25961]);
	}

	// SCN-4.3c test 3: Novalis ship class 531 has 1 / 1 / 2 frames in sequences 0..2, and sequence 1's frame runs at speed 0.
	#[test]
	fn novalis_ship_flight_sequences() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let a = animation(&level, 531).unwrap().unwrap();
		let n: Vec<usize> = a.sequences.iter().take(3).map(|s| s.frames.len()).collect();
		assert_eq!(n, vec![1, 1, 2]);
		assert_eq!(a.sequences[1].frames[0].speed.to_bits(), 0);
	}

	#[test]
	fn ratchet() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let r = &classes[&0];
		// CLASS-tint test 2: class +0x40 and the 48-vertex tail packet.
		assert_eq!(r.glow, 0x808c8c8c);
		assert_eq!(r.colour_start, r.mesh.positions.len() - 48);
		let anim = animation(&level, 0).unwrap().unwrap();
		assert_eq!(anim.skeleton.parents.len(), 111);
		assert_eq!(anim.sequences.len(), 134);

		let (pos, nrm) = skin(r, &anim.pose_frame(0, 0));
		assert_eq!(pos.len(), nrm.len());
		let (lo, hi) = bbox(&pos);
		println!("seq 0 frame 0 bbox {lo:?} .. {hi:?}");
		let height = hi[2] - lo[2];
		assert!((height - 1.41).abs() < 0.05, "height {height}");
		assert!(lo[2].abs() < 0.06, "feet {}", lo[2]);

		for (seq, frac) in [(0, 0.5), (3, 0.5), (4, 0.3), (64, 0.5)] {
			let s = &anim.sequences[seq];
			let (pos, _) = skin(r, &anim.pose_after(seq, s.tick_len() * frac));
			let dist = distortion(r, &pos);
			println!("seq {seq} mid-pose edge distortion {dist:.4}");
			assert!(dist < 0.03, "seq {seq} distortion {dist}");
		}

		println!("seq frames speed  dur(s)  root(joint1 local) x-range y-range z-range  triggers");
		for (i, s) in anim.sequences.iter().enumerate() {
			if s.frames.is_empty() {
				continue;
			}
			let mut lo = [f32::MAX; 3];
			let mut hi = [f32::MIN; 3];
			for f in &s.frames {
				let t = f.translation.iter().find(|x| x.0 == 1).map_or(anim.skeleton.bind_translation[1], |x| x.1);
				for k in 0..3 {
					lo[k] = lo[k].min(t[k]);
					hi[k] = hi[k].max(t[k]);
				}
			}
			println!(
				"{i:3} {:3} {:.4} {:6.2}  x {:7.0}..{:7.0}  y {:7.0}..{:7.0}  z {:7.0}..{:7.0}  trig {}",
				s.frames.len(), s.speed, s.tick_len() / 50.0, lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], s.triggers.len()
			);
		}
	}

}

#[cfg(test)]
mod clock_tests {
	use super::*;

	fn frame(speed: f32) -> Frame {
		Frame { speed, number: 0.0, rotation: Vec::new(), scale: Vec::new(), post_scale: Vec::new(), translation: Vec::new() }
	}

	fn seq(n: usize, rate: f32, triggers: Vec<u32>) -> Sequence {
		Sequence { frames: (0..n).map(|_| frame(0.0)).collect(), speed: rate, sphere: [0.0; 4], triggers, loop_sound: 0xff }
	}

	// AUD-11d test 4: the cue gate is per tick, so a stale `cues_off` no longer silences the scan.
	#[test]
	fn cues_off_resets_each_tick() {
		let s = seq(4, 0.25, vec![(20 << 16) | 7]);
		let mut c = AnimClock::new(&s, 0, 1);
		c.cues_off = true;
		c.p = 0.0;
		c.r = 0.25;
		let mut got = None;
		for _ in 0..8 {
			got = got.or(c.step(&s).cue);
		}
		assert_eq!(got, Some(7));
	}

	// AUD-11d test 3: stepping `seek_cues` position to position fires the trigger exactly once, on the tick a free
	// `step` loop first returns it.
	#[test]
	fn snap_window_advances() {
		let s = seq(4, 0.5, vec![]);
		let mut c = AnimClock::new(&s, 1, 0);
		c.p = 0.9970;
		c.m = 1.0;
		c.r = 0.0;
		c.r = 0.0005; // 0.9970 + 0.0005 -> inside the snap window
		let out = c.step(&s);
		assert_eq!(out.flags & 1, 1);
		assert_eq!(c.cur, 1);
		assert!(c.p < 0.5);
		// outside the window: stored only
		let mut c = AnimClock::new(&s, 1, 0);
		c.p = 0.9;
		c.r = 0.0005;
		assert_eq!(c.step(&s).flags, 0);
		assert_eq!(c.cur, 0);
	}

	#[test]
	fn reverse_loop_wraps() {
		let s = seq(3, 0.25, vec![]);
		let mut c = AnimClock::new(&s, 1, 0);
		c.p = 0.1;
		c.m = -1.0;
		let out = c.step(&s);
		assert_eq!(out.flags & 2, 2);
		assert_eq!(c.cur, 2);
		assert!(c.p >= 0.0 && c.p < 1.0);
	}

	#[test]
	fn cue_scan_window() {
		// trigger id 7 at clock time 20 (frame 1, 4/16)
		let s = seq(4, 0.25, vec![(20 << 16) | 7]);
		assert_eq!(s.scan_cue(16, 20), Some(7));
		assert_eq!(s.scan_cue(20, 24), None);
		assert_eq!(s.scan_cue(24, 16), None);
		let mut c = AnimClock::new(&s, 1, 1);
		c.p = 0.0;
		c.r = 0.25;
		assert_eq!(c.step(&s).cue, Some(7)); // 16+0 -> 16+4 crosses 20
	}

	#[test]
	fn snapshot_threshold() {
		let s = seq(4, 0.25, vec![]);
		let mut t = SnapshotTable::default();
		let mut c = AnimClock::new(&s, 1, 0);
		c.p = 0.025;
		assert!(!c.blend(&mut t, 1, &s, 2, 0, 10, None));
		assert_eq!(c.old_seq, 1);
		c.p = 0.026;
		assert!(c.blend(&mut t, 1, &s, 2, 0, 10, None));
		assert_eq!(c.old_seq, 0xff);
		assert_eq!(c.prev_seq, 1);
		assert!((c.r - 0.1).abs() < 1e-6 && c.p == 0.0);
		// forced by flag 4 even at p == 0; same owner reuses its slot
		c.p = 0.0;
		assert!(c.blend(&mut t, 1, &s, 3, 0, 10, Some(4)));
		assert_eq!(c.slot, Some(0));
		// 16 slots exhaust
		for o in 2..17 { assert!(t.alloc(o).is_some()); }
		assert!(t.alloc(99).is_none());
	}

	#[test]
	fn prop_clock_advances_one_frame_per_tick_at_rate_one() {
		// No class data: the 30-frame, one-frame-a-tick stand-in, rate 1.
		let mut a = PropAnim::new(0);
		for i in 1..=10 {
			let out = a.step();
			assert_eq!(a.clock.cur, i, "frame after tick {i}");
			assert_eq!(out.flags & 1, 1);
			assert_eq!(a.age, i as u32);
		}
		// A real sequence at 0.5 frames a tick crosses a frame every second tick.
		let s = std::rc::Rc::new(vec![seq(8, 0.5, vec![])]);
		a.attach(Some(s));
		a.set(0, 0);
		let frames: Vec<i32> = (0..6).map(|_| { a.step(); a.clock.cur }).collect();
		assert_eq!(frames, vec![0, 1, 1, 2, 2, 3]);
		// Blend holds the new sequence on its start frame until the fade ends.
		a.step(); // mid-frame: p > 0.025, so the blend snapshots the pose
		a.blend(0, 2, 4);
		assert!(a.fading());
		for _ in 0..3 {
			a.step();
			assert!(a.fading() && a.clock.next == 2);
		}
		a.step();
		assert!(!a.fading());
	}

	#[test]
	fn held_weapon_switch_blend_lasts_ten_ticks() {
		let s = std::rc::Rc::new(vec![seq(20, 1.0, vec![]), seq(20, 1.0, vec![])]);
		let mut w = PropAnim::new(0);
		w.attach(Some(s));
		w.step();
		w.blend_ex(1, 0, 10, 5);
		// Flag 4 forces the snapshot even at progress 0: the new sequence holds frame 0 while the old pose fades out.
		assert!(w.fading());
		for tick in 1..=9 {
			w.step();
			assert!(w.fading() && w.clock.next == 0, "still fading after tick {tick}");
		}
		w.step();
		assert!(!w.fading(), "the fade ends on the tenth tick");
		assert_eq!(w.seq, 1);
	}

	#[test]
	fn vu0_quantise_truncates() {
		assert_eq!(vu0_quantise([1.65, -2.85, 8.95], 0.5), [1.5, -2.5, 8.5]);
		assert_eq!(vu0_quantise([20000.2, 0.0, 0.0], 0.5)[0], -12768.0);
	}

}
