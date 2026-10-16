//! Widescreen (16:9) for the GS-full display: `--wide` / RAC_WIDE=1.
//!
//! The game draws a 4:3 view into its 512x448 buffer and the display stretches it. For 16:9 the picture is drawn with a
//! wider horizontal field of view in the same buffer (squeezed 0.75 horizontally; the window stretches it back), the
//! vertical view stays as it was, and the 2D HUD is squeezed back by the GS-level rule (`apply_hud`).
//!
//! The field of view is one retail function, FUN_001ea1e8 (Eudora 0x1ea1e8, Rilgar 0x226568). It reads the horizontal
//! tan(half-FOV) H at base+0xb0, derives the vertical one V = H * 0.775 (0.756 with the flag at 0x15ed80), the
//! projection and culling constants, and writes ~150 words. Game code cannot be patched (the recompiled crates ignore RAM
//! code, hands are matched by code hash), so this is a *wrapping hand* (ee_oracle::rec::Hand): it runs the retail
//! function itself, and fixes the data it wrote:
//!   1. pass A, retail, with the game's own H0: the V-only words (V = H0 * k and everything derived from V alone) are kept;
//!   2. pass B, retail, with H' = H0 * 4/3: every word derived from H is right; V words are 4/3 too large, so they are
//!      overwritten with the pass-A ones;
//!   3. the one word that mixes both (cd28, copied to ce28) is recomputed with the same FPU operations.
//! The result is what the function writes with H' and with V's immediates scaled by 3/4 (the prototype's code patch), up
//! to V's rounding (V is exactly the retail value here). Memory then holds H' in cbf0: the VU0 culling code reads that
//! vector. The game sometimes reads cbf0 back and calls the function again (a clamp, a save/restore); an input equal to
//! the last H' written is taken as already wide (`REC`).
use super::gs::Gs;
use ee_oracle::cpu::Cpu;
use ee_oracle::fpu;
use ee_oracle::rec::{register_hand, Hand};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// Wide view requested (`--wide`; RAC_WIDE=1 does the same).
static WIDE: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    WIDE.store(true, Ordering::SeqCst);
}

/// Whether the 16:9 view is on (default off).
pub fn enabled() -> bool {
    WIDE.load(Ordering::SeqCst) || std::env::var("RAC_WIDE").is_ok_and(|v| v != "0" && !v.is_empty())
}

/// The picture's display aspect.
pub const ASPECT: f32 = 16.0 / 9.0;

/// The GS-level HUD rule: prims drawn after the last PATH1 (VU1) draw of a field are squeezed horizontally by 0.75
/// toward the nearer edge, except prims that read the frame buffer itself.
pub fn apply_hud(g: &mut Gs) {
    g.hud_squeeze = 0.75;
    g.hud_mask = 0;
}

/// A level's copy of the function.
struct Lv {
    name: &'static str,
    slot: usize,
    entry: u32,
    /// The data block the function addresses (`$s0` = base): H at +0xb0.
    base: u32,
    /// The retail leaf functions it calls: atan2(y = f12, x = f13) -> f0, cos(f12) -> f0.
    atan2: u32,
    cos: u32,
}

const EUDORA: Lv = Lv { name: "eudora", slot: 0, entry: 0x1ea1e8, base: 0x16cb40, atan2: 0x1f1e48, cos: 0x1f1d80 };
/// `EUDORA` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const EUDORA_USA: Lv = Lv { name: "eudora", slot: 0, entry: 0x1ea2e8, base: 0x16cbc0, atan2: 0x1f2318, cos: 0x1f2250 };
const RILGAR: Lv = Lv { name: "rilgar", slot: 1, entry: 0x226568, base: 0x16ce40, atan2: 0x22e148, cos: 0x22e080 };
/// `RILGAR` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const RILGAR_USA: Lv = Lv { name: "rilgar", slot: 1, entry: 0x2265e8, base: 0x16ce40, atan2: 0x22e598, cos: 0x22e4d0 };

// Offsets from `base`.
const H: u32 = 0xb0;
const V: u32 = 0xb4;
const H_COS: u32 = 0xb8;
const V_COS: u32 = 0xbc;
const AX: u32 = 0xa8;
const AY: u32 = 0xac;
const MIX: [u32; 2] = [0x1e8, 0x2e8];
/// Words the function derives from V alone: V, 1/cos(atan V) (cbfc, cd24, ce0c, ce24), the copy of V in ce00's vector,
/// the projection matrix's y scale (cc14, its copies cc54, cc94) and ccf4.
const V_ONLY: [u32; 10] = [V, V_COS, 0x1e4, 0x2c4, 0x2cc, 0x2e4, 0xd4, 0x114, 0x154, 0x1b4];

