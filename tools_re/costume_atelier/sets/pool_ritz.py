"""07 수영장 파티 리츠 — 리츠 전용, 기본 지급. A summer pool party: cool black wayfarer sunglasses (Ritz's own
hair stays), bare chest with a hibiscus lei, red-and-white striped swim trunks and flip-flops; pool toys
for weapons — a striped beach parasol, a twisted balloon sword, swim flippers, a water pistol and a big
water blaster, and a beach ball. Every pixel drawn here (169차)."""
from engine import Mat
import kit
from _pool_shared import fam, compose, WHITE, BALL_YEL
import _pool_shared as P

NUM = '07'
FOLDER = '07_pool_ritz'
HERO = 'ritz'
HELMET_COVERS_HAIR = False
TEXT = """# 의상 7 — 기본 지급 · 리츠 전용 (hero = ritz: 다른 주인공의 옷장에는 안 나온다).
# 169차: 사용자 요청으로 재채색 없이 프레임마다 새로 그렸다 — tools_re/costume_atelier/sets/pool_ritz.py.
set            = 수영장 파티 리츠
acquire        = starter
hero           = ritz
helmet         = 수영장 선글라스
helmet.desc    = 멋을 낸 검은 선글라스
armor          = 수영장 꽃목걸이
armor.desc     = 맨몸에 건 꽃목걸이
gloves         = 수영장 맨팔
gloves.desc    = 소매 없이 드러낸 팔
shoes          = 수영장 수영복
shoes.desc     = 줄무늬 빨간 수영복과 샌들
weapon         = 수영장 장난감
weapon.desc    = 물놀이 장난감 무기
weapon.spear   = 수영장 파라솔
weapon.sword   = 수영장 풍선 검
weapon.knife   = 수영장 물갈퀴
weapon.gun     = 수영장 물총
weapon.rifle   = 수영장 대형 물총
shield         = 수영장 비치볼
shield.desc    = 알록달록한 비치볼
"""

TRUNKS = '3a0810 701420 a8202c d83a3c f26a5a ffb0a0'
SOLE = '3a0810 701420 a8202c d83a3c f26a5a ffb0a0'
BAND = TRUNKS
BLACK = '040406 0c0c10 18181e 2c2c38 4a4a5c 9a9ab0'

BLADE = '500818 901428 d02434 f8504a ff9888 fff0e8'
GUARD = '6a3c00 b07400 eaa800 ffd23a fff08a fffbe0'
STRIPE = '4a0810 8a1420 c82430 f04848 ff9a8a fff0ec'
FIN = '062a48 0e5080 1880c0 30b0e8 80dcff e0f8ff'
POCKET = '5a1c00 a03c00 e06810 ff9a30 ffc878 fff0d8'

ORANGE = '5a1c00 a03c00 e06810 ff9a30 ffc878 fff0d8'
WATER = '0a2a50 145890 2a90d0 60c8f8 b0ecff ffffff'
NOZZLE = '4a0610 8e1420 cc2a30 f04a46 ff9282 fff0ea'
STRAP = '0a0c18 1a2038 2a3458 3c4a78 586898 8090c0'

# ── sunglasses: black wayfarers, a white glint on each lens ──────────────────────────────────────────
HPAL = {'w': Mat(WHITE, z=5, flat=5)}
HPAL.update(fam('kac', BLACK, 3, 'shades', tones=(0, 1, 3)))

E21 = '.' * 21
E23 = '.' * 23
E18 = '.' * 18
HELM0 = '\n'.join([E21] * 19 + [
    '.....k..........k....',
] + [E21] * 8)
HELM1 = '\n'.join([E23] * 19 + [
    '...kkkkkk..............',
    '...kawaakkkkk..........',
    '...kaaackawaak.........',
    '....kkk..aaack.........',
    '.........kkkk..........',
] + [E23] * 4)
HELM2 = '\n'.join([E21] * 19 + [
    '...kkkkk.............',
    '...kwaakkkkkk........',
    '...kaack.............',
    '....kkk..............',
] + [E21] * 5)
HELM3 = '\n'.join([E18] * 18 + [
    '...kkkkkkkkkkkk...',
    '...kawaa..awaak...',
    '....kack..kack....',
] + [E18] * 5)


