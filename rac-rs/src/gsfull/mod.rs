//! GS-full: the game's own complete picture. ee-oracle runs the retail EE code and hands every graphics DMA transfer of
//! a field over as a `Job`; this module runs the VIF1 stream and the VU1 microprograms (`vif`, `vu1`, `vu1aot`) and
//! rasterises all three GIF paths with a software GS (`gs`, `gs_raster`, `gs_queue`, `gs_mem`), then shows the display
//! buffer. `pipe` ties it together and runs it on a thread of its own (retail mode, RAC_GSFULL=1 / `--gsfull`).
//! Notes: <dev>/scratch_gsfull/GSFULL.md, <dev>/scratch_play/handoff/GS_integrate.md.
pub mod crtc;
pub mod gs;
pub mod gs_mem;
pub mod gs_queue;
pub mod gs_raster;
pub mod pipe;
pub mod vif;
pub mod vu1;
pub mod vu1aot;
pub mod wide;
