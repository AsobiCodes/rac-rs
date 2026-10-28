//! The new Graphics / Camera texts of the PC menu: ids and the five languages (moved out of pcext.rs 2026-10-24, lane ycld: a pure move
//! plus the renamed Volumetric fog row and the new Volumetric clouds row; `use super::*` reaches the same ids).
use super::*;

pub(super) const NEW_IDS: [u32; 33] = [
	ID_CLASSIC,
	ID_MODERN,
	ID_STRAFE,
	ID_BUYALL,
	ID_BUYALL_PROMPT,
	ID_GRAPHICS,
	ID_GRAPHICS_HDR,
	ID_GRAPHICS_DESC,
	ID_LIGHTING,
	ID_LIGHT_CLASSIC,
	ID_ENHANCED,
	ID_AO,
	ID_SHADOWS,
	ID_DISPLAY,
	ID_WINDOWED,
	ID_FULLSCREEN,
	ID_WATER,
	ID_PARTICLES,
	ID_SUNRAYS,
	ID_PART_CLASSIC,
	ID_SMOOTHING,
	ID_VEGETATION,
	ID_VEG_MODERATE,
	ID_VEG_LUSH,
	ID_LOW,
	ID_MEDIUM,
	ID_HIGH,
	ID_CULLING,
	ID_GI,
	ID_WAVES,
	ID_DRAW,
	ID_FOG,
	ID_CLOUDS,
];

