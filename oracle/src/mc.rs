//! Memory card server: the IOP side of the EE's libmc (boot 0x1233f0..0x124400, SIF RPC client 0x159a00 bound to sid
//! 0x80000400). The IOP is not emulated, so the card file system mcman keeps and the replies mcserv sends are made here,
//! from a PCSX2 memory card image (`.ps2`, 528-byte raw pages): ORACLE_MCD (port 0) / ORACLE_MCD2 (port 1), else
//! oracle/memcards/Mcd00N.ps2 (project-local copies, $ORACLE_MCD_DIR overrides the folder; never PCSX2's own memory card folder); no image = no
//! card in that port. The images are read at the first card RPC; writes change only the copy in memory and the
//! savestate (the image file is never written). fndiff's checkpoint-cache key hashes the content of the images
//! `image_path` names.
//!
//! Without this the RPC reply word (0x15afc0, `sceMcSync`'s result) kept whatever value the savestate held, so a save
//! "wrote" 0 bytes: the game's card state machine (Eudora 0x23dc10.., error 0x13d374) then reports a failed save and
//! opens the mode-4 message screen (FUN_001f4840), where PCSX2 (real mcman on the card) saved normally.
//!
//! Replies, as the libmc wrappers send them (send struct 0x159a80 / 0x159ab0, reply word = recv[0]):
//!   1 GetInfo (+4 port, +8 slot, +0x1c rdata): 0 (the card the game already saw), rdata +0 type 2, +4 free clusters,
//!     +0x90 format 1; the end callback 0x123d10 copies them to the pointers libmc saved at 0x159a28/2c/30.
//!     No card in the port: -10, type/free/format 0. An unformatted card: 0, type 2, free 0, format 0, and the file
//!     functions return -2.
//!   2 Open (+0 port, +4 slot, +8 flags, +0x14 path): fd (lowest free of 3), or -4 no entry (flag 0x200 creates the
//!     file, 0x400 truncates it), -5 a directory, -3 no space, -7 no free fd; flags 0x40 = Mkdir (libmc 0x1236d8):
//!     0, or -4 (exists / no parent). [C] fd 0 and the reply of a save (Eudora states: 0x13d360 = 0, 0x15afc0 = 0
//!     after close); the error codes of the paths the game has not taken here are libmc's documented ones [I].
//!   3 Close (+0 fd): 0.   4 Seek (+0 fd, +0x10 offset, +0x14 whence): new position.
//!   5 Read (+0 fd, +0xc size, +0x18 buf, +0x1c rdata): bytes read; the 64-byte aligned middle goes straight to buf,
//!     the head and tail go through rdata {+0 head len, +4 tail len, +8 head dst, +0xc tail dst, +0x10 head, +0x50
//!     tail}, which the end callback 0x1238a0 copies ([C] layout from the callback; [I] the 64-byte split: only the
//!     buffer contents are certain, no read happens on the levels run so far).
//!   6 Write (+0 fd, +0x14 head len, +0x20 head bytes, +0x18 src, +0xc size after the head): bytes written.
//!   0xa Flush: 0.   0xd GetDir (+8 flags 0 = new listing, +0xc max entries, +0x10 table, +0x14 pattern): entries
//!     (64-byte sceMcTblGetDir records: created, modified, size, mode, attr, name [I]).   0xf Delete (+0x14 path):
//!     0 / -4 / -6 (directory not empty). New and written files take the host's local time (clock.rs) in created and
//!     modified, and a written file's directory takes it in modified [I].
//! Other functions stop the run (none is used by the game's card state machine on the levels run so far).
//! The free count is mcman's: free FAT entries among the first `max_allocatable_clusters` clusters (IOP mcman
//! 0x716b8..0x71784: (clusters_per_card / 1000) * 1000 + 1 clusters from alloc_offset, skipping bad blocks, at most
//! up to alloc_end; 8001 on an 8 MB card, which gives the 7645 the Eudora states hold at 0x13d29c).
use crate::cpu::Cpu;

/// IOP server struct address of sid 0x80000400 (mcserv) in the retail IOP RAM; `sif_bind_rpc` binds to it.
pub const MCSERV: u32 = 0x5b148;
/// libmc's GetInfo end callback and its saved destination pointers (type, free, format).
const CB_GETINFO: u32 = 0x123d10;
const GETINFO_PTRS: [(u32, u32); 3] = [(0x159a28, 0), (0x159a2c, 4), (0x159a30, 0x90)];
/// libmc's Chdir end callback (copies the current directory string at 0x159fc0 to the caller's buffer, when it gave one).
const CB_CHDIR: u32 = 0x124038;
const CHDIR_DIR: u32 = 0x159fc0;
/// libmc's Read end callback (copies the unaligned head and tail from rdata).
const CB_READ: u32 = 0x1238a0;
const MAX_FDS: usize = 3;
const M_EXISTS: u16 = 0x8000;
const M_DIR: u16 = 0x0020;
/// Mode of a file / directory mcman creates (read, write, exec, file or dir, exists).
const M_NEW_FILE: u16 = 0x8497;
const M_NEW_DIR: u16 = 0x8427;

