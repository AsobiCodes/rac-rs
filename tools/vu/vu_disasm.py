#!/usr/bin/env python3
"""PS2 VU micro-mode disassembler. usage: vu_disasm.py image.bin [out.s] [--base 0xADDR]
Encoding tables follow PCSX2 VUops.cpp / PS2 VU User's Manual.
Each 64-bit pair: lower word at lower address, upper word second."""
import struct, sys

DEST = 'xyzw'


def dst(c):
    return ''.join(DEST[i] for i in range(4) if c >> (24 - i) & 1)


def fv(n): return 'vf%02d' % n
def iv(n): return 'vi%02d' % n
def fsf(c, hi): return DEST[(c >> hi) & 3]


UP_MAIN = {}
bc = 'xyzw'
for base, nm in ((0x00, 'ADD'), (0x04, 'SUB'), (0x08, 'MADD'), (0x0C, 'MSUB'),
                 (0x10, 'MAX'), (0x14, 'MINI'), (0x18, 'MUL')):
    for i in range(4):
        UP_MAIN[base + i] = (nm + bc[i], 'bc', bc[i])
UP_MAIN.update({
    0x1C: ('MULq', 'q', 0), 0x1D: ('MAXi', 'i', 0), 0x1E: ('MULi', 'i', 0), 0x1F: ('MINIi', 'i', 0),
    0x20: ('ADDq', 'q', 0), 0x21: ('MADDq', 'q', 0), 0x22: ('ADDi', 'i', 0), 0x23: ('MADDi', 'i', 0),
    0x24: ('SUBq', 'q', 0), 0x25: ('MSUBq', 'q', 0), 0x26: ('SUBi', 'i', 0), 0x27: ('MSUBi', 'i', 0),
    0x28: ('ADD', 'vv', 0), 0x29: ('MADD', 'vv', 0), 0x2A: ('MUL', 'vv', 0), 0x2B: ('MAX', 'vv', 0),
    0x2C: ('SUB', 'vv', 0), 0x2D: ('MSUB', 'vv', 0), 0x2E: ('OPMSUB', 'op', 0), 0x2F: ('MINI', 'vv', 0)})

UP_SPEC = {}
for col in range(4):
    b = bc[col]
    UP_SPEC[(col, 0)] = ('ADDA' + b, 'abc', b)
    UP_SPEC[(col, 1)] = ('SUBA' + b, 'abc', b)
    UP_SPEC[(col, 2)] = ('MADDA' + b, 'abc', b)
    UP_SPEC[(col, 3)] = ('MSUBA' + b, 'abc', b)
    UP_SPEC[(col, 4)] = ('ITOF' + ['0', '4', '12', '15'][col], 'cvt', 0)
    UP_SPEC[(col, 5)] = ('FTOI' + ['0', '4', '12', '15'][col], 'cvt', 0)
    UP_SPEC[(col, 6)] = ('MULA' + b, 'abc', b)
UP_SPEC[(0, 7)] = ('MULAq', 'aq', 0); UP_SPEC[(1, 7)] = ('ABS', 'cvt', 0)
UP_SPEC[(2, 7)] = ('MULAi', 'ai', 0); UP_SPEC[(3, 7)] = ('CLIP', 'clip', 0)
UP_SPEC[(0, 8)] = ('ADDAq', 'aq', 0); UP_SPEC[(1, 8)] = ('MADDAq', 'aq', 0)
UP_SPEC[(2, 8)] = ('ADDAi', 'ai', 0); UP_SPEC[(3, 8)] = ('MADDAi', 'ai', 0)
UP_SPEC[(0, 9)] = ('SUBAq', 'aq', 0); UP_SPEC[(1, 9)] = ('MSUBAq', 'aq', 0)
UP_SPEC[(2, 9)] = ('SUBAi', 'ai', 0); UP_SPEC[(3, 9)] = ('MSUBAi', 'ai', 0)
UP_SPEC[(0, 10)] = ('ADDA', 'avv', 0); UP_SPEC[(1, 10)] = ('MADDA', 'avv', 0)
UP_SPEC[(2, 10)] = ('MULA', 'avv', 0)
UP_SPEC[(0, 11)] = ('SUBA', 'avv', 0); UP_SPEC[(1, 11)] = ('MSUBA', 'avv', 0)
UP_SPEC[(2, 11)] = ('OPMULA', 'aop', 0); UP_SPEC[(3, 11)] = ('NOP', 'nop', 0)


