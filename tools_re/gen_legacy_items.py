"""Regenerate the Lv 70~99 legacy-item draft tables in `LEGACY_ITEMS_70_99.md`.

READ-ONLY. It never touches the asset - it decodes the shipped `iN_dat` equipment
files, finds the strongest surviving row per slot (the FLOOR a new tier must beat),
and prints the draft tables plus the invariant check.

WHY A GENERATOR. Every number in that document is derived, not typed.

  * The floor is NOT the Lv70 white row. `{리버` (Lv65, 75차) is 1.2x the white in
    every weapon slot and in the shield, so anchoring on Lv70 would put a Lv71 item
    BELOW a Lv65 one.
  * Scaling each slot from ITS OWN 리버 row is what "리버 무기들이랑 비교해서" means:
    the ratios BETWEEN categories (리버다크스톤 481 vs 리버건 380 vs 리버홀리스톤 368)
    are carried forward unchanged, and the run proves it.
  * Re-running after any asset change re-derives the floor and fails loudly if the
    ladder stops clearing it.

THE LADDER IS THE TRILOGY. Weapon rungs are hero1(74) -> hero2(82) -> hero3(90), and
the six armour sets climb the same way. Lv99 is NOT a set - it is the 전설 tier, ten
standalone pieces, each with one unmistakable concept (§5.4).

WEAPON OWNERSHIP is not a guess - `char_dat` says it. All ten classes were decoded
and their starting weapon category agrees 10/10 with the published class list:
다크·홀리 are 네오솔티아(케이) only, 건·라이플 are 아스크라(리츠) only, and
스피어·스워드·나이프 are shared. Names follow that split.

Usage:
    python tools_re/gen_legacy_items.py [--zip <asset.zip>] [--growth 1.036]
    python tools_re/gen_legacy_items.py --spec      # 통합 스펙표만
"""
import argparse
import io
import math
import os
import struct
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import acc_options as AO  # noqa: E402  (150차: the 11 accessories' three options — the one source)

DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")

SLOT = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", 4: "스피어", 5: "스워드",
        6: "나이프", 7: "건", 8: "라이플", 9: "다크", 10: "홀리", 11: "방패"}
# name-leading text colour escapes; 0x22 (") is the one the stock data never uses
COLOUR = {0x40: "@", 0x7C: "|", 0x24: "$", 0x7B: "{", 0x7D: "}",
          0x27: chr(0x27), 0x22: chr(0x22)}
L0 = 70          # the highest 요구레벨 that ships today
LEG_LV = 99      # the 전설 tier
Q = chr(0x22)    # the colour escape every new name carries
NEW_MARK = 0x22  # that escape as the raw name-leading byte

# ★ THE FLOOR MUST NOT BECOME OUR OWN ROWS (133차). Once `add_legacy_items` ships the
# 77 rows, re-running this generator against the shipped asset would otherwise scale
# the ladder from itself (and 131차-C's `@섬광의군화` experiment row, Lv1 성능 199,
# would become the 신발 floor and inflate the whole boot column). Excluded: every
# `"`-coloured row, and the boots row by name whatever its colour.
FLOOR_EXCLUDE_NAMES = {"섬광의군화"}

# ★★★ 152차-B (2026-10-02): THE WEAPON FLOOR IS FROZEN AT THE 75차 리버 SPEC. Every weapon
# cat's floor used to be the `{리버` row `add_reaver_weapons` appends — Lv65, the strongest
# `@` white × 6/5 — and every legacy weapon's number (the 전설 row, then 85제 = 90 % / 90제 =
# 98 % of it) is scaled from that floor. 152차-B moved the reaver rows themselves to 80제
# (user: «리버무기의 스펙을 바꾸자 80제로 바꾸고 스펙을 올려줘»), so reading their NEW bytes
# as the floor would silently re-scale all 78 legacy rows. ⇒ the floor hunt skips the reaver
# rows by name and takes the ANCHOR instead: the numbers the 75차 row carried, recomputed from
# the same white row with the same expression (`anchor_row`). Byte-identical legacy rows are
# asserted by diffing the jar members (`PLAY_FEEDBACK_FIXES.md §152차-B`).
# The reaver display names live HERE (the anchor needs them and this module must not import
# `add_reaver_weapons`, which imports it); that tool uses this table.
REAVER_NAMES = {
    4: "리버스피어",
    5: "리버스워드",
    6: "리버나이프",
    7: "리버건",
    8: "리버캐논",
    9: "리버다크스톤",
    10: "리버홀리스톤",
    11: "리버쉴드",
}
ANCHOR_SCALE = 1.2   # the 75차 리버: main / sub = round(white x 1.2)
ANCHOR_REQ = 65      # its 요구레벨 (shown by `--rework`; `scale_row` reads main/sub/price only)
WHITE_MARK = 0x40    # `@` — the Lv70 white finals the anchor is cut from


def white_best(rows):
    """The strongest `@` row's stat block among `(raw_name, stats)` pairs (first max wins) —
    the same pick `add_reaver_weapons` cuts its row from. `None` when the member has none."""
    best = None
    for raw, stats in rows:
        if not raw or raw[0] != WHITE_MARK or len(stats) < 16:
            continue
        main = struct.unpack_from("<H", stats, 12)[0]
        if best is None or main > best[0]:
            best = (main, stats)
    return best[1] if best else None


def anchor_row(cat, white_stats):
    """The 75차 리버 row as a floor tuple `(name, colour, req, main, sub, price)`: the white's
    main / sub x 1.2 (`int(round(..))`, exactly the expression that tool used) and its price."""
    main = struct.unpack_from("<H", white_stats, 12)[0]
    sub = struct.unpack_from("<H", white_stats, 14)[0]
    price = struct.unpack_from("<I", white_stats, 0)[0]
    return (REAVER_NAMES[cat], COLOUR[0x7B], ANCHOR_REQ,
            int(round(main * ANCHOR_SCALE)), int(round(sub * ANCHOR_SCALE)), price)


def floor_from_rows(cat, rows):
    """`(name, colour, req, main, sub, price)` = the strongest row of one member that is not
    ours (no `"` row, no FLOOR_EXCLUDE_NAMES) — and, for a weapon cat, not the reaver row
    either: its 75차 ANCHOR stands in for it (152차-B). `rows` = `(raw_name, stats)` pairs in
    record order; ties keep the first, as the reaver row (appended last) always did."""
    rows = list(rows)
    best = None
    for raw, stats in rows:
        if not raw or len(stats) < 16:
            continue
        mark = COLOUR.get(raw[0], "")
        name = (raw[1:] if mark else raw).decode("euc-kr", "replace")
        if raw[0] == NEW_MARK or name in FLOOR_EXCLUDE_NAMES:
            continue          # our own rows are not the floor (see FLOOR_EXCLUDE_NAMES)
        if REAVER_NAMES.get(cat) == name:
            continue          # 152차-B: the anchor below stands in for the reaver row
        main = struct.unpack_from("<H", stats, 12)[0]
        row = (name, mark, stats[8], main,
               struct.unpack_from("<H", stats, 14)[0],
               struct.unpack_from("<I", stats, 0)[0])
        if best is None or main > best[3]:
            best = row
    if cat in REAVER_NAMES:
        white = white_best(rows)
        if white is not None:
            anchor = anchor_row(cat, white)
            if best is None or anchor[3] > best[3]:
                best = anchor
    return best

PCT_IDS = {0x05, 0x06, 0x11}          # these option ids are percentages, not flats

# statId per option label (`addStat` 0x4ee34 jump table, LEGACY_ITEMS_70_99.md §4) —
# what `add_legacy_items` writes into b16..19 / b14..17. ⛔ 0x00/0x01/0x04 are outside
# the gate and do nothing; they are deliberately absent.
OPT_ID = {"최대HP": 0x02, "HP자동회복": 0x03, "물리공격%": 0x05, "특수공격%": 0x06,
          "물리방어": 0x07, "특수방어": 0x08, "명중": 0x09, "회피": 0x0A, "블록": 0x0B,
          "기절": 0x0C, "기절저항": 0x0D, "SP소모감소": 0x0E, "SP회복": 0x0F,
          "HP흡수": 0x10, "쿨타임-%": 0x11, "방패무시": 0x12}

