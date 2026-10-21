//! GS capture (opt-in): the GIF packets the game sends over VIF1 DIRECT and
//! GIF DMA (paths 2 and 3) drive a GS register model, local memory (VRAM,
//! loaded from the savestate's GS.bin) and a software rasterizer for 2D work.
//! The 3D world goes through VU1 XGKICK (path 1) and is not captured: rac-rs
//! draws it. Prims aimed at the display buffers land in an overlay that rac-rs
//! composites over its own frame; everything else (render-to-texture panels)
//! is drawn into VRAM so later prims can sample it.

mod draw;

#[derive(Clone, Copy, Default, Debug)]
pub struct Vtx {
    pub x: u16,
    pub y: u16,
    pub z: u32,
    pub rgba: [u8; 4],
    pub q: f32,
    pub s: f32,
    pub t: f32,
    pub u: u16,
    pub v: u16,
    pub f: u8,
}

#[derive(Default)]
struct Tag {
    nloop: u32,
    flg: u32,
    nreg: u32,
    regs: u64,
    j: u32,
}

/// [PC] (speed) Recycled VIF1 word buffers: the consumer of a `Job::Vif1` hands its word Vec back with `recycle_words`, and
/// `Mem::vif1_kick` takes one with `take_words` instead of allocating ~3 MB (and page-faulting it in) every kick. At most 4 are kept;
/// a consumer that never recycles only costs the old allocation. Contents are never read across kicks (len is reset to 0).
static WORDS_POOL: std::sync::Mutex<Vec<Vec<u32>>> = std::sync::Mutex::new(Vec::new());

/// An empty word Vec with room for about `want` words (the biggest recycled buffer, else a fresh one).
pub fn take_words(want: usize) -> Vec<u32> {
    let got = WORDS_POOL.lock().ok().and_then(|mut p| {
        let i = (0..p.len()).max_by_key(|&i| p[i].capacity())?;
        Some(p.swap_remove(i))
    });
    match got {
        Some(mut v) => {
            v.clear();
            v.reserve(want);
            v
        }
        None => Vec::with_capacity(want),
    }
}

/// Gives a finished VIF1 word buffer back (small ones are just dropped).
pub fn recycle_words(mut v: Vec<u32>) {
    if v.capacity() < 1 << 16 {
        return;
    }
    v.clear();
    if let Ok(mut p) = WORDS_POOL.lock() {
        if p.len() < 4 {
            p.push(v);
        }
    }
}

/// A packet for the capture: a VIF1 stream (words, block marks) or GIF path 3 qwords, with the DISPFB then.
pub enum Job {
    Vif1(Vec<u32>, Vec<(usize, u32)>, u64),
    Gif(Vec<u128>, u64),
}

/// A host -> local transfer waiting to be written: its BITBLTBUF, TRXPOS and TRXREG, block range and data.
struct Pend {
    bb: u64,
    pos: u64,
    rr: u64,
    lo: u32,
    hi: u32,
    data: Vec<u128>,
}

#[derive(Default, Clone, Copy)]
struct Trx {
    x: u32,
    y: u32,
}

/// The overlay: premultiplied colour plus how much of the scene shows through (final = rgb + a * scene).
pub struct Overlay {
    pub w: u32,
    pub h: u32,
    /// Rows of the frame (the main buffer's scissor height); the rest is unused.
    pub rows: u32,
    /// The game restored a grabbed frame into the main buffer (the pause menu): the scene is frozen.
    pub frozen: bool,
    pub px: Vec<[f32; 4]>,
}

