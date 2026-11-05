//! R&C2 tie colours as the game sends them (the R&C2 pack, rac-rs/src/rac2, recolours its ties from these): `capture(Some)`
//! starts it. The tie chain builder REFs per packet `entry[10]` RGBA8 colours from the colour pool at `u16 cull record
//! +0x14` qwords, records 0x20 bytes, one per tie instance, reset every frame (rac2 notes_tiecol.md). Each VIF1 chain
//! maps its pool blocks to instances by the records as they are then. The bases sit in a level-data block {record base,
//! record end = base + 0x20 count, ?, count, pool base} [C: Aranos 0x1b2534 = 0x18a4140, 0x18b3b40, .., 2000, 0x750000;
//! it moves per level like the other overlay data], found by that signature with `count` = the level's tie instances.
//! Hooked into `Mem::vif1_kick` (kick_start / block / kick_end); off (one atomic load per VIF1 kick) unless started.
use crate::mem::{Mem, RAM};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct TieCap {
    pub count: u32,
    /// The block's address once found.
    pub at: Option<u32>,
    /// VIF1 chains to wait before searching again after a failed search (the search reads 1 MB).
    pub wait: u32,
    /// Instance -> the colour words of its last draw (its pool blocks in chain order, padding included).
    pub got: HashMap<u32, Vec<u32>>,
}

pub static TIE_CAP: std::sync::Mutex<Option<TieCap>> = std::sync::Mutex::new(None);
static TIE_ON: AtomicBool = AtomicBool::new(false);

/// Start (Some) or stop (None) the tie colour capture.
pub fn capture(c: Option<TieCap>) {
    TIE_ON.store(c.is_some(), Ordering::Relaxed);
    if let Ok(mut t) = TIE_CAP.lock() {
        *t = c;
    }
}

/// The colours the game sent since the last call, by instance.
pub fn take() -> HashMap<u32, Vec<u32>> {
    TIE_CAP.lock().ok().and_then(|mut g| g.as_mut().map(|g| std::mem::take(&mut g.got))).unwrap_or_default()
}

/// One VIF1 chain's tie colour blocks.
pub(crate) struct TieKick {
    rec: u32,
    pool: u32,
    count: u32,
    /// (pool offset in qwords, size in qwords, instance), sorted; built at the first pool block.
    ranges: Option<Vec<(u32, u32, u32)>>,
    cur: HashMap<u32, Vec<u32>>,
}

/// A VIF1 chain starts (one that goes to the GS capture): the level's record and pool bases, when the capture is on.
pub(crate) fn kick_start(m: &Mem) -> Option<TieKick> {
    if !TIE_ON.load(Ordering::Relaxed) {
        return None;
    }
    let mut g = TIE_CAP.lock().ok();
    let t = g.as_mut().and_then(|g| g.as_mut())?;
    let ok = |a: u32| {
        let w = |o: u32| u32::from_le_bytes(m.ram[(a + o) as usize..(a + o) as usize + 4].try_into().unwrap());
        w(0xc) == t.count && w(0) != 0 && w(0) < RAM as u32 && w(4) == w(0).wrapping_add(t.count * 0x20)
    };
    if !t.at.is_some_and(ok) {
        if t.wait > 0 {
            t.wait -= 1;
            return None;
        }
        t.at = (0x10_0000..0x20_0000u32).step_by(4).find(|&a| ok(a));
        t.wait = if t.at.is_none() { 500 } else { 0 };
    }
    let a = t.at? as usize;
    let w = |o: usize| u32::from_le_bytes(m.ram[a + o..a + o + 4].try_into().unwrap()) & 0x01ff_ffff;
    Some(TieKick { rec: w(0), pool: w(0x10), count: t.count, ranges: None, cur: HashMap::new() })
}

/// A VIF1 block from the tie colour pool: append it to its instance (the record whose pool range holds it; a block at
/// the range start begins the instance anew).
pub(crate) fn block(m: &Mem, t: &mut TieKick, src: u32, q: u32) {
    if t.pool == 0 || src & 0x8000_0000 != 0 || src < t.pool || src >= t.pool + 0x80_0000 {
        return;
    }
    let o = (src - t.pool) / 16;
    let ram = &m.ram;
    let ranges = t.ranges.get_or_insert_with(|| {
        let mut v = Vec::new();
        for i in 0..t.count {
            let a = (t.rec + i * 0x20) as usize;
            if a + 0x18 > RAM {
                break;
            }
            let off = u16::from_le_bytes([ram[a + 0x14], ram[a + 0x15]]) as u32;
            let size = u16::from_le_bytes([ram[a + 0x16], ram[a + 0x17]]) as u32;
            if off != 0xffff && size != 0 {
                v.push((off, size, i));
            }
        }
        v.sort_unstable();
        v
    });
    let k = ranges.partition_point(|r| r.0 <= o);
    let Some(&(off, size, inst)) = k.checked_sub(1).and_then(|k| ranges.get(k)) else { return };
    if o >= off + size {
        return;
    }
    let e = t.cur.entry(inst).or_default();
    if o == off {
        e.clear();
    }
    m.dma_words(src, q, e);
}

/// The chain ended: its instances' colours go to `TIE_CAP.got`.
pub(crate) fn kick_end(t: TieKick) {
    if t.cur.is_empty() {
        return;
    }
    if let Ok(mut g) = TIE_CAP.lock() {
        if let Some(g) = g.as_mut() {
            g.got.extend(t.cur);
        }
    }
}
