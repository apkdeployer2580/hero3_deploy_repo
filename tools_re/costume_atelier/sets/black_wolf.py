"""11 흑랑 — 케이 전용. A feral hunter: a black shaggy wolf-pelt hood with ears, a teal cloth mask over
the nose and mouth (the eyes stay bare), a bare chest, dark navy baggy trousers bound tight at the
shins, black fur bracers and dark clawed gloves; black-metal wolf-fang weapons with teal glints.
(Reference: the user's fourth screenshot.)"""
from engine import Mat
import bodies
import circles
import kit

NUM = '11'
FOLDER = '11_black_wolf'
HERO = 'kei'
TEXT = """# 의상 11 — 노말 엔딩 보상 · 케이 전용 (175차: 기본 지급 → 노말 엔딩 보상). 169차: 사용자 참고 그림 4 (검은 털 두건 · 청록 복면 · 맨몸 · 남색 바지 · 검은 발톱).
# 171차: 투구 시트가 얼굴까지 머리 전체를 그린다 — 게임은 이 세트의 투구를 입은 동안 주인공 자신의 머리를 안 그리고, 머리는 helmfit 이 원래보다 작게 깎는다.
head = own
# tools_re/costume_atelier/sets/black_wolf.py 가 프레임마다 그린다(재채색 없음).
set            = 흑랑
acquire        = clear.normal
hero           = kei
helmet         = 흑랑 두건
helmet.desc    = 검은 털 두건과 청록 복면
armor          = 흑랑 상의
armor.desc     = 청록 목도리를 두른 맨몸
gloves         = 흑랑 장갑
gloves.desc    = 검은 털 토시와 발톱 장갑
shoes          = 흑랑 하의
shoes.desc     = 감발을 친 짙은 남색 바지
weapon         = 흑랑 무기
weapon.desc    = 늑대의 송곳니 같은 무기
weapon.spear   = 흑랑 창
weapon.sword   = 흑랑 도
weapon.knife   = 흑랑 발톱
weapon.dark    = 흑랑 그림자진
weapon.holy    = 흑랑 달빛진
shield         = 흑랑 방패
shield.desc    = 늑대 머리를 새긴 방패
"""

# ── palettes (tones 0 outline · 1 deep · 2 shadow · 3 mid · 4 light · 5 highlight) ──────────────
SKIN = '310810 5a2829 734041 a46962 e3978a f6dac5'      # Kei's own skin ramp (face, arm template)
FUR = '060509 110e18 1d1a28 2b2739 3e3b52 5c5b78'       # black wolf pelt, cool blue-black
TEAL = '051a22 093540 0e5a62 16847e 33b29c 8ae2c4'      # the mask / scarf / sash
NAVY = '05071a 0c1330 151f4a 1e2c62 2b3d7c 42559a'      # baggy trousers
WRAP = '0c0b0e 1d1b21 302d36 46424e 625d6c 8a8496'      # shin wraps, charcoal cloth
BOOT = '050305 110a0e 1c1217 2a1c22 3c2a32 584250'      # black leather
METAL = '06070b 121620 1e2530 2e3846 485668 7c90a4'     # black steel (claws, blades)

# ── torso ──────────────────────────────────────────────────────────────────────────────────────
TORSO = {}
TORSO.update(kit.family('qrstuv', SKIN, z=2, fam='skin'))
TORSO.update(kit.family('abcdef', TEAL, z=4, fam='teal'))
TORSO.update(kit.family('012345', FUR, z=3, fam='fur'))
TORSO.update(kit.family('pwxyz', NAVY, z=1, fam='navy', tones=(0, 1, 2, 3, 4)))

