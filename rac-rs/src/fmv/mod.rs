//! FMV (PSS movie) support: program stream demux and stream ADPCM audio.
//! See decomp/notes/cutscenes.md.

pub mod adpcm_stream;
pub mod pss;
pub mod mpeg2;
pub mod player;
pub mod present;
