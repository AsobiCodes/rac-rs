#!/usr/bin/env python3
"""mingw_as.py - unpacks the GNU assembler (as.exe) and the four DLLs it needs from pinned MSYS2 packages.

  python -I tools/mingw_as.py DEST PKG.pkg.tar.zst [PKG.pkg.tar.zst ...]

Called by setup.ps1 step (a). Why: on the GNU host, rustc makes the Windows API import libraries of the
'raw-dylib' crates (windows-link and its users) with dlltool.exe, which comes with Rust, and dlltool runs the
GNU assembler 'as', which does not come with Rust. setup.ps1 downloads the packages (SHA-256 checked) and this
script copies only these five files out of them into DEST (Rust's self-contained linker folder, next to
dlltool.exe). Uses only the standard library (tarfile reads .tar.zst since Python 3.14).
"""
import os
import sys
import tarfile

WANT = {
    'mingw64/bin/as.exe': 'as.exe',                 # binutils
    'mingw64/bin/libintl-8.dll': 'libintl-8.dll',   # gettext-runtime (as.exe imports it)
    'mingw64/bin/libiconv-2.dll': 'libiconv-2.dll', # libiconv (libintl-8.dll imports it)
    'mingw64/bin/zlib1.dll': 'zlib1.dll',           # zlib (as.exe)
    'mingw64/bin/libzstd.dll': 'libzstd.dll',       # zstd (as.exe)
}


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    dest = sys.argv[1]
    os.makedirs(dest, exist_ok=True)
    found = set()
    for pkg in sys.argv[2:]:
        with tarfile.open(pkg, 'r:zst') as t:
            for m in t.getmembers():
                name = WANT.get(m.name)
                if name is None or not m.isfile():
                    continue
                data = t.extractfile(m).read()
                tmp = os.path.join(dest, name + '.part')
                with open(tmp, 'wb') as f:
                    f.write(data)
                os.replace(tmp, os.path.join(dest, name))
                found.add(name)
    missing = sorted(set(WANT.values()) - found)
    if missing:
        sys.exit('mingw_as.py: not found in the packages: ' + ', '.join(missing))
    print('  unpacked ' + ', '.join(sorted(found)))


if __name__ == '__main__':
    main()