TORSO_FRAMES = [
    # 0 back: the scarf round the nape, its two tails hanging down the spine, shoulder blades in skin,
    #   fur tufts on the shoulder caps, the sash, the trousers' waistband
    """
............
............
.03adeedca30
032ubddbt210
.0uubccbts0.
..qutcbusq..
..qubcdbtq..
..qtsbcatq..
..adeeedca..
...pyyxwp...
""",
    # 1 front: the scarf round the neck, pectorals and abs in skin clusters, the sash, the waistband
    """
............
............
.03adeedca30
032vbccbt210
.0vvuusuts0.
..uutsrssr..
..tuusutsr..
..sttstsrq..
..adeeedca..
...pyyxwp...
""",
    # 2 side (faces left): the chest at the front edge, the scarf's tails trailing behind
    """
...........
...........
...adeedca.
..qvbcccba.
.qvuutstsc.
.quutstsrcb
.qtuustsrb.
.qsttsrsq..
..qtsrrsq..
..adeedca..
..pyyyxwp..
""",
]
# ── legs: navy baggy trousers, charcoal shin wraps, black boots ──────────────────────────────────
PAL = {
    'N': Mat(NAVY, z=2, style='cloth'),
    'Z': Mat(WRAP, z=3, fam='wrap'),
    'Y': Mat(WRAP, z=3, fam='wrap', shift=1),
    'B': Mat(BOOT, z=3, gloss=True),
    'S': Mat(SKIN, z=2, shift=1),
    'F': Mat(FUR, z=3, style='cloth', gloss=True),
    'T': Mat(TEAL, z=4),
    'G': Mat(METAL, z=4, gloss=True, style='metal'),
}
LEGMAP = {'W': 'N', 'T': 'N', 't': 'N', 'U': 'N', 'u': 'N', 'S': 'Z', 's': 'Z', 'A': 'Z', 'a': 'Z',
          'F': 'B', 'f': 'B'}
ARMMAP = {'U': 'S', 'L': 'F', 'C': 'T', 'H': 'G'}


def _puff(g, i):
    """Baggy trousers: the knee run of each leg balloons one pixel outward (where there is room)."""
    zones = bodies.legs()[i][0]
    adds = []
    for y in range(g.h):
        for x in range(g.w):
            z = zones[y][x] if x < len(zones[y]) else '.'
            if z not in 'UuTt' or g.keys[y][x] != 'N':
                continue
            if z in 'Tt':
                below = zones[y + 1][x] if y + 1 < g.h and x < len(zones[y + 1]) else '.'
                if below not in 'Uu':
                    continue
            for dx in (-1, 1):
                X = x + dx
                clear = all(not (0 <= X + k * dx < g.w) or g.keys[y][X + k * dx] is None for k in (1, 2))
                if 0 <= X < g.w and g.keys[y][X] is None and clear:
                    adds.append((X, y, x))
    for X, y, x in adds:
        g.keys[y][X] = 'N'
        g.tones[y][X] = None
        inner = x - (X - x)
        if 0 <= inner < g.w and g.tones[y][inner] is not None and g.tones[y][x] == 0:
            g.tones[y][x] = g.tones[y][inner]


def _wraps(g, i):
    """Bands on the shin wraps (every other row a tone lighter)."""
    for y in range(g.h):
        for x in range(g.w):
            if g.keys[y][x] == 'Z' and y % 2 == 0:
                g.keys[y][x] = 'Y'


def post_legs(g, i):
    _puff(g, i)
    _wraps(g, i)


def build():
    parts = {}
    parts['armor'] = kit.frames(TORSO_FRAMES, TORSO, 'armor')
    parts['gloves'] = kit.arms(ARMMAP, PAL)
    parts['shoes'] = kit.legs(LEGMAP, PAL, post=post_legs)
    parts.update(weapons())
    parts['helmet'] = kit.frames([_fur(b, j) for j, b in enumerate([HELM0, HELM1, HELM2, HELM3])], HPAL, 'helmet')
    parts['shield'] = kit.frames([SHIELD], WPAL, 'shield')
    return parts


# ── helmet: the black wolf-pelt hood + the teal mask ──────────────────────────────────────────────
# The hood is drawn as a silhouette of symbols; `_fur` gives it fur tones: locks fanning out from the
# crown (each lock lit on one edge, parted on the other), lit from the top-left, a cleanup pass so no
# pixel stands alone. Symbols:
#   #  fur                r  ruff (one tone lighter)   -  shadowed fur (one darker)   S  a parting
#   +  highlight (5)      *  light (4)                 M  tone 2   D  deep shadow (1)
#   I  inner ear, dark (1)   i  inner ear, its light fur (3)
# A fur pixel on the silhouette takes tone 0 (the pelt's own darkest tone is its edge); a fur pixel
# next to the face (helmguide 'f') takes tone 1 — the hood's inner shadow. Letters a..f = the mask
# (teal tones 0..5), drawn as they are.
import math  # noqa: E402
from collections import Counter  # noqa: E402

