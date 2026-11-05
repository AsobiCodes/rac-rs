//! The Display tab: Screen and HUD. Row ids are the bridge's (`retail/pcext/ui_bridge.rs`). Aspect ratio (GitHub #45) has no RAM byte:
//! the engine's `viewer::aspect_box` (options file key `aspect`), set and published by `retail/pcext/ui_bridge_more.rs`; modern menu only.
use super::costs;
use super::text::*;
use rac_engine::ui::{Cost, UiRow};

const WINDOWED: &[rac_engine::ui::T5] = &[["Windowed", "Fen\u{ea}tr\u{e9}", "Fenster", "Ventana", "Finestra"], ["Fullscreen", "Plein \u{e9}cran", "Vollbild", "Pantalla completa", "Schermo intero"]];
const FPS_VALUES: &[rac_engine::ui::T5] = &[["Display", "\u{c9}cran", "Anzeige", "Pantalla", "Schermo"], t("30"), t("60"), t("120"), t("144"), OFF];
const RS_VALUES: &[rac_engine::ui::T5] = &[t("50%"), t("75%"), t("100%"), t("125%"), t("150%"), t("200%")];
const HUD_VALUES: &[rac_engine::ui::T5] = &[["Sharp", "Net", "Scharf", "N\u{ed}tido", "Nitido"], ["Original", "Original", "Original", "Original", "Originale"]];
/// `rac_engine::viewer::aspect_box::ALL` order.
const ASPECT_VALUES: &[rac_engine::ui::T5] = &[t("Auto"), t("4:3"), t("16:9"), t("16:10"), t("21:9"), t("32:9")];
const POS_VALUES: &[rac_engine::ui::T5] = &[t("4:3"), ["Corners", "Coins", "Ecken", "Esquinas", "Angoli"]];

pub static SCREEN: [UiRow; 5] = [
	row(
		"displaymode",
		CH,
		["Display mode", "Mode d'affichage", "Anzeigemodus", "Modo de pantalla", "Modalit\u{e0} schermo"],
		[
			"Play in a window or on the whole screen.",
			"Jouer dans une fen\u{ea}tre ou sur tout l'\u{e9}cran.",
			"Im Fenster oder auf dem ganzen Bildschirm spielen.",
			"Juega en una ventana o en toda la pantalla.",
			"Gioca in una finestra o a schermo intero.",
		],
		WINDOWED,
		0,
		Cost::None,
		None,
	),
	row(
		"aspect",
		CH,
		["Aspect ratio", "Format d'image", "Seitenverh\u{e4}ltnis", "Relaci\u{f3}n de aspecto", "Proporzioni"],
		[
			"Auto fills the window. Another shape draws the game in the largest box of that shape, with black bars around it (ReShade is off while they show). 4:3 is the shape of the original PS2 picture.",
			"Auto remplit la fen\u{ea}tre. Un autre format dessine le jeu dans le plus grand cadre de ce format, avec des bandes noires autour (ReShade est d\u{e9}sactiv\u{e9} tant qu'elles sont affich\u{e9}es). 4:3 est le format de l'image PS2 d'origine.",
			"Auto f\u{fc}llt das Fenster. Ein anderes Format zeichnet das Spiel im gr\u{f6}\u{df}ten Rahmen dieser Form, mit schwarzen Balken drumherum (solange sie zu sehen sind, ist ReShade aus). 4:3 ist das Format des Original-PS2-Bilds.",
			"Auto llena la ventana. Otro formato dibuja el juego en el mayor recuadro de esa forma, con barras negras alrededor (ReShade se desactiva mientras se ven). 4:3 es el formato de la imagen original de PS2.",
			"Auto riempie la finestra. Un altro formato disegna il gioco nel riquadro pi\u{f9} grande di quella forma, con bande nere intorno (ReShade \u{e8} disattivato finch\u{e9} sono visibili). 4:3 \u{e8} il formato dell'immagine PS2 originale.",
		],
		ASPECT_VALUES,
		0,
		Cost::None,
		None,
	),
	row(
		"rscale",
		CH,
		["Render scale", "Taille du rendu", "Render-Skalierung", "Escala de render", "Scala di rendering"],
		[
			"Draws the picture at this size of the window, then fits it to the window. Below 100% runs faster, above 100% looks sharper.",
			"Dessine l'image \u{e0} cette taille de la fen\u{ea}tre, puis l'ajuste. Sous 100 %, c'est plus rapide; au-dessus, plus net.",
			"Zeichnet das Bild in dieser Gr\u{f6}\u{df}e des Fensters und passt es dann an. Unter 100 % l\u{e4}uft es schneller, dar\u{fc}ber sch\u{e4}rfer.",
			"Dibuja la imagen a este tama\u{f1}o de la ventana y la ajusta. Por debajo de 100 % va m\u{e1}s r\u{e1}pido, por encima se ve m\u{e1}s n\u{ed}tido.",
			"Disegna l'immagine a questa dimensione della finestra e poi la adatta. Sotto il 100% \u{e8} pi\u{f9} veloce, sopra \u{e8} pi\u{f9} nitida.",
		],
		RS_VALUES,
		2,
		costs::RSCALE,
		None,
	),
	row(
		"fps",
		CH,
		["FPS limit", "Limite FPS", "FPS-Limit", "L\u{ed}mite de FPS", "Limite FPS"],
		[
			"The most frames a second the game draws. Display matches your monitor, Off removes the limit.",
			"Le maximum d'images par seconde. \u{c9}cran suit votre moniteur, Non retire la limite.",
			"Die meisten Bilder pro Sekunde, die das Spiel zeichnet. Anzeige folgt deinem Monitor, Aus hebt das Limit auf.",
			"El m\u{e1}ximo de fotogramas por segundo. Pantalla sigue tu monitor, No quita el l\u{ed}mite.",
			"Il massimo di fotogrammi al secondo. Schermo segue il monitor, No toglie il limite.",
		],
		FPS_VALUES,
		0,
		Cost::None,
		None,
	),
	row(
		"smoothing",
		TOG,
		["Frame smoothing", "Lissage des images", "Bildgl\u{e4}ttung", "Suavizado de im\u{e1}genes", "Fluidit\u{e0} immagini"],
		[
			"Draws in-between frames so motion looks smooth on fast monitors. The game itself still runs at its original speed.",
			"Dessine des images interm\u{e9}diaires pour un mouvement fluide sur les \u{e9}crans rapides. Le jeu garde sa vitesse d'origine.",
			"Zeichnet Zwischenbilder, damit Bewegung auf schnellen Monitoren fl\u{fc}ssig wirkt. Das Spiel l\u{e4}uft weiter mit Originaltempo.",
			"Dibuja fotogramas intermedios para que el movimiento sea fluido en monitores r\u{e1}pidos. El juego sigue a su velocidad original.",
			"Disegna fotogrammi intermedi per un movimento fluido sui monitor veloci. Il gioco gira alla velocit\u{e0} originale.",
		],
		NO,
		1,
		costs::SMOOTHING,
		None,
	),
];

