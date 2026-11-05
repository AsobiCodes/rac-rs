//! Which Ratchet & Clank game a level comes from, and the layout facts that differ between the games (engine split stage E5,
//! 2026-10-27). The formats of R&C1 and R&C2 are the same Insomniac formats with small differences; the shared decoders read the
//! differences from `Layout` (one table per game) instead of literals, so a third game adds a row, not a branch.

/// The game a `Level` was read for. R&C1 is the default of every R&C1 constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
	Rc1,
	Rc2,
}

/// Per-game layout facts. Evidence tags: [C] read in code or data, [I] inferred, [?] unknown.
pub struct Layout {
	/// Gameplay offset of the shrub instance list (count, then 0x70-byte records from +16) [C: R&C1 +0x3c, R&C2 +0x40].
	pub shrub_instances: usize,
	/// Size of one tie instance record in the gameplay block [C: R&C1 0xe0, R&C2 0x60].
	pub tie_record: usize,
	/// The core index's gadget class table (count, offset; 16-byte entries) that bounds core blocks [C: R&C1 only].
	pub gadget_table: Option<(usize, usize)>,
	/// Tie material records carry the GS CLAMP registers the face keys' wrap bits come from [C: R&C1]. R&C2 records are read
	/// as REPEAT (the picture lane rc2 drew) until their CLAMP words are checked against the game [?].
	pub tie_clamp_state: bool,
	/// A tie packet may have no colour index stream (`entry[11] == 0`): every vertex gets slot 0 [C: R&C2].
	pub tie_stream_optional: bool,
	/// Moby AD blocks carry the TEX1 / CLAMP source words that give the face keys their wrap and K bits [C: R&C1]. R&C2 blocks
	/// are keyed by the slot only (what lane rc2 drew) [?].
	pub moby_ad_state: bool,
	/// A moby texture slot the class remap leaves at 0xff is drawn flat (R&C2: such classes take their textures from elsewhere at
	/// run time) instead of being an error [C].
	pub moby_flat_unmapped: bool,
	/// The moby packet's vertex table header is seven u16 fields of 0x10 bytes (R&C1: eight u32 of 0x20) [C: R&C2].
	pub moby_vertex_u16: bool,
}

pub const RC1: Layout = Layout {
	shrub_instances: 0x3c,
	tie_record: 0xe0,
	gadget_table: Some((0x80, 16)),
	tie_clamp_state: true,
	tie_stream_optional: false,
	moby_ad_state: true,
	moby_flat_unmapped: false,
	moby_vertex_u16: false,
};

pub const RC2: Layout = Layout {
	shrub_instances: 0x40,
	tie_record: 0x60,
	gadget_table: None,
	tie_clamp_state: false,
	tie_stream_optional: true,
	moby_ad_state: false,
	moby_flat_unmapped: true,
	moby_vertex_u16: true,
};

pub fn layout(game: Game) -> &'static Layout {
	match game {
		Game::Rc1 => &RC1,
		Game::Rc2 => &RC2,
	}
}
