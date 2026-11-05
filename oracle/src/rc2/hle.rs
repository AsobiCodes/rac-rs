//! R&C2's IOP services (the IOP is not emulated; each reply is what the game reads back), reached from hle.rs `rpc_answer` (`rpc_rc2`)
//! while `ver::rc2()`: libcdvd reads, the IOP data cache, 989snd's command server (results from R&C1's snd989 driver model)
//! and bank loader, with the CD channel read finished at the next field tick. Unknown calls get a zero-filled reply,
//! reported once on stderr (`note_call`; RAC2_RPC_QUIET=1 keeps it silent). Facts: rac2/NOTES.md "RPC servers".
use crate::cpu::Cpu;

/// RPC client struct -> the sid it was bound to (sceSifBindRpc).
static SIDS: std::sync::Mutex<std::collections::BTreeMap<u32, u32>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
/// (sid, fno) pairs already reported once on stderr (which IOP services the game asks for).
static SEEN: std::sync::Mutex<std::collections::BTreeSet<(u32, u32)>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// sceSifBindRpc bound `client` to `sid` (hle.rs `sif_bind_rpc`).
pub fn note_bind(client: u32, sid: u32) {
    SIDS.lock().unwrap().insert(client, sid);
}

impl Cpu {
    /// The sid of `client`, and a one-time stderr line for each new (sid, fno) pair.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn note_call(&mut self, client: u32, fno: u32, send: u32, ssize: u32, recv: u32, rsize: u32, end: u32) -> u32 {
        let sid = SIDS.lock().unwrap().get(&client).copied().unwrap_or(u32::MAX);
        if SEEN.lock().unwrap().insert((sid, fno)) && std::env::var("RAC2_RPC_QUIET").is_err() {
            eprintln!(
                "rpc first: sid {sid:#x} fno {fno:#x} client {client:x} send {send:x} ssize {ssize:#x} recv {recv:x} rsize {rsize:#x} end {end:x} w0 {:08x} ra {:08x} icount {}",
                if send != 0 { self.mem.rd32(send) } else { 0 },
                self.g(31) as u32,
                self.icount
            );
        }
        sid
    }

    /// 989snd's IOP CD channel finishes the read issued in the last field: busy and status words cleared (the IOP
    /// writes them back over SIF on real hardware, some time after the request). Nothing pending on R&C1.
    #[inline]
    pub fn snd_cd_finish(&mut self) {
        if std::mem::take(&mut self.snd_cd_pending) {
            self.mem.wr32(super::SND_CD_BUSY, 0);
            self.mem.wr32(super::SND_CD_BUSY + 0x10, 0);
        }
    }

    /// An R&C2 RPC call's reply (hle.rs `rpc_answer`): the memory card server (mc.rs, then the save file), else R&C2's servers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rpc_rc2(&mut self, client: u32, fno: u32, send: u32, ssize: u32, recv: u32, rsize: u32, end: u32, end_param: u32) {
        let sid = self.note_call(client, fno, send, ssize, recv, rsize, end);
        if self.mem.rd32(client + 0x24) == crate::mc::MCSERV {
            self.mc_rpc(fno, send, recv, end, end_param);
            crate::mc::persist_after(self, fno);
            return;
        }
        self.rpc_reply_rc2(client, sid, fno, send, ssize, recv, rsize, end, end_param);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rpc_reply_rc2(&mut self, client: u32, sid: u32, fno: u32, send: u32, ssize: u32, recv: u32, rsize: u32, end: u32, end_param: u32) {
        let _ = (sid, ssize, end, end_param);
        use super::*;
        // libcdvd N-command client, fno 1 = sceCdRead {lba, sectors, buf, mode..}: the read completes at once and the
        // end callback's effects are applied (result = command, busy flag and command cleared), as R&C1's hle.rs did.
        if client == CD_NCMD_CLIENT && fno == 1 {
            let (lba, n, buf) = (self.mem.rd32(send), self.mem.rd32(send + 4), self.mem.rd32(send + 8));
            if std::env::var("ORACLE_RPC").is_ok() || std::env::var("RAC2_CDLOG").is_ok() {
                eprintln!("sceCdRead lba {lba:#x} sectors {n} buf {buf:#x} ra {:08x} icount {}", self.g(31) as u32, self.icount);
            }
            if !self.iso_read(lba, n, buf & 0x01ff_ffff) {
                self.stop = Some(format!("sceCdRead lba {lba:#x} sectors {n}: outside the disc image (ORACLE_ISO)"));
                return;
            }
            let cmd = self.mem.rd32(CD_CMD);
            self.mem.wr32(CD_RESULT, cmd);
            self.mem.wr32(CD_BUSY, 0);
            self.mem.wr32(CD_CMD, 0);
            return;
        }
        if client == CD_NCMD_CLIENT && fno == 14 {
            self.mem.wr32(recv, 0x14);
            return;
        }
        // IOP data cache (sid 0x11, client 0x1a72c8; R&C1 had it in its overlays): the EE parks blobs in IOP RAM with
        // sceSifSetDma (0x2efca8, table 0x215080 {iop addr, qwords, tag}) and copies them back with fno 1 {iop addr} ->
        // recv, rsize bytes (0x2efe20). fno 2 = the module's buffer {base, size, ..} (0x2efXXX init); opaque to the EE
        // beyond the room it gives, so any consistent region of the 2 MB IOP RAM works (IOP_CACHE_BASE/SIZE).
        if sid == IOP_CACHE_SID && fno == 2 && recv != 0 {
            for k in 0..rsize / 4 {
                self.mem.wr32(recv + 4 * k, 0);
            }
            self.mem.wr32(recv, IOP_CACHE_BASE);
            self.mem.wr32(recv + 4, IOP_CACHE_SIZE);
            return;
        }
        if sid == IOP_CACHE_SID && fno == 1 {
            let (src, n) = (self.mem.rd32(send) as usize & 0x1f_ffff, rsize as usize);
            if src + n > self.iop.len() {
                self.iop.resize((src + n).max(2 << 20), 0);
            }
            for k in 0..n {
                self.mem.wr8(recv.wrapping_add(k as u32), self.iop[src + k]);
            }
            return;
        }
        // 989snd (sid 0x123456, client 0x1a70c0): the EE side (0x132280) polls its reply for -1 at word 0 and at word
        // count + 1 (count = the batched commands, 0x1a7504). Every word -1 frames any count; -1 is also what R&C1's
        // HLE answered for commands without a result. No sound is produced (groundwork: the driver is not emulated).
        // 989snd bank loads (R&C1's path above, client 0x15ebe8): without them every play command names handle 0 and no
        // sound effect plays (only the streams: music, voices).
        if sid == SND_LOAD_SID && (fno == 3 || fno == 0x57) && send != 0 && recv != 0 {
            let arg = self.mem.rd32(send);
            let inner = self.snd.ensure(&self.iop, &self.mem.ram, true, 0);
            inner.advance_to(inner.vb);
            let h = inner.load_bank(fno, arg, &self.mem.ram, &mut self.iso);
            if std::env::var("ORACLE_SNDTRACE").is_ok() {
                eprintln!("sndtrace bank load fno {fno:#x} arg {arg:#x} -> {h:#x}{}", inner.notes.last().map_or(String::new(), |n| format!(" ({n})")));
            }
            for k in 0..rsize / 4 {
                self.mem.wr32(recv + 4 * k, 0);
            }
            self.mem.wr32(recv, h);
            return;
        }
        if sid == SND_SID {
            self.snd_rpc_rc2(fno, send, ssize, recv, rsize);
            return;
        }
        if recv != 0 {
            for k in 0..rsize / 4 {
                self.mem.wr32(recv + 4 * k, 0);
            }
        }
    }

    /// 989snd (sid 0x123456).
    fn snd_rpc_rc2(&mut self, fno: u32, send: u32, ssize: u32, recv: u32, rsize: u32) {
        // 989snd's command server, the same module and protocol as R&C1 (`rpc_reply`, client 0x15ebc0): direct calls
        // (fno = cid, argument words in the send buffer, result at recv + 4; fno 0 module init, 42 stream init) and
        // the batch fno 0x4d (built by 0x132e50): {count, then per command {i16 cid, i16 len, data padded to 4}}, one
        // result word per command at recv + 4 + 4k, read by the command's callback (0x132088). The results come from
        // R&C1's 989snd driver model (snd989): stream handles, playing / buffered state. cid 56 is the disc read of
        // the IOP CD channel (0x133518 with 0x1a750c set): {lba, sectors, buf}; done at the next field tick.
        let n = rsize / 4;
        let words = |me: &mut Self, at: u32, len: u32| -> [u32; 7] { std::array::from_fn(|j| if (j as u32) < len { me.mem.rd32(at + 4 * j as u32) } else { 0 }) };
        if fno != 0x4d {
            let d = if send != 0 { words(self, send, (ssize / 4).min(7)) } else { [0; 7] };
            let res = {
                let inner = self.snd.ensure(&self.iop, &self.mem.ram, true, 0);
                let now = inner.vb;
                inner.advance_to(now);
                match fno {
                    0 => {
                        inner.module_init();
                        u32::MAX
                    }
                    42 => {
                        inner.stream_init(d[0] as usize, d[1]);
                        u32::MAX
                    }
                    _ => inner.command(now, fno as i16, &d, &mut self.iso).unwrap_or(u32::MAX),
                }
            };
            if std::env::var("ORACLE_SNDTRACE").is_ok() {
                eprintln!("sndtrace direct cid {fno} {:08x} {:08x} {:08x} -> {res:08x}", d[0], d[1], d[2]);
            }
            if recv != 0 && n >= 2 {
                for k in 1..n - 1 {
                    self.mem.wr32(recv + 4 * k, 0);
                }
                self.mem.wr32(recv, u32::MAX);
                self.mem.wr32(recv + 4, res);
                self.mem.wr32(recv + 4 * (n - 1), u32::MAX);
            }
            return;
        }
        if recv == 0 || n < 2 {
            return;
        }
        for k in 1..n - 1 {
            self.mem.wr32(recv + 4 * k, 0);
        }
        if send != 0 {
            let count = self.mem.rd32(send).min(n - 2);
            let mut p = send + 4;
            for k in 0..count {
                let cid = self.mem.rd16(p) as i16;
                let len = self.mem.rd16(p + 2) as i16 as i32;
                let d = words(self, p + 4, 7);
                let res = if cid == 56 {
                    let (lba, sectors, buf) = (d[0], d[1], d[2]);
                    if std::env::var("RAC2_CDLOG").is_ok() {
                        eprintln!("stream read lba {lba:#x} sectors {sectors} buf {buf:#x} icount {}", self.icount);
                        if std::env::var("RAC2_CDSTACK").is_ok() {
                            let sp = self.g(29) as u32;
                            let words: Vec<String> = (0..96u32)
                                .map(|i| self.mem.rd32(sp + i * 4))
                                .filter(|v| (0x100000..0x450000).contains(v) && v % 4 == 0)
                                .map(|v| format!("{v:x}"))
                                .collect();
                            eprintln!("  ra {:x} sp {sp:x} stack code words: {}", self.g(31) as u32, words.join(" "));
                        }
                    }
                    if !self.iso_read(lba, sectors, buf & 0x01ff_ffff) {
                        self.stop = Some(format!("stream read lba {lba:#x} sectors {sectors}: outside the disc image"));
                        return;
                    }
                    // Done at the next field tick (`snd_cd_finish`), not inside this call: the game marks the request
                    // pending (0x1a642c = 1) after the issue call returns, so a completion inside it is lost and the
                    // level loader (0x3308a0) waits forever.
                    self.snd_cd_pending = true;
                    u32::MAX
                } else {
                    let inner = self.snd.ensure(&self.iop, &self.mem.ram, true, 0);
                    let now = inner.vb;
                    inner.command(now, cid, &d, &mut self.iso).unwrap_or(u32::MAX)
                };
                if std::env::var("ORACLE_SNDTRACE").is_ok() {
                    eprintln!("sndtrace cid {cid} {:08x} {:08x} {:08x} -> {res:08x}", d[0], d[1], d[2]);
                }
                self.mem.wr32(recv + 4 + 4 * k, res);
                p += 4 + ((len.max(0) as u32 + 3) & !3);
            }
        }
        self.mem.wr32(recv, u32::MAX);
        self.mem.wr32(recv + 4 * (n - 1), u32::MAX);
    }
}
