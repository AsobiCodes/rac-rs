//! R&C2: the GIF reads an XGKICKed packet over time, not at the kick (child module of `vu1`).
//!
//! The GIF reads the packet one qword per `gif_rate()` VU cycles after the kick, so stores the program makes after XGKICK
//! to qwords the GIF has not reached yet are sent. R&C2's VU1 program 2 writes a particle's packet, kicks, then writes the
//! next particle's tag behind it; read at the kick, the GIF met stale colour data where that tag goes and drew screen-wide
//! garbage triangles [C: rac2-play]. The first qword (the tag) is read after the instruction behind the XGKICK: program 2
//! ends with `XGKICK vi13 [E]` and sets the last particle's EOP in the delay slot (`ISW.x vi02, 0(vi01)`, vi02 = 0x8001;
//! rac2/decomp/vu1/prog2.s 01f0). Read one cycle after the kick, a one-particle packet had no EOP yet and the GIF read on
//! into stale memory; on Aranos (EE ~2067) that stale data held an A+D TRXPOS (1508, 2047), and the game, which sets TRXPOS
//! once, sent every later texture upload there [C: rac2-play]. A new XGKICK or the end of the run reads the rest at once.
//! R&C1: rate 0 (the whole packet at the XGKICK, as before).
use super::{Path1, Vu1};

/// An XGKICK in flight.
pub(super) struct Kick {
    addr: usize,
    /// Next qword address, qwords left in the current GIF tag's data, whether that tag had EOP, read time of the next qword.
    a: usize,
    left: usize,
    eop: bool,
    t: u64,
    got: Vec<(usize, [u32; 4])>,
}

/// VU1 cycles per qword the GIF reads from a kicked packet: 2 for R&C2 (env RAC2_GIFRATE), 0 = the whole packet at the
/// XGKICK (R&C1).
fn gif_rate() -> u64 {
    static R: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *R.get_or_init(|| if ee_oracle::ver::rc2() { std::env::var("RAC2_GIFRATE").ok().and_then(|v| v.parse().ok()).unwrap_or(2) } else { 0 })
}

impl Vu1 {
    /// Starts an XGKICK at `addr` (VU cycle `cyc`); the one still in flight is read to its end first. The GIF reads the tag
    /// after the next instruction pair.
    pub(super) fn kick_start(&mut self, addr: usize, cyc: u64, sink: &mut dyn Path1) {
        self.kick_finish(sink);
        if gif_rate() == 0 {
            sink.xgkick(&self.data, addr);
            return;
        }
        // (the pair at `cyc` holds the XGKICK; the next pair, its delay slot, runs before the GIF reads the tag)
        self.kick = Some(Kick { addr, a: addr, left: 0, eop: false, t: cyc + 2, got: Vec::new() });
    }

    /// The GIF reads the in-flight packet's qwords due by VU cycle `cyc`.
    pub(super) fn kick_step(&mut self, cyc: u64, sink: &mut dyn Path1) {
        let Some(k) = self.kick.as_mut() else { return };
        let mut done = false;
        while k.t <= cyc {
            let q = self.data[k.a];
            k.got.push((k.a, q));
            if k.left == 0 {
                // a tag (the packet reader in `gif_packet` / vif `Out::xgkick` gives the same lengths)
                let nloop = (q[0] & 0x7fff) as usize;
                let nreg = match ((q[1] >> 28) & 15) as usize {
                    0 => 16,
                    n => n,
                };
                k.left = match (q[1] >> 26) & 3 {
                    0 => nloop * nreg,
                    1 => (nloop * nreg).div_ceil(2),
                    _ => nloop,
                };
                k.eop = (q[0] >> 15) & 1 != 0;
                if k.left + 1 > 1024 {
                    done = true;
                    break;
                }
            } else {
                k.left -= 1;
            }
            k.a = (k.a + 1) & 0x3ff;
            k.t += gif_rate();
            if k.left == 0 && k.eop {
                done = true;
                break;
            }
        }
        if done {
            self.kick_finish(sink);
        }
    }

    /// Sends the in-flight packet: the qwords the GIF read so far as they were then, the rest as memory is now.
    pub(super) fn kick_finish(&mut self, sink: &mut dyn Path1) {
        let Some(k) = self.kick.take() else { return };
        if k.got.iter().all(|&(a, q)| self.data[a] == q) {
            sink.xgkick(&self.data, k.addr);
            return;
        }
        let mut img = self.data.clone();
        for &(a, q) in &k.got {
            img[a] = q;
        }
        sink.xgkick(&img, k.addr);
    }
}
