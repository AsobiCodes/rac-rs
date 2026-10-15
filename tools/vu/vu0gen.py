"""Generates oracle/src/vu0_prog0.rs: prog0 (decomp/vu1/prog0.bin) translated pair by pair into straight-line Rust,
specialised over the interpreter's pair-level hazard state (see vugraph.py). Usage: VU_BINS=<dir with prog0.bin and prog0.s> python vu0gen.py OUT.rs (tools/vugen_iso.py sets it up)"""
import os, sys, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))   # the embeddable Python that setup.ps1 downloads does not add the script folder to sys.path
from vudec import *
import vugraph as G
import vuclean as C

ENTRIES = [0x000, 0x888, 0x908, 0x968, 0x0f0, 0x1c8, 0x2f0, 0x418, 0x560, 0x6a8, 0x780, 0x858]
PARKS = [0x0f0, 0x1c8, 0x2f0, 0x418, 0x560, 0x6a8, 0x780, 0x858]
CODE = G.CODE
QT = "'"  # a quote for the Rust loop label

DIS = {}
for ln in open(os.path.join(os.environ['VU_BINS'], 'prog0.s'), encoding='utf-8'):
    m = re.match(r'\s+([0-9a-f]{4}): [0-9a-f]{8} [0-9a-f]{8}\s+(.*)', ln)
    if m:
        DIS[int(m.group(1), 16)] = ' '.join(m.group(2).split())


def vf(r):
    return f'vf.add({r * 4})'


def rev4(d):
    return ((d & 8) >> 3) | ((d & 4) >> 1) | ((d & 2) << 1) | ((d & 1) << 3)


def t_expr(u):
    ts = u.tsel
    if ts[0] == 'full':
        return f'ldv({vf(u.ft)})'
    if ts[0] == 'bc':
        return f'bc::<{ts[1]}>(ldv({vf(u.ft)}))'
    if ts[0] == 'q':
        return 'splat(vi[22])'
    return 'splat(vi[21])'


def upper_expr(u, st):
    """The upper op's result expression; `st` = clean-operand state before the pair (vuclean.py)."""
    S = f'ldv({vf(u.fs)})'
    if u.kind == 'fmac':
        T = t_expr(u)
        sc, tc, ac = C.operand_masks(u, st)
        ns, nt, na = str(not sc).lower(), str(not tc).lower(), str(not ac).lower()
        return {ADD: f'add4::<{ns}, {nt}>({S}, {T})', SUB: f'sub4::<{ns}, {nt}>({S}, {T})',
                MUL: f'mul4::<{ns}, {nt}>({S}, {T})',
                MADD: f'madd4::<{na}, {ns}, {nt}>(acc, {S}, {T})', MSUB: f'msub4::<{na}, {ns}, {nt}>(acc, {S}, {T})',
                MAX: f'max4({S}, {T})', MIN: f'min4({S}, {T})'}[u.k]
    if u.kind == 'itof':
        return f'itof4::<{u.sci}>({S})'
    if u.kind == 'ftoi':
        return f'ftoi4::<{u.sci}>({S})'
    if u.kind == 'abs':
        return f'abs4({S})'
    raise ValueError(u.kind)


def cond_expr(L):
    a = f'(vi[{L.is_}] as u16 as i16)'
    b = f'(vi[{L.it}] as u16 as i16)'
    return {'ibeq': f'{a} == {b}', 'ibne': f'{a} != {b}', 'ibltz': f'{a} < 0', 'ibgtz': f'{a} > 0',
            'iblez': f'{a} <= 0', 'ibgez': f'{a} >= 0'}[L.name]


def write_vf(dst, mask, expr):
    """Lines storing `expr` (a V) to vf[dst] lanes `mask`."""
    if dst == 0 or mask == 0:
        return []
    if mask == 15:
        return [f'stv({vf(dst)}, {expr});']
    return [f'stv({vf(dst)}, blendm::<{mask}>(ldv({vf(dst)}), {expr}));']


