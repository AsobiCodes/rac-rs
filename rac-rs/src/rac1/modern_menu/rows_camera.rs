//! The Camera tab (Camera, Mouse, Aiming) and the Game tab (Vendor, Menus: Menu style). The Controller and Start sections, which have their own
//! setters (`retail/pcext/ui_bridge_more.rs`), are in `rows_system.rs`.
use super::text::*;
use rac_engine::ui::{Cost, Needs, NeedsRule, UiRow, T5};

/// Classic in its feminine form for Spanish and Italian (la c\u{e1}mara, la telecamera).
const CAMERAS: &[T5] = &[["Classic", "Classique", "Klassisch", "Cl\u{e1}sica", "Classica"], ["Static", "Fixe", "Statisch", "Est\u{e1}tica", "Statica"], ["Smooth", "Fluide", "Sanft", "Suave", "Fluida"]];
const STRAFES: &[T5] = &[["Raised", "Sur\u{e9}lev\u{e9}e", "Erh\u{f6}ht", "Elevada", "Rialzata"], ["Shoulder", "\u{c9}paule", "Schulter", "Hombro", "Spalla"], ["Steady", "Stable", "Ruhig", "Estable", "Stabile"], OFF];
const LOCKONS: &[T5] = &[["Auto", "Auto", "Auto", "Auto", "Auto"], ON, OFF];
const STYLES: &[T5] = &[["Modern", "Moderne", "Modern", "Moderno", "Moderno"], CLASSIC];

pub static CAMERA: [UiRow; 2] = [
	row(
		"camera",
		CH,
		["Camera", "Cam\u{e9}ra", "Kamera", "C\u{e1}mara", "Telecamera"],
		[
			"Classic is the original camera and controls. Static is the mouse camera. Smooth is the mouse camera with a gentle follow.",
			"Classique est la cam\u{e9}ra et les commandes d'origine. Fixe est la cam\u{e9}ra \u{e0} la souris. Fluide ajoute un suivi doux.",
			"Klassisch ist die originale Kamera samt Steuerung. Statisch ist die Maus-Kamera. Sanft ist die Maus-Kamera mit weichem Nachf\u{fc}hren.",
			"Cl\u{e1}sica es la c\u{e1}mara y los controles originales. Est\u{e1}tica es la c\u{e1}mara con rat\u{f3}n. Suave a\u{f1}ade un seguimiento progresivo.",
			"Classica \u{e8} la telecamera con i comandi originali. Statica \u{e8} la telecamera col mouse. Fluida aggiunge un inseguimento dolce.",
		],
		CAMERAS,
		2,
		Cost::None,
		None,
	),
	row(
		"camspeed",
		slider(10, 200, 10, ""),
		["Camera speed", "Vitesse de la cam\u{e9}ra", "Kameratempo", "Velocidad de c\u{e1}mara", "Velocit\u{e0} telecamera"],
		[
			"How fast the stick turns the camera. 50 is the original Fast setting.",
			"Vitesse de rotation de la cam\u{e9}ra au stick. 50 est le r\u{e9}glage Rapide d'origine.",
			"Wie schnell der Stick die Kamera dreht. 50 ist die originale Einstellung Schnell.",
			"Con qu\u{e9} rapidez gira la c\u{e1}mara el stick. 50 es el ajuste R\u{e1}pido original.",
			"Quanto velocemente lo stick ruota la telecamera. 50 \u{e8} l'impostazione Veloce originale.",
		],
		NO,
		50,
		Cost::None,
		None,
	),
];

