# scratch_slow: emits the EE half of the skinning fast path (oracle src/skinff_ee.rs): the one-joint vertex loop
# (skinned_vertex_stream +0x694 .. +0x928, 4 vertices per pass, 5 compiled blocks) as Rust on local registers, one
# statement per retail instruction with the semantics of ee-core (cpu.rs mmi / the recompiler's loads and stores).
# usage: python skinff_ee.py STATE_DIR_NAME FN_ADDR_HEX OUT.rs     (reads the code words from the savestate's RAM)
#        python skinff_ee.py MEM.bin FN_ADDR_HEX OUT.rs            (MEM.bin: a file holding the code at FN_ADDR_HEX; tools/vugen_iso.py
#                                                                   passes the loop words it found on the disc this way)
import os, struct, sys
_arg = sys.argv[1]
mem = open(_arg if os.path.isfile(_arg) else os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'oracle', 'states', _arg, 'eeMemory.bin'), 'rb').read()
base = int(sys.argv[2], 16)
LOOP, END = 0x694, 0x92c
word = lambda off: struct.unpack('<I', mem[base + off:base + off + 4])[0]
# Register classes: a register that only ever sees 64-bit integer operations is a u64 local (its high half is never read or
# written, so it stays what it was); one that is also used by a vector operation (or loaded / stored as 128 bits) is a V
# (__m128i) local, its scalar reads / writes touch the low 64 bits only. Pass 1 collects the uses, pass 2 emits.
S, VV = set(), set()
CLS = {}
def cls(i): return CLS.get(i, 's')
def LO(i):      # scalar read (low 64 bits, u64)
    if i == 0: return '0u64'
    S.add(i)
    return f'st.e.r{i}' if cls(i) == 's' else f'lo64v(st.e.r{i})'
def VEC(i):     # vector read (V)
    if i == 0: return 'zero()'
    VV.add(i)
    return f'st.e.r{i}'
def lo64(i): return LO(i)
def set64(rd, e):  # c.s: the low 64 bits written, the high 64 kept
    if not rd: return ''
    S.add(rd)
    return f'st.e.r{rd} = ({e}) as u64;' if cls(rd) == 's' else f'st.e.r{rd} = setlo(st.e.r{rd}, ({e}) as u64);'
def q(rd, e):
    if not rd: return ''
    VV.add(rd)
    return f'st.e.r{rd} = {e};'

def ins(off, w, info):
    op = w >> 26; rs = (w >> 21) & 31; rt = (w >> 16) & 31; rd = (w >> 11) & 31; sa = (w >> 6) & 31; fn = w & 63
    imm = w & 0xffff; simm = imm - 0x10000 if imm & 0x8000 else imm
    ea = f'({lo64(rs)} as u32).wrapping_add({simm & 0xffffffff:#x})'
    if w == 0: return ''
    if op == 0:
        if fn == 0: return set64(rd, f'sx32(({lo64(rt)} as u32) << {sa})')
        if fn == 2: return set64(rd, f'sx32(({lo64(rt)} as u32) >> {sa})')
        if fn == 32: return set64(rd, f'sx32(({lo64(rs)} as u32).wrapping_add({lo64(rt)} as u32))')
        if fn == 62: return set64(rd, f'{lo64(rt)} >> {sa + 32}')
        raise SystemExit(f'{off:#x}: special {fn}')
    if op == 8: return set64(rt, f'sx32(({lo64(rs)} as u32).wrapping_add({simm & 0xffffffff:#x}))')
    if op == 12: return set64(rt, f'{lo64(rs)} & {imm:#x}')
    if op == 30: return q(rt, f'st.rd128({ea} & !15)')                       # lq
    if op == 55: return set64(rt, f'st.rd64({ea})')                          # ld
    if op == 31: return f'st.wr128({ea} & !15, {VEC(rt)});'                  # sq
    if op == 63: return f'st.wr64({ea}, {lo64(rt)});'                        # sd
    if op == 43: return f'st.wr32({ea}, {lo64(rt)} as u32);'                 # sw
    if op == 44: return f'{{ let a = {ea}; let sh = (a & 7) as usize; let mm = st.rd64(a & !7); st.wr64(a & !7, ({lo64(rt)} >> (56 - sh * 8)) | (mm & SDL_MASK[sh])); }}'
    if op == 45: return f'{{ let a = {ea}; let sh = (a & 7) as usize; let mm = st.rd64(a & !7); st.wr64(a & !7, ({lo64(rt)} << (sh * 8)) | (mm & SDR_MASK[sh])); }}'
    if op == 54: return f'{{ let v = st.rd128({ea} & !15); st.vset({rt}, v); }}' if rt else ''   # lqc2
    if op == 18:
        if rs == 1: return q(rt, f'st.vget({rd})')                          # QMFC2
        if rs == 5: return f'{{ let v = {VEC(rt)}; st.vset({rd}, v); }}' if rd else ''          # QMTC2
        if rs == 6:                                                          # CTC2 rt, vi(rd)
            assert rt == 0 and rd in (1, 2), hex(w)
            info.append(rd)
            return f'st.ctc2({rd});'
        raise SystemExit(f'{off:#x}: cop2 {rs}')
    if op == 28:
        a, b = VEC(rs), VEC(rt)
        if fn == 8:
            if sa == 22: return q(rd, f'pextlh({a}, {b})')
            if sa == 23: return q(rd, f'ppach({a}, {b})')
            if sa == 26: return q(rd, f'pextlb({a}, {b})')
            if sa == 27: return q(rd, f'ppacb({a}, {b})')
        if fn == 40:
            if sa == 22: return q(rd, f'pextuh({a}, {b})')
            if sa == 26: return q(rd, f'pextub({a}, {b})')
        if fn == 9:
            if sa == 14: return q(rd, f'pcpyld({a}, {b})')                  # PCPYLD
            if sa == 28: return '{ ' + f'let (x, y) = ({a}, {b}); ' + (q(rd, 'st.pmulth(x, y)') if rd else 'let _ = st.pmulth(x, y);') + ' }'
        if fn == 41:
            if sa == 14: return q(rd, f'pcpyud({a}, {b})')                  # PCPYUD
        if fn == 48 and sa == 4: return q(rd, 'pmfhl_sh(st.lo, st.hi)')
        if fn == 55: return q(rd, f'psrah::<{sa & 15}>({b})')
        if fn == 60: return q(rd, f'psllw::<{sa}>({b})')
        if fn == 63: return q(rd, f'psraw::<{sa}>({b})')
        raise SystemExit(f'{off:#x}: mmi fn {fn} sa {sa}')
    raise SystemExit(f'{off:#x}: op {op} ({w:08x})')

