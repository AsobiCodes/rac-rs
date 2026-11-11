//! The Game tab's Gameplay section: Wrench throw (setter `retail/wrench_throw.rs`), Quick Select pauses the game (`retail/qs_pause.rs`),
//! Auto-equip gadgets (`retail/auto_gadget.rs`) and Fast travel (`retail/fast_travel.rs`), all via `retail/pcext/ui_bridge_more.rs`.
//! Quick Select pause and Fast travel also have classic rows (Graphics page, Gameplay section: `retail/pcmore.rs`); Wrench throw and
//! Auto-equip gadgets are modern only. Extras: Supporter drawings (`retail/pcsupp.rs`), moved from Graphics -> Effects on 2026-11-08
//! (user: it is not a graphics setting).
use super::text::*;
use rac_engine::ui::{Cost, UiRow, T5};

const THROWS: &[T5] = &[CLASSIC, ["Moving", "En mouvement", "In Bewegung", "En movimiento", "In movimento"]];

pub static GAMEPLAY: [UiRow; 4] = [
	row(
		"wrenchthrow",
		CH,
		["Wrench throw", "Lancer de cl\u{e9}", "Schl\u{fc}sselwurf", "Lanzar la llave", "Lancio della chiave"],
		[
			"Classic: Ratchet stands still until the wrench is back. Moving: he can move once it leaves his hand, and it flies where he aims.",
			"Classique : Ratchet reste immobile jusqu'au retour de la cl\u{e9}. En mouvement : il bouge d\u{e8}s qu'elle part, et elle vole o\u{f9} il vise.",
			"Klassisch: Ratchet steht still, bis der Schl\u{fc}ssel zur\u{fc}ck ist. In Bewegung: Er l\u{e4}uft, sobald er fliegt, und er fliegt, wohin er zielt.",
			"Cl\u{e1}sico: Ratchet se queda quieto hasta que vuelve la llave. En movimiento: se mueve en cuanto la suelta, y vuela hacia donde apunta.",
			"Classico: Ratchet resta fermo finch\u{e9} la chiave non torna. In movimento: si muove appena la lancia, e vola dove mira.",
		],
		THROWS,
		0,
		Cost::None,
		None,
	),
	row(
		"qspause",
		TOG,
		[
			"Quick Select pauses the game",
			"S\u{e9}lection rapide met le jeu en pause",
			"Schnellauswahl h\u{e4}lt das Spiel an",
			"La selecci\u{f3}n r\u{e1}pida pausa el juego",
			"La selezione rapida mette in pausa il gioco",
		],
		[
			"While the Quick Select ring is open, enemies, shots and the world stand still. Off: the world keeps moving, as in the original.",
			"Tant que l'anneau de s\u{e9}lection rapide est ouvert, ennemis, tirs et d\u{e9}cor s'arr\u{ea}tent. Non : tout continue de bouger, comme \u{e0} l'origine.",
			"Solange der Schnellauswahl-Ring offen ist, stehen Gegner, Sch\u{fc}sse und die Welt still. Aus: Die Welt l\u{e4}uft weiter wie im Original.",
			"Mientras el anillo de selecci\u{f3}n r\u{e1}pida est\u{e1} abierto, enemigos, disparos y mundo se detienen. No: el mundo sigue movi\u{e9}ndose, como en el original.",
			"Finch\u{e9} l'anello della selezione rapida \u{e8} aperto, nemici, colpi e mondo restano fermi. No: il mondo continua a muoversi, come nell'originale.",
		],
		NO,
		0, // Off by default = retail (`retail/qs_pause.rs`: options file key `qspause` only, no RAM byte, no classic row)
		Cost::None,
		None,
	),
	row(
		"autogadget",
		TOG,
		[
			"Auto-equip gadgets",
			"Gadgets automatiques",
			"Gadgets automatisch ausr\u{fc}sten",
			"Equipar gadgets autom\u{e1}ticamente",
			"Equipaggia gadget in automatico",
		],
		[
			"Fire at a Versa-Target, a Hydrodisplacer pad or a Trespasser lock to use that gadget; then your weapon comes back. Off: as in the original.",
			"Tirez vers une Versa-cible, un socle d'Hydrod\u{e9}placeur ou une serrure \u{e0} Crocheteur pour l'utiliser ; puis l'arme revient. Non : comme \u{e0} l'origine.",
			"Feuer auf ein Versa-Ziel, ein Hydrobeamer-Feld oder ein Dietrich-Schloss nimmt das Gadget; danach kommt die Waffe zur\u{fc}ck. Aus: wie im Original.",
			"Dispara a un blanco flotante, una base de Hidrodesplazador o una cerradura de Ganzuotr\u{f3}n para usarlo; luego vuelve el arma. No: como en el original.",
			"Spara a un Versa-bersaglio, una base dell'Idrosostitutore o una serratura del Trapassatore per usarlo; poi torna l'arma. No: come nell'originale.",
		],
		NO,
		0, // Off by default = retail (`retail/auto_gadget.rs`: options file key `autogadget` only, no RAM byte, no classic row)
		Cost::None,
		None,
	),
	row(
		"fasttravel",
		TOG,
		["Fast travel", "Voyage rapide", "Schnellreise", "Viaje r\u{e1}pido", "Viaggio rapido"],
		[
			"Skips the flight through space between planets and after loading a save: the takeoff and the story movies stay, the landing follows as soon as the planet is loaded. Off: as in the original.",
			"Saute le vol dans l'espace entre les plan\u{e8}tes et apr\u{e8}s le chargement d'une partie : le d\u{e9}collage et les films restent, l'atterrissage suit d\u{e8}s que la plan\u{e8}te est charg\u{e9}e. Non : comme \u{e0} l'origine.",
			"\u{dc}berspringt den Flug durchs All zwischen den Planeten und nach dem Laden eines Spielstands: Start und Filmsequenzen bleiben, die Landung folgt, sobald der Planet geladen ist. Aus: wie im Original.",
			"Omite el vuelo por el espacio entre planetas y al cargar una partida: el despegue y las cinem\u{e1}ticas se mantienen, el aterrizaje sigue en cuanto el planeta est\u{e1} cargado. No: como en el original.",
			"Salta il volo nello spazio tra i pianeti e dopo il caricamento di una partita: il decollo e i filmati restano, l'atterraggio segue appena il pianeta \u{e8} caricato. No: come nell'originale.",
		],
		NO,
		1, // On by default (`retail/fast_travel.rs`: options file key `fasttravel`; a replay or test run without the key keeps retail's flight)
		Cost::None,
		None,
	),
];

