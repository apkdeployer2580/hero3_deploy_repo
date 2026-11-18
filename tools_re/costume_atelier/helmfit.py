"""Make a 투구 costume's head smaller — 171차 (사용자: "머리가 너무큰거같은데 좀만 줄여줄수없어?").

    python tools_re/costume_atelier/helmfit.py            # how much each set's head shrinks

A hair/hat costume is drawn OVER the hero's own head, so it can never be smaller than that head
(every hair pixel must stay hidden — `helmguide`), and the 169차 drawings went past it on every
side: the shipped game's helmets are ~216 px and 17 wide, ours 250..400 px and the full 21, so a
costumed hero read as big-headed next to everyone else. Two ways out, chosen per set:

* **`head = own`** in the set's TEXT (costume.txt) — the costume carries the WHOLE head. `finish`
  composites the hero's own head under the drawing at its main placement (so the face is the
  hero's, pixel for pixel), then carves straight seams out of it: `cols` columns (one each side
  of the face) and `rows` rows above the face, each the seam whose neighbours match best — the
  head shrinks around a face that does not move, and spikes, horns and brims keep their shapes.
  The game draws it in place of the hero's head: the painter hook `handle_head_paint_lgt`
  (`binary_patches/avatar.rs`) skips the head piece while such a costume is in the 투구 layer.
  A set for both heroes can do this only when its drawing hides both heads completely (a mask —
  there is one sheet, and the two faces differ); `problems` says so otherwise.
* otherwise (a costume that keeps the hero's own head under it — 이안, whose face shows for both
  heroes) **`tighten`**: the pixels just inside the outline, wherever it reaches beyond the head,
  are removed so the silhouette moves in by `keep` of its overhang; nothing over the head moves,
  so coverage holds by construction (and is still checked). Sunglasses-only helmets
  (`HELMET_COVERS_HAIR = False`) are left alone.

Per set: `HELMET_SHRINK = (cols, rows)` for an own-head set, `HELMET_FIT = (keep_side, keep_top,
margin)` for the other kind, `None` to opt out.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import helmguide  # noqa: E402
from PIL import Image  # noqa: E402

MARGIN = 0
FIT_DEFAULT = (0.5, 0.5, MARGIN)
SHRINK_DEFAULT = (2, 2)
CLEAR = (0, 0, 0, 0)


def heroes_of(mod):
    hero = getattr(mod, 'HERO', None)
    return [hero] if hero else ['kei', 'ritz']


def own_head(mod):
    """Does the set's costume.txt say `head = own`?"""
    return any(re.fullmatch(r'\s*head\s*=\s*own\s*', line.split('#', 1)[0]) for line in mod.TEXT.splitlines())


# ── the hero's head in a helmet frame ─────────────────────────────────────────────────────────

