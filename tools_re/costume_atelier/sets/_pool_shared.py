"""Parts shared by 06 수영장 파티 케이 and 07 수영장 파티 리츠 (a summer pool party, drawn pixel by pixel):
bare skin in the heroes' own tones, a hibiscus lei, swim trunks over bare legs and flip-flops, and the
pool toys both heroes carry — a beach parasol (spear), a twisted balloon sword, swim flippers (claws) and
a beach ball (shield). Each hero's module picks the colours and draws what is his alone (sunglasses,
the trunks' print, Kei's water circles, Ritz's water guns)."""
from engine import Mat
import kit

# ── skin: the heroes' own face/arm tones (d5898b · eea18b · f6dac5), shadows cooler ──────────────────
SKIN = '310810 7c3a3c b4665c de8e7e f2ae96 fcdcc6'

# ── the lei: hibiscus pink, plumeria yellow and white, glossy leaves ──────────────────────────────────
PINK = '4a0c2a 921e56 d43a7e f66aa6 ffa6cc fff0f8'
YELLOW = '5a3600 a06c00 e0a410 ffd23a fff08a fffce0'
WHITE = '4e4e6a 8a8aa8 c0c0d8 e6e6f2 ffffff ffffff'
LEAF = '082a10 145418 23842a 42b236 7ede58 c4ffa0'

# ── toys ──────────────────────────────────────────────────────────────────────────────────────────────
BALL_RED = '4a0610 8e1420 cc2a30 f04a46 ff9282 fff0ea'
BALL_YEL = '603a00 a87400 e4b000 ffe040 fff4a0 fffff0'
BALL_BLUE = '0a2660 164ea6 2476de 46a4fe 9ad4ff f0faff'
BALL_WHITE = '4a4a64 8c8ca6 c2c2d6 e8e8f4 ffffff ffffff'
INK = '120a14 120a14 120a14 120a14 120a14 120a14'
WOOD = '2e1a0a 5c3a1c 8e6436 c0945e e6c48e fff0d4'
RUBBER = '06080e 10141e 1e2434 2e3850 4a5878 7486a8'


def fam(keys, ramp, z, name, tones=None):
    return kit.family(keys, ramp, z=z, fam=name, tones=tones)


def compose(w, h, stamps):
    """A text grid w x h from (block, x, y) stamps in order ('.' keeps what is below)."""
    cv = [['.'] * w for _ in range(h)]
    for block, ox, oy in stamps:
        rows = [r for r in block.strip('\n').split('\n')]
        for y, r in enumerate(rows):
            for x, c in enumerate(r):
                if c not in '. ' and 0 <= x + ox < w and 0 <= y + oy < h:
                    cv[y + oy][x + ox] = c
    return '\n'.join(''.join(r) for r in cv)


# ── torso: bare chest, the lei round the neck ─────────────────────────────────────────────────────────
TORSO = {}
TORSO.update(fam('012345', SKIN, 2, 'skin'))
TORSO.update(fam('pPQ', PINK, 4, 'lei', tones=(2, 3, 4)))
TORSO.update(fam('yYZ', YELLOW, 4, 'lei', tones=(2, 3, 4)))
TORSO.update(fam('wWV', WHITE, 4, 'lei', tones=(2, 3, 4)))
TORSO.update(fam('gG', LEAF, 4, 'lei', tones=(2, 3)))

TORSO_FRAMES = [
    # 0 back: the lei across the back of the neck, shoulder blades, the spine
    """
............
............
..PQgVWgZY..
..pPgwWgYy..
..04434430..
..04323320..
..03323220..
..03223210..
...000000...
............
""",
    # 1 front: four flowers hanging in a U from the shoulders, the chest below
    """
............
............
..QP3443VW..
..PpG44GWw..
..0ZY43QP0..
..0yY32Pp0..
..04333220..
..03432220..
...000000...
............
""",
    # 2 side (faces left): the lei over the shoulder and down the chest
    """
...........
...........
..34433QP0.
..VWG43Pp0.
..WwYZ3320.
..34yY3210.
..44333210.
..34332210.
...00000...
...........
...........
""",
]


def torso():
    return kit.frames(TORSO_FRAMES, TORSO, 'armor')


