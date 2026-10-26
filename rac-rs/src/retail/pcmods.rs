//! The Mods pages: texture packs, script / native mods and the mods' settings, switched from inside the game. Two flavours of one page:
//!
//! - PAUSE (Pause -> Options -> Mods): the "Texture packs" master tick, one On / Off row per installed texture pack, then the BASIC
//!   settings of the script mods that are running (rows from the loader's own list, `omni::live_settings::loaded`). Script and native
//!   mods are NOT switched on or off here (user 2026-10-23: never in the game menu); a settings row writes `mods/settings.json` and
//!   changes the running mod at once (`omni.setting` returns the new value, the "setting" event fires).
//! - TITLE (title screen -> Options -> Mods, `build_title`): the "Load mods" master tick, one On / Off row per installed script / native
//!   mod each followed by its settings rows (read from the mod's manifest) while it is on, then the Texture packs tick and the packs.
//!   The loader starts once per run, so On / Off here (`mods/active.txt`, the launcher's format) applies at the next start; a settings
//!   row of a mod that is running applies at once as in the pause menu.
//!
//! Built like the Controls page (`pcctl.rs`): the Options list gains a row, the row opens a copy of a retail value page with its own
//! header and multi-choice item; the row table (pause: 24-byte records `{label id, byte*, v0, v1, v2, v3}`, title: the front end's 5-word
//! rows `{label id, byte*, v0, v1, 0}`, each ended by a zero record) is data the host rewrites. More than six items on the pause page
//! (title: more than 11, or more than `MAX_SETS` settings rows) make sections with a "Page" stepper (row 0; Cross or Left / Right on
//! it); every item is on exactly one section. The title mirror scrolls a column that is taller than its panel with the cursor, so the
//! 13 rows of a title section are all reachable.
//!
//! The files: `mods/active.txt` in EXACTLY the launcher's format (`texpack/active.rs`, `texpack/store.rs`) and `mods/settings.json`
//! (`omni/plugins.rs`, a write changes one key and keeps the rest). Every flip is written at once. Rows with more than two values: the
//! pause item cycles them; the title's value item only flips a byte, so `sync` steps such a row on from its last value (as pcext.rs).
//!
//! The Options list shows the pause row only when an installed mod (a pack or a script / native mod) is switched on in `active.txt`
//! (`load_store`, once per planet load) and the title row when one is installed (`build_title`, at every entry to Options).
//!
//! Reserved RAM (`MBASE`, inside pcext's BASE window, above pcctl's blocks): the row bytes (byte i = record i; pause 1 = On, settings
//! rows hold the value index), the pause page copy, its header and value items, row table and text pool (all dynamic labels and values
//! are text ids of this pool), then the title page, its header and value items, row table and the title Options list records. All of it
//! is written only after a build set the magic word, and a flip counts only while its page is the open one.
use super::mouse_menu;
use super::pcctl::{self, Pager};
use super::pcext::BASE;
use crate::omni::json::Json;
use crate::omni::{live_settings, modset, plugins};
use crate::texpack::active::Master;
use crate::texpack::store::{Installed, Store};
use ee_oracle::cpu::Cpu;
use ee_oracle::mem::Mem;
use modset::SetRow;
#[cfg(not(test))]
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The region: BASE + 0xC000 .. + 0xD800 (the canary of prompts.rs skips it).
pub const MBASE: u32 = BASE + 0xC000;
pub const RANGE: std::ops::Range<u32> = MBASE..MBASE + 0x1800;
const MAGIC_AT: u32 = MBASE + 0xfc;
const MAGIC: u32 = 0x5344_4f4d;
const MPAGE: u32 = MBASE + 0x100;
const MHDR: u32 = MBASE + 0x200;
const MVAL: u32 = MBASE + 0x280;
const MTAB: u32 = MBASE + 0x300;
/// Text pool: one 0x40-byte slot per id (`ID_FIRST..ID_END`).
const POOL: u32 = MBASE + 0x400;
const SLOT: u32 = 0x40;
/// The title page (a copy of Video Options), its header item, its value item, that item's 5-word rows, the Options list records.
pub const TPAGE: u32 = MBASE + 0x1400;
const THDR: u32 = MBASE + 0x1500;
pub const TVAL: u32 = MBASE + 0x1580;
const TTAB: u32 = MBASE + 0x1600;
pub const TRECS: u32 = MBASE + 0x1740;
/// The pause page's address (the Options record's argument).
pub const PAGE_AT: u32 = MPAGE;

