//! Tests of `gpu_repl`: record word 1 and table texel packing, the shelf packer, the id book (stable ids, nearly-full reset), the
//! bind group layout and gpu.wgsl bindings 9 / 10, a naga parse + validation of gpu.wgsl, and (when a Vulkan adapter exists) an upload.

use super::*;

fn name(n: u64) -> TexName {
    TexName::palette(n, n + 1, 0x1dd3)
}

#[test]
fn word_one_packs_id_and_origin() {
    for (id, x, y) in [(1, 0, 0), (4095, 1023, 1023), (7, 512, 3), (300, 17, 960)] {
        let w = pack_word(id, x, y);
        assert_eq!(unpack_word(w), (id, x, y));
        assert_ne!(w, 0, "an image is never word 0");
    }
    assert_eq!(pack_word(1, 0, 0), 1);
    assert_eq!(pack_word(0, 1, 0), 1 << 12);
    assert_eq!(pack_word(0, 0, 1), 1 << 22);
    assert_eq!(unpack_word(0), (0, 0, 0), "word 0 = the original texture");
}

#[test]
fn table_texel_layout() {
    assert_eq!(table_texel(8, 9, 100, 200, 25, 50), [8 | 9 << 16, 100 | 200 << 16, 25 | 50 << 16, 0]);
}

#[test]
fn atlas_size_from_mb() {
    assert_eq!(atlas_rows(None), 8192, "256 MB = 8192 rows of 8192 texels");
    assert_eq!(atlas_rows(Some(64)), 2048);
    assert_eq!(atlas_rows(Some(0)), 32, "at least 1 MB");
    assert_eq!(atlas_rows(Some(4096)), 8192, "never past the texture limit");
}

/// Every box inside the area, none overlapping another (gutter included in the box).
fn assert_disjoint(placed: &[(u32, u32, u32, u32)], w: u32, h: u32) {
    for (i, a) in placed.iter().enumerate() {
        assert!(a.0 + a.2 <= w && a.1 + a.3 <= h, "box {a:?} leaves {w}x{h}");
        for b in &placed[i + 1..] {
            let apart = a.0 + a.2 + GUTTER <= b.0 || b.0 + b.2 + GUTTER <= a.0 || a.1 + a.3 + GUTTER <= b.1 || b.1 + b.3 + GUTTER <= a.1;
            assert!(apart, "{a:?} and {b:?} touch or overlap");
        }
    }
}

#[test]
fn shelves_place_without_overlap_and_reuse_rows() {
    let mut s = Shelves::new(256, 256);
    let sizes = [(68, 68), (68, 68), (68, 68), (100, 30), (100, 29), (120, 60), (16, 16), (200, 90), (30, 3)];
    let mut placed = Vec::new();
    for &(w, h) in &sizes {
        let (x, y) = s.alloc(w, h).unwrap_or_else(|| panic!("{w}x{h} must fit"));
        placed.push((x, y, w, h));
    }
    assert_disjoint(&placed, 256, 256);
    assert_eq!(placed[0], (0, 0, 68, 68));
    assert_eq!(placed[1].1, 0, "the second 68x68 sits on the first shelf");
    assert_eq!(placed[1].0, 69, "after the first image and its gutter");
    assert_eq!(placed[4].1, placed[3].1, "a 29-high image fits the 31-high shelf of the 30-high one");
}

#[test]
fn shelves_refuse_what_does_not_fit() {
    let mut s = Shelves::new(64, 64);
    assert_eq!(s.alloc(64, 10), None, "the gutter does not fit the width");
    assert_eq!(s.alloc(0, 5), None);
    assert!(s.alloc(63, 40).is_some());
    assert_eq!(s.alloc(10, 30), None, "no row left");
    assert!(s.alloc(10, 20).is_some(), "but a smaller one still fits");
    assert_eq!(s.alloc(u32::MAX, 4), None);
    assert_eq!(s.alloc(4, u32::MAX), None);
}

#[test]
fn a_shelf_is_not_wasted_on_a_tiny_image() {
    let mut s = Shelves::new(512, 512);
    s.alloc(100, 100).unwrap();
    let (_, y) = s.alloc(20, 20).unwrap();
    assert_eq!(y, 101, "a 20-high image does not sit in a 101-high shelf: it opens its own");
}

#[test]
fn ids_are_stable_and_start_at_one() {
    let mut b = Book::new(1024);
    let a = b.place(name(1), 64, 64).unwrap();
    let c = b.place(name(2), 64, 64).unwrap();
    assert_eq!((a.id, c.id), (1, 2));
    assert_eq!(b.id_of(&name(1)), Some(1));
    assert_eq!(b.id_of(&name(3)), None);
    assert_eq!(b.len(), 2);
}

