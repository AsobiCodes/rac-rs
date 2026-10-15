//! The driver RNG: R250 lag generator FUN_0000ad14 (989snd 0xad14) over a u16[250] table that is pre-filled in the
//! module's .data and never reseeded (the seeder 0xabe0 has no caller). One shared stream, advanced by every consumer in
//! the order they run. Note: decomp/notes/sound_grains.md "RNG". The table comes from the player's disc IRX at runtime
//! (`from_irx`) or from a savestate's IOP RAM (`from_iop`); it is never embedded in source.

use super::irx::Irx;

/// Module address of the u16[250] table and of the u32 index word.
pub const TABLE_ADDR: u32 = 0x18f44;
pub const INDEX_ADDR: u32 = 0x18f40;
pub const LEN: usize = 250;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng250 {
    t: [u16; LEN],
    i: usize,
}

impl Rng250 {
    pub fn from_state(t: [u16; LEN], i: usize) -> Result<Rng250, String> {
        if i >= LEN {
            return Err(format!("R250 index {i} out of range"));
        }
        Ok(Rng250 { t, i })
    }

    /// Initial state: the table and index in the IRX's .data.
    pub fn from_irx(irx: &Irx) -> Result<Rng250, String> {
        let raw = irx.bytes(TABLE_ADDR, LEN * 2).ok_or("R250 table not in the IRX image")?;
        let i = irx.u32_at(INDEX_ADDR).ok_or("R250 index not in the IRX image")? as usize;
        Rng250::from_state(decode(raw), i)
    }

    /// Continue exactly where a savestate's IOP RAM is: `base` = the module's load address
    /// (`Irx::find_base_in_ram`), table at base + 0x18f44, index word at base + 0x18f40.
    pub fn from_iop(iop: &[u8], base: u32) -> Result<Rng250, String> {
        let t = (base + TABLE_ADDR) as usize;
        let raw = iop.get(t..t + LEN * 2).ok_or("R250 table outside the IOP RAM image")?;
        let ix = (base + INDEX_ADDR) as usize;
        let i = u32::from_le_bytes(iop.get(ix..ix + 4).ok_or("R250 index outside the IOP RAM image")?.try_into().unwrap()) as usize;
        Rng250::from_state(decode(raw), i)
    }

    /// Write the state back (savestate save): table bytes and index word at the same module addresses.
    pub fn to_iop(&self, iop: &mut [u8], base: u32) -> Result<(), String> {
        let t = (base + TABLE_ADDR) as usize;
        let ix = (base + INDEX_ADDR) as usize;
        if t + LEN * 2 > iop.len() || ix + 4 > iop.len() {
            return Err("R250 state outside the IOP RAM image".into());
        }
        for (k, v) in self.t.iter().enumerate() {
            iop[t + 2 * k..t + 2 * k + 2].copy_from_slice(&v.to_le_bytes());
        }
        iop[ix..ix + 4].copy_from_slice(&(self.i as u32).to_le_bytes());
        Ok(())
    }

    pub fn state(&self) -> (&[u16; LEN], usize) {
        (&self.t, self.i)
    }

    /// FUN_0000ad14: j = i < 0x93 ? i + 0x67 : i - 0x93; v = t[i] ^ t[j]; t[i] = v; i = i > 0xf8 ? 0 : i + 1; return v.
    pub fn next(&mut self) -> u16 {
        let i = self.i;
        let j = if i < 0x93 { i + 0x67 } else { i - 0x93 };
        let v = self.t[i] ^ self.t[j];
        self.t[i] = v;
        self.i = if i > 0xf8 { 0 } else { i + 1 };
        v
    }

    /// `(rand & 0xffff) % n` with C's signed `%` of a positive count: the form every consumer uses (grains 25, 26, 31,
    /// 37, the random-pan / random-volume operands, ...). `n` must be positive.
    pub fn next_mod(&mut self, n: i32) -> i32 {
        debug_assert!(n > 0);
        (self.next() as i32) % n
    }
}

fn decode(raw: &[u8]) -> [u16; LEN] {
    let mut t = [0u16; LEN];
    for (k, c) in raw.chunks_exact(2).enumerate() {
        t[k] = u16::from_le_bytes([c[0], c[1]]);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snd989::testdata;

    fn irx_bytes() -> Option<Vec<u8>> {
        let blob = testdata::read("globals/irx.bin")?;
        Some(Irx::find_in_blob(&blob, crate::snd989::irx::MODULE_989SND)?.to_vec())
    }

    #[test]
    fn first_six_outputs() {
        let Some(elf) = irx_bytes() else {
            eprintln!("skip: extracted/globals/irx.bin absent");
            return;
        };
        let irx = Irx::parse(&elf).unwrap();
        let mut r = Rng250::from_irx(&irx).unwrap();
        let got: Vec<u16> = (0..6).map(|_| r.next()).collect();
        assert_eq!(got, [1173, 53403, 52352, 35341, 27497, 54011]);
    }

    #[test]
    fn index_wraps_and_modulo() {
        // synthetic table: checks the recurrence (j offsets, wrap at index 249) without any game data
        let mut t = [0u16; LEN];
        for (k, v) in t.iter_mut().enumerate() {
            *v = (k as u16).wrapping_mul(2654).wrapping_add(17);
        }
        let mut r = Rng250::from_state(t, 0).unwrap();
        let mut reference = t;
        let mut i = 0usize;
        for _ in 0..1000 {
            let j = if i < 0x93 { i + 0x67 } else { i - 0x93 };
            let v = reference[i] ^ reference[j];
            reference[i] = v;
            i = if i == LEN - 1 { 0 } else { i + 1 };
            assert_eq!(r.next(), v);
        }
        assert_eq!(r.state().1, 1000 % LEN);
        let mut a = Rng250::from_state(t, 3).unwrap();
        let mut b = a.clone();
        assert_eq!(a.next_mod(7), (b.next() as i32) % 7);
        assert!(Rng250::from_state(t, LEN).is_err());
    }

    #[test]
    fn savestate_round_trip_and_adoption() {
        let Some(elf) = irx_bytes() else {
            eprintln!("skip: extracted/globals/irx.bin absent");
            return;
        };
        let irx = Irx::parse(&elf).unwrap();
        // write a drawn state into a fake IOP RAM image and read it back
        let mut ram = vec![0u8; 0x20_0000];
        let base = 0x9_9fa0u32;
        let mut r = Rng250::from_irx(&irx).unwrap();
        for _ in 0..777 {
            r.next();
        }
        r.to_iop(&mut ram, base).unwrap();
        let mut back = Rng250::from_iop(&ram, base).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.next(), r.next());

        // real savestate: the module is found through its sine table; its RNG is still the initial state (no draw yet)
        let Some(iop) = testdata::read_state("s07/iopMemory.bin") else {
            eprintln!("skip: oracle/states/s07 absent");
            return;
        };
        let found = irx.find_base_in_ram(&iop).expect("989snd module in the s07 IOP RAM");
        assert_eq!(found, 0x9_9fa0);
        let adopted = Rng250::from_iop(&iop, found).unwrap();
        assert_eq!(adopted, Rng250::from_irx(&irx).unwrap());
    }
}
