#!/usr/bin/env python3
"""steam_shortcut.py - "Add to Steam" for Linux (the Steam Deck and desktop Steam): puts Ratchet & Clank (RAC-RS) into Steam's
list of non-Steam games, or takes it out again. The Linux counterpart of the -AddToSteam / -RemoveFromSteam part of setup.ps1; the
file format, the fields and the number Steam gives the entry (appid) are the same as there.

    python3 -I tools/steam_shortcut.py add|remove|status --root <install folder>

  add      one entry "Ratchet & Clank (RAC-RS)" in the shortcuts.vdf of every Steam user of every Steam folder found
           (~/.local/share/Steam, ~/.steam/steam, Flatpak ~/.var/app/com.valvesoftware.Steam/.local/share/Steam, Snap). The entry starts
           the GAME (<root>/build/release/rac-rs, start folder = <root>/), not the launcher, so it works from Gaming Mode with a controller.
  remove   takes our entry out again (and the pictures this tool made in the Steam grid folder, while they are still ours).
  status   prints one line, "found=1 has=0": Steam and a Steam user found / our entry is in some user's list.

Steam rewrites shortcuts.vdf when it closes, so it must be CLOSED first (we only look, we never close it). Before a file is changed it is
copied to shortcuts.vdf.bak; the new file is written beside it and moved into place. Every other entry is copied byte for byte.
Safe to run twice (no duplicate: an entry is ours when its AppName or its appid is ours).

Exit codes (the same as setup.ps1): 0 done (or nothing to do), 1 error, 2 Steam (or a Steam user) not found, 3 Steam is running (nothing changed).
Test switches: RAC_TEST_STEAM_DIR replaces the search for the Steam folder (tests never look at the real Steam); with it only
RAC_TEST_STEAM_RUNNING=1 says "Steam is running".
Python 3.8 or newer, standard library only.
"""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import zlib

APP_NAME = 'Ratchet & Clank (RAC-RS)'
EXE_REL = 'build/release/rac-rs'
ICON_REL = 'rac-rs/assets/icon/rac-rs-256.png'
LAUNCH_OPTIONS = 'RAC_RUN_LOG=1 %command%'   # the game writes player/log.txt (it does so by itself only when its stderr goes nowhere)
ART_LIST = 'generated/steam_art.txt'


class VdfError(Exception):
    pass


# --------------------------------------------------------------------------------------------------------------------- binary VDF
# A section is  0  key  0  fields...  8 ; a field is  type  key  0  value  with type 1 = text (then 0), 2 = 32-bit number, 8 = end of section.
# The file is  0 "shortcuts" 0  [entry "0", entry "1", ...]  8  8.

def _nul(b, pos):
    i = b.find(b'\x00', pos)
    if i < 0:
        raise VdfError('a text is not closed')
    return i + 1


def _skip(b, vtype, pos):
    """The index just after a value of this type that starts at pos (nested sections are skipped whole)."""
    if vtype == 0:
        while True:
            if pos >= len(b):
                raise VdfError('a section is not closed')
            t = b[pos]
            pos += 1
            if t == 8:
                return pos
            pos = _nul(b, pos)
            pos = _skip(b, t, pos)
    if vtype == 1:
        return _nul(b, pos)
    if vtype in (2, 3, 4, 6):
        e = pos + 4
    elif vtype in (7, 10):
        e = pos + 8
    else:
        raise VdfError('unknown value type %d' % vtype)
    if e > len(b):
        raise VdfError('the file is cut off')
    return e


def _text(b, a, z):
    return bytes(b[a:z]).decode('utf-8', 'replace')


def fields(b, pos, end):
    """The fields of one section body, from pos to end (= just after the closing 8): dicts type, key, from, vfrom, to."""
    out = []
    limit = end - 1
    while pos < limit:
        start = pos
        t = b[pos]
        pos += 1
        ks = pos
        pos = _nul(b, pos)
        key = _text(b, ks, pos - 1)
        vs = pos
        pos = _skip(b, t, pos)
        out.append({'type': t, 'key': key, 'from': start, 'vfrom': vs, 'to': pos})
    return out


def empty_shortcuts():
    return b'\x00shortcuts\x00\x08\x08'


def read_shortcuts(b):
    """Entries: key, from (the entry's type byte), body (after its key), to (after its closing 8)."""
    if len(b) < 13 or b[0] != 0:
        raise VdfError('not a shortcuts file')
    pos = _nul(b, 1)
    if bytes(b[1:pos - 1]) != b'shortcuts':
        raise VdfError('the first section is not "shortcuts"')
    body_from = pos
    entries = []
    while True:
        if pos >= len(b):
            raise VdfError('the file is cut off')
        t = b[pos]
        if t == 8:
            break
        if t != 0:
            raise VdfError('an entry is not a section')
        start = pos
        pos += 1
        ks = pos
        pos = _nul(b, pos)
        key = _text(b, ks, pos - 1)
        bs = pos
        pos = _skip(b, 0, pos)
        entries.append({'key': key, 'from': start, 'body': bs, 'to': pos})
    if pos + 2 != len(b) or b[pos + 1] != 8:
        raise VdfError('unexpected content after the shortcuts section')
    return {'body_from': body_from, 'dict_end': pos, 'entries': entries}


