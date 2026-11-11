//! Graphics -> Effects -> Metal: shared live renderer option, persisted in pc_options.txt.
//! v0.5.1 test build 17 (user 2026-11-09: "it should be one with metal reflection detail. 3 options and off option"): ONE row,
//! Off / Low / Medium / High, in the modern menu (`ui_value` / `set_ui`) and the classic one (the RAM byte `pcctl::METAL_B`: `byte`,
//! `sync`). Off = `metal=off`; Low..High = `metal=on` + `metal_detail=low|medium|high` (both keys kept as they were, so old files
//! and the launcher map 1:1; Off leaves the detail key alone).
use super::pcopts::PcOptions;
use rac_engine::viewer::metal_detail::{self, Detail};

/// R&C1's Metal reflection detail default (lane mres, user 2026-11-08: "the new low becomes the new high"): High = the planar floor
/// reflection at a quarter of the render size per axis. ONE place: the Graphics -> Effects row's `def` and every `init` use it.
pub const DETAIL_DEFAULT: Detail = Detail::High;

/// Start-up (pcext init, ui_bridge test resets): the Metal reflection detail from the options file, else `DETAIL_DEFAULT`.
pub fn init_detail(opts: &PcOptions) {
    metal_detail::init(opts, DETAIL_DEFAULT);
}