# ── arms: bare, a wristband ───────────────────────────────────────────────────────────────────────────

def arms(band):
    pal = {'S': Mat(SKIN, z=2, shift=1), 'B': Mat(band, z=3, flat=3)}
    return kit.arms({'U': 'S', 'L': 'S', 'C': 'B', 'H': 'S'}, pal)


# ── legs: trunks (W hip + T/t thighs), bare legs, flip-flops ──────────────────────────────────────────
LEGMAP = {'W': 'R', 'T': 'R', 't': 'R', 'U': 'S', 'u': 'S', 'S': 'S', 's': 'S', 'A': 'S', 'a': 'S',
          'F': 'X', 'f': 'x'}


def legs_pal(trunks, sole, strap, print_keys):
    pal = {
        'R': Mat(trunks, z=3),
        'S': Mat(SKIN, z=2, shift=1),
        'N': Mat(sole, z=4, flat=1),
        'n': Mat(sole, z=4, flat=3),
        'v': Mat(strap, z=4, flat=4),
    }
    pal.update(print_keys)
    return pal


def flipflops(g):
    """The feet (X near, x far) -> skin toes on a two-tone sole, a thong strap across the instep."""
    for foot in ('X', 'x'):
        cells = [(x, y) for y in range(g.h) for x in range(g.w) if g.keys[y][x] == foot]
        if not cells:
            continue
        cols = {}
        for x, y in cells:
            cols.setdefault(x, []).append(y)
        for x, ys in cols.items():
            ys.sort()
            for y in ys:
                g.keys[y][x] = 'S'
                g.tones[y][x] = max(2, g.tones[y][x] or 3)
            g.keys[ys[-1]][x] = 'N'
            if len(ys) >= 3:
                g.keys[ys[-2]][x] = 'n'
        # the strap: the top row of the foot's middle third, where skin is
        top = min(y for _, y in cells)
        xs = sorted(x for x, y in cells if y == top + 1) or sorted(x for x, y in cells if y == top)
        row = top + 1 if any(y == top + 1 for _, y in cells) else top
        if xs:
            mid = xs[len(xs) // 2]
            for x in (mid - 1, mid):
                if 0 <= x < g.w and g.keys[row][x] == 'S':
                    g.keys[row][x] = 'v'
    return g


def interior(g, x, y, key):
    return all(0 <= x + dx < g.w and 0 <= y + dy < g.h and g.keys[y + dy][x + dx] == key
               for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))


def legs(pal, post):
    def p(g, i):
        flipflops(g)
        post(g, i)
    return kit.legs(LEGMAP, pal, post=p)


# ── weapons ───────────────────────────────────────────────────────────────────────────────────────────

def balloon_pal(blade, guard):
    pal = {'w': Mat(WHITE, z=5, flat=5)}
    pal.update(fam('k12345', blade, 2, 'w_blade'))
    pal.update(fam('KABCDE', guard, 3, 'w_guard'))
    return pal


BULB_HANDLE = """
...kkk...
.kk454kk.
k4454332k
k3333221k
.kk221kk.
...kkk...
"""
BULB = """
...kkkkkkkkkkk...
.kk45w54444333kk.
k445544433333322k
k433333333222221k
.kk22222221111kk.
...kkkkkkkkkkk...
"""


GUARD = """
.KKKKK.
KDEEDCK
KEDDCBK
.KCCBK.
.KCCBK.
KCCBBAK
KBBBAAK
.KKKKK.
"""
KNOT = """
.k.
k4k
k2k
.k.
"""
TAIL = """
.kk.
k32k
.kk.
"""


def balloon_sword():
    return compose(48, 8, [
        (BULB_HANDLE, 2, 1),   # the handle bulb
        (KNOT, 0, 2),          # the tied knot at the pommel
        (BULB, 13, 1),         # blade bulb 1
        (BULB, 29, 1),         # blade bulb 2 (the shared column is the pinched waist)
        (TAIL, 44, 2),         # the little uninflated tail at the tip
        (GUARD, 8, 0),         # the yellow loops of the guard, tied round the blade
    ])


