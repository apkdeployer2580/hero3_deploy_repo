"""Hair drawn as LOCKS — used by sets 14 (흑의 사신) and 15 (검은 정장의 킬러), 175차.

A helmet frame of hair is the hero's own head (`helmguide`, every hair pixel covered, the face left
alone) plus a tapered spike for every lock TIP that reaches past it. Every hair pixel belongs to the
lock whose direction from the CROWN (the whorl the locks grow from) is nearest to its own — so the
locks are wedges radiating from the crown, bent by `swirl` (radians per pixel of distance: locks that
sweep back instead of standing straight). Lighting follows the game's own heads (`helmguide.head`):

* no black outline — the darkest tone IS the edge (a shade lighter where the light falls on it);
* light from the top-left over the whole head;
* every lock lit on the side that faces the light and shadowed on the other, with a one-pixel shadow
  line where it meets the next lock (none near the crown, where the locks start);
* a highlight band across the crown on the lit side (`band`).

`render` returns a text block of tone digits (a `kit.hair_pal` frame); `pins` touches single pixels
afterwards ({(x, y): '0'..'5' or '.'}) — the hand finish.
"""
import math

import helmguide

LIGHT = (-0.6, -0.8)        # toward the light: up and to the left


def _tri(tip, base, w):
    """Pixels whose centres lie in the triangle tip -> base±w (the base perpendicular to the axis)."""
    tx, ty = tip
    bx, by = base
    ax, ay = bx - tx, by - ty
    n = math.hypot(ax, ay) or 1.0
    px, py = -ay / n * w, ax / n * w
    pts = [(tx, ty), (bx + px, by + py), (bx - px, by - py)]
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    out = set()

    def inside(x, y):
        s = []
        for i in range(3):
            x1, y1 = pts[i]
            x2, y2 = pts[(i + 1) % 3]
            s.append((x2 - x1) * (y - y1) - (y2 - y1) * (x - x1))
        return all(v >= -1e-9 for v in s) or all(v <= 1e-9 for v in s)
    for y in range(math.floor(min(ys)), math.ceil(max(ys)) + 1):
        for x in range(math.floor(min(xs)), math.ceil(max(xs)) + 1):
            if inside(x + 0.5, y + 0.5):
                out.add((x, y))
    return out


def _ang(dx, dy):
    return math.atan2(dy, dx)


def _wrap(a):
    while a > math.pi:
        a -= 2 * math.pi
    while a < -math.pi:
        a += 2 * math.pi
    return a


def render(hero, j, root, tips, swirl=0.0, band=(0.10, 0.34, 0.55), r0=2.2, taper=2.5, add=(), cut=(),
           pins=None, cover_face=(), keep=None, smooth=False):
    """root = (x, y) of the crown; tips = [(x, y, half-width)]; add / cut = extra pixels in or out
    of the silhouette; cover_face = face pixels the design draws over (a beard); keep = only these
    tips get a spike (None = every tip outside the head); smooth = drop the one-pixel tips a spike
    leaves outside the head (sleek hair — 15's swept-back hair, not 14's spikes)."""
    W, H = helmguide.HELM[j]
    gd = helmguide.guide(hero)[j]
    head = {(x, y) for y in range(H) for x in range(W) if gd[y][x] in 'ho'}
    face = {(x, y) for y in range(H) for x in range(W) if gd[y][x] in 'f,'} - set(cover_face)
    rx, ry = root
    hair = set(head) | set(add) | set(cover_face)
    for tx, ty, w in tips:
        ax, ay = rx - tx, ry - ty
        d = math.hypot(ax, ay) or 1.0
        # the spike is a solid wedge from where its axis enters the head out to the tip
        enter = 0.0
        while enter < d and (int(tx + ax / d * enter), int(ty + ay / d * enter)) not in head:
            enter += 0.25
        reach = min(d, enter + taper)
        base = (tx + ax / d * reach, ty + ay / d * reach)
        hair |= _tri((tx, ty), base, w)
    hair = {(x, y) for (x, y) in hair if 0 <= x < W and 0 <= y < H} - face - set(cut)
    if smooth:
        lone = {(x, y) for (x, y) in hair - head
                if sum((x + dx, y + dy) in hair for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))) <= 1}
        hair -= lone

    # the lock each pixel belongs to: the tip whose (swirled) direction from the crown is nearest
    def dirn(x, y):
        dx, dy = x + 0.5 - rx, y + 0.5 - ry
        return _ang(dx, dy) + swirl * math.hypot(dx, dy)
    tipang = [_ang(tx - rx, ty - ry) + swirl * math.hypot(tx - rx, ty - ry) for tx, ty, _ in tips]
    lock = {}
    for (x, y) in hair:
        a = dirn(x, y)
        lock[(x, y)] = min(range(len(tips)), key=lambda i: abs(_wrap(a - tipang[i])))

    xs = [p[0] for p in hair]
    ys = [p[1] for p in hair]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    lx, ly = LIGHT
    out = {}
    for (x, y) in hair:
        ny, nx = (y - y0) / max(1, y1 - y0), (x - x0) / max(1, x1 - x0)
        L = 1.10 - 0.92 * ny - 0.50 * nx
        nb = [(x + dx, y + dy) in hair for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))]
        if not all(nb):
            lit = not nb[3] or not nb[1]
            out[(x, y)] = 1 if (lit and L > 0.62 and sum(nb) >= 2) else 0
            continue
        i = lock[(x, y)]
        tx, ty, w = tips[i]
        ax, ay = tx - rx, ty - ry
        n = math.hypot(ax, ay) or 1.0
        ax, ay = ax / n, ay / n
        vx, vy = x + 0.5 - rx, y + 0.5 - ry
        along = vx * ax + vy * ay
        px, py = vx - along * ax, vy - along * ay
        side = (px * lx + py * ly)            # > 0 = on the lock's lit side
        dist = math.hypot(vx, vy)
        t = 5 if L > 1.0 else 4 if L > 0.72 else 3 if L > 0.42 else 2 if L > 0.14 else 1
        if dist > r0:
            if side > 0.6:
                t += 1
            elif side < -0.6:
                t -= 1
        if band and band[0] <= ny <= band[1] and nx <= band[2] and side > 0.9 and dist > r0:
            t = max(t, 5)
        elif t >= 5 and not (band and ny <= band[1]):
            t = 4
        # the shadow line where this lock meets the next (on the dark side of the pair)
        if dist > r0:
            for dx, dy in ((1, 0), (0, 1)):
                q = (x + dx, y + dy)
                if q in hair and lock.get(q) != i and side < 0.4:
                    t = 1
                    break
        out[(x, y)] = max(1, min(5, t))
    for (x, y), v in (pins or {}).items():
        if v == '.':
            out.pop((x, y), None)
        else:
            out[(x, y)] = int(v) if v.isdigit() else v
    rows = []
    for y in range(H):
        rows.append(''.join(str(out[(x, y)]) if (x, y) in out else '.' for x in range(W)))
    return '\n'.join(rows)