class Unk(Exception):
    pass


def upper(c):
    ft = (c >> 16) & 31; fs = (c >> 11) & 31; fd = (c >> 6) & 31
    d = dst(c); op = c & 63
    if op < 0x30:
        if op not in UP_MAIN: raise Unk
        nm, k, b = UP_MAIN[op]
        if k == 'bc': return '%s.%s %s, %s, %s%s' % (nm, d, fv(fd), fv(fs), fv(ft), b)
        if k == 'q': return '%s.%s %s, %s, Q' % (nm, d, fv(fd), fv(fs))
        if k == 'i': return '%s.%s %s, %s, I' % (nm, d, fv(fd), fv(fs))
        if k == 'vv': return '%s.%s %s, %s, %s' % (nm, d, fv(fd), fv(fs), fv(ft))
        if k == 'op': return '%s.xyz %sxyz, %sxyz, %sxyz' % (nm, fv(fd), fv(fs), fv(ft))
    if op < 0x3C: raise Unk
    col = op & 3; sub = (c >> 6) & 31
    if (col, sub) not in UP_SPEC: raise Unk
    nm, k, b = UP_SPEC[(col, sub)]
    if k == 'abc': return '%s.%s ACC, %s, %s%s' % (nm, d, fv(fs), fv(ft), b)
    if k == 'aq': return '%s.%s ACC, %s, Q' % (nm, d, fv(fs))
    if k == 'ai': return '%s.%s ACC, %s, I' % (nm, d, fv(fs))
    if k == 'avv': return '%s.%s ACC, %s, %s' % (nm, d, fv(fs), fv(ft))
    if k == 'aop': return '%s.xyz ACCxyz, %sxyz, %sxyz' % (nm, fv(fs), fv(ft))
    if k == 'cvt': return '%s.%s %s, %s' % (nm, d, fv(ft), fv(fs))
    if k == 'clip': return 'CLIP.xyz %sxyz, %sw' % (fv(fs), fv(ft))
    if k == 'nop': return 'NOP'
    raise Unk


def sx(v, bits):
    return v - (1 << bits) if v >> (bits - 1) else v


