//! High-level kernel: syscalls return at once (no threads, no IOP).

use crate::cpu::Cpu;

const VBCOUNT: u32 = 0x15ed48;
/// IOP server struct of the level data cache module (sid 0x11).
const IOP_CACHE: u32 = 0x1ef188;

impl Cpu {
    pub fn syscall(&mut self, pc: u32) {
        if self.cold.is_some() && crate::cold::syscall_hook(self, pc) {
            return;
        }
        let n = self.g(3) as u32 as i32;
        *self.syscalls.entry(n).or_insert(0) += 1;
        // -5: return from a user INTC/DMAC handler to the kernel dispatcher
        // (the dispatcher itself runs from the BIOS image in RAM). 0x10..0x17 (Add/Remove Intc/Dmac handler,
        // Enable/Disable Intc/Dmac) run in that kernel too: its dispatcher reads the handler lists they edit
        // (nodes {next, prev, fn, gp, arg, state} at 0x19xxx), so a level's overlay handlers come and go.
        if n == -5 || (0x10..=0x17).contains(&n.unsigned_abs()) {
            self.cp0[14] = pc;
            self.cp0[13] = (self.cp0[13] & !0x8000_007c) | 8 << 2;
            self.cp0[12] |= 2;
            self.pc = 0x8000_0180;
            self.npc = 0x8000_0184;
            return;
        }
        let a0 = self.g(4) as u32;
        let a1 = self.g(5) as u32;
        let v: i64 = match n.unsigned_abs() {
            // SetGsCrt(interlace, omode, ffmd), from sceGsResetGraph 0x121878: at boot and at every switch of the PAL disc's
            // 50/60 Hz option (PAL flag 0x15ed80 set from 0x16030c in FUN_001e9558 / the title loop FUN_001eb698). omode 2 =
            // NTSC, 3 = PAL: the field clock (vblank period, T1 counts, sound ticks, rac-rs pacing) follows the mode.
            0x02 => {
                if a1 == 2 || a1 == 3 {
                    let ntsc = a1 == 2;
                    if ntsc != crate::ver::ntsc() {
                        self.snd.video_mode(ntsc);
                        crate::ver::set_ntsc(ntsc);
                        eprintln!("oracle: video mode {} (SetGsCrt omode {a1}, icount {})", if ntsc { "NTSC 59.94 Hz" } else { "PAL 50 Hz" }, self.icount);
                    }
                }
                0
            }
            0x71 => 0,
            0x70 => 0x7f00,
            0x20 => {
                self.sema_next += 1;
                self.sema_next as i64
            }
            0x2f => 1,
            0x40 => {
                self.sema_next += 1;
                self.sema_next as i64
            }
            0x42..=0x46 => a0 as i64,
            // FlushCache / iFlushCache: the game runs code it just copied (a level's overlay at loader state 7).
            0x64 | 0x68 => {
                self.rec_flush = true;
                0
            }
            // SetVSyncFlag(int *flag, u64 *csr): written at the next vblank.
            0x73 => {
                self.vsync = Some((a0, a1));
                self.vsync_armed = true;
                if std::env::var("ORACLE_DBG").is_ok() { eprintln!("SetVSyncFlag {a0:08x} {a1:08x} ra {:08x} icount {}", self.g(31) as u32, self.icount); }
                0
            }
            // Deci2Call: no debugger attached. ReqSend fails, so the TTY
            // writer (0x1197xx) does not wait for a send-done callback.
            0x7c => {
                if a0 == 3 && std::env::var("ORACLE_TTY").is_ok() {
                    let mut t = Vec::new();
                    for k in 0..0x100 {
                        let c = self.mem.rd8(0x154a8c + k);
                        if c == 0 {
                            break;
                        }
                        t.push(c);
                    }
                    eprint!("TTY: {}", String::from_utf8_lossy(&t));
                }
                -1
            }
            0x76 => -1,
            // sceSifSetDma(transfers, n): EE -> IOP RAM copies ({src, dest, size, attr} each).
            0x77 => {
                for k in 0..a1 {
                    let t = a0 + 16 * k;
                    let (src, dst, n) = (self.mem.rd32(t), self.mem.rd32(t + 4) as usize & 0x1f_ffff, self.mem.rd32(t + 8) as usize);
                    if self.iop.len() < dst + n {
                        self.iop.resize((dst + n).max(2 << 20), 0);
                    }
                    for j in 0..n {
                        self.iop[dst + j] = self.mem.rd8(src + j as u32);
                    }
                }
                1
            }
            0x7f => 32 << 20,
            _ => 0,
        };
        self.r[2] = (self.r[2] & !(u64::MAX as u128)) | (v as u64) as u128;
    }

