//! Pure-Rust MPEG-2 video (ISO/IEC 13818-2, Main Profile, 4:2:0) elementary stream decoder.
//!
//! Frame and field pictures, frame/field DCT, frame/field/16x8 motion, skipped macroblocks,
//! alternate scan, intra VLC format, q_scale_type, concealment vectors (parsed), quant matrices.
//! Dual-prime vectors are parsed but predicted as plain field vectors.
//! Frames come out in display order (I/P frames are held back until the next reference).

use std::collections::VecDeque;
use std::rc::Rc;

/// A decoded frame (both fields merged), cropped to the display size.
#[derive(Debug, Clone)]
pub struct Frame {
	pub width: u32,
	pub height: u32,
	pub y: Vec<u8>,
	pub cb: Vec<u8>,
	pub cr: Vec<u8>,
	pub pts: Option<u64>,
	/// 3 = coded as a frame picture; 1/2 = coded as two fields (first field's structure).
	pub picture_structure: u8,
	pub top_field_first: bool,
	pub progressive: bool,
	pub repeat_first_field: bool,
	pub aspect_ratio_information: u8,
	pub frame_rate_code: u8,
	pub temporal_reference: u16,
	pub coding_type: u8,
}

impl Frame {
	#[cfg(test)]
	pub fn chroma_width(&self) -> usize {
		(self.width as usize).div_ceil(2)
	}
}

// ---------------------------------------------------------------- bit reader

struct Br<'a> {
	d: &'a [u8],
	pos: usize,
}

impl<'a> Br<'a> {
	fn new(d: &'a [u8]) -> Br<'a> {
		Br { d, pos: 0 }
	}
	#[inline]
	fn peek(&self, n: u32) -> u32 {
		if n == 0 {
			return 0;
		}
		let byte = self.pos >> 3;
		let sh = (self.pos & 7) as u32;
		let v = if byte + 8 <= self.d.len() {
			u64::from_be_bytes(self.d[byte..byte + 8].try_into().unwrap())
		} else {
			let mut a = [0u8; 8];
			for (i, x) in a.iter_mut().enumerate() {
				*x = *self.d.get(byte + i).unwrap_or(&0);
			}
			u64::from_be_bytes(a)
		};
		((v << sh) >> (64 - n)) as u32
	}
	#[inline]
	fn skip(&mut self, n: u32) {
		self.pos += n as usize;
	}
	#[inline]
	fn read(&mut self, n: u32) -> u32 {
		let v = self.peek(n);
		self.pos += n as usize;
		v
	}
	fn eof(&self) -> bool {
		self.pos >= self.d.len() * 8
	}
}

// ---------------------------------------------------------------- tables

struct Vlc {
	bits: u32,
	t: Vec<(u8, i16, i16)>,
}

impl Vlc {
	fn new(bits: u32, ents: &[(u16, u8, i16, i16)]) -> Vlc {
		let mut t = vec![(0u8, 0i16, 0i16); 1 << bits];
		for &(code, len, a, b) in ents {
			let sh = bits - len as u32;
			for k in 0..(1u32 << sh) {
				t[((code as u32) << sh | k) as usize] = (len, a, b);
			}
		}
		Vlc { bits, t }
	}
	#[inline]
	fn get(&self, br: &Br) -> (u8, i16, i16) {
		self.t[br.peek(self.bits) as usize]
	}
}

const EOB: i16 = -1;
const ESC: i16 = -2;

struct Tabs {
	mba: Vlc,
	mb_i: Vlc,
	mb_p: Vlc,
	mb_b: Vlc,
	cbp: Vlc,
	mv: Vlc,
	dcl: Vlc,
	dcc: Vlc,
	b14f: Vlc,
	b14n: Vlc,
	b15: Vlc,
	cos: [[f64; 8]; 8],
	zz: [usize; 64],
	alt: [usize; 64],
}

const ZZ: [usize; 64] = [
	0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49,
	56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
// ALT_INV[row*8+col] = scan index (spec figure 7-3 as [v][u]).
const ALT_INV: [u8; 64] = [
	0, 4, 6, 20, 22, 36, 38, 52, 1, 5, 7, 21, 23, 37, 39, 53, 2, 8, 19, 24, 34, 40, 50, 54, 3, 9, 18, 25, 35, 41, 51, 55, 10, 17, 26, 30,
	42, 46, 56, 60, 11, 16, 27, 31, 43, 47, 57, 61, 12, 15, 28, 32, 44, 48, 58, 62, 13, 14, 29, 33, 45, 49, 59, 63,
];
const DEFAULT_INTRA: [u8; 64] = [
	8, 16, 19, 22, 26, 27, 29, 34, 16, 16, 22, 24, 27, 29, 34, 37, 19, 22, 26, 27, 29, 34, 34, 38, 22, 22, 26, 27, 29, 34, 37, 40, 22, 26,
	27, 29, 32, 35, 40, 48, 26, 27, 29, 32, 35, 40, 48, 58, 26, 27, 29, 34, 38, 46, 56, 69, 27, 29, 35, 38, 46, 56, 69, 83,
];
const NONLIN_Q: [i32; 32] =
	[0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 72, 80, 88, 96, 104, 112];

fn flags(list: &[(u16, u8, i16)]) -> Vec<(u16, u8, i16, i16)> {
	list.iter().map(|&(c, l, f)| (c, l, f, 0)).collect()
}

impl Tabs {
	fn new() -> Tabs {
		let mut mba = MBA.to_vec();
		mba.push((0b00000001000, 11, -1, 0));
		let mb_i = flags(&[(1, 1, 16), (0b01, 2, 17)]);
		let mb_p = flags(&[(1, 1, 10), (0b01, 2, 8), (0b001, 3, 2), (0b00011, 5, 16), (0b00010, 5, 11), (0b00001, 5, 9), (0b000001, 6, 17)]);
		let mb_b = flags(&[
			(0b10, 2, 6),
			(0b11, 2, 14),
			(0b010, 3, 4),
			(0b011, 3, 12),
			(0b0010, 4, 2),
			(0b0011, 4, 10),
			(0b00011, 5, 16),
			(0b00010, 5, 15),
			(0b000011, 6, 11),
			(0b000010, 6, 13),
			(0b000001, 6, 17),
		]);
		let esc = (0b000001u16, 6u8, ESC, 0i16);
		// B14 "first" variant: 1-bit '1s' form, no '11s', no EOB.
		let mut f: Vec<_> = B14.iter().copied().filter(|e| !(e.1 == 2 && e.0 == 3 && e.2 == 0 && e.3 == 1)).collect();
		f.push(esc);
		let mut n: Vec<_> = B14.iter().copied().filter(|e| !(e.1 == 1 && e.0 == 1)).collect();
		n.push(esc);
		n.push((0b10, 2, EOB, 0));
		let mut b15 = B15.to_vec();
		b15.push(esc);
		b15.push((0b0110, 4, EOB, 0));
		let mut cos = [[0.0; 8]; 8];
		for (u, row) in cos.iter_mut().enumerate() {
			for (x, c) in row.iter_mut().enumerate() {
				let cu = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
				*c = cu / 2.0 * (((2 * x + 1) * u) as f64 * std::f64::consts::PI / 16.0).cos();
			}
		}
		let mut alt = [0usize; 64];
		for (r, &n) in ALT_INV.iter().enumerate() {
			alt[n as usize] = r;
		}
		Tabs {
			mba: Vlc::new(11, &mba),
			mb_i: Vlc::new(2, &mb_i),
			mb_p: Vlc::new(6, &mb_p),
			mb_b: Vlc::new(6, &mb_b),
			cbp: Vlc::new(9, CBP),
			mv: Vlc::new(11, MV),
			dcl: Vlc::new(9, DCL),
			dcc: Vlc::new(10, DCC),
			b14f: Vlc::new(16, &f),
			b14n: Vlc::new(16, &n),
			b15: Vlc::new(16, &b15),
			cos,
			zz: ZZ,
			alt,
		}
	}

	/// Inverse DCT (double precision reference), in place; output clamped to -256..=255.
	fn idct(&self, f: &mut [i32; 64]) {
		let mut tmp = [0f64; 64];
		for v in 0..8 {
			let row = &f[v * 8..v * 8 + 8];
			if row.iter().all(|&c| c == 0) {
				continue;
			}
			for x in 0..8 {
				let mut s = 0.0;
				for u in 0..8 {
					s += row[u] as f64 * self.cos[u][x];
				}
				tmp[v * 8 + x] = s;
			}
		}
		for y in 0..8 {
			for x in 0..8 {
				let mut s = 0.0;
				for v in 0..8 {
					s += tmp[v * 8 + x] * self.cos[v][y];
				}
				f[y * 8 + x] = (s.round() as i32).clamp(-256, 255);
			}
		}
	}
}

// ---------------------------------------------------------------- state

#[derive(Clone)]
struct Seq {
	width: usize,
	height: usize,
	mbw: usize,
	mbh: usize,
	aspect: u8,
	frame_rate: u8,
	progressive: bool,
	intra_q: [u8; 64],
	nonintra_q: [u8; 64],
}

#[derive(Clone, Default)]
struct Pic {
	coding_type: u8,
	tref: u16,
	f_code: [[u32; 2]; 2],
	dc_prec: u32,
	structure: u8,
	tff: bool,
	fpfd: bool,
	conceal: bool,
	qst: bool,
	ivf: bool,
	alt: bool,
	rff: bool,
	prog_frame: bool,
	pts: Option<u64>,
}

struct FrameBuf {
	p: [Vec<u8>; 3],
	stride: [usize; 3],
	rows: [usize; 3],
	pts: Option<u64>,
	tff: bool,
	prog: bool,
	rff: bool,
	structure: u8,
	tref: u16,
	coding_type: u8,
}

impl FrameBuf {
	fn new(mbw: usize, mbh: usize) -> FrameBuf {
		let (w, h) = (mbw * 16, mbh * 16);
		FrameBuf {
			p: [vec![0; w * h], vec![128; w * h / 4], vec![128; w * h / 4]],
			stride: [w, w / 2, w / 2],
			rows: [h, h / 2, h / 2],
			pts: None,
			tff: false,
			prog: true,
			rff: false,
			structure: 3,
			tref: 0,
			coding_type: 0,
		}
	}
}

struct CurPic {
	buf: Box<FrameBuf>,
	coding_type: u8,
	structure: u8,
	fields: u8,
	skip: bool,
}

pub struct Decoder {
	tabs: Tabs,
	buf: Vec<u8>,
	base: usize,
	pts_q: VecDeque<(usize, u64)>,
	seq: Option<Seq>,
	pic: Option<Pic>,
	in_pic: bool,
	started: bool,
	cur: Option<CurPic>,
	half: Option<CurPic>,
	ref_a: Option<Rc<FrameBuf>>,
	ref_b: Option<Rc<FrameBuf>>,
	out: VecDeque<Frame>,
	second_field: bool,
	pub errors: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
	Frame,
	Field,
	Part16x8,
}

struct SliceDec<'a> {
	seq: &'a Seq,
	pic: &'a Pic,
	t: &'a Tabs,
	cur: &'a mut FrameBuf,
	fwd: Option<&'a FrameBuf>,
	bwd: Option<&'a FrameBuf>,
	second_p: bool,
	mb_rows: usize,
	qcode: u32,
	dc_pred: [i32; 3],
	pmv: [[[i32; 2]; 2]; 2],
	mv: [[[i32; 2]; 2]; 2],
	fsel: [[usize; 2]; 2],
	kind: Kind,
	last_fwd: bool,
	last_bwd: bool,
	err: bool,
}

fn find_sc(b: &[u8], mut i: usize) -> Option<usize> {
	while i + 3 <= b.len() {
		if b[i + 2] > 1 {
			i += 3;
		} else if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
			return Some(i);
		} else {
			i += 1;
		}
	}
	None
}