# ---- A. six armour sets, hero1 -> hero2 -> hero3 -----------------------------
# (lv, 세트, 시대, {cat: 이름}, 옵션1, 옵션2, flat값, 근거)
SETS = [
    (73, "템플러", "hero1",
     {0: "템플러헬름", 1: "템플러플레이트", 2: "템플러건틀릿", 3: "템플러그리브",
      11: "케네스방패"},
     (0x07, "물리방어"), (0x08, "특수방어"), 20,
     "hero1 장로회 직속 무장조직 · 방패는 호위 기사 케네스"),
    (79, "크루세이더", "hero1",
     {0: "크루세이더후드", 1: "크루세이더코트", 2: "크루세이더핸드", 3: "크루세이더워커",
      11: "닉스의문장"},
     (0x0A, "회피"), (0x0C, "기절"), 26,
     "hero1 저항세력 · 창시자 닉스는 hero3 에서 닉스 코어의 어원이 된다"),
    (85, "레갈리스근위", "hero2",
     {0: "근위관", 1: "레갈리스법의", 2: "레갈리스성갑", 3: "근위장화",
      11: "디미트리의방패"},
     (0x0B, "블록"), (0x0F, "SP회복"), 33,
     "hero2 레갈리스 교단 · 방패는 근위대장 디미트리"),
    (91, "코르버스", "hero2 → hero3",
     {0: "코르버스후드", 1: "코르버스암살복", 2: "코르버스크로우", 3: "코르버스스텝",
      11: "듀리안의버클러"},
     (0x09, "명중"), (0x10, "HP흡수"), 41,
     "hero2 도적 길드 → hero3 에서 케네스가 이끈다(붉은 늑대 문신)"),
    (97, "붉은마녀", "hero3 케이 · 네오솔티아",
     {0: "붉은마녀헬름", 1: "붉은마녀로브", 2: "붉은마녀장갑", 3: "붉은마녀신발",
      11: "리파이너의가호"},
     (0x06, "특수공격%"), (0x02, "최대HP"), 50,
     "hero3 네메시스 고원을 막아선 「붉은 마녀」 레아 · 네오솔티아를 지킨다"),
    (97, "빙해", "hero3 리츠 · 아스크라",
     {0: "빙해의투구", 1: "빙해의갑옷", 2: "빙해의장갑", 3: "빙해의장화",
      11: "왕국의철벽"},
     (0x05, "물리공격%"), (0x12, "방패무시"), 50,
     "hero2·hero3 「빙해의 검사」 이안 · 섬광의 기사단을 전멸시킨 돌파력"),
]
SET_PCT = {73: 12, 79: 15, 85: 18, 91: 21, 97: 25}

# ---- B. weapon ladder, 7 categories x 3 rungs (Lv99 은 전설로 빠졌다) ---------
WLV = [74, 82, 90]
ERA = ["hero1", "hero2", "hero3"]
W_PCT = {74: 20, 82: 22, 90: 25}
W_FLAT = {74: 20, 82: 26, 90: 33}
WEAPONS = {
    5: ("스워드", "공용",
        [("이안의송검", "hero1 이안 · 레어 양손검 「송 블레이드」"),
         ("클레르클레이모어", "hero2 클레르(버서커) · 양손대검"),
         ("가면검사의검", "hero3 아스크라가 고용한 최강의 용병 「가면의 검사」")],
        (0x05, "물리공격%"), (0x0C, "기절")),
    4: ("스피어", "공용",
        [("레아의유니콘창", "hero1 레아 · 레어 창 「유니콘의뿔」"),
         ("디미트리의창", "hero2 근위대장 디미트리"),
         ("멜페토의공국창", "hero3 멜페토 · 멜하르 공국 기사단 부단장 출신")],
        (0x05, "물리공격%"), (0x09, "명중")),
    6: ("나이프", "공용",
        [("더글라스의밀검", "hero1 크루세이더에 숨어든 템플러 더글라스"),
         ("하인츠의쌍단검", "hero2 코르버스 대장 하인츠"),
         ("큐의암살단검", "hero3 네오솔티아 군사지도자 큐 · 검은 공장 선제공격")],
        (0x05, "물리공격%"), (0x11, "쿨타임-%")),
    7: ("건", "아스크라 전용",
        [("에레보스화승총", "hero1 · 케네스가 두 눈으로 확인한 지상 세계"),
         ("볼프강의호신총", "hero2 윈터하츠의 정보업자 볼프강"),
         ("에이블의밀수총", "hero3 토레즈 시장 에이블 · 전쟁으로 돈을 번다")],
        (0x06, "특수공격%"), (0x11, "쿨타임-%")),
    8: ("라이플", "아스크라 전용",
        [("에레보스공성포", "hero1 · 총기로 분쟁을 반복해온 전란의 대륙"),
         ("통일전쟁중포", "hero2 배경 · 이안과 케네스가 주도한 3년 전쟁"),
         ("토레즈중포", "hero3 아스크라 대도시 토레즈의 군수품")],
        (0x06, "특수공격%"), (0x0C, "기절")),
    9: ("다크", "네오솔티아 전용",
        [("잊혀진신전의핵", "hero1 최종장소 · 가디언 핵 보관소"),
         ("오딘의코어조각", "hero2 초대 교황이 넷으로 쪼갠 바람의 가디언 코어"),
         ("일레느의연구석", "hero3 일레느 · 200년 만에 닉스 코어를 완성한다")],
        (0x06, "특수공격%"), (0x10, "HP흡수")),
    10: ("홀리", "네오솔티아 전용",
         [("리파이너의성석", "hero1 가디언을 제어하는 리파이너"),
          ("굴베이그의유산", "hero2 레갈리스 교단 창시자"),
          ("테너의성표", "hero3 네오솔티아의 장로 · 온건파")],
         (0x06, "특수공격%"), (0x0F, "SP회복")),
}

# ---- C. 전설 — 방어구 10종 + a(무기 7 · 방패 1). 전부 Lv99, 세트가 아니라 단품 ----
# 값 사다리: 기본 = %형 38 / flat 75   (= Lv97 풀세트 옵션의 30 %)
#            치중축 = %형 45 / flat 90 (= 36 %)
LEG_PCT, LEG_PCT_MAX = 38, 45
LEG_FLAT, LEG_FLAT_MAX = 75, 90

# ★ 사용자 결정: 전설 「10종」은 방어구만 센다 — 투구·갑옷·장갑·신발 2개씩(8) +
#   컨셉이 가장 선명한 두 부위(갑옷·장갑)에 하나씩 더(2). 무기와 방패는 그 10 밖의 +a 다.
LEGENDS_ARMOUR = [
    (0, "예언자의눈", "명중 · 치명",
     (0x09, "명중", LEG_FLAT_MAX), (0x0C, "기절", LEG_FLAT_MAX),
     "hero2 예언자의 무덤 · 굴베이그의 글이 남은 곳"),
    # ★ 158차 P5·P7 (2026-10-02, 사용자 «전부 권장대로»): 기절저항 90 → 최대HP 90 — 저항은 Lv 99 의 헬에서
    #   효과 0(몬스터 크리 = 2·레벨차 − 저항, 헬 레벨차 ≤ 1), 탑 100층도 42 면 바닥(`ITEM_BALANCE_AUDIT_156.md §0`).
    (0, "장로회의관", "정신 방어 · 극",
     (0x08, "특수방어", LEG_FLAT_MAX), (0x02, "최대HP", LEG_FLAT_MAX),
     "hero1 장로회 · 가디언을 부려 솔티아를 유지하던 수뇌부"),
    (1, "멜하르의흉갑", "극방 · 불굴",
     (0x07, "물리방어", LEG_FLAT_MAX), (0x02, "최대HP", LEG_FLAT_MAX),
     "hero3 이안에게 무너진 멜하르 공국 · 리츠와 멜페토가 나온 기사단"),
    # ★ 158차 P5: HP자동회복 90 → 최대HP 90 — 자연 회복은 가만히 서 있을 때만 돈다(비전투). 멜하르의흉갑
    #   (물방 90 + HP 90)의 특수방어 짝이 된다.
    (1, "율리케의성의", "자연력 방어",
     (0x08, "특수방어", LEG_FLAT_MAX), (0x02, "최대HP", LEG_FLAT_MAX),
     "hero2 교황 율리케 · 오딘을 부린 리파이너의 제의"),
    (1, "광전사의흑갑", "공격 완전 치중",
     (0x05, "물리공격%", LEG_PCT_MAX), (0x06, "특수공격%", LEG_PCT_MAX),
     "hero1 레어 양손검 「광전사의 검」 — 끝없는 분노를 일으킨다"),
    (2, "집행자의손", "극공(물리)",
     (0x05, "물리공격%", LEG_PCT_MAX), (0x0C, "기절", LEG_FLAT_MAX),
     "hero1 레어 한손검 「집행자의 칼」 — 대대로 참수형에 쓰인 칼"),
    (2, "오딘의손길", "극공(특수)",
     (0x06, "특수공격%", LEG_PCT_MAX), (0x11, "쿨타임-%", LEG_PCT),
     "hero2 바람의 가디언 오딘 · 자연력 그 자체"),
    # ★ 158차 P5: SP회복 90 → 물리공격 45 % — SP소모 30 에서 SP 가 바닥이라 SP회복은 쓸 데가 없었다(§0 2).
    #   짐승의 발톱 = 물리. 집행자의손(물공 45 · 기절 90) · 오딘의손길(특공 45 · 쿨타임 38)과 다른 쌍.
    (2, "키드의발톱", "흡혈 · 지속",
     (0x10, "HP흡수", LEG_FLAT_MAX), (0x05, "물리공격%", LEG_PCT_MAX),
     "hero2 야수 「키드」 · 진화제로 키워내는 전투 동반자"),
    (3, "바질의발걸음", "회피 · 블록",
     (0x0A, "회피", LEG_FLAT_MAX), (0x0B, "블록", LEG_FLAT_MAX),
     "hero2 언더시티의 바질 · 갇힌 클레르를 빠져나가게 한다"),
    (3, "섬광의군화", "스킬 사용속도",
     (0x11, "쿨타임-%", LEG_PCT_MAX), (0x0F, "SP회복", LEG_FLAT_MAX),
     "hero3 창 주력기 「섬광」 · 게임 자신의 패시브 「속공」과 같은 (0x11 + 0x0f) 조합"),
]

