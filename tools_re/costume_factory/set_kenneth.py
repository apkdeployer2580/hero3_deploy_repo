"""05 케네스 — the masked leader of the Corvus guild, from his own NPC sprite (npc0008/1008/2008/4008)."""
import factory as F

# ── palette (sampled from the NPC sheets) ───────────────────────────────────
STEEL = F.ramp('413041', '52406a', '736183', '9489ac', 'acaac5', 'c5bad5', 'e6e2e6')
STEEL_D = F.ramp('292029', '312831', '413041', '52406a', '736183', '9489ac')
CRIMSON = F.ramp('3a1420', '5a2831', '733841', 'a4504a', 'cd615a')
OLIVE = F.ramp('312831', '393839', '525041', '7b815a', '9ca07a')
GOLD = F.ramp('5a3800', '835000', 'b48131', 'e6ba73', 'fff2c5')
BLADE = F.ramp('4a4862', '6a698b', '8b89a4', 'acaabd', 'cdcad5', 'f6f2f6')
OUTLINE = F.hx('201820')
MASK_BACK = F.hx('7b385a')
MASK_EDGE = F.hx('413041')

# helmet frame j -> (hero head frame k, hero head top-left inside the helmet frame) — from every
# cif record that draws both layers (head f1 -> helm f3 at d=(0,-3), f2 -> f2 (1,-4), f0 -> f0
# (-1,-4), f3 -> f1 (1,-4)), converted to top-left offsets.
HEAD_IN_HELM = {0: (0, (2, 7)), 1: (3, (0, 8)), 2: (2, (-1, 8)), 3: (1, (0, 6))}
# hero head frame -> (npc0008 frame, its top-left inside the hero head frame): eyes on eyes
# (front: the mask's eye row 11 on the heroes' row 14; side: the mask's eye (8,12) on Ritz's (8,12)),
# the back view bottom-aligned.
MASK_IN_HEAD = {0: (0, (0, 3)), 1: (1, (0, 3)), 2: (2, (0, 0)), 3: (3, (0, 1))}


def helmet():
    heads = F.hero_heads()
    mask = F.npc('npc0008')
    out = []
    for j, (w, h) in enumerate(F.AS.FRAME_SIZES[0]):
        k, (hx, hy) = HEAD_IN_HELM[j]
        m, (mx, my) = MASK_IN_HEAD[k]
        im = F.blank((w, h))
        cover = F.head_pixels_in_helmet(j, heads)
        mpx = {(x + hx + mx, y + hy + my): c for x, y, c in F.pixels(mask[m])}
        back = {p for p in cover if p not in mpx}
        fill = back | {p for p in mpx if 0 <= p[0] < w and 0 <= p[1] < h}
        edge = F.edge(fill)
        for p in back:
            im.putpixel(p, (MASK_EDGE if p in edge else MASK_BACK) + (255,))
        for (x, y), c in mpx.items():
            if 0 <= x < w and 0 <= y < h:
                im.putpixel((x, y), c + (255,))
        out.append(im)
    return out


STEEL_LEG = F.ramp('292029', '312831', '413041', '52406a', '736183', '9489ac', 'c5bad5')
DARK_CIRCLE = F.ramp('200810', '3a1420', '5a2831', '7b385a', 'a44862', 'cd615a', 'ffaaa4')
HOLY_CIRCLE = F.ramp('292029', '413041', '736183', '9489ac', 'c5bad5', 'e6e2e6', 'ffffff')
KEEP_SHIPPED = {'weapon_rifle': (4, 5, 6)}


def keep_shipped(part, sprite, frames):
    keep = KEEP_SHIPPED.get(part, ())
    if keep:
        orig = F.template(part, sprite)
        frames = [orig[i] if i in keep else f for i, f in enumerate(frames)]
    return frames


def armor(s=15):
    return F.recolor_eq(F.template('armor', s), STEEL, CRIMSON, outline=OUTLINE, bias={'main': 0.15})


def gloves(s=11):
    return F.recolor_eq(F.template('gloves', s), STEEL, CRIMSON, skin=CRIMSON, outline=OUTLINE, bias={'main': 0.1})


def shoes(s=9):
    return F.recolor_eq(F.template('shoes', s), STEEL_LEG, OLIVE, outline=OUTLINE)


def weapon(kind, s):
    part = 'weapon_' + kind
    t = F.template(part, s)
    if kind == 'dark':
        return F.recolor_eq(t, DARK_CIRCLE, DARK_CIRCLE)
    if kind == 'holy':
        return F.recolor_eq(t, HOLY_CIRCLE, HOLY_CIRCLE)
    accent = CRIMSON if kind in ('spear', 'knife') else GOLD
    return keep_shipped(part, s, F.recolor_eq(t, BLADE, accent, outline=OUTLINE))


def shield(s=7):
    return F.recolor_eq(F.template('shield', s), STEEL, CRIMSON, outline=OUTLINE)


WEAPONS = {'spear': 5, 'sword': 2, 'knife': 10, 'gun': 8, 'rifle': 0, 'dark': 0, 'holy': 0}


def build():
    parts = {'helmet': helmet(), 'armor': armor(), 'gloves': gloves(), 'shoes': shoes(), 'shield': shield()}
    for kind, s in WEAPONS.items():
        parts['weapon_' + kind] = weapon(kind, s)
    return parts
