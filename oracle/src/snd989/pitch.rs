//! Voice pitch: IOP FUN_000160b4 (bend / pitch-mod -> note + fine in 1/128 semitone), FUN_0001600c (tone centre note
//! and fine -> SPU2 PITCH register value) and libsd's sceSdNote2Pitch (libsd 0x38a0, export 13) it calls.
//! Note: decomp/notes/sound_grains.md "Pitch". The centre fine is ADDED (the old rac-rs `Tone::rate` subtracted it).
//! Golden values: scratch_grain/pitch_sim.py (literal replay of the MIPS), embedded in the tests below.

use super::irx::Irx;

/// libsd's two pitch tables: T1[k] = floor(0x8000 * 2^(k/12)) (k 0..12) at libsd 0x5150,
/// T2[k] = floor(0x8000 * 2^(k/1536)) (k 0..128) at libsd 0x5168. Pure math, so `from_formula` needs no disc;
/// `from_libsd` reads the player's own tables (a test proves both are identical).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note2PitchTables {
    pub t1: [u16; 12],
    pub t2: [u16; 128],
}

pub const LIBSD_T1: u32 = 0x5150;
pub const LIBSD_T2: u32 = 0x5168;

impl Note2PitchTables {
    pub fn from_formula() -> Note2PitchTables {
        let mut t1 = [0u16; 12];
        let mut t2 = [0u16; 128];
        for (k, v) in t1.iter_mut().enumerate() {
            *v = (32768.0 * (k as f64 / 12.0).exp2()).floor() as u16;
        }
        for (k, v) in t2.iter_mut().enumerate() {
            *v = (32768.0 * (k as f64 / 1536.0).exp2()).floor() as u16;
        }
        Note2PitchTables { t1, t2 }
    }

    /// From the libsd IRX (module "Sound_Device_Library") of the player's disc.
    pub fn from_libsd(libsd: &Irx) -> Result<Note2PitchTables, String> {
        let mut t = Note2PitchTables { t1: [0; 12], t2: [0; 128] };
        for (k, v) in t.t1.iter_mut().enumerate() {
            *v = libsd.u16_at(LIBSD_T1 + 2 * k as u32).ok_or("libsd pitch table 1 missing")?;
        }
        for (k, v) in t.t2.iter_mut().enumerate() {
            *v = libsd.u16_at(LIBSD_T2 + 2 * k as u32).ok_or("libsd pitch table 2 missing")?;
        }
        Ok(t)
    }
}

/// FUN_000160b4: pitch-bend term plus the pitch modulation, split into (note, fine) with fine in -127..=127
/// (C truncating division by 128, so a negative total gives a negative fine).
/// `bend_down` / `bend_up` = tone bytes 8 / 9 (s8, semitone ranges), `bend` = handler bend sum (h+0x2e, s16),
/// `pitch_mod` = h+0x14 (s16, 1/128 semitone), `base_note` / `base_fine` = voice bytes +0x18 / +0x19 (60 / 0).
pub fn split_note(bend_down: i8, bend_up: i8, bend: i32, pitch_mod: i32, base_note: i32, base_fine: i32) -> (i32, i32) {
    let term = if bend < 0 {
        let mut t = bend_down as i32 * bend * 0x80;
        if t < 0 {
            t += 0x7fff;
        }
        t >> 15
    } else {
        (bend_up as i32 * bend * 0x80) / 0x7fff
    };
    let total = base_note * 0x80 + base_fine + pitch_mod + term;
    let adj = if total < 0 { total + 0x7f } else { total };
    let note = adj >> 7;
    (note, total - note * 0x80)
}