/// Rows of the pause page (a taller page would pack the rows closer than the font is high) and of the title page (the Graphics
/// page's 13 fit its panel).
const ROWS: usize = 7;
const TROWS: usize = 13;
/// Pause: records of the row table with its zero record.
#[cfg(test)]
const TABLE_RECS: usize = ROWS + 1;
/// Packs listed at most (more installed ones are not shown).
const MAX_MODS: usize = 60;
/// Settings rows on one section: each takes up to four value texts of the pool (`ID_VAL..ID_END` = 35 ids), so more rows page.
const MAX_SETS: usize = 8;

/// Text ids 0x7fc0..0x8000 (free: pcctl has 0x7f40..0x7f80 and 0x7fa0..0x7fb0, camopts 0x7f80..0x7f84).
pub const ID_FIRST: u32 = 0x7fc0;
pub const ID_END: u32 = 0x8000;
/// The Options list row, its description, the page header.
pub const ID_MODS: u32 = 0x7fc0;
pub const ID_DESC: u32 = 0x7fc1;
pub const ID_HEADER: u32 = 0x7fc2;
/// The packs master row's label, the "Load mods" tick's label, the note row's label (on the title another text) and (empty) value, the
/// stepper's value "< 1/3 >".
const ID_PACKS: u32 = 0x7fc3;
const ID_TICK: u32 = 0x7fc4;
const ID_NOTE_L: u32 = 0x7fc5;
const ID_NOTE_V: u32 = 0x7fc6;
const ID_STEP: u32 = 0x7fc7;
/// The stepper row's label "Page" on the title (the mirror takes its texts from this pool; the pause page uses pcctl's).
const ID_PAGE_T: u32 = 0x7fc8;
/// The labels of pack, mod and settings rows: 0x7fd0 + the row's record index.
const ID_ROW: u32 = 0x7fd0;
/// The value texts of settings rows (choice names, numbers) are handed out from here, one id per text.
const ID_VAL: u32 = ID_ROW + TROWS as u32;
const ID_ON: u32 = 0x4f5a;
const ID_OFF: u32 = 0x4f5b;

// ---- texts ---------------------------------------------------------------------------------------------------------------

type T5 = [&'static [u8]; 5];
static MODS: T5 = [b"Mods", b"Mods", b"Mods", b"Mods", b"Mod"];
static DESC: T5 = [b"Mods and texture packs", b"Mods et packs de textures", b"Mods und Texturpakete", b"Mods y paq. de texturas", b"Mod e pacchetti texture"];
static HEADER: T5 = [b"Mods Options", b"Options des mods", b"Mod-Optionen", b"Opciones de mods", b"Opzioni mod"];
static PACKS: T5 = [b"Texture packs", b"Packs textures", b"Texturpakete", b"Paq. texturas", b"Pacchetti texture"];
static TICK: T5 = [b"Load mods", b"Charger les mods", b"Mods laden", b"Cargar mods", b"Carica mod"];
static PAGE: T5 = [b"Page", b"Page", b"Seite", b"P\x84gina", b"Pagina"];
/// The note row (its value stays empty): texture packs take effect at the next planet load; on the title, mods at the next start.
static NOTE: T5 = [b"Packs change at the next planet", b"Effet au prochain niveau", b"Gilt ab dem n\x87chsten Planeten", b"Cambia en el siguiente planeta", b"Cambia al prossimo pianeta"];
static NOTE_T: T5 = [b"Mods change at the next start", b"Effet au prochain d\x82marrage", b"Gilt ab dem n\x87chsten Start", b"Cambia al reiniciar", b"Cambia al riavvio"];

/// What a row of the page is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
	/// The section stepper (record 0 of a page with sections).
	Step,
	/// The "Texture packs" master tick.
	Packs,
	/// Installed texture pack `j` (index into `packs`).
	Pack(usize),
	/// The "Load mods" master tick (title).
	Tick,
	/// Installed script / native mod `j` (index into `scripts`; title).
	Mod(usize),
	/// Settings row `k` (index into `St::sets`).
	Set(usize),
	/// The inert row that tells when changes apply.
	Note,
}

/// The installed texture packs, in the store's order (by name).
fn packs(store: &Store) -> Vec<&Installed> {
	store.mods.iter().filter(|m| Master::of_kind(&m.kind) == Master::Textures).collect()
}

/// The installed script / native mods, in the store's order.
fn scripts(store: &Store) -> Vec<&Installed> {
	store.mods.iter().filter(|m| Master::of_kind(&m.kind) == Master::Mods).collect()
}

