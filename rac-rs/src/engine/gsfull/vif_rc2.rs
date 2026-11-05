//! R&C2's VU1 program ids (`vif::identify` for an R&C2 disc): retail programs 0..12 (rac2 decomp/vu1/progN.bin).
//!
//! The first 16 words only say a chunk starts like program `first` (many share the matrix loads; programs 3 and 4 start
//! alike), so the whole first chunk must match one of `FULL`. R&C1's program 11 is R&C2's program 12. Ids only tag the
//! output (`Out::segs`, `Stats`); VU1 runs the microcode by content.

/// (FNV-1a of the first 16 words of a program's first chunk, program id); the first entry wins.
const PROGS: [(u64, u8); 13] = [
    (0xac94c7d56d963410, 0),
    (0x64b9278f738ff6bb, 1),
    (0x01c3ac6fa1c5eced, 2),
    (0xefdae0d82adfbcd6, 3),
    (0xefdae0d82adfbcd6, 4),
    (0x4e329bc8a744a9c9, 5),
    (0xd28121151fdf0150, 6),
    (0x34e596a3057c1fcc, 7),
    (0x672d80120ff43f30, 8),
    (0x5fd86134df75989d, 9),
    (0xcfcb763ca14d3b55, 10),
    (0xe3e3e7a06238f39b, 11),
    (0x68a07c118d24d2ca, 12),
];

/// (words, FNV-1a of the whole first chunk) of each program (its chunk at VU address 0).
const FULL: [(u32, u64); 13] = [
    (512, 0x1a7556433b787316),
    (402, 0xc40d025d357541ea),
    (444, 0x0cda4e64b3822880),
    (512, 0x00b14575051bd5d6),
    (512, 0x91a6de33d4f8f0a8),
    (512, 0x803c02c025a1675d),
    (512, 0x6edcb086f115875f),
    (512, 0x3a0176026f6d1de3),
    (512, 0xbd1880ccc7a902ac),
    (512, 0xc2bd5de66f16715d),
    (512, 0xebf0416d29b0d0b3),
    (512, 0xb8c2fc750d9f4798),
    (512, 0x34aa1f609bffb056),
];

fn fnv(w: &[u32]) -> u64 {
    w.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Program id of a chunk uploaded at address 0 (15 = unknown).
pub fn identify(w: &[u32]) -> u8 {
    let h = fnv(&w[..w.len().min(16)]);
    if !PROGS.iter().any(|p| p.0 == h) {
        return 15;
    }
    let full = (w.len() as u32, fnv(w));
    FULL.iter().position(|&f| f == full).map_or(15, |i| i as u8)
}
