//! The Game tab's Gameplay section: Wrench throw (setter `retail/wrench_throw.rs`), Quick Select pauses the game (`retail/qs_pause.rs`)
//! and Auto-equip gadgets (`retail/auto_gadget.rs`), all via `retail/pcext/ui_bridge_more.rs`. Modern menu only: the classic Camera Options page has no free row (the classic menu
//! stays as it is).
use super::text::*;
use rac_engine::ui::{Cost, UiRow, T5};

const THROWS: &[T5] = &[CLASSIC, ["Moving", "En mouvement", "In Bewegung", "En movimiento", "In movimento"]];

pub static GAMEPLAY: [UiRow; 3] = [
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
];
