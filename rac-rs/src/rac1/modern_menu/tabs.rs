//! The R&C1 modern menu table: tabs, presets, the Lighting dependency, notices and the fixed words, and the `Pack::ui` hooks.
use super::rows_camera::{AIMING, CAMERA, MENUS, MOUSE, NEEDS_LOCKON, VENDOR};
use super::rows_display::{HUD, SCREEN};
use super::rows_effects::{EFFECTS, NEEDS_METAL};
use super::rows_game::GAMEPLAY;
use super::rows_graphics::{NEEDS_WATER, QUALITY, SHADING, WORLD};
use super::rows_upscale::{NEEDS_UPSCALER, UPSCALING};
#[cfg(not(target_os = "linux"))]
use super::rows_system::RENDERER;
use super::rows_system::{CONTROLLER, START};
use super::text::t;
use rac_engine::ui::{Notice, NeedsRule, UiChrome, UiMenu, UiPack, UiPresetDef, UiSection, UiTab};

#[cfg(not(target_os = "linux"))]
static DISPLAY_SECS: [UiSection; 3] = [
	UiSection { title: ["Screen", "\u{c9}cran", "Bildschirm", "Pantalla", "Schermo"], rows: &SCREEN },
	UiSection { title: t("HUD"), rows: &HUD },
	UiSection { title: ["Renderer", "Rendu", "Renderer", "Motor gr\u{e1}fico", "Renderer"], rows: &RENDERER },
];
/// Linux has no Renderer section (the choice is Vulkan only there), like `pcctl::GSCREEN`.
#[cfg(target_os = "linux")]
static DISPLAY_SECS: [UiSection; 2] = [UiSection { title: ["Screen", "\u{c9}cran", "Bildschirm", "Pantalla", "Schermo"], rows: &SCREEN }, UiSection { title: t("HUD"), rows: &HUD }];
static GRAPHICS_SECS: [UiSection; 5] = [
	UiSection { title: ["Quality", "Qualit\u{e9}", "Qualit\u{e4}t", "Calidad", "Qualit\u{e0}"], rows: &QUALITY },
	UiSection { title: ["Resolution", "R\u{e9}solution", "Aufl\u{f6}sung", "Resoluci\u{f3}n", "Risoluzione"], rows: &UPSCALING },
	UiSection { title: ["Lighting and shadows", "Lumi\u{e8}re et ombres", "Licht und Schatten", "Luz y sombras", "Luce e ombre"], rows: &SHADING },
	UiSection { title: ["World", "Monde", "Welt", "Mundo", "Mondo"], rows: &WORLD },
	UiSection { title: ["Effects", "Effets", "Effekte", "Efectos", "Effetti"], rows: &EFFECTS },
];
static CAMERA_SECS: [UiSection; 4] = [
	UiSection { title: ["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"], rows: &CAMERA },
	UiSection { title: ["Mouse", "Souris", "Maus", "Rat\u{f3}n", "Mouse"], rows: &MOUSE },
	UiSection { title: ["Aiming", "Vis\u{e9}e", "Zielen", "Punter\u{ed}a", "Mira"], rows: &AIMING },
	UiSection { title: ["Controller", "Manette", "Controller", "Mando", "Controller"], rows: &CONTROLLER },
];
static GAME_SECS: [UiSection; 4] = [
	UiSection { title: ["Gameplay", "Jouabilit\u{e9}", "Spielablauf", "Jugabilidad", "Gameplay"], rows: &GAMEPLAY },
	UiSection { title: ["Vendor", "Vendeur", "H\u{e4}ndler", "Vendedor", "Venditore"], rows: &VENDOR },
	UiSection { title: ["Start", "D\u{e9}marrage", "Start", "Inicio", "Avvio"], rows: &START },
	UiSection { title: ["Menus", "Menus", "Men\u{fc}s", "Men\u{fa}s", "Menu"], rows: &MENUS },
];
/// Tab ids "display", "graphics", "camera" and "game" are the ones `retail::modern_swallow::Tab` opens.
static TABS: [UiTab; 4] = [
	UiTab { id: "display", name: ["Display", "Affichage", "Anzeige", "Pantalla", "Schermo"], sections: &DISPLAY_SECS },
	UiTab { id: "graphics", name: ["Graphics", "Graphismes", "Grafik", "Gr\u{e1}ficos", "Grafica"], sections: &GRAPHICS_SECS },
	UiTab { id: "camera", name: ["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"], sections: &CAMERA_SECS },
	UiTab { id: "game", name: ["Game", "Jeu", "Spiel", "Juego", "Gioco"], sections: &GAME_SECS },
];

const LOW: [(&str, i32); 18] = [("lighting", 0), ("draw", 0), ("smoothing", 0), ("hud", 1), ("ao", 1), ("shadows", 1), ("water", 1), ("particles", 1), ("sunrays", 1), ("vegetation", 0), ("gi", 0), ("waves", 0), ("fog", 0), ("clouds", 0), ("fur", 0), ("fire", 0), ("smoke", 0), ("culling", 1)];
const MEDIUM: [(&str, i32); 18] = [("lighting", 0), ("draw", 0), ("smoothing", 1), ("hud", 0), ("ao", 2), ("shadows", 2), ("water", 2), ("particles", 2), ("sunrays", 2), ("vegetation", 1), ("gi", 0), ("waves", 1), ("fog", 1), ("clouds", 1), ("fur", 1), ("fire", 1), ("smoke", 1), ("culling", 1)];
const HIGH: [(&str, i32); 18] = [("lighting", 0), ("draw", 1), ("smoothing", 1), ("hud", 0), ("ao", 3), ("shadows", 3), ("water", 3), ("particles", 3), ("sunrays", 3), ("vegetation", 1), ("gi", 0), ("waves", 1), ("fog", 3), ("clouds", 3), ("fur", 3), ("fire", 3), ("smoke", 3), ("culling", 1)];
/// The mockup's PRESETS = `assets/gfx_presets.txt` (a test compares the bytes both produce).
pub static PRESETS: [UiPresetDef; 3] = [
	UiPresetDef { name: super::text::LOW, sets: &LOW },
	UiPresetDef { name: super::text::MEDIUM, sets: &MEDIUM },
	UiPresetDef { name: super::text::HIGH, sets: &HIGH },
];

