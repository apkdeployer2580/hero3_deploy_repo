#!/usr/bin/env python3
"""아바타 옷장 (167차) — Claude's FIRST DRAFT of the four costume sets, as indexed PNG frames.

    python tools_re/costume_draft.py [NN ...] [--force]

★ This is the record of how the first drafts were made, nothing more (hero2's rule: 「Claude 초안 →
사용자 수정」). Each piece starts from a SHIPPED sheet of the same group — so every frame has the
size and the silhouette the paper doll already places (`avatar_sheets.FRAME_SIZES`) — and is
repainted by palette: skin stays skin, near-black outlines stay dark, the dominant hue family goes
to the set's MAIN ramp and everything else to its ACCENT ramp, each by its own luminance.

⛔ It refuses to overwrite a folder that already has PNGs — once the user edits a frame, running
this again would silently throw the edit away. `--force` is for a folder nobody has touched.
After editing a PNG: `python tools_re/avatar_sheets.py check`, then the asset chain
(`patch_asset.py --apply` …) packs it.
"""
import colorsys
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import avatar_sheets as AS  # noqa: E402

# Ramps, dark -> light (sRGB).
RAMPS = {
    'lightblue': [(38, 66, 118), (78, 128, 190), (128, 182, 234), (188, 224, 250), (236, 248, 255)],
    'silk': [(118, 118, 138), (176, 176, 196), (222, 222, 234), (244, 244, 250), (255, 255, 255)],
    'blonde': [(104, 64, 18), (168, 118, 38), (220, 176, 70), (246, 216, 122), (255, 242, 192)],
    'silver': [(66, 72, 84), (118, 126, 140), (168, 176, 190), (208, 214, 226), (244, 248, 252)],
    'worn': [(48, 34, 24), (88, 64, 44), (128, 98, 70), (164, 134, 100), (198, 174, 140)],
    'leather': [(44, 27, 17), (84, 54, 34), (124, 84, 54), (160, 114, 74), (194, 150, 104)],
    'iron': [(44, 40, 38), (84, 77, 71), (124, 114, 104), (164, 151, 139), (204, 194, 184)],
    'bone': [(88, 78, 58), (148, 138, 108), (198, 190, 164), (228, 224, 204), (250, 248, 234)],
    'black': [(10, 10, 14), (26, 26, 32), (44, 44, 54), (66, 66, 78), (92, 92, 106)],
    'blush': [(168, 108, 128), (214, 158, 178), (240, 198, 214), (250, 224, 234), (255, 244, 250)],
    'ice': [(28, 58, 108), (58, 108, 168), (108, 168, 218), (168, 214, 244), (224, 244, 255)],
    'pumpkin': [(88, 34, 4), (158, 68, 10), (218, 118, 20), (244, 168, 58), (255, 214, 118)],
}

# set -> part -> (template sprite, main ramp, accent ramp, frames to keep as shipped)
# (the part names are `avatar_sheets.PART_GROUP`'s; the template is a shipped sprite of that group)
RECIPES = {
    1: {  # 클레르의 잠옷 — 기본 지급
        'helmet': (9, 'blonde', 'blonde', ()),
        'armor': (9, 'lightblue', 'silk', ()),
        'gloves': (3, 'lightblue', 'silk', ()),
        'shoes': (0, 'lightblue', 'blush', ()),
    },
    2: {  # 빙해의 검사 — 노말 엔딩 보상
        'helmet': (9, 'silver', 'silver', ()),
        'armor': (14, 'worn', 'iron', ()),
        'gloves': (12, 'leather', 'worn', ()),
        'shoes': (11, 'worn', 'leather', ()),
        'weapon_spear': (3, 'iron', 'leather', ()),
        'weapon_sword': (0, 'iron', 'leather', ()),
        'weapon_knife': (3, 'leather', 'iron', ()),
        'weapon_gun': (6, 'iron', 'leather', ()),
        'weapon_rifle': (6, 'worn', 'iron', (4, 5, 6)),
        'weapon_dark': (0, 'ice', 'ice', ()),
        'weapon_holy': (0, 'ice', 'ice', ()),
        'shield': (1, 'worn', 'iron', ()),
    },
    3: {  # 율리케의 드레스코드 — 하드코어 엔딩 보상
        'helmet': (2, 'silk', 'silk', ()),
        'armor': (20, 'silk', 'blush', ()),
        'gloves': (10, 'silk', 'blush', ()),
        'shoes': (7, 'silk', 'blush', ()),
    },
    4: {  # 할로윈 — 헬 엔딩 보상
        'helmet': (12, 'bone', 'black', ()),
        'armor': (19, 'black', 'bone', ()),
        'gloves': (5, 'black', 'bone', ()),
        'shoes': (5, 'black', 'bone', ()),
        'weapon_spear': (0, 'bone', 'bone', ()),
        'weapon_sword': (5, 'bone', 'bone', ()),
        'weapon_knife': (8, 'bone', 'bone', ()),
        'weapon_gun': (0, 'bone', 'bone', ()),
        'weapon_rifle': (2, 'bone', 'bone', (4, 5, 6)),
        'weapon_dark': (4, 'pumpkin', 'pumpkin', ()),
        'weapon_holy': (4, 'pumpkin', 'pumpkin', ()),
        'shield': (2, 'bone', 'bone', ()),
    },
}