impl Default for Decoder {
	fn default() -> Self {
		Self::new()
	}
}

impl Decoder {
	pub fn new() -> Decoder {
		Decoder {
			tabs: Tabs::new(),
			buf: Vec::new(),
			base: 0,
			pts_q: VecDeque::new(),
			seq: None,
			pic: None,
			in_pic: false,
			started: false,
			cur: None,
			half: None,
			ref_a: None,
			ref_b: None,
			out: VecDeque::new(),
			second_field: false,
			errors: 0,
		}
	}

	/// Feed elementary-stream bytes; `pts` applies to the first picture starting in this chunk.
	pub fn push(&mut self, es: &[u8], pts: Option<u64>) {
		if let Some(p) = pts {
			self.pts_q.push_back((self.base + self.buf.len(), p));
		}
		self.buf.extend_from_slice(es);
		self.process(false);
	}

	/// End of stream: decode the trailing unit and release held frames.
	pub fn flush(&mut self) {
		self.process(true);
		self.end_picture();
		if let Some(h) = self.half.take() {
			self.finish(h);
		}
		if let Some(b) = self.ref_b.take() {
			self.out.push_back(self.crop(&b));
		}
		self.ref_a = None;
	}

	pub fn next_frame(&mut self) -> Option<Frame> {
		self.out.pop_front()
	}

	fn process(&mut self, flush: bool) {
		let buf = std::mem::take(&mut self.buf);
		let mut pos = 0;
		loop {
			let Some(s) = find_sc(&buf, pos) else {
				pos = buf.len().saturating_sub(3).max(pos);
				break;
			};
			let e = match find_sc(&buf, s + 4) {
				Some(e) => e,
				None if flush && s + 4 <= buf.len() => buf.len(),
				None => {
					pos = s;
					break;
				}
			};
			let code = buf[s + 3];
			let abs = self.base + s;
			self.unit(code, &buf[s + 4..e], abs);
			pos = e;
			if e == buf.len() {
				break;
			}
		}
		self.base += pos;
		self.buf = buf[pos..].to_vec();
	}