pub fn init(opts: &PcOptions) {
    super::pcmrefl::init(opts);
    rac_engine::metal_option::set(opts.extra_get("metal").is_some_and(|v| v.eq_ignore_ascii_case("on")));
}
pub fn enabled() -> bool { rac_engine::metal_option::enabled() }
/// The Metal row's value: 0 Off, 1 Low, 2 Medium, 3 High (a `metal_detail` Full, offered by no row, shows as High).
pub fn ui_value() -> i32 {
    if enabled() { saved_ui_value() } else { 0 }
}
/// The Low..High value the saved reflection detail gives (1 Low, 2 Medium, 3 High; default High): what switching Metal on from
/// Off restores (the modern menu's "Switch Metal On" button, `NEEDS_METAL`), so a saved Low / Medium is not overwritten.
pub fn saved_ui_value() -> i32 {
    metal_detail::get().ui().clamp(0, 2) + 1
}
/// The classic row's byte (`pcctl::METAL_ROW`: Off, High, Medium, Low, the Fire / Smoke rows' order): 0 Off, 1 High, 2 Medium, 3 Low.
pub fn byte() -> u8 {
    ui_value_to_byte(ui_value())
}
pub(super) fn ui_value_to_byte(v: i32) -> u8 {
    if v <= 0 { 0 } else { (4 - v.min(3)) as u8 }
}
fn byte_to_ui_value(b: u8) -> i32 {
    if b == 0 { 0 } else { 4 - i32::from(b.min(3)) }
}
/// The menu set the Metal row to `v` (0 Off, 1 Low, 2 Medium, 3 High): applied and remembered (`metal`, and for Low..High
/// `metal_detail`); true = the options file changed.
pub fn set_ui(opts: &mut PcOptions, v: i32) -> bool {
    if v == ui_value() { return false; }
    let on = v > 0;
    let mut changed = false;
    if on {
        changed |= metal_detail::set_from_ui(opts, Detail::from_ui(v.clamp(1, 3) - 1));
    }
    if enabled() != on {
        rac_engine::metal_option::set(on);
        opts.extra_set("metal", Some(if on { "on" } else { "off" }.to_string()));
        changed = true;
    }
    changed
}
/// The classic row changed its byte: applied and remembered as `set_ui`. Any change of the byte is one press that steps the row
/// on from its value (Off -> High -> Medium -> Low -> Off, as `pcext::rows::step_multi` does for Fire / Smoke): the pause page's
/// multi-choice item cycles the byte (+1), the title page's value item only flips it (nonzero <-> 0), so reading the byte as the
/// value would never reach Medium or Low there (tb17 review). The modern menu's bridge writes the target byte before `set_ui`,
/// so its changes are not seen as a press.
pub fn sync(mem: &mut ee_oracle::mem::Mem, opts: &mut PcOptions) -> bool {
    if !super::pcctl::gfx_built(mem) { return false; }
    let last = byte();
    if mem.rd8(super::pcctl::METAL_B) == last { return false; }
    let next = (last.min(3) + 1) % 4;
    mem.wr8(super::pcctl::METAL_B, next);
    set_ui(opts, byte_to_ui_value(next))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_changes_apply_and_persist() {
        let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut opts = PcOptions::new();
        init(&opts);
        let mut mem = ee_oracle::mem::Mem::new();
        assert!(!sync(&mut mem, &mut opts));
        let rows = [[0u32; 6]; 17];
        let table = 0x50000;
        init_detail(&opts);
        super::super::pcctl::build_gfx(&mut mem, table, &rows, true, false);
        super::super::pcctl::test_gfx_section(&mut mem, 1, &rows, table);
        assert_eq!(mem.rd32(table + 120), 0x7faf);
        assert_eq!(mem.rd32(table + 124), super::super::pcctl::METAL_B);
        assert_eq!(mem.rd8(super::super::pcctl::METAL_B), 0);
        init_detail(&opts);
        mem.wr8(super::super::pcctl::METAL_B, 1);
        assert!(sync(&mut mem, &mut opts));
        assert!(enabled());
        assert_eq!(ui_value(), 3, "byte 1 = High (the old On with the default detail)");
        assert!(!sync(&mut mem, &mut opts));
        let saved = opts.to_text();
        assert_eq!(PcOptions::parse(&saved).extra_get("metal"), Some("on"));
        init(&PcOptions::parse(&saved));
        assert!(enabled());
        mem.wr8(super::super::pcctl::METAL_B, 2);
        assert!(sync(&mut mem, &mut opts));
        assert_eq!(mem.rd8(super::super::pcctl::METAL_B), 2, "a press steps High on to Medium");
        mem.wr8(super::super::pcctl::METAL_B, 3);
        assert!(sync(&mut mem, &mut opts));
        assert_eq!((ui_value(), metal_detail::get()), (1, Detail::Low), "byte 3 = Low");
        assert_eq!(PcOptions::parse(&opts.to_text()).extra_get("metal_detail"), Some("low"));
        mem.wr8(super::super::pcctl::METAL_B, 0);
        assert!(sync(&mut mem, &mut opts));
        assert!(!enabled());
        assert_eq!(PcOptions::parse(&opts.to_text()).extra_get("metal"), Some("off"));
        assert_eq!(PcOptions::parse(&opts.to_text()).extra_get("metal_detail"), Some("low"), "Off keeps the detail key");
        init_detail(&PcOptions::new());
    }

    /// The one Metal row maps 1:1 onto the two options-file keys, both ways (old files, the launcher).
    #[test]
    fn the_metal_row_reads_and_writes_both_keys() {
        let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (metal, detail, want) in [("off", "high", 0), ("off", "low", 0), ("on", "low", 1), ("on", "medium", 2), ("on", "high", 3), ("on", "", 3)] {
            let text = if detail.is_empty() { format!("metal={metal}\n") } else { format!("metal={metal}\nmetal_detail={detail}\n") };
            let opts = PcOptions::parse(&text);
            init(&opts);
            init_detail(&opts);
            assert_eq!(ui_value(), want, "{text}");
            assert_eq!(byte_to_ui_value(byte()), want);
        }
        init(&PcOptions::new());
        init_detail(&PcOptions::new());
        // One options file across the steps, as in the game (a fresh file per step lacks the key an earlier step wrote).
        let mut opts = PcOptions::new();
        for v in [3, 2, 1, 0, 2] {
            set_ui(&mut opts, v);
            assert_eq!(ui_value(), v);
            let back = PcOptions::parse(&opts.to_text());
            init(&back);
            init_detail(&back);
            assert_eq!(ui_value(), v, "the saved keys give the same row value");
        }
        assert_eq!([0, 1, 2, 3].map(ui_value_to_byte), [0, 3, 2, 1]);
        init(&PcOptions::new());
        init_detail(&PcOptions::new());
    }

    /// The title page's value item only flips the byte (nonzero <-> 0); each flip steps the row on, so Metal reaches every value:
    /// Off -> High -> Medium -> Low -> Off -> High (tb17 review: it never reached Medium / Low).
    #[test]
    fn the_title_flip_steps_metal_through_all_four_values() {
        let _guard = crate::pcopts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut opts = PcOptions::new();
        init(&opts);
        init_detail(&opts);
        let mut mem = ee_oracle::mem::Mem::new();
        let rows = [[0u32; 6]; 17];
        super::super::pcctl::build_gfx(&mut mem, 0x50000, &rows, true, false);
        let b = super::super::pcctl::METAL_B;
        assert_eq!(mem.rd8(b), 0);
        let mut seen = Vec::new();
        for _ in 0..5 {
            let cur = mem.rd8(b);
            mem.wr8(b, if cur == 0 { 1 } else { 0 }); // what the title's flip writes
            assert!(sync(&mut mem, &mut opts));
            assert_eq!(mem.rd8(b), byte(), "the byte holds the row's value");
            seen.push(ui_value());
        }
        assert_eq!(seen, [3, 2, 1, 0, 3]);
        assert!(!sync(&mut mem, &mut opts), "no change = no press");
        // The pause page's multi-choice item cycles the byte (+1): the same steps.
        mem.wr8(b, 2);
        assert!(sync(&mut mem, &mut opts));
        assert_eq!(ui_value(), 2);
        // Off keeps the saved detail; switching on restores it.
        set_ui(&mut opts, 1);
        set_ui(&mut opts, 0);
        assert_eq!(saved_ui_value(), 1, "Off keeps Low");
        init(&PcOptions::new());
        init_detail(&PcOptions::new());
    }
}