/// The pause page's items in order: the packs master tick (when packs are installed), one row per pack, then the settings rows. `layout`
/// pages this list.
fn pause_items(npacks: usize, nsets: usize) -> Vec<Row> {
	let packs = npacks.min(MAX_MODS);
	(npacks > 0).then_some(Row::Packs).into_iter().chain((0..packs).map(Row::Pack)).chain((0..nsets).map(Row::Set)).collect()
}

/// The title page's items: the "Load mods" tick, each mod with its settings rows, the packs tick and the packs (`layout` pages the
/// list). `owner[k]` = the script index that settings row `k` belongs to.
fn title_items(nscripts: usize, owner: &[usize], npacks: usize) -> Vec<Row> {
	let mut v: Vec<Row> = Vec::new();
	if nscripts > 0 {
		v.push(Row::Tick);
		for j in 0..nscripts {
			v.push(Row::Mod(j));
			v.extend(owner.iter().enumerate().filter(|(_, o)| **o == j).map(|(k, _)| Row::Set(k)));
		}
	}
	if npacks > 0 {
		v.push(Row::Packs);
		v.extend((0..npacks.min(MAX_MODS)).map(Row::Pack));
	}
	v
}

/// The item ranges of the sections of a page with at most `rows` rows: one section when everything and the note fit, else the
/// stepper and the note leave `rows - 2` items, of which at most `MAX_SETS` are settings rows (the value text pool).
fn chunks(items: &[Row], rows: usize) -> Vec<std::ops::Range<usize>> {
	let is_set = |r: &Row| matches!(r, Row::Set(_));
	if items.len() < rows && items.iter().filter(|r| is_set(r)).count() <= MAX_SETS {
		return vec![0..items.len()];
	}
	let (mut out, mut start, mut sets) = (Vec::new(), 0, 0);
	for (i, r) in items.iter().enumerate() {
		if i - start == rows - 2 || sets + is_set(r) as usize > MAX_SETS {
			out.push(start..i);
			(start, sets) = (i, 0);
		}
		sets += is_set(r) as usize;
	}
	out.push(start..items.len());
	out
}

/// Sections of a page with these items and at most `rows` rows.
fn sections(items: &[Row], rows: usize) -> usize {
	chunks(items, rows).len()
}

/// The rows of section `sec` (clamped): [stepper], the section's items, the note. At most `rows`.
fn layout(items: &[Row], sec: usize, rows: usize) -> Vec<Row> {
	let ch = chunks(items, rows);
	let mut out = Vec::new();
	if ch.len() > 1 {
		out.push(Row::Step);
	}
	out.extend_from_slice(&items[ch[sec.min(ch.len() - 1)].clone()]);
	out.push(Row::Note);
	out
}

/// The row label of an installed pack or mod: its name cut to `MAX_LABEL` characters.
fn label(m: &Installed) -> Vec<u8> {
	modset::font_text(&m.name, modset::MAX_LABEL, false).into_bytes()
}

/// The text id of a record's label.
fn label_id(r: Row, rec: usize, title: bool) -> u32 {
	match r {
		Row::Step => if title { ID_PAGE_T } else { pcctl::ID_PAGE },
		Row::Note => ID_NOTE_L,
		Row::Packs => ID_PACKS,
		Row::Tick => ID_TICK,
		Row::Pack(_) | Row::Mod(_) | Row::Set(_) => ID_ROW + rec as u32,
	}
}

/// Every text of the page in `lang`.
fn text(id: u32, lang: usize, st: &St) -> Vec<u8> {
	let l = lang.min(4);
	let fixed = |t: &T5| t[l].to_vec();
	let Some(store) = &st.store else { return Vec::new() };
	match id {
		ID_MODS => fixed(&MODS),
		ID_DESC => fixed(&DESC),
		ID_HEADER => fixed(&HEADER),
		ID_PACKS => fixed(&PACKS),
		ID_PAGE_T => fixed(&PAGE),
		ID_TICK => fixed(&TICK),
		// The pause note is about texture packs; with none installed (settings only, they apply at once) the row stays blank.
		ID_NOTE_L if !st.title && packs(store).is_empty() => Vec::new(),
		ID_NOTE_L => fixed(if st.title { &NOTE_T } else { &NOTE }),
		ID_STEP => format!("< {}/{} >", st.pager.sec + 1, st.sections()).into_bytes(),
		_ if (ID_ROW..ID_VAL).contains(&id) => match st.rows.get((id - ID_ROW) as usize) {
			Some(Row::Pack(j)) => packs(store).get(*j).map(|m| label(m)).unwrap_or_default(),
			Some(Row::Mod(j)) => scripts(store).get(*j).map(|m| label(m)).unwrap_or_default(),
			Some(Row::Set(k)) => st.sets.get(*k).map(|s| s.name.clone().into_bytes()).unwrap_or_default(),
			_ => Vec::new(),
		},
		_ if id >= ID_VAL => st.vtext.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone().into_bytes()).unwrap_or_default(),
		_ => Vec::new(),
	}
}