#[test]
fn full_book_refuses_and_keeps_what_it_has() {
    let mut b = Book::new(100);
    assert!(b.place(name(1), 8000, 90).is_ok());
    assert_eq!(b.place(name(2), 8000, 90), Err(Refuse::Full), "the second row would pass 100 rows");
    assert_eq!(b.id_of(&name(2)), None, "a refused image gets no id");
    assert_eq!(b.id_of(&name(1)), Some(1));
}

#[test]
fn ids_run_out_before_the_12_bits_do() {
    let mut b = Book::new(8192);
    let mut last = 0;
    for n in 0..MAX_IDS as u64 {
        match b.place(name(n), 1, 1) {
            Ok(p) => last = p.id,
            Err(e) => {
                assert_eq!(e, Refuse::Full);
                break;
            }
        }
    }
    assert_eq!(last, MAX_IDS - 1, "ids 1..=4095");
    assert_eq!(b.place(name(99999), 1, 1), Err(Refuse::Full));
}

#[test]
fn a_full_book_evicts_the_least_recently_used_shelf_and_places_again() {
    // 100 rows: three shelves of 31 rows (30 + gutter). Names 1, 2, 3 were last drawn at frames 5, 1, 9 (name 4 is new).
    let mut b = Book::new(100);
    for n in 1..=3 {
        b.place(name(n), 8000, 30).unwrap();
    }
    assert_eq!(b.place(name(4), 8000, 30), Err(Refuse::Full));
    for (n, f) in [(1, 5), (2, 1), (3, 9)] {
        b.touch_name(&name(n), f);
    }
    let need = [(8000, 30)];
    assert_eq!(b.plan(&need, 1), None, "every shelf was drawn at frame 1 or later: nothing may go");
    assert_eq!(b.plan(&need, 2), Some(vec![31]), "only the shelf of name 2 (frame 1) is free to go");
    let gone = b.evict(&need, 6);
    assert_eq!(gone, vec![name(2)], "only the oldest shelf goes, and only as much as the refused image needs");
    assert_eq!((b.id_of(&name(1)), b.id_of(&name(2)), b.id_of(&name(3))), (Some(1), None, Some(3)));
    let p = b.place(name(4), 8000, 30).unwrap();
    assert_eq!((p.id, p.y), (2, 31), "the freed shelf and the freed id are used again");
    assert_eq!(b.evict(&[], 6), Vec::<TexName>::new(), "nothing is evicted when there is room");
}

#[test]
fn freed_shelves_merge_are_cut_to_the_image_and_give_back_the_end_rows() {
    let mut b = Book::new(300);
    for n in 1..=3 {
        b.place(name(n), 5000, 90).unwrap(); // too wide to share: three shelves of 91 rows at y = 0, 91, 182
    }
    // Names 1 and 2 are old, 3 is newer: the first two shelves go (they merge into one 182-row free shelf), the end shelf stays.
    b.touch_name(&name(3), 1);
    let gone = b.evict(&[(8000, 150)], u64::MAX);
    assert_eq!(gone.len(), 2);
    assert!(gone.contains(&name(1)) && gone.contains(&name(2)));
    let p = b.place(name(9), 100, 30).unwrap();
    assert_eq!((p.x, p.y), (0, 0), "a free shelf more than twice as tall is cut down to the image");
    let q = b.place(name(10), 100, 40).unwrap();
    assert_eq!((q.x, q.y), (0, 31), "the rest of it is a new free shelf, cut again");
    // Everything gone: the rows of the end shelf are given back, so an image as tall as the atlas fits again.
    let mut c = Book::new(300);
    for n in 1..=3 {
        c.place(name(n), 5000, 90).unwrap();
    }
    assert_eq!(c.evict(&[(100, 297)], u64::MAX).len(), 3, "room for the whole atlas needs every shelf gone");
    assert_eq!(c.len(), 0);
    assert_eq!(c.place(name(20), 100, 298).map(|p| p.y), Ok(0));
    // Images wider than the atlas (or taller than all rows) never fit: no eviction would help.
    assert_eq!(b.place(name(11), ATLAS_W, 10), Err(Refuse::Never));
    assert_eq!(b.place(name(12), 10, 300), Err(Refuse::Never));
}

