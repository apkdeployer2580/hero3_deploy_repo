"""04 할로윈-해골 가면 — a skull mask over the whole head and a black skeleton suit."""
import factory as F

BONE = F.ramp('3d3426', '6b6250', '9a917a', 'c4bda4', 'e2ddc8', 'f7f4e8')
BONE_OUT = F.hx('2a2418')
HOLE = F.hx('140c10')
GLOW = F.hx('ff8a1a')
GLOW_D = F.hx('b04a10')
BLACK = F.ramp('0e0c12', '18161e', '24222c', '34323e', '48465a')
BLACK_OUT = F.hx('08060a')
PUMPKIN = F.ramp('2a0e00', '5a2200', '9a4206', 'd46c14', 'f69a2a', 'ffd070')
GHOST = F.ramp('10141c', '1e2a30', '2e4a46', '4a7a68', '7ab894', 'bff0c8')


def put(im, p, c):
    x, y = p
    if 0 <= x < im.width and 0 <= y < im.height:
        im.putpixel((x, y), c + (255,))


# face features per helmet frame, in the frame's own coords (the shipped round helm h60012's
# visor opening is exactly where the heroes' faces are)
FACE = {
    3: dict(sockets=[(4, 17, 3), (11, 17, 3)], nose=[(8, 20), (9, 20)], mouth=(5, 12, 21)),
    2: dict(sockets=[(2, 19, 3)], nose=[(1, 22)], mouth=(1, 6, 23)),
    1: dict(sockets=[(4, 19, 3), (10, 19, 2)], nose=[(8, 22)], mouth=(4, 11, 23)),
    0: dict(sockets=[], nose=[], mouth=None),
}


