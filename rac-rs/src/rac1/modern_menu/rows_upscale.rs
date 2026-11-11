//! The Graphics tab's Resolution section (lane ndlss, stage 5): Upscaler and Upscale quality; Frame Generation (lane fgen, S4). No
//! RAM byte: the choices live in the engine (`rac_engine::viewer::upscale_choice`, options file keys `upscaler` / `upscale_q`;
//! `rac_engine::viewer::fgen_choice`, key `framegen`), set and published by `retail/pcext/ui_bridge_more.rs`. An upscaler change
//! applies at the next frame; Frame Generation follows it, live where the running maker allows (rows_fgen.rs, the row's note). Classic copies: Upscaler in
//! `retail/pcmore.rs`, Frame Generation in `retail/pcfgen.rs`; Upscale quality is modern-only. No DLSS preset or MSAA row since
//! 2026-11-08 (user: "too many options"): DLSS preset stays the engine's options file key `dlss_preset` (Auto = K; the launcher's
//! row), MSAA is the pack's mode (engine scene_msaa.rs: R&C1 off while DLSS / FSR runs, else 4x).
use super::costs;
use super::text::*;
use rac_engine::ui::{Cost, Needs, NeedsRule, UiRow, T5};

/// Off, the newest FSR of the PC's FidelityFX runtime (the engine shows its version, "FSR 3.1" / "FSR 4.1", engine ui/live.rs,
/// once known; the name below until then) and DLSS (lane dlss2: "DLSS 4.5" once NVIDIA's NGX reported the model). Product names:
/// the same in all 5 languages. An upscaler this PC cannot run (no DLLs / GPU / runtime refused) is not offered (engine choice.rs
/// `set_runs` -> ui/live.rs `offer`) and a file that chose it runs Off. FSR 1 was removed in lane nfsr1 (`upscaler=fsr1` = FSR).
const KIND_VALUES: &[T5] = &[OFF, t("FSR 3 / 4"), t("DLSS")];
/// The share of the window each side is drawn at (slot.rs `QUALITY`: 1, 1.5, 1.7, 2, 3); with DLSS the engine names the first one "DLAA (100%)" (choice.rs).
const Q_VALUES: &[T5] = &[
	["Native AA (100%)", "AA natif (100 %)", "Natives AA (100 %)", "AA nativo (100 %)", "AA nativo (100%)"],
	["Quality (67%)", "Qualit\u{e9} (67 %)", "Qualit\u{e4}t (67 %)", "Calidad (67 %)", "Qualit\u{e0} (67%)"],
	["Balanced (59%)", "\u{c9}quilibr\u{e9} (59 %)", "Ausgewogen (59 %)", "Equilibrado (59 %)", "Bilanciato (59%)"],
	["Performance (50%)", "Performance (50 %)", "Leistung (50 %)", "Rendimiento (50 %)", "Prestazioni (50%)"],
	["Ultra performance (33%)", "Ultra performance (33 %)", "Ultra-Leistung (33 %)", "Ultra rendimiento (33 %)", "Ultra prestazioni (33%)"],
];

/// Frame Generation's values in `fgen_choice::KINDS` order: Off, FSR 2x (AMD FidelityFX frame generation), DLSS 2x / 3x / 4x
/// (NVIDIA DLSS frame generation, S3). Product names, the same in all 5 languages; a value this PC cannot run is not offered (engine
/// fgen/choice.rs `set_runs` -> ui/live.rs `offer`), so with no DLLs the row offers Off only.
pub const FG_VALUES: &[T5] = &[OFF, t("FSR 2x"), t("DLSS 2x"), t("DLSS 3x"), t("DLSS 4x")];