#[test]
fn an_evicted_id_goes_back_to_the_pool_when_the_ids_ran_out() {
    let mut b = Book::new(8192);
    for n in 1..MAX_IDS as u64 {
        b.place(name(n), 1, 1).unwrap();
    }
    assert_eq!(b.place(name(99999), 1, 1), Err(Refuse::Full));
    // The images sit 4096 to a shelf of 2 rows, so the first shelf holds all of them.
    for n in 2..MAX_IDS as u64 {
        b.touch_name(&name(n), 1);
    }
    let gone = b.evict(&[(1, 1)], u64::MAX);
    assert_eq!(gone.len(), MAX_IDS as usize - 1, "no id was spare, so a shelf is freed even though the rows suffice: all 4095 1 x 1 images share one shelf");
    assert!(gone.contains(&name(1)));
    assert!(b.place(name(99999), 1, 1).is_ok());
}

fn wgsl() -> naga::Module {
    let src = include_str!("gpu.wgsl");
    naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("gpu.wgsl: {}", e.emit_to_string(src)))
}

#[test]
fn gpu_wgsl_parses_and_validates() {
    let m = wgsl();
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("gpu.wgsl validates");
}

#[test]
fn wgsl_bindings_match_the_layout() {
    let m = wgsl();
    let layout = layout_entries();
    let mut seen = 0;
    for (_, v) in m.global_variables.iter() {
        let Some(b) = &v.binding else { continue };
        assert_eq!(b.group, 0);
        let e = layout.iter().find(|e| e.binding == b.binding).unwrap_or_else(|| panic!("binding {} has no layout entry", b.binding));
        let class_texture = matches!(m.types[v.ty].inner, naga::TypeInner::Image { .. });
        assert_eq!(class_texture, matches!(e.ty, wgpu::BindingType::Texture { .. }), "binding {} kind", b.binding);
        if let naga::TypeInner::Image { class: naga::ImageClass::Sampled { kind, .. }, .. } = m.types[v.ty].inner {
            let want = match e.ty {
                wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, .. } => naga::ScalarKind::Uint,
                _ => naga::ScalarKind::Float,
            };
            assert_eq!(kind, want, "binding {} sample type", b.binding);
        }
        seen += 1;
    }
    assert_eq!(seen, layout.len(), "every layout entry is declared in gpu.wgsl");
    assert_eq!(layout.iter().map(|e| e.binding).collect::<Vec<_>>(), (0..11).collect::<Vec<_>>());
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = wgpu::Backends::VULKAN;
    let inst = wgpu::Instance::new(desc);
    let ad = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::LowPower, compatible_surface: None, force_fallback_adapter: false, apply_limit_buckets: false })).ok()?;
    pollster::block_on(ad.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

#[test]
fn upload_smoke_when_a_vulkan_device_exists() {
    let Some((dev, q)) = device() else { return };
    let mut r = ReplGpu::new(&dev);
    assert!(!r.active() && r.images() == 0 && r.id_of(&name(1)).is_none());
    let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &layout_entries()[9..] });
    let (a, t) = r.views();
    let _ = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(a) }, wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(t) }],
    });
    let n = TexName::palette(1, 2, 0x1dd3);
    let img = HudImg { w: 4, h: 2, rgba: vec![255; 32] };
    assert_eq!(r.add(&dev, &q, n, &img, 4, 2), Ok((1, true)));
    assert!(r.active());
    assert_eq!(r.add(&dev, &q, n, &img, 4, 2), Ok((1, false)), "the same name gives the same id and uploads nothing");
    let n2 = TexName::palette(3, 4, 0x1dd3);
    assert_eq!(r.add(&dev, &q, n2, &img, 4, 2), Ok((2, false)));
    assert_eq!(r.add(&dev, &q, n2, &HudImg { w: 4, h: 2, rgba: vec![0; 7] }, 4, 2), Err(Refuse::Bad), "a wrong pixel count is refused first");
    assert_eq!(r.add(&dev, &q, TexName::palette(5, 6, 0), &HudImg { w: 4, h: 2, rgba: vec![0; 7] }, 4, 2), Err(Refuse::Bad));
    assert_eq!(r.images(), 2);
    assert_eq!(r.plan_evict::<()>(&[], u64::MAX), (Vec::new(), Vec::new()), "no eviction while there is room");
    assert_eq!(r.images(), 2);
    q.submit([]);
}

