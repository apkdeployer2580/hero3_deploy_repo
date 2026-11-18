"""150차 (2026-10-01) — the 11 reworked accessories: THE one source of truth for all three options.

User decisions (LEGACY_ITEMS_REWORK_85_90.md §11 → §12, 2026-10-01):

    율리케의목걸이 (85제)     특수공격 +30% · SP소모 -20%   | 특수공격 +30%
    레갈리스교단반지 (85제)   특수공격 +30% · 특수방어 +30% | 물리방어 +30%
    엘지스의증표 (85제)       회피+40 · 기절저항 +40        | 블록+40              (버티기)
    이안의맹세 (85제)         물리방어 +30% · 특수방어 +30% | 피해감소 +10%
    레아의목걸이 (85제)       최대 HP50% · 5초당 HP회복+10% (158차: 20 → 10) | 적 처치 시 20초간 최대 HP +20%
    일레느의각인 (85제)       특수공격 +30% · SP소모 −30%   | 쿨타임 −15%
    클레르의가보 (90제)       물리 +30% · 기절 + 20         | 물리공격 +30%        (순수 화력)
    굴베이그의탈리스만 (90제) 최대HP +20% · 쿨타임 −30%     | 피격 시 20초간 최대 HP +20%
    닉스의펜던트 (90제)       블록+30 · HP흡수 +40%         | 회피+20
    타이탄의 파편 (보스)      회피+15 · 쿨타임 −20%         | 공격속도 +50
    검은 눈물 (보스)          쿨타임 −30% · SP소모 -30%     | 주는 피해 +25%

    all 11: 성능(= 저항) 0 · 요구 Lv 1.  레아's option 2 = a NEW option (user: "새 옵션: 5초마다
    최대HP 20%") — heal 20 % of the true max HP every 5 s of play in the field, the host's
    work (the game's own HP회복 0x03 only heals max × x / 1000 per ~2 s of standing still).
    Summary-box wording for the two long thirds (user: "처치 20초 최대HP+20%").

The first two options are the asset row's two `(statId, i8)` pairs (18-byte cat-0x0c row
b14..17) — the game shows them with `InGame_txt[80 + id]` and its own `%` rule. The third
lives OUTSIDE the asset (the parser reads exactly two pairs; a longer record shifts every
later row — LEGACY_ITEMS_REWORK_85_90.md §12): `gen_acc_options.py` turns this table into
`vendor/wie/wie_util/src/acc_options.rs`, which the guest hooks (the wear recompute's
`addStat` for `Stat` thirds, the bag summary line) and the host (the effects) both read.

Consumers of THIS file:
  * gen_legacy_items.py / add_legacy_items.py — the nine `"` rows (options, 성능 0, 요구 Lv 1)
  * add_boss_accessories.py                    — the two `{` rows
  * gen_acc_options.py                         — acc_options.rs (+ --check against the asset)
⛔ Names are identities here (the tools resolve (cat, idx) by name in the shipped asset).
⛔ Every text must encode in KS X 1001 + ASCII (the game font has nothing else: no '−', no '·').
"""

REQ_LV = 1     # 18-byte row b6
PERF = 0       # 18-byte row b10..11 — the 성능 is the character's 저항 (§11.2), gone

# option ids — `addStat 0x4ee34`'s jump table (0x5e1cc) + the two host stats
HPMAX, HPREGEN = 0x02, 0x03
PATK, MATK, PDEF, MDEF = 0x05, 0x06, 0x07, 0x08
ACCURACY, DODGE, BLOCK, STUN, RESIST = 0x09, 0x0A, 0x0B, 0x0C, 0x0D
SPCOST, SPREGEN, DRAIN, COOLDOWN, PIERCE = 0x0E, 0x0F, 0x10, 0x11, 0x12
ATK_SPEED = 0x76   # 131차-D 공격속도 (host), label row 198
REGEN = 0x78       # ★ 150차 5초마다 최대 HP n % 회복 (host), label row 200 (add_regen_label.py)
REGEN_LABEL_ROW = 80 + REGEN   # = 200 — the detail window prints InGame_txt[80 + id]
REGEN_LABEL = "5초당 HP회복 +"  # the game appends the value (and, via the `%` hook, '%')
REGEN_PERIOD_MS = 5_000

# the ids a `Stat` third may use: addStat's additive arms (0x04 = no-op, 0x13.. = modes)
GUEST_STAT_IDS = frozenset({HPMAX, HPREGEN, PATK, MATK, PDEF, MDEF, ACCURACY, DODGE, BLOCK,
                            STUN, RESIST, SPCOST, SPREGEN, DRAIN, COOLDOWN, PIERCE})

# third-option kinds (the Rust enum `acc_options::Third`)
STAT, ATKSPEED, TAKEN, DEALT, KILL_MAX_HP, HIT_MAX_HP = (
    "Stat", "AtkSpeed", "Taken", "Dealt", "KillMaxHp", "HitMaxHp")

