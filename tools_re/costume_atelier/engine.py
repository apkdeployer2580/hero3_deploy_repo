"""Costume atelier engine — hand-drawn MATERIAL grids -> shaded pixel art (PIL RGBA).

Every frame is text, one character per pixel ('.' or ' ' = transparent, anything else = a key
into the set's palette of `Mat`s). Two ways to build a frame:

* `paint(rows, pal)`            a grid drawn from scratch (torso, helmet, weapon, shield)
* `dress(base, mapping, ...)`   a BODY template (legs/arms traced from the shipped sheets: zone
                                letters + the shipped form shading) with each zone mapped to a
                                material, then OVERLAY grids drawn on top (coat tails, fur, plates)

Shading is automatic (light from the top-left, a dark outline round the silhouette and round any
part that sits in front of another) and every pixel can be pinned by a TONE grid written next to
the material grid after a '|':

    tone 0 outline · 1 deep shadow · 2 shadow · 3 mid · 4 light · 5 highlight

A body template carries the shipped sheet's own form shading as tones, so trousers keep their
knees and boots their toes whatever the colours; a material with `auto=True` ignores it.
"""
from PIL import Image

N4 = ((1, 0), (-1, 0), (0, 1), (0, -1))


def hx(s):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


class Mat:
    """A material.

    ramp   six colours (hex strings, or a space separated string) for tones 0..5
           (outline, deep shadow, shadow, mid, light, highlight); fewer are padded
    z      depth: the higher part outlines its edge against a lower one and casts a shadow
           on the pixels below/right of it
    auto   ignore the body template's tones (overlays are always auto)
    flat   every pixel takes this tone (gems, eyes, glow)
    noline the outer silhouette is not outlined (fur tips, smoke, glow)
    gloss  the top-left inner corner gets the highlight
    line   the tone used for the outline (default 0)
    edge   the tone for an edge in front of a lower part (default = line)
    shift  added to every automatic tone (a darker or lighter cousin of the same ramp)
    fam    materials of one family shade as one region (a lining strip is not an outline edge)
    style  'plain' (first pixel in from the top-left lit, from the bottom-right dark) ·
           'cloth' (cylinder: lit left, falling into shadow right and down) · 'metal' (hard
           specular top-left, dark far side)
    """

    def __init__(self, ramp, z=0, auto=False, flat=None, noline=False, gloss=False, line=0, edge=None, shift=0,
                 fam=None, style='plain'):
        if isinstance(ramp, str):
            ramp = ramp.split()
        r = [hx(c) if isinstance(c, str) else tuple(c) for c in ramp]
        if len(r) == 4:   # outline, shadow, mid, light -> spread over six
            r = [r[0], r[1], mix(r[1], r[2], 0.5), r[2], r[3], mix(r[3], (255, 255, 255), 0.35)]
        elif len(r) == 5:
            r = [r[0], r[1], mix(r[1], r[2], 0.5), r[2], r[3], r[4]]
        while len(r) < 6:
            r.append(r[-1])
        self.ramp = r
        self.z = z
        self.auto = auto
        self.flat = flat
        self.noline = noline
        self.gloss = gloss
        self.line = line
        self.edge = edge
        self.shift = shift
        self.fam = fam
        self.style = style

    def like(self, **kw):
        m = Mat(self.ramp, self.z, self.auto, self.flat, self.noline, self.gloss, self.line, self.edge, self.shift,
                self.fam, self.style)
        for k, v in kw.items():
            setattr(m, k, v)
        return m


# ── text ─────────────────────────────────────────────────────────────────────

def split(block):
    """'mat rows' or 'mat | tone rows' -> (mat rows, tone rows or None). Blank edge lines dropped."""
    if not isinstance(block, str):
        return block
    lines = block.split('\n')
    while lines and lines[0].strip() == '':
        lines.pop(0)
    while lines and lines[-1].strip() == '':
        lines.pop()
    if any('|' in l for l in lines):
        mats, tones = [], []
        for l in lines:
            a, _, b = l.partition('|')
            mats.append(a.rstrip())
            tones.append(b.strip())
        return mats, tones
    return [l.rstrip() for l in lines], None


class Grid:
    """A material grid: keys[y][x] (None = transparent) + optional fixed tones[y][x]."""

    def __init__(self, w, h):
        self.w, self.h = w, h
        self.keys = [[None] * w for _ in range(h)]
        self.tones = [[None] * w for _ in range(h)]   # body-template tone (None = automatic)
        self.pins = [[None] * w for _ in range(h)]    # tone override from a '|' grid

    def get(self, x, y):
        if 0 <= x < self.w and 0 <= y < self.h:
            return self.keys[y][x]
        return None

    @staticmethod
    def of(block, size=None, name='?'):
        rows, trows = split(block)
        w = max([len(r) for r in rows] + [0])
        h = len(rows)
        if size is not None:
            if h != size[1] or w > size[0]:
                raise ValueError(f'{name}: grid {w}x{h} does not fit frame {size[0]}x{size[1]}')
            w, h = size
        g = Grid(w, h)
        for y, r in enumerate(rows):
            for x, c in enumerate(r):
                if c not in '. ':
                    g.keys[y][x] = c
        if trows is not None:
            for y, r in enumerate(trows):
                for x, c in enumerate(r[:w]):
                    if c.isdigit():
                        g.pins[y][x] = int(c)
        return g


# ── building a frame ─────────────────────────────────────────────────────────

