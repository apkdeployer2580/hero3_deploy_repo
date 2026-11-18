"""15 검은 정장의 킬러 — 리츠 전용 (하드코어 엔딩 보상). A legendary hired killer: dark indigo hair parted
and swept back, falling to the jaw at the sides and to the nape behind, a short full beard; a black
three-piece suit — a single-breasted jacket over a charcoal waistcoat, a white shirt and a black tie,
a silver lapel pin — white shirt cuffs, black pinstriped trousers and glossy black shoes; a katana with
a plain black grip, a silver pistol, a pump shotgun, a slim black lance, a knuckle knife and a
bullet-proofed black briefcase for a shield. (Reference: the user's screenshot 2, 2026-10-09.)"""
from engine import Mat
import kit
import _shared_hair as hair

NUM = '15'
FOLDER = '15_hitman'
HERO = 'ritz'
TEXT = """# 의상 15 — 하드코어 엔딩 보상 · 리츠 전용. 175차: 사용자 참고 그림 2 (넘긴 남색 장발 · 수염 · 검은 쓰리피스 정장 · 넥타이 · 일본도 · 은빛 권총).
# 투구 시트가 얼굴까지 머리 전체를 그린다(171차 head = own) — 게임은 이 세트의 투구를 입은 동안 주인공 자신의 머리를 안 그린다.
head = own
# tools_re/costume_atelier/sets/hitman.py 가 프레임마다 그린다(재채색 없음).
set            = 검은 정장의 킬러
acquire        = clear.hard
hero           = ritz
helmet         = 킬러의 머리
helmet.desc    = 뒤로 넘긴 남색 장발과 수염
armor          = 킬러의 정장
armor.desc     = 넥타이를 맨 검은 쓰리피스
gloves         = 킬러의 소매
gloves.desc    = 흰 커프스를 단 정장 소매
shoes          = 킬러의 하의
shoes.desc     = 검은 정장 바지와 구두
weapon         = 킬러의 무기
weapon.desc    = 손에 익은 일격필살의 무기
weapon.spear   = 킬러의 창
weapon.sword   = 킬러의 일본도
weapon.knife   = 킬러의 단검
weapon.gun     = 킬러의 은빛 권총
weapon.rifle   = 킬러의 산탄총
shield         = 킬러의 서류가방
shield.desc    = 방탄 처리한 검은 서류가방
"""

# ── palettes (tone 0 outline · 1 deep · 2 shadow · 3 mid · 4 light · 5 highlight) ─────────────────
SKIN = '310810 5a2829 734041 a46962 e3978a f6dac5'      # Ritz's own skin
SUIT = '050507 101014 1b1b22 282832 3a3a48 56566c'      # the jacket (neutral black, a cool sheen)
VEST = '0a0a0e 15151b 202028 2c2c36 3c3c4a 545466'      # the charcoal waistcoat
TROUS = '060608 111116 1d1d24 2a2a33 3c3c48 545464'     # trousers (a hair lighter than the jacket)
SHIRT = '2a2a34 58586a 9494a6 c6c6d4 e8e8f2 ffffff'
TIE = '020203 06060a 0c0c12 16161e 22222c 34343f'
SHOE = '020203 08080c 121218 1e1e28 34344a 6e6e88'      # glossy black, a blue sheen
HAIR = '050409 0d0b17 17142a 241f3c 353053 4e4774'      # dark indigo, nearly black
STEEL = '1a1c24 363a46 5a6070 8a92a4 c4cad8 f4f6fa'     # the pistol, the blade
FLASH = '7a2a04 c45a0a f08a1a ffc040 fff090 ffffff'

