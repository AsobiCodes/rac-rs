// PLAYFEED-BEGIN
// playfeed: feeds a recorded PCSX2 playthrough window (oracle/play_runs/<run>/windows/NNNN.raw + .pokes, staged as
// STATE/pad.raw + STATE/pad.pokes by oracle/pcsx2rec/play_wire.py) into an oracle run, field by field, exactly as
// scratch_pcsx2rec/winrun does (winrun.rs:73-122): the 18-byte pad report of field f goes straight into Cpu.pad (a
// field without a line repeats the previous report; before the first line the report is ff ff 7f 7f 7f 7f + 12 zero
// bytes), and the pokes of field f are written into RAM at the start of the field, before the pad.
// Enabled by the pad value MARK ("@play") in the 7-column vector row (FNDIFF_PAD / MOVE_PAD / COV_PAD). Shared source:
// fndiff (hand/examples), movetest and fnprof (game/examples) include this file with an ABSOLUTE #[path] (their example
// dirs are copied elsewhere by setup.py, latent_refresh, wire.py hands and cutfinish); the cutrt3 fndiff inlines the
// text between the PLAYFEED markers byte for byte (block identity is checked). `code_hash` of that text keys the
// fndiff checkpoint cache, so any edit here re-keys the play chains by itself; bump VER when the meaning of the files
// changes (VER is also in `digest`, so the play_wire ledgers and origin.txt digests change with it).
// Every stage refuses (exit 2) instead of running a silently different feed: FNDIFF_PAD_OR with @play, a missing or
// malformed pad.raw / pad.pokes, more run fields than recorded fields, a state whose INTC_STAT vblank bit is clear
// (winrun would raise a synthetic vblank there; no eudora1 state needs it).
// Each run that opens a feed prints `# playfeed VER DIGEST fields N pokes K code CODE` on stderr; the Python wrappers
// (integrate.fd_parse, latent_refresh, play_wire) accept a @play result only with that line and the digest of the
// state's files (an old exe given "@play" runs with no input at all and prints no such line).
// Trail probe: the env TRAIL (FNDIFF_PLAYTRAIL / MOVE_PLAYTRAIL / COV_PLAYTRAIL, per runner) = FILE appends
// `F HASH` (RAM 0x80000..0x1f00000, after the field's pokes and pad) every 50 fields: the cross-runner identity check.
use ee_oracle::cpu::Cpu;

