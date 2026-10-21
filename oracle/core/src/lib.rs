//! ee-core: the stable interface between the generated crates (oracle/rec, from recgen) and ee-oracle.
//!
//! The generated code depends on this crate only, not on ee-oracle: a change inside ee-oracle (sound, VIF, GS,
//! the interpreter, the kernel HLE, ...) rebuilds ee-oracle, the hands and the game, but none of the generated
//! crates. It holds what that code inlines (register access, VU0 macro ops, MMI, the FPU, the memory fast path,
//! the hand-written-function test) and declares the slow paths that ee-oracle implements (`extern "Rust"` in
//! cpu.rs, `#[no_mangle]` in ee-oracle rec/mod.rs). Change it only when that code needs something new: every
//! generated crate rebuilds then.
//!
//! Its `Cpu` and `Mem` are views of the first fields of ee-oracle's (same order and types, both `#[repr(C)]`;
//! ee-oracle asserts every offset at compile time).

pub mod cpu;
pub mod fpu;
pub mod mem;
pub mod rec;
pub mod vu;