pub struct Gs {
    pub vram: Vec<u8>,
    // VIF1 stream state
    vif_skip: u32,
    vif_direct: u32,
    vif_cl: u32,
    vif_wl: u32,
    vif_bad: bool,
    vif_unpack: bool,
    vif_phase: u32,
    vif_hist: Vec<(usize, u32)>,
    /// Debug: (word offset, source address, qwords, tag id) per block of the current VIF1 kick.
    pub blocks: Vec<(usize, u32, u32, u32)>,
    qw: [u32; 4],
    qn: usize,
    // GIF
    tag: Tag,
    pub r: [u64; 0x80],
    rgbaq: ([u8; 4], f32),
    st: (f32, f32),
    packed_q: f32,
    uv: (u16, u16),
    fog: u8,
    vq: Vec<Vtx>,
    trx: Trx,
    /// Frame buffer pages (FBP) the display reads from (DISPFB1/2).
    pub disp_fbp: Vec<u32>,
    pub cur: Overlay,
    /// The last finished overlay (published when the game starts drawing into the other display buffer).
    pub done: Option<Overlay>,
    pend: Vec<Pend>,
    deferring: bool,
    main_fbp: Option<u32>,
    last_copy: bool,
    /// Main buffer width (64-pixel units, from the copy's TBW) and its end block (rows the frame scissors).
    main_bw: u32,
    main_end: u32,
    /// VU1 microprogram starts (MSCAL/MSCALF/MSCNT) since the last overlay was published.
    mscal: u32,
    /// DISPFB of the displayed circuit (set by the DMA kick).
    pub dispfb: u64,
    /// Overlays published so far.
    pub frames: u64,
    clut: Vec<[f32; 4]>,
    clut_off: usize,
    cbp0: u32,
    cbp1: u32,
    pub stats: std::collections::BTreeMap<String, u64>,
    pub log: bool,
    /// Overlay prims whose blend subtracts the source (ALPHA with B = Cs, A != Cs, e.g. 0x42 Cd - Cs*As) are left out of
    /// the overlay: its premultiplied colour clamps at 0 when published, so a subtract over the overlay's own colour
    /// would be lost there and then applied a second time by a viewer that draws the filter itself (rac-rs
    /// `queue_b::screen_quads`, the Visibomb screen filter FUN_001ec660, last 2D draw of the frame) [C: weapon_fx.md
    /// stage 12]. Off by default (the overlay as before).
    pub ov_skip_sub: bool,
}

/// VU1 programs a frame starts before its scene: the sky's own (0 on Eudora, 2 on Rilgar); the scene starts ~2700.
const SKY_PROGRAMS: u32 = 64;

const W: u32 = 512;
const H: u32 = 512;

impl Gs {
    pub fn new(vram: Vec<u8>) -> Gs {
        Gs {
            vram,
            vif_skip: 0,
            vif_direct: 0,
            vif_cl: 1,
            vif_wl: 1,
            vif_bad: false,
            vif_unpack: false,
            vif_phase: 0,
            vif_hist: Vec::new(),
            blocks: Vec::new(),
            qw: [0; 4],
            qn: 0,
            tag: Tag::default(),
            r: [0; 0x80],
            rgbaq: ([0x80; 4], 1.0),
            st: (0.0, 0.0),
            packed_q: 1.0,
            uv: (0, 0),
            fog: 0,
            vq: Vec::new(),
            trx: Trx::default(),
            disp_fbp: Vec::new(),
            cur: Overlay { w: W, h: H, rows: H, frozen: false, px: vec![[0.0, 0.0, 0.0, 1.0]; (W * H) as usize] },
            done: None,
            main_fbp: None,
            last_copy: false,
            pend: Vec::new(),
            deferring: false,
            main_bw: 8,
            main_end: 0,
            mscal: 0,
            dispfb: 0,
            frames: 0,
            clut: vec![[0.0; 4]; 512],
            clut_off: 0,
            cbp0: 0,
            cbp1: 0,
            stats: Default::default(),
            log: std::env::var("ORACLE_GSLOG").is_ok(),
            ov_skip_sub: false,
        }
    }

    fn stat(&mut self, k: String) {
        *self.stats.entry(k).or_insert(0) += 1;
    }

    /// Loads local memory from a PCSX2 GS.bin (freeze version 9: 425-byte header, 4 MB, 84-byte tail).
    pub fn load_vram(path: &str) -> Option<Vec<u8>> {
        let d = std::fs::read(path).ok()?;
        (d.len() == 425 + (4 << 20) + 84).then(|| d[425..425 + (4 << 20)].to_vec())
    }

    /// VIF1 words (DMA order, TTE words included). `marks` gives (word index, phase in its qword): a
    /// DMA block starts at phase 0, TTE words at phase 2. Only DIRECT/DIRECTHL data reaches the GIF.
    pub fn run(&mut self, job: Job) {
        match job {
            Job::Vif1(w, marks, d) => {
                self.dispfb = d;
                self.vif1_reset();
                self.vif1(&w, &marks);
                self.blocks.clear();
                recycle_words(w);
            }
            Job::Gif(qs, d) => {
                self.dispfb = d;
                for q in qs {
                    self.gif(q);
                }
            }
        }
    }