/// The text lookup hand for the page's ids (`pcext::text_hook`, `ID_FIRST..ID_END`): the string goes into the id's pool slot and its
/// address into v0, in the game's language `lang`.
pub fn text_hook(c: &mut Cpu, id: u32, lang: usize) -> bool {
	let s = state(|st| text(id, lang, st));
	let at = POOL + SLOT * (id - ID_FIRST);
	let n = s.len().min(SLOT as usize - 1);
	for (i, b) in s[..n].iter().enumerate() {
		c.mem.wr8(at + i as u32, *b);
	}
	c.mem.wr8(at + n as u32, 0);
	c.s(2, at as u64);
	true
}

/// Every text of the page in `lang` as the title mirror's strings (font bytes as chars), for the ids that have one now.
pub fn texts(lang: usize) -> Vec<(u32, String)> {
	state(|st| (ID_FIRST..ID_END).map(|id| (id, text(id, lang, st))).filter(|(_, t)| !t.is_empty()).map(|(id, t)| (id, t.iter().map(|&b| b as char).collect())).collect())
}

// ---- state ---------------------------------------------------------------------------------------------------------------

struct St {
	/// The mods folder as read at the last page build (None until a page was built on this process).
	store: Option<Store>,
	/// The title page is the built one (else the pause page).
	title: bool,
	pager: Pager,
	/// Fields Left / Right have been held (auto-repeat).
	held: [u32; 2],
	/// The rows of the section shown, by record.
	rows: Vec<Row>,
	/// The settings rows (`Row::Set(k)`), and for the title the script each belongs to.
	sets: Vec<SetRow>,
	owner: Vec<usize>,
	/// Per record: the value shown (0 / 1, or the settings value index), the byte as last seen (title steps), the value text ids.
	shown: Vec<u8>,
	vals: Vec<u8>,
	vids: Vec<[u32; 4]>,
	/// The dynamic value texts: (id, text).
	vtext: Vec<(u32, String)>,
	/// A failed write was reported.
	warned: bool,
}

impl St {
	const NEW: St = St {
		store: None,
		title: false,
		pager: Pager { sec: 0, seen: 0xff },
		held: [0; 2],
		rows: Vec::new(),
		sets: Vec::new(),
		owner: Vec::new(),
		shown: Vec::new(),
		vals: Vec::new(),
		vids: Vec::new(),
		vtext: Vec::new(),
		warned: false,
	};

	fn max_rows(&self) -> usize {
		if self.title { TROWS } else { ROWS }
	}

	fn items(&self) -> Vec<Row> {
		let Some(store) = &self.store else { return Vec::new() };
		if self.title { title_items(scripts(store).len(), &self.owner, packs(store).len()) } else { pause_items(packs(store).len(), self.sets.len()) }
	}

	fn sections(&self) -> usize {
		sections(&self.items(), self.max_rows())
	}
}

/// The worker's state: a mutex in the game, a thread-local in tests (each test thread has its own, like `pcctl`).
#[cfg(not(test))]
fn state<R>(f: impl FnOnce(&mut St) -> R) -> R {
	static ST: Mutex<St> = Mutex::new(St::NEW);
	f(&mut ST.lock().unwrap_or_else(|e| e.into_inner()))
}
#[cfg(test)]
fn state<R>(f: impl FnOnce(&mut St) -> R) -> R {
	thread_local! { static ST: std::cell::RefCell<St> = const { std::cell::RefCell::new(St::NEW) }; }
	ST.with(|s| f(&mut s.borrow_mut()))
}

/// The title page is the open page (set by `front_field`; read by the mirror).
static TITLE_OPEN: AtomicBool = AtomicBool::new(false);

pub fn title_open() -> bool {
	TITLE_OPEN.load(Ordering::Relaxed)
}

// ---- the page ------------------------------------------------------------------------------------------------------------

