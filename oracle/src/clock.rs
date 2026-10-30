//! The console's clock, the way a real PS2 presents it to the game.
//!
//! A PS2 has a real time clock (the CDVD chip's RTC) that always runs in Japan Standard Time (UTC+9), and the OSD
//! (system configuration) holds the time zone (minutes east of UTC) and the summer time flag the user chose. The game
//! reads the RTC with sceCdReadClock (CDVD S-command 1, boot 0x121780: BCD fields sec, min, hour, pad, day, month, year)
//! and shifts it with the OSD settings (boot 0x12d6d8: minutes + time zone + 60 * summer time - 540) before it stores it
//! in a save, so a console set to the user's time zone shows the user's local time. This module hands the game exactly
//! that: the RTC in JST, the host's standard offset as the OSD time zone (cold.rs `osd_config_word`, hle.rs syscall
//! 0x4b) and the host's summer time flag as the OSD second parameter block (hle.rs syscall 0x6f).
//!
//! The clock is real only when the host program says so (`set_real`): replays, hash runs and savestate tests keep the
//! zeroed reply they always had (the RAM they produce would otherwise change with the day), so nothing here runs unless
//! rac-rs switches it on for normal play.
//!
//! The reply conversion was found by DeSynkro (rac-rs-linux, PR #38: the game converts it as if the console were set to
//! Japan and subtracts nine hours; their `clock.rs`). Here the same result comes from the console's own settings rather
//! than from shifting the reply by hand.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static REAL: AtomicBool = AtomicBool::new(false);

/// Switches the host clock on (normal play) or off (the fixed, zeroed clock of tests and replays; the default).
pub fn set_real(on: bool) {
    REAL.store(on, Ordering::Relaxed);
}

/// True when the game gets the host's clock.
pub fn real() -> bool {
    #[cfg(test)]
    if let Some(f) = fake::get() {
        return f.real;
    }
    REAL.load(Ordering::Relaxed)
}

/// Test-only stand-in for the host: a thread-local clock, zone and switch, so a test can run the oracle's clock paths with a known
/// time and time zone (cargo runs tests in parallel threads; nothing here touches the process-wide switch).
#[cfg(test)]
pub(crate) mod fake {
    use super::Zone;
    use std::cell::Cell;

    #[derive(Clone, Copy)]
    pub(crate) struct Fake {
        pub real: bool,
        pub unix: i64,
        pub zone: Zone,
    }

    thread_local! {
        static FAKE: Cell<Option<Fake>> = const { Cell::new(None) };
    }

    pub(crate) fn get() -> Option<Fake> {
        FAKE.with(|f| f.get())
    }

    /// Runs `body` with the fake host in place on this thread, then removes it.
    pub(crate) fn with<R>(f: Fake, body: impl FnOnce() -> R) -> R {
        FAKE.with(|c| c.set(Some(f)));
        let r = body();
        FAKE.with(|c| c.set(None));
        r
    }
}

/// A calendar date and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
}

/// The host's time zone as the console's OSD settings hold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    /// Standard (winter) offset from UTC in minutes, east positive.
    pub std_min: i32,
    /// Summer time is in effect now.
    pub dst: bool,
}

/// Seconds since 1970-01-01 00:00:00 UTC.
pub fn unix_now() -> i64 {
    #[cfg(test)]
    if let Some(f) = fake::get() {
        return f.unix;
    }
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The calendar date and time `t` Unix seconds mean (proleptic Gregorian calendar, no leap seconds).
pub fn civil_from_unix(t: i64) -> Civil {
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400) as u32;
    // Days since 0000-03-01, then the era / year-of-era split (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u32;
    Civil { year, month, day, hour: rem / 3600, min: rem % 3600 / 60, sec: rem % 60 }
}

/// Hours the console's RTC runs ahead of UTC (Japan Standard Time).
pub const JST_HOURS: i64 = 9;

/// The RTC at Unix time `t`: Japan Standard Time.
pub fn jst(t: i64) -> Civil {
    civil_from_unix(t + JST_HOURS * 3600)
}

