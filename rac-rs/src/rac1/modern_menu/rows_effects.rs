//! The Graphics tab, part 2: Effects (particles, metal, fire, smoke, fur, fur style); all need Enhanced lighting.
use super::costs;
use super::text::*;
use rac_engine::ui::{Cost, Needs, NeedsRule, UiRow, T5};

const FUR_STYLES: &[T5] = &[["Soft", "Doux", "Weich", "Suave", "Morbido"], ["Fluffy", "Duveteux", "Flauschig", "Esponjoso", "Soffice"]];

pub static EFFECTS: [UiRow; 8] = [
	row(
		"particles",
		CH,
		["Particles", "Particules", "Partikel", "Part\u{ed}culas", "Particelle"],
		[
			"Classic keeps the original sparks and explosions. Low to High use the improved effects.",
			"Classique garde les \u{e9}tincelles et explosions d'origine. Faible \u{e0} \u{c9}lev\u{e9} utilisent les effets am\u{e9}lior\u{e9}s.",
			"Klassisch beh\u{e4}lt die originalen Funken und Explosionen. Niedrig bis Hoch nutzen die verbesserten Effekte.",
			"Cl\u{e1}sico mantiene las chispas y explosiones originales. De Baja a Alta usan los efectos mejorados.",
			"Classico mantiene le scintille e le esplosioni originali. Da Bassa ad Alta usano gli effetti migliorati.",
		],
		QC,
		3,
		costs::PARTICLES,
		LIGHT,
	),
	row(
		"metal",
		TOG,
		["Metal", "M\u{e9}tal", "Metall", "Metal", "Metallo"],
		[
			"Shiny metal: reflections of the sky and the scene, and sharp highlights.",
			"M\u{e9}tal brillant : reflets du ciel et de la sc\u{e8}ne, et brillances nettes.",
			"Gl\u{e4}nzendes Metall: Spiegelungen von Himmel und Szene und scharfe Glanzlichter.",
			"Metal brillante: reflejos del cielo y de la escena, y brillos n\u{ed}tidos.",
			"Metallo lucido: riflessi del cielo e della scena, e luci nitide.",
		],
		NO,
		0,
		costs::METAL,
		LIGHT,
	),
	row(
		"metal_reflect",
		slider(0, 100, 5, "%"),
		["Metal reflections", "Reflets du m\u{e9}tal", "Metallspiegelung", "Reflejos del metal", "Riflessi del metallo"],
		[
			"How strongly metal mirrors the sky and the scene. 100% = the full shiny look; lower = more matte; 0% = no reflections. Highlights from lights stay.",
			"Intensit\u{e9} des reflets du ciel et de la sc\u{e8}ne sur le m\u{e9}tal. 100 % = tout l'effet brillant ; moins = plus mat ; 0 % = aucun reflet. Les brillances des lumi\u{e8}res restent.",
			"Wie stark Metall Himmel und Szene spiegelt. 100 % = voller Glanz; weniger = matter; 0 % = keine Spiegelung. Glanzlichter von Lampen bleiben.",
			"Cu\u{e1}nto refleja el metal el cielo y la escena. 100 % = todo el brillo; menos = m\u{e1}s mate; 0 % = sin reflejos. Los brillos de las luces se mantienen.",
			"Quanto il metallo riflette cielo e scena. 100% = tutto l'effetto lucido; meno = pi\u{f9} opaco; 0% = nessun riflesso. Le luci nitide restano.",
		],
		NO,
		75,
		Cost::None,
		Some(Needs { row: "metal", min: 1, max: 1 }),
	),
	row(
		"fire",
		CH,
		["Fire", "Feu", "Feuer", "Fuego", "Fuoco"],
		[
			"Realistic flames for weapons, effects and burning buildings. Off is the original fire.",
			"Flammes r\u{e9}alistes pour les armes, les effets et les b\u{e2}timents en feu. Avec Non, c'est le feu d'origine.",
			"Realistische Flammen f\u{fc}r Waffen, Effekte und brennende Geb\u{e4}ude. Aus ist das Originalfeuer.",
			"Llamas realistas para armas, efectos y edificios en llamas. Con No se mantiene el fuego original.",
			"Fiamme realistiche per armi, effetti ed edifici in fiamme. Con No resta il fuoco originale.",
		],
		Q4,
		3,
		costs::FIRE,
		LIGHT,
	),
	row(
		"smoke",
		CH,
		["Smoke", "Fum\u{e9}e", "Rauch", "Humo", "Fumo"],
		[
			"Volumetric smoke from fires and explosions. Off is the original smoke.",
			"Fum\u{e9}e volum\u{e9}trique des feux et des explosions. Avec Non, c'est la fum\u{e9}e d'origine.",
			"Volumetrischer Rauch von Feuern und Explosionen. Aus ist der Originalrauch.",
			"Humo volum\u{e9}trico de fuegos y explosiones. Con No se mantiene el humo original.",
			"Fumo volumetrico da fuochi ed esplosioni. Con No resta il fumo originale.",
		],
		Q4,
		3,
		costs::SMOKE,
		LIGHT,
	),
	row(
		"fur",
		CH,
		["Fur", "Fourrure", "Fell", "Pelo", "Pelo"],
		[
			"Fur shading for Ratchet. Off is the original look.",
			"Ombrage de la fourrure de Ratchet. Avec Non, c'est l'aspect d'origine.",
			"Fell-Schattierung f\u{fc}r Ratchet. Aus ist der Originallook.",
			"Sombreado del pelo de Ratchet. Con No se mantiene el aspecto original.",
			"Ombreggiatura del pelo di Ratchet. Con No resta l'aspetto originale.",
		],
		Q4,
		3,
		costs::FUR,
		LIGHT,
	),
	row(
		"furstyle",
		CH,
		["Fur style", "Style de fourrure", "Fellstil", "Estilo de pelo", "Stile del pelo"],
		[
			"Two looks for the fur: Soft or Fluffy.",
			"Deux aspects pour la fourrure : Doux ou Duveteux.",
			"Zwei Looks f\u{fc}r das Fell: Weich oder Flauschig.",
			"Dos aspectos para el pelo: Suave o Esponjoso.",
			"Due aspetti per il pelo: Morbido o Soffice.",
		],
		FUR_STYLES,
		0,
		Cost::None,
		LIGHT,
	),
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

/// Metal reflections is dimmed while Metal is Off. No banner over the tab: only the row's box says why.
pub const NEEDS_METAL: NeedsRule = NeedsRule {
	row: "metal",
	set_to: 1,
	banner: t(""),
	detail: [
		"Applies while Metal is On. Metal is {} now.",
		"S'applique quand le m\u{e9}tal est sur Oui. Il est sur {} pour l'instant.",
		"Gilt, solange Metall an ist. Metall steht auf {}.",
		"Se aplica con el metal en S\u{ed}. Ahora est\u{e1} en {}.",
		"Vale con il metallo su S\u{ec}. Ora \u{e8} su {}.",
	],
	button: ["Switch Metal On", "Activer le m\u{e9}tal", "Metall einschalten", "Activar el metal", "Attiva il metallo"],
};