LEGENDS_WEAPON = [
    (5, "빙해의대검", "극공 · 일격",
     (0x05, "물리공격%", LEG_PCT_MAX), (0x0C, "기절", LEG_FLAT_MAX),
     "이안의 이명 「빙해의 검사」 · 공격에 완전히 치중한 한 자루"),
    (4, "닉스의수호창", "공수겸비",
     (0x05, "물리공격%", LEG_PCT), (0x07, "물리방어", LEG_FLAT),
     "hero1 크루세이더 창시자이자 굴베이그의 수호 기사"),
    (6, "비스트의송곳니", "속공 · 흡혈",
     (0x11, "쿨타임-%", LEG_PCT_MAX), (0x10, "HP흡수", LEG_FLAT_MAX),
     "굴베이그 일기 · 폭주해 비스트라 명명된 가디언 실패작"),
    (7, "검은공장의걸작", "연사 · 명중",
     (0x11, "쿨타임-%", LEG_PCT), (0x09, "명중", LEG_FLAT_MAX),
     "hero3 아스크라의 검은 공장 · 큐가 선제공격한 그곳"),
    (8, "시즈타이탄주포", "극공(특수) · 파괴",
     (0x06, "특수공격%", LEG_PCT_MAX), (0x0C, "기절", LEG_FLAT_MAX),
     "hero3 기계형 보스 · 꼭대기 캐논 3연속 속사"),
    (9, "게브의파편", "흡혈 · 생존",
     (0x10, "HP흡수", LEG_FLAT_MAX), (0x02, "최대HP", LEG_FLAT_MAX),
     "hero1 땅의 가디언 게브 · Lv82 오딘 조각과 같은 가디언 코어 계보"),
    # ★ 158차 P5: SP회복 90 + SP소모 90 → 특수공격 45 % + 쿨타임 −38 % — 두 옵션 다 죽어 있었다(SP소모는 30 에서
    #   바닥, 그 뒤 SP회복은 무의미 → 화력 +2 % 로 90제 테너의성표보다 약한 전설, §0 1). 「지속 · 지원」은 쿨타임으로
    #   남긴다(P2 상한 60 % 안에서 유효). 검은공장의걸작(쿨타임 38 · 명중 90)과 다른 쌍.
    (10, "시엔의목소리", "지속 · 지원",
     (0x06, "특수공격%", LEG_PCT_MAX), (0x11, "쿨타임-%", LEG_PCT),
     "hero3 시엔 · 감정을 잃고도 자아를 남긴 홀리 가디언"),
    (11, "영혼의수호", "극방",
     (0x07, "물리방어", LEG_FLAT_MAX), (0x08, "특수방어", LEG_FLAT_MAX),
     "굴베이그가 닉스에게 지은 「영혼의 시」 — 「그대 영혼의 수호가 나와 함께」"),
]
LEGENDS = LEGENDS_ARMOUR + LEGENDS_WEAPON

# ---- D. accessories (cat 0x0c, 18-byte records) ------------------------------
ACCESSORIES = [
    ("율리케의목걸이", 75, 18, ("특수공격%", 30), ("SP회복", 40),
     "hero2 교황 율리케 · 오딘을 부리는 리파이너"),
    ("레갈리스교단반지", 75, 18, ("물리방어", 40), ("특수방어", 40),
     "hero2 레갈리스 교단"),
    ("엘지스의증표", 75, 18, ("블록", 40), ("기절저항", 40),
     "hero3 전쟁을 반대한 조직 · 멜페토에게 몰살당한다"),
    ("이안의맹세", 85, 20, ("물리공격%", 30), ("기절", 40),
     "hero1 itm/21 「반지」(연인의 증표) + 이안"),
    ("레아의목걸이", 85, 20, ("명중", 40), ("회피", 40),
     "hero1 itm/21 「목걸이」(보석 장식) + 레아"),
    ("일레느의각인", 85, 20, ("특수공격%", 30), ("SP소모감소", 40),
     "hero3 닉스 코어를 완성한 연구자"),
    ("클레르의가보", 95, 24, ("물리공격%", 35), ("특수공격%", 35),
     "hero2 클레르의 가보 = 오딘 코어 4조각 중 하나"),
    ("굴베이그의탈리스만", 95, 24, ("최대HP", 60), ("쿨타임-%", 30),
     "hero2 초대 교황 굴베이그"),
    ("닉스의펜던트", 95, 24, ("명중", 60), ("HP흡수", 40),
     "굴베이그 일기 811.6.11 「닉스의 펜던트 신호가 사라졌다」"),
]

SET_PARTS = (0, 1, 2, 3, 11)


def records(blob):
    p = i = 0
    while p + 2 <= len(blob):
        rl = blob[p] | (blob[p + 1] << 8)
        if rl == 0 or p + 2 + rl > len(blob):
            break
        yield i, blob[p + 2:p + 2 + rl]
        p += 2 + rl
        i += 1


def load_floor(zip_path):
    """-> {cat: (name, colour, req, main, sub, price)} = the strongest row per slot."""
    z = zipfile.ZipFile(zip_path)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit("expected exactly one jar in %s" % zip_path)
    j = zipfile.ZipFile(io.BytesIO(z.read(jars[0])))
    dats = {}
    for n in j.namelist():
        b = os.path.basename(n)
        if b.startswith("i") and b.endswith("_dat") and b[1:-4].isdigit():
            dats[int(b[1:-4])] = j.read(n)
    floor = {}
    for cat in range(0, 12):
        rows = []
        for _idx, body in records(dats[cat]):
            nl = body[0]
            raw, stats = body[1:1 + nl], body[1 + nl:]
            if len(stats) < 20:
                raise SystemExit("i%d_dat: stats are %d B, want 20" % (cat, len(stats)))
            rows.append((raw, stats))
        floor[cat] = floor_from_rows(cat, rows)   # 152차-B: weapons take the 75차 anchor
    return floor


def scale_row(floor_row, lv, g):
    """One (main, sub, price) rung from a single floor tuple — the shape
    `add_legacy_items` has (it sees one member at a time, never the whole zip)."""
    _name, _mk, _req, main, sub, price = floor_row
    f = g ** (lv - L0)
    m = int(math.ceil(main * f))
    s = int(round(m * sub / main)) if sub else 0
    p = int(round(price * f / 1000) * 1000)
    if p % 256 == 0:          # canDeposit 0x6d70 returns the price's low byte
        p += 1000
    return m, s, p


def scale(floor, cat, lv, g):
    return scale_row(floor[cat], lv, g)


def set_opt(lv, opt, flat):
    return SET_PCT[lv] if opt[0] in PCT_IDS else flat


# ★ 131차-C shipped the boots (i3 idx 0x31, `@섬광의군화`, 쿨타임-45 + 공격속도+127) as
# the first attack-speed carrier. The file must not hold two rows of that name, and the
# shipped row is STRONGER than this draft's (attack speed exists now), so the draft row
# is the one that yields. 77 rows ship, not 78.
SKIP_SHIPPED = {"섬광의군화"}


def legacy_planned_rows(cat, floor_row, g=1.036):
    """The 133차 rows (요구레벨 73~99) — what `planned_rows` returned until 142차. Kept
    because the 142차 rework is DEFINED against it (same rows, same order, same names;
    only level / stats / options move) and `--rework` prints the two side by side.

    The rows `add_legacy_items` appends to i<cat>_dat, IN APPEND ORDER (= the idx
    order the hell drop table relies on): sets by SETS order, then the weapon ladder,
    then legends in LEGENDS order, then accessories. Option ids included.

    `floor_row` is the member's own floor tuple (see `load_floor` for the shape); for
    cat 12 it is unused (accessories carry fixed 성능/원가, rule R7)."""
    out = []
    if cat in SET_PARTS:
        for lv, _sname, _era, pieces, o1, o2, flat, _why in SETS:
            m, s, p = scale_row(floor_row, lv, g)
            v1, v2 = set_opt(lv, o1, flat), set_opt(lv, o2, flat)
            out.append(dict(name=pieces[cat], lv=lv, main=m, sub=s, price=p,
                            o1=(o1[0], v1), o2=(o2[0], v2)))
    if cat in WEAPONS:
        _t, _side, names, o1, o2 = WEAPONS[cat]
        for i, lv in enumerate(WLV):
            m, s, p = scale_row(floor_row, lv, g)
            v1 = W_PCT[lv] if o1[0] in PCT_IDS else W_FLAT[lv]
            v2 = W_PCT[lv] if o2[0] in PCT_IDS else W_FLAT[lv]
            out.append(dict(name=names[i][0], lv=lv, main=m, sub=s, price=p,
                            o1=(o1[0], v1), o2=(o2[0], v2)))
    for lcat, nm, _concept, o1, o2, _why in LEGENDS:
        if lcat != cat or nm in SKIP_SHIPPED:
            continue
        m, s, p = scale_row(floor_row, LEG_LV, g)
        out.append(dict(name=nm, lv=LEG_LV, main=m, sub=s, price=p,
                        o1=(o1[0], o1[2]), o2=(o2[0], o2[2])))
    if cat == 12:
        for n, lv, m, o1, o2, _why in ACCESSORIES:
            p = 30000 + (lv - L0) * 2000
            if p % 256 == 0:
                p += 1000
            out.append(dict(name=n, lv=lv, main=m, sub=0, price=p,
                            o1=(OPT_ID[o1[0]], o1[1]), o2=(OPT_ID[o2[0]], o2[1])))
    return out