# ── torso: the jacket's lapels open over the waistcoat, the shirt and the tie, a silver lapel pin ──
TORSO = {}
TORSO.update(kit.family('012345', SUIT, z=2, fam='suit'))
TORSO.update(kit.family('vV', VEST, z=3, fam='vest', tones=(2, 4)))
TORSO.update(kit.family('wW', SHIRT, z=3, fam='shirt', tones=(3, 5)))
TORSO.update(kit.family('nN', TIE, z=4, fam='tie', tones=(1, 3)))
TORSO.update(kit.family('G', STEEL, z=4, fam='pin', tones=(5,)))
TORSO.update(kit.family('b', SUIT, z=3, fam='button', tones=(0,)))

TORSO_FRAMES = [
    # 0 back: the jacket's back, the centre seam opening into the vent
    """
............
............
.0444444440.
044444433220
044443433220
..04433320..
..04432320..
..04431320..
...043120...
............
""",
    # 1 front: the lapels open in a deep V over the white shirt and the black tie (the waistcoat
    #    showing at its edges), the silver lapel pin, the one button the jacket is closed with
    """
............
............
.044WWWW440.
0444WnNW4420
04G3WnNW3320
..043vNv320.
..0433b3320.
..04333320..
...043320...
............
""",
    # 2 side (faces left): the shirt and the tie at the front edge, the lapel, the jacket behind
    """
...........
...........
...04440...
..WW44430..
..NW443320.
..nWv43320.
..0vb43220.
..04332220.
...433220..
...03220...
...........
""",
]

# ── arms: the jacket's sleeves, white shirt cuffs, bare hands ────────────────────────────────────
APAL = {
    'U': Mat(SUIT, z=3, fam='sleeve'),
    'L': Mat(SUIT, z=3, fam='sleeve'),
    'C': Mat(SHIRT, z=4, flat=4, fam='cuff'),         # the template's cuff is a dark band — the shirt cuff is pinned white
    'H': Mat(SKIN, z=4),
}
ARMMAP = {'U': 'U', 'L': 'L', 'C': 'C', 'H': 'H'}

# ── legs: the jacket's skirts over the hips, pinstriped black trousers, glossy black shoes ─────────
PAL = {
    'J': Mat(SUIT, z=3),
    'P': Mat(TROUS, z=2, fam='trous'),
    'p': Mat(TROUS, z=2, fam='trous', shift=1),        # the pale pinstripe
    'B': Mat(SHOE, z=3, gloss=True),
}
LEGMAP = {'W': 'J', 'T': 'P', 't': 'P', 'U': 'P', 'u': 'P', 'S': 'P', 's': 'P', 'A': 'P', 'a': 'P',
          'F': 'B', 'f': 'B'}
JACKET_ROWS = 3       # the jacket hangs this many rows below the top of the seat zone


def post_legs(g, i):
    seat = [y for y in range(g.h) for x in range(g.w) if g.keys[y][x] == 'J']
    top = min(seat) if seat else 0
    for y in range(g.h):
        for x in range(g.w):
            k = g.keys[y][x]
            if k == 'P' and y <= top + JACKET_ROWS:
                g.keys[y][x] = 'J'
            elif k == 'P' and x % 3 == 0:
                g.keys[y][x] = 'p'


# ── helmet: dark indigo hair parted in the middle and swept back, falling to the jaw at the sides and
#    to the nape behind; a short full beard and moustache drawn over the jaw. Locks from the crown
#    (`_shared_hair.render`), then the beard and the parting by hand. ─────────────────────────────
HPAL = kit.hair_pal(HAIR, extra={
    's': Mat(SKIN, z=2, flat=3, fam='skin'),
    'S': Mat(SKIN, z=2, flat=4, fam='skin'),
})


def beard(pts, tone='1'):
    return {p: tone for p in pts}


def pins(spec):
    """'x,y:t x,y:t …' -> {(x, y): t} (t = a tone digit, 's'/'S' skin, '.' clear)."""
    out = {}
    for item in spec.split():
        xy, t = item.split(':')
        x, y = xy.split(',')
        out[(int(x), int(y))] = t
    return out


