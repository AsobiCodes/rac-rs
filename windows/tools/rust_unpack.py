#!/usr/bin/env python3
"""rust_unpack.py - installs the official Rust packages (.tar.xz) into ONE toolchain folder, without rustup.

  python -I tools/rust_unpack.py DEST PKG.tar.xz [PKG.tar.xz ...]

Called by setup.ps1 step (a) on a drive that cannot make hard links (exFAT, FAT32, some network shares): rustup needs hard
links for its cargo.exe / rustc.exe proxies and fails there. setup.ps1 downloads the four packages of rustup's "minimal" profile
(rustc, cargo, rust-std, rust-mingw; SHA-256 checked against the release manifest) and this script lays them out exactly as
rustup would: every file that a package's manifest.in lists goes to the same relative path under DEST (bin\\rustc.exe,
bin\\cargo.exe, lib\\rustlib\\<host>\\...). DEST is the folder rustup would have made (toolchain\\rustup\\toolchains\\<version>-<host>),
so the linker folder setup.ps1 puts on PATH is where it always was. It is built in DEST.part and renamed at the end: a stopped
install leaves no half toolchain. Uses only the standard library (tarfile reads .tar.xz).
"""
import os
import shutil
import sys
import tarfile


def read_manifest(pkg):
    """Pass 1: (top folder, {component: manifest.in text}) of one package."""
    comps, man, top = [], {}, ''
    with tarfile.open(pkg, 'r:xz') as t:
        for m in t:
            p = m.name.split('/')
            top = p[0]
            if m.isfile() and len(p) == 2 and p[1] == 'components':
                comps = t.extractfile(m).read().decode('utf-8').split()
            elif m.isfile() and len(p) == 3 and p[2] == 'manifest.in':
                man[p[1]] = t.extractfile(m).read().decode('utf-8')
    if not comps or any(c not in man for c in comps):
        sys.exit('rust_unpack.py: %s has no components / manifest.in' % os.path.basename(pkg))
    return top, comps, man


def safe(rel):
    return rel and not rel.startswith('/') and ':' not in rel and '..' not in rel.split('/')


def unpack(pkg, part):
    top, comps, man = read_manifest(pkg)
    n = 0
    for comp in comps:
        files, dirs = set(), []
        for line in man[comp].splitlines():
            kind, _, rel = line.strip().partition(':')
            if kind == 'file' and safe(rel):
                files.add(rel)
            elif kind == 'dir' and safe(rel):
                dirs.append(rel.rstrip('/'))
        done = set()
        with tarfile.open(pkg, 'r:xz') as t:
            for m in t:
                p = m.name.split('/')
                if len(p) < 3 or p[0] != top or p[1] != comp or not m.isfile():
                    continue
                rel = '/'.join(p[2:])
                if rel not in files and not any(rel.startswith(d + '/') for d in dirs):
                    continue
                dst = os.path.join(part, *p[2:])
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                with t.extractfile(m) as src, open(dst, 'wb') as out:
                    shutil.copyfileobj(src, out, 1 << 20)
                done.add(rel)
                n += 1
        missing = sorted(files - done)
        if missing:
            sys.exit('rust_unpack.py: %s: %d files of manifest.in are not in the package (first: %s)' % (comp, len(missing), missing[0]))
        print('  %s: %d files' % (comp, len(done)), flush=True)
    return n


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    dest = os.path.abspath(sys.argv[1])
    part = dest + '.part'
    if os.path.isdir(part):
        shutil.rmtree(part)
    os.makedirs(part)
    total = 0
    for pkg in sys.argv[2:]:
        total += unpack(pkg, part)
    for need in ('bin/rustc.exe', 'bin/cargo.exe'):
        if not os.path.isfile(os.path.join(part, *need.split('/'))):
            sys.exit('rust_unpack.py: %s is missing after unpacking' % need)
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    os.replace(part, dest)
    print('  %d files unpacked into %s' % (total, dest), flush=True)


if __name__ == '__main__':
    main()