def build_rows(fl, g):
    """모든 신규 행을 한 목록으로. 통합 스펙표가 이것 하나를 쓴다."""
    out = []
    for lv, sname, era, pieces, o1, o2, flat, _why in SETS:
        v1, v2 = set_opt(lv, o1, flat), set_opt(lv, o2, flat)
        for cat in SET_PARTS:
            m, _s, p = scale(fl, cat, lv, g)
            out.append(dict(kind="set", cat=cat, lv=lv, name=pieces[cat], main=m,
                            sub=None, o1=(o1[1], v1), o2=(o2[1], v2), price=p,
                            line="%s 세트 (%s)" % (sname, era)))
    for cat in (5, 4, 6, 7, 8, 9, 10):
        tname, side, names, o1, o2 = WEAPONS[cat]
        for i, lv in enumerate(WLV):
            m, sb, p = scale(fl, cat, lv, g)
            v1 = W_PCT[lv] if o1[0] in PCT_IDS else W_FLAT[lv]
            v2 = W_PCT[lv] if o2[0] in PCT_IDS else W_FLAT[lv]
            out.append(dict(kind="weapon", cat=cat, lv=lv, name=names[i][0], main=m,
                            sub=sb, o1=(o1[1], v1), o2=(o2[1], v2), price=p,
                            line="%s · %s" % (ERA[i], side)))
    for cat, nm, concept, o1, o2, _why in LEGENDS:
        m, sb, p = scale(fl, cat, LEG_LV, g)
        out.append(dict(kind="legend", cat=cat, lv=LEG_LV, name=nm, main=m,
                        sub=sb if 4 <= cat <= 10 else None,
                        o1=(o1[1], o1[2]), o2=(o2[1], o2[2]),
                        price=p, line="전설 · %s" % concept))
    for n, lv, m, o1, o2, _why in ACCESSORIES:
        p = 30000 + (lv - L0) * 2000
        if p % 256 == 0:
            p += 1000
        out.append(dict(kind="acc", cat=12, lv=lv, name=n, main=m, sub=None,
                        o1=o1, o2=o2, price=p, line="악세"))
    return out


def spec_table(fl, g):
    """슬롯별로 묶은 통합 스펙표."""
    rows = build_rows(fl, g)
    for r in rows:                       # 자가검사
        if r["cat"] == 12:
            continue
        m, sb, p = scale(fl, r["cat"], r["lv"], g)
        assert r["main"] == m and r["price"] == p, r["name"]
        assert r["sub"] in (None, sb), r["name"]
    print("### 5.7 통합 스펙표 — 슬롯별 %d행" % len(rows))
    print()
    print("스탯 블록으로 옮기면: `b7`=5(등급) · `b8`=Lv · `b9`=8(홈) · `b12,13`=주스탯 ·")
    print("`b14,15`=보조 · `b16..19`=(옵션id,값)×2 · `b0..3`=원가. 악세(cat 0x0c)는 18바이트라")
    print("`b5`=등급 · `b6`=Lv · `b10,11`=성능 · `b14..17`=옵션이고 `b7`=0(혼 아님).")
    for cat in [0, 1, 2, 3, 11, 4, 5, 6, 7, 8, 9, 10, 12]:
        mine = [r for r in rows if r["cat"] == cat]
        if not mine:
            continue
        mine.sort(key=lambda r: (r["lv"], r["name"]))
        label = "악세·혼" if cat == 12 else SLOT[cat]
        base = "" if cat == 12 else "   (바닥값 %s %d%s)" % (
            fl[cat][0], fl[cat][3], "/%d" % fl[cat][4] if fl[cat][4] else "")
        print()
        print("**cat 0x%02x · %s**%s" % (cat, label, base))
        print()
        unit = "성능" if cat == 12 else ("공격력" if 4 <= cat <= 10 else "방어력")
        print("| Lv | 이름 | %s | 보조 | 옵션 1 | 옵션 2 | 원가 | 계열 |" % unit)
        print("|---|---|---|---|---|---|---|---|")
        for r in mine:
            print("| %d | `%s%s` | **%d** | %s | %s+%d | %s+%d | %s | %s |"
                  % (r["lv"], Q, r["name"], r["main"],
                     r["sub"] if r["sub"] else "—",
                     r["o1"][0], r["o1"][1], r["o2"][0], r["o2"][1],
                     format(r["price"], ","), r["line"]))
    print()
    print("합계 **%d행** — 세트 30 · 무기 사다리 21 · 전설 18(방어구 10 + 무기·방패 8) · 악세 9."
          % len(rows))


# ==== 142차 개편안 (2026-09-30) — 요구레벨 두 단: 85제(컨셉) / 90제(최종) ==============
# 사용자 지시: 최종 아이템 = 90제, 나머지 전부 85제(73제 같은 것은 없다). 90제 = 지금의 99제
# 그대로, 90~97 은 99제와 거의 같게, 73~85 는 전부 85제 — 스탯은 90제보다 살짝 낮고 대신 컨셉
# (세트 효과 등)으로 갈린다. 정본 = LEGACY_ITEMS_REWORK_85_90.md.
#
# ★ 142차 확정(사용자 2026-09-30): `planned_rows`(= add_legacy_items 가 에셋에 쓰는 것)가
#   이 절이다. 옛 표는 `legacy_planned_rows` 로 남아 `--rework` 가 둘을 나란히 찍는다. 행
#   순서·이름·개수(78)는 옛 표와 **같다** — idx 가 안 움직여야 세이브·헬 드랍 표·조합서가
#   그대로 산다(이 절의 불변식이 검사한다). 기본 출력(§5 표)은 133차 설계의 기록이다.
RW_FINAL, RW_CONCEPT = 90, 85
RW_FINAL_OF_LEGEND = 0.98    # 90~97 → 99제(=전설) 주스탯의 98 % («거의 동일»)
RW_CONCEPT_OF_LEGEND = 0.90  # 85제 → 전설 주스탯의 90 % («살짝 떨어져도»)
RW_ATK_SPEED = 0x76          # atkspeed::ATTACK_SPEED_STAT (131차-D, 라벨 인덱스 198 − 80)

