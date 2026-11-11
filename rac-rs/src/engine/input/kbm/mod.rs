//! The native keyboard + mouse system (v0.5.1, design `V051_KBM_DESIGN.md`): inputs (key, mouse button, wheel notch) -> actions (per
//! layer, two slots each) -> outputs (a pad bit, a stick direction, a PC action). Game-neutral: a game pack hands over its action
//! table (`pack::Pack::kbm`); the retail game code still reads a PS2 pad, which only the last step (`eval`) produces.
//!
//! - `types`: Input / Output / Layer / ActionDef / Table, the Menu layer's MenuKey.  - `binds`: assign / clear / reset / shared.
//! - `eval`: layers with shadowing, today's stick rule.  - `file`: `kbm=` text, the conversion of an old `keys=` line, the old
//!   24-function view for the classic page.  - `live`: the global state + the capture machine.  - `padeval`: the controller's actions.  - `padfile`: `gamepad=` text + the conversion of an old `pad=` line.  - `names`, `generic` (R&C2's table).

pub mod binds;
pub mod eval;
pub mod file;
pub mod generic;
pub mod live;
pub mod names;
pub mod padeval;
pub mod padfile;
pub mod types;

pub use binds::Binds;
pub use eval::{eval, Ctx, Out};
pub use padeval::{eval_pad, PadEval};
pub use live::*;
pub use types::*;

/// Tests that use the global live state (init / capture) take this lock.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
mod pad_tests;
#[cfg(test)]
mod tests;
