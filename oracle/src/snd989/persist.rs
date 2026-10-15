//! Byte writer / reader for the driver's savestate section (little endian, no framing). No oracle types: `Snd989::save`
//! wraps the bytes into snapfile's `Wr`. A reader panics on a truncated section like snapfile's `Rd`.
#[derive(Default)]
pub struct Out(pub Vec<u8>);

impl Out {
    pub fn new() -> Out {
        Out(Vec::new())
    }
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn i8(&mut self, v: i8) {
        self.0.push(v as u8);
    }
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A length or index (always 32 bit in the file).
    pub fn len(&mut self, v: usize) {
        self.u32(v as u32);
    }
    /// `None` = u32::MAX.
    pub fn opt_usize(&mut self, v: Option<usize>) {
        self.u32(v.map_or(u32::MAX, |x| x as u32));
    }
    pub fn opt_u32(&mut self, v: Option<u32>) {
        self.bool(v.is_some());
        self.u32(v.unwrap_or(0));
    }
    pub fn mask(&mut self, m: &[u32; 2]) {
        self.u32(m[0]);
        self.u32(m[1]);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.len(b.len());
        self.0.extend_from_slice(b);
    }
}

pub struct In<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> In<'a> {
    pub fn new(b: &'a [u8]) -> In<'a> {
        In { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        s
    }
    pub fn done(&self) -> bool {
        self.p == self.b.len()
    }
    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    pub fn i8(&mut self) -> i8 {
        self.u8() as i8
    }
    pub fn bool(&mut self) -> bool {
        self.u8() != 0
    }
    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    pub fn i16(&mut self) -> i16 {
        self.u16() as i16
    }
    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    pub fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    pub fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    pub fn len(&mut self) -> usize {
        self.u32() as usize
    }
    pub fn opt_usize(&mut self) -> Option<usize> {
        let v = self.u32();
        (v != u32::MAX).then_some(v as usize)
    }
    pub fn opt_u32(&mut self) -> Option<u32> {
        let some = self.bool();
        let v = self.u32();
        some.then_some(v)
    }
    pub fn mask(&mut self) -> [u32; 2] {
        [self.u32(), self.u32()]
    }
    pub fn bytes(&mut self) -> &'a [u8] {
        let n = self.len();
        self.take(n)
    }
}
