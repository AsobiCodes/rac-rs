//! The words of the modern vendor shop page (v0.5 screens stage E) in the game's five languages (English, French, German, Spanish,
//! Italian). New PC words, so they are plain `T5` tables like `title.rs`'s (no text ids): retail's own words (item names) come from
//! the disc's text bank. Native-speaker review is open [I]. A `{}` in a word is where a name or a number goes (`fill`).
use rac_engine::ui::T5;

pub const HEADER: T5 = ["Gadgetron Vendor", "Vendeur Gadgetron", "Gadgetron-H\u{e4}ndler", "Vendedor Gadgetron", "Venditore Gadgetron"];
pub const TITLE: T5 = ["Weapons and ammo", "Armes et munitions", "Waffen und Munition", "Armas y munici\u{f3}n", "Armi e munizioni"];
pub const NEW: T5 = ["New", "Nouveau", "Neu", "Nuevo", "Nuovo"];
pub const OWNED: T5 = ["Owned", "Poss\u{e9}d\u{e9}", "Im Besitz", "Tienes", "Posseduto"];
pub const FULL: T5 = ["Full", "Plein", "Voll", "Lleno", "Pieno"];
pub const NEED_BOLTS: T5 = ["Need bolts", "Boulons requis", "Zu wenig Bolzen", "Faltan tuercas", "Servono bulloni"];
/// "Ammo 22 / 40 . 5 Bolts each" is `AMMO` + counts + `EACH` (the bolts word is `title::BOLTS`).
pub const AMMO: T5 = ["Ammo", "Munitions", "Munition", "Munici\u{f3}n", "Munizioni"];
/// The singular of `title::BOLTS` ("1 Bolt each", user 2026-11-02 F5).
pub const BOLT: T5 = ["Bolt", "Boulon", "Bolzen", "Tuerca", "Bullone"];
pub const EACH: T5 = ["each", "pi\u{e8}ce", "pro St\u{fc}ck", "c/u", "cadauna"];
pub const PRICE: T5 = ["Price", "Prix", "Preis", "Precio", "Prezzo"];
pub const FILL_UP: T5 = ["Fill up {} ammo", "Remplir {} munitions", "{} Munition auff\u{fc}llen", "Rellenar {} de munici\u{f3}n", "Riempi {} munizioni"];
pub const AMMO_FULL: T5 = ["Ammo is full", "Munitions pleines", "Munition voll", "Munici\u{f3}n llena", "Munizioni piene"];
pub const BUY_AMMO: T5 = ["Buy ammo", "Acheter des munitions", "Munition kaufen", "Comprar munici\u{f3}n", "Compra munizioni"];
pub const BUY_WEAPON: T5 = ["Buy weapon", "Acheter l'arme", "Waffe kaufen", "Comprar arma", "Compra arma"];
pub const NOT_ENOUGH: T5 = ["Not enough bolts", "Pas assez de boulons", "Nicht genug Bolzen", "Faltan tuercas", "Bulloni insufficienti"];
pub const ASK_WEAPON: T5 = ["Buy {}?", "Acheter {} ?", "{} kaufen?", "\u{bf}Comprar {}?", "Comprare {}?"];
pub const ASK_AMMO: T5 = ["Buy {} ammo", "Acheter munitions {}", "Munition f\u{fc}r {} kaufen", "Comprar munici\u{f3}n de {}", "Compra munizioni {}"];
pub const HOW_MANY: T5 = ["How many", "Combien", "Wie viele", "Cu\u{e1}ntas", "Quante"];
pub const TOTAL: T5 = ["Total", "Total", "Gesamt", "Total", "Totale"];
pub const BUY: T5 = ["Buy", "Acheter", "Kaufen", "Comprar", "Acquista"];
pub const CANCEL: T5 = ["Cancel", "Annuler", "Abbrechen", "Cancelar", "Annulla"];
pub const LEAVE: T5 = ["Leave", "Quitter", "Verlassen", "Salir", "Esci"];
pub const BUY_ALL: T5 = ["Buy all ammo", "Acheter toutes les munitions", "Alle Munition kaufen", "Comprar toda la munici\u{f3}n", "Compra tutte le munizioni"];
pub const FEWER: T5 = ["Fewer", "Moins", "Weniger", "Menos", "Meno"];
pub const MORE: T5 = ["More", "Plus", "Mehr", "M\u{e1}s", "Di pi\u{f9}"];
/// The Buy all ammo card: the ammo it fills ("Refills: Blaster, Pyrocitor").
pub const REFILLS: T5 = ["Refills: {}", "Recharge : {}", "F\u{fc}llt auf: {}", "Recarga: {}", "Ricarica: {}"];
/// The names `REFILLS` has no room for, counted ("Refills: Blaster, Pyrocitor, +4 more").
pub const REFILLS_MORE: T5 = ["+{} more", "+{} autres", "+{} weitere", "+{} m\u{e1}s", "+{} altri"];

/// The word in language `lang` (0..4, clamped).
pub fn word(t: &T5, lang: usize) -> &'static str {
	t[lang.min(4)]
}

/// The word with its `{}` replaced by `x`.
pub fn fill(t: &T5, lang: usize, x: &str) -> String {
	word(t, lang).replace("{}", x)
}

/// A number with thousands commas (retail's "%d,%03d", and on for the bolts past a million).
pub fn commas(n: u32) -> String {
	let s = n.to_string();
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if i > 0 && (s.len() - i) % 3 == 0 {
			out.push(',');
		}
		out.push(c);
	}
	out
}

/// Every table, for the font-coverage test.
pub fn all() -> [&'static T5; 27] {
	[&HEADER, &TITLE, &NEW, &OWNED, &FULL, &NEED_BOLTS, &AMMO, &BOLT, &EACH, &PRICE, &FILL_UP, &AMMO_FULL, &BUY_AMMO, &BUY_WEAPON, &NOT_ENOUGH, &ASK_WEAPON, &ASK_AMMO, &HOW_MANY, &TOTAL, &BUY, &CANCEL, &LEAVE, &BUY_ALL, &FEWER, &MORE, &REFILLS, &REFILLS_MORE]
}
