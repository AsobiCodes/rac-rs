"""Static "clean operand" analysis for the AOT generator.

A VF lane is *clean* when its exponent field is not 255 (no Inf / NaN bits). The interpreter reads every float operand
through `fin` (exponent 255 -> sign | FLT_MAX); for a clean operand `fin` is the identity, so the native code may skip
the clamp. Results of ADD / SUB / MUL / MADD / MSUB / ITOF are clean by construction (the SSE ops run with MXCSR
round-toward-zero, where an overflow gives the largest finite number, never Inf, and finite operands never give NaN),
MAX / MIN / ABS / MOVE pass the cleanliness of their sources on, and LQ / FTOI results (arbitrary bits) are not clean.

State per node entry: acc = 4-bit lane mask of known-clean ACC lanes, vf[r] = 4-bit lane mask (bit 3 = x) of the known
clean lanes of VF r. Every entry point (start or resume: the EE may have changed any VF register or ACC) knows nothing,
except vf00 = (0, 0, 0, 1.0). Q and I are never treated as clean. The lattice is a meet (AND) over all predecessors;
a forward fixpoint over the node graph gives the state before each node.
"""
from vudec import *

UNKNOWN = (0, tuple([15] + [0] * 31))
TOP = (15, tuple([15] * 32))


def meet(a, b):
    return (a[0] & b[0], tuple(x & y for x, y in zip(a[1], b[1])))


def operand_masks(u, st):
    """For an upper op `u` of kind fmac, the lane-clean tests of its operands under state st:
    (S clean over dest lanes, T clean over dest lanes (bc: its lane), acc clean over dest lanes)."""
    acc, vf = st
    M = u.dest
    sc = (vf[u.fs] & M) == M
    ts = u.tsel
    if ts[0] == 'full':
        tc = (vf[u.ft] & M) == M
    elif ts[0] == 'bc':
        tc = (vf[u.ft] & lane(ts[1])) != 0
    else:
        tc = False
    ac = (acc & M) == M
    return sc, tc, ac


def result_clean(u, st):
    """Lane mask (over u.dest) of the clean lanes of the upper op's result."""
    M = u.dest
    if u.kind == 'fmac':
        if u.k in (MAX, MIN):
            sc, tc, _ = operand_masks(u, st)
            return M if (sc and tc) else 0
        return M
    if u.kind == 'itof':
        return M
    if u.kind == 'ftoi':
        return 0
    if u.kind == 'abs':
        return M if (st[1][u.fs] & M) == M else 0
    return 0


def transfer(p, st):
    acc, vf = st
    nvf = list(vf)
    nacc = acc
    L = p.lr
    if L is not None and not p.ibit:
        if L.name == 'lq' and L.it != 0:
            nvf[L.it] &= ~L.dest & 15
        elif L.name == 'move' and L.it != 0:
            nvf[L.it] = (nvf[L.it] & ~L.dest & 15) | (vf[L.is_] & L.dest)
        elif L.name == 'mr32' and L.it != 0:
            rot = 0
            for l in range(4):
                if vf[L.is_] & lane((l + 1) & 3):
                    rot |= lane(l)
            nvf[L.it] = (nvf[L.it] & ~L.dest & 15) | (rot & L.dest)
    u = p.up
    if u.kind != 'nop' and u.dst != NONE and u.dst != 0 and u.mask:
        res = result_clean(u, st)
        if u.dst == ACC:
            nacc = (acc & ~u.mask & 15) | (res & u.mask)
        else:
            nvf[u.dst] = (nvf[u.dst] & ~u.mask & 15) | (res & u.mask)
    nvf[0] = 15
    return (nacc, tuple(nvf))


def analyse(graph, entset):
    """State before every node of `graph` ({node: (info, succs)}); `entset`: the entry nodes."""
    pred = {n: [] for n in graph}
    for n, (info, succs) in graph.items():
        for k, s in succs:
            pred[s].append(n)
    ins = {n: (UNKNOWN if n in entset else TOP) for n in graph}
    outs = {}
    work = list(graph)
    while work:
        n = work.pop()
        o = transfer(graph[n][0]['p'], ins[n])
        if outs.get(n) == o:
            continue
        outs[n] = o
        for k, s in graph[n][1]:
            if s in entset:
                continue
            ni = meet(ins[s], o)
            if ni != ins[s]:
                ins[s] = ni
                work.append(s)
            elif s not in outs:
                work.append(s)
    return ins
