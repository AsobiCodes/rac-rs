//! Weapon ammo counters and the pending-flag pass over the object slot table (retail overlay code, Eudora addresses).
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Slot table base (the `lui 0x14` / `addiu -0xcb0` pair), 0x50-byte records, 0x230 bytes of them.
const SLOT_BASE: u32 = 0x0013_F350;
const SLOT_END: u32 = SLOT_BASE + 0x230;
const SLOT_STRIDE: u32 = 0x50;
/// Object flag word (u16) at +0x34 of the linked objects.
const FLAG_WORD: u32 = 0x34;
/// Current weapon id, read when the caller passes -1.
const CUR_WEAPON: u32 = 0x0014_0408;
/// Per-weapon record table, 0x18 bytes each; max ammo is the u16 at +0xe.
const WEAPON_TABLE: u32 = 0x001C_4130;
const WEAPON_STRIDE: u32 = 0x18;
const WEAPON_MAX: u32 = 0xe;
/// Ammo held per weapon id (u32 array), and the used-ammo statistic beside it.
const AMMO: u32 = 0x0013_D428;
const USED: u32 = 0x0013_DEA0;
/// GP-relative slot read by the pending-flag pass.
const GP_SLOT: i32 = -0x765c;

/// Sets bit 0 of the flag word of the object at `p` (a null pointer does nothing).
fn set_flag(c: &mut Cpu, p: u32) {
    if p != 0 {
        let v = c.r16(p + FLAG_WORD) | 1;
        c.w16(p + FLAG_WORD, v);
    }
}

/// 0x226128 mark_pending_flags(): sets the flag bit of the object linked at 0x13f350+0xa88 (retail has no null test
/// there: a null link hits the halfword at 0x34), of every object linked from the slot table at 0x13f350, the
/// single-object links, and clears bits 0x41 of the object at 0x13f350+0x1090 when the table state is 8.
pub fn mark_pending_flags(c: &mut Cpu) -> bool {
    let head = c.r32(SLOT_BASE + 0xa88);
    let v = c.r16(head + FLAG_WORD) | 1;
    c.w16(head + FLAG_WORD, v);
    let mut slot = SLOT_BASE;
    loop {
        let first = c.r32(slot + 0x1090);
        set_flag(c, first);
        let second = c.r32(slot + 0x1094);
        set_flag(c, second);
        slot += SLOT_STRIDE;
        if slot >= SLOT_END {
            break;
        }
    }
    let linked = c.r32(SLOT_BASE + 0x118c);
    set_flag(c, linked);
    if c.r8(SLOT_BASE + 0x20a4) == 1 {
        let obj = c.r32(SLOT_BASE + 0x1624);
        set_flag(c, obj);
    }
    // Retail's last write to v0 is the value it loaded last on the path taken, so v0 is set to that value.
    let state = c.r32(SLOT_BASE + 0x10b8);
    if state != 8 {
        c.ret(state);
        return true;
    }
    let obj = c.r32(SLOT_BASE + 0x1090);
    // The delay slot of the null test loads the gp slot into v0, so it is read (and returned) on the null path too.
    let gp = (c.g(28) as u32).wrapping_add(GP_SLOT as u32);
    let gp_slot = c.r32(gp);
    if obj == 0 || gp_slot != 0 {
        c.ret(gp_slot);
        return true;
    }
    let kind = c.r8(obj + 0x20);
    if kind == 0 {
        c.ret(kind as u32);
        return true;
    }
    let flags = c.r16(obj + FLAG_WORD) & 0xffbe;
    c.w16(obj + FLAG_WORD, flags);
    c.ret(flags as u32);
    true
}

/// 0x226ef8 consume_ammo(id, n) -> 1 if n ammo was taken (or the weapon has no limit), else 0; id -1 = current.
pub fn consume_ammo(c: &mut Cpu) -> bool {
    let mut id = c.arg(0);
    let n = c.arg(1);
    if id == u32::MAX {
        id = c.r32(CUR_WEAPON);
    }
    let entry = WEAPON_TABLE.wrapping_add(id.wrapping_mul(WEAPON_STRIDE));
    if c.r16(entry + WEAPON_MAX) == 0 {
        c.ret(1);
        return true;
    }
    let ammo_at = AMMO.wrapping_add(id << 2);
    let have = c.r32(ammo_at);
    if (have as i32) < (n as i32) {
        c.ret(0);
        return true;
    }
    c.w32(ammo_at, have.wrapping_sub(n));
    let used_at = USED.wrapping_add(id << 2);
    let used = c.r32(used_at).wrapping_add(n);
    c.w32(used_at, used);
    c.ret(1);
    true
}

/// 0x226f80 add_ammo(id, n) -> overflow beyond the weapon's max (0 if none): adds n ammo, clamped to max.
pub fn add_ammo(c: &mut Cpu) -> bool {
    let id = c.arg(0);
    let n = c.arg(1);
    let entry = WEAPON_TABLE.wrapping_add(id.wrapping_mul(WEAPON_STRIDE));
    let ammo_at = AMMO.wrapping_add(id << 2);
    let total = c.r32(ammo_at).wrapping_add(n);
    let max = c.r16(entry + WEAPON_MAX) as u32;
    c.w32(ammo_at, total);
    if max == 0 {
        c.ret(0);
        return true;
    }
    let over = total.wrapping_sub(max) as i32;
    if over > 0 {
        c.w32(ammo_at, max);
        c.ret(over as u32);
    } else {
        c.ret(0);
    }
    true
}
