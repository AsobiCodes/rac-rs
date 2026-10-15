//! 989snd SPU2 core mix registers: what the driver writes at init and what the game's boot commands change.
//! Sources (all [C], IOP module addresses, disassembly `scratch_grain/full.dis`):
//! - snd init 0x6918 (RPC cid 0 / export 4; sync wrapper boot FUN_0012da28 from FUN_0022c3e0): sceSdInit(0) (libsd stub
//!   0x174f0), then the SetParam block 0x69f4-0x6a64, SetCoreAttr 0x6a6c / 0x6a78, then cid-9 style group volumes
//!   0x6a84-0x6ae0 (groups 0-6 and 8 = 0x400). No other init helper (0xd5f0, 0x6f80, 0x14880, 0xc728, 0x6b0c, 0x6db0)
//!   calls a libsd register function.
//! - cid 9 group 8 (0x16300, tail 0x163b0-0x163e0): master volume of core 1: v = clamp(v, 0, 0x400);
//!   MVOLL = MVOLR = ((v << 14) - v) >> 10, i.e. v * 0x3fff / 1024, written to params 0x981 / 0xa81.
//! - cid 13 (0x149e8, boot sends mode 0, param 1): core 1 MMIX (param 0x801) read-modify-write, `param != 0` sets
//!   bits 0-1 (0x3) else clears them (`& 0xfffc`); then 0x14a38 (voice pool ranges, handled by `voices`).
//! - cid 11 (0x16410): `[0x198b0] = a0` (the 16-bit output mode word read by the L/R volume routine).
//!
//! libsd parameter ids (low bit = core): MMIX 0x800, MVOLL 0x980, MVOLR 0xa80, EVOLL 0xb80, EVOLR 0xc80, AVOLL 0xd80,
//! AVOLR 0xe80, BVOLL 0xf80, BVOLR 0x1080 (0xa6d8 writes 0xf80 / 0x1080 for the movie ADMA path only).
//! SetCoreAttr ids: bit 0 = core, `2` = EFFECT_ENABLE (reverb on/off), `0xa` = SPDIF mode.
//!
//! Output effect (what the PCM mixer must know): core 0 voices pass core 0 MVOL (0x3fff = about 0.5) and reach core 1
//! through its external input at AVOL 0x7fff (MMIX bits 3 and 2 = ext dry L/R); core 1 voices pass only core 1 MVOL.
//! So a voice on core 0 (pool voices 0-23) is quieter than the same voice on core 1 (24-47) by one master stage.
//! Reverb return volumes (EVOL) are 0 at init; the game's cid 16 / 80 / 81 raise them later (`ReverbDepth`).
//! Pure Rust, no oracle types; the caller forwards the returned [`Write`]s to its SPU2 model.

/// libsd SetParam ids (without the core bit).
pub const P_MMIX: u16 = 0x800;
pub const P_MVOLL: u16 = 0x980;
pub const P_MVOLR: u16 = 0xa80;
pub const P_EVOLL: u16 = 0xb80;
pub const P_EVOLR: u16 = 0xc80;
pub const P_AVOLL: u16 = 0xd80;
pub const P_AVOLR: u16 = 0xe80;
pub const P_BVOLL: u16 = 0xf80;
pub const P_BVOLR: u16 = 0x1080;
/// libsd SetCoreAttr ids (without the core bit).
pub const C_EFFECT_ENABLE: u16 = 2;
pub const C_SPDIF_MODE: u16 = 0xa;

/// One libsd call the driver makes at init or on a boot command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    /// sceSdSetParam(id | core, value)
    Param { id: u16, value: u16 },
    /// sceSdSetCoreAttr(id | core, value)
    CoreAttr { id: u16, value: u16 },
}

const fn p(id: u16, core: u16, value: u16) -> Write {
    Write::Param { id: id | core, value }
}

/// The init writes, in order (0x69f8-0x6a7c). sceSdInit(0) comes first and is not listed.
pub const INIT: [Write; 12] = [
    p(P_MMIX, 0, 0xf00),
    p(P_MMIX, 1, 0xf0c),
    p(P_MVOLL, 0, 0x3fff),
    p(P_MVOLR, 0, 0x3fff),
    p(P_EVOLL, 0, 0),
    p(P_EVOLR, 0, 0),
    p(P_EVOLL, 1, 0),
    p(P_EVOLR, 1, 0),
    p(P_AVOLL, 1, 0x7fff),
    p(P_AVOLR, 1, 0x7fff),
    Write::CoreAttr { id: C_SPDIF_MODE, value: 0 },
    Write::CoreAttr { id: C_EFFECT_ENABLE, value: 0 },
];

/// Group volume the init sets for groups 0-6 and 8 (cid-9 style calls 0x6a84-0x6ae0). Group 7 is invalid.
pub const INIT_GROUP_VOLUME: i32 = 0x400;

