"""10 광전사 — 케이 전용. A wild warrior: long crimson hair under a black blindfold visor, a bare torso
inked with crimson tribal lines, black-and-pink striped wraps on the forearms and hands, black baggy
trousers under a tattered black loincloth, bare feet in wraps; dark jagged weapons with a red glow.
(Reference: the user's third screenshot.)"""
import math

from engine import Mat
import kit

NUM = '10'
FOLDER = '10_berserker'
HERO = 'kei'
TEXT = """# 의상 10 — 노말 엔딩 보상 · 케이 전용 (175차: 기본 지급 → 노말 엔딩 보상). 169차: 사용자 참고 그림 3 (붉은 장발 · 검은 눈가리개 · 붉은 문신 맨몸 · 줄무늬 붕대 · 해진 천).
# 171차: 투구 시트가 얼굴까지 머리 전체를 그린다 — 게임은 이 세트의 투구를 입은 동안 주인공 자신의 머리를 안 그리고, 머리는 helmfit 이 원래보다 작게 깎는다.
head = own
# tools_re/costume_atelier/sets/berserker.py 가 프레임마다 그린다(재채색 없음).
set            = 광전사
acquire        = clear.normal
hero           = kei
helmet         = 광전사 가면
helmet.desc    = 붉은 장발과 검은 눈가리개
armor          = 광전사 문신
armor.desc     = 붉은 문신을 새긴 맨몸
gloves         = 광전사 붕대
gloves.desc    = 줄무늬 붕대를 감은 팔
shoes          = 광전사 하의
shoes.desc     = 해진 천을 두른 검은 바지
weapon         = 광전사 무기
weapon.desc    = 피에 굶주린 검은 무기
weapon.spear   = 광전사 대낫
weapon.sword   = 광전사 대검
weapon.knife   = 광전사 손톱
weapon.dark    = 광전사 혈마법진
weapon.holy    = 광전사 월마법진
shield         = 광전사 방패
shield.desc    = 해진 검은 방패
"""

# ── palettes (tone 0 outline .. 5 highlight; shadows cooler, highlights warmer) ──────────────────
SKIN = '2e0c12 5e2a2e 9a5452 d5898b eea18b fadcc6'      # Kei's own face tones
INK = '2a0408 5c0814 8c0e1e b8182a de3040 f86a6a'       # the crimson tattoos
HAIR = '2c0614 4e0a20 78102a a8182e d02c36 f06a5a'
VISOR = '060608 0e0e14 18181f 262632 3e3e4e 6c6c80'
PINK = '3c0a28 70123e a81c5e de3484 fa66ac ffb0d6'
WRAP = '0a080c 141018 1e1824 2a2232 3c3246 54486a'      # the black stripes of the wraps
PANTS = '08080e 12121c 1c1c2a 282838 3a3a50 52526c'     # cool black trousers
CLOTH = '040203 0c0608 160b0f 22111a 341a26 4c2836'     # warm black loincloth

# ── helpers ──────────────────────────────────────────────────────────────────────────────────────


def seg(w, h, rows):
    """A w x h overlay block from {row: (x, 'segment')} (or a list of such pairs per row)."""
    g = [['.'] * w for _ in range(h)]
    for y, s in rows.items():
        for x, txt in (s if isinstance(s, list) else [s]):
            for i, c in enumerate(txt):
                if c != '.' and 0 <= x + i < w:
                    g[y][x + i] = c
    return '\n'.join(''.join(r) for r in g)


LEG_SIZES = None


def leg_size(i):
    import bodies
    z, _ = bodies.legs()[i]
    return max(len(r) for r in z), len(z)


# ── torso ────────────────────────────────────────────────────────────────────────────────────────
TORSO = {}
TORSO.update(kit.family('012345', SKIN, z=2, fam='skin'))
TORSO.update(kit.family('qrR', INK, z=2, fam='skin', tones=(1, 2, 3)))
TORSO.update(kit.family('ABCDEF', HAIR, z=3, fam='hair'))
TORSO.update(kit.family('kKLM', WRAP, z=4, fam='belt', tones=(0, 1, 2, 3)))
TORSO.update(kit.family('pP', PINK, z=4, fam='belt', tones=(3, 4)))