def lower_lines(p):
    L = p.lr
    out = []
    if p.ibit or L is None:
        return out
    n = L.name
    uimm = L.imm11 & 0xffffffff
    if n == 'lq':
        out.append(f'let a = ((vi[{L.is_}].wrapping_add({uimm}u32) & 0xff) << 4) as usize;')
        out += write_vf(L.it, L.dest, 'ldv(dm.add(a) as *const u32)')
    elif n == 'sq':
        out.append(f'let a = ((vi[{L.it}].wrapping_add({uimm}u32) & 0xff) << 4) as usize;')
        if L.dest == 15:
            out.append(f'stv(dm.add(a) as *mut u32, ldv({vf(L.is_)}));')
        elif L.dest:
            out.append(f'stv(dm.add(a) as *mut u32, blendm::<{L.dest}>(ldv(dm.add(a) as *const u32), ldv({vf(L.is_)})));')
    elif n == 'mtir':
        if L.it != 0:
            out.append(f'vi[{L.it}] = *vf.add({L.is_ * 4 + L.fsf}) & 0xffff;')
    elif n == 'move':
        out += write_vf(L.it, L.dest, f'ldv({vf(L.is_)})')
    elif n in ('iaddiu', 'isubiu'):
        if L.it != 0:
            op = 'wrapping_add' if n == 'iaddiu' else 'wrapping_sub'
            out.append(f'vi[{L.it}] = vi[{L.is_}].{op}({L.im}u32) & 0xffff;')
    elif n == 'iand':
        if L.id_ != 0:
            out.append(f'vi[{L.id_}] = (vi[{L.is_}] & vi[{L.it}]) & 0xffff;')
    elif n == 'div':
        out.append(f'qp = crate::fpu::div_nc(*vf.add({L.is_ * 4 + L.fsf}), *vf.add({L.it * 4 + L.ftf}));')
    elif n == 'rsqrt':
        out.append(f'qp = crate::fpu::rsqrt_nc(*vf.add({L.is_ * 4 + L.fsf}), *vf.add({L.it * 4 + L.ftf}));')
    elif n in ('b', 'nop'):
        pass
    elif n.startswith('ib'):
        out.append(f'let c = {cond_expr(L)};')
    else:
        raise NotImplementedError(n)
    return out


def check_regs(p):
    """The native code keeps VI 0..15 and Q / I in the local array; flag registers (16..) must not be operands."""
    L = p.lr
    if L is None or p.ibit:
        return
    n = L.name
    if n == 'lq':
        assert L.is_ < 16
    elif n == 'sq':
        assert L.it < 16
    elif n in ('iaddiu', 'isubiu'):
        assert L.is_ < 16 and L.it < 16
    elif n.startswith('ib'):
        assert L.is_ < 16 and L.it < 16
    elif n == 'iand':
        assert max(L.is_, L.it, L.id_) < 16
    elif n == 'mtir':
        assert L.it < 16