#[derive(Clone)]
pub struct Ent {
    pub name: String,
    pub mode: u16,
    /// Files: byte length (data.len()); directories: entry count as on the card ("." and ".." keep theirs).
    pub len: u32,
    pub created: [u8; 8],
    pub modified: [u8; 8],
    pub attr: u32,
    pub data: Vec<u8>,
    pub kids: Vec<Ent>,
}

impl Ent {
    fn is_dir(&self) -> bool {
        self.mode & M_DIR != 0
    }
    fn live(&self) -> bool {
        self.mode & M_EXISTS != 0
    }
    fn dot(&self) -> bool {
        self.name == "." || self.name == ".."
    }
    fn new(name: &str, dir: bool, now: [u8; 8]) -> Ent {
        let mut e = Ent { name: name.into(), mode: if dir { M_NEW_DIR } else { M_NEW_FILE }, len: 0, created: now, modified: now, attr: 0, data: Vec::new(), kids: Vec::new() };
        if dir {
            let mut dot = e.clone();
            dot.name = ".".into();
            let mut dotdot = e.clone();
            dotdot.name = "..".into();
            dotdot.mode = 0xa426;
            e.kids = vec![dot, dotdot];
            e.len = 2;
        }
        e
    }
    fn save(&self, w: &mut crate::snapfile::Wr) {
        w.str(&self.name);
        w.u32(self.mode as u32);
        w.u32(self.len);
        w.0.extend_from_slice(&self.created);
        w.0.extend_from_slice(&self.modified);
        w.u32(self.attr);
        w.bytes(&self.data);
        w.u64(self.kids.len() as u64);
        for k in &self.kids {
            k.save(w);
        }
    }
    fn load(r: &mut crate::snapfile::Rd) -> Ent {
        let name = r.str();
        let mode = r.u32() as u16;
        let len = r.u32();
        let mut created = [0u8; 8];
        let mut modified = [0u8; 8];
        for b in created.iter_mut() {
            *b = r.u8();
        }
        for b in modified.iter_mut() {
            *b = r.u8();
        }
        let attr = r.u32();
        let data = r.bytes().to_vec();
        let n = r.u64() as usize;
        let kids = (0..n).map(|_| Ent::load(r)).collect();
        Ent { name, mode, len, created, modified, attr, data, kids }
    }
}

#[derive(Clone)]
pub struct Card {
    pub root: Ent,
    /// Free clusters as mcman counts them (GetInfo).
    pub free: u32,
    pub cluster: u32,
    /// The superblock holds the format magic (an image of an unformatted card, e.g. all 0xff, does not).
    pub formatted: bool,
}

