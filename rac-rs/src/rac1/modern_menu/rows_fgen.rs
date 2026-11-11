//! The Frame Generation row's help text and dependency (lane fglive, user 2026-11-08: "choose fg according to the upscaler ure
//! using ... greyed out and not switchable without an upscaler"). The row itself sits in `rows_upscale.rs` (Graphics ->
//! Resolution); its rules live in the engine (`rac_engine::viewer::fgen_follow`): the values on offer follow the running Upscaler,
//! the row is hidden while the Upscaler is Off (`NEEDS`, as Upscale quality; user 2026-11-08: hidden, not dimmed), and its note says
//! "Applies at once" or "Applies after a restart" (engine follow.rs `refresh`).
use rac_engine::ui::{Needs, NeedsRule, T5};

/// Hidden while the Upscaler is Off (its rule: `NEEDS_RULE`).
pub const NEEDS: Needs = Needs { row: "upscaler", min: 1, max: i32::MAX };

/// The row's box while the Upscaler is Off (review 2): the button sets the Upscaler of the frame generation's maker (engine
/// follow.rs `fix_upscaler`: DLSS for DLSS 2x..4x, FSR for FSR 2x; nothing chosen: the maker this run started, else the first
/// upscaler this PC runs), and names it (`{}`), so the chosen frame generation comes back.
pub const NEEDS_RULE: NeedsRule = NeedsRule {
	row: "upscaler",
	set_to: rac_engine::viewer::upscale_choice::FFX_VALUE,
	for_row: "framegen",
	fix: Some(rac_engine::viewer::fgen_follow::fix_upscaler),
	banner: [""; 5],
	detail: [
		"Needs the Upscaler of its maker. Upscaler is {} now.",
		"Exige la mise \u{e0} l'\u{e9}chelle de son fabricant. Elle est sur {}.",
		"Braucht die Hochskalierung des Herstellers. Sie steht auf {}.",
		"Necesita el escalado de su fabricante. Ahora est\u{e1} en {}.",
		"Richiede l'upscaling del suo produttore. Ora \u{e8} su {}.",
	],
	// `{}` = the Upscaler value the button sets (engine model_edit.rs `fix_text`).
	button: [
		"Switch the Upscaler to {}",
		"Passer la mise \u{e0} l'\u{e9}chelle sur {}",
		"Hochskalierung auf {} stellen",
		"Cambiar el escalado a {}",
		"Imposta l'upscaling su {}",
	],
};

/// What it does, that it follows the Upscaler, the pause under black bars.
pub const DESC: T5 = [
	"Adds generated frames for smoother motion; a little more input lag. Follows the Upscaler (DLSS or FSR). Paused with black bars (Aspect ratio) or a non-Desktop fullscreen Resolution.",
	"Ajoute des images g\u{e9}n\u{e9}r\u{e9}es, plus fluide ; l\u{e9}ger retard. Suit la mise \u{e0} l'\u{e9}chelle (DLSS ou FSR). En pause avec bandes noires (Format d'image) ou plein \u{e9}cran hors Bureau.",
	"Mehr Bilder, fl\u{fc}ssiger; etwas Verz\u{f6}gerung. Folgt der Hochskalierung (DLSS oder FSR). Pause bei schwarzen Balken oder Vollbild au\u{df}er Desktop.",
	"M\u{e1}s fluidez, algo de retraso. Sigue al escalado (DLSS o FSR). Pausa con franjas negras o pantalla completa fuera de Escritorio.",
	"Frame generati: pi\u{f9} fluido, un po' di ritardo. Segue l'upscaling (DLSS o FSR). In pausa con bande nere (Proporzioni) o schermo intero non Desktop.",
];

#[cfg(test)]
mod tests {
	use super::*;
	use rac_engine::viewer::fgen_follow as f;

	/// The row: hidden by the Upscaler row, help text in 5 languages that names both makers and keeps the black-bars sentence.
	#[test]
	fn the_framegen_row_follows_the_upscaler() {
		let r = super::super::rows_upscale::UPSCALING.iter().find(|r| r.id == "framegen").unwrap();
		assert_eq!(r.needs, Some(NEEDS));
		assert_eq!(r.desc, DESC);
		assert!(DESC.iter().all(|s| s.contains("DLSS") && s.contains("FSR") && s.contains('(')));
		assert!(f::NEEDS_UPSCALER_NOTE.iter().chain(f::AT_ONCE_NOTE.iter()).chain(f::RESTART_NOTE.iter()).all(|s| !s.is_empty() && s.len() <= 70));
		assert!(f::NEEDS_UPSCALER_NOTE.iter().all(|s| s.contains("DLSS") && s.contains("FSR")));
		// Review 2: its own box, the button names the Upscaler it sets; the ES text names fullscreen and Desktop like EN.
		assert!(NEEDS_RULE.detail.iter().all(|s| s.matches("{}").count() == 1));
		assert!(NEEDS_RULE.button.iter().all(|s| s.matches("{}").count() == 1 && s.len() <= 48));
		assert!(DESC[3].contains("pantalla completa") && DESC[3].contains("Escritorio"));
	}

	/// The menu model uses the row's own rule (not the Upscale quality one) and its button sets the frame generation's maker.
	#[test]
	fn the_framegen_box_switches_to_its_makers_upscaler() {
		let menu = super::super::menu();
		assert_eq!(menu.validate(), Vec::<String>::new());
		let m = rac_engine::ui::model::Model::new(menu, 0);
		let (fg, q) = (m.row_index("framegen").unwrap(), m.row_index("upscale_q").unwrap());
		assert_eq!(m.rule(fg).map(|r| r.for_row), Some("framegen"));
		assert_eq!(m.rule(q).map(|r| r.for_row), Some(""), "Upscale quality keeps the shared rule");
		let up = rac_engine::viewer::upscale_choice::DLSS_VALUE;
		assert_eq!(f::upscaler_for(4, None, &|_| true), up);
	}
}