/// The Game tab's Extras section: Supporter drawings (the Ko-fi garage cards, `retail/pcsupp.rs`; set in `retail/pcext/ui_bridge.rs`).
pub static EXTRAS: [UiRow; 1] = [
	row(
		"supporters",
		TOG,
		["Supporter drawings", "Dessins des soutiens", "Fan-Zeichnungen", "Dibujos de mecenas", "Disegni sostenitori"],
		[
			"Shows the children's drawings made for the Ko-fi supporters on the walls of Ratchet's garage on Veldin. Works with any Lighting.",
			"Affiche sur les murs du garage de Ratchet, sur Veldin, les dessins d'enfant faits pour les soutiens Ko-fi. Fonctionne avec tous les \u{e9}clairages.",
			"Zeigt die Kinderzeichnungen f\u{fc}r die Ko-fi-Unterst\u{fc}tzer an den W\u{e4}nden von Ratchets Garage auf Veldin. Funktioniert mit jeder Beleuchtung.",
			"Muestra en las paredes del garaje de Ratchet, en Veldin, los dibujos infantiles hechos para los mecenas de Ko-fi. Funciona con cualquier iluminaci\u{f3}n.",
			"Mostra sulle pareti del garage di Ratchet, su Veldin, i disegni dei bambini fatti per i sostenitori di Ko-fi. Funziona con qualsiasi illuminazione.",
		],
		NO,
		0,
		Cost::None,
		None,
	),
];