	fn unit(&mut self, code: u8, d: &[u8], abs: usize) {
		match code {
			0x00 => {
				self.end_picture();
				self.picture_header(d, abs);
			}
			0xb3 => {
				self.end_picture();
				self.sequence_header(d);
			}
			0xb8 | 0xb7 => self.end_picture(),
			0xb5 => self.extension(d),
			0x01..=0xaf => self.slice(code, d),
			_ => {}
		}
	}

	fn sequence_header(&mut self, d: &[u8]) {
		if d.len() < 8 {
			return;
		}
		let mut br = Br::new(d);
		let width = br.read(12) as usize;
		let height = br.read(12) as usize;
		let aspect = br.read(4) as u8;
		let frame_rate = br.read(4) as u8;
		br.skip(18 + 1 + 10 + 1);
		let mut intra_q = DEFAULT_INTRA;
		let mut nonintra_q = [16u8; 64];
		if br.read(1) == 1 {
			for i in 0..64 {
				intra_q[ZZ[i]] = br.read(8) as u8;
			}
		}
		if br.read(1) == 1 {
			for i in 0..64 {
				nonintra_q[ZZ[i]] = br.read(8) as u8;
			}
		}
		if width == 0 || height == 0 {
			return;
		}
		let (mbw, mbh) = (width.div_ceil(16), height.div_ceil(16));
		let changed = self.seq.as_ref().is_none_or(|s| s.width != width || s.height != height);
		if changed {
			self.ref_a = None;
			self.ref_b = None;
		}
		self.seq = Some(Seq { width, height, mbw, mbh, aspect, frame_rate, progressive: true, intra_q, nonintra_q });
	}

	fn extension(&mut self, d: &[u8]) {
		if d.is_empty() {
			return;
		}
		let mut br = Br::new(d);
		let id = br.read(4);
		match id {
			1 => {
				let Some(seq) = self.seq.as_mut() else { return };
				br.skip(8);
				seq.progressive = br.read(1) == 1;
				let chroma = br.read(2);
				let hx = br.read(2) as usize;
				let vx = br.read(2) as usize;
				if chroma != 1 {
					self.errors += 1;
				}
				if hx != 0 || vx != 0 {
					seq.width |= hx << 12;
					seq.height |= vx << 12;
					seq.mbw = seq.width.div_ceil(16);
					seq.mbh = seq.height.div_ceil(16);
				}
			}
			3 => {
				let Some(seq) = self.seq.as_mut() else { return };
				if br.read(1) == 1 {
					for i in 0..64 {
						seq.intra_q[ZZ[i]] = br.read(8) as u8;
					}
				}
				if br.read(1) == 1 {
					for i in 0..64 {
						seq.nonintra_q[ZZ[i]] = br.read(8) as u8;
					}
				}
			}
			8 => {
				let Some(pic) = self.pic.as_mut() else { return };
				for s in 0..2 {
					for t in 0..2 {
						pic.f_code[s][t] = br.read(4);
					}
				}
				pic.dc_prec = br.read(2);
				pic.structure = br.read(2) as u8;
				pic.tff = br.read(1) == 1;
				pic.fpfd = br.read(1) == 1;
				pic.conceal = br.read(1) == 1;
				pic.qst = br.read(1) == 1;
				pic.ivf = br.read(1) == 1;
				pic.alt = br.read(1) == 1;
				pic.rff = br.read(1) == 1;
				br.skip(1);
				pic.prog_frame = br.read(1) == 1;
			}
			_ => {}
		}
	}

	fn picture_header(&mut self, d: &[u8], abs: usize) {
		if d.len() < 4 || self.seq.is_none() {
			return;
		}
		let mut br = Br::new(d);
		let tref = br.read(10) as u16;
		let ct = br.read(3) as u8;
		br.skip(16);
		let mut pic = Pic { coding_type: ct, tref, structure: 3, prog_frame: true, ..Default::default() };
		if ct == 2 || ct == 3 {
			br.skip(1);
			let f = br.read(3);
			pic.f_code[0] = [f, f];
		}
		if ct == 3 {
			br.skip(1);
			let f = br.read(3);
			pic.f_code[1] = [f, f];
		}
		let mut pts = None;
		while let Some(&(off, p)) = self.pts_q.front() {
			if off <= abs {
				pts = Some(p);
				self.pts_q.pop_front();
			} else {
				break;
			}
		}
		pic.pts = pts;
		self.pic = Some(pic);
		self.in_pic = true;
		self.started = false;
	}

	fn begin_picture(&mut self) -> bool {
		let (Some(dims), Some(pic)) = (self.seq.as_ref().map(|s| (s.mbw, s.mbh)), self.pic.clone()) else { return false };
		let (mbw, mbh) = dims;
		let ct = pic.coding_type;
		if !(1..=3).contains(&ct) {
			return false;
		}
		let mut second = false;
		let mut cur = None;
		if let Some(h) = self.half.take() {
			if pic.structure != 3 && h.structure != 3 && pic.structure != h.structure {
				cur = Some(h);
				second = true;
			} else {
				self.finish(h);
			}
		}
		let cur = cur.unwrap_or_else(|| {
			let mut b = Box::new(FrameBuf::new(mbw, mbh));
			b.pts = pic.pts;
			b.tff = pic.tff;
			b.prog = pic.prog_frame;
			b.rff = pic.rff;
			b.structure = pic.structure;
			b.tref = pic.tref;
			b.coding_type = ct;
			let missing = match ct {
				2 => self.ref_b.is_none(),
				3 => self.ref_a.is_none() || self.ref_b.is_none(),
				_ => false,
			};
			CurPic { buf: b, coding_type: ct, structure: pic.structure, fields: 0, skip: missing }
		});
		self.second_field = second;
		self.cur = Some(cur);
		true
	}

	fn slice(&mut self, code: u8, d: &[u8]) {
		if !self.in_pic {
			return;
		}
		if !self.started {
			self.started = true;
			if !self.begin_picture() {
				self.in_pic = false;
				return;
			}
		}
		let (Some(seq), Some(pic), Some(cur)) = (self.seq.as_ref(), self.pic.as_ref(), self.cur.as_mut()) else { return };
		if cur.skip {
			return;
		}
		let (fwd, bwd) = match pic.coding_type {
			2 => (self.ref_b.as_deref(), None),
			3 => (self.ref_a.as_deref(), self.ref_b.as_deref()),
			_ => (None, None),
		};
		let mb_rows = if pic.structure == 3 { seq.mbh } else { seq.mbh / 2 };
		let row = code as usize - 1;
		if row >= mb_rows {
			return;
		}
		let mut br = Br::new(d);
		let qcode = br.read(5);
		if br.peek(1) == 1 {
			br.skip(1 + 1 + 1 + 6);
			while br.peek(1) == 1 && !br.eof() {
				br.skip(9);
			}
		}
		br.skip(1);
		let mut sd = SliceDec {
			seq,
			pic,
			t: &self.tabs,
			cur: &mut cur.buf,
			fwd,
			bwd,
			second_p: self.second_field && pic.coding_type == 2,
			mb_rows,
			qcode,
			dc_pred: [0; 3],
			pmv: [[[0; 2]; 2]; 2],
			mv: [[[0; 2]; 2]; 2],
			fsel: [[0; 2]; 2],
			kind: Kind::Frame,
			last_fwd: true,
			last_bwd: false,
			err: false,
		};
		sd.run(&mut br, row);
		if sd.err {
			self.errors += 1;
		}
	}