/// libsd sceSdNote2Pitch(centre_note, centre_fine, note, fine), instruction by instruction. `centre_fine` is the
/// sign-extended tone byte; the code masks it to 16 bits before ADDING it to the fine. Returns the low 16 bits.
pub fn note2pitch(tab: &Note2PitchTables, centre: i32, centre_fine: i32, note: i32, fine: i32) -> u16 {
    let mut a3 = (fine as i16 as i32) + (centre_fine & 0xffff); // fine' = fine + centre fine
    let v1 = (if a3 < 0 { a3 + 0x7f } else { a3 }) >> 7; // trunc(fine' / 128)
    let n = (note + v1 - centre) as i16 as i32; // semitones from the centre note
    let hi = ((n as i64 * 0x2aaa_aaab_i64) >> 32) as i32;
    a3 -= v1 << 7; // f = fine' - 128 * v1
    let q = (hi >> 1) - (n >> 31); // trunc(n / 12)
    let mut t0 = q - 2;
    let rem = n - 12 * q;
    let mut r = rem;
    let mut fix = true; // 0x3938: the f < 0 correction is looked at
    if rem < 0 {
        r = rem + 12;
        t0 = q - 3;
    } else if rem == 0 {
        if a3 >= 0 {
            fix = false;
        } else {
            r = 12;
            t0 = q - 3;
        }
    }
    if fix && a3 < 0 {
        r = (r - 1) + v1;
        a3 += (v1 + 1) << 7;
    }
    let prod = (tab.t1[(r as i16 as i32) as usize] as u32).wrapping_mul(tab.t2[a3 as usize] as u32) as i32;
    let p = prod >> 16;
    let t0 = t0 as i16 as i32;
    let p = if t0 < 0 {
        let s = (-t0) as u32;
        (p as u32).wrapping_add(1u32 << ((s - 1) & 31)) >> (s & 31)
    } else {
        p as u32
    };
    (p & 0xffff) as u16
}

/// FUN_0001600c: the tone's centre note byte (negative = native 48 kHz sample, non-negative = a 44.1 kHz sample that
/// is scaled by 44100/48000), centre fine, and the (note, fine) from `split_note` -> PITCH register value.
pub fn tone_pitch(tab: &Note2PitchTables, centre_byte: i8, centre_fine: i8, note: i32, fine: i32) -> u16 {
    let non_negative = centre_byte >= 0;
    let centre = if non_negative { centre_byte } else { centre_byte.wrapping_neg() };
    let p = note2pitch(tab, centre as i32, centre_fine as i32, note, fine) as u32;
    let p = if non_negative { (p * 0xac44) / 48000 } else { p };
    (p & 0xffff) as u16
}

