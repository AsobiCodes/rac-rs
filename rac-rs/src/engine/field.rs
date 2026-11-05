//! The game's field clock (engine split stage E2 batch 4a, 2026-10-26: moved out of rac1/retail/disc.rs): one retail field in
//! seconds by the disc's video standard (`ee_oracle::ver`: PAL 50 fields/s, NTSC 60000/1001).

/// One retail field in seconds (0.02 PAL; NTSC 1001 / 60000).
#[inline]
pub fn field_dt() -> f32 {
	if ee_oracle::ver::ntsc() { 1001.0 / 60000.0 } else { 0.02 }
}

/// Retail fields a second (50 PAL; NTSC 60000 / 1001). PAL stays the exact 50.0 for code that divides by it.
#[inline]
pub fn field_rate() -> f32 {
	if ee_oracle::ver::ntsc() { 60000.0 / 1001.0 } else { 50.0 }
}

/// Two fields (the catch-up cap of the viewer's step loops).
#[inline]
pub fn field_dt2() -> f32 {
	if ee_oracle::ver::ntsc() { 2.0 * 1001.0 / 60000.0 } else { 0.04 }
}
