#!/usr/bin/env python3
"""share_iso.py: compile repeated retail functions once (run after tools/recgen_iso.py has written every level).

Most engine functions exist in every level's overlay with the same code at a different address, and with some different
constants (addresses of data, call targets). recgen_iso.py emits each copy as its own Rust function. This pass groups the
functions of all crates under <oracle>/rec whose generated Rust is the same up to hex literals and same-crate function names
(the "skeleton"), and gives each group one shared body (a template) in new crates rec/sh_NN. Each copy becomes a one-line
wrapper with the same name, which calls the template with its entry address and its own values:

  pub fn f_2017d0(c: &mut Cpu) { rec_sh_03::t_12(c, 0x2017d0, &V_2017d0, &F_2017d0); }

In the template every hex literal is, per position: the literal itself (same value in every copy), `zb_.wrapping_add(OFF)` (the
same offset from the entry in every copy: PCs, labels, branch targets), or `zv_[k]` (a value that differs: read from the copy's
table). Same-crate function names become `zf_[k]` (a table of fn pointers to the copy's own callees). Dispatch-form matches
`match c.pc { 0x.. => ...` become `match c.pc.wrapping_sub(zb_) { OFF => ...` (wrapping_sub is a bijection: same arm taken).
So every copy computes with exactly its own original values: the change is in where a value comes from, never what it is.
A literal without a type suffix gets its type from the context, as the literal did (`as _`); next to a bit operator on a
register value (`c.g(N) | 0x..`, where the literal is a u64) the cast names the type, `as zu64_` (alias of u64), because
`u64 | (x as _)` has two candidate impls (u64 and &u64) where the literal had one.
The pass then re-reads the files it wrote and checks, for every copy, that substituting its wrapper's values into its
template gives back the original function token for token (value and type suffix of every literal), else it fails.

The FNS tables (entry, end, code hash, labels, f) are untouched: hands, dispatch and the hash check see the same functions.
The template crates have an empty FNS so the facade and setup's crate count stay as they are.
Work is spread over --jobs processes (default: the CPU count, at most 12); the output does not depend on it.
Usage: python share_iso.py --rec <oracle>/rec [--min-insns 24] [--chunk 12000] [--jobs N]

File times: cargo rebuilds a crate when one of its files is newer than the last build. recgen_iso.py only writes files whose
content differs, but before this pass every level crate on disk is the shared form, so recgen_iso.py rewrites them all (and
the sh_ crates are deleted and made again): every crate would be rebuilt after any regeneration. So setup.ps1 runs
  python share_iso.py --rec <oracle>/rec --snapshot FILE      before the generators (content hash + time of every file)
  python share_iso.py --rec <oracle>/rec --keep-times FILE    after this pass and the facade
and every file that ends up with the same content as before gets its old time back: only crates that really changed rebuild."""
import argparse, collections, hashlib, os, re, sys
from array import array
import multiprocessing as mp

TOK = re.compile(r"'[A-Za-z_][A-Za-z0-9_]*|0x[0-9a-f]+(?:u8|u16|u32|u64|u128|i8|i16|i32|i64|usize)?|[A-Za-z_][A-Za-z0-9_]*|\d+|\s+|.", re.S)
HEX = re.compile(r'0x([0-9a-f]+)(\w*)$')
FN_HDR = re.compile(r'^pub fn ([A-Za-z0-9_]+)\(c: &mut Cpu\) \{$')
ARM = re.compile(r'^0x([0-9a-f]+) => \{$')
FNS_ROW = re.compile(r'ee_core::rec::Fn \{ entry: (0x[0-9a-f]+), end: (0x[0-9a-f]+),.* f: ([A-Za-z0-9_]+) \},$')
WRAP = re.compile(r'^pub fn ([A-Za-z0-9_]+)\(c: &mut Cpu\) \{ (rec_sh_\d+)::(t_\d+)\(c, (0x[0-9a-f]+), &V_\1, &F_\1\); \}$')
TPL_HDR = re.compile(r'^pub fn (t_\d+)\(c: &mut Cpu, zb_: u32, zv_: &\[u64; (\d+)\], zf_: &\[fn\(&mut Cpu\); (\d+)\]\) \{$')
SH_DIR = re.compile(r'sh_\d+$')
ALIAS = '#[allow(non_camel_case_types, dead_code)] type zu64_ = u64;'
# the value tables are named after their function (V_f_2017d0): without this every one is a compiler warning, 50,000 in all, and
# cargo prints them all again on every later build (a 2-minute stall before an update build can use more than one CPU)
GLOBALS = 'non_upper_case_globals'


