"""06 수영장 파티 케이 · 07 수영장 파티 리츠 — sunglasses, bare chest with a lei, swim trunks, toys."""
import math

import factory as F

SKIN = F.ramp('5a3031', '734041', 'a46962', 'd5898b', 'eea18b', 'f6dac5')
SKIN_OUT = F.hx('3a2026')

THEMES = {
    'kei': dict(
        trunks=F.ramp('0e2a4a', '163e6a', '1f5a92', '2f7cbe', '4aa2e0'),
        trunk_dot=F.hx('ffd23a'),
        band=F.hx('f6f2f6'),
        lei=[F.hx('ffd23a'), F.hx('ffffff'), F.hx('ff9ad0')],
        lens=F.ramp('0a1830', '18406e', '3a78b8'),
        frame=F.hx('1a3a6a'),
        sandal=F.ramp('1a2a3a', '2a6a8a', '48a0c8'),
        toy=F.ramp('6a4a00', 'b88a00', 'e8c020', 'ffe470', 'fff8d0'),
        toy2=F.ramp('0a3a6a', '1a6aaa', '3aa0e0', '9ad8ff'),
        stripe=(F.hx('2f7cbe'), F.hx('f6f2f6')),
        ring=F.ramp('062a4a', '0a4a7a', '1a7ab8', '4ab0e8', 'a8e6ff', 'ffffff'),
        bubble=F.ramp('2a2a5a', '6a5ab0', 'b08ad8', 'f0a8d8', 'ffe0f0', 'ffffff'),
    ),
    'ritz': dict(
        trunks=F.ramp('4a0a10', '7a1218', 'b0202a', 'e03a3a', 'ff7060'),
        trunk_dot=F.hx('ffffff'),
        band=F.hx('ffffff'),
        lei=[F.hx('ff4a6a'), F.hx('ffd23a'), F.hx('ffffff')],
        lens=F.ramp('080808', '202020', '4a4a4a'),
        frame=F.hx('d02020'),
        sandal=F.ramp('3a1a0a', '7a3a1a', 'c06a2a'),
        toy=F.ramp('0a2a6a', '1a4aaa', '2a7ae0', '6ab0ff', 'd0ecff'),
        toy2=F.ramp('6a1000', 'b02a00', 'f05a10', 'ffa040'),
        stripe=(F.hx('e03a3a'), F.hx('ffffff')),
        ring=F.ramp('062a4a', '0a4a7a', '1a7ab8', '4ab0e8', 'a8e6ff', 'ffffff'),
        bubble=F.ramp('2a2a5a', '6a5ab0', 'b08ad8', 'f0a8d8', 'ffe0f0', 'ffffff'),
    ),
}
HERO_HEAD = {'kei': 0, 'ritz': 1}
# eye boxes per hero head frame, (x0, x1, y0, y1) head coords — read off the frames
EYES = {
    'kei': {1: (6, 11, 13, 14), 2: (5, 8, 12, 13)},
    'ritz': {1: (6, 11, 13, 14), 2: (5, 8, 11, 12)},
}
HEAD_IN_HELM = {0: (0, (2, 7)), 1: (3, (0, 8)), 2: (2, (-1, 8)), 3: (1, (0, 6))}


def put(im, p, c):
    x, y = p
    if 0 <= x < im.width and 0 <= y < im.height:
        im.putpixel((x, y), c + (255,))


# ── sunglasses ──────────────────────────────────────────────────────────────