impl Card {
    /// Parse a PCSX2 memory card image (raw 528-byte pages, or plain 512-byte pages).
    pub fn parse(img: &[u8]) -> Result<Card, String> {
        let ps = if img.len() % 528 == 0 { 528 } else { 512 };
        let page = |n: usize| -> Result<&[u8], String> { img.get(n * ps..n * ps + 512).ok_or_else(|| format!("page {n} outside the image")) };
        let sb = page(0)?;
        if &sb[..28] != b"Sony PS2 Memory Card Format " {
            return Ok(Card { root: Ent::new(".", true, [0; 8]), free: 0, cluster: 1024, formatted: false });
        }
        let h16 = |o: usize| u16::from_le_bytes(sb[o..o + 2].try_into().unwrap()) as usize;
        let w32 = |o: usize| u32::from_le_bytes(sb[o..o + 4].try_into().unwrap());
        let (page_len, ppc, ppb) = (h16(0x28), h16(0x2a), h16(0x2c));
        let (cpc, aoff, aend, rootc) = (w32(0x30), w32(0x34), w32(0x38), w32(0x3c));
        if page_len != 512 || ppc == 0 || ppb < ppc {
            return Err(format!("unsupported card geometry page {page_len} x {ppc}"));
        }
        let csize = page_len * ppc;
        let cluster = |c: u32| -> Result<Vec<u8>, String> {
            let mut v = Vec::with_capacity(csize);
            for i in 0..ppc {
                v.extend_from_slice(page(c as usize * ppc + i)?);
            }
            Ok(v)
        };
        let per = csize / 4;
        let word = |b: &[u8], k: usize| u32::from_le_bytes(b[4 * k..4 * k + 4].try_into().unwrap());
        // FAT: indirect FAT clusters (superblock +0x50) -> FAT clusters -> entries (bit 31 allocated, 0x7fffffff end).
        let want = (aend as usize).div_ceil(per);
        let mut fat: Vec<u32> = Vec::new();
        'ifc: for i in 0..32 {
            let ic = w32(0x50 + 4 * i);
            if ic == 0 {
                break;
            }
            let icb = cluster(ic)?;
            for k in 0..per {
                if fat.len() >= want * per {
                    break 'ifc;
                }
                let fc = word(&icb, k);
                if fc == u32::MAX {
                    break 'ifc;
                }
                let fb = cluster(fc)?;
                fat.extend((0..per).map(|j| word(&fb, j)));
            }
        }
        // max_allocatable_clusters (mcman 0x71664..0x71784).
        let bad: Vec<u32> = (0..32).map(|i| w32(0xd0 + 4 * i)).collect();
        let cpb = (ppb / ppc) as u32;
        let t1 = (cpc / 1000) * 1000 + 1;
        let (mut a3, mut t0, mut ok) = (aoff, 0u32, true);
        while t0 < t1 && a3 < aend + aoff {
            if a3 % cpb == 0 || a3 == aoff {
                ok = !bad.contains(&(a3 / cpb));
            }
            if ok {
                t0 += 1;
            }
            a3 += 1;
        }
        let max_alloc = (a3 - aoff) as usize;
        if fat.len() < max_alloc {
            return Err(format!("FAT has {} entries, {max_alloc} needed", fat.len()));
        }
        let free = fat[..max_alloc].iter().filter(|&&v| v & 0x8000_0000 == 0).count() as u32;
        let read_chain = |c0: u32, n: usize| -> Result<Vec<u8>, String> {
            let mut out = Vec::new();
            let mut c = c0;
            while out.len() < n {
                out.extend_from_slice(&cluster(c + aoff)?);
                let v = *fat.get(c as usize).ok_or("cluster outside the FAT")?;
                if v & 0x8000_0000 == 0 || v & 0x7fff_ffff == 0x7fff_ffff {
                    break;
                }
                c = v & 0x7fff_ffff;
            }
            if out.len() < n {
                return Err(format!("chain at {c0} shorter than {n} bytes"));
            }
            out.truncate(n);
            Ok(out)
        };
        fn ent(raw: &[u8]) -> (Ent, u32) {
            let w = |o: usize| u32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
            let name = raw[0x40..0x60].split(|&b| b == 0).next().unwrap_or(&[]);
            let e = Ent {
                name: String::from_utf8_lossy(name).into_owned(),
                mode: u16::from_le_bytes([raw[0], raw[1]]),
                len: w(4),
                created: raw[8..16].try_into().unwrap(),
                modified: raw[0x18..0x20].try_into().unwrap(),
                attr: w(0x20),
                data: Vec::new(),
                kids: Vec::new(),
            };
            (e, w(0x10))
        }
        fn fill(e: &mut Ent, c: u32, depth: u32, rd: &dyn Fn(u32, usize) -> Result<Vec<u8>, String>) -> Result<(), String> {
            if !e.live() {
                return Ok(());
            }
            if e.is_dir() {
                if depth > 8 {
                    return Err("directory tree too deep".into());
                }
                let raw = rd(c, e.len as usize * 512)?;
                for k in 0..e.len as usize {
                    let (mut kid, kc) = ent(&raw[k * 512..k * 512 + 512]);
                    if !kid.dot() {
                        fill(&mut kid, kc, depth + 1, rd)?;
                    }
                    e.kids.push(kid);
                }
            } else if e.len > 0 {
                e.data = rd(c, e.len as usize)?;
            }
            Ok(())
        }
        let (mut root, _) = ent(&read_chain(rootc, 512)?);
        fill(&mut root, rootc, 0, &read_chain)?;
        Ok(Card { root, free, cluster: csize as u32, formatted: true })
    }

    fn clusters_of(&self, e: &Ent) -> u32 {
        let bytes = if e.is_dir() { e.kids.len() * 512 } else { e.data.len() };
        bytes.div_ceil(self.cluster as usize) as u32
    }
}

