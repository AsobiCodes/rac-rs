//! What the HUD position's top corner cells (viewer/handler/hud_place.rs) must know about a picture: which HUD pieces they would tear.
//!
//! The capture notes the bigger pieces of the upper picture while it draws the overlay (`note`); when the picture is published they are
//! grouped into widgets (`guard_of`: sprites with the same rows that touch each other, like the three sprites of a prompt bar) and each
//! widget is sorted by where it sits across the cells' inner edges (`GUARD_COL`, 0.34 and 0.66 of the width):
//!
//! - a CENTRED widget crosses both inner edges (the top help prompts "T Activate Gadgetron Vendor" / "T Enter ship", the Gadgetron title
//!   letters), or crosses one but sits mostly outside that corner: the cells must end above it (`Guard::centre`);
//! - a CORNER widget crosses one inner edge and sits mostly inside that corner (the hoverboard race's progress bar and position panel,
//!   top left): it belongs to the corner, the cell is widened to cover it whole (`Guard::left` / `right`).
//!
//! The Guard goes with the overlay it was measured on (`gscap::Overlay::guard`), not through a global: the picture on screen can be
//! several pictures behind the capture.

use std::sync::Mutex;

/// A corner cell's inner edge as a fraction of the overlay's width (hud_place.rs `CELL_W`; a test keeps them equal).
pub const GUARD_COL: f32 = 0.34;
/// Only pieces that start above this row (fraction of the picture's height) matter: the tallest the corner cells get (hud_place.rs
/// `CELL_H`; a test keeps them equal).
pub const CELL_ROWS: f32 = 0.18;
/// Pieces narrower than this (fraction of the picture's width) are sparks, digits and icons, not bars (a prompt bar's end caps are 32 of 512 px).
const MIN_W: f32 = 0.05;
/// Pieces taller than this (fraction of the picture's height) are fades and panels, not bars (a prompt bar is 32 or 54 rows of 448).
const MAX_H: f32 = 0.18;
/// A piece that reaches both edges of the scissor within this many pixels is a full-width fill (the HUD draws them from -0.5 to 511.5).
const FULL_SLACK: f32 = 1.0;
/// The most pieces kept per picture (a HUD has a handful of bars; more are ignored).
const MAX_PIECES: usize = 96;
/// Sprites of one widget share their rows (pixel rows, tolerance) and touch each other (fraction of the width, ~1.5 px of 512).
const SAME_ROWS: f32 = 1.0;
const TOUCH_U: f32 = 1.5 / 512.0;
/// The top help prompt's bar sprites start this many rows above where the cells may end (measured PAL vendor prompt: sprite top row 41.5,
/// first visible pixels 44.4; the USA vendor prompt: sprite 53.5, pixels from 56.47): the bar's texture has a transparent top margin of
/// 2.9 rows and the cut keeps 1 row of it (the overlay is sampled linearly: the cell's last window row must not blend the prompt's first
/// row in, and the stretch beside the cell repeats that row across the gap). PAL: cut at 43.4, above the boxes' bottom 41.5; USA: 55.4, above
/// 52.3. Only a widget as tall as a prompt bar (`BAR_ROWS`) has it.
const PROMPT_MARGIN_ROWS: f32 = 1.9;
/// The overlay's width in pixels (`gscap::W`, the same constant): the pieces' `u` is a fraction of it, whatever the scissor.
const FRAME_W: f32 = super::super::W as f32;
/// Heights (rows) of a one-line and a two-line prompt bar.
const BAR_ROWS: [f32; 2] = [32.0, 54.0];

/// A corner widget's reach: how far it goes towards the centre (`u`, fraction of the width: its right end for the left corner, its left
/// end for the right one) and the rows it spans (fractions of the picture's height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub u: f32,
    pub top: f32,
    pub bottom: f32,
}

/// The measurements of one overlay picture for the corner cells (all fractions of the picture's size).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Guard {
    /// First visible row of the highest centred widget: the cells must end above it. None: nothing centred in the upper picture.
    pub centre: Option<f32>,
    /// The highest centred widget is a top help prompt's bar (a widget as tall as `BAR_ROWS`): it fades in over several fields, so a
    /// fallback it asks for is real, never a one-picture transient (viewer/handler/hud_hold.rs). False when `centre` is None.
    pub centre_bar: bool,
    /// The widest widget of the left / right corner that crosses its cell's inner edge.
    pub left: Option<Reach>,
    pub right: Option<Reach>,
}

