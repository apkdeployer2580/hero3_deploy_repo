"""14 흑의 사신 — 케이 전용 (하드코어 엔딩 보상). A black reaper's long coat worn open over the bare
chest, its inner collar crimson, a crimson sash with a hanging tail, the coat's tails ragged at the
hem and lined in pale silver, grey hakama with dark pinstripes, black lacquer sandals with crimson
thongs, black sleeves with a white-and-crimson bandage at the wrist, wild upswept orange hair; a long
katana with a black-and-white wrapped grip, a glaive, a reaper's sickle, a black shield with a
white crossbones mark and two moon circles. (Reference: the user's screenshot 1, 2026-10-09.)"""
import re

from engine import Mat
import circles
import garments
import kit

NUM = '14'
FOLDER = '14_reaper'
HERO = 'kei'
TEXT = """# 의상 14 — 하드코어 엔딩 보상 · 케이 전용. 175차: 사용자 참고 그림 1 (뻗친 주황 머리 · 가슴을 연 검은 사신복 · 붉은 깃과 띠 · 해진 롱코트 · 일본도).
# 투구 시트가 얼굴까지 머리 전체를 그린다(171차 head = own) — 게임은 이 세트의 투구를 입은 동안 주인공 자신의 머리를 안 그린다.
head = own
# tools_re/costume_atelier/sets/reaper.py 가 프레임마다 그린다(재채색 없음).
set            = 흑의 사신
acquire        = clear.hard
hero           = kei
helmet         = 흑의 사신 머리
helmet.desc    = 하늘로 뻗친 주황 머리
armor          = 흑의 사신 상의
armor.desc     = 붉은 깃을 단 검은 사신복
gloves         = 흑의 사신 소매
gloves.desc    = 붕대를 감은 검은 소매
shoes          = 흑의 사신 하의
shoes.desc     = 해진 자락과 줄무늬 바지
weapon         = 흑의 사신 무기
weapon.desc    = 칠흑의 영혼을 벼린 무기
weapon.spear   = 흑의 사신 언월도
weapon.sword   = 흑의 사신 참도
weapon.knife   = 흑의 사신 낫
weapon.dark    = 사신의 흑월진
weapon.holy    = 사신의 백월진
shield         = 흑의 사신 방패
shield.desc    = 해골 문장을 새긴 검은 방패
"""

# ── palettes (tone 0 outline · 1 deep · 2 shadow · 3 mid · 4 light · 5 highlight) ─────────────────
SKIN = '2e0c12 5e2a2e 9a5452 d5898b eea18b fadcc6'      # Kei's own skin
ROBE = '060509 0f0c15 1a1622 27212f 393144 544a64'      # black reaper's coat (a cool violet sheen)
SILK = '24242e 484a5a 727488 9ea2b4 c8ccd8 f2f2f8'      # the coat's pale silver lining / the bandage
RED = '2a0306 5a0810 8c1018 bc1a1e e0322a f86a4a'       # inner collar, sash, sandal thongs
HAKAMA = '15151c 2a2a33 41414c 5a5a68 787888 9e9eb0'    # grey hakama
LACQUER = '040306 0c0a10 18141e 262030 3a3248 5a5070'   # the sandals' black lacquer
HAIR = '4a1004 8a2610 c4501c ee7c30 f6b648 fff0a8'      # Kei's orange, a touch brighter
STEEL = '14161e 2c3240 4c5466 7a8498 b2bccc eef2fa'


def rle(s):
    """'3. 1k 4b' -> '...kbbbb' (a row written as counted runs)."""
    return ''.join(c * int(n) for n, c in re.findall(r'(\d+)(\S)', s))


# ── torso: the coat open over the bare chest, a crimson inner collar framing it, the sash ─────────
TORSO = {}
TORSO.update(kit.family('012345', ROBE, z=2, fam='robe'))
TORSO.update(kit.family('rRQ', RED, z=3, fam='red', tones=(2, 3, 4)))
TORSO.update(kit.family('sST', SKIN, z=2, fam='skin', tones=(2, 3, 4)))
TORSO.update(kit.family('wW', SILK, z=3, fam='silk', tones=(3, 5)))

