//! The Graphics tab, part 1: Quality, Lighting and shadows, World. Effect rows carry `needs: Lighting >= Enhanced`; Waves and
//! Underwater look `needs: Water >= Low` (`NEEDS_WATER`; Water needs Lighting, so they dim with either).
use super::costs;
use super::text::*;
use rac_engine::ui::{Cost, NeedsRule, RowKind, UiRow, T5};

const LIGHT_VALUES: &[T5] = &[CLASSIC, ["Enhanced", "Am\u{e9}lior\u{e9}", "Verbessert", "Mejorado", "Migliorato"]];
const VEG_VALUES: &[T5] = &[OFF, ["Moderate", "Mod\u{e9}r\u{e9}e", "M\u{e4}\u{df}ig", "Moderada", "Moderata"], ["Lush", "Luxuriante", "\u{dc}ppig", "Frondosa", "Rigogliosa"]];

pub static QUALITY: [UiRow; 2] = [
	row(
		"preset",
		RowKind::Preset,
		["Preset", "Pr\u{e9}r\u{e9}glage", "Voreinstellung", "Preajuste", "Preset"],
		[
			"Low, Medium and High set the graphics rows in one go. Change any row yourself and the preset shows Custom.",
			"Faible, Moyen et \u{c9}lev\u{e9} r\u{e8}glent toutes les lignes d'un coup. Changez une ligne et le pr\u{e9}r\u{e9}glage devient Personnalis\u{e9}.",
			"Niedrig, Mittel und Hoch stellen alle Grafikzeilen auf einmal ein. \u{c4}nderst du eine Zeile, zeigt die Voreinstellung Eigene.",
			"Baja, Media y Alta ajustan todas las filas de golpe. Si cambias una fila, el preajuste pasa a Personal.",
			"Bassa, Media e Alta impostano tutte le righe in un colpo. Se cambi una riga, il preset diventa Personale.",
		],
		NO,
		0,
		Cost::None,
		None,
	),
	row(
		"lighting",
		CH,
		["Lighting", "\u{c9}clairage", "Beleuchtung", "Iluminaci\u{f3}n", "Illuminazione"],
		[
			"Classic is the original PS2 lighting. Enhanced is the new PC lighting. The effects below need it.",
			"Classique est l'\u{e9}clairage PS2 d'origine. Am\u{e9}lior\u{e9} est le nouvel \u{e9}clairage PC. Les effets ci-dessous en ont besoin.",
			"Klassisch ist die originale PS2-Beleuchtung. Verbessert ist die neue PC-Beleuchtung. Die Effekte darunter brauchen sie.",
			"Cl\u{e1}sico es la iluminaci\u{f3}n original de PS2. Mejorado es la nueva iluminaci\u{f3}n de PC. Los efectos de abajo la necesitan.",
			"Classico \u{e8} l'illuminazione PS2 originale. Migliorato \u{e8} la nuova illuminazione PC. Gli effetti qui sotto ne hanno bisogno.",
		],
		LIGHT_VALUES,
		0,
		costs::LIGHTING,
		None,
	),
];

