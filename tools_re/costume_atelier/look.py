"""Look at a costume on the heroes: the game's own paper doll (cif records), offline.

    python tools_re/costume_atelier/look.py costumes/08_red_fox out.png [--hero ritz] [--weapons 7,8]

`pose_sheet(parts, ...)` takes a dict part -> [PIL frames] straight from a set script, so a set can be
looked at before anything is written to costumes/.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'costume_factory'))
sys.path.insert(0, os.path.dirname(HERE))

from PIL import Image, ImageDraw  # noqa: E402

import bmview as V  # noqa: E402
import cif as C  # noqa: E402
import doll as D  # noqa: E402
import avatar_sheets as AS  # noqa: E402

HEADS = {'ritz': 0, 'kei': 1}   # 169차 기기 확인: 이름 리츠(CHAR+0xb8) = 은발 머리 h00000
WEAPON_CIF = {4: 5, 5: 10, 6: 9, 7: 6, 8: 7, 9: 8, 10: 4}
_CIF = {}


def cif(i):
    if i not in _CIF:
        _CIF[i] = C.parse(V.jar().read(f'res/hero/h{i}_cif'))
    return _CIF[i]


def folder_parts(folder):
    import re
    parts = {}
    for p in ['helmet', 'armor', 'gloves', 'shoes', 'shield'] + [f'weapon_{k}' for k in AS.WEAPON_KINDS]:
        fr = []
        for n in os.listdir(folder):
            m = re.fullmatch(re.escape(p) + r'_(\d+)\.png', n)
            if m:
                fr.append((int(m.group(1)), n))
        if fr:
            parts[p] = [Image.open(os.path.join(folder, n)).convert('RGBA') for _, n in sorted(fr)]
    return parts


def layers(parts, hero, wcat, helmet_fallback=None, own_head=False):
    """`own_head` (171차, costume.txt `head = own`): the 투구 frames carry the whole head, so the hero's
    head piece is left out wherever a 투구 piece follows it — what the game's painter hook does."""
    L = {0: D.load_sheet('h000', HEADS[hero])}
    if own_head and 'helmet' in parts:
        L['own_head'] = True
    L[1] = parts.get('armor') or D.load_sheet('h100', 0)
    L[2] = parts.get('gloves') or D.load_sheet('h200', 0)
    L[3] = parts.get('shoes') or D.load_sheet('h300', 0)
    if 'helmet' in parts:
        L[6] = parts['helmet']
    elif helmet_fallback is not None:
        L[6] = D.load_sheet('h600', helmet_fallback)
    wk = AS.WEAPON_KINDS[wcat - 4]
    L[5] = parts.get(f'weapon_{wk}') or V.frames(f'res/hero/h5{wcat:02d}00_bm')
    if 'shield' in parts:
        L[7] = parts['shield']
    return L


# (anim, dir, frame) picks per weapon cif that show idle/walk in four directions + a few attacks
PICKS = [(0, 1, 0), (0, 2, 0), (0, 0, 0), (0, 3, 0), (1, 1, 1), (1, 2, 1), (1, 0, 1), (2, 1, 1), (2, 2, 1),
         (3, 1, 2), (3, 2, 2), (4, 2, 3)]


def records(ci, picks):
    r = cif(ci)
    out = []
    for (a, d, f) in picks:
        if a < len(r['anims']) and d < len(r['anims'][a]) and f < len(r['anims'][a][d]):
            out.append(r['anims'][a][d][f])
    return out


def contact(rows, out, scale=4, pad=4, bg=(126, 150, 120, 255)):
    lab_w = 64
    width = max(sum(i.width * scale + pad for i in imgs) for _, imgs in rows) + pad + lab_w
    height = sum(max([i.height for i in imgs] + [8]) * scale + pad for _, imgs in rows) + pad
    sheet = Image.new('RGBA', (width, height), bg)
    d = ImageDraw.Draw(sheet)
    y = pad
    for lab, imgs in rows:
        h = max([i.height for i in imgs] + [8]) * scale
        d.text((2, y), str(lab), fill=(255, 255, 255, 255))
        x = pad + lab_w
        for img in imgs:
            big = img.resize((img.width * scale, img.height * scale), Image.NEAREST)
            sheet.alpha_composite(big, (x, y))
            x += big.width + pad
        y += h + pad
    sheet.save(out)
    return sheet.size


def pose_sheet(parts, out, heroes=('ritz',), wcats=(5,), picks=PICKS, scale=4, size=(80, 72), anchor=(40, 56),
               helmet_fallback=4, extra_rows=(), own_head=False):
    rows = []
    for hero in heroes:
        for wcat in wcats:
            L = layers(parts, hero, wcat, helmet_fallback, own_head=own_head)
            recs = records(WEAPON_CIF[wcat], picks)
            imgs = [D.compose(rec, L, size=size, anchor=anchor) for rec in recs]
            rows.append((f'{hero}/{AS.WEAPON_KINDS[wcat - 4]}', imgs))
    rows.extend(extra_rows)
    return contact(rows, out, scale=scale)


def frames_sheet(parts, out, scale=6):
    rows = [(k, v) for k, v in parts.items()]
    return contact(rows, out, scale=scale, bg=(110, 128, 104, 255))


def main():
    import argparse
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('folder')
    ap.add_argument('out')
    ap.add_argument('--hero', choices=['kei', 'ritz', 'both'], default='both')
    ap.add_argument('--weapons', default='5')
    ap.add_argument('--scale', type=int, default=4)
    a = ap.parse_args()
    heroes = ('kei', 'ritz') if a.hero == 'both' else (a.hero,)
    print(pose_sheet(folder_parts(a.folder), a.out, heroes, tuple(int(w) for w in a.weapons.split(',')), scale=a.scale))


if __name__ == '__main__':
    main()