TORSO_FRAMES = [
    # 0 back: the coat's back, the sash
    """
............
............
.0444433320.
044444333220
044434332220
..04433220..
..0QRRRRr0..
..04433220..
...043320...
............
""",
    # 1 front: the open coat, the crimson inner collar meeting in a V over the bare chest, the sash
    #    knotted on the left with its tail hanging
    """
............
............
.044RTSr320.
0444RTSr3320
04434Rr33220
..04333320..
..0QRRRRr0..
..04Rr3220..
...04r320...
............
""",
    # 2 side (faces left): the bare chest at the front edge, the collar, the coat behind
    """
...........
...........
...04440...
..TR443200.
..SR443320.
..R4433220.
..QRRRRr0..
..r443320..
..r43320...
...0320....
...........
""",
]

# ── arms: black sleeves, a white bandage with crimson turns at the wrist, bare hands ────────────
APAL = {
    'U': Mat(ROBE, z=3, fam='sleeve'),
    'L': Mat(ROBE, z=3, fam='sleeve'),
    'C': Mat(SILK, z=4, flat=4, fam='wrap'),          # the template's cuff is a dark band — the bandage is pinned light
    'c': Mat(RED, z=4, flat=3, fam='wrap'),
    'H': Mat(SKIN, z=4),
}
ARMMAP = {'U': 'U', 'L': 'L', 'C': 'C', 'H': 'H'}


def post_arms(g, i):
    """Crimson turns across the bandage: every other diagonal of the cuff zone."""
    for y in range(g.h):
        for x in range(g.w):
            if g.keys[y][x] == 'C' and (x + y) % 3 == 0:
                g.keys[y][x] = 'c'


# ── legs: grey pinstriped hakama, black sandals with crimson thongs, the long open coat with its
#    tails torn at the hem and its front edges lined in silver ──────────────────────────────────
PAL = {
    'P': Mat(HAKAMA, z=2, fam='hakama'),
    'p': Mat(HAKAMA, z=2, fam='hakama', shift=-1),     # the dark pinstripe
    'B': Mat(LACQUER, z=3, gloss=True),
    'Q': Mat(RED, z=3),
}
PAL.update(garments.coat_keys(ROBE, SILK, ROBE))
LEGMAP = {'W': 'P', 'T': 'P', 't': 'P', 'U': 'P', 'u': 'P', 'S': 'P', 's': 'P', 'A': 'P', 'a': 'P',
          'F': 'B', 'f': 'B'}


def post_legs(g, i):
    for y in range(g.h):
        for x in range(g.w):
            if g.keys[y][x] == 'P' and x % 3 == 1:
                g.keys[y][x] = 'p'
    kit.cuff(g, 'B', {'P', 'p'}, 'Q')
    # the hem: no band, the coat's own black torn into strips — every third column hangs a pixel lower
    hem = [(x, y) for y in range(g.h) for x in range(g.w) if g.keys[y][x] in ('G', 'H', 'g', 'h')]
    for x, y in hem:
        front = g.keys[y][x] in ('G', 'H')
        g.keys[y][x] = ('2' if g.keys[y][x] == 'G' else '1') if front else 'b'
    for x, y in hem:
        if (x + i) % 3 == 1 and y + 1 < g.h:
            below = g.keys[y + 1][x]
            if below is None or (below in 'PpBQ' and g.keys[y][x] in '12'):
                g.keys[y + 1][x] = '1' if g.keys[y][x] in '12' else 'a'


# ── helmet: wild upswept orange hair — spikes thrown up and back over the crown, three fringe spikes
#    over the forehead (one falling between the eyes), long spikes at the nape. Drawn as locks from the
#    crown (`_shared_hair.render`: the tips below are the hand-placed design), toned like the game's
#    own heads. Covers every pixel of Kei's hair, leaves his face. ──
import _shared_hair as hair  # noqa: E402

HPAL = kit.hair_pal(HAIR)

