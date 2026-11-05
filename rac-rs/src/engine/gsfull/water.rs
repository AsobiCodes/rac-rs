//! R&C2 water surfaces for the native picture (rc2/native): the batches the game sends VU1 program 10 (the water surface [C: level 1 field 2700
//! RAC2_ONLYPROG=10 draws the swamp plane]), taken from VU1 data memory when the program starts, with the GS drawing state
//! (TEX0, ALPHA, ...) the GIF packets before it set.
//!
//! Program 10 input at TOP [C: decomp/vu1/prog10.s L_0080 / L_0498]: a GIF tag (NLOOP in the low 15 bits, PRE / PRIM), then
//! per vertex three qwords: ST (f32 s, t, q; the program multiplies them by its Q), RGBAQ (passed through untouched) and XYZ
//! (f32, transformed by the matrix at VU 932..935 (L_0080) or 937..940 (L_0498, clipped) and overwritten in place with the
//! screen XYZF). [I] world space: the matrix is the camera's (checked with RAC2_WATERLOG).

use std::sync::Mutex;

/// One vertex: ST (s, t, q), RGBA bytes, position.
#[derive(Clone, Copy, Debug, Default)]
pub struct WVert {
    pub st: [f32; 3],
    pub rgba: [u8; 4],
    pub xyz: [f32; 3],
}

/// One program-10 run: the GS state of the drawing context it uses and its vertices.
#[derive(Clone, Debug, Default)]
pub struct WaterBatch {
    pub prim: u64,
    pub tex0: u64,
    pub tex1: u64,
    pub clamp: u64,
    pub alpha: u64,
    pub test: u64,
    pub verts: Vec<WVert>,
}

/// The batches of the last field the pipe processed (replaced every field, so water the game stops sending goes away).
pub static WATER: Mutex<Vec<WaterBatch>> = Mutex::new(Vec::new());

/// The VU1 program id of the water surface.
pub const WATER_PROG: u8 = 10;

/// GS register values as the GIF packets left them (A+D writes and tag PRIM), tracked over every path in field order.
#[derive(Clone)]
pub struct GifRegs {
    /// Register units left in the current packet (PACKED: qwords; REGLIST: 64-bit halves; IMAGE: qwords).
    left: u32,
    nregs: u32,
    regs: u64,
    k: u32,
    flg: u32,
    pub r: [u64; 0x64],
}

impl Default for GifRegs {
    fn default() -> Self {
        GifRegs { left: 0, nregs: 0, regs: 0, k: 0, flg: 0, r: [0; 0x64] }
    }
}

impl GifRegs {
    pub fn feed(&mut self, q: u128) {
        if self.left == 0 {
            let nloop = (q & 0x7fff) as u32;
            if (q >> 46) & 1 != 0 {
                self.r[0] = ((q >> 47) & 0x7ff) as u64;
            }
            self.flg = ((q >> 58) & 3) as u32;
            self.nregs = match ((q >> 60) & 0xf) as u32 {
                0 => 16,
                n => n,
            };
            self.regs = (q >> 64) as u64;
            self.k = 0;
            self.left = match self.flg {
                0 => nloop * self.nregs,
                1 => nloop * self.nregs,
                _ => nloop,
            };
            return;
        }
        match self.flg {
            0 => {
                let reg = (self.regs >> (4 * self.k)) & 0xf;
                match reg {
                    0xe => {
                        let addr = ((q >> 64) & 0xff) as usize;
                        if addr < self.r.len() {
                            self.r[addr] = q as u64;
                        }
                    }
                    0 => self.r[0] = (q as u64) & 0x7ff,
                    _ => {}
                }
                self.k = (self.k + 1) % self.nregs;
                self.left -= 1;
            }
            1 => self.left = self.left.saturating_sub(2),
            _ => self.left -= 1,
        }
    }

    /// The batch state for the context PRIM bit 9 picks.
    pub fn batch(&self, prim: u64) -> WaterBatch {
        let c = ((prim >> 9) & 1) as usize;
        WaterBatch { prim, tex0: self.r[0x06 + c], tex1: self.r[0x14 + c], clamp: self.r[0x08 + c], alpha: self.r[0x42 + c], test: self.r[0x47 + c], verts: Vec::new() }
    }
}

/// Reads one program-10 input block at `top` (VU1 data memory in qwords).
pub fn capture(data: &[[u32; 4]], top: usize, regs: &GifRegs) -> Option<WaterBatch> {
    let tag = data.get(top)?;
    let n = (tag[0] & 0x7fff) as usize;
    if n == 0 || top + 1 + 3 * n > data.len() {
        return None;
    }
    let lo = tag[0] as u64 | (tag[1] as u64) << 32;
    let prim = if (lo >> 46) & 1 != 0 { (lo >> 47) & 0x7ff } else { regs.r[0] };
    let mut b = regs.batch(prim);
    let f = |w: u32| f32::from_bits(w);
    for k in 0..n {
        let (st, c, p) = (data[top + 1 + 3 * k], data[top + 2 + 3 * k], data[top + 3 + 3 * k]);
        let v = WVert { st: [f(st[0]), f(st[1]), f(st[2])], rgba: c.map(|x| x.min(255) as u8), xyz: [f(p[0]), f(p[1]), f(p[2])] };
        if !v.xyz.iter().all(|x| x.is_finite()) {
            return None;
        }
        b.verts.push(v);
    }
    if std::env::var_os("RAC2_WATERLOG").is_some() {
        let m: Vec<[f32; 4]> = (932..936).map(|a| data[a].map(f)).collect();
        eprintln!("water: {} verts prim {:#x} tex0 {:#x} alpha {:#x} test {:#x} first {:?} matrix {:?}", n, prim, b.tex0, b.alpha, b.test, b.verts[0], m);
    }
    Some(b)
}