# blocks: (start, end exclusive); the branch is the second-last instruction, the delay slot the last
blocks = []
s = LOOP
for off in range(LOOP, END, 4):
    w = word(off)
    op = w >> 26; rt = (w >> 16) & 31
    if op in (1, 4, 5):
        blocks.append((s, off + 8)); s = off + 8
assert s == END, hex(s)
def gen():
    out = []
    total = 0
    for bi, (s, e) in enumerate(blocks):
        n = (e - s) // 4
        total += n
        br = word(e - 8)
        op = br >> 26; brs = (br >> 21) & 31; brt = (br >> 16) & 31
        simm = (br & 0xffff) - (0x10000 if br & 0x8000 else 0)
        tgt = e - 4 + simm * 4
        if op == 1 and brt == 0: cond = f'(({LO(brs)}) as i64) < 0'
        elif op == 5: cond = f'({LO(brs)}) != ({LO(brt)})'
        else: raise SystemExit(f'branch {br:08x}')
        out.append(f'// block {bi}: +{s:#x} .. +{e - 4:#x} ({n} instructions), branch to +{tgt:#x}')
        out.append('#[inline(always)]')
        out.append(f'pub fn block_{bi}(st: &mut St) -> (bool, u64, u32) {{')
        body = []
        for off in range(s, e, 4):
            w = word(off)
            if off == e - 8:
                body.append(f'let taken = {cond};')
                continue
            stt = ins(off, w, [])
            if stt:
                body.append(f'/* +{off:03x} */ ' + stt)
        for b in body:
            out.append('    ' + b)
        out.append(f'    (taken, {n}, {tgt:#x})')
        out.append('}')
        out.append('')
    return out, total

gen()
for i in S | VV:
    CLS[i] = 'v' if i in VV else 's'
body, total = gen()
regs = sorted(CLS)
hdr = ['//! @generated by tools/skinff_ee.py from skinned_vertex_stream (+0x694 .. +0x928). Do not edit.',
       '//! One pass of the one-joint vertex loop, block by block; see skinff.rs. Registers used only by 64-bit integer',
       '//! operations are u64 locals, the others (vector operations, 128-bit loads and stores) are __m128i locals.',
       '#![allow(unused_variables, unused_parens, clippy::all)]', 'use super::*;',
       f'pub const PASS_INSNS: u64 = {total};',
       '/// Upper bound of the stores (log entries) of one pass.',
       f'pub const PASS_STORES: usize = {sum(l.count("st.wr") for l in body)};',
       f'pub const LOOP_END: u32 = {END:#x};',
       f'pub const BLOCKS: [u32; {len(blocks)}] = [{", ".join(hex(s) for s, _ in blocks)}];', '',
       '/// The registers the loop touches.', 'pub struct Regs {']
hdr += [f'    pub r{i}: {"u64" if CLS[i] == "s" else "V"},' for i in regs]
hdr += ['}', '', 'impl Regs {', '    #[inline(always)]', '    pub fn load(r: &[u128; 32]) -> Regs {', '        Regs {']
hdr += [f'            r{i}: {"r[%d] as u64" % i if CLS[i] == "s" else "m(r[%d])" % i},' for i in regs]
hdr += ['        }', '    }', '    /// The high halves of the u64 registers stay as they were.', '    #[inline(always)]', '    pub fn store(&self, r: &mut [u128; 32]) {']
hdr += [f'        r[{i}] = {"(r[%d] & HI64) | self.r%d as u128" % (i, i) if CLS[i] == "s" else "u(self.r%d)" % i};' for i in regs]
hdr += ['    }', '}', '']
out = hdr + body
open(sys.argv[3], 'w', encoding='utf-8', newline='\n').write('\n'.join(out) + '\n')
print(len(blocks), 'blocks', total, 'instructions;', len([i for i in regs if CLS[i]=='s']), 'u64 regs', len([i for i in regs if CLS[i]=='v']), 'vector regs', file=sys.stderr)
