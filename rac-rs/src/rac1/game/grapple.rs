//! ARI-46: the held Swingshot (class 208) grapple states 2-7 and their launch from the idle states 0 / 1 (FUN_002c5498,
//! decomp/notes/aridia_gadgets.md "ARI-46 port spec", Order "Held grapple"). The held actor's words belong to the
//! ARI-47c updater (game/swingshot.rs `update_swingshot`, `Held208`); these steps take them by reference:
//! `grapple_launch` in its 0 / 1 arm after the wrap and the hook follow (before its 0x2c test), `grapple_step` for
//! held states 2..=7. The caller applies `GrappleOut` (held blend `blend(seq, 0, scale(10))`, rope flag, cues as class
//! 208 sounds); the hook's own states 3-7 run in the ARI-47c hook step H.