def lower(c, pc):
    """returns (text, branch_target_or_None)"""
    ft = (c >> 16) & 31; fs = (c >> 11) & 31; d = dst(c)
    it, is_ = ft, fs
    op = c >> 25
    imm11 = sx(c & 0x7FF, 11)
    tgt = lambda: pc + 8 + imm11 * 8
    if op == 0x00: return 'LQ.%s %s, %d(%s)' % (d, fv(ft), imm11, iv(is_)), None
    if op == 0x01: return 'SQ.%s %s, %d(%s)' % (d, fv(fs), imm11, iv(it)), None
    if op == 0x04: return 'ILW.%s %s, %d(%s)' % (d, iv(it), imm11, iv(is_)), None
    if op == 0x05: return 'ISW.%s %s, %d(%s)' % (d, iv(it), imm11, iv(is_)), None
    if op in (0x08, 0x09):
        imm = ((c >> 21) & 0xF) << 11 | (c & 0x7FF)
        return '%s %s, %s, 0x%x' % ('IADDIU' if op == 8 else 'ISUBIU', iv(it), iv(is_), imm), None
    if op in (0x10, 0x11, 0x12, 0x13):
        nm = ('FCEQ', 'FCSET', 'FCAND', 'FCOR')[op - 0x10]
        imm = c & 0xFFFFFF
        return ('%s 0x%06x' % (nm, imm) if op == 0x11 else '%s vi01, 0x%06x' % (nm, imm)), None
    if op in (0x14, 0x15, 0x16, 0x17):
        nm = ('FSEQ', 'FSSET', 'FSAND', 'FSOR')[op - 0x14]
        imm = ((c >> 21) & 1) << 11 | (c & 0x7FF)
        return ('%s 0x%03x' % (nm, imm) if op == 0x15 else '%s %s, 0x%03x' % (nm, iv(it), imm)), None
    if op in (0x18, 0x1A, 0x1B):
        nm = {0x18: 'FMEQ', 0x1A: 'FMAND', 0x1B: 'FMOR'}[op]
        return '%s %s, %s' % (nm, iv(it), iv(is_)), None
    if op == 0x1C: return 'FCGET %s' % iv(it), None
    if op == 0x20: return 'B', tgt()
    if op == 0x21: return 'BAL %s,' % iv(it), tgt()
    if op == 0x24: return 'JR %s' % iv(is_), None
    if op == 0x25: return 'JALR %s, %s' % (iv(it), iv(is_)), None
    if op in (0x28, 0x29): return '%s %s, %s,' % ('IBEQ' if op == 0x28 else 'IBNE', iv(it), iv(is_)), tgt()
    if op in (0x2C, 0x2D, 0x2E, 0x2F):
        return '%s %s,' % (('IBLTZ', 'IBGTZ', 'IBLEZ', 'IBGEZ')[op - 0x2C], iv(is_)), tgt()
    if op != 0x40: raise Unk
    o6 = c & 63
    id_ = (c >> 6) & 31
    if o6 == 0x30: return 'IADD %s, %s, %s' % (iv(id_), iv(is_), iv(it)), None
    if o6 == 0x31: return 'ISUB %s, %s, %s' % (iv(id_), iv(is_), iv(it)), None
    if o6 == 0x32: return 'IADDI %s, %s, %d' % (iv(it), iv(is_), sx(id_, 5)), None
    if o6 == 0x34: return 'IAND %s, %s, %s' % (iv(id_), iv(is_), iv(it)), None
    if o6 == 0x35: return 'IOR %s, %s, %s' % (iv(id_), iv(is_), iv(it)), None
    if o6 < 0x3C: raise Unk
    col = o6 & 3; sub = id_
    fsfv = fv(fs) + fsf(c, 21); ftfv = fv(ft) + fsf(c, 23)
    if c == 0x8000033C: return 'NOP', None
    T = {
        (0, 0x0C): 'MOVE.%s %s, %s' % (d, fv(ft), fv(fs)),
        (0, 0x0D): 'LQI.%s %s, (%s++)' % (d, fv(ft), iv(is_)),
        (0, 0x0E): 'DIV Q, %s, %s' % (fsfv, ftfv),
        (0, 0x0F): 'MTIR %s, %s' % (iv(it), fsfv),
        (0, 0x10): 'RNEXT.%s %s, R' % (d, fv(ft)),
        (0, 0x19): 'MFP.%s %s, P' % (d, fv(ft)),
        (0, 0x1A): 'XTOP %s' % iv(it),
        (0, 0x1B): 'XGKICK %s' % iv(is_),
        (0, 0x1C): 'ESADD P, %sxyz' % fv(fs),
        (0, 0x1D): 'EATANxy P, %s' % fv(fs),
        (0, 0x1E): 'ESQRT P, %s' % fsfv,
        (0, 0x1F): 'ESIN P, %s' % fsfv,
        (1, 0x0C): 'MR32.%s %s, %s' % (d, fv(ft), fv(fs)),
        (1, 0x0D): 'SQI.%s %s, (%s++)' % (d, fv(fs), iv(it)),
        (1, 0x0E): 'SQRT Q, %s' % ftfv,
        (1, 0x0F): 'MFIR.%s %s, %s' % (d, fv(ft), iv(is_)),
        (1, 0x10): 'RGET.%s %s, R' % (d, fv(ft)),
        (1, 0x1A): 'XITOP %s' % iv(it),
        (1, 0x1C): 'ERSADD P, %sxyz' % fv(fs),
        (1, 0x1D): 'EATANxz P, %s' % fv(fs),
        (1, 0x1E): 'ERSQRT P, %s' % fsfv,
        (1, 0x1F): 'EATAN P, %s' % fsfv,
        (2, 0x0D): 'LQD.%s %s, (--%s)' % (d, fv(ft), iv(is_)),
        (2, 0x0E): 'RSQRT Q, %s, %s' % (fsfv, ftfv),
        (2, 0x0F): 'ILWR.%s %s, (%s)' % (d, iv(it), iv(is_)),
        (2, 0x10): 'RINIT R, %s' % fsfv,
        (2, 0x1C): 'ELENG P, %sxyz' % fv(fs),
        (2, 0x1D): 'ESUM P, %s' % fv(fs),
        (2, 0x1E): 'ERCPR P, %s' % fsfv,
        (2, 0x1F): 'EEXP P, %s' % fsfv,
        (3, 0x0D): 'SQD.%s %s, (--%s)' % (d, fv(fs), iv(it)),
        (3, 0x0E): 'WAITQ',
        (3, 0x0F): 'ISWR.%s %s, (%s)' % (d, iv(it), iv(is_)),
        (3, 0x10): 'RXOR R, %s' % fsfv,
        (3, 0x1C): 'ERLENG P, %sxyz' % fv(fs),
        (3, 0x1E): 'WAITP',
    }
    if (col, sub) not in T: raise Unk
    return T[(col, sub)], None