# 90제 비전설 옵션: 무기 사다리는 전설 「기본」 값(38 % / 75), 세트는 97 세트의 크기(25 % / 50).
RW_W90 = {"pct": LEG_PCT, "flat": LEG_FLAT}
RW_SET90_FLAT, RW_SET90_PCT = 50, 25
# 85제 세트 조각: 세 세트 모두 같은 크기(조각당 flat 30) — 차이는 세트 효과가 낸다. (156차부터 붉은마녀
# 밖의 조각은 `RW_SET_PIECE_OPTIONS` 가 쌍을 정하고, 이 둘은 붉은마녀와 값 규칙 설명에만 남는다.)
RW_SET85_FLAT = 30
# ★★★ 156차 (2026-10-02, 사용자 확정 — `LEGACY_ITEMS_REWORK_85_90.md §13.3` 초안 + 수정 셋): 세트 조각 다섯이
#   서로 다른 옵션 쌍을 갖는다(142차~155차는 `SETS` 의 한 쌍을 다섯 조각에 그대로 복사했다 — 신규 표의 세트 절
#   다섯 줄이 효과 칸까지 같았다). 컨셉 유지 = 세트의 대표 옵션이 조각에 남는다 · 값은 85제 30 · 90제 25 %/50
#   그대로(공격속도만 85제에서도 +50 — 132차-B 실측 +25 ≈ 0) · 붉은마녀는 표에 없다(= `SETS` 한 쌍 ×5, 사용자
#   예외 「예외: 붉은 마녀세트」). 사용자 수정(초안 대비, 2026-10-02): 왕국의철벽 방패무시 +50% → 블록 +30 ·
#   근위관 SP회복 → 물리방어 +30% · 레갈리스법의 SP회복 → 최대HP +50% · 근위장화 SP회복 → SP소모 -30%
#   (⚠ Lv70+ 에서 스킬 SP 가 사실상 0 — §11.2; 일레느의각인 · 검은 눈물과 같은 값, 사용자가 알고 고른 것) —
#   그래서 레갈리스근위에는 SP회복이 남지 않고 대표 옵션은 블록 하나다. ⛔ 이름이 열쇠다(`SETS` 의 조각 이름 그대로)
#   — `rw_set_piece_options_check` 가 25행 · 세트 안 다섯 쌍 서로 다름 · id 게이트 · i8 값 · 붉은마녀 = 한 쌍을
#   못 박고 `--rework` 의 ALL OK 에 든다. 바뀌는 에셋 멤버 = i0 · i1 · i2 · i3 · i11 + 조합서 문구 i15(151차).
#   스탯은 DB 행이라 이미 가진 조각도 다음 로드부터 새 옵션이다.
RW_SET_PIECE_FREE = "붉은마녀"       # the one set that keeps `SETS`' pair on every piece
RW_SET_PIECE_OPTIONS = {
    # 템플러 (85제 · 수비 · 수호의 맹세) — 대표 = 물리방어 · 특수방어
    # ★ 158차 P7 (2026-10-02, 사용자 «전부 권장대로» — 권장 = 「저항 → HP% 는 하고, 방어% 는 템플러 둘만」):
    #   헬름 저항 +30 → 블록 +30(저항은 헬에서 0; HP% 는 그리브가 이미 가져 「같은 쌍 둘」 금지에 걸린다 — 템플러
    #   = 나이트템플러의 세트, 창 직업은 회피 대신 블록을 굴린다 `0x4d2ec`) · 플레이트 특수방어 → 최대HP +30% ·
    #   건틀릿 물리방어 → 블록 +30. 방어%(+30 = 생존 +1~4 %)는 플레이트(물방) · 그리브(특방) · 케네스방패(물방)에
    #   남아 대표 옵션은 유지된다. 근위관 · 레갈리스성갑 · 왕국의철벽은 156차 사용자 수정이라 손대지 않았다.
    "템플러헬름":     ((0x08, 30), (0x0B, 30)),   # 특수방어 +30% · 블록 +30         (158차: 저항 → 블록)
    "템플러플레이트": ((0x07, 30), (0x02, 30)),   # 물리방어 +30% · 최대HP +30%      (158차: 특수방어 → 최대HP)
    "템플러건틀릿":   ((0x0B, 30), (0x03, 30)),   # 블록 +30 · HP회복 +30%           (158차: 물리방어 → 블록)
    "템플러그리브":   ((0x08, 30), (0x02, 30)),   # 특수방어 +30% · 최대HP +30%
    "케네스방패":     ((0x07, 30), (0x0B, 30)),   # 물리방어 +30% · 블록 +30
    # 크루세이더 (85제 · 기습) — 대표 = 회피 · 기절
    "크루세이더후드": ((0x0A, 30), (0x09, 30)),   # 회피 +30 · 명중 +30
    "크루세이더코트": ((0x0A, 30), (0x0C, 30)),   # 회피 +30 · 기절 +30             (= SETS)
    "크루세이더핸드": ((0x0C, 30), (0x12, 30)),   # 기절 +30 · 방패무시 +30%
    "크루세이더워커": ((0x0A, 30), (RW_ATK_SPEED, 50)),   # 회피 +30 · 공격속도 +50 (85제 예외)
    "닉스의문장":     ((0x0C, 30), (0x0B, 30)),   # 기절 +30 · 블록 +30
    # 레갈리스근위 (85제 · 교단의 가호) — 대표 = 블록 (SP회복은 사용자 수정으로 전부 빠졌다)
    "근위관":         ((0x07, 30), (0x08, 30)),   # 물리방어 +30% · 특수방어 +30%   (사용자: SP회복 → 물리방어)
    "레갈리스법의":   ((0x0B, 30), (0x02, 50)),   # 블록 +30 · 최대HP +50%          (사용자: SP회복 → 최대HP 50)
    "레갈리스성갑":   ((0x0B, 30), (0x07, 30)),   # 블록 +30 · 물리방어 +30%
    "근위장화":       ((0x0E, 30), (0x03, 30)),   # SP소모 -30% · HP회복 +30%       (사용자: SP회복 → SP소모)
    "디미트리의방패": ((0x0B, 30), (0x02, 30)),   # 블록 +30 · 최대HP +30%
    # 코르버스 (90제 · 처형) — 대표 = 명중 · HP흡수
    "코르버스후드":   ((0x09, 50), (0x0C, 50)),   # 명중 +50 · 기절 +50
    "코르버스암살복": ((0x09, 50), (0x10, 50)),   # 명중 +50 · HP흡수 +50%          (= SETS)
    "코르버스크로우": ((0x10, 50), (0x11, 25)),   # HP흡수 +50% · 쿨타임 -25%
    "코르버스스텝":   ((0x09, 50), (0x0A, 50)),   # 명중 +50 · 회피 +50
    "듀리안의버클러": ((0x10, 50), (0x0B, 50)),   # HP흡수 +50% · 블록 +50
    # 빙해 (90제 · 빙해의 일격) — 대표 = 물리공격 · 방패무시
    "빙해의투구":     ((0x12, 50), (0x09, 50)),   # 방패무시 +50% · 명중 +50
    "빙해의갑옷":     ((0x05, 25), (0x12, 50)),   # 물리공격 +25% · 방패무시 +50%   (= SETS)
    "빙해의장갑":     ((0x05, 25), (0x0C, 50)),   # 물리공격 +25% · 기절 +50
    "빙해의장화":     ((0x05, 25), (RW_ATK_SPEED, 50)),   # 물리공격 +25% · 공격속도 +50
    "왕국의철벽":     ((0x0B, 30), (0x07, 50)),   # 블록 +30 · 물리방어 +50%        (사용자: 방패무시 +50% → 블록 +30)
}
# `addStat 0x4ee34` 의 게이트(id-2 <= 0x15, 0x04 는 무동작) + 호스트가 읽는 공격속도.
RW_OPT_GATE = {i for i in range(0x02, 0x18) if i != 0x04} | {RW_ATK_SPEED}
# 85제 무기: 주옵션 30 % + 무기마다 다른 둘째 옵션(같은 카테고리의 두 자루가 같은 행이 되지 않게).
RW_W85_PCT, RW_W85_FLAT, RW_W85_SPEED, RW_W85_CD = 30, 60, 50, 25
RW_W85_SECOND = {
    "이안의송검": (0x0C, "기절"),            "클레르클레이모어": (RW_ATK_SPEED, "공격속도"),
    "레아의유니콘창": (0x09, "명중"),        "디미트리의창": (0x0B, "블록"),
    "더글라스의밀검": (0x0A, "회피"),        "하인츠의쌍단검": (RW_ATK_SPEED, "공격속도"),
    # ★ 158차 P6 (2026-10-02, 사용자 «전부 권장대로»): 둘째 옵션이 화력 0 이던 네 자루 교체 — 볼프강 쿨타임 25 →
    #   공격속도 50 · 오딘의코어조각 SP소모 60 → 명중 60 · 리파이너의성석 SP회복 60 → 공격속도 50 · 굴베이그의유산
    #   HP회복 60 → 기절 60. 잊혀진신전의핵(HP흡수 60)은 유지(흡수는 유효하되 30~50 에서 포화). 같은 카테고리의 두
    #   자루는 여전히 다른 둘째 옵션이다(총 명중/공속 · 다크 흡수/명중 · 홀리 공속/기절).
    "에레보스화승총": (0x09, "명중"),        "볼프강의호신총": (RW_ATK_SPEED, "공격속도"),
    "에레보스공성포": (0x12, "방패무시"),    "통일전쟁중포": (0x0C, "기절"),
    "잊혀진신전의핵": (0x10, "HP흡수"),      "오딘의코어조각": (0x09, "명중"),
    "리파이너의성석": (RW_ATK_SPEED, "공격속도"),  "굴베이그의유산": (0x0C, "기절"),
}
# 85제 악세: 142차엔 셋이 세트의 여섯째 조각(방패를 못 드는 직업의 다섯째 칸), 셋이 단품이었다. (155차:
# 레갈리스교단반지 = 85제 레갈리스근위 · 이안의맹세 = 90제 빙해 · 레아의목걸이 = 90제 붉은마녀, 나머지 여섯은
# 단품. RW_SET_ACCESSORY)
# ★★ 150차 (2026-10-01, `LEGACY_ITEMS_REWORK_85_90.md §11`·§12, 사용자 결정): 장신구 아홉(과 보스 둘)은
#   성능(= 캐릭터의 저항) 0 · 요구 Lv 1 · 옵션 두 쌍은 `acc_options.py` 결정 표가 정한다(셋째 옵션은
#   에셋 밖 — `acc_options.rs`). ⛔ `lv` 열은 계속 등급(85/90제)이다 — 헬 드랍 풀 · 하늘성 보상 ·
#   조합서 · 세트 표 · 가방 정렬이 전부 그 열로 85/90제를 가르고(`legacy_tiers`), 에셋의 요구레벨
#   바이트(b6)는 새 `req` 열(= 1)이 나른다.
RW_ACC_MAIN = AO.PERF        # b10..11 성능 = 0 (142차: 85제 22 · 90제 24)
RW_ACC_REQ_LV = AO.REQ_LV    # b6 요구레벨 = 1 (142차: 등급과 같은 85 · 90)
RW_ACC85_PRICE = 60000
# 세트 소속 장신구 — 세트마다 0개 또는 1개. ⛔ 150차부터 세트 장신구도 자기 옵션(acc_options)을
# 갖는다: 세트 옵션으로 덮어쓰던 142차 분기는 없어졌고, 이 표는 「어느 세트의 조각인가」만 말한다
# (`gen_set_items.py` → `set_items.rs`, 세트 소속은 호스트 표만 안다).
# ★★ 154차 (2026-10-02, 사용자 결정): 세트는 4·5·6부위다 — 「세트가 4개일 수도있고 5개일수도있고 6개일
#   수도있는거야」 · 「4or5or6 이야 7도아니고 3도아니고」(`gen_set_items.PIECES_MIN/MAX` 가 지킨다).
#   레아의목걸이 = 붉은마녀(세트 설명의 「붉은 마녀」 레아 — 142차엔 크루세이더였다) · 크루세이더 = 장신구
#   없이 5부위(「굳이 목걸이를 채울 필요없어」) · 굴베이그의탈리스만 = 세트 밖 단품(142차-C 엔 붉은마녀).
#   ⛔ 장신구에는 세트 등급(85/90제)을 묻지 않는다 — 요구 Lv 1(150차)이라 누구나 끼고, 85제 레아가
#   90제 세트의 조각이다. 등급은 드랍 풀 · 하늘성 보상 · 조합서가 쓰는 열일 뿐이다.
# ★★ 155차 (2026-10-02, 사용자 지시 1 · 3 · 4 — `LEGACY_ITEMS_REWORK_85_90.md §13.2`): 템플러에서 이안의맹세,
#   코르버스에서 닉스의펜던트를 뺀다(둘 다 5부위) · 빙해의 장신구 = 이안의맹세(「빙해의 검사」 이안 — 85제
#   장신구가 90제 세트의 조각, 레아와 같은 규칙) · 클레르의가보 · 닉스의펜던트 = 세트 없는 단품. 세트 장신구
#   셋(레갈리스교단반지 · 레아의목걸이 · 이안의맹세) · 세트 밖 여섯. 조합서 사슬(`add_tower_recipes.ACC_BASE`)은
#   세트와 무관해 그대로다.
RW_SET_ACCESSORY = {"레갈리스근위": "레갈리스교단반지", "붉은마녀": "레아의목걸이", "빙해": "이안의맹세"}
# ★★ 150차 결정 2 — 원작의 방어구 4부위 세트 판정(`0x4f014` 안의 `0x4f270..0x4f354`: 착용 칸
# 1·4·3·7 = 투구·갑옷·장갑·신발의 행 `+0x14` = 20 B 방어구 행 **b11** 이 넷 다 같고 > 0 이면 최대HP ·
# 물리/특수공격 · 물리/특수방어 +20 %, SP소모 −20). 레거시 방어구 34행은 @팬텀 템플릿을 복제해
# 전부 b11 = 6 이었다 — 네 부위를 아무렇게나 섞어도 원작 보너스가 켜졌다. 이제 세트마다 자기 id
# (출고 id 1~5 초록 · 6 @팬텀 · 7 @홀리 다음), 전설 방어구(섬광의군화 포함)는 0, 방패·무기는 0.
RW_NATIVE_SET_ID = {"템플러": 8, "크루세이더": 9, "레갈리스근위": 10,
                    "코르버스": 11, "붉은마녀": 12, "빙해": 13}
