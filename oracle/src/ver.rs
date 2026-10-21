//! Which retail disc runs, and the video mode the game chose: R&C1 PAL v1.00 (SCES-50916, boot ELF SCES_509.16) or USA
//! v1.00 (SCUS-97199, boot ELF SCUS_971.99). Both are the same program; the USA build only moves the level code and data
//! (address maps: tools/vermap.py) and changes a few functions. The game picks 50 or 60 Hz itself: its engine init reads
//! SYSTEM.CNF (sector 0x121) and sets the PAL flag 0x15ed80 when byte 0x33 is not 'N' (VMODE = PAL / NTSC); see
//! decomp/notes/region_versions.md. Defaults are PAL, so nothing changes for the PAL disc. The PAL disc's 50/60 Hz option
//! switches it at run time: every switch resets the GS through SetGsCrt (hle.rs syscall 0x02), which sets the mode here.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disc {
    Pal,
    Usa,
}

static DISC: AtomicU8 = AtomicU8::new(0);
static NTSC: AtomicBool = AtomicBool::new(false);

impl Disc {
    /// The boot ELF on the disc (ISO9660 root, without the ";1" version).
    pub fn boot_file(self) -> &'static str {
        match self {
            Disc::Pal => "SCES_509.16",
            Disc::Usa => "SCUS_971.99",
        }
    }
    /// The serial as the memory card directory and the setup check name it.
    pub fn serial(self) -> &'static str {
        match self {
            Disc::Pal => "SCES-50916",
            Disc::Usa => "SCUS-97199",
        }
    }
    /// Short tag for logs and file names.
    pub fn tag(self) -> &'static str {
        match self {
            Disc::Pal => "pal",
            Disc::Usa => "usa",
        }
    }
}

/// The disc of this process (set by `Machine::cold` / `Machine::load` from the disc image; PAL until then).
pub fn disc() -> Disc {
    if DISC.load(Ordering::Relaxed) == 1 { Disc::Usa } else { Disc::Pal }
}

pub fn set_disc(d: Disc) {
    DISC.store((d == Disc::Usa) as u8, Ordering::Relaxed);
}

/// True while the game runs in NTSC mode (60 fields a second).
pub fn ntsc() -> bool {
    NTSC.load(Ordering::Relaxed)
}

pub fn set_ntsc(on: bool) {
    NTSC.store(on, Ordering::Relaxed);
}

/// One field in EE instructions (294.912 MHz / 50, or / 59.94) when nothing waits at the vblank poll.
pub fn period() -> u64 {
    if ntsc() { 4_920_000 } else { 5_900_000 }
}

/// T1_COUNT counts in one field (BUSCLK / 256 / 50 = 11520; NTSC 147456000 / 256 / 59.94 = 9609.6 -> 9610).
pub fn t1_field() -> u32 {
    if ntsc() { 9610 } else { 11520 }
}

/// Fields per second of the current video mode (50, or 59.94 for NTSC).
pub fn field_hz() -> f64 {
    if ntsc() { 60000.0 / 1001.0 } else { 50.0 }
}

/// Called with the RAM on every rec table rebuild (after each overlay load), before any code of the new overlay runs.
static ON_REBUILD: std::sync::OnceLock<fn(&[u8])> = std::sync::OnceLock::new();

/// Sets the rebuild callback (rac-rs: which overlay is in RAM, for its address map on the USA disc). Once per process.
pub fn on_rebuild(f: fn(&[u8])) {
    let _ = ON_REBUILD.set(f);
}

pub(crate) fn rebuilt(ram: &[u8]) {
    if let Some(f) = ON_REBUILD.get() {
        f(ram);
    }
}

/// The disc in an image: whichever boot ELF its root directory holds (None: neither, or unreadable).
pub fn detect(iso: &str) -> Option<Disc> {
    let mut f = std::fs::File::open(iso).ok()?;
    for d in [Disc::Pal, Disc::Usa] {
        if crate::cold::iso_find(&mut f, &format!("{};1", d.boot_file())).is_some() {
            return Some(d);
        }
    }
    None
}

/// VMODE of the image's SYSTEM.CNF as the game reads it (sector 0x121, byte 0x33 'N' = NTSC); None if unreadable.
pub fn vmode_ntsc(iso: &str) -> Option<bool> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(iso).ok()?;
    f.seek(SeekFrom::Start(0x121 * 2048 + 0x33)).ok()?;
    let mut b = [0u8; 1];
    f.read_exact(&mut b).ok()?;
    Some(b[0] == b'N')
}

/// Sets the disc and the video mode from an image (cold boot: the mode the game will pick from SYSTEM.CNF).
pub fn from_iso(iso: &str) {
    if let Some(d) = detect(iso) {
        set_disc(d);
    }
    if let Some(n) = vmode_ntsc(iso) {
        set_ntsc(n);
    }
}
