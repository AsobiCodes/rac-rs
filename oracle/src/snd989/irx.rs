//! Read-only view of the PLAYER'S 989snd.irx (an IOP ELF32 inside the disc's globals file `irx.bin`).
//! Nothing of the module is embedded in this repo: tables the driver needs (R250 table, later the sine and pan
//! tables, libsd pitch tables) are read from the bytes the caller got from the player's own disc.

use std::collections::HashMap;

/// IOP module names (from the module header) of the sound driver and of libsd inside the globals `irx.bin`.
pub const MODULE_989SND: &str = "989snd_Library";
pub const MODULE_LIBSD: &str = "Sound_Device_Library";

const PT_LOAD: u32 = 1;
const PT_IOPMOD: u32 = 0x7000_0080;
/// `.data` anchor for finding the module in an IOP RAM image: the 2048-entry s16 sine table (relocation-free data).
const SINE_TABLE: u32 = 0x17d60;

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    vaddr: u32,
    off: u32,
    filesz: u32,
    memsz: u32,
}

/// An IOP module (ELF32 little-endian MIPS) addressed by module virtual address (module base 0).
pub struct Irx<'a> {
    elf: &'a [u8],
    segs: Vec<Seg>,
    name: Option<&'a str>,
}

impl<'a> Irx<'a> {
    /// Parse the ELF header and program headers (PT_LOAD for the address map, the IOP module header for the name).
    pub fn parse(elf: &'a [u8]) -> Result<Irx<'a>, String> {
        if elf.get(0..4) != Some(b"\x7fELF") {
            return Err("not an ELF file".into());
        }
        if elf.get(4) != Some(&1) || elf.get(5) != Some(&1) {
            return Err("not a 32-bit little-endian ELF".into());
        }
        let phoff = u32le(elf, 0x1c).ok_or("short ELF")? as usize;
        let phentsize = u16le(elf, 0x2a).ok_or("short ELF")? as usize;
        let phnum = u16le(elf, 0x2c).ok_or("short ELF")? as usize;
        if phentsize < 32 {
            return Err("bad program header size".into());
        }
        let mut segs = Vec::new();
        let mut name = None;
        for i in 0..phnum {
            let o = phoff + i * phentsize;
            let f = |k: usize| u32le(elf, o + k * 4).ok_or_else(|| "short ELF program header".to_string());
            let (ty, off, vaddr, filesz, memsz) = (f(0)?, f(1)?, f(2)?, f(4)?, f(5)?);
            if ty == PT_LOAD {
                if off as usize + filesz as usize > elf.len() {
                    return Err("PT_LOAD segment runs past the end of the file".into());
                }
                segs.push(Seg { vaddr, off, filesz, memsz });
            } else if ty == PT_IOPMOD {
                // IOP module header: modinfo, entry, gp, text/data/bss sizes, u16 version, then the NUL-terminated name
                let start = off as usize + 0x1a;
                let end = elf.get(start..).and_then(|s| s.iter().position(|&c| c == 0)).map(|n| start + n);
                name = end.and_then(|e| std::str::from_utf8(&elf[start..e]).ok());
            }
        }
        if segs.is_empty() {
            return Err("no PT_LOAD segment".into());
        }
        Ok(Irx { elf, segs, name })
    }

    /// The module name from the IOP module header ("989snd_Library").
    pub fn module_name(&self) -> Option<&str> {
        self.name
    }

    /// File-backed bytes at module address `addr`.
    pub fn bytes(&self, addr: u32, len: usize) -> Option<&'a [u8]> {
        let s = self.segs.iter().find(|s| addr >= s.vaddr && (addr as u64 + len as u64) <= s.vaddr as u64 + s.filesz as u64)?;
        let o = (s.off + (addr - s.vaddr)) as usize;
        self.elf.get(o..o + len)
    }

    pub fn u8_at(&self, addr: u32) -> Option<u8> {
        self.read_zero_filled(addr, 1).map(|v| v as u8)
    }

    pub fn u16_at(&self, addr: u32) -> Option<u16> {
        self.read_zero_filled(addr, 2).map(|v| v as u16)
    }

    pub fn u32_at(&self, addr: u32) -> Option<u32> {
        self.read_zero_filled(addr, 4)
    }