#[test]
fn free_texels_are_not_room_a_fitting_shelf_is_freed_for_a_tall_image() {
    // 512 rows. One 8000 x 299 image takes a shelf of 300 rows; 212 rows (1.7 M texels) stay unopened: a 400 x 400 image (160 K texels)
    // is refused although the free AREA is ten times what it needs (the old test `free_area >= need` never evicted anything here).
    let mut b = Book::new(512);
    b.place(name(1), 8000, 299).unwrap();
    assert_eq!(b.place(name(2), 400, 400), Err(Refuse::Full));
    b.touch_name(&name(1), 3);
    assert_eq!(b.plan(&[(400, 400)], 4), Some(vec![0]), "the shelf of name 1 goes (last drawn at frame 3, the ended field is 4)");
    assert_eq!(b.plan(&[(400, 400)], 3), None, "drawn in the field that just ended: kept, nothing freed, the image stays refused");
    assert_eq!(b.evict(&[(400, 400)], 4), vec![name(1)]);
    assert!(b.place(name(2), 400, 400).is_ok(), "placed after the eviction");
    // Two refused images that cannot both fit: the tallest is given up, the other one is made room for.
    let mut c = Book::new(512);
    c.place(name(1), 8000, 299).unwrap();
    c.touch_name(&name(1), 3);
    assert_eq!(c.plan(&[(8000, 500), (400, 400)], 4), Some(vec![0]));
    // Gutter: an image exactly as tall as the atlas (511 + 1) fits an empty atlas, one row more never does.
    assert_eq!(c.plan(&[(8000, 511)], 4), Some(vec![0]));
    assert_eq!(c.plan(&[(8000, 512)], 4), None);
}

#[test]
fn fragmented_free_shelves_need_a_tall_enough_run_not_just_area() {
    // Three shelves of 100 rows at y = 0, 100, 200 in a 310-row atlas (10 end rows); the middle one is old, the others younger.
    let mut b = Book::new(310);
    for n in 1..=3 {
        b.place(name(n), 5000, 99).unwrap();
    }
    for (n, f) in [(1, 9), (2, 1), (3, 9)] {
        b.touch_name(&name(n), f);
    }
    assert_eq!(b.plan(&[(5000, 99)], 5), Some(vec![100]), "the old shelf alone takes a 99-row image (100 with the gutter)");
    assert_eq!(b.plan(&[(5000, 140)], 5), None, "the two young shelves are protected: 141 rows cannot be made, nothing is freed");
    assert_eq!(b.plan(&[(5000, 140)], 10), Some(vec![100, 0]), "unprotected: the old shelf, then its neighbour (they merge into 200 rows)");
}

#[test]
fn a_screen_change_in_an_atlas_that_holds_one_screen_shows_the_new_art_after_the_eviction() {
    // The glue's loop (`repl_resolve` -> need -> `repl_frame_end` -> placed again) on the bookkeeping: an atlas of 1100 rows holds one
    // screen (three 8000 x 330 images, one shelf each); screen B has three different ones of the same shape.
    let mut b = Book::new(1100);
    let shown = |b: &mut Book, frame: u64, screen: &[u64], needs: &mut Vec<(u32, u32)>| -> usize {
        let mut n = 0;
        for &k in screen {
            let nm = name(k);
            if b.id_of(&nm).is_some() || b.place(nm, 8000, 330).is_ok() {
                b.touch_name(&nm, frame);
                n += 1;
            } else {
                needs.push((8000, 330));
            }
        }
        n
    };
    let (a, c) = ([1u64, 2, 3], [11u64, 12, 13]);
    let mut needs = Vec::new();
    assert_eq!(shown(&mut b, 1, &a, &mut needs), 3, "screen A fits");
    assert_eq!(shown(&mut b, 2, &c, &mut needs), 0, "screen B finds no room: 3 x 331 rows are taken");
    // End of field 2: everything drawn in field 2 is protected (keep_from = frame - 1 = 2 after the frame counter moved to 3 is 2).
    // Screen A was last drawn in field 1, screen B's refusals never drew: the field-2 end (frame counter 3) frees A.
    let rows = b.plan(&needs, 2).expect("a plan");
    assert_eq!(rows.len(), 3, "three shelves of 331 rows are the least recently used");
    let gone = b.free_rows(&rows);
    assert_eq!(gone.len(), 3);
    let mut needs2 = Vec::new();
    assert_eq!(shown(&mut b, 3, &c, &mut needs2), 3, "screen B is placed after the eviction");
    assert!(needs2.is_empty());
    // Back to A: B was drawn in field 3, so at the end of field 4 (keep_from 4) it may go, at the end of field 3 (keep_from 3) it may not.
    let mut needs3 = Vec::new();
    assert_eq!(shown(&mut b, 4, &a, &mut needs3), 0);
    assert_eq!(b.plan(&needs3, 3), None, "B was drawn in field 3: protected at the end of field 3 (a thrash guard)");
    assert!(b.plan(&needs3, 4).is_some(), "a field later B is old");
}
