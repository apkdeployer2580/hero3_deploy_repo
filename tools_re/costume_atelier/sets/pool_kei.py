"""06 수영장 파티 케이 — 케이 전용, 기본 지급. A summer pool party: sporty wraparound sunglasses with blue
mirrored lenses (Kei's own hair stays), bare chest with a hibiscus lei, blue floral swim trunks and
flip-flops; pool toys for weapons — a striped beach parasol, a twisted balloon sword, swim flippers, a
splash ring and a ring of soap bubbles, and a beach ball. Every pixel drawn here (169차)."""
import math

from engine import Mat
import circles
import kit
from _pool_shared import fam, YELLOW, WHITE
import _pool_shared as P

NUM = '06'
FOLDER = '06_pool_kei'
HERO = 'kei'
HELMET_COVERS_HAIR = False
TEXT = """# 의상 6 — 기본 지급 · 케이 전용 (hero = kei: 다른 주인공의 옷장에는 안 나온다).
# 169차: 사용자 요청으로 재채색 없이 프레임마다 새로 그렸다 — tools_re/costume_atelier/sets/pool_kei.py.
set            = 수영장 파티 케이
acquire        = starter
hero           = kei
helmet         = 수영장 선글라스
helmet.desc    = 파란 렌즈의 선글라스
armor          = 수영장 꽃목걸이
armor.desc     = 맨몸에 건 꽃목걸이
gloves         = 수영장 맨팔
gloves.desc    = 소매 없이 드러낸 팔
shoes          = 수영장 수영복
shoes.desc     = 꽃무늬 파란 수영복과 샌들
weapon         = 수영장 장난감
weapon.desc    = 물놀이 장난감 무기
weapon.spear   = 수영장 파라솔
weapon.sword   = 수영장 풍선 검
weapon.knife   = 수영장 물갈퀴
weapon.dark    = 수영장 물보라
weapon.holy    = 수영장 비눗방울
shield         = 수영장 비치볼
shield.desc    = 알록달록한 비치볼
"""

TRUNKS = '0a1838 13306a 1f4e9c 2e74c8 4c9ce6 8ccaf8'
SOLE = '0a1838 1a3a7a 2c64b8 4a90e0 80c0ff c0e4ff'
BAND = '0a1838 13306a 1f4e9c 2e74c8 4c9ce6 8ccaf8'
FRAME = '0a0e1e 141c34 22305a 34488a 5068b0 7a90d0'
LENS = '061030 0c2a6a 1650b0 2a88e8 70c8ff e0f8ff'

BLADE = '0a2050 1446a0 1e70d8 3aa0ff 8ad0ff eaf8ff'
GUARD = '6a3c00 b07400 eaa800 ffd23a fff08a fffbe0'
STRIPE = '0c2458 1a48a0 2a72d8 4aa0f8 9ad4ff e8f8ff'
FIN = '5a3a00 a07000 e0a800 ffd040 fff090 fffde8'
POCKET = '061030 0e2458 1a3c86 2a5cb4 4a86da 8ab8f4'

# ── sunglasses: a one-piece sport shield, blue mirror, a dark frame ──────────────────────────────────
HPAL = {'w': Mat(WHITE, z=5, flat=5)}
HPAL.update(fam('kmn', FRAME, 3, 'frame', tones=(0, 2, 4)))
HPAL.update(fam('abcde', LENS, 3, 'lens', tones=(1, 2, 3, 4, 5)))

E21 = '.' * 21
E23 = '.' * 23
E18 = '.' * 18
HELM0 = '\n'.join([E21] * 19 + [
    '....mm..........mm...',
    '......mmmmmmmmmm.....',
] + [E21] * 7)
HELM1 = '\n'.join([E23] * 18 + [
    '...kkkkkk..............',
    '...kwbbkkkkkk..........',
    '...kddck.bwbbk.........',
    '....kkk..kddck.........',
    '..........kkk..........',
] + [E23] * 5)
HELM2 = '\n'.join([E21] * 18 + [
    '....kkkkk............',
    '....kwbbkkkkkk.......',
    '....kddck............',
    '.....kkk.............',
] + [E21] * 6)
HELM3 = '\n'.join([E18] * 18 + [
    '....kkkkkkkkkk....',
    '...kbwbbaabwbbk...',
    '...kdddc..cdddk...',
    '.....kkk..kkk.....',
] + [E18] * 4)