def entry_info(b, e):
    """Lower-case key -> value (text as str, numbers as int) of the fields of one entry."""
    info = {}
    for f in fields(b, e['body'], e['to']):
        k = f['key'].lower()
        if f['type'] == 1:
            info[k] = _text(b, f['vfrom'], f['to'] - 1)
        elif f['type'] == 2:
            info[k] = struct.unpack_from('<I', b, f['vfrom'])[0]
    return info


def vdf_field(key, value):
    kb = key.encode('utf-8') + b'\x00'
    if isinstance(value, str):
        return b'\x01' + kb + value.encode('utf-8') + b'\x00'
    return b'\x02' + kb + struct.pack('<I', value)


def new_entry(key, d):
    """A new entry with the fields Steam writes itself."""
    out = [b'\x00', key.encode('utf-8'), b'\x00']
    out.append(vdf_field('appid', d['appid']))
    out.append(vdf_field('AppName', d['AppName']))
    out.append(vdf_field('Exe', d['Exe']))
    out.append(vdf_field('StartDir', d['StartDir']))
    out.append(vdf_field('icon', d['icon']))
    out.append(vdf_field('ShortcutPath', ''))
    out.append(vdf_field('LaunchOptions', d.get('LaunchOptions', '')))
    for k, v in (('IsHidden', 0), ('AllowDesktopConfig', 1), ('AllowOverlay', 1), ('OpenVR', 0), ('Devkit', 0)):
        out.append(vdf_field(k, v))
    out.append(vdf_field('DevkitGameID', ''))
    out.append(vdf_field('DevkitOverrideAppID', 0))
    out.append(vdf_field('LastPlayTime', 0))
    out.append(vdf_field('FlatpakAppID', ''))
    out.append(b'\x00tags\x00\x08')
    out.append(b'\x08')
    return b''.join(out)


def update_entry(b, e, d):
    """Our entry exists: set only the five fields we own (appid, AppName, Exe, StartDir, icon); everything else in it (tags, play time,
    launch options ...) stays byte for byte."""
    own = {'appid': d['appid'], 'appname': d['AppName'], 'exe': d['Exe'], 'startdir': d['StartDir'], 'icon': d['icon']}
    names = {'appid': 'appid', 'appname': 'AppName', 'exe': 'Exe', 'startdir': 'StartDir', 'icon': 'icon'}
    seen = set()
    out = [bytes(b[e['from']:e['body']])]
    for f in fields(b, e['body'], e['to']):
        lk = f['key'].lower()
        if lk in own and lk not in seen:
            seen.add(lk)
            out.append(vdf_field(f['key'], own[lk]))
        else:
            out.append(bytes(b[f['from']:f['to']]))
    for lk in ('appid', 'appname', 'exe', 'startdir', 'icon'):
        if lk not in seen:
            out.append(vdf_field(names[lk], own[lk]))
    out.append(b'\x08')
    return b''.join(out)


def update_shortcuts(orig, mode, d):
    """mode 'add' (add our entry, or update it when it is there) or 'remove'. Returns (new bytes, changed, action) with action one of
    added updated unchanged removed absent. An entry is ours when its AppName is ours or its appid is ours. A removal from a file with
    consecutive keys 0,1,2 ... renumbers the keys after it (only the key text changes)."""
    orig = bytes(orig)
    if not orig and mode == 'remove':
        return orig, False, 'absent'
    b = orig if orig else empty_shortcuts()
    v = read_shortcuts(b)
    consecutive = True
    nxt = 0
    for idx, e in enumerate(v['entries']):
        if e['key'] != str(idx):
            consecutive = False
        if e['key'].isascii() and e['key'].isdigit():
            nxt = max(nxt, int(e['key']) + 1)
    out = [b[:v['body_from']]]
    action = 'added' if mode == 'add' else 'absent'
    matched = False
    keep = 0
    for e in v['entries']:
        info = entry_info(b, e)
        mine = (isinstance(info.get('appname'), str) and info['appname'] == d['AppName']) or \
               (isinstance(info.get('appid'), int) and info['appid'] == d['appid'])
        if mine and mode == 'remove':
            action = 'removed'
            continue
        if mine:
            if matched:                         # a second copy of ours is dropped
                action = 'updated'
                continue
            matched = True
            out.append(update_entry(b, e, d))
            action = 'updated'
            keep += 1
            continue
        if mode == 'remove' and consecutive:
            out.append(b'\x00' + str(keep).encode('ascii') + b'\x00' + b[e['body']:e['to']])
        else:
            out.append(b[e['from']:e['to']])
        keep += 1
    if mode == 'add' and not matched:
        out.append(new_entry(str(nxt), d))
    out.append(b[v['dict_end']:])
    result = b''.join(out)
    changed = result != orig
    if not changed:
        action = 'unchanged' if mode == 'add' else 'absent'
    return result, changed, action


