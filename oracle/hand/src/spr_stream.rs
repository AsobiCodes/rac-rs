//! Scratchpad DMA wait, the DMA packet table builder, and the packed-stream unpacker that reads its input
//! through the 16 KB scratchpad ring.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Channel 9 (RAM to scratchpad). MADR, QWC and SADR sit at +0x10, +0x20 and +0x80; CHCR is the base.
const SPR_TO: u32 = 0x1000_d400;
/// CHCR bit 8: transfer started / still running.
const DMA_BUSY: u32 = 0x100;
/// Scratchpad as the retail code addresses it.
const SPR_BASE: u32 = 0x7000_0000;
/// One half of the scratchpad ring; also the size of one input chunk.
const HALF: u32 = 0x2000;
/// Quadword count of one kick (one half).
const QWC_HALF: u32 = 0x200;
/// Size of one packet written by the table builder.
const PACKET: u32 = 0x70;
/// Packet table in RAM: 16-byte entries, count kept at gp-0x77e8.
const TABLE: u32 = 0x0016_ce80;
/// Stream header: halfword and byte that must match these xor keys.
const MAGIC_WORD: u32 = 0x4157;
const MAGIC_BYTE: u32 = 0x44;
/// Retail `lui t2, 0x8000` sign-extended: an end pointer above every scratchpad address.
const NO_LIMIT: u64 = 0xffff_ffff_8000_0000;
/// Zero quadword.
const ZERO: V = V([0; 4]);

/// Sign-extends a 32-bit value to 64 bits, as MIPS64 32-bit ops leave it.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Spins while the RAM-to-scratchpad DMA is busy (the retail wait loop at 0x240140).
fn wait_dma(c: &mut Cpu) {
    while c.r32(SPR_TO) & DMA_BUSY != 0 {}
}

/// The retail kick at 0x240120: source (scratchpad), quadword count, RAM address, then CHCR start.
/// The copy itself is synchronous in this emulator.
fn kick(c: &mut Cpu, spr: u32, mem: u32) {
    c.w32(SPR_TO + 0x80, spr);
    c.w32(SPR_TO + 0x20, QWC_HALF);
    c.w32(SPR_TO + 0x10, mem);
    c.w32(SPR_TO, DMA_BUSY);
}

/// 0x23fc18 wait_spr_to_idle() -> 0: spins until the scratchpad DMA (channel 9) is idle, returns 0.
pub fn wait_spr_to_idle(c: &mut Cpu) -> bool {
    wait_dma(c);
    c.s(1, SPR_TO as u64); // $at keeps the address the retail lui/ori left in it
    c.ret(0);
    true
}

/// Writes the common packet head: five template quadwords, a zero quad, and the 0x53 tag at +0x48.
fn write_head(c: &mut Cpu, p: u32, t: &[V; 5]) {
    c.wv(p, t[0]);
    c.wv(p + 0x10, t[1]);
    c.wv(p + 0x20, t[2]);
    c.wv(p + 0x30, t[3]);
    c.wv(p + 0x40, ZERO);
    c.wv(p + 0x50, t[4]);
    c.w32(p + 0x48, 0x53);
}

/// Writes the tail of a packet: the shifted mask at +0x50, a zero quad, and the two address words.
fn write_tail(c: &mut Cpu, p: u32, at: u32, mask: u32) {
    c.w32(p + 0x50, mask | 0x8000);
    c.wv(p + 0x60, ZERO);
    c.w32(p + 0x64, at);
    c.w32(p + 0x60, 0x3000_0000u32.wrapping_add(mask));
    c.w32(p + 0x6c, 0x5000_0000u32.wrapping_add(mask));
}

/// First packet of a table entry (taken when its first word is nonzero).
fn packet_a(c: &mut Cpu, p: u32, t: &[V; 5], at: u32, h: u32, h2: u32) {
    write_head(c, p, t);
    c.w8(p + 0x27, h as u8);
    c.w16(p + 0x24, h2 as u16);
    c.w32(p + 0x30, 0x10);
    c.w32(p + 0x34, 0x10);
    let mask = 0x40u32.wrapping_shr(h >> 1);
    write_tail(c, p, at, mask);
}

/// Second packet of a table entry (always written).
fn packet_b(c: &mut Cpu, p: u32, t: &[V; 5], hw: u32, h2: u32, at: u32) {
    write_head(c, p, t);
    c.w16(p + 0x24, h2 as u16);
    let lo = hw & 0xff;
    let hi = hw >> 8;
    let shift = lo as i32 - 6;
    if shift > 0 {
        c.w8(p + 0x26, 1u32.wrapping_shl(shift as u32) as u8);
    }
    let bits_hi = 1u32.wrapping_shl(hi);
    let bits_lo = 1u32.wrapping_shl(lo);
    c.w32(p + 0x34, bits_hi);
    let mask = bits_hi.wrapping_shl(lo) >> 4;
    c.w32(p + 0x30, bits_lo);
    write_tail(c, p, at, mask);
}

