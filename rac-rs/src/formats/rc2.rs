//! R&C2 (PAL SCES_516.07) disc layout facts for the shared decoders (engine split stage E5, 2026-10-27; from lane rc2's `rc2/world`).
//! Everything here reads the disc image or a decoded level block; nothing reads PS2 RAM (the R&C2 pack does that). The shared
//! decoders branch on `Level::game` / `game::layout`; this module holds what only R&C2 needs: the ISO9660 file reads, the
//! level loader, the gameplay tables (lights, fog, tie placements), Ratchet's armor files, the moby class decoder for a class
//! the game built in RAM, and the lit tie instances.

pub mod armor;
pub mod gameplay;
pub mod iso9660;
pub mod level;
pub mod mobies;
pub mod ties;