HELM0 = hair.render('kei', 0, (10.5, 13.0), [
    (7.5, 6.0, 3.0), (12.5, 5.0, 3.2), (17.5, 7.0, 3.0), (20.6, 12.0, 2.8), (20.4, 18.0, 2.6),
    (17.5, 24.0, 2.4), (11.5, 27.6, 2.4), (5.5, 25.0, 2.4), (1.2, 19.5, 2.6), (1.0, 13.0, 2.8),
    (3.0, 8.0, 2.8)], swirl=0.05)

HELM1 = hair.render('kei', 1, (9.5, 12.0), [
    (6.5, 7.5, 3.0), (11.5, 6.2, 3.2), (16.0, 7.2, 3.0), (19.8, 10.5, 2.8), (21.0, 15.5, 2.6),
    (19.0, 21.0, 2.2), (14.5, 25.2, 1.6), (7.5, 20.2, 1.0), (2.5, 24.5, 1.8), (0.6, 17.0, 2.6),
    (1.6, 11.0, 2.8)], swirl=0.06)

HELM2 = hair.render('kei', 2, (11.5, 12.5), [
    (9.0, 6.5, 3.0), (14.0, 6.0, 3.2), (18.5, 8.0, 3.0), (20.6, 12.5, 2.8), (20.4, 18.5, 2.6),
    (17.5, 24.0, 2.2), (11.0, 26.6, 1.8), (2.5, 24.5, 1.6), (0.3, 18.5, 2.4), (0.5, 13.5, 2.6),
    (4.5, 9.0, 2.8)], swirl=0.07)

HELM3 = hair.render('kei', 3, (9.0, 10.5), [
    (6.5, 4.8, 3.2), (11.5, 4.5, 3.2), (15.8, 7.0, 3.0), (17.6, 11.0, 2.8), (17.4, 16.5, 2.4),
    (15.4, 22.6, 1.8), (12.6, 21.6, 1.2), (8.6, 19.6, 0.9), (4.2, 21.6, 1.5), (2.4, 23.6, 1.4),
    (0.6, 15.5, 2.6), (1.5, 9.5, 2.8)], swirl=0.0)

# ── weapons ──────────────────────────────────────────────────────────────────────────────────────
INK = '0c060a 0c060a 0c060a 0c060a 0c060a 0c060a'
WPAL = {'k': Mat(INK, flat=0)}
WPAL.update(kit.family('nsSW', STEEL, z=2, fam='w_steel', tones=(1, 3, 4, 5)))
WPAL.update(kit.family('h', SILK, z=2, fam='w_hamon', tones=(4,)))
WPAL.update(kit.family('xbB', LACQUER, z=2, fam='w_black', tones=(1, 3, 5)))
WPAL.update(kit.family('w', SILK, z=2, fam='w_wrap', tones=(4,)))
WPAL.update(kit.family('rRQ', RED, z=2, fam='w_red', tones=(2, 3, 4)))
WPAL.update(kit.family('dD', ROBE, z=2, fam='w_face', tones=(2, 3)))
WPAL.update(kit.family('Z', SILK, z=2, fam='w_mark', tones=(5,)))

# the katana: a black-and-white diamond wrapped grip, a black guard with a crimson heart, a long
# steel blade with a dark spine, a pale temper line and a bright edge
SWORD = """
........kk......................................
.kkkkkkkxxk.....................................
kbwbwbwkxRxkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk..
kwbwbwbkRQRnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnk.
kbwbwbwkxRxSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSWk.
kwbwbwbkxxkhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhWWk..
.kkkkkkkkkWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWkkk....
..........kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk.......
"""

# the glaive: a long single-edged blade (tip at the left), a black collar with a crimson tassel
SPEAR = """
............................................
......kkkkkk.......kk.......................
...kkkWWWSSSkk....kRRk......................
.kkWWSSSSSSnnnkkkkxRQRxkkkkkkkkkkkkkkkkkkkkk
kWWSSSSSSSSnnnnnkkxRRRxbbbbbbbbbbbbbbbbbbbbb
.kkSSSSSnnnnkkkkk.xRRRxkkkkkkkkkkkkkkkkkkkkk
....kkkkkkk.......kRrRk.....................
...................kRk......................
....................k.......................
"""

