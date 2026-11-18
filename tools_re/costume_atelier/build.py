#!/usr/bin/env python3
"""아바타 옷장 169차 — every costume set drawn pixel by pixel (no recolouring of shipped art).

    python tools_re/costume_atelier/build.py --lint [NN|slug ..]     # draw in memory and check everything
    python tools_re/costume_atelier/build.py --check [NN ..]         # rebuild, compare with costumes/ pixel by pixel
    python tools_re/costume_atelier/build.py --out <dir> [NN ..]     # write the sets into <dir>
    python tools_re/costume_atelier/build.py --force [NN ..]         # overwrite costumes/NN_* (all sets if none named)
    python tools_re/costume_atelier/preview.py <slug> --out <dir>    # look at a set on the heroes

★ The PNGs under `costumes/` are the source of truth (`costumes/README.md`); a set script here is the
record of how they were drawn. Nothing is written to `costumes/` without `--force`, because once a
frame is touched up by hand a rerun would throw the edit away — `--check` tells whether that happened.

A set is a module `sets/<slug>.py` with
    NUM    = '08'                      the set number (never reused — saves remember it)
    FOLDER = '08_red_fox'              costumes/<FOLDER>
    HERO   = 'ritz' | 'kei' | None     whose wardrobe shows it (None = both heroes)
    TEXT   = '''…'''                   costume.txt (names, descriptions, how it is acquired)
    build() -> {part: [PIL RGBA frames]}
"""
import argparse
import importlib
import os
import shutil
import sys
import tempfile

try:
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')
except AttributeError:
    pass

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'sets'))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'costume_factory'))
sys.path.insert(0, os.path.dirname(HERE))

import avatar_sheets as AS  # noqa: E402
import factory as F  # noqa: E402
import gen_avatar_catalog as CAT  # noqa: E402
import helmguide  # noqa: E402
import helmfit  # noqa: E402

ROOT = AS.COSTUMES
# set modules in set-number order (a module that does not exist yet is skipped with a note)
SETS = ['ian', 'halloween', 'kenneth', 'pool_kei', 'pool_ritz',
        'red_fox', 'imperial_guard', 'berserker', 'black_wolf', 'snow_lord', 'sheriff', 'reaper', 'hitman']
WEAPONS = {'kei': ['spear', 'sword', 'knife', 'dark', 'holy'],
           'ritz': ['spear', 'sword', 'knife', 'gun', 'rifle'],
           None: list(AS.WEAPON_KINDS)}
BODY = ['helmet', 'armor', 'gloves', 'shoes', 'shield']


def modules(keys):
    out = []
    for slug in SETS:
        try:
            m = importlib.import_module(slug)
        except ModuleNotFoundError as e:
            if e.name == slug:
                print(f'{slug}: no set script yet — skipped')
                continue
            raise
        if keys and m.NUM not in keys and slug not in keys:
            continue
        out.append(m)
    for k in keys:
        if not any(k in (m.NUM, m.__name__) for m in out):
            sys.exit(f'unknown set {k}')
    return out