	fn end_picture(&mut self) {
		self.in_pic = false;
		self.started = false;
		if let Some(mut c) = self.cur.take() {
			if c.structure == 3 {
				self.finish(c);
			} else {
				c.fields += 1;
				if c.fields >= 2 {
					self.finish(c);
				} else {
					self.half = Some(c);
				}
			}
		}
	}

	fn finish(&mut self, c: CurPic) {
		if c.skip {
			return;
		}
		if c.coding_type == 3 {
			let f = self.crop(&c.buf);
			self.out.push_back(f);
		} else {
			let rc: Rc<FrameBuf> = Rc::new(*c.buf);
			let old = self.ref_b.replace(rc);
			if let Some(o) = &old {
				let f = self.crop(o);
				self.out.push_back(f);
			}
			self.ref_a = old;
		}
	}

	fn crop(&self, b: &FrameBuf) -> Frame {
		let seq = self.seq.as_ref().unwrap();
		let (w, h) = (seq.width, seq.height);
		let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
		let mut pl = [Vec::with_capacity(w * h), Vec::with_capacity(cw * ch), Vec::with_capacity(cw * ch)];
		for (i, (pw, ph)) in [(w, h), (cw, ch), (cw, ch)].into_iter().enumerate() {
			for r in 0..ph {
				pl[i].extend_from_slice(&b.p[i][r * b.stride[i]..r * b.stride[i] + pw]);
			}
		}
		let [y, cb, cr] = pl;
		Frame {
			width: w as u32,
			height: h as u32,
			y,
			cb,
			cr,
			pts: b.pts,
			picture_structure: b.structure,
			top_field_first: b.tff,
			progressive: b.prog,
			repeat_first_field: b.rff,
			aspect_ratio_information: seq.aspect,
			frame_rate_code: seq.frame_rate,
			temporal_reference: b.tref,
			coding_type: b.coding_type,
		}
	}
}

// ---------------------------------------------------------------- slice / macroblock decode

fn fetch(src: &FrameBuf, plane: usize, x: i32, y: i32, hx: i32, hy: i32, w: usize, h: usize, vstep: usize, voff: usize, out: &mut [u8], os: usize) {
	let pl = &src.p[plane];
	let stride = src.stride[plane];
	let pw = stride as i32;
	let fh = (src.rows[plane] / vstep) as i32;
	let s = |xx: i32, yy: i32| -> i32 {
		let xx = xx.clamp(0, pw - 1);
		let yy = yy.clamp(0, fh - 1);
		pl[(voff + yy as usize * vstep) * stride + xx as usize] as i32
	};
	for j in 0..h {
		let yy = y + j as i32;
		for i in 0..w {
			let xx = x + i as i32;
			let v = match (hx, hy) {
				(0, 0) => s(xx, yy),
				(1, 0) => (s(xx, yy) + s(xx + 1, yy) + 1) >> 1,
				(0, _) => (s(xx, yy) + s(xx, yy + 1) + 1) >> 1,
				_ => (s(xx, yy) + s(xx + 1, yy) + s(xx, yy + 1) + s(xx + 1, yy + 1) + 2) >> 2,
			};
			out[j * os + i] = v as u8;
		}
	}
}

