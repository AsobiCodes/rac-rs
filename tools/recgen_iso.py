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
    """SCES_509.16 via a minimal ISO9660 walk (root directory record)."""
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
        if name.upper().startswith('SCES_509.16'):
            return d.bytes(iso_u32(dirdata, o + 2), iso_u32(dirdata, o + 10))
        o += ln
    raise FileNotFoundError('SCES_509.16')


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
    """Size (and optionally SHA-1) of R&C PAL v1.00 (SCES-50916); the boot ELF lookup then checks the serial file."""
    n = os.path.getsize(path)
    if n != ISO_SIZE:
        sys.exit(f'{path}: {n} bytes, expected {ISO_SIZE} (Ratchet & Clank PAL v1.00, SCES-50916)')
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
ap.add_argument('--bounds', default=os.path.join(HERE, 'fn_bounds.tsv'))
ap.add_argument('--out', default=os.path.join(HERE, '..', 'generated'))
ap.add_argument('--oracle', default=os.path.join(HERE, '..', 'oracle'))
ap.add_argument('--stubs', default=os.path.join(HERE, '..', 'oracle', 'stubs.txt'))
ap.add_argument('--names', default=None, help='dev only: decomp fn_names.tsv (function idents)')
ap.add_argument('--keep-boot', action='store_true')
ap.add_argument('--no-comments', action='store_true', help='no per-instruction disassembly / block comments')
ap.add_argument('--compact', action='store_true', help='no indentation, no blank lines')
ap.add_argument('--shared-slow', action='store_true', help='cold shared helpers for the slow path of memory accesses and block polls')
ap.add_argument('--outline-mem', action='store_true', help='memory fast paths through one #[inline(never)] helper per access width in each crate (about 2x less LLVM work, same behaviour)')
ap.add_argument('--chunk', type=int, default=12000)
ap.add_argument('--hot', default=None, help='optional hot list (lines: LEVEL ENTRY_HEX, LEVEL = boot|N): translate only these functions; the rest run in the interpreter')
ap.add_argument('--rec-opt', type=int, default=None, help='opt-level for the rec-* crates only (facade profile)')
ap.add_argument('--verify-sha1', action='store_true', help='also check the ISO SHA-1 (setup.ps1 does it already)')
ap.add_argument('--ptrs-iso', action='store_true', help='equivalence test only: with --src state, take the code-pointer label set from the ISO image')
A = ap.parse_args()

def _rel(target, frm):
    """Crate dependency path relative to the crate directory (generated trees hold no absolute paths)."""
    return os.path.relpath(os.path.abspath(target), os.path.abspath(frm)).replace(chr(92), '/')


def write_facade(out, oracle):
    REC = os.path.join(out, 'rec')
    # ee-game: every crate under rec/ (all levels generated so far).
    all_crates = sorted(x for x in os.listdir(REC) if os.path.isfile(os.path.join(REC, x, 'Cargo.toml')))
    G = os.path.join(out, 'game')
    os.makedirs(os.path.join(G, 'src'), exist_ok=True)
    os.makedirs(REC, exist_ok=True)
    deps = '\n'.join(f'rec-{c.replace("_", "-")} = {{ path = "../rec/{c}" }}' for c in all_crates)
    open(os.path.join(G, 'Cargo.toml'), 'w', newline='\n').write(
        '# Generated by recgen.py.\n[package]\nname = "ee-game"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\nee-oracle = {{ path = "{_rel(oracle, G)}" }}\nee-hand = {{ path = "{_rel(oracle, G)}/hand" }}\n{deps}\n')
    def _reg(c):
        # A level crate (lN_MM) is tied to its level byte N: Table::build probes it only while that level is current.
        m = re.fullmatch(r'l(\d+)_\d+', c)
        if m:
            return f'    ee_oracle::rec::register_level({int(m.group(1))}, rec_{c}::FNS);'
        return f'    ee_oracle::rec::register(rec_{c}::FNS);'
    reg = '\n'.join(_reg(c) for c in all_crates)
    open(os.path.join(G, 'src', 'lib.rs'), 'w', newline='\n').write(
        '//! Generated by recgen.py: the recompiled retail code (crates under oracle/rec) for ee-oracle.\n'
        'pub use ee_hand;\npub use ee_oracle;\n\n/// Registers every recompiled set and the hand-written functions with ee-oracle (before `Machine::enable_rec`).\n'
        f'pub fn register() {{\n{reg}\n    ee_hand::register();\n}}\n')
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
lvl = A.level
OV_START = 0x15ef00
VBLANK_WAIT = 0x118cb0

