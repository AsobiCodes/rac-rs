//! The generated code's memory fast path: plain RAM, scratchpad and VU0 data memory, on a view of the first fields
//! of ee-oracle's `Mem` (same layout; ee-oracle checks every offset at compile time). Moved verbatim from
//! ee-oracle rec/mod.rs.

/// EE main RAM size.
pub const RAM: usize = 32 << 20;

/// The first fields of ee-oracle's `Mem`, in its order (both `#[repr(C)]`). Only ever a view.
#[repr(C)]
#[non_exhaustive]
pub struct Mem {
    pub ram: Vec<u8>,
    pub spr: Vec<u8>,
    pub vu0d: Vec<u8>,
    /// Debug: log CPU writes into VU0 data memory too (env COLD_WATCH_VU0D); `vu0d_cpu_writes` counts them (all of
    /// them while the log is on; otherwise recompiled code stores there directly, see rec/mod.rs `fbuf`).
    pub watch_vu: bool,
    /// An interrupt line may be asserted: the CPU re-tests INTC/DMAC.
    pub irq_dirty: bool,
    /// When the pending VIF1 stop becomes visible (u64::MAX: none).
    pub due: u64,
    /// 0 while `irq_dirty`, else `due`: the block-entry poll is `icount + n <= limit`.
    pub limit: u64,
}

impl Mem {
    /// The RAM or scratchpad bytes at `a..a+n`, when the address is plain memory (no side effects).
    #[inline(always)]
    pub fn fbuf(&mut self, a: u32, n: usize) -> Option<&mut [u8]> {
        if a >> 28 == 7 {
            let i = (a & 0x0fff_ffff) as usize;
            return if i + n <= 0x4000 { Some(&mut self.spr[i..i + n]) } else { None };
        }
        let p = if (0x8000_0000..0xc000_0000).contains(&a) { a & 0x1fff_ffff } else { a };
        // [PC] (speed) VU0 data memory (0x1100_4000 and its mirrors to 0x1100_7fff, `map`'s Loc::Vu(1)) is plain bytes
        // too: the EE's direct stores into it (Blarg's skinning, skinned_vertex_stream) took the hardware path per
        // word and ~10% of the EE thread. Not while COLD_WATCH_VU0D logs every write there (`Mem::wr`).
        if p >> 14 == 0x1100_4000 >> 14 && !self.watch_vu {
            let i = (p & 0xfff) as usize;
            return if i + n <= 0x1000 { self.vu0d.get_mut(i..i + n) } else { None };
        }
        let hi = p >> 28;
        if (hi == 0 || hi == 2 || hi == 3) && p & 0x0e00_0000 == 0 {
            let i = (p & 0x01ff_ffff) as usize;
            if i + n <= RAM {
                return Some(&mut self.ram[i..i + n]);
            }
        }
        None
    }

    #[inline(always)]
    #[doc(hidden)]
    pub fn fast(&mut self, a: u32, n: usize) -> bool {
        self.fbuf(a, n).is_some()
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn frd8(&mut self, a: u32) -> Option<u8> {
        self.fbuf(a, 1).map(|b| b[0])
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn frd16(&mut self, a: u32) -> Option<u16> {
        self.fbuf(a, 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()))
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn frd32(&mut self, a: u32) -> Option<u32> {
        self.fbuf(a, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn frd64(&mut self, a: u32) -> Option<u64> {
        self.fbuf(a, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn frd128(&mut self, a: u32) -> Option<u128> {
        self.fbuf(a, 16).map(|b| u128::from_le_bytes(b.try_into().unwrap()))
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn fwr8(&mut self, a: u32, v: u8) -> bool {
        self.fbuf(a, 1).map(|b| b[0] = v).is_some()
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn fwr16(&mut self, a: u32, v: u16) -> bool {
        self.fbuf(a, 2).map(|b| b.copy_from_slice(&v.to_le_bytes())).is_some()
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn fwr32(&mut self, a: u32, v: u32) -> bool {
        self.fbuf(a, 4).map(|b| b.copy_from_slice(&v.to_le_bytes())).is_some()
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn fwr64(&mut self, a: u32, v: u64) -> bool {
        self.fbuf(a, 8).map(|b| b.copy_from_slice(&v.to_le_bytes())).is_some()
    }
    #[inline(always)]
    #[doc(hidden)]
    pub fn fwr128(&mut self, a: u32, v: u128) -> bool {
        self.fbuf(a, 16).map(|b| b.copy_from_slice(&v.to_le_bytes())).is_some()
    }
}
