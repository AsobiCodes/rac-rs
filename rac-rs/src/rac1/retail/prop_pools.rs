//! The retail prop pools handed to `Game::with_story`: counting them from the moby table, widening them for the
//! level being loaded, the deferred writes of a level prefetch, and the growth of a pool that ran out.

use super::*;

pub(super) static POOLS: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

/// Spare props per class beyond the savestate's count, for mobys the game spawns later.
pub const SPARE: u32 = 4;

/// Props per class for a class the level loads but neither the start state nor a placement holds (spawned mobys:
/// bombs, debris, Glove of Doom bots, gadget mobys); `Retail::sync` grows a pool past it on demand.
pub const SPAWN: u32 = 8;

/// Classes the viewer has a prop model for (non-empty faces), set by `load_level`: `sync` grows only their pools.
static MODELLED: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);

/// [PC] The landing hint (`pools_hint`) takes a moby table only up to this many slots and with class ids below `HINT_CLASS`. Real
/// tables (43 whole-game savestates) span 551-1789 slots and hold class ids up to 1973; on some trips the level byte already names the
/// new level while the table pointers still hold other data (Rilgar -> Blarg: a "table" with 26,000+ classes, counted every field of
/// a ~30 s landing, each new hint a whole prop slot order on the main thread: 15-19 fps).
pub(super) const HINT_SLOTS: u32 = 4096;

pub(super) const HINT_CLASS: u32 = 0x1000;

/// (class, props) of a level's live moby table: the count per class plus half of it (at least `SPARE`).
pub fn count_pools(level: u32, rd32: impl Fn(u32) -> u32, rd16: impl Fn(u32) -> u16) -> Vec<(u32, u32)> {
	let (s, e) = table(level);
	let (s, e) = (rd32(s) & 0x01ff_ffff, rd32(e) & 0x01ff_ffff);
	let mut n: HashMap<u32, u32> = HashMap::new();
	let mut a = s;
	while a < e && a < 0x0200_0000 - 0x100 {
		let class = rd16(a + 0xa6) as u32;
		if rd32(a + 0x24) != 0 && class != 0 {
			*n.entry(class).or_insert(0) += 1;
		}
		a += 0x100;
	}
	// [PC] A class with many live mobys at the start spawns more of them: half its count again (at least `SPARE`), so its
	// pool rarely grows after the load (`Retail::sync`).
	let mut pools: Vec<(u32, u32)> = n.into_iter().map(|(c, k)| (c, k + SPARE.max(k / 2))).collect();
	pools.sort();
	pools
}

/// (class, count) of the retail prop pools `Game::with_story` appends in retail mode.
pub fn pools() -> Vec<(u32, u32)> {
	if let Some(p) = POOLS_OVERRIDE.with(|o| o.borrow().clone()) {
		return p;
	}
	POOLS.lock().map(|v| v.clone()).unwrap_or_default()
}

thread_local! {
	static POOLS_OVERRIDE: std::cell::RefCell<Option<Vec<(u32, u32)>>> = const { std::cell::RefCell::new(None) };
}

/// [PC] Runs `f` with `pools()` answering `p` on this thread (the level switch asks for the prop slot order of pools it has not
/// set yet).
pub fn with_pools<T>(p: Vec<(u32, u32)>, f: impl FnOnce() -> T) -> T {
	POOLS_OVERRIDE.with(|o| *o.borrow_mut() = Some(p));
	let r = f();
	POOLS_OVERRIDE.with(|o| *o.borrow_mut() = None);
	r
}

/// [PC] (level, pools) of the level retail is landing on, counted from its moby table before the arrival (worker, every field of
/// the landing once the table holds mobys); None after the arrival. Only a guess for the level prefetch: the switch uses the arrival's.
pub(super) static POOLS_HINT: Mutex<Option<(u32, Vec<(u32, u32)>)>> = Mutex::new(None);

pub fn pools_hint(level: u32) -> Option<Vec<(u32, u32)>> {
	POOLS_HINT.lock().ok().and_then(|h| h.as_ref().filter(|h| h.0 == level).map(|h| h.1.clone()))
}

/// The pools of the level retail travelled to (`Retail::switch`), for the next `Game::with_story`.
pub fn set_pools(p: Vec<(u32, u32)>) {
	if let Ok(mut v) = POOLS.lock() {
		*v = p;
	}
}

/// Widens the pools of the level being loaded (viewer `load_level`, before the prop slot order and the game are built):
/// each placed class gets at least its placement count + `SPARE`, every other class the level loads `SPARE`. The game
/// rewrites a moby's class in place (+0xa6 / +0x22 / +0x24, e.g. the Eudora bridges 0x41c -> 0x1b0 when built and
/// back on a reload, FUN_002c6180) and spawns classes the start state did not hold; without a pool `sync` finds no free
/// prop and the moby is not drawn. Idempotent (a second call leaves the pools as they are).
pub fn widen_pools(placed: &[crate::gameplay::MobyInstance], loaded: impl IntoIterator<Item = (u32, u32)>) {
	if deferring() {
		let (k, loaded) = (placed_counts(placed), loaded.into_iter().collect::<Vec<(u32, u32)>>());
		DEFER_WIDEN.with(|d| d.borrow_mut().push((k.clone(), loaded.clone())));
		defer(Box::new(move || set_pools(widened_counts(pools(), k, loaded))));
		return;
	}
	set_pools(widened(pools(), placed, loaded));
}