static NEEDS: [NeedsRule; 5] = [NeedsRule {
	row: "lighting",
	set_to: 1,
	banner: ["Most effects need Enhanced lighting", "La plupart des effets demandent l'\u{e9}clairage am\u{e9}lior\u{e9}", "Die meisten Effekte brauchen verbesserte Beleuchtung", "La mayor\u{ed}a de los efectos necesitan iluminaci\u{f3}n mejorada", "Quasi tutti gli effetti richiedono l'illuminazione migliorata"],
	detail: ["This only shows with Lighting set to Enhanced. Lighting is {} now.", "Visible seulement avec l'\u{e9}clairage am\u{e9}lior\u{e9}. L'\u{e9}clairage est {} pour l'instant.", "Das ist nur mit verbesserter Beleuchtung zu sehen. Die Beleuchtung steht auf {}.", "Solo se ve con la iluminaci\u{f3}n mejorada. Ahora est\u{e1} en {}.", "Si vede solo con l'illuminazione migliorata. Ora \u{e8} {}."],
	button: ["Switch Lighting to Enhanced", "Passer \u{e0} l'\u{e9}clairage am\u{e9}lior\u{e9}", "Auf verbesserte Beleuchtung umstellen", "Cambiar a iluminaci\u{f3}n mejorada", "Passa all'illuminazione migliorata"],
}, NEEDS_UPSCALER, NEEDS_WATER, NEEDS_LOCKON, NEEDS_METAL];
static NOTICES: [Notice; 1] = [Notice { row: "menustyle", value: 1, text: ["Classic menus from the next time you open Options", "Menus classiques \u{e0} la prochaine ouverture des Options", "Klassische Men\u{fc}s beim n\u{e4}chsten \u{d6}ffnen der Optionen", "Men\u{fa}s cl\u{e1}sicos la pr\u{f3}xima vez que abras Opciones", "Menu classici alla prossima apertura delle Opzioni"] }];

const CHROME: UiChrome = UiChrome {
	title: ["Paused - Options", "Pause - Options", "Pause - Optionen", "Pausa - Opciones", "Pausa - Opzioni"],
	title_front: ["Options", "Options", "Optionen", "Opciones", "Opzioni"],
	autosave: ["Saved automatically", "Enregistr\u{e9} automatiquement", "Automatisch gespeichert", "Se guarda solo", "Salvataggio automatico"],
	now: ["Now", "Actuel", "Jetzt", "Ahora", "Ora"],
	default: ["Default", "Par d\u{e9}faut", "Standard", "Original", "Predefinito"],
	speed_impact: ["Speed impact", "Impact sur la vitesse", "Leistungsbedarf", "Impacto en velocidad", "Impatto sulla velocit\u{e0}"],
	cost_names: [["Low", "Faible", "Gering", "Bajo", "Basso"], ["Medium", "Moyen", "Mittel", "Medio", "Medio"], ["High", "\u{c9}lev\u{e9}", "Hoch", "Alto", "Alto"]],
	peek_note: ["Look at the game behind", "Regarder le jeu derri\u{e8}re", "Das Spiel dahinter ansehen", "Mirar el juego de fondo", "Guarda il gioco dietro"],
	view_game: ["View game", "Voir le jeu", "Spiel ansehen", "Ver juego", "Vedi gioco"],
	back_to_menu: ["Back to the menu", "Retour au menu", "Zur\u{fc}ck zum Men\u{fc}", "Volver al men\u{fa}", "Torna al menu"],
	change: ["Change", "Modifier", "\u{c4}ndern", "Cambiar", "Cambia"],
	back: ["Back", "Retour", "Zur\u{fc}ck", "Atr\u{e1}s", "Indietro"],
	reset: ["Reset to default", "Valeur par d\u{e9}faut", "Zur\u{fc}cksetzen", "Restablecer", "Ripristina"],
	switch_tab: ["Switch tab", "Changer d'onglet", "Tab wechseln", "Cambiar pesta\u{f1}a", "Cambia scheda"],
	off_on: [super::text::OFF, super::text::ON],
	custom: ["Custom", "Personnalis\u{e9}", "Eigene", "Personal", "Personale"],
};

/// What every preset sends besides its rows: the texture filter (no row here) goes to Smooth, as the classic `presets::apply_preset`
/// does (`assets/gfx_presets.txt` has `texfilter = smooth` in all three).
static PRESET_ALSO: [(&str, i32); 1] = [("texfilter", 1)];
static MENU: UiMenu = UiMenu { tabs: &TABS, presets: &PRESETS, preset_reset: 2, preset_also: &PRESET_ALSO, needs: &NEEDS, notices: &NOTICES, chrome: CHROME };

pub fn menu() -> &'static UiMenu {
	&MENU
}

fn lang() -> usize {
	crate::retail::pcext::ui_bridge::get("lang").unwrap_or(0).clamp(0, 4) as usize
}

/// `Pack::ui` for R&C1 (bound in pack_bind.rs once the host is wired, S3c).
pub static PACK: UiPack = UiPack { menu, get: crate::retail::pcext::ui_bridge::get, set: crate::retail::pcext::ui_bridge::set, lang, glyphs: crate::retail::glyphs::byte };
