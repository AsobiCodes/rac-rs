//! Speed hands (lane hs, merged into dev s93 on 2026-10-22): exact native rewrites of the hottest retail functions, each GENERATED
//! by the lane's cut/hsfinish.py from gated region units (scratch_perf_hs; the gate, corpora and tools stay there).
//! `rec::Table::build` installs every one at each recompiled function whose code words match its template (any level, either disc).
// The generated hands keep the stitcher's register model: dead stores, spare `mut`s and parens are expected there (~2000 lints).
#![allow(unused)]

pub mod f_2805e0;
pub mod f_282218;
pub mod f_2755d0;
pub mod f_2440c8;
pub mod f_23f6e8;
pub mod f_1e94d8;
pub mod f_1e80c0;
pub mod f_2815c8;
pub mod f_23bfc0;
pub mod f_255768;
pub mod f_1ea5c8;

/// Built-in relocatable speed hands (S4): `rec::Table::build` installs them by itself (no call from rac-rs needed); `RAC_HS=0` leaves them out.
pub fn reloc_all() -> Vec<&'static crate::hs_reloc::RelocHand> {
    vec![
        &f_2805e0::RELOC_HAND,
        &f_282218::RELOC_HAND,
        &f_2755d0::RELOC_HAND,
        &f_2440c8::RELOC_HAND,
        &f_23f6e8::RELOC_HAND,
        &f_1e94d8::RELOC_HAND,
        &f_1e80c0::RELOC_HAND,
        &f_2815c8::RELOC_HAND,
        &f_23bfc0::RELOC_HAND,
        &f_255768::RELOC_HAND,
        &f_1ea5c8::RELOC_HAND,
    ]
}
