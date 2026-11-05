//! Dev-only demo menu (`RAC_UI_DEMO`): a static options screen with sample rows (no game, no real values) so the picture of the
//! modern UI (fonts, panels, blur, glyphs) can be looked at in a build before the game is wired to it. The text is sample text, a
//! few rows in five languages. `RAC_UI_DEMO=1` shows the first tab; comma-separated words choose more: `tab=N`, `lang=en|fr|de|es|it`
//! (or 0..4), `focus=N` (row number), `peek`, `pad` (pad hints), `letters` / `switch` (button glyph sets). Parsed once.
use super::input::{Device, GlyphSet};
use super::kit::TextMeasure;
use super::rows::*;
use super::Ui;
use std::sync::OnceLock;

const fn t(s: &'static str) -> T5 {
	[s; LANGS]
}

const NO: &[T5] = &[];

const fn row(id: &'static str, kind: RowKind, label: T5, desc: T5, values: &'static [T5], def: i32, cost: Cost, needs: Option<Needs>) -> UiRow {
	UiRow { id, kind, label, desc, values, def, cost, needs, note: None }
}

const LIGHT_VALUES: &[T5] = &[["Classic", "Classique", "Klassisch", "Clasica", "Classica"], ["Enhanced", "Amelioree", "Verbessert", "Mejorada", "Migliorata"]];
const QUALITY: &[T5] = &[["Off", "Desactive", "Aus", "No", "No"], ["Low", "Faible", "Niedrig", "Baja", "Bassa"], ["Medium", "Moyen", "Mittel", "Media", "Media"], ["High", "Eleve", "Hoch", "Alta", "Alta"]];
const STYLE: &[T5] = &[t("Modern"), t("Classic")];
const GLYPHS: &[T5] = &[t("PlayStation"), t("Letters"), t("Letters (Switch)")];
const LIGHT: Option<Needs> = Some(Needs { row: "lighting", min: 1, max: i32::MAX });
const PCT: RowKind = RowKind::Slider { min: 0, max: 100, step: 10, unit: "%" };
const TOG: RowKind = RowKind::Toggle;
const CH: RowKind = RowKind::Choice;
const DESC: T5 = [
	"What this setting does, in a sentence or two, and what it costs in speed.",
	"Ce que fait ce r\u{e9}glage, en une phrase ou deux, et ce qu'il co\u{fb}te en vitesse.",
	"Was diese Einstellung bewirkt und wie viel Geschwindigkeit sie kostet.",
	"Qu\u{e9} hace este ajuste, en una o dos frases, y cu\u{e1}nta velocidad cuesta. \u{bf}Lo activas? \u{a1}Pru\u{e9}balo!",
	"Cosa fa questa impostazione, in una frase o due, e quanta velocit\u{e0} costa.",
];

