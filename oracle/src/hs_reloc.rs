//! Relocatable speed hands (lane hs, stage S2i, 2026-10-20).
//!
//! The recompiler compiles a retail function that exists in every level's overlay (and on the USA disc) ONCE: a template whose pcs are
//! `zb_ + OFF` and whose differing constants are `zv_[k]`. A speed hand is relocatable in the same way:
//!   `fr.pc(OFF)`  = base + OFF, base = the function's entry in the copy that runs (`Frame::base`);
//!   `fr.zv(k)`    = the template's `zv_[k]` for that copy, decoded here from the code words in RAM (`Reloc::zvals`), so it needs no rec crate and
//!                   equals the `V_` table of the level crate by construction (cut/hsreloc.py checks it for all 18 copies).
//! A copy matches the hand when its code words equal the template with the relocated fields masked out (`Reloc::matches`): the 16-bit immediate
//! of every instruction that has a `zv` slot is masked, and an absolute `j` target inside the function is made base-relative. The spec is written by
//! `cut/hsreloc.py spec` (text lines `reloc`, `mhash`, `mask`, `jabs`, `zv`) and parsed here from the same text (unit meta.txt / final meta).
use crate::mem::Mem;

pub const ZV_MAX: usize = 64;
const FNV_O: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_P: u64 = 0x100_0000_01b3;

/// One `zv_[k]` slot: the instruction at byte offset `off`; kind 0 = sign-extended imm16 (as u32), 1 = zero-extended imm16, 2 = `rem << 32 | word`,
/// 3 = `hi`: the `lui` immediate << 16 (recgen's `sx32(0xHHHH0000)` literal, u32 value, not sign-extended), 4 = `jt`: the target of a `j`/`jal`
/// (`((off + base + 4) & 0xf000_0000) | (imm26 << 2)`, recgen `target()`), 5 = `jt4`: `jt + 4` (the `c.npc` literal).
#[derive(Clone, Debug)]
pub struct Zv {
    pub off: u32,
    pub kind: u8,
    pub rem: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Reloc {
    pub name: String,
    pub entry: u32,
    /// Function length in bytes.
    pub len: u32,
    pub mhash: u64,
    wmask: Vec<u32>,
    wj: Vec<bool>,
    pub zv: Vec<Zv>,
}

fn hx(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(0)
}

#[inline]
fn jnorm(w: u32, base: u32) -> u32 {
    (w & 0xfc00_0000) | ((((w & 0x03ff_ffff) << 2).wrapping_sub(base) & 0x0fff_ffff) >> 2)
}

#[inline(always)]
fn rd(mem: &Mem, a: u32) -> u32 {
    u32::from_le_bytes(mem.ram[a as usize..a as usize + 4].try_into().unwrap())
}

impl Reloc {
    /// Parses the spec lines out of a text (other lines are ignored); None when the text has no `reloc` line.
    pub fn parse(txt: &str) -> Option<Reloc> {
        let mut r = Reloc::default();
        let (mut masks, mut jabs) = (vec![], vec![]);
        let mut seen = false;
        for l in txt.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w.first().copied() {
                Some("reloc") if w.len() >= 6 => {
                    r.name = w[1].into();
                    r.entry = hx(w[3]) as u32;
                    r.len = hx(w[5]) as u32;
                    seen = true;
                }
                Some("mhash") if w.len() >= 2 => r.mhash = hx(w[1]),
                Some("mask") if w.len() >= 3 => masks.push((hx(w[1]) as u32, hx(w[2]) as u32)),
                Some("jabs") if w.len() >= 2 => jabs.push(hx(w[1]) as u32),
                Some("zv") if w.len() >= 5 => {
                    let k = w[1].parse::<usize>().ok()?;
                    if k != r.zv.len() || k >= ZV_MAX {
                        return None;
                    }
                    let kind = match w[3] {
                        "simm" => 0,
                        "zimm" => 1,
                        "word" => 2,
                        "hi" => 3,
                        "jt" => 4,
                        "jt4" => 5,
                        _ => return None,
                    };
                    r.zv.push(Zv { off: hx(w[2]) as u32, kind, rem: hx(w[4]) as u32 });
                }
                _ => {}
            }
        }
        if !seen || r.len == 0 || r.len % 4 != 0 {
            return None;
        }
        r.wmask = vec![0; (r.len / 4) as usize];
        r.wj = vec![false; (r.len / 4) as usize];
        for (o, m) in masks {
            *r.wmask.get_mut((o / 4) as usize)? |= m;
        }
        for o in jabs {
            *r.wj.get_mut((o / 4) as usize)? = true;
        }
        if r.zv.iter().any(|z| z.off >= r.len) {
            return None;
        }
        Some(r)
    }

    /// FNV-1a over the code words at `base`, relocated fields masked, `j` targets made base-relative (same fold as `rec::hash`).
    pub fn masked_hash(&self, mem: &Mem, base: u32) -> u64 {
        self.masked_hash_by(base, |a| rd(mem, a))
    }

    /// `masked_hash` over any word reader (`read(addr)` = the code word at `addr`); the spec check tool reads copies from files.
    #[inline(always)]
    pub fn masked_hash_by<F: Fn(u32) -> u32>(&self, base: u32, read: F) -> u64 {
        let mut h = FNV_O;
        for (i, (&m, &j)) in self.wmask.iter().zip(&self.wj).enumerate() {
            let mut w = read(base + 4 * i as u32);
            if j {
                w = jnorm(w, base);
            }
            h = (h ^ (w & !m) as u64).wrapping_mul(FNV_P);
        }
        h
    }

    /// Does the code at `base` equal the template (relocated fields masked)?
    pub fn matches(&self, mem: &Mem, base: u32) -> bool {
        (base as u64 + self.len as u64) <= mem.ram.len() as u64 && self.masked_hash(mem, base) == self.mhash
    }

    /// The `zv_[k]` values of the copy at `base` (decoded from its code words).
    pub fn zvals(&self, mem: &Mem, base: u32) -> [u64; ZV_MAX] {
        self.zvals_by(base, |a| rd(mem, a))
    }

    /// `zvals` over any word reader (the decode is this one function; the spec check tool compares it with the `V_` tables of the rec crates).
    #[inline(always)]
    pub fn zvals_by<F: Fn(u32) -> u32>(&self, base: u32, read: F) -> [u64; ZV_MAX] {
        let mut v = [0u64; ZV_MAX];
        for (k, z) in self.zv.iter().enumerate() {
            let w = read(base + z.off);
            v[k] = match z.kind {
                0 => (w as u16 as i16 as i32 as u32) as u64,
                1 => (w & 0xffff) as u64,
                2 => ((z.rem as u64) << 32) | w as u64,
                3 => ((w & 0xffff) << 16) as u64,
                _ => {
                    let jt = (base.wrapping_add(z.off).wrapping_add(4) & 0xf000_0000) | ((w & 0x03ff_ffff) << 2);
                    (if z.kind == 4 { jt } else { jt.wrapping_add(4) }) as u64
                }
            };
        }
        v
    }
}

/// A relocatable speed hand: registered with `rec::register_reloc_hand`; `Table::build` installs it (as an EXACT hand, `est` = `hs::EXACT`)
/// at the entry of every recompiled function whose code matches `reloc()`. `f` runs with `c.pc == entry` (= base).
pub struct RelocHand {
    pub name: &'static str,
    pub reloc: fn() -> &'static Reloc,
    pub est: u64,
    pub f: fn(&mut crate::cpu::Cpu) -> bool,
}