HELM0 = hair.render('ritz', 0, (10.5, 11.0), [
    (6.0, 10.0, 3.0), (10.5, 9.0, 3.2), (15.0, 9.5, 3.0), (18.0, 12.5, 2.8), (18.6, 18.0, 2.6),
    (17.5, 24.5, 2.4), (13.5, 27.6, 2.4), (8.5, 27.6, 2.4), (4.5, 25.0, 2.4), (2.6, 18.5, 2.6),
    (3.2, 13.0, 2.8)], swirl=0.03, band=(0.04, 0.2, 0.5), smooth=True)

# 3/4: the forehead bared by the swept-back hair, a short beard along the jaw and the chin
HELM1 = hair.render('ritz', 1, (9.0, 11.5), [
    (5.0, 11.0, 3.0), (9.5, 10.0, 3.2), (13.5, 11.0, 3.0), (17.0, 13.5, 2.8), (18.3, 18.0, 2.6),
    (17.5, 24.0, 2.2), (15.5, 26.8, 1.8), (4.0, 24.8, 1.6), (1.6, 19.0, 2.4), (2.0, 14.0, 2.6)],
    swirl=0.02, band=(0.04, 0.2, 0.45), smooth=True,
    pins=pins('6,19:S 7,19:S 8,19:S 9,19:s 7,18:s 8,18:s '
              '11,23:1 11,24:1 8,24:2 9,24:2 7,25:1 8,25:1 9,25:1 10,25:1'))

# side (faces left): the hairline pulled back off the brow, the beard along the jaw to the chin
HELM2 = hair.render('ritz', 2, (11.0, 12.0), [
    (6.0, 10.0, 3.0), (11.0, 9.5, 3.2), (15.5, 11.0, 3.0), (17.4, 14.5, 2.8), (17.4, 20.0, 2.6),
    (15.0, 26.0, 2.4), (11.0, 27.0, 2.0), (2.5, 25.0, 1.6), (0.6, 19.0, 2.4), (1.0, 13.5, 2.6),
    (3.0, 10.5, 2.8)], swirl=0.05, band=(0.04, 0.2, 0.45), smooth=True,
    pins=pins('4,19:S 5,19:S 6,19:s 5,18:s '
              '8,22:1 8,23:1 6,23:2 7,23:2 6,24:1 7,24:1 8,24:1'))

# front: parted in the middle and swept back, the forehead bared, strands falling to the jaw; a beard
HELM3 = hair.render('ritz', 3, (8.5, 10.0), [
    (4.5, 9.0, 3.0), (8.5, 9.2, 3.2), (12.5, 10.0, 3.0), (15.2, 11.8, 2.8), (16.2, 16.5, 2.6),
    (15.0, 24.5, 2.0), (12.6, 24.8, 1.4), (4.0, 24.8, 1.4), (1.6, 24.0, 2.0), (1.6, 16.0, 2.6),
    (2.2, 11.0, 2.8)], swirl=0.0, band=(0.04, 0.2, 0.45), smooth=True,
    pins=pins('7,18:S 8,18:S 9,18:S 10,18:s 8,17:s 9,17:s '
              '8,22:2 9,22:2 6,23:1 7,23:1 8,23:2 9,23:2 10,23:1 11,23:1 8,24:1 9,24:1'))

# ── weapons ──────────────────────────────────────────────────────────────────────────────────────
INK = '040406 040406 040406 040406 040406 040406'
WPAL = {'k': Mat(INK, flat=0)}
WPAL.update(kit.family('nsSW', STEEL, z=2, fam='w_steel', tones=(1, 3, 4, 5)))
WPAL.update(kit.family('h', SHIRT, z=2, fam='w_hamon', tones=(4,)))
WPAL.update(kit.family('xbB', SUIT, z=2, fam='w_black', tones=(1, 3, 5)))
WPAL.update(kit.family('gG', STEEL, z=2, fam='w_trim', tones=(2, 4)))
WPAL.update(kit.family('fFY', FLASH, z=2, fam='w_flash', tones=(2, 4, 5)))