def placed_head(j, hero):
    """The hero's head frame under helmet frame j, at the main placement (`helmguide.PLACE`)."""
    k, offs = helmguide.PLACE[j]
    ox, oy = offs[0]
    wm, hm = helmguide.HELM[j]
    im = helmguide.head(hero)[k]
    canvas = Image.new('RGBA', (wm, hm), CLEAR)
    canvas.alpha_composite(im, (wm // 2 - im.width // 2 - ox, hm // 2 - im.height // 2 - oy))
    return canvas


def face_px(j, heroes):
    return {(x, y) for h in heroes for y, row in enumerate(helmguide.guide(h)[j]) for x, c in enumerate(row) if c == 'f'}


def head_px(j, heroes, chars='hf'):
    return {(x, y) for h in heroes for y, row in enumerate(helmguide.guide(h)[j]) for x, c in enumerate(row) if c in chars}


# ── own head: composite, then carve ───────────────────────────────────────────────────────────

def _d(a, b):
    if (a[3] > 0) != (b[3] > 0):
        return 600
    if not a[3]:
        return 0
    return abs(a[0] - b[0]) + abs(a[1] - b[1]) + abs(a[2] - b[2])


def carve(im, face, cols, rows):
    """Remove `rows` rows above the face (what is above moves down; the chin stays) and `cols`
    columns, alternately left and right of the face (each side moves toward it)."""
    w, h = im.size
    g = [[im.getpixel((x, y)) for x in range(w)] for y in range(h)]
    face = set(face)

    def box():
        op = [(x, y) for y in range(h) for x in range(w) if g[y][x][3]]
        return min(x for x, _ in op), min(y for _, y in op), max(x for x, _ in op), max(y for _, y in op)

    for _ in range(rows):
        bx0, by0, bx1, by1 = box()
        top = min((y for _, y in face), default=by0 + (by1 - by0) * 6 // 10)
        cand = range(by0 + 1, top - 1)
        if not cand:
            break
        y = min(cand, key=lambda y: (sum(_d(g[y - 1][x], g[y + 1][x]) for x in range(w)), -y))
        g = [[CLEAR] * w] + g[:y] + g[y + 1:]
        face = {(fx, fy + 1 if fy < y else fy) for fx, fy in face}
    for i in range(cols):
        bx0, by0, bx1, by1 = box()
        if face:
            fx0, fx1 = min(x for x, _ in face), max(x for x, _ in face)
        else:
            fx0 = fx1 = (bx0 + bx1) // 2
        left = i % 2 == 0
        cand = range(bx0 + 1, fx0) if left else range(fx1 + 1, bx1)
        if not cand:
            continue
        x = min(cand, key=lambda x: (sum(_d(g[y][x - 1], g[y][x + 1]) for y in range(h)), abs(x - (fx0 + fx1) / 2)))
        if left:
            g = [[CLEAR] + r[:x] + r[x + 1:] for r in g]
            face = {(fx + 1 if fx < x else fx, fy) for fx, fy in face}
        else:
            g = [r[:x] + r[x + 1:] + [CLEAR] for r in g]
            face = {(fx - 1 if fx > x else fx, fy) for fx, fy in face}
    out = Image.new('RGBA', (w, h), CLEAR)
    for y in range(h):
        for x in range(w):
            out.putpixel((x, y), g[y][x])
    return out


def own_frame(im, j, heroes, cols, rows):
    if len(heroes) == 1:
        c = placed_head(j, heroes[0])
        c.alpha_composite(im)
    else:
        c = im.copy()        # a mask: it hides both heads completely (`problems` checks)
    return carve(c, face_px(j, heroes), cols, rows)


# ── keeps the head: pull the overhang in ──────────────────────────────────────────────────────

def bounds(j, heroes):
    """(left, right, top) of the hero heads under helmet frame j, both placements."""
    pts = head_px(j, heroes, 'hfo,')
    return min(x for x, _ in pts), max(x for x, _ in pts), min(y for _, y in pts)


def _keep(over, keep):
    """How much of an `over`-px overhang stays (half rounds up, so a 1 px overhang stays)."""
    return int(over * keep + 0.5)


def _pull(px, line, keep):
    """One row (list of RGBA, read from its outer edge inward): the pixels from the edge to `line`
    shrink to `keep` of their length. The pixels removed are taken from the MIDDLE of that stretch,
    so the outline and the tones next to it (a highlight, a brim) stay, and a longer spike stays
    longer than a short one."""
    op = [i for i, p in enumerate(px) if p[3]]
    if not op or op[0] >= line:
        return px
    e = op[0]
    over = line - e
    d = over - _keep(over, keep)
    if d <= 0:
        return px
    s = e + 1 + (over - 1 - d) // 2
    return [CLEAR] * d + px[:s] + px[s + d:]


def tighten(im, j, heroes, keep_side, keep_top, margin=MARGIN):
    left, right, head_top = bounds(j, heroes)
    margin = int(margin)
    a, b, t = left - margin, right + margin, head_top - margin
    w, h = im.size
    g = [[im.getpixel((x, y)) for x in range(w)] for y in range(h)]
    if keep_top < 1:
        for x in range(w):
            col = _pull([g[y][x] for y in range(h)], t, keep_top)
            for y in range(h):
                g[y][x] = col[y]
    if keep_side < 1:
        g = [_pull(_pull(row, a, keep_side)[::-1], w - 1 - b, keep_side)[::-1] for row in g]
    out = Image.new('RGBA', (w, h), CLEAR)
    for y in range(h):
        for x in range(w):
            out.putpixel((x, y), g[y][x])
    return out


# ── what the tools call ───────────────────────────────────────────────────────────────────────

def problems(mod, parts):
    """What `finish` cannot do for this set (said by the lint)."""
    bad = []
    if own_head(mod) and 'helmet' in parts and len(heroes_of(mod)) > 1:
        for j, im in enumerate(parts['helmet']):
            bare = [p for p in head_px(j, heroes_of(mod)) if im.getpixel(p)[3] < 128]
            if bare:
                bad.append(f'head = own for both heroes needs a drawing that hides both heads: j{j} leaves {len(bare)} px {bare[:4]}')
    if own_head(mod) and not getattr(mod, 'HELMET_COVERS_HAIR', True):
        bad.append('head = own with HELMET_COVERS_HAIR = False: the hero\'s hair would be gone')
    return bad


def finish(mod, parts):
    """What every tool that turns a set script into frames applies after `build()`."""
    if 'helmet' not in parts or not getattr(mod, 'HELMET_COVERS_HAIR', True):
        return parts
    parts = dict(parts)
    heroes = heroes_of(mod)
    if own_head(mod):
        shrink = getattr(mod, 'HELMET_SHRINK', SHRINK_DEFAULT)
        if shrink:
            cols, rows = shrink
            parts['helmet'] = [own_frame(im, j, heroes, cols, rows) for j, im in enumerate(parts['helmet'])]
        return parts
    fit = getattr(mod, 'HELMET_FIT', FIT_DEFAULT)
    if fit:
        side, top, *rest = fit
        margin = rest[0] if rest else MARGIN
        parts['helmet'] = [tighten(im, j, heroes, side, top, margin) for j, im in enumerate(parts['helmet'])]
    return parts


def main():
    import importlib
    import importlib.util
    sys.path.insert(0, os.path.join(HERE, 'sets'))
    spec = importlib.util.spec_from_file_location('atelier_build', os.path.join(HERE, 'build.py'))
    build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(build)
    n = lambda im: sum(1 for p in im.getdata() if p[3])  # noqa: E731
    bb = lambda im: im.getchannel('A').getbbox()  # noqa: E731
    for slug in build.SETS:
        m = importlib.import_module(slug)
        raw = m.build()['helmet']
        done = finish(m, {'helmet': list(raw)})['helmet']
        how = 'own head' if own_head(m) else ('keeps the head' if getattr(m, 'HELMET_COVERS_HAIR', True) else 'untouched')
        print(f'{m.FOLDER}: {how} ' + ' | '.join(
            f'j{j} {n(r)}->{n(d)} px {bb(r)[2]-bb(r)[0]}x{bb(r)[3]-bb(r)[1]}->{bb(d)[2]-bb(d)[0]}x{bb(d)[3]-bb(d)[1]}'
            for j, (r, d) in enumerate(zip(raw, done))))


if __name__ == '__main__':
    main()