import helmguide  # noqa: E402

FUR_SYM = '#r-S+*MDIiX'
_FACE = {}
# per helmet frame: the crown the locks fan out from, the lock width (radians), the light (bbox fraction)
FURSET = {0: ((10.5, 4.0), 0.32, (0.2, 0.1)), 1: ((9.0, 5.0), 0.32, (0.2, 0.1)),
          2: ((10.0, 5.0), 0.32, (0.2, 0.1)), 3: ((8.5, 3.0), 0.30, (0.2, 0.1))}


def _face(j):
    if not _FACE:
        for jj, g in helmguide.guide(HERO).items():
            _FACE[jj] = {(x, y) for y, r in enumerate(g) for x, c in enumerate(r) if c in 'f,'}
    return _FACE[j]


def _h32(n):
    n = (n * 2654435761) & 0xffffffff
    n ^= n >> 15
    return (n * 2246822519 & 0xffffffff) / 0xffffffff


def _fur(block, j, spread=1.75, wave=0.25):
    (cx, cy), dth, light = FURSET[j]
    rows = block.strip('\n').split('\n')
    h, w = len(rows), max(len(r) for r in rows)
    rows = [r.ljust(w, '.') for r in rows]
    face = _face(j)
    cells = [(x, y) for y in range(h) for x in range(w) if rows[y][x] in FUR_SYM]
    x0, x1 = min(x for x, _ in cells), max(x for x, _ in cells)
    y0, y1 = min(y for _, y in cells), max(y for _, y in cells)
    lx, ly = x0 + light[0] * (x1 - x0), y0 + light[1] * (y1 - y0)
    diag = math.hypot(x1 - x0, y1 - y0)

    def empty(X, Y):
        return not (0 <= X < w and 0 <= Y < h) or rows[Y][X] in '. '
    T = {}
    for (x, y) in cells:
        b = 1 - spread * math.hypot(x + 0.5 - lx, y + 0.5 - ly) / diag
        dx, dy = x + 0.5 - cx, y + 0.5 - cy
        u = math.atan2(dx, dy) / dth + wave * math.sin(math.hypot(dx, dy) * 0.45)
        k = math.floor(u)
        b += (_h32(k + 17) - 0.5) * 0.22
        b += 0.15 if u - k < 0.3 else -0.3 if u - k > 0.8 else 0
        T[x, y] = 5 if b > 1.02 else 4 if b > 0.82 else 3 if b > 0.52 else 2 if b > 0.15 else 1
    for _ in range(2):
        for (x, y) in cells:
            if rows[y][x] != '#':
                continue
            nb = [T[p] for p in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)) if p in T]
            if len(nb) == 4 and T[x, y] not in nb:
                T[x, y] = Counter(nb).most_common(1)[0][0]
    out = []
    for y in range(h):
        line = ''
        for x in range(w):
            c = rows[y][x]
            if (x, y) not in T:
                line += c
                continue
            nb = ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1))
            bg = any(empty(X, Y) and (X, Y) not in face for X, Y in nb)
            fc = any(empty(X, Y) and (X, Y) in face for X, Y in nb)
            t = T[x, y]
            if bg and c != 'D':
                t = 0
            elif c == '#':
                t = 1 if fc else t
            elif c == 'r':
                t = min(4, max(3, t + 1))
            elif c == '-':
                t = 1 if fc else max(1, t - 1)
            elif c == 'S':
                t = max(0, t - 2)
            else:
                t = {'+': 5, '*': 4, 'M': 2, 'D': 1, 'I': 1, 'i': 3, 'X': 0}[c]
            line += str(t)
        out.append(line)
    return '\n'.join(out)


HPAL = kit.hair_pal(FUR, extra=kit.family('abcdef', TEAL, z=3, fam='mask'))

# j0 back: the hood's crown, the backs of the ears, a shaggy fringe over the nape
HELM0 = """
.....................
.....................
.....................
.....................
.....#.........#.....
....###.......###....
....####.....####....
...#####.....#####...
...######...#######..
...################..
..#################..
..##################.
.###################.
####################.
####################.
.####################
.####################
.###################.
####################.
####################.
.####################
.####################
.###################.
..##################.
..#################..
..##.############.#..
.....#.######.##.....
.........###..#......
"""

