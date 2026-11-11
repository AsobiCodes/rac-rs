//! R&C1's side of the keyboard + mouse system: evaluation with the R&C1 table (`rac1/kbm_actions.rs`) and the live bindings.

use rac_engine::input::kbm::{self, Ctx, Input, Out};

/// The pad this field's held inputs give under the live bindings.
pub fn eval(down: &dyn Fn(Input) -> bool, ctx: &Ctx) -> Out {
	crate::kbm_actions::install();
	kbm::eval(kbm::table(), &kbm::binds(), down, ctx)
}