/// The value text ids of a record: Off / On, or the setting's own values.
fn value_ids(r: Row, st: &mut St, next: &mut u32) -> [u32; 4] {
	match r {
		Row::Step => [ID_STEP, ID_STEP, 0, 0],
		Row::Note => [ID_NOTE_V, ID_NOTE_V, 0, 0],
		Row::Set(k) => {
			let mut ids = [0; 4];
			let values: Vec<String> = st.sets[k].values.iter().map(|v| v.1.clone()).collect();
			if values.len() == 2 && values[0] == "Off" && values[1] == "On" {
				return [ID_OFF, ID_ON, 0, 0];
			}
			for (slot, v) in ids.iter_mut().zip(values) {
				// The same text as an earlier value (numbers, "Normal") shares its id; past the last id the last one is reused.
				if let Some((id, _)) = st.vtext.iter().find(|(_, t)| *t == v) {
					*slot = *id;
					continue;
				}
				*slot = (*next).min(ID_END - 1);
				st.vtext.push((*slot, v));
				*next += 1;
			}
			ids
		}
		_ => [ID_OFF, ID_ON, 0, 0],
	}
}

/// The row table of the rows: pause 6-word records (byte i = `MBASE + i`), title 5-word rows `{label, byte, v0, v1, 0}`.
fn table(rows: &[Row], vids: &[[u32; 4]], title: bool) -> Vec<u32> {
	let mut t = Vec::new();
	for (i, (r, v)) in rows.iter().zip(vids).enumerate() {
		let at = MBASE + i as u32;
		if title {
			t.extend([label_id(*r, i, true), at, v[0], v[1], 0]);
		} else {
			t.extend([label_id(*r, i, false), at, v[0], v[1], v[2], v[3]]);
		}
	}
	t
}

/// Writes a table at `at`, padded with zero records to the capacity of `n` records.
fn write_table(mem: &mut Mem, at: u32, t: &[u32], words: usize, n: usize) {
	let mut t = t.to_vec();
	t.resize(n * words, 0);
	for (i, w) in t.iter().enumerate() {
		let a = at + 4 * i as u32;
		if mem.rd32(a) != *w {
			mem.wr32(a, *w);
		}
	}
}

/// The value index a record shows now.
fn value_of(st: &St, store: &Store, r: Row) -> u8 {
	match r {
		Row::Step | Row::Note => 0,
		Row::Packs => store.masters.is_on(Master::Textures) as u8,
		Row::Pack(j) => packs(store).get(j).is_some_and(|m| store.is_on(&m.id)) as u8,
		Row::Tick => store.masters.is_on(Master::Mods) as u8,
		Row::Mod(j) => scripts(store).get(j).is_some_and(|m| store.is_on(&m.id)) as u8,
		Row::Set(k) => st.sets.get(k).map_or(0, |s| s.cur as u8),
	}
}

/// The row bytes from the store and the settings (the stepper's byte is the selector and stays; a note row reads 0).
fn pull(mem: &mut Mem, st: &mut St) {
	let Some(store) = &st.store else { return };
	let shown: Vec<u8> = st.rows.iter().map(|r| value_of(st, store, *r)).collect();
	for (i, r) in st.rows.iter().enumerate() {
		if *r != Row::Step && mem.rd8(MBASE + i as u32) != shown[i] {
			mem.wr8(MBASE + i as u32, shown[i]);
		}
	}
	st.vals = shown.clone();
	st.shown = shown;
}

/// The page shows section `sec`: its rows, value texts, bytes and table.
fn go(mem: &mut Mem, st: &mut St, sec: usize) {
	if st.store.is_none() {
		return;
	}
	let items = st.items();
	let rows = st.max_rows();
	let sec = sec.min(sections(&items, rows) - 1);
	st.pager.sec = sec;
	st.rows = layout(&items, sec, rows);
	st.vtext.clear();
	let mut next = ID_VAL;
	let rs = st.rows.clone();
	st.vids = rs.iter().map(|r| value_ids(*r, st, &mut next)).collect();
	pull(mem, st);
	let t = table(&st.rows, &st.vids, st.title);
	if st.title {
		write_table(mem, TTAB, &t, 5, TROWS + 1);
	} else {
		write_table(mem, MTAB, &t, 6, ROWS + 1);
	}
}

/// The settings rows of the mods the loader runs (the pause page): from its own list, with the current values.
fn loaded_sets() -> Vec<SetRow> {
	live_settings::loaded().iter().flat_map(|m| m.settings.iter().filter_map(|s| modset::row(&m.id, s, s.get("value")))).collect()
}

/// The title page's settings rows: for each script mod that is switched on, its manifest's basic settings with the stored values.
fn title_sets(store: &Store) -> (Vec<SetRow>, Vec<usize>) {
	let doc = plugins::read_settings(&store.root);
	let (mut sets, mut owner) = (Vec::new(), Vec::new());
	for (j, m) in scripts(store).iter().enumerate().filter(|(_, m)| store.is_on(&m.id)) {
		let Some((mid, _, list)) = modset::manifest(&store.root.join(&m.folder)) else { continue };
		let rows = modset::rows(&mid, &list, &|k| plugins::value_of(&doc, &mid, k).cloned());
		owner.extend(rows.iter().map(|_| j));
		sets.extend(rows);
	}
	(sets, owner)
}

