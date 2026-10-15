//! Build tag: ties an executable to the exact hand sources it was built from (hand/auto/common.py `src_hash`).
//!
//! Every pipeline build (setup.py, check.py --build, integrate.py, latent_refresh.py, sweep.py, drop_and_build.py) hashes
//! `src/**/*.rs` first and passes the 16 hex digits in the environment variable EEHAND_SRC_HASH. `option_env!` makes rustc
//! record the variable in its dep-info, so cargo recompiles this crate whenever the hash changes, even when a source file was
//! put back with an older modification time (cargo's own mtime test then says Fresh and keeps the stale code). The tag ends up
//! in every executable that calls `register()`: `common.py exe_tag()` reads it back from the file, and the checkers refuse a
//! verdict from an executable whose tag is not the hash of the sources under test. A build without the variable carries the
//! all-zero tag, which is never trusted.

/// "EEHAND-SRC:" and the 16 hex digits.
pub static SRC_TAG: [u8; 27] = tag();

const fn tag() -> [u8; 27] {
    let hash: &[u8] = match option_env!("EEHAND_SRC_HASH") {
        Some(h) => h.as_bytes(),
        None => b"0000000000000000",
    };
    let mut t = *b"EEHAND-SRC:0000000000000000";
    let mut i = 0;
    while i < 16 && i < hash.len() {
        t[11 + i] = hash[i];
        i += 1;
    }
    t
}

/// Keeps the tag in the linked executable (called by `register()`).
pub fn touch() {
    std::hint::black_box(&SRC_TAG);
}