pub static UPSCALING: [UiRow; 3] = [
	row(
		"upscaler",
		CH,
		["Upscaler", "Mise \u{e0} l'\u{e9}chelle", "Hochskalierung", "Escalado", "Upscaling"],
		[
			// The last sentence is the in-app credit NVIDIA's DLSS SDK licence asks for (supplement 7.1 b; user 2026-11-04).
			"Draws the game smaller, then scales it up to the window: faster and nearly as sharp. With Off, Render scale applies. Upscaling by AMD FidelityFX Super Resolution (FSR) and NVIDIA DLSS.",
			"Dessine le jeu plus petit, puis l'agrandit \u{e0} la fen\u{ea}tre : plus rapide et presque aussi net. Avec Non, la taille du rendu s'applique. Mise \u{e0} l'\u{e9}chelle par AMD FidelityFX Super Resolution (FSR) et NVIDIA DLSS.",
			"Zeichnet das Spiel kleiner und skaliert es dann auf das Fenster hoch: schneller und fast so scharf. Bei Aus gilt die Render-Skalierung. Hochskalierung durch AMD FidelityFX Super Resolution (FSR) und NVIDIA DLSS.",
			"Dibuja el juego m\u{e1}s peque\u{f1}o y luego lo ampl\u{ed}a a la ventana: m\u{e1}s r\u{e1}pido y casi igual de n\u{ed}tido. Con No se aplica la escala de render. Escalado de AMD FidelityFX Super Resolution (FSR) y NVIDIA DLSS.",
			"Disegna il gioco pi\u{f9} piccolo, poi lo ingrandisce alla finestra: pi\u{f9} veloce e quasi altrettanto nitido. Con No vale la scala di rendering. Upscaling di AMD FidelityFX Super Resolution (FSR) e NVIDIA DLSS.",
		],
		KIND_VALUES,
		0,
		costs::UPSCALER,
		None,
	),
	row(
		"upscale_q",
		CH,
		["Upscale quality", "Qualit\u{e9} de mise \u{e0} l'\u{e9}chelle", "Skalierungsqualit\u{e4}t", "Calidad de escalado", "Qualit\u{e0} upscaling"],
		[
			"The size the game is drawn at before the upscaler, in percent of the window. Lower is faster and softer.",
			"Taille du rendu avant mise \u{e0} l'\u{e9}chelle, en % de la fen\u{ea}tre. Plus bas : plus rapide, plus doux.",
			"Die Gr\u{f6}\u{df}e, in der das Spiel vor dem Hochskalieren gezeichnet wird, in Prozent des Fensters. Weniger ist schneller und weicher.",
			"El tama\u{f1}o al que se dibuja el juego antes del escalado, en porcentaje de la ventana. Menos es m\u{e1}s r\u{e1}pido y m\u{e1}s suave.",
			"La dimensione a cui si disegna il gioco prima dell'upscaling, in percentuale della finestra. Meno \u{e8} pi\u{f9} veloce e pi\u{f9} morbido.",
		],
		Q_VALUES,
		rac_engine::viewer::upscale_choice::DEFAULT_Q as i32,
		costs::UPSCALE_Q,
		Some(Needs { row: "upscaler", min: 1, max: i32::MAX }),
	),
	UiRow {
		note: Some([
			"Applies after a restart",
			"S'applique apr\u{e8}s un red\u{e9}marrage",
			"Gilt nach einem Neustart",
			"Se aplica al reiniciar",
			"Si applica dopo il riavvio",
		]),
		..row(
			"framegen",
			CH,
			["Frame Generation", "G\u{e9}n\u{e9}ration d'images", "Frame-Generierung", "Generaci\u{f3}n de fotogramas", "Generazione di frame"],
			super::rows_fgen::DESC, // lane fglive: follows the Upscaler (rows_fgen.rs)
			FG_VALUES,
			0,
			Cost::None,
			Some(super::rows_fgen::NEEDS), // lane fglive: hidden while the Upscaler is Off
		)
	},
];

/// Upscale quality is hidden while the Upscaler is Off. No banner over the tab (empty `banner`): the default
/// Off is not a problem to point at.
pub const NEEDS_UPSCALER: NeedsRule = NeedsRule {
	row: "upscaler",
	set_to: rac_engine::viewer::upscale_choice::FFX_VALUE,
	for_row: "",
	fix: None,
	banner: t(""),
	detail: [
		"Applies while an upscaler runs. Upscaler is {} now.",
		"S'applique avec une mise \u{e0} l'\u{e9}chelle active. Elle est sur {} pour l'instant.",
		"Gilt, solange hochskaliert wird. Die Hochskalierung steht auf {}.",
		"Se aplica mientras hay escalado. Ahora est\u{e1} en {}.",
		"Vale mentre l'upscaling \u{e8} attivo. Ora \u{e8} su {}.",
	],
	// Generic: the button sets FSR, or DLSS where FSR is not offered (engine model_edit.rs `fix_value`).
	button: [
		"Switch the Upscaler on",
		"Activer la mise \u{e0} l'\u{e9}chelle",
		"Hochskalierung einschalten",
		"Activar el escalado",
		"Attiva l'upscaling",
	],
};

#[cfg(test)]
mod tests {
	use super::*;
	use rac_engine::viewer::upscale_choice::{DLSS_VALUE, FFX_VALUE, KEY, KINDS};

	/// Lane nfsr1: Off, FSR, DLSS in the engine's order (choice.rs `KINDS`); FSR 1 named nowhere, in no language.
	#[test]
	fn the_upscaler_row_offers_off_fsr_and_dlss_and_no_fsr1() {
		let r = &UPSCALING[0];
		assert_eq!(r.id, KEY);
		assert_eq!(r.values.len(), 3);
		// 2026-11-08: Upscaler, Upscale quality, Frame Generation; no DLSS preset or MSAA row (user: "too many options").
		assert_eq!(UPSCALING.each_ref().map(|r| r.id), [KEY, "upscale_q", "framegen"]);
		assert_eq!(r.values.len(), KINDS.len(), "one value per engine kind");
		assert_eq!(r.values[0], OFF);
		assert!(r.values[FFX_VALUE as usize].iter().all(|s| s.starts_with("FSR")));
		assert!(r.values[DLSS_VALUE as usize].iter().all(|s| *s == "DLSS"));
		let rows = UPSCALING.iter().flat_map(|r| r.values.iter().chain([&r.label, &r.desc]));
		for t in rows.chain([&NEEDS_UPSCALER.detail, &NEEDS_UPSCALER.button]) {
			assert!(t.iter().all(|s| !s.contains("FSR 1")), "{t:?}");
		}
		assert_eq!(NEEDS_UPSCALER.set_to, FFX_VALUE);
		assert!(NEEDS_UPSCALER.button.iter().all(|s| !s.is_empty() && !s.contains("FSR")), "generic: the fix may pick DLSS");
		// NVIDIA's DLSS licence: the app credits the SDKs it uses (supplement 7.1 b), in every language.
		let up = UPSCALING.iter().find(|r| r.id == "upscaler").unwrap();
		assert!(up.desc.iter().all(|s| s.contains("NVIDIA DLSS") && s.contains("AMD FidelityFX Super Resolution")), "{:?}", up.desc);
	}
}
