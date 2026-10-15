//! The PS2 CRTC: which part of local memory the TV shows, from the GS privileged registers.
//!
//! PMODE (read circuits 1/2 on or off, how they blend), SMODE2 (interlace, FRAME/FIELD read), DISPFB1/2 (frame buffer
//! address, width, format, top-left corner DBX/DBY) and DISPLAY1/2 (position DX/DY, magnification MAGH/MAGV, size
//! DW/DH in video clocks / lines), BGCOLOR. `Crtc::from_regs` snapshots them from the 0x2000-byte privileged register
//! block ee-oracle keeps (`Mem::gs`); `present` reads the picture out of the software GS the way the CRTC does.
//!
//! Model (PCSX2's, which the project uses as the reference picture):
//! * One circuit enabled: its rectangle (DBX/DBY from DISPFB, `(DW+1)/(MAGH+1)` x `(DH+1)/(MAGV+1)` pixels from DISPLAY).
//!   Interlaced with FFMD=0 ("read every other line", what R&C1 sets: SMODE2 = 1, DISPLAY DH 447, buffer 512x448) each
//!   field takes its own parity of the buffer's rows, so the woven picture is every row. Interlaced with FFMD=1 ("read
//!   every line") the buffer holds one field: half of the DISPLAY lines are rows of the buffer. Progressive: every row.
//! * Both enabled: circuit 2 is blended over circuit 1 (SLBG=0) or over BGCOLOR (SLBG=1) with alpha ALP (MMOD=1) or the
//!   pixel alpha of circuit 1 / 2 (MMOD=0, AMOD=0 / 1): out = (c2 - c1) * a / 128 + c1, a clamped to 0..255; circuit 2 is
//!   placed relative to circuit 1 by the difference of their DX / DY.
//! * Neither enabled: the background colour.
//! DX / DY otherwise only place the picture on the TV; like PCSX2 the window shows just the picture, stretched to the
//! TV's 4:3.
use super::gs::Gs;

/// The registers the CRTC reads (one snapshot per EE field).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Crtc {
    pub pmode: u64,
    pub smode2: u64,
    /// DISPFB1, DISPFB2.
    pub dispfb: [u64; 2],
    /// DISPLAY1, DISPLAY2.
    pub display: [u64; 2],
    pub bgcolor: u64,
}

impl Crtc {
    /// From the privileged register block (offsets of 0x1200_0000 + o).
    pub fn from_regs(r: &[u8]) -> Crtc {
        let q = |o: usize| r.get(o..o + 8).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()));
        Crtc { pmode: q(0), smode2: q(0x20), dispfb: [q(0x70), q(0x90)], display: [q(0x80), q(0xa0)], bgcolor: q(0xe0) }
    }

    /// Circuit k (0 = 1, 1 = 2) is on.
    pub fn enabled(&self, k: usize) -> bool {
        self.pmode >> k & 1 != 0
    }

    /// The buffer `ee_oracle::gs::Job` calls DISPFB: circuit 2 when only it is on, else circuit 1.
    pub fn job_dispfb(&self) -> u64 {
        if self.pmode & 3 == 2 { self.dispfb[1] } else { self.dispfb[0] }
    }

    fn interlaced(&self) -> bool {
        self.smode2 & 1 != 0
    }

    fn frame_mode(&self) -> bool {
        self.smode2 & 2 != 0
    }

    /// Circuit k's source rectangle and placement.
    fn circuit(&self, k: usize) -> Circuit {
        let (f, d) = (self.dispfb[k], self.display[k]);
        let (magh, magv) = ((d >> 23 & 0xf) as u32 + 1, (d >> 27 & 3) as u32 + 1);
        let (dw, dh) = ((d >> 32 & 0xfff) as u32 + 1, (d >> 44 & 0x7ff) as u32 + 1);
        let mut rows = dh / magv;
        if self.interlaced() && self.frame_mode() {
            rows /= 2;
        }
        Circuit {
            fbp: (f & 0x1ff) as u32 * 32,
            fbw: ((f >> 9 & 0x3f) as u32).max(1),
            psm: (f >> 15 & 0x1f) as u32,
            dbx: (f >> 32 & 0x7ff) as u32,
            dby: (f >> 43 & 0x7ff) as u32,
            w: (dw / magh).clamp(1, 2048),
            h: rows.clamp(1, 2048),
            dx: (d & 0xfff) as i32 / magh as i32,
            dy: (d >> 12 & 0x7ff) as i32 / magv as i32,
        }
    }

    /// BGCOLOR as RGB.
    fn bg(&self) -> [u8; 3] {
        [self.bgcolor as u8, (self.bgcolor >> 8) as u8, (self.bgcolor >> 16) as u8]
    }
}