def shortcut_appid(exe_quoted, name):
    """The number Steam gives a non-Steam shortcut: CRC-32 of (the Exe field as written, with its quotes) + AppName, top bit set.
    Grid artwork files are named after it."""
    return (zlib.crc32((exe_quoted + name).encode('utf-8')) & 0xFFFFFFFF) | 0x80000000


# --------------------------------------------------------------------------------------------------------------------- the entry
def entry_def(root):
    root = os.path.abspath(root).rstrip('/')
    exe = '"' + root + '/' + EXE_REL + '"'
    icon = root + '/' + ICON_REL
    return {
        'AppName': APP_NAME,
        'Exe': exe,
        'StartDir': '"' + root + '/"',
        'icon': icon if os.path.isfile(icon) else '',
        'LaunchOptions': LAUNCH_OPTIONS,
        'appid': shortcut_appid(exe, APP_NAME),
    }


# --------------------------------------------------------------------------------------------------------------------- Steam folders
def _is_steam_dir(p):
    return os.path.isdir(p) and (os.path.isdir(os.path.join(p, 'userdata')) or os.path.isfile(os.path.join(p, 'steam.sh')))


def steam_roots(home=None):
    """(real path of every Steam folder found, the Flatpak ones among them). RAC_TEST_STEAM_DIR replaces the search."""
    test = os.environ.get('RAC_TEST_STEAM_DIR')
    if test:
        return ([os.path.realpath(test)] if os.path.isdir(test) else []), []
    home = home or os.path.expanduser('~')
    data = os.environ.get('XDG_DATA_HOME') or os.path.join(home, '.local', 'share')
    flat = os.path.join(home, '.var', 'app', 'com.valvesoftware.Steam')
    cands = [
        os.path.join(data, 'Steam'),
        os.path.join(home, '.local', 'share', 'Steam'),
        os.path.join(home, '.steam', 'steam'),
        os.path.join(home, '.steam', 'root'),
        os.path.join(flat, '.local', 'share', 'Steam'),
        os.path.join(flat, 'data', 'Steam'),
        os.path.join(home, 'snap', 'steam', 'common', '.local', 'share', 'Steam'),
    ]
    roots, flatpak, seen = [], [], set()
    for c in cands:
        if not _is_steam_dir(c):
            continue
        rp = os.path.realpath(c)               # ~/.steam/steam is usually a link to ~/.local/share/Steam: one folder, one entry
        if rp in seen:
            continue
        seen.add(rp)
        roots.append(rp)
        if rp.startswith(os.path.realpath(flat) + os.sep):
            flatpak.append(rp)
    return roots, flatpak


def user_dirs(steam):
    ud = os.path.join(steam, 'userdata')
    if not os.path.isdir(ud):
        return []
    out = []
    for n in sorted(os.listdir(ud)):
        p = os.path.join(ud, n)
        if n.isascii() and n.isdigit() and n != '0' and os.path.isdir(p):
            out.append(p)
    return out


