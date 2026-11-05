use super::*;

fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            v.extend(f(x, y));
        }
    }
    v
}

#[test]
fn box_table_of_the_same_size_keeps_the_pixels_with_alpha_on_the_gs_scale() {
    let i = img(4, 2, |x, y| [x as u8 * 10, y as u8 * 20, 7, if x == 3 { 0 } else { 255 }]);
    let t = box_table(&i, 4, 2, 4, 2);
    assert_eq!(t.len(), 8);
    assert_eq!(t[0], [0, 0, 7, 128]);
    assert_eq!(t[2], [20, 0, 7, 128]);
    assert_eq!(t[4 + 1], [10, 20, 7, 128]);
    // Alpha 0 everywhere in the texel: the plain colour stays, alpha 0.
    assert_eq!(t[3], [30, 0, 7, 0]);
}

#[test]
fn box_table_averages_the_pixels_a_texel_covers() {
    // 4 x 4 image, 2 x 2 table: each texel covers a 2 x 2 block. Left column of the image red, right column blue (all opaque).
    let i = img(4, 4, |x, _| if x < 2 { [200, 0, 0, 255] } else { [0, 0, 100, 255] });
    let t = box_table(&i, 4, 4, 2, 2);
    assert_eq!(t, vec![[200, 0, 0, 128], [0, 0, 100, 128], [200, 0, 0, 128], [0, 0, 100, 128]]);
    // One block half transparent: colour is weighted by alpha (only the red half counts), alpha is the mean on the 128 scale.
    let i = img(2, 2, |x, _| if x == 0 { [255, 0, 0, 255] } else { [0, 255, 0, 0] });
    let t = box_table(&i, 2, 2, 1, 1);
    assert_eq!(t, vec![[255, 0, 0, 64]]);
}

#[test]
fn box_table_of_a_bigger_image_samples_inside_the_image() {
    // 8 x 8 -> 3 x 3 (a non-integer ratio): every sample lands inside the image, so a flat image gives a flat table.
    let i = img(8, 8, |_, _| [10, 20, 30, 255]);
    let t = box_table(&i, 8, 8, 3, 3);
    assert_eq!(t.len(), 9);
    assert!(t.iter().all(|p| *p == [10, 20, 30, 128]));
    // Enlarging (a 2 x 2 image for a 4 x 4 rectangle) repeats the pixels.
    let i = img(2, 2, |x, y| [(x * 100) as u8, (y * 100) as u8, 0, 255]);
    let t = box_table(&i, 2, 2, 4, 4);
    assert_eq!(t[0], [0, 0, 0, 128]);
    assert_eq!(t[3], [100, 0, 0, 128]);
    assert_eq!(t[12], [0, 100, 0, 128]);
    assert_eq!(t[15], [100, 100, 0, 128]);
}

#[test]
fn rp_view_reads_inside_the_rectangle_and_clamps_outside() {
    let tab = Arc::new(CpuRepl { w: 2, h: 2, serial: 1, px: vec![[1, 2, 3, 128], [4, 5, 6, 128], [7, 8, 9, 0], [10, 11, 12, 64]] });
    let sel = CpuSel { tab: tab.clone(), x: 16, y: 8 };
    let v = RpView::new(Some(&sel));
    assert!(v.on());
    assert_eq!(v.texel(16, 8), [1.0, 2.0, 3.0, 128.0]);
    assert_eq!(v.texel(17, 8), [4.0, 5.0, 6.0, 128.0]);
    assert_eq!(v.texel(16, 9), [7.0, 8.0, 9.0, 0.0]);
    assert_eq!(v.texel(17, 9), [10.0, 11.0, 12.0, 64.0]);
    // Outside the rectangle (a bilinear tap past the edge, a texel left of it): the nearest edge texel.
    assert_eq!(v.texel(0, 0), [1.0, 2.0, 3.0, 128.0]);
    assert_eq!(v.texel(900, 900), [10.0, 11.0, 12.0, 64.0]);
    assert_eq!(v.texel(17, 0), [4.0, 5.0, 6.0, 128.0]);
}

#[test]
fn rp_view_is_off_without_a_table() {
    assert!(!RpView::NONE.on());
    assert!(!RpView::new(None).on());
    let empty = CpuSel { tab: Arc::new(CpuRepl { w: 0, h: 0, serial: 2, px: Vec::new() }), x: 0, y: 0 };
    assert!(!RpView::new(Some(&empty)).on());
}