SHAFT = """
kkkkkkkkkkkkkkkkkkkk
bbbbbbbRbbbbbbbbRbbb
kkkkkkkkkkkkkkkkkkkk
"""

# the sickle: a crimson-wrapped haft from the upper right, the blade sweeping out to the lower left
CLAW = """
..................................
..................................
..................................
............................kkk...
...........................kRQk...
..........................kRRk....
.........................kbRk.....
........................kbbk......
.......................kRbk.......
......................kbbk........
.....................kbRk.........
..........kkkkkkkkkkkRbk..........
.......kkkWWWWWWWSSSkbbk..........
.....kkWWSSSSSSSnnnnkkk...........
....kWWSSnnnnnnkkkkk..............
...kWSSnnkkkkkk...................
..kWSnkk..........................
..kWSk............................
...kWk............................
....kk............................
..................................
"""

# the shield: black, a crimson rim, a white crossbones mark
SHIELD = """
....................
......kkkkkk........
....kkRRRRRRkk......
..kkRdddddddRkk.....
.kRddDDDDDDdddRk....
kRddZdDDDDDZdddRk...
kRdddZdDDDZddddRk...
kRddddZZZZdddddRk...
.kRdddZddZddddRk....
..kRdZddddZddRk.....
...kkRdddddRRkk.....
.....kkRRRRkk.......
.......kkkk.........
....................
"""

# magic circles: dark = a crimson ring with a black crescent moon, holy = a pale full moon
CRESCENT = """
..xxx..
.xoo...
xoo....
xo.....
xoo....
.xoo...
..xxx..
"""
MOON = """
..xxx..
.xooox.
xooooox
xoooxox
xooooox
.xooox.
..xxx..
"""
DARK = [('ring', 0.95, 2), ('ring', 0.84, 1), ('ticks', 12, 0.84, 0.95, 15, 1), ('ring', 0.52, 2),
        ('arc', 0.68, 200, 340, 1), ('arc', 0.68, 20, 160, 1), ('dots', 6, 0.68, 0, 2),
        ('stamp', CRESCENT, {'x': 'f06a5a', 'o': 'ffd0c0'})]
HOLY = [('ring', 0.95, 2), ('ring', 0.88, 1), ('ticks', 24, 0.80, 0.88, 0, 1), ('poly', 6, 2, 0.74, -90, 1),
        ('ring', 0.46, 1), ('dots', 6, 0.74, -90, 2), ('stamp', MOON, {'x': 'c8d0ee', 'o': 'ffffff'})]


def weapons():
    out = {}
    out['weapon_sword'] = kit.weapon('sword', kit.straight(SWORD, WPAL, 'sword'))
    out['weapon_spear'] = kit.weapon('spear', kit.straight(SPEAR, WPAL, 'spear'),
                                     hand={'shaft': kit.straight(SHAFT, WPAL, 'shaft')})
    out['weapon_knife'] = kit.weapon('knife', kit.straight(CLAW, WPAL, 'knife'))
    out['weapon_dark'] = kit.circle(DARK, circles.ramp('d83838'))
    out['weapon_holy'] = kit.circle(HOLY, circles.ramp('c8d4f0', sat=0.6))
    return out


def build():
    parts = {}
    parts['armor'] = kit.frames(TORSO_FRAMES, TORSO, 'armor')
    parts['gloves'] = kit.arms(ARMMAP, APAL, post=post_arms)
    parts['shoes'] = kit.legs(LEGMAP, PAL, [garments.LONG_COAT], under=garments.COAT_UNDER, post=post_legs)
    parts.update(weapons())
    parts['helmet'] = kit.frames([HELM0, HELM1, HELM2, HELM3], HPAL, 'helmet')
    parts['shield'] = kit.frames([SHIELD], WPAL, 'shield')
    return parts