/// Builds the Mods page in reserved RAM: a copy of the Camera Options page `cp` (header item `hdr`, multi-choice item `val`), the
/// same way `pcctl::build_page` builds the Controls page; `sound_seq` is the Sound page's slot-3 panel sequence (the taller value
/// panel that holds the 7 rows). `store` is the mods folder as `load_store` read it for this planet load (the page is rebuilt at
/// every planet load, only when the Options list shows the row). The caller repoints the Options list afterwards.
pub fn build_page(mem: &mut Mem, cp: u32, hdr: u32, val: u32, sound_seq: Option<u32>, store: Store) {
	build_with(mem, [cp, hdr, val], sound_seq, store, loaded_sets());
}

/// The mods folder (`RAC_MODS_DIR`, else `<install root>/mods`) read for the Options list build, only when the row is wanted: at
/// least one installed mod is switched on in `active.txt`, whatever its kind and whatever the two master ticks say. None = no mods
/// folder to read (no install root and no `RAC_MODS_DIR`, e.g. the dev gate) or no mod on: the list is exactly the retail one
/// plus Graphics and Controls. Decided once per planet load: switching the last mod off in the game keeps the row (and the page)
/// until the next load, so a row never vanishes under the cursor.
pub fn load_store() -> Option<Store> {
	read_store().filter(row_wanted)
}

/// The mods folder, read.
fn read_store() -> Option<Store> {
	#[cfg(test)]
	if let Some(root) = TEST_ROOT.with(|r| r.borrow().clone()) {
		return Some(Store::load(&root));
	}
	crate::texpack::mods::mods_root().map(|r| Store::load(&r))
}

fn row_wanted(s: &Store) -> bool {
	s.mods.iter().any(|m| s.is_on(&m.id))
}

#[cfg(test)]
thread_local! { static TEST_ROOT: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) }; }

/// Test hook: `load_store` / `build_title` of this thread read `root` (None = the real lookup).
#[cfg(test)]
pub fn test_root(root: Option<&std::path::Path>) {
	TEST_ROOT.with(|r| *r.borrow_mut() = root.map(|p| p.to_path_buf()));
}

/// The copy of a retail value page: `page` at `at` (0x100 bytes), its header item `hdr` at `h` and its value item `val` at `v`; `src` =
/// [page, header item, value item] of the retail page, `cp_words` = words of the page struct to copy.
fn copy_page(mem: &mut Mem, [src, hdr, val]: [u32; 3], seq: Option<u32>, [at, h, v, table]: [u32; 4], page_words: u32, slots: std::ops::Range<u32>) {
	pcctl::copy_words(mem, src, at, page_words);
	for o in (page_words..0x100).step_by(4) {
		mem.wr32(at + o, 0);
	}
	if let Some(seq) = seq {
		mem.wr32(at + 0xc, seq);
	}
	for o in slots.step_by(4) {
		match mem.rd32(at + o) {
			a if a == hdr => mem.wr32(at + o, h),
			a if a == val => mem.wr32(at + o, v),
			_ => {}
		}
	}
	pcctl::copy_words(mem, hdr, h, 0x58);
	mem.wr32(h + 0x34, ID_HEADER);
	pcctl::copy_words(mem, val, v, 0x40);
	mem.wr32(v + 0x34, table);
	mem.wr32(v + 0x38, 0);
	mem.wr32(v + 0x3c, 0);
}

fn build_with(mem: &mut Mem, items: [u32; 3], sound_seq: Option<u32>, store: Store, sets: Vec<SetRow>) {
	copy_page(mem, items, sound_seq, [MPAGE, MHDR, MVAL, MTAB], 0x60, 0x40..0x60);
	state(|st| {
		*st = St { store: Some(store), sets, ..St::NEW };
		TITLE_OPEN.store(false, Ordering::Relaxed);
		mem.wr8(MBASE, 0);
		go(mem, st, 0);
	});
	mem.wr32(MAGIC_AT, MAGIC);
}

/// The title's Options list (`pcext::build_title`): builds the title Mods page when a mod or pack is installed and returns its Options
/// record. `[video, hdr, val]` are Video Options' page, header item and value item, `seq` the Sound page's slot-3 panel sequence.
pub fn build_title(mem: &mut Mem, items: [u32; 3], seq: u32) -> Option<[u32; 3]> {
	let store = read_store().filter(|s| !s.mods.is_empty())?;
	build_title_with(mem, items, seq, store);
	Some([ID_MODS | 3 << 16, TPAGE, 0])
}