impl Card {
    /// The raw image of a freshly formatted, empty 8 MB memory card, as a PS2 formats it (layout read off a real card:
    /// 512-byte pages, 2 pages per cluster, 16 per block, 8192 clusters; the indirect FAT cluster is 8, the 32 FAT
    /// clusters are 9..=40, the allocatable area starts at cluster 41 and runs to 8135, the root directory is its
    /// cluster 0 holding "." and ".."; erase-backup blocks 1022 and 1023; mcman's 8001 allocatable clusters).
    /// Plain 512-byte pages without the spare/ECC bytes: `parse` reads those images too. No timestamps.
    pub fn blank_image() -> Vec<u8> {
        const CL: usize = 1024;
        let mut img = vec![0u8; 8192 * CL];
        let mut put = |o: usize, v: u32| img[o..o + 4].copy_from_slice(&v.to_le_bytes());
        for (o, v) in [
            (0x28, 0x0002_0200), // page length 512, 2 pages per cluster
            (0x2c, 0xff00_0010), // 16 pages per block, 0xff00
            (0x30, 8192),        // clusters per card
            (0x34, 41),          // first allocatable cluster
            (0x38, 8135),        // allocatable clusters
            (0x3c, 0),           // root directory (relative to the allocatable area)
            (0x40, 1023),        // erase backup blocks
            (0x44, 1022),
            (0x50, 8), // indirect FAT cluster list
            (0x150, 0x2b02), // card type 2, flags 0x2b
            (0x154, 0x400),
            (0x158, 0x100),
            (0x15c, 8),
            (0x160, u32::MAX),
            (0x170, 8001), // max allocatable clusters (mcman)
            (0x17c, u32::MAX),
        ] {
            put(o, v);
        }
        for o in (0xd0..0x150).step_by(4).chain((0x184..0x200).step_by(4)) {
            put(o, u32::MAX); // bad block list (none) and the unused tail
        }
        img[..35].copy_from_slice(b"Sony PS2 Memory Card Format 1.2.0.0");
        // indirect FAT cluster 8: FAT clusters 9..=40, then unused
        for k in 0..256usize {
            let v = if k < 32 { 9 + k as u32 } else { u32::MAX };
            img[8 * CL + 4 * k..8 * CL + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        // FAT (32 clusters of 256 entries): entry 0 = the root directory (allocated, end of chain), free entries 0x7fffffff up to the
        // allocatable end, 0xffffffff after it
        for i in 0..32 * 256usize {
            let v = if i == 0 || i >= 8135 { u32::MAX } else { 0x7fff_ffff };
            img[9 * CL + 4 * i..9 * CL + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        // root directory cluster (at 41): "." (mode 0x8427) and ".." (0xa426), two entries
        let root = 41 * CL;
        for (k, (mode, len, name)) in [(0x8427u16, 2u32, "."), (0xa426, 0, "..")].into_iter().enumerate() {
            let e = root + 512 * k;
            img[e..e + 2].copy_from_slice(&mode.to_le_bytes());
            img[e + 4..e + 8].copy_from_slice(&len.to_le_bytes());
            img[e + 0x40..e + 0x40 + name.len()].copy_from_slice(name.as_bytes());
        }
        img
    }

    /// A freshly formatted empty card: `blank_image` read by `parse`, so it is exactly what a real blank card gives
    /// (free = mcman's 8001 allocatable clusters minus the root directory's one = 8000).
    pub fn blank() -> Card {
        Card::parse(&Card::blank_image()).expect("the blank card image parses")
    }
}

#[derive(Clone)]
pub struct Fd {
    pub port: u32,
    pub path: Vec<String>,
    pub pos: u32,
}

#[derive(Default, Clone)]
pub struct Mc {
    /// The images were read (on the first memory card RPC).
    pub loaded: bool,
    pub cards: [Option<Card>; 2],
    pub fds: Vec<Option<Fd>>,
    /// The GetDir listing in progress and the next entry it returns.
    pub list: Vec<[u8; 64]>,
    pub list_pos: usize,
    /// Current directory of each port (sceMcChdir; empty = the root). Not part of the savestate: it restarts at the root.
    pub cwd: [Vec<String>; 2],
    /// Content version, not machine state (never saved): a new process-wide unique number at every change (`bump`),
    /// so equal numbers mean equal contents. fndiff's per-sample snapshot copies the card state only when it moved.
    pub gen: u64,
    /// Cold boot (`Machine::cold` sets it; never saved): the memcards folder is not consulted, port 0 starts from
    /// `Card::blank()` (a freshly formatted empty card) unless ORACLE_MCD names an image.
    pub blank_default: bool,
}

/// The last `Mc::gen` handed out (unique across all machines of the process).
static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The image of memory card port `p` (0 or 1): ORACLE_MCD (port 0) / ORACLE_MCD2 (port 1), else
/// Mcd00N.ps2 in $ORACLE_MCD_DIR (default: the `memcards` folder of this crate, oracle/memcards) when that file exists;
/// None = no card in the port.
pub fn image_path(p: usize) -> Option<String> {
    if let Some(v) = explicit_image(p) {
        return Some(v);
    }
    let dir = std::env::var("ORACLE_MCD_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/memcards").into());
    let path = format!("{dir}/Mcd00{}.ps2", p + 1);
    std::path::Path::new(&path).exists().then_some(path)
}

/// The image named by ORACLE_MCD (port 0) / ORACLE_MCD2 (port 1), if set.
pub fn explicit_image(p: usize) -> Option<String> {
    std::env::var(if p == 0 { "ORACLE_MCD" } else { "ORACLE_MCD2" }).ok()
}

/// libmc's send buffer for a call of `ssize` bytes: every libmc RPC (boot 0x1233f0..0x124400, client 0x159a00)
/// sends either the 0x30-byte struct at 0x159a80 (fno 1 3 4 5 6 0x10 0x11 0xfe) or the 0x414-byte one at 0x159ab0
/// (fno 2 0xc 0xd 0xf, the ones with a path). [C] the twelve sceSifCallRpc call sites.
pub fn send_buffer(ssize: u32) -> Option<u32> {
    match ssize {
        0x30 => Some(0x159a80),
        0x414 => Some(0x159ab0),
        _ => None,
    }
}

fn path_of(name: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in name.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                v.pop();
            }
            x => v.push(x.to_string()),
        }
    }
    v
}

fn find<'a>(root: &'a Ent, path: &[String]) -> Option<&'a Ent> {
    let mut e = root;
    for c in path {
        e = e.kids.iter().find(|k| k.live() && !k.dot() && k.name == *c)?;
    }
    Some(e)
}

fn find_mut<'a>(root: &'a mut Ent, path: &[String]) -> Option<&'a mut Ent> {
    let mut e = root;
    for c in path {
        e = e.kids.iter_mut().find(|k| k.live() && !k.dot() && k.name == *c)?;
    }
    Some(e)
}

