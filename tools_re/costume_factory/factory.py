"""Costume factory: the helpers the 168차 set scripts build their frames with (PIL RGBA images)."""
import colorsys
import os
import sys
from collections import deque

from PIL import Image

import bmview as V

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import avatar_sheets as AS  # noqa: E402

GROUP_OF = {p: g for g, (_, p) in AS.GROUPS.items()}


# ── colour helpers ───────────────────────────────────────────────────────────

def hx(s):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))


def ramp(*hexes):
    return [hx(h) for h in hexes]


def q565(rgb):
    return AS.to_rgb(AS.to_565(rgb))


def lum(rgb):
    r, g, b = rgb[:3]
    return 0.299 * r + 0.587 * g + 0.114 * b


def hsv(rgb):
    return colorsys.rgb_to_hsv(*(v / 255 for v in rgb[:3]))


def is_skin(rgb):
    h, s, v = hsv(rgb)
    deg = h * 360
    return (deg <= 40 or deg >= 340) and 0.12 <= s <= 0.62 and v >= 0.40


def pick(rmp, t):
    t = min(1.0, max(0.0, t))
    return rmp[min(len(rmp) - 1, int(t * len(rmp)))]


# ── frames ──────────────────────────────────────────────────────────────────

def template(part, sprite):
    """Shipped sheet frames as RGBA images (`part` = avatar_sheets part name)."""
    g = GROUP_OF[part]
    return V.frames(AS.shipped_member(g, sprite))


def npc(name):
    return V.frames(f'res/npc/{name}_bm')


def blank(size):
    return Image.new('RGBA', size, (0, 0, 0, 0))


def opaque(im, x, y):
    return 0 <= x < im.width and 0 <= y < im.height and im.getpixel((x, y))[3] > 0


def pixels(im):
    for y in range(im.height):
        for x in range(im.width):
            p = im.getpixel((x, y))
            if p[3]:
                yield x, y, p[:3]


def paste_over(dst, src, ox, oy):
    for x, y, c in pixels(src):
        X, Y = x + ox, y + oy
        if 0 <= X < dst.width and 0 <= Y < dst.height:
            dst.putpixel((X, Y), c + (255,))


def lum_range(frames, keep=lambda c: True):
    ls = [lum(c) for im in frames for _, _, c in pixels(im) if keep(c)]
    return (min(ls), max(ls)) if ls else (0, 255)


def remap(frames, classify, ramps, per_frame=False):
    """Recolour every pixel by its class's ramp, by luminance within that class (sheet-wide)."""
    ranges = {}
    if not per_frame:
        for k in ramps:
            ranges[k] = lum_range(frames, lambda c, k=k: classify(c) == k)
    out = []
    for im in frames:
        if per_frame:
            ranges = {k: lum_range([im], lambda c, k=k: classify(c) == k) for k in ramps}
        o = blank(im.size)
        for x, y, c in pixels(im):
            k = classify(c)
            if k is None or k not in ramps:
                o.putpixel((x, y), c + (255,))
                continue
            lo, hi = ranges[k]
            t = 0.5 if hi - lo < 1e-6 else (lum(c) - lo) / (hi - lo)
            o.putpixel((x, y), ramps[k][min(len(ramps[k]) - 1, int(t * len(ramps[k])))] + (255,))
        out.append(o)
    return out


# ── geometry helpers ────────────────────────────────────────────────────────

def mask_of(im):
    return {(x, y) for x, y, _ in pixels(im)}


def edge(mask):
    return {(x, y) for (x, y) in mask if any((x + dx, y + dy) not in mask for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))}


def geodesic(mask, seeds):
    dist = {s: 0 for s in seeds if s in mask}
    q = deque(dist)
    while q:
        x, y = q.popleft()
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (x + dx, y + dy)
            if n in mask and n not in dist:
                dist[n] = dist[(x, y)] + 1
                q.append(n)
    return dist


# ── PNG out ─────────────────────────────────────────────────────────────────

def save_frames(folder, part, frames):
    g = GROUP_OF[part]
    sizes = AS.FRAME_SIZES[g]
    if len(frames) != len(sizes):
        raise SystemExit(f'{part}: {len(frames)} frames, want {len(sizes)}')
    os.makedirs(folder, exist_ok=True)
    for i, (im, want) in enumerate(zip(frames, sizes)):
        if im.size != want:
            raise SystemExit(f'{part}_{i}: {im.size} != {want}')
        pal = [(0, 0, 0)]
        idx = {}
        ind = []
        for y in range(im.height):
            for x in range(im.width):
                p = im.getpixel((x, y))
                if p[3] < 128:
                    ind.append(0)
                    continue
                c = q565(p[:3])
                if c not in idx:
                    idx[c] = len(pal)
                    pal.append(c)
                ind.append(idx[c])
        if len(pal) > 256:
            raise SystemExit(f'{part}_{i}: {len(pal) - 1} colours')
        AS.write_png(os.path.join(folder, f'{part}_{i}.png'), im.width, im.height, ind, pal)


