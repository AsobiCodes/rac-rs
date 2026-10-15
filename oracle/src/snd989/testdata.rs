//! Test-only access to the extracted disc files and the oracle savestates. Every test that needs them skips
//! (returns early with a printed note) when they are absent, so `cargo test` also passes on a machine without the game.
use std::path::PathBuf;

fn first_existing(cands: &[Option<String>]) -> Option<PathBuf> {
    cands.iter().flatten().map(PathBuf::from).find(|p| p.is_dir())
}

/// `RAC_EXTRACTED`, else ../extracted next to the crate.
pub fn extracted() -> Option<PathBuf> {
    first_existing(&[
        std::env::var("RAC_EXTRACTED").ok(),
        Some(format!("{}/../extracted", env!("CARGO_MANIFEST_DIR"))),
    ])
}

/// `RAC_STATES`, else states/ in the crate.
pub fn states() -> Option<PathBuf> {
    first_existing(&[std::env::var("RAC_STATES").ok(), Some(concat!(env!("CARGO_MANIFEST_DIR"), "/states").into())])
}

pub fn read(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(extracted()?.join(rel)).ok()
}

pub fn read_state(rel: &str) -> Option<Vec<u8>> {
    std::fs::read(states()?.join(rel)).ok()
}

/// The global bank (extracted/globals/sound_bank.bin) and the level banks (extracted/levels/N_name/sound_bank.bin),
/// in level-number order. Empty when the extracted files are absent.
pub fn all_banks() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let Some(root) = extracted() else { return out };
    if let Ok(b) = std::fs::read(root.join("globals/sound_bank.bin")) {
        out.push(("global".to_string(), b));
    }
    let mut dirs: Vec<(u32, PathBuf)> = std::fs::read_dir(root.join("levels"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let n: u32 = name.split('_').next()?.parse().ok()?;
                    Some((n, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    for (n, p) in dirs {
        if let Ok(b) = std::fs::read(p.join("sound_bank.bin")) {
            out.push((format!("level{n}"), b));
        }
    }
    out
}
