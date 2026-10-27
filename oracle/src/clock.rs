//! The host's clock, in the two layouts the console uses: the CDVD real time clock reply (sceCdCLOCK: BCD fields)
//! and the memory card's directory entry time (binary fields, a four digit year). The emulated PS2 has no clock of
//! its own: the CDVD S-command the game reads the time with (hle.rs) and the card server's file timestamps (mc.rs)
//! both report the host's local time, timezone and DST included.
//!
//! The reply is not the local time as it stands. The game converts it as if the console were set to Japan (with the
//! console's timezone unset it falls back to 540 minutes, boot 0x12d6d8) and subtracts nine hours before the clock is
//! stored in a save, so the reply hands it the local time shifted forward by those nine hours.

/// The local time, broken down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
}

/// The host's local time (`localtime_r`, so TZ and DST apply).
pub fn local() -> Local {
    // SAFETY: localtime_r fills the caller's struct with the broken down local time; a zeroed tm is a valid start.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: time writes the current time through the pointer, or returns it when the pointer is null.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    // SAFETY: the time pointer and the struct are valid.
    unsafe { libc::localtime_r(&now, &mut tm) };
    Local {
        year: (tm.tm_year + 1900) as u32,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        min: tm.tm_min as u32,
        sec: tm.tm_sec as u32,
    }
}

/// The console clock reply for the host's local time.
pub fn local_rtc() -> [u8; 8] {
    rtc(local())
}

/// The host's local time in the card layout.
pub fn local_card() -> [u8; 8] {
    card(local())
}

/// The hours the game subtracts from the reply before it saves: JST, the fallback when the console's timezone is
/// unset.
const HOURS_AHEAD: u32 = 9;

/// sceCdCLOCK, the CDVD RTC reply: status (zero), sec, min, hour, pad, day, month, year (two digits), all BCD, with the
/// clock shifted forward by the nine hours the game subtracts (see the module doc).
pub fn rtc(t: Local) -> [u8; 8] {
    let t = later(t, HOURS_AHEAD);
    [0, bcd(t.sec), bcd(t.min), bcd(t.hour), 0, bcd(t.day), bcd(t.month), bcd(t.year % 100)]
}

/// The time `hours` hours later, the calendar rolling over as it goes.
fn later(mut t: Local, hours: u32) -> Local {
    t.hour += hours;
    while t.hour >= 24 {
        t.hour -= 24;
        t.day += 1;
        if t.day > days_in_month(t.year, t.month) {
            t.day = 1;
            t.month += 1;
            if t.month > 12 {
                t.month = 1;
                t.year += 1;
            }
        }
    }
    t
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 31,
    }
}

fn is_leap(year: u32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// A memory card directory entry's time (the file system's "Time of Day"): unused, sec, min, hour, day, month, then
/// the year as a little endian word, all binary.
pub fn card(t: Local) -> [u8; 8] {
    let y = (t.year as u16).to_le_bytes();
    [0, t.sec as u8, t.min as u8, t.hour as u8, t.day as u8, t.month as u8, y[0], y[1]]
}

fn bcd(v: u32) -> u8 {
    (((v / 10) << 4) | (v % 10)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Local = Local { year: 2026, month: 10, day: 23, hour: 16, min: 5, sec: 9 };

    /// The CDVD reply is the console's own layout and is BCD with a two digit year, the card's time is binary with a
    /// four digit year.
    #[test]
    fn the_reply_and_the_card_time_keep_their_layouts() {
        assert_eq!(rtc(T), [0x00, 0x09, 0x05, 0x01, 0x00, 0x24, 0x10, 0x26]);
        assert_eq!(card(T), [0, 9, 5, 16, 23, 10, 0xea, 0x07]);
    }

    /// The game converts the reply as if the console were in Japan and so subtracts nine hours before saving: the
    /// reply hands it the local time shifted forward by those nine hours, and the saved clock then holds the local
    /// date and time again.
    #[test]
    fn the_reply_is_nine_hours_ahead_so_the_saved_clock_holds_the_local_time() {
        assert_eq!(rtc(T), [0x00, 0x09, 0x05, 0x01, 0x00, 0x24, 0x10, 0x26]);
    }

    /// The shift carries into the next day, month and year.
    #[test]
    fn the_reply_shift_rolls_the_calendar_over() {
        let at = |year, month, day| Local { year, month, day, hour: 20, min: 0, sec: 0 };
        assert_eq!(rtc(at(2026, 12, 31)), [0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x01, 0x27]);
        assert_eq!(rtc(at(2027, 2, 28)), [0x00, 0x00, 0x00, 0x05, 0x00, 0x01, 0x03, 0x27]);
        assert_eq!(rtc(at(2028, 2, 28)), [0x00, 0x00, 0x00, 0x05, 0x00, 0x29, 0x02, 0x28]);
    }

    /// The host's clock is read: a plausible date and a four digit year.
    #[test]
    fn the_hosts_local_time_is_read() {
        let t = local();
        assert!((2000..=2099).contains(&t.year), "year {}", t.year);
        assert!((1..=12).contains(&t.month), "month {}", t.month);
        assert!((1..=31).contains(&t.day), "day {}", t.day);
        assert!(t.hour < 24 && t.min < 60 && t.sec < 60);
    }
}
