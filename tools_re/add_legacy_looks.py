"""Give the 69 drawn `"` legacy items their own LOOK — a sprite (b4) and a new colour
palette (b5) each (139차, 2026-09-29).

WHY. `add_legacy_items.py` copies every new row's stat block from the strongest shipped row
of its category, so all eight new helmets wore `@팬텀스카우터`'s sprite, all nine armours
`@팬텀윙슈트`'s, and so on — worn, the 템플러·크루세이더·붉은마녀·빙해 sets looked identical.
The user's decision: pick a different shipped sprite per item (shape) AND recolour it with a
new palette named after the item (colour) — 붉은마녀 red, 빙해 blue, 섬광의군화 yellow, ...

HOW THE GAME DRAWS WORN EQUIPMENT (disassembly, 139차):

  * The character is a paper doll. The layer loader (0x495a0; the one-layer reload at
    0x51880 is the same code) walks eight layers; the equipment ones read the worn record
    `EQUIPPED[slot]` -> the item row `*(*(grid)+cat*4) + idx*0x20` and take
      row+0x0c (s8)  = stat block **b4** = the SPRITE NUMBER,
      row+0x0e (u16) = stat block **b5** = the PALETTE NUMBER, 0xff = the sprite's own colours
    (the parse loop 0x53a38.. stores b4 at +0x0c and b5 at +0x0e; accessories get -1).
  * Sprite files (`res/hero/`, the `_bm` format of `gen_h3_icons.py`):
      helmet  cat 0  -> h600NN (layer 6 <- slot 1)      armour cat 1 -> h100NN (layer 1 <- slot 4)
      gloves  cat 2  -> h200NN (layer 2 <- slot 3)      boots  cat 3 -> h300NN (layer 3 <- slot 7)
      weapon  cat 4..10 -> h5<cat>NN (layer 5 <- slot 0) shield cat 11 -> h511NN (layer 7 <- slot 5)
    Accessories are not drawn. NN must name a shipped file — `SPRITE_FILES` below.
  * A palette is `res/dat/ph%03d_pa` (the name is built by 0x19f54 from the id; nothing
    counts or preloads them, so ids 158..254 are free): `[u8 n][n x (u16 from, u16 to)]`,
    colours RGB565 LE. The recolour 0x36ae4 MERGE-JOINS it with each frame's palette
    (entries 1.. — entry 0 is the transparent 0xf81f), replacing `from` by `to`: both lists
    must be ascending — every one of the 1,091 shipped hero frames is, and so is every
    shipped `ph` file — so the entries here are written sorted by `from`.

HOW THE COLOURS ARE MADE — the shipped palettes' own rule, measured on all 81 of them: the
VALUE of every colour is kept (optionally lifted or dropped by the theme) and the HUE and
SATURATION are replaced; skin (the hands of the glove sprites, a helmet's face) and the
near-black outline are left alone. `groups()` splits the rest into a BODY (the theme's
primary colour) and a TRIM (its accent colour) — the larger of {the greys, the dominant hue}
is the body, everything else the trim — so a two-tone sprite stays two-tone; a sprite of one
material gets its brightest colours as the trim instead (a shine) when the theme asks.
Skin = the skin colours of the bare hand `h20015` and the bare face `h00002`, and only on the
body layers (a weapon has no hand in it; a pink blade must still recolour).

THE SPRITES are picked so that no new item wears its template's sprite AND no palette lands
on a sprite that already has the theme's colours (a red robe recoloured red is the same
robe) — `--preview` draws every look before and after.

CONTRACT: owned by `patch_asset.py` (registered LAST, wrapping i0..i11): the row edit is two
bytes per `"` row (b4, b5 — the price tool never touches them) and the palettes are NEW jar
members, which `patch_asset` adds through `extra_members`. `--check` there compares both.

Usage: python add_legacy_looks.py [<pristine zip>] [--preview out.png] [--plan]
"""
import colorsys
import io
import os
import struct
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_legacy_items as GL  # noqa: E402

MARK = GL.NEW_MARK            # 0x22 `"` — the legacy colour escape
PALETTE_FIRST = 158           # the shipped palettes are ph000..ph157
PALETTE_NONE = 0xFF           # b5 = "the sprite's own colours"
TRANSPARENT = 0xF81F          # palette entry 0 of every frame
OUTLINE_V = 0.20              # value below which a colour is outline and kept
SAT_MIN = 0.25                # below this a colour is NEUTRAL (a grey — steel, cloth, bone)
HUE_WINDOW = 20               # the dominant hue = the 5° step with the most pixels within this
ACCENT_DEG = 28               # hue distance from the dominant hue that makes another material
                              # (red cloth h355..15 vs its gold trim h30..50 must split)
