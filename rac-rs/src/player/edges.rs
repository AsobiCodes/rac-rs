//! Stick edge bits and the 30-tick edge ring (decomp/notes/hero_crouch_ledge.md, HERO-8 port spec A).
//! The retail pad words are rebuilt from the camera-space stick [I]: an axis outside the 47/127 dead
//! zone is a held bit, its rising edge and the flick rule are the "any direction press" (mask 0x1f000).