impl SliceDec<'_> {
	fn frame_pic(&self) -> bool {
		self.pic.structure == 3
	}
	fn parity(&self) -> usize {
		(self.pic.structure == 2) as usize
	}
	fn qscale(&self) -> i32 {
		if self.pic.qst { NONLIN_Q[self.qcode as usize] } else { self.qcode as i32 * 2 }
	}
	fn reset_dc(&mut self) {
		self.dc_pred = [1 << (7 + self.pic.dc_prec); 3];
	}

	fn run(&mut self, br: &mut Br, row: usize) {
		self.reset_dc();
		let mbw = self.seq.mbw;
		let mut addr = (row * mbw) as isize - 1;
		let mut first = true;
		loop {
			let mut inc = 0;
			loop {
				let (len, a, _) = self.t.mba.get(br);
				if len == 0 {
					self.err = true;
					return;
				}
				br.skip(len as u32);
				if a == -1 {
					inc += 33;
				} else {
					inc += a as isize;
					break;
				}
			}
			if first {
				addr += inc;
			} else {
				for _ in 1..inc {
					addr += 1;
					if addr as usize >= mbw * self.mb_rows {
						self.err = true;
						return;
					}
					self.skipped(addr as usize);
				}
				addr += 1;
			}
			first = false;
			if addr as usize >= mbw * self.mb_rows {
				self.err = true;
				return;
			}
			self.macroblock(br, addr as usize);
			if self.err {
				return;
			}
			if br.eof() || br.peek(23) == 0 {
				return;
			}
		}
	}

	fn zero_mv_pred(&mut self) {
		self.mv = [[[0; 2]; 2]; 2];
		let p = self.parity();
		self.fsel = [[p; 2]; 2];
		self.kind = if self.frame_pic() { Kind::Frame } else { Kind::Field };
	}

	fn skipped(&mut self, addr: usize) {
		self.reset_dc();
		let (mbx, mby) = (addr % self.seq.mbw, addr / self.seq.mbw);
		if self.pic.coding_type == 2 {
			self.pmv = [[[0; 2]; 2]; 2];
			self.zero_mv_pred();
			self.predict(mbx, mby, true, false);
		} else {
			let (f, b) = (self.last_fwd, self.last_bwd);
			self.predict(mbx, mby, f, b);
		}
	}

	fn mv_comp(&mut self, br: &mut Br, s: usize, t: usize, pred: i32) -> i32 {
		let f_code = self.pic.f_code[s][t];
		if f_code == 0 || f_code > 14 {
			self.err = true;
			return 0;
		}
		let (len, code, _) = self.t.mv.get(br);
		if len == 0 {
			self.err = true;
			return 0;
		}
		br.skip(len as u32);
		let rsize = f_code - 1;
		let code = code as i32;
		let delta = if code == 0 {
			0
		} else {
			let res = br.read(rsize) as i32;
			let d = ((code.abs() - 1) << rsize) + res + 1;
			if code < 0 { -d } else { d }
		};
		let f = 1i32 << rsize;
		let (low, high, range) = (-16 * f, 16 * f - 1, 32 * f);
		let mut v = pred + delta;
		if v < low {
			v += range;
		} else if v > high {
			v -= range;
		}
		v
	}

	fn motion_vectors(&mut self, br: &mut Br, s: usize, dual: bool) {
		let frame = self.frame_pic();
		if dual {
			let mx = self.mv_comp(br, s, 0, self.pmv[0][s][0]);
			br.skip(if br.peek(1) == 0 { 1 } else { 2 });
			let my = self.mv_comp(br, s, 1, self.pmv[0][s][1]);
			br.skip(if br.peek(1) == 0 { 1 } else { 2 });
			self.pmv[0][s] = [mx, my];
			self.pmv[1][s] = [mx, my];
			self.mv[0][s] = [mx, my];
			self.mv[1][s] = [mx, my];
			if frame {
				self.fsel[0][s] = 0;
				self.fsel[1][s] = 1;
			} else {
				self.fsel[0][s] = self.parity();
				self.fsel[1][s] = self.parity();
			}
			return;
		}
		match (frame, self.kind) {
			(true, Kind::Frame) => {
				let mx = self.mv_comp(br, s, 0, self.pmv[0][s][0]);
				let my = self.mv_comp(br, s, 1, self.pmv[0][s][1]);
				self.pmv[0][s] = [mx, my];
				self.pmv[1][s] = [mx, my];
				self.mv[0][s] = [mx, my];
			}
			(true, _) => {
				for r in 0..2 {
					self.fsel[r][s] = br.read(1) as usize;
					let mx = self.mv_comp(br, s, 0, self.pmv[r][s][0]);
					let my = self.mv_comp(br, s, 1, self.pmv[r][s][1] >> 1);
					self.mv[r][s] = [mx, my];
					self.pmv[r][s] = [mx, 2 * my];
				}
			}
			(false, Kind::Part16x8) => {
				for r in 0..2 {
					self.fsel[r][s] = br.read(1) as usize;
					let mx = self.mv_comp(br, s, 0, self.pmv[r][s][0]);
					let my = self.mv_comp(br, s, 1, self.pmv[r][s][1]);
					self.mv[r][s] = [mx, my];
					self.pmv[r][s] = [mx, my];
				}
			}
			(false, _) => {
				self.fsel[0][s] = br.read(1) as usize;
				let mx = self.mv_comp(br, s, 0, self.pmv[0][s][0]);
				let my = self.mv_comp(br, s, 1, self.pmv[0][s][1]);
				self.pmv[0][s] = [mx, my];
				self.pmv[1][s] = [mx, my];
				self.mv[0][s] = [mx, my];
				self.mv[1][s] = [mx, my];
				self.fsel[1][s] = self.fsel[0][s];
			}
		}
	}

	fn macroblock(&mut self, br: &mut Br, addr: usize) {
		let (mbx, mby) = (addr % self.seq.mbw, addr / self.seq.mbw);
		let tab = match self.pic.coding_type {
			1 => &self.t.mb_i,
			2 => &self.t.mb_p,
			_ => &self.t.mb_b,
		};
		let (len, f, _) = tab.get(br);
		if len == 0 {
			self.err = true;
			return;
		}
		br.skip(len as u32);
		let f = f as u32;
		let (quant, mut fwd, bwd, pat, intra) = (f & 1 != 0, f & 2 != 0, f & 4 != 0, f & 8 != 0, f & 16 != 0);
		let frame = self.frame_pic();
		let mut dual = false;
		self.kind = if frame { Kind::Frame } else { Kind::Field };
		if fwd || bwd {
			if frame && self.pic.fpfd {
				self.kind = Kind::Frame;
			} else {
				let c = br.read(2);
				match (frame, c) {
					(true, 1) => self.kind = Kind::Field,
					(true, 2) => self.kind = Kind::Frame,
					(false, 1) => self.kind = Kind::Field,
					(false, 2) => self.kind = Kind::Part16x8,
					(_, 3) => {
						dual = true;
						self.kind = if frame { Kind::Frame } else { Kind::Field };
					}
					_ => {
						self.err = true;
						return;
					}
				}
			}
		}
		let mut dct_field = false;
		if frame && !self.pic.fpfd && (intra || pat) {
			dct_field = br.read(1) == 1;
		}
		if quant {
			self.qcode = br.read(5);
			if self.qcode == 0 {
				self.err = true;
				return;
			}
		}
		if fwd || (intra && self.pic.conceal) {
			self.motion_vectors(br, 0, dual);
		}
		if bwd {
			self.motion_vectors(br, 1, dual);
		}
		if intra && self.pic.conceal {
			br.skip(1);
		}
		if self.err {
			return;
		}
		if intra {
			if !self.pic.conceal {
				self.pmv = [[[0; 2]; 2]; 2];
			}
		} else {
			self.reset_dc();
			if !fwd && !bwd {
				// P macroblock without motion: zero vector.
				self.pmv = [[[0; 2]; 2]; 2];
				self.zero_mv_pred();
				fwd = true;
			}
			self.last_fwd = fwd;
			self.last_bwd = bwd;
		}
		let cbp = if pat {
			let (len, c, _) = self.t.cbp.get(br);
			if len == 0 {
				self.err = true;
				return;
			}
			br.skip(len as u32);
			c as u32
		} else if intra {
			63
		} else {
			0
		};
		if intra {
			self.reset_dc_if_needed();
		} else {
			self.predict(mbx, mby, fwd, bwd);
		}
		let mut coefs = [0i32; 64];
		for b in 0..6 {
			if intra || cbp & (32 >> b) != 0 {
				if !self.block(br, b, intra, &mut coefs) {
					self.err = true;
					return;
				}
				self.t.idct(&mut coefs);
				self.put_block(mbx, mby, b, intra, dct_field, &coefs);
			}
		}
	}

	fn reset_dc_if_needed(&mut self) {}

	fn block(&mut self, br: &mut Br, b: usize, intra: bool, out: &mut [i32; 64]) -> bool {
		out.fill(0);
		let q = self.qscale();
		let scan = if self.pic.alt { &self.t.alt } else { &self.t.zz };
		let wm = if intra { &self.seq.intra_q } else { &self.seq.nonintra_q };
		let mut i;
		let mut sum = 0i32;
		let vlc;
		if intra {
			let (tab, cc) = if b < 4 { (&self.t.dcl, 0) } else { (&self.t.dcc, b - 3) };
			let (len, size, _) = tab.get(br);
			if len == 0 {
				return false;
			}
			br.skip(len as u32);
			let size = size as u32;
			let mut diff = 0;
			if size > 0 {
				diff = br.read(size) as i32;
				if diff < (1 << (size - 1)) {
					diff += (-1 << size) + 1;
				}
			}
			self.dc_pred[cc] += diff;
			out[0] = self.dc_pred[cc] << (3 - self.pic.dc_prec);
			sum += out[0];
			i = 1;
			vlc = if self.pic.ivf { &self.t.b15 } else { &self.t.b14n };
		} else {
			i = 0;
			vlc = &self.t.b14f;
		}
		let mut vlc = vlc;
		let next = if intra && self.pic.ivf { &self.t.b15 } else { &self.t.b14n };
		loop {
			let (len, a, lv) = vlc.get(br);
			if len == 0 {
				return false;
			}
			br.skip(len as u32);
			if a == EOB {
				break;
			}
			let (run, level);
			if a == ESC {
				run = br.read(6) as usize;
				let mut l = br.read(12) as i32;
				if l >= 2048 {
					l -= 4096;
				}
				level = l;
			} else {
				run = a as usize;
				level = if br.read(1) == 1 { -(lv as i32) } else { lv as i32 };
			}
			i += run;
			if i > 63 {
				return false;
			}
			let pos = scan[i];
			let mag = level.abs();
			let v = if intra { (mag * wm[pos] as i32 * q) >> 4 } else { ((2 * mag + 1) * wm[pos] as i32 * q) >> 5 };
			let v = if level < 0 { -v } else { v };
			let v = v.clamp(-2048, 2047);
			out[pos] = v;
			sum += v;
			i += 1;
			vlc = next;
			if i > 63 {
				// A block may fill all 64 coefficients; EOB follows.
				let (len, a, _) = vlc.get(br);
				if len != 0 && a == EOB {
					br.skip(len as u32);
				}
				break;
			}
			if br.eof() {
				return false;
			}
		}
		if sum & 1 == 0 {
			out[63] ^= 1;
		}
		true
	}

	fn put_block(&mut self, mbx: usize, mby: usize, b: usize, intra: bool, dct_field: bool, c: &[i32; 64]) {
		let (row_mul, row_off) = if self.frame_pic() { (1, 0) } else { (2, self.parity()) };
		let (plane, x0, r0, rstep) = if b < 4 {
			let bx = (b & 1) * 8;
			if dct_field { (0, mbx * 16 + bx, mby * 16 + (b >> 1), 2) } else { (0, mbx * 16 + bx, mby * 16 + (b >> 1) * 8, 1) }
		} else {
			(b - 3, mbx * 8, mby * 8, 1)
		};
		let stride = self.cur.stride[plane];
		let pl = &mut self.cur.p[plane];
		for k in 0..8 {
			let line = (r0 + k * rstep) * row_mul + row_off;
			let base = line * stride + x0;
			for i in 0..8 {
				let v = c[k * 8 + i];
				let d = &mut pl[base + i];
				*d = if intra { v.clamp(0, 255) as u8 } else { (*d as i32 + v).clamp(0, 255) as u8 };
			}
		}
	}

	fn pick_src(&self, s: usize, fs: usize) -> &FrameBuf {
		if s == 0 && self.second_p && fs != self.parity() {
			return &*self.cur;
		}
		let r = if s == 0 { self.fwd } else { self.bwd };
		r.unwrap_or(&*self.cur)
	}

	fn predict(&mut self, mbx: usize, mby: usize, fwd: bool, bwd: bool) {
		let mut pb = [[0u8; 384]; 2];
		let mut n = 0;
		for s in 0..2 {
			if (s == 0 && !fwd) || (s == 1 && !bwd) {
				continue;
			}
			let mut buf = [0u8; 384];
			self.predict_dir(s, mbx, mby, &mut buf);
			pb[n] = buf;
			n += 1;
		}
		if n == 0 {
			return;
		}
		let (row_mul, row_off) = if self.frame_pic() { (1, 0) } else { (2, self.parity()) };
		for plane in 0..3 {
			let (bw, bh, off) = if plane == 0 { (16, 16, 0) } else { (8, 8, 256 + (plane - 1) * 64) };
			let stride = self.cur.stride[plane];
			let pl = &mut self.cur.p[plane];
			for j in 0..bh {
				let line = (mby * bh + j) * row_mul + row_off;
				let base = line * stride + mbx * bw;
				for i in 0..bw {
					let a = pb[0][off + j * bw + i] as u32;
					pl[base + i] = if n == 2 { ((a + pb[1][off + j * bw + i] as u32 + 1) >> 1) as u8 } else { a as u8 };
				}
			}
		}
	}

	fn predict_dir(&self, s: usize, mbx: usize, mby: usize, out: &mut [u8; 384]) {
		let (mbx, mby) = (mbx as i32, mby as i32);
		let (ly, rest) = out.split_at_mut(256);
		let (cb, cr) = rest.split_at_mut(64);
		let frame = self.frame_pic();
		// (vector index, field select, dest first line, lines, vstep, y origin luma, y origin chroma, dest line stride mult)
		struct Part {
			r: usize,
			vstep: usize,
			y0: i32,
			cy0: i32,
			lines: usize,
			dst: usize,
			dmul: usize,
		}
		let parts: Vec<Part> = match (frame, self.kind) {
			(true, Kind::Frame) => vec![Part { r: 0, vstep: 1, y0: mby * 16, cy0: mby * 8, lines: 16, dst: 0, dmul: 1 }],
			(true, _) => (0..2).map(|r| Part { r, vstep: 2, y0: mby * 8, cy0: mby * 4, lines: 8, dst: r, dmul: 2 }).collect(),
			(false, Kind::Part16x8) => {
				(0..2).map(|r| Part { r, vstep: 2, y0: mby * 16 + r as i32 * 8, cy0: mby * 8 + r as i32 * 4, lines: 8, dst: r * 8, dmul: 1 }).collect()
			}
			(false, _) => vec![Part { r: 0, vstep: 2, y0: mby * 16, cy0: mby * 8, lines: 16, dst: 0, dmul: 1 }],
		};
		for p in parts {
			let v = self.mv[p.r][s];
			let fs = if frame && self.kind == Kind::Frame { 0 } else { self.fsel[p.r][s] };
			let src = self.pick_src(s, fs);
			let (vstep, voff) = if p.vstep == 1 { (1, 0) } else { (2, fs) };
			let (mx, my) = (v[0], v[1]);
			let x = mbx * 16 + (mx >> 1);
			let y = p.y0 + (my >> 1);
			let o = p.dst * 16;
			fetch(src, 0, x, y, mx & 1, my & 1, 16, p.lines, vstep, voff, &mut ly[o..], 16 * p.dmul);
			let (cmx, cmy) = (mx / 2, my / 2);
			let cx = mbx * 8 + (cmx >> 1);
			let cyy = p.cy0 + (cmy >> 1);
			let co = if p.dmul == 2 { p.dst * 8 } else { (p.dst / 2) * 8 };
			let cl = p.lines / 2;
			fetch(src, 1, cx, cyy, cmx & 1, cmy & 1, 8, cl, vstep, voff, &mut cb[co..], 8 * p.dmul);
			fetch(src, 2, cx, cyy, cmx & 1, cmy & 1, 8, cl, vstep, voff, &mut cr[co..], 8 * p.dmul);
		}
	}
}