# ── trunks: red with white stripes ───────────────────────────────────────────────────────────────────
LEGPAL = P.legs_pal(TRUNKS, SOLE, WHITE, {'f': Mat(WHITE, z=4, flat=4)})


def stripes(g, i):
    marks = [(x, y) for y in range(g.h) if y % 3 == 2 for x in range(g.w)
             if g.keys[y][x] == 'R' and P.interior(g, x, y, 'R')]
    for x, y in marks:
        g.keys[y][x] = 'f'


# ── water guns ───────────────────────────────────────────────────────────────────────────────────────
GPAL = {'w': Mat(WHITE, z=5, flat=5)}
GPAL.update(fam('oabcde', ORANGE, 2, 'g_body'))
GPAL.update(fam('TABCDE', WATER, 3, 'g_tank'))
GPAL.update(fam('rR', NOZZLE, 2, 'g_noz', tones=(2, 4)))

PISTOL_BODY = """
.ooooooooooooooooo.
oedddddddddddddccbo
odccccccccccccccbbo
obbbbbbbbbbbbbbbbao
.ooooooooooooooooo.
"""
PISTOL_TANK = """
.TTTTTT.
TEEDDCBT
TDDCCBAT
TCBBBAAT
.TTTTTT.
"""
PISTOL_NOZZLE = """
oooo.
RRrRo
oooo.
"""
PISTOL_GRIP = """
.odcbao
.odcbo.
odcbao.
odcbo..
oooo...
"""
PISTOL_TRIGGER = """
.oo.
ow.o
oooo
"""


def pistol():
    return compose(26, 15, [
        (PISTOL_GRIP, 2, 8),
        (PISTOL_TRIGGER, 9, 8),
        (PISTOL_BODY, 2, 4),
        (PISTOL_NOZZLE, 20, 5),
        (PISTOL_TANK, 7, 0),
    ])


GUN_DOWN = """
...........
...........
...........
...........
....ooo....
...odcbo...
..TTodbo...
.TEDTdbo...
.TDCTcbo...
.TCBTcbo...
..TTodbo...
...odcbo...
...odcbo...
...obbao...
....ooo....
....oRo....
....oRo....
....oro....
....oRo....
.....o.....
...........
...........
...........
...........
"""
GUN_UP = """
..........
..........
..........
..........
..........
..........
..........
....o.....
...oRo....
...oRo....
...oro....
...ooo....
..odcbo...
..odcbTT..
..odcTEDT.
..odcTDCT.
..odcTCBT.
..odbbTT..
..odcbo...
..obbao...
...ooo....
..........
"""

RPAL = {'w': Mat(WHITE, z=5, flat=5)}
RPAL.update(fam('oabcde', BALL_YEL, 2, 'r_body'))
RPAL.update(fam('TABCDE', WATER, 3, 'r_tank'))
RPAL.update(fam('rR', NOZZLE, 2, 'r_noz', tones=(2, 4)))
RPAL.update(fam('Ppq', ORANGE, 3, 'r_pump', tones=(0, 2, 4)))
RPAL.update(fam('x', STRAP, 1, 'r_strap', tones=(3,)))

SOAKER_BODY = """
.oooooooooooooooooooooo.
oeddddddddddddddddddddco
odcccccccccccccccccccbbo
obbbbbbbbbbbbbbbbbbbbaao
.oooooooooooooooooooooo.
"""
SOAKER_TANK = """
.TTTTTTTTTT.
TEEDDDCCCBBT
TDDCCCBBBAAT
TCCBBBAAAAAT
.TTTTTTTTTT.
"""
SOAKER_NOZZLE = """
.ooooo
rRRRRo
.ooooo
"""
SOAKER_PUMP = """
PPPPPPPPP
PqqqqqqqP
PPPPPPPPP
"""
SOAKER_GRIP = """
oaao
odbo
odbo
oooo
"""
SOAKER_STOCK = """
.oooo..
odccbo.
odcbbao
obbbaao
.oooooo
"""