# ── trunks: blue with white hibiscus ─────────────────────────────────────────────────────────────────
LEGPAL = P.legs_pal(TRUNKS, SOLE, WHITE, {
    'f': Mat(WHITE, z=4, flat=4),
    'y': Mat(YELLOW, z=4, flat=3),
})


def hibiscus(g, i):
    marks = []
    for y in range(g.h):
        for x in range(g.w):
            if g.keys[y][x] == 'R' and P.interior(g, x, y, 'R') and y % 3 == 1 and (x - (y // 3) * 3) % 6 == 2:
                marks.append((x, y))
    for x, y in marks:
        g.keys[y][x] = 'y'
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            if g.keys[y + dy][x + dx] == 'R':
                g.keys[y + dy][x + dx] = 'f'


# ── circles: a splash ring (dark) and soap bubbles (holy) ────────────────────────────────────────────
SPLASH = [('lobe', 1.05 * math.cos(math.radians(a)), 1.05 * math.sin(math.radians(a)), 0.075)
          for a in range(0, 360, 36)] + [
    ('ring', 0.93, 2), ('ring', 0.72, 1), ('ring', 0.5, 1), ('ring', 0.27, 2),
    ('dots', 10, 1.05, 0, 2.0), ('dots', 8, 0.83, 22.5, 2.2), ('dots', 12, 0.61, 15, 1.2),
    ('dots', 1, 0, 0, 3.0),
]
BUBBLES_BASE = [('lobe', 1.0 * math.cos(math.radians(a)), 1.0 * math.sin(math.radians(a)), r)
                for a, r in ((20, 0.12), (75, 0.09), (140, 0.13), (200, 0.1), (255, 0.08), (310, 0.12))] + [
    ('ring', 0.93, 2), ('ring', 0.62, 1), ('ticks', 12, 0.66, 0.89, 15, 1),
]
BUBBLE_PAL = dict(out='000000', g0='120c1c', g1='1c1428', g2='281c38', g3='362848', halo='4a3a62',
                  line='8a74b8', bright='d0b4ff', core='fff0ff')
IRIS = ['ffb0e0', 'b0f0ff', 'fff0a0', 'd0b8ff']


def bubbles():
    img = circles.render(BUBBLES_BASE, BUBBLE_PAL)
    px = img.load()
    spots = [(32.5, 22.0, 6.0), (19, 14, 3.6), (46, 14, 4.0), (16, 29, 4.2), (49, 30, 3.2), (33, 36.5, 2.8),
             (30, 7.5, 2.4), (57, 21, 2.4), (8.5, 21, 2.2), (41, 27, 1.8), (24, 27, 1.8)]
    for n, (cx, cy, r) in enumerate(spots):
        for y in range(int(cy - r - 1), int(cy + r + 2)):
            for x in range(int(cx - r - 1), int(cx + r + 2)):
                d = math.hypot(x + 0.5 - cx, y + 0.5 - cy)
                if 0 <= x < 66 and 0 <= y < 45 and px[x, y][3] and d < r - 0.5 and r > 2.5:
                    c = px[x, y]
                    px[x, y] = tuple(min(255, int(c[i] * 0.6 + (60, 40, 90)[i])) for i in range(3)) + (255,)
                if abs(d - r) <= 0.5 and 0 <= x < 66 and 0 <= y < 45 and px[x, y][3]:
                    a = math.degrees(math.atan2(y + 0.5 - cy, x + 0.5 - cx)) % 360
                    col = IRIS[(int(a // 90) + n) % 4]
                    px[x, y] = tuple(int(col[i:i + 2], 16) for i in (0, 2, 4)) + (255,)
        for hx, hy in ((int(cx - r * 0.45), int(cy - r * 0.45)), (int(cx - r * 0.45) + 1, int(cy - r * 0.45))):
            if r > 3 or (hx, hy) == (int(cx - r * 0.45), int(cy - r * 0.45)):
                if px[hx, hy][3]:
                    px[hx, hy] = (255, 255, 255, 255)
    return [img]


def build():
    parts = {}
    parts['armor'] = P.torso()
    parts['gloves'] = P.arms(BAND)
    parts['shoes'] = P.legs(LEGPAL, hibiscus)
    parts['helmet'] = kit.frames([HELM0, HELM1, HELM2, HELM3], HPAL, 'helmet')
    parts.update(P.toys(BLADE, GUARD, STRIPE, FIN, POCKET))
    parts['weapon_dark'] = kit.circle(SPLASH, circles.ramp('4aa8ff'))
    parts['weapon_holy'] = bubbles()
    return parts
