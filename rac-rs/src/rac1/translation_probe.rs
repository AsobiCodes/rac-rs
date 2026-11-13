// SPDX-License-Identifier: GPL-3.0-only
//! Font compatibility for an active localization mod. Text is replaced by sector overlays, not RAM string scans.
use std::sync::Mutex;
#[path = "translation_assets.rs"]
mod assets;
#[path = "locale_registry.rs"]
pub(crate) mod locale_registry;
pub(crate) use assets::{
    accent_index, rgba as patch_texture_rgba, table_bytes as patch_font_table,
    unicode as localized_char, words as patch_texture_words,
};

const DISCOVERY_BYTES: usize = 256 * 1024;
const LOAD_SETTLE_FIELDS: u32 = 64;
#[derive(Default)]
struct State {
    machine: usize,
    context: u64,
    last_field: u32,
    ready: u32,
    cursor: usize,
    positions: Vec<(usize, usize)>,
    steps: usize,
    max_ns: u128,
}
fn find(data: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > data.len() {
        return Vec::new();
    }
    let n = needle.len();
    let mut skip = [n; 256];
    for (i, &b) in needle[..n - 1].iter().enumerate() {
        skip[b as usize] = n - 1 - i;
    }
    let mut found = Vec::new();
    let mut at = 0;
    while at <= data.len() - n {
        if data[at + n - 1] == needle[n - 1] && data[at..at + n] == *needle {
            found.push(at);
            at += 1
        } else {
            at += skip[data[at + n - 1] as usize]
        }
    }
    found
}
impl State {
    fn step(&mut self, ram: &mut [u8], tables: &[assets::Table], field: u32, context: u64) {
        let machine = ram.as_ptr() as usize;
        if self.machine != machine || self.context != context || field < self.last_field {
            *self = Self {
                machine,
                context,
                ready: field.saturating_add(LOAD_SETTLE_FIELDS),
                ..Self::default()
            };
        }
        self.last_field = field;
        for &(i, at) in &self.positions {
            let t = &tables[i];
            if ram.get(at..at + t.old.len()) == Some(t.old.as_slice()) {
                ram[at..at + t.new.len()].copy_from_slice(&t.new);
            }
        }
        if field < self.ready || self.cursor >= ram.len() {
            return;
        }
        let began = std::time::Instant::now();
        let start = self.cursor;
        let end = (start + DISCOVERY_BYTES).min(ram.len());
        for (i, t) in tables.iter().enumerate() {
            let hi = end
                .saturating_add(t.old.len().saturating_sub(1))
                .min(ram.len());
            for offset in find(&ram[start..hi], &t.old) {
                let at = start + offset;
                if at >= end {
                    continue;
                }
                ram[at..at + t.new.len()].copy_from_slice(&t.new);
                self.positions.push((i, at));
            }
        }
        self.cursor = end;
        self.steps += 1;
        self.max_ns = self.max_ns.max(began.elapsed().as_nanos());
        if end == ram.len() {
            eprintln!(
                "locale fonts: context {context:#x}, {} tables, {} finite steps, max {:.3} ms; text supplied by disc overlay",
                self.positions.len(),
                self.steps,
                self.max_ns as f64 / 1e6
            );
        }
    }
}
pub(crate) fn patch_ram(ram: &mut [u8], field: u32, context: u64) {
    let tables = assets::tables();
    if tables.is_empty() {
        return;
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    if let Ok(mut state) = STATE.lock() {
        state
            .get_or_insert_with(State::default)
            .step(ram, tables, field, context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_font_discovery_crosses_boundaries_and_never_recurs_on_a_timer() {
        let tables = [assets::Table {
            old: b"original descriptor".to_vec(),
            new: b"replaced descriptor".to_vec(),
        }];
        let mut ram = vec![0; DISCOVERY_BYTES * 2 + 100];
        let at = DISCOVERY_BYTES - 5;
        ram[at..at + 19].copy_from_slice(&tables[0].old);
        let mut state = State::default();
        for field in 0..1000 {
            state.step(&mut ram, &tables, field, 1);
        }
        assert_eq!(state.steps, 3);
        assert_eq!(&ram[at..at + 19], tables[0].new);
        ram[at..at + 19].copy_from_slice(&tables[0].old);
        state.step(&mut ram, &tables, 1100, 1);
        assert_eq!(&ram[at..at + 19], tables[0].new);
        ram[at..at + 19].fill(9);
        state.step(&mut ram, &tables, 1101, 1);
        assert_eq!(&ram[at..at + 19], &[9; 19]);
        assert_eq!(state.steps, 3);
    }
    #[test]
    fn search_matches_a_naive_search() {
        for data in [b"aaaaaaa".as_slice(), b"", b"abc"] {
            for needle in [b"aa".as_slice(), b"abc", b"X"] {
                assert_eq!(
                    find(data, needle),
                    data.windows(needle.len())
                        .enumerate()
                        .filter_map(|(i, b)| (b == needle).then_some(i))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