def steam_running():
    """Is Steam running for this user? The program is called steam (what `pgrep -x steam` finds). We only look, we never close it."""
    if os.environ.get('RAC_TEST_STEAM_DIR'):
        return os.environ.get('RAC_TEST_STEAM_RUNNING') == '1'
    try:
        names = os.listdir('/proc')
        me = os.getuid()
    except (OSError, AttributeError):
        names = None
    if names is not None:
        for n in names:
            if not n.isdigit():
                continue
            try:
                with open('/proc/%s/comm' % n, 'rb') as f:
                    if f.read().strip() != b'steam':
                        continue
                if os.stat('/proc/%s' % n).st_uid == me:
                    return True
            except OSError:
                continue
        return False
    try:
        return subprocess.call(['pgrep', '-x', 'steam'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) == 0
    except OSError:
        return False


# --------------------------------------------------------------------------------------------------------------------- one user's file
def set_shortcuts_file(path, mode, d):
    """One user's shortcuts.vdf. A .bak of the file as it was is written first; the new file is written beside it and moved into place.
    Returns the action."""
    exists = os.path.isfile(path)
    if not exists and mode == 'remove':
        return 'absent'
    orig = b''
    if exists:
        with open(path, 'rb') as f:
            orig = f.read()
    new, changed, action = update_shortcuts(orig, mode, d)
    if not changed:
        return action
    if exists:
        shutil.copyfile(path, path + '.bak')
    else:
        os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + '.rac-tmp'
    with open(tmp, 'wb') as f:
        f.write(new)
    os.replace(tmp, path)
    return action


# --------------------------------------------------------------------------------------------------------------------- the three commands
def invoke(mode, root):
    """mode 'add' or 'remove', for every Steam user. Returns (status, text lines): status ok / nosteam / running / error."""
    roots, flatpak = steam_roots()
    if not roots:
        return 'nosteam', ['Steam was not found on this PC.']
    users = [(r, u) for r in roots for u in user_dirs(r)]
    if not users:
        return 'nosteam', ['Steam is here, but no Steam user has signed in yet. Start Steam once, sign in, close it, then try again.']
    if steam_running():
        return 'running', ['Steam is running. Close Steam first (it rewrites its shortcut list when it closes), then try again.',
                           'On the Steam Deck: switch to Desktop Mode, then Steam menu > Exit. In Gaming Mode Steam is always running.']
    d = entry_def(root)
    if mode == 'add' and not os.path.isfile(os.path.join(os.path.abspath(root), EXE_REL)):
        return 'error', ['The game is not built yet (%s is missing). Finish the setup first.' % EXE_REL]
    acts, errs = [], []
    for r, u in users:
        vdf = os.path.join(u, 'config', 'shortcuts.vdf')
        try:
            acts.append(set_shortcuts_file(vdf, mode, d))
        except (OSError, VdfError) as e:
            errs.append('%s: %s' % (vdf, e))
    if mode == 'remove':
        remove_art(root)
    if not acts:
        return 'error', ['Steam could not be changed: ' + '; '.join(errs)]
    if mode == 'add' and all(a == 'unchanged' for a in acts):
        text = '"%s" is already in Steam. Nothing was changed.' % APP_NAME
    elif mode == 'add':
        text = 'Added "%s" to Steam. Start Steam to see it in your library (on the Steam Deck: Library > Non-Steam).' % APP_NAME
    elif all(a == 'absent' for a in acts):
        text = '"%s" was not in Steam. Nothing was changed.' % APP_NAME
    else:
        text = 'Removed "%s" from Steam.' % APP_NAME
    lines = [text]
    if errs:
        lines.append('(Not every Steam user could be changed: ' + '; '.join(errs) + ')')
    if mode == 'add' and flatpak:
        lines.append('Steam from Flatpak can only start the game if it may read this folder: '
                     'flatpak override --user --filesystem="%s" com.valvesoftware.Steam' % os.path.abspath(root))
    return 'ok', lines


def status(root):
    """(found, has): Steam and at least one user are there; our entry is in some user's shortcuts.vdf."""
    roots, _ = steam_roots()
    users = [u for r in roots for u in user_dirs(r)]
    if not users:
        return False, False
    d = entry_def(root)
    for u in users:
        f = os.path.join(u, 'config', 'shortcuts.vdf')
        if os.path.isfile(f):
            try:
                with open(f, 'rb') as h:
                    if update_shortcuts(h.read(), 'remove', d)[2] == 'removed':
                        return True, True
            except (OSError, VdfError):
                pass
    return True, False


def remove_art(root):
    """Pictures this tool made (rows "SHA-256 TAB path" in generated/steam_art.txt) go only while their hash is still ours."""
    lst = os.path.join(os.path.abspath(root), ART_LIST)
    if not os.path.isfile(lst):
        return
    try:
        with open(lst, encoding='utf-8') as f:
            rows = f.read().splitlines()
    except OSError:
        return
    for line in rows:
        parts = line.split('\t', 1)
        if len(parts) != 2 or not parts[0].strip() or not os.path.isfile(parts[1]):
            continue
        try:
            with open(parts[1], 'rb') as f:
                if hashlib.sha256(f.read()).hexdigest().lower() == parts[0].strip().lower():
                    os.remove(parts[1])
        except OSError:
            pass
    try:
        os.remove(lst)
    except OSError:
        pass


EXIT = {'ok': 0, 'nosteam': 2, 'running': 3, 'error': 1}


def main(argv):
    args = list(argv)
    if not args or args[0] not in ('add', 'remove', 'status') or '--root' not in args or args.index('--root') + 1 >= len(args):
        sys.stderr.write(__doc__.split('\n\n')[1] + '\n')
        return 1
    mode = args[0]
    root = args[args.index('--root') + 1]
    try:
        if mode == 'status':
            found, has = status(root)
            print('found=%d has=%d' % (found, has))
            return 0
        st, lines = invoke(mode, root)
    except Exception as e:                      # never a traceback in front of a player
        print('Steam could not be changed: %s' % e)
        return 1
    for l in lines:
        print(l)
    return EXIT[st]


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