TORSO_FRAMES = [
    # 0 back: bare shoulders, the long crimson hair down the middle of the back, the belt
    """
............
............
..0CDEDCB0..
.04CDEDCB30.
.03BCDCCB20.
..02BCDB20..
..03BCB320..
..kKBDBKKk..
...kkBBkk...
............
""",
    # 1 front: a bare muscular chest, a crimson chevron tattoo on the pecs, the belt with a pink knot
    """
............
............
.AD345543DA.
.BD4r54r4DB.
.BC3r32r3CB.
..B3q32q3B..
..A033230A..
..kKLKKpPKk.
...kkkkkk...
............
""",
    # 2 side (faces left): the chest at the front edge, the hair behind on the right
    """
...........
...........
..0345CDB0.
.04544CDDB0
.0r43BCDDB0
..0r32BCB0.
..03q3BCB0.
..kKKpPBk..
...kkkkB...
...........
...........
""",
]

# ── arms: bare upper arm (a tattoo band at the bicep), striped wraps from forearm to hand ──────────
APAL = {
    'S': Mat(SKIN, z=3, auto=True),
    'Q': Mat(INK, z=3, flat=2),
    'K': Mat(WRAP, z=4, fam='wrap'),
    'N': Mat(PINK, z=4, fam='wrap'),
}
ARMMAP = {'U': 'S', 'L': 'L', 'C': 'C', 'H': 'H'}


