#!/usr/bin/env python3
"""update_apply.py - helper of setup.sh (Linux). Standard library only; run it with the private or the system Python.

  update_apply.py apply --root DIR --version X.Y.Z [--finish-only] [--test-base FOLDER_OR_URL]
      The update step of setup.sh --apply-update: a port of Get-UpdateZip, Expand-UpdateZip and Sync-Update of setup.ps1.
      Downloads v<X.Y.Z>.zip of the GitHub repository (the tag; the main branch only if the tag does not exist), unpacks it
      without its top folder, copies the files that are new or differ (SHA-256) over the install (each file written to a
      temporary name and renamed, so a running setup.sh or game is never half-overwritten; the copy gets the CURRENT time,
      because cargo rebuilds by file time), deletes the files the old release_files.txt lists and the new one does not, and
      never touches protected paths (toolchain, build, generated code, saves, mods, the disc image, ...).
      Output lines as setup.ps1 -ApplyUpdate; the exit code is 0 done, 1 the update could not be applied,
      3 the update could not be downloaded. Writes generated/update_expect.txt ("expect-compiles: N") for setup.sh.
  update_apply.py stamp --root DIR      set the time of the release files to now, once (a release unzipped by hand over an older
                                        install carries the ZIP's file times; cargo would keep the old game)
  update_apply.py fetch URL FILE        download one URL (used when neither curl nor wget exist)
  update_apply.py untar ARCHIVE DEST    unpack a .tar.xz/.tar.gz into DEST (used when xz is missing)
Downloads go through curl or wget when the machine has one (they use the system's certificate store, which already worked for the
tools), else through urllib.
"""
import hashlib
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.request
import zipfile

REPO_ARCHIVE = 'https://github.com/AsobiCodes/rac-rs/archive'


def out(msg):
    print(msg, flush=True)


# ---------------------------------------------------------------- paths an update never touches
def is_protected(rel):
    r = rel.replace('\\', '/').lstrip('/').lower()
    for p in ('toolchain/', 'build/', 'generated/', 'player/', 'oracle/rec/', 'oracle/game/', 'test_shots/', 'mods/', 'profiles/'):
        if r.startswith(p):
            return True
    if r in ('launcher.cfg', 'library.json', 'portable.txt', 'rac-rs launcher.exe', 'rac-rs launcher.exe.old', 'play.cmd',
             'oracle/src/vu0_prog0.rs', 'oracle/src/vu0_prog1.rs', 'oracle/src/vu0_prog9hi.rs', 'oracle/src/skinff_ee.rs',
             'oracle/src/skinff_vu.rs', 'rac-rs/src/gsfull/vu1aot_gen.rs', 'rac-rs/src/particles/orb_tables.rs'):
        return True
    if '/' not in r and (r.startswith('rac-rs-launcher') or r.startswith('rac-rs launcher')):
        return True      # the launcher program itself (any platform name)
    if re.match(r'^rac-rs/src/gsfull/vu1_prog\d+\.rs$', r):
        return True
    return r.endswith('.iso')


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def read_text(path):
    with open(path, 'rb') as f:
        return f.read().decode('utf-8', 'replace')


def version_tuple(v):
    return tuple(int(x) for x in v.split('.'))


# ---------------------------------------------------------------- download
def _http_error(url, code):
    return urllib.error.HTTPError(url, code, 'HTTP %d' % code, None, None)


def download_external(url, dest, timeout):
    """curl or wget, when there is one: they use the system's certificate store (urllib's bundled OpenSSL may not find it) and
    ignore the player's rc files. Returns False when neither exists; raises on a failed download (HTTPError for 404/410)."""
    pipe = subprocess.PIPE
    curl = shutil.which('curl')
    if curl:
        r = subprocess.run([curl, '-q', '-sSL', '--connect-timeout', '30', '--max-time', str(timeout), '-o', dest, '-w', '%{http_code}', url],
                           stdout=pipe, stderr=pipe, universal_newlines=True)
        code = (r.stdout or '').strip()[-3:]
        if r.returncode == 0 and code == '200':
            return True
        try:        # curl writes the body of an error page (14 bytes for a 404) into the file: no half result stays behind
            os.remove(dest)
        except OSError:
            pass
        if code in ('404', '410'):
            raise _http_error(url, int(code))
        raise RuntimeError('curl: %s' % ((r.stderr or '').strip() or 'exit %d, HTTP %s' % (r.returncode, code)))
    wget = shutil.which('wget')
    if wget:
        h = subprocess.run([wget, '--help'], stdout=pipe, stderr=subprocess.STDOUT, universal_newlines=True).stdout or ''
        opts = [o for o in ('--no-config', '--no-hsts') if o in h]
        r = subprocess.run([wget] + opts + ['-nv', '-T', '60', '-O', dest, url], stdout=pipe, stderr=pipe, universal_newlines=True)
        if r.returncode == 0:
            return True
        try:        # wget -O leaves an empty file behind
            os.remove(dest)
        except OSError:
            pass
        err = (r.stderr or '').strip()
        m = re.search(r'ERROR (404|410)', err)
        if m:
            raise _http_error(url, int(m.group(1)))
        raise RuntimeError('wget: %s' % (err.splitlines()[-1] if err else 'exit %d' % r.returncode))
    return False