if A.src == 'iso':
    if not A.iso:
        sys.exit('--iso PATH is required (the game disc image)')
    iso_check(A.iso, A.verify_sha1)
    _d = IsoDisc(A.iso)
    ram = bytes(iso_build_image(_d, lvl, zero_from=OV_START)[0])
else:
    ram = open(os.path.join(A.src.split(':', 1)[1], 'eeMemory.bin'), 'rb').read()
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
if A.hot:
    _hot = set()
    for _l in open(A.hot):
        _p = _l.split()
        if len(_p) == 2 and not _l.startswith('#') and (_p[0] == 'boot' or int(_p[0]) == lvl):
            _hot.add(int(_p[1], 16))
    fns = [f for f in fns if f[0] in _hot]

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

covered = None

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

# Words anywhere in the image pointing into code: jump-table targets (and harmless extras) become block labels.
ptrs = {w for w in W if 0x100000 <= w < 0x400000 and not w & 3}


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
    return s.replace('c.mem.f', 'c.mem.x') if OUTLINE else s


def ins_code(op, pc, k, nxt, rem, last, brl):
    """Code for the k-th instruction (from 1) of a block of which `rem` follow; `nxt` is the PC after it.
    A hardware access runs through the interpreter with its exact count, and if an event is now due before the block
    ends the rest of the block is interpreted too."""
    if kind(op) == 'm':
        if SHARED:
            if rem:
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
    return f'let v = c.fmac({k}, c.vf[{fs}], {tv}); c.vu_out({dst}, v, {dest:#x}, {mac});'


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
    global jtab
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
        for i, (pc, op) in enumerate(seq):
            if not NOCOM:
                body.append(f'// {pc:08x}: {dis(op, pc)}')
            s = ins_code(op, pc, i + 1, f'{pc + 4:#x}u32', n - i - 1, last, brl)
            if s:
                body.append(s)
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
        emit(ind, "'b: {")
        if n:
            emit(ind + 1, f"if !c.poll({n}) {{ " + (f"match pl(c, {x:#x}, {last:#x}, {brl:#x}) {{ 1 => break 'b, _ => return }}" if SHARED else f"if c.pc != {x:#x} || !c.slow({last:#x}, {brl:#x}) {{ return; }} break 'b;") + " }")
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


chosen = [(lo, hi) for lo, hi in fns if covered is None or any(p in covered for p in range(lo, hi, 4))]
if KEEP_BOOT:
    chosen = [(lo, hi) for lo, hi in chosen if lo >= OV_START]
fn_lab = [(lo, hi, fn_labels(lo, hi)) for lo, hi in chosen]
fn_lab = [x for x in fn_lab if x[2]]

# Crates of about CHUNK instructions, address-ordered (neighbours tend to call each other).
CHUNK = A.chunk
HELPERS = [
    '#[cold] #[inline(never)] fn hw(c: &mut Cpu, op: u32, pc: u32, ic: u64, nxt: u32) { c.hw(op, pc, ic, nxt); }',
    '#[cold] #[inline(never)] fn sp(c: &mut Cpu, op: u32, pc: u32, ic: u64, nxt: u32, rem: u64, last: u32, brl: u32) -> u8 {',
    '    c.hw(op, pc, ic, nxt);',
    '    if !c.go(rem) { if !c.slow(last, brl) { return 2; } return 1; }',
    '    0',
    '}',
    '#[cold] #[inline(never)] fn pl(c: &mut Cpu, l: u32, last: u32, brl: u32) -> u8 { if c.pc != l || !c.slow(last, brl) { 2 } else { 1 } }',
]
# --outline-mem: the fast path of every access goes through one out-of-line call per width (Mem::frd*/fwr* unchanged inside).
XMEM = (['#[allow(dead_code)] trait XMem {']
        + [f'    fn xrd{w}(&mut self, a: u32) -> Option<u{w}>;' for w in (8, 16, 32, 64, 128)]
        + [f'    fn xwr{w}(&mut self, a: u32, v: u{w}) -> bool;' for w in (8, 16, 32, 64, 128)]
        + ['}', 'impl XMem for ee_oracle::mem::Mem {']
        + [f'    #[inline(never)] fn xrd{w}(&mut self, a: u32) -> Option<u{w}> {{ self.frd{w}(a) }}' for w in (8, 16, 32, 64, 128)]
        + [f'    #[inline(never)] fn xwr{w}(&mut self, a: u32, v: u{w}) -> bool {{ self.fwr{w}(a, v) }}' for w in (8, 16, 32, 64, 128)]
        + ['}'])
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
import shutil, re
if os.path.isdir(REC):
    for d in os.listdir(REC):
        if re.fullmatch(rf'l{lvl}_\d+' if KEEP_BOOT else rf'(boot|l{lvl})_\d+', d):
            shutil.rmtree(os.path.join(REC, d))