/// Register image of one SPU2 core as far as the driver sets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Core {
    pub mmix: u16,
    pub mvol: [u16; 2],
    pub evol: [u16; 2],
    pub avol: [u16; 2],
    pub bvol: [u16; 2],
    pub effect_enable: bool,
}

/// Both cores plus the output mode word (cid 11).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CoreMix {
    pub core: [Core; 2],
    pub out_mode: i16,
}

/// Master volume register of group 8 (cid 9, 0x163b0): `v` clamped to 0..=0x400, value `v * 0x3fff >> 10`.
pub fn master_volume_reg(v: i32) -> u16 {
    let v = v.clamp(0, 0x400);
    (((v << 14) - v) >> 10) as u16
}

/// cid 13 MMIX update (0x149e8): bits 0-1 follow `param != 0`.
pub fn voice_mode_mmix(prev: u16, param: u32) -> u16 {
    if param != 0 {
        prev | 3
    } else {
        prev & 0xfffc
    }
}

impl CoreMix {
    /// State right after sceSdInit(0) and before the driver's own writes. libsd's reset values are not decoded
    /// ([?]); every register the driver depends on is overwritten by [`INIT`] and the group-8 write.
    pub fn after_sdinit() -> CoreMix {
        CoreMix::default()
    }

    /// Apply one libsd call to the image (params the model does not track are ignored).
    pub fn apply(&mut self, w: Write) {
        match w {
            Write::Param { id, value } => {
                let c = (id & 1) as usize;
                match id & !1 {
                    P_MMIX => self.core[c].mmix = value,
                    P_MVOLL => self.core[c].mvol[0] = value,
                    P_MVOLR => self.core[c].mvol[1] = value,
                    P_EVOLL => self.core[c].evol[0] = value,
                    P_EVOLR => self.core[c].evol[1] = value,
                    P_AVOLL => self.core[c].avol[0] = value,
                    P_AVOLR => self.core[c].avol[1] = value,
                    P_BVOLL => self.core[c].bvol[0] = value,
                    P_BVOLR => self.core[c].bvol[1] = value,
                    _ => {}
                }
            }
            Write::CoreAttr { id, value } => {
                if id & !1 == C_EFFECT_ENABLE {
                    self.core[(id & 1) as usize].effect_enable = value != 0;
                }
            }
        }
    }

    /// The module init 0x6918 after sceSdInit: [`INIT`], output mode 0 (0x69ec calls 0x16410(0)), then the group-8
    /// master volume 0x400. Returns every call in order; the image equals the state the game's boot starts from.
    pub fn boot_init() -> (CoreMix, Vec<Write>) {
        let mut m = CoreMix::after_sdinit();
        let mut calls: Vec<Write> = Vec::new();
        for w in INIT {
            m.apply(w);
            calls.push(w);
        }
        m.out_mode = 0;
        calls.extend(m.set_master_volume(INIT_GROUP_VOLUME));
        (m, calls)
    }

    /// cid 9 with group 8: core 1 master volume (params 0x981 / 0xa81).
    pub fn set_master_volume(&mut self, v: i32) -> Vec<Write> {
        let r = master_volume_reg(v);
        let ws = vec![p(P_MVOLL, 1, r), p(P_MVOLR, 1, r)];
        for &w in &ws {
            self.apply(w);
        }
        ws
    }

    /// cid 13 (boot sends mode 0, param 1): the MMIX write; the voice pool ranges are `voices`' business.
    pub fn set_voice_mode(&mut self, param: u32) -> Write {
        let w = p(P_MMIX, 1, voice_mode_mmix(self.core[1].mmix, param));
        self.apply(w);
        w
    }

    /// cid 11: output mode word (0 = what the init sets; the game sends 0 or 1, FUN_0022c3e0).
    pub fn set_out_mode(&mut self, mode: i16) {
        self.out_mode = mode;
    }
}

/// Reverb return depth (cid 16 / cid 80 via 0xc560): the depth halves are stored and written to EVOLL / EVOLR of the
/// core unless the core's effect work area is still being cleared (pending word `[0x19710 + 4 * core] != 0`, set by a
/// reverb type change 0xc198; the tick 0xc5e0 writes the stored depth after 10 ticks and re-enables the effect).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReverbDepth {
    pub left: u16,
    pub right: u16,
}

impl ReverbDepth {
    /// 0xc560: store `left` / `right`; returns the two EVOL writes when `pending` is false.
    pub fn set(&mut self, core: u16, left: u16, right: u16, pending: bool) -> Vec<Write> {
        self.left = left;
        self.right = right;
        if pending {
            Vec::new()
        } else {
            vec![p(P_EVOLL, core, left), p(P_EVOLR, core, right)]
        }
    }
}

#[cfg(test)]
#[path = "coreinit_tests.rs"]
mod tests;