RW_ARMOUR_CATS = (0, 1, 2, 3)   # 투구·갑옷·장갑·신발 — the four row tables the game's set check reads
# 150차 결정 5: b10 = the wear-class label (`InGame_txt[0xbb + b10]` on the 상세보기, 0 = none) — the
# 34 legacy armour rows carried the template's 6 「솔티안 전용」 although anyone may wear them.
RW_FACTION_NONE = 0
# 85제 세트 효과 초안(구현 전 — 사용자 확정 대상). 이름 · 한 줄 · 메커니즘.
RW_SET_BONUS = {
    "템플러": ("수호의 맹세",
              "HP 가 10 이하로 떨어지는 피해(치사 포함)를 받으면 HP 1 로 버티고 5초 무적 · 쿨타임 100초",
              "호스트: 매 프레임 HP·상태 바이트 감시 → 101차 제자리 부활 쓰기 + 5초 HP 스냅샷 유지 "
              "(또는 게스트 훅: 몬스터→플레이어 피해 적용 지점)"),
    "크루세이더": ("기습",
                "HP 가 가득 찬 적에게 주는 첫 타격 피해 2배 (158차 P4: ×3 → ×2)",
                "게스트: 기존 dmg_apply_lgt 훅(0x386cd) 안의 분기 하나 — 레코드 hp_cur == hp_max(가상 HP "
                "워드 포함)면 raw 피해 x2 (`wie_util::stats::AMBUSH_MULT`)"),
    "레갈리스근위": ("교단의 가호",
                  "스킬 사용 시 30 % 확률로 쿨타임이 돌지 않는다",
                  "호스트: 쿨타임은 호스트 UI 게이트다(app.rs skill_cd, 원작엔 쿨타임이 없다) — 삽입을 확률로 건너뛴다"),
}


def rw_tier(old_lv):
    return RW_FINAL if old_lv >= 90 else RW_CONCEPT


def rw_scale(floor_row, ratio, g):
    """(main, sub, price) = 전설(99) 행 x ratio. 카테고리 간 리버 비율은 전설 행이 이미 들고 있다."""
    m99, s99, p99 = scale_row(floor_row, LEG_LV, g)
    if ratio == 1.0:
        return m99, s99, p99
    m = int(round(m99 * ratio))
    s = int(round(m * s99 / m99)) if s99 else 0
    p = int(round(p99 * ratio / 1000) * 1000)
    if p % 256 == 0:
        p += 1000
    return m, s, p


def rw_set_name_of(cat, name):
    for _lv, sname, _era, pieces, *_ in SETS:
        if pieces.get(cat) == name:
            return sname
    return None


def rw_set_of_accessory(name):
    """The set this accessory is a piece of (`RW_SET_ACCESSORY`), or None."""
    return next((s for s, a in RW_SET_ACCESSORY.items() if a == name), None)


def rework_planned_rows(cat, floor_row, g=1.036):
    """`planned_rows` 와 같은 행을 같은 순서로 — 레벨·스탯·옵션만 개편안 값으로.

    Every row carries `lv` = the TIER (85 / 90제) and `req` = the 요구레벨 byte the asset gets
    (equipment b8 = the tier; ★ 150차 accessory b6 = `acc_options.REQ_LV` = 1). Equipment rows
    also carry `set_id` = the native set id (b11; 150차 결정 2 — `RW_NATIVE_SET_ID` on the 24
    set armour rows, 0 everywhere else)."""
    out = []
    for r in legacy_planned_rows(cat, floor_row, g):
        r = dict(r)
        old = r["lv"]
        r["old_lv"] = old
        r["lv"] = rw_tier(old)
        sname = rw_set_name_of(cat, r["name"]) if cat in SET_PARTS else None
        is_leg = old == LEG_LV
        if cat == 12:
            # ★ 150차: 성능 0 · 요구 Lv 1 · 옵션 두 쌍 = `acc_options` 결정 표. 세트 장신구도 자기
            # 옵션이다 — 142차의 세트 옵션 덮어쓰기(RW_SET85_FLAT)는 삭제됐다.
            o1, o2 = AO.options_of(r["name"])
            r["o1"], r["o2"] = tuple(o1), tuple(o2)
            r["main"] = RW_ACC_MAIN
            r["req"] = RW_ACC_REQ_LV
            if r["lv"] == RW_CONCEPT:
                r["price"] = RW_ACC85_PRICE + (1000 if RW_ACC85_PRICE % 256 == 0 else 0)
            owner = rw_set_of_accessory(r["name"])
            if owner:
                r["set"] = owner
            out.append(r)
            continue
        r["req"] = r["lv"]
        r["set_id"] = RW_NATIVE_SET_ID[sname] if sname and cat in RW_ARMOUR_CATS else 0
        if is_leg:
            m, s, p = rw_scale(floor_row, 1.0, g)
        elif r["lv"] == RW_FINAL:
            m, s, p = rw_scale(floor_row, RW_FINAL_OF_LEGEND, g)
        else:
            m, s, p = rw_scale(floor_row, RW_CONCEPT_OF_LEGEND, g)
        r["main"], r["sub"], r["price"] = m, s, p
        if sname:
            r["set"] = sname
            piece = RW_SET_PIECE_OPTIONS.get(r["name"])
            if piece:
                # ★ 156차: 조각마다 다른 쌍 — 붉은마녀만 아래의 `SETS` 한 쌍 ×5 로 남는다.
                r["o1"], r["o2"] = tuple(piece[0]), tuple(piece[1])
            else:
                sp = next(x for x in SETS if x[1] == sname)
                ids = (sp[4][0], sp[5][0])
                if r["lv"] == RW_CONCEPT:
                    vals = (RW_SET85_FLAT, RW_SET85_FLAT)
                else:
                    vals = tuple(RW_SET90_PCT if i in PCT_IDS else RW_SET90_FLAT for i in ids)
                r["o1"], r["o2"] = (ids[0], vals[0]), (ids[1], vals[1])
        elif cat in WEAPONS and not is_leg:
            first = WEAPONS[cat][3][0]
            if r["lv"] == RW_FINAL:
                r["o1"] = (first, RW_W90["pct"])
                sid = WEAPONS[cat][4][0]
                r["o2"] = (sid, RW_W90["pct"] if sid in PCT_IDS else RW_W90["flat"])
            else:
                sid, _lab = RW_W85_SECOND[r["name"]]
                val = (RW_W85_SPEED if sid == RW_ATK_SPEED else
                       RW_W85_CD if sid in PCT_IDS else RW_W85_FLAT)
                r["o1"], r["o2"] = (first, RW_W85_PCT), (sid, val)
        out.append(r)
    return out


def planned_rows(cat, floor_row, g=1.036):
    """The rows `add_legacy_items` appends to i<cat>_dat, IN APPEND ORDER — since 142차
    the 85/90제 rework (`rework_planned_rows`). ⛔ Order, names and count are the 133차
    ones on purpose: the (cat, idx) of every row is a save-file identity."""
    return rework_planned_rows(cat, floor_row, g)


LEGACY_ROWS = 78   # 77 planned + `섬광의군화` (add_legacy_boots)


def legacy_tiers():
    """★ 150차: `{(cat, name): 85 | 90}` — the TIER (85제 / 90제) of all 78 `"` rows: the 77
    `planned_rows` plus `섬광의군화` (the boots row `add_legacy_boots` owns; it is in `LEGENDS`).

    Since 150차 an accessory's 요구레벨 byte is 1 (`acc_options.REQ_LV`), so the tier can no
    longer be read off the asset: every downstream table that means 85/90제 — the hell drop
    pools (`gen_helldrop_table`), the tower rewards (`gen_guardian_rewards`), the recipe bands,
    the set table (`gen_set_items`), the bag sort (`gen_item_levels`) — keys on THIS. Pure (no
    archive, no floor): `rw_tier(the 133차 level)`, the rule `rework_planned_rows` applies.
    `gen_helldrop_table.legacy_rows` cross-checks it against the asset (equipment b8 == tier,
    accessory b6 == 1)."""
    out = {}

    def put(cat, name, old_lv):
        if (cat, name) in out:
            raise SystemExit(f"legacy_tiers: ({cat}, {name}) is listed twice")
        out[(cat, name)] = rw_tier(old_lv)

    for lv, _sname, _era, pieces, *_ in SETS:
        for cat, name in pieces.items():
            put(cat, name, lv)
    for cat, (_t, _side, names, _o1, _o2) in WEAPONS.items():
        for (name, _why), lv in zip(names, WLV):
            put(cat, name, lv)
    for cat, name, *_ in LEGENDS:
        put(cat, name, LEG_LV)
    for name, lv, *_ in ACCESSORIES:
        put(12, name, lv)
    if len(out) != LEGACY_ROWS:
        raise SystemExit(f"legacy_tiers: {len(out)} rows, expected {LEGACY_ROWS}")
    return out


