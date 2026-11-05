//! Graphics -> Effects -> Metal: shared live renderer option, persisted in pc_options.txt.
use super::pcopts::PcOptions;

pub fn init(opts: &PcOptions) {
    super::pcmrefl::init(opts);
    rac_engine::metal_option::set(opts.extra_get("metal").is_some_and(|v| v.eq_ignore_ascii_case("on")));
}
pub fn enabled() -> bool { rac_engine::metal_option::enabled() }
/// The modern screen set Metal (`pcext/ui_bridge.rs`): applied and remembered like the classic row's byte; true = the file changed.
pub fn set_from_ui(opts: &mut PcOptions, on: bool) -> bool {
    if enabled() == on { return false; }
    rac_engine::metal_option::set(on);
    opts.extra_set("metal", Some(if on { "on" } else { "off" }.to_string()));
    true
}
pub fn sync(mem: &mut ee_oracle::mem::Mem, opts: &mut PcOptions) -> bool {
    if !super::pcctl::gfx_built(mem) { return false; }
    let on = mem.rd8(super::pcctl::METAL_B) != 0;
    if enabled() == on { return false; }
    rac_engine::metal_option::set(on);
    opts.extra_set("metal", Some(if on { "on" } else { "off" }.to_string()));
    true
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
        super::super::pcctl::build_gfx(&mut mem, table, &rows, true, false);
        super::super::pcctl::test_gfx_section(&mut mem, 1, &rows, table);
        assert_eq!(mem.rd32(table + 120), 0x7faf);
        assert_eq!(mem.rd32(table + 124), super::super::pcctl::METAL_B);
        assert_eq!(mem.rd8(super::super::pcctl::METAL_B), 0);
        mem.wr8(super::super::pcctl::METAL_B, 1);
        assert!(sync(&mut mem, &mut opts));
        assert!(enabled());
        assert!(!sync(&mut mem, &mut opts));
        let saved = opts.to_text();
        assert_eq!(PcOptions::parse(&saved).extra_get("metal"), Some("on"));
        init(&PcOptions::parse(&saved));
        assert!(enabled());
        mem.wr8(super::super::pcctl::METAL_B, 0);
        assert!(sync(&mut mem, &mut opts));
        assert!(!enabled());
        assert_eq!(PcOptions::parse(&opts.to_text()).extra_get("metal"), Some("off"));
    }
}