pub static MOUSE: [UiRow; 1] = [row(
	"mousespeed",
	slider(10, 200, 10, ""),
	["Mouse speed", "Vitesse de la souris", "Mausgeschwindigkeit", "Velocidad del rat\u{f3}n", "Velocit\u{e0} del mouse"],
	[
		"How fast the mouse turns the camera. Separate from the stick speed.",
		"Vitesse de rotation de la cam\u{e9}ra \u{e0} la souris. S\u{e9}par\u{e9}e de celle du stick.",
		"Wie schnell die Maus die Kamera dreht. Getrennt vom Stick-Tempo.",
		"Con qu\u{e9} rapidez gira la c\u{e1}mara el rat\u{f3}n. Aparte de la velocidad del stick.",
		"Quanto velocemente il mouse ruota la telecamera. Separata dalla velocit\u{e0} dello stick.",
	],
	NO,
	50,
	Cost::None,
	None,
)];

pub static AIMING: [UiRow; 3] = [
	row(
		"strafe",
		CH,
		["Strafe aiming", "Vis\u{e9}e en d\u{e9}placement", "Zielen beim Strafen", "Apuntar al desplazarse", "Mira in movimento"],
		[
			"Hold the right mouse button to strafe. Raised lifts the camera and shows the aim circle, Shoulder aims over the shoulder, Steady keeps the camera as it is, like Ratchet & Clank 2.",
			"Maintenez le bouton droit pour viser en vous d\u{e9}pla\u{e7}ant. Sur\u{e9}lev\u{e9}e l\u{e8}ve la cam\u{e9}ra et montre le cercle de vis\u{e9}e, \u{c9}paule vise par-dessus l'\u{e9}paule, Stable garde la cam\u{e9}ra telle quelle, comme dans Ratchet & Clank 2.",
			"Halte die rechte Maustaste zum Strafen. Erh\u{f6}ht hebt die Kamera und zeigt den Zielkreis, Schulter zielt \u{fc}ber die Schulter, Ruhig l\u{e4}sst die Kamera unver\u{e4}ndert, wie in Ratchet & Clank 2.",
			"Mant\u{e9}n el bot\u{f3}n derecho del rat\u{f3}n para apuntar al desplazarte. Elevada sube la c\u{e1}mara y muestra el c\u{ed}rculo de punter\u{ed}a, Hombro apunta sobre el hombro, Estable deja la c\u{e1}mara tal cual, como en Ratchet & Clank 2.",
			"Tieni premuto il tasto destro del mouse per mirare mentre ti sposti. Rialzata alza la telecamera e mostra il cerchio di mira, Spalla mira da sopra la spalla, Stabile lascia la telecamera com'\u{e8}, come in Ratchet & Clank 2.",
		],
		STRAFES,
		0,
		Cost::None,
		None,
	),
	row(
		"lockon",
		CH,
		["Lock-on", "Verrouillage", "Zielerfassung", "Fijaci\u{f3}n de objetivo", "Aggancio"],
		[
			"Aim help while strafing. Auto is on with a controller or with Steady, and off with keyboard and mouse.",
			"Aide \u{e0} la vis\u{e9}e en d\u{e9}placement. Auto est actif avec une manette ou avec Stable, et inactif au clavier et \u{e0} la souris.",
			"Zielhilfe beim Strafen. Auto ist mit Controller oder mit Ruhig an und mit Tastatur und Maus aus.",
			"Ayuda de punter\u{ed}a al desplazarte. Auto est\u{e1} activa con mando o con Estable, y desactivada con teclado y rat\u{f3}n.",
			"Aiuto alla mira mentre ti sposti. Auto \u{e8} attivo col controller o con Stabile, e spento con tastiera e mouse.",
		],
		LOCKONS,
		0,
		Cost::None,
		None,
	),
	row(
		"lockon_strength",
		slider(0, 100, 10, "%"),
		["Lock-on strength", "Force du verrouillage", "St\u{e4}rke der Zielerfassung", "Fuerza de fijaci\u{f3}n", "Forza dell'aggancio"],
		[
			"How strongly the aim snaps to enemies while strafing. 100% = full lock-on; lower = you must aim closer to an enemy before it locks; 0% = no lock-on.",
			"Force avec laquelle la vis\u{e9}e s'accroche aux ennemis en d\u{e9}placement. 100 % = verrouillage complet ; moins = il faut viser plus pr\u{e8}s d'un ennemi pour le verrouiller ; 0 % = aucun verrouillage.",
			"Wie stark das Zielen beim Strafen auf Gegner springt. 100 % = volle Zielerfassung; weniger = man muss n\u{e4}her auf einen Gegner zielen, bevor er erfasst wird; 0 % = keine Zielerfassung.",
			"Con qu\u{e9} fuerza la punter\u{ed}a se fija en los enemigos al desplazarte. 100 % = fijaci\u{f3}n completa; menos = hay que apuntar m\u{e1}s cerca de un enemigo para fijarlo; 0 % = sin fijaci\u{f3}n.",
			"Quanto la mira si aggancia ai nemici mentre ti sposti. 100% = aggancio completo; meno = devi mirare pi\u{f9} vicino a un nemico perch\u{e9} si agganci; 0% = nessun aggancio.",
		],
		NO,
		100,
		Cost::None,
		Some(Needs { row: "lockon", min: 0, max: 1 }),
	),
];