# j1 rare 3/4: the eyes look out under the brow's ruff, the mask below
HELM1 = """
.......................
.......................
.......................
.....#........#........
....###......###.......
....#iI#....##I#.......
...##iI#...##iI##......
...##iI#####IiI##......
..################.....
..#################....
.##################....
.###################...
.##################....
####################...
####################...
.###################...
.####################..
.###r###r###r########..
.#rrrr#rrrrr#r######...
.#rrrrrrrrrrr#######...
.#r--..rr..-########...
..r-D..ef..D#######....
..r-Dcedd..D#######....
.##-bdeedcbD####.#.....
..##.bcddcbD###........
......bccba.D#.........
.......abba............
.......................
"""

# j2 side (faces LEFT): the hood's open front over the brow, ears up, the mask over nose and mouth
HELM2 = """
.....................
.....................
.....................
.....................
......#.......#......
.....###.....###.....
.....#iI#...##I#.....
....##iI#..##iI##....
....##iI######I##....
...###############...
.##################..
.###################.
..rr################.
.#rr################.
#rr################..
#rr#################.
.#rr#################
.#rr#################
.#rr################.
.#rrrrr#############.
.#r---..##########...
..befe..D########....
..bdeedb-########....
..bcddcb-########....
...acdca-######......
....bca.#####.#......
........#.#.#........
.....................
"""

# j3 front: two ears, the crown, a ruff of lighter fur across the brow (tufts between the eyes) and
# down both cheeks, the eyes in the hood's shadow, the mask from under the eyes to a point below the chin
HELM3 = """
..................
..................
...0..........0...
..030........020..
..0310......0120..
..03110....01120..
.0#3112#..#21122#.
.################.
.################.
.#################
##################
#################.
.################.
.################.
.#################
##################
#--#r-#-r-#-r-#-#.
.#rrr#rrrrr#rrr##.
.#rrrrrrrrrrrrrr#.
.##rrrr.rr..rrr##.
.##rr-..rr..-rr##.
##rr#-cefedb-#rr##
.#rr#-bdedca-#r##.
.##r#-acdcba-#r##.
.##.##.bcba.##.##.
.#...#..aa..#...#.
"""

# ── weapons ──────────────────────────────────────────────────────────────────────────────────────
INK = '050508 050508 050508 050508 050508 050508'
WOOD = '0c0606 1e1210 2e1c18 40281f 56382a 704c38'

WPAL = {'k': Mat(INK, flat=0)}
WPAL.update(kit.family('mMW', METAL, z=2, fam='w_metal', tones=(2, 3, 5)))
WPAL.update(kit.family('nN', METAL, z=2, fam='w_metal', tones=(1, 4)))
WPAL.update(kit.family('tTe', TEAL, z=2, fam='w_teal', tones=(2, 3, 5)))
WPAL.update(kit.family('fFg', FUR, z=2, fam='w_fur', tones=(1, 3, 5)))
WPAL.update(kit.family('oO', WOOD, z=2, fam='w_wood', tones=(2, 4)))
WPAL.update(kit.family('lL', BOOT, z=2, fam='w_leather', tones=(2, 4)))

# a long black fang of a blade (a kukri's belly), the cutting edge glinting teal; a teal-wrapped grip
SWORD = """
................................................
.kkk.....kk.....................kkkkkkkkkk......
knNnk...kTk.kkkkkkkkkkkkkkkkkkkkWWWWWWWWWWkkk...
kLlLkkkkkTkkWWWWWWWWWWWWWWWWWWWWMMMMMMMMMMWWWkk.
klLlLtTtnTnMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMWk
kLlLkkkkkTkmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmek.
knNnk...kTkkkkkkkkkkttttteeeeeeeeeeeeeeeeeeek...
.kkk.....kk.........kkkkkkkkkkkkkkkkkkkkkkkk....
"""

# a black leaf-blade spearhead with teal edges, a teal binding and a tuft of black fur, a dark shaft
SPEAR = """
............................................
......kkk.......kk..........................
...kkkWWMkk....kfFk.k.......................
kkkWWWWMMMnkk.kfgFfkFk.kkkkkkkkkkkkkkkkkkkkk
eWWWWWMMMMmmnktkfgFfFfkTtToOoOoOoOoOoOoOoOoO
kkkeemmmmmmkk.kfFFffkfk.kkkkkkkkkkkkkkkkkkkk
...kkkeemkk....kffFk.k......................
......kkk.......kfk.........................
.................k..........................
"""