def ramp_at(ramp, t):
    t = min(1.0, max(0.0, t)) * (len(ramp) - 1)
    i = min(int(t), len(ramp) - 2)
    f = t - i
    a, b = ramp[i], ramp[i + 1]
    return tuple(round(a[k] + (b[k] - a[k]) * f) for k in range(3))


def lum(rgb):
    r, g, b = rgb
    return 0.299 * r + 0.587 * g + 0.114 * b


def hsv(rgb):
    return colorsys.rgb_to_hsv(*(v / 255 for v in rgb))


def is_skin(rgb):
    h, s, v = hsv(rgb)
    deg = h * 360
    return (deg <= 45 or deg >= 345) and 0.12 <= s <= 0.62 and v >= 0.55


def hue_dist(a, b):
    d = abs(a - b) % 360
    return min(d, 360 - d)


def recolor(frames, main, accent, keep):
    """A new palette per frame: skin kept, outlines kept dark, the dominant hue family -> `main`,
    the rest -> `accent`, each spread over its ramp by its own luminance range."""
    usage = Counter()
    for fi, f in enumerate(frames):
        if fi in keep:
            continue
        for y in range(f.h):
            for x in range(f.cw):
                i = f.pixels[y * f.w + x]
                if i:
                    usage[AS.to_rgb(f.palette[i])] += 1
    body = [c for c in usage if not is_skin(c) and hsv(c)[2] >= 0.16]
    buckets = Counter()
    for c in body:
        h, s, _v = hsv(c)
        if s >= 0.2:
            buckets[int(h * 12) % 12] += usage[c]
    dom = (buckets.most_common(1)[0][0] * 30 + 15) if buckets else None

    def in_main(c):
        h, s, _v = hsv(c)
        return s < 0.2 or dom is None or hue_dist(h * 360, dom) <= 45

    groups = {'main': [c for c in body if in_main(c)], 'accent': [c for c in body if not in_main(c)]}
    out = {}
    for name, ramp in (('main', RAMPS[main]), ('accent', RAMPS[accent])):
        cs = groups[name]
        if not cs:
            continue
        lo, hi = min(map(lum, cs)), max(map(lum, cs))
        for c in cs:
            t = (lum(c) - lo) / (hi - lo) if hi > lo else 0.6
            out[c] = ramp_at(ramp, 0.1 + 0.85 * t)
    dark = ramp_at(RAMPS[main], 0.0)

    def mapped(rgb):
        if rgb in out:
            return out[rgb]
        if is_skin(rgb):
            return rgb
        if hsv(rgb)[2] < 0.16:
            return tuple(round(v * 0.55) for v in dark)
        return ramp_at(RAMPS[main], 0.1 + 0.85 * lum(rgb) / 255)

    new = []
    for fi, f in enumerate(frames):
        if fi in keep:
            new.append(f)
            continue
        pal = [f.palette[0]] + [AS.to_565(mapped(AS.to_rgb(v))) for v in f.palette[1:]]
        new.append(AS.Frame(f.w, f.h, f.cw, pal, f.pixels, f.bpp))
    return new


def draft(set_id: int, force: bool) -> int:
    folder = next((d for d in AS.costume_folders() if AS.set_id_of(d) == set_id), None)
    if folder is None:
        print(f'no costumes/{set_id:02d}_* folder')
        return 1
    if not force and any(fn.endswith('.png') for fn in os.listdir(folder)):
        print(f'{os.path.relpath(folder, AS.ROOT)}: already has PNGs - refusing to overwrite (the user may have edited them); --force for an untouched folder')
        return 1
    jar = AS.jar_of()
    n = 0
    for part, (sprite, main, accent, keep) in RECIPES[set_id].items():
        g = AS.PART_GROUP[part]
        frames = AS.decode(jar.read(AS.shipped_member(g, sprite)))
        sizes = [f.size() for f in frames]
        if sizes != AS.FRAME_SIZES[g]:
            print(f'template {AS.shipped_member(g, sprite)} has sizes {sizes}, not the group signature - pick another')
            return 1
        for i, f in enumerate(recolor(frames, main, accent, set(keep))):
            AS.frame_to_png(os.path.join(folder, f'{part}_{i}.png'), f)
            n += 1
    print(f'{os.path.relpath(folder, AS.ROOT)}: wrote {n} frame(s)')
    return 0


def main(argv) -> int:
    force = '--force' in argv
    ids = [int(a) for a in argv if a.isdigit()] or sorted(RECIPES)
    return max(draft(i, force) for i in ids)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