def soaker():
    return compose(32, 13, [
        (SOAKER_GRIP, 17, 8),
        (SOAKER_PUMP, 4, 8),
        (SOAKER_BODY, 5, 4),
        (SOAKER_STOCK, 25, 4),
        (SOAKER_NOZZLE, 0, 5),
        (SOAKER_TANK, 12, 0),
    ])


RIFLE_V0 = """
..ooooo..
..odcbo..
..odcbo..
..odcbo..
.TTTcbo..
TEDCTbo..
TDCBTbo..
TCBATbo..
TCBATbo..
TBAATbo..
TBAATbo..
.TTTcbo..
..odcbo..
..odcbooo
..odcboao
..odcboao
..odcbooo
..odcbo..
.PPPPPPP.
.PqqqqqP.
.PqqqqqP.
.PPPPPPP.
..odcbo..
...orRo..
...oRro..
...oRro..
...oooo..
.........
"""
RIFLE_V1 = """
....oo....
...oRRo...
...oRro...
...oRro...
...odco...
..PPPPPPP.
..PqqqqqP.
..PqqqqqP.
..PPPPPPP.
...odcbo..
...odcbooo
...odcboao
...odcboao
...odcbooo
...odcbo..
..TTTcbo..
.TEDCTbo..
.TDCBTbo..
.TCBATbo..
.TCBATbo..
.TBAATbo..
.TBAATbo..
..TTTcbo..
...odcbo..
...odcbo..
...odcbo..
...odcbo..
...odcbo..
...ooooo..
..........
"""
RIFLE_SLUNG = """
...............
......oo.......
.....oRRo......
.....oRro......
....PPPPPP.....
...xPqqqqP.....
..x.PPPPPP.....
.x...odcbo.....
.x...odcbo.....
.x..TTTcbo.....
.x.TEDCTbo.....
.x.TDCBTbo.....
.x.TCBATbo.....
..xTBAATbo.....
...xTTTcbo.....
....xodcbo.....
.....odcbo.....
.....odcbx.....
.....odcbox....
.....odcbo.x...
.....odcbo..x..
.....oooooo..x.
...............
...............
"""
SHOT = """
.EEDC.C.B..
EwwEDDCCBB.
.EEDC.C.B..
"""
SHOT_DIAG = """
......B
....C.B
...DC..
..EDC..
.wED...
wwE....
.E.....
"""


def guns():
    out = {}
    out['weapon_gun'] = kit.weapon('gun', kit.straight(pistol(), GPAL, 'gun'),
                                   hand={2: kit.paint(GUN_DOWN, GPAL, (11, 24)), 4: kit.paint(GUN_UP, GPAL, (10, 22))})
    out['weapon_rifle'] = kit.weapon('rifle', kit.straight(soaker(), RPAL, 'rifle'),
                                     hand={0: kit.paint(RIFLE_V0, RPAL, (9, 28)), 1: kit.paint(RIFLE_V1, RPAL, (10, 30)),
                                           4: kit.paint(RIFLE_SLUNG, RPAL, (15, 24)), 5: kit.paint(SHOT, RPAL, (11, 3)),
                                           6: kit.paint(SHOT_DIAG, RPAL, (7, 7))})
    return out


def build():
    parts = {}
    parts['armor'] = P.torso()
    parts['gloves'] = P.arms(BAND)
    parts['shoes'] = P.legs(LEGPAL, stripes)
    parts['helmet'] = kit.frames([HELM0, HELM1, HELM2, HELM3], HPAL, 'helmet')
    parts.update(P.toys(BLADE, GUARD, STRIPE, FIN, POCKET))
    parts.update(guns())
    return parts
