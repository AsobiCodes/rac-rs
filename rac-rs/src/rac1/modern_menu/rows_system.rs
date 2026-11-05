//! The rows that belong to a module's own setter (`retail/pcext/ui_bridge_more.rs`): the Renderer section of the Display tab (not on
//! Linux), the Controller section of the Camera tab and the Start section of the Game tab. Defaults equal the option defaults.
use super::text::*;
use rac_engine::ui::{Cost, UiRow, T5};

const RENDERERS: &[T5] = &[t("Vulkan"), t("DirectX 12")];
const NORMAL_REVERSED: &[T5] = &[["Normal", "Normal", "Normal", "Normal", "Normale"], ["Reversed", "Invers\u{e9}", "Invertiert", "Invertido", "Invertito"]];
const DEAD_ZONES: &[T5] = &[HIGH, MEDIUM, LOW];
const GLYPH_SETS: &[T5] = &[t("PlayStation"), ["Letters", "Lettres", "Buchstaben", "Letras", "Lettere"], ["Letters (Switch)", "Lettres (Switch)", "Buchst. (Switch)", "Letras (Switch)", "Lettere (Switch)"]];

/// The Renderer section (Display tab). The change is stored at once and used from the next start.
#[cfg(not(target_os = "linux"))]
pub static RENDERER: [UiRow; 1] = [UiRow {
	note: Some([
		"Applies after a restart",
		"S'applique apr\u{e8}s un red\u{e9}marrage",
		"Gilt nach einem Neustart",
		"Se aplica al reiniciar",
		"Si applica dopo il riavvio",
	]),
	..row(
		"renderer",
		CH,
		["Renderer", "Rendu", "Renderer", "Motor gr\u{e1}fico", "Renderer"],
		[
			"The graphics system the game draws with. Vulkan is the default. Try DirectX 12 if the game does not start or stutters.",
			"Le syst\u{e8}me graphique utilis\u{e9} par le jeu. Vulkan est celui par d\u{e9}faut. Essayez DirectX 12 si le jeu ne d\u{e9}marre pas ou saccade.",
			"Das Grafiksystem, mit dem das Spiel zeichnet. Vulkan ist der Standard. Probiere DirectX 12, wenn das Spiel nicht startet oder ruckelt.",
			"El sistema gr\u{e1}fico con el que dibuja el juego. Vulkan es el normal. Prueba DirectX 12 si el juego no arranca o va a tirones.",
			"Il sistema grafico con cui disegna il gioco. Vulkan \u{e8} quello normale. Prova DirectX 12 se il gioco non parte o scatta.",
		],
		RENDERERS,
		0,
		Cost::None,
		None,
	)
}];