def save_url(url, dest, timeout=600):
    if download_external(url, dest, timeout):
        return
    req = urllib.request.Request(url, headers={'User-Agent': 'rac-rs-setup'})
    with urllib.request.urlopen(req, timeout=timeout) as r, open(dest, 'wb') as f:
        shutil.copyfileobj(r, f, 1 << 20)


def get_update_zip(ver, zpath, test_base):
    """v<ver>.zip from the GitHub tag; with --test-base from there. Raises on failure."""
    if test_base:
        if re.match(r'^(https?|file)://', test_base):
            save_url(test_base.rstrip('/') + '/v%s.zip' % ver, zpath)
            return
        f = os.path.join(test_base, 'v%s.zip' % ver)
        if not os.path.isfile(f):
            raise RuntimeError('not found: %s' % f)
        shutil.copyfile(f, zpath)
        return
    not_found = False
    for attempt in (1, 2, 3):
        try:
            save_url('%s/refs/tags/v%s.zip' % (REPO_ARCHIVE, ver), zpath)
            return
        except urllib.error.HTTPError as e:
            if e.code == 404:
                not_found = True
                break
            out('  download failed (%s), retrying' % e)
            if attempt == 3:
                raise
            time.sleep(3 * attempt)
        except Exception as e:      # network errors
            out('  download failed (%s), retrying' % e)
            if attempt == 3:
                raise
            time.sleep(3 * attempt)
    if not_found:
        out('  no tag for this version yet: using the main branch')
        save_url('%s/refs/heads/main.zip' % REPO_ARCHIVE, zpath)


def expand_update_zip(zpath, outdir):
    """Unpacks the zip without its single top folder (GitHub: rac-rs-<version>/). Refuses paths that leave outdir."""
    os.makedirs(outdir, exist_ok=True)
    out_full = os.path.abspath(outdir) + os.sep
    top = None
    with zipfile.ZipFile(zpath) as za:
        for info in za.infolist():
            n = info.filename.replace('\\', '/')
            if n.startswith('/') or re.search(r'(^|/)\.\.(/|$)', n):
                raise RuntimeError('unsafe path in the update file: %s' % n)
            i = n.find('/')
            if i < 1:
                continue
            t = n[:i]
            if top is None:
                top = t
            elif top != t:
                raise RuntimeError('the update file has an unexpected layout')
            rel = n[i + 1:]
            if not rel or rel.endswith('/'):
                continue
            dst = os.path.abspath(os.path.join(out_full, *rel.split('/')))
            if not dst.startswith(out_full):
                raise RuntimeError('unsafe path in the update file: %s' % n)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            with za.open(info) as src, open(dst, 'wb') as f:
                shutil.copyfileobj(src, f, 1 << 20)
            mode = (info.external_attr >> 16) & 0o777
            if rel.endswith('.sh') or (stat.S_ISREG(info.external_attr >> 16) and mode & 0o111):
                os.chmod(dst, 0o755)
    for need in ('VERSION', 'rac-rs/Cargo.toml'):
        if not os.path.exists(os.path.join(out_full, *need.split('/'))):
            raise RuntimeError('the update file is not a rac-rs release (%s missing)' % need)
    if not (os.path.exists(os.path.join(out_full, 'setup.ps1')) or os.path.exists(os.path.join(out_full, 'setup.sh'))):
        raise RuntimeError('the update file is not a rac-rs release (setup.sh missing)')


# ---------------------------------------------------------------- copying
def read_list(path):
    if not os.path.isfile(path):
        return []
    return [l.strip() for l in read_text(path).splitlines() if l.strip()]


def put_file(src, dst):
    """Copies src to dst through a temporary name and a rename (the old file stays whole until the new one is complete)."""
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    tmp = dst + '.rac-new'
    shutil.copyfile(src, tmp)
    mode = os.stat(src).st_mode & 0o777
    if dst.endswith('.sh'):
        mode |= 0o755
    os.chmod(tmp, mode | 0o200)
    os.replace(tmp, dst)
    os.utime(dst, None)          # now: cargo rebuilds by file time


