//! 989snd driver port (R&C1 PAL, IOP module 989snd.irx). Stage 1 contents: IRX access, R250, banks, pitch math.
//! Everything here is plain Rust with no oracle types; memory is reached through the tiny traits in `bank`.
pub mod adopt;
pub mod bank;
pub mod coreinit;
pub mod driver;
pub mod handler;
pub mod irx;
pub mod lfo;
pub mod persist;
pub mod pitch;
pub mod reverb;
pub mod rng;
pub mod streams;
pub mod voices;

#[cfg(test)]
pub(crate) mod testdata;