/// Lock-on strength is hidden while Lock-on is Off (the search does not run; user 2026-11-08: hidden, not dimmed). No banner.
pub const NEEDS_LOCKON: NeedsRule = NeedsRule {
	row: "lockon",
	set_to: 0,
	for_row: "",
	fix: None,
	banner: t(""),
	detail: [
		"Applies while Lock-on is Auto or On. Lock-on is {} now.",
		"S'applique quand le verrouillage est sur Auto ou Oui. Il est sur {} pour l'instant.",
		"Gilt, solange die Zielerfassung auf Auto oder Ein steht. Sie steht auf {}.",
		"Se aplica con la fijaci\u{f3}n en Auto o S\u{ed}. Ahora est\u{e1} en {}.",
		"Vale con l'aggancio su Auto o S\u{ec}. Ora \u{e8} su {}.",
	],
	button: ["Switch Lock-on to Auto", "Passer le verrouillage sur Auto", "Zielerfassung auf Auto stellen", "Poner la fijaci\u{f3}n en Auto", "Imposta l'aggancio su Auto"],
};

pub static VENDOR: [UiRow; 1] = [row(
	"refill",
	TOG,
	["Refill all ammo", "Tout recharger", "Alle Munition auff\u{fc}llen", "Rellenar toda la munici\u{f3}n", "Ricarica tutte le munizioni"],
	[
		"At a vendor, one button fills the ammo of every weapon.",
		"Chez un vendeur, un seul bouton remplit les munitions de chaque arme.",
		"Beim H\u{e4}ndler f\u{fc}llt eine Taste die Munition jeder Waffe auf.",
		"Con un vendedor, un bot\u{f3}n rellena la munici\u{f3}n de cada arma.",
		"Da un venditore, un tasto ricarica le munizioni di ogni arma.",
	],
	NO,
	1, // Buy all ammo is On by default (`PcOptions::buyall_on`)
	Cost::None,
	None,
)];

pub static MENUS: [UiRow; 1] = [
	row(
		"menustyle",
		CH,
		["Menu style", "Style des menus", "Men\u{fc}stil", "Estilo de men\u{fa}s", "Stile dei menu"],
		[
			"Modern is this menu. Classic brings back the original-style option pages.",
			"Moderne est ce menu. Classique ram\u{e8}ne les pages d'options dans le style d'origine.",
			"Modern ist dieses Men\u{fc}. Klassisch bringt die Optionsseiten im Originalstil zur\u{fc}ck.",
			"Moderno es este men\u{fa}. Cl\u{e1}sico recupera las p\u{e1}ginas de opciones al estilo original.",
			"Moderno \u{e8} questo menu. Classico riporta le pagine delle opzioni in stile originale.",
		],
		STYLES,
		0,
		Cost::None,
		None,
	),
];