/// `*` and `?` wildcards (mcman's GetDir pattern).
fn matches(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => matches(&p[1..], s) || (!s.is_empty() && matches(p, &s[1..])),
        (Some(b'?'), Some(_)) => matches(&p[1..], &s[1..]),
        (Some(a), Some(b)) => a == b && matches(&p[1..], &s[1..]),
        _ => false,
    }
}

impl Mc {
    /// Marks a change of the contents (see `gen`).
    pub fn bump(&mut self) {
        self.gen = GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    }

    /// The cards a fresh machine starts with: the images named by `image_path`, or (cold boot, `blank_default`) the blank
    /// card in port 0 unless ORACLE_MCD names an image. Both ports parse or nothing changes (the error stops the run).
    pub fn base_cards(&self) -> Result<[Option<Card>; 2], String> {
        let mut cards: [Option<Card>; 2] = [None, None];
        for (p, card) in cards.iter_mut().enumerate() {
            let path = if self.blank_default { explicit_image(p) } else { image_path(p) };
            if let Some(path) = path {
                let img = std::fs::read(&path).map_err(|e| format!("memory card image {path}: {e}"))?;
                *card = Some(Card::parse(&img).map_err(|e| format!("memory card image {path}: {e}"))?);
            } else if p == 0 && self.blank_default {
                *card = Some(Card::blank());
            }
        }
        Ok(cards)
    }

    /// Reads the images (once).
    fn load(&mut self) -> Result<(), String> {
        if self.loaded {
            return Ok(());
        }
        let cards = self.base_cards()?;
        self.cards = cards;
        self.fds = (0..MAX_FDS).map(|_| None).collect();
        self.loaded = true;
        self.bump();
        Ok(())
    }

    pub fn save(&self, w: &mut crate::snapfile::Wr) {
        w.bool(self.loaded);
        for c in &self.cards {
            match c {
                Some(c) => {
                    w.bool(true);
                    w.u32(c.free);
                    w.u32(c.cluster);
                    w.bool(c.formatted);
                    c.root.save(w);
                }
                None => w.bool(false),
            }
        }
        w.u64(self.fds.len() as u64);
        for f in &self.fds {
            match f {
                Some(f) => {
                    w.bool(true);
                    w.u32(f.port);
                    w.u32(f.pos);
                    w.str(&f.path.join("/"));
                }
                None => w.bool(false),
            }
        }
        w.u64(self.list.len() as u64);
        for e in &self.list {
            w.0.extend_from_slice(e);
        }
        w.u64(self.list_pos as u64);
    }

    pub fn load_dyn(&mut self, r: &mut crate::snapfile::Rd) {
        self.loaded = r.bool();
        for c in self.cards.iter_mut() {
            *c = if r.bool() {
                let (free, cluster, formatted) = (r.u32(), r.u32(), r.bool());
                Some(Card { free, cluster, formatted, root: Ent::load(r) })
            } else {
                None
            };
        }
        let n = r.u64() as usize;
        self.fds = (0..n)
            .map(|_| if r.bool() { Some(Fd { port: r.u32(), pos: r.u32(), path: path_of(&r.str()) }) } else { None })
            .collect();
        let n = r.u64() as usize;
        self.list = (0..n)
            .map(|_| {
                let mut e = [0u8; 64];
                for b in e.iter_mut() {
                    *b = r.u8();
                }
                e
            })
            .collect();
        self.list_pos = r.u64() as usize;
        self.bump();
    }
}

impl Cpu {
    fn mc_str(&mut self, a: u32) -> String {
        let mut v = Vec::new();
        for k in 0..0x400 {
            let b = self.mem.rd8(a + k);
            if b == 0 {
                break;
            }
            v.push(b);
        }
        String::from_utf8_lossy(&v).into_owned()
    }