    /// sceSifBindRpc(client, sid, mode): bound at once to the IOP server the retail modules register for `sid`
    /// (server struct addresses from the s07 IOP RAM; others get a made-up nonzero address).
    pub fn sif_bind_rpc(&mut self) {
        let (client, sid) = (self.g(4) as u32, self.g(5) as u32);
        let server = match sid {
            0x11 => IOP_CACHE,
            0x123456 => 0x1f0190,
            0x123457 => 0x1ef990,
            0x8000_0400 => crate::mc::MCSERV,
            0x8000_0593 => 0x410f0,
            0x8000_0595 => 0x41060,
            _ => 0x1e_0000 | (sid & 0xfff) << 4,
        };
        if std::env::var("ORACLE_RPC").is_ok() {
            eprintln!("bind client {client:x} sid {sid:x} -> server {server:x}");
        }
        self.mem.wr32(client + 0x24, server);
    }

    /// sceSifCallRpc(client, fno, mode, send, ssize, recv, rsize, end) with
    /// the IOP not emulated: write the reply the sound driver would send.
    /// The sound queue (boot 0x12e9d8) frames its reply with -1 words that
    /// FUN_0012de70 polls; each command gets one result word in between.
    /// Retail replies (s03..s06): 17 (play) returns a handle 0x85SSNNNN,
    /// 33 (update) echoes the handle while the sound plays and 0 once it has
    /// ended, 25 echoes its first word, 50 returns 0, 9 has no callback.
    /// Read `n` 2048-byte sectors at `lba` of the disc image into EE memory at `buf`.
    fn iso_read(&mut self, lba: u32, n: u32, buf: u32) -> bool {
        let mut data = vec![0u8; n as usize * 2048];
        let ok = self.iso.as_mut().is_some_and(|f| {
            use std::io::{Read, Seek, SeekFrom};
            f.seek(SeekFrom::Start(lba as u64 * 2048)).is_ok() && f.read_exact(&mut data).is_ok()
        });
        if ok {
            for (k, b) in data.iter().enumerate() {
                self.mem.wr8(buf.wrapping_add(k as u32), *b);
            }
        }
        ok
    }

    pub fn sif_call_rpc(&mut self) {
        let (client, fno, send, recv) = (self.g(4) as u32, self.g(5) as u32, self.g(7) as u32, self.g(9) as u32);
        let (ssize, rsize) = (self.g(8) as u32, self.g(10) as u32);
        // The end function (eighth argument) and its argument (the ninth, on the stack).
        let (end, sp) = (self.g(11) as u32, self.g(29) as u32);
        let end_param = self.mem.rd32(sp);
        self.rpc_reply(client, fno, send, ssize, recv, rsize, end, end_param);
    }

