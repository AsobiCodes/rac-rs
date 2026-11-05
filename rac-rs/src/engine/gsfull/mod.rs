//! GS-full: the game's own complete picture. ee-oracle runs the retail EE code and hands every graphics DMA transfer of
//! a field over as a `Job`; this module runs the VIF1 stream and the VU1 microprograms (`vif`, `vu1`, `vu1aot`) and
//! rasterises all three GIF paths with a software GS (`gs`, `gs_raster`, `gs_queue`, `gs_mem`), then shows the display
//! buffer. `pipe` ties it together and runs it on a thread of its own (retail mode, RAC_GSFULL=1 / `--gsfull`).
//! R&C2 (engine split stage E5, from lane rc2; all gated on `ee_oracle::ver::rc2()`, off for R&C1): its VU1 program table
//! (`vif_rc2`), the timed XGKICK (`vu1_kick`), the HUD overlay mode (`overlay`, `gs_overlay`, `ovl_pipe`: the GS draws only
//! the HUD and the effect programs over the pack's world picture), textures read back from VRAM (`texreq`), water batches (`water`).
//! Notes: <dev>/scratch_gsfull/GSFULL.md, <dev>/scratch_play/handoff/GS_integrate.md.
pub mod crtc;
pub mod gs;
pub mod gs_mem;
pub mod gs_queue;
pub mod gs_raster;
pub mod overlay;
pub mod ovl_pipe;
pub mod pipe;
pub mod texreq;
pub mod vif;
pub mod vu1;
pub mod vif_rc2;
pub mod vu1aot;
pub mod water;
pub mod wide;