TRIM_MIN = 0.04               # a trim under this share of the pixels = a one-material sprite
SHINE_FRAC = 0.18             # the shine takes the brightest colours up to this share ...
SHINE_MAX = 0.35              # ... (the brightest one alone may take up to this) ...
SHINE_SPAN = 0.30             # ... and only colours this close to the brightest one
SKIN_SOURCES = ("h20015_bm", "h00002_bm")  # the bare hand, the bare face

# How many sprite files each category ships (`res/hero/`), i.e. the valid b4 range.
SPRITE_FILES = {0: 17, 1: 23, 2: 16, 3: 13, 4: 13, 5: 14, 6: 13, 7: 13, 8: 13,
                9: 10, 10: 10, 11: 13}


# The sprite every `"` row wore before this module — `add_legacy_items` copies the stat block
# of its category's strongest shipped row (@팬텀스카우터 · @팬텀윙슈트 · ...). A look must
# not land back on it, or the new item keeps the shape of the old one.
TEMPLATE_SPRITE = {0: 16, 1: 19, 2: 13, 3: 9, 4: 12, 5: 12, 6: 12, 7: 12, 8: 8, 9: 9, 10: 9, 11: 12}


def sprite_member(cat: int, b4: int) -> str:
    layer = {0: "60", 1: "10", 2: "20", 3: "30"}
    if cat in layer:
        return f"h{layer[cat]}0{b4:02d}_bm"
    return f"h5{cat:02d}{b4:02d}_bm"


# ── themes ───────────────────────────────────────────────────────────────────
# A tone is (hue°, saturation, value bias, value gain): value' = bias + gain * value.
def tone(h, s, vb=0.0, vg=1.0):
    return (h, s, vb, vg)


SILVER = tone(215, 0.10, 0.22, 0.85)
GOLD = tone(44, 0.80, 0.05, 1.0)
IVORY = tone(42, 0.13, 0.22, 0.85)
CRIMSON = tone(354, 0.85, 0.0, 0.97)
ROYAL = tone(224, 0.72, 0.02, 0.95)
RAVEN = tone(258, 0.32, 0.02, 0.62)
VIOLET = tone(282, 0.70, 0.05, 0.95)
ICE = tone(197, 0.62, 0.14, 0.90)
FROST = tone(186, 0.14, 0.35, 0.70)
BLOOD = tone(356, 0.90, 0.02, 1.0)
WHITE = tone(50, 0.08, 0.40, 0.65)
SKY = tone(200, 0.60, 0.10, 1.0)
EMERALD = tone(150, 0.75, 0.02, 1.0)
ORANGE = tone(25, 0.85, 0.05, 1.0)
CYAN = tone(186, 0.75, 0.10, 1.0)
BLACK = tone(0, 0.12, 0.0, 0.50)