    pub fn vif1(&mut self, w: &[u32], marks: &[(usize, u32)]) {
        if let Ok(a) = std::env::var("ORACLE_VIFDUMP") {
            if let Ok(a) = usize::from_str_radix(&a, 16) {
                if a + 64 <= w.len() && self.stats.get("dumped").is_none() {
                    self.stat("dumped".into());
                    eprintln!("dump {a:x}: {:08x?}", &w[a..a + 64]);
                }
            }
        }
        for (k, &(at, ph)) in marks.iter().enumerate() {
            let end = marks.get(k + 1).map(|m| m.0).unwrap_or(w.len()).min(w.len());
            self.vif_phase = ph;
            let mut i = at;
            while i < end {
                // UNPACK data in bulk (the last word goes through vif_word for its pad check).
                if self.vif_skip > 1 && self.vif_direct == 0 {
                    let k = ((self.vif_skip - 1) as usize).min(end - i);
                    self.vif_skip -= k as u32;
                    self.vif_phase = (self.vif_phase + k as u32) & 3;
                    i += k;
                    continue;
                }
                self.vif_word(w[i], i, end - i - 1);
                self.vif_phase = (self.vif_phase + 1) & 3;
                i += 1;
            }
        }
    }

    /// `left`: words after this one in its DMA block.
    fn vif_word(&mut self, word: u32, i: usize, left: usize) {
        if self.vif_direct > 0 {
            self.qw[self.qn] = word;
            self.qn += 1;
            if self.qn == 4 {
                self.qn = 0;
                let q = self.qw;
                self.gif(q[0] as u128 | (q[1] as u128) << 32 | (q[2] as u128) << 64 | (q[3] as u128) << 96);
            }
            self.vif_direct -= 1;
            return;
        }
        if self.vif_skip > 0 {
            self.vif_skip -= 1;
            // A DMA block whose UNPACK data ends inside its last qword: the pad words are not VIF codes
            // (Eudora vertex packets leave stale words there, some with the i-bit set that would stall
            // VIF1 for good; the game never restarts it) [I].
            if self.vif_skip == 0 && self.vif_unpack && left < 4 && left == (3 - self.vif_phase) as usize {
                self.vif_skip = left as u32;
            }
            if self.vif_skip == 0 {
                self.vif_unpack = false;
            }
            return;
        }
        if self.vif_bad {
            return;
        }
        let code = word;
        if self.log {
            self.vif_hist.push((i * 4 + self.vif_phase as usize, code));
            if self.vif_hist.len() > 60 {
                self.vif_hist.remove(0);
            }
        }
        let cmd = (code >> 24) & 0x7f;
        let imm = code & 0xffff;
        let num = (code >> 16) & 0xff;
        match cmd {
            0x00 | 0x02..=0x07 | 0x10 | 0x11 | 0x13 => {}
            0x14 | 0x15 | 0x17 => self.mscal += 1,
            0x01 => {
                self.vif_cl = imm & 0xff;
                self.vif_wl = (imm >> 8) & 0xff;
            }
            0x20 => self.vif_skip = 1,
            0x30 | 0x31 => self.vif_skip = 4,
            0x4a => self.vif_skip = if num == 0 { 512 } else { num * 2 },
            0x50 | 0x51 => {
                self.vif_direct = if imm == 0 { 65536 * 4 } else { imm * 4 };
                self.qn = 0;
            }
            0x60..=0x7f => {
                let vn = (cmd >> 2) & 3;
                let vl = cmd & 3;
                let n = if num == 0 { 256 } else { num };
                let (cl, wl) = (self.vif_cl, self.vif_wl);
                let items = if wl <= cl || wl == 0 { n } else { (n / wl) * cl + (n % wl).min(cl) };
                let bits = if vl == 3 { 16 } else { (vn + 1) * (32 >> vl) };
                self.vif_skip = (items * bits).div_ceil(32);
                self.vif_unpack = self.vif_skip > 0;
            }
            _ => {
                if self.log {
                    eprintln!("vif1 bad code {code:08x} at word {i} after {:08x?} cl {} wl {}", self.vif_hist, self.vif_cl, self.vif_wl);
                    eprintln!("blocks {:x?}", self.blocks);
                }
                self.stat(format!("vif1 bad code {code:08x}"));
                self.vif_bad = true;
            }
        }
    }