def sync_update(src, dst):
    """Copies the new version over this install. See the module doc. Returns (changed, removed)."""
    src = os.path.abspath(src)
    dst = os.path.abspath(dst)
    old_list = read_list(os.path.join(dst, 'release_files.txt'))
    new_list = read_list(os.path.join(src, 'release_files.txt'))
    last = ['VERSION', 'release_files.txt']
    items = []
    for root, dirs, files in os.walk(src):
        dirs.sort()
        for f in sorted(files):
            full = os.path.join(root, f)
            rel = os.path.relpath(full, src).replace(os.sep, '/')
            if is_protected(rel) or rel in last:
                continue
            items.append((rel, full))
    for rel in last:
        p = os.path.join(src, rel)
        if os.path.isfile(p):
            items.append((rel, p))
    changed = []
    for rel, full in items:
        d = os.path.join(dst, *rel.split('/'))
        copy = True
        if os.path.isfile(d) and not os.path.islink(d):
            if os.path.getsize(d) == os.path.getsize(full) and sha256_of(d) == sha256_of(full):
                copy = False
        if copy:
            put_file(full, d)
            changed.append(rel)
            out('  updated %s' % rel)
    removed = []
    if old_list and new_list:
        keep = set(new_list)
        for o in old_list:
            if o in keep or o in last or is_protected(o) or re.search(r'(^|/)\.\.(/|$)', o) or o.startswith('/'):
                continue
            d = os.path.join(dst, *o.split('/'))
            if os.path.isfile(d) or os.path.islink(d):
                os.remove(d)
                removed.append(o)
                out('  removed %s' % o)
                parent = os.path.dirname(d)
                while len(parent) > len(dst) and os.path.isdir(parent) and not os.listdir(parent):
                    os.rmdir(parent)
                    parent = os.path.dirname(parent)
    # Marker for the next setup run: this version's files were put in place HERE (changed files carry the current time).
    gdir = os.path.join(dst, 'generated')
    os.makedirs(gdir, exist_ok=True)
    ver = read_text(os.path.join(dst, 'VERSION')).strip() if os.path.isfile(os.path.join(dst, 'VERSION')) else ''
    if re.match(r'^\d+\.\d+\.\d+$', ver):
        with open(os.path.join(gdir, 'synced_' + ver), 'w', encoding='utf-8', newline='\n') as f:
            f.write(ver + '\n')
    return changed, removed


def set_release_file_times(root):
    """Sets the time of every release file to now (files unzipped by hand carry the ZIP's times; cargo would keep the old game)."""
    rels = read_list(os.path.join(root, 'release_files.txt'))
    if not rels:
        for d in ('oracle', 'rac-rs', 'tools'):
            for r, _, files in os.walk(os.path.join(root, d)):
                for f in files:
                    rels.append(os.path.relpath(os.path.join(r, f), root).replace(os.sep, '/'))
    n = 0
    for r in rels:
        if is_protected(r) or r.startswith('/') or re.search(r'(^|/)\.\.(/|$)', r):
            continue
        p = os.path.join(root, *r.split('/'))
        if os.path.isfile(p):
            os.utime(p, None)
            n += 1
    return n