/// [PC] A level prefetch (viewer `app.rs` load_level, run on a background thread while the game is still in the travel
/// sequence) must not touch the globals of the level being played: its writes to them are queued here (in call order) and
/// run by `run_deferred` on the main thread at the level switch, where the same calls would have run.
pub type Effect = Box<dyn FnOnce() + Send>;

/// One deferred `widen_pools`: the placement counts per class and the loaded classes.
pub type Widen = (HashMap<u32, u32>, Vec<(u32, u32)>);

thread_local! {
	static DEFER: std::cell::RefCell<Option<Vec<Effect>>> = const { std::cell::RefCell::new(None) };
	static DEFER_WIDEN: std::cell::RefCell<Vec<Widen>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The `widen_pools` calls this thread deferred, in call order (their effects are in `defer_take`'s queue).
pub fn defer_take_widen() -> Vec<Widen> {
	DEFER_WIDEN.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// The pools the deferred widenings leave when run on `pools` (what `set_pools` holds after the queued effects ran).
pub fn widen_all(mut pools: Vec<(u32, u32)>, widen: &[Widen]) -> Vec<(u32, u32)> {
	for (k, loaded) in widen {
		pools = widened_counts(pools, k.clone(), loaded.iter().copied());
	}
	pools
}

/// This thread queues its global writes (the prefetch thread, between `defer_begin` and `defer_take`).
pub fn deferring() -> bool {
	DEFER.with(|d| d.borrow().is_some())
}

pub fn defer_begin() {
	DEFER.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

pub fn defer_take() -> Vec<Effect> {
	DEFER.with(|d| d.borrow_mut().take().unwrap_or_default())
}

pub fn defer(f: Effect) {
	DEFER.with(|d| d.borrow_mut().get_or_insert_with(Vec::new).push(f));
}

/// `widen_pools` as a pure function (the audit tool replays it): `pools` widened so each placed class holds at least its
/// placement count + `SPARE` and every other loaded class `SPAWN`.
pub fn widened(pools: Vec<(u32, u32)>, placed: &[crate::gameplay::MobyInstance], loaded: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
	widened_counts(pools, placed_counts(placed), loaded)
}

fn placed_counts(placed: &[crate::gameplay::MobyInstance]) -> HashMap<u32, u32> {
	let mut k: HashMap<u32, u32> = HashMap::new();
	for m in placed.iter().filter(|m| m.class_id != 0) {
		*k.entry(m.class_id).or_insert(0) += 1;
	}
	k
}

fn widened_counts(pools: Vec<(u32, u32)>, k: HashMap<u32, u32>, loaded: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
	let mut n: HashMap<u32, u32> = pools.into_iter().collect();
	for (c, k) in k {
		let e = n.entry(c).or_insert(0);
		*e = (*e).max(k + SPARE);
	}
	for (c, verts) in loaded.into_iter().filter(|&(c, _)| c != 0) {
		let e = n.entry(c).or_insert(0);
		*e = (*e).max(spawn_pool(verts));
	}
	let mut p: Vec<(u32, u32)> = n.into_iter().collect();
	p.sort();
	p
}

/// The classes the viewer built a prop model for (`load_level`, retail mode); `sync` grows only their pools.
pub fn set_modelled(c: HashMap<u32, u32>) {
	if let Ok(mut v) = MODELLED.lock() {
		*v = Some(c);
	}
}

pub(super) fn modelled(c: u32) -> bool {
	MODELLED.lock().is_ok_and(|v| v.as_ref().is_some_and(|s| s.contains_key(&c)))
}

/// [PC] Vertices of the prop model of class `c` (`set_modelled`), 0 when unknown.
fn model_verts(c: u32) -> u32 {
	MODELLED.lock().ok().and_then(|v| v.as_ref().and_then(|s| s.get(&c).copied())).unwrap_or(0)
}

/// [PC] Pool size of a class the level loads but nothing holds (spawned mobys): `SPAWN`, more for a small model. Growing a
/// pool at run time rebuilds the level's whole prop index (about 14 ms on a laptop, a visible hitch), the spawn bursts seen are
/// 9-12 live of a 8-prop pool, and a spare prop of a 200-vertex model costs a few KB: a vertex budget decides, 8..=`SPAWN_MAX`.
pub fn spawn_pool(verts: u32) -> u32 {
	if no_room() {
		return SPAWN;
	}
	(SPAWN_VERTS / verts.max(1)).clamp(SPAWN, SPAWN_MAX)
}

/// RAC_POOL_ROOM=0: the old pool sizes (SPAWN props, doubling), to compare the growth hitch.
fn no_room() -> bool {
	static V: OnceLock<bool> = OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_POOL_ROOM").is_ok_and(|v| v == "0"))
}

/// Spare vertices a spawn-only class pool aims at, and the most props that gets (see `spawn_pool`).
const SPAWN_VERTS: u32 = 6000;

const SPAWN_MAX: u32 = 32;

/// Vertices a grown pool leaves as spare room at least (`grown`), and the most props that gets.
const GROW_VERTS: u32 = 24000;

const GROW_MAX: u32 = 96;

/// A pool that ran out in `sync`: `need` live mobys of the class this field. The new count leaves room for the next
/// burst: `need` + `SPAWN`, and at least twice the old count and twice `need` so a growing class rebuilds the slots rarely.
pub fn grown(class: u32, old: u32, need: u32) -> u32 {
	let room = if no_room() { 0 } else { (GROW_VERTS / model_verts(class).max(1)).clamp(SPAWN, GROW_MAX) };
	(need + SPAWN).max(old * 2).max(need * 2).max(need + room)
}