/// 0x23fce0 emit_spr_packets(): for each 16-byte entry of the table at 0x16ce80 (count at gp-0x77e8), appends one or
/// two 0x70-byte DMA packets built from the quadword templates at gp-0x6460..gp-0x6420, at the write pointer kept in
/// gp-0x5ac0, and stores the advanced pointer back.
pub fn emit_spr_packets(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let t = [
        c.rv(gp.wrapping_sub(0x6460)),
        c.rv(gp.wrapping_sub(0x6450)),
        c.rv(gp.wrapping_sub(0x6440)),
        c.rv(gp.wrapping_sub(0x6430)),
        c.rv(gp.wrapping_sub(0x6420)),
    ];
    let mut left = c.r32(gp.wrapping_sub(0x77e8)) as i32;
    let mut entry = TABLE;
    let mut out = c.r32(gp.wrapping_sub(0x5ac0));
    while left != 0 {
        let at_a = c.r32(entry);
        let ha = c.r16(entry + 4) as u32;
        let ha2 = c.r16(entry + 6) as u32;
        left = left.wrapping_sub(1);
        if at_a != 0 {
            packet_a(c, out, &t, at_a, ha, ha2);
            out = out.wrapping_add(PACKET);
        }
        let at_b = c.r32(entry + 8);
        let hb = c.r16(entry + 0xc) as u32;
        let hb2 = c.r16(entry + 0xe) as u32;
        packet_b(c, out, &t, hb, hb2, at_b);
        out = out.wrapping_add(PACKET);
        entry = entry.wrapping_add(0x10);
    }
    c.w32(gp.wrapping_sub(0x5ac0), out);
    true
}

/// Where the unpacker is in its control flow (labels of the retail routine).
enum Next {
    /// First control byte of a stream chunk: a literal run when it is above 17.
    Control,
    /// Control byte below 16 or above 16: literal run of 3 plus a count, or a match.
    Check,
    /// After a literal run: reads the next control byte.
    AfterLiteral,
    /// Match dispatch on the control byte.
    Match(u8),
    /// Trailing literal count of a match.
    Tail,
    Done,
}

/// Decoder state, named after the retail registers it lives in.
struct Stream {
    /// t7: read pointer in the scratchpad ring.
    ring: u32,
    /// t2: end of the valid input in the ring (no limit while more than one half remains).
    end: u64,
    /// t1: input bytes left, relative to the current fill.
    remain: u32,
    /// t3: next RAM address to fetch an input chunk from.
    input: u32,
    /// t6: ring half the next kick fills.
    half: u32,
    /// t5: output cursor.
    out: u32,
}

impl Stream {
    fn has_input(&self) -> bool {
        (self.ring as u64) < self.end
    }

    fn byte(&mut self, c: &mut Cpu) -> u8 {
        let b = c.r8(self.ring);
        self.ring = self.ring.wrapping_add(1);
        b
    }

    fn put(&mut self, c: &mut Cpu, b: u8) {
        c.w8(self.out, b);
        self.out = self.out.wrapping_add(1);
    }

    /// Copies `count` input bytes (at least one: the loop is a do-while in the retail code).
    fn literal(&mut self, c: &mut Cpu, count: i32) {
        for _ in 0..count.max(1) {
            let b = self.byte(c);
            self.put(c, b);
        }
    }

    /// Sets the end pointer from the bytes left: unlimited while more than one half remains.
    fn set_end(&mut self) {
        self.end = if sx(self.remain) > 0x1fff {
            NO_LIMIT
        } else {
            sx(self.ring.wrapping_add(self.remain))
        };
    }

    /// Kicks the DMA of the next input chunk into the half that t6 names, then flips t6.
    fn kick_half(&mut self, c: &mut Cpu) {
        kick(c, self.half, self.input);
        self.half ^= HALF;
    }

    /// Waits for the ring, fetches the next chunk, and restarts reading at the new half.
    fn refill(&mut self, c: &mut Cpu) {
        wait_dma(c);
        self.ring = SPR_BASE;
        self.remain = self.remain.wrapping_sub(HALF);
        self.kick_half(c);
        self.input = self.input.wrapping_add(HALF);
        self.ring = SPR_BASE.wrapping_add(self.half);
        self.set_end();
    }

    /// Copies a back-reference: two bytes, then `count` bytes (at least one), reading each after it is written.
    fn copy(&mut self, c: &mut Cpu, from: u32, count: i32) {
        let mut m = from;
        let b0 = c.r8(m);
        self.put(c, b0);
        let b1 = c.r8(m.wrapping_add(1));
        self.put(c, b1);
        m = m.wrapping_add(2);
        for _ in 0..count.max(1) {
            let b = c.r8(m);
            m = m.wrapping_add(1);
            self.put(c, b);
        }
    }

