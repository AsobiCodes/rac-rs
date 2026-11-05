//! The game's own item pictures in the modern menus (v5i, user 2026-11-02: "the weapons and gadgets and items should have their original
//! sprites there"). The ids the host registers them under (`host/ui_icons.rs` decodes the level's HUD sheets 0xea60 + item id once per level into
//! the engine's picture atlas, `rac_engine::ui::images`) and the one helper the pause grids and the vendor list draw them with.
//! Frames of an item sheet [C: decomp/notes/ui_item_icons.md]: 0 = the menu icon (blue weapons, green gadgets: Quick Select, the pause
//! grids, the vendor list), 1 = worn, 2 = the shop's ammo row, 3 = the ammo HUD, 4 = the gold version. Pure: no statics of its own.
use rac_engine::ui::images;
use rac_engine::ui::prim::{Prim, Rect, Rgba};

/// Item sheet ids: the HUD sheet 0xea60 + `id`, frame `frame` (0..=4).
const ITEM_BASE: u32 = 0x1000;
/// The Items page's nine pictures in `frontend::ITEM_GRID` order.
const GRID_BASE: u32 = 0x2000;
/// The item sheet frames the host registers.
pub const FRAMES: [u8; 4] = [0, 1, 2, 4];
pub const FRAME_MENU: u8 = 0;
pub const FRAME_AMMO_ROW: u8 = 2;

/// The picture id of item `id`'s sheet frame `frame`.
pub fn item(id: u8, frame: u8) -> u32 {
	ITEM_BASE + id as u32 * 8 + frame as u32
}

/// The picture id of the Items page's grid cell `j` (`frontend::ITEM_GRID` index).
pub fn grid(j: usize) -> u32 {
	GRID_BASE + j as u32
}

#[cfg(test)]
thread_local! {
	/// Tests: the pictures this thread pretends the atlas holds, (id, width, height); None = ask the real atlas.
	static FAKE: std::cell::RefCell<Option<Vec<(u32, u32, u32)>>> = const { std::cell::RefCell::new(None) };
}

/// The size of picture `id` in the atlas (None = not registered).
fn size(id: u32) -> Option<(u32, u32)> {
	#[cfg(test)]
	if let Some(fake) = FAKE.with(|f| f.borrow().clone()) {
		return fake.iter().find(|p| p.0 == id).map(|p| (p.1, p.2));
	}
	images::size_of(id)
}

/// Tests: runs `f` with `pics` (id, width, height) standing in for the atlas on this thread only (tests run in parallel).
#[cfg(test)]
pub fn with_pictures<R>(pics: &[(u32, u32, u32)], f: impl FnOnce() -> R) -> R {
	FAKE.with(|c| *c.borrow_mut() = Some(pics.to_vec()));
	let r = f();
	FAKE.with(|c| *c.borrow_mut() = None);
	r
}

/// The largest rect of picture `id`'s own aspect inside `bx`, centred (a picture the atlas does not hold: `bx`).
pub fn fit(id: u32, bx: Rect) -> Rect {
	let Some((w, h)) = size(id) else { return bx };
	let s = (bx.w / w as f32).min(bx.h / h as f32);
	let (pw, ph) = (w as f32 * s, h as f32 * s);
	Rect::new(bx.cx() - pw / 2.0, bx.cy() - ph / 2.0, pw, ph)
}

/// Draws picture `id` inside `bx` (aspect kept, centred, corners a little rounded); false when it is not registered (the caller draws
/// its fallback shape).
pub fn draw(out: &mut Vec<Prim>, id: u32, bx: Rect, tint: Rgba) -> bool {
	if size(id).is_none() {
		return false;
	}
	let rect = fit(id, bx);
	out.push(Prim::Image { rect, id, radius: rect.w.min(rect.h) * 0.08, tint });
	true
}

/// Draws item `id`'s menu icon (frame 0).
pub fn draw_item(out: &mut Vec<Prim>, id: u8, bx: Rect, tint: Rgba) -> bool {
	draw(out, item(id, FRAME_MENU), bx, tint)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ids_are_distinct_and_in_their_ranges() {
		let mut all: Vec<u32> = (0..64u8).flat_map(|i| FRAMES.map(|f| item(i, f))).chain((0..9).map(grid)).collect();
		let n = all.len();
		all.sort_unstable();
		all.dedup();
		assert_eq!(all.len(), n);
		assert!(item(63, 7) < GRID_BASE && grid(0) >= GRID_BASE);
	}

	#[test]
	fn an_unregistered_picture_draws_nothing_and_keeps_the_box() {
		let bx = Rect::new(1.0, 2.0, 30.0, 40.0);
		let mut out = Vec::new();
		assert!(!draw(&mut out, 0xFFFF_0001, bx, Rgba::rgba(255, 255, 255, 1.0)) && out.is_empty());
		assert_eq!(fit(0xFFFF_0001, bx), bx);
	}

	#[test]
	fn a_registered_picture_is_drawn_at_its_aspect_inside_the_box() {
		let bx = Rect::new(100.0, 200.0, 56.0, 56.0);
		let mut out = Vec::new();
		let drawn = with_pictures(&[(item(7, 0), 41, 38)], || draw_item(&mut out, 7, bx, Rgba::rgba(255, 255, 255, 1.0)));
		assert!(drawn);
		let [Prim::Image { rect, id, .. }] = &out[..] else { panic!("one image prim: {out:?}") };
		assert_eq!(*id, item(7, 0));
		assert!((rect.w / rect.h - 41.0 / 38.0).abs() < 1e-3 && rect.w <= bx.w + 1e-3 && (rect.cx() - bx.cx()).abs() < 1e-3);
		assert!(!with_pictures(&[], || draw_item(&mut out, 7, bx, Rgba::rgba(255, 255, 255, 1.0))));
	}
}
