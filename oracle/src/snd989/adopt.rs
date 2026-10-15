//! Savestate adoption of a running 989snd driver is a development aid (it reads the fields of an emulator savestate) and is not
//! part of the published source. A cold boot never reaches it: `Inner::adopt_with` returns before this module is used, so the
//! driver starts from the disc and the EE's own boot calls. This stub keeps the two call sites in driver.rs compiling.
use super::driver::Inner;

/// Per-voice SPU2 state of a savestate (not available in this build).
pub struct SpuvRec;

/// No savestate voice file is understood here.
pub fn parse_spuv(_b: &[u8]) -> Option<Vec<SpuvRec>> {
    None
}

impl Inner {
    /// No running driver to adopt: reports "nothing adopted".
    pub fn adopt_driver(&mut self, _iop: &[u8], _vb: u32, _spuv: Option<&[SpuvRec]>) -> bool {
        false
    }
}