    /// Resyncs the VIF1 stream at a chain start (a stuck parse never outlives one kick).
    pub fn vif1_reset(&mut self) {
        self.vif_hist.clear();
        if self.vif_bad {
            self.vif_bad = false;
            self.vif_skip = 0;
            self.vif_direct = 0;
        }
    }

    /// One GIF qword (paths 2/3).
    pub fn gif(&mut self, q: u128) {
        if self.tag.nloop == 0 {
            let nloop = (q & 0x7fff) as u32;
            let pre = (q >> 46) & 1 != 0;
            let prim = ((q >> 47) & 0x7ff) as u64;
            let flg = ((q >> 58) & 3) as u32;
            let nreg = ((q >> 60) & 0xf) as u32;
            self.tag = Tag { nloop, flg, nreg: if nreg == 0 { 16 } else { nreg }, regs: (q >> 64) as u64, j: 0 };
            if pre && flg == 0 {
                self.reg(0, prim);
            }
            return;
        }
        match self.tag.flg {
            0 => {
                let d = (self.tag.regs >> (4 * self.tag.j)) & 0xf;
                self.packed(d as u8, q);
                self.next_reg();
            }
            1 => {
                for half in [q as u64, (q >> 64) as u64] {
                    if self.tag.nloop == 0 {
                        break;
                    }
                    let d = (self.tag.regs >> (4 * self.tag.j)) & 0xf;
                    if d < 0xe {
                        self.reg(d as u8, half);
                    }
                    self.next_reg();
                }
            }
            _ => {
                self.image(q);
                self.tag.nloop -= 1;
            }
        }
    }

    fn next_reg(&mut self) {
        self.tag.j += 1;
        if self.tag.j == self.tag.nreg {
            self.tag.j = 0;
            self.tag.nloop -= 1;
        }
    }

    fn packed(&mut self, d: u8, q: u128) {
        let lo = q as u64;
        match d {
            0 => self.reg(0, lo & 0x7ff),
            1 => {
                let c = |s: u32| ((q >> s) & 0xff) as u8;
                self.rgbaq = ([c(0), c(32), c(64), c(96)], self.packed_q);
            }
            2 => {
                self.st = (f32::from_bits(q as u32), f32::from_bits((q >> 32) as u32));
                self.packed_q = f32::from_bits((q >> 64) as u32);
            }
            3 => self.uv = ((q & 0x3fff) as u16, ((q >> 32) & 0x3fff) as u16),
            4 => {
                let v = (q & 0xffff) as u64
                    | (((q >> 32) & 0xffff) as u64) << 16
                    | (((q >> 68) & 0xff_ffff) as u64) << 32
                    | (((q >> 100) & 0xff) as u64) << 56;
                self.reg(if (q >> 111) & 1 != 0 { 0x0c } else { 0x04 }, v);
            }
            5 => {
                let v = (q & 0xffff) as u64 | (((q >> 32) & 0xffff) as u64) << 16 | (((q >> 64) & 0xffff_ffff) as u64) << 32;
                self.reg(if (q >> 111) & 1 != 0 { 0x0d } else { 0x05 }, v);
            }
            0xa => self.fog = ((q >> 100) & 0xff) as u8,
            0xe => self.reg(((q >> 64) & 0x7f) as u8, lo),
            0xf => {}
            _ => self.reg(d, lo),
        }
    }