/// sceCdCLOCK, the CDVD RTC reply: stat (zero = ok), sec, min, hour, pad, day, month, year (two digits), all BCD.
pub fn rtc(t: Civil) -> [u8; 8] {
    [0, bcd(t.sec), bcd(t.min), bcd(t.hour), 0, bcd(t.day), bcd(t.month), bcd(t.year % 100)]
}

/// A memory card directory entry's time (the card file system's "time of day", in the console's own JST): unused, sec,
/// min, hour, day, month, then the year as a little endian word, all binary.
pub fn card(t: Civil) -> [u8; 8] {
    let y = (t.year as u16).to_le_bytes();
    [0, t.sec as u8, t.min as u8, t.hour as u8, t.day as u8, t.month as u8, y[0], y[1]]
}

/// The RTC reply for now.
pub fn rtc_now() -> [u8; 8] {
    rtc(jst(unix_now()))
}

/// The card directory time for now, or the zero time the card had before there was a clock.
pub fn card_now() -> [u8; 8] {
    if real() {
        card(jst(unix_now()))
    } else {
        [0; 8]
    }
}

/// The OSD configuration word's time zone field (bits 21..31, signed minutes) for a zone.
pub fn osd_zone_bits(z: Zone) -> u32 {
    (z.std_min as u32 & 0x7ff) << 21
}