/// A key-on / update pitch as the driver computes it: `split_note` then `tone_pitch`.
#[allow(clippy::too_many_arguments)]
pub fn voice_pitch(
    tab: &Note2PitchTables,
    centre_byte: i8,
    centre_fine: i8,
    bend_down: i8,
    bend_up: i8,
    bend: i16,
    pitch_mod: i16,
    base_note: u8,
    base_fine: u8,
) -> u16 {
    let (note, fine) = split_note(bend_down, bend_up, bend as i32, pitch_mod as i32, base_note as i32, base_fine as i32);
    tone_pitch(tab, centre_byte, centre_fine, note, fine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snd989::testdata;

    // note2pitch: (centre note byte as given to 0x1600c (s8), centre fine (s8), note, fine, pitch)
    const TONE_PITCH_GOLDEN: &[(i8, i8, i16, i16, u16)] = &[
        (60, 0, 60, 0, 3763),
        (60, 0, 48, 127, 1992),
        (60, 0, 30, -3, 664),
        (60, 1, 72, -50, 7361),
        (60, 1, 127, 127, 23893),
        (60, 64, 60, -100, 3702),
        (60, 64, 0, 0, 121),
        (60, 125, 60, -1, 3980),
        (60, 125, 59, -127, 3549),
        (48, 0, 60, 0, 7526),
        (48, 0, 48, 127, 3985),
        (48, 0, 30, -3, 1328),
        (48, 1, 72, -50, 14722),
        (48, 1, 127, 127, 23893),
        (48, 64, 60, -100, 7405),
        (48, 64, 0, 0, 242),
        (48, 125, 60, -1, 7959),
        (48, 125, 59, -127, 7097),
        (72, 0, 60, 0, 1881),
        (72, 0, 48, 127, 995),
        (72, 0, 30, -3, 332),
        (72, 1, 72, -50, 3680),
        (72, 1, 127, 127, 23893),
        (72, 64, 60, -100, 1851),
        (72, 64, 0, 0, 60),
        (72, 125, 60, -1, 1990),
        (72, 125, 59, -127, 1774),
        (36, 0, 60, 0, 15052),
        (36, 0, 48, 127, 7970),
        (36, 0, 30, -3, 2657),
        (36, 1, 72, -50, 29445),
        (36, 1, 127, 127, 23893),
        (36, 64, 60, -100, 14810),
        (36, 64, 0, 0, 484),
        (36, 125, 60, -1, 15918),
        (36, 125, 59, -127, 14194),
        (0, 0, 60, 0, 15052),
        (0, 0, 48, 127, 15940),
        (0, 0, 30, -3, 21258),
        (0, 1, 72, -50, 29445),
        (0, 1, 127, 127, 23893),
        (0, 64, 60, -100, 29619),
        (0, 64, 0, 0, 3873),
        (0, 125, 60, -1, 15918),
        (0, 125, 59, -127, 28389),
        (12, 0, 60, 0, 15052),
        (12, 0, 48, 127, 15940),
        (12, 0, 30, -3, 10629),
        (12, 1, 72, -50, 29445),
        (12, 1, 127, 127, 23893),
        (12, 64, 60, -100, 29619),
        (12, 64, 0, 0, 1936),
        (12, 125, 60, -1, 15918),
        (12, 125, 59, -127, 28389),
        (127, 0, 60, 0, 78),
        (127, 0, 48, 127, 41),
        (127, 0, 30, -3, 13),
        (127, 1, 72, -50, 153),
        (127, 1, 127, 127, 3987),
        (127, 64, 60, -100, 77),
        (127, 64, 0, 0, 2),
        (127, 125, 60, -1, 82),
        (127, 125, 59, -127, 74),
        (-60, 0, 60, 0, 4096),
        (-60, 0, 48, 127, 2169),
        (-60, 0, 30, -3, 723),
        (-60, 1, 72, -50, 8013),
        (-60, 1, 127, 127, 26007),
        (-60, 64, 60, -100, 4030),
        (-60, 64, 0, 0, 132),
        (-60, 125, 60, -1, 4332),
        (-60, 125, 59, -127, 3863),
        (-48, 0, 60, 0, 8192),
        (-48, 0, 48, 127, 4338),
        (-48, 0, 30, -3, 1446),
        (-48, 1, 72, -50, 16025),
        (-48, 1, 127, 127, 26007),
        (-48, 64, 60, -100, 8060),
        (-48, 64, 0, 0, 264),
        (-48, 125, 60, -1, 8663),
        (-48, 125, 59, -127, 7725),
        (-1, 0, 60, 0, 30928),
        (-1, 0, 48, 127, 32752),
        (-1, 0, 30, -3, 21840),
        (-1, 1, 72, -50, 30251),
        (-1, 1, 127, 127, 24548),
        (-1, 64, 60, -100, 30429),
        (-1, 64, 0, 0, 3979),
        (-1, 125, 60, -1, 32707),
        (-1, 125, 59, -127, 29165),
        (29, 117, 29, 20, 4002),
        (57, 26, 43, 26, 1716),
        (100, 83, 81, -21, 1291),
        (60, 78, -18, 87, 45),
        (12, 115, 16, 8, 5011),
        (-5, 18, 55, 58, 19031),
        (86, 37, 144, -105, 26008),
        (-13, 83, 52, 48, 20670),
        (-54, 70, 149, 12, 32094),
        (-69, 5, 11, -44, 141),
        (25, 35, 150, -45, 20002),
        (-33, 6, -1, -21, 571),
        (-113, 87, -16, 31, 3),
        (2, 105, 82, -43, 24571),
        (-8, 69, -13, 60, 1291),
        (7, 122, 106, 110, 19876),
        (61, 27, 56, -8, 2843),
        (-15, 19, 76, -85, 16848),
        (34, 64, 46, -11, 7708),
        (77, 99, 62, 28, 1675),
        (-5, 42, 148, 78, 32649),
        (124, 120, 92, 120, 660),
        (42, 110, 110, -104, 23958),
        (36, 115, 137, -122, 20028),
        (-15, 101, 120, -124, 27269),
        (-10, 72, 117, 62, 16428),
        (-104, 42, 75, 94, 816),
        (72, 55, 132, -30, 15222),
        (-60, 0, -15, 68, 55),
        (-112, 108, 25, 58, 29),
        (-57, 46, -17, 32, 59),
        (114, 108, 144, -63, 21723),
        (-17, 118, 76, 31, 16540),
        (-80, 90, 30, 28, 241),
        (-43, 103, 130, -97, 19535),
        (7, 47, 101, -57, 26698),
        (112, 3, 62, -124, 198),
        (-119, 38, 74, -25, 306),
        (-91, 112, 147, 16, 27554),
        (-24, 37, 84, -62, 32399),
        (124, 121, 17, 98, 8),
        (-125, 117, 100, 41, 1038),
        (48, 78, 39, -76, 2239),
        (99, 58, 20, -54, 39),
        (-87, 122, -2, -53, 25),
        (-23, 98, 16, -94, 2739),
        (3, 40, -6, 86, 2368),
        (-13, 35, -12, 52, 1005),
        (13, 10, 131, 89, 28045),
        (-36, 33, 104, 108, 27715),
        (7, 0, 151, 72, 15549),
        (24, 23, 86, -10, 16994),
        (-77, 20, 54, 104, 1147),
        (6, 15, 10, 77, 4941),
        (-38, 2, 17, 7, 1223),
        (110, 35, 114, 63, 4955),
        (92, 6, 70, -55, 1032),
        (-60, 16, 97, 20, 17641),
        (-61, 39, 16, -30, 306),
        (3, 14, 11, 75, 6218),
        (118, 63, 54, -89, 91),
        (-122, 74, 72, 111, 248),
        (8, 38, -19, -63, 781),
        (62, 64, 105, -93, 22259),
        (12, 84, 16, -107, 4692),
        (-122, 106, 3, -84, 4),
        (119, 52, 49, 33, 68),
        (-47, 9, 95, -61, 32007),
        (-64, 30, 36, 41, 839),
        (-77, 30, 23, -11, 183),
        (34, 7, -17, 115, 208),
        (-75, 96, 82, 68, 6608),
        (-19, 72, 115, -84, 32589),
        (-31, 64, 20, -47, 2186),
        (-82, 110, 72, -29, 2384),
    ];
    // split_note: (bend down s8, bend up s8, bend, mod, note, fine)
    const SPLIT_GOLDEN: &[(i8, i8, i32, i32, i32, i32)] = &[
        (2, 2, 0, 0, 60, 0),
        (2, 2, 0, 1000, 67, 104),
        (2, 2, 1, 64, 60, 64),
        (2, 2, 1, -1000, 52, 24),
        (2, 2, -1, -64, 59, 64),
        (2, 2, 32767, 0, 62, 0),
        (2, 2, 32767, 1000, 69, 104),
        (2, 2, -32768, 64, 58, 64),
        (2, 2, -32768, -1000, 50, 24),
        (2, 2, 12345, -64, 60, 32),
        (2, 2, -12345, 0, 59, 32),
        (2, 2, -12345, 1000, 67, 8),
        (2, 2, 100, 64, 60, 64),
        (2, 2, 100, -1000, 52, 24),
        (2, 2, -100, -64, 59, 64),
        (12, 12, 0, 0, 60, 0),
        (12, 12, 0, 1000, 67, 104),
        (12, 12, 1, 64, 60, 64),
        (12, 12, 1, -1000, 52, 24),
        (12, 12, -1, -64, 59, 64),
        (12, 12, 32767, 0, 72, 0),
        (12, 12, 32767, 1000, 79, 104),
        (12, 12, -32768, 64, 48, 64),
        (12, 12, -32768, -1000, 40, 24),
        (12, 12, 12345, -64, 64, 2),
        (12, 12, -12345, 0, 55, 62),
        (12, 12, -12345, 1000, 63, 38),
        (12, 12, 100, 64, 60, 68),
        (12, 12, 100, -1000, 52, 28),
        (12, 12, -100, -64, 59, 60),
        (-12, 12, 0, 0, 60, 0),
        (-12, 12, 0, 1000, 67, 104),
        (-12, 12, 1, 64, 60, 64),
        (-12, 12, 1, -1000, 52, 24),
        (-12, 12, -1, -64, 59, 64),
        (-12, 12, 32767, 0, 72, 0),
        (-12, 12, 32767, 1000, 79, 104),
        (-12, 12, -32768, 64, 72, 64),
        (-12, 12, -32768, -1000, 64, 24),
        (-12, 12, 12345, -64, 64, 2),
        (-12, 12, -12345, 0, 64, 66),
        (-12, 12, -12345, 1000, 72, 42),
        (-12, 12, 100, 64, 60, 68),
        (-12, 12, 100, -1000, 52, 28),
        (-12, 12, -100, -64, 59, 68),
        (0, 0, 0, 0, 60, 0),
        (0, 0, 0, 1000, 67, 104),
        (0, 0, 1, 64, 60, 64),
        (0, 0, 1, -1000, 52, 24),
        (0, 0, -1, -64, 59, 64),
        (0, 0, 32767, 0, 60, 0),
        (0, 0, 32767, 1000, 67, 104),
        (0, 0, -32768, 64, 60, 64),
        (0, 0, -32768, -1000, 52, 24),
        (0, 0, 12345, -64, 59, 64),
        (0, 0, -12345, 0, 60, 0),
        (0, 0, -12345, 1000, 67, 104),
        (0, 0, 100, 64, 60, 64),
        (0, 0, 100, -1000, 52, 24),
        (0, 0, -100, -64, 59, 64),
        (24, 7, 0, 0, 60, 0),
        (24, 7, 0, 1000, 67, 104),
        (24, 7, 1, 64, 60, 64),
        (24, 7, 1, -1000, 52, 24),
        (24, 7, -1, -64, 59, 64),
        (24, 7, 32767, 0, 67, 0),
        (24, 7, 32767, 1000, 74, 104),
        (24, 7, -32768, 64, 36, 64),
        (24, 7, -32768, -1000, 28, 24),
        (24, 7, 12345, -64, 62, 17),
        (24, 7, -12345, 0, 50, 123),
        (24, 7, -12345, 1000, 58, 99),
        (24, 7, 100, 64, 60, 66),
        (24, 7, 100, -1000, 52, 26),
        (24, 7, -100, -64, 59, 55),
        (127, -127, 0, 0, 60, 0),
        (127, -127, 0, 1000, 67, 104),
        (127, -127, 1, 64, 60, 64),
        (127, -127, 1, -1000, 52, 24),
        (127, -127, -1, -64, 59, 64),
        (127, -127, 32767, 0, -67, 0),
        (127, -127, 32767, 1000, -59, -24),
        (127, -127, -32768, 64, -66, -64),
        (127, -127, -32768, -1000, -74, -104),
        (127, -127, 12345, -64, 11, 84),
        (127, -127, -12345, 0, 12, 20),
        (127, -127, -12345, 1000, 19, 124),
        (127, -127, 100, 64, 60, 15),
        (127, -127, 100, -1000, 51, 103),
        (127, -127, -100, -64, 59, 15),
        (-45, 115, -12996, 18982, 226, 18),
        (-118, -105, 24070, 22042, 155, 10),
        (34, 33, -24660, 19225, 184, 78),
        (-20, -91, -17329, 7665, 130, 58),
        (-103, 13, -24539, -24956, -57, -107),
        (-35, -51, -207, -9206, -11, -90),
        (59, -13, 4972, -23174, -123, -2),
        (-20, -117, -22595, 8355, 139, 8),
        (88, -104, 2613, 29373, 281, 24),
        (-55, 56, 17798, 12714, 189, 95),
        (-112, -72, 4906, -15816, -74, -43),
        (-85, -13, 19876, 3648, 80, 79),
        (118, -68, -12987, -21892, -157, -102),
        (-55, -126, -13674, 22144, 255, 121),
        (103, 95, 18661, 19407, 265, 92),
        (-110, 125, -5405, 24985, 273, 43),
        (115, -34, -29426, -23552, -227, -34),
        (-6, -96, -17649, 31204, 307, 1),
        (-101, 64, 12141, 1934, 98, 105),
        (49, 12, -29224, 6303, 65, 70),
        (70, -70, 10441, -3534, 10, 11),
        (5, -1, 13836, -28970, -166, -96),
        (79, 112, 13044, 15025, 221, 123),
        (-4, 32, -32518, 30077, 298, 121),
        (73, 55, -6643, 29888, 278, 90),
        (-25, 63, -21638, -11947, -16, -106),
        (29, 84, 29406, 13160, 238, 25),
        (-16, 125, -7235, -5107, 23, 81),
        (12, -20, -15588, -24786, -139, -44),
        (9, -89, -30317, 24920, 246, 47),
        (15, -112, 9959, 30472, 264, 3),
        (70, -102, 26499, -29116, -249, -122),
        (-12, 3, 29889, -308, 60, 42),
        (-96, -27, 25181, 8648, 106, 105),
    ];

    #[test]
    fn unity_octaves_and_44k_scaling() {
        let t = Note2PitchTables::from_formula();
        // tone centred on the SFX note 60: negative centre byte = 48 kHz sample -> exactly 0x1000
        assert_eq!(tone_pitch(&t, -60, 0, 60, 0), 0x1000);
        // non-negative centre byte = 44.1 kHz sample -> 0x1000 * 44100 / 48000
        assert_eq!(tone_pitch(&t, 60, 0, 60, 0), 3763);
        assert_eq!(tone_pitch(&t, -48, 0, 60, 0), 0x2000);
        assert_eq!(tone_pitch(&t, -72, 0, 60, 0), 0x0800);
        // the centre fine is ADDED: +64/128 semitone sounds higher
        assert!(tone_pitch(&t, -60, 64, 60, 0) > 0x1000);
        assert_eq!(tone_pitch(&t, -60, 64, 60, 0), tone_pitch(&t, -60, 0, 60, 64));
        // voice_pitch with no bend / mod
        assert_eq!(voice_pitch(&t, -60, 0, 2, 2, 0, 0, 60, 0), 0x1000);
        // one semitone of pitch mod (128) = one note up
        assert_eq!(voice_pitch(&t, -60, 0, 2, 2, 0, 128, 60, 0), tone_pitch(&t, -60, 0, 61, 0));
        assert_eq!(split_note(2, 2, 0, 0, 60, 0), (60, 0));
        assert_eq!(split_note(2, 2, 0, 128, 60, 0), (61, 0));
        // total 7679 -> note 59, fine 127
        assert_eq!(split_note(2, 2, 0, -1, 60, 0), (59, 127));
        // full bend up with a 2 semitone range: +2 notes
        assert_eq!(split_note(2, 2, 0x7fff, 0, 60, 0), (62, 0));
    }

    #[test]
    fn matches_the_python_replay() {
        let t = Note2PitchTables::from_formula();
        for &(c, cf, note, fine, want) in TONE_PITCH_GOLDEN {
            assert_eq!(tone_pitch(&t, c, cf, note as i32, fine as i32), want, "centre {c} cf {cf} note {note} fine {fine}");
        }
        for &(bd, bu, bend, md, note, fine) in SPLIT_GOLDEN {
            assert_eq!(split_note(bd, bu, bend, md, 60, 0), (note, fine), "down {bd} up {bu} bend {bend} mod {md}");
        }
    }

    #[test]
    fn tables_equal_the_libsd_tables() {
        let Some(blob) = testdata::read("globals/irx.bin") else {
            eprintln!("skip: extracted/globals/irx.bin absent");
            return;
        };
        let libsd = Irx::parse(Irx::find_in_blob(&blob, crate::snd989::irx::MODULE_LIBSD).unwrap()).unwrap();
        assert_eq!(Note2PitchTables::from_libsd(&libsd).unwrap(), Note2PitchTables::from_formula());
    }
}