/// A piece of the picture in the capture: `u0`..`u1` as fractions of the overlay's width, `top`..`bottom` in pixel rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    u0: f32,
    u1: f32,
    top: f32,
    bottom: f32,
}

static PIECES: Mutex<Vec<Piece>> = Mutex::new(Vec::new());

/// The piece a primitive with the screen-space vertices `v` makes for the guard, `None` when it cannot matter: too narrow, taller than a bar,
/// a full-width fill, wholly above the scissor (parked off screen: draws nothing) or starting below the cells' rows.
pub fn piece(sc: [i32; 4], v: &[(f32, f32)]) -> Option<Piece> {
    let (sx0, sx1) = (sc[0] as f32, sc[1] as f32 + 1.0);
    let (sy0, sy1) = (sc[2] as f32, sc[3] as f32 + 1.0);
    let sh = sy1 - sy0;
    let (mut lx, mut hx, mut ly, mut hy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in v {
        lx = lx.min(p.0);
        hx = hx.max(p.0);
        ly = ly.min(p.1);
        hy = hy.max(p.1);
    }
    let full = lx <= sx0 + FULL_SLACK && hx >= sx1 - FULL_SLACK;
    // Only the part inside the scissor is drawn: its width and its place count, as fractions of the overlay.
    let (cx0, cx1, top) = (lx.max(sx0), hx.min(sx1), ly.max(sy0));
    let ok = cx1 - cx0 >= MIN_W * FRAME_W && !full && hy > sy0 + 0.5 && top < sy0 + CELL_ROWS * sh && hy - ly <= MAX_H * sh;
    ok.then_some(Piece { u0: cx0 / FRAME_W, u1: cx1 / FRAME_W, top, bottom: hy.min(sy1) })
}

/// Notes a visible primitive (see `edge_cut::visible`) of the overlay for the guard.
pub fn note(sc: [i32; 4], v: &[(f32, f32)]) {
    if let Some(p) = piece(sc, v) {
        let mut ps = PIECES.lock().unwrap_or_else(|e| e.into_inner());
        if ps.len() < MAX_PIECES {
            ps.push(p);
        }
    }
}

/// The picture was published with `rows` rows: its guard (pieces grouped, rows as fractions of `rows`); the next picture starts clean.
pub fn finish(rows: usize) -> Guard {
    let ps = std::mem::take(&mut *PIECES.lock().unwrap_or_else(|e| e.into_inner()));
    guard_of(&ps, rows.max(1) as f32)
}

/// Whether the two pieces are sprites of one widget: the same rows, touching or overlapping in x.
fn same_widget(a: &Piece, b: &Piece) -> bool {
    (a.top - b.top).abs() <= SAME_ROWS && (a.bottom - b.bottom).abs() <= SAME_ROWS && a.u0 <= b.u1 + TOUCH_U && b.u0 <= a.u1 + TOUCH_U
}

fn union(a: &Piece, b: &Piece) -> Piece {
    Piece { u0: a.u0.min(b.u0), u1: a.u1.max(b.u1), top: a.top.min(b.top), bottom: a.bottom.max(b.bottom) }
}

/// The guard of a picture of `rows` rows from its pieces.
pub fn guard_of(pieces: &[Piece], rows: f32) -> Guard {
    let mut widgets: Vec<Piece> = Vec::new();
    for p in pieces {
        let mut p = *p;
        while let Some(i) = widgets.iter().position(|w| same_widget(w, &p)) {
            p = union(&p, &widgets.swap_remove(i));
        }
        widgets.push(p);
    }
    let (cl, cr) = (GUARD_COL, 1.0 - GUARD_COL);
    let mut g = Guard::default();
    for w in &widgets {
        let (left, right) = (w.u0 < cl && w.u1 > cl, w.u0 < cr && w.u1 > cr);
        // Mostly inside the corner it crosses into: a corner widget. Any other piece across an inner edge is centred.
        let in_left = left && !right && cl - w.u0 > w.u1 - cl;
        let in_right = right && !left && w.u1 - cr > cr - w.u0;
        if in_left {
            let r = g.left.get_or_insert(Reach { u: w.u1, top: w.top / rows, bottom: w.bottom / rows });
            *r = Reach { u: r.u.max(w.u1), top: r.top.min(w.top / rows), bottom: r.bottom.max(w.bottom / rows) };
        } else if in_right {
            let r = g.right.get_or_insert(Reach { u: w.u0, top: w.top / rows, bottom: w.bottom / rows });
            *r = Reach { u: r.u.min(w.u0), top: r.top.min(w.top / rows), bottom: r.bottom.max(w.bottom / rows) };
        } else if left || right {
            let bar = BAR_ROWS.iter().any(|h| (w.bottom - w.top - h).abs() <= 1.0);
            let first = (w.top + if bar { PROMPT_MARGIN_ROWS } else { 0.0 }) / rows;
            // The highest centred widget decides (on a tie the bar counts).
            if g.centre.map_or(true, |c| first < c) {
                g.centre = Some(first);
                g.centre_bar = bar;
            } else if g.centre == Some(first) {
                g.centre_bar |= bar;
            }
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    const SC: [i32; 4] = [0, 511, 0, 447];

    fn guard(prims: &[&[(f32, f32)]]) -> Guard {
        let ps: Vec<Piece> = prims.iter().filter_map(|v| piece(SC, v)).collect();
        guard_of(&ps, 448.0)
    }

    /// The vendor prompt: a bar of three sprites (caps and a stretched middle) x 107..405, y 41.5..73.5; the middle crosses both edges.
    #[test]
    fn a_vendor_prompt_is_centred_and_the_cells_end_at_its_first_visible_row() {
        let g = guard(&[&[(107.0, 41.5), (139.0, 73.5)], &[(139.0, 41.5), (373.0, 73.5)], &[(373.0, 41.5), (405.0, 73.5)]]);
        assert_eq!((g.left, g.right), (None, None));
        assert!((g.centre.unwrap() * 448.0 - 43.4).abs() < 1e-3 && g.centre_bar, "{g:?}");
        // The USA vendor prompt: the same bar 12 rows lower (pixels from row 56.47, measured): the cut keeps a row of the margin.
        let u = guard(&[&[(107.0, 53.5), (139.0, 85.5)], &[(139.0, 53.5), (373.0, 85.5)], &[(373.0, 53.5), (405.0, 85.5)]]);
        assert!((u.centre.unwrap() * 448.0 - 55.4).abs() < 1e-3, "{u:?}");
    }

    /// The ship prompt is a short bar x 166..346: each end cap alone crosses only ONE inner edge (174 / 338) and sits mostly outside that
    /// corner, so even without the stretched middle it is centred, not a corner piece.
    #[test]
    fn a_short_prompt_bars_end_caps_are_centred_not_corner_pieces() {
        let caps = guard(&[&[(166.0, 41.5), (198.0, 73.5)], &[(314.0, 41.5), (346.0, 73.5)]]);
        assert_eq!((caps.left, caps.right), (None, None));
        assert!((caps.centre.unwrap() * 448.0 - 43.4).abs() < 1e-3 && caps.centre_bar);
        let whole = guard(&[&[(166.0, 41.5), (198.0, 73.5)], &[(198.0, 41.5), (314.0, 73.5)], &[(314.0, 41.5), (346.0, 73.5)]]);
        assert_eq!(whole, caps, "with the middle it is one widget crossing both edges");
        // The bar that is drawn with alpha 0 is never noted (`edge_cut::visible`), so nothing is left here.
        assert_eq!(guard(&[]), Guard::default());
    }

    /// The hoverboard race: a progress bar (19.5, 14.5)-(275.5, 78.5) and a position panel of three sprites ending in a cap at 163.5..195.5
    /// (rows 54.5..86.5) start inside the left column and cross only its inner edge: the left corner's, no centred guard.
    #[test]
    fn the_race_bar_and_panel_belong_to_the_left_corner() {
        let g = guard(&[
            &[(19.5, 14.5), (275.5, 78.5)],
            &[(105.5, 54.5), (131.5, 86.5)],
            &[(131.5, 54.5), (163.5, 86.5)],
            &[(163.5, 54.5), (195.5, 86.5)],
        ]);
        assert_eq!((g.centre, g.right, g.centre_bar), (None, None, false));
        let l = g.left.unwrap();
        assert!((l.u - 275.5 / 512.0).abs() < 1e-5, "reaches the bar's right end: {l:?}");
        assert!((l.top * 448.0 - 14.5).abs() < 1e-3 && (l.bottom * 448.0 - 86.5).abs() < 1e-3, "{l:?}");
        // The mirror image on the right.
        let r = guard(&[&[(236.5, 14.5), (492.5, 78.5)]]).right.unwrap();
        assert!((r.u - 236.5 / 512.0).abs() < 1e-5 && r.top < r.bottom);
    }

    /// Gadgetron title letters over the ammo box: a big letter across an inner edge is centred (no margin: it is no prompt bar).
    #[test]
    fn a_title_letter_across_an_edge_is_centred_without_the_prompt_margin() {
        let g = guard(&[&[(160.0, 20.0), (200.0, 60.0)]]);
        assert!((g.centre.unwrap() * 448.0 - 20.0).abs() < 1e-3 && !g.centre_bar, "{g:?}");
    }

    /// `centre_bar` says whether the HIGHEST centred widget is a prompt bar: a bar with letters above it is not, letters below it are.
    #[test]
    fn the_highest_centred_widget_decides_whether_it_is_a_bar() {
        let bar: [&[(f32, f32)]; 3] = [&[(107.0, 41.5), (139.0, 73.5)], &[(139.0, 41.5), (373.0, 73.5)], &[(373.0, 41.5), (405.0, 73.5)]];
        let above: &[(f32, f32)] = &[(160.0, 5.0), (200.0, 30.0)];
        let below: &[(f32, f32)] = &[(160.0, 120.0), (200.0, 160.0)];
        let g = guard(&[bar[0], bar[1], bar[2], above]);
        assert!(!g.centre_bar && (g.centre.unwrap() * 448.0 - 5.0).abs() < 1e-3, "{g:?}");
        let g = guard(&[bar[0], bar[1], bar[2], below]);
        assert!(g.centre_bar && (g.centre.unwrap() * 448.0 - 43.4).abs() < 1e-3, "{g:?}");
        assert!(!guard(&[]).centre_bar, "nothing centred: no bar");
    }

    /// The pieces' u is a fraction of the 512-pixel overlay under any scissor; only the part inside the scissor counts.
    #[test]
    fn u_is_a_fraction_of_the_overlay_not_of_the_scissor() {
        let narrow = [64, 447, 0, 447];
        let p = piece(narrow, &[(100.0, 14.5), (356.0, 78.5)]).unwrap();
        assert!((p.u0 - 100.0 / 512.0).abs() < 1e-6 && (p.u1 - 356.0 / 512.0).abs() < 1e-6, "{p:?}");
        // Cut by the scissor's left edge: the visible part starts there. A sliver of 20 visible pixels is no bar.
        assert!((piece(narrow, &[(30.0, 14.5), (300.0, 78.5)]).unwrap().u0 - 64.0 / 512.0).abs() < 1e-6);
        assert_eq!(piece(narrow, &[(30.0, 14.5), (84.0, 78.5)]), None);
    }

    #[test]
    fn sparks_fades_panels_and_parked_sprites_are_no_pieces() {
        // A spark crossing the edge (16 px), the red triangles of the Hoven ship scene (14 px).
        assert_eq!(piece(SC, &[(167.5, 57.5), (183.5, 73.5)]), None);
        assert_eq!(piece(SC, &[(332.4, 110.75), (346.75, 102.0), (338.75, 114.75)]), None);
        // Full-width fills, also the HUD's half-pixel convention -0.5 .. 511.5; a tall panel; a piece far down.
        assert_eq!(piece(SC, &[(-1.0, 0.0), (513.0, 60.0)]), None);
        assert_eq!(piece(SC, &[(-0.5, 10.5), (511.5, 70.5)]), None);
        assert_eq!(piece(SC, &[(100.0, 0.0), (400.0, 300.0)]), None);
        assert_eq!(piece(SC, &[(100.0, 300.0), (400.0, 330.0)]), None);
        // Wholly above the scissor (the retail HUD parks sprites there): draws nothing.
        assert_eq!(piece(SC, &[(190.0, -60.0), (330.0, -10.0)]), None);
        assert!(piece(SC, &[(190.0, -30.0), (330.0, 10.0)]).is_some(), "partly on screen counts");
        // The pieces inside a corner (the ammo box) are kept but cross no edge.
        let g = guard(&[&[(10.0, 10.0), (150.0, 42.0)], &[(190.0, 10.0), (320.0, 40.0)]]);
        assert_eq!(g, Guard::default());
    }
}