# (name, colour escape, option 1, option 2, (kind, stat id | None, value), summary, detail)
#   summary = the bag summary box's first line — ≤ 134 px in the game's font 1
#             (Hangul 11 · ASCII 8 · space 5): the hook writes it over "레벨N 장신구/저항M".
#   detail  = the same option in the 상세보기 body (the host wraps it at the box width).
# Stat thirds use the GAME's own label words ("방패 +" is the game's name for 블록, row 91;
# "쿨타임 -" row 97) so the three lines read as one list.
ACC = [
    ("타이탄의 파편", "{", (DODGE, 15), (COOLDOWN, 20), (ATKSPEED, None, 50),
     "공격속도 +50", "공격속도 +50"),
    ("검은 눈물", "{", (COOLDOWN, 30), (SPCOST, 30), (DEALT, None, 25),
     "주는 피해 +25%", "주는 피해 +25%"),
    ("율리케의목걸이", '"', (MATK, 30), (SPCOST, 20), (STAT, MATK, 30),
     "특수공격 +30%", "특수공격 +30%"),
    ("레갈리스교단반지", '"', (MATK, 30), (MDEF, 30), (STAT, PDEF, 30),
     "물리방어 +30%", "물리방어 +30%"),
    # ★ 158차 P7 (2026-10-02, 사용자 «전부 권장대로»): 기절저항 40 → 최대HP 40 % — 저항은 Lv 99 의 헬에서 0
    #   (몬스터 크리 = 2·레벨차 − 저항), 「버티기」는 HP 로.
    ("엘지스의증표", '"', (DODGE, 40), (HPMAX, 40), (STAT, BLOCK, 40),
     "방패 +40", "방패 +40"),
    ("이안의맹세", '"', (PDEF, 30), (MDEF, 30), (TAKEN, None, 10),
     "피해감소 +10%", "피해감소 +10%"),
    # ★ 158차 P9 (2026-10-02, 사용자 «전부 권장대로»): 5초당 HP회복 20 → 10 % — 장신구 생존 1위(+45 %, 2위
    #   +21 %)는 유지하되 회복을 반감(`ITEM_BALANCE_AUDIT_156.md §0 5`).
    ("레아의목걸이", '"', (HPMAX, 50), (REGEN, 10), (KILL_MAX_HP, None, 20),
     "처치 20초 최대HP+20%", "적 처치 시 20초간 최대 HP +20%"),
    ("일레느의각인", '"', (MATK, 30), (SPCOST, 30), (STAT, COOLDOWN, 15),
     "쿨타임 -15%", "쿨타임 -15%"),
    ("클레르의가보", '"', (PATK, 30), (STUN, 20), (STAT, PATK, 30),
     "물리공격 +30%", "물리공격 +30%"),
    ("굴베이그의탈리스만", '"', (HPMAX, 20), (COOLDOWN, 30), (HIT_MAX_HP, None, 20),
     "피격 20초 최대HP+20%", "피격 시 20초간 최대 HP +20%"),
    ("닉스의펜던트", '"', (BLOCK, 30), (DRAIN, 40), (STAT, DODGE, 20),
     "회피 +20", "회피 +20"),
]

# concept notes the user gave (docs only)
CONCEPT = {"엘지스의증표": "버티기", "클레르의가보": "순수 화력"}


def by_name():
    return {a[0]: a for a in ACC}


def options_of(name):
    """(o1, o2) — the asset row's two pairs for `name`."""
    a = by_name()[name]
    return a[2], a[3]


# ---- the game font's widths (font 1 = the summary box; 0x3600's rule) ---------------
SUMMARY_MAX_PX = 134


def summary_width(text):
    w = 0
    for ch in text:
        if ch == " ":
            w += 5
        elif ch in ".,":
            w += 4
        elif ord(ch) < 0x80:
            w += 8
        else:
            w += 11
    return w


def validate():
    """Every rule the table must satisfy — raises on the first violation."""
    seen = set()
    for name, colour, o1, o2, third, summary, detail in ACC:
        if name in seen:
            raise ValueError(f"{name}: listed twice")
        seen.add(name)
        if colour not in ('"', "{"):
            raise ValueError(f"{name}: colour {colour!r} is neither the legacy '\"' nor the boss '{{'")
        for sid, v in (o1, o2):
            if not (-128 <= v <= 127):
                raise ValueError(f"{name}: option value {v} is outside i8")
            if sid not in GUEST_STAT_IDS and sid not in (ATK_SPEED, REGEN):
                raise ValueError(f"{name}: option id {sid:#x} is neither a guest stat nor a host stat")
        kind, sid, v = third
        if kind == STAT and sid not in GUEST_STAT_IDS:
            raise ValueError(f"{name}: a Stat third needs an addStat id, got {sid!r}")
        if kind != STAT and sid is not None:
            raise ValueError(f"{name}: a {kind} third carries no stat id")
        if not (0 < v <= 127):
            raise ValueError(f"{name}: third value {v} must be 1..127")
        for t in (summary, detail):
            t.encode("euc-kr")  # raises on a syllable the font cannot draw
            if ";" in t or any(c in t for c in "@|$" + "{}'\""):
                raise ValueError(f"{name}: {t!r} holds a line break or a colour escape")
        if summary_width(summary) > SUMMARY_MAX_PX:
            raise ValueError(f"{name}: summary {summary!r} is {summary_width(summary)} px > {SUMMARY_MAX_PX}")
        if len(summary.encode("euc-kr")) > 51:
            raise ValueError(f"{name}: summary is longer than the painter's 52-byte buffer")
    if len(ACC) != 11:
        raise ValueError(f"expected 11 accessories, got {len(ACC)}")


validate()
