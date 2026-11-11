//! The Display tab's Resolution row (in Screen, under Display mode) and its Picture section (Brightness, Contrast), v0.5.1 (user
//! 2026-11-07). No RAM byte: the engine's `viewer::resolution` (key `resolution`) and `viewer::picture_adjust` (keys `brightness`,
//! `contrast`), set and published by `retail/pcext/ui_bridge_more.rs`; modern menu only. The Resolution row's values past Desktop
//! are named at run time with the monitor's sizes (`resolution::publish`: "1920x1080"); unused ones are withdrawn.
use super::text::*;
use rac_engine::ui::{Cost, UiRow};

/// Desktop, then `resolution::SLOTS - 1` places for the monitor's sizes (their static name is never shown: the engine names or
/// withdraws every one of them at start-up).
const RES_VALUES: &[rac_engine::ui::T5] = &[
	["Desktop", "Bureau", "Desktop", "Escritorio", "Desktop"],
	t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"),
	t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"), t("-"),
];

pub const RESOLUTION: UiRow = row(
	"resolution",
	CH,
	["Resolution", "R\u{e9}solution", "Aufl\u{f6}sung", "Resoluci\u{f3}n", "Risoluzione"],
	[
		"Desktop: in a window, the window keeps the size you give it; in fullscreen, the screen's own size. In a window, another size sets the window to it (a size bigger than the screen's free area acts as Desktop). In fullscreen the game is drawn at that size and scaled to the screen; Frame Generation pauses then.",
		"Bureau : en fen\u{ea}tre, la fen\u{ea}tre garde la taille que vous lui donnez ; en plein \u{e9}cran, la taille de l'\u{e9}cran. En fen\u{ea}tre, une autre taille r\u{e8}gle la fen\u{ea}tre (une taille plus grande que l'espace libre de l'\u{e9}cran agit comme Bureau). En plein \u{e9}cran, le jeu est dessin\u{e9} \u{e0} cette taille puis ajust\u{e9} \u{e0} l'\u{e9}cran ; la G\u{e9}n\u{e9}ration d'images est alors en pause.",
		"Desktop: im Fenster beh\u{e4}lt das Fenster die Gr\u{f6}\u{df}e, die du ihm gibst; im Vollbild die Gr\u{f6}\u{df}e des Bildschirms. Im Fenster stellt eine andere Gr\u{f6}\u{df}e das Fenster darauf ein (eine Gr\u{f6}\u{df}e \u{fc}ber der freien Bildschirmfl\u{e4}che wirkt wie Desktop). Im Vollbild wird das Spiel in dieser Gr\u{f6}\u{df}e gezeichnet und auf den Bildschirm skaliert; die Frame-Generierung pausiert dann.",
		"Escritorio: en ventana, la ventana mantiene el tama\u{f1}o que le des; en pantalla completa, el tama\u{f1}o de la pantalla. En ventana, otro tama\u{f1}o ajusta la ventana a \u{e9}l (un tama\u{f1}o mayor que el \u{e1}rea libre de la pantalla act\u{fa}a como Escritorio). En pantalla completa el juego se dibuja a ese tama\u{f1}o y se escala a la pantalla; la Generaci\u{f3}n de fotogramas queda en pausa.",
		"Desktop: in finestra, la finestra mantiene la dimensione che le dai; a schermo intero, la dimensione dello schermo. In finestra, un'altra dimensione imposta la finestra (una dimensione pi\u{f9} grande dell'area libera dello schermo vale come Desktop). A schermo intero il gioco \u{e8} disegnato a quella dimensione e scalato sullo schermo; la Generazione di frame \u{e8} in pausa.",
	],
	RES_VALUES,
	0,
	Cost::None,
	None,
);

pub static PICTURE: [UiRow; 2] = [
	row(
		"brightness",
		slider(0, 100, 5, ""),
		["Brightness", "Luminosit\u{e9}", "Helligkeit", "Brillo", "Luminosit\u{e0}"],
		[
			"Lifts or darkens the shadows and mid-tones of the game picture. Black and white stay as they are; the HUD and menus are not changed. 50 is the original picture.",
			"\u{c9}claircit ou assombrit les ombres et les tons moyens de l'image du jeu. Le noir et le blanc restent tels quels; le HUD et les menus ne changent pas. 50 est l'image d'origine.",
			"Hellt die Schatten und Mittelt\u{f6}ne des Spielbilds auf oder dunkelt sie ab. Schwarz und Wei\u{df} bleiben; HUD und Men\u{fc}s bleiben unver\u{e4}ndert. 50 ist das Originalbild.",
			"Aclara u oscurece las sombras y los tonos medios de la imagen del juego. El negro y el blanco no cambian; el HUD y los men\u{fa}s tampoco. 50 es la imagen original.",
			"Schiarisce o scurisce le ombre e i mezzitoni dell'immagine di gioco. Il nero e il bianco restano uguali; HUD e menu non cambiano. 50 \u{e8} l'immagine originale.",
		],
		NO,
		50,
		Cost::None,
		None,
	),
	row(
		"contrast",
		slider(0, 100, 5, ""),
		["Contrast", "Contraste", "Kontrast", "Contraste", "Contrasto"],
		[
			"Spreads the game picture's tones apart (above 50) or brings them closer to grey (below 50). The HUD and menus are not changed. 50 is the original picture.",
			"\u{c9}carte les tons de l'image du jeu (au-dessus de 50) ou les rapproche du gris (en dessous). Le HUD et les menus ne changent pas. 50 est l'image d'origine.",
			"Zieht die T\u{f6}ne des Spielbilds auseinander (\u{fc}ber 50) oder r\u{fc}ckt sie n\u{e4}her an Grau (unter 50). HUD und Men\u{fc}s bleiben unver\u{e4}ndert. 50 ist das Originalbild.",
			"Separa los tonos de la imagen del juego (por encima de 50) o los acerca al gris (por debajo). El HUD y los men\u{fa}s no cambian. 50 es la imagen original.",
			"Allontana i toni dell'immagine di gioco (sopra 50) o li avvicina al grigio (sotto 50). HUD e menu non cambiano. 50 \u{e8} l'immagine originale.",
		],
		NO,
		50,
		Cost::None,
		None,
	),
];

#[cfg(test)]
mod tests {
	#[test]
	fn one_value_per_engine_slot() {
		assert_eq!(super::RES_VALUES.len(), rac_engine::viewer::resolution::SLOTS);
	}
}