static ON: AtomicBool = AtomicBool::new(true);
/// Test switch: false widens nothing (H' = H0) so that the wrapper must reproduce the plain retail run word for word.
static SCALE: AtomicBool = AtomicBool::new(true);
static IN: AtomicBool = AtomicBool::new(false);
/// Per level: (the game's H0, the H' written for it) of the last call.
static REC: Mutex<[(u32, u32); 2]> = Mutex::new([(0, 0); 2]);
/// Calls, calls whose input was already wide, distinct game inputs seen (RAC_WIDE_LOG prints them).
static CALLS: AtomicU64 = AtomicU64::new(0);
static BY_LEVEL: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static REWIDE: AtomicU64 = AtomicU64::new(0);
static SEEN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        IN.store(false, Ordering::SeqCst);
    }
}

fn eudora(cpu: &mut Cpu) -> bool {
    wrap(cpu, &(*crate::retail::disc::pick(&EUDORA, &EUDORA_USA)))
}

fn rilgar(cpu: &mut Cpu) -> bool {
    wrap(cpu, &(*crate::retail::disc::pick(&RILGAR, &RILGAR_USA)))
}

static HANDS: [Hand; 2] = [
    Hand { entry: 0x1ea1e8, end: 0x1ea804, hash: 0x508455e717086073, est: 0, f: eudora },
    Hand { entry: 0x226568, end: 0x226b84, hash: 0x435daab94236138b, est: 0, f: rilgar },
];

/// Registers the wrapping hands (before `Machine::enable_rec`). They apply only to code that matches their hash.
pub fn register() {
    register_hand(crate::retail::disc::hands(&HANDS));
}

/// Test switch: false makes the registered hands step aside (the plain retail function runs).
pub fn set_active(on: bool) {
    ON.store(on, Ordering::SeqCst);
}

/// Calls per level (Eudora, Rilgar).
pub fn by_level() -> [u64; 2] {
    [BY_LEVEL[0].load(Ordering::Relaxed), BY_LEVEL[1].load(Ordering::Relaxed)]
}

/// (calls, calls whose input was already wide, distinct inputs).
pub fn stats() -> (u64, u64, Vec<u32>) {
    (CALLS.load(Ordering::Relaxed), REWIDE.load(Ordering::Relaxed), SEEN.lock().map(|s| s.clone()).unwrap_or_default())
}

/// H' = H * 4/3 with the EE FPU's operations (0.63 -> 0.84, bit-exact with the prototype's patch).
fn widen(h: u32, csr: u32) -> u32 {
    fpu::div(fpu::mul(h, 0x4080_0000), 0x4040_0000, csr)
}

/// 1 / cos(atan2(1, x)), as the function computes it.
fn a_of(cpu: &mut Cpu, lv: &Lv, x: u32) -> u32 {
    cpu.f[12] = 0x3f80_0000;
    cpu.f[13] = x;
    cpu.call(lv.atan2);
    cpu.f[12] = cpu.f[0];
    cpu.call(lv.cos);
    fpu::div(0x3f80_0000, cpu.f[0], cpu.div_csr)
}

