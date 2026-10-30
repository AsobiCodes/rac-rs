//! Smooth texture detail transitions, enabled by Lighting Enhanced unless pc_options.txt says texfilter=classic.
//! The existing Lighting row is the player-facing switch; no new retail menu bytes or records.
use std::sync::atomic::{AtomicBool, Ordering};

/// Requested filtering mode; lighting and the Classic split pass still gate it at draw time.
static SMOOTH: AtomicBool = AtomicBool::new(true);

pub(super) fn set(on: bool) {
    SMOOTH.store(on, Ordering::Relaxed);
}

/// The applied mode (the Preset row's detection reads it).
pub(super) fn get() -> bool {
    SMOOTH.load(Ordering::Relaxed)
}

/// Enhanced draw gate, including the Classic half of F10 split view.
pub fn enabled(enhanced: bool) -> bool {
    crate::retail::splitview::fx(enhanced && SMOOTH.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::super::pcopts::PcOptions;

    #[test]
    fn saved_mode_defaults_and_round_trips() {
        assert_eq!(PcOptions::new().texfilter.unwrap_or(true), true);
        for (word, on) in [("classic", false), ("smooth", true), ("SMOOTH", true)] {
            let o = PcOptions::parse(&format!("texfilter={word}\n"));
            assert_eq!(o.texfilter, Some(on));
            assert_eq!(PcOptions::parse(&o.to_text()).texfilter, Some(on));
        }
        assert_eq!(PcOptions::parse("texfilter=bad\n").texfilter, None);
    }

    #[test]
    fn classic_lighting_never_smooths() {
        assert!(!super::enabled(false));
    }
}