SHAFT = """
kkkkkkkkkkkkkkkkkkkk
oOoOoOoOoTtToOoOoOoO
kkkkkkkkkkkkkkkkkkkk
"""

# the signature: three hooked black-steel claws from a gauntlet with a teal stone, teal glints at
# the tips (generated from three blade paths; the gauntlet drawn over their roots)
CLAW = """
..................................
..................................
.........................kkkk.....
.......................kkNNNNkk...
...................kkkkNNnnnnNNk..
..............kkkkkkWknNlltTlnNk..
...........kkkkWWWWWknnllteeTlnk..
......kkkkkkWWNMMMMknnlllltTllnk..
....kkkNWWWWMMMmmmknnllllllllnk...
...kkWWMMMMMmmmkkkknlllllllllk....
..kkWeMtmmmmkkkWWWWknlllllllk.....
..kWektkkkkkWWNMMMMMkklllllk......
..kekkkNWWWWMMMmmmmmkkkkkkk.......
..kkkkWMMMMMmmmkkkkWWNk...........
..kkkWettmmmkkNWWWWMMMk...........
...kWekkkWWWWWMMMMMmmmk...........
...kekkkWeMMMMmmmmmkkkk...........
...kkkkWekttmmkkkkkk..............
...kkkkekkkkkkk...................
......kkkk........................
......kkk.........................
"""

# a round shield of dark planks, a rim of black fur, a teal wolf's head (ears, brow, muzzle)
SHIELD = """
....................
.....kkkkkkk........
...kkfFgFFFfkk......
..kfFoOOOOOoFfk.....
.kfoOtOOOOtOoOfk....
kfoOOttOOttOOOofk...
kfoOOtTTTTTOOOofk...
kfoOOOteTetOOOofk...
kfooOOOtTtOOOoofk...
.kfooOOOtOOOoofk....
..kfooOOOOoofk......
...kkffffffkk.......
.....kkkkkk.........
....................
"""


def weapons():
    out = {}
    out['weapon_sword'] = kit.weapon('sword', kit.straight(SWORD, WPAL, 'sword'))
    out['weapon_spear'] = kit.weapon('spear', kit.straight(SPEAR, WPAL, 'spear'),
                                     hand={'shaft': kit.straight(SHAFT, WPAL, 'shaft')})
    out['weapon_knife'] = kit.weapon('knife', kit.straight(CLAW, WPAL, 'knife'))
    out['weapon_dark'] = kit.circle(DARK, circles.ramp('3ad8c0'))
    out['weapon_holy'] = kit.circle(HOLY, circles.ramp('e2ecff', sat=0.55))
    return out


# dark: a shadow-teal circle — a ring of fangs, three claw slashes, a wolf's head at the centre
WOLF_STAMP = """
.a.......a.
.aa.....aa.
.abaaaaaba.
.aaaaaaaaa.
..aacacaa..
...aaaaa...
....aaa....
.....a.....
"""
DARK = [
    ('ring', 0.97, 1), ('ring', 0.86, 2), ('ticks', 18, 0.86, 0.97, 10, 1),
    ('seg', -0.62, -0.5, 0.0, 0.62, 2), ('seg', -0.32, -0.62, 0.3, 0.5, 2), ('seg', -0.02, -0.72, 0.6, 0.38, 2),
    ('ring', 0.5, 1),
    ('stamp', WOLF_STAMP, {'a': '7ff0dc', 'b': '0a2a2a', 'c': '0a2a2a'}),
]
# holy: a pale moonlight circle — a crescent moon, a ring of stars
HOLY = [
    ('ring', 0.97, 1), ('ring', 0.88, 2), ('ticks', 16, 0.88, 0.97, 0, 1),
    ('dots', 8, 0.74, 22.5, 2.6), ('dots', 8, 0.74, 0, 1.2),
    ('arc', 0.5, 100, 340, 3), ('arc', 0.36, 125, 315, 2),
    ('dots', 1, 0.0, 0, 3.4), ('dots', 4, 0.2, 45, 1.4),
]
