use super::*;

#[test]
fn init_sequence_is_the_disassembly() {
    // 0x69f8..0x6a7c, in order (SetParam(id, value) / SetCoreAttr(id, value)).
    let want: [(u16, u16, bool); 12] = [
        (0x800, 0xf00, false),
        (0x801, 0xf0c, false),
        (0x980, 0x3fff, false),
        (0xa80, 0x3fff, false),
        (0xb80, 0, false),
        (0xc80, 0, false),
        (0xb81, 0, false),
        (0xc81, 0, false),
        (0xd81, 0x7fff, false),
        (0xe81, 0x7fff, false),
        (0xa, 0, true),
        (2, 0, true),
    ];
    for (w, (id, value, attr)) in INIT.iter().zip(want) {
        let e = if attr { Write::CoreAttr { id, value } } else { Write::Param { id, value } };
        assert_eq!(*w, e);
    }
}

#[test]
fn boot_image() {
    let (m, calls) = CoreMix::boot_init();
    assert_eq!(calls.len(), 14);
    assert_eq!(calls[12], Write::Param { id: 0x981, value: 0x3fff });
    assert_eq!(calls[13], Write::Param { id: 0xa81, value: 0x3fff });
    assert_eq!(m.core[0].mmix, 0xf00);
    assert_eq!(m.core[1].mmix, 0xf0c);
    assert_eq!(m.core[0].mvol, [0x3fff, 0x3fff]);
    assert_eq!(m.core[1].mvol, [0x3fff, 0x3fff]);
    assert_eq!(m.core[0].evol, [0, 0]);
    assert_eq!(m.core[1].evol, [0, 0]);
    assert_eq!(m.core[1].avol, [0x7fff, 0x7fff]);
    assert_eq!(m.core[0].avol, [0, 0]);
    assert!(!m.core[0].effect_enable && !m.core[1].effect_enable);
    assert_eq!(m.out_mode, 0);
}

#[test]
fn master_volume_formula() {
    assert_eq!(master_volume_reg(0x400), 0x3fff);
    assert_eq!(master_volume_reg(0), 0);
    assert_eq!(master_volume_reg(0x200), 0x1fff);
    // clamp as 0x16314-0x1632c: > 0x400 -> 0x400, negative -> 0
    assert_eq!(master_volume_reg(0x1000), 0x3fff);
    assert_eq!(master_volume_reg(-5), 0);
    // equals v * 0x3fff >> 10 for every v
    for v in 0..=0x400 {
        assert_eq!(master_volume_reg(v) as i32, v * 0x3fff >> 10);
    }
}

#[test]
fn voice_mode_sets_mmix_bits() {
    let (mut m, _) = CoreMix::boot_init();
    // the game sends mode 0, param 1 (boot FUN_0022c3e0): core 1 MMIX 0xf0c -> 0xf0f
    let w = m.set_voice_mode(1);
    assert_eq!(w, Write::Param { id: 0x801, value: 0xf0f });
    assert_eq!(m.core[1].mmix, 0xf0f);
    let w = m.set_voice_mode(0);
    assert_eq!(w, Write::Param { id: 0x801, value: 0xf0c });
    assert_eq!(voice_mode_mmix(0xffff, 0), 0xfffc);
}

#[test]
fn reverb_depth_waits_for_the_work_area() {
    let mut d = ReverbDepth::default();
    assert!(d.set(1, 0x1000, 0x1100, true).is_empty());
    assert_eq!((d.left, d.right), (0x1000, 0x1100));
    let ws = d.set(1, 0x2000, 0x2100, false);
    assert_eq!(ws, vec![Write::Param { id: 0xb81, value: 0x2000 }, Write::Param { id: 0xc81, value: 0x2100 }]);
    let (mut m, _) = CoreMix::boot_init();
    for w in ws {
        m.apply(w);
    }
    assert_eq!(m.core[1].evol, [0x2000, 0x2100]);
}