/// `pcext::build_title`: appends the Mods record (when there is one) to `recs`; returns the record table to use (`table`, or the Mods one).
pub fn title_extend(mem: &mut Mem, items: [u32; 3], seq: u32, recs: &mut Vec<[u32; 3]>, table: u32) -> u32 {
	let extra = build_title(mem, items, seq);
	let t = if extra.is_some() { TRECS } else { table };
	recs.extend(extra);
	t
}

fn build_title_with(mem: &mut Mem, items: [u32; 3], seq: u32, store: Store) {
	copy_page(mem, items, Some(seq), [TPAGE, THDR, TVAL, TTAB], 0x88, 0x40..0x80);
	let (sets, owner) = title_sets(&store);
	state(|st| {
		*st = St { store: Some(store), title: true, sets, owner, ..St::NEW };
		TITLE_OPEN.store(false, Ordering::Relaxed);
		mem.wr8(MBASE, 0);
		go(mem, st, 0);
	});
	mem.wr32(MAGIC_AT, MAGIC);
}

/// The title page as the mirror draws it (viewer/front_mirror.rs): each row's label and value text ids; None unless it is the open page.
pub fn title_rows() -> Option<Vec<(u32, u32)>> {
	if !title_open() {
		return None;
	}
	state(|st| {
		let rows = st.rows.iter().zip(&st.vids).zip(&st.shown).enumerate();
		Some(rows.map(|(i, ((r, v), s))| (label_id(*r, i, true), v[(*s as usize).min(3)])).collect())
	})
}

// ---- worker --------------------------------------------------------------------------------------------------------------

/// Worker, each field of a level (`pcext::pre_field`, after `pcctl::field`): the section stepper, when the page has sections.
/// `buttons` is the field's pad word (Left / Right, also the mouse wheel).
pub fn field(mem: &mut Mem, level: u32, buttons: u16) {
	field_with(mem, buttons, &|mem| pcctl::on_selector(mem, level, MPAGE, MVAL));
}

/// `field` with the question "is the page open with the cursor on the stepper" given.
fn field_with(mem: &mut Mem, buttons: u16, on_row: &dyn Fn(&mut Mem) -> bool) {
	if mem.rd32(MAGIC_AT) != MAGIC {
		return;
	}
	state(|st| {
		if st.title {
			return;
		}
		let mut dir = [false; 2];
		for (i, bit) in [mouse_menu::LEFT, mouse_menu::RIGHT].into_iter().enumerate() {
			st.held[i] = if buttons & bit != 0 { st.held[i].saturating_add(1) } else { 0 };
			dir[i] = pcctl::repeat_due(st.held[i]);
		}
		let n = st.sections();
		if n <= 1 {
			return;
		}
		let sel = mem.rd8(MBASE);
		if let Some(next) = pcctl::pager_step(&mut st.pager, sel, dir, n, || on_row(mem)) {
			go(mem, st, next);
		}
	});
}

/// Worker, every field after the page patch (`pcext::sync`): reads the row bytes the retail item flipped and writes `active.txt` /
/// `settings.json`. Does nothing until `build_page` ran on this RAM.
pub fn sync(mem: &mut Mem, level: u32) {
	if mem.rd32(MAGIC_AT) != MAGIC {
		return;
	}
	let open = pcctl::open_page(mem, level, MPAGE, |_, _| true);
	sync_with(mem, open, false);
}

/// Worker, each field of the front end (`worker/field_run.rs`): the title page's rows, while it is the open page.
pub fn front_field(mem: &mut Mem) {
	if mem.rd32(MAGIC_AT) != MAGIC || !state(|st| st.title) {
		TITLE_OPEN.store(false, Ordering::Relaxed);
		return;
	}
	let open = mem.rd32(crate::retail::disc::va(mouse_menu::FRONT_PAGE_PTR)) == TPAGE;
	TITLE_OPEN.store(open, Ordering::Relaxed);
	if open {
		let on_row = mem.rd32(TVAL + 0x38) == 0;
		title_step(mem, on_row);
	}
	sync_with(mem, open, true);
}

/// The title page's stepper (only when the items need sections): the value item flips the row's byte on Cross and on Left / Right
/// alike, so any flip with the cursor on row 0 goes on to the next section (wrapping).
fn title_step(mem: &mut Mem, on_row: bool) {
	state(|st| {
		let n = st.sections();
		if n > 1 {
			let sel = mem.rd8(MBASE);
			if let Some(next) = pcctl::pager_step(&mut st.pager, sel, [false; 2], n, || on_row) {
				go(mem, st, next);
			}
		}
	});
}