    fn reg(&mut self, a: u8, v: u64) {
        let a = a & 0x7f;
        match a {
            0x00 => {
                self.r[0] = v;
                self.vq.clear();
            }
            0x01 => self.rgbaq = ([v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8], f32::from_bits((v >> 32) as u32)),
            0x02 => self.st = (f32::from_bits(v as u32), f32::from_bits((v >> 32) as u32)),
            0x03 => self.uv = ((v & 0x3fff) as u16, ((v >> 16) & 0x3fff) as u16),
            0x04 | 0x05 | 0x0c | 0x0d => {
                let f = a == 0x04 || a == 0x0c;
                if f {
                    self.fog = (v >> 56) as u8;
                }
                let vt = Vtx {
                    x: v as u16,
                    y: (v >> 16) as u16,
                    z: if f { ((v >> 32) & 0xff_ffff) as u32 } else { (v >> 32) as u32 },
                    rgba: self.rgbaq.0,
                    q: self.rgbaq.1,
                    s: self.st.0,
                    t: self.st.1,
                    u: self.uv.0,
                    v: self.uv.1,
                    f: self.fog,
                };
                self.kick(vt, a == 0x04 || a == 0x05);
            }
            0x0a => self.fog = (v >> 56) as u8,
            0x53 => {
                if self.log && std::env::var("ORACLE_GSSEQ").is_ok() {
                    let (bb, rr) = (self.r[0x50], self.r[0x52]);
                    eprintln!("seq f{} trx dir {} bitblt {bb:x} reg {}x{}", self.frames, v & 3, rr & 0xfff, (rr >> 32) & 0xfff);
                }
                if self.log {
                    let (bb, rr) = (self.r[0x50], self.r[0x52]);
                    self.stat(format!("trx dir {} psm {:x} {}x{} dbp {:x}", v & 3, (bb >> 56) & 0x3f, rr & 0xfff, (rr >> 32) & 0xfff, (bb >> 32) & 0x3fff));
                }
                self.r[0x53] = v;
                self.trx = Trx::default();
                self.deferring = false;
                match v & 3 {
                    0 => self.trx_start(),
                    2 => {
                        self.flush_all();
                        self.local_copy();
                    }
                    _ => {}
                }
            }
            0x54 => self.image(v as u128),
            0x06 | 0x07 => {
                self.r[a as usize] = v;
                self.clut_load(v);
            }
            0x16 | 0x17 => {
                // TEX2: PSM and the CLUT fields replace those of TEX0.
                let k = (a - 0x10) as usize;
                let m = (0x3fu64 << 20) | (!0u64 << 37);
                self.r[k] = (self.r[k] & !m) | (v & m);
                let t = self.r[k];
                self.clut_load(t);
            }
            _ => self.r[a as usize] = v,
        }
    }

    fn kick(&mut self, v: Vtx, draw: bool) {
        let prim = self.r[0];
        let ty = prim & 7;
        self.vq.push(v);
        let n = self.vq.len();
        let need = match ty {
            0 => 1,
            1 | 2 | 6 => 2,
            3..=5 => 3,
            _ => {
                self.vq.clear();
                return;
            }
        };
        if n < need {
            return;
        }
        let vs: Vec<Vtx> = self.vq.clone();
        match ty {
            2 => {
                self.vq.remove(0);
            }
            4 => {
                self.vq.remove(0);
            }
            5 => {
                self.vq.remove(1);
            }
            _ => self.vq.clear(),
        }
        if draw {
            self.draw(ty as u32, &vs);
        }
    }

    /// Effective prim attributes (PRMODECONT.AC = 0 takes them from PRMODE).
    fn attrs(&self) -> u64 {
        if self.r[0x1a] & 1 != 0 { self.r[0] } else { (self.r[0] & 7) | (self.r[0x1b] & !7) }
    }

    /// A host upload into the main buffer (the pause menu restores its grabbed frame each field): the overlay shows
    /// the scene there, which rac-rs draws.
    fn upload_main(&mut self, a: usize) {
        let Some(fb) = self.main_fbp else { return };
        let blk = (a / 256) as u32;
        if blk < fb * 32 || blk >= self.main_end {
            return;
        }
        let word = (a / 4) as u32 - fb * 32 * 64;
        let (page, b, w) = (word / 2048, (word / 64) % 32, word % 64);
        let (mut bx, mut by, mut cx, mut cy) = (0, 0, 0, 0);
        for (r, row) in BT32.iter().enumerate() {
            if let Some(c) = row.iter().position(|&v| v as u32 == b) {
                (bx, by) = (c as u32, r as u32);
            }
        }
        for (r, row) in CT32.iter().enumerate() {
            if let Some(c) = row.iter().position(|&v| v as u32 == w) {
                (cx, cy) = (c as u32, r as u32);
            }
        }
        let bw = self.main_bw.max(1);
        let (x, y) = ((page % bw) * 64 + bx * 8 + cx, (page / bw) * 32 + by * 8 + cy);
        if x < self.cur.w && y < self.cur.h {
            self.cur.px[(y * self.cur.w + x) as usize] = [0.0, 0.0, 0.0, 1.0];
        }
        // The restored frame stands in for the 3D phase: later draws are overlay.
        self.mscal = self.mscal.max(SKY_PROGRAMS);
        self.cur.frozen = true;
    }

