//! Water currents 613 and 679 (decomp/notes/novalis_environment.md "Water currents 613 and 679"):
//! 613 adds a flow along its path to the swimming hero's motion (additive displacement 0x13f440) (nearest current wins via a
//! per-tick latch); 679 steers the water slide along its paths and keeps the slide timer alive.
//! Velocities here are u/s (the notes' per-tick values are x60 [I]). The exit case of 613 (hero just
//! left the water, groups 4/5) and the re-entry cap are ported (HERO-5).

