def skull_frame(j, base):
    src = F.template('helmet', 12)[j]
    im = base.copy()
    sil = F.mask_of(src)
    e = F.edge(sil)
    # the visor opening (the shipped helm's near-black inside its outline) becomes bone
    for x, y, c in F.pixels(src):
        if (x, y) not in e and F.lum(c) < 40:
            put(im, (x, y), BONE[3] if y % 2 else BONE[4])
    f = FACE[j]
    for sx, sy, wd in f['sockets']:
        for dx in range(wd):
            for dy in range(3):
                if dy in (0, 2) and dx in (0, wd - 1) and wd > 2:
                    continue
                put(im, (sx + dx, sy + dy), HOLE)
        put(im, (sx + wd // 2, sy + 1), GLOW)
    for p in f['nose']:
        put(im, p, HOLE)
    if f['mouth']:
        mx0, mx1, my = f['mouth']
        for x in range(mx0, mx1 + 1):
            put(im, (x, my), HOLE)
            put(im, (x, my + 1), BONE[5] if (x - mx0) % 2 == 0 else BONE[1])
            if mx0 < x < mx1:
                put(im, (x, my + 2), HOLE)
    if j == 0:
        for p in [(12, 11), (13, 12), (13, 13), (14, 14)]:
            put(im, p, BONE[1])
    return im


def helmet():
    base = F.recolor_eq(F.template('helmet', 12), BONE, BONE, outline=BONE_OUT, bias={'main': 0.25, 'accent': 0.25})
    return [skull_frame(j, base[j]) for j in range(4)]


# ── the skeleton suit ───────────────────────────────────────────────────────
# torso patterns over the shipped base torso h10000's silhouette (bbox origin (2, 2)):
# o outline · k black · b bone · d bone shadow
TORSO = {
    0: ["oooooooo",
        "okkbbkko",
        "obbddbbo",
        "okkbbkko",
        "obbddbbo",
        "okkbbkko",
        "oooooooo"],
    1: ["oooooooo",
        "obbbbbbo",
        "okkbbkko",
        "obbbbbbo",
        "okkbbkko",
        "okbbbbko",
        "oooooooo"],
    2: ["ooooooo",
        "obbbkbo",
        "okkkkbo",
        "obbbkbo",
        "okkkkbo",
        "obbkkbo",
        "ooooooo"],
}
INK = {'o': BLACK_OUT, 'k': BLACK[2], 'b': BONE[4], 'd': BONE[2]}


def armor():
    tpl = F.template('armor', 0)
    out = []
    for i, src in enumerate(tpl):
        im = F.blank(src.size)
        sil = F.mask_of(src)
        for dy, row in enumerate(TORSO[i]):
            for dx, ch in enumerate(row):
                p = (2 + dx, 2 + dy)
                if p in sil:
                    put(im, p, INK[ch])
        for p in sil:  # anything the pattern missed
            if im.getpixel(p)[3] == 0:
                put(im, p, BLACK_OUT)
        out.append(im)
    return out


def bone_lines(frames, skip_skin=True, color=None):
    color = color or BONE[3]
    out = []
    for im in frames:
        o = im.copy()
        body = {(x, y) for x, y, c in F.pixels(im) if not (skip_skin and c == BONE[5])}
        inner = body - F.edge(body)
        sk = F.thin(inner) if len(inner) > 3 else set()
        for p in sk:
            put(o, p, color)
        out.append(o)
    return out


def gloves(s=0):
    t = F.template('gloves', s)
    cl = F.classifier(t)

    def classify(c):
        k = cl(c)
        return 'skin' if k == 'skin' else ('outline' if k == 'outline' else 'cloth')
    base = F.remap_eq(t, classify, {'cloth': BLACK, 'skin': BONE[3:], 'outline': [BLACK_OUT]})
    # skeletal hands: brightest bone for the hand's own pixels so the bone line can skip them
    res = []
    for src, im in zip(t, base):
        o = im.copy()
        for x, y, c in F.pixels(src):
            if cl(c) == 'skin':
                put(o, (x, y), BONE[5] if F.lum(c) > 120 else BONE[3])
        res.append(o)
    return bone_lines(res)


def shoes(s=0):
    t = F.template('shoes', s)
    cl = F.classifier(t)

    def classify(c):
        return 'outline' if cl(c) == 'outline' else 'cloth'
    base = F.remap_eq(t, classify, {'cloth': BLACK, 'outline': [BLACK_OUT]})
    return bone_lines(base, skip_skin=False)


def weapon(kind, s):
    part = 'weapon_' + kind
    t = F.template(part, s)
    if kind == 'dark':
        return F.recolor_eq(t, PUMPKIN, PUMPKIN)
    if kind == 'holy':
        return F.recolor_eq(t, GHOST, GHOST)
    cl = F.classifier(t)

    def classify(c):
        k = cl(c)
        return {'skin': 'main'}.get(k, k)
    out = F.remap_eq(t, classify, {'main': BONE, 'accent': BLACK, 'outline': [BONE_OUT]}, bias={'main': 0.2})
    if kind == 'rifle':
        orig = F.template(part, s)
        out = [orig[i] if i in (4, 5, 6) else f for i, f in enumerate(out)]
    return out


def shield(s=3):
    t = F.template('shield', s)
    base = F.recolor_eq(t, BONE, BONE, outline=BONE_OUT, bias={'main': 0.15})
    o = base[0].copy()
    sil = F.mask_of(t[0])
    inner = sil - F.edge(sil)
    ys = sorted({y for _, y in inner})
    for y in ys[1::3]:  # rib gaps
        for x in range(o.width):
            if (x, y) in inner:
                put(o, (x, y), BONE[1])
    return [o]


WEAPONS = {'spear': 0, 'sword': 0, 'knife': 8, 'gun': 0, 'rifle': 0, 'dark': 0, 'holy': 0}


def build():
    parts = {'helmet': helmet(), 'armor': armor(), 'gloves': gloves(), 'shoes': shoes(), 'shield': shield()}
    for kind, s in WEAPONS.items():
        parts['weapon_' + kind] = weapon(kind, s)
    return parts