THEMES = {  # name: (primary, accent, shine) — `shine`: on a one-material sprite, the
    #            brightest colours (the specular highlights) take the accent instead.
    # the six sets — every piece of a set, its shield included, wears the same colours
    "templar": (SILVER, GOLD, True),                              # 템플러 — 백은 + 금
    "crusader": (IVORY, CRIMSON, False),                          # 크루세이더 — 상아 + 진홍
    "regalis": (ROYAL, GOLD, True),                               # 레갈리스 근위 — 로열 블루 + 금
    "corvus": (RAVEN, VIOLET, True),                              # 코르버스(까마귀) — 칠흑 + 보라
    "redwitch": (CRIMSON, GOLD, True),                            # 붉은마녀 — 진홍 + 금
    "icesea": (ICE, FROST, True),                                 # 빙해 — 빙청 + 서리
    # legends
    "flash": (tone(50, 0.92, 0.30, 0.75), tone(56, 0.45, 0.38, 0.66), True),  # 섬광의군화 — 번개 노랑 + 백열
    "prophet": (tone(272, 0.55, 0.03, 0.95), GOLD, True),         # 예언자의눈 — 신비 보라 + 금
    "elders": (tone(150, 0.50, 0.02, 0.90), GOLD, True),          # 장로회의관 — 심록 + 금
    "berserker": (BLACK, BLOOD, True),                            # 광전사의흑갑 — 흑 + 핏빛
    "melhar": (tone(28, 0.62, 0.02, 0.92), tone(42, 0.90, 0.10, 1.0), True),  # 멜하르 — 청동 + 호박
    "yulike": (tone(140, 0.18, 0.25, 0.80), EMERALD, True),       # 율리케의성의 — 백옥 + 에메랄드
    "odin": (tone(46, 0.75, 0.05, 1.0), SKY, True),               # 오딘 — 금 + 하늘
    "odincore": (tone(200, 0.55, 0.12, 0.95), GOLD, True),        # 오딘의 코어 — 수정 하늘 + 금
    "executioner": (tone(220, 0.08, 0.0, 0.72), BLOOD, True),     # 집행자의손 — 무쇠 + 핏빛
    "kid": (tone(300, 0.55, 0.0, 0.80), tone(350, 0.85), True),   # 키드의발톱 — 흡혈 보라 + 진홍
    "basil": (tone(112, 0.62, 0.02, 0.95), tone(70, 0.65, 0.10, 1.0), True),  # 바질 — 바질 녹색 + 연두
    "unicorn": (tone(285, 0.32, 0.20, 0.85), tone(48, 0.55, 0.22, 0.82), True),  # 레아의유니콘 — 라일락 + 옅은 금
    "melpeto": (tone(345, 0.72, 0.0, 0.82), tone(290, 0.45, 0.05, 0.85), True),  # 멜페토 — 암적 + 보라
    "nyx": (tone(246, 0.62, 0.0, 0.85), CYAN, True),              # 닉스 — 한밀 남색 + 청록 빛
    "ian": (tone(214, 0.66, 0.03, 0.95), tone(210, 0.08, 0.20, 0.85), True),  # 이안 — 사파이어 + 은
    "claire": (tone(140, 0.60, 0.03, 0.95), GOLD, True),          # 클레르 — 에메랄드 + 금
    "masked": (tone(268, 0.30, 0.0, 0.70), tone(316, 0.72, 0.05, 1.0), True),  # 가면검사 — 흑보라 + 자홍
    "douglas": (tone(128, 0.45, 0.0, 0.85), tone(210, 0.08, 0.15, 1.0), True),  # 더글라스 — 숲 녹색 + 강철
    "heinz": (tone(240, 0.22, 0.02, 0.78), tone(280, 0.55, 0.10, 0.95), True),  # 하인츠(코르버스 대장) — 흑강 + 보라
    "q": (tone(0, 0.10, 0.0, 0.55), tone(352, 0.90), True),       # 큐 — 흑 + 진홍
    "beast": (tone(355, 0.70, 0.0, 0.85), tone(38, 0.22, 0.20, 0.82), True),  # 비스트의송곳니 — 핏빛 + 뼈
    "erebos": (tone(38, 0.66, 0.03, 0.95), tone(18, 0.75), True),  # 에레보스 — 황동 + 구리
    "wolfgang": (tone(220, 0.30, 0.0, 0.80), IVORY, False),        # 볼프강 — 청강 + 상아 손잡이
    "abel": (tone(78, 0.45, 0.0, 0.88), tone(30, 0.50, 0.0, 0.80), True),  # 에이블 — 올리브 + 가죽
    "blackfactory": (tone(220, 0.14, 0.0, 0.55), GOLD, True),     # 검은공장 — 흑철 + 금
    "unification": (tone(88, 0.25, 0.02, 0.85), tone(38, 0.66), True),  # 통일전쟁 — 포금 올리브 + 황동
    "torez": (tone(210, 0.40, 0.04, 1.0), tone(205, 0.06, 0.25, 0.80), True),  # 토레즈 — 강철 청 + 백강
    "siegetitan": (tone(12, 0.66, 0.0, 0.85), tone(30, 0.90, 0.10, 1.0), True),  # 시즈타이탄 — 녹슨 쇠 + 불빛
    "templecore": (tone(158, 0.60, 0.02, 1.0), tone(48, 0.45, 0.10), False),  # 잊혀진신전 — 비취
    "ilene": (tone(298, 0.60, 0.02, 1.0), CYAN, False),           # 일레느 — 자주
    "geb": (tone(350, 0.80, 0.0, 0.95), tone(10, 0.55, 0.25, 0.80), False),  # 게브의파편 — 핏빛 진홍
    "refiner": (tone(48, 0.32, 0.12, 0.92), GOLD, False),         # 리파이너 — 백금
    "gulveig": (tone(45, 0.55, 0.05, 1.0), WHITE, False),         # 굴베이그 — 옅은 금
    "tenner": (tone(180, 0.60, 0.05, 1.0), WHITE, False),         # 테너 — 아쿠아
    "sien": (tone(332, 0.45, 0.10, 0.92), WHITE, False),          # 시엔의목소리 — 장미
    "soul": (tone(188, 0.30, 0.20, 0.85), tone(275, 0.55, 0.05, 1.0), True),  # 영혼의수호 — 영혼 청백 + 보라
}

