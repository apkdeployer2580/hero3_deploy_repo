"""Magic circles for the dark/holy stones (the 66x45 frame drawn at the caster's feet).

The shipped circles (h509NN / h510NN) are all built the same way, and so is this:

  * a silhouette (an ellipse, plus any extra filled shape — a star's points, four lobes …) with a
    near-black outline and a dark fill that brightens toward the rim and sinks toward the centre;
  * one-pixel lines ('line', or 'bright' for the main strokes) with a one-pixel 'halo' around them.

The game draws the sprite translucent, so the dark fill becomes a shadow on the ground and the lines
glow. A design is a list of primitives in a unit disk seen from above, squashed to the ellipse:

    ('ring', r, w)                  circle of radius r (0..1)
    ('poly', n, step, r, rot, w)    star polygon: n points joined every `step` (6,2 = hexagram)
    ('ticks', n, r0, r1, rot, w)    n radial strokes from r0 to r1
    ('arc', r, a0, a1, w)           an arc, degrees (0 = right, 90 = down)
    ('seg', u0, v0, u1, v1, w)      a straight stroke in unit-disk coordinates
    ('dots', n, r, rot, size)       n small discs on radius r (n=1, r=0 = a centre dot)
    ('blob', n, r, rot)             adds a filled regular n-gon (radius r, may exceed 1) to the silhouette
    ('lobe', u, v, r)               adds a filled small ellipse to the silhouette (and nothing else)
    ('stamp', block, keymap)        a small pixel picture at the centre ('.' = keep)

Stroke weight w: 1 = a line (pal 'line'), 2 = a bright line ('bright'), 3 = bold (2 px, 'core' centre).
`render(prims, pal)` -> RGBA 66x45, pal = dict(out, g0, g1, g2, g3, halo, line, bright, core) as hex
(g0 darkest/centre .. g3 rim). `PALETTES` has ready ramps; `ramp(base)` builds one from a colour.
"""
import colorsys
import math

from PIL import Image

W, H = 66, 45
CX, CY = 32.5, 22.0
RX, RY = 30.0, 19.5

PALETTES = {
    # sampled from the shipped sheets (gold = h509xx, violet = h510xx, jade = h51053)
    'gold':   dict(out='000000', g0='101008', g1='202010', g2='312818', g3='4a4029', halo='5a5931',
                   line='8b794a', bright='cdb26a', core='eed27b'),
    'violet': dict(out='000000', g0='100818', g1='201020', g2='292039', g3='39284a', halo='4a305a',
                   line='735094', bright='ac79e6', core='d8a8ff'),
    'jade':   dict(out='000000', g0='100810', g1='101418', g2='202831', g3='31484a', halo='3a5a56',
                   line='4a796a', bright='7bb694', core='bdeec5'),
}


def ramp(base, sat=1.0):
    """A circle palette around one hue (`base` = the bright line colour, hex)."""
    r, g, b = (int(base[i:i + 2], 16) / 255 for i in (0, 2, 4))
    h, l, s = colorsys.rgb_to_hls(r, g, b)

    def c(light, ss):
        rr, gg, bb = colorsys.hls_to_rgb(h, light, min(1.0, ss * sat))
        return '%02x%02x%02x' % (round(rr * 255), round(gg * 255), round(bb * 255))
    return dict(out='000000', g0=c(0.04, 0.45), g1=c(0.08, 0.40), g2=c(0.13, 0.38), g3=c(0.19, 0.36),
                halo=c(0.25, 0.40), line=c(0.40, 0.42), bright=c(0.62, 0.62), core=c(0.84, 0.85))


def _hx(s):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))


def _to_px(u, v):
    return CX + u * RX, CY + v * RY


def _line(a, b):
    n = max(2, int(math.hypot(b[0] - a[0], b[1] - a[1]) * 3))
    return [(a[0] + (b[0] - a[0]) * i / n, a[1] + (b[1] - a[1]) * i / n) for i in range(n + 1)]


def _paths(prims):
    """Each stroke as (points in pixel space, weight), dense enough to plot pixel by pixel."""
    for p in prims:
        kind = p[0]
        if kind in ('ring', 'arc'):
            if kind == 'ring':
                _, r, w = p
                d0, d1 = 0.0, 360.0
            else:
                _, r, d0, d1, w = p
            n = max(16, int(abs(d1 - d0) * max(r, 0.05) * 1.6))
            yield [_to_px(r * math.cos(math.radians(d0 + (d1 - d0) * i / n)),
                          r * math.sin(math.radians(d0 + (d1 - d0) * i / n))) for i in range(n + 1)], w
        elif kind == 'poly':
            _, n, step, r, rot, w = p
            pts = [(r * math.cos(math.radians(rot) + 2 * math.pi * i / n),
                    r * math.sin(math.radians(rot) + 2 * math.pi * i / n)) for i in range(n)]
            seen = set()
            for i in range(n):
                j = (i + step) % n
                key = tuple(sorted((i, j)))
                if key in seen:
                    continue
                seen.add(key)
                yield _line(_to_px(*pts[i]), _to_px(*pts[j])), w
        elif kind == 'ticks':
            _, n, r0, r1, rot, w = p
            for i in range(n):
                a = math.radians(rot) + 2 * math.pi * i / n
                yield _line(_to_px(r0 * math.cos(a), r0 * math.sin(a)),
                            _to_px(r1 * math.cos(a), r1 * math.sin(a))), w
        elif kind == 'seg':
            _, u0, v0, u1, v1, w = p
            yield _line(_to_px(u0, v0), _to_px(u1, v1)), w