def disasm(img, base=0):
    n = len(img) // 8
    words = struct.unpack('<%dI' % (n * 2), img[:n * 8])
    items = []; labels = set(); unk = 0; xg = 0; ebits = 0
    for i in range(n):
        lo, hi = words[2 * i], words[2 * i + 1]
        pc = base + i * 8
        flags = ''.join(ch for ch, b in zip('IEMDT', (31, 30, 29, 28, 27)) if hi >> b & 1)
        try: ut = upper(hi)
        except Unk: ut = None
        tg = None
        if hi >> 31 & 1:
            lt = 'LOI %r' % struct.unpack('<f', struct.pack('<I', lo))[0]
        else:
            try: lt, tg = lower(lo, pc)
            except Unk: lt = None
        if ut is None or lt is None: unk += 1
        if tg is not None and 0 <= tg < base + n * 8: labels.add(tg)
        if lt and lt.startswith('XGKICK'): xg += 1
        if 'E' in flags: ebits += 1
        items.append((pc, lo, hi, flags, ut, lt, tg))
    return items, labels, unk, xg, ebits


def render(img, base=0):
    items, labels, unk, xg, eb = disasm(img, base)
    out = []
    for pc, lo, hi, fl, ut, lt, tg in items:
        if pc in labels: out.append('L_%04x:' % pc)
        u = ut if ut is not None else '.unk_up 0x%08x' % hi
        l = lt if lt is not None else '.unk_lo 0x%08x' % lo
        if tg is not None: l += (' L_%04x' % tg) if tg in labels else (' 0x%04x' % tg)
        out.append('  %04x: %08x %08x  %-44s ; %s' % (pc, lo, hi, u, l) + ('   [%s]' % fl if fl else ''))
    return '\n'.join(out) + '\n', len(items), unk, xg, eb


if __name__ == '__main__':
    a = sys.argv[1:]
    base = 0
    if '--base' in a:
        i = a.index('--base'); base = int(a[i + 1], 0); del a[i:i + 2]
    img = open(a[0], 'rb').read()
    txt, n, unk, xg, eb = render(img, base)
    if len(a) > 1: open(a[1], 'w').write(txt)
    else: sys.stdout.write(txt)
    print('; insns=%d unknown=%d xgkick=%d E-bit=%d' % (n, unk, xg, eb), file=sys.stderr)