def allow_globals(header):
    return [(l.replace('#![allow(', '#![allow(%s, ' % GLOBALS, 1) if l.startswith('#![allow(') and GLOBALS not in l else l) for l in header]
BITOPS = ('|', '&', '^')


def read_crate(rec, name):
    p = os.path.join(rec, name, 'src', 'lib.rs')
    lines = open(p, encoding='utf-8').read().split('\n')
    i = 0
    while i < len(lines) and not lines[i].startswith('pub fn '):
        i += 1
    header = lines[:i]
    fns, cur, depth, tail = [], None, 0, []
    for ln in lines[i:]:
        if cur is None:
            m = FN_HDR.match(ln)
            if m:
                cur = (m.group(1), [])
                depth = 1
            else:
                tail.append(ln)
            continue
        depth += ln.count('{') - ln.count('}')
        if depth == 0:
            fns.append(cur)
            cur = None
        else:
            cur[1].append(ln)
    assert cur is None, name
    meta = {}
    for ln in tail:
        m = FNS_ROW.search(ln)
        if m:
            meta[m.group(3)] = (int(m.group(1), 16), int(m.group(2), 16))
    assert len(meta) == len(fns), (name, len(meta), len(fns))
    return header, fns, tail, meta


def toks(body, names):
    """Function body -> token list: ('A', off) dispatch arm, ('H', value, suffix) hex literal, ('F', name) same-crate fn,
    ('L', name) lifetime, ('W', text) whitespace, ('T', text) anything else."""
    out = []
    for ln in body:
        m = ARM.match(ln)
        if m:
            out.append(('A', int(m.group(1), 16)))
            out.append(('T', '\n'))
            continue
        for t in TOK.findall(ln):
            if t.isspace():
                out.append(('W', t))
            elif t.startswith("'"):
                out.append(('L', t))
            elif t.startswith('0x'):
                h = HEX.match(t)
                out.append(('H', int(h.group(1), 16), h.group(2)))
            elif t in names:
                out.append(('F', t))
            else:
                out.append(('T', t))
        out.append(('T', '\n'))
    return out


def skeleton(tk, entry):
    life = {}
    s = []
    for t in tk:       # whitespace stays in: copies of one group then have the same token positions
        if t[0] == 'A':
            s.append('@A%x' % (t[1] - entry))
        elif t[0] == 'H':
            s.append('@H' + t[2])
        elif t[0] == 'F':
            s.append('@F')
        elif t[0] == 'L':
            s.append(life.setdefault(t[1], "'L%d" % len(life)))
        else:
            s.append(t[1])
    return hashlib.sha1('\x00'.join(s).encode()).hexdigest()


# ---- fast scan: skeleton(toks(body), entry) and the H / F values of toks(body), from a per-process cache of lines
# A line's key is its text with every lifetime (LIFECUT) and every hex literal (HEXCUT) cut out: the same patterns as the
# lifetime and hex tokens of TOK, not after an identifier character or '. The first line with a key is tokenized with toks()
# and kept only when the cut matches are exactly its lifetime and hex tokens. Every other line with that key then has the same
# tokens apart from the lifetime names and hex digits: the text between the cuts is the same, a cut never follows an identifier
# character, and after a lifetime or a hex literal comes a character that neither can take, so every cut is one token and
# starts and ends at a token boundary. Its skeleton part is then the kept one with its own lifetimes renamed in order, and its
# H values are its HEXCUT matches. Lines with a same-crate fn name, arms and lines whose key was not kept go through toks().
HEXCUT = re.compile(r"(?<![A-Za-z0-9_'])0x([0-9a-f]+)((?:u8|u16|u32|u64|u128|i8|i16|i32|i64|usize)?)")
LIFECUT = re.compile(r"(?<![A-Za-z0-9_'])'[A-Za-z_][A-Za-z0-9_]*")
_LINES = {}


