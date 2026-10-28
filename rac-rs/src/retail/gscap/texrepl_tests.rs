use super::*;
use std::collections::BTreeSet;

fn blocks(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32) -> BTreeSet<u32> {
    let mut s = BTreeSet::new();
    for_blocks(psm, bp, bw, x, y, w, h, |b| {
        s.insert(b);
    });
    s
}

#[test]
fn for_blocks_counts_the_blocks_of_a_rectangle_per_format() {
    // CT32: 8 x 8 pixel blocks, so a 32 x 32 rectangle at the origin of a buffer is 16 blocks, a 1 x 1 one is 1.
    assert_eq!(blocks(0x00, 0, 1, 0, 0, 32, 32).len(), 16);
    assert_eq!(blocks(0x00, 0, 1, 0, 0, 1, 1).len(), 1);
    // A rectangle that starts inside a block and ends inside another covers both (and everything between).
    assert_eq!(blocks(0x00, 0, 1, 4, 4, 8, 8).len(), 4);
    // CT16: 16 x 8 pixel blocks; T8: 16 x 16; T4: 32 x 16.
    assert_eq!(blocks(0x02, 0, 1, 0, 0, 32, 16).len(), 4);
    assert_eq!(blocks(0x13, 0, 1, 0, 0, 32, 32).len(), 4);
    assert_eq!(blocks(0x14, 0, 1, 0, 0, 64, 32).len(), 4);
    // Formats the walk does not know give no block; an empty rectangle neither.
    assert!(blocks(0x30, 0, 1, 0, 0, 8, 8).is_empty());
    assert!(blocks(0x00, 0, 1, 0, 0, 0, 8).is_empty());
    assert!(blocks(0x00, 0, 1, 0, 0, 8, 0).is_empty());
}

#[test]
fn for_blocks_follows_the_base_pointer() {
    // One base-pointer unit is one 256-byte block.
    let a = blocks(0x00, 0, 1, 0, 0, 8, 8);
    let b = blocks(0x00, 32, 1, 0, 0, 8, 8);
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_eq!(b.iter().next().unwrap() - a.iter().next().unwrap(), 32);
}

#[test]
fn touch_versions_and_dirty_bits() {
    let mut s = ReplState::default();
    s.ensure();
    assert_eq!(s.max_ver((0, 100)), 0);
    s.touch((10, 20), true);
    assert_eq!(s.stamp, 1);
    assert_eq!(s.max_ver((10, 20)), 1);
    assert_eq!(s.max_ver((0, 10)), 0, "blocks outside the range keep their version");
    assert!(s.any_dirty((0, 100)));
    assert!(!s.any_dirty((0, 10)));
    assert!(!s.any_dirty((20, 100)));
    s.touch((15, 16), false);
    assert_eq!(s.max_ver((10, 20)), 2);
    assert!(s.any_dirty((10, 15)));
    assert!(!s.any_dirty((15, 16)), "an upload makes a block clean again");
    // Ranges past the end are cut, empty ones do nothing.
    s.touch((BLOCKS as u32 - 1, BLOCKS as u32 + 50), true);
    assert!(s.any_dirty((BLOCKS as u32 - 1, BLOCKS as u32)));
    let st = s.stamp;
    s.touch((5, 5), true);
    assert_eq!(s.max_ver((0, 10)), 0);
    assert_eq!(s.stamp, st + 1);
    assert_eq!(s.len(), 0);
}

#[test]
fn destination_registers_decode() {
    let bb = 0x14u64 << 56 | 8u64 << 48 | 0x1234u64 << 32;
    let pos = 7u64 << 48 | 5u64 << 32;
    let rr = 32u64 << 32 | 64;
    assert_eq!(dest(bb, pos, rr), (0x1234, 8, 0x14, 5, 7, 64, 32));
}

#[test]
fn record_word_round_trip() {
    for (id, x, y) in [(1, 0, 0), (4095, 1023, 1023), (77, 16, 8)] {
        let w = pack_word(id, x, y);
        assert_eq!((w & 0xfff, (w >> 12) & 0x3ff, w >> 22), (id, x, y));
        assert_ne!(w, 0, "an id of 1 or more is never the original-texture word");
    }
    assert_eq!(MAX_IDS, 4096);
}

#[test]
fn versions_keep_counting_past_u32() {
    let mut s = ReplState::default();
    s.ensure();
    s.stamp = u64::from(u32::MAX) - 1;
    s.touch((0, 1), true);
    s.touch((1, 2), true);
    s.touch((2, 3), true);
    assert_eq!(s.max_ver((2, 3)), u64::from(u32::MAX) + 2, "a 32-bit stamp would have wrapped to 0 here");
    assert!(s.max_ver((2, 3)) > s.max_ver((0, 1)), "a later change stays newer than the version a name was built at");
}