# the katana: a plain black grip with dark wraps, a round silver guard, a long bright blade
SWORD = """
........kk......................................
.kkkkkkkGGk.....................................
kbxbxbxkGWgkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk..
kxbxbxbkGGgsssssssssssssssssssssssssssssssssssk.
kbxbxbxkgGgSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSWk.
kxbxbxbkggkhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhhWWk..
.kkkkkkkkkWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWkkk....
..........kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk.......
"""

# the lance: a slim leaf-shaped steel head, a silver collar, a black shaft
SPEAR = """
............................................
............................................
.......kkkkkk...............................
...kkkkWWWWSSkk.kkk.........................
kkkWWWWSSSSSSsskgGGkkkkkkkkkkkkkkkkkkkkkkkkk
...kkkkssssssnnkgGgbbbbbbbbbbbbbbbbbbbbbbbbb
.......kkkkkk..kkkkkkkkkkkkkkkkkkkkkkkkkkkkk
............................................
............................................
"""

SHAFT = """
kkkkkkkkkkkkkkkkkkkk
bbbbbbbbbbbbbbbbbbbb
kkkkkkkkkkkkkkkkkkkk
"""

# the knuckle knife: a black knuckle guard at the upper right, a straight steel blade to the lower left
CLAW = """
..................................
..................................
..................................
.......................kkkkkk.....
......................kbBBBbbk....
.....................kbBkkkBbk....
.....................kbkk.kbbk....
.....................kBbkkkbbk....
....................kkbBBBbbk.....
..................kkSWkbbbbk......
................kkSWWskkbkk.......
..............kkSWWssnk.kk........
............kkSWWssnkk............
..........kkSWWssnkk..............
........kkSWWssnkk................
......kkSWWssnkk..................
.....kSWWssnkk....................
....kWWssnkk......................
....kkkkkk........................
..................................
..................................
"""

# the pistol: a long silver slide, a black frame and grip
GUN = """
..........................
..........................
..........................
.....kkkkkkkkkkkkkkkkkkk..
.....kWWWWWWWWWWWWWWWWWSk.
.....kSSSSSSSSSSSSSSSSSSk.
....kknnnnnnnnnnnnnnnnnnk.
...kbbkkSSSSkkkkkkkkkkkk..
...kbBbkkkkSk.............
...kbBbbk.kSk.............
...kbBbbkkkk..............
...kbbbbk.................
...kbbbbk.................
....kkkk..................
..........................
"""

GUN_DOWN = """
...........
...........
...........
...........
....kkkk...
...kbbbbk..
...kbBbbk..
...kbBbbkkk
...kbbbkSk.
...kknkkSk.
....kWSnk..
....kWSnk..
....kWSnk..
....kWSnk..
....kWSnk..
....kWSnk..
....kWSnk..
....kWSnk..
....kkkkk..
...........
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
...kkkkk..
...kWSnk..
...kWSnk..
...kWSnk..
...kWSnk..
...kWSnk..
...kWSnk..
...kWSnk..
..kkknkk..
.kSkbbbk..
kkkbbBbk..
..kbbBbk..
..kbbbbk..
...kkkk...
..........
"""

# the pump shotgun: a silver barrel over a black magazine tube, a ribbed pump, a black stock
RIFLE = """
................................
................................
................................
kkkkkkkkkkkkkkkkkkkkkkkk........
kWWWWWWWWWWWWWWWWWWWWWSkkkkkk...
kSSSSSSSSSSSSSSSSSSSSSSkbbbbbkk.
kkkkkkkkkkkkkkkkkkkkkkkkbBbbbbbk
.kbbbbxbxbxbxbbbbbbbbbkkbbbbbbbk
.kkkkkkkkkkkkkkkkkkkkkkkbbbbBbbk
.......................kkbbbbbk.
.........................kkbbk..
...........................kk...
................................
"""