def cmd_apply(argv):
    root = None
    ver = None
    finish = False
    test_base = None
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == '--root':
            root = argv[i + 1]; i += 2
        elif a == '--version':
            ver = argv[i + 1]; i += 2
        elif a == '--test-base':
            test_base = argv[i + 1]; i += 2
        elif a == '--finish-only':
            finish = True; i += 1
        else:
            print('update_apply.py: unknown option %s' % a, file=sys.stderr)
            return 1
    if not root or not ver or not re.match(r'^\d+\.\d+\.\d+$', ver):
        print('ERROR: update_apply.py apply needs --root DIR --version X.Y.Z', file=sys.stderr)
        return 1
    root = os.path.abspath(root)
    gdir = os.path.join(root, 'generated')
    os.makedirs(gdir, exist_ok=True)
    expect_file = os.path.join(gdir, 'update_expect.txt')
    local_ver_file = os.path.join(root, 'VERSION')
    exp = None           # None = a full rebuild: setup.sh counts the crates itself (the number depends on the platform)
    if not finish:
        utmp = os.path.join(root, 'toolchain', 'tmp', 'rac-rs-update', ver)      # inside the folder: nothing is written elsewhere
        try:
            if os.path.isdir(utmp):
                shutil.rmtree(utmp)
            os.makedirs(utmp)
            out('  downloading v%s ...' % ver)
            get_update_zip(ver, os.path.join(utmp, 'update.zip'), test_base)
            out('  unpacking ...')
            expand_update_zip(os.path.join(utmp, 'update.zip'), os.path.join(utmp, 'x'))
        except Exception as e:
            print('ERROR: could not download the update: %s' % e, file=sys.stderr, flush=True)
            shutil.rmtree(utmp, ignore_errors=True)
            return 3
        try:
            got = read_text(os.path.join(utmp, 'x', 'VERSION')).strip()
            local = read_text(local_ver_file).strip() if os.path.isfile(local_ver_file) else '0.0.0'
            if not re.match(r'^\d+\.\d+\.\d+$', got) or version_tuple(got) <= version_tuple(local):
                raise RuntimeError('the downloaded version (%s) is not newer than this one (%s)' % (got, local))
            out('  applying v%s ...' % got)
            changed, removed = sync_update(os.path.join(utmp, 'x'), root)
        except Exception as e:
            print('ERROR: could not apply the update: %s' % e, file=sys.stderr, flush=True)
            shutil.rmtree(utmp, ignore_errors=True)
            return 1
        shutil.rmtree(utmp, ignore_errors=True)
        # compiles to expect (progress bar): rac-rs only: the game library, the two build-order crates and the exe; the generated
        # crates depend on oracle/core (ee-core) only, so they rebuild when it or the generators change
        exp = 4
        if any(c.startswith('oracle/core/') or c.startswith('tools/') for c in changed):
            exp = None
        elif any(c.startswith('oracle/src/') or c == 'oracle/Cargo.toml' for c in changed):
            exp = 8
        elif any(c.startswith('oracle/hand/') for c in changed):
            exp = 3
        out('  %d files updated, %d removed' % (len(changed), len(removed)))
    else:
        out('  the files are already updated; only the rebuild is left')
    if exp is not None:
        out('expect-compiles: %d' % exp)
        with open(expect_file, 'w', encoding='utf-8', newline='\n') as f:
            f.write('expect-compiles: %d\n' % exp)
    elif os.path.isfile(expect_file):
        os.remove(expect_file)
    # Files that were not copied by sync_update (no generated/synced_<version>: a ZIP unzipped over the folder) carry the
    # ZIP's times: stamp them, or cargo may keep the old game.
    try:
        stamp_if_needed(root)
    except Exception as e:
        print('ERROR: could not apply the update: %s' % e, file=sys.stderr, flush=True)
        return 1
    # Executable bit of the scripts (a GitHub zip does not keep it).
    for f in os.listdir(root):
        if f.endswith('.sh') and os.path.isfile(os.path.join(root, f)):
            os.chmod(os.path.join(root, f), 0o755)
    return 0


def stamp_if_needed(root):
    """No generated/synced_<VERSION> marker: the release files came from a ZIP unzipped by hand. Sets their time to now (so that
    cargo rebuilds what changed) and writes the marker."""
    lv = read_text(os.path.join(root, 'VERSION')).strip()
    gdir = os.path.join(root, 'generated')
    mark = os.path.join(gdir, 'synced_' + lv)
    if os.path.isfile(mark):
        return
    nst = set_release_file_times(root)
    out('  the files were put in place by hand: set the time of %d files to now, so that everything is rebuilt once' % nst)
    os.makedirs(gdir, exist_ok=True)
    with open(mark, 'w', encoding='utf-8', newline='\n') as f:
        f.write(lv + '\n')


def cmd_stamp(argv):
    if len(argv) != 2 or argv[0] != '--root':
        print('ERROR: usage: update_apply.py stamp --root DIR', file=sys.stderr)
        return 2
    stamp_if_needed(os.path.abspath(argv[1]))
    return 0


def cmd_fetch(argv):
    if len(argv) != 2:
        print('usage: update_apply.py fetch URL FILE', file=sys.stderr)
        return 2
    try:
        save_url(argv[0], argv[1], timeout=1800)
    except Exception as e:
        print('download failed: %s' % e, file=sys.stderr)
        return 1
    return 0


def cmd_untar(argv):
    if len(argv) != 2:
        print('usage: update_apply.py untar ARCHIVE DEST', file=sys.stderr)
        return 2
    dest = os.path.abspath(argv[1])
    with tarfile.open(argv[0], 'r:*') as t:
        for m in t:
            n = m.name.replace('\\', '/')
            if n.startswith('/') or re.search(r'(^|/)\.\.(/|$)', n):
                print('unsafe path in the archive: %s' % n, file=sys.stderr)
                return 1
        if hasattr(tarfile, 'data_filter'):
            t.extractall(dest, filter='data')
        else:
            t.extractall(dest)
    return 0


def main():
    cmds = {'apply': cmd_apply, 'stamp': cmd_stamp, 'fetch': cmd_fetch, 'untar': cmd_untar}
    if len(sys.argv) < 2 or sys.argv[1] not in cmds:
        print(__doc__)
        return 2
    try:
        return cmds[sys.argv[1]](sys.argv[2:])
    except Exception as e:      # whatever it is: the launcher shows the last "ERROR:" line
        print('ERROR: update_apply.py %s failed: %s: %s' % (sys.argv[1], type(e).__name__, e), file=sys.stderr, flush=True)
        return 1


if __name__ == '__main__':
    sys.exit(main())