    /// Reads the two-byte distance field used by the M3 and M4 matches.
    fn dist_pair(&mut self, c: &mut Cpu) -> u32 {
        let a = self.byte(c) as u32;
        let b = self.byte(c) as u32;
        (a >> 2) + (b << 6)
    }

    fn control(&mut self, c: &mut Cpu) -> Next {
        let v = c.r8(self.ring) as i32 - 0x11;
        if v <= 0 {
            return Next::Check;
        }
        self.ring = self.ring.wrapping_add(1);
        self.literal(c, v);
        Next::AfterLiteral
    }

    fn check(&mut self, c: &mut Cpu) -> Next {
        if !self.has_input() {
            return Next::Done;
        }
        let v = self.byte(c);
        if v >= 16 {
            return Next::Match(v);
        }
        let mut n = v as i32;
        if v == 0 {
            n = self.byte(c) as i32 + 15;
        }
        for _ in 0..3 {
            let b = self.byte(c);
            self.put(c, b);
        }
        self.literal(c, n);
        Next::AfterLiteral
    }

    fn after_literal(&mut self, c: &mut Cpu) -> Next {
        if !self.has_input() {
            return Next::Done;
        }
        Next::Match(self.byte(c))
    }

    fn matched(&mut self, c: &mut Cpu, v: u8) -> Next {
        let v = v as u32;
        if v >= 0x40 {
            let low = (v >> 2) & 7;
            let high = self.byte(c) as u32;
            let from = self.out.wrapping_sub(1).wrapping_sub(low).wrapping_sub(high << 3);
            self.copy(c, from, (v >> 5) as i32 - 1);
        } else if v >= 0x20 {
            let mut len = v & 0x1f;
            if len == 0 {
                len = self.byte(c) as u32 + 0x1f;
            }
            let dist = self.dist_pair(c);
            self.copy(c, self.out.wrapping_sub(1).wrapping_sub(dist), len as i32);
        } else if v >= 0x10 {
            let mut len = v & 7;
            let mut from = self.out.wrapping_sub((v & 8) << 11);
            if len == 0 {
                len = self.byte(c) as u32 + 7;
            }
            from = from.wrapping_sub(self.dist_pair(c));
            if from != self.out {
                self.copy(c, from.wrapping_sub(0x4000), len as i32);
            } else if len != 1 {
                // Zero distance and length other than 1: the chunk is used up, fetch the next one.
                self.refill(c);
                return Next::Check;
            }
            // Zero distance and length 1: end-of-stream marker.
        } else {
            // The retail code executes a trap here (teq); a valid stream never gets here.
            panic!("spr_lzo_unpack: invalid control byte {v:#x}");
        }
        Next::Tail
    }

    fn tail(&mut self, c: &mut Cpu) -> Next {
        let n = (c.r8(self.ring.wrapping_sub(2)) & 3) as u32;
        if n == 0 {
            return Next::Check;
        }
        let bytes = [c.r8(self.ring), c.r8(self.ring.wrapping_add(1)), c.r8(self.ring.wrapping_add(2))];
        for (i, b) in bytes.into_iter().enumerate() {
            c.w8(self.out.wrapping_add(i as u32), b);
        }
        self.out = self.out.wrapping_add(n);
        self.ring = self.ring.wrapping_add(n);
        Next::AfterLiteral
    }
}

/// 0x23fe50 spr_lzo_unpack(src, dst) -> output length: decodes the packed stream at src (magic check, size word at
/// +3, data from +0x10) into dst, fetching input through the scratchpad ring. Declines streams without the magic.
pub fn spr_lzo_unpack(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let dst = c.arg(1);
    if (c.r16(src) as u32 ^ MAGIC_WORD) != (c.r8(src + 2) as u32 ^ MAGIC_BYTE) {
        return false;
    }
    let size = u32::from_le_bytes([c.r8(src + 3), c.r8(src + 4), c.r8(src + 5), c.r8(src + 6)]);
    let mut s = Stream {
        ring: SPR_BASE,
        end: NO_LIMIT,
        remain: size.wrapping_sub(0x10),
        input: src + 0x10,
        half: 0,
        out: dst,
    };
    s.kick_half(c);
    wait_dma(c);
    s.input = s.input.wrapping_add(HALF);
    s.kick_half(c);
    s.input = s.input.wrapping_add(HALF);
    s.set_end();
    let mut next = Next::Control;
    loop {
        next = match next {
            Next::Control => s.control(c),
            Next::Check => s.check(c),
            Next::AfterLiteral => s.after_literal(c),
            Next::Match(v) => s.matched(c, v),
            Next::Tail => s.tail(c),
            Next::Done => break,
        };
    }
    c.ret(s.out.wrapping_sub(dst));
    true
}