    /// mcserv's reply to libmc's RPC `fno` (see the module comment); `end` / `end_param` are sceSifCallRpc's end
    /// callback and its argument, run here as the EE would run it when the reply arrives.
    pub(crate) fn mc_rpc(&mut self, fno: u32, send: u32, recv: u32, end: u32, end_param: u32) {
        if let Err(e) = self.mc.load() {
            self.stop = Some(e);
            return;
        }
        // Every function but GetInfo and Flush changes mcman's state (files, open fds and their positions, listing).
        if fno != 1 && fno != 0xa {
            self.mc.bump();
        }
        let w = |c: &mut Cpu, o: u32| c.mem.rd32(send + o);
        let res: i32 = match fno {
            1 => {
                let (port, rdata) = (w(self, 4) as usize & 1, w(self, 0x1c));
                let (r, ty, free, fmt) = match &self.mc.cards[port] {
                    Some(c) => (0, 2, c.free, c.formatted as u32),
                    None => (-10, 0, 0, 0),
                };
                self.mem.wr32(rdata, ty);
                self.mem.wr32(rdata + 4, free);
                self.mem.wr32(rdata + 0x90, fmt);
                r
            }
            2 => {
                let (port, flags) = (w(self, 0) as usize & 1, w(self, 8));
                let name = self.mc_str(send + 0x14);
                self.mc_open(port, flags, &name)
            }
            3 => {
                let fd = w(self, 0) as usize;
                match self.mc.fds.get_mut(fd) {
                    Some(f @ Some(_)) => {
                        *f = None;
                        0
                    }
                    _ => -4,
                }
            }
            4 => {
                let (fd, off, whence) = (w(self, 0) as usize, w(self, 0x10) as i32, w(self, 0x14));
                let len = self.mc_file(fd).map(|e| e.data.len() as i32);
                match (len, self.mc.fds.get_mut(fd).and_then(|f| f.as_mut())) {
                    (Some(len), Some(f)) => {
                        let base = match whence {
                            0 => 0,
                            1 => f.pos as i32,
                            _ => len,
                        };
                        f.pos = (base + off).max(0) as u32;
                        f.pos as i32
                    }
                    _ => -4,
                }
            }
            5 => {
                let (fd, size, buf, rdata) = (w(self, 0) as usize, w(self, 0xc), w(self, 0x18), w(self, 0x1c));
                let pos = self.mc.fds.get(fd).and_then(|f| f.as_ref()).map(|f| f.pos as usize);
                match (pos, self.mc_file(fd).map(|e| e.data.clone())) {
                    (Some(pos), Some(data)) => {
                        // A Seek may have moved past the end: nothing to read there.
                        let pos = pos.min(data.len());
                        let n = (size as usize).min(data.len() - pos);
                        let bytes = &data[pos..pos + n];
                        let head = ((64 - buf % 64) % 64).min(n as u32) as usize;
                        let tail = (n - head) % 64;
                        let mid = n - head - tail;
                        for (k, b) in bytes[head..head + mid].iter().enumerate() {
                            self.mem.wr8(buf + (head + k) as u32, *b);
                        }
                        self.mem.wr32(rdata, head as u32);
                        self.mem.wr32(rdata + 4, tail as u32);
                        self.mem.wr32(rdata + 8, buf);
                        self.mem.wr32(rdata + 0xc, buf + (head + mid) as u32);
                        for (k, b) in bytes[..head].iter().enumerate() {
                            self.mem.wr8(rdata + 0x10 + k as u32, *b);
                        }
                        for (k, b) in bytes[head + mid..].iter().enumerate() {
                            self.mem.wr8(rdata + 0x50 + k as u32, *b);
                        }
                        if let Some(Some(f)) = self.mc.fds.get_mut(fd) {
                            f.pos += n as u32;
                        }
                        n as i32
                    }
                    _ => -4,
                }
            }
            6 => {
                let (fd, size, head, src) = (w(self, 0) as usize, w(self, 0xc), w(self, 0x14), w(self, 0x18));
                let mut bytes: Vec<u8> = (0..head.min(16)).map(|k| self.mem.rd8(send + 0x20 + k)).collect();
                bytes.extend((0..size).map(|k| self.mem.rd8(src + k)));
                self.mc_write(fd, &bytes)
            }
            0xa => 0,
            // Chdir (+0 port, +4 slot, +0x14 path; the reply string, the new current directory, goes to 0x159fc0).
            // [C] the call site (frontbin card state machine 7/8, boot FUN_001240c0); the result codes are mcman's [I].
            0xc => {
                let port = w(self, 0) as usize & 1;
                let name = self.mc_str(send + 0x14);
                let path = self.mc_path(port, &name);
                let r = match &self.mc.cards[port] {
                    None => -10,
                    Some(c) if !c.formatted => -2,
                    Some(c) => match find(&c.root, &path) {
                        Some(e) if e.is_dir() => 0,
                        _ => -4,
                    },
                };
                if r == 0 {
                    self.mc.cwd[port] = path;
                }
                let cur = format!("/{}", self.mc.cwd[port].join("/"));
                for (k, b) in cur.bytes().chain(std::iter::once(0)).enumerate() {
                    self.mem.wr8(CHDIR_DIR + k as u32, b);
                }
                r
            }
            0xd => {
                let (port, flags, max, table) = (w(self, 0) as usize & 1, w(self, 8), w(self, 0xc) as i32, w(self, 0x10));
                let name = self.mc_str(send + 0x14);
                self.mc_getdir(port, flags, max, table, &name)
            }
            // sceMcInit's version query (cold boot only: savestates are taken after the init): result 0, then the
            // mcserv and mcman versions, which libmc checks against 0x20a / 0x20e.
            0xfe => {
                self.mem.wr32(recv + 4, 0x20b);
                self.mem.wr32(recv + 8, 0x20e);
                0
            }
            0xf => {
                let port = w(self, 0) as usize & 1;
                let name = self.mc_str(send + 0x14);
                self.mc_delete(port, &name)
            }
            _ => {
                self.stop = Some(format!("memory card RPC fno {fno:#x} not emulated (mc.rs)"));
                return;
            }
        };
        self.mem.wr32(recv, res as u32);
        // The end callback (libmc's own code, run when the reply has arrived).
        match end {
            0 => {}
            CB_GETINFO => {
                for (p, o) in GETINFO_PTRS {
                    let dst = self.mem.rd32(p);
                    if dst != 0 {
                        let v = self.mem.rd32(end_param + o);
                        self.mem.wr32(dst, v);
                    }
                }
            }
            CB_CHDIR => {
                if end_param != 0 {
                    for k in 0..0x3ff {
                        let b = self.mem.rd8(CHDIR_DIR + k);
                        self.mem.wr8(end_param + k, b);
                        if b == 0 {
                            break;
                        }
                    }
                }
            }
            CB_READ => {
                let (n0, n1) = (self.mem.rd32(end_param) as i32, self.mem.rd32(end_param + 4) as i32);
                let (d0, d1) = (self.mem.rd32(end_param + 8), self.mem.rd32(end_param + 0xc));
                for k in 0..n0.max(0) as u32 {
                    let b = self.mem.rd8(end_param + 0x10 + k);
                    self.mem.wr8(d0 + k, b);
                }
                for k in 0..n1.max(0) as u32 {
                    let b = self.mem.rd8(end_param + 0x50 + k);
                    self.mem.wr8(d1 + k, b);
                }
            }
            _ => self.stop = Some(format!("memory card RPC fno {fno:#x}: end callback {end:08x} not emulated (mc.rs)")),
        }
    }