/// The Controller section (Camera tab).
pub static CONTROLLER: [UiRow; 5] = [
	row(
		"leftright",
		CH,
		["Left / Right", "Gauche / Droite", "Links / Rechts", "Izquierda / Derecha", "Sinistra / Destra"],
		[
			"Turns the camera sideways with the right stick. Reversed swaps the direction.",
			"Tourne la cam\u{e9}ra sur le c\u{f4}t\u{e9} avec le stick droit. Invers\u{e9} change le sens.",
			"Dreht die Kamera mit dem rechten Stick zur Seite. Invertiert kehrt die Richtung um.",
			"Gira la c\u{e1}mara hacia los lados con el stick derecho. Invertido cambia el sentido.",
			"Ruota la telecamera di lato con lo stick destro. Invertito scambia la direzione.",
		],
		NORMAL_REVERSED,
		0,
		Cost::None,
		None,
	),
	row(
		"updown",
		CH,
		["Up / Down", "Haut / Bas", "Oben / Unten", "Arriba / Abajo", "Su / Gi\u{f9}"],
		[
			"Tilts the camera up and down with the right stick. Reversed swaps the direction.",
			"Incline la cam\u{e9}ra vers le haut et le bas avec le stick droit. Invers\u{e9} change le sens.",
			"Neigt die Kamera mit dem rechten Stick nach oben und unten. Invertiert kehrt die Richtung um.",
			"Inclina la c\u{e1}mara hacia arriba y abajo con el stick derecho. Invertido cambia el sentido.",
			"Inclina la telecamera su e gi\u{f9} con lo stick destro. Invertito scambia la direzione.",
		],
		NORMAL_REVERSED,
		0,
		Cost::None,
		None,
	),
	row(
		"stickdz",
		CH,
		["Stick dead zone", "Zone morte du stick", "Stick-Totzone", "Zona muerta del stick", "Zona morta dello stick"],
		[
			"How far a stick must move before the game notices it. High ignores small drifts of a worn stick.",
			"De combien le stick doit bouger avant que le jeu le remarque. \u{c9}lev\u{e9}e ignore les petites d\u{e9}rives d'un stick us\u{e9}.",
			"Wie weit sich ein Stick bewegen muss, bevor das Spiel ihn bemerkt. Hoch ignoriert kleines Driften eines abgenutzten Sticks.",
			"Cu\u{e1}nto debe moverse un stick para que el juego lo note. Alta ignora la deriva leve de un stick gastado.",
			"Quanto deve muoversi uno stick prima che il gioco lo noti. Alta ignora le piccole derive di uno stick consumato.",
		],
		DEAD_ZONES,
		0,
		Cost::None,
		None,
	),
	row(
		"wheel",
		TOG,
		["Quick Select mouse", "Roue par souris", "Schnellwahl per Maus", "Rueda con rat\u{f3}n", "Ruota con mouse"],
		[
			"Opens the Quick Select ring with the mouse and picks a weapon by moving the mouse.",
			"Ouvre la roue de s\u{e9}lection rapide \u{e0} la souris et choisit une arme en la d\u{e9}pla\u{e7}ant.",
			"\u{d6}ffnet den Schnellwahl-Ring mit der Maus und w\u{e4}hlt eine Waffe per Mausbewegung.",
			"Abre la rueda de selecci\u{f3}n r\u{e1}pida con el rat\u{f3}n y elige un arma moviendo el rat\u{f3}n.",
			"Apre la ruota di selezione rapida col mouse e sceglie un'arma muovendo il mouse.",
		],
		NO,
		0,
		Cost::None,
		None,
	),
	row(
		"glyphs",
		CH,
		["Button glyphs", "Symboles des boutons", "Tastensymbole", "Iconos de botones", "Simboli dei tasti"],
		[
			"The button pictures shown in prompts and menus. Letters shows A B X Y, the Switch set swaps the letters like a Switch pad.",
			"Les images de boutons affich\u{e9}es dans les messages et les menus. Lettres montre A B X Y, le style Switch \u{e9}change les lettres comme une manette Switch.",
			"Die Tastenbilder in Hinweisen und Men\u{fc}s. Buchstaben zeigt A B X Y, der Switch-Satz tauscht die Buchstaben wie ein Switch-Controller.",
			"Las im\u{e1}genes de botones en avisos y men\u{fa}s. Letras muestra A B X Y, el estilo Switch intercambia las letras como un mando de Switch.",
			"Le immagini dei tasti nei messaggi e nei menu. Lettere mostra A B X Y, il set Switch scambia le lettere come un controller Switch.",
		],
		GLYPH_SETS,
		0,
		Cost::None,
		None,
	),
];

/// The Start section (Game tab).
pub static START: [UiRow; 2] = [
	row(
		"intro",
		TOG,
		["Skip intro", "Passer l'intro", "Intro \u{fc}berspringen", "Saltar la intro", "Salta l'intro"],
		[
			"Skips the logos and the opening movies when the game starts.",
			"Passe les logos et les vid\u{e9}os d'ouverture au d\u{e9}marrage du jeu.",
			"\u{dc}berspringt beim Spielstart die Logos und die Er\u{f6}ffnungsfilme.",
			"Salta los logotipos y las pel\u{ed}culas de apertura al iniciar el juego.",
			"Salta i loghi e i filmati di apertura all'avvio del gioco.",
		],
		NO,
		0,
		Cost::None,
		None,
	),
	row(
		"skipcut",
		TOG,
		["Start skips cutscenes", "Start passe les sc\u{e8}nes", "Start \u{fc}berspringt Szenen", "Start salta escenas", "Start salta le scene"],
		[
			"Pressing Start during a cutscene skips it. Off plays every cutscene to the end.",
			"Appuyer sur Start pendant une sc\u{e8}ne la passe. Non les joue toutes jusqu'au bout.",
			"Start w\u{e4}hrend einer Zwischensequenz \u{fc}berspringt sie. Aus spielt jede bis zum Ende.",
			"Pulsar Start durante una escena la salta. No las reproduce todas hasta el final.",
			"Premere Start durante una scena la salta. No le riproduce tutte fino alla fine.",
		],
		NO,
		1,
		Cost::None,
		None,
	),
];
