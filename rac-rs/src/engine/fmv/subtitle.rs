// SPDX-License-Identifier: GPL-3.0-only
//! Timed, premultiplied RGBA overlays rendered offline from the approved ASS with libass.

pub(super) const WIDTH: u32 = 512;
pub(super) const HEIGHT: u32 = 416;
const RGBA_LEN: usize = WIDTH as usize * HEIGHT as usize * 4;

struct Cue {
    start: u32,
    end: u32,
    payload: std::ops::Range<usize>,
}

pub(super) struct Track {
    pub lba: u32,
    cues: Vec<Cue>,
    bytes: Vec<u8>,
}

impl Track {
    pub fn open(lba: u32) -> Option<Self> {
        let pack = crate::localization::active_pack()?;
        let dir = &pack.root;
        let custom = dir.join("movie-subtitles-custom");
        let track = std::fs::read_to_string(custom.join("current.txt"))
            .ok()
            .and_then(|id| {
                let id = id.trim();
                (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
                    .then(|| Self::from_folder(lba, &custom.join(id)))
                    .flatten()
            })
            .or_else(|| {
                Self::from_folder(lba, &pack.file("movie_subtitles", "movie-subtitles")?)
            })?;
        eprintln!("fmv subtitles: LBA {lba}, {} intervals", track.cues.len());
        Some(track)
    }

    fn from_folder(lba: u32, folder: &std::path::Path) -> Option<Self> {
        use crate::omni::json;
        let map = std::fs::read_to_string(folder.join("map.json")).ok()?;
        let map = json::parse(&map).ok()?;
        let name = map.get(&lba.to_string())?.str()?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || !name.ends_with(".bin")
        {
            return None;
        }
        let file = folder.join(name);
        if file.metadata().ok()?.len() > 8 * 1024 * 1024 {
            return None;
        }
        let bytes = std::fs::read(file).ok()?;
        Self::parse(lba, bytes)
    }

    fn parse(lba: u32, bytes: Vec<u8>) -> Option<Self> {
        if !bytes.starts_with(b"RCSUB001") {
            return None;
        }
        let mut at = 8usize;
        let mut previous = 0;
        let mut cues = Vec::new();
        while at < bytes.len() {
            let field = |i| {
                bytes
                    .get(i..i + 4)
                    .and_then(|b| b.try_into().ok())
                    .map(u32::from_le_bytes)
            };
            let (start, end, len) = (field(at)?, field(at + 4)?, field(at + 8)? as usize);
            at += 12;
            if start < previous || end <= start || len == 0 || len > bytes.len() - at {
                return None;
            }
            cues.push(Cue {
                start,
                end,
                payload: at..at + len,
            });
            at += len;
            previous = end;
        }
        Some(Self { lba, cues, bytes })
    }

    pub fn cue(&self, ms: u64) -> Option<usize> {
        let i = self.cues.partition_point(|cue| u64::from(cue.end) <= ms);
        self.cues
            .get(i)
            .filter(|cue| u64::from(cue.start) <= ms)
            .map(|_| i)
    }

    pub fn rgba(&self, cue: usize) -> Option<Vec<u8>> {
        let bytes = &self.bytes[self.cues.get(cue)?.payload.clone()];
        crate::texpack::inflate::zlib(bytes, RGBA_LEN)
            .ok()
            .filter(|rgba| rgba.len() == RGBA_LEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(intervals: &[(u32, u32)]) -> Vec<u8> {
        let mut b = b"RCSUB001".to_vec();
        for &(start, end) in intervals {
            b.extend_from_slice(&start.to_le_bytes());
            b.extend_from_slice(&end.to_le_bytes());
            b.extend_from_slice(&1u32.to_le_bytes());
            b.push(0);
        }
        b
    }
    #[test]
    fn intervals_use_the_movie_clock_and_exclude_the_end() {
        let t = Track::parse(1, track(&[(10, 20), (30, 40)])).unwrap();
        assert_eq!(t.cue(9), None);
        assert_eq!(t.cue(10), Some(0));
        assert_eq!(t.cue(19), Some(0));
        assert_eq!(t.cue(20), None);
        assert_eq!(t.cue(30), Some(1));
        assert_eq!(t.cue(40), None);
        assert!(t.rgba(0).is_none()); // Malformed compression cannot reach GPU upload.
    }
    #[test]
    fn refuses_overlap_empty_reversed_and_truncated_records() {
        for intervals in [&[(10, 20), (19, 30)][..], &[(10, 10)][..], &[(20, 10)][..]] {
            assert!(Track::parse(1, track(intervals)).is_none());
        }
        let mut b = track(&[(10, 20)]);
        b.pop();
        assert!(Track::parse(1, b).is_none());
        assert!(Track::parse(1, b"RCSUB002".to_vec()).is_none());
    }
}
