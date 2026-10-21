# Static recompiler: retail EE functions (boot + one level overlay) -> Rust crates under oracle/rec/, registered by
# the ee-game crate (oracle/game, also generated here).
# usage: python recgen.py STATE_DIR LEVEL_DIR_NAME [COVER_TXT] [--keep-boot]
#   e.g. python recgen.py states/s07 4_eudora
#   --keep-boot: compile only the overlay; rec/boot_NN stay as they are (not removed, not rewritten).
# Boot functions go to rec/boot_NN (shared by all levels), the level's overlay functions to rec/l<level>_NN. Each
# crate is one set: its functions call each other directly; calls into other crates go through the dispatcher table.
# A function is compiled when the coverage run executed any of its PCs (all functions without COVER_TXT). Each
# function becomes one Rust fn entered at its first instruction, with structured control flow (loops, labeled blocks,
# if/match on the PC after each basic block; all state in `Cpu`). Functions whose graph is irreducible keep the
# dispatch form (a loop over `match c.pc`, any block an entry). Other entries (a savestate resumed mid-function, an
# interrupt returning into another thread) run in the interpreter until they reach a compiled entry. Calls are direct
# Rust calls inside a crate (rec/mod.rs `call_dyn` otherwise); `jr ra` returns. Interrupts and DMA stops keep the
# interpreter's per-instruction timing: a block runs inline only when no event is due inside it, else it is
# interpreted with interrupts serviced in place (`slow`). At the vblank wait the host's frame callback runs in place
# (the game runs on its own thread, ee-oracle `Machine`), so nothing unwinds there.
# Instructions the generator does not inline run through the interpreter's own `exec` (same semantics by construction).
import os, sys, struct, argparse, re, shutil
from array import array

# recgen_iso.py (race step 6): same translator as oracle/recgen.py; inputs differ.
#   code bytes : --src iso   -> the player's own ISO (boot ELF SCES_509.16 + the level's overlay records, isoin.py/img.py)
#                --src state:DIR -> DIR/eeMemory.bin (the old path, kept only for the equivalence test)
#   function set: --bounds fn_bounds.tsv (entry end level; addresses only, no decomp output); every listed function
#   output     : --out DIR  (DIR/rec/<boot|lN>_NN crates, DIR/game facade); --oracle = path of the ee-oracle crate
HERE = os.path.dirname(os.path.abspath(__file__))

# ---- the player's disc image (formerly isoin.py + img.py): boot ELF SCES_509.16 and the level overlay records, read straight from the .iso
# Layout facts from rac-rs/src/{iso,toc,level,wad}.rs. No savestate, no extracted/ dir, no decomp output.
ISO_SIZE = 4213997568
ISO_SHA1 = 'b0dfbd8eac0d84f173b6fc25d8a02a5ff22c652f'
# Supported discs: tag -> (image size, SHA-1, boot ELF, serial, bounds file next to this script). The USA build is the same
# program with its level code moved (tools/vermap.py writes fn_bounds_usa.tsv from the PAL list).
ISO_VERSIONS = {
    'pal': (4213997568, 'b0dfbd8eac0d84f173b6fc25d8a02a5ff22c652f', 'SCES_509.16', 'SCES-50916', 'fn_bounds.tsv'),
    'usa': (4214095872, 'aee9275384f739c9b24cd5de6f7e200292db7b01', 'SCUS_971.99', 'SCUS-97199', 'fn_bounds_usa.tsv'),
}
ISO_VER = 'pal'
ISO_SEC = 0x800
ISO_LEVELS = ['0_veldin_tutorial', '1_novalis', '2_aridia', '3_kerwan', '4_eudora', '5_rilgar', '6_nebula_g34', '7_umbris',
          '8_batalia', '9_gaspar', '10_orxon', '11_pokitaru', '12_hoven', '13_oltanis_orbit', '14_oltanis', '15_quartu',
          '16_kalebo_iii', '17_veldin_orbit', '18_veldin_finale']


def iso_u32(b, o):
    return struct.unpack_from('<I', b, o)[0]


class IsoDisc:
    def __init__(self, path):
        self.f = open(path, 'rb')
        self.n = os.path.getsize(path) // ISO_SEC

    def sectors(self, lba, n):
        self.f.seek(lba * ISO_SEC)
        return self.f.read(n * ISO_SEC)

    def bytes(self, lba, n):
        self.f.seek(lba * ISO_SEC)
        return self.f.read(n)


def iso_wad_decompress(data):
    """Insomniac WAD LZ (port of rac-rs/src/wad.rs)."""
    assert data[:3] == b'WAD'
    end = min(iso_u32(data, 3), len(data))
    out = bytearray()
    src = 16
    last = 0xff
    if src < end and data[src] > 0x11:
        n = data[src] - 0x11
        out += data[src + 1:src + 1 + n]
        src += 1 + n
        last = 0
    while src < end:
        flag = data[src]
        if flag < 0x10:
            if last < 0x10:
                raise ValueError('double literal')
            if flag == 0:
                ln, st = data[src + 1] + 18, src + 2
            else:
                ln, st = flag + 3, src + 1
            out += data[st:st + ln]
            src = st + ln
        else:
            if flag >= 0x40:
                b = data[src + 1]
                cl = (flag >> 5) + 1
                dist = b * 8 + ((flag >> 2) & 7) + 1
                tail = flag & 3
                nxt = src + 2
            elif flag >= 0x20:
                if flag == 0x20:
                    cl, p = data[src + 1] + 33, src + 2
                else:
                    cl, p = (flag & 0x1f) + 2, src + 1
                an, b = data[p], data[p + 1]
                dist = (an >> 2) + b * 0x40 + 1
                tail = an & 3
                nxt = p + 2
            else:
                a = (flag >> 3) & 1
                m3 = flag & 7
                if m3 == 0:
                    cl, p = data[src + 1] + 9, src + 2
                else:
                    cl, p = m3 + 2, src + 1
                bn, c = data[p], data[p + 1]
                b = bn >> 2
                tail = bn & 3
                nxt = p + 2
                if a == 0 and b == 0 and c == 0:
                    if m3 != 1:
                        s = nxt
                        while s < end and (s % 0x2000) != 0x10:
                            s += 1
                        src = s
                        last = flag
                        continue
                    cl = 0
                    dist = 0
                else:
                    dist = 0x4000 + a * 0x4000 + b + c * 0x40
            if cl > 0:
                if dist > len(out):
                    raise ValueError('bad offset')
                frm = len(out) - dist
                for i in range(cl):
                    out.append(out[frm + i])
            out += data[nxt:nxt + tail]
            src = nxt + tail
        last = flag
    return bytes(out)


