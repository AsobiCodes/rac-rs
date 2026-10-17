"""Static decode of the VU0 interpreter's (oracle/src/vu.rs) upper/lower instruction semantics for the AOT generator.
Everything here mirrors vu_upper_g / lower_info / vu_lower (timing-relevant metadata: reads, writes, kinds)."""
NONE = 99
ACC = 32
ADD, SUB, MADD, MSUB, MAX, MIN, MUL = 0, 1, 2, 3, 4, 5, 6
BC_KIND = [ADD, SUB, MADD, MSUB, MAX, MIN, MUL]
QI = [(MUL, False), (MAX, True), (MUL, True), (MIN, True), (ADD, False), (MADD, False),
      (ADD, True), (MADD, True), (SUB, False), (MSUB, False), (SUB, True), (MSUB, True)]
FULL = [ADD, MADD, MUL, MAX, SUB, MSUB, 255, MIN]
KNAME = {ADD: 'ADD', SUB: 'SUB', MADD: 'MADD', MSUB: 'MSUB', MAX: 'MAX', MIN: 'MIN', MUL: 'MUL'}


def lane(l):
    return 8 >> l


class Up:
    """Upper op metadata. kind: 'fmac' (k, tsel) / 'itof' / 'ftoi' / 'abs' / 'nop'.
    tsel: ('full',) | ('bc', n) | ('q',) | ('i',).  dst: vf index / ACC / NONE. mask: lanes (bit3 = x)."""
    def __init__(self):
        self.dst = NONE
        self.mask = 0
        self.mac = False
        self.clip = False
        self.reads = []
        self.kind = 'nop'
        self.k = None
        self.tsel = None
        self.fs = self.ft = self.fd = 0
        self.dest = 0
        self.sc = None


def decode_upper(hi):
    op = hi & 0x07ffffff
    dest = (op >> 21) & 15
    ft = (op >> 16) & 31
    fs = (op >> 11) & 31
    fd = (op >> 6) & 31
    bc = op & 3
    f = op & 0x3f
    u = Up()
    u.fs, u.ft, u.fd, u.dest = fs, ft, fd, dest
    u.mask = dest
    acc_read = lambda k: (ACC, dest) if k in (MADD, MSUB) else (NONE, 0)
    if f < 0x3c:
        if f <= 27:
            k = BC_KIND[f >> 2]
            tsel = ('bc', bc)
            tr = (ft, lane(bc))
        elif f <= 39:
            k, isi = QI[f - 28]
            tsel = ('i',) if isi else ('q',)
            tr = (NONE, 0)
        elif f == 46:
            raise NotImplementedError('OPMSUB')
        elif 40 <= f <= 47:
            k = FULL[f - 40]
            tsel = ('full',)
            tr = (ft, dest)
        else:
            raise ValueError('unknown upper %08x' % op)
        u.kind = 'fmac'
        u.k, u.tsel = k, tsel
        u.dst = fd
        u.reads = [(fs, dest), tr, acc_read(k)]
        u.mac = k not in (MAX, MIN)
        return u
    x = (op & 3) | ((op >> 4) & 0x7c)
    if x <= 15 or 24 <= x <= 27:
        k = BC_KIND[x >> 2]
        u.kind, u.k, u.tsel = 'fmac', k, ('bc', bc)
        u.dst = ACC
        u.reads = [(fs, dest), (ft, lane(bc)), acc_read(k)]
        u.mac = True
    elif 16 <= x <= 19:
        u.kind = 'itof'
        u.sc = [1.0, 0.0625, 1.0 / 4096.0, 1.0 / 32768.0][x - 16]
        u.sci = x - 16
        u.dst = ft
        u.reads = [(fs, dest), (NONE, 0), (NONE, 0)]
    elif 20 <= x <= 23:
        u.kind = 'ftoi'
        u.sci = x - 20
        u.dst = ft
        u.reads = [(fs, dest), (NONE, 0), (NONE, 0)]
    elif x == 29:
        u.kind = 'abs'
        u.dst = ft
        u.reads = [(fs, dest), (NONE, 0), (NONE, 0)]
    elif x == 31:
        raise NotImplementedError('CLIP')
    elif x in (28, 30) or 32 <= x <= 39:
        k, isi = QI[x - 28]
        u.kind, u.k = 'fmac', k
        u.tsel = ('i',) if isi else ('q',)
        u.dst = ACC
        u.reads = [(fs, dest), (NONE, 0), acc_read(k)]
        u.mac = True
    elif 40 <= x <= 42 or x in (44, 45):
        k = FULL[x - 40]
        u.kind, u.k, u.tsel = 'fmac', k, ('full',)
        u.dst = ACC
        u.reads = [(fs, dest), (ft, dest), acc_read(k)]
        u.mac = True
    elif x == 46:
        raise NotImplementedError('OPMULA')
    elif x == 47:
        u.kind = 'nop'
    else:
        raise ValueError('unknown upper %08x' % op)
    return u