/// New texts per language (En, Fr, De, Es, It) in the game's font bytes: a-acute 0x84, e-acute 0x8c, e-grave 0x8d, a-umlaut 0x87,
/// o-acute 0x9c, e-circumflex 0x8e. Short like the retail labels (the German ones are abbreviated to fit). [I] wording, native review wanted. The
/// Graphics header and description follow the retail Camera Options ones ("Options cam\x8cra", "Adjust camera control settings").
pub(super) static TEXTS: [[&[u8]; 5]; 33] = [
	[b"Classic", b"Classique", b"Klassisch", b"Cl\x84sica", b"Classica"],
	[b"Modern", b"Moderne", b"Modern", b"Moderna", b"Moderna"],
	[b"Strafe aiming", b"Vis\x8ce lat\x8crale", b"Strafe-Zielen", b"Apuntado lateral", b"Mira laterale"],
	[b"Refill all ammo", b"Tout remplir", b"Alles f\xa7llen", b"Llenar todo", b"Ricarica tutto"],
	// The Buy plate: retail's own "[Cross] Buy" (id 0x5234), a newline (byte 1), the Square glyph (0x13) and a one-word label that fits
	// the plate on one line (vendor.rs test `plate_text_is_two_lines`; the plate holds about 7 letters after a glyph).
	[b"\x10 Buy\x01\x13 Refill", b"\x10 Acheter\x01\x13 Remplir", b"\x10 Kaufen\x01\x13 F\xa7llen", b"\x10 Comprar\x01\x13 Llenar", b"\x10 Compra\x01\x13 Ricarica"],
	[b"Graphics", b"Graphismes", b"Grafik", b"Gr\x84ficos", b"Grafica"],
	[b"Graphics Options", b"Options graphismes", b"Grafik-Optionen", b"Opciones de gr\x84ficos", b"Opzioni grafica"],
	[b"Adjust lighting and shadow settings", b"R\x8cgler l'\x8cclairage et les ombres", b"Beleuchtung und Schatten \x87ndern", b"Ajusta la iluminaci\x9cn y las sombras", b"Regola illuminazione e ombre"],
	[b"Lighting", b"Lumi\x8dre", b"Beleuchtung", b"Iluminaci\x9cn", b"Illuminazione"],
	// Lighting values (masculine: "le rendu", "el modo"; the Camera row's Classic text 0x7f01 is feminine in Es / It).
	[b"Classic", b"Classique", b"Klassisch", b"Cl\x84sico", b"Classico"],
	[b"Enhanced", b"Am\x8clior\x8c", b"Verbessert", b"Mejorado", b"Migliorato"],
	[b"Ambient occlusion", b"Occlusion ambiante", b"Umgebungsverdeckung", b"Oclusi\x9cn ambiental", b"Occlusione ambientale"],
	[b"Dynamic shadows", b"Ombres dynamiques", b"Dynamische Schatten", b"Sombras din\x84micas", b"Ombre dinamiche"],
	[b"Display mode", b"Affichage", b"Anzeige", b"Pantalla", b"Schermo"],
	[b"Windowed", b"Fen\x8etre", b"Fenster", b"Ventana", b"Finestra"],
	[b"Fullscreen", b"Plein \x8ccran", b"Vollbild", b"Completa", b"Intero"],
	[b"Water", b"Eau", b"Wasser", b"Agua", b"Acqua"],
	// Spanish "Efectos": "Part\xedculas" needs an i-acute the font table notes do not list.
	[b"Particles", b"Particules", b"Partikel", b"Efectos", b"Particelle"],
	[b"Sun rays", b"Rayons de soleil", b"Sonnenstrahlen", b"Rayos de sol", b"Raggi di sole"],
	[b"Classic", b"Classiques", b"Klassisch", b"Cl\x84sicos", b"Classiche"],
	[b"Frame smoothing", b"Lissage d'image", b"Bildgl\x87ttung", b"Suavizado de imagen", b"Movimento fluido"],
	[b"Vegetation", b"V\x8cg\x8ctation", b"Vegetation", b"Vegetaci\x9cn", b"Vegetazione"],
	[b"Moderate", b"Mod\x8cr\x8ce", b"Mittel", b"Moderada", b"Moderata"],
	// German "Dicht": "Uppig" needs a capital U-umlaut the font table notes do not list.
	[b"Lush", b"Luxuriante", b"Dicht", b"Exuberante", b"Rigogliosa"],
	// Quality levels. French High is "Haut": the font table notes list no capital E-acute for "\xc9lev\xe9".
	[b"Low", b"Bas", b"Niedrig", b"Bajo", b"Basso"],
	[b"Medium", b"Moyen", b"Mittel", b"Medio", b"Medio"],
	[b"High", b"Haut", b"Hoch", b"Alto", b"Alto"],
	// [PC] Culling: hiding what is out of view or behind walls.
	[b"Culling", b"Masquage", b"Culling", b"Ocultaci\x9cn", b"Occultamento"],
	// [PC] Global illumination: light bounced off nearby surfaces (viewer/post.rs `fs_gi`).
	[b"Global illumination", b"Illumination globale", b"Globale Beleuchtung", b"Iluminaci\x9cn global", b"Illuminazione globale"],
	// [PC] Waves: the sea's rolling waves (viewer/waterfx.rs `geo_look`).
	[b"Waves", b"Vagues", b"Wellen", b"Olas", b"Onde"],
	// [PC] Draw distance: how far the world is drawn (viewer/fardraw.rs).
	[b"Draw distance", b"Distance d'affichage", b"Sichtweite", b"Distancia de dibujado", b"Distanza visiva"],
	// [PC] Volumetric fog and clouds (viewer/volfog.rs, viewer/volclouds.rs). Up to ~21 letters like "Distancia de dibujado".
	[b"Volumetric fog", b"Brouillard volumique", b"Volumetrischer Nebel", b"Niebla volum\x8ctrica", b"Nebbia volumetrica"],
	[b"Volumetric clouds", b"Nuages volumiques", b"Volumetrische Wolken", b"Nubes volum\x8ctricas", b"Nuvole volumetriche"],
];

/// The string pool slot of new text `k` (`NEW_IDS` order): `POOL`, `POOL2`, then `POOL3`.
pub(super) fn pool_slot(k: usize) -> u32 {
	match k {
		_ if k < POOL_SLOTS => POOL + 0x40 * k as u32,
		_ if k < POOL_SLOTS + POOL2_SLOTS => POOL2 + 0x40 * (k - POOL_SLOTS) as u32,
		_ => POOL3 + 0x40 * (k - POOL_SLOTS - POOL2_SLOTS) as u32,
	}
}

pub(super) fn text_for(k: usize, lang: usize) -> Vec<u8> {
	TEXTS[k][lang].to_vec()
}

/// The game's language as an index into the text arrays (0 En .. 4 It).
pub(super) fn lang_of(mem: &mut Mem) -> usize {
	Lang::from_word(mem.rd32(LANG_WORD)).map_or(0, |l| l as usize)
}