// ---------------------------------------------------------------- VLC tables (ISO/IEC 13818-2 Annex B): (code, bits, a, b)
const B14: &[(u16, u8, i16, i16)] = &[(1,1,0,1),(3,2,0,1),(3,3,1,1),(4,4,0,2),(5,4,2,1),(5,5,0,3),(7,5,3,1),(6,5,4,1),(6,6,1,2),(7,6,5,1),(5,6,6,1),(4,6,7,1),(6,7,0,4),(4,7,2,2),(7,7,8,1),(5,7,9,1),(38,8,0,5),(33,8,0,6),(37,8,1,3),(36,8,3,2),(39,8,10,1),(35,8,11,1),(34,8,12,1),(32,8,13,1),(10,10,0,7),(12,10,1,4),(11,10,2,3),(15,10,4,2),(9,10,5,2),(14,10,14,1),(13,10,15,1),(8,10,16,1),(29,12,0,8),(24,12,0,9),(19,12,0,10),(16,12,0,11),(27,12,1,5),(20,12,2,4),(28,12,3,3),(18,12,4,3),(30,12,6,2),(21,12,7,2),(17,12,8,2),(31,12,17,1),(26,12,18,1),(25,12,19,1),(23,12,20,1),(22,12,21,1),(26,13,0,12),(25,13,0,13),(24,13,0,14),(23,13,0,15),(22,13,1,6),(21,13,1,7),(20,13,2,5),(19,13,3,4),(18,13,5,3),(17,13,9,2),(16,13,10,2),(31,13,22,1),(30,13,23,1),(29,13,24,1),(28,13,25,1),(27,13,26,1),(31,14,0,16),(30,14,0,17),(29,14,0,18),(28,14,0,19),(27,14,0,20),(26,14,0,21),(25,14,0,22),(24,14,0,23),(23,14,0,24),(22,14,0,25),(21,14,0,26),(20,14,0,27),(19,14,0,28),(18,14,0,29),(17,14,0,30),(16,14,0,31),(24,15,0,32),(23,15,0,33),(22,15,0,34),(21,15,0,35),(20,15,0,36),(19,15,0,37),(18,15,0,38),(17,15,0,39),(16,15,0,40),(31,15,1,8),(30,15,1,9),(29,15,1,10),(28,15,1,11),(27,15,1,12),(26,15,1,13),(25,15,1,14),(19,16,1,15),(18,16,1,16),(17,16,1,17),(16,16,1,18),(20,16,6,3),(26,16,11,2),(25,16,12,2),(24,16,13,2),(23,16,14,2),(22,16,15,2),(21,16,16,2),(31,16,27,1),(30,16,28,1),(29,16,29,1),(28,16,30,1),(27,16,31,1)];
const B15: &[(u16, u8, i16, i16)] = &[(2,2,0,1),(2,3,1,1),(6,3,0,2),(5,5,2,1),(7,4,0,3),(7,5,3,1),(6,6,4,1),(6,5,1,2),(7,6,5,1),(6,7,6,1),(4,7,7,1),(28,5,0,4),(7,7,2,2),(5,7,8,1),(120,7,9,1),(29,5,0,5),(5,6,0,6),(121,7,1,3),(38,8,3,2),(122,7,10,1),(33,8,11,1),(37,8,12,1),(36,8,13,1),(4,6,0,7),(39,8,1,4),(252,8,2,3),(253,8,4,2),(4,9,5,2),(5,9,14,1),(7,9,15,1),(13,10,16,1),(123,7,0,8),(124,7,0,9),(35,8,0,10),(34,8,0,11),(32,8,1,5),(12,10,2,4),(28,12,3,3),(18,12,4,3),(30,12,6,2),(21,12,7,2),(17,12,8,2),(31,12,17,1),(26,12,18,1),(25,12,19,1),(23,12,20,1),(22,12,21,1),(250,8,0,12),(251,8,0,13),(254,8,0,14),(255,8,0,15),(22,13,1,6),(21,13,1,7),(20,13,2,5),(19,13,3,4),(18,13,5,3),(17,13,9,2),(16,13,10,2),(31,13,22,1),(30,13,23,1),(29,13,24,1),(28,13,25,1),(27,13,26,1),(31,14,0,16),(30,14,0,17),(29,14,0,18),(28,14,0,19),(27,14,0,20),(26,14,0,21),(25,14,0,22),(24,14,0,23),(23,14,0,24),(22,14,0,25),(21,14,0,26),(20,14,0,27),(19,14,0,28),(18,14,0,29),(17,14,0,30),(16,14,0,31),(24,15,0,32),(23,15,0,33),(22,15,0,34),(21,15,0,35),(20,15,0,36),(19,15,0,37),(18,15,0,38),(17,15,0,39),(16,15,0,40),(31,15,1,8),(30,15,1,9),(29,15,1,10),(28,15,1,11),(27,15,1,12),(26,15,1,13),(25,15,1,14),(19,16,1,15),(18,16,1,16),(17,16,1,17),(16,16,1,18),(20,16,6,3),(26,16,11,2),(25,16,12,2),(24,16,13,2),(23,16,14,2),(22,16,15,2),(21,16,16,2),(31,16,27,1),(30,16,28,1),(29,16,29,1),(28,16,30,1),(27,16,31,1)];
const CBP: &[(u16, u8, i16, i16)] = &[(7,3,60,0),(13,4,4,0),(12,4,8,0),(11,4,16,0),(10,4,32,0),(19,5,12,0),(18,5,48,0),(17,5,20,0),(16,5,40,0),(15,5,28,0),(14,5,44,0),(13,5,52,0),(12,5,56,0),(11,5,1,0),(10,5,61,0),(9,5,2,0),(8,5,62,0),(15,6,24,0),(14,6,36,0),(13,6,3,0),(12,6,63,0),(23,7,5,0),(22,7,9,0),(21,7,17,0),(20,7,33,0),(19,7,6,0),(18,7,10,0),(17,7,18,0),(16,7,34,0),(31,8,7,0),(30,8,11,0),(29,8,19,0),(28,8,35,0),(27,8,13,0),(26,8,49,0),(25,8,21,0),(24,8,41,0),(23,8,14,0),(22,8,50,0),(21,8,22,0),(20,8,42,0),(19,8,15,0),(18,8,51,0),(17,8,23,0),(16,8,43,0),(15,8,25,0),(14,8,37,0),(13,8,26,0),(12,8,38,0),(11,8,29,0),(10,8,45,0),(9,8,53,0),(8,8,57,0),(7,8,30,0),(6,8,46,0),(5,8,54,0),(4,8,58,0),(7,9,31,0),(6,9,47,0),(5,9,55,0),(4,9,59,0),(3,9,27,0),(2,9,39,0),(1,9,0,0)];
const MBA: &[(u16, u8, i16, i16)] = &[(1,1,1,0),(3,3,2,0),(2,3,3,0),(3,4,4,0),(2,4,5,0),(3,5,6,0),(2,5,7,0),(7,7,8,0),(6,7,9,0),(11,8,10,0),(10,8,11,0),(9,8,12,0),(8,8,13,0),(7,8,14,0),(6,8,15,0),(23,10,16,0),(22,10,17,0),(21,10,18,0),(20,10,19,0),(19,10,20,0),(18,10,21,0),(35,11,22,0),(34,11,23,0),(33,11,24,0),(32,11,25,0),(31,11,26,0),(30,11,27,0),(29,11,28,0),(28,11,29,0),(27,11,30,0),(26,11,31,0),(25,11,32,0),(24,11,33,0)];
const MV: &[(u16, u8, i16, i16)] = &[(25,11,-16,0),(27,11,-15,0),(29,11,-14,0),(31,11,-13,0),(33,11,-12,0),(35,11,-11,0),(19,10,-10,0),(21,10,-9,0),(23,10,-8,0),(7,8,-7,0),(9,8,-6,0),(11,8,-5,0),(7,7,-4,0),(3,5,-3,0),(3,4,-2,0),(3,3,-1,0),(1,1,0,0),(2,3,1,0),(2,4,2,0),(2,5,3,0),(6,7,4,0),(10,8,5,0),(8,8,6,0),(6,8,7,0),(22,10,8,0),(20,10,9,0),(18,10,10,0),(34,11,11,0),(32,11,12,0),(30,11,13,0),(28,11,14,0),(26,11,15,0),(24,11,16,0)];
const DCL: &[(u16, u8, i16, i16)] = &[(4,3,0,0),(0,2,1,0),(1,2,2,0),(5,3,3,0),(6,3,4,0),(14,4,5,0),(30,5,6,0),(62,6,7,0),(126,7,8,0),(254,8,9,0),(510,9,10,0),(511,9,11,0)];
const DCC: &[(u16, u8, i16, i16)] = &[(0,2,0,0),(1,2,1,0),(2,2,2,0),(6,3,3,0),(14,4,4,0),(30,5,5,0),(62,6,6,0),(126,7,7,0),(254,8,8,0),(510,9,9,0),(1022,10,10,0),(1023,10,11,0)];

