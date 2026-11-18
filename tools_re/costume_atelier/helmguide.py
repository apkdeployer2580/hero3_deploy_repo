"""Where a helmet frame sits on each hero's head — the guide for drawing hair/hats/masks.

    python tools_re/costume_atelier/helmguide.py ritz      # print the four guides
    python tools_re/costume_atelier/helmguide.py kei

The paper doll draws helmet frame j over head frame k with a fixed offset (cif records, measured
over every pose): j0 (front) on head 0, j1 (rare 3/4) on head 3, j2 (back) on head 2, j3 (side,
faces LEFT) on head 1. In each guide:

    h  a pixel of the hero's own HAIR under the helmet -> a hair/hat helmet must cover it
    f  a pixel of the FACE (skin, eyes, mouth)         -> leave it transparent (or draw a mask on it)
    o  hair only at the second (rarer) placement       -> cover it too if you can
    ,  face only at the second placement

`uncovered(frames, hero)` lists the `h` pixels a set of four helmet frames leaves bare.
"""
import os
import sys

try:
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')
except AttributeError:
    pass

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'costume_factory'))
sys.path.insert(0, os.path.dirname(HERE))

import avatar_sheets as AS  # noqa: E402
import bmview as V  # noqa: E402

HELM = AS.FRAME_SIZES[0]
# helmet frame j -> (head frame k, [offsets helmet centre - head centre], main placement first)
PLACE = {0: (0, [(-1, -4), (0, -4)]), 1: (3, [(1, -4)]), 2: (2, [(1, -4), (2, -4)]), 3: (1, [(0, -3), (0, -4)])}
HEAD_SHEET = {'ritz': 'res/hero/h00000_bm', 'kei': 'res/hero/h00001_bm'}   # 169차 기기 확인: 이름 리츠 = 은발 h00000
FACE = {   # face colours/boxes per HEAD (169차: keys follow HEAD_SHEET — ritz = silver h00000, kei = orange h00001)
    'kei': {'cols': {'de797b', 'ff997b', 'ffdabd', 'fff2e6', '3991ac', '290008', '6a1020', '7b3839', 'ac4839',
                      '624039', '291829', '413031', '83595a', 'fff2c5'},
             'box': {1: (12, 17, 5, 13), 2: (11, 17, 4, 10), 3: (11, 17, 4, 12)}},
    'ritz': {'cols': {'d5898b', 'eea18b', 'f6dac5', 'ffdabd', 'eee2f6', 'fff2e6', '6a1020', '734041', 'ac4839',
                     '3991ac', '290008', '201018', '413031'},
            'box': {1: (13, 17, 5, 13), 2: (12, 17, 3, 9), 3: (12, 17, 3, 11)}},
}
_HEADS = {}


def head(hero):
    if hero not in _HEADS:
        _HEADS[hero] = V.frames(HEAD_SHEET[hero])
    return _HEADS[hero]


def guide(hero):
    """{helmet frame j: rows of 'h'/'f'/'o'/','/'.'} at the helmet frame's size."""
    heads = head(hero)
    out = {}
    for j, (k, offs) in PLACE.items():
        wm, hm = HELM[j]
        im = heads[k]
        wh, hh = im.size
        g = [['.'] * wm for _ in range(hm)]
        for oi, (ox, oy) in enumerate(offs):
            hx = wm // 2 - wh // 2 - ox
            hy = hm // 2 - hh // 2 - oy
            for y in range(hh):
                for x in range(wh):
                    p = im.getpixel((x, y))
                    if not p[3]:
                        continue
                    X, Y = x + hx, y + hy
                    if not (0 <= X < wm and 0 <= Y < hm):
                        continue
                    col = '%02x%02x%02x' % p[:3]
                    box = FACE[hero]['box'].get(k)
                    isface = bool(box) and box[0] <= y <= box[1] and box[2] <= x <= box[3] and col in FACE[hero]['cols']
                    c = 'f' if isface else 'h'
                    if oi > 0:
                        if g[Y][X] == '.':
                            g[Y][X] = 'o' if c == 'h' else ','
                    else:
                        g[Y][X] = c
        out[j] = g
    return out


def uncovered(frames, hero):
    """{j: [(x, y), ...]} hair pixels of `hero` that helmet frame j leaves transparent."""
    out = {}
    for j, g in guide(hero).items():
        miss = []
        for y, row in enumerate(g):
            for x, c in enumerate(row):
                if c == 'h' and frames[j].getpixel((x, y))[3] < 128:
                    miss.append((x, y))
        out[j] = miss
    return out


def on_head(frames, hero, size=40):
    """Each helmet frame composited on the hero's head at its main placement (PIL images)."""
    from PIL import Image
    heads = head(hero)
    out = []
    for j, (k, offs) in PLACE.items():
        ox, oy = offs[0]
        hm, hd = frames[j], heads[k]
        canvas = Image.new('RGBA', (size, size), (0, 0, 0, 0))
        c = size // 2
        canvas.alpha_composite(hd, (c - hd.width // 2, c + 2 - hd.height // 2))
        canvas.alpha_composite(hm, (c + ox - hm.width // 2, c + 2 + oy - hm.height // 2))
        out.append(canvas)
    return out


def main():
    hero = sys.argv[1] if len(sys.argv) > 1 else 'ritz'
    for j, g in guide(hero).items():
        print(f'== {hero} helmet {j} {HELM[j]}   h = hair to cover · f = face (keep) · o/, = 2nd placement only')
        print('   ' + ''.join(str(x % 10) for x in range(len(g[0]))))
        for y, r in enumerate(g):
            print(f'{y:2d} ' + ''.join(r))


if __name__ == '__main__':
    main()
