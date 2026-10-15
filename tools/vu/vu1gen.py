"""VU1 microprogram -> Rust ahead-of-time translator for rac-rs (src/gsfull/vu1_progN.rs + vu1aot_gen.rs).

The output reproduces the interpreter in src/gsfull/vu1.rs bit for bit. The interpreter's timing state (per-register
VF/ACC ready cycles, the pending Q result, the MAC/clip flag queue, the pending branch, the E-bit slot and the branch
hazard register) is explored at generation time: one node = one instruction pair specialised over that state, so the
native code carries no ready tables, only the values. Nodes are chained into basic blocks of a `loop { match }` state
machine. Anything not covered (unknown or unsupported op, a branch in a delay slot, a JR target not in the table, the
runaway cap) "bails": the native code hands the interpreter its exact in-flight state (vu1::Pipe) and the interpreter
finishes the run.

usage: python vu1gen.py [--progs 2,3,4,5,6,7,8,10,11] --bins DIR [--profile profile.txt]
                        --out DIR
profile.txt (optional; from the vu1check tool): lines `prog N entry PC` and `prog N jr SITE TARGET` (hex) add observed
run entries and JR/JALR targets to the static ones (pc 0, the slot after each E bit, every BAL/JALR return address).
"""
import argparse
import os
import struct
import sys

NONE = 99
ACC = 32
ADD, SUB, MADD, MSUB, MAX, MIN, MUL = range(7)
BC_KIND = [ADD, SUB, MADD, MSUB, MAX, MIN, MUL]
QI = [(MUL, 0), (MAX, 1), (MUL, 1), (MIN, 1), (ADD, 0), (MADD, 0), (ADD, 1), (MADD, 1), (SUB, 0), (MSUB, 0), (SUB, 1), (MSUB, 1)]
FULL = [ADD, MADD, MUL, MAX, SUB, MSUB, None, MIN]
GUARD = 3_000_000
NOP_LO = 0x8000033c


def sx(v, bits):
    v &= (1 << bits) - 1
    return v - (1 << bits) if v & (1 << (bits - 1)) else v


def rev4(d):
    """VU dest (bit 3 = x) -> SSE lane mask (bit 0 = lane 0 = x)."""
    return ((d >> 3) & 1) | ((d >> 1) & 2) | ((d << 1) & 4) | ((d << 3) & 8)


def first_lane(d):
    for l in range(4):
        if d & (8 >> l):
            return l
    return 0


class Up:
    pass


def dec_upper(hi):
    op = hi & 0x07ffffff
    u = Up()
    u.op = op
    dest = (op >> 21) & 15
    ft = (op >> 16) & 31
    fs = (op >> 11) & 31
    fd = (op >> 6) & 31
    bc = op & 3
    f = op & 0x3f
    u.dest, u.fs, u.ft, u.fd, u.bc = dest, fs, ft, fd, bc
    u.mask = dest
    u.dst = NONE
    u.mac = False
    u.clip = False
    u.reads = []
    u.kind = None
    u.err = None
    accr = lambda k: [ACC] if k in (MADD, MSUB) else []
    if f < 0x3c:
        if f <= 27:
            k, tsel, tr = BC_KIND[f >> 2], ('bc', bc), [ft]
        elif f <= 39:
            k, isi = QI[f - 28]
            tsel, tr = (('i',) if isi else ('q',)), []
        elif f == 46:
            u.kind = 'opmsub'
            u.dst, u.mask, u.mac, u.reads = fd, dest & 0xe, True, [fs, ft, ACC]
            return u
        elif f <= 47:
            k, tsel, tr = FULL[f - 40], ('full',), [ft]
        else:
            u.err = 'unknown upper %08x' % op
            return u
        u.kind, u.k, u.tsel = 'fmac', k, tsel
        u.dst = fd
        u.reads = [fs] + tr + accr(k)
        u.mac = k not in (MAX, MIN)
        return u
    x = (op & 3) | ((op >> 4) & 0x7c)
    if x <= 15 or 24 <= x <= 27:
        k = BC_KIND[x >> 2]
        u.kind, u.k, u.tsel, u.dst = 'fmac', k, ('bc', bc), ACC
        u.reads = [fs, ft] + accr(k)
        u.mac = True
    elif 16 <= x <= 19:
        u.kind, u.sc, u.dst, u.reads = 'itof', x - 16, ft, [fs]
    elif 20 <= x <= 23:
        u.kind, u.sc, u.dst, u.reads = 'ftoi', x - 20, ft, [fs]
    elif x == 29:
        u.kind, u.dst, u.reads = 'abs', ft, [fs]
    elif x == 31:
        u.kind, u.clip, u.reads = 'clip', True, [fs, ft]
    elif x in (28, 30) or 32 <= x <= 39:
        k, isi = QI[x - 28]
        u.kind, u.k, u.tsel, u.dst = 'fmac', k, (('i',) if isi else ('q',)), ACC
        u.reads = [fs] + accr(k)
        u.mac = True
    elif 40 <= x <= 42 or x in (44, 45):
        k = FULL[x - 40]
        u.kind, u.k, u.tsel, u.dst = 'fmac', k, ('full',), ACC
        u.reads = [fs, ft] + accr(k)
        u.mac = True
    elif x == 46:
        u.kind, u.dst, u.mask, u.mac, u.reads = 'opmula', ACC, dest & 0xe, True, [fs, ft]
    elif x == 47:
        u.kind = 'nop'
    else:
        u.err = 'unknown upper %08x' % op
    return u


class Lo:
    pass