RIFLE_V0 = """
..kkkk...
..kbbbk..
..kbBbk..
..kbbbk..
..kbbbk..
..kbBbk..
..kbbbk..
..kkkkk..
..kSWkk..
..kSWbk..
..kSWxk..
..kSWbk..
..kSWxk..
..kSWbk..
..kSWxk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kSWbk..
..kkkkk..
.........
"""

RIFLE_V1 = """
...kkkkk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWbk..
...kSWxk..
...kSWbk..
...kSWxk..
...kSWbk..
...kSWxk..
...kSWbk..
...kkkkk..
...kbbbk..
...kbBbk..
...kbbbk..
...kbbbk..
...kbBbk..
...kbbbk..
...kbbbk..
...kbBbk..
...kbbbk..
...kkkk...
..........
"""

RIFLE_SLUNG = """
.......kk......
......kSWk.....
......kSWk.....
......kSWk.....
......kSWk.....
....xkSWbk.....
...x.kSWxk.....
..x..kSWbk.....
..x..kSWxk.....
..x..kSWbk.....
..x.kkkkkk.....
..x.kbbbbk.....
..x.kbBbbk.....
...xkbbbbk.....
....kbbBbk.....
....kbbbbk.....
....kbBbbk.....
....kbbbbbk....
....kkbbbbk....
.....kbbbbk....
.....kbBbbk....
.....kbbbk.....
......kkk......
...............
"""

SHOT = """
.YFffF.....
YYYFFFff...
.YFffF.....
"""

SHOT_DIAG = """
......F
....fFf
...FFf.
..YFf..
.YYf...
YYF....
.Y.....
"""

# the briefcase: black leather, a handle on top, two silver latches, a stitched rim
SHIELD = """
....................
......kkkkkk........
......kgkkgk........
..kkkkkkkkkkkkkkk...
.kBBBBBBBBBBBBBBBk..
.kbbGbbbbbbbbbGbbk..
.kbbgbbbbbbbbbgbbk..
.kxxxxxxxxxxxxxxxk..
.kbbbbbbbbbbbbbbbk..
.kbbbbbbbbbbbbbbbk..
.kbbbbbbbbbbbbbbxk..
..kkkkkkkkkkkkkkk...
....................
....................
"""


def weapons():
    out = {}
    out['weapon_sword'] = kit.weapon('sword', kit.straight(SWORD, WPAL, 'sword'))
    out['weapon_spear'] = kit.weapon('spear', kit.straight(SPEAR, WPAL, 'spear'),
                                     hand={'shaft': kit.straight(SHAFT, WPAL, 'shaft')})
    out['weapon_knife'] = kit.weapon('knife', kit.straight(CLAW, WPAL, 'knife'))
    out['weapon_gun'] = kit.weapon('gun', kit.straight(GUN, WPAL, 'gun'),
                                   hand={2: kit.paint(GUN_DOWN, WPAL, (11, 24)), 4: kit.paint(GUN_UP, WPAL, (10, 22))})
    out['weapon_rifle'] = kit.weapon('rifle', kit.straight(RIFLE, WPAL, 'rifle'),
                                     hand={0: kit.paint(RIFLE_V0, WPAL, (9, 28)), 1: kit.paint(RIFLE_V1, WPAL, (10, 30)),
                                           4: kit.paint(RIFLE_SLUNG, WPAL, (15, 24)), 5: kit.paint(SHOT, WPAL, (11, 3)),
                                           6: kit.paint(SHOT_DIAG, WPAL, (7, 7))})
    return out


def build():
    parts = {}
    parts['armor'] = kit.frames(TORSO_FRAMES, TORSO, 'armor')
    parts['gloves'] = kit.arms(ARMMAP, APAL)
    parts['shoes'] = kit.legs(LEGMAP, PAL, [], post=post_legs)
    parts.update(weapons())
    parts['helmet'] = kit.frames([HELM0, HELM1, HELM2, HELM3], HPAL, 'helmet')
    parts['shield'] = kit.frames([SHIELD], WPAL, 'shield')
    return parts
