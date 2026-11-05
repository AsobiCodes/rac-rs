//! The overlay a `Gs` publishes carries the guard measured on THAT picture, not the one before it (the HUD position's top corner cells,
//! viewer/handler/hud_place.rs, are laid out with it): the CPU `publish`, the CPU `field_end`, and `take_finished`, the step the GPU
//! capture's `publish` takes the finished picture with (the GPU itself cannot run in a unit test).

use super::super::edge_cut::{self, guard, Guard, TEST_SERIAL};
use super::super::{H, W};
use super::*;

const SC: [i32; 4] = [0, 511, 0, 447];

/// The vendor prompt's bar: three sprites (caps and a stretched middle), x 107..405, 32 rows from `y`.
fn note_prompt_bar(y: f32) {
    for v in [[(107.0, y), (139.0, y + 32.0)], [(139.0, y), (373.0, y + 32.0)], [(373.0, y), (405.0, y + 32.0)]] {
        guard::note(SC, &v);
    }
}

fn cpu_gs() -> Gs {
    let mut g = Gs::new(vec![0; 4 << 20]);
    g.gpu = None;
    g
}

fn first_row_of(o: &Overlay) -> f32 {
    o.guard.centre.expect("a centred widget") * o.rows as f32
}

/// A prompt shows in one picture only: that picture carries its guard, the next (empty) one carries none, and the one after the bar
/// came back carries it again (nothing left over from the picture before, nothing early).
#[test]
fn a_cpu_publish_carries_the_guard_of_its_own_picture() {
    let _one = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut g = cpu_gs();
    edge_cut::publish(448);
    g.publish();
    assert_eq!(g.done.take().unwrap().guard, Guard::default(), "no prompt in the first picture");
    note_prompt_bar(41.5);
    g.publish();
    let with = g.done.take().unwrap();
    assert!((first_row_of(&with) - 43.4).abs() < 1e-3, "the picture with the prompt has its guard: {:?}", with.guard);
    g.publish();
    assert_eq!(g.done.take().unwrap().guard, Guard::default(), "the next picture is empty again");
    note_prompt_bar(53.5);
    g.publish();
    assert!((first_row_of(&g.done.take().unwrap()) - 55.4).abs() < 1e-3, "the USA prompt row");
}

/// The display-buffer picture (`field_end`) is measured the same way.
#[test]
fn a_cpu_field_end_carries_the_guard_of_its_own_picture() {
    let _one = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut g = cpu_gs();
    let blank = || Some(vec![[0.0, 0.0, 0.0, 1.0]; (W * H) as usize]);
    g.publish(); // sets the row count the display pictures use (`held_rows`)
    g.done = None;
    g.dcur = blank();
    g.field_end();
    assert_eq!(g.done.take().unwrap().guard, Guard::default());
    note_prompt_bar(41.5);
    g.dcur = blank();
    g.field_end();
    let with = g.done.take().unwrap();
    assert!(with.disp && (first_row_of(&with) - 43.4).abs() < 1e-3, "{:?}", with.guard);
    g.dcur = blank();
    g.field_end();
    assert_eq!(g.done.take().unwrap().guard, Guard::default());
}

/// The GPU capture's publish: the picture taken out of `cur` has the new guard and its rows; the blank that takes `cur`'s place (the next
/// picture's overlay) has none, or the viewer would draw every picture with the guard of the one before (the first prompt picture torn).
#[test]
fn the_finished_gpu_picture_gets_the_new_guard_and_the_blank_none() {
    let prompt = Guard { centre: Some(43.4 / 448.0), ..Default::default() };
    let before = Guard { centre: Some(0.2), ..Default::default() };
    let mut cur = Overlay { w: 512, h: 448, rows: 448, frozen: true, disp: false, px: vec![[1.0; 4]; 3], rgba: Vec::new(), hd: None, guard: Guard::default() };
    let o = take_finished(&mut cur, 400, prompt);
    assert_eq!((o.guard, o.rows, o.px.len(), o.frozen), (prompt, 400, 3, true));
    assert_eq!((cur.guard, cur.rows, cur.px.len(), cur.frozen), (Guard::default(), 448, 0, false));
    // Whatever the old overlay carried is replaced, not kept.
    cur.guard = before;
    assert_eq!(take_finished(&mut cur, 448, Guard::default()).guard, Guard::default());
}