def dec_lower(lo, pc):
    """Mirror of vu1.rs lower_info + lower. L.unsup: reason the native code does not cover it (-> bail)."""
    L = Lo()
    L.lo = lo
    dest = (lo >> 21) & 15
    it = (lo >> 16) & 31
    is_ = (lo >> 11) & 31
    id_ = (lo >> 6) & 31
    opc = lo >> 25
    L.dest, L.it, L.is_, L.id_ = dest, it, is_, id_
    L.imm11 = sx(lo, 11)
    L.reads = []       # vf regs read for stalls (lower_info)
    L.waitq = False
    L.div = False
    L.lat = 0
    L.wvf = 0          # vf reg written (ready +4), 0 = none
    L.wvi = 0          # vi reg written (1..15), 0 = none
    L.branch = None    # None | 'b' | 'c' | 'd'
    L.breads = []      # vi regs read through the branch hazard
    L.tgt = None
    L.unsup = None
    L.name = '?'
    vir = []           # vi regs read (non-flag)
    viw = []           # vi regs written
    if opc == 0x00:
        L.name = 'lq'
        vir = [is_]
        if it:
            L.wvf = it
    elif opc == 0x01:
        L.name = 'sq'
        L.reads = [is_]
        vir = [it]
    elif opc == 0x04:
        L.name = 'ilw'
        vir, viw = [is_], [it]
    elif opc == 0x05:
        L.name = 'isw'
        vir = [is_, it]
    elif opc in (0x08, 0x09):
        L.name = 'iaddiu' if opc == 8 else 'isubiu'
        L.im = ((lo >> 10) & 0x7800) | (lo & 0x7ff)
        vir, viw = [is_], [it]
    elif 0x10 <= opc <= 0x13:
        L.name = ['fceq', 'fcset', 'fcand', 'fcor'][opc - 0x10]
        L.im = lo & 0xffffff
        if opc != 0x11:
            viw = [1]
    elif 0x14 <= opc <= 0x17:
        L.name = ['fseq', 'fsset', 'fsand', 'fsor'][opc - 0x14]
        L.im = ((lo >> 10) & 0x800) | (lo & 0x7ff)
        if opc != 0x15:
            viw = [it]
    elif opc in (0x18, 0x1a, 0x1b):
        L.name = {0x18: 'fmeq', 0x1a: 'fmand', 0x1b: 'fmor'}[opc]
        vir, viw = [is_], [it]
    elif opc == 0x1c:
        L.name = 'fcget'
        viw = [it]
    elif opc in (0x20, 0x21):
        L.name = 'b' if opc == 0x20 else 'bal'
        L.branch = 'b'
        L.tgt = (pc + 8 + L.imm11 * 8) & 0x3ff8
        if opc == 0x21:
            viw = [it]
    elif opc in (0x24, 0x25):
        L.name = 'jr' if opc == 0x24 else 'jalr'
        L.branch = 'd'
        L.breads = [is_]
        vir = [is_]
        if opc == 0x25:
            viw = [it]
    elif opc in (0x28, 0x29, 0x2c, 0x2d, 0x2e, 0x2f):
        L.name = {0x28: 'ibeq', 0x29: 'ibne', 0x2c: 'ibltz', 0x2d: 'ibgtz', 0x2e: 'iblez', 0x2f: 'ibgez'}[opc]
        L.branch = 'c'
        L.tgt = (pc + 8 + L.imm11 * 8) & 0x3ff8
        L.breads = [is_, it] if opc in (0x28, 0x29) else [is_]
        vir = list(L.breads)
    elif opc == 0x40:
        f = lo & 0x3f
        if f in (0x30, 0x31, 0x34, 0x35):
            L.name = {0x30: 'iadd', 0x31: 'isub', 0x34: 'iand', 0x35: 'ior'}[f]
            vir, viw = [is_, it], [id_]
        elif f == 0x32:
            L.name = 'iaddi'
            L.im = sx((lo >> 6) & 31, 5)
            vir, viw = [is_], [it]
        elif 0x3c <= f <= 0x3f:
            x = (lo & 3) | ((lo >> 4) & 0x7c)
            L.fsf = (lo >> 21) & 3
            L.ftf = (lo >> 23) & 3
            L.x = x
            if x == 0x30 or x == 0x31:
                L.name = 'move' if x == 0x30 else 'mr32'
                L.reads = [is_]
                if it:
                    L.wvf = it
            elif x in (0x34, 0x36):
                L.name = 'lqi' if x == 0x34 else 'lqd'
                vir, viw = [is_], [is_]
                if it:
                    L.wvf = it
            elif x in (0x35, 0x37):
                L.name = 'sqi' if x == 0x35 else 'sqd'
                L.reads = [is_]
                vir, viw = [it], [it]
            elif x in (0x38, 0x3a):
                L.name = 'div' if x == 0x38 else 'rsqrt'
                L.reads = [is_, it]
                L.div = True
                L.lat = 7 if x == 0x38 else 13
            elif x == 0x39:
                L.name = 'sqrt'
                L.reads = [it]
                L.div = True
                L.lat = 7
            elif x == 0x3b:
                L.name = 'waitq'
                L.waitq = True
            elif x == 0x3c:
                L.name = 'mtir'
                L.reads = [is_]
                viw = [it]
            elif x == 0x3d:
                L.name = 'mfir'
                vir = [is_]
                if it:
                    L.wvf = it
            elif x == 0x3e:
                L.name = 'ilwr'
                vir, viw = [is_], [it]
            elif x == 0x3f:
                L.name = 'iswr'
                vir = [is_, it]
            elif x in (0x40, 0x41):
                L.name = 'rnext' if x == 0x40 else 'rget'
                if it:
                    L.wvf = it
            elif x in (0x42, 0x43):
                L.name = 'rinit' if x == 0x42 else 'rxor'
                L.reads = [is_]
            elif x in (0x68, 0x69):
                L.name = 'xtop' if x == 0x68 else 'xitop'
                viw = [it]
            elif x == 0x6c:
                L.name = 'xgkick'
                vir = [is_]
            else:
                L.unsup = 'unknown lower %08x' % lo
        else:
            L.unsup = 'unknown lower %08x' % lo
    else:
        L.unsup = 'unknown lower %08x' % lo
    if L.unsup is None:
        if any(r >= 16 for r in vir) or any(r >= 16 for r in viw):
            L.unsup = 'vi register >= 16 as an integer operand'
    L.vir = [r for r in vir if r]
    # flag reads (FSAND / FCAND / FMAND / FCGET / ...) never delay a following branch (microVU analyzeBranchVI)
    L.wvi = 0 if 0x10 <= opc <= 0x1c else next((r for r in viw if r and r < 16), 0)
    L.breads = [r for r in L.breads if r]
    return L


