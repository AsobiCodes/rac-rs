"""Abstract exploration of the interpreter's per-pair timing/branch state for a VU0 program. Node = (pc, T, pend, end_next):
T = (rd: tuple(((reg, lane_idx), remaining cycles > 0)), q: None | remaining cycles >= 0 of the pending Q result, fx);
fx = (F: tuple((remaining cycles, slot)) of the MAC results in flight, oldest first (only for a program that reads the
MAC flag, FLAGS), LF: the slot of the newest landed MAC result (None: the value from before the start), ret: None |
(VI register, value) the BAL return address a later JR jumps to);
pend = None | ('always', tgt) | ('cond', tgt): the branch of the previous pair, applied after this (delay slot) pair.
VU0_PROG (env) = prog0 (default), prog1 or prog9hi (prog9's piece at micro address 0xc80, BASE); VU_BINS/<prog>.bin."""
import sys
from vudec import *

import os
PROG = os.environ.get('VU0_PROG', 'prog0')
BASE = 0xc80 if PROG == 'prog9hi' else 0
CODE = bytes(BASE) + open(os.path.join(os.environ['VU_BINS'], PROG + '.bin'), 'rb').read()
SIZE = len(CODE)


def word(a):
    return int.from_bytes(CODE[a:a + 4], 'little')


def is_spin(t, pc):
    end = pc + 8
    if end - t > 64:
        return False
    a = t
    while a <= end:
        lo, hi = word(a), word(a + 4)
        if hi & 0xf8000000 != 0 or hi & 0x07ffffff != 0x000002ff:
            return False
        op = lo >> 25
        if not (lo == 0x8000033c or op == 0x20 or 0x28 <= op <= 0x2f):
            return False
        a += 8
    return True


class PairInfo:
    pass


_PI = {}


def pair(pc):
    if pc in _PI:
        return _PI[pc]
    p = PairInfo()
    p.pc = pc
    p.lo, p.hi = word(pc), word(pc + 4)
    p.ibit = bool(p.hi & 0x80000000)
    p.ebit = bool(p.hi & 0x40000000)
    p.up = decode_upper(p.hi)
    p.lr = None
    if not p.ibit and p.lo != 0x8000033c:
        p.lr = decode_lower(p.lo, pc)
    p.lreads = []
    if not p.ibit:
        # lower_info is computed even for the NOP lower (MOVE vf0,vf0: reads vf0 which is never pending)
        p.lreads = p.lr.reads if p.lr else []
    p.waitq = bool(p.lr and p.lr.waitq)
    p.div = bool(p.lr and p.lr.div)
    p.branch = p.lr.branch if p.lr else None
    p.tgt = branch_target(p.lo, pc) if p.branch and p.branch[0] != 'jr' else None
    _PI[pc] = p
    return p


# A program that reads the MAC flag mid-run (FMEQ / FMAND / FMOR) needs the landing time of every MAC result.
def _reads_flags():
    for a in range(BASE, SIZE - 7, 8):
        lo, hi = word(a), word(a + 4)
        if not hi & 0x80000000 and lo >> 25 in (0x18, 0x1a, 0x1b):
            return True
    return False


FLAGS = _reads_flags()