static GFX: [UiRow; 6] = [
	row("preset", RowKind::Preset, ["Preset", "Pr\u{e9}r\u{e9}glage", "Voreinstellung", "Ajuste", "Preset"], DESC, NO, 0, Cost::None, None),
	row("lighting", CH, ["Lighting", "\u{c9}clairage", "Beleuchtung", "Iluminaci\u{f3}n", "Illuminazione"], DESC, LIGHT_VALUES, 0, Cost::High, None),
	row("shadows", CH, ["Shadows", "Ombres", "Schatten", "Sombras", "Ombre"], DESC, QUALITY, 3, Cost::Medium, LIGHT),
	row("bloom", TOG, ["Bloom", "Flou lumineux", "Blendeffekt", "Resplandor", "Bagliore"], DESC, NO, 1, Cost::Low, LIGHT),
	row("reflect", CH, ["Reflections", "R\u{e9}flexions", "Spiegelungen in Gew\u{e4}ssern", "Reflejos", "Riflessi"], DESC, QUALITY, 2, Cost::High, LIGHT),
	row("scale", PCT, ["Render scale", "\u{c9}chelle de rendu", "Renderaufl\u{f6}sung", "Escala de render", "Scala di rendering"], DESC, NO, 100, Cost::Medium, None),
];
static VIEW: [UiRow; 2] = [
	row("fov", RowKind::Slider { min: 60, max: 110, step: 5, unit: "\u{b0}" }, ["Field of view", "Champ de vision", "Sichtfeld", "Campo de visi\u{f3}n", "Campo visivo"], DESC, NO, 90, Cost::None, None),
	row("vsync", TOG, ["Vertical sync", "Synchro verticale", "Vertikale Synchronisation", "Sincron\u{ed}a vertical", "Sincronia verticale"], DESC, NO, 1, Cost::None, None),
];
static CAM: [UiRow; 3] = [
	row("mode", CH, ["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"], DESC, &[t("Classic"), t("Static"), t("Smooth")], 2, Cost::None, None),
	row("sens", PCT, ["Mouse speed", "Vitesse de la souris", "Mausgeschwindigkeit", "Velocidad del rat\u{f3}n", "Velocit\u{e0} del mouse"], DESC, NO, 50, Cost::None, None),
	row("invert", TOG, ["Invert vertical", "Inverser l'axe vertical", "Vertikal umkehren", "Invertir vertical", "Inverti verticale"], DESC, NO, 0, Cost::None, None),
];
static AUDIO: [UiRow; 2] = [
	row("music", PCT, ["Music", "Musique", "Musik", "M\u{fa}sica", "Musica"], DESC, NO, 80, Cost::None, None),
	row("sfx", PCT, ["Sound effects", "Effets sonores", "Soundeffekte", "Efectos de sonido", "Effetti sonori"], DESC, NO, 100, Cost::None, None),
];
static SYS_A: [UiRow; 2] = [
	row("menustyle", CH, ["Menu style", "Style des menus", "Men\u{fc}stil", "Estilo de men\u{fa}s", "Stile dei menu"], DESC, STYLE, 0, Cost::None, None),
	row("glyphs", CH, ["Button glyphs", "Symboles des boutons", "Tastensymbole", "S\u{ed}mbolos de botones", "Simboli dei pulsanti"], DESC, GLYPHS, 0, Cost::None, None),
];

static GFX_SECS: [UiSection; 2] = [UiSection { title: ["Look", "Aspect", "Darstellung", "Aspecto", "Aspetto"], rows: &GFX }, UiSection { title: t("View"), rows: &VIEW }];
static CAM_SECS: [UiSection; 1] = [UiSection { title: t("Camera"), rows: &CAM }];
static AUDIO_SECS: [UiSection; 1] = [UiSection { title: t("Volume"), rows: &AUDIO }];
static SYS_SECS: [UiSection; 1] = [UiSection { title: t("Menus"), rows: &SYS_A }];
static TABS: [UiTab; 4] = [
	UiTab { id: "gfx", name: ["Graphics", "Graphismes", "Grafik", "Gr\u{e1}ficos", "Grafica"], sections: &GFX_SECS },
	UiTab { id: "cam", name: ["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"], sections: &CAM_SECS },
	UiTab { id: "audio", name: ["Audio", "Audio", "Ton", "Audio", "Audio"], sections: &AUDIO_SECS },
	UiTab { id: "sys", name: ["System", "Syst\u{e8}me", "System", "Sistema", "Sistema"], sections: &SYS_SECS },
];
static PRESETS: [UiPresetDef; 3] = [
	UiPresetDef { name: QUALITY[1], sets: &[("lighting", 0), ("shadows", 0), ("bloom", 0), ("reflect", 0)] },
	UiPresetDef { name: QUALITY[2], sets: &[("lighting", 1), ("shadows", 2), ("bloom", 1), ("reflect", 1)] },
	UiPresetDef { name: QUALITY[3], sets: &[("lighting", 1), ("shadows", 3), ("bloom", 1), ("reflect", 2)] },
];
static NEEDS: [NeedsRule; 1] = [NeedsRule {
	row: "lighting",
	set_to: 1,
	banner: ["Most effects need Enhanced lighting", "La plupart des effets demandent l'\u{e9}clairage am\u{e9}lior\u{e9}", "Die meisten Effekte brauchen verbesserte Beleuchtung", "La mayor\u{ed}a de los efectos necesitan iluminaci\u{f3}n mejorada", "Quasi tutti gli effetti richiedono l'illuminazione migliorata"],
	detail: ["This only shows with Lighting set to Enhanced. Lighting is {} now.", "Visible seulement avec l'\u{e9}clairage am\u{e9}lior\u{e9}. L'\u{e9}clairage est {} pour l'instant.", "Das ist nur mit verbesserter Beleuchtung zu sehen. Die Beleuchtung steht auf {}.", "Solo se ve con la iluminaci\u{f3}n mejorada. Ahora est\u{e1} en {}.", "Si vede solo con l'illuminazione migliorata. Ora \u{e8} {}."],
	button: ["Switch Lighting to Enhanced", "Passer \u{e0} l'\u{e9}clairage am\u{e9}lior\u{e9}", "Auf verbesserte Beleuchtung umstellen", "Cambiar a iluminaci\u{f3}n mejorada", "Passa all'illuminazione migliorata"],
}];
static NOTICES: [Notice; 1] = [Notice { row: "menustyle", value: 1, text: t("Classic menus from the next time") }];

