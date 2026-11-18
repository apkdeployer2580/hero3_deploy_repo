"""Look at a set script's output before anything is written to costumes/.

    python tools_re/costume_atelier/preview.py red_fox --out <dir> [--hero ritz] [--weapons 5,7]
    python tools_re/costume_atelier/preview.py --zones --out <dir>     # the body templates' zone maps

Writes into <dir>:
    <set>_frames.png   every frame of every part, big
    <set>_poses.png    worn by the hero(es) in idle / walk / attack poses, one row per weapon kind
    <set>_zoom.png     six poses, very big (judge the pixels here)
    <set>_game.png     what the game shows (1 game pixel = 3 screen pixels) on a field green
    <set>_helmet.png   the helmet frames alone and on the head; uncovered hair is printed
"""
import argparse
import importlib
import os
import sys

try:
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')
except AttributeError:
    pass

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'sets'))

import look  # noqa: E402  (puts costume_factory and tools_re on sys.path)
import doll as D  # noqa: E402
import helmguide  # noqa: E402
import helmfit  # noqa: E402
import avatar_sheets as AS  # noqa: E402
from PIL import Image  # noqa: E402

KIND_CAT = {k: 4 + i for i, k in enumerate(AS.WEAPON_KINDS)}   # spear 4 .. holy 10


def kinds_of(parts):
    return [KIND_CAT[p[len('weapon_'):]] for p in parts if p.startswith('weapon_')]


def game_sheet(parts, hero, wcats, out, cols=8, size=(56, 60), anchor=(28, 48), scale=3, own_head=False):
    cells = []
    for wc in wcats:
        L = look.layers(parts, hero, wc, None, own_head=own_head)
        for rec in look.records(look.WEAPON_CIF[wc], look.PICKS)[:cols]:
            cells.append(D.compose(rec, L, size=size, anchor=anchor))
    rows = (len(cells) + cols - 1) // cols
    sheet = Image.new('RGBA', (cols * size[0], max(1, rows) * size[1]), (78, 104, 70, 255))
    for i, c in enumerate(cells):
        sheet.alpha_composite(c, ((i % cols) * size[0], (i // cols) * size[1]))
    sheet.resize((sheet.width * scale, sheet.height * scale), Image.NEAREST).save(out)


def preview(name, out, heroes=None, wcats=None):
    mod = importlib.import_module(name)
    parts = helmfit.finish(mod, mod.build())
    own = helmfit.own_head(mod)
    heroes = heroes or ((getattr(mod, 'HERO', None),) if getattr(mod, 'HERO', None) else ('kei', 'ritz'))
    wcats = wcats or kinds_of(parts) or [5]
    os.makedirs(out, exist_ok=True)
    p = lambda s: os.path.join(out, f'{name}_{s}.png')  # noqa: E731
    look.frames_sheet(parts, p('frames'), scale=6)
    look.pose_sheet(parts, p('poses'), heroes=heroes, wcats=wcats, scale=5, size=(60, 64), anchor=(30, 50),
                    helmet_fallback=None, own_head=own)
    look.pose_sheet(parts, p('zoom'), heroes=heroes[:1], wcats=wcats[:1], picks=look.PICKS[:6], scale=8,
                    size=(52, 58), anchor=(26, 46), helmet_fallback=None, own_head=own)
    game_sheet(parts, heroes[0], wcats, p('game'), own_head=own)
    if 'helmet' in parts:
        rows = [('frames', parts['helmet'])]
        for hero in heroes:
            rows.append((hero, parts['helmet'] if own else helmguide.on_head(parts['helmet'], hero)))
            miss = helmguide.uncovered(parts['helmet'], hero)
            print(f'{hero}: uncovered hair per helmet frame', {j: len(m) for j, m in miss.items()},
                  {j: m[:8] for j, m in miss.items() if m})
        look.contact(rows, p('helmet'), scale=8)
    print({k: len(v) for k, v in parts.items()})
    print('written', out)


def zones(out):
    import bodies
    from PIL import ImageDraw
    colours = {}
    palette = [(230, 80, 80), (80, 200, 90), (80, 120, 230), (230, 200, 60), (200, 90, 220), (70, 210, 210),
               (240, 140, 50), (150, 150, 150), (120, 60, 200), (60, 160, 120), (200, 120, 160), (110, 90, 40)]

    def img(base):
        zr, tr = base
        w, h = max(len(r) for r in zr), len(zr)
        im = Image.new('RGBA', (w, h), (0, 0, 0, 0))
        for y in range(h):
            for x, k in enumerate(zr[y]):
                if k in '. ':
                    continue
                if k not in colours:
                    colours[k] = palette[len(colours) % len(palette)]
                c = colours[k]
                t = tr[y][x] if tr[y][x] is not None else 3
                f = 0.45 + 0.13 * t
                im.putpixel((x, y), tuple(min(255, int(v * f)) for v in c) + (255,))
        return im
    rows = [('legs', [img(bodies.legs()[i]) for i in range(18)]), ('arms', [img(bodies.arms()[i]) for i in range(12)])]
    path = os.path.join(out, 'zones.png')
    look.contact(rows, path, scale=8)
    legend = Image.open(path)
    d = ImageDraw.Draw(legend)
    x = 4
    for k, c in colours.items():
        d.rectangle((x, legend.height - 14, x + 10, legend.height - 4), fill=c + (255,))
        d.text((x + 13, legend.height - 15), k, fill=(255, 255, 255, 255))
        x += 30
    legend.save(path)
    print('written', path)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('set', nargs='?')
    ap.add_argument('--out', required=True)
    ap.add_argument('--hero', choices=['kei', 'ritz', 'both'])
    ap.add_argument('--weapons')
    ap.add_argument('--zones', action='store_true')
    a = ap.parse_args()
    if a.zones:
        return zones(a.out)
    heroes = None if not a.hero else (('kei', 'ritz') if a.hero == 'both' else (a.hero,))
    wcats = [int(w) for w in a.weapons.split(',')] if a.weapons else None
    preview(a.set, a.out, heroes, wcats)


if __name__ == '__main__':
    main()