def lint(m, parts):
    """Everything the game and the catalog will refuse later, said now."""
    bad = []
    hero = getattr(m, 'HERO', None)
    if not m.FOLDER.startswith(m.NUM + '_'):
        bad.append(f'FOLDER {m.FOLDER} does not start with {m.NUM}_')
    want = BODY + ['weapon_' + k for k in WEAPONS[hero]]
    for p in want:
        if p not in parts:
            bad.append(f'{p}: missing')
    for p, frames in parts.items():
        g = AS.PART_GROUP.get(p)
        if g is None:
            bad.append(f'{p}: not a part')
            continue
        sizes = AS.FRAME_SIZES[g]
        if len(frames) != len(sizes):
            bad.append(f'{p}: {len(frames)} frames, want {len(sizes)}')
            continue
        for i, (im, s) in enumerate(zip(frames, sizes)):
            if im.size != tuple(s):
                bad.append(f'{p}_{i}: {im.size} != {tuple(s)}')
            cols = {px[:3] for px in im.getdata() if px[3] >= 128}
            if not cols:
                bad.append(f'{p}_{i}: empty')
            if len(cols) > 255:
                bad.append(f'{p}_{i}: {len(cols)} colours')
    if 'helmet' in parts and getattr(m, 'HELMET_COVERS_HAIR', True) and not helmfit.own_head(m):
        for h in ([hero] if hero else ['kei', 'ritz']):
            miss = {j: v for j, v in helmguide.uncovered(parts['helmet'], h).items() if v}
            if miss:
                bad.append(f'helmet leaves {h}\'s hair bare: ' + ', '.join(f'j{j} {len(v)} px {v[:4]}' for j, v in miss.items()))
    # costume.txt through the catalog's own rules
    tmp = tempfile.mkdtemp()
    try:
        path = os.path.join(tmp, 'costume.txt')
        with open(path, 'w', encoding='utf-8') as f:
            f.write(m.TEXT)
        kv = CAT.parse(path)
        if kv.get('hero') != hero and not (hero is None and 'hero' not in kv):
            bad.append(f'TEXT hero = {kv.get("hero")} but HERO = {hero}')
        if kv.get('acquire') not in CAT.ACQUIRE:
            bad.append(f'TEXT acquire = {kv.get("acquire")}')
        known = {'set', 'acquire', 'hero', 'head'} | set(CAT.SLOTS) | {s + '.desc' for s in CAT.SLOTS} | {'weapon.' + w for w in CAT.WEAPON_KINDS}
        for k, v in kv.items():
            if k not in known:
                bad.append(f'TEXT: unknown key {k}')
                continue
            if k in ('acquire', 'hero', 'head'):
                continue
            limit = CAT.INFO_PX if (k == 'set' or k.endswith('.desc')) else CAT.ROW_PX
            if not CAT.drawable(v):
                bad.append(f'TEXT {k} {v!r}: a glyph the game font cannot draw')
            if CAT.width(v) > limit:
                bad.append(f'TEXT {k} {v!r}: {CAT.width(v)} px > {limit}')
        for k in WEAPONS[hero]:
            if 'weapon.' + k not in kv:
                bad.append(f'TEXT: no weapon.{k} name')
        for s in CAT.SLOTS:
            if s not in kv:
                bad.append(f'TEXT: no {s} name')
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return bad


def differences(built, shipped):
    bad = []
    for n in sorted(set(os.listdir(built)) | set(os.listdir(shipped) if os.path.isdir(shipped) else [])):
        a, b = os.path.join(built, n), os.path.join(shipped, n)
        if not os.path.exists(b):
            bad.append(f'{n}: built but not in costumes/')
        elif not os.path.exists(a):
            bad.append(f'{n}: in costumes/ but not built')
        elif n.endswith('.png'):
            if AS.read_png(a) != AS.read_png(b):
                bad.append(f'{n}: pixels differ')
        elif open(a, 'rb').read().replace(b'\r\n', b'\n') != open(b, 'rb').read().replace(b'\r\n', b'\n'):
            bad.append(f'{n}: differs')
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('sets', nargs='*', help='set numbers or slugs (default: every set)')
    how = ap.add_mutually_exclusive_group(required=True)
    how.add_argument('--lint', action='store_true')
    how.add_argument('--check', action='store_true')
    how.add_argument('--out')
    how.add_argument('--force', action='store_true')
    a = ap.parse_args()
    tmp = tempfile.mkdtemp() if a.check else None
    failed = False
    try:
        for m in modules(a.sets):
            raw = m.build()
            parts = helmfit.finish(m, raw)      # 171차: the head made smaller (helmfit.py)
            problems = helmfit.problems(m, raw) + lint(m, parts)
            if problems:
                print(f'{m.FOLDER}: LINT')
                for p in problems:
                    print('   ', p)
                failed = True
                if not a.lint:
                    continue
            if a.lint:
                if not problems:
                    print(f'{m.FOLDER}: OK')
                continue
            dst = os.path.join(tmp or a.out or ROOT, m.FOLDER)
            F.write_set(dst, parts, m.TEXT)
            if not a.check:
                print(f'{m.FOLDER}: written to {dst}')
                continue
            bad = differences(dst, os.path.join(ROOT, m.FOLDER))
            print(f'{m.FOLDER}: {"OK" if not bad else "STALE"}')
            for b in bad[:12]:
                print('   ', b)
            failed |= bool(bad)
    finally:
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