RW_OPT_NAME = {v: k for k, v in OPT_ID.items()}
RW_OPT_NAME[RW_ATK_SPEED] = "공격속도"
RW_OPT_NAME[AO.REGEN] = "5초당HP회복%"   # 150차 host stat 0x78 (InGame_txt row 200)


def _third_text(name):
    """The 150차 third option of an accessory, as the 상세보기 prints it (`acc_options`)."""
    a = AO.by_name().get(name)
    return a[6] if a else "—"


def _set_cell(cat, r):
    if cat == 12:
        return "%s (장신구 조각)" % r["set"] if r.get("set") else ""
    s = r.get("set", "")
    return "%s (b11 %d)" % (s, r["set_id"]) if r.get("set_id") else s


def rework_table(fl, g):
    """개편안 78행 + 불변식. 옛 행과 같은 순서·이름·개수인지부터 본다."""
    ok = True
    allrows = []
    print("### 개편안 — 슬롯별 (옛 요구레벨 → 85/90제 등급 · 150차 장신구 요구Lv 1)\n")
    print("| cat | 옛 Lv | **등급** | 요구Lv | 이름 | 옛 주스탯 | **새 주스탯** | 보조 | 옵션 1 | 옵션 2 "
          "| 3번째 옵션 | 원가 | 세트 |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for cat in [0, 1, 2, 3, 11, 4, 5, 6, 7, 8, 9, 10, 12]:
        old = legacy_planned_rows(cat, fl.get(cat), g)
        new = planned_rows(cat, fl.get(cat), g)
        if [r["name"] for r in old] != [r["name"] for r in new]:
            ok = False
            print("FAIL cat %d: 행 순서/이름이 옛 표와 다르다" % cat)
        for o, r in zip(old, new):
            allrows.append((cat, r))
            if r["price"] % 256 == 0:
                ok = False
            print("| %d | %d | **%d** | %d | `%s%s` | %d | **%d** | %s | %s+%d | %s+%d | %s | %s | %s |"
                  % (cat, o["lv"], r["lv"], r["req"], Q, r["name"], o["main"], r["main"],
                     r["sub"] if r.get("sub") else "—",
                     RW_OPT_NAME.get(r["o1"][0], hex(r["o1"][0])), r["o1"][1],
                     RW_OPT_NAME.get(r["o2"][0], hex(r["o2"][0])), r["o2"][1],
                     _third_text(r["name"]) if cat == 12 else "—",
                     format(r["price"], ","), _set_cell(cat, r)))
    n85 = sum(1 for _c, r in allrows if r["lv"] == RW_CONCEPT)
    n90 = sum(1 for _c, r in allrows if r["lv"] == RW_FINAL)
    shipped_boots = 1   # `"섬광의군화` 는 add_legacy_boots 의 행(i3 0x31) — planned_rows 밖, 90제
    print("\n```")
    print("행: planned %d + 섬광의군화 %d = %d  (85제 %d · 90제 %d)"
          % (len(allrows), shipped_boots, len(allrows) + shipped_boots, n85, n90 + shipped_boots))
    for cat in sorted(c for c in fl if c <= 11):
        rs = [r for c, r in allrows if c == cat]
        lo = max([r["main"] for r in rs if r["lv"] == RW_CONCEPT], default=0)
        hi = min([r["main"] for r in rs if r["lv"] == RW_FINAL], default=10 ** 9)
        good = lo < hi and lo > fl[cat][3]
        ok &= good
        print("cat %-2d %-4s 바닥 %-4d  85제 최대 %-5d < 90제 최소 %-5d %s"
              % (cat, SLOT[cat], fl[cat][3], lo, hi, "OK" if good else "FAIL"))

    # ★ 150차 — the accessories carry the decision table (acc_options), the tier column is the
    # tier (legacy_tiers), and the native set ids sit on exactly the 24 set armour rows.
    acc = [r for c, r in allrows if c == 12]
    want_names = sorted(a[0] for a in AO.ACC if a[1] == Q)
    good = sorted(r["name"] for r in acc) == want_names
    bad = [r["name"] for r in acc
           if r["name"] not in AO.by_name() or r["req"] != AO.REQ_LV or r["main"] != AO.PERF
           or (r["o1"], r["o2"]) != tuple(tuple(p) for p in AO.options_of(r["name"]))]
    good &= not bad
    ok &= good
    print("장신구 %d행: 요구Lv %d · 성능 %d · 옵션 = acc_options %s%s"
          % (len(acc), AO.REQ_LV, AO.PERF, "OK" if good else "FAIL",
             "" if not bad else " (%s)" % ", ".join(bad)))
    tiers = legacy_tiers()
    off = [(c, r["name"]) for c, r in allrows if tiers.get((c, r["name"])) != r["lv"]]
    boots = tiers.get((3, "섬광의군화"))
    good = not off and boots == RW_FINAL and len(tiers) == len(allrows) + shipped_boots
    ok &= good
    print("legacy_tiers %d행 = planned 등급 + 섬광의군화 %s %s%s"
          % (len(tiers), boots, "OK" if good else "FAIL", "" if not off else " (%s)" % off))
    ids = {(c, r["name"]): r.get("set_id") for c, r in allrows if c != 12}
    want = {}
    for _lv, sname, _era, pieces, *_ in SETS:
        for cat in RW_ARMOUR_CATS:
            want[(cat, pieces[cat])] = RW_NATIVE_SET_ID[sname]
    wrong = [k for k, v in ids.items() if v != want.get(k, 0)]
    distinct = len(set(RW_NATIVE_SET_ID.values())) == len(SETS)
    in_range = all(8 <= v <= 127 for v in RW_NATIVE_SET_ID.values())   # > the shipped 1..7, i8 > 0
    good = not wrong and distinct and in_range and sum(1 for v in ids.values() if v) == 24
    ok &= good
    print("원작 세트 id(b11) 방어구 %d행 %s → %s · 전설·방패·무기 0 %s%s"
          % (sum(1 for v in ids.values() if v),
             "/".join("%s %d" % (s, i) for s, i in RW_NATIVE_SET_ID.items()),
             "구분됨" if distinct else "겹침", "OK" if good else "FAIL",
             "" if not wrong else " (%s)" % wrong))
    good, msgs = rw_set_piece_options_check(allrows)
    ok &= good
    print("세트 조각 옵션 %d행(붉은마녀 제외) — 세트 안 다섯 쌍 서로 다름 · 두 id 다름 · addStat 게이트 · i8 · 붉은마녀 = 한 쌍 %s%s"
          % (len(RW_SET_PIECE_OPTIONS), "OK" if good else "FAIL", "" if good else " (%s)" % "; ".join(msgs)))
    print("ALL OK: %s" % ok)
    print("```")
    return ok


def rw_set_piece_options_check(allrows):
    """156차 불변식 — (ok, 어긋난 것들). `allrows` = `rework_table` 의 (cat, row) 목록."""
    msgs = []
    owner = {}
    for _lv, sname, _era, pieces, *_ in SETS:
        for cat in SET_PARTS:
            owner[pieces[cat]] = sname
    extra = sorted(set(RW_SET_PIECE_OPTIONS) - set(owner))
    if extra:
        msgs.append("세트 조각이 아닌 이름 %s" % extra)
    missing = sorted(n for n, s in owner.items() if s != RW_SET_PIECE_FREE and n not in RW_SET_PIECE_OPTIONS)
    if missing:
        msgs.append("표에 없는 조각 %s" % missing)
    witch = sorted(n for n in RW_SET_PIECE_OPTIONS if owner.get(n) == RW_SET_PIECE_FREE)
    if witch:
        msgs.append("붉은마녀 조각이 표에 있다 %s" % witch)
    for _lv, sname, _era, _pieces, o1, o2, *_ in SETS:
        rows = [r for c, r in allrows if c in SET_PARTS and r.get("set") == sname]
        if len(rows) != len(SET_PARTS):
            msgs.append("%s: 조각 %d행(%d 이어야)" % (sname, len(rows), len(SET_PARTS)))
        pairs = [(tuple(r["o1"]), tuple(r["o2"])) for r in rows]
        if sname == RW_SET_PIECE_FREE:
            ids = (o1[0], o2[0])
            want = tuple((i, RW_SET90_PCT if i in PCT_IDS else RW_SET90_FLAT) for i in ids)
            if any(p != want for p in pairs):
                msgs.append("%s: 조각이 SETS 의 한 쌍 %s 이 아니다" % (sname, want))
        elif len(set(pairs)) != len(pairs):
            msgs.append("%s: 같은 쌍이 둘 이상" % sname)
        for r in rows:
            if r["o1"][0] == r["o2"][0]:
                msgs.append("%s: 두 옵션이 같은 id" % r["name"])
            for oid, val in (r["o1"], r["o2"]):
                if oid not in RW_OPT_GATE:
                    msgs.append("%s: id %#x 는 addStat 게이트 밖" % (r["name"], oid))
                if not 1 <= val <= 127:
                    msgs.append("%s: 값 %d 는 i8 밖" % (r["name"], val))
    return (not msgs), msgs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--growth", type=float, default=1.036,
                    help="per-level growth; 1.036 == guardian::HP_GROWTH")
    ap.add_argument("--spec", action="store_true", help="통합 스펙표만 낸다")
    ap.add_argument("--rework", action="store_true",
                    help="142차 개편안(85/90제) 표만 낸다 — 에셋·planned_rows 무관")
    a = ap.parse_args()
    fl = load_floor(a.zip)
    g = a.growth
    if a.spec:
        spec_table(fl, g)
        raise SystemExit(0)
    if a.rework:
        raise SystemExit(0 if rework_table(fl, g) else 1)
    rows = 0

    print("### 5.1 방어구 세트 6종 — 투구·갑옷·장갑·신발·방패 x 6 = 30행\n")
    print("| 세트 | 시대 | Lv | 슬롯 | 이름 | 주스탯 | 현 최강(바닥) | 배수 | 옵션1 | 옵션2 | 원가 |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for lv, sname, era, pieces, o1, o2, flat, _why in SETS:
        v1, v2 = set_opt(lv, o1, flat), set_opt(lv, o2, flat)
        for cat in SET_PARTS:
            m, _s, p = scale(fl, cat, lv, g)
            rows += 1
            print("| %s | %s | %d | %s | `%s%s` | **%d** | %s %d | x%.2f | %s+%d | %s+%d | %s |"
                  % (sname, era, lv, SLOT[cat], Q, pieces[cat], m, fl[cat][0], fl[cat][3],
                     m / fl[cat][3], o1[1], v1, o2[1], v2, format(p, ",")))

    print("\n### 5.2 세트 효과 — 다섯 조각이 같은 옵션을 갖는다\n")
    print("hero3 에 세트 메커니즘이 없어서(§7.3) 조각마다 **같은 옵션 2개**를 넣었다.")
    print("그래서 풀세트 = 한 조각의 **5배**다.\n")
    print("| 세트 | Lv | 시대 | 조각 하나 | **풀세트(5부위)** | 정체성 |")
    print("|---|---|---|---|---|---|")
    ident = {73: "수비", 79: "기습", 85: "지속", 91: "암살"}
    for lv, sname, era, _pieces, o1, o2, flat, why in SETS:
        v1, v2 = set_opt(lv, o1, flat), set_opt(lv, o2, flat)
        tag = ident.get(lv) or ("자연력·생존" if sname == "붉은마녀" else "돌파·화력")
        print("| **%s** | %d | %s | %s+%d / %s+%d | **%s+%d / %s+%d** | %s — %s |"
              % (sname, lv, era, o1[1], v1, o2[1], v2,
                 o1[1], v1 * 5, o2[1], v2 * 5, tag, why))

    print("\n### 5.3 무기 사다리 — 7 카테고리 x 3단계(hero1 74 / hero2 82 / hero3 90) = 21행\n")
    print("| 카테고리 | 진영 | Lv | 이름 | 공격력 | 보조 | 현 최강(바닥) | 배수 | 옵션1 | 옵션2 | 원가 | 이름 근거 |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for cat in (5, 4, 6, 7, 8, 9, 10):
        tname, side, names, o1, o2 = WEAPONS[cat]
        for i, lv in enumerate(WLV):
            m, s, p = scale(fl, cat, lv, g)
            v1 = W_PCT[lv] if o1[0] in PCT_IDS else W_FLAT[lv]
            v2 = W_PCT[lv] if o2[0] in PCT_IDS else W_FLAT[lv]
            rows += 1
            print("| %s | %s | %d | `%s%s` | **%d** | %d | %s %d/%d | x%.2f | %s+%d | %s+%d | %s | %s |"
                  % (tname, side, lv, Q, names[i][0], m, s, fl[cat][0], fl[cat][3],
                     fl[cat][4], m / fl[cat][3], o1[1], v1, o2[1], v2,
                     format(p, ","), names[i][1]))

    def legend_table(head, lst, note):
        print(head)
        print(note)
        print("| # | 슬롯 | 이름 | **컨셉** | 주스탯 | 보조 | 옵션1 | 옵션2 | 세트 대비 | 원가 | 이름 근거 |")
        print("|---|---|---|---|---|---|---|---|---|---|---|")
        n = 0
        for cat, nm, concept, o1, o2, why in lst:
            m, sb, pr = scale(fl, cat, LEG_LV, g)
            pct = " / ".join("%d%%" % round(100 * ov / (125 if oid in PCT_IDS else 250))
                             for oid, _on, ov in (o1, o2))
            n += 1
            print("| %d | %s | `%s%s` | **%s** | **%d** | %s | %s+%d | %s+%d | %s | %s | %s |"
                  % (n, SLOT[cat], Q, nm, concept, m, sb if 4 <= cat <= 10 else "—",
                     o1[1], o1[2], o2[1], o2[2], pct, format(pr, ","), why))
        return n

    NLX = chr(10)
    note = ("옵션 값은 **Lv97 풀세트의 30 %% 를 기준으로 잡았다 — 기본 %d %% / %d, "
            "치중축 %d %% / %d." % (LEG_PCT, LEG_FLAT, LEG_PCT_MAX, LEG_FLAT_MAX))
    note = note.replace("%%", "%") + NLX
    note += ("Lv97 세트가 풀로 주는 것이 **125 % / 250** 이므로 "
             "전설 **하나**가 그 30~36 % 를 혼자 낸다." + NLX)
    rows += legend_table(
        NLX + "### 5.4 전설 · 방어구 10종 — 투구 2 · 갑옷 3 · 장갑 3 · 신발 2, 전부 Lv99" + NLX,
        LEGENDS_ARMOUR, note)
    rows += legend_table(
        NLX + "### 5.5 전설 · 무기 7 + 방패 1 (+a) — 위 10종 밖이다" + NLX,
        LEGENDS_WEAPON,
        "★ 사용자 결정: 전설 「10종」은 **방어구만** 센다. 무기 일곱과 방패 하나는 "
        "같은 Lv99 에 따로 붙는 +a 다." + NLX)

    print("\n### 5.6 악세·혼 (cat 0x0c, 18바이트 레코드) — 9행\n")
    print("| 이름 | Lv | 성능 | 옵션1 | 옵션2 | 원가 | 이름 근거 |")
    print("|---|---|---|---|---|---|---|")
    for n, lv, m, o1, o2, why in ACCESSORIES:
        p = 30000 + (lv - L0) * 2000
        if p % 256 == 0:
            p += 1000
        rows += 1
        print("| `%s%s` | %d | %d | %s+%d | %s+%d | %s | %s |"
              % (Q, n, lv, m, o1[0], o1[1], o2[0], o2[1], format(p, ","), why))

    # ---------------- invariants ----------------
    print("\n### 불변식\n```")
    ok = True
    leg_cats = {c for c, *_ in LEGENDS}
    for cat in sorted(fl):
        lvs = set()
        if cat in WEAPONS:
            lvs |= set(WLV)
        if cat in SET_PARTS:
            lvs |= {s[0] for s in SETS}
        if cat in leg_cats:
            lvs.add(LEG_LV)
        lvs = sorted(lvs)
        prev, prevlv, gains = fl[cat][3], fl[cat][2], []
        for lv in lvs:
            m, _s, _p = scale(fl, cat, lv, g)
            if m <= prev:
                ok = False
                print("FAIL cat %d lv %d: %d <= %d" % (cat, lv, m, prev))
            gains.append((m, (m - prev) / (lv - prevlv)))
            prev, prevlv = m, lv
        rising = all(gains[i][1] < gains[i + 1][1] for i in range(len(gains) - 1))
        if not rising:
            ok = False
        print("cat %-2d %-5s floor=%-4d -> %-30s 레벨당 %s"
              % (cat, SLOT[cat], fl[cat][3], str([x[0] for x in gains]),
                 " < ".join("%.1f" % x[1] for x in gains) if rising else "*비단조*"))

    print()
    base = fl[7][3]                       # 리버건 = 1.00
    worst = 0.0
    for cat in (9, 8, 5, 6, 4, 7, 10):
        want = fl[cat][3] / base
        m99, s99, _p = scale(fl, cat, LEG_LV, g)
        got = m99 / scale(fl, 7, LEG_LV, g)[0]
        worst = max(worst, abs(got - want))
        print("%-5s 리버 %-4d /리버건 = %.4f   Lv99 %-5d /Lv99건 = %.4f   보조/주 %.4f -> %.4f"
              % (SLOT[cat], fl[cat][3], want, m99, got,
                 fl[cat][4] / fl[cat][3], s99 / m99))
    if worst > 0.005:
        ok = False
        print("FAIL 리버 비율이 보존되지 않았다 (최대 오차 %.4f)" % worst)
    else:
        print("리버 카테고리 간 비율 보존: 최대 오차 %.4f (반올림뿐)" % worst)

    print()
    for cat, nm, _c, _o1, _o2, _w in LEGENDS:      # 전설 > 그 슬롯 최고 세트 조각
        if cat not in SET_PARTS:
            continue
        top = max(scale(fl, cat, s[0], g)[0] for s in SETS)
        m, _s, _p = scale(fl, cat, LEG_LV, g)
        if m <= top:
            ok = False
        print("전설 %-9s %-4s Lv99 %-4d vs 최고 세트 조각(Lv97) %-4d %s"
              % (nm, SLOT[cat], m, top, "OK" if m > top else "FAIL"))
    print("신규 행 %d개 / ALL ABOVE FLOOR + RISING GAIN: %s" % (rows, ok))
    print("```")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