pub const MARK: &str = "@play";
pub const VER: &str = "playfeed-1";
const RAM_LEN: u64 = 32 << 20;
const START: [u8; 18] = [0xff, 0xff, 0x7f, 0x7f, 0x7f, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

pub struct Feed {
	pads: Vec<[u8; 18]>,
	pokes: Vec<(u32, u32, usize, u64)>,
	trail: Option<String>,
	pub digest: u64,
}

fn fail(msg: &str) -> ! {
	eprintln!("playfeed: {msg} (refused, exit 2)");
	std::process::exit(2)
}

/// FNV-1a 64 over B, continuing from H (start: 0xcbf29ce484222325).
pub fn fnv(mut h: u64, b: &[u8]) -> u64 {
	for &x in b {
		h = (h ^ x as u64).wrapping_mul(0x0100_0000_01b3);
	}
	h
}

/// The feed's identity: FNV-1a 64 of VER, pad.raw, one 0xff byte, pad.pokes (empty when absent). None: no pad.raw.
pub fn digest(state: &str) -> Option<u64> {
	let raw = std::fs::read(format!("{state}/pad.raw")).ok()?;
	let pokes = std::fs::read(format!("{state}/pad.pokes")).unwrap_or_default();
	let mut h = fnv(0xcbf2_9ce4_8422_2325, VER.as_bytes());
	h = fnv(h, &raw);
	h = fnv(h, &[0xff]);
	Some(fnv(h, &pokes))
}

/// FNV-1a 64 of the text between the PLAYFEED markers of SRC (this file, or a file that inlines the block).
pub fn code_hash(src: &str) -> u64 {
	let (b, e) = (concat!("// PLAYFEED", "-BEGIN"), concat!("// PLAYFEED", "-END"));
	let i = src.find(b).expect("no PLAYFEED-BEGIN marker");
	let j = src[i..].find(e).expect("no PLAYFEED-END marker") + i;
	fnv(0xcbf2_9ce4_8422_2325, src[i..j + e.len()].as_bytes())
}

/// The feed for PAD == MARK (None for any other pad value). STATE is the state dir, FIELDS the run length, TRAIL the
/// name of this runner's trail variable, CODE its `code_hash` (printed on the header line).
pub fn open(pad: Option<&str>, pad_or: bool, state: &str, fields: u32, trail: &str, code: u64) -> Option<Feed> {
	if pad != Some(MARK) {
		return None;
	}
	if pad_or {
		fail("pad @play cannot be ORed onto the walk script (unset the *_PAD_OR variable)");
	}
	let rawp = format!("{state}/pad.raw");
	let text = std::fs::read_to_string(&rawp).unwrap_or_else(|e| fail(&format!("{rawp}: {e}")));
	let mut lines: Vec<(u32, [u8; 18])> = Vec::new();
	for (n, l) in text.lines().enumerate() {
		let bad = || -> ! { fail(&format!("{rawp} line {}: not FIELD<tab>36 hex digits: {l:?}", n + 1)) };
		let (f, h) = l.split_once('\t').unwrap_or_else(|| bad());
		let f: u32 = f.parse().unwrap_or_else(|_| bad());
		if h.len() != 36 || !h.bytes().all(|c| c.is_ascii_hexdigit()) || lines.last().is_some_and(|x| x.0 >= f) {
			bad();
		}
		let mut r = [0u8; 18];
		for (k, x) in r.iter_mut().enumerate() {
			*x = u8::from_str_radix(&h[2 * k..2 * k + 2], 16).unwrap_or_else(|_| bad());
		}
		lines.push((f, r));
	}
	let n = lines.last().map_or(0, |x| x.0 + 1);
	if fields > n {
		fail(&format!("{fields} fields asked, {rawp} records {n}"));
	}
	let mut pads = Vec::with_capacity(n as usize);
	let (mut last, mut it) = (START, lines.iter().peekable());
	for f in 0..n {
		if let Some((_, r)) = it.next_if(|x| x.0 == f) {
			last = *r;
		}
		pads.push(last);
	}
	let pokp = format!("{state}/pad.pokes");
	let mut pokes = Vec::new();
	if let Ok(t) = std::fs::read_to_string(&pokp) {
		for (k, l) in t.lines().enumerate() {
			let bad = || -> ! { fail(&format!("{pokp} line {}: not FIELD<tab>ADDR_HEX<tab>SIZE(1..8)<tab>VALUE_DEC: {l:?}", k + 1)) };
			let p: Vec<&str> = l.split('\t').collect();
			if p.len() != 4 {
				bad();
			}
			let f: u32 = p[0].parse().unwrap_or_else(|_| bad());
			let a = u32::from_str_radix(p[1], 16).unwrap_or_else(|_| bad());
			let s: usize = p[2].parse().unwrap_or_else(|_| bad());
			let v: u64 = p[3].parse().unwrap_or_else(|_| bad());
			if !(1..=8).contains(&s) || a as u64 + s as u64 > RAM_LEN {
				bad();
			}
			pokes.push((f, a, s, v));
		}
	} else if std::path::Path::new(&pokp).exists() {
		fail(&format!("{pokp}: unreadable"));
	}
	let hw = std::fs::read(format!("{state}/eeHwRegs.bin")).unwrap_or_else(|e| fail(&format!("{state}/eeHwRegs.bin: {e}")));
	if hw.get(0xf000).is_none_or(|b| b & 4 == 0) {
		fail(&format!("{state}: INTC_STAT vblank bit clear (winrun would raise a synthetic vblank; not supported)"));
	}
	let digest = digest(state).unwrap_or_else(|| fail(&format!("{rawp}: unreadable")));
	let trail = std::env::var(trail).ok().filter(|v| !v.is_empty());
	eprintln!("# playfeed {VER} {digest:016x} fields {n} pokes {} code {code:016x}", pokes.len());
	Some(Feed { pads, pokes, trail, digest })
}

impl Feed {
	/// Field F's input: its pokes into RAM (file order), then its pad report. Stateless per field (cache restores and
	/// FNDIFF_FROM skips stay exact).
	pub fn apply(&self, c: &mut Cpu, f: u32) {
		for &(pf, a, s, v) in &self.pokes {
			if pf == f {
				c.mem.ram[a as usize..a as usize + s].copy_from_slice(&v.to_le_bytes()[..s]);
			}
		}
		c.pad = Some(self.pads.get(f as usize).or(self.pads.last()).copied().unwrap_or(START));
		if let (Some(p), true) = (&self.trail, f % 50 == 0) {
			let r = &c.mem.ram[0x80000..0x1f00000.min(c.mem.ram.len())];
			let h = r.chunks_exact(8).fold(0xcbf2_9ce4_8422_2325u64, |h, w| {
				(h ^ u64::from_le_bytes(w.try_into().unwrap())).wrapping_mul(0x0100_0000_01b3).rotate_left(23)
			});
			use std::io::Write;
			let mut fh = std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap_or_else(|e| fail(&format!("trail {p}: {e}")));
			writeln!(fh, "{f} {h:016x}").unwrap_or_else(|e| fail(&format!("trail {p}: {e}")));
		}
	}
}
// PLAYFEED-END