def helmet(hero):
    th = THEMES[hero]
    L = th['lens']
    fr = th['frame']
    out = []
    for j, (w, h) in enumerate(F.AS.FRAME_SIZES[0]):
        k, (hx, hy) = HEAD_IN_HELM[j]
        im = F.blank((w, h))
        if k == 1:  # front: two lenses and a bridge
            x0, x1, y0, _y1 = EYES[hero][1]
            ex0, ex1 = x0 - 1, x1 + 1
            for x in range(ex0, ex1 + 1):
                put(im, (x + hx, y0 + hy), fr if x in (ex0, ex1) or ex0 + 3 < x < ex1 - 3 else L[1])
            for lx in (ex0 + 1, ex1 - 3):
                for dx in range(3):
                    put(im, (lx + dx + hx, y0 + 1 + hy), L[0])
                put(im, (lx + hx, y0 + hy), L[2])
        elif k == 2:  # side: one lens and the temple to the ear
            x0, x1, y0, _y1 = EYES[hero][2]
            for x in range(x0, x1 + 1):
                put(im, (x + hx, y0 + hy), L[1])
                put(im, (x + hx, y0 + 1 + hy), L[0])
            put(im, (x0 + hx, y0 + hy), L[2])
            for x in range(x1 + 1, x1 + 6):
                put(im, (x + hx, y0 + hy), fr)
        elif k == 3:  # three-quarter: like the front, shifted
            x0, x1, y0, _y1 = EYES[hero][1]
            for x in range(x0 - 1, x1 + 1):
                put(im, (x + hx, y0 + hy), L[1] if x not in (x0 + 2, x0 + 3) else fr)
                if x not in (x0 + 2, x0 + 3):
                    put(im, (x + hx, y0 + 1 + hy), L[0])
        else:  # back: the temples' tips over the ears
            for x in (1, 16):
                put(im, (x + hx, 13 + hy), fr)
                put(im, (x + hx, 14 + hy), fr)
        out.append(im)
    return out


# ── bare chest with a lei ───────────────────────────────────────────────────

CHEST = {
    0: ["oooooooo",
        "osLLssso",
        "oLsssLso",
        "ossssdso",
        "osdssdso",
        "ossssso",
        "oooooooo"],
    1: ["oooooooo",
        "oLssssso",
        "olelelo.",
        "ossddsso",
        "osssssso",
        "ossddsso",
        "oooooooo"],
    2: ["ooooooo",
        "oLsssso",
        "oelelso",
        "osssdso",
        "ossssso",
        "osssdso",
        "ooooooo"],
}


def armor(hero):
    th = THEMES[hero]
    tpl = F.template('armor', 0)
    out = []
    for i, src in enumerate(tpl):
        im = F.blank(src.size)
        sil = F.mask_of(src)
        lei_i = 0
        for dy, row in enumerate(CHEST[i]):
            for dx, ch in enumerate(row):
                p = (2 + dx, 2 + dy)
                if p not in sil:
                    continue
                if ch == 'o':
                    c = SKIN_OUT
                elif ch == 'L':
                    c = SKIN[4]
                elif ch == 'd':
                    c = SKIN[2]
                elif ch in 'le':
                    c = th['lei'][lei_i % len(th['lei'])] if ch == 'l' else th['lei'][(lei_i + 1) % len(th['lei'])]
                    lei_i += 1
                else:
                    c = SKIN[3]
                put(im, p, c)
        for p in sil:
            if im.getpixel(p)[3] == 0:
                put(im, p, SKIN_OUT)
        out.append(im)
    return out


# ── bare arms ───────────────────────────────────────────────────────────────

def gloves(hero, s=0):
    t = F.template('gloves', s)
    cl = F.classifier(t)

    def classify(c):
        return 'outline' if cl(c) == 'outline' else 'skin'
    return F.remap_eq(t, classify, {'skin': SKIN[1:], 'outline': [SKIN_OUT]})


# ── swim trunks, bare legs, sandals ─────────────────────────────────────────

TRUNK_T = 0.34


def shoes(hero, s=7, trunk_t=None):
    th = THEMES[hero]
    t = F.template('shoes', s)
    out = []
    for src in t:
        im = F.blank(src.size)
        mask = F.mask_of(src)
        top = min(y for _, y in mask)
        seeds = [p for p in mask if p[1] <= top + 1]
        dist = F.geodesic(mask, seeds)
        maxd = max(dist.values()) if dist else 1
        e = F.edge(mask)
        lums = {(x, y): F.lum(c) for x, y, c in F.pixels(src)}
        for (x, y) in mask:
            d = dist.get((x, y), maxd)
            t_ = d / maxd if maxd else 0
            L = lums[(x, y)]
            if t_ <= (trunk_t if trunk_t is not None else TRUNK_T):  # trunks
                ramp = th['trunks']
                if y <= top:
                    c = th['band']
                elif (x, y) in e:
                    c = ramp[0]
                else:
                    c = ramp[min(len(ramp) - 1, 1 + int(L / 255 * (len(ramp) - 1)))]
                    if hero == 'kei' and (x + 2 * y) % 5 == 0 and (x, y) not in e:
                        c = th['trunk_dot']
                    if hero == 'ritz' and (x, y) not in e and t_ > 0.3:
                        c = th['stripe'][1] if (x + y) % 4 == 0 else c
            elif t_ >= 0.9:  # sandals
                ramp = th['sandal']
                c = ramp[0] if (x, y) in e else ramp[1 + (int(L) % 2)]
            else:  # bare legs
                c = SKIN_OUT if (x, y) in e else SKIN[min(len(SKIN) - 1, 2 + int(L / 255 * 4))]
            put(im, (x, y), c)
        out.append(im)
    return out


