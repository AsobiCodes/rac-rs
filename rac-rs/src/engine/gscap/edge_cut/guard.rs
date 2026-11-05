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
//! It also tells whether a game's right-edge band (`set_side`, R&C1's breath tank) may move: `Guard::side` (`SideMark`).
//!
//! The Guard goes with the overlay it was measured on (`gscap::Overlay::guard`), not through a global: the picture on screen can be
//! several pictures behind the capture.

// GAME-HOOK (follow-up, dev s138 update): the thresholds (GUARD_COL, CELL_ROWS, PROMPT_MARGIN_ROWS, widget sizes) are R&C1 HUD measurements shared with
// rac1 host/handler/hud_place.rs; R&C2 gets the same measuring on its pictures (harmless, unused).

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
    /// The game's right-edge band (`set_side`) holds its widget and nothing else crosses the band's edges: only then may the HUD position
    /// move it (`SideMark`). False when the game declares no band.
    pub side: bool,
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
    if let Some(b) = SIDE.get() {
        if let Some(h) = side_hit(*b, sc, v) {
            SIDE_MARK.lock().unwrap_or_else(|e| e.into_inner()).add(h);
        }
    }
}

/// The picture was published with `rows` rows: its guard (pieces grouped, rows as fractions of `rows`); the next picture starts clean.
pub fn finish(rows: usize) -> Guard {
    let ps = std::mem::take(&mut *PIECES.lock().unwrap_or_else(|e| e.into_inner()));
    let side = std::mem::take(&mut *SIDE_MARK.lock().unwrap_or_else(|e| e.into_inner())).holds();
    Guard { side, ..guard_of(&ps, rows.max(1) as f32) }
}

/// The game's RIGHT-EDGE BAND (`viewer::hud_place::SideBand`, R&C1: the breath tank; GitHub #65): its left edge `u0` (fraction of the
/// width) and rows `v0`..`v1` (fractions of the scissor's height), declared once by the game (`set_side`; none = nothing measured). The HUD
/// position may move the band to the window's edge only when the picture holds the band's widget (pieces wholly inside it that span at least
/// `SIDE_TALL` of its rows) and no other visible piece crosses its edges (review R8-1: a tall HelpDesk box (x up to 478, its top above
/// row 324 from 7 lines on, decomp/notes/ingame_menu_leftovers.md P1-a), a marker, another level's widget would be torn at the band's edge
/// and smeared over its old place; a small piece wholly inside, with no tank, would be carried off to the window's edge).
static SIDE: std::sync::OnceLock<[f32; 3]> = std::sync::OnceLock::new();
/// The pieces wholly inside the band must span at least this part of its rows (the PAL tank: rows 115..301 of the band's 104..324).
const SIDE_TALL: f32 = 0.5;
/// A piece may pass the band's edges by this much (pixels) and still count as inside (linear sampling, half-pixel sprite edges).
const SIDE_SLACK: f32 = 0.5;

static SIDE_MARK: Mutex<SideMark> = Mutex::new(SideMark::EMPTY);

/// Declares the game's right-edge band (see `SIDE`); the first call wins (it is a fact of the game, the same every frame).
pub fn set_side(u0: f32, v0: f32, v1: f32) {
    let _ = SIDE.set([u0, v0, v1]);
}

/// What one visible primitive does to the band: lies wholly inside it (its rows in pixels) or crosses one of its edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SideHit {
    Inside { top: f32, bottom: f32, band_rows: f32 },
    Crossed,
}