    /// A path as mcman resolves it: absolute with a leading '/', else relative to the port's current directory.
    fn mc_path(&self, port: usize, name: &str) -> Vec<String> {
        if name.starts_with('/') {
            path_of(name)
        } else {
            path_of(&format!("{}/{}", self.mc.cwd[port & 1].join("/"), name))
        }
    }

    fn mc_file(&self, fd: usize) -> Option<&Ent> {
        let f = self.mc.fds.get(fd)?.as_ref()?;
        let c = self.mc.cards[f.port as usize].as_ref()?;
        find(&c.root, &f.path).filter(|e| !e.is_dir())
    }

    fn mc_open(&mut self, port: usize, flags: u32, name: &str) -> i32 {
        let path = self.mc_path(port, name);
        let Some(card) = self.mc.cards[port].as_mut() else { return -10 };
        if !card.formatted {
            return -2;
        }
        let now = crate::clock::local_card();
        let Some((last, parent)) = path.split_last() else { return -4 };
        let exists = find(&card.root, &path).is_some();
        if flags & 0x40 != 0 {
            // Mkdir.
            if exists {
                return -4;
            }
            let d = Ent::new(last, true, now);
            let need = 1;
            let Some(par) = find_mut(&mut card.root, parent).filter(|p| p.is_dir()) else { return -4 };
            let old = (par.kids.len() * 512).div_ceil(card.cluster as usize) as u32;
            let new = ((par.kids.len() + 1) * 512).div_ceil(card.cluster as usize) as u32;
            if card.free < need + new - old {
                return -3;
            }
            par.kids.push(d);
            par.len = par.kids.len() as u32;
            card.free -= need + new - old;
            return 0;
        }
        if !exists {
            if flags & 0x200 == 0 {
                return -4;
            }
            let Some(par) = find_mut(&mut card.root, parent).filter(|p| p.is_dir()) else { return -4 };
            let old = (par.kids.len() * 512).div_ceil(card.cluster as usize) as u32;
            let new = ((par.kids.len() + 1) * 512).div_ceil(card.cluster as usize) as u32;
            if card.free < new - old {
                return -3;
            }
            par.kids.push(Ent::new(last, false, now));
            par.len = par.kids.len() as u32;
            card.free -= new - old;
        } else if find(&card.root, &path).is_some_and(|e| e.is_dir()) {
            return -5;
        }
        if flags & 0x400 != 0 {
            let cl = card.cluster as usize;
            let e = find_mut(&mut card.root, &path).unwrap();
            let freed = e.data.len().div_ceil(cl) as u32;
            e.data.clear();
            e.len = 0;
            e.modified = now;
            card.free += freed;
        }
        let Some(slot) = self.mc.fds.iter().position(|f| f.is_none()) else { return -7 };
        self.mc.fds[slot] = Some(Fd { port: port as u32, path, pos: 0 });
        slot as i32
    }

    fn mc_write(&mut self, fd: usize, bytes: &[u8]) -> i32 {
        let Some(Some(f)) = self.mc.fds.get(fd) else { return -4 };
        let (port, pos, path) = (f.port as usize, f.pos as usize, f.path.clone());
        let Some(card) = self.mc.cards[port].as_mut() else { return -10 };
        let cl = card.cluster as usize;
        let Some(e) = find_mut(&mut card.root, &path).filter(|e| !e.is_dir()) else { return -4 };
        let end = pos + bytes.len();
        let grow = end.div_ceil(cl).saturating_sub(e.data.len().div_ceil(cl)) as u32;
        if grow > card.free {
            return -3;
        }
        if e.data.len() < end {
            e.data.resize(end, 0);
        }
        e.data[pos..end].copy_from_slice(bytes);
        e.len = e.data.len() as u32;
        let now = crate::clock::local_card();
        e.modified = now;
        card.free -= grow;
        // The directory the file is in changed too.
        if let Some(par) = find_mut(&mut card.root, &path[..path.len() - 1]) {
            par.modified = now;
        }
        if let Some(Some(f)) = self.mc.fds.get_mut(fd) {
            f.pos = end as u32;
        }
        bytes.len() as i32
    }