def write_set(folder, parts, costume_txt):
    os.makedirs(folder, exist_ok=True)
    for f in os.listdir(folder):
        if f.endswith('.png'):
            os.remove(os.path.join(folder, f))
    for part, frames in parts.items():
        save_frames(folder, part, frames)
    with open(os.path.join(folder, 'costume.txt'), 'w', encoding='utf-8', newline='\n') as f:
        f.write(costume_txt)


def hue_dist(a, b):
    d = abs(a - b) % 360
    return min(d, 360 - d)


def dominant_hue(frames, exclude=is_skin):
    from collections import Counter
    buckets = Counter()
    for im in frames:
        for _, _, c in pixels(im):
            if exclude(c):
                continue
            h, s, v = hsv(c)
            if s >= 0.2 and v >= 0.16:
                buckets[int(h * 12) % 12] += 1
    return (buckets.most_common(1)[0][0] * 30 + 15) if buckets else None


def recolor_hue(frames, main, accent=None, skin=None, outline=None, dark_v=0.16, spread=45, dom=None):
    """main/accent by the sheet's dominant hue family; skin by `is_skin`; near-black -> outline."""
    dom = dominant_hue(frames) if dom is None else dom

    def classify(c):
        h, s, v = hsv(c)
        if v < dark_v:
            return 'outline'
        if is_skin(c):
            return 'skin'
        if s < 0.2 or dom is None or hue_dist(h * 360, dom) <= spread:
            return 'main'
        return 'accent'

    ramps = {'main': main}
    if accent is not None:
        ramps['accent'] = accent
    else:
        ramps['accent'] = main
    if skin is not None:
        ramps['skin'] = skin
    out = remap(frames, classify, ramps)
    if outline is not None:
        res = []
        for im, o in zip(frames, out):
            for x, y, c in pixels(im):
                if classify(c) == 'outline':
                    o.putpixel((x, y), outline + (255,))
            res.append(o)
        out = res
    return out


# ── head ↔ helmet placement, from every cif record that draws both ───────────

def head_helmet_variants(min_count=4):
    """[(head frame k, helmet frame j, flip, (ox, oy), count)] — helmet centre minus head centre."""
    from collections import Counter
    import cif as C
    cnt = Counter()
    for i in range(12):
        r = C.parse(V.jar().read(f'res/hero/h{i}_cif'))
        for dl in r['anims']:
            for fr in dl:
                for rec in fr:
                    hs = [e for e in rec if e['layer'] == 0]
                    ms = [e for e in rec if e['layer'] == 6]
                    for h in hs:
                        for m in ms:
                            if (h['flags'] | m['flags']) & 0xc0:
                                continue  # rotated / v-flipped: rare and not worth covering
                            if (h['flags'] & 0x20) != (m['flags'] & 0x20):
                                continue
                            cnt[(h['frame'], m['frame'], bool(h['flags'] & 0x20), (m['dx'] - h['dx'], m['dy'] - h['dy']))] += 1
    return [(k, j, fl, o, c) for (k, j, fl, o), c in cnt.items() if c >= min_count]