    // ---- local memory addressing (GS swizzle) ----

    fn a32(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 32) * bw + x / 64;
        ((bp * 256 + page * 8192 + luts().0[((y % 32) * 64 + x % 64) as usize]) & 0x3f_ffff) as usize
    }

    fn a16(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 64) * bw + x / 64;
        ((bp * 256 + page * 8192 + luts().1[((y % 64) * 64 + x % 64) as usize]) & 0x3f_ffff) as usize
    }

    fn a8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 64) * (bw / 2).max(1) + x / 128;
        ((bp * 256 + page * 8192 + luts().2[((y % 64) * 128 + x % 128) as usize]) & 0x3f_ffff) as usize
    }

    /// Nibble address.
    fn a4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 128) * (bw / 2).max(1) + x / 128;
        ((bp * 512 + page * 16384 + luts().3[((y % 128) * 128 + x % 128) as usize]) & 0x7f_ffff) as usize
    }

    fn rd32(&self, a: usize) -> u32 {
        u32::from_le_bytes(self.vram[a..a + 4].try_into().unwrap())
    }

    fn wr_px(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
        match psm {
            0x00 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a..a + 4].copy_from_slice(&v.to_le_bytes());
            }
            0x01 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a..a + 3].copy_from_slice(&v.to_le_bytes()[..3]);
            }
            0x02 => {
                let a = Self::a16(bp, bw, x, y);
                self.vram[a..a + 2].copy_from_slice(&(v as u16).to_le_bytes());
            }
            0x13 => {
                let a = Self::a8(bp, bw, x, y);
                self.vram[a] = v as u8;
            }
            0x14 => {
                let n = Self::a4(bp, bw, x, y);
                let b = &mut self.vram[n >> 1];
                *b = if n & 1 != 0 { (*b & 0x0f) | ((v as u8) << 4) } else { (*b & 0xf0) | (v as u8 & 0xf) };
            }
            0x1b => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = v as u8;
            }
            0x24 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = (self.vram[a + 3] & 0xf0) | (v as u8 & 0xf);
            }
            0x2c => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = (self.vram[a + 3] & 0x0f) | ((v as u8) << 4);
            }
            _ => {}
        }
    }

    pub fn rd_px(&self, psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
        match psm {
            0x00 => self.rd32(Self::a32(bp, bw, x, y)),
            0x01 => self.rd32(Self::a32(bp, bw, x, y)) & 0xff_ffff,
            0x02 => {
                let a = Self::a16(bp, bw, x, y);
                u16::from_le_bytes([self.vram[a], self.vram[a + 1]]) as u32
            }
            0x13 => self.vram[Self::a8(bp, bw, x, y)] as u32,
            0x14 => {
                let n = Self::a4(bp, bw, x, y);
                (self.vram[n >> 1] >> ((n & 1) * 4)) as u32 & 0xf
            }
            0x1b => self.vram[Self::a32(bp, bw, x, y) + 3] as u32,
            0x24 => self.vram[Self::a32(bp, bw, x, y) + 3] as u32 & 0xf,
            0x2c => (self.vram[Self::a32(bp, bw, x, y) + 3] >> 4) as u32,
            _ => 0,
        }
    }

    /// Bits per pixel of a transfer format.
    fn bpp(psm: u32) -> u32 {
        match psm {
            0x00 => 32,
            0x01 => 24,
            0x02 | 0x0a => 16,
            0x13 | 0x1b => 8,
            _ => 4,
        }
    }

    /// HWREG data (EE -> local memory, TRXDIR 0).
    fn image(&mut self, q: u128) {
        if self.r[0x53] & 3 != 0 {
            return;
        }
        if self.deferring {
            if let Some(p) = self.pend.last_mut() {
                p.data.push(q);
                return;
            }
        }
        let mut t = std::mem::take(&mut self.trx);
        self.put(self.r[0x50], self.r[0x51], self.r[0x52], &mut t, q);
        self.trx = t;
    }

    /// One HWREG qword of a transfer (BITBLTBUF, TRXPOS, TRXREG) at position `t`.
    fn put(&mut self, bb: u64, pos: u64, rr: u64, t: &mut Trx, q: u128) {
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 {
            return;
        }
        let bpp = Self::bpp(dpsm);
        let mut bits = q;
        let mut left = 128u32;
        while left >= bpp {
            if t.y >= rh {
                return;
            }
            let v = (bits & ((1u128 << bpp) - 1)) as u32;
            bits >>= bpp;
            left -= bpp;
            self.wr_px(dpsm, dbp, dbw, dx + t.x, dy + t.y, v);
            if dpsm == 0 {
                self.upload_main(Self::a32(dbp, dbw, dx + t.x, dy + t.y));
            }
            t.x += 1;
            if t.x >= rw {
                t.x = 0;
                t.y += 1;
            }
        }
        if left > 0 && bpp == 24 {
            // 24-bit pixels straddle qwords: carry the remainder (rare; not modelled).
            self.stat("image ct24 straddle".into());
        }
    }

    /// TRXDIR 0 start. The game streams its 3D textures every field (~1M texels) and the capture samples few of
    /// them, so a transfer waits in `pend` until a draw, CLUT load or copy touches its blocks; uploads into the
    /// main buffer go through at once (they mark the overlay).
    fn trx_start(&mut self) {
        let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 || rh == 0 {
            return;
        }
        let (lo, hi) = Self::span(dpsm, dbp, dbw, dx, dy, rw, rh);
        let main = self.main_fbp.map_or((0, 0), |f| (f * 32, self.main_end));
        if lo < main.1 && main.0 < hi {
            self.flush(lo, hi);
            return;
        }
        // The same upload again replaces the waiting one.
        self.pend.retain(|p| !(p.bb == bb && p.pos == pos && p.rr == rr));
        self.pend.push(Pend { bb, pos, rr, lo, hi, data: Vec::new() });
        self.deferring = true;
    }

    /// Block range `[lo, hi)` a rectangle of a buffer touches.
    fn span(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
        let (sw, sh, f): (u32, u32, fn(u32, u32, u32, u32) -> usize) = match psm {
            0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (8, 8, Self::a32),
            0x02 | 0x0a => (16, 8, Self::a16),
            0x13 => (16, 16, Self::a8),
            0x14 => (32, 16, |b, w, x, y| Self::a4(b, w, x, y) / 2),
            _ => return (0, u32::MAX),
        };
        let (mut lo, mut hi) = (u32::MAX, 0);
        for xx in (x..x + w).step_by(sw as usize).chain(std::iter::once(x + w - 1)) {
            for yy in (y..y + h).step_by(sh as usize).chain(std::iter::once(y + h - 1)) {
                let b = (f(bp, bw, xx, yy) / 256) as u32;
                lo = lo.min(b);
                hi = hi.max(b + 1);
            }
        }
        (lo, hi)
    }

    /// Writes the waiting transfers that touch blocks `[lo, hi)`, with every older one overlapping those, in order.
    fn flush(&mut self, lo: u32, hi: u32) {
        let hit = |p: &Pend, lo: u32, hi: u32| p.lo < hi && lo < p.hi;
        if !self.pend.iter().any(|p| hit(p, lo, hi)) {
            return;
        }
        let n = self.pend.len();
        let mut sel: Vec<bool> = self.pend.iter().map(|p| hit(p, lo, hi)).collect();
        for j in (0..n).rev() {
            if sel[j] {
                let (l, h) = (self.pend[j].lo, self.pend[j].hi);
                for i in 0..j {
                    if !sel[i] && hit(&self.pend[i], l, h) {
                        sel[i] = true;
                    }
                }
            }
        }
        let active = self.deferring && sel[n - 1];
        let (mut run, mut keep) = (Vec::new(), Vec::new());
        for (p, s) in std::mem::take(&mut self.pend).into_iter().zip(sel) {
            if s {
                run.push(p);
            } else {
                keep.push(p);
            }
        }
        self.pend = keep;
        for p in &run {
            let mut t = Trx::default();
            for &q in &p.data {
                self.put(p.bb, p.pos, p.rr, &mut t, q);
            }
            self.trx = t;
        }
        // A transfer still receiving data continues straight into VRAM.
        if active {
            self.deferring = false;
        }
    }

    /// Writes every waiting transfer (before reading VRAM from outside).
    pub fn flush_all(&mut self) {
        self.flush(0, u32::MAX);
    }

    /// TRXDIR 2: local -> local copy.
    fn local_copy(&mut self) {
        let bb = self.r[0x50];
        let (sbp, sbw, spsm) = ((bb & 0x3fff) as u32, ((bb >> 16) & 0x3f) as u32, ((bb >> 24) & 0x3f) as u32);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let pos = self.r[0x51];
        let (sx, sy, dx, dy) = ((pos & 0x7ff) as u32, ((pos >> 16) & 0x7ff) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let rr = self.r[0x52];
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        for y in 0..rh {
            for x in 0..rw {
                let v = self.rd_px(spsm, sbp, sbw, sx + x, sy + y);
                self.wr_px(dpsm, dbp, dbw, dx + x, dy + y, v);
            }
        }
        self.stat(format!("local copy {rw}x{rh} psm {spsm:x}->{dpsm:x}"));
    }
}