# (cat, name, sprite b4, theme) — ORDER IS THE PALETTE NUMBERING (158 + index), so append only.
LOOKS = [
    (0, "템플러헬름", 6, "templar"),        # 깃장식 대투구 — 깃이 금
    (0, "크루세이더후드", 12, "crusader"),  # 테두리 두른 둥근 투구
    (0, "근위관", 10, "regalis"),           # 볏 달린 관
    (0, "코르버스후드", 2, "corvus"),       # 두건
    (0, "붉은마녀헬름", 14, "redwitch"),    # 날개 서클릿
    (0, "빙해의투구", 5, "icesea"),         # 면갑 투구
    (0, "예언자의눈", 3, "prophet"),        # 눈 박힌 서클릿
    (0, "장로회의관", 1, "elders"),         # 납작한 관
    (1, "템플러플레이트", 14, "templar"),  # 금빛 흉갑 -> 백은, 빛이 금
    (1, "크루세이더코트", 4, "crusader"),   # 붉은 옷 -> 상아, 금 장식 -> 진홍
    (1, "레갈리스법의", 10, "regalis"),
    (1, "코르버스암살복", 2, "corvus"),    # 로브 -> 칠흑
    (1, "붉은마녀로브", 9, "redwitch"),
    (1, "빙해의갑옷", 7, "icesea"),
    (1, "멜하르의흉갑", 8, "melhar"),
    (1, "율리케의성의", 20, "yulike"),
    (1, "광전사의흑갑", 18, "berserker"),
    (2, "템플러건틀릿", 1, "templar"),
    (2, "크루세이더핸드", 8, "crusader"),
    (2, "레갈리스성갑", 12, "regalis"),
    (2, "코르버스크로우", 14, "corvus"),    # 갈퀴
    (2, "붉은마녀장갑", 3, "redwitch"),
    (2, "빙해의장갑", 2, "icesea"),
    (2, "집행자의손", 6, "executioner"),
    (2, "오딘의손길", 10, "odin"),
    (2, "키드의발톱", 7, "kid"),
    (3, "섬광의군화", 10, "flash"),
    (3, "템플러그리브", 2, "templar"),
    (3, "크루세이더워커", 4, "crusader"),   # 문장이 진홍으로 남는다
    (3, "근위장화", 8, "regalis"),
    (3, "코르버스스텝", 1, "corvus"),
    (3, "붉은마녀신발", 6, "redwitch"),
    (3, "빙해의장화", 7, "icesea"),
    (3, "바질의발걸음", 3, "basil"),
    (4, "레아의유니콘창", 7, "unicorn"),    # 물결 날 = 뿔
    (4, "디미트리의창", 6, "regalis"),
    (4, "멜페토의공국창", 5, "melpeto"),
    (4, "닉스의수호창", 8, "nyx"),
    (5, "이안의송검", 2, "ian"),            # 창백한 날 -> 사파이어, 금 손잡이 -> 은
    (5, "클레르클레이모어", 11, "claire"),
    (5, "가면검사의검", 10, "masked"),
    (5, "빙해의대검", 3, "icesea"),
    (6, "더글라스의밀검", 5, "douglas"),
    (6, "하인츠의쌍단검", 1, "heinz"),
    (6, "큐의암살단검", 6, "q"),
    (6, "비스트의송곳니", 8, "beast"),      # 세 갈퀴
    (7, "에레보스화승총", 7, "erebos"),
    (7, "볼프강의호신총", 0, "wolfgang"),
    (7, "에이블의밀수총", 9, "abel"),
    (7, "검은공장의걸작", 10, "blackfactory"),
    (8, "에레보스공성포", 0, "erebos"),
    (8, "통일전쟁중포", 5, "unification"),
    (8, "토레즈중포", 4, "torez"),
    (8, "시즈타이탄주포", 6, "siegetitan"),
    (9, "잊혀진신전의핵", 2, "templecore"),
    (9, "오딘의코어조각", 5, "odincore"),
    (9, "일레느의연구석", 6, "ilene"),
    (9, "게브의파편", 8, "geb"),
    (10, "리파이너의성석", 2, "refiner"),
    (10, "굴베이그의유산", 5, "gulveig"),
    (10, "테너의성표", 6, "tenner"),
    (10, "시엔의목소리", 8, "sien"),
    (11, "케네스방패", 10, "templar"),
    (11, "닉스의문장", 5, "crusader"),
    (11, "디미트리의방패", 3, "regalis"),
    (11, "듀리안의버클러", 2, "corvus"),
    (11, "리파이너의가호", 11, "redwitch"),
    (11, "왕국의철벽", 7, "icesea"),
    (11, "영혼의수호", 9, "soul"),
]