pub static MENU: UiMenu = UiMenu {
	tabs: &TABS,
	presets: &PRESETS,
	preset_reset: 2,
	preset_also: &[],
	needs: &NEEDS,
	notices: &NOTICES,
	chrome: UiChrome {
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
		off_on: [["Off", "Non", "Aus", "No", "No"], ["On", "Oui", "Ein", "S\u{ed}", "S\u{ec}"]],
		custom: ["Custom", "Personnalis\u{e9}", "Eigene", "Personal", "Personale"],
	},
};

pub fn menu() -> &'static UiMenu {
	&MENU
}

/// What `RAC_UI_DEMO` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DemoSpec {
	pub tab: usize,
	pub lang: usize,
	pub focus: usize,
	pub peek: bool,
	pub pad: bool,
	pub glyphs: u8,
}

/// Parses the value of `RAC_UI_DEMO`; None = off ("", "0", "off").
pub fn parse(v: &str) -> Option<DemoSpec> {
	let v = v.trim();
	if v.is_empty() || v == "0" || v.eq_ignore_ascii_case("off") {
		return None;
	}
	let mut s = DemoSpec::default();
	for part in v.split(',').map(str::trim) {
		match part.split_once('=') {
			Some(("tab", n)) => s.tab = n.parse().unwrap_or(0),
			Some(("focus", n)) => s.focus = n.parse().unwrap_or(0),
			Some(("lang", l)) => s.lang = ["en", "fr", "de", "es", "it"].iter().position(|x| x.eq_ignore_ascii_case(l)).or_else(|| l.parse().ok()).unwrap_or(0).min(LANGS - 1),
			None if part == "peek" => s.peek = true,
			None if part == "pad" => s.pad = true,
			None if part == "letters" => s.glyphs = 1,
			None if part == "switch" => s.glyphs = 2,
			_ => {}
		}
	}
	Some(s)
}

/// The demo request from the environment (read once).
pub fn enabled() -> Option<DemoSpec> {
	static SPEC: OnceLock<Option<DemoSpec>> = OnceLock::new();
	*SPEC.get_or_init(|| std::env::var("RAC_UI_DEMO").ok().and_then(|v| parse(&v)))
}

/// The demo screen, set up as the spec says.
pub fn new_ui(spec: DemoSpec, measure: Box<dyn TextMeasure + Send>) -> Ui {
	let mut ui = Ui::new(menu(), spec.lang, measure);
	ui.begin(false, None);
	let m = &mut ui.model;
	m.set_tab(spec.tab.min(m.menu.tabs.len() - 1));
	let rows = m.cur_rows();
	m.focus = (rows.start + spec.focus).min(rows.end.saturating_sub(1));
	m.peek = spec.peek;
	m.device = if spec.pad { Device::Pad } else { Device::Keyboard };
	m.glyphs = GlyphSet::from_index(spec.glyphs);
	ui
}