    /// RPC calls in flight in the loaded state: the game called sceSifCallRpc on the real IOP and its reply had not
    /// come back when the savestate was taken. The IOP is not emulated, so nothing would ever complete them (the game
    /// then polls its reply forever: 989snd's FUN_0012de70 behind FUN_0012e548). Finds them as retail
    /// sceSifCheckStatRpc (boot 0x11b3b8) sees them: client {pkt_addr, rpc_id, ..}, packet +0x10 rec_id bit 0 set and
    /// +0x18 rpc_id equal to the client's. The call packet keeps the call ({+0x14 own address, +0x1c client,
    /// +0x20 fno, +0x24 send size, +0x28 recv, +0x2c recv size}). Each one gets the reply the HLE gives that call
    /// (rpc_reply), then what the EE side does when the IOP's end command arrives (boot FUN_0011ade0 for an
    /// RPC_CALL, 0x8000000a): the client's end function (none in the states seen; refused otherwise), the packet
    /// freed (FUN_0011ad90: rpc_id 0, rec_id bit 0 cleared) and the client's pkt_addr cleared.
    /// A libmc call (server mcserv) completes the same way through mc.rs: its send data is still in libmc's send
    /// buffer (`mc::send_buffer`; libmc's semaphore 0x132dac stays taken until sceMcSync sees the reply, so no other
    /// card call can have rewritten it), and its end function / argument are the client's +0x1c / +0x20 (what boot
    /// FUN_0011ade0 calls), which mc_rpc runs as it does for a call made after the load.
    /// A blocking call (client +8, the semaphore of sceSifCallRpc mode 0, is a semaphore id >= 0; a no-wait call keeps
    /// -1 there, libmc's included) has a thread waiting in WaitSema for the IOP's end command, which nothing here
    /// signals: the run stops ('thread blocked in RPC').
    /// Runs at load for machines that run (`load`); a state only compared against (`load_raw`) keeps retail's RAM.
    pub fn finish_rpcs_in_flight(&mut self) {
        let rd = |m: &[u8], a: u32| u32::from_le_bytes(m[a as usize..a as usize + 4].try_into().unwrap());
        let mut calls = Vec::new();
        let top = self.mem.ram.len() as u32;
        let mut a = 0x10_0000u32;
        while a + 0x40 <= top {
            let r = &self.mem.ram;
            if rd(r, a + 0x14) & 0x1fff_ffff == a && rd(r, a + 0x10) & 1 != 0 && rd(r, a + 8) == 0x8000_000a {
                let c = rd(r, a + 0x1c) & 0x01ff_ffff;
                if c != 0 && c + 0x28 <= top && rd(r, c) & 0x1fff_ffff == a && rd(r, c + 4) == rd(r, a + 0x18) {
                    calls.push((a, c));
                }
            }
            a += 0x40;
        }
        for (pkt, client) in calls {
            let r = &self.mem.ram;
            let (fno, ssize, recv, rsize) = (rd(r, pkt + 0x20), rd(r, pkt + 0x24), rd(r, pkt + 0x28), rd(r, pkt + 0x2c));
            let sema = self.mem.rd32(client + 8) as i32;
            if sema >= 0 {
                self.stop = Some(format!("RPC in flight at load: thread blocked in RPC (client {client:x} fno {fno:#x} semaphore {sema:#x})"));
                return;
            }
            if self.mem.rd32(client + 0x24) == crate::mc::MCSERV {
                let Some(send) = crate::mc::send_buffer(ssize) else {
                    self.stop = Some(format!("memory card RPC in flight at load: client {client:x} fno {fno:#x} send size {ssize:#x} (no libmc send buffer of that size)"));
                    return;
                };
                let (end, end_param) = (self.mem.rd32(client + 0x1c), self.mem.rd32(client + 0x20));
                if std::env::var("ORACLE_RPC").is_ok() {
                    eprintln!("in flight at load: client {client:x} fno {fno:#x} send {send:x} recv {recv:x} end {end:x} ({end_param:x}) (mcserv)");
                }
                self.rpc_reply(client, fno, send, ssize, recv, rsize, end, end_param);
                if self.stop.is_some() {
                    return;
                }
                self.mem.wr32(pkt + 0x18, 0);
                let id = self.mem.rd32(pkt + 0x10);
                self.mem.wr32(pkt + 0x10, id & !1);
                self.mem.wr32(client, 0);
                continue;
            }
            if ssize != 0 || self.mem.rd32(client + 0x1c) != 0 {
                // The send data is only in the IOP server's buffer now, and an end function would have to run as
                // EE code here: neither occurs in the states seen.
                self.stop = Some(format!("RPC in flight at load: client {client:x} fno {fno:#x} send size {ssize:#x} end fn {:x}", self.mem.rd32(client + 0x1c)));
                return;
            }
            if std::env::var("ORACLE_RPC").is_ok() {
                eprintln!("in flight at load: client {client:x} fno {fno:#x} recv {recv:x} rsize {rsize:#x}");
            }
            self.rpc_reply(client, fno, 0, 0, recv, rsize, 0, 0);
            self.mem.wr32(pkt + 0x18, 0);
            let id = self.mem.rd32(pkt + 0x10);
            self.mem.wr32(pkt + 0x10, id & !1);
            self.mem.wr32(client, 0);
        }
    }