class Lo:
    def __init__(self):
        self.reads = []
        self.waitq = False
        self.div = False
        self.name = 'nop'
        self.wvf = None      # (reg, mask) written (ready +4)
        self.branch = None   # None | ('b',) | ('cond', opc) | ('jr',)  (BAL is ('b',), its link register in `link`)
        self.op = 0
        self.wvi = None      # VI register written (None: none)
        self.link = None     # BAL: the VI register that gets the return address
        self.flagread = False  # FMEQ / FMAND / FMOR: reads the MAC flag register


def sx11(op):
    v = op & 0x7ff
    return v - 0x800 if v & 0x400 else v


def decode_lower(lo, pc):
    """Mirror of lower_info + vu_lower effect shape."""
    L = Lo()
    L.op = lo
    dest = (lo >> 21) & 15
    it = (lo >> 16) & 31
    is_ = (lo >> 11) & 31
    id_ = (lo >> 6) & 31
    opc = lo >> 25
    L.dest, L.it, L.is_, L.id_ = dest, it, is_, id_
    L.imm11 = sx11(lo)
    if lo == 0x8000033c:
        L.name = 'nop'
        return L
    if opc == 0x00:
        L.name = 'lq'
        L.wvf = (it, dest)
    elif opc == 0x01:
        L.name = 'sq'
        L.reads = [(is_, dest)]
    elif opc in (0x08, 0x09):
        L.name = 'iaddiu' if opc == 8 else 'isubiu'
        L.im = ((lo >> 10) & 0x7800) | (lo & 0x7ff)
        L.wvi = it
    elif opc in (0x18, 0x1a, 0x1b):
        L.name = {0x18: 'fmeq', 0x1a: 'fmand', 0x1b: 'fmor'}[opc]
        L.flagread = True
        L.wvi = it
    elif opc == 0x20:
        L.name = 'b'
        L.branch = ('b',)
    elif opc == 0x21:
        L.name = 'bal'
        L.branch = ('b',)
        L.link = it
        L.wvi = it
    elif opc == 0x24:
        L.name = 'jr'
        L.branch = ('jr',)
    elif opc in (0x28, 0x29, 0x2c, 0x2d, 0x2e, 0x2f):
        L.name = {0x28: 'ibeq', 0x29: 'ibne', 0x2c: 'ibltz', 0x2d: 'ibgtz', 0x2e: 'iblez', 0x2f: 'ibgez'}[opc]
        L.branch = ('cond', opc)
    elif opc == 0x40:
        f = lo & 0x3f
        if f == 0x34:
            L.name = 'iand'
            L.wvi = id_
        elif f == 0x35:
            L.name = 'ior'
            L.wvi = id_
        elif f == 0x30:
            L.name = 'iadd'
            L.wvi = id_
        elif f == 0x31:
            L.name = 'isub'
            L.wvi = id_
        elif f == 0x32:
            L.name = 'iaddi'
            L.wvi = it
        elif f in (0x3c, 0x3d, 0x3e, 0x3f):
            x = (lo & 3) | ((lo >> 4) & 0x7c)
            fsf = (lo >> 21) & 3
            ftf = (lo >> 23) & 3
            L.fsf, L.ftf = fsf, ftf
            if x == 0x30:
                L.name = 'move'
                L.reads = [(is_, dest)]
                L.wvf = (it, dest)
            elif x == 0x31:
                # MR32: lane l of ft = lane l+1 of fs (the read mask is the dest lanes rotated, as in lower_info)
                L.name = 'mr32'
                m = 0
                for l in range(4):
                    if dest & lane(l):
                        m |= lane((l + 1) & 3)
                L.reads = [(is_, m)]
                L.wvf = (it, dest)
            elif x == 0x38 or x == 0x3a:
                L.name = 'div' if x == 0x38 else 'rsqrt'
                L.reads = [(is_, lane(fsf)), (it, lane(ftf))]
                L.div = True
            elif x == 0x3b:
                L.name = 'waitq'
                L.waitq = True
            elif x == 0x3c:
                L.name = 'mtir'
                L.reads = [(is_, lane(fsf))]
                L.wvi = it
            else:
                raise NotImplementedError('lower special %x at %04x' % (x, pc))
        else:
            raise NotImplementedError('lower f=%x at %04x' % (f, pc))
    else:
        raise NotImplementedError('lower opc %x at %04x' % (opc, pc))
    return L


def branch_target(lo, pc):
    return (pc + 8 + sx11(lo) * 8) & 0xff8
