//! FMV (PSS movie) support: program stream demux and stream ADPCM audio.
//! See decomp/notes/cutscenes.md.

pub mod adpcm_stream;
pub mod feed;
pub mod pss;
pub mod mpeg2;
pub mod player;
pub(crate) mod rtx_video;
pub mod present;
pub mod synclog;

/// An FMV movie (a PSS file on the disc): where a game's movie table says it is (R&C1: `scene::movie`).
#[derive(Debug, Clone)]
pub struct Movie {
	pub lba: u32,
	pub bytes: u64,
	pub fps: f32,
}