def iso_boot_elf(d):
    """The boot ELF (SCES_509.16 or SCUS_971.99, see ISO_VER) via a minimal ISO9660 walk (root directory record)."""
    pvd = d.sectors(16, 1)
    assert pvd[1:6] == b'CD001'
    root_lba, root_len = iso_u32(pvd, 156 + 2), iso_u32(pvd, 156 + 10)
    dirdata = d.bytes(root_lba, root_len)
    o = 0
    while o < len(dirdata):
        ln = dirdata[o]
        if ln == 0:
            o = (o // ISO_SEC + 1) * ISO_SEC
            continue
        name = dirdata[o + 33:o + 33 + dirdata[o + 32]].decode('latin1')
        if name.upper().startswith(ISO_VERSIONS[ISO_VER][2]):
            return d.bytes(iso_u32(dirdata, o + 2), iso_u32(dirdata, o + 10))
        o += ln
    raise FileNotFoundError(ISO_VERSIONS[ISO_VER][2])


def iso_level_section(d, lvl, idx):
    toc = d.bytes(1500, 0x2960)
    assert iso_u32(toc, 0) == 1 and iso_u32(toc, 4) == 0x2960
    hdr = d.bytes(iso_u32(toc, 0x28c8 + lvl * 8), 0x2434)
    assert iso_u32(hdr, 0) == lvl and iso_u32(hdr, 4) == 0x2434
    data = d.sectors(iso_u32(hdr, 8), iso_u32(hdr, 12))
    off, size = iso_u32(data, idx * 8), iso_u32(data, idx * 8 + 4)
    raw = data[off:off + size]
    return iso_wad_decompress(raw) if raw[:3] == b'WAD' else raw


def iso_elf_load(elf):
    """(vaddr, bytes) of the one PT_LOAD."""
    phoff = iso_u32(elf, 28)
    phentsize, phnum = struct.unpack_from('<HH', elf, 42)
    segs = []
    for i in range(phnum):
        t, off, va, pa, fs, ms, fl, al = struct.unpack_from('<8I', elf, phoff + i * phentsize)
        if t == 1:
            segs.append((va, elf[off:off + fs], ms))
    return segs


def iso_overlay_code(d, lvl):
    return iso_level_section(d, lvl, 0)


def iso_build_image(d, lvl, size=0x2000000, zero_from=None):
    """Fresh-RAM image from the disc: boot ELF PT_LOAD(s) then the level's overlay records (placed at their load addresses)."""
    ram = bytearray(size)
    boot = iso_boot_elf(d)
    for va, seg, ms in iso_elf_load(boot):
        ram[va:va + len(seg)] = seg
    if zero_from is not None:
        ram[zero_from:] = bytes(size - zero_from)
    recs = []
    ov = iso_overlay_code(d, lvl)
    p = 0
    while p < len(ov):
        load, sz, fl, ent = struct.unpack_from('<4I', ov, p)
        recs.append((load, sz, fl, ent))
        ram[load:load + sz] = ov[p + 16:p + 16 + sz]
        p += 16 + sz
    assert p == len(ov), (p, len(ov))
    return ram, recs, boot


def iso_check(path, sha1=False):
    """Size (and optionally SHA-1) of R&C PAL v1.00 (SCES-50916) or USA v1.00 (SCUS-97199); sets ISO_VER. The boot ELF
    lookup then checks the serial file."""
    global ISO_VER, ISO_SIZE, ISO_SHA1
    n = os.path.getsize(path)
    hit = [k for k, v in ISO_VERSIONS.items() if v[0] == n]
    if not hit:
        sys.exit(f'{path}: {n} bytes, expected ' + ' or '.join(f'{v[0]} (Ratchet & Clank {k.upper()} v1.00, {v[3]})' for k, v in ISO_VERSIONS.items()))
    ISO_VER = hit[0]
    ISO_SIZE, ISO_SHA1 = ISO_VERSIONS[ISO_VER][:2]
    if sha1:
        import hashlib
        h = hashlib.sha1()
        with open(path, 'rb') as f:
            for b in iter(lambda: f.read(1 << 24), b''):
                h.update(b)
        if h.hexdigest() != ISO_SHA1:
            sys.exit(f'{path}: SHA-1 {h.hexdigest()}, expected {ISO_SHA1}')

ap = argparse.ArgumentParser()
ap.add_argument('--src', default='iso')
ap.add_argument('--iso', default=None)
ap.add_argument('--level', type=int, default=None)
ap.add_argument('--facade-only', action='store_true', help='only rewrite game/ (Cargo.toml + lib.rs) from the crates under OUT/rec')
ap.add_argument('--bounds', default=None, help='function bounds (default: fn_bounds.tsv for PAL, fn_bounds_usa.tsv for USA, next to this script)')
ap.add_argument('--out', default=os.path.join(HERE, '..', 'generated'))
ap.add_argument('--oracle', default=os.path.join(HERE, '..', 'oracle'))
ap.add_argument('--stubs', default=os.path.join(HERE, '..', 'oracle', 'stubs.txt'))
ap.add_argument('--names', default=None, help='dev only: decomp fn_names.tsv (function idents)')
ap.add_argument('--keep-boot', action='store_true')
ap.add_argument('--no-comments', action='store_true', help='no per-instruction disassembly / block comments')
ap.add_argument('--compact', action='store_true', help='no indentation, no blank lines')
ap.add_argument('--shared-slow', action='store_true', help='cold shared helpers for the slow path of memory accesses and block polls')
ap.add_argument('--outline-mem', action='store_true', help='memory fast paths through one #[inline(never)] helper per access width in each crate (about 2x less LLVM work, same behaviour)')
ap.add_argument('--skin-ff', action='store_true', help='[PC] (speed) the one-joint skinning loop head (matched by its 166 code words) offers the loop to the native fast path (ee-oracle skinff.rs)')
ap.add_argument('--direct-mem', action='store_true', help='with --outline-mem: RAM and scratchpad accesses inline (one bounds check), only the rest through the out-of-line helper (same behaviour)')
ap.add_argument('--no-win-mem', action='store_true', help='dev/test: with --direct-mem, test RAM / scratchpad through the Vec bounds (slice get) instead of constant address ranges')
ap.add_argument('--vu-guard', action='store_true', help='[xlat] --vu-clean, and runs of COP2 FMAC ops share one MXCSR guard (vu_guard_regions)')
ap.add_argument('--no-holes', action='store_true', help='dev/test: do not compile the code gaps between the listed functions (discover_holes)')
ap.add_argument('--chunk', type=int, default=12000)
ap.add_argument('--verify-sha1', action='store_true', help='also check the ISO SHA-1 (setup.ps1 does it already)')
ap.add_argument('--ptrs-iso', action='store_true', help='equivalence test only: with --src state, take the code-pointer label set from the ISO image')
A = ap.parse_args()

def _rel(target, frm):
    """Crate dependency path relative to the crate directory (generated trees hold no absolute paths)."""
    return os.path.relpath(os.path.abspath(target), os.path.abspath(frm)).replace(chr(92), '/')


def _put(path, text):
    """Write `text` (LF line ends) to `path` only when the content differs. Unchanged generated files keep their mtime,
    so a regenerate of an up-to-date install does not make cargo rebuild anything (cargo rebuilds by file mtime)."""
    data = text.encode('utf-8')
    try:
        with open(path, 'rb') as f:
            if f.read() == data:
                return False
    except OSError:
        pass
    with open(path, 'wb') as f:
        f.write(data)
    return True


def write_facade(out, oracle):
    REC = os.path.join(out, 'rec')
    # ee-game: every crate under rec/ (all levels generated so far).
    all_crates = sorted(x for x in os.listdir(REC) if os.path.isfile(os.path.join(REC, x, 'Cargo.toml')))
    G = os.path.join(out, 'game')
    os.makedirs(os.path.join(G, 'src'), exist_ok=True)
    os.makedirs(REC, exist_ok=True)
    deps = '\n'.join(f'rec-{c.replace("_", "-")} = {{ path = "../rec/{c}" }}' for c in all_crates)
    _put(os.path.join(G, 'Cargo.toml'),
        '# Generated by recgen.py.\n[package]\nname = "ee-game"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\nee-oracle = {{ path = "{_rel(oracle, G)}" }}\n{deps}\n')
    def _reg(c):
        # A level crate (lN_MM) is tied to its level byte N: Table::build probes it only while that level is current.
        m = re.fullmatch(r'l(\d+)_\d+', c)
        if m:
            return f'    ee_oracle::rec::register_level({int(m.group(1))}, rec_{c}::FNS);'
        return f'    ee_oracle::rec::register(rec_{c}::FNS);'
    reg = '\n'.join(_reg(c) for c in all_crates)
    _put(os.path.join(G, 'src', 'lib.rs'),
        '//! Generated by recgen.py: the recompiled retail code (crates under oracle/rec) for ee-oracle.\n'
        'pub use ee_oracle;\n\n/// Registers every recompiled set with ee-oracle (before `Machine::enable_rec`).\n'
        f'pub fn register() {{\n{reg}\n}}\n')
    return all_crates


if A.facade_only:
    write_facade(A.out, A.oracle)
    print('facade rewritten')
    sys.exit(0)
if A.level is None:
    ap.error('--level is required')
KEEP_BOOT = A.keep_boot
NOCOM = A.no_comments
SHARED = A.shared_slow
OUTLINE = A.outline_mem
DIRECT = A.direct_mem and A.outline_mem
WINMEM = DIRECT and not A.no_win_mem
VUCLEAN = A.vu_guard
VUGUARD = A.vu_guard
VUGUARD_MIN = int(os.environ.get('XLAT_GUARD_MIN', '3'))
VUA = {}
VUG = set()
lvl = A.level
OV_START = 0x15ef00
VBLANK_WAIT = 0x118cb0
# --skin-ff: skinned_vertex_stream's one-joint vertex loop (+0x694 .. +0x928 of f_1f7440 and its twins): first word, FNV-1a
# 64 of the 166 words (checked again by nothing at run time: ee-oracle skinff.rs depends on exactly these words).
SKIN_FF = (0x784b0000, 166, 0xe0a6afc1c2f6f8db)
def skin_head(L):
    if word(L) != SKIN_FF[0]:
        return False
    h = 0xcbf29ce484222325
    for a in range(L, L + 4 * SKIN_FF[1], 4):
        h = ((h ^ word(a)) * 0x100000001b3) & 0xffffffffffffffff
    return h == SKIN_FF[2]

# Vblank-start poll (sceGsSyncV FUN_00118bd0 +0x40, which spins there while no vsync callback is registered, e.g. on the title
# menu after the PAL 50/60 Hz switch): `lw rt, off(rs)`; `andi rt, rt, 4`; 3 nops; `beq rt, zero, head`; nop. Its head calls the
# native loop hook kind 1 (ee-oracle rec/mod.rs `poll_ff`), which skips the idle iterations before the next event.
def poll_head(L):
    w = word(L)
    rs, rt = w >> 21 & 31, w >> 16 & 31
    return (w >> 26 == 0x23 and rt != 0 and rs != rt and word(L + 4) == 0x30000004 | rt << 21 | rt << 16
            and word(L + 8) == word(L + 12) == word(L + 16) == 0 and word(L + 20) == 0x1000fffa | rt << 21 and word(L + 24) == 0)

if A.src == 'iso':
    if not A.iso:
        sys.exit('--iso PATH is required (the game disc image)')
    iso_check(A.iso, A.verify_sha1)
    if A.bounds is None:
        A.bounds = os.path.join(HERE, ISO_VERSIONS[ISO_VER][4])
    _d = IsoDisc(A.iso)
    ram = bytes(iso_build_image(_d, lvl, zero_from=OV_START)[0])
else:
    ram = open(os.path.join(A.src.split(':', 1)[1], 'eeMemory.bin'), 'rb').read()
if A.bounds is None:
    A.bounds = os.path.join(HERE, 'fn_bounds.tsv')
W = array('I', ram[:len(ram) // 4 * 4])
if A.ptrs_iso and A.src != 'iso':
    _r = bytes(iso_build_image(IsoDisc(A.iso), lvl, zero_from=OV_START)[0])
    W = array('I', _r[:len(_r) // 4 * 4])
if sys.byteorder == 'big':
    W.byteswap()


def word(a):
    return struct.unpack_from('<I', ram, a)[0]


fns = []
for _line in open(A.bounds):
    if _line.startswith('#') or not _line.strip():
        continue
    _a, _e, _l = _line.split()
    if _l == 'boot' or int(_l) == lvl:
        fns.append((int(_a, 16), int(_e, 16)))
fns.sort()
names = {}
if A.names:
    LEVEL_KEY = 'veldin' if lvl == 0 else [x for x in ISO_LEVELS if x.startswith(f'{lvl}_')][0].split('_', 1)[-1]
    for line in open(A.names, encoding='utf-8'):
        if line.startswith('#'):
            continue
        p = line.rstrip('\n').split('\t')
        a = int(p[0], 16)
        if (a < OV_START and p[1] == 'boot') or (a >= OV_START and p[1] == LEVEL_KEY):
            names[a] = p[2]

# Stops: the vblank poll, the stubbed functions (interpreter HLE) and the pad read.
stops = {0x124bd8}
for line in open(A.stubs):
    p = line.split()
    if not p or p[0].startswith('#'):
        continue
    if p[0].startswith('@'):
        if int(p[0][1:]) == lvl:
            stops.add(int(p[1], 16))
    else:
        stops.add(int(p[0], 16))

def sx16(v):
    return v - 0x10000 if v & 0x8000 else v


def u32(v):
    return v & 0xffffffff


# ---- instruction classes ----
def kind(op):
    """'b' control flow, 'x' interpreter-only (ends a block before it), 'm' memory, 'n' anything else."""
    o = op >> 26
    rs, rt, f = (op >> 21) & 31, (op >> 16) & 31, op & 63
    if o == 0:
        if f in (8, 9):
            return 'b'
        if f in (12, 13):
            return 'x'
        return 'n'
    if o == 1:
        if rt in (0, 1, 2, 3, 16, 17, 18, 19):
            return 'b'
        if rt in (8, 9, 10, 11, 12, 14, 24, 25):
            return 'n'
        return 'x'
    if o in (2, 3, 4, 5, 6, 7, 20, 21, 22, 23):
        return 'b'
    if o == 16:
        return 'x'
    if o == 17:
        return 'b' if rs == 8 else 'n'
    if o == 18:
        if rs == 8:
            return 'b'
        return 'n' if rs in (1, 2, 5, 6) or rs >= 16 else 'x'
    if o in (8, 9, 10, 11, 12, 13, 14, 15, 24, 25, 28, 47, 51):
        return 'n'
    if o in (26, 27, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 49, 54, 55, 57, 62, 63):
        return 'm'
    return 'x'


def target(op, pc):
    o = op >> 26
    if o in (2, 3):
        return ((pc + 4) & 0xf0000000) | ((op & 0x3ffffff) << 2)
    if o == 0:
        return None
    return u32(pc + 4 + (sx16(op & 0xffff) << 2))


# Words anywhere in the image pointing into code: jump-table targets (and harmless extras) become block labels.
ptrs = {w for w in W if 0x100000 <= w < 0x400000 and not w & 3}
# ---- discover_holes: functions the Ghidra bounds list misses (compiled code instead of the interpreter) ----
PAD_WORDS = (0, 0xcdcdcdcd)
_SPECIAL_OK = frozenset([0, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 18, 19, 20, 22, 23, 24, 25, 26, 27, 32, 33, 34, 35, 36, 37,
                         38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 54, 56, 58, 59, 60, 62, 63])
_OP_OK = frozenset([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 25, 26, 27, 28, 30, 31,
                    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, 51, 54, 55, 57, 62, 63])


def _valid_op(op):
    """A real R5900 instruction word (strict: data and padding words fail)."""
    if op == 0xcdcdcdcd:
        return False
    o = op >> 26
    if o not in _OP_OK:
        return False
    if o == 0:
        f = op & 63
        if f not in _SPECIAL_OK:
            return False
        if f == 8:
            return (op & 0x001fffc0) == 0 and (op >> 21) & 31 != 0
        if f == 9:
            return (op & 0x001f07c0) == 0
        return True
    if o == 1:
        return ((op >> 16) & 31) in (0, 1, 2, 3, 8, 9, 10, 11, 12, 14, 16, 17, 18, 19, 24, 25)
    if o == 16:
        return ((op >> 21) & 31) in (0, 4, 8, 16)
    if o == 17:
        return ((op >> 21) & 31) in (0, 2, 4, 6, 8, 16, 20)
    if o == 18:
        rs = (op >> 21) & 31
        return rs in (0, 1, 2, 4, 5, 6, 8) or rs >= 16
    return True


def discover_holes(fns, ptrs, limit_hi=0x400000, max_words=8192):
    """Code between the listed functions that the bounds list misses (Ghidra cut or skipped it: leaf routines called through
    function pointers, shared epilogues). Code-only logic, no addresses, so it works for every disc version: a flow scan from
    seeds inside each gap (gap start, branch / jump / call targets from listed functions, constants built by lui + addiu/ori,
    code pointers in the image) accepts a region only if every reachable word is a valid instruction, branches stay in the gap
    and every path ends in jr / j / eret. Regions are split at the seeds other code enters them by (so the dispatcher finds
    those entries). Any split is exact (compiled code behaves like the interpreter whatever its bounds); wrong bounds only
    cost speed. Returns the new (lo, hi) list."""
    import bisect
    NW = len(W)

    def wd(a):
        return W[a >> 2] if 0 <= (a >> 2) < NW else 0

    iv = []
    for lo, hi in sorted(fns):
        if iv and lo <= iv[-1][1]:
            iv[-1][1] = max(iv[-1][1], hi)
        else:
            iv.append([lo, hi])
    gaps = []
    for i, (a0, b0) in enumerate(iv):
        nxt = iv[i + 1][0] if i + 1 < len(iv) else limit_hi
        s, e = b0, min(nxt, limit_hi)
        if s < OV_START < e:
            gaps.append((s, OV_START))
            s = OV_START
        if s < e:
            gaps.append((s, e))
    if not gaps:
        return []
    gstarts = [g0 for g0, _ in gaps]

    def gap_of(a):
        i = bisect.bisect_right(gstarts, a) - 1
        if i >= 0 and gaps[i][0] <= a < gaps[i][1]:
            return i
        return -1

    # hard seeds: entered from other code (branch / j / jal outside the function, lui + addiu/ori constants)
    hard = [set() for _ in gaps]
    soft = [set() for _ in gaps]
    for lo, hi in fns:
        for pc in range(lo, hi, 4):
            op = wd(pc)
            o = op >> 26
            if kind(op) == 'b':
                t = target(op, pc)
                if t is not None and not t & 3 and not lo <= t < hi:
                    gi = gap_of(t)
                    if gi >= 0:
                        hard[gi].add(t)
            elif o == 15:
                r, h = (op >> 16) & 31, (op & 0xffff) << 16
                for b in range(pc + 4, min(pc + 36, hi), 4):
                    q = wd(b)
                    if q >> 26 in (9, 13) and (q >> 21) & 31 == r:
                        v = (h + sx16(q & 0xffff)) & 0xffffffff if q >> 26 == 9 else h | (q & 0xffff)
                        if not v & 3:
                            gi = gap_of(v)
                            if gi >= 0:
                                hard[gi].add(v)
                        break
    for p in ptrs:
        gi = gap_of(p)
        if gi >= 0:
            soft[gi].add(p)

    def scan(s, ge, gs):
        """Flow scan from s inside [gs, ge): (visited pcs, call targets in the gap, has a switch jr) or None."""
        seen = set()
        calls = set()
        stack = [s]
        sw = False
        while stack:
            pc = stack.pop()
            while True:
                if pc in seen:
                    break
                if not gs <= pc < ge or pc & 3:
                    return None
                op = wd(pc)
                if not _valid_op(op):
                    return None
                # Zero words are valid nops but also .bss / empty data in the image: a run of 6 or more is not code.
                if op == 0 and (pc == s or all(wd(pc + 4 * i) == 0 for i in range(1, 6))):
                    return None
                seen.add(pc)
                if len(seen) > max_words:
                    return None
                if kind(op) != 'b':
                    if op >> 26 == 16 and op & 63 == 24:   # eret
                        break
                    pc += 4
                    continue
                d = wd(pc + 4)
                if not gs <= pc + 4 < ge or not _valid_op(d) or kind(d) == 'b':
                    return None
                seen.add(pc + 4)
                o = op >> 26
                rs, rt = (op >> 21) & 31, (op >> 16) & 31
                if o == 0:   # jr / jalr
                    if op & 63 == 9:
                        pc += 8
                        continue
                    if rs != 31:
                        sw = True
                    break
                t = target(op, pc)
                if o == 2:
                    if gs <= t < ge:
                        pc = t
                        continue
                    if not 0x100000 <= t < limit_hi:
                        return None
                    break
                if o == 3:
                    if not 0x100000 <= t < limit_hi:
                        return None
                    if gs <= t < ge:
                        calls.add(t)
                    pc += 8
                    continue
                if not gs <= t < ge:
                    return None
                link = (o == 1 and rt >= 16)
                always = (o in (4, 20) and rs == rt) or (o in (6, 22) and rs == 0) or (o == 1 and rs == 0 and rt in (1, 3, 17, 19))
                stack.append(t)
                if always and not link:
                    break
                pc += 8
        return seen, calls, sw

    out = []
    for gi, (gs, ge) in enumerate(gaps):
        first = gs
        while first < ge and wd(first) in PAD_WORDS:
            first += 4
        if first >= ge:
            continue
        starts = set()
        ext = []            # accepted (start, end) extents
        visited = set()
        done = set()
        queue = [first] + sorted(s for s in hard[gi] if s >= first)
        qi = 0
        while qi < len(queue):
            s = queue[qi]
            qi += 1
            if s & 3:
                continue
            if s in visited:
                starts.add(s)
                continue
            if s in done:
                continue
            done.add(s)
            r = scan(s, ge, gs)
            if r is None:
                # data words in front of the code at the gap start: retry a few words later
                if s == first and first + 4 - gs < 64 and first + 4 < ge:
                    queue.append(first + 4)
                continue
            seen, calls, sw = r
            if sw:
                # jump table: code pointers inside the extent are cases (flow), not entries
                top = max(seen) + 4
                grew = True
                while grew:
                    grew = False
                    for p in sorted(soft[gi]):
                        if s < p < top and p not in seen:
                            r2 = scan(p, ge, gs)
                            if r2 is not None:
                                seen |= r2[0]
                                calls |= r2[1]
                                top = max(top, max(r2[0]) + 4)
                                grew = True
            starts.add(s)
            visited |= seen
            ext.append((s, max(seen) + 4))
            queue.extend(sorted(calls))
        # code pointers not reached by flow: function pointers into unvisited code. A pointer into visited code is a
        # function start only right after a `jr ra` (the previous function's end), else a jump-table case.
        for p in sorted(soft[gi]):
            if p & 3 or p < first or p in done:
                continue
            if p in visited:
                if wd(p - 8) == 0x03e00008:
                    starts.add(p)
                continue
            r = scan(p, ge, gs)
            if r is not None:
                done.add(p)
                seen, calls, sw = r
                starts.add(p)
                visited |= seen
                ext.append((p, max(seen) + 4))
        ext.sort()
        comps = []
        for s, e in ext:
            if comps and s < comps[-1][1]:
                comps[-1][1] = max(comps[-1][1], e)
            else:
                comps.append([s, e])
        for cs, ce in comps:
            # a start must not fall between a branch and its delay slot
            st = sorted(a for a in starts if cs < a < ce and kind(wd(a - 4)) != 'b')
            st.insert(0, cs)
            for i, a in enumerate(st):
                b = st[i + 1] if i + 1 < len(st) else ce
                if b > a:
                    out.append((a, b))
    return out

# Functions the bounds list misses (flow-scanned code in the gaps between the listed functions, see discover_holes). They are
# compiled like the listed ones; HOLES marks them (those that contain calls use the dispatch form, see the crate loop).
HOLES = set()
if not A.no_holes:
    HOLES = set(discover_holes(fns, ptrs))
    fns = sorted(set(fns) | HOLES)
    print(f'discover_holes: {len(HOLES)} functions, {sum((h - l) // 4 for l, h in HOLES)} words')

# [PC] (speed) Also code addresses a function builds itself (lui + addiu / ori into the same register within the next
# 8 words) that point inside that function: a continuation stored for a later `jr` (Kerwan tie_proc 0x282218 keeps
# 0x282b30 / 0x283310 in scratchpad and jumps there). Without a label there the jump left the compiled code and the
# rest of the call ran in the interpreter (~25% of the EE thread on Kerwan). Extra labels only split blocks.
for _lo, _hi in fns:
    for _a in range(_lo, _hi, 4):
        _w = W[_a >> 2] if (_a >> 2) < len(W) else 0
        if _w >> 26 != 15:
            continue
        _r, _h = (_w >> 16) & 31, (_w & 0xffff) << 16
        for _b in range(_a + 4, min(_a + 36, _hi), 4):
            _o = W[_b >> 2]
            if _o >> 26 in (9, 13) and (_o >> 21) & 31 == _r:
                _v = (_h + ((_o & 0xffff) - 0x10000 if (_o >> 26 == 9 and _o & 0x8000) else (_o & 0xffff))) & 0xffffffff
                if _o >> 26 == 13:
                    _v = _h | (_o & 0xffff)
                if _lo < _v < _hi and not _v & 3:
                    ptrs.add(_v)
                break





def g(r):
    return '0u64' if r == 0 else f'c.g({r})'


def g32(r):
    return '0u32' if r == 0 else f'(c.g({r}) as u32)'


def alu(op, pc):
    """Inline code for a non-memory, non-control instruction, or None to run it through exec."""
    o = op >> 26
    rs, rt, rd, sa, f = (op >> 21) & 31, (op >> 16) & 31, (op >> 11) & 31, (op >> 6) & 31, op & 63
    imm = sx16(op & 0xffff)
    immu = op & 0xffff
    if o in (47, 51):
        return ''
    if o == 0:
        if f == 15:
            return ''
        a, b = g(rs), g(rt)
        e = {
            0: f'sx32({g32(rt)} << {sa})', 2: f'sx32({g32(rt)} >> {sa})', 3: f'sx32((({g32(rt)} as i32) >> {sa}) as u32)',
            4: f'sx32({g32(rt)} << ({a} & 31))', 6: f'sx32({g32(rt)} >> ({a} & 31))',
            7: f'sx32((({g32(rt)} as i32) >> ({a} & 31)) as u32)',
            16: 'c.hi as u64', 18: 'c.lo as u64',
            20: f'{b} << ({a} & 63)', 22: f'{b} >> ({a} & 63)', 23: f'(({b} as i64) >> ({a} & 63)) as u64',
            32: f'sx32({g32(rs)}.wrapping_add({g32(rt)}))', 33: f'sx32({g32(rs)}.wrapping_add({g32(rt)}))',
            34: f'sx32({g32(rs)}.wrapping_sub({g32(rt)}))', 35: f'sx32({g32(rs)}.wrapping_sub({g32(rt)}))',
            36: f'{a} & {b}', 37: f'{a} | {b}', 38: f'{a} ^ {b}', 39: f'!({a} | {b})',
            42: f'(({a} as i64) < ({b} as i64)) as u64', 43: f'({a} < {b}) as u64',
            44: f'{a}.wrapping_add({b})', 45: f'{a}.wrapping_add({b})', 46: f'{a}.wrapping_sub({b})', 47: f'{a}.wrapping_sub({b})',
            56: f'{b} << {sa}', 58: f'{b} >> {sa}', 59: f'(({b} as i64) >> {sa}) as u64',
            60: f'{b} << {sa + 32}', 62: f'{b} >> {sa + 32}', 63: f'(({b} as i64) >> {sa + 32}) as u64',
        }.get(f)
        if e is not None:
            return '' if rd == 0 else f'c.s({rd}, {e});'
        if f == 10:
            return '' if rd == 0 else f'if {b} == 0 {{ c.s({rd}, {a}); }}'
        if f == 11:
            return '' if rd == 0 else f'if {b} != 0 {{ c.s({rd}, {a}); }}'
        if f in (24, 25, 26, 27):
            if f == 24:
                return (f'{{ let p = ({g32(rs)} as i32 as i64) * ({g32(rt)} as i32 as i64); c.set_lh(sx32(p as u32), sx32((p >> 32) as u32));'
                        + (f' c.s({rd}, sx32(p as u32));' if rd else '') + ' }')
            if f == 25:
                return (f'{{ let p = ({g32(rs)} as u64) * ({g32(rt)} as u64); c.set_lh(sx32(p as u32), sx32((p >> 32) as u32));'
                        + (f' c.s({rd}, sx32(p as u32));' if rd else '') + ' }')
            return f'{{ let (l, h) = {"div32" if f == 26 else "divu32"}({g32(rs)}, {g32(rt)}); c.set_lh(l, h); }}'
        return None
    e = {
        8: f'sx32({g32(rs)}.wrapping_add({u32(imm):#x}))', 9: f'sx32({g32(rs)}.wrapping_add({u32(imm):#x}))',
        10: f'(({g(rs)} as i64) < {imm}i64) as u64', 11: f'({g(rs)} < {imm & 0xffffffffffffffff:#x}u64) as u64',
        12: f'{g(rs)} & {immu:#x}', 13: f'{g(rs)} | {immu:#x}', 14: f'{g(rs)} ^ {immu:#x}',
        15: f'sx32({u32(immu << 16):#x})',
        24: f'{g(rs)}.wrapping_add({imm & 0xffffffffffffffff:#x})', 25: f'{g(rs)}.wrapping_add({imm & 0xffffffffffffffff:#x})',
    }.get(o)
    if e is not None:
        return '' if rt == 0 else f'c.s({rt}, {e});'
    if o == 17 and rs == 16:
        fs, ft, fd = (op >> 11) & 31, (op >> 16) & 31, (op >> 6) & 31
        A, B = f'c.f[{fs}]', f'c.f[{ft}]'
        e = {0: f'fpu::add({A}, {B})', 1: f'fpu::sub({A}, {B})', 2: f'fpu::mul({A}, {B})', 3: f'fpu::div({A}, {B}, c.div_csr)',
             4: f'fpu::sqrt({B}, c.div_csr)', 5: f'{A} & 0x7fff_ffff', 6: A, 7: f'{A} ^ 0x8000_0000',
             22: f'fpu::rsqrt({A}, {B}, c.div_csr)', 28: f'fpu::add(c.facc, fpu::mul({A}, {B}))',
             29: f'fpu::sub(c.facc, fpu::mul({A}, {B}))', 36: f'fpu::cvt_w({A})', 40: f'fpu::max({A}, {B})', 41: f'fpu::min({A}, {B})'}.get(f)
        if e is not None:
            return f'c.f[{fd}] = {e};'
        e = {24: f'fpu::add({A}, {B})', 25: f'fpu::sub({A}, {B})', 26: f'fpu::mul({A}, {B})',
             30: f'fpu::add(c.facc, fpu::mul({A}, {B}))', 31: f'fpu::sub(c.facc, fpu::mul({A}, {B}))'}.get(f)
        if e is not None:
            return f'c.facc = {e};'
        if f in (48, 50, 52, 54):
            cmp = {48: 'false', 50: 'x == y', 52: 'x < y', 54: 'x <= y'}[f]
            return (f'{{ let (x, y) = (fpu::fz({A}), fpu::fz({B})); if {cmp} {{ c.fcr31 |= 1 << 23 }} else {{ c.fcr31 &= !(1 << 23) }} }}')
        return None
    if o == 17 and rs == 20 and f == 32:
        return f'c.f[{(op >> 6) & 31}] = fpu::cvt_s(c.f[{(op >> 11) & 31}]);'
    if o == 18 and rs == 1:
        return '' if rt == 0 else f'c.r[{rt}] = uw4(c.vf[{rd}]);'
    if o == 18 and rs == 5:
        return '' if rd == 0 else f'c.vf[{rd}] = w4(c.r[{rt}]);'
    if o == 18 and rs == 2:
        v = 'c.vi[20] & 0x7f_ffff' if rd == 20 else f'c.vi[{rd}]'
        return '' if rt == 0 else f'c.s({rt}, sx32({v}));'
    if o == 17 and rs == 0:
        return '' if rt == 0 else f'c.s({rt}, sx32(c.f[{(op >> 11) & 31}]));'
    if o == 17 and rs == 4:
        return f'c.f[{(op >> 11) & 31}] = {g(rt)} as u32;'
    return None


LOADS = {32: ('rd8', lambda v: f'{v} as i8 as i64 as u64'), 33: ('rd16', lambda v: f'{v} as i16 as i64 as u64'),
         35: ('rd32', lambda v: f'sx32({v})'), 36: ('rd8', lambda v: f'{v} as u64'), 37: ('rd16', lambda v: f'{v} as u64'),
         39: ('rd32', lambda v: f'{v} as u64'), 55: ('rd64', lambda v: v)}
STORES = {40: ('wr8', lambda r: f'{g(r)} as u8'), 41: ('wr16', lambda r: f'{g(r)} as u16'),
          43: ('wr32', lambda r: f'{g(r)} as u32'), 63: ('wr64', lambda r: g(r))}


def mem_ins(op, pc, slow):
    """A load/store: plain RAM / scratchpad inline, anything else through `slow` (the interpreter's access)."""
    o = op >> 26
    rs, rt = (op >> 21) & 31, (op >> 16) & 31
    imm = u32(sx16(op & 0xffff))
    ea = f'{g32(rs)}.wrapping_add({imm:#x})' if rs else f'{imm:#x}u32'
    L = [f'let a = {ea};']
    if o in LOADS:
        fn, conv = LOADS[o]
        st = f'c.s({rt}, {conv("v")});' if rt else ''
        L.append(f'match c.mem.f{fn}(a) {{ Some(v) => {{ {st} }} None => {{ {slow} }} }}')
    elif o in (30, 54):
        st = (f'c.r[{rt}] = v;' if rt else '') if o == 30 else f'c.vf[{rt}] = w4(v);'
        L.append(f'match c.mem.frd128(a & !15) {{ Some(v) => {{ {st} }} None => {{ {slow} }} }}')
    elif o == 49:
        L.append(f'match c.mem.frd32(a) {{ Some(v) => {{ c.f[{rt}] = v; }} None => {{ {slow} }} }}')
    elif o in STORES:
        fn, val = STORES[o]
        L.append(f'if !c.mem.f{fn}(a, {val(rt)}) {{ {slow} }}')
    elif o == 31:
        L.append(f'if !c.mem.fwr128(a & !15, c.r[{rt}]) {{ {slow} }}')
    elif o == 57:
        L.append(f'if !c.mem.fwr32(a, c.f[{rt}]) {{ {slow} }}')
    elif o == 62:
        L.append(f'if !c.mem.fwr128(a & !15, uw4(c.vf[{rt}])) {{ {slow} }}')
    else:
        # LWL/LWR/LDL/LDR/SWL/SWR/SDL/SDR on the aligned word (cpu.rs `exec`'s merge masks).
        n = 8 if o in (26, 27, 44, 45) else 4
        w = 'rd64' if n == 8 else 'rd32'
        gt = g(rt) if n == 8 else f'({g32(rt)})'
        top = 56 if n == 8 else 24
        if o in (26, 27, 34, 38):
            M = {26: 'LDL_MASK', 27: 'LDR_MASK', 34: 'LWL_MASK', 38: 'LWR_MASK'}[o]
            v = f'({gt} & {M}[sh]) | ' + (f'(m << ({top} - sh * 8))' if o in (26, 34) else '(m >> (sh * 8))')
            if not rt:
                st = ''
            elif n == 8:
                st = f'c.s({rt}, {v});'
            elif o == 34:
                st = f'c.s({rt}, sx32({v}));'
            else:
                st = f'let v = {v}; if sh == 0 {{ c.s({rt}, sx32(v)); }} else {{ c.s({rt}, ({g(rt)} & !0xffff_ffff) | v as u64); }}'
            L.append(f'match c.mem.f{w}(a & !{n - 1}) {{ Some(m) => {{ let sh = (a & {n - 1}) as usize; {st} }} None => {{ {slow} }} }}')
        else:
            M = {42: 'SWL_MASK', 44: 'SDL_MASK', 45: 'SDR_MASK', 46: 'SWR_MASK'}[o]
            v = (f'({gt} >> ({top} - sh * 8))' if o in (42, 44) else f'({gt} << (sh * 8))') + f' | (m & {M}[sh])'
            L.append(f'match c.mem.f{w}(a & !{n - 1}) {{ Some(m) => {{ let sh = (a & {n - 1}) as usize; '
                     f'c.mem.fwr{n * 8}(a & !{n - 1}, {v}); }} None => {{ {slow} }} }}')
    s = ' '.join(L)
    return s.replace('c.mem.f', 'c.mem.d' if DIRECT else 'c.mem.x') if OUTLINE else s


def slow_enc(pc, rem, last, brl):
    """Compact slow-path calls (--shared-slow): (rem, last, brl) of a block as one number, or None when they do not fit the usual shape. `rem` more
    instructions follow, last = pc + 4 * (rem + d) with d = 1 when the block ends in a branch plus its delay slot, and brl is
    either 0 or the branch just before `last` (branch-likely). The helpers rebuild last and brl from pc and this number."""
    d = (last - pc) // 4 - rem
    if rem < 0 or rem > 0xffff or (last - pc) % 4 or d not in (0, 1):
        return None
    if brl and brl != last - 4:
        return None
    return rem | d << 16 | (2 << 16 if brl else 0)


def pl_call(L, n, last, brl):
    """The cold block-poll call (the poll itself is `if !c.poll(n)`)."""
    e = slow_enc(L, n - 1, last, brl)
    if e is not None:
        return f"match pq(c, {L:#x}, {e:#x}) {{ 1 => break 'b, _ => return }}"
    return f"match pl(c, {L:#x}, {last:#x}, {brl:#x}) {{ 1 => break 'b, _ => return }}"


def ins_code(op, pc, k, nxt, rem, last, brl):
    """Code for the k-th instruction (from 1) of a block of which `rem` follow; `nxt` is the PC after it.
    A hardware access runs through the interpreter with its exact count, and if an event is now due before the block
    ends the rest of the block is interpreted too."""
    if kind(op) == 'm':
        if SHARED:
            if rem and nxt == f'{pc + 4:#x}u32' and slow_enc(pc, rem, last, brl) is not None:
                # compact call: nxt = pc + 4;  (sq: 4 register arguments instead of 8) rem, last and brl ride in the high half of the op word
                slow = f"match sq(c, {op | (slow_enc(pc, rem, last, brl) << 32):#x}, {pc:#x}, t + {k}) {{ 0 => {{}} 1 => break 'b, _ => return }}"
            elif not rem and nxt == f'{pc + 4:#x}u32':
                slow = f'hq(c, {op:#x}, {pc:#x}, t + {k});'
            elif rem:
                slow = f"match sp(c, {op:#x}, {pc:#x}, t + {k}, {nxt}, {rem}, {last:#x}, {brl:#x}) {{ 0 => {{}} 1 => break 'b, _ => return }}"
            else:
                slow = f'hw(c, {op:#x}, {pc:#x}, t + {k}, {nxt});'
        else:
            chk = f" if !c.go({rem}) {{ if !c.slow({last:#x}, {brl:#x}) {{ return; }} break 'b; }}" if rem else ''
            slow = f'c.hw({op:#010x}, {pc:#x}, t + {k}, {nxt});{chk}'
        return mem_ins(op, pc, slow)
    s = alu(op, pc)
    if s is None:
        s = vu_code(op, pc)
    if s is None:
        o = op >> 26
        rs, rt, rd, sa = (op >> 21) & 31, (op >> 16) & 31, (op >> 11) & 31, (op >> 6) & 31
        if o == 18 and rs >= 16:
            s = f'c.vu_special{2 if op & 0x3f >= 60 else 1}({op:#010x}, {pc:#x});'
        elif o == 18:
            s = f'c.cop2({op:#010x}, {pc:#x}, {rs}, {rt}, {rd});'
        elif o == 28:
            s = f'c.mmi({op:#010x}, {pc:#x}, {rs}, {rt}, {rd}, {sa});'
        else:
            s = f'c.exec({op:#010x}, {pc:#x});'
    return s


# ---- VU0 macro mode (COP2) ----
VK = ['ADD', 'SUB', 'MADD', 'MSUB', 'MAX', 'MIN', 'MUL']           # cpu.rs BC_KIND order
VK_NAME = {'ADD': 'add', 'SUB': 'sub', 'MADD': 'madd', 'MSUB': 'msub', 'MAX': 'max', 'MIN': 'mini', 'MUL': 'mul'}
QI = [('MUL', 0), ('MAX', 1), ('MUL', 1), ('MIN', 1), ('ADD', 0), ('MADD', 0),
      ('ADD', 1), ('MADD', 1), ('SUB', 0), ('MSUB', 0), ('SUB', 1), ('MSUB', 1)]
FULLK = ['ADD', 'MADD', 'MUL', 'MAX', 'SUB', 'MSUB', None, 'MIN']


def vu_upper_form(op):
    """A VU0 macro FMAC op as (kind, dest, dst, fs, ft operand source, operand text), dst 32 = ACC; None for others."""
    if op >> 26 != 18 or (op >> 21) & 31 < 16:
        return None
    dest = (op >> 21) & 15
    ft, fs, fd = (op >> 16) & 31, (op >> 11) & 31, (op >> 6) & 31
    bc = op & 3
    f = op & 0x3f
    if f < 0x3c:
        if f < 28:
            return VK[f >> 2], dest, fd, fs, f'[c.vf[{ft}][{bc}]; 4]', f'vf{ft}{"xyzw"[bc]}'
        if f < 40:
            k, isi = QI[f - 28]
            return k, dest, fd, fs, f'[c.vi[{21 if isi else 22}]; 4]', 'I' if isi else 'Q'
        if f < 48 and FULLK[f - 40]:
            return FULLK[f - 40], dest, fd, fs, f'c.vf[{ft}]', f'vf{ft}'
        return None
    x = (op & 3) | ((op >> 4) & 0x7c)
    if x < 16 or 24 <= x < 28:
        return VK[x >> 2], dest, 32, fs, f'[c.vf[{ft}][{bc}]; 4]', f'vf{ft}{"xyzw"[bc]}'
    if x in (28, 30) or 32 <= x < 40:
        k, isi = QI[x - 28]
        return k, dest, 32, fs, f'[c.vi[{21 if isi else 22}]; 4]', 'I' if isi else 'Q'
    if x in (40, 41, 42, 44, 45):
        return FULLK[x - 40], dest, 32, fs, f'c.vf[{ft}]', f'vf{ft}'
    return None


def dest_s(d):
    return ''.join(ch for i, ch in enumerate('xyzw') if d & (8 >> i))


def vu_code(op, pc):
    u = vu_upper_form(op)
    if u is None:
        return None
    k, dest, dst, fs, tv, _ = u
    mac = 'false' if k in ('MAX', 'MIN') else 'true'
    if VUCLEAN:
        ps, pt, pa, live = VUA.get(pc, (False, False, False, True))
        f = 0 if k in ('MAX', 'MIN') else (1 if live else 2)
        fm = f'c.fmac_c::<{str(ps).lower()}, {str(pt).lower()}, {str(pa).lower()}>' if (ps or pt or pa) and k not in ('MAX', 'MIN') else 'c.fmac'
        if pc in VUG:
            fm = 'c.fmac_g::<true>'
        return f'let v = {fm}({k}, c.vf[{fs}], {tv}); c.vu_out_f::<{f}>({dst}, v, {dest:#x});'
    return f'let v = c.fmac({k}, c.vf[{fs}], {tv}); c.vu_out({dst}, v, {dest:#x}, {mac});'


def vu_guard_regions(seq):
    """--vu-guard: runs of a block's straight-line instructions that hold at least VUGUARD_MIN COP2 FMAC ops and
    otherwise only inline integer instructions (no memory op, no FPU, no call out of the block), trimmed to start and end
    at a FMAC op. Inside one, a single fpu::ChopGuard (MXCSR round toward zero + DAZ + FTZ) serves every FMAC op, which then
    takes the FAST form `fmac_g::<true>` (ee-core's reference ops: the same bits as the MXCSR-free ones). Nothing but
    those ops' own SSE code runs under the guard. Returns (pcs inside, first pcs, last pcs)."""
    inside, starts, ends = set(), set(), set()

    def ok(pc, op):
        if vu_upper_form(op) is not None:
            return True
        o = op >> 26
        if kind(op) != 'n' or o in (17, 28) or (o == 18 and (op >> 21) & 31 not in (1, 2, 5)):
            return False
        return alu(op, pc) is not None

    run = []
    for pc, op in seq + [(None, None)]:
        if pc is not None and ok(pc, op):
            run.append((pc, op))
            continue
        f = [j for j, (p, o) in enumerate(run) if vu_upper_form(o) is not None]
        if len(f) >= VUGUARD_MIN:
            r = run[f[0]:f[-1] + 1]
            inside.update(p for p, _ in r)
            starts.add(r[0][0])
            ends.add(r[-1][0])
        run = []
    return inside, starts, ends


def vu_annot(ops, likely=False):
    """--vu-clean: per COP2 FMAC op of a block (its straight-line instructions in order, delay slot included):
    (fs prepared, ft prepared, ACC prepared, flags live). A lane is prepared when an ADD/SUB/MUL/MADD/MSUB result was
    the last write to it in this block (`vprep` leaves such a value unchanged); an operand counts when every lane the op
    writes is (ft broadcast: its one lane). Anything but a FMAC op or a plain non-COP2, non-memory instruction forgets
    everything (a memory op can leave the block or reach hardware that runs VU0). Flags are dead when a later FMAC op
    of the block that writes them comes before any such instruction and the block end."""
    clean = {}
    res = {}
    for pc, op in ops:
        u = vu_upper_form(op)
        if u is None:
            if kind(op) != 'n' or op >> 26 == 18:
                clean = {}
            continue
        k, dest, dst, fs, tv, ts = u
        need = dest & 15
        ps = need != 0 and clean.get(fs, 0) & need == need
        if ts in ('I', 'Q'):
            pt = False
        elif ts[-1] in 'xyzw':
            ft = int(ts[2:-1])
            pt = clean.get(ft, 0) & (8 >> 'xyzw'.index(ts[-1])) != 0
        else:
            pt = need != 0 and clean.get(int(ts[2:]), 0) & need == need
        pa = k in ('MADD', 'MSUB') and need != 0 and clean.get(32, 0) & need == need
        res[pc] = [ps, pt, pa, True]
        if dst != 0 and need:
            if k in ('MAX', 'MIN'):
                clean[dst] = clean.get(dst, 0) & ~need
            else:
                clean[dst] = clean.get(dst, 0) | need
    live = True
    for j, (pc, op) in reversed(list(enumerate(ops))):
        if likely and j == len(ops) - 1:
            if pc in res:
                res[pc][3] = True
            continue
        u = vu_upper_form(op)
        if u is None:
            if kind(op) != 'n' or op >> 26 == 18:
                live = True
            continue
        if u[0] in ('MAX', 'MIN'):
            continue
        res[pc][3] = live
        live = False
    return {pc: tuple(v) for pc, v in res.items()}


def vu_dis(op):
    u = vu_upper_form(op)
    if u is None:
        return None
    k, dest, dst, fs, _, ts = u
    name = VK_NAME[k] + ('a' if dst == 32 else '')
    if ts in ('I', 'Q'):
        name += ts.lower()
    elif ts[-1] in 'xyzw':
        name += ts[-1]
    d = 'ACC' if dst == 32 else f'vf{dst}'
    return f'v{name}.{dest_s(dest)} {d}, vf{fs}, {ts}'


def cond(op, pc):
    """Branch condition (evaluated before the delay slot), and whether it is a likely branch, and the link register."""
    o = op >> 26
    rs, rt = (op >> 21) & 31, (op >> 16) & 31
    if o == 1:
        c = f'({g(rs)} as i64) < 0' if rt & 1 == 0 else f'({g(rs)} as i64) >= 0'
        return c, rt in (2, 3, 18, 19), 31 if rt >= 16 else None
    if o in (4, 20):
        return f'{g(rs)} == {g(rt)}', o == 20, None
    if o in (5, 21):
        return f'{g(rs)} != {g(rt)}', o == 21, None
    if o in (6, 22):
        return f'({g(rs)} as i64) <= 0', o == 22, None
    if o in (7, 23):
        return f'({g(rs)} as i64) > 0', o == 23, None
    if o == 17:
        b = 'c.fcr31 & (1 << 23) != 0'
        return (f'!({b})' if rt & 1 == 0 else b), rt >= 2, None
    if o == 18:
        return ('true' if rt & 1 == 0 else 'false'), rt >= 2, None
    raise ValueError


def dis(op, pc):
    try:
        import capstone
        md = dis.md
    except AttributeError:
        import capstone
        dis.md = md = capstone.Cs(capstone.CS_ARCH_MIPS, capstone.CS_MODE_MIPS64 | capstone.CS_MODE_LITTLE_ENDIAN)
    vd = vu_dis(op)
    if vd:
        return vd
    i = list(md.disasm(struct.pack('<I', op), pc))
    return f'{i[0].mnemonic} {i[0].op_str}' if i else f'.word {op:#010x}'


def fn_labels(lo, hi):
    labels = {lo}
    if lo <= VBLANK_WAIT < hi:
        labels.add(VBLANK_WAIT)  # the frame callback runs here (rec/mod.rs vblank_hook)
    for pc in range(lo, hi, 4):
        op = word(pc)
        k = kind(op)
        if k == 'b':
            t = target(op, pc)
            if t is not None and lo <= t < hi:
                labels.add(t)
            labels.add(pc + 8)
        elif k == 'x':
            labels.add(pc + 4)
        if pc in ptrs:
            labels.add(pc)
    return sorted(l for l in labels if lo <= l < hi and l not in stops)


def links(op):
    """Whether a control op links (a call)."""
    o = op >> 26
    return o == 3 or (o == 0 and op & 63 == 9) or (o == 1 and ((op >> 16) & 31) >= 16)


def call(t, ret):
    """A call to `t` returning to `ret` (state already at `t`); resolved by `resolve_calls`."""
    return f'@CALL:{t:x}:{ret:x}@'


def resolve_calls(line):
    """Direct when `t` is a compiled entry of this crate (any label of a dispatch-form function), else the table."""
    def one(m):
        t, ret = int(m.group(1), 16), int(m.group(2), 16)
        f = label_fn.get(t)
        return f'{f}(c);' if f else f'c.call_dyn({t:#x}, {ret:#x});'
    return CALL_RE.sub(one, line)


jr_cache = {}


def jr_targets(P):
    """Targets of a `jr` that is not a return: the GCC switch table (lui/addu/lw feeding the jr, its size from the
    sltiu bound check), else every code-pointer label of the function."""
    lo, hi, lset = jr_cache['fn']
    r = (word(P) >> 21) & 31
    lw = lui = n = None
    adds = {}
    for a in range(P - 4, max(lo, P - 48) - 4, -4):
        op = word(a)
        o = op >> 26
        if lw is None and o == 35 and (op >> 16) & 31 == r:
            lw = (op >> 21) & 31, sx16(op & 0xffff)
        elif lw is not None and lui is None and o == 9 and (op >> 16) & 31 == (op >> 21) & 31:
            adds.setdefault((op >> 16) & 31, sx16(op & 0xffff))
        elif lw is not None and lui is None and o == 15:
            lui = ((op & 0xffff) << 16) + adds.get((op >> 16) & 31, 0)
        elif o == 11 and n is None:
            n = op & 0xffff
    if lw is not None and lui is not None:
        base = u32(lui + lw[1])
        out = []
        for i in range(n if n else 512):
            if base + 4 * i + 4 > len(ram):
                break
            t = word(base + 4 * i)
            if not (lo <= t < hi and t in lset):
                if n:
                    return [l for l in lset if l in ptrs and l != lo]
                break
            out.append(t)
        if out:
            return out
    return sorted(l for l in lset if l in ptrs and l != lo)


def compile_fn(lo, hi, labels):
    global jtab, VUA, VUG
    lset = set(labels)
    jtab = [l for l in labels if l in ptrs and l != lo]
    jr_cache.clear()
    jr_cache['fn'] = (lo, hi, lset)
    arms = []
    for L in labels:
        seq, pc = [], L
        while True:
            if pc >= hi or pc in stops or (pc != L and pc in lset):
                term = ('fall', pc)
                break
            op = word(pc)
            kd = kind(op)
            if kd == 'x':
                term = ('xop', pc)
                break
            if kd == 'b':
                d = word(pc + 4)
                if kind(d) in ('b', 'x') or pc + 4 >= hi or pc + 4 in stops:
                    term = ('pair', pc)
                else:
                    term = ('br', pc)
                break
            seq.append((pc, op))
            pc += 4
        tk, P = term
        k = len(seq)
        if tk == 'fall':
            n, last, brl = k, P - 4, 0
        elif tk == 'xop':
            n, last, brl = k, P, 0
        else:
            bop = word(P)
            likely = bop >> 26 not in (0, 2, 3) and cond(bop, P)[1]
            n = k + 2 if tk == 'br' else k
            last, brl = P + 4, (P if likely else 0)
        body = []
        # A likely branch's delay slot runs only when taken: it may use what the block knows, but cannot make an earlier
        # op's flags dead.
        VUA = vu_annot(seq + ([(P + 4, word(P + 4))] if tk == 'br' else []), tk == 'br' and likely) if VUCLEAN else {}
        VUG, gs, ge = vu_guard_regions(seq) if VUGUARD else (set(), set(), set())
        for i, (pc, op) in enumerate(seq):
            if not NOCOM:
                body.append(f'// {pc:08x}: {dis(op, pc)}')
            if pc in gs:
                body.append('{ let _g = fpu::ChopGuard::new();')
            s = ins_code(op, pc, i + 1, f'{pc + 4:#x}u32', n - i - 1, last, brl)
            if s:
                body.append(s)
            if pc in ge:
                body.append('}')
        after = 'continue;'
        ex = ('', [P] if tk == 'fall' else [P + 4], tk == 'xop')
        if tk == 'pair':
            bop = word(P)
            bo = bop >> 26
            if bo == 0:
                ex = ('', None, False) if (bop >> 21) & 31 == 31 and bop & 63 == 8 else ('', jr_targets(P), True)
            else:
                t = target(bop, P)
                ex = ('', [t] if bo in (2, 3) else [t, P + 8], True)
        if tk in ('xop', 'pair'):
            body.append(f'c.icount = t + {k}; c.pc = {P:#x}; c.npc = {P + 4:#x};')
            body.append(f'if !c.slow({last:#x}, {brl:#x}) {{ return; }}')
            if tk == 'pair' and links(word(P)):
                after = f'if c.pc != {P + 8:#x} {{ let j = c.pc; c.call_dyn(j, {P + 8:#x}); }} continue;'
                ex = (f'if c.pc != {P + 8:#x} {{ let j = c.pc; c.call_dyn(j, {P + 8:#x}); }}', [P + 8], True)
        elif tk == 'fall':
            body.append(f'c.icount = t + {k}; c.pc = {P:#x}; c.npc = {P + 4:#x};')
        else:
            op, d = word(P), word(P + 4)
            o = op >> 26
            if not NOCOM:
                body.append(f'// {P:08x}: {dis(op, P)}')
            dcomment = '' if NOCOM else f'// {P + 4:08x}: {dis(d, P + 4)}'
            if o in (2, 3):
                t = target(op, P)
                if o == 3:
                    body.append(f'c.s(31, {P + 8:#x});')
                body.append(dcomment) if dcomment else None
                body.append(ins_code(d, P + 4, k + 2, f'{t:#x}u32', 0, last, brl))
                body.append(f'c.icount = t + {k + 2}; c.pc = {t:#x}; c.npc = {t + 4:#x};')
                ex = ('', [t], False)
                if o == 3:
                    after = call(t, P + 8) + ' continue;'
                    ex = (call(t, P + 8), [P + 8], True)
            elif o == 0:
                rs, rd = (op >> 21) & 31, (op >> 11) & 31
                body.append(f'let j = {g32(rs)};')
                if op & 63 == 9 and rd:
                    body.append(f'c.s({rd}, {P + 8:#x});')
                body.append(dcomment) if dcomment else None
                body.append(ins_code(d, P + 4, k + 2, 'j', 0, last, brl))
                body.append(f'c.icount = t + {k + 2}; c.pc = j; c.npc = j.wrapping_add(4);')
                ex = ('', jr_targets(P), True)
                if op & 63 == 9:
                    after = f'let j = c.pc; c.call_dyn(j, {P + 8:#x}); continue;'
                    ex = (f'let j = c.pc; c.call_dyn(j, {P + 8:#x});', [P + 8], True)
                elif rs == 31:
                    after = 'return;'
                    ex = ('', None, False)
            else:
                cnd, likely, link = cond(op, P)
                t = target(op, P)
                body.append(f'let b = {cnd};')
                if link:
                    body.append(f'c.s(31, {P + 8:#x});')
                body.append(dcomment) if dcomment else None
                if likely:
                    ds = ins_code(d, P + 4, k + 2, f'{t:#x}u32', 0, last, brl)
                    body.append(f'if b {{ {ds} c.icount = t + {k + 2}; c.pc = {t:#x}; c.npc = {t + 4:#x}; }} '
                                f'else {{ c.icount = t + {k + 1}; c.pc = {P + 8:#x}; c.npc = {P + 12:#x}; }}')
                else:
                    body.append(ins_code(d, P + 4, k + 2, f'(if b {{ {t:#x}u32 }} else {{ {P + 8:#x}u32 }})', 0, last, brl))
                    body.append(f'c.icount = t + {k + 2}; let nx = if b {{ {t:#x}u32 }} else {{ {P + 8:#x}u32 }}; '
                                f'c.pc = nx; c.npc = nx.wrapping_add(4);')
                rs_, rt_ = (op >> 21) & 31, (op >> 16) & 31
                always = (o in (4, 20) and rs_ == rt_) or (o in (6, 22) and rs_ == 0) or                     (o == 1 and rs_ == 0 and rt_ in (1, 3, 17, 19))
                ex = ('', [t] if always else [t, P + 8], False)
                if link and t != P + 8:
                    after = f'if c.pc == {t:#x} {{ {call(t, P + 8)} }} continue;'
                    ex = (f'if c.pc == {t:#x} {{ {call(t, P + 8)} }}', [P + 8], True)
        arms.append((L, n, last, brl, body, after, ex))
    return arms


def structure(lo, arms):
    """Structured control flow for a function entered only at `lo` (Ramsey, "Beyond Relooper"): loops for loop
    headers, labeled blocks for merge points, if/match on the PC after each block. None when the graph is
    irreducible or too deep (the function keeps the dispatch form)."""
    node = {a[0]: a for a in arms}
    if lo not in node:
        why['stub entry'] = why.get('stub entry', 0) + 1
        return None
    succ = {}
    for a in arms:
        tg = a[6][1]
        out = []
        for t in tg or []:
            if t in node and t not in out:
                out.append(t)
        succ[a[0]] = out
    order, seen, st = [], {lo}, [(lo, iter(succ[lo]))]
    while st:
        v, it = st[-1]
        w = next(it, None)
        if w is None:
            order.append(v)
            st.pop()
        elif w not in seen:
            seen.add(w)
            st.append((w, iter(succ[w])))
    rpo = order[::-1]
    num = {v: i for i, v in enumerate(rpo)}
    preds = {v: [] for v in rpo}
    for u in rpo:
        for v in succ[u]:
            preds[v].append(u)
    idom = {lo: lo}

    def meet(a, b):
        while a != b:
            while num[a] > num[b]:
                a = idom[a]
            while num[b] > num[a]:
                b = idom[b]
        return a
    changed = True
    while changed:
        changed = False
        for v in rpo[1:]:
            ps = [p for p in preds[v] if p in idom]
            d = ps[0]
            for p in ps[1:]:
                d = meet(p, d)
            if idom.get(v) != d:
                idom[v] = d
                changed = True

    def dominates(a, b):
        while b != a and b != lo:
            b = idom[b]
        return b == a
    for u in rpo:
        for v in succ[u]:
            if num[v] <= num[u] and not dominates(v, u):
                why['irreducible'] = why.get('irreducible', 0) + 1
                return None
    loops = {v for u in rpo for v in succ[u] if num[v] <= num[u]}
    merge = {v for v in rpo if sum(1 for p in preds[v] if num[p] < num[v]) >= 2}
    kids = {v: [] for v in rpo}
    for v in rpo[1:]:
        kids[idom[v]].append(v)
    out = []
    MAXD = 300

    def emit(ind, line):
        if ind > MAXD:
            raise RecursionError
        out.append('    ' * ind + line)

    def branch(src, t, ind):
        if t not in num:
            emit(ind, 'c.tail(); return;')
        elif num[t] <= num[src]:
            emit(ind, f"continue 'l_{t:x};")
        elif t in merge:
            emit(ind, f"break 'm_{t:x};")
        else:
            tree(t, ind)

    def tree(x, ind):
        ys = sorted((y for y in kids[x] if y in merge), key=lambda y: num[y], reverse=True)
        if x in loops:
            emit(ind, f"'l_{x:x}: loop {{")
            within(x, ys, ind + 1)
            emit(ind, '}')
        else:
            within(x, ys, ind)

    def within(x, ys, ind):
        if ys:
            emit(ind, f"'m_{ys[0]:x}: {{")
            within(x, ys[1:], ind + 1)
            emit(ind, '}')
            tree(ys[0], ind)
            return
        L, n, last, brl, body, after, (pre, tg, check) = node[x]
        if not NOCOM:
            emit(ind, f'// {x:08x}')
        if x == VBLANK_WAIT:
            emit(ind, 'if c.vsync_armed && !c.vblank_hook() { return; }')
        if poll_head(x):
            emit(ind, f'c.ff(1, {x:#x});')
        emit(ind, "'b: {")
        if n:
            emit(ind + 1, f"if !c.poll({n}) {{ " + (pl_call(x, n, last, brl) if SHARED else f"if c.pc != {x:#x} || !c.slow({last:#x}, {brl:#x}) {{ return; }} break 'b;") + " }")
        emit(ind + 1, 'let t = c.icount;')
        for b in body:
            emit(ind + 1, b)
        emit(ind, '}')
        if pre:
            emit(ind, pre)
        if tg is None:
            emit(ind, 'return;')
            return
        ts = []
        for t in tg:
            if t not in ts:
                ts.append(t)
        if not ts:
            emit(ind, 'c.tail(); return;')
        elif len(ts) == 1:
            if check:
                emit(ind, f'if c.pc != {ts[0]:#x} {{ c.tail(); return; }}')
            branch(x, ts[0], ind)
        elif len(ts) == 2 and not check:
            emit(ind, f'if c.pc == {ts[0]:#x} {{')
            branch(x, ts[0], ind + 1)
            emit(ind, '} else {')
            branch(x, ts[1], ind + 1)
            emit(ind, '}')
        else:
            emit(ind, 'match c.pc {')
            for t in ts:
                emit(ind + 1, f'{t:#x} => {{')
                branch(x, t, ind + 2)
                emit(ind + 1, '}')
            emit(ind + 1, '_ => { c.tail(); return; }')
            emit(ind, '}')
    try:
        tree(lo, 1)
    except RecursionError:
        why['too deep'] = why.get('too deep', 0) + 1
        return None
    return out


def fname(a):
    n = names.get(a)
    return f'{n}_{a:06x}' if n else f'f_{a:06x}'


chosen = list(fns)
if KEEP_BOOT:
    chosen = [(lo, hi) for lo, hi in chosen if lo >= OV_START]
fn_lab = [(lo, hi, fn_labels(lo, hi)) for lo, hi in chosen]
fn_lab = [x for x in fn_lab if x[2]]

# Crates of about CHUNK instructions, address-ordered (neighbours tend to call each other).
CHUNK = A.chunk
# Cold slow-path helpers (--shared-slow). Every crate gets the same set (share_iso.py needs equal headers), chosen by the flags;
# the plain forms (hw, sp, pl) are only the fallback for blocks of an odd shape, so each helper carries allow(dead_code).
HELPERS = {
    'hw': ['#[cold] #[inline(never)] fn hw(c: &mut Cpu, op: u32, pc: u32, ic: u64, nxt: u32) { c.hw(op, pc, ic, nxt); }'],
    'sp': ['#[cold] #[inline(never)] fn sp(c: &mut Cpu, op: u32, pc: u32, ic: u64, nxt: u32, rem: u64, last: u32, brl: u32) -> u8 {',
           '    c.hw(op, pc, ic, nxt);',
           '    if !c.go(rem) { if !c.slow(last, brl) { return 2; } return 1; }',
           '    0',
           '}'],
    'pl': ['#[cold] #[inline(never)] fn pl(c: &mut Cpu, l: u32, last: u32, brl: u32) -> u8 { if c.pc != l || !c.slow(last, brl) { 2 } else { 1 } }'],
    # hq = hw with nxt = pc + 4; sq = sp with nxt = pc + 4 and rem, last, brl from bits 32.. of od (slow_enc); pq = pl likewise
    'hq': ['#[cold] #[inline(never)] fn hq(c: &mut Cpu, op: u32, pc: u32, ic: u64) { c.hw(op, pc, ic, pc.wrapping_add(4)); }'],
    'sq': ['#[cold] #[inline(never)] fn sq(c: &mut Cpu, od: u64, pc: u32, ic: u64) -> u8 {',
           '    let e = od >> 32; let rem = e & 0xffff;',
           '    c.hw(od as u32, pc, ic, pc.wrapping_add(4));',
           '    if !c.go(rem) {',
           '        let last = pc.wrapping_add(4 * (rem as u32 + ((e >> 16) & 1) as u32));',
           '        if !c.slow(last, if e & 0x20000 != 0 { last.wrapping_sub(4) } else { 0 }) { return 2; }',
           '        return 1;',
           '    }',
           '    0',
           '}'],
    'pq': ['#[cold] #[inline(never)] fn pq(c: &mut Cpu, l: u32, e: u64) -> u8 {',
           '    let last = l.wrapping_add(4 * ((e & 0xffff) as u32 + ((e >> 16) & 1) as u32));',
           '    if c.pc != l || !c.slow(last, if e & 0x20000 != 0 { last.wrapping_sub(4) } else { 0 }) { 2 } else { 1 }',
           '}'],
}
HELPER_LINES = [('#[allow(dead_code)] ' + l if l.startswith('#[cold]') else l) for ls in HELPERS.values() for l in ls]
# --outline-mem: the fast path of every access goes through one out-of-line call per width (Mem::frd*/fwr* unchanged inside).
XMEM = (['#[allow(dead_code)] trait XMem {']
        + [f'    fn xrd{w}(&mut self, a: u32) -> Option<u{w}>;' for w in (8, 16, 32, 64, 128)]
        + [f'    fn xwr{w}(&mut self, a: u32, v: u{w}) -> bool;' for w in (8, 16, 32, 64, 128)]
        + ['}', 'impl XMem for ee_core::mem::Mem {']
        + [f'    #[inline(never)] fn xrd{w}(&mut self, a: u32) -> Option<u{w}> {{ self.frd{w}(a) }}' for w in (8, 16, 32, 64, 128)]
        + [f'    #[inline(never)] fn xwr{w}(&mut self, a: u32, v: u{w}) -> bool {{ self.fwr{w}(a, v) }}' for w in (8, 16, 32, 64, 128)]
        + ['}'])
# --direct-mem: plain RAM (index = address, 32 MB) and scratchpad (0x7000_0000) inline, the same bytes Mem::fbuf picks for those
# addresses (RAM and SPR are allocated at exactly RAM / 0x4000 bytes); any other address goes to the out-of-line helper above.
def _dm(w):
    n = w // 8
    if WINMEM:
        # [PC] (speed) Constant address ranges instead of the slice bounds (no Vec length load, no bounds check): ee-oracle's Mem
        # allocates ram / spr at exactly RAM (32 MB) / 0x4000 bytes and never resizes them, so every address picks the same bytes
        # as the slice form below. RAM is the straight-line path; std::hint::cold_path marks the scratchpad test.
        rm, sm = (32 << 20) - n, 0x4000 - n
        def rdh(p, ix, m): return f'if {ix} <= {m:#x} {{ return Some(u{w}::from_le(unsafe {{ (self.{p}.as_ptr().add({ix} as usize) as *const u{w}).read_unaligned() }})); }}'
        def wrh(p, ix, m): return f'if {ix} <= {m:#x} {{ unsafe {{ (self.{p}.as_mut_ptr().add({ix} as usize) as *mut u{w}).write_unaligned(v.to_le()); }} return true; }}'
        S_ = '        let s = a.wrapping_sub(0x7000_0000);'
        return ([f'    #[inline(always)] fn drd{w}(&mut self, a: u32) -> Option<u{w}> {{', '        ' + rdh('ram', 'a', rm),
                 '        std::hint::cold_path();', S_, '        ' + rdh('spr', 's', sm), f'        self.xrd{w}(a)', '    }',
                 f'    #[inline(always)] fn dwr{w}(&mut self, a: u32, v: u{w}) -> bool {{', '        ' + wrh('ram', 'a', rm),
                 '        std::hint::cold_path();', S_, '        ' + wrh('spr', 's', sm), f'        self.xwr{w}(a, v)', '    }'])
    return [f'    #[inline(always)] fn drd{w}(&mut self, a: u32) -> Option<u{w}> {{',
            f'        let i = a as usize; if let Some(b) = self.ram.get(i..i + {n}) {{ return Some(u{w}::from_le_bytes(b.try_into().unwrap())); }}',
            f'        let s = a.wrapping_sub(0x7000_0000) as usize; if let Some(b) = self.spr.get(s..s + {n}) {{ return Some(u{w}::from_le_bytes(b.try_into().unwrap())); }}',
            f'        self.xrd{w}(a)', '    }',
            f'    #[inline(always)] fn dwr{w}(&mut self, a: u32, v: u{w}) -> bool {{',
            f'        let i = a as usize; if let Some(b) = self.ram.get_mut(i..i + {n}) {{ b.copy_from_slice(&v.to_le_bytes()); return true; }}',
            f'        let s = a.wrapping_sub(0x7000_0000) as usize; if let Some(b) = self.spr.get_mut(s..s + {n}) {{ b.copy_from_slice(&v.to_le_bytes()); return true; }}',
            f'        self.xwr{w}(a, v)', '    }']
DMEM = (['#[allow(dead_code)] trait DMem {']
        + [f'    fn drd{w}(&mut self, a: u32) -> Option<u{w}>;' for w in (8, 16, 32, 64, 128)]
        + [f'    fn dwr{w}(&mut self, a: u32, v: u{w}) -> bool;' for w in (8, 16, 32, 64, 128)]
        + ['}', 'impl DMem for ee_core::mem::Mem {'] + [l for w in (8, 16, 32, 64, 128) for l in _dm(w)] + ['}'])
groups = {'boot': [x for x in fn_lab if x[0] < OV_START], f'l{lvl}': [x for x in fn_lab if x[0] >= OV_START]}
crates = []
for grp, fl in groups.items():
    cur, n = [], 0
    for x in fl:
        cur.append(x)
        n += (x[1] - x[0]) // 4
        if n >= CHUNK:
            crates.append((f'{grp}_{len([c for c in crates if c[0].startswith(grp + "_")]):02d}', cur))
            cur, n = [], 0
    if cur:
        crates.append((f'{grp}_{len([c for c in crates if c[0].startswith(grp + "_")]):02d}', cur))

def _rel(target, frm):
    """Crate dependency path relative to the crate directory (generated trees hold no absolute paths)."""
    return os.path.relpath(os.path.abspath(target), os.path.abspath(frm)).replace('\\', '/')


REC = os.path.join(A.out, 'rec')
# This level's and the boot crates are replaced (only this level's with --keep-boot); other levels' crates stay.
# The new crates are written in place (only files whose content differs are touched, see _put); crates of this level
# that the new run no longer makes are removed after the loop below.
import shutil, re
OLD_CRATES = set()
if os.path.isdir(REC):
    for d in os.listdir(REC):
        if re.fullmatch(rf'l{lvl}_\d+' if KEEP_BOOT else rf'(boot|l{lvl})_\d+', d):
            OLD_CRATES.add(d)

import re as _re
CALL_RE = _re.compile(r'@CALL:([0-9a-f]+):([0-9a-f]+)@')
sys.setrecursionlimit(20000)
total_fns = total = n_struct = n_skin = n_poll = 0
why = {}
for cname, fl in crates:
    comp = []
    for lo, hi, labs in fl:
        arms = compile_fn(lo, hi, labs)
        # A discovered function with calls uses the dispatch form: every block label is then an entry of the map, so the
        # return points of its calls (continuations reached through a `jr` of a callee with its own return register) are found.
        hole_calls = (lo, hi) in HOLES and any(links(word(a)) for a in range(lo, hi, 4) if kind(word(a)) == 'b')
        comp.append((lo, hi, arms, None if hole_calls else structure(lo, arms)))
    # Direct calls: structured functions by entry, dispatch-form ones at any label.
    label_fn = {}
    for lo, hi, arms, st in comp:
        for l in ([lo] if st is not None else [a[0] for a in arms]):
            label_fn.setdefault(l, fname(lo))
    src = ['// Generated by recgen.py from retail EE code (do not edit by hand; hand rewrites replace entries in FNS).',
           '#![allow(unused_variables, unused_mut, unused_parens, unused_labels, unreachable_code, unused_imports, '
           'non_snake_case, clippy::all)]',
           'use ee_core::cpu::{div32, divu32, sx32, uw4, w4, Cpu, ADD, MADD, MAX, MIN, MSUB, MUL, SUB, '
           'LDL_MASK, LDR_MASK, LWL_MASK, LWR_MASK, SDL_MASK, SDR_MASK, SWL_MASK, SWR_MASK};', 'use ee_core::fpu;', '']
    if SHARED:
        src[3:3] = HELPER_LINES
    if OUTLINE:
        src[3:3] = XMEM + (DMEM if DIRECT else [])
    table = []
    for lo, hi, arms, st in comp:
        total += (hi - lo) // 4
        name = fname(lo)
        h = 0xcbf29ce484222325
        for a in range(lo, hi, 4):
            h = ((h ^ word(a)) * 0x100000001b3) & 0xffffffffffffffff
        labs = [lo] if st is not None else [a[0] for a in arms]
        table.append(f'    ee_core::rec::Fn {{ entry: {lo:#x}, end: {hi:#x}, hash: {h:#x}, labels: &[{", ".join(f"{a:#x}" for a in labs)}], f: {name} }},')
        src.append(f'pub fn {name}(c: &mut Cpu) {{')
        # A PC hook (rec::Hand, registered by rac-rs) runs instead when registered; the entry stub bit is tested in the same prologue.
        src.append(f'    if c.hand({lo:#x}) {{ return; }}')
        if st is not None:
            n_struct += 1
            src += [resolve_calls(x) for x in st]
            src.append('}')
            src.append('')
            continue
        src.append('    loop {')
        src.append('        match c.pc {')
        for L, n, last, brl, body, after, ex in arms:
            src.append(f'            {L:#x} => {{')
            if L == VBLANK_WAIT:
                src.append('                if c.vsync_armed && !c.vblank_hook() { return; }')
            if A.skin_ff and skin_head(L):
                src.append(f'                if c.ff(0, {L:#x}) {{ continue; }}')
                n_skin += 1
            if poll_head(L):
                src.append(f'                c.ff(1, {L:#x});')
            src.append("                'b: {")
            if n:
                src.append(f"                    if !c.poll({n}) {{ " + (pl_call(L, n, last, brl) if SHARED else f"if c.pc != {L:#x} || !c.slow({last:#x}, {brl:#x}) {{ return; }} break 'b;") + " }")
            src.append('                    let t = c.icount;')
            for b in body:
                src.append('                    ' + resolve_calls(b))
            src.append('                }')
            src.append('                ' + resolve_calls(after))
            src.append('            }')
        src.append('            _ => { c.tail(); return; }')
        src.append('        }')
        src.append('    }')
        src.append('}')
        src.append('')
    src.append('pub static FNS: &[ee_core::rec::Fn] = &[')
    src += table
    src.append('];')
    d = os.path.join(REC, cname)
    os.makedirs(os.path.join(d, 'src'), exist_ok=True)
    _put(os.path.join(d, 'Cargo.toml'),
        f'[package]\nname = "rec-{cname.replace("_", "-")}"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\nee-core = {{ path = "{_rel(A.oracle, d)}/core" }}\n')
    if A.compact:
        src = [x.strip() for x in src if x.strip()]
    _put(os.path.join(d, 'src', 'lib.rs'), '\n'.join(src) + '\n')
    total_fns += len(table)
    n_poll += sum(x.strip().startswith('c.ff(1, ') for x in src)

for _d in sorted(OLD_CRATES - {c[0] for c in crates}):
    shutil.rmtree(os.path.join(REC, _d))
all_crates = write_facade(A.out, A.oracle)
print(f'{total_fns} functions ({n_struct} structured, {n_skin} skin-ff loop heads, {n_poll} poll-ff loop heads), {total} instructions, {len(crates)} crates ({len(all_crates)} in ee-game); dispatch form: {why}')