pub static SHADING: [UiRow; 4] = [
	row(
		"ao",
		CH,
		["Ambient occlusion", "Occlusion ambiante", "Umgebungsverdeckung", "Oclusi\u{f3}n ambiental", "Occlusione ambientale"],
		[
			"Soft shadows in corners, creases and under objects.",
			"Ombres douces dans les coins, les creux et sous les objets.",
			"Weiche Schatten in Ecken, Falten und unter Objekten.",
			"Sombras suaves en rincones, pliegues y bajo los objetos.",
			"Ombre morbide negli angoli, nelle pieghe e sotto gli oggetti.",
		],
		Q4,
		3,
		costs::AO,
		LIGHT,
	),
	row(
		"shadows",
		CH,
		["Dynamic shadows", "Ombres dynamiques", "Dynamische Schatten", "Sombras din\u{e1}micas", "Ombre dinamiche"],
		[
			"Real-time shadows cast by the level and the characters.",
			"Ombres en temps r\u{e9}el projet\u{e9}es par le niveau et les personnages.",
			"Echtzeit-Schatten von Level und Figuren.",
			"Sombras en tiempo real del nivel y de los personajes.",
			"Ombre in tempo reale proiettate dal livello e dai personaggi.",
		],
		Q4,
		3,
		costs::SHADOWS,
		LIGHT,
	),
	row(
		"gi",
		CH,
		["Global illumination", "Illumination globale", "Globale Beleuchtung", "Iluminaci\u{f3}n global", "Illuminazione globale"],
		[
			"Light bounces off walls and floors; fire and weapon shots light up what is nearby.",
			"La lumi\u{e8}re rebondit sur les murs et les sols ; le feu et les tirs \u{e9}clairent les alentours.",
			"Licht prallt von W\u{e4}nden und B\u{f6}den ab; Feuer und Sch\u{fc}sse erhellen die Umgebung.",
			"La luz rebota en paredes y suelos; el fuego y los disparos iluminan lo que hay cerca.",
			"La luce rimbalza su muri e pavimenti; fuoco e colpi illuminano ci\u{f2} che \u{e8} vicino.",
		],
		Q4,
		0,
		costs::GI,
		LIGHT,
	),
	row(
		"sunrays",
		CH,
		["Sun rays", "Rayons de soleil", "Sonnenstrahlen", "Rayos de sol", "Raggi di sole"],
		[
			"Shafts of light around the sun and bright skies.",
			"Faisceaux de lumi\u{e8}re autour du soleil et des ciels clairs.",
			"Lichtstrahlen um die Sonne und an hellem Himmel.",
			"Haces de luz alrededor del sol y de los cielos brillantes.",
			"Fasci di luce attorno al sole e ai cieli luminosi.",
		],
		Q4,
		3,
		costs::SUNRAYS,
		LIGHT,
	),
];