import re as _re
CALL_RE = _re.compile(r'@CALL:([0-9a-f]+):([0-9a-f]+)@')
sys.setrecursionlimit(20000)
total_fns = total = n_struct = 0
why = {}
for cname, fl in crates:
    comp = []
    for lo, hi, labs in fl:
        arms = compile_fn(lo, hi, labs)
        comp.append((lo, hi, arms, structure(lo, arms)))
    # Direct calls: structured functions by entry, dispatch-form ones at any label.
    label_fn = {}
    for lo, hi, arms, st in comp:
        for l in ([lo] if st is not None else [a[0] for a in arms]):
            label_fn.setdefault(l, fname(lo))
    src = ['// Generated by recgen.py from retail EE code (do not edit by hand; hand rewrites replace entries in FNS).',
           '#![allow(unused_variables, unused_mut, unused_parens, unused_labels, unreachable_code, unused_imports, '
           'non_snake_case, clippy::all)]',
           'use ee_oracle::cpu::{div32, divu32, sx32, uw4, w4, Cpu, ADD, MADD, MAX, MIN, MSUB, MUL, SUB, '
           'LDL_MASK, LDR_MASK, LWL_MASK, LWR_MASK, SDL_MASK, SDR_MASK, SWL_MASK, SWR_MASK};', 'use ee_oracle::fpu;', '']
    if SHARED:
        src[3:3] = HELPERS
    if OUTLINE:
        src[3:3] = XMEM
    table = []
    for lo, hi, arms, st in comp:
        total += (hi - lo) // 4
        name = fname(lo)
        h = 0xcbf29ce484222325
        for a in range(lo, hi, 4):
            h = ((h ^ word(a)) * 0x100000001b3) & 0xffffffffffffffff
        labs = [lo] if st is not None else [a[0] for a in arms]
        table.append(f'    ee_oracle::rec::Fn {{ entry: {lo:#x}, end: {hi:#x}, hash: {h:#x}, labels: &[{", ".join(f"{a:#x}" for a in labs)}], f: {name} }},')
        src.append(f'pub fn {name}(c: &mut Cpu) {{')
        # A hand-written replacement (crate ee-hand) runs instead when registered.
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
            src.append("                'b: {")
            if n:
                src.append(f"                    if !c.poll({n}) {{ " + (f"match pl(c, {L:#x}, {last:#x}, {brl:#x}) {{ 1 => break 'b, _ => return }}" if SHARED else f"if c.pc != {L:#x} || !c.slow({last:#x}, {brl:#x}) {{ return; }} break 'b;") + " }")
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
    src.append('pub static FNS: &[ee_oracle::rec::Fn] = &[')
    src += table
    src.append('];')
    d = os.path.join(REC, cname)
    os.makedirs(os.path.join(d, 'src'), exist_ok=True)
    open(os.path.join(d, 'Cargo.toml'), 'w', newline='\n').write(
        f'[package]\nname = "rec-{cname.replace("_", "-")}"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\nee-oracle = {{ path = "{_rel(A.oracle, d)}" }}\n')
    if A.compact:
        src = [x.strip() for x in src if x.strip()]
    open(os.path.join(d, 'src', 'lib.rs'), 'w', newline='\n').write('\n'.join(src) + '\n')
    total_fns += len(table)

all_crates = write_facade(A.out, A.oracle)
print(f'{total_fns} functions ({n_struct} structured), {total} instructions, {len(crates)} crates ({len(all_crates)} in ee-game); dispatch form: {why}')