def _pixel_perfect(points):
    """The pixels a stroke crosses, one per step, with L-shaped corner pixels removed."""
    px = []
    for x, y in points:
        q = (math.floor(x), math.floor(y))
        if not px or px[-1] != q:
            px.append(q)
    out = []
    for q in px:
        if len(out) >= 2:
            a, b = out[-2], out[-1]
            if (abs(q[0] - a[0]) == 1 and abs(q[1] - a[1]) == 1
                    and b[0] in (a[0], q[0]) and b[1] in (a[1], q[1])):
                out.pop()
        out.append(q)
    return out


def _poly_contains(pts, x, y):
    inside = False
    n = len(pts)
    for i in range(n):
        x0, y0 = pts[i]
        x1, y1 = pts[(i + 1) % n]
        if (y0 > y) != (y1 > y):
            if x < x0 + (y - y0) * (x1 - x0) / (y1 - y0):
                inside = not inside
    return inside


def silhouette(prims):
    """Which pixels the circle covers: the ellipse plus every blob/lobe."""
    shapes = []
    for p in prims:
        if p[0] == 'blob':
            _, n, r, rot = p
            shapes.append(('poly', [_to_px(r * math.cos(math.radians(rot) + 2 * math.pi * i / n),
                                           r * math.sin(math.radians(rot) + 2 * math.pi * i / n))
                                    for i in range(n)]))
        elif p[0] == 'lobe':
            _, u, v, r = p
            shapes.append(('ell', _to_px(u, v), r * RX, r * RY))
    mask = [[False] * W for _ in range(H)]
    for y in range(H):
        for x in range(W):
            px, py = x + 0.5, y + 0.5
            ins = ((px - CX) / (RX + 1.5)) ** 2 + ((py - CY) / (RY + 1.5)) ** 2 <= 1.0
            for s in shapes:
                if ins:
                    break
                if s[0] == 'poly':
                    ins = _poly_contains(s[1], px, py)
                else:
                    (ex, ey), rx, ry = s[1], s[2], s[3]
                    ins = ((px - ex) / (rx + 0.5)) ** 2 + ((py - ey) / (ry + 0.5)) ** 2 <= 1.0
            mask[y][x] = ins
    return mask


def strokes(prims):
    """{(x, y): level} — 1 = a line, 2 = a bright line, 3 = core (bold strokes, dot centres)."""
    lv = {}

    def put(q, v):
        if 0 <= q[0] < W and 0 <= q[1] < H and lv.get(q, 0) < v:
            lv[q] = v
    for pts, w in _paths(prims):
        path = _pixel_perfect(pts)
        for q in path:
            put(q, 2 if w >= 2 else 1)
        if w >= 3:
            for i, q in enumerate(path):
                a, b = path[max(0, i - 1)], path[min(len(path) - 1, i + 1)]
                dx, dy = b[0] - a[0], b[1] - a[1]
                side = (0, 1) if abs(dx) >= abs(dy) else (1, 0)
                put((q[0] + side[0], q[1] + side[1]), 2)
                put(q, 3)
    for p in prims:
        if p[0] == 'dots':
            _, n, r, rot, size = p
            for i in range(n):
                a = math.radians(rot) + 2 * math.pi * i / n
                x, y = _to_px(r * math.cos(a), r * math.sin(a))
                rad = size / 2
                for yy in range(int(y - rad - 1), int(y + rad + 2)):
                    for xx in range(int(x - rad - 1), int(x + rad + 2)):
                        d = math.hypot(xx + 0.5 - x, yy + 0.5 - y)
                        if d <= rad:
                            put((xx, yy), 3 if d <= rad - 0.9 else 2)
    return lv


N4 = ((1, 0), (-1, 0), (0, 1), (0, -1))


def render(prims, pal):
    c = {k: _hx(v) for k, v in pal.items()}
    mask = silhouette(prims)
    lv = strokes(prims)
    img = Image.new('RGBA', (W, H), (0, 0, 0, 0))
    px = img.load()

    def outside(x, y):
        return not (0 <= x < W and 0 <= y < H) or not mask[y][x]
    for y in range(H):
        for x in range(W):
            if not mask[y][x]:
                continue
            v = lv.get((x, y), 0)
            if any(outside(x + dx, y + dy) for dx, dy in N4):
                col = c['out']
            elif v == 3:
                col = c['core']
            elif v == 2:
                col = c['bright']
            elif v == 1:
                col = c['line']
            elif any(lv.get((x + dx, y + dy), 0) for dx, dy in N4):
                col = c['halo']
            elif any(outside(x + 2 * dx, y + 2 * dy) for dx, dy in N4):
                col = c['g0']
            else:
                rho = math.hypot((x + 0.5 - CX) / RX, (y + 0.5 - CY) / RY)
                col = c['g3'] if rho > 0.88 else c['g2'] if rho > 0.6 else c['g1'] if rho > 0.3 else c['g0']
            px[x, y] = col + (255,)
    for p in prims:
        if p[0] == 'stamp':
            _, block, keymap = p
            rows = block.strip('\n').split('\n')
            bh, bw = len(rows), max(len(r) for r in rows)
            ox, oy = round(CX - bw / 2), round(CY - bh / 2)
            for yy, r in enumerate(rows):
                for xx, ch in enumerate(r):
                    if ch in keymap:
                        px[ox + xx, oy + yy] = _hx(keymap[ch]) + (255,)
    return img
