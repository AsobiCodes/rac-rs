//! Shop ticker: the scrolling LED text strip of the shop. `shop_ticker_set` writes a message into the strip,
//! `shop_ticker_panel` scrolls it each frame and picks a random filler message when it runs out, and
//! `shop_ticker_overlay` blanks the strip and draws an extra overlay in mode 1.
use crate::h;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Text pointer of the strip (shop state block + 0x2c).
const TEXT_PTR: u32 = 0x1ca56c;
/// Scroll counter of the strip (shop state block + 0x44), in pixels.
const SCROLL: u32 = 0x1ca584;
/// Filler message table (0x1d0000 - 0x5ec8): one message pointer per entry.
const FILLER_TABLE: u32 = 0x1ca138;
/// Number of filler messages.
const FILLER_COUNT: u32 = 0x18;
/// Box id of the strip, passed to `TICKER_TEXT_SET` as a1.
const TICKER_BOX: u32 = 0x1db150;
/// Pixels the strip scrolls per frame.
const SCROLL_STEP: i32 = 2;
/// Divisor turning the scroll counter into a frame count.
const SCROLL_DIV: i32 = 20;
/// Overlay mode word (0x161108, overlay data read at call time).
const MODE_WORD: u32 = 0x161108;
/// Mode value that draws the overlay.
const MODE_OVERLAY: u32 = 1;
/// Argument word of the overlay draw (0x1d0000 - 0x5a9c).
const OVERLAY_ARG: u32 = 0x1ca564;
/// Upper half of the shop state base, left in v0 before the overlay draw.
const SHOP_HI: u32 = 0x1d_0000;
/// Shop state block (s0 in the retail code), set in v0 by the panel.
const SHOP: u32 = 0x1ca540;

/// 0x116248 ticker text set (text, box, message).
const TICKER_TEXT_SET: u32 = 0x116248;
/// 0x1166cc scroll limit of the strip (scroll counter argument).
const PANEL_LEAD: u32 = 0x1166cc;
/// 0x1f4670 fill of a screen rectangle (seven arguments).
const FILL_RECT: u32 = 0x1f4670;
/// 0x1f6b80 filler message id from a table entry.
const FILLER_TEXT: u32 = 0x1f6b80;
/// 0x28e2e8 panel scroll draw (text, -scroll, 8; f12 = 2.0).
const PANEL_SCROLL: u32 = 0x28e2e8;
/// 0x2427d0 overlay draw (argument, 1).
const OVERLAY_DRAW: u32 = 0x2427d0;

/// Scroll factor 2.0 as a single (f12 of `PANEL_SCROLL`).
const TWO: F = F(0x4000_0000);

/// Sign-extends a 32-bit value into a register the way lw/addu do.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x28b890 shop_ticker_set(msg) -> void: writes `msg` into the strip box and restarts the scroll counter.
pub fn shop_ticker_set(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let msg = c.arg(0);
    h::frame(c, 0x20, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let text = c.r32(TEXT_PTR);
        h::call_keep(c, TICKER_TEXT_SET, &[text, TICKER_BOX, msg], &[]);
    });
    c.w32(SCROLL, 0);
    true
}

/// 0x28e4f8 shop_ticker_panel() -> void: per frame, clears the strip band, scrolls the strip by two pixels, or when the
/// scroll ran out picks a random filler message, then draws the panel and blanks both edges.
pub fn shop_ticker_panel(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x30, &[(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)], |c, _sp| {
        h::call_keep(c, FILL_RECT, &[0, 0, 0x200, 0x80, 0x200, 0x80, 0], &[]);
        let text = c.r32(TEXT_PTR);
        let (lead, _) = h::call_keep(c, PANEL_LEAD, &[text], &[]);
        let count = c.r32(SCROLL) as i32;
        let step = count / SCROLL_DIV;
        let left = (lead as i32).wrapping_sub(step);
        c.s(3, sx(step as u32));
        c.s(4, sx(count as u32));
        if left > 0 {
            c.w32(SCROLL, count.wrapping_add(SCROLL_STEP) as u32);
        } else {
            let (idx, _) = h::run(c, rand::rand_below, &[FILLER_COUNT], &[]);
            let entry = c.r32(FILLER_TABLE + idx * 4);
            let (msg, _) = h::call_keep(c, FILLER_TEXT, &[entry], &[]);
            h::run_keep(c, shop_ticker_set, &[msg], &[]);
        }
        c.s(2, sx(SHOP));
        let text = c.r32(TEXT_PTR);
        let neg = c.r32(SCROLL).wrapping_neg();
        h::call_keep(c, PANEL_SCROLL, &[text, neg, 8], &[TWO]);
        h::call_keep(c, FILL_RECT, &[0, 0, 4, 0x40, 0x200, 0x80, 0], &[]);
        h::call_keep(c, FILL_RECT, &[0xe2, 0, 0xe6, 0x40, 0x200, 0x80, 0], &[]);
    });
    true
}

/// 0x28e608 shop_ticker_overlay() -> void: clears the strip band and, in overlay mode, draws the overlay.
pub fn shop_ticker_overlay(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        h::call_keep(c, FILL_RECT, &[0, 0, 0x200, 0x80, 0x200, 0x80, 0], &[]);
        let mode = c.r32(MODE_WORD);
        c.s(2, 1);
        c.s(3, sx(mode));
        if mode == MODE_OVERLAY {
            c.s(2, sx(SHOP_HI));
            let arg = c.r32(OVERLAY_ARG);
            h::call_keep(c, OVERLAY_DRAW, &[arg, 1], &[]);
        }
    });
    true
}