def head_pixels_in_helmet(j, heads, min_count=4):
    """Every helmet-frame pixel (helmet j's own unflipped coords) that a hero head covers in some
    common pose — `heads` = list of per-hero head frame lists. The union is what a full-coverage
    helmet must paint over so no hair shows round it."""
    wm, hm = AS.FRAME_SIZES[0][j]
    out = set()
    for k, jj, flip, (ox, oy), _c in head_helmet_variants(min_count):
        if jj != j:
            continue
        for hh in heads:
            im = hh[k]
            wh, hh_ = im.size
            htl = (-(wh // 2), -(hh_ // 2))
            mtl = (ox - wm // 2, oy - hm // 2)
            for x, y, _ in pixels(im):
                X = htl[0] + ((wh - 1 - x) if flip else x)
                Y = htl[1] + y
                xm = X - mtl[0]
                if flip:
                    xm = wm - 1 - xm
                ym = Y - mtl[1]
                if 0 <= xm < wm and 0 <= ym < hm:
                    out.add((xm, ym))
    return out


def hero_heads():
    return [V.frames('res/hero/h00000_bm'), V.frames('res/hero/h00001_bm')]


def remap_eq(frames, classify, ramps, bias=None):
    """Like `remap` but by luminance RANK within each class (sheet-wide, pixel-weighted), so a
    dark template still spreads over the whole ramp. `bias[k]` (0..1) shifts the class lighter."""
    from collections import Counter
    bias = bias or {}
    counts = {k: Counter() for k in ramps}
    for im in frames:
        for _, _, c in pixels(im):
            k = classify(c)
            if k in counts:
                counts[k][round(lum(c), 1)] += 1
    rank = {}
    for k, cnt in counts.items():
        total = sum(cnt.values())
        acc = 0
        for L in sorted(cnt):
            mid = acc + cnt[L] / 2
            rank[(k, L)] = mid / total if total else 0.5
            acc += cnt[L]
    out = []
    for im in frames:
        o = blank(im.size)
        for x, y, c in pixels(im):
            k = classify(c)
            if k not in ramps:
                o.putpixel((x, y), c + (255,))
                continue
            t = rank[(k, round(lum(c), 1))]
            t = min(0.999, t * (1 - bias.get(k, 0)) + bias.get(k, 0))
            r = ramps[k]
            o.putpixel((x, y), r[int(t * len(r))] + (255,))
        out.append(o)
    return out


def classifier(frames, dark_v=0.16, spread=45, dom=None):
    dom = dominant_hue(frames) if dom is None else dom

    def classify(c):
        h, s, v = hsv(c)
        if v < dark_v:
            return 'outline'
        if is_skin(c):
            return 'skin'
        if s < 0.2 or dom is None or hue_dist(h * 360, dom) <= spread:
            return 'main'
        return 'accent'
    return classify


def recolor_eq(frames, main, accent=None, skin=None, outline=None, bias=None, dark_v=0.16, spread=45):
    cl = classifier(frames, dark_v, spread)
    ramps = {'main': main, 'accent': accent or main}
    if skin is not None:
        ramps['skin'] = skin
    if outline is not None:
        ramps['outline'] = [outline]
    return remap_eq(frames, cl, ramps, bias)


def hull_fill(points):
    """The convex hull of `points` (pixel coords), filled."""
    pts = sorted(set(points))
    if len(pts) < 3:
        return set(pts)

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    # use pixel corners so the hull encloses whole pixels
    corners = sorted({(x + dx, y + dy) for x, y in pts for dx in (0, 1) for dy in (0, 1)})
    lower, upper = [], []
    for p in corners:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(corners):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    hull = lower[:-1] + upper[:-1]
    xs = [p[0] for p in hull]
    ys = [p[1] for p in hull]
    out = set()
    for y in range(min(ys), max(ys)):
        for x in range(min(xs), max(xs)):
            cx, cy = x + 0.5, y + 0.5
            inside = True
            for i in range(len(hull)):
                a, b = hull[i], hull[(i + 1) % len(hull)]
                if (b[0] - a[0]) * (cy - a[1]) - (b[1] - a[1]) * (cx - a[0]) < 0:
                    inside = False
                    break
            if inside:
                out.add((x, y))
    return out


def thin(mask):
    """Zhang-Suen thinning of a pixel set."""
    m = set(mask)

    def nb(x, y):
        return [(x, y - 1) in m, (x + 1, y - 1) in m, (x + 1, y) in m, (x + 1, y + 1) in m,
                (x, y + 1) in m, (x - 1, y + 1) in m, (x - 1, y) in m, (x - 1, y - 1) in m]
    changed = True
    while changed:
        changed = False
        for step in (0, 1):
            kill = []
            for (x, y) in m:
                p = nb(x, y)
                b = sum(p)
                a = sum(1 for i in range(8) if not p[i] and p[(i + 1) % 8])
                if not (2 <= b <= 6 and a == 1):
                    continue
                if step == 0:
                    if p[0] and p[2] and p[4]:
                        continue
                    if p[2] and p[4] and p[6]:
                        continue
                else:
                    if p[0] and p[2] and p[6]:
                        continue
                    if p[0] and p[4] and p[6]:
                        continue
                kill.append((x, y))
            if kill:
                changed = True
                m.difference_update(kill)
    return m


def shade_shape(shape, ramp, light=(-1, -1), outline=None):
    """A soft round shading of a pixel set: edge pixels darkest, the light side brightest."""
    import math
    xs = [p[0] for p in shape]
    ys = [p[1] for p in shape]
    cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
    rx, ry = max(1, (max(xs) - min(xs)) / 2), max(1, (max(ys) - min(ys)) / 2)
    e = edge(shape)
    out = {}
    lx, ly = light
    ln = math.hypot(lx, ly)
    for (x, y) in shape:
        nx, ny = (x - cx) / rx, (y - cy) / ry
        t = 0.62 - 0.38 * (nx * lx + ny * ly) / ln - 0.15 * (nx * nx + ny * ny)
        if (x, y) in e:
            out[(x, y)] = outline if outline is not None else ramp[0]
            continue
        out[(x, y)] = ramp[max(1, min(len(ramp) - 1, int(t * len(ramp))))]
    return out