def _cut_key(ln):
    if "'" in ln:
        ln = LIFECUT.sub('\x02', ln)
    return HEXCUT.sub('\x01\\2', ln)


def _line_entry(ln):
    raw = TOK.findall(ln)
    tk = toks([ln], ())[:-1]
    assert len(raw) == len(tk)
    hs, ls, pos = [], [], 0
    for r, t in zip(raw, tk):
        if t[0] == 'H':
            hs.append((pos, pos + len(r)))
        elif t[0] == 'L':
            ls.append((pos, pos + len(r)))
        pos += len(r)
    if [m.span() for m in HEXCUT.finditer(ln)] != hs or [m.span() for m in LIFECUT.finditer(ln)] != ls:
        return False
    part = '\x00'.join(['@H' + t[2] if t[0] == 'H' else '\x02' if t[0] == 'L' else t[1] for t in tk] + ['\n'])
    return (part.split('\x02') if ls else part), frozenset(t[1] for t in tk if t[0] == 'T'), bool(hs), bool(ls)


def scan_fn(body, names, entry, want):
    """(skeleton(toks(body, names), entry), H values, F names) of toks(body, names); the values only when want."""
    parts, hv, fv = [], [], []
    life = {}
    for ln in body:
        if ARM.match(ln) is None and '\x01' not in ln and '\x02' not in ln:
            key = _cut_key(ln)
            e = _LINES.get(key)
            if e is None:
                e = _LINES[key] = _line_entry(ln)
            if e and names.isdisjoint(e[1]):
                if e[3]:
                    pc = e[0]
                    out = [pc[0]]
                    for nm, p in zip(LIFECUT.findall(ln), pc[1:]):
                        out.append(life.setdefault(nm, "'L%d" % len(life)))
                        out.append(p)
                    parts.append(''.join(out))
                else:
                    parts.append(e[0])
                if want and e[2]:
                    hv += [int(d, 16) for d, _ in HEXCUT.findall(ln)]
                continue
        s = []
        for t in toks([ln], names):
            if t[0] == 'A':
                s.append('@A%x' % (t[1] - entry))
            elif t[0] == 'H':
                s.append('@H' + t[2])
                hv.append(t[1])
            elif t[0] == 'F':
                s.append('@F')
                fv.append(t[1])
            elif t[0] == 'L':
                s.append(life.setdefault(t[1], "'L%d" % len(life)))
            else:
                s.append(t[1])
        parts.append('\x00'.join(s))
    sk = hashlib.sha1('\x00'.join(parts).encode()).hexdigest()
    return sk, (pack(hv) if want else None), (fv if want else None)


def pack(vals):
    try:
        return array('Q', vals)
    except OverflowError:
        return list(vals)