class Pair:
    pass


class Prog:
    def __init__(self, n, code):
        self.n = n
        self.code = code  # list of u64 pairs
        self.size = len(code) * 8
        self._p = {}

    def pair(self, pc):
        if pc in self._p:
            return self._p[pc]
        P = Pair()
        P.pc = pc
        P.unsup = None
        if pc >= self.size:
            P.unsup = 'outside the program'
            P.ibit = P.ebit = False
            P.branch = None
            P.breads = []
            P.wvi = 0
            self._p[pc] = P
            return P
        w = self.code[pc >> 3]
        P.lo, P.hi = w & 0xffffffff, w >> 32
        P.ibit = bool(P.hi & 0x80000000)
        P.ebit = bool(P.hi & 0x40000000)
        P.up = dec_upper(P.hi)
        if P.up.err:
            P.unsup = P.up.err
        P.L = None if P.ibit else dec_lower(P.lo, pc)
        if P.L is not None and P.L.unsup and not P.unsup:
            P.unsup = P.L.unsup
        P.lreads = P.L.reads if P.L else []
        P.waitq = bool(P.L and P.L.waitq)
        P.div = bool(P.L and P.L.div)
        P.branch = P.L.branch if P.L else None
        P.breads = P.L.breads if P.L else []
        P.wvi = P.L.wvi if P.L else 0
        P.wvf = P.L.wvf if P.L else 0
        self._p[pc] = P
        return P


# ---------------------------------------------------------------- exploration
# node = (pc, rd, q, fl, pend, end, hz)
#   rd: tuple(sorted (reg, rem>0))  VF/ACC registers still in flight (stall if read)
#   q: None | rem>=0                pending Q result
#   fl: tuple((rem>=0, kind 0 MAC / 1 clip, producer pc, instance)) in issue order
#   pend: None | ('b', tgt) | ('c', tgt) | ('d', None)   branch of the previous pair (this pair is its delay slot)
#   end: this pair is the E-bit slot (run ends after it)
#   hz: vi register the previous pair's lower wrote, when this pair's branch reads it (old value in `hzo`)

class Step:
    pass


def step(prog, node):
    pc, rd, q, fl, pend, end, hz = node
    P = prog.pair(pc)
    S = Step()
    S.P = P
    S.bail = None
    S.save_hz = False
    if P.unsup:
        S.bail = P.unsup
        return S
    if pend is not None and P.branch:
        S.bail = 'branch in a delay slot'
        return S
    rdd = dict(rd)
    s = 0
    for r in list(P.up.reads) + list(P.lreads):
        s = max(s, rdd.get(r, 0))
    if (P.waitq or P.div) and q is not None:
        s = max(s, q)
    S.stall = s
    S.q_land = q is not None and s >= q
    land = [e for e in fl if e[0] <= s]
    keep = [e for e in fl if e[0] > s]
    assert fl[:len(land)] == tuple(land), 'flag queue not FIFO'
    S.land = land
    nrd = {r: v - s - 1 for r, v in rdd.items() if v - s - 1 > 0}
    if P.wvf:
        nrd[P.wvf] = 3
    if P.up.dst not in (NONE, 0):
        nrd[P.up.dst] = 3
    nfl = [(e[0] - s - 1, e[1], e[2], e[3]) for e in keep]
    S.new_entries = []
    kinds = []
    if P.up.mac or P.up.clip:
        kinds.append(0 if P.up.mac else 1)
    if P.L and P.L.name == 'fcset':
        assert not P.up.clip, 'CLIP and FCSET in one pair at %04x' % pc
        kinds.append(1)  # FCSET: a new clip flag instance, issued after the upper's
    for kind in kinds:
        used = {e[3] for e in nfl if e[1] == kind and e[2] == pc}
        inst = 0
        while inst in used:
            inst += 1
        e = (3, kind, pc, inst)
        S.new_entries.append(e)
        nfl.append(e)
    if P.div:
        nq = P.L.lat - 1
    elif q is None or S.q_land:
        nq = None
    else:
        nq = q - s - 1
    S.T = (tuple(sorted(nrd.items())), nq, tuple(nfl))
    # successors
    S.succ = []        # list of (label, node)   label: 'goto' | 'tk' | 'nt' | ('arm', target)
    S.exit = False
    S.dyn = False
    if end:
        S.exit = True
        return S
    nend = P.ebit
    npend = None
    if P.branch == 'b':
        npend = ('b', P.L.tgt)
    elif P.branch == 'c':
        npend = ('c', P.L.tgt)
    elif P.branch == 'd':
        npend = ('d', None)

    def mk(t):
        t &= 0x3ff8
        h = 0
        if P.wvi:
            Q = prog.pair(t)
            if P.wvi in Q.breads:
                h = P.wvi
        return (t, S.T[0], S.T[1], S.T[2], npend, nend, h)

    if pend is None:
        S.succ.append(('goto', mk(pc + 8)))
    elif pend[0] == 'b':
        S.succ.append(('goto', mk(pend[1])))
    elif pend[0] == 'c':
        S.succ.append(('tk', mk(pend[1])))
        S.succ.append(('nt', mk(pc + 8)))
    else:
        S.dyn = True
        # this pair is the JR's delay slot (a JR in a delay slot bails), so the JR sits at pc - 8
        for t in sorted(prog.jr_targets((pc - 8) & 0x3ff8)):
            S.succ.append((('arm', t), mk(t)))
    # does the next pair need the old value of the register this lower writes?
    S.save_hz = bool(P.wvi) and (S.dyn or any(n[6] == P.wvi for _, n in S.succ))
    return S


def explore(prog, entries):
    graph = {}
    order = []
    ent = {}
    work = []
    for e in entries:
        n = (e, (), None, (), None, False, 0)
        ent[e] = n
        work.append(n)
    while work:
        n = work.pop()
        if n in graph:
            continue
        S = step(prog, n)
        graph[n] = S
        order.append(n)
        if S.bail is None:
            for _, m in S.succ:
                if m not in graph:
                    work.append(m)
    return graph, order, ent