def overlay(g, block, mode='over', name='?', dx=0, dy=0, under=frozenset()):
    """Draw a grid on g. mode 'over' = on top · 'under' = only where g is transparent; keys in
    `under` behave as 'under' whatever the mode (a coat's back panel among its front panels).
    In the overlay '_' ERASES (makes transparent) and '~' keeps what is there."""
    if block is None:
        return g
    o = Grid.of(block, name=name)
    for y in range(o.h):
        for x in range(o.w):
            c = o.keys[y][x]
            X, Y = x + dx, y + dy
            if c is None or c == '~' or not (0 <= X < g.w and 0 <= Y < g.h):
                continue
            if c == '_':
                g.keys[Y][X] = None
                g.tones[Y][X] = None
                g.pins[Y][X] = None
                continue
            if (mode == 'under' or c in under) and g.keys[Y][X] is not None:
                continue
            g.keys[Y][X] = c
            g.tones[Y][X] = None
            g.pins[Y][X] = o.pins[y][x]
    return g


def dress(base, mapping, overlays=(), pins=None, under=frozenset()):
    """base = (zone rows, tone rows) of a body template; mapping zone -> material key
    (a zone mapped to None/'' vanishes). overlays = [(block, mode)]. pins = a '|'-style tone block."""
    zones, tones = base
    h = len(zones)
    w = max(len(r) for r in zones)
    g = Grid(w, h)
    for y in range(h):
        for x, z in enumerate(zones[y]):
            if z in '. ':
                continue
            k = mapping.get(z, z)
            if not k:
                continue
            g.keys[y][x] = k
            g.tones[y][x] = tones[y][x]
    for ov in overlays:
        if isinstance(ov, tuple):
            block, mode = ov
        else:
            block, mode = ov, 'over'
        overlay(g, block, mode, under=under)
    if pins is not None:
        p = Grid.of(pins)
        for y in range(min(h, p.h)):
            for x in range(min(w, p.w)):
                if p.pins[y][x] is not None:
                    g.pins[y][x] = p.pins[y][x]
    return g


# ── shading ──────────────────────────────────────────────────────────────────

def _fam(pal, k):
    m = pal[k]
    return m.fam if m.fam is not None else k


def _run(g, pal, x, y, dx, dy, f):
    n = 0
    while True:
        k = g.get(x + dx * (n + 1), y + dy * (n + 1))
        if k is None or _fam(pal, k) != f:
            return n
        n += 1


def tone_of(g, x, y, pal):
    k = g.keys[y][x]
    m = pal[k]
    if g.pins[y][x] is not None:
        return g.pins[y][x]
    if m.flat is not None:
        return m.flat
    f = _fam(pal, k)
    nb = [g.get(x + dx, y + dy) for dx, dy in N4]
    if any(n is None for n in nb):
        return 3 if m.noline else m.line
    for n in nb:
        if _fam(pal, n) != f and pal[n].z < m.z:
            return m.edge if m.edge is not None else m.line
    shadow = False
    for n in (g.get(x, y - 1), g.get(x - 1, y)):
        if n is not None and _fam(pal, n) != f and pal[n].z > m.z:
            shadow = True
    base = g.tones[y][x]
    if base is not None and not m.auto:
        t = base + m.shift
        if shadow:
            t = min(t, 2) if t > 1 else t
        return max(1, min(5, t))
    u, d = _run(g, pal, x, y, 0, -1, f), _run(g, pal, x, y, 0, 1, f)
    l, r = _run(g, pal, x, y, -1, 0, f), _run(g, pal, x, y, 1, 0, f)
    if m.style == 'cloth':
        w = l + r + 1
        hgt = u + d + 1
        pos = l / max(1, w - 1)
        vpos = u / max(1, hgt - 1)
        v = 0.95 - 1.1 * pos - 0.45 * vpos
        if l <= 1:
            v += 0.35
        if r <= 1:
            v -= 0.2
        if u <= 1:
            v += 0.15
        if d <= 1:
            v -= 0.15
        if v > 0.95 and m.gloss:
            t = 5
        elif v > 0.55:
            t = 4
        elif v > 0.15:
            t = 3
        elif v > -0.25:
            t = 2
        else:
            t = 1
    elif m.style == 'metal':
        w = l + r + 1
        pos = l / max(1, w - 1)
        if l <= 1 and u <= 2:
            t = 5
        elif l <= 1 or u <= 1:
            t = 4
        elif r <= 1 or d <= 1:
            t = 2
        elif pos < 0.5:
            t = 3
        else:
            t = 2
    else:
        lit = u <= 1 or l <= 1
        dark = d <= 1 or r <= 1
        if lit and dark:
            t = 4 if (u + l) <= (d + r) else 2
        elif lit:
            t = 5 if (m.gloss and u <= 1 and l <= 1) else 4
        elif dark:
            t = 2
        else:
            t = 3
    if shadow:
        t = min(t, 2)
    return max(1, min(5, t + m.shift))


def shade(g, pal, name='?'):
    img = Image.new('RGBA', (g.w, g.h), (0, 0, 0, 0))
    for y in range(g.h):
        for x in range(g.w):
            k = g.keys[y][x]
            if k is None:
                continue
            if k not in pal:
                raise KeyError(f'{name}: no material {k!r} (at {x},{y})')
            t = tone_of(g, x, y, pal)
            img.putpixel((x, y), pal[k].ramp[max(0, min(5, t))] + (255,))
    return img


def paint(block, pal, size=None, name='?'):
    return shade(Grid.of(block, size, name), pal, name)


def check_size(img, size, name):
    if img.size != tuple(size):
        raise ValueError(f'{name}: {img.size} != {tuple(size)}')
    return img
