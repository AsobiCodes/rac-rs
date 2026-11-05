//! R&C2 (PAL SCES_516.07, disc "RATCHETANDCLANK2", VER 1.00): the game profile of the oracle. Every R&C2 boot-ELF
//! address the cold boot and the IOP replies need lives here; the R&C1 constants they replace are named next to each
//! (found with rac2/tools/sigmatch.py and addrmap.py: masked-code matches of R&C1's SDK functions, so the libraries are the
//! same code at other addresses). Used only while `ver::rc2()`. Facts and evidence: rac2/NOTES.md.
//! Children: `hle.rs` (R&C2's IOP services), `tiecap.rs` (tie colours for the R&C2 pack's world picture, hooks in
//! `Mem::vif1_kick`; engine split stage E5), `stubs.txt` and `cold_hooks.txt` (its stub and cold-boot rules).

pub mod hle;
pub mod tiecap;

/// The oracle's own hooks (ver::Addrs; R&C1 values in ver::RC1_ADDRS): sceGsSyncV's vblank poll 0x11b190 (= R&C1 0x118cb0,
/// masked-code match), scePadRead 0x128a50 (R&C1 0x124bd8), sceSifCallRpc 0x11d620 / sceSifBindRpc 0x11d450 (R&C1 0x11b1c8 /
/// 0x11aff8). No poll head: the R&C2 copy never had the idle skip (0xffff_fff0 is never a pc).
pub const ADDRS: crate::ver::Addrs = crate::ver::Addrs { vblank_wait: 0x11b190, poll_head: 0xffff_fff0, pad_read: 0x128a50, sif_call_rpc: 0x11d620, sif_bind_rpc: 0x11d450 };

/// The level the game is in (0 = Aranos, also during New Game's first load) and the loader's destination level word.
pub const LEVEL: u32 = 0x1a7a78;
pub const NEXT_LEVEL: u32 = 0x1b1680;

/// Stub rules (oracle/stubs.txt format) and cold-boot hook rules (cold_hooks.txt format) of R&C2, used instead of R&C1's.
pub const STUBS: &str = include_str!("stubs.txt");
pub const HOOKS: &str = include_str!("cold_hooks.txt");

/// libmc (memory card) end callbacks and pointers mc.rs needs (R&C1 values in mc.rs): GetInfo callback 0x127c68 (R&C1
/// 0x123d10) and its three output pointers; no Chdir call (R&C2 libmc makes none: no fno 0xc in 0x127348..); Read callback
/// 0x1277f8 (R&C1 0x1238a0); the send structs of size 0x30 / 0x414 (R&C1 0x159a80 / 0x159ab0).
pub const MC: crate::mc::McAddrs = crate::mc::McAddrs {
    cb_getinfo: 0x127c68,
    getinfo_ptrs: [(0x141ba8, 0), (0x141bac, 4), (0x141bb0, 0x90)],
    // libmc makes no Chdir call (no fno 0xc in 0x127348..): never-hit address.
    cb_chdir: 0xffff_fff0,
    chdir_dir: 0x0010_0000,
    cb_read: 0x1277f8,
    send_30: 0x141c00,
    send_414: 0x141c30,
};

/// libcdvd N-command RPC client (R&C1 0x132490; sceCdRead 0x1253a8 = R&C1 0x121450) and the words its end callback
/// (0x124630 = R&C1 0x1206d8) leaves: result = command, busy flag and command cleared (R&C1 0x131318 / 0x1312f0 / 0x131314).
pub const CD_NCMD_CLIENT: u32 = 0x1375d0;
pub const CD_CMD: u32 = 0x136454;
pub const CD_BUSY: u32 = 0x136430;
pub const CD_RESULT: u32 = 0x136458;

/// 989snd's RPC sid (R&C1 bound the same sid; client 0x1a70c0, reply poll 0x132280).
pub const SND_SID: u32 = 0x123456;
/// 989snd's bank loader server (client 0x1a71c0; R&C1 client 0x15ebe8): fno 3 = bank from a disc sector, fno 0x57 = bank
/// from EE RAM (Oozla: 0x769480, the level's bank); the reply is the bank handle the play commands pass back.
pub const SND_LOAD_SID: u32 = 0x123457;
/// 989snd's CD channel busy word (the IOP clears it when a cid 56 read is done; R&C1 0x137b00) and, at +0x10, its
/// status word (R&C1 0x137b10).
pub const SND_CD_BUSY: u32 = 0x1a7180;
/// IOP data cache module (sid 0x11): its buffer as fno 2 reports it (R&C1 replied {0xde4d0, 0x94000}; R&C2 parks more,
/// so a larger region of the 2 MB IOP RAM).
pub const IOP_CACHE_SID: u32 = 0x11;
pub const IOP_CACHE_BASE: u32 = 0x8_0000;
pub const IOP_CACHE_SIZE: u32 = 0x18_0000;