pub static WORLD: [UiRow; 8] = [
	row(
		"draw",
		CH,
		["Draw distance", "Distance d'affichage", "Sichtweite", "Distancia visual", "Distanza visiva"],
		[
			"How far into the distance the level is drawn.",
			"Jusqu'o\u{f9} le niveau est dessin\u{e9} au loin.",
			"Wie weit der Level in die Ferne gezeichnet wird.",
			"Hasta qu\u{e9} distancia se dibuja el nivel.",
			"Fino a che distanza viene disegnato il livello.",
		],
		LMH,
		1,
		costs::DRAW,
		None,
	),
	row(
		"culling",
		TOG,
		["Culling", "Masquage", "Culling", "Descarte", "Culling"],
		[
			"Skips drawing what the camera cannot see. On is faster and looks the same.",
			"Ne dessine pas ce que la cam\u{e9}ra ne voit pas. Avec Oui, c'est plus rapide et identique \u{e0} l'\u{e9}cran.",
			"\u{dc}berspringt, was die Kamera nicht sieht. Ein ist schneller und sieht gleich aus.",
			"Omite lo que la c\u{e1}mara no ve. Con S\u{ed} es m\u{e1}s r\u{e1}pido y se ve igual.",
			"Salta ci\u{f2} che la telecamera non vede. Con S\u{ec} \u{e8} pi\u{f9} veloce e l'immagine non cambia.",
		],
		NO,
		1,
		Cost::None,
		None,
	),
	row(
		"vegetation",
		CH,
		["Vegetation", "V\u{e9}g\u{e9}tation", "Vegetation", "Vegetaci\u{f3}n", "Vegetazione"],
		[
			"Extra grass and plants on the planets. Off is the original.",
			"Herbe et plantes en plus sur les plan\u{e8}tes. Avec Non, c'est l'original.",
			"Zus\u{e4}tzliches Gras und Pflanzen auf den Planeten. Aus ist das Original.",
			"Hierba y plantas extra en los planetas. Con No se mantiene el original.",
			"Erba e piante in pi\u{f9} sui pianeti. Con No resta l'originale.",
		],
		VEG_VALUES,
		1,
		costs::VEGETATION,
		LIGHT,
	),
	row(
		"water",
		CH,
		["Water", "Eau", "Wasser", "Agua", "Acqua"],
		[
			"Classic is the original water. Low to High use the improved water with reflections.",
			"Classique est l'eau d'origine. Faible \u{e0} \u{c9}lev\u{e9} utilisent l'eau am\u{e9}lior\u{e9}e avec reflets.",
			"Klassisch ist das Originalwasser. Niedrig bis Hoch nutzen das verbesserte Wasser mit Spiegelungen.",
			"Cl\u{e1}sico es el agua original. De Baja a Alta usan el agua mejorada con reflejos.",
			"Classico \u{e8} l'acqua originale. Da Bassa ad Alta usano l'acqua migliorata con riflessi.",
		],
		QC,
		3,
		costs::WATER,
		LIGHT,
	),
	row(
		"waves",
		TOG,
		["Waves", "Vagues", "Wellen", "Olas", "Onde"],
		[
			"Moving waves on the water surface.",
			"Vagues mobiles \u{e0} la surface de l'eau.",
			"Bewegte Wellen auf der Wasseroberfl\u{e4}che.",
			"Olas en movimiento sobre la superficie del agua.",
			"Onde in movimento sulla superficie dell'acqua.",
		],
		NO,
		1,
		costs::WAVES,
		WATER,
	),
	row(
		"underwater",
		TOG,
		["Underwater look", "Rendu sous l'eau", "Unterwasser-Optik", "Aspecto submarino", "Aspetto subacqueo"],
		[
			"Fog while the camera is under water, caustics from Water Medium, light shafts at High. Off is the original there; the surface stays improved.",
			"Brouillard quand la cam\u{e9}ra est sous l'eau, caustiques d\u{e8}s l'eau en Moyen, rayons de lumi\u{e8}re en \u{c9}lev\u{e9}. Avec Non, c'est l'original ; la surface reste am\u{e9}lior\u{e9}e.",
			"Nebel unter Wasser, Lichtmuster ab Wasser Mittel, Lichtstrahlen bei Hoch. Aus ist das Original, die Oberfl\u{e4}che bleibt verbessert.",
			"Niebla con la c\u{e1}mara bajo el agua, c\u{e1}usticas desde el agua en Media y haces de luz en Alta. Con No se mantiene el original; la superficie sigue mejorada.",
			"Nebbia con la telecamera sott'acqua, caustiche dall'acqua Media e fasci di luce con Alta. Con No resta l'originale; la superficie rimane migliorata.",
		],
		NO,
		1,
		costs::UNDERWATER,
		WATER,
	),
	row(
		"fog",
		CH,
		["Volumetric fog", "Brouillard volum\u{e9}trique", "Volumetrischer Nebel", "Niebla volum\u{e9}trica", "Nebbia volumetrica"],
		[
			"Classic is the original fog. Low to High give the fog depth, so light shines through it.",
			"Classique est le brouillard d'origine. Faible \u{e0} \u{c9}lev\u{e9} lui donnent de la profondeur, la lumi\u{e8}re le traverse.",
			"Klassisch ist der Originalnebel. Niedrig bis Hoch geben ihm Tiefe, sodass Licht hindurchscheint.",
			"Cl\u{e1}sico es la niebla original. De Baja a Alta le dan profundidad y la luz la atraviesa.",
			"Classico \u{e8} la nebbia originale. Da Bassa ad Alta le danno profondit\u{e0} e la luce la attraversa.",
		],
		QC,
		3,
		costs::FOG,
		LIGHT,
	),
	row(
		"clouds",
		CH,
		["Volumetric clouds", "Nuages volum\u{e9}triques", "Volumetrische Wolken", "Nubes volum\u{e9}tricas", "Nuvole volumetriche"],
		[
			"Classic is the original sky. Low to High add 3D clouds.",
			"Classique est le ciel d'origine. Faible \u{e0} \u{c9}lev\u{e9} ajoutent des nuages en 3D.",
			"Klassisch ist der Originalhimmel. Niedrig bis Hoch f\u{fc}gen 3D-Wolken hinzu.",
			"Cl\u{e1}sico es el cielo original. De Baja a Alta a\u{f1}aden nubes en 3D.",
			"Classico \u{e8} il cielo originale. Da Bassa ad Alta aggiungono nuvole 3D.",
		],
		QC,
		3,
		costs::CLOUDS,
		LIGHT,
	),
];

/// Waves and Underwater look are dimmed while Water is Classic (they change only the improved water). No banner over the tab: only
/// the row's box says why.
pub const NEEDS_WATER: NeedsRule = NeedsRule {
	row: "water",
	set_to: 3,
	banner: t(""),
	detail: [
		"Applies to the improved water. Water is {} now.",
		"S'applique \u{e0} l'eau am\u{e9}lior\u{e9}e. L'eau est sur {} pour l'instant.",
		"Gilt f\u{fc}r das verbesserte Wasser. Das Wasser steht auf {}.",
		"Se aplica al agua mejorada. Ahora est\u{e1} en {}.",
		"Vale per l'acqua migliorata. Ora \u{e8} su {}.",
	],
	button: ["Switch Water to High", "Passer l'eau en \u{c9}lev\u{e9}", "Wasser auf Hoch stellen", "Poner el agua en Alta", "Imposta l'acqua su Alta"],
};