def main(out):
    graph, ent = G.explore(ENTRIES)
    nodes = sorted(graph, key=lambda n: (n[0], repr(n[1]), repr(n[2]), n[3]))
    nid = {n: i for i, n in enumerate(nodes)}
    indeg = {n: 0 for n in nodes}
    for n, (info, succs) in graph.items():
        for k, s in succs:
            indeg[s] += 1
    entset = set(ent.values())
    cst = C.analyse(graph, entset)
    for e in entset:
        indeg[e] += 1
    # Arm heads: entries, nodes with other than one predecessor, back-edge targets, two-way transition targets.
    heads = set(entset)
    for n, (info, succs) in graph.items():
        for k, s in succs:
            if len(succs) == 2 or s[0] <= n[0]:
                heads.add(s)
    for n in nodes:
        if indeg[n] != 1:
            heads.add(n)
    emitted = set()

    def emit(n, ind, lines):
        info, succs = graph[n]
        p = info['p']
        pc, T, pend, end_next = n
        check_regs(p)
        assert n not in emitted, 'node emitted twice %04x' % pc
        emitted.add(n)
        I = ' ' * ind
        L = lambda s: lines.append(I + s)
        extra = ''
        if info['q_land']:
            extra += ', Q lands'
        if pend:
            extra += ', pend ' + str(pend)
        if end_next:
            extra += ', E slot'
        L(f'// {pc:04x}  {DIS.get(pc, "")}   [stall {info["stall"]}{extra}]')
        L('if COV { *cov.add(%d) += 1; }' % nid[n])
        if info['stall']:
            L(f'cyc += {info["stall"]};')
        L('pairs += 1;')
        if info['q_land']:
            L('vi[22] = qp;')
        u = p.up
        has_u = u.kind != 'nop'
        if has_u:
            L(f'let u = {upper_expr(u, cst[n])};')
        for s in lower_lines(p):
            L(s)
        q_pending_now = T[1] is not None and not info['q_land']
        if info['park']:
            assert not info['park_cond'], 'conditional park not supported'
            if q_pending_now:
                L('vi[22] = qp;')
            L(f'break {QT}run ({p.tgt}usize, true);')
            return
        if has_u and u.dst != NONE and u.dst != 0:
            if u.dst == ACC:
                if u.mask:
                    L(f'acc = blendm::<{u.mask}>(acc, u);')
            else:
                for s in write_vf(u.dst, u.mask, 'u'):
                    L(s)
        if has_u and u.mac:
            lm = rev4(u.dest)
            L(f'{{ let (z, s) = zs_masks(u, {lm}); zl = z; sl = s; zo |= z; so |= s; fl_any = true; }}')
        if p.ibit:
            L(f'vi[21] = {p.lo}u32;')
        L('cyc += 1;')
        nT_q = info['nT'][1]
        if end_next:
            if nT_q is not None:
                L('vi[22] = qp;')
            L(f'break {QT}run ({pc}usize, false);')
            return
        if p.branch and p.branch[0] == 'cond':
            L('tk = c;')
        cap = 'if pairs > PAIR_CAP { return Err(format!("VU0 microprogram ran {} pairs without ending or waiting (native, pc {:04x})", pairs, %d)); }'

        def go(s):
            tpc = s[0]
            back = tpc <= pc
            if back:
                L(cap % tpc)
            if s not in heads:
                assert not back
                emit(s, ind, lines)
            else:
                L(f'n = {nid[s]}; continue;')

        if len(succs) == 1:
            go(succs[0][1])
        elif len(succs) == 2:
            (k1, s1), (k2, s2) = succs
            assert k1 == 'tk' and k2 == 'nt'
            assert s1 in heads and s2 in heads
            L('if tk {')
            if s1[0] <= pc:
                lines.append(I + '    ' + cap % s1[0])
            lines.append(I + '    n = %d;' % nid[s1])
            lines.append(I + '} else {')
            lines.append(I + '    n = %d;' % nid[s2])
            lines.append(I + '}')
            L('continue;')
        else:
            raise AssertionError('no successor at %04x' % pc)

    arms_text = []
    for n in nodes:
        if n not in heads:
            continue
        lines = []
        emit(n, 20, lines)
        arms_text.append(f'                {nid[n]} => {{\n' + '\n'.join(lines) + '\n                }')
    missing = [n for n in nodes if n not in emitted]
    assert not missing, ('nodes never emitted', [(hex(m[0]), m) for m in missing[:5]])

    ent_arms = '\n'.join(f'        0x{e:03x} => Some({nid[ent[e]]}),' for e in ENTRIES)
    park_arms = []
    for e in PARKS:
        info = graph[ent[e]][0]
        p = info['p']
        assert p.lr.name == 'ibeq' and p.lr.is_ == 0 and p.lr.it in (1, 2), hex(e)
        park_arms.append(f'        0x{e:03x} => Some({p.lr.it}),')
    code_lit = ', '.join(str(b) for b in CODE)
    body = '\n'.join(arms_text)
    parks = '\n'.join(park_arms)

    src = f'''//! @generated by scratch gen/vu0gen.py from decomp/vu1/prog0.bin ({len(CODE)} bytes, {len(nodes)} nodes). Do not edit.
//! prog0 translated pair by pair; each node is one pair specialised over the interpreter's hazard state (stalls and
//! Q timing are constants), see vu0aot.rs.
#![allow(unused_mut, unused_assignments, unused_variables, unused_unsafe, clippy::all)]
use crate::cpu::Cpu;
use crate::vu0aot::*;

pub const NODES: usize = {len(nodes)};

pub static PROG0: [u8; {len(CODE)}] = [{code_lit}];

/// Native node for a start / resume at micro address `pc`.
pub fn entry_node(pc: usize) -> Option<u32> {{
    match pc {{
{ent_arms}
        _ => None,
    }}
}}

/// The wait loops: the VI register their `IBEQ viN, vi00` tests (the loop is left when it is 0).
pub fn park_reg(pc: usize) -> Option<usize> {{
    match pc {{
{parks}
        _ => None,
    }}
}}

impl Cpu {{
    /// Runs prog0 from `node` (see `entry_node`) to its E bit or to the next wait loop. Needs a live ChopGuard.
    /// Returns (cycles, pairs, last pc (parked: the loop head), parked).
    pub(crate) fn vu0_prog0_run<const COV: bool>(&mut self, mut n: u32) -> Result<(u64, u64, usize, bool), String> {{
        let vf = self.vf.as_mut_ptr() as *mut u32;
        let dm = self.mem.vu0d.as_mut_ptr();
        let cov = self.vu0_aot.cov.as_mut_ptr();
        let mut vi: [u32; 32] = self.vi;
        let mut acc: V = unsafe {{ ldv(self.vacc.as_ptr()) }};
        let mut qp: u32 = 0;
        let mut tk = false;
        let (mut fl_any, mut zl, mut sl, mut zo, mut so) = (false, 0u32, 0u32, 0u32, 0u32);
        let (mut cyc, mut pairs) = (0u64, 0u64);
        let (last_pc, parked) = unsafe {{
            {QT}run: loop {{
                match n {{
{body}
                    _ => unreachable!(),
                }}
            }}
        }};
        if fl_any {{
            // The pending MAC / status results land in order: the last MAC is kept, the sticky status bits are the
            // OR of every result's zero / sign flags (`vu0_flag_lands`).
            vi[17] = REV4[zl as usize] | REV4[sl as usize] << 4;
            let st = (zl != 0) as u32 | ((sl != 0) as u32) << 1;
            vi[16] = (vi[16] & !0xf) | st | (((zo != 0) as u32) << 6) | (((so != 0) as u32) << 7);
        }}
        self.vi = vi;
        unsafe {{ stv(self.vacc.as_mut_ptr(), acc) }};
        Ok((cyc, pairs, last_pc, parked))
    }}
}}
'''
    open(out, 'w', encoding='utf-8', newline='\n').write(src)
    print('nodes', len(nodes), 'heads', len(heads), 'lines', src.count('\n'))


if __name__ == '__main__':
    main(sys.argv[1])