#[cfg(test)]
mod tests {
	use super::*;
	use crate::iso::Disc;
	use std::path::PathBuf;

	/// Video PES payloads (stream 0xE0) of a program stream, with PTS.
	fn video_es(ps: &[u8]) -> Vec<(Vec<u8>, Option<u64>)> {
		let mut out = Vec::new();
		let mut i = 0;
		while let Some(p) = find_sc(ps, i) {
			let code = ps[p + 3];
			i = p + 4;
			match code {
				0xba => i = p + 14 + (ps[p + 13] & 7) as usize,
				0xb9 => break,
				0xbb | 0xbc | 0xbd | 0xbe | 0xbf | 0xc0..=0xdf | 0xe0..=0xef => {
					let len = ((ps[p + 4] as usize) << 8) | ps[p + 5] as usize;
					if code == 0xe0 {
						let flags = ps[p + 7];
						let hl = ps[p + 8] as usize;
						let pts = if flags & 0x80 != 0 {
							let b = &ps[p + 9..p + 14];
							Some((((b[0] >> 1) & 7) as u64) << 30 | (b[1] as u64) << 22 | ((b[2] >> 1) as u64) << 15 | (b[3] as u64) << 7 | (b[4] >> 1) as u64)
						} else {
							None
						};
						out.push((ps[p + 9 + hl..p + 6 + len].to_vec(), pts));
					}
					i = p + 6 + len;
				}
				_ => {}
			}
		}
		out
	}