# ── toys ────────────────────────────────────────────────────────────────────

def parasol(hero, s=2):
    th = THEMES[hero]
    t = F.template('weapon_spear', s)
    cl = F.classifier(t)
    out = []
    for src in t:
        im = F.blank(src.size)
        for x, y, c in F.pixels(src):
            k = cl(c)
            if k == 'main':  # the head: a closed, striped canopy
                col = th['stripe'][((x + y) // 2) % 2]
            elif F.lum(c) < 18:
                col = F.hx('3a3a44')
            else:  # the shaft: a white pole
                col = F.hx('f4f4f4') if F.lum(c) > 45 else F.hx('b8b8c4')
            put(im, (x, y), col)
        out.append(im)
    return out


def balloon(part, hero, s, ramp_key='toy', accent_key='toy2', keep=()):
    th = THEMES[hero]
    t = F.template(part, s)
    cl = F.classifier(t)

    def classify(c):
        k = cl(c)
        return {'skin': 'main', 'outline': 'outline'}.get(k, k)
    out = F.remap_eq(t, classify, {'main': th[ramp_key], 'accent': th[accent_key], 'outline': [F.hx('202028')]}, bias={'main': 0.2})
    if keep:
        out = [t[i] if i in keep else f for i, f in enumerate(out)]
    return out


def circle(hero, kind, s=0):
    th = THEMES[hero]
    t = F.template('weapon_' + kind, s)
    r = th['ring'] if kind == 'dark' else th['bubble']
    return F.recolor_eq(t, r, r)


def beachball(hero, s=3):
    t = F.template('shield', s)
    src = t[0]
    im = F.blank(src.size)
    mask = F.mask_of(src)
    xs = [p[0] for p in mask]
    ys = [p[1] for p in mask]
    cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
    cols = [F.hx('e03a3a'), F.hx('ffffff'), F.hx('2f7cbe'), F.hx('ffd23a')] if hero == 'ritz' else \
           [F.hx('2f7cbe'), F.hx('ffffff'), F.hx('ffd23a'), F.hx('3ac06a')]
    e = F.edge(mask)
    for (x, y) in mask:
        if (x, y) in e:
            put(im, (x, y), F.hx('3a3a44'))
            continue
        a = math.atan2(y - cy, (x - cx) * 0.8)
        c = cols[int(((a + math.pi) / (2 * math.pi)) * 6) % 4]
        if abs(x - cx) <= 1 and abs(y - cy) <= 0.5:
            c = F.hx('ffffff')
        put(im, (x, y), c)
    # a highlight
    put(im, (int(cx) - 2, int(cy) - 2), F.hx('ffffff'))
    return [im]


WEAPONS = {
    'kei': ['spear', 'sword', 'knife', 'dark', 'holy'],
    'ritz': ['spear', 'sword', 'knife', 'gun', 'rifle'],
}


def build(hero):
    parts = {'helmet': helmet(hero), 'armor': armor(hero), 'gloves': gloves(hero), 'shoes': shoes(hero), 'shield': beachball(hero)}
    for kind in WEAPONS[hero]:
        if kind == 'spear':
            parts['weapon_spear'] = parasol(hero)
        elif kind == 'sword':
            parts['weapon_sword'] = balloon('weapon_sword', hero, 0)
        elif kind == 'knife':
            parts['weapon_knife'] = balloon('weapon_knife', hero, 8, 'toy2', 'toy')
        elif kind == 'gun':
            parts['weapon_gun'] = balloon('weapon_gun', hero, 0, 'toy2', 'toy')
        elif kind == 'rifle':
            parts['weapon_rifle'] = balloon('weapon_rifle', hero, 0, 'toy', 'toy2', keep=(4, 5, 6))
        elif kind in ('dark', 'holy'):
            parts['weapon_' + kind] = circle(hero, kind)
    return parts