pub static HUD: [UiRow; 3] = [
	row(
		"hud",
		CH,
		["HUD", "HUD", "HUD", "HUD", "HUD"],
		[
			"Sharp draws the HUD at your screen resolution. Original keeps the PS2 look.",
			"Net dessine le HUD \u{e0} la r\u{e9}solution de l'\u{e9}cran. Original garde l'aspect PS2.",
			"Scharf zeichnet das HUD in deiner Bildschirmaufl\u{f6}sung. Original beh\u{e4}lt den PS2-Look.",
			"N\u{ed}tido dibuja el HUD a la resoluci\u{f3}n de tu pantalla. Original mantiene el aspecto de PS2.",
			"Nitido disegna l'HUD alla risoluzione dello schermo. Originale mantiene l'aspetto PS2.",
		],
		HUD_VALUES,
		0,
		Cost::None,
		None,
	),
	row(
		"hudpos",
		CH,
		["HUD position", "Position du HUD", "HUD-Position", "Posici\u{f3}n del HUD", "Posizione HUD"],
		[
			"4:3 keeps the HUD where the original game has it. Corners moves the top boxes (nanotech, bolts and ammo) to the screen corners on wide screens.",
			"4:3 garde le HUD o\u{f9} le jeu d'origine le met. Coins place les bo\u{ee}tes du haut (nanotech, boulons, munitions) aux coins des \u{e9}crans larges.",
			"4:3 l\u{e4}sst das HUD, wo das Originalspiel es hat. Ecken setzt die oberen Felder (Nanotech, Bolzen, Munition) auf Breitbild in die Ecken.",
			"4:3 deja el HUD donde lo pone el juego original. Esquinas lleva las cajas de arriba (nanotecnolog\u{ed}a, tornillos y munici\u{f3}n) a las esquinas en pantallas anchas.",
			"4:3 lascia l'HUD dove lo mette il gioco originale. Angoli sposta i riquadri in alto (nanotech, bulloni e munizioni) negli angoli sugli schermi larghi.",
		],
		POS_VALUES,
		0,
		Cost::None,
		None,
	),
	row(
		"hudscale",
		slider(75, 150, 5, "%"),
		["HUD scale", "Taille du HUD", "HUD-Skalierung", "Escala del HUD", "Scala HUD"],
		[
			"Size of the top HUD boxes. 100% is the original size.",
			"Taille des bo\u{ee}tes du haut du HUD. 100 % est la taille d'origine.",
			"Gr\u{f6}\u{df}e der oberen HUD-Felder. 100 % ist die Originalgr\u{f6}\u{df}e.",
			"Tama\u{f1}o de las cajas superiores del HUD. 100 % es el tama\u{f1}o original.",
			"Dimensione dei riquadri in alto dell'HUD. 100% \u{e8} la dimensione originale.",
		],
		NO,
		100,
		Cost::None,
		None,
	),
];