    /// The reply the IOP side gives sceSifCallRpc(client, fno, mode, send, ssize, recv, rsize, end, end_param), written
    /// at once (`end` / `end_param`: the call's end function and its argument; only the memory card server uses them).
    #[allow(clippy::too_many_arguments)]
    fn rpc_reply(&mut self, client: u32, fno: u32, send: u32, ssize: u32, recv: u32, rsize: u32, end: u32, end_param: u32) {
        if std::env::var("ORACLE_RPC").is_ok() {
            eprintln!("rpc client {client:x} fno {fno} send {send:x} ssize {ssize:x} recv {recv:x} rsize {rsize:x} w0 {:08x}", self.mem.rd32(send));
        }
        let server = self.mem.rd32(client + 0x24);
        // Memory card server (mc.rs).
        if server == crate::mc::MCSERV {
            self.mc_rpc(fno, send, recv, end, end_param);
            return;
        }
        // IOP data cache (sid 0x11, Eudora FUN_002845d8): fno 1 copies rsize bytes from IOP address send[0] to
        // recv; fno 2 returns its IOP buffer {base, size} (as on Eudora s07).
        if server == IOP_CACHE && fno == 2 {
            self.mem.wr32(recv, 0xde4d0);
            self.mem.wr32(recv + 4, 0x94000);
            self.mem.wr32(recv + 8, 0);
            self.mem.wr32(recv + 12, 0);
            return;
        }
        if server == IOP_CACHE && fno == 1 {
            let (src, n) = (self.mem.rd32(send) as usize & 0x1f_ffff, rsize as usize);
            if src + n <= self.iop.len() {
                for k in 0..n {
                    self.mem.wr8(recv + k as u32, self.iop[src + k]);
                }
            } else {
                self.stop = Some(format!("IOP read {src:x}+{n:x} outside IOP RAM ({} bytes loaded)", self.iop.len()));
            }
            return;
        }
        // libcdvd N-command client (boot 0x121450 sceCdRead, 0x120b98 sceCdGetDiskType) and S-command client
        // (0x121630 sceCdGetError): the read completes at once, then the end callback 0x120788 -> 0x1206d8 runs
        // (result 0x131318 = command 0x131314, busy flag 0x1312f0 and command cleared; no user callback set).
        if client == 0x132490 && fno == 1 {
            let (lba, n, buf) = (self.mem.rd32(send), self.mem.rd32(send + 4), self.mem.rd32(send + 8));
            let mode = self.mem.rd8(send + 0xe);
            if std::env::var("ORACLE_RPC").is_ok() {
                eprintln!("sceCdRead lba {lba:#x} sectors {n} buf {buf:#x} mode {mode} ra {:08x}", self.g(31) as u32);
            }
            if mode != 0 || !self.iso_read(lba, n, buf) {
                self.stop = Some(format!("sceCdRead lba {lba:#x} sectors {n} mode {mode}: no disc image (ORACLE_ISO)"));
                return;
            }
            let cmd = self.mem.rd32(0x131314);
            self.mem.wr32(0x131318, cmd);
            self.mem.wr32(0x1312f0, 0);
            self.mem.wr32(0x131314, 0);
            return;
        }
        if client == 0x132490 && fno == 14 {
            self.mem.wr32(recv, 0x14);
            return;
        }
        // 989snd snd_BankLoadByLoc (boot 0x12df20 fno 3, async 0x12e088 fno 0x57): the reply is the bank's IOP
        // address, which the game only passes back in play commands (rac-rs plays from the level's own bank).
        if client == 0x15ebe8 && (fno == 3 || fno == 0x57) {
            let vb = self.mem.rd32(VBCOUNT);
            let arg = self.mem.rd32(send);
            let inner = self.snd.ensure(&self.iop, &self.mem.ram, self.cold.is_some(), vb);
            inner.advance_to(inner.vb);
            let h = inner.load_bank(fno, arg, &self.mem.ram, &mut self.iso);
            self.mem.wr32(recv, h);
            return;
        }
        if client == 0x132d08 && fno == 4 {
            self.mem.wr32(recv, 0);
            return;
        }
        // The CDVD S-command client's clock read (boot 0x121780 sceCdReadClock): the IOP answers status 0 and the
        // 8-byte BCD time at recv + 4 (sceCdCLOCK: sec, min, hour, pad, day, month, year), which the wrapper copies
        // into the caller's sceCdCLOCK.
        if client == 0x132d08 && fno == 1 && recv != 0 {
            self.mem.wr32(recv, 0);
            if rsize >= 0xc {
                for (k, b) in crate::clock::local_rtc().iter().enumerate() {
                    self.mem.wr8(recv + 4 + k as u32, *b);
                }
                self.mem.wr32(recv + 0xc, 0);
            }
            return;
        }
        // 989snd direct calls on the command client: the sync wrapper (boot ~0x12dfd0, used by FUN_0012da28) and
        // FUN_0012e6e0 for a command without arguments while nothing is batched send ONE command with fno = its cid and
        // the send buffer = its argument words (none: send 0); the caller reads the result at recv + 4 (DAT_00133104).
        // fno 0x4d is the batch below. fno 0 = the module init 0x6918 and fno 42 = stream init {handlers, buffer bytes}
        // (boot FUN_0022c3e0; a savestate's driver had both applied before the save, `Inner::boot`).
        if client == 0x15ebc0 && fno != 0x4d {
            let vb = self.mem.rd32(VBCOUNT);
            let words = (ssize / 4).min(7);
            let d: [u32; 7] = std::array::from_fn(|j| if send != 0 && (j as u32) < words { self.mem.rd32(send + 4 * j as u32) } else { 0 });
            let res = {
                let inner = self.snd.ensure(&self.iop, &self.mem.ram, self.cold.is_some(), vb);
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
                eprintln!("sndtrace vb {vb} direct cid {fno} {:08x} {:08x} {:08x} -> {res:08x}", d[0], d[1], d[2]);
            }
            if recv != 0 && rsize >= 8 {
                let n = rsize / 4;
                for k in 1..n - 1 {
                    self.mem.wr32(recv + 4 * k, 0);
                }
                self.mem.wr32(recv, u32::MAX);
                self.mem.wr32(recv + 4, res);
                self.mem.wr32(recv + 4 * (n - 1), u32::MAX);
            }
            return;
        }
        if recv == 0 || rsize < 8 {
            return;
        }
        let n = rsize / 4;
        for k in 1..n - 1 {
            self.mem.wr32(recv + 4 * k, 0);
        }
        let dbg = std::env::var("ORACLE_SND").is_ok();
        let trace = std::env::var("ORACLE_SNDTRACE").is_ok();
        let vb = self.mem.rd32(VBCOUNT);
        if send != 0 {
            let count = self.mem.rd32(send).min(n - 2);
            let cbt = if send == self.mem.rd32(0x15eca0) { self.mem.rd32(0x15ecb0) } else { self.mem.rd32(0x15ecb4) };
            let mut p = send + 4;
            for k in 0..count {
                let cid = self.mem.rd16(p) as i16;
                let len = self.mem.rd16(p + 2) as i16 as i32;
                let d: [u32; 7] = std::array::from_fn(|j| self.mem.rd32(p + 4 + 4 * j as u32));
                let (cf, ca) = (self.mem.rd32(cbt + 16 * k), self.mem.rd32(cbt + 16 * k + 8));
                if std::env::var("ORACLE_SNDRAW").is_ok() && (cid == 17 || cid == 33) {
                    eprintln!("sndraw vb {vb} cid {cid} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} arg {ca:x}", d[0], d[1], d[2], d[3], d[4], d[5]);
                }
                let snd_res = {
                    let inner = self.snd.ensure(&self.iop, &self.mem.ram, self.cold.is_some(), vb);
                    // the driver runs on its own field clock (stage 10a); ORACLE_SNDCLOCK=1 reports where it and the game's
                    // VBCOUNT do not advance together (debug)
                    if std::env::var("ORACLE_SNDCLOCK").is_ok() && inner.vb.wrapping_sub(vb) != inner.tick_off_vb {
                        eprintln!("sndclock vb {vb} clock {} cid {cid}", inner.vb);
                        inner.tick_off_vb = inner.vb.wrapping_sub(vb);
                    }
                    let now = inner.vb;
                    inner.command(now, cid, &d, &mut self.iso)
                };
                let res = if let Some(r) = snd_res {
                    r
                } else {
                    match cid {
                    // Stream-channel disc read (boot 0x12ed58 with 0x15ec8c set): {lba, sectors, buf}; the driver
                    // clears the busy word 0x137b00 and leaves error 0x137b10 = 0 when it is done (FUN_0012ee08).
                    56 => {
                        if self.iso_read(d[0], d[1], d[2]) {
                            self.mem.wr32(0x137b00, 0);
                            self.mem.wr32(0x137b10, 0);
                        } else {
                            self.stop = Some(format!("stream read lba {:#x} sectors {}: no disc image (ORACLE_ISO)", d[0], d[1]));
                        }
                        if std::env::var("ORACLE_RPC").is_ok() {
                            eprintln!("stream read lba {:#x} sectors {} buf {:#x}", d[0], d[1], d[2]);
                        }
                        u32::MAX
                    }
                    _ => {
                        if dbg {
                            eprintln!("snd vb {vb} cmd {cid} len {len} {:08x} {:08x} cb {cf:x} arg {ca:x}", d[0], d[1]);
                        }
                        u32::MAX
                    }
                    }
                };
                self.mem.wr32(recv + 4 + 4 * k, res);
                if trace {
                    eprintln!("sndtrace vb {vb} cid {cid} {:08x} {:08x} {:08x} -> {res:08x}", d[0], d[1], d[2]);
                }
                p += 4 + ((len.max(0) as u32 + 3) & !3);
            }
        }
        self.mem.wr32(recv, u32::MAX);
        self.mem.wr32(recv + 4 * (n - 1), u32::MAX);
    }
}