def stripe_arm(g, i):
    pts = {k: [(x, y) for y in range(g.h) for x in range(g.w) if g.keys[y][x] == k] for k in 'SLCH'}
    up = pts['S'] or pts['L']
    hand = pts['H'] or pts['C'] or pts['L']
    ox = sum(p[0] for p in up) / len(up)
    oy = sum(p[1] for p in up) / len(up)
    hx = sum(p[0] for p in hand) / len(hand)
    hy = sum(p[1] for p in hand) / len(hand)
    dx, dy = hx - ox, hy - oy
    n = max(1e-6, (dx * dx + dy * dy) ** 0.5)
    ux, uy = dx / n, dy / n

    def d(p):
        return (p[0] - ox) * ux + (p[1] - oy) * uy
    lo = min(d(p) for p in pts['L'] + pts['C'] + pts['H'])
    for p in pts['L'] + pts['C'] + pts['H']:
        k = int((d(p) - lo) // 1)
        g.keys[p[1]][p[0]] = 'N' if k % 3 == 1 else 'K'
    if len(pts['S']) >= 4:
        top = max(d(p) for p in pts['S'])
        for p in pts['S']:
            if d(p) > top - 1.6 and (p[0] + p[1]) % 2 == 0:
                g.keys[p[1]][p[0]] = 'Q'


# ── legs: black baggy trousers, a tattered black loincloth (front + back flaps, a sash tail on the
#    near hip), wraps at the ankles, bare feet ──────────────────────────────────────────────────
LPAL = {
    'P': Mat(PANTS, z=2),
    'N': Mat(PINK, z=3),
    'K': Mat(WRAP, z=3),
    'S': Mat(SKIN, z=3),
    'c': Mat(CLOTH, z=5, style='cloth', fam='cloth'),
    'd': Mat(CLOTH, z=5, flat=1, fam='cloth'),
    'e': Mat(CLOTH, z=5, flat=4, fam='cloth'),
    'C': Mat(CLOTH, z=1, style='cloth', fam='clothb', shift=-1),
}
LEGMAP = {'W': 'P', 'T': 'P', 't': 'P', 'U': 'P', 'u': 'P', 'S': 'P', 's': 'P', 'A': 'K', 'a': 'K',
          'F': 'S', 'f': 'S'}


def flap(x0, y0, w, n, hem, fold=None, lit=None, taper=True):
    """A hanging cloth flap: columns x0..x0+w-1 from row y0 (the outer columns a row lower: it is
    gathered under the belt and flares), each column n + hem[c] long — a ragged, torn hem."""
    rows = {}
    for c in range(w):
        L = n + hem[c]
        top = y0 + (1 if taper and c in (0, w - 1) else 0)
        for y in range(top, y0 + L):
            ch = 'c'
            if fold is not None and c == fold and y > y0 + 1 and y < y0 + L - 1:
                ch = 'd'
            if lit is not None and c == lit and y > y0 and y < y0 + L - 2:
                ch = 'e'
            rows.setdefault(y, []).append((x0 + c, ch))
    return rows


def cloth(i, *flaps):
    w, h = leg_size(i)
    rows = {}
    for f in flaps:
        for y, s in f.items():
            rows.setdefault(y, []).extend(s)
    return seg(w, h, rows)


# hem patterns: long torn strips with deep slits between them
H7 = [-2, 2, 0, -4, 1, 3, -1]
H6 = [-2, 2, -3, 1, 3, -1]
H5 = [-1, 2, -3, 1, 0]
H4 = [-1, 2, -2, 1]
CLOTH_OV = {
    0: cloth(0, flap(7, 2, 7, 11, H7, fold=4, lit=1)),
    1: cloth(1, flap(7, 2, 7, 11, H7, fold=4, lit=1), flap(16, 2, 2, 5, [0, 2], taper=False)),
    2: cloth(2, flap(6, 2, 6, 10, H6, fold=3, lit=1)),
    3: cloth(3, flap(5, 2, 6, 10, H6, fold=3, lit=1)),
    4: cloth(4, flap(7, 2, 6, 8, [0, 1, -2, 1, 0, -1], fold=3, lit=1)),
    5: cloth(5, flap(8, 2, 6, 11, H6, fold=3, lit=1), flap(15, 3, 2, 5, [0, 2], taper=False)),
    6: cloth(6, flap(10, 2, 7, 10, H7, fold=4, lit=1), flap(5, 2, 2, 5, [2, 0], taper=False)),
    7: cloth(7, flap(4, 1, 5, 10, H5, fold=3, lit=1)),
    8: cloth(8, flap(4, 1, 5, 10, H5, fold=3, lit=1)),
    9: cloth(9, flap(8, 1, 6, 9, H6, fold=3, lit=1)),
    10: cloth(10, flap(10, 2, 6, 10, H6, fold=3, lit=1)),
    11: cloth(11, flap(2, 2, 3, 9, [-1, 2, 1], lit=1), flap(10, 2, 2, 6, [0, 2], taper=False)),
    12: cloth(12, flap(2, 2, 3, 9, [-1, 2, 1], lit=1), flap(11, 2, 2, 6, [0, 2], taper=False)),
    13: cloth(13, flap(8, 2, 4, 10, H4, lit=1)),
    14: cloth(14, flap(13, 3, 4, 10, H4, lit=1)),
    15: cloth(15, flap(2, 1, 3, 9, [-1, 2, 1], lit=1)),
    16: cloth(16, flap(5, 1, 4, 10, H4, lit=1), flap(12, 2, 2, 5, [0, 2], taper=False)),
    17: cloth(17, flap(4, 2, 4, 8, H4, lit=1)),
}


PUFF = {
    5: seg(22, 19, {11: [(1, 'P'), (20, 'P')], 12: [(1, 'P'), (20, 'P')]}),
    6: seg(26, 18, {8: (1, 'P'), 9: (1, 'P'), 10: (0, 'P')}),
    11: seg(17, 18, {9: (16, 'P'), 10: (16, 'P')}),
    12: seg(20, 16, {8: [(9, 'P'), (19, 'P')], 9: (19, 'P')}),
}


def post_legs(g, i):
    kit.cuff(g, 'K', {'P'}, 'N')


# ── helmet: long wild crimson hair (hand-toned digits, no black outline of its own) and a black
#    blindfold visor across the eyes, knotted at the back with two trailing tails ─────────────────
HPAL = kit.hair_pal(HAIR, extra={
    'X': Mat(VISOR, z=4, flat=0, fam='visor'),
    'V': Mat(VISOR, z=4, flat=1, fam='visor'),
    'U': Mat(VISOR, z=4, flat=2, fam='visor'),
    'W': Mat(VISOR, z=4, flat=3, fam='visor'),
    'Y': Mat(VISOR, z=4, flat=4, fam='visor'),
    'R': Mat(INK, z=4, flat=2, fam='visor'),
})

# j0 back of the head: crown spikes, the band across the back with its knot and two trailing tails
HELM0 = """
.....................
.....................
.....................
.....................
.....................
.....................
.....................
.......00....00......
....000530..0530.0...
...04543320144330200.
..045214333213422210.
..042143332132222110.
.043214333213222210..
.042143332132222210..
.0421433321322211310.
.0314333321322211310.
03113222112111111210.
03113221121111111210.
021322221XYX32221110.
0XUWWWUUVXYXVVVVVVX20
0XVVVVVVVXVXVVVVVVX20
02132221XV2VX32211210
03113221XV2VX32221110
03113221XV2VX32221110
03211321XV2VX3222110.
.0211000XV1VX2100110.
..00....X211X10..00..
.00......000.00....00
"""

# j1 the rare 3/4 view
HELM1 = """
.......................
.......................
.......................
.......................
.......................
.......................
.......................
.......................
.......00....00........
....005430..0540.0.....
..00533214003214010.0..
.0442143333321322210...
..0421433332132222110..
..0314333321322222110..
.04214333213222222110..
.03143332132222113210..
.021322112111111121110.
.013221121111111121110.
.013211211111211112110.
.013112111121111XX2110.
.XUWWWUVVVVVVVVVXYVX20.
..XVVVVVVVVVVVVVXVVX20.
..031VVVVVV.0321XVX310.
..020........031XVX3210
...00.......03211322010
...0.......021111000.00
............00110...00.
............0..00......
"""

# j2 side (faces left): the band from the eye round to the knot behind, its tails, the long hair down the back
HELM2 = """
.....................
.....................
.....................
.....................
.....................
.....................
.....................
.....................
........00....00.....
.....005430..0430.0..
.0050432143003243010.
.043214333332132210..
..0421433332132222110
..0314333321322222110
.04214333213222222110
.03143332132222113210
.02132211211111112110
.01322112111111112110
.01321121111121111210
.01311211112111XX2110
.XWWWWUUVVVVVVVXYVX20
.XVVVVVVVVVVVVVXVVX20
.0310...032113221XVX0
..020...031132221XVX0
..0310..0311322221XVX
...00...0112111111100
........02100111000..
.........00..000..00.
"""

# j3 front: spiky crown, fringe locks over the forehead, the visor across the eyes, side locks past the chin
HELM3 = """
..................
..................
..................
..................
..................
..................
......00...00...0.
...00040.0040..00.
..0454321443200120
.04521432134211310
.04214332132211310
.0421433213221120.
.0314333213222110.
.0433333214333210.
.0322221132222110.
032222211212111130
.03222113112111110
03222211311211110.
03222113211211110.
031VVVVVVVVVVVV310
031VVRVVVVRVVVV310
02120.......003210
021310........0110
021000........0310
000..0.......00010
0...............00
"""

# ── weapons ──────────────────────────────────────────────────────────────────────────────────────
OUT = '0a0608 0a0608 0a0608 0a0608 0a0608 0a0608'
DARK = '0e0a10 1c1820 2e2834 444050 6a6478 9a94a8'      # blackened steel
GLOW = '3a0408 7a0a12 b8141e e8283a ff5a5a ffa090'      # the red glow on edges and cracks
BONE = '2a1c14 4a3426 6e5038 947058 bc9a7c e0c8a8'

WPAL = {'k': Mat(OUT, flat=0)}
WPAL.update(kit.family('mMnN', DARK, z=2, fam='w_dark', tones=(1, 2, 3, 4)))
WPAL.update(kit.family('rRg', GLOW, z=2, fam='w_glow', tones=(2, 3, 4)))
WPAL.update(kit.family('wW', WRAP, z=2, fam='w_wrap', tones=(1, 3)))
WPAL.update(kit.family('pP', PINK, z=2, fam='w_pink', tones=(2, 4)))
WPAL.update(kit.family('bB', BONE, z=2, fam='w_bone', tones=(2, 4)))

SWORD = """
........kk......................................
.kkkkkkkMMkkk.kkk..kkk..kkk..kkk..kkkk..........
kwpwwpkMNNMkkkMNkkkMNkkkMNkkkMNkkkMNNNkkkk......
kwpwwpkmNMmMNNNNNNNNNNNNNNNNNNNNNNNNNNNNNMMkkk..
kwpwwpkmMNmmMMMMMrrRRgRRrrrMMMMRRgRRrMMMMMMMMMk.
kwpwwpkmmMmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmkk..
.kkkkkkkmmkrrRRrrrrRRRrrrrRRRrrrrRRRRrrrrrkk....
........kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk......
"""

SPEAR = """
.....kkk.kkkkkk.............................
...kkMNNkNNNNMMkk...........................
.kkMNNMMMMMMMMMmMk..........................
kNMmRRRRRRRRRRRkmMkkkkkkkkkkkkkkkkkkkkkkkkkk
kMmRkkkkkkkkkkk.kmMMwwpwwwwpwwwwpwwwwpwwwwpw
kmRk............kmMkkkkkkkkkkkkkkkkkkkkkkkkk
kmRk.............kk.........................
.kgk........................................
..k.........................................
"""

SHAFT = """
kkkkkkkkkkkkkkkkkkkk
wwwpwwwpwwwpwwwpwwwp
kkkkkkkkkkkkkkkkkkkk
"""

CLAW = """
..................................
..................................
..................................
.........................kkkk.....
.......................kkwWWwk....
.....................kkwPwPwWwk...
...................kkwwPwPwPwwk...
.................kkwwwPwPwPwwwk...
...............kkMMwwwwPwPwwwwk...
.............kkMNNMkwwwwwwwwwk....
...........kkMNNMMkkwwwwwwwwk.....
.........kkMNNMrkkMMkkwwwkkk......
.......kkMNNMrkkMNNMkkkkk.........
.....kkMNNMrkkMNNMrk..............
...kkMNMrkk.kMNNMrk...............
..kNMrkk....kMMrk.................
..kkk.......kkk...................
..................................
..................................
..................................
..................................
"""

SHIELD = """
....................
.....kkkkkk.........
...kkMMNNMMkk.......
..kMMNNmNNMMMk......
.kMNNMrNNrMMmMk.....
.kMNMmRrMrRMmMk.....
kMNNMrRMrRMrMmmk....
kMNMMRrMRrMRmmmk....
.kMNrRMrRMRmmmk.....
.kMMRMMRMmMmmmk.....
..kMMmMmmmmmk.k.....
...kkmmkmmkk........
.....k.kkk..........
....................
"""


def weapons():
    out = {}
    out['weapon_sword'] = kit.weapon('sword', kit.straight(SWORD, WPAL, 'sword'))
    out['weapon_spear'] = kit.weapon('spear', kit.straight(SPEAR, WPAL, 'spear'),
                                     hand={'shaft': kit.straight(SHAFT, WPAL, 'shaft')})
    out['weapon_knife'] = kit.weapon('knife', kit.straight(CLAW, WPAL, 'knife'))
    out['weapon_dark'] = kit.circle(DARK_CIRCLE, DARK_PAL)
    out['weapon_holy'] = kit.circle(HOLY_CIRCLE, HOLY_PAL)
    return out


DARK_PAL = dict(out='000000', g0='0c0305', g1='160508', g2='22080c', g3='300c12', halo='4a1018',
                line='9a1a28', bright='e8303e', core='ff9080')
HOLY_PAL = dict(out='000000', g0='0e0608', g1='1a0a0e', g2='281016', g3='3a161e', halo='5a2028',
                line='b8384a', bright='ff6a7a', core='ffd8d0')


def _polar(r, deg):
    return r * math.cos(math.radians(deg)), r * math.sin(math.radians(deg))


def thorns(n, r0, r1, rot, w):
    """A zigzag ring (a crown of thorns): n points out at r1, n notches in at r0."""
    out = []
    for i in range(n):
        a = rot + 360 * i / n
        p0, p1, p2 = _polar(r0, a), _polar(r1, a + 180 / n), _polar(r0, a + 360 / n)
        out += [('seg', p0[0], p0[1], p1[0], p1[1], w), ('seg', p1[0], p1[1], p2[0], p2[1], w)]
    return out


def arc_at(cu, cv, r, a0, a1, w, n=24):
    """An arc of a circle centred off the middle (the inner edge of a crescent)."""
    pts = [(cu + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
            cv + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]
    return [('seg', pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1], w) for i in range(n)]


DARK_CIRCLE = ([('blob', 3, 1.08, -90), ('blob', 3, 1.08, -50), ('blob', 3, 1.08, -10),
                ('ring', 0.94, 2), ('ring', 0.50, 1)]
               + thorns(14, 0.62, 0.86, -90, 2)
               + [('seg', -0.30, -0.26, -0.06, 0.22, 3), ('seg', -0.08, -0.32, 0.16, 0.18, 3),
                  ('seg', 0.14, -0.28, 0.34, 0.10, 3)])

HOLY_CIRCLE = ([('ring', 0.94, 2), ('ring', 0.84, 1), ('ticks', 16, 0.84, 0.94, 0, 1),
                ('arc', 0.44, 35, 325, 3)]
               + arc_at(0.20, 0.0, 0.34, 70, 290, 2)
               + [('dots', 5, 0.66, -90, 2), ('dots', 5, 0.70, -54, 1)])


def build():
    parts = {}
    parts['armor'] = kit.frames(TORSO_FRAMES, TORSO, 'armor')
    parts['gloves'] = kit.arms(ARMMAP, APAL, post=stripe_arm)
    parts['shoes'] = kit.legs(LEGMAP, LPAL, [PUFF, CLOTH_OV], under=frozenset('C'), post=post_legs)
    parts.update(weapons())
    parts['helmet'] = kit.frames([HELM0, HELM1, HELM2, HELM3], HPAL, 'helmet')
    parts['shield'] = kit.frames([SHIELD], WPAL, 'shield')
    return parts