/// In-page swizzle offsets (bytes; nibbles for PSMT4) of CT32, CT16, T8 and T4, by `y * page width + x`.
type Luts = (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>);

fn luts() -> &'static Luts {
    static L: std::sync::OnceLock<Luts> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let t = |w: u32, h: u32, f: fn(u32, u32, u32, u32) -> usize, bw: u32| (0..w * h).map(|i| f(0, bw, i % w, i / w) as u32).collect();
        (t(64, 32, r32, 1), t(64, 64, r16, 1), t(128, 64, r8, 2), t(128, 128, r4, 2))
    })
}

fn r32(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 32) * bw + x / 64;
    let blk = BT32[((y % 32) / 8) as usize][((x % 64) / 8) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + CT32[(y % 8) as usize][(x % 8) as usize] as u32) * 4) & 0x3f_ffff) as usize
}

fn r16(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * bw + x / 64;
    let blk = BT16[((y % 64) / 8) as usize][((x % 64) / 16) as usize] as u32;
    let hw = CT32[(y % 8) as usize][(x % 8) as usize] as u32 * 2 + ((x >> 3) & 1);
    ((((bp + page * 32 + blk) * 128 + hw) * 2) & 0x3f_ffff) as usize
}

fn r8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * (bw / 2).max(1) + x / 128;
    let blk = BT32[((y % 64) / 16) as usize][((x % 128) / 16) as usize] as u32;
    let (xx, yy) = (x & 15, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 4 + ((xx >> 3) & 1) * 2 + ((yy >> 1) & 1)) & 0x3f_ffff) as usize
}