def euc(s: str) -> bytes:
    return s.encode("euc-kr")


def looks_by_name() -> dict:
    """{raw name bytes (`"` included): (cat, b4, palette id, theme)}"""
    out = {}
    for i, (cat, name, b4, theme) in enumerate(LOOKS):
        out[bytes([MARK]) + euc(name)] = (cat, b4, PALETTE_FIRST + i, theme)
    return out


def _check_tables() -> None:
    names = [n for _c, n, _b, _t in LOOKS]
    if len(set(names)) != len(names):
        raise SystemExit("add_legacy_looks: a name appears twice in LOOKS")
    if PALETTE_FIRST + len(LOOKS) - 1 >= PALETTE_NONE:
        raise SystemExit("add_legacy_looks: palette ids would reach 0xff")
    for cat, name, b4, theme in LOOKS:
        if not 0 <= b4 < SPRITE_FILES[cat]:
            raise SystemExit(f"add_legacy_looks: {name} b4 {b4} — category {cat} ships "
                             f"{SPRITE_FILES[cat]} sprite(s)")
        if theme not in THEMES:
            raise SystemExit(f"add_legacy_looks: {name} names unknown theme {theme!r}")
        if b4 == TEMPLATE_SPRITE[cat]:
            raise SystemExit(f"add_legacy_looks: {name} b4 {b4} is its template's own sprite")
    per_cat = {}
    for cat, name, b4, _t in LOOKS:
        if b4 in per_cat.setdefault(cat, set()):
            raise SystemExit(f"add_legacy_looks: {name} — category {cat} already uses sprite {b4}")
        per_cat[cat].add(b4)


_check_tables()