/// The hit of a primitive with the screen-space vertices `v` on the band `b` (`SIDE`) under the scissor `sc`; None = it does not touch
/// the band, draws nothing (outside the scissor) or is a full-width fill (fades and letterbox bars: the stretch keeps them whole).
pub fn side_hit(b: [f32; 3], sc: [i32; 4], v: &[(f32, f32)]) -> Option<SideHit> {
    let (sx0, sx1) = (sc[0] as f32, sc[1] as f32 + 1.0);
    let (sy0, sy1) = (sc[2] as f32, sc[3] as f32 + 1.0);
    let (mut lx, mut hx, mut ly, mut hy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in v {
        (lx, hx, ly, hy) = (lx.min(p.0), hx.max(p.0), ly.min(p.1), hy.max(p.1));
    }
    let full = lx <= sx0 + FULL_SLACK && hx >= sx1 - FULL_SLACK;
    let (cx0, cx1, cy0, cy1) = (lx.max(sx0), hx.min(sx1), ly.max(sy0), hy.min(sy1));
    let (bx, bt, bb) = (b[0] * FRAME_W, sy0 + b[1] * (sy1 - sy0), sy0 + b[2] * (sy1 - sy0));
    if full || cx1 <= cx0 || cy1 <= cy0 || cx1 <= bx || cy1 <= bt || cy0 >= bb {
        return None;
    }
    let inside = cx0 >= bx - SIDE_SLACK && cy0 >= bt - SIDE_SLACK && cy1 <= bb + SIDE_SLACK;
    Some(if inside { SideHit::Inside { top: cy0, bottom: cy1, band_rows: bb - bt } } else { SideHit::Crossed })
}

/// The hits of one picture on the band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideMark {
    top: f32,
    bottom: f32,
    band_rows: f32,
    crossed: bool,
}

impl SideMark {
    pub const EMPTY: SideMark = SideMark { top: f32::MAX, bottom: f32::MIN, band_rows: 0.0, crossed: false };

    pub fn add(&mut self, h: SideHit) {
        match h {
            SideHit::Inside { top, bottom, band_rows } => {
                (self.top, self.bottom, self.band_rows) = (self.top.min(top), self.bottom.max(bottom), band_rows);
            }
            SideHit::Crossed => self.crossed = true,
        }
    }

    /// The band holds its widget and nothing crosses its edges (`Guard::side`).
    pub fn holds(&self) -> bool {
        !self.crossed && self.band_rows > 0.0 && self.bottom - self.top >= SIDE_TALL * self.band_rows
    }
}

impl Default for SideMark {
    fn default() -> SideMark {
        SideMark::EMPTY
    }
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

    /// Review R8-1: R&C1's breath-tank band (x 440.., rows 104..324 of 448). The tank (PAL, measured: frame x 451..490, rows 115..301,
    /// bubbles inside) lets it move, also while it slides in across the right edge; a HelpDesk box of 7 lines (retail layout P1-a: x 34..478,
    /// rows 309..436) crossing the band's bottom edge, a marker across its left edge, or a small piece alone (no tank) do not.
    #[test]
    fn the_side_band_moves_only_with_its_tank_and_nothing_across_its_edges() {
        let band = [440.0 / 512.0, 104.0 / 448.0, 324.0 / 448.0];
        let mark = |prims: &[&[(f32, f32)]]| {
            let mut m = SideMark::EMPTY;
            prims.iter().filter_map(|v| side_hit(band, SC, v)).for_each(|h| m.add(h));
            m.holds()
        };
        let tank: &[(f32, f32)] = &[(451.0, 115.0), (490.0, 301.0)];
        let bubble: &[(f32, f32)] = &[(470.0, 200.0), (474.0, 204.0)];
        let sliding: &[(f32, f32)] = &[(500.0, 111.0), (540.0, 303.0)];
        let help_box: &[(f32, f32)] = &[(34.0, 309.0), (478.0, 436.0)];
        let marker: &[(f32, f32)] = &[(420.0, 150.0), (460.0, 190.0)];
        let fade: &[(f32, f32)] = &[(-0.5, -0.5), (511.5, 447.5)];
        assert!(mark(&[tank, bubble]) && mark(&[sliding]) && mark(&[tank, fade]));
        assert!(!mark(&[tank, help_box]) && !mark(&[tank, marker]));
        assert!(!mark(&[help_box]) && !mark(&[bubble]) && !mark(&[]) && !mark(&[fade]));
        // A 6-line box (top row 325) stays below the band: the tank still moves beside it.
        assert!(mark(&[tank, &[(34.0, 325.0), (478.0, 436.0)]]));
        // Without a declared band nothing is measured: the guard of the tank picture says no.
        assert!(!guard(&[tank]).side);
    }
}