/// Persists settings row `k` at value `idx`: `settings.json` (one key) and, when the mod is running, the change at the next tick.
fn persist(root: &std::path::Path, row: &SetRow, idx: usize) -> std::io::Result<()> {
	let value: Json = row.values[idx].0.clone();
	let mut doc = plugins::read_settings(root);
	plugins::set_value(&mut doc, &row.mod_id, &row.key, value.clone());
	plugins::write_settings(root, &doc)?;
	live_settings::queue(&row.mod_id, &row.key, &value.to_text());
	Ok(())
}

/// `sync` with the question "is this the open page" given and which flavour is wanted. A flip counts only while the page is open;
/// otherwise the bytes are put back from the store (a savestate's stale bytes never change a file).
fn sync_with(mem: &mut Mem, open: bool, title: bool) {
	state(|st| {
		if st.title != title || st.store.is_none() {
			return;
		}
		let mut store = st.store.take().expect("checked");
		let (mut files, mut rebuild) = (Vec::new(), false);
		let rows = st.rows.clone();
		for (i, r) in rows.iter().enumerate() {
			let at = MBASE + i as u32;
			let mut b = mem.rd8(at);
			let want = b != 0;
			match *r {
				Row::Note if want => mem.wr8(at, 0),
				Row::Packs if open && want != store.masters.is_on(Master::Textures) => {
					store.masters.set(Master::Textures, want);
					files.push(format!("{} {}", Master::Textures.label(), if want { "on" } else { "off" }));
				}
				Row::Tick if open && want != store.masters.is_on(Master::Mods) => {
					store.masters.set(Master::Mods, want);
					files.push(format!("{} {}", Master::Mods.label(), if want { "on" } else { "off" }));
				}
				Row::Pack(j) | Row::Mod(j) => {
					let m = if matches!(r, Row::Pack(_)) { packs(&store) } else { scripts(&store) };
					if let Some(id) = m.get(j).map(|m| m.id.clone()).filter(|id| open && want != store.is_on(id)) {
						store.set_mod(&id, want);
						files.push(format!("{id} {}", if want { "on" } else { "off" }));
						rebuild |= matches!(r, Row::Mod(_));
					}
				}
				Row::Set(k) => {
					let n = st.sets[k].values.len() as u8;
					if st.title && n > 2 && b != st.vals[i] {
						b = (st.vals[i] + 1) % n;
						mem.wr8(at, b);
					}
					b = b.min(n - 1);
					if b != mem.rd8(at) {
						mem.wr8(at, b);
					}
					if open && b as usize != st.sets[k].cur {
						match persist(&store.root, &st.sets[k], b as usize) {
							Ok(()) => {
								eprintln!("retail: PC options: mods: setting {}.{} = {} ({})", st.sets[k].mod_id, st.sets[k].key, st.sets[k].values[b as usize].1, store.root.join(plugins::SETTINGS_FILE).display());
								st.sets[k].cur = b as usize;
							}
							Err(e) => {
								if !std::mem::replace(&mut st.warned, true) {
									eprintln!("retail: PC options: cannot write {}: {e}", store.root.join(plugins::SETTINGS_FILE).display());
								}
							}
						}
					}
				}
				_ => {}
			}
		}
		if !files.is_empty() {
			match store.save() {
				Ok(()) => eprintln!("retail: PC options: mods: {} ({})", files.join(", "), store.root.join("active.txt").display()),
				Err(e) => {
					if !std::mem::replace(&mut st.warned, true) {
						eprintln!("retail: PC options: cannot write {}: {e}", store.root.join("active.txt").display());
					}
					store = Store::load(&store.root.clone());
					rebuild = true;
				}
			}
		}
		st.store = Some(store);
		if rebuild && st.title {
			// A mod switched on or off: its settings rows appear or go.
			let (sets, owner) = title_sets(st.store.as_ref().expect("put back"));
			(st.sets, st.owner) = (sets, owner);
			go(mem, st, 0);
		} else {
			pull(mem, st);
		}
	});
}

/// Test hook: the pause page is built from the mods folder `root` (RAM at `0x300000..`), as if the player opened Options; `sets` = the
/// settings rows of the running mods.
#[cfg(test)]
pub fn test_open(mem: &mut Mem, root: &std::path::Path) {
	build_with(mem, [0x30_0000, 0x30_0200, 0x30_0300], None, Store::load(root), Vec::new());
}

#[cfg(test)]
#[path = "pcmods_tests.rs"]
mod tests;