def put(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    data_ = text.encode('utf-8')
    try:
        if open(path, 'rb').read() == data_:
            return
    except OSError:
        pass
    open(path, 'wb').write(data_)


def u64_context(tk, i):
    """True when the literal at tk[i] is the right operand of `c.g(N) |` (or & ^): there it is a u64."""
    j = i - 1
    prev = []
    while j >= 0 and len(prev) < 6:
        if tk[j][0] != 'W':
            prev.append(tk[j][1] if tk[j][0] == 'T' else None)
        j -= 1
    while j >= 0 and tk[j][0] == 'W':
        j -= 1
    return len(prev) == 6 and prev[0] in BITOPS and prev[1] == ')' and prev[2] is not None and prev[2].isdigit() \
        and prev[3] == '(' and prev[4] == 'g' and prev[5] == '.' and j >= 0 and tk[j] == ('T', 'c')


# ---- worker jobs (each runs in a child process; arguments and results are plain data)

def job_scan(a):
    """Skeleton of every function; hex values and callee names of the ones large enough to share."""
    rec, cr, min_insns = a
    header, fns, tail, meta = read_crate(rec, cr)
    names = set(n for n, _ in fns)
    out = []
    for name, body in fns:
        entry, end = meta[name]
        n = (end - entry) // 4
        out.append((name, entry, n) + scan_fn(body, names, entry, n >= min_insns))
    return cr, '\n'.join(header), out


def job_templates(a):
    """Template text of the groups whose copy 0 is in this crate."""
    rec, cr, want = a      # want: [(tname, fn name, entry, kinds of the H positions, fslot of the F positions)]
    header, fns, tail, meta = read_crate(rec, cr)
    names = set(n for n, _ in fns)
    bodies = dict(fns)
    res = {}
    for tname, name, entry, kh, kf in want:
        m0 = toks(bodies[name], names)
        out, line = [], []
        ih = jf = 0
        for i, t in enumerate(m0):
            if t == ('T', '\n'):
                out.append(''.join(line))
                line = []
            elif t[0] == 'H':
                k = kh[ih]
                ih += 1
                ty = t[2] or ('zu64_' if k[0] != 'const' and u64_context(m0, i) else '_')
                if k[0] == 'const':
                    line.append('0x%x%s' % (t[1], t[2]))
                elif k[0] == 'rel':
                    e = 'zb_' if k[1] == 0 else 'zb_.wrapping_add(0x%x)' % k[1]
                    line.append(e if t[2] == 'u32' else '(%s as %s)' % (e, ty))
                else:
                    line.append('(zv_[%d] as %s)' % (k[1], ty))
            elif t[0] == 'F':
                line.append('(zf_[%d])' % kf[jf])
                jf += 1
            elif t[0] == 'A':
                line.append('0x%x => {' % (t[1] - entry))
            else:
                line.append(t[1])
        assert ih == len(kh) and jf == len(kf), (cr, name)
        # dispatch: the match on c.pc in front of the (now entry-relative) arms
        for j in range(len(out) - 1):
            if out[j] == 'match c.pc {' and ARM.match(out[j + 1]):
                out[j] = 'match c.pc.wrapping_sub(zb_) {'
        res[tname] = out
    return res


_TPL = {}      # per worker process: sh crate -> {template name: body text}, read from the written files


def templates_of(rec, sh):
    if sh not in _TPL:
        d = {}
        cur = None
        for ln in open(os.path.join(rec, sh, 'src', 'lib.rs'), encoding='utf-8').read().split('\n'):
            m = TPL_HDR.match(ln)
            if m:
                cur = (m.group(1), [])
                depth = 1
                continue
            if cur is not None:
                depth += ln.count('{') - ln.count('}')
                if depth == 0:
                    d[cur[0]] = '\n'.join(cur[1])
                    cur = None
                else:
                    cur[1].append(ln)
        _TPL[sh] = d
    return _TPL[sh]


def norm(tk):
    # lifetimes: a template keeps copy 0's label names; compare them up to consistent renaming
    life = {}
    return [(('L', life.setdefault(t[1], len(life))) if t[0] == 'L' else t) for t in tk if t[0] != 'W']


def life_text(s):
    """s with its lifetimes renamed in order of appearance ('\\x02' + number), or None when s has a ' that does not start a
    LIFECUT match (a char literal, a lifetime after an identifier character) or has a \\x02 of its own. Every LIFECUT match is a
    lifetime token of toks() (see the fast scan) and every L token starts with ', so two texts with equal results have equal
    tokens up to a consistent renaming of lifetimes: norm() of both is the same."""
    if '\x02' in s:
        return None
    d = {}
    r, n = LIFECUT.subn(lambda m: "'\x02%d" % d.setdefault(m.group(0), len(d)), s)
    return r if n == s.count("'") else None


def job_rewrite(a):
    """Shared copies become wrappers + value tables; then re-read the file and prove every copy."""
    rec, cr, inst = a      # inst: fn name -> (sh crate, template name, entry, values, callee names)
    header, fns, tail, meta = read_crate(rec, cr)
    names = set(n for n, _ in fns)
    src = allow_globals(header)
    used = set()
    orig = {}
    for name, body in fns:
        if name in inst:
            sh, tname, entry, vals, fn = inst[name]
            used.add(sh)
            orig[name] = body
            src.append(f'static V_{name}: [u64; {len(vals)}] = [{", ".join("%#x" % v for v in vals)}];')
            src.append(f'static F_{name}: [fn(&mut Cpu); {len(fn)}] = [{", ".join(fn)}];')
            src.append(f'pub fn {name}(c: &mut Cpu) {{ rec_{sh}::{tname}(c, {entry:#x}, &V_{name}, &F_{name}); }}')
        else:
            src.append(f'pub fn {name}(c: &mut Cpu) {{')
            src += body
            src.append('}')
    src += tail
    while src and src[-1] == '':
        src.pop()
    put(os.path.join(rec, cr, 'src', 'lib.rs'), '\n'.join(src) + '\n')
    deps = ''.join(f'rec-{s.replace("_", "-")} = {{ path = "../{s}" }}\n' for s in sorted(used))
    ct = open(os.path.join(rec, cr, 'Cargo.toml'), encoding='utf-8').read()
    ct = re.sub(r'\nrec-sh-\d+ = \{[^\n]*\}', '', ct).rstrip('\n') + '\n' + deps
    put(os.path.join(rec, cr, 'Cargo.toml'), ct)

    # proof: from the files as written
    text = open(os.path.join(rec, cr, 'src', 'lib.rs'), encoding='utf-8').read().split('\n')
    vt, ft = {}, {}
    for ln in text:
        m = re.match(r'^static V_(\w+): \[u64; \d+\] = \[(.*)\];$', ln)
        if m:
            vt[m.group(1)] = [int(x, 16) for x in m.group(2).split(', ') if x]
        m = re.match(r'^static F_(\w+): \[fn\(&mut Cpu\); \d+\] = \[(.*)\];$', ln)
        if m:
            ft[m.group(1)] = [x for x in m.group(2).split(', ') if x]
    suf = lambda s: '' if s in ('_', 'zu64_') else s
    checked = 0
    for ln in text:
        m = WRAP.match(ln)
        if not m:
            continue
        name, sh, tname, b = m.group(1), m.group(2)[4:], m.group(3), int(m.group(4), 16)
        if meta[name][0] != b:
            return cr, f'{name}: wrapper entry {b:#x} != FNS entry {meta[name][0]:#x}'
        v, f = vt[name], ft[name]
        # instantiate: replace template expressions by the copy's literal text, then compare token streams
        rebuilt = []
        for tl in templates_of(rec, sh)[tname].split('\n'):
            tl = tl.replace('match c.pc.wrapping_sub(zb_) {', 'match c.pc {')
            ma = re.match(r'^0x([0-9a-f]+) => \{$', tl)
            if ma:
                rebuilt.append('0x%x => {' % (b + int(ma.group(1), 16)))
                continue
            if 'z' not in tl:
                rebuilt.append(tl)
                continue
            tl = re.sub(r'\(zv_\[(\d+)\] as (\w+)\)', lambda q: '0x%x%s' % (v[int(q.group(1))], suf(q.group(2))), tl)
            tl = re.sub(r'\(zb_(?:\.wrapping_add\(0x([0-9a-f]+)\))? as (\w+)\)', lambda q: '0x%x%s' % (b + int(q.group(1) or '0', 16), suf(q.group(2))), tl)
            tl = re.sub(r'\bzb_\.wrapping_add\(0x([0-9a-f]+)\)', lambda q: '0x%xu32' % (b + int(q.group(1), 16)), tl)
            tl = re.sub(r'\bzb_\b', '0x%xu32' % b, tl)
            tl = re.sub(r'\(zf_\[(\d+)\]\)', lambda q: f[int(q.group(1))], tl)
            if re.search(r'\bz[bvf]_\b|zu64_', tl):
                return cr, f'{name}: template expression left after instantiation: {tl}'
            rebuilt.append(tl)
        ra, oa = '\n'.join(rebuilt), '\n'.join(orig[name])
        if ra == oa:
            checked += 1
            continue
        la = life_text(ra)
        if la is not None and la == life_text(oa):
            checked += 1
            continue
        x, y = norm(toks(rebuilt, names)), norm(toks(orig[name], names))
        if x != y:
            for i, (p, q) in enumerate(zip(x, y)):
                if p != q:
                    return cr, f'{name} at token {i}: {p} != {q}'
            return cr, f'{name}: length {len(x)} != {len(y)}'
        checked += 1
    if checked != len(inst):
        return cr, f'{checked} wrappers found, {len(inst)} written'
    return cr, checked


def gen_files(rec):
    """Every file of the generated crates (rec/* and the facade ../game), as paths relative to the oracle folder."""
    base = os.path.dirname(rec)
    out = []
    for top in (rec, os.path.join(base, 'game')):
        for d, sub, files in os.walk(top):
            sub[:] = [x for x in sub if x != 'target']
            out += [os.path.relpath(os.path.join(d, f), base) for f in files]
    return base, sorted(out)


def snapshot(rec, path):
    base, files = gen_files(rec)
    with open(path, 'w', encoding='utf-8', newline='\n') as w:
        for f in files:
            full = os.path.join(base, f)
            w.write('%s %d %s\n' % (hashlib.sha1(open(full, 'rb').read()).hexdigest(), os.stat(full).st_mtime_ns, f))
    print(f'snapshot: {len(files)} files')


def keep_times(rec, path):
    base, files = gen_files(rec)
    old = {}
    for ln in open(path, encoding='utf-8').read().split('\n'):
        if ln:
            h, t, f = ln.split(' ', 2)
            old[f] = (h, int(t))
    kept = 0
    for f in files:
        if f in old:
            full = os.path.join(base, f)
            h, t = old[f]
            if hashlib.sha1(open(full, 'rb').read()).hexdigest() == h:
                os.utime(full, ns=(t, t))
                kept += 1
    print(f'file times: {kept} of {len(files)} files unchanged (old time kept), {len(files) - kept} new or changed')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--rec', required=True)
    ap.add_argument('--snapshot', help='only write content hash + time of every generated file to this file')
    ap.add_argument('--keep-times', help='only give files whose content equals the --snapshot file the time they had then')
    ap.add_argument('--min-insns', type=int, default=24, help='smaller functions stay as they are (direct calls to them can be inlined)')
    ap.add_argument('--chunk', type=int, default=12000, help='instructions per template crate (as recgen_iso.py --chunk)')
    ap.add_argument('--jobs', type=int, default=0, help='worker processes (0 = CPU count, at most 12)')
    ap.add_argument('--dry', action='store_true', help='report only, write nothing')
    A = ap.parse_args()
    rec = os.path.abspath(A.rec)
    if A.snapshot:
        return snapshot(rec, A.snapshot)
    if A.keep_times:
        return keep_times(rec, A.keep_times)
    jobs = A.jobs or min(os.cpu_count() or 1, 12)

    crates = sorted(d for d in os.listdir(rec) if os.path.isfile(os.path.join(rec, d, 'Cargo.toml')))
    if any(SH_DIR.fullmatch(c) for c in crates):
        sys.exit('share_iso.py: rec/ already has sh_ crates (run it once, on fresh recgen_iso.py output)')
    # biggest crates first (shorter tail when the last ones run); the scan results are then put back in name order
    size = {c: os.path.getsize(os.path.join(rec, c, 'src', 'lib.rs')) for c in crates}
    big = sorted(crates, key=lambda c: (-size[c], c))
    with mp.Pool(jobs) as pool:
        groups = collections.defaultdict(list)   # skeleton -> [(crate, name, entry, insns, hex values, callees)]
        headers = set()
        scans = sorted(pool.imap_unordered(job_scan, [(rec, c, A.min_insns) for c in big]), key=lambda r: r[0])
        for cr, header, out in scans:
            headers.add(header)
            for name, entry, n, h, hv, fv in out:
                groups[h].append((cr, name, entry, n, hv, fv))
        scans = None
        assert len(headers) == 1, 'crates have different headers (generator flags differ)'
        HEADER = headers.pop().split('\n')

        # ---- templates: groups of 2+ copies with at least --min-insns instructions, in order of first appearance
        ok = lambda g: len(g) >= 2 and g[0][3] >= A.min_insns and all(m[4] is not None for m in g)
        order = sorted((min((m[0], m[2]) for m in g), h) for h, g in groups.items() if ok(g))
        tjobs = collections.defaultdict(list)    # crate of copy 0 -> template jobs
        tsh = []                                 # (template name, sh crate, nv, nf)
        inst = collections.defaultdict(dict)     # crate -> fn name -> (sh crate, template name, entry, values, callees)
        cur_sh, cur_n = 0, 0
        for idx, (_, h) in enumerate(order):
            g = sorted(groups[h], key=lambda m: (m[0], m[2]))
            # classify every hex position: as is, entry-relative, or a per-copy value; every callee position: a fn slot
            kh, slots, nv = [], [], 0
            for i, col in enumerate(zip(*[m[4] for m in g])):
                if len(set(col)) == 1:
                    kh.append(('const',))
                else:
                    offs = set(v - m[2] for v, m in zip(col, g))
                    if len(offs) == 1 and min(offs) >= 0 and max(col) < 1 << 32:
                        kh.append(('rel', offs.pop()))
                    else:
                        kh.append(('slot', nv))
                        slots.append(i)
                        nv += 1
            assert all(len(m[4]) == len(kh) for m in g)
            fslot, kf = {}, []
            for col in zip(*[m[5] for m in g]):
                kf.append(fslot.setdefault(col, len(fslot)))
            assert all(len(m[5]) == len(kf) for m in g)
            nf = len(fslot)
            tname = 't_%d' % idx
            if cur_n >= A.chunk:
                cur_sh, cur_n = cur_sh + 1, 0
            cur_n += g[0][3]
            sh = 'sh_%02d' % cur_sh
            tsh.append((tname, sh, nv, nf))
            tjobs[g[0][0]].append((tname, g[0][1], g[0][2], kh, kf))
            for m in g:
                fn = [None] * nf
                for j, k in enumerate(kf):
                    fn[k] = m[5][j]
                inst[m[0]][m[1]] = (sh, tname, m[2], [m[4][i] for i in slots], fn)

        tot = sum(sum(m[3] for m in g) for g in groups.values())
        left = sum(sum(m[3] for m in g) for h, g in groups.items() if not ok(g)) + sum(groups[h][0][3] for _, h in order)
        ncopies = sum(len(v) for v in inst.values())
        print(f'{len(crates)} crates, {sum(len(g) for g in groups.values())} functions, {tot} instructions; {len(tsh)} templates for {ncopies} copies '
              f'in {cur_sh + 1} sh crates; instructions compiled: {left} ({100.0 * left / tot:.1f}%)', flush=True)
        groups = None
        if A.dry:
            return

        # ---- write the template crates
        body = {}
        for res in pool.imap_unordered(job_templates, [(rec, c, w) for c, w in sorted(tjobs.items())]):
            body.update(res)
        by_sh = collections.defaultdict(list)
        for t in tsh:
            by_sh[t[1]].append(t)
        for sh, ts in sorted(by_sh.items()):
            src = list(HEADER) + [ALIAS]
            for tname, _, nv, nf in ts:
                src.append(f'pub fn {tname}(c: &mut Cpu, zb_: u32, zv_: &[u64; {nv}], zf_: &[fn(&mut Cpu); {nf}]) {{')
                src += body[tname]
                src.append('}')
            src.append('pub static FNS: &[ee_core::rec::Fn] = &[];')
            d = os.path.join(rec, sh)
            put(os.path.join(d, 'Cargo.toml'), f'[package]\nname = "rec-{sh.replace("_", "-")}"\nversion = "0.1.0"\nedition = "2021"\n\n'
                f'[dependencies]\nee-core = {{ path = "../../core" }}\n')
            put(os.path.join(d, 'src', 'lib.rs'), '\n'.join(src) + '\n')
        body = None

        # ---- rewrite the level/boot crates (shared copies become wrappers + value tables), each proved from the written files
        checked, failed = 0, []
        for cr, r in pool.imap_unordered(job_rewrite, [(rec, c, inst.get(c, {})) for c in big]):
            if isinstance(r, str):
                failed.append(f'{cr} {r}')
            else:
                checked += r
    if failed:
        sys.exit('share_iso.py: PROOF FAILED ' + '; '.join(sorted(failed)[:5]))
    assert checked == ncopies, (checked, ncopies)
    print(f'proof: {checked} copies rebuilt from wrapper + template == original tokens')


if __name__ == '__main__':
    main()