/// The OSD second parameter block as the game reads the summer time flag (byte 1, bit 4).
pub fn osd2_byte(z: Zone, index: usize) -> u8 {
    if index == 1 && z.dst {
        0x10
    } else {
        0
    }
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

/// The zone for a host whose offset from UTC right now is `current_east_min` minutes east and whose summer time is in effect
/// (`dst`) or not. The game adds 60 minutes to the OSD time zone when the summer time flag is set, so the standard offset it needs
/// is the current offset minus those 60 minutes, whatever the host's own summer time shift is (Lord Howe Island shifts by 30,
/// some zones count winter as "summer time"): the game's result is then always the host's current local time.
pub fn zone_from_current(current_east_min: i32, dst: bool) -> Zone {
    Zone { std_min: current_east_min - if dst { 60 } else { 0 }, dst }
}

/// The zone from Windows' TIME_ZONE_INFORMATION values and the id GetTimeZoneInformation returned with them (UNKNOWN = 0,
/// STANDARD = 1, DAYLIGHT = 2, failure = 0xffffffff). UTC = local + (bias + the bias of the current period), so east = minus that.
pub fn zone_from_windows(bias: i32, standard_bias: i32, daylight_bias: i32, id: u32) -> Zone {
    if id == 0xffff_ffff {
        return Zone { std_min: 0, dst: false };
    }
    let dst = id == 2;
    zone_from_current(-(bias + if dst { daylight_bias } else { standard_bias }), dst)
}

/// The host's time zone as of this moment (read again at every use: the game reads it at each save, and a session can cross
/// a summer time change). Tests can replace it (`fake`).
pub fn zone() -> Zone {
    #[cfg(test)]
    if let Some(f) = fake::get() {
        return f.zone;
    }
    host_zone()
}

#[cfg(windows)]
fn host_zone() -> Zone {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SystemTimeW([u16; 8]);
    #[repr(C)]
    struct Tzi {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTimeW,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTimeW,
        daylight_bias: i32,
    }
    extern "system" {
        fn GetTimeZoneInformation(tzi: *mut Tzi) -> u32;
    }
    // SAFETY: an all-zero Tzi is valid plain data; GetTimeZoneInformation fills the struct it is given.
    let mut tzi: Tzi = unsafe { std::mem::zeroed() };
    // SAFETY: the pointer is valid for the call.
    let id = unsafe { GetTimeZoneInformation(&mut tzi) };
    zone_from_windows(tzi.bias, tzi.standard_bias, tzi.daylight_bias, id)
}

#[cfg(unix)]
fn host_zone() -> Zone {
    // The C library's broken down time (glibc and BSD layout: nine ints, then the offset east of UTC in seconds and the
    // abbreviation pointer).
    #[repr(C)]
    struct Tm {
        ints: [i32; 9],
        gmtoff: isize,
        zone: *const u8,
    }
    extern "C" {
        fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    }
    let now = unix_now();
    let mut tm = Tm { ints: [0; 9], gmtoff: 0, zone: std::ptr::null() };
    // SAFETY: both pointers are valid for the call; localtime_r fills the struct.
    let ok = unsafe { !localtime_r(&now, &mut tm).is_null() };
    if !ok {
        return Zone { std_min: 0, dst: false };
    }
    zone_from_current(tm.gmtoff as i32 / 60, tm.ints[8] > 0)
}

/// The host's time zone: no time zone information on this target.
#[cfg(not(any(windows, unix)))]
fn host_zone() -> Zone {
    Zone { std_min: 0, dst: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn at(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> Civil {
        Civil { year, month, day, hour, min, sec }
    }

    /// 2026-10-28 14:07:09 UTC (Unix 1_793_196_429) and the epoch itself.
    #[test]
    fn unix_seconds_become_calendar_dates() {
        assert_eq!(civil_from_unix(0), at(1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_unix(1_793_196_429), at(2026, 10, 28, 14, 7, 9));
        assert_eq!(civil_from_unix(951_782_400), at(2000, 2, 29, 0, 0, 0));
        assert_eq!(civil_from_unix(-1), at(1969, 12, 31, 23, 59, 59));
    }

    /// The RTC runs on JST: nine hours ahead of UTC, rolling the calendar over (month end, year end, leap day).
    #[test]
    fn the_rtc_is_jst() {
        assert_eq!(jst(1_793_196_429), at(2026, 10, 28, 23, 7, 9));
        assert_eq!(jst(1_793_196_429 + 3600), at(2026, 10, 29, 0, 7, 9));
        // 2026-12-31 20:00:00 UTC is 2027-01-01 05:00:00 JST.
        assert_eq!(jst(1_798_747_200), at(2027, 1, 1, 5, 0, 0));
        // 2028-02-28 20:00:00 UTC is 2028-02-29 05:00:00 JST (leap year); 2027-02-28 20:00 UTC is 2027-03-01 05:00 JST.
        assert_eq!(jst(1_835_380_800), at(2028, 2, 29, 5, 0, 0));
        assert_eq!(jst(1_803_844_800), at(2027, 3, 1, 5, 0, 0));
    }

    /// sceCdCLOCK is BCD with a two digit year; the card time is binary with a four digit little endian year.
    #[test]
    fn the_reply_and_the_card_time_keep_their_layouts() {
        let t = at(2026, 10, 23, 16, 5, 9);
        assert_eq!(rtc(t), [0x00, 0x09, 0x05, 0x16, 0x00, 0x23, 0x10, 0x26]);
        assert_eq!(card(t), [0, 9, 5, 16, 23, 10, 0xea, 0x07]);
        assert_eq!(rtc(at(2100, 12, 31, 23, 59, 59))[7], 0x00);
    }

    /// The time zone goes into the OSD word as 11 bit signed minutes (bits 21..31) the game reads back with an
    /// arithmetic shift; the summer time flag is bit 4 of byte 1 of the second block.
    #[test]
    fn the_osd_settings_round_trip() {
        for std_min in [0, 60, 120, 540, 345, 840, -60, -300, -720] {
            let w = osd_zone_bits(Zone { std_min, dst: false });
            assert_eq!((w as i32) >> 21, std_min, "zone {std_min}");
            assert_eq!(w & 0x001f_ffff, 0);
        }
        assert_eq!(osd2_byte(Zone { std_min: 60, dst: true }, 1), 0x10);
        assert_eq!(osd2_byte(Zone { std_min: 60, dst: true }, 0), 0);
        assert_eq!(osd2_byte(Zone { std_min: 60, dst: false }, 1), 0);
    }

    /// What the game's own conversion (boot 0x12d6d8: RTC + time zone + 60 * summer time - 540 minutes) makes of the reply:
    /// the host's local time, for zones with and without summer time, across a date change.
    #[test]
    fn the_games_conversion_gives_local_time() {
        let game = |utc: i64, z: Zone| {
            let shift = i64::from(z.std_min) + if z.dst { 60 } else { 0 } - 540;
            // the game adds `shift` minutes to the RTC reply; do the same on seconds since the epoch
            civil_from_unix(utc + JST_HOURS * 3600 + shift * 60)
        };
        // 2026-10-28 14:07:09 UTC: Berlin (CET, no summer time any more) shows 15:07:09, Los Angeles (PDT) 07:07:09.
        assert_eq!(game(1_793_196_429, Zone { std_min: 60, dst: false }), at(2026, 10, 28, 15, 7, 9));
        assert_eq!(game(1_793_196_429, Zone { std_min: -480, dst: true }), at(2026, 10, 28, 7, 7, 9));
        // Tokyo shows the RTC itself, Kathmandu (+5:45) crosses midnight: 2026-10-28 20:00 UTC is 2026-10-29 01:45 there.
        assert_eq!(game(1_793_196_429, Zone { std_min: 540, dst: false }), jst(1_793_196_429));
        assert_eq!(game(1_793_217_600, Zone { std_min: 345, dst: false }), at(2026, 10, 29, 1, 45, 0));
    }

    /// The standard offset the game needs is the current offset minus its own 60 minute summer time step, whatever the host's
    /// summer time shift is (Windows values: bias, standard bias, daylight bias, id).
    #[test]
    fn zones_from_windows_and_c_library_values() {
        let z = |std_min, dst| Zone { std_min, dst };
        // Berlin: winter and summer (bias -60, daylight bias -60); Los Angeles (bias 480)
        assert_eq!(zone_from_windows(-60, 0, -60, 1), z(60, false));
        assert_eq!(zone_from_windows(-60, 0, -60, 2), z(60, true));
        assert_eq!(zone_from_windows(480, 0, -60, 2), z(-480, true));
        // zone without summer time, id UNKNOWN
        assert_eq!(zone_from_windows(-540, 0, 0, 0), z(540, false));
        // Lord Howe: +10:30 and +11:00 in summer (daylight bias -30): the game's +60 on 600 gives 660
        assert_eq!(zone_from_windows(-630, 0, -30, 1), z(630, false));
        assert_eq!(zone_from_windows(-630, 0, -30, 2), z(600, true));
        // a positive daylight bias (clocks back during Ramadan in Morocco, +1 -> +0): standard -60 with the flag on gives 0
        assert_eq!(zone_from_windows(-60, 0, 60, 2), z(-60, true));
        // failure
        assert_eq!(zone_from_windows(-60, 0, -60, 0xffff_ffff), z(0, false));
        // C library: Dublin in winter (offset 0, isdst 1) and Kathmandu
        assert_eq!(zone_from_current(0, true), z(-60, true));
        assert_eq!(zone_from_current(345, false), z(345, false));
        // the game's sum (std + 60 * dst) is always the current offset
        for (cur, dst) in [(0, true), (60, false), (660, true), (-420, true), (345, false)] {
            let zz = zone_from_current(cur, dst);
            assert_eq!(zz.std_min + if zz.dst { 60 } else { 0 }, cur);
        }
    }

    /// The fixed clock stays the default: no host time reaches a test, and the card keeps its zero times.
    #[test]
    fn the_default_is_the_fixed_clock() {
        assert!(!real());
        assert_eq!(card_now(), [0; 8]);
    }

    /// The host's time zone and clock can be read.
    #[test]
    fn the_hosts_zone_is_read() {
        let z = zone();
        assert!((-840..=840).contains(&z.std_min), "zone {z:?}");
        let c = civil_from_unix(unix_now());
        assert!((2024..=2099).contains(&c.year), "year {}", c.year);
        assert!(c.month >= 1 && c.month <= 12 && c.day >= 1 && c.day <= 31);
    }
}