def step(node):
    """Returns (info dict). Static facts of executing `node`."""
    pc, (rd, q, fx), pend, end_next = node
    F, LF, ret = fx
    p = pair(pc)
    rdd = dict(rd)
    stall = 0
    for (r, m) in list(p.up.reads) + list(p.lreads):
        if r == NONE:
            continue
        for l in range(4):
            if m & lane(l):
                stall = max(stall, rdd.get((r, l), 0))
    if (p.waitq or p.div) and q is not None:
        stall = max(stall, q)
    q_land = q is not None and stall >= q
    # MAC results in flight land (oldest first) once their 4 cycles have passed, before the lower op reads the flags.
    landed = [sl for r, sl in F if r - stall <= 0]
    nF = [(r - stall - 1, sl) for r, sl in F if r - stall > 0]
    mac_vis = landed[-1] if landed else LF
    tgt = p.tgt
    if p.branch and p.branch[0] == 'jr':
        assert ret is not None and ret[0] == p.lr.is_, 'JR vi%d at %04x without a known BAL return address' % (p.lr.is_, pc)
        tgt = (ret[1] * 8) & 0xff8
    nret = ret
    if p.lr and not p.ibit:
        if p.lr.link is not None:
            nret = (p.lr.link, ((pc + 16) // 8) & 0xffff)
        elif p.branch and p.branch[0] == 'jr':
            nret = None
        elif ret is not None and p.lr.wvi == ret[0]:
            nret = None
    mac_slot = None
    if FLAGS and p.up.kind != 'nop' and p.up.mac:
        used = {sl for _, sl in nF} | {mac_vis}
        mac_slot = min(set(range(8)) - used)
        nF.append((3, mac_slot))
    nfx = (tuple(nF), mac_vis, nret) if FLAGS else ((), None, nret)
    # park?
    park = False
    park_cond = False
    if p.branch and tgt <= pc and pend is None and not end_next and is_spin(tgt, pc):
        park = True
        park_cond = p.branch[0] == 'cond'
    # next T (assuming the pair completes)
    nrd = {k: v - stall - 1 for k, v in rdd.items() if v - stall - 1 > 0}
    if p.lr and p.lr.wvf:
        r, m = p.lr.wvf
        if r != 0:
            for l in range(4):
                if m & lane(l):
                    nrd[(r, l)] = 3
    u = p.up
    if u.dst != NONE and u.dst != 0:
        for l in range(4):
            if u.mask & lane(l):
                nrd[(u.dst, l)] = 3
    if q is None:
        nq = None
    elif q_land:
        nq = None
    else:
        nq = max(0, q - stall - 1)
    qissue = None
    if p.lr and p.lr.name in ('div', 'rsqrt'):
        qissue = 7 if p.lr.name == 'div' else 13
        nq = qissue - 1
    nT = (tuple(sorted(nrd.items())), nq, nfx)
    return dict(p=p, stall=stall, q_land=q_land, park=park, park_cond=park_cond, nT=nT, qissue=qissue, tgt=tgt,
                mac_vis=mac_vis, mac_slot=mac_slot)


def explore(entries):
    """entries: list of pcs. Returns (nodes: dict node -> (info, succs list of (kind, node)), entry nodes)."""
    graph = {}
    work = []
    ent = {}
    for e in entries:
        n = (e, ((), None, ((), None, None)), None, False)
        ent[e] = n
        work.append(n)
    while work:
        n = work.pop()
        if n in graph:
            continue
        info = step(n)
        p = info['p']
        pc, T, pend, end_next = n
        succs = []
        if end_next:
            pass  # exit E after this pair
        else:
            if p.branch and pend is not None:
                raise NotImplementedError('branch in a delay slot at %04x' % pc)
            nend = p.ebit
            if pend is not None:
                kind, tgt = pend
                npend = None
                if kind == 'always':
                    succs.append(('goto', (tgt, info['nT'], npend, nend)))
                else:
                    succs.append(('tk', (tgt, info['nT'], npend, nend)))
                    succs.append(('nt', ((pc + 8) & 0xff8, info['nT'], npend, nend)))
            else:
                npend = None
                if p.branch:
                    npend = ('cond' if p.branch[0] == 'cond' else 'always', info['tgt'])
                if info['park'] and not info['park_cond']:
                    succs = []  # exits (park)
                else:
                    succs.append(('goto', ((pc + 8) & 0xff8, info['nT'], npend, nend)))
        for k, s in succs:
            assert BASE <= s[0] < SIZE, 'flow leaves %s at %04x -> %04x' % (PROG, pc, s[0])
            if s not in graph:
                work.append(s)
        graph[n] = (info, succs)
    return graph, ent


if __name__ == '__main__':
    entries = [0x000, 0x888, 0x908, 0x968, 0xf0, 0x1c8, 0x2f0, 0x418, 0x560, 0x6a8, 0x780, 0x858]
    g, ent = explore(entries)
    print('nodes', len(g))
    from collections import Counter
    c = Counter(n[0] for n in g)
    print('pcs', len(c), 'max variants per pc', max(c.values()), c.most_common(12))
    print('stall histogram', Counter(i['stall'] for i, _ in g.values()))
    print('with pend', sum(1 for n in g if n[2] is not None), 'end_next', sum(1 for n in g if n[3]))