fn r4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 128) * (bw / 2).max(1) + x / 128;
    let blk = BT16[((y % 128) / 16) as usize][((x % 128) / 32) as usize] as u32;
    let (xx, yy) = (x & 31, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 8 + ((xx >> 3) & 3) * 2 + ((yy >> 1) & 1)) & 0x7f_ffff) as usize
}

const BT32: [[u8; 8]; 4] = [
    [0, 1, 4, 5, 16, 17, 20, 21],
    [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29],
    [10, 11, 14, 15, 26, 27, 30, 31],
];
const BT16: [[u8; 4]; 8] = [
    [0, 2, 8, 10],
    [1, 3, 9, 11],
    [4, 6, 12, 14],
    [5, 7, 13, 15],
    [16, 18, 24, 26],
    [17, 19, 25, 27],
    [20, 22, 28, 30],
    [21, 23, 29, 31],
];
const CT32: [[u8; 8]; 8] = [
    [0, 1, 4, 5, 8, 9, 12, 13],
    [2, 3, 6, 7, 10, 11, 14, 15],
    [16, 17, 20, 21, 24, 25, 28, 29],
    [18, 19, 22, 23, 26, 27, 30, 31],
    [32, 33, 36, 37, 40, 41, 44, 45],
    [34, 35, 38, 39, 42, 43, 46, 47],
    [48, 49, 52, 53, 56, 57, 60, 61],
    [50, 51, 54, 55, 58, 59, 62, 63],
];
