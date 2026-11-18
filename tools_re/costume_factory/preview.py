"""Costume preview: composite a costume (PNG folder or frame dict) on both heroes in many poses."""
import os
import re
import sys

from PIL import Image, ImageDraw

import bmview as V
import cif
import doll as D

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import avatar_sheets as AS  # noqa: E402

HEADS = {'ritz': 0, 'kei': 1}   # 169차 기기 확인: 이름 리츠(CHAR+0xb8) = 은발 h00000 (168차의 반대 가정은 틀렸다)
# weapon category -> cif file with that weapon's full animation set
WEAPON_CIF = {4: 5, 5: 10, 6: 9, 7: 6, 8: 7, 9: 8, 10: 4}
PART_LAYER = {'helmet': 6, 'armor': 1, 'gloves': 2, 'shoes': 3, 'shield': 7}


def png_frames(folder, part):
    out = []
    for name in os.listdir(folder):
        m = re.fullmatch(re.escape(part) + r'_(\d+)\.png', name)
        if m:
            out.append((int(m.group(1)), name))
    out.sort()
    return [Image.open(os.path.join(folder, n)).convert('RGBA') for _, n in out]


def folder_parts(folder):
    parts = {}
    for p in ['helmet', 'armor', 'gloves', 'shoes', 'shield'] + [f'weapon_{k}' for k in AS.WEAPON_KINDS]:
        fr = png_frames(folder, p)
        if fr:
            parts[p] = fr
    return parts


def layers_for(parts, hero, wcat, base=None):
    base = base or {}
    L = {0: D.load_sheet('h000', HEADS[hero])}
    L[1] = parts.get('armor') or base.get('armor') or D.load_sheet('h100', 0)
    L[2] = parts.get('gloves') or base.get('gloves') or D.load_sheet('h200', 0)
    L[3] = parts.get('shoes') or base.get('shoes') or D.load_sheet('h300', 0)
    if 'helmet' in parts:
        L[6] = parts['helmet']
    wk = AS.WEAPON_KINDS[wcat - 4]
    if f'weapon_{wk}' in parts:
        L[5] = parts[f'weapon_{wk}']
    else:
        L[5] = V.frames(f'res/hero/h5{wcat:02d}00_bm')
    if 'shield' in parts:
        L[7] = parts['shield']
    return L


def pose_list(cif_index, picks):
    r = cif.parse(V.jar().read(f'res/hero/h{cif_index}_cif'))
    out = []
    for (a, d, f) in picks:
        if a < len(r['anims']) and d < len(r['anims'][a]) and f < len(r['anims'][a][d]):
            out.append(r['anims'][a][d][f])
    return out


DEFAULT_PICKS = [(0, 1, 0), (0, 0, 0), (0, 2, 0), (0, 3, 0), (1, 1, 1), (1, 2, 1),
                 (2, 1, 1), (2, 3, 2), (3, 1, 2), (4, 2, 3)]


def sheet(parts, out, wcats=(5,), heroes=('kei', 'ritz'), picks=DEFAULT_PICKS, scale=4, title=None):
    rows = []
    for hero in heroes:
        for wcat in wcats:
            L = layers_for(parts, hero, wcat)
            recs = pose_list(WEAPON_CIF[wcat], picks)
            imgs = [D.compose(rec, L, size=(90, 84), anchor=(45, 62)) for rec in recs]
            rows.append((f'{hero}/{AS.WEAPON_KINDS[wcat - 4]}', imgs))
    size = V.contact(rows, out, scale=scale, bg=(126, 150, 120, 255))
    return size


def main():
    import argparse
    ap = argparse.ArgumentParser(description='Composite a costume folder on the heroes in many poses.')
    ap.add_argument('folder', help='costumes/NN_name (or a --out folder of build.py)')
    ap.add_argument('out', help='output PNG')
    ap.add_argument('--weapons', default='5', help='weapon categories 4..10, comma separated (default 5 = sword)')
    ap.add_argument('--hero', choices=['kei', 'ritz', 'both'], default='both')
    ap.add_argument('--scale', type=int, default=4)
    a = ap.parse_args()
    heroes = ('kei', 'ritz') if a.hero == 'both' else (a.hero,)
    wcats = tuple(int(w) for w in a.weapons.split(','))
    print(sheet(folder_parts(a.folder), a.out, wcats=wcats, heroes=heroes, scale=a.scale))


if __name__ == '__main__':
    main()