struct Circuit {
    fbp: u32,
    fbw: u32,
    psm: u32,
    dbx: u32,
    dby: u32,
    w: u32,
    h: u32,
    dx: i32,
    dy: i32,
}

impl Circuit {
    fn read(&self, g: &mut Gs) -> Vec<u8> {
        g.read_rect(self.fbp, self.fbw, self.psm, self.dbx, self.dby, self.w, self.h)
    }
}

/// What the TV shows for one field: RGBA8, alpha 255.
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Reads the picture of `c` out of the GS (applies the deferred uploads that touch the displayed buffers).
pub fn present(g: &mut Gs, c: &Crtc) -> Picture {
    let opaque = |mut rgba: Vec<u8>| {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        rgba
    };
    match (c.enabled(0), c.enabled(1)) {
        (false, false) => {
            let [r, gr, b] = c.bg();
            Picture { w: 512, h: 448, rgba: [r, gr, b, 255].repeat(512 * 448) }
        }
        (true, false) | (false, true) => {
            let k = c.enabled(1) as usize;
            let ci = c.circuit(k);
            Picture { w: ci.w, h: ci.h, rgba: opaque(ci.read(g)) }
        }
        (true, true) => merge(g, c),
    }
}

/// Both circuits on: circuit 2 over circuit 1 (or over BGCOLOR when SLBG).
fn merge(g: &mut Gs, c: &Crtc) -> Picture {
    let (c1, c2) = (c.circuit(0), c.circuit(1));
    let (slbg, mmod, amod, alp) = (c.pmode >> 7 & 1 != 0, c.pmode >> 5 & 1 != 0, c.pmode >> 6 & 1 != 0, (c.pmode >> 8 & 0xff) as i32);
    let p2 = c2.read(g);
    // The base: circuit 1's picture, or the background over circuit 2's rectangle.
    let (w, h, base, ox, oy) = if slbg {
        let [r, gr, b] = c.bg();
        (c2.w, c2.h, [r, gr, b, 255].repeat((c2.w * c2.h) as usize), 0, 0)
    } else {
        (c1.w, c1.h, c1.read(g), c2.dx - c1.dx, c2.dy - c1.dy)
    };
    let mut out = base;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let (sx, sy) = (x - ox, y - oy);
            if sx < 0 || sy < 0 || sx >= c2.w as i32 || sy >= c2.h as i32 {
                continue;
            }
            let (o, s) = (((y as u32 * w + x as u32) * 4) as usize, ((sy as u32 * c2.w + sx as u32) * 4) as usize);
            let a = if mmod { alp } else if amod { p2[s + 3] as i32 } else { out[o + 3] as i32 };
            for i in 0..3 {
                let (d, sc) = (out[o + i] as i32, p2[s + i] as i32);
                out[o + i] = ((sc - d) * a / 128 + d).clamp(0, 255) as u8;
            }
        }
    }
    out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    Picture { w, h, rgba: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_registers() {
        // R&C1: interlaced, FFMD 0, DISPLAY DW 2559 MAGH 4 DH 447 over a 512x448 buffer.
        let c = Crtc { pmode: 1, smode2: 1, dispfb: [8 << 9, 0], display: [(2559u64 << 32) | (4 << 23) | (447 << 44), 0], bgcolor: 0 };
        let r = c.circuit(0);
        assert_eq!((r.w, r.h, r.fbw), (512, 448, 8));
        let f = Crtc { smode2: 3, ..c };
        assert_eq!(f.circuit(0).h, 224);
    }
}