# ---------------------------------------------------------------- emission

def vi(r):
    return '0u32' if r == 0 else 'i%d' % r


def bvi(r, hz):
    if r == 0:
        return '0u32'
    return 'hzo' if r == hz else 'i%d' % r


def fv(r):
    return 'f%d' % r


def mvar(e):
    return ('m_%x_%d' if e[1] == 0 else 'c_%x_%d') % (e[2], e[3])


def m4(dest, inl, outl):
    """_mm_set_epi32 constant: lanes in dest get inl, others outl."""
    v = [inl if dest & (8 >> l) else outl for l in range(4)]
    return '_mm_set_epi32(%d, %d, %d, %d)' % (v[3], v[2], v[1], v[0])


def u32lit(v):
    return '0x%x' % (v & 0xffffffff)


class Gen:
    def __init__(self, prog, graph, order, ent):
        self.prog = prog
        self.g = graph
        self.order = order
        self.ent = ent
        self.id = {n: k for k, n in enumerate(order)}
        self.mvars = set()
        self.wvf = set()

    # --- upper
    def operand(self, r):
        return 'acc' if r == ACC else fv(r)

    def tval(self, u):
        ts = u.tsel
        if ts[0] == 'bc':
            return '_mm_shuffle_ps::<0x%02x>(%s, %s)' % (u.bc * 0x55, fv(u.ft), fv(u.ft))
        if ts[0] == 'q':
            return 'splat!(q)'
        if ts[0] == 'i':
            return 'splat!(ri)'
        return fv(u.ft)

    def upper_expr(self, u):
        """Expression of the upper result (computed before the lower runs) or None."""
        if u.kind == 'fmac':
            s = fv(u.fs)
            t = self.tval(u)
            k = u.k
            if k == ADD:
                return '_mm_add_ps(cl!(%s), cl!(%s))' % (s, t)
            if k == SUB:
                return '_mm_sub_ps(cl!(%s), cl!(%s))' % (s, t)
            if k == MUL:
                return '_mm_mul_ps(cl!(%s), cl!(%s))' % (s, t)
            if k == MADD:
                return '_mm_add_ps(cl!(acc), _mm_mul_ps(cl!(%s), cl!(%s)))' % (s, t)
            if k == MSUB:
                return '_mm_sub_ps(cl!(acc), _mm_mul_ps(cl!(%s), cl!(%s)))' % (s, t)
            if k == MAX:
                return 'vmax!(%s, %s)' % (s, t)
            if k == MIN:
                return 'vmin!(%s, %s)' % (s, t)
        if u.kind in ('opmula', 'opmsub'):
            p = '_mm_mul_ps(cl!(_mm_shuffle_ps::<0xc9>(%s, %s)), cl!(_mm_shuffle_ps::<0xd2>(%s, %s)))' % (
                fv(u.fs), fv(u.fs), fv(u.ft), fv(u.ft))
            return p if u.kind == 'opmula' else '_mm_sub_ps(cl!(acc), %s)' % p
        if u.kind == 'itof':
            return 'itof!(%s, %d)' % (fv(u.fs), u.sc)
        if u.kind == 'ftoi':
            return 'ftoi!(%s, %d)' % (fv(u.fs), u.sc)
        if u.kind == 'abs':
            return '_mm_and_ps(%s, absm!())' % fv(u.fs)
        return None

    # --- lower
    def addr(self, r, off):
        if r == 0:
            return '%d' % (off & 0x3ff)
        if off == 0:
            return '(%s & 0x3ff) as usize' % vi(r)
        return '(%s.wrapping_add(%s) & 0x3ff) as usize' % (vi(r), u32lit(off))

    def wvf_code(self, r, val, dest):
        if r == 0 or dest == 0:
            return []
        self.wvf.add(r)
        if dest == 15:
            return ['%s = %s;' % (fv(r), val)]
        return ['%s = _mm_blend_ps::<0x%x>(%s, %s);' % (fv(r), rev4(dest), fv(r), val)]

    def store_code(self, a, val, dest):
        if dest == 0:
            return []
        if dest == 15:
            return ['st!(d, %s, %s);' % (a, val)]
        return ['{ let a = %s; st!(d, a, _mm_blend_ps::<0x%x>(ld!(d, a), %s)); }' % (a, rev4(dest), val)]

    def setvi(self, r, expr, mask=True):
        if r == 0:
            return []
        return ['i%d = (%s)%s;' % (r, expr, ' & 0xffff' if mask else '')]

    def lower_code(self, P, node, S):
        L = P.L
        hz = node[6]
        out = []
        if L is None or L.lo == NOP_LO:
            return out
        if S.save_hz:
            out.append('hzo = i%d;' % P.wvi)
        n = L.name
        dest = L.dest
        if n == 'lq':
            out += self.wvf_code(L.it, 'ld!(d, %s)' % self.addr(L.is_, L.imm11), dest)
        elif n == 'sq':
            out += self.store_code(self.addr(L.it, L.imm11), fv(L.is_), dest)
        elif n == 'ilw':
            out += self.setvi(L.it, '(*d.add(%s))[%d]' % (self.addr(L.is_, L.imm11), first_lane(dest)))
        elif n == 'isw':
            if dest:
                lanes = [l for l in range(4) if dest & (8 >> l)]
                out.append('{ let a = %s; let v = %s & 0xffff; %s }' % (
                    self.addr(L.is_, L.imm11), vi(L.it), ' '.join('(*d.add(a))[%d] = v;' % l for l in lanes)))
        elif n in ('iaddiu', 'isubiu'):
            op = 'wrapping_add' if n == 'iaddiu' else 'wrapping_sub'
            out += self.setvi(L.it, '%s.%s(%s)' % (vi(L.is_), op, u32lit(L.im)))
        elif n == 'fceq':
            out.append('i1 = ((c18 & 0xffffff) == %s) as u32;' % u32lit(L.im))
        elif n == 'fcset':
            # a new clip flag instance, like CLIP: the next CLIP shifts it; FCAND & co see it once it lands
            pass
        elif n == 'fcand':
            out.append('i1 = ((c18 & 0xffffff & %s) != 0) as u32;' % u32lit(L.im))
        elif n == 'fcor':
            out.append('i1 = (((c18 | %s) & 0xffffff) == 0xffffff) as u32;' % u32lit(L.im))
        elif n == 'fcget':
            out += self.setvi(L.it, 'c18 & 0xfff', mask=False)
        elif n in ('fsand', 'fseq', 'fsor'):
            im = L.im
            if n == 'fsand' and im & 0xfc0 == 0:
                s = '(if vism == NOMAC { b16 & 0x3f } else { (b16 & 0x30) | st_of(mac_of(arr!(visv), vism)) })'
            else:
                s = '(mat16(b16, visv, vism, zacc, sacc) & 0xfff)'
            if n == 'fsand':
                out += self.setvi(L.it, '%s & %s' % (s, u32lit(im)), mask=False)
            elif n == 'fseq':
                out += self.setvi(L.it, '(%s == %s) as u32' % (s, u32lit(im)), mask=False)
            else:
                out += self.setvi(L.it, '((%s | %s) == 0xfff) as u32' % (s, u32lit(im)), mask=False)
        elif n == 'fsset':
            out.append('{ let v = (mat16(b16, visv, vism, zacc, sacc) & 0x3f) | %s; b17 = mat17(b17, visv, vism); b16 = v; vism = NOMAC; zacc = _mm_set1_epi32(-1); sacc = _mm_setzero_si128(); }'
                       % u32lit(L.im & 0xfc0))
        elif n in ('fmeq', 'fmand', 'fmor'):
            e = {'fmeq': '((mat17(b17, visv, vism) & 0xffff) == %s) as u32', 'fmand': 'mat17(b17, visv, vism) & %s', 'fmor': 'mat17(b17, visv, vism) | %s'}[n] % vi(L.is_)
            out += self.setvi(L.it, e, mask=True)
        elif n in ('b', 'bal'):
            if n == 'bal':
                out += self.setvi(L.it, '%d' % ((P.pc + 16) // 8), mask=False)
        elif n in ('jr', 'jalr'):
            out.append('jt = ((%s as usize) * 8) & 0x3ff8;' % bvi(L.is_, hz))
            if n == 'jalr':
                out += self.setvi(L.it, '%d' % ((P.pc + 16) // 8), mask=False)
        elif L.branch == 'c':
            a = '(%s as u16 as i16)' % bvi(L.is_, hz)
            b = '(%s as u16 as i16)' % bvi(L.it, hz)
            c = {'ibeq': '%s == %s' % (a, b), 'ibne': '%s != %s' % (a, b), 'ibltz': '%s < 0' % a,
                 'ibgtz': '%s > 0' % a, 'iblez': '%s <= 0' % a, 'ibgez': '%s >= 0' % a}[n]
            out.append('tk = %s;' % c)
        elif n in ('iadd', 'isub', 'iand', 'ior'):
            e = {'iadd': '%s.wrapping_add(%s)', 'isub': '%s.wrapping_sub(%s)', 'iand': '(%s & %s)', 'ior': '(%s | %s)'}[n] % (vi(L.is_), vi(L.it))
            out += self.setvi(L.id_, e)
        elif n == 'iaddi':
            out += self.setvi(L.it, '%s.wrapping_add(%s)' % (vi(L.is_), u32lit(L.im)))
        elif n in ('move', 'mr32'):
            v = fv(L.is_) if n == 'move' else '_mm_shuffle_ps::<0x39>(%s, %s)' % (fv(L.is_), fv(L.is_))
            out += self.wvf_code(L.it, v, dest)
        elif n in ('lqi', 'lqd'):
            if n == 'lqd':
                out += self.setvi(L.is_, '%s.wrapping_sub(1)' % vi(L.is_))
            out += self.wvf_code(L.it, 'ld!(d, %s)' % self.addr(L.is_, 0), dest)
            if n == 'lqi':
                out += self.setvi(L.is_, '%s.wrapping_add(1)' % vi(L.is_))
        elif n in ('sqi', 'sqd'):
            if n == 'sqd':
                out += self.setvi(L.it, '%s.wrapping_sub(1)' % vi(L.it))
            out += self.store_code(self.addr(L.it, 0), fv(L.is_), dest)
            if n == 'sqi':
                out += self.setvi(L.it, '%s.wrapping_add(1)' % vi(L.it))
        elif n in ('div', 'sqrt', 'rsqrt'):
            s = 'lane!(%s, %d)' % (fv(L.is_), L.fsf)
            t = 'lane!(%s, %d)' % (fv(L.it), L.ftf)
            e = {'div': 'fdiv(%s, %s)' % (s, t), 'sqrt': 'fsqrt(%s)' % t, 'rsqrt': 'fdiv(%s, fsqrt(%s))' % (s, t)}[n]
            out.append('qn = %s;' % e)
        elif n == 'waitq':
            pass
        elif n == 'mtir':
            out += self.setvi(L.it, 'lane!(%s, %d)' % (fv(L.is_), L.fsf))
        elif n == 'mfir':
            out += self.wvf_code(L.it, 'splat!(%s as u16 as i16 as i32 as u32)' % vi(L.is_), dest)
        elif n == 'ilwr':
            out += self.setvi(L.it, '(*d.add(%s))[%d]' % (self.addr(L.is_, 0), first_lane(dest)))
        elif n == 'iswr':
            if dest:
                lanes = [l for l in range(4) if dest & (8 >> l)]
                out.append('{ let a = %s; let v = %s & 0xffff; %s }' % (
                    self.addr(L.is_, 0), vi(L.it), ' '.join('(*d.add(a))[%d] = v;' % l for l in lanes)))
        elif n in ('rnext', 'rget'):
            if n == 'rnext':
                out.append('rr = (((rr << 1) ^ (((rr >> 4) & 1) ^ ((rr >> 22) & 1))) & 0x7fffff) | 0x3f800000;')
            out += self.wvf_code(L.it, 'splat!(rr)', dest)
        elif n == 'rinit':
            out.append('rr = 0x3f800000 | (lane!(%s, %d) & 0x7fffff);' % (fv(L.is_), L.fsf))
        elif n == 'rxor':
            out.append('rr = 0x3f800000 | ((rr ^ lane!(%s, %d)) & 0x7fffff);' % (fv(L.is_), L.fsf))
        elif n == 'xtop':
            out += self.setvi(L.it, 'top')
        elif n == 'xitop':
            out += self.setvi(L.it, 'itop')
        elif n == 'xgkick':
            out.append('vu.st.xgkicks += 1; sink.xgkick(std::slice::from_raw_parts(d, 1024), (%s & 0x3ff) as usize);' % vi(L.is_))
        else:
            raise ValueError('no emitter for %s' % n)
        return out

    # --- flags
    def land_code(self, entries):
        """Landing of flag entries (issue order). MAC: visible flag + sticky; clip: vi18."""
        out = []
        macs = [e for e in entries if e[1] == 0]
        for e in macs:
            m = mvar(e)
            dest = self.prog.pair(e[2]).up.mask
            self.mvars.add(('m', m))
            out.append('zacc = _mm_min_epu32(zacc, _mm_or_si128(_mm_slli_epi32::<1>(_mm_castps_si128(%s)), %s));' % (m, m4(dest, 0, -1)))
            out.append('sacc = _mm_or_si128(sacc, _mm_and_si128(_mm_castps_si128(%s), %s));' % (m, m4(dest, -0x80000000, 0)))
        if macs:
            e = macs[-1]
            out.append('visv = %s; vism = %d;' % (mvar(e), self.prog.pair(e[2]).up.mask))
        clips = [e for e in entries if e[1] == 1]
        if clips:
            self.mvars.add(('c', mvar(clips[-1])))
            out.append('c18 = %s;' % mvar(clips[-1]))
        return out

    def pair_code(self, node, S):
        P = S.P
        out = ['// %04x%s' % (node[0], ' [E]' if P.ebit else '')]
        if S.q_land:
            out.append('q = qn;')
        out += self.land_code(S.land)
        u = P.up
        ex = self.upper_expr(u)
        if ex is not None:
            out.append('let u = %s;' % ex)
        if u.kind == 'clip':
            out.append('let cnew = clip!(%s, %s, cl);' % (fv(u.fs), fv(u.ft)))
        out += self.lower_code(P, node, S)
        if ex is not None and u.dst not in (NONE, 0):
            if u.dst == ACC:
                if u.mask == 15:
                    out.append('acc = u;')
                elif u.mask:
                    out.append('acc = _mm_blend_ps::<0x%x>(acc, u);' % rev4(u.mask))
            else:
                out += self.wvf_code(u.dst, 'u', u.mask)
        for k, e in enumerate(S.new_entries):
            self.mvars.add(('m' if e[1] == 0 else 'c', mvar(e)))
            if e[1] == 0:
                out.append('%s = u;' % mvar(e))
            elif P.L and P.L.name == 'fcset' and k == len(S.new_entries) - 1:
                out.append('cl = %s; %s = cl;' % (u32lit(P.L.im), mvar(e)))
            else:
                out.append('cl = cnew; %s = cnew;' % mvar(e))
        if P.ibit:
            out.append('ri = %s;' % u32lit(P.lo))
        return out

    def exit_code(self, node, S):
        """After the E-bit slot: flush Q and the flag queue, then leave."""
        out = []
        _, nq, nfl = S.T
        if nq is not None:
            out.append('q = qn;')
        out += self.land_code(nfl)
        out.append('tpc = 0x%x; break \'run;' % ((node[0] + 8) & 0x3ff8))
        return out

    # --- blocks
    def build(self):
        g = self.g
        preds = {n: 0 for n in self.order}
        for n in self.order:
            S = g[n]
            if S.bail is None:
                for _, m in S.succ:
                    preds[m] += 1
        # loop headers: targets of back edges in a DFS from the entries
        headers = set()
        state = {}
        for e in self.ent.values():
            if e in state:
                continue
            stack = [(e, iter([m for _, m in g[e].succ] if g[e].bail is None else []))]
            state[e] = 1
            while stack:
                n, it = stack[-1]
                m = next(it, None)
                if m is None:
                    state[n] = 2
                    stack.pop()
                    continue
                st = state.get(m)
                if st == 1:
                    headers.add(m)
                elif st is None:
                    state[m] = 1
                    stack.append((m, iter([x for _, x in g[m].succ] if g[m].bail is None else [])))
        self.headers = headers
        leaders = set(self.ent.values()) | headers
        for n in self.order:
            S = g[n]
            if preds[n] != 1 or S.bail is not None:
                leaders.add(n)
            if S.bail is None and len(S.succ) != 1:
                for _, m in S.succ:
                    leaders.add(m)
            if S.bail is None and S.dyn:
                for _, m in S.succ:
                    leaders.add(m)
        self.leaders = leaders
        blocks = []
        for n in self.order:
            if n not in leaders:
                continue
            chain = [n]
            while True:
                S = g[chain[-1]]
                if S.bail is not None or S.exit or S.dyn or len(S.succ) != 1:
                    break
                m = S.succ[0][1]
                if m in leaders:
                    break
                chain.append(m)
            blocks.append(chain)
        self.blocks = blocks
        self.bid = {b[0]: k for k, b in enumerate(blocks)}
        covered = sum(len(b) for b in blocks)
        assert covered == len(self.order), (covered, len(self.order))

    def emit(self):
        g = self.g
        self.bails = []   # (bail id, node, pc expr, label)
        maxpath = len(self.order) + 8
        lim = GUARD - maxpath
        arms = []
        for k, chain in enumerate(self.blocks):
            head = chain[0]
            body = []
            if g[head].bail is not None:
                bid = self.add_bail(head, None, g[head].bail)
                arms.append('%d => { bail = %d; break \'run; } // %s' % (k, bid, g[head].bail))
                continue
            if head in self.headers:
                bid = self.add_bail(head, None, 'runaway cap')
                body.append('if pairs > %d { bail = %d; break \'run; }' % (lim, bid))
            cyc = sum(g[n].stall + 1 for n in chain)
            body.append('pairs += %d; cyc += %d;' % (len(chain), cyc))
            for n in chain:
                body += self.pair_code(n, g[n])
            last = chain[-1]
            S = g[last]
            if S.exit:
                body += self.exit_code(last, S)
            elif S.dyn:
                lines = []
                for lab, m in S.succ:
                    lines.append('0x%x => %d,' % (lab[1], self.target(m)))
                bid = self.add_bail(last, 'jt', 'JR target not in the table', after=True)
                lines.append('_ => { bail = %d; break \'run; }' % bid)
                body.append('n = match jt { %s };' % ' '.join(lines))
            elif len(S.succ) == 2:
                (la, a), (lb, b) = S.succ
                assert la == 'tk' and lb == 'nt'
                body.append('n = if tk { %d } else { %d };' % (self.target(a), self.target(b)))
            else:
                body.append('n = %d;' % self.target(S.succ[0][1]))
            arms.append('%d => {\n            %s\n        }' % (k, '\n            '.join(body)))
        return arms

    def target(self, m):
        return self.bid[m]

    def add_bail(self, node, pcexpr, why, after=False):
        """after=True: the state *after* `node` (its successor state with a dynamic pc)."""
        self.bails.append((len(self.bails), node, pcexpr, why, after))
        return len(self.bails) - 1

    def bail_arm(self, bid, node, pcexpr, why, after):
        g = self.g
        out = []
        if after:
            S = g[node]
            rd, q, fl = S.T
            assert node[4] == ('d', None) and S.P.branch is None
            end = S.P.ebit
            pcs = 'jt'
            hz = S.P.wvi if S.save_hz else 0
            # the pending branch of the JR's delay slot is consumed; a JR in the delay slot was refused earlier
            pend = None
        else:
            pc, rd, q, fl, pend, end, hz = node
            pcs = '0x%x' % pc
        out.append('p.pc = %s;' % pcs)
        for r, rem in rd:
            out.append('p.ready[%d] = cyc + %d;' % (r, rem))
        if q is not None:
            out.append('p.q_pend = Some((cyc + %d, qn));' % q)
        for e in fl:
            if e[1] == 0:
                dest = self.prog.pair(e[2]).up.mask
                self.mvars.add(('m', mvar(e)))
                out.append('{ let m = mac_of(arr!(%s), %d); p.flags.push_back((cyc + %d, 0, m, st_of(m))); }' % (mvar(e), dest, e[0]))
            else:
                self.mvars.add(('c', mvar(e)))
                out.append('p.flags.push_back((cyc + %d, 1, %s, 0));' % (e[0], mvar(e)))
        if pend is not None:
            if pend[0] == 'b':
                out.append('p.taken = Some(0x%x);' % pend[1])
            elif pend[0] == 'c':
                out.append('p.taken = if tk { Some(0x%x) } else { None };' % pend[1])
            else:
                out.append('p.taken = Some(jt);')
        if end:
            out.append('p.end_next = true;')
        if hz:
            out.append('p.prev_old.copy_from_slice(&vu.vi[..16]); p.prev_old[%d] = hzo; p.prev_mask = ((hzo != i%d) as u32) << %d;' % (hz, hz, hz))
        return '%d => { %s } // %s' % (bid, ' '.join(out), why)


def file_text(prog, gen, arms, entries_map):
    g = gen.g
    used_vf = set()
    for n in gen.order:
        P = prog.pair(n[0])
        if P.unsup or not hasattr(P, 'up'):
            continue
        u = P.up
        for r in (u.fs, u.ft, u.fd):
            used_vf.add(r)
        if P.L:
            used_vf.update([P.L.is_, P.L.it])
    used_vf = sorted(r for r in used_vf if r < 32)
    bail_arms = [gen.bail_arm(*b) for b in gen.bails]
    lines = []
    w = lines.append
    w('// Generated by rac-rs/tools/vu1aot/vu1gen.py from decomp/vu1/prog%d.bin. Do not edit.' % prog.n)
    w('// %d pairs, %d nodes, %d blocks, %d bail points, entries %s.' % (
        len(prog.code), len(gen.order), len(gen.blocks), len(gen.bails), ' '.join('%x' % e for e in sorted(entries_map))))
    w('#![allow(unused_mut, unused_variables, unused_assignments, unused_unsafe, unused_imports, unused_parens, unreachable_code, clippy::all)]')
    w('use crate::gsfull::vu1::{fdiv, fsqrt, mac_of, st_of, Path1, Pipe, Vu1};')
    w('use crate::gsfull::vu1aot::{absm, arr, cl, clip, ftoi, itof, lane, ld, ldr, mat16, mat17, splat, st, vmax, vmin, Exec, NOMAC};')
    w('use std::arch::x86_64::*;')
    w('')
    w('pub(crate) static CODE: [u64; %d] = [' % len(prog.code))
    for k in range(0, len(prog.code), 4):
        w('    ' + ' '.join('0x%016x,' % x for x in prog.code[k:k + 4]))
    w('];')
    w('')
    w('#[target_feature(enable = "sse4.1")]')
    w('pub(crate) unsafe fn run(vu: &mut Vu1, entry: usize, top: u32, itop: u32, sink: &mut dyn Path1) -> Exec {')
    w('    unsafe {')
    w('    let mut n: u32 = match entry & 0x3ff8 {')
    for e in sorted(entries_map):
        w('        0x%x => %d,' % (e, gen.bid[entries_map[e]]))
    w('        _ => return Exec::NotCovered,')
    w('    };')
    w('    let d: *mut [u32; 4] = vu.data.as_mut_ptr();')
    for r in range(32):
        w('    let mut f%d: __m128 = ldr!(vu.vf[%d]);' % (r, r))
    w('    let mut acc: __m128 = ldr!(vu.acc);')
    for r in range(1, 16):
        w('    let mut i%d: u32 = vu.vi[%d];' % (r, r))
    w('    let (mut b16, mut b17, mut c18, mut rr, mut ri, mut q) = (vu.vi[16], vu.vi[17], vu.vi[18], vu.vi[20], vu.vi[21], vu.vi[22]);')
    w('    let mut cl: u32 = c18;')
    w('    let mut qn: u32 = 0;')
    w('    let mut visv: __m128 = _mm_setzero_ps();')
    w('    let mut vism: u32 = NOMAC;')
    w('    let mut zacc: __m128i = _mm_set1_epi32(-1);')
    w('    let mut sacc: __m128i = _mm_setzero_si128();')
    w('    let mut tk = false;')
    w('    let mut jt: usize = 0;')
    w('    let mut hzo: u32 = 0;')
    w('    let mut cyc: u64 = 0;')
    w('    let mut pairs: u64 = 0;')
    w('    let mut bail: u32 = u32::MAX;')
    w('    let mut tpc: usize = 0;')
    for kind, name in sorted(gen.mvars):
        if kind == 'm':
            w('    let mut %s: __m128 = _mm_setzero_ps();' % name)
        else:
            w('    let mut %s: u32 = 0;' % name)
    w("    'run: loop {")
    w('        match n {')
    for a in arms:
        w('        ' + a)
    w('        _ => unreachable!(),')
    w('        }')
    w('    }')
    w('    for (k, v) in [%s].into_iter().enumerate() {' % ', '.join('f%d' % r for r in range(32)))
    w('        vu.vf[k] = arr!(v);')
    w('    }')
    w('    vu.acc = arr!(acc);')
    for r in range(1, 16):
        w('    vu.vi[%d] = i%d;' % (r, r))
    w('    vu.vi[16] = mat16(b16, visv, vism, zacc, sacc);')
    w('    vu.vi[17] = mat17(b17, visv, vism);')
    w('    vu.vi[18] = c18;')
    w('    vu.vi[20] = rr;')
    w('    vu.vi[21] = ri;')
    w('    vu.vi[22] = q;')
    w('    if bail == u32::MAX {')
    w('        vu.tpc = tpc;')
    w('        vu.st.pairs += pairs;')
    w('        vu.st.cycles += cyc;')
    w('        return Exec::Done(cyc);')
    w('    }')
    w('    let mut p = Pipe::fresh(0, cl);')
    w('    p.cyc = cyc;')
    w('    p.pairs = pairs;')
    w('    match bail {')
    for a in bail_arms:
        w('        ' + a)
    w('        _ => unreachable!(),')
    w('    }')
    w('    Exec::Bail(Box::new(p))')
    w('    }')
    w('}')
    return '\n'.join(lines) + '\n'


def load_profile(path):
    ent = {}
    jr = {}
    if not path or not os.path.exists(path):
        return ent, jr
    for l in open(path):
        p = l.split()
        if len(p) >= 4 and p[0] == 'prog':
            n = int(p[1])
            if p[2] == 'entry':
                ent.setdefault(n, set()).add(int(p[3], 16))
            elif p[2] == 'jr' and len(p) >= 5:
                jr.setdefault(n, {}).setdefault(int(p[3], 16), set()).add(int(p[4], 16))
    return ent, jr


def gen_prog(n, bins, prof_ent, prof_jr, outdir):
    b = open(os.path.join(bins, 'prog%d.bin' % n), 'rb').read()
    code = [struct.unpack_from('<Q', b, i)[0] for i in range(0, len(b) - len(b) % 8, 8)]
    prog = Prog(n, code)
    links = set()
    ebits = set()
    for k in range(len(code)):
        P = prog.pair(k * 8)
        if P.unsup:
            continue
        if P.L and P.L.name in ('bal', 'jalr'):
            links.add((k * 8 + 16) & 0x3ff8)
        if P.ebit:
            ebits.add((k * 8 + 16) & 0x3ff8)
    jr_obs = prof_jr.get(n, {})

    def jr_targets(site):
        return set(t for t in (links | jr_obs.get(site, set())) if t < prog.size)

    prog.jr_targets = jr_targets
    entries = {0} | {e for e in ebits if e < prog.size} | {e for e in prof_ent.get(n, set()) if e < prog.size}
    graph, order, ent = explore(prog, sorted(entries))
    gen = Gen(prog, graph, order, ent)
    gen.build()
    arms = gen.emit()
    txt = file_text(prog, gen, arms, ent)
    path = os.path.join(outdir, 'vu1_prog%d.rs' % n)
    with open(path, 'w', newline='\n') as f:
        f.write(txt)
    nb = sum(1 for n_ in order if graph[n_].bail is not None)
    from collections import Counter
    c = Counter(n_[0] for n_ in order)
    why = Counter(graph[n_].bail for n_ in order if graph[n_].bail is not None)
    print('prog%d: %d pairs, %d nodes (max %d per pc), %d blocks, %d headers, %d bail nodes %s, %d entries, %d lines -> %s' % (
        n, len(code), len(order), max(c.values()), len(gen.blocks), len(gen.headers), nb, dict(why), len(ent),
        txt.count('\n'), path))
    return prog


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--progs', default='2,3,4,5,6,7,8,10,11')
    ap.add_argument('--bins', required=True)
    ap.add_argument('--profile', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'profile.txt'))
    ap.add_argument('--out', required=True)
    a = ap.parse_args()
    pe, pj = load_profile(a.profile)
    progs = [int(x) for x in a.progs.split(',') if x]
    for n in progs:
        gen_prog(n, a.bins, pe, pj, a.out)
    reg = ['// Generated by rac-rs/tools/vu1aot/vu1gen.py. Do not edit.', '// Native VU1 programs: (id, code, entry).', '']
    for n in progs:
        reg.append('#[path = "vu1_prog%d.rs"]' % n)
        reg.append('pub(crate) mod prog%d;' % n)
    reg.append('')
    reg.append('pub(crate) static PROGS: &[super::Native] = &[')
    for n in progs:
        reg.append('    super::Native { id: %d, code: &prog%d::CODE, run: prog%d::run },' % (n, n, n))
    reg.append('];')
    with open(os.path.join(a.out, 'vu1aot_gen.rs'), 'w', newline='\n') as f:
        f.write('\n'.join(reg) + '\n')


if __name__ == '__main__':
    main()