# ── sprites and colours ──────────────────────────────────────────────────────
def frames(d: bytes) -> list:
    """Every frame of a `_bm` as (palette list, pixel indices) — the layout `gen_h3_icons`
    documents, asserted exact."""
    count = struct.unpack_from("<H", d, 0)[0]
    off = 2
    out = []
    for _ in range(count):
        (flen,) = struct.unpack_from("<I", d, off)
        off += 4
        t = d[off]
        w, h, cw, palcnt = struct.unpack_from("<HHHH", d, off + 1)
        pal = [struct.unpack_from("<H", d, off + 9 + 2 * k)[0] for k in range(palcnt)]
        po = off + 9 + palcnt * 2
        nbytes = w * h if t == 0x0C else (w * h + 1) // 2
        if 9 + palcnt * 2 + nbytes != flen:
            raise SystemExit("add_legacy_looks: a sprite frame does not add up")
        idx = []
        for k in range(w * h):
            if k % w >= cw:
                continue
            idx.append(d[po + k] if t == 0x0C else
                       ((d[po + k // 2] >> 4) if k % 2 == 0 else (d[po + k // 2] & 0x0F)))
        out.append((pal, idx, cw, h))
        off = po + nbytes
    if off != len(d):
        raise SystemExit("add_legacy_looks: a sprite file has trailing bytes")
    return out


def usage(d: bytes) -> dict:
    """{colour: pixels} over every frame (palette entry 0 is transparent)."""
    use = {}
    for pal, idx, _cw, _h in frames(d):
        for i in idx:
            if 0 < i < len(pal):
                use[pal[i]] = use.get(pal[i], 0) + 1
    return use


def rgb(c: int):
    return ((c >> 11) & 31) / 31, ((c >> 5) & 63) / 63, (c & 31) / 31


def hsv(c: int):
    h, s, v = colorsys.rgb_to_hsv(*rgb(c))
    return h * 360, s, v


def to565(h: float, s: float, v: float) -> int:
    r, g, b = colorsys.hsv_to_rgb((h % 360) / 360, max(0.0, min(1.0, s)), max(0.0, min(1.0, v)))
    c = (round(r * 31) << 11) | (round(g * 63) << 5) | round(b * 31)
    return 0xF01F if c == TRANSPARENT else c


def hue_dist(a: float, b: float) -> float:
    d = abs(a - b) % 360
    return min(d, 360 - d)


def is_skin(c: int) -> bool:
    h, s, v = hsv(c)
    return (h >= 330 or h <= 35) and 0.10 <= s <= 0.62 and v >= 0.35


def skin_colours(member) -> set:
    out = set()
    for base in SKIN_SOURCES:
        out |= {c for c in usage(member(base)) if is_skin(c)}
    return out


def groups(use: dict, skin: set, shine: bool) -> tuple:
    """Split one sprite's colours into (kept, body, trim) sets.

    kept = skin + the near-black outline. The rest falls into the NEUTRAL colours (greys —
    steel, cloth, bone) and the SATURATED ones, and those into the dominant hue and the other
    hues. The body is the larger of {neutral, dominant hue} by pixels; the trim is everything
    else — so a grey coat with a red belt keeps a belt, and a red robe with grey clasps keeps
    its clasps. A sprite made of ONE material (the trim is under `TRIM_MIN` of its pixels)
    gets a shine when its theme asks for one: its brightest colours join the trim."""
    live = {c: n for c, n in use.items() if c not in skin and hsv(c)[2] >= OUTLINE_V}
    col = {c: hsv(c) for c in live}
    neutral = {c for c in live if col[c][1] < SAT_MIN}
    sat = [c for c in live if col[c][1] >= SAT_MIN]
    main = set()
    if sat:
        dom = max(range(0, 360, 5),
                  key=lambda h0: sum(live[c] for c in sat if hue_dist(col[c][0], h0) <= HUE_WINDOW))
        main = {c for c in sat if hue_dist(col[c][0], dom) <= ACCENT_DEG}
    other = set(sat) - main
    if sum(live[c] for c in neutral) >= sum(live[c] for c in main):
        body, trim = neutral, main | other
    else:
        body, trim = main, neutral | other
    total = sum(live.values())
    if shine and total and sum(live[c] for c in trim) < TRIM_MIN * total and len(body) >= 3:
        ranked = sorted(body, key=lambda c: col[c][2], reverse=True)
        top = col[ranked[0]][2]
        lit, acc = set(), 0
        for c in ranked:
            if col[c][2] < top - SHINE_SPAN:
                break
            if acc + live[c] > (SHINE_MAX if not lit else SHINE_FRAC) * total:
                continue
            lit.add(c)
            acc += live[c]
        body, trim = body - lit, trim | lit
    kept = set(use) - set(live)
    return kept, body, trim


def recolour(use: dict, theme: str, skin: set) -> dict:
    """{from: to} for one sprite and one theme (colours that do not change are left out)."""
    primary, accent, shine = THEMES[theme]
    _kept, body, trim = groups(use, skin, shine)
    out = {}
    for c in body | trim:
        _h, s, v = hsv(c)
        th, ts, vb, vg = accent if c in trim else primary
        new = to565(th, ts * (0.6 + 0.4 * min(1.0, s / 0.5)), max(0.05, vb + vg * v))
        if new != c:
            out[c] = new
    return out


def palette_bytes(mapping: dict) -> bytes:
    if len(mapping) > 255:
        raise SystemExit(f"add_legacy_looks: {len(mapping)} colours do not fit a u8 count")
    body = b"".join(struct.pack("<HH", a, mapping[a]) for a in sorted(mapping))
    return bytes([len(mapping)]) + body


def apply_palette(pal: list, pbytes: bytes) -> list:
    """The game's merge-join (0x36ae4, the 16-bit arm) over one frame palette."""
    out = list(pal)
    n = pbytes[0]
    j = 0
    i = 1
    while j < n and i < len(out):
        a, b = struct.unpack_from("<HH", pbytes, 1 + 4 * j)
        if a == out[i]:
            out[i] = b
            j += 1
            i += 1
        elif a < out[i]:
            j += 1
        else:
            i += 1
    return out


# ── the jar ──────────────────────────────────────────────────────────────────
def _jar_member_reader(jar_bytes: bytes):
    j = zipfile.ZipFile(io.BytesIO(jar_bytes))
    by_base = {}
    for n in j.namelist():
        by_base.setdefault(n.rsplit("/", 1)[-1], []).append(n)

    def member(base: str) -> bytes:
        names = by_base.get(base, [])
        if len(names) != 1:
            raise SystemExit(f"add_legacy_looks: expected exactly one {base} in the jar, got {names}")
        return j.read(names[0])

    def path(base: str) -> str:
        names = by_base.get(base, [])
        if len(names) != 1:
            raise SystemExit(f"add_legacy_looks: expected exactly one {base} in the jar, got {names}")
        return names[0]
    return member, path


def palettes(jar_bytes: bytes) -> dict:
    """{jar path of ph%03d_pa: bytes} for every look — built from the jar's own sprites."""
    member, path = _jar_member_reader(jar_bytes)
    folder = path("ph000_pa").rsplit("/", 1)[0]
    skin = skin_colours(member)
    out = {}
    for i, (cat, name, b4, theme) in enumerate(LOOKS):
        pid = PALETTE_FIRST + i
        use = usage(member(sprite_member(cat, b4)))
        mapping = recolour(use, theme, skin if cat <= 3 else set())
        if not mapping:
            raise SystemExit(f"add_legacy_looks: {name} would not change colour")
        out[f"{folder}/ph{pid:03d}_pa"] = palette_bytes(mapping)
    for p in out:
        if p.rsplit("/", 1)[-1] in {n.rsplit("/", 1)[-1] for n in zipfile.ZipFile(io.BytesIO(jar_bytes)).namelist()}:
            raise SystemExit(f"add_legacy_looks: {p} already exists in the jar")
    return out


def extra_members(jar_bytes: bytes) -> dict:
    """What `patch_asset` adds to the jar: the new palettes."""
    return palettes(jar_bytes)


# ── the item rows ────────────────────────────────────────────────────────────
def _records(blob: bytes):
    p = 0
    out = []
    while p + 2 <= len(blob):
        n = blob[p] | blob[p + 1] << 8
        if n == 0 or p + 2 + n > len(blob):
            break
        out.append((p + 2, blob[p + 2:p + 2 + n]))
        p += 2 + n
    if p != len(blob):
        raise SystemExit("add_legacy_looks: an item member does not parse byte-exact")
    return out


def restyle(cat: int, blob: bytes) -> bytes:
    """Set b4/b5 of this category's `"` rows named in LOOKS; nothing else moves."""
    want = {k: v for k, v in looks_by_name().items() if v[0] == cat}
    out = bytearray(blob)
    seen = set()
    for off, body in _records(blob):
        name = bytes(body[1:1 + body[0]])
        if name in want:
            _cat, b4, pid, _theme = want[name]
            stats = off + 1 + body[0]
            out[stats + 4] = b4
            out[stats + 5] = pid
            seen.add(name)
    missing = set(want) - seen
    if missing:
        raise SystemExit(f"add_legacy_looks: i{cat}_dat has no row "
                         f"{sorted(m[1:].decode('euc-kr') for m in missing)}")
    return bytes(out)


def verify_cat(cat: int, before: bytes, after: bytes) -> None:
    a, b = _records(after), _records(before)
    if len(a) != len(b):
        raise SystemExit(f"VERIFY i{cat}_dat: row count changed")
    want = {k: v for k, v in looks_by_name().items() if v[0] == cat}
    changed = 0
    for (oa, ba), (_ob, bb) in zip(a, b):
        name = bytes(ba[1:1 + ba[0]])
        s = 1 + ba[0]
        if name in want:
            _c, b4, pid, _t = want[name]
            if (ba[s + 4], ba[s + 5]) != (b4, pid):
                raise SystemExit(f"VERIFY i{cat}_dat {name[1:].decode('euc-kr')}: look not written")
            if ba[:s + 4] + ba[s + 6:] != bb[:s + 4] + bb[s + 6:]:
                raise SystemExit(f"VERIFY i{cat}_dat {name[1:].decode('euc-kr')}: more than b4/b5 changed")
            changed += 1
        elif ba != bb:
            raise SystemExit(f"VERIFY i{cat}_dat: a row outside LOOKS changed")
    if changed != len(want):
        raise SystemExit(f"VERIFY i{cat}_dat: {changed} of {len(want)} looks written")
    print(f"VERIFY OK: i{cat}_dat {changed} look(s) — sprite b4 + palette b5 of the `\"` rows only")


class Handler:
    """One of i0..i11, wrapping whatever handler was registered before this module."""

    def __init__(self, cat: int, inner):
        self.cat = cat
        self.inner = inner
        tail = f" after {getattr(inner, '__name__', type(inner).__name__)}" if inner else ""
        self.__name__ = f"add_legacy_looks_i{cat}{tail}"

    def bind(self, jar_bytes: bytes) -> None:
        if self.inner is not None and hasattr(self.inner, "bind"):
            self.inner.bind(jar_bytes)

    def build(self, src: bytes) -> bytes:
        mid = self.inner.build(src) if self.inner else src
        return restyle(self.cat, mid)

    def verify(self, before: bytes, after: bytes) -> None:
        mid = self.inner.build(before) if self.inner else before
        if self.inner:
            self.inner.verify(before, mid)
        verify_cat(self.cat, mid, after)


def handlers(prev: dict) -> dict:
    """{member: Handler} for i0..i11 (accessories are not drawn), wrapping `prev`."""
    return {f"i{cat}_dat": Handler(cat, prev.get(f"i{cat}_dat")) for cat in range(12)}


def looks_of(blob: bytes) -> list:
    """(b4, b5) of every row of an item member — what `patch_asset --check` compares."""
    return [(body[1 + body[0] + 4], body[1 + body[0] + 5]) for _off, body in _records(blob)]


# ── preview ──────────────────────────────────────────────────────────────────
def preview(jar_bytes: bytes, out_png: str) -> None:
    from PIL import Image, ImageDraw, ImageFont
    member, _path = _jar_member_reader(jar_bytes)
    pals = palettes(jar_bytes)
    by_id = {int(p[-6:-3]): b for p, b in pals.items()}
    try:
        font = ImageFont.truetype(r"C:\Windows\Fonts\malgun.ttf", 14)
    except OSError:
        font = ImageFont.load_default()
    S = 3
    rows = []
    for i, (cat, name, b4, theme) in enumerate(LOOKS):
        fr = frames(member(sprite_member(cat, b4)))
        if cat in (4, 5, 6, 7, 8):
            fr = fr[:4]
        rows.append((cat, name, b4, theme, fr, by_id[PALETTE_FIRST + i]))
    rowh = [max(h for _p, _i, _cw, h in fr) * S + 8 for *_x, fr, _pb in rows]
    width = 300 + max(sum(cw * S + 6 for _p, _i, cw, _h in fr) * 2 + 40 for *_x, fr, _pb in rows)
    img = Image.new("RGB", (width, sum(rowh) + 10), (54, 56, 66))
    d = ImageDraw.Draw(img)
    y = 5
    for (cat, name, b4, theme, fr, pb), h in zip(rows, rowh):
        d.text((6, y + 2), f"{name}", font=font, fill=(255, 240, 120))
        d.text((6, y + 20), f"cat{cat} b4={b4} {theme}", font=font, fill=(180, 180, 190))
        x = 300
        for colours in ("orig", "new"):
            for pal, idx, cw, fh in fr:
                p = apply_palette(pal, pb) if colours == "new" else pal
                for k, pi in enumerate(idx):
                    if 0 < pi < len(p):
                        r, g, b = rgb(p[pi])
                        px, py = x + (k % cw) * S, y + (k // cw) * S
                        d.rectangle([px, py, px + S - 1, py + S - 1],
                                    fill=(round(r * 255), round(g * 255), round(b * 255)))
                x += cw * S + 6
            x += 40
        y += h
    img.save(out_png)
    print(f"preview -> {out_png} ({img.size[0]}x{img.size[1]})")


def _jar_of(zpath: str) -> bytes:
    z = zipfile.ZipFile(zpath)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit(f"expected exactly one jar in {zpath}, got {jars}")
    return z.read(jars[0])


def main() -> int:
    # The sprites are the same in every build of the asset, but the palettes are only NEW in
    # the pristine one — so a plan or a preview reads the pristine backup by default.
    zips = [a for a in sys.argv[1:] if a.lower().endswith((".zip", ".orig"))]
    zpath = zips[0] if zips else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..",
                                              "android_app", "asset_backup",
                                              "hero3_fixed.zip.orig")
    jb = _jar_of(zpath)
    if "--plan" in sys.argv or "--preview" not in sys.argv:
        pals = palettes(jb)
        for (cat, name, b4, theme), (p, b) in zip(LOOKS, pals.items()):
            print(f"  cat{cat:2d} {name:<10} b4={b4:2d} {p.rsplit('/', 1)[-1]} {theme:<12} {b[0]:2d} colour(s)")
    if "--preview" in sys.argv:
        out = sys.argv[sys.argv.index("--preview") + 1]
        preview(jb, out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