    /// Little-endian read of 1/2/4 bytes; the part beyond a segment's file size (.bss) reads as zero.
    fn read_zero_filled(&self, addr: u32, len: u32) -> Option<u32> {
        let mut v = 0u32;
        for k in 0..len {
            let a = addr + k;
            let s = self.segs.iter().find(|s| a >= s.vaddr && a < s.vaddr + s.memsz)?;
            let b = if a - s.vaddr < s.filesz { *self.elf.get((s.off + (a - s.vaddr)) as usize)? } else { 0 };
            v |= (b as u32) << (8 * k);
        }
        Some(v)
    }

    /// The module with name `name` inside a blob of concatenated IRX files (the disc's globals `irx.bin`).
    /// Returns the bytes from its ELF header to the next ELF header (or the end of the blob).
    pub fn find_in_blob(blob: &'a [u8], name: &str) -> Option<&'a [u8]> {
        let starts: Vec<usize> = blob.windows(4).enumerate().filter(|(_, w)| *w == b"\x7fELF").map(|(i, _)| i).collect();
        for (k, &s) in starts.iter().enumerate() {
            let end = starts.get(k + 1).copied().unwrap_or(blob.len());
            if let Ok(irx) = Irx::parse(&blob[s..end]) {
                if irx.module_name() == Some(name) {
                    return Some(&blob[s..end]);
                }
            }
        }
        None
    }

    /// Load address of this module in an IOP RAM image (module addresses are relative to the base): found through the
    /// sine table (`.data`, no relocations) confirmed by the start of `.text`. None when absent or ambiguous.
    pub fn find_base_in_ram(&self, ram: &[u8]) -> Option<u32> {
        let pat = self.bytes(SINE_TABLE, 512)?;
        let head = self.bytes(0, 32)?;
        let mut found = None;
        let mut from = 0;
        while let Some(p) = ram.get(from..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)) {
            let pos = from + p;
            // a second copy of the table can sit elsewhere in RAM (s07: 0xd5290): the real module also starts with its .text
            let base = pos.wrapping_sub(SINE_TABLE as usize);
            if pos >= SINE_TABLE as usize && ram.get(base..base + head.len()) == Some(head) {
                if found.is_some() {
                    return None;
                }
                found = Some((pos - SINE_TABLE as usize) as u32);
            }
            from = pos + 1;
        }
        found
    }
}

/// Convenience for callers that keep several modules: name -> ELF bytes of a blob.
pub fn modules_in_blob(blob: &[u8]) -> HashMap<String, &[u8]> {
    let starts: Vec<usize> = blob.windows(4).enumerate().filter(|(_, w)| *w == b"\x7fELF").map(|(i, _)| i).collect();
    let mut out = HashMap::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(blob.len());
        if let Ok(irx) = Irx::parse(&blob[s..end]) {
            if let Some(n) = irx.module_name() {
                out.insert(n.to_string(), &blob[s..end]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snd989::testdata;

    #[test]
    fn finds_989snd_in_the_globals_blob() {
        let Some(blob) = testdata::read("globals/irx.bin") else {
            eprintln!("skip: extracted/globals/irx.bin absent");
            return;
        };
        let elf = Irx::find_in_blob(&blob, MODULE_989SND).expect("989snd module in irx.bin");
        let irx = Irx::parse(elf).unwrap();
        assert_eq!(irx.module_name(), Some(MODULE_989SND));
        // module layout from the note: .text 0..0x175d0, .rodata, .data 0x17ba0.., .bss to 0x19900+
        assert!(irx.bytes(0, 4).is_some());
        assert!(irx.u32_at(0x18f40).is_some()); // R250 index word
        assert_eq!(irx.u32_at(0x18f40), Some(0)); // initial index 0
        assert!(irx.u32_at(0x19000).is_some()); // .bss reads as zero
        assert_eq!(irx.bytes(0x19900, 4), None); // not file-backed
        assert!(modules_in_blob(&blob).contains_key(MODULE_989SND));
        let libsd = Irx::parse(Irx::find_in_blob(&blob, MODULE_LIBSD).expect("libsd")).unwrap();
        assert_eq!(libsd.module_name(), Some(MODULE_LIBSD));
        assert!(Irx::find_in_blob(&blob, "no_such_module").is_none());
    }
}