	fn to_png(f: &Frame) -> Vec<u8> {
		let (w, h) = (f.width as usize, f.height as usize);
		let cw = f.chroma_width();
		let mut rgb = Vec::with_capacity(w * h * 3);
		for y in 0..h {
			for x in 0..w {
				let yy = (f.y[y * w + x] as f32 - 16.0) * 1.164;
				let cb = f.cb[(y / 2) * cw + x / 2] as f32 - 128.0;
				let cr = f.cr[(y / 2) * cw + x / 2] as f32 - 128.0;
				rgb.push((yy + 1.596 * cr).clamp(0.0, 255.0) as u8);
				rgb.push((yy - 0.392 * cb - 0.813 * cr).clamp(0.0, 255.0) as u8);
				rgb.push((yy + 2.017 * cb).clamp(0.0, 255.0) as u8);
			}
		}
		crate::render::png(&crate::render::Image { w, h, rgb })
	}

	#[test]
	fn decode_movie_one() {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = Disc::open(&path).unwrap();
		let m = crate::scene::movie(&mut disc, 0x4001).unwrap();
		let bytes = disc.read_bytes(m.lba as u64, (m.bytes as usize).min(12 << 20)).unwrap();
		let pes = video_es(&bytes);
		let mut dec = Decoder::new();
		let mut frames = Vec::new();
		let t0 = std::time::Instant::now();
		'o: for (es, pts) in &pes {
			dec.push(es, *pts);
			while let Some(f) = dec.next_frame() {
				frames.push(f);
				if frames.len() >= 100 {
					break 'o;
				}
			}
		}
		let dt = t0.elapsed().as_secs_f64();
		println!("{} frames in {dt:.2}s = {:.1} fps, errors {}", frames.len(), frames.len() as f64 / dt, dec.errors);
		assert!(frames.len() >= 100);
		let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test_out");
		std::fs::create_dir_all(dir).unwrap();
		for n in [0, 12, 50, 99] {
			let f = &frames[n];
			println!("frame {n}: {}x{} type {} struct {} tff {} prog {} pts {:?}", f.width, f.height, f.coding_type, f.picture_structure, f.top_field_first, f.progressive, f.pts);
			std::fs::write(format!("{dir}/mpeg_{n}.png"), to_png(f)).unwrap();
		}
	}

	/// REN-8: the horizontal banding of the first picture of 22.pss is in the bitstream (period 4 rows, locked to the 8-row
	/// block grid), so retail shows it too; this pins the decoder to that pattern.
	#[test]
	fn source_banding_profile() {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = Disc::open(&path).unwrap();
		let m = crate::scene::movie(&mut disc, 0x4001).unwrap();
		let bytes = disc.read_bytes(m.lba as u64, (m.bytes as usize).min(2 << 20)).unwrap();
		let mut dec = Decoder::new();
		let mut first = None;
		for (es, pts) in &video_es(&bytes) {
			dec.push(es, *pts);
			if let Some(f) = dec.next_frame() {
				first = Some(f);
				break;
			}
		}
		let f = first.expect("a decoded picture");
		let w = f.width as usize;
		assert_eq!((f.width, f.height), (512, 416));
		let r: Vec<f64> = (0..f.height as usize).map(|y| f.y[y * w..(y + 1) * w].iter().map(|&v| v as f64).sum::<f64>() / w as f64).collect();
		let (mut sum, mut n) = ([0.0f64; 8], [0u32; 8]);
		for y in 8..408 {
			let mean = r[y - 4..=y + 4].iter().sum::<f64>() / 9.0;
			sum[y % 8] += r[y] - mean;
			n[y % 8] += 1;
		}
		let bin: Vec<f64> = (0..8).map(|k| sum[k] / n[k] as f64).collect();
		for k in 0..8 {
			if matches!(k, 0 | 1 | 4 | 5) {
				assert!(bin[k] < -0.5, "bin {k} = {}", bin[k]);
			} else {
				assert!(bin[k] > 0.5, "bin {k} = {}", bin[k]);
			}
		}
	}
}