    fn mc_getdir(&mut self, port: usize, flags: u32, max: i32, table: u32, name: &str) -> i32 {
        if flags == 0 {
            self.mc.list.clear();
            self.mc.list_pos = 0;
            let Some(card) = self.mc.cards[port].as_ref() else { return -10 };
            if !card.formatted {
                return -2;
            }
            let path = self.mc_path(port, name);
            let (pat, parent) = match path.split_last() {
                Some((l, p)) => (l.clone(), p.to_vec()),
                None => ("*".to_string(), Vec::new()),
            };
            let Some(dir) = find(&card.root, &parent).filter(|d| d.is_dir()) else { return -4 };
            for k in dir.kids.iter().filter(|k| k.live() && matches(pat.as_bytes(), k.name.as_bytes())) {
                let mut r = [0u8; 64];
                r[0..8].copy_from_slice(&k.created);
                r[8..16].copy_from_slice(&k.modified);
                let len = if k.is_dir() && !k.dot() { k.kids.len() as u32 } else { k.len };
                r[16..20].copy_from_slice(&len.to_le_bytes());
                r[20..22].copy_from_slice(&k.mode.to_le_bytes());
                r[24..28].copy_from_slice(&k.attr.to_le_bytes());
                let nb = k.name.as_bytes();
                r[32..32 + nb.len().min(31)].copy_from_slice(&nb[..nb.len().min(31)]);
                self.mc.list.push(r);
            }
        }
        let left = self.mc.list.len() - self.mc.list_pos.min(self.mc.list.len());
        if max <= 0 {
            return left as i32;
        }
        let n = left.min(max as usize);
        for i in 0..n {
            let r = self.mc.list[self.mc.list_pos + i];
            for (k, b) in r.iter().enumerate() {
                self.mem.wr8(table + (i * 64 + k) as u32, *b);
            }
        }
        self.mc.list_pos += n;
        n as i32
    }

    fn mc_delete(&mut self, port: usize, name: &str) -> i32 {
        let path = self.mc_path(port, name);
        let Some(card) = self.mc.cards[port].as_mut() else { return -10 };
        if !card.formatted {
            return -2;
        }
        let Some((last, parent)) = path.split_last() else { return -4 };
        let Some(e) = find(&card.root, &path) else { return -4 };
        if e.is_dir() && e.kids.iter().any(|k| k.live() && !k.dot()) {
            return -6;
        }
        let freed = card.clusters_of(e);
        let par = find_mut(&mut card.root, parent).unwrap();
        let i = par.kids.iter().position(|k| k.live() && !k.dot() && k.name == *last).unwrap();
        // mcman clears the exists bit; the entry stays in the directory.
        par.kids[i].mode &= !M_EXISTS;
        par.kids[i].data.clear();
        par.kids[i].kids.clear();
        card.free += freed;
        0
    }
}

#[cfg(test)]
mod blank_tests {
    use super::*;

    #[test]
    fn blank_card_is_formatted_and_empty() {
        let c = Card::blank();
        assert!(c.formatted);
        assert_eq!((c.free, c.cluster), (8000, 1024));
        let names: Vec<&str> = c.root.kids.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, [".", ".."]);
        assert_eq!((c.root.name.as_str(), c.root.mode, c.root.len), (".", 0x8427, 2));
    }

    /// A new file, and the "." and ".." of a new directory, take the time they are given (mc_open and mc_write pass
    /// the host's local time; Card::blank keeps the zero times of a real formatted card).
    #[test]
    fn a_new_entry_takes_the_time_it_is_given() {
        let now = [0, 9, 5, 16, 23, 10, 0xea, 0x07];
        let f = Ent::new("save0.bin", false, now);
        assert_eq!((f.created, f.modified), (now, now));
        let d = Ent::new("BASCUS-97199RATCHET", true, now);
        assert_eq!((d.created, d.modified), (now, now));
        assert_eq!((d.kids[0].created, d.kids[1].modified), (now, now));
    }

    /// A cold-boot machine (`Mc::blank_default`) starts from the blank card in port 0, never from the memcards folder.
    #[test]
    fn cold_machine_base_card_is_blank() {
        if std::env::var("ORACLE_MCD").is_ok() {
            return;
        }
        let mc = Mc { blank_default: true, ..Default::default() };
        let c = mc.base_cards().unwrap();
        assert!(c[1].is_none());
        let b = c[0].as_ref().unwrap();
        assert_eq!((b.free, b.cluster, b.formatted), (8000, 1024, true));
        assert_eq!(b.root.kids.len(), 2);
    }

    /// MC_BLANK_DUMP=<file> also writes the raw image (to compare its metadata with a real card).
    #[test]
    fn blank_image_dump() {
        if let Ok(p) = std::env::var("MC_BLANK_DUMP") {
            std::fs::write(p, Card::blank_image()).unwrap();
        }
    }
}