def parasol_pal(stripe):
    pal = {}
    pal.update(fam('oabcde', stripe, 2, 'w_canopy'))
    pal.update(fam('ZYXWV', WHITE, 2, 'w_canopy', tones=(1, 2, 3, 4, 5)))
    pal.update(fam('Ppqr', WOOD, 2, 'w_pole', tones=(0, 2, 3, 4)))
    pal.update(fam('mM', INK, 2, 'w_tip', tones=(0, 0)))
    pal['s'] = Mat(BALL_YEL, z=2, flat=3, fam='w_canopy')
    pal['h'] = Mat(BALL_YEL, z=2, flat=5, fam='w_canopy')
    return pal


CANOPY = """
.............oooosos
..........ooodddddhs
.......ooocWWWWWWohs
...oooocccccccccbohs
oeddcccccccccccbbbos
...ooooabbbbbbbbbohs
.......oooaXXXXXYohs
..........oooabbaohs
.............oooosos
"""
KNOB = """
.PP.
PrqP
PqpP
.PP.
"""
POLE = """
PPPPPPPPPPPPPPPPPPPPP
rrqqqqqqqqqqqqqqqqqqq
PPPPPPPPPPPPPPPPPPPPP
"""


def parasol():
    return compose(44, 9, [
        (POLE, 23, 3),
        (CANOPY, 4, 0),
        (KNOB, 0, 3),
    ])


SHAFT = """
PPPPPPPPPPPPPPPPPPPP
qqqqqqqPqqqqqqqqqqqq
PPPPPPPPPPPPPPPPPPPP
"""


def fin_pal(fin, rubber):
    pal = {}
    pal.update(fam('oabcde', fin, 2, 'w_fin'))
    pal.update(fam('qrstu', rubber, 3, 'w_pocket', tones=(0, 1, 2, 3, 4)))
    return pal


FLIPPER = """
..................................
..................................
.........................qqqq.....
.......................qquuuuqq...
.....................qqtuutttq....
...................oottttttssq....
..............oooooccatttsssqq....
.......oooooooddccbbcasssrqq......
.oooooodddddcccbbbcbbaorqq........
.oddddddccccbbbccbbcao............
..odcccccbbbbccbbcaoo.............
..occcbbbbcccbbccao...............
...oooobcccbbccaoo................
.......ocbbbcaao..................
.......obbccaao...................
......obccaaoo....................
.....occcaao......................
.....ocaaao.......................
.....oaaao........................
......oao.........................
......oo..........................
"""

BALL_PAL = {'k': Mat(INK, z=2, flat=0)}
BALL_PAL['R'] = Mat(BALL_RED, z=2, fam='ball')
BALL_PAL['Y'] = Mat(BALL_YEL, z=2, fam='ball')
BALL_PAL['B'] = Mat(BALL_BLUE, z=2, fam='ball')
BALL_PAL['W'] = Mat(BALL_WHITE, z=2, fam='ball')

BEACH_BALL = """
.....kkkk........... | .....0000...........
...kkYYWWkk......... | ...00444500.........
..kRYWWWWBBk........ | ..0453455330........
.kRRRWWWBBBBk....... | .044544433220.......
.kRRRWWWBBBBk....... | .044444332220.......
.kRRRWWWBBBBk....... | .044443332220.......
.kRRRWWWWBBBk....... | .044433322220.......
.kRRWWWWWBBBk....... | .033333322220.......
.kRRWWWWWWBBk....... | .033332222220.......
.kRRWWWWWWBBk....... | .032222222220.......
..kRRWWWWWBk........ | ..0222222220........
...kkWWWWkk......... | ...00222200.........
.....kkkk........... | .....0000...........
.................... | ....................
"""


def beach_ball():
    return kit.frames([BEACH_BALL], BALL_PAL, 'shield')


def toys(blade, guard, stripe, fin, rubber):
    out = {}
    out['weapon_sword'] = kit.weapon('sword', kit.straight(balloon_sword(), balloon_pal(blade, guard), 'sword'))
    pp = parasol_pal(stripe)
    out['weapon_spear'] = kit.weapon('spear', kit.straight(parasol(), pp, 'spear'),
                                     hand={'shaft': kit.straight(SHAFT, pp, 'shaft')})
    out['weapon_knife'] = kit.weapon('knife', kit.straight(FLIPPER, fin_pal(fin, rubber), 'knife'))
    out['shield'] = beach_ball()
    return out