fn wrap(cpu: &mut Cpu, lv: &Lv) -> bool {
    if !ON.load(Ordering::Relaxed) || IN.swap(true, Ordering::SeqCst) {
        // Off, or the retail body called from the wrapper itself.
        return false;
    }
    let _g = Guard;
    let b = lv.base;
    let csr = cpu.div_csr;
    let x = cpu.mem.rd32(b + H);
    let (raw, wide) = REC.lock().map(|r| r[lv.slot]).unwrap_or((0, 0));
    let again = wide != 0 && x == wide && wide != raw;
    let h0 = if again { raw } else { x };
    CALLS.fetch_add(1, Ordering::Relaxed);
    BY_LEVEL[lv.slot].fetch_add(1, Ordering::Relaxed);
    if again {
        REWIDE.fetch_add(1, Ordering::Relaxed);
    }
    if std::env::var_os("RAC_WIDE_LOG").is_some() && !again {
        if let Ok(mut s) = SEEN.lock() {
            if !s.contains(&h0) {
                s.push(h0);
                eprintln!("wide: {} game H0 {:08x} = {} -> {:08x}", lv.name, h0, f32::from_bits(h0), widen(h0, csr));
            }
        }
    }
    // Pass A: retail with the game's own H.
    cpu.mem.wr32(b + H, h0);
    cpu.call(lv.entry);
    let mut keep = [0u32; V_ONLY.len()];
    for (k, o) in keep.iter_mut().zip(V_ONLY) {
        *k = cpu.mem.rd32(b + o);
    }
    // Pass B: retail with the wide H.
    let hw = if SCALE.load(Ordering::Relaxed) { widen(h0, csr) } else { h0 };
    cpu.mem.wr32(b + H, hw);
    cpu.call(lv.entry);
    for (k, o) in keep.iter().zip(V_ONLY) {
        cpu.mem.wr32(b + o, *k);
    }
    // The word that mixes H and V: (A(H*ax) / A(H) + A(V*ay) / A(V)) * 0.5.
    let (ax, ay) = (cpu.mem.rd32(b + AX), cpu.mem.rd32(b + AY));
    let (h_cos, v_cos, v) = (cpu.mem.rd32(b + H_COS), cpu.mem.rd32(b + V_COS), cpu.mem.rd32(b + V));
    let hx = fpu::mul(hw, ax);
    let t1 = fpu::div(a_of(cpu, lv, hx), h_cos, csr);
    let vy = fpu::mul(v, ay);
    let t2 = fpu::div(a_of(cpu, lv, vy), v_cos, csr);
    let mix = fpu::mul(fpu::add(t1, t2), 0x3f00_0000);
    for o in MIX {
        cpu.mem.wr32(b + o, mix);
    }
    if let Ok(mut r) = REC.lock() {
        r[lv.slot] = (h0, hw);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use ee_oracle::Machine;

    const FIRST: u32 = 0x10_0000;

    fn state() -> String {
        std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s07").into())
    }

    /// RAM image after calling the function once from a fresh load.
    fn call_once(rec: bool, scale: bool, on: bool, patch: bool) -> Vec<u8> {
        ee_game::register();
        register();
        ON.store(on, Ordering::SeqCst);
        SCALE.store(scale, Ordering::SeqCst);
        *REC.lock().unwrap() = [(0, 0); 2];
        let mut m = Machine::load(&state());
        if rec {
            m.enable_rec();
        }
        let b = EUDORA.base;
        if patch {
            // The prototype's code patch (GSFULL.md section 5) and the wide scalar, interpreted.
            for (a, w) in [(0x1ea224u32, 0x3c013f11u32), (0x1ea228, 0x342126e9), (0x1ea23c, 0x3c013f14), (0x1ea240, 0x3421cccd)] {
                m.cpu.mem.wr32(a, w);
            }
            let h0 = m.cpu.mem.rd32(b + H);
            let hw = widen(h0, m.cpu.div_csr);
            m.cpu.mem.wr32(b + H, hw);
        }
        m.cpu.r[29] = 0x01f0_0000;
        m.cpu.call(EUDORA.entry);
        ON.store(true, Ordering::SeqCst);
        SCALE.store(true, Ordering::SeqCst);
        m.cpu.mem.ram.to_vec()
    }

    fn diff(a: &[u8], b: &[u8]) -> Vec<(u32, u32, u32)> {
        let mut d = vec![];
        for i in (FIRST as usize..a.len().min(b.len())).step_by(4) {
            let (x, y) = (u32::from_le_bytes(a[i..i + 4].try_into().unwrap()), u32::from_le_bytes(b[i..i + 4].try_into().unwrap()));
            if x != y {
                d.push((i as u32, x, y));
            }
        }
        d
    }

    /// The wrapper with no widening reproduces the plain retail run in every RAM word; with widening it equals the
    /// interpreted, code-patched function except for V's rounding (reported).
    #[test]
    #[ignore]
    fn wide_hand_matches_retail_and_patch() {
        let plain = call_once(true, true, false, false);
        let same = call_once(true, false, true, false);
        let d0 = diff(&plain, &same);
        eprintln!("wrapper(no scale) vs plain retail: {} words differ {:x?}", d0.len(), &d0[..d0.len().min(16)]);
        assert!(d0.is_empty());
        let wide = call_once(true, true, true, false);
        let truth = call_once(false, true, true, true);
        let d1: Vec<_> = diff(&wide, &truth).into_iter().filter(|d| !(0x1ea224..0x1ea244).contains(&d.0)).collect();
        eprintln!("wrapper(wide) vs interpreted patch: {} words differ", d1.len());
        for (a, x, y) in &d1 {
            eprintln!("  {a:06x}: wrapper {x:08x} ({}) patch {y:08x} ({})", f32::from_bits(*x), f32::from_bits(*y));
        }
        let vs: Vec<u32> = V_ONLY.iter().chain(MIX.iter()).map(|o| EUDORA.base + o).collect();
        let bad: Vec<_> = d1.iter().filter(|d| d.0 < 0x1ef0000 && (!vs.contains(&d.0) || (d.1 as i64 - d.2 as i64).abs() > 4)).collect();
        eprintln!("outside the V words or more than 4 ulp: {}", bad.len());
        assert!(bad.is_empty());
        // Wide must differ from plain, and the scalar is the prototype's 0.84.
        let dp = diff(&plain, &wide);
        eprintln!("wide vs plain: {} words differ (H-derived data)", dp.len());
        assert!(!dp.is_empty());
        assert_eq!(u32::from_le_bytes(wide[(EUDORA.base + H) as usize..][..4].try_into().unwrap()), 0x3f57_0a3d);
    }
}
