"""Generate the tower's floor table - 하늘성 / 가디언의 전당.

    python tools_re/gen_guardian_floors.py            # print the table
    python tools_re/gen_guardian_floors.py --out wie_guardian_floors.txt
    python tools_re/gen_guardian_floors.py --check    # verify a written table

WHY A GENERATOR AND NOT A HAND-WRITTEN FILE

  100 rows is too many to keep consistent by hand, and the tier rules the request
  states ("1~9층 HP 상승, 11~19층 +방어력, 21~29층 패턴 몬스터, …") are rules, not
  data - so they belong in code where changing the balance is one edit and a re-run.
  `GUARDIAN_CASTLE_FEASIBILITY.md` §3.2 asked for exactly this.

WHAT PICKS THE SCENES

  A floor is a SCENE, not a map (§A.0) - 244 scenes share 135 maps, and the guest's
  move primitive takes a scene id.  The palette comes from the shipped assets:

    * the tower runs on the HARD monster table, because that is where the levels are:
      of the 180 scenes that spawn anything, 89 reach level >= 60 on `enemyh_dat`
      against 8 on `enemy_dat` (§A.5).  The app does not switch tables - the player's
      character is already in 2회차, which is the tower's entry gate (§C.2).
    * every 10th floor is a BOSS floor, and there are 16 boss scenes for the 10 the
      tower needs.
    * the rest are ordinary scenes sorted by level, so a floor is never easier than
      the one below it.

  ! A scene id must fit ONE BYTE (the warp destination is one byte), and the shipped
  244 all do.  The generator refuses anything else rather than truncating.

THE COLUMNS (112차, 2026-09-19 — the LEVEL carries the curve now)

  `lv` is what the host forks into each live monster record, and since 112차 it is the
  whole difficulty curve: floor 1 = 60 -> floor 100 = 120, linear (`level_for`, mirrored
  by `guardian::Floor::level_for`).  The host grows every record from its SPAWN level to
  that level by the game's OWN per-level ratios measured on its own tables (HP x1.036,
  attack x1.043, defence x1.0285 per level — `guardian::{HP,ATK,DEF}_GROWTH`), so
  `hp% def% atk%` sit at 100 in the generated table (they still multiply on top if a
  hand-tuned table wants them).  The level is one signed byte (120 < 127) and the game
  reads it only through CLAMPED rolls (kill EXP ±9, the player's miss roll 10..60 %, the
  monster's hit rolls capped 90/95 %, a ±lv/2 damage jitter), so a Lv 120 monster is
  "at the caps" for a Lv 70..99 player and nothing worse.
  `count` is the inverse of the level (Lv 60 -> 20 monsters, Lv 120 -> 1), and `exp` is
  the EXP the floor pays on clear — `lv x 300`, boss floors `lv x 1000` — which the host
  hands to the game's own `checkLevelUp` (the 천상의 영약 mechanism).  Why EXP at all:
  the player levels to 99 since 111차 and the kill formula pays `1 + exp` five levels
  above a monster, so without Lv 71+ monsters and a clear bonus 99 was unreachable.
"""
import argparse
import random
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0].rsplit("\\", 1)[0])

import gen_tower_rooms as TOWER_ROOMS  # noqa: E402  (the generated floors)
from dump_map_catalog import (  # noqa: E402
    BOSS_BASE, DEFAULT_ZIP, SPECIES_NONE, creature_table, load, open_assets, scene_level,
    species_of,
)

FLOORS = 100
LEVEL_MAX = 120                        # == guardian::LEVEL_MAX (112차: 60 -> 120)
BOSS_EVERY = 10

# ★★★ THE GENERATED ROOMS WIN (user decision 2026-09-12, 87차): three square halls that
# differ only in where their pillars stand, rotating across the ordinary floors, plus one
# wide-open square hall for every boss floor. `tools_re/gen_tower_rooms.py` builds them
# and owns the geometry; this module decides which scene a floor gets AND, since 88차,
# which monsters (the `s0 s1 s2` columns the host writes over the scene header).
#
# It supersedes 86차's single 40x40 arena (`gen_tower_arena.py`, `ARENA_SCENE = 243`),
# which shipped as vc298 and made 90 floors identical.
#
# ⛔ Fail SOFT, not closed: an archive without the rooms (a pristine dump, or one rebuilt
# by `patch_asset.py` before the chain reaches the rooms tool) still gets a playable table
# from the dungeon palette. A missing room must not stop the table from generating.
#
# ★★★ 88차 (2026-09-12) — THE MONSTERS ARE PER FLOOR, and three player rules:
#
#   * "몬스터는 1~100층 전부 계속 바뀌어야해": every floor rolls its own species triple
#     (`species_for`), seeded, distinct within the floor and never repeating the floor
#     below. The count tier (how many of the three slots are live) climbs with depth.
#   * "박스같은건 넣으면 안돼 몬스터만 넣어야해" (second pass, after playing the first):
#     the nineteen harmless critters — boxes, rats, bees, ghosts, dust — are EXCLUDED
#     everywhere. `gen_tower_rooms.attackless()` / `attackers()` are the pools (one
#     definition); only `attackers()` (114 species at hard Lv >= 40) is drawn from.
#     100 floors need 180 species slots, so a species may appear TWICE in the tower,
#     never on consecutive floors and never more often than any less-used one in its
#     level window (`species_for` picks by use count first).
#   * "보스 몬스터 HP … 공격력 … 하드코어 종결보스급으로 … 둘다 대폭상승": the boss floors
#     carry their own multipliers (`boss_tiers`) plus the new `atk%` column, and the
#     host keeps the HP that does not fit an i16 in a virtual pool it refills the record
#     from (`guardian::Pool`). The boss list walks `bossh_dat` easiest to hardest, ending
#     on 홀리가디언 at floor 100. ⚠ The 87차 fork scaled bosses from record fields that
#     are ZERO on a boss record, which is why they were 1-HP pushovers (see
#     `guardian.rs`'s header) — that is fixed host-side, these numbers are on top.
TIER_BANDS = (1, 34, 67)               # ordinary floor >= band -> live slots 1, 2, 3
SPECIES_SEED = 0x544f5745              # "TOWE" - the per-floor roll is deterministic
# Fighters are sorted by hard level and each ordinary floor draws from a window that
# slides up the list with depth, so the deep floors meet the deep species.
SPECIES_WINDOW = 40

# The boss ladder, easiest first, by `bossh_dat` level / HP. `bossh_dat` species ids are
# 161 + row. 리츠/케이 (161..163 / 165..167) are the rival duels and stay out; the
# player's request was the game's real bosses. 홀리가디언 (175) is the final boss.
# Nine real bosses for ten floors: 큐 (168) returns on floor 90 as the last gate before
# 홀리가디언 (its 66/27600 row is the strongest after the guardian's).
BOSS_LADDER = (169, 170, 174, 172, 164, 168, 171, 173, 168, 175)
# ★★★ 137차 결정 ③ + 138차 결정 ⑭ (2026-09-25 / 2026-09-29): floors 10..70 are the boss
# ALONE; 80 / 90 / 100 are RAIDS — the throne hall's 4 + 4 add points filled with two
# FIXED species each (spawn types 1 and 2). ⛔ Not a rng pick any more:
#   * 80 오르도 (173) — a 「비스트」 guarding 시엔 (scene 218 「멈춰, 오르도.」, scene 215),
#     and a beast is a failed guardian (scene 189's record). 커브스쿠툼 134 is called a
#     beast by scene 212's dialogue; 아이언비스트 140 by its name.
#   * 90 큐 (168) — the Soltian army's captain (scenes 1 · 2 · 235); 솔티안버서커 34 and
#     솔티안워락 12 are the Soltian ranks of WAR_OF_RUIN, where 큐 fights.
#   * 100 홀리가디언 (175) — kept byte-identical to the shipped table («100층은 지금 그대로»):
#     커브아르보르 143 + 식인꽃 129. ⛔ `species_for` rolls from floor 1 with one rng, so any
#     change below would otherwise move this line-up.
# All six are `attackers()` (HP < 10,000, can attack); the picker RESERVES them on the
# neighbouring floors so no floor repeats the one below.
RAID_ESCORTS = {80: (134, 140), 90: (34, 12), 100: (143, 129)}

# ★★★ 137차 결정 ② + 138차 결정 ⑯: from floor 71 the 「갑자기 어려워진 분위기」 — ONE 네임드
# (the eleven `{` red-name species of `enemyh_dat`) in slot 2 of every ODD floor 71..99,
# and the HP ramp 100 -> 150 % on every ordinary floor 71..99. The even floors are the
# hordes in between: no named, and the count cap stays at MAX_MONSTERS.
NAMED_FROM = 71
NAMED_CAP = 5          # spawn order 0,1,2,0,1 -> at most five keeps exactly one slot-2 monster
HP_RAMP_TOP = 150      # hp% on floor 99 (결정 ②)
COUNT_MIN = 3          # a 1-2 monster floor has no meaning in an open hall (86차 §5)

# ★★★ 137차 결정 ①: the count is no longer the inverse of the level. It is the SMALLEST
# number of monsters, taken in spawn order (slot 0,1,2,0,1,2,… — `gen_tower_rooms.
# spawn_points` types the hall points `i % 3`, and `guardian::park_indices` keeps the
# array's FIRST `count`), whose grown HP reaches the floor's target total T(f):
#   T = 60k -> 220k over floors 1..70 (geometric), 220k -> 430k over 70..99.
# That is what removes the flat, species-noisy totals of the 112차 table (floor 12 26k,
# floor 92 19k — `TOWER_REWARD_BALANCE_DRAFT.md §0.1`). The endpoints are the knobs the
# play calibration moves (§1.6); the SHAPE is the decision.
T_FLOOR1, T_ANCHOR, T_TOP = 60_000, 220_000, 430_000
HP_GROWTH = 1.036      # == guardian::HP_GROWTH (the host grows each record by this)
# A SWARM species — spawn HP under this on the hard table: the bats, booby traps, bugs,
# bees and mice (~25 of the fighters, 561..1,951 HP against 2,000..7,300 for the rest).
# Three of them on one floor cannot reach the target even at twenty (floor 13 of the
# first 137차 table weighed 47k against 75k), and on a named floor they make the one 네임드
# the whole floor (95: 243k named + four ~10k critters, 27 % under the target) — which is
# what leaves the count rule nothing to steer. So a floor takes at most ONE swarm species,
# and a named floor none (`species_for`). They are still fought — on the other floors.
SWARM_HP = 2000


def level_for(floor):
    """Mirror of `guardian::Floor::level_for` - kept identical on purpose.

    137차 결정 ④ + the 70층 anchor: 60 -> 100 over floors 1..70 (floor 70 = a hell Lv 100
    monster, the user's definition), 100 -> 120 over 70..100 (홀리가디언 stays Lv 120).
    Round half up, in integers, so the Rust mirror cannot drift by a float.
    """
    f = max(1, min(FLOORS, floor))
    if f <= 70:
        return 60 + (2 * 40 * (f - 1) + 69) // 138
    return min(100 + (2 * 20 * (f - 70) + 30) // 60, LEVEL_MAX)


def hp_pct_for(floor, is_boss=False):
    """The row's hp%: 100 through floor 70, then 100 -> HP_RAMP_TOP over 71..99 (결정 ②).
    Boss rows stay 100 (the level growth is the boss's; 100층 is kept as it was)."""
    if is_boss or floor < NAMED_FROM:
        return 100
    return 100 + (2 * (HP_RAMP_TOP - 100) * (floor - 70) + 29) // 58


def named_floor(floor):
    """Does this floor field one 네임드 in slot 2? Odd ordinary floors 71..99 (결정 ⑯)."""
    return floor >= NAMED_FROM and floor % 2 == 1 and floor % BOSS_EVERY != 0


def target_hp(floor):
    """The floor's target total HP (결정 ① — see T_FLOOR1 above)."""
    if floor <= 70:
        return T_FLOOR1 * (T_ANCHOR / T_FLOOR1) ** ((floor - 1) / 69)
    return T_ANCHOR * (T_TOP / T_ANCHOR) ** ((floor - 70) / 29)


def grown_hp(rows, species, lv, pct):
    """`guardian::grown_pct(hp0, HP_GROWTH, lv - lv0, pct)` for one spawn of `species`:
    the hard table's spawn HP grown to `lv`, then the row's percentage (integer, as the host)."""
    _name, lv0, hp, _atk, _tail = rows[species]
    g = max(1, round(hp * HP_GROWTH ** (min(lv, LEVEL_MAX) - lv0)))
    return g * max(1, pct) // 100


def count_for(floor, trio, rows):
    """How many of the hall's 20 spawns this floor keeps (결정 ①).

    The smallest k whose first k spawns — slots `trio[i % 3]`, the array order — reach
    `target_hp(floor)`, clamped to COUNT_MIN..cap; cap = NAMED_CAP on a named floor (so the
    one slot-2 spawn kept is the 네임드), MAX_MONSTERS otherwise.
    """
    lv, pct = level_for(floor), hp_pct_for(floor)
    cap = NAMED_CAP if named_floor(floor) else MAX_MONSTERS
    goal = target_hp(floor)
    total = 0
    for k in range(1, cap + 1):
        total += grown_hp(rows, trio[(k - 1) % 3], lv, pct)
        if total >= goal:
            return max(COUNT_MIN, k)
    return cap


def floor_total(floor, trio, count, rows):
    """The HP the kept monsters really carry — `--check` compares it against the target."""
    lv, pct = level_for(floor), hp_pct_for(floor)
    return sum(grown_hp(rows, trio[i % 3], lv, pct) for i in range(count))


def exp_for(lv, is_boss):
    """The clear EXP a floor pays: `lv x 300`, a boss floor `lv x 1000` (112차)."""
    return lv * (1000 if is_boss else 300)


# ★ The 2026-09-16 count rule (a count that is the inverse of a per-floor strength roll)
# and its 112차 successor (the inverse of the LEVEL) are both superseded by 137차 결정 ① —
# `count_for` above weighs the floor's actual species. What survives of them is the fail-
# soft path (`legacy_count`), for an archive without the generated halls.
MAX_MONSTERS = 20                       # == guardian::MAX_MONSTERS (the halls' 20 spawns)


def ordinary_roll(floor):
    """(hp%, def%, atk%) for an ORDINARY floor.

    ★ 137차: attack and defence stay 100 — the level growth carries them, and a monster's
    attack is already one-shot territory in the 90s (`TOWER_REWARD_BALANCE_DRAFT.md §0.1`:
    the lever is how MANY hit at once and how LONG they stand, never how hard). hp% is the
    71..99 ramp (`hp_pct_for`). ⛔ Do not raise atk%; def% waits on the damage formula.
    """
    return hp_pct_for(floor), 100, 100


def legacy_count(floor):
    """The 112차 count (the inverse of the level: Lv 60 -> 20 ... Lv 120 -> 1) — only for the
    fail-soft table built without the generated halls, whose scenes bring their own species
    and so cannot be weighed by `count_for`."""
    lv = level_for(floor)
    count = MAX_MONSTERS - ((lv - 60) * (MAX_MONSTERS - 1) * 2 + 60) // 120
    return max(1, min(MAX_MONSTERS, count))


def boss_tiers(floor):
    """(hp%, def%, atk%) for a BOSS floor — 100 since 112차.

    The boss is grown to the floor's level like everything else (홀리가디언's 28.6k HP at
    Lv 120 is ~189k through the host's virtual HP word; 큐's 27.6k at Lv 65 is its own),
    and a `bossh_dat` record already carries four times an ordinary record's HP at the
    same level, so no extra multiplier is stacked. The columns remain for hand tuning.
    """
    return 100, 100, 100


# ── 출구 없는 씬만 쓴다 (2026-09-11 사용자 결정) ──────────────────────────────
#
# *"지금처럼 나가는 문이 있는 던전이 아니라 나가는 문이 없는 던전으로 들어가져야해"*
#
# 층은 **닫힌 방**이어야 한다. 그런데 씬은 원본 게임의 던전이라 대개 문이 있고, 그 문을
# 밟으면 게임 자신이 워프해서 런이 끝난다(`LeftTheField`).
#
# ★★ **출구 = `0x28` 로 시작하는 B 레코드를 가리키는 A 행**이다. `0x28 <목적지 씬> <x> <y> <dir>`
#    이고, 이미지 전체 499개 전부 둘째 바이트가 **실존 씬 id** 다(그것이 이 판독의 근거다).
#    씬 111 의 스프라이트 없는 A 행 여섯이 마을 건물 문 다섯 + 맵 출구 하나로 맞아떨어진다.
# ⛔ **kind 로 좁히지 말 것** — 걸어서 닿는 것만 세려고 kind 3/5/6 으로 좁혀 봤지만 일반 씬이
#    15개 대 14개로 하나밖에 안 늘고, kind 0/4 의 워프도 스크립트가 상황에 따라 쏠 수 있다.
#    「문이 없다」는 요청에는 **어느 kind든 워프가 0개**가 정직한 해석이다.
#
# ★ 결과가 우연히 아주 좋다: 남는 30개가 정확히 게임의 **폐쇄 아레나**이고, 일반 14개 안에
#   **GUARDIAN_CAVE_2 / 4 / 7**(이름부터 가디언의 동굴)과 GULBEIG_RUIN_5/7 · WAR_OF_RUIN_1
#   같은 제대로 된 던전이 들어 있으며, **보스 씬 16개는 전부 원래부터 출구가 없다.**
# ⚠ 대가는 다양성이다 — 일반 팔레트가 125 → **14**. 90개 일반 층이 그 14개를 여섯 번쯤
#   돌려 쓴다. 그것이 싫으면 대안은 「출구 행을 IN_TOWER 플래그로 감싸기」인데, 원본 게임의
#   던전 **82개 · 출구 행 190개**를 고치는 일이라 이 회차는 하지 않았다.
# ★★ 옵코드 오퍼랜드 길이 표 (`GUARDIAN_CASTLE_FEASIBILITY.md §J.1`, 디스패처 `0x3cf48`
#    / 점프표 `0x5d8a4`). 검산 = 244씬의 B 레코드 **2143/2143 이 정확히 소진된다** —
#    `--check` 가 그것을 매번 다시 확인한다.
OPLEN = [3, 3, 3, 2, 0, 1, 2, 2, 5, 4, 0, 0, 3, 5, 2, 1,
         1, 1, 3, 4, 3, 4, 1, 4, 1, 1, 4, 1, 1, 3, 1, 4,
         4, 1, 2, 2, 0, 1, 2, 1, 4, 1, 1, 4, 2, 2, 4, 3,
         2, 1, 1, 2, 1, 1, 3, 5, 4, 3]
OP_CHOICE_B = 0xFF
OP_WARP = 0x28
# ★★ 조건 옵코드의 **총 길이**(옵코드 1 + 오퍼랜드). `GUARDIAN_CASTLE_FEASIBILITY.md §J.3`:
#    fa/fb/fd = 오퍼랜드 2 · fe(아이템) = 3 · fc(소지금) = 4. A 행의 조건 컨테이너를 걷는
#    워커 셋(`exit_rows` 여기, `patch_tower_gate._block_payload` / `selectable`)이 전부 이 표
#    하나를 읽는다 — 오퍼랜드 길이가 바뀌면 여기만 바꾼다 (코드리뷰 2026-09-11).
COND_LEN = {0xFA: 3, 0xFB: 3, 0xFD: 3, 0xFE: 4, 0xFC: 5}
COND_FLAG = 0xFA
CONTAINER = 0xFF
# ★★★ `fa ff 00` = 「이 로드에서 진입 스크립트가 아직 안 돌았다」. 코퍼스의 kind-0 행 244개 중
#    232개가 그 조건으로 시작하고, 모든 씬의 B[0] 가 `23 ff 01 33 00 08` = 표시를 세우고 8프레임
#    페이드인이다(writer 241개, 값은 전부 1 · reader 232개, 값은 전부 0). 로더가 로드마다 되돌린다 —
#    프로브 실측: 마을(씬 111)의 B[0] 를 돈 캐릭터가 씬 9/121 로 워프해도 그 씬의 페이드인이 돈다.
#    ⇒ `23 ff` 는 퀘스트 플래그 쓰기가 **아니고**, 이것을 막으면 그 씬은 탑 안에서 **검은 화면**이다
#    (2026-09-11 코드리뷰 실측: vc293 층 표의 12개 층이 그랬다 — 씬 13·51 비검정 118,349 → 2,569).
FLAG_ENTRY_MARK = 0xFF
OP_SETFLAG = 0x23
# `33 <dir> <frames>` = 페이드. dir 0 = 검정에서 밝아짐(진입), 1 = 검정으로(컷신이 `33 01 10 … 33 00 10`
# 로 쓴다). 탑에서 층이 보이려면 이것이 **확실히** 한 번은 돌아야 한다 (`patch_tower_gate.tower_ready`).
OP_FADE = 0x33

# ★★★ A 행 payload 의 실행기 = `0x52148(ctx, stream, nested)` — 디스어셈 2026-09-11 (코드리뷰).
#    옛 모델(「블록 = 조건들 + payload 하나」)은 틀렸다. 실제는 **순서열 실행기**다:
#      바이트 0..count-1  → 그 B 레코드를 실행하고 **계속**            (0x521c0, ran = 1)
#      ff n len[n] 블록×n → 블록을 차례로 재귀 평가, **무언가 실행한 첫 블록**이 이기고
#                            컨테이너 끝으로 seek 한 뒤 **계속**       (0x52228..0x5224e)
#      fa..fe 조건        → 거짓이면 **이 스트림의 나머지를 버리고 return ran** (0x5227c)
#    그래서 `fa ff 00 <중첩> 00` 의 꼬리 `00`(= B[0] 페이드인)이 진입 때마다 돌고, 블록 안의 payload
#    여러 개도 전부 돈다. 옛 워커는 중첩 컨테이너 뒤에서 return 해 그 꼬리를 못 봤다 — 그것이
#    씬 9/121 은 우연히 렌더되고 `fa ff 00 00` 단순형(씬 13 등)은 검게 만든 차이다.
CERTAIN, MAYBE, NEVER = 2, 1, 0


def container_at(b, i):
    """`b[i] == ff`: `(blocks, end)`, 길이 바이트가 스트림을 넘으면 `None`."""
    if i + 2 > len(b):
        return None
    n = b[i + 1]
    if i + 2 + n > len(b):
        return None
    lens = list(b[i + 2:i + 2 + n])
    o = i + 2 + n
    blocks = []
    for length in lens:
        if o + length > len(b):
            return None
        blocks.append(bytes(b[o:o + length]))
        o += length
    return blocks, o


def evaluate(stream, decide):
    """`0x52148` 을 기호적으로 돈다. `decide(op, args)` 는 조건의 답: True / False / None(게임 조건 —
    어느 쪽도 가능). 돌려주는 것은 `(reach, ran)` — `reach[idx]` 는 그 레코드가 도는 확실성
    (CERTAIN = 어떤 플래그 상태에서도 · MAYBE = 어떤 상태에서는 · 없음 = 절대), `ran` 은 이
    스트림이 「무언가 실행했다」를 돌려줄 확실성. 못 읽는 스트림은 `None` (fail closed)."""
    reach = {}
    state = CERTAIN
    ran = NEVER
    i = 0
    while i < len(stream):
        b = stream[i]
        if b == CONTAINER:
            got = container_at(stream, i)
            if got is None:
                return None
            blocks, i = got
            entered = state
            box_ran = NEVER
            for blk in blocks:
                if entered == NEVER:
                    break
                sub = evaluate(blk, decide)
                if sub is None:
                    return None
                r, st = sub
                for k, c in r.items():
                    c = min(entered, c)
                    if c > reach.get(k, NEVER):
                        reach[k] = c
                box_ran = max(box_ran, min(entered, st))
                if st == CERTAIN:
                    entered = NEVER              # 들어오기만 하면 이 블록이 이긴다
                elif st == MAYBE:
                    entered = MAYBE              # 이 블록이 실패했을 때만 다음 블록
            ran = max(ran, box_ran)
            continue
        if b in COND_LEN:
            n = COND_LEN[b]
            if i + n > len(stream):
                return None
            d = decide(b, bytes(stream[i + 1:i + n]))
            if d is False:
                return reach, ran                # 0x5227c: 나머지는 안 돈다
            if d is None and state == CERTAIN:
                state = MAYBE
            i += n
            continue
        if b >= 0xF0:
            return None                          # i8 로 평가기에 넘어가는 쓰레기 — 못 읽는 행
        if state > reach.get(b, NEVER):
            reach[b] = state
        ran = max(ran, state)
        i += 1
    return reach, ran


def fade_in_records(B):
    """검정에서 밝아지는 페이드(`33 00 xx`)를 가진 B 레코드의 인덱스 집합."""
    out = set()
    for i, rec in enumerate(B):
        ops, args = [], []
        if walk_script(rec, ops, args) != len(rec):
            continue
        if any(op == OP_FADE and a[:1] == b"\x00" for op, a in zip(ops, args)):
            out.add(i)
    return out


def walk_script(rec, out_ops=None, out_args=None):
    """Walk one B record exactly. Returns the number of bytes consumed.

    `out_ops` / `out_args` collect every opcode and its operand bytes, in order (a
    choice `ff` contributes `b""` as its operands), branches included.

    ⛔ **Do NOT just look at `rec[0]`.** Scene 207's exit is a `0x28` sitting INSIDE a
    choice branch of a record that starts with `0x34` (say) — reading only the first
    opcode called that scene sealed and put a door back in the tower (2026-09-11).
    ⛔ And do not scan for the byte `0x28` either: operand bytes take every value, so a
    scan invents doors. The length table is what makes an exact walk possible.
    """
    i = 0
    while i < len(rec):
        op = rec[i]
        if out_ops is not None:
            out_ops.append(op)
        if op == OP_CHOICE_B:
            if out_args is not None:
                out_args.append(b"")
            # `ff <p> <n> (<label> <delta>) x n` then n branch bodies of those deltas.
            if i + 3 > len(rec):
                return -1
            n = rec[i + 2]
            i += 3
            if i + 2 * n > len(rec):
                return -1
            deltas = [rec[i + 2 * k + 1] for k in range(n)]
            i += 2 * n
            for d in deltas:
                if i + d > len(rec):
                    return -1
                if walk_script(rec[i:i + d], out_ops, out_args) != d:
                    return -1
                i += d
            continue
        if op >= len(OPLEN):
            return -1
        if out_args is not None:
            out_args.append(bytes(rec[i + 1:i + 1 + OPLEN[op]]))
        i += 1 + OPLEN[op]
    return i


def exit_rows(jar):
    """scene id -> how many A rows can reach a script that contains a warp (`0x28`)."""
    from patch_guardian_npc import parse_scn as _parse

    def payloads(row):
        """every record an A row can run under SOME flag state, and whether the walk was exact.

        ⛔ The VM (`evaluate`) is the model, not a hand walker: an A row is a sequence — a
        block can run several records, a container can be followed by more, and a record
        that comes AFTER a nested container in a block (scene 9's fade-in B[0]) runs too.
        Scene 212's boss arena is `ff 01 05 <block>` followed by `ff 02 07 07 <block>
        <block>`: the second container picks the post-boss scripts, which warp out.
        Reading only the first container said every boss arena was sealed and the floor
        table filled with rooms that run a story cutscene the moment you clear them
        (reported 2026-09-11 as "I killed the monster and a quest suddenly advanced").

        ⛔ Returns `ok = False` when the row does not walk exactly. Fail closed: a row
        we cannot read is a row that might be a door.
        """
        got = evaluate(bytes(row[6:]), lambda op, args: None)
        if got is None:
            return [], False
        return sorted(got[0]), True
    out = {}
    for name in jar.namelist():
        if not (name.startswith("res/event/e") and name.endswith("_scn")):
            continue
        try:
            _, (a_rows, b_recs, _c) = _parse(jar.read(name))
        except Exception:
            continue
        # A record whose exact walk fails is a record we do not understand: count it
        # as a door (fail closed) rather than declaring the scene sealed.
        warpy = []
        for rec in b_recs:
            ops = []
            warpy.append(OP_WARP in ops if walk_script(rec, ops) == len(rec) else True)
        n = 0
        for row in a_rows:
            if len(row) < 7:
                continue
            ps, exact = payloads(row)
            if not exact:
                n += 1                              # unreadable row: fail closed
                continue
            for p in ps:
                if p < len(b_recs) and warpy[p]:
                    n += 1
        out[int(name[-8:-4])] = n
    return out


# ── 걸어서 닿지 않는 몬스터가 있으면 안 된다 (2026-09-11 사용자 결정) ──────────
#
# *"어떤 맵에서 걸어서 갈수 없는곳에 몬스터가 있다거나 그러면 그쪽으로 갈수없으니
#   모든 몬스터를 잡을 수없잖아"*
#
# ★★ **충돌은 `attr[y*w + x] & 0x80`** 이다 — 게임 자신의 검사(`0x17a6f8`)이고
#    `minimap_render.rs::ATTR_BLOCKED` 가 이미 그 비트를 쓴다.
# ★★ **스폰 레코드는 `(type, x, y)`** 다. 3바이트 중 **첫 바이트가 type**(값이 정확히
#    0/1/2 뿐)이고, 그 배치로 읽으면 1,297개 스폰이 **범위 밖 0건 · 막힌 타일 0.4 %**,
#    뒤집어 읽으면 범위 밖 235건 · 막힌 타일 44.7 % 다. 그것이 판독 근거다.
# ⛔ `(x, y, type)` 으로 읽지 말 것 — 그렇게 재면 20x20 보스방의 스폰이 전부 「막힌
#    타일」로 나와서 팔레트가 0개가 된다(이 회차가 한 번 그렇게 읽었다).
#
# 판정: 살아 있는 스폰(씬의 species 가 NONE 이 아닌 type)이 **전부 걸을 수 있는 타일**에
# 있고 **모두 같은 연결 구역**이며 그 구역이 **가장 큰 구역**일 것. 이동은 4방향이다
# (`MIGRATION_LGT.md §2.7` — 축이 넷이다).
# ⛔ 「걸을 수 있는 구역이 하나뿐」을 요구하지 말 것 — 몬스터가 없는 1~4타일짜리 죽은
#    구석이 흔하고(예: SMALL_CAVE_3 는 [557,4,1,1,1]), 그것은 클리어를 막지 않는다.
def unreachable_monsters(jar, maps, scenes):
    """scene id -> True when a live spawn cannot be walked to from the main region."""
    BLOCKED = 0x80

    def regions(mp):
        w, h, attr = mp["w"], mp["h"], mp["attr"]
        seen = [-1] * (w * h)
        sizes = []
        for start in range(w * h):
            if seen[start] != -1 or (attr[start] & BLOCKED):
                continue
            k = len(sizes)
            stack = [start]
            seen[start] = k
            n = 0
            while stack:
                i = stack.pop()
                n += 1
                x, y = i % w, i // w
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = x + dx, y + dy
                    if 0 <= nx < w and 0 <= ny < h:
                        j = ny * w + nx
                        if seen[j] == -1 and not (attr[j] & BLOCKED):
                            seen[j] = k
                            stack.append(j)
            sizes.append(n)
        return seen, sizes

    out = {}
    for sid, sc in scenes.items():
        mp = maps.get(sc["map"])
        if mp is None:
            out[sid] = True                   # cannot prove it: fail closed
            continue
        w = mp["w"]
        seen, sizes = regions(mp)
        if not sizes:
            out[sid] = True
            continue
        biggest = max(range(len(sizes)), key=lambda k: sizes[k])
        live = [(x, y) for (t, x, y) in mp["spawns"]
                if t < len(sc["species"]) and sc["species"][t] != SPECIES_NONE]
        bad = False
        for (x, y) in live:
            if not (0 <= x < w and 0 <= y < mp["h"]):
                bad = True
                break
            if seen[y * w + x] != biggest:    # blocked tile (-1) or a side pocket
                bad = True
                break
        out[sid] = bad or not live
    return out


def gated_scenes(jar):
    """scene ids whose A rows carry the tower gate (`tools_re/patch_tower_gate.py`).

    ⛔ This is the floor table's SEAL. A scene without it can run its own scripts while
    the player is in the tower, and those scripts are the original game's — dialogue,
    quest flags and a warp out of the room.
    """
    from patch_tower_gate import tower_ready

    out = set()
    for name in jar.namelist():
        if not (name.startswith("res/event/e") and name.endswith("_scn")):
            continue
        try:
            if tower_ready(jar.read(name)):
                out.add(int(name[-8:-4]))
        except Exception:
            continue
    return out


def palette(jar, hard=True):
    """(ordinary, boss) scene ids usable as floors, each sorted by monster level."""
    maps, scenes = load(jar)
    crea = creature_table(jar, hard)
    # ⛔⛔ THE DOOR CHECK IS NOT WHAT SEALS A FLOOR ANY MORE (2026-09-11).
    #
    # It used to be: pick scenes with no exit. Two things killed that. `exit_rows`
    # read only the FIRST top-level container of an A row, and boss arenas have two —
    # so the 24 "sealed" scenes it found were not sealed, and clearing one ran the
    # game's own post-boss cutscene ("몬스터 잡았더니 갑자기 퀘스트가 진행됐어"). With
    # the walker fixed, the number of scenes in the whole game that have monsters, no
    # exit and no interaction is **zero**.
    #
    # So the seal is now something we INSTALL: `tools_re/patch_tower_gate.py` puts
    # `fa <TOWER> 00` in front of every block of every A row of every candidate scene,
    # and the host raises that flag for the length of a run. Inside the tower no A row
    # can run a FORBIDDEN record — no door, no dialogue, no reward, no `setFlag` — while
    # the entry script (the once-per-load fade-in, `FLAG_ENTRY_MARK`) still runs; and
    # outside it the selectable set is byte-identical to the shipped one (the tool
    # proves that per scene). `doors` is still computed, but only to be REPORTED.
    doors = exit_rows(jar)
    gated = gated_scenes(jar)
    stranded = unreachable_monsters(jar, maps, scenes)
    ordinary, boss = [], []
    for sid, sc in sorted(scenes.items()):
        if sid > 0xFF:
            continue                      # the warp destination is one byte
        species = species_of(sc, maps)
        if not species:
            continue                      # nothing to fight
        # ⛔ A scene we cannot prove is sealed is NOT usable — fail closed. Being
        #    "sealed" now means the tower gate is installed in it.
        if sid not in gated:
            continue                      # see the note in this function's head
        # ⛔ Fail closed again: an unmeasurable map counts as stranded.
        if stranded.get(sid, True):
            continue                      # a monster you cannot walk to; see above
        lv = scene_level(sc, maps, crea)
        if lv is None:
            continue
        (boss if any(s >= BOSS_BASE for s in species) else ordinary).append((lv, sid))
    ordinary.sort()
    boss.sort()
    return ordinary, boss


def build(jar, hard=True):
    ordinary, boss = palette(jar, hard)
    # Hardest last. ★ No level floor any more: the sealed palette is small (see
    # `palette`), and the host FORKS every monster's level to the floor's own `lv`
    # column anyway — so a scene's shipped level decides which monsters you meet, not
    # how hard they are. Dropping scenes for it would only cost variety.
    #
    # ★★ **BOSS SCENES ARE ORDINARY FLOORS TOO** (user decision 2026-09-11). Only seven
    # scenes in the whole game survive the four floor conditions as ordinary
    # (`§K.7.4`), and every one is a 20x20 arena with one or two live monsters, so a
    # tower built from those alone is seven rooms repeated thirteen times. The boss
    # arenas pass the same conditions — they are sealed by the original game's own rule
    # — so folding them in takes the ordinary palette from 7 to 19.
    # ⛔ The boss FLOORS still draw from `boss` only: every tenth floor must be a boss.
    ordinary = sorted(ordinary + boss)
    ordinary = [s for lv, s in ordinary]
    boss = [s for lv, s in boss]
    halls, boss_hall = None, None
    if TOWER_ROOMS.present(jar):
        halls = TOWER_ROOMS.hall_scenes()      # [scene, ...] one per ordinary hall
        boss_hall = TOWER_ROOMS.boss_scene()
    if not ordinary or not boss:
        raise SystemExit("no usable scenes - is this the right archive?")
    species = species_for(jar) if halls else {}
    weights = TOWER_ROOMS.species_rows(jar) if halls else None
    rows = []
    # ⛔ Count ORDINARY floors, do not index by `floor - 1`: every 10th floor is a boss
    # and `BOSS_EVERY` is 10, so `(floor - 1) % 10` never takes the value 9 on an
    # ordinary floor — a 10-scene palette silently loses its last entry for ever
    # (GUARDIAN_CAVE_7 was that entry on 2026-09-11).
    k = 0
    for floor in range(1, FLOORS + 1):
        is_boss = floor % BOSS_EVERY == 0
        if is_boss:
            scene = boss_hall if boss_hall else boss[(floor // BOSS_EVERY - 1) % len(boss)]
        elif halls:
            # ★ The three halls ROTATE, which is what "총 3개를 돌려가면서" asks for, and
            # the rotation counts ordinary floors so every hall gets an equal share.
            scene = halls[k % len(halls)]
            k += 1
        else:
            # Walk the ordinary palette in order and wrap; with 100 floors and a
            # palette in the dozens every scene is reused a few times, which is the
            # "recombine what exists" principle the request set out with.
            scene = ordinary[k % len(ordinary)]
            k += 1
        if is_boss:
            hp, dfn, atk = boss_tiers(floor)
            count = None                # boss floors carry no count (never parked)
        else:
            hp, dfn, atk = ordinary_roll(floor)
            # ★ 137차 결정 ①: the count WEIGHS this floor's own three species (spawn order,
            # grown to the floor's level and hp%) against the target total. The fail-soft
            # dungeon palette brings the scene's own species, which cannot be weighed here.
            count = count_for(floor, species[floor], weights) if species else legacy_count(floor)
        lv = level_for(floor)
        rows.append((floor, scene, lv, hp, dfn, is_boss, atk, species.get(floor), count, exp_for(lv, is_boss)))
    return rows


def named_species(jar):
    """The eleven `{` red-name species of the hard table (the game's own 네임드 — the same
    set as `helldrop_table::HELL_NAMED_SPECIES`), by (level, id)."""
    rows = TOWER_ROOMS.species_rows(jar)
    return [s for _lv, s in sorted((lv, s) for s, (name, lv, _hp, _atk, _t) in rows.items()
                                   if name.startswith("{"))]


def named_sequence(named, floors):
    """`{floor: species}` for the named floors (결정 ⑯ — odd 71..99, fifteen of them).

    The eleven once in (level, id) order — the ladder climbs from 포레스트로커 to 다크니스이라
    — then the strongest again from the top, so the last named floor (99) is the strongest
    and no two NEIGHBOURING named floors share a species (91 다크니스이라 -> 93 와일드스쿠툼).
    """
    if not named:
        raise SystemExit("no `{` species in the hard table - is this the right archive?")
    seq = list(named)
    extra = len(floors) - len(seq)
    if extra > 0:
        if extra >= len(named):
            raise SystemExit("%d named floors for %d named species" % (len(floors), len(named)))
        # ⛔ one extra would repeat the last species on the next named floor.
        seq += named[-extra:] if extra > 1 else [named[-2]]
    return dict(zip(floors, seq))


def species_for(jar):
    """`{floor: (s0, s1, s2)}` — every floor's own monsters. `SPECIES_NONE` = empty slot.

    * ordinary floors: THREE fighters, always (137차 결정 ① weighs the count over all
      three slots in spawn order, so an ordinary floor uses every slot), from the
      fighters sorted by hard level: a `SPECIES_WINDOW`-wide window slides from the bottom
      of the list at floor 1 to the top at floor 99; a floor never repeats a species of
      the floor below NOR a raid escort of the boss floor above it, and always prefers
      the species used LEAST so far (ties broken by the seeded rng). A named floor (odd
      71..99, 결정 ⑯) draws two and puts its 네임드 in slot 2 (`named_sequence`).
    * boss floors: `BOSS_LADDER[k]` alone in slot 0 on floors 10..70 (결정 ③); the raids
      80 / 90 / 100 add their fixed `RAID_ESCORTS` in slots 1 and 2 (결정 ⑭).
    """
    harmless, fighters, table = TOWER_ROOMS.species_pools(jar)
    fighters = sorted(fighters, key=lambda s: (table[s][1], s))
    weights = TOWER_ROOMS.species_rows(jar)
    for floor, escorts in RAID_ESCORTS.items():
        bad = [s for s in escorts if s not in fighters]
        if bad:
            raise SystemExit("floor %d: raid escort(s) %s are not fighters" % (floor, bad))
    named_at = named_sequence(named_species(jar),
                              [f for f in range(1, FLOORS + 1) if named_floor(f)])
    rng = random.Random(SPECIES_SEED)
    out, prev = {}, set()
    uses = {s: 0 for s in fighters}
    for escorts in RAID_ESCORTS.values():       # the raids are uses too
        for s in escorts:
            uses[s] += 1

    swarm = {s for s in fighters if weights[s][2] < SWARM_HP}

    def pick(window, n, avoid, max_swarm):
        # Least-used first, the level window second, the rng only orders the ties — so a
        # species is never picked again while a peer anywhere in the list is used less
        # (255 ordinary slots over 114 fighters: two or three uses each). Boxes and the
        # rest of the harmless set are not in `fighters`, so they cannot be picked at all.
        # Then at most `max_swarm` swarm species (see SWARM_HP) — skipped ones stay first
        # in line for the next floor, because their use count did not move.
        inside = set(window)
        pool = [s for s in fighters if s not in prev and s not in avoid]
        pool.sort(key=lambda s: (uses[s], s not in inside, rng.random()))
        got, swarmed = [], 0
        for s in pool:
            if len(got) == n:
                break
            if s in swarm:
                if swarmed >= max_swarm:
                    continue
                swarmed += 1
            got.append(s)
        if len(got) < n:
            raise SystemExit("floor %d: only %d species to pick from" % (floor, len(got)))
        for s in got:
            uses[s] += 1
        return got

    for floor in range(1, FLOORS + 1):
        is_boss = floor % BOSS_EVERY == 0
        if is_boss:
            k = floor // BOSS_EVERY - 1
            trio = [BOSS_LADDER[k]] + list(RAID_ESCORTS.get(floor, ()))
        else:
            span = max(0, len(fighters) - SPECIES_WINDOW)
            lo = span * (floor - 1) // max(1, FLOORS - 1)
            window = fighters[lo:lo + SPECIES_WINDOW]
            above = set(RAID_ESCORTS.get(floor + 1, ()))   # the raid above must differ
            if named_floor(floor):
                trio = pick(window, 2, above, 0) + [named_at[floor]]
            else:
                trio = pick(window, 3, above, 1)
        prev = set(trio)
        trio = trio + [SPECIES_NONE] * (3 - len(trio))
        out[floor] = tuple(trio)
    return out


def render(rows):
    out = ["# 하늘성 층 테이블 - GENERATED by tools_re/gen_guardian_floors.py",
           "# ⛔ 손으로 고치지 말 것: 밸런스는 그 스크립트의 level_for() / ordinary_roll() / exp_for() 에 있다.",
           "# 137차: lv 60 -> 100(70층) -> 120(100층) · 71~99 hp% 100 -> 150 · 71~99 홀수 층 slot 2 = 네임드(`{`),",
           "#        count = 층 목표 총 HP 를 스폰 순서(slot 0,1,2,…)로 채우는 최소 마릿수(3~20, 네임드 층 <= 5),",
           "#        보스 10~70 단독 · 80/90/100 레이드(에스코트 고정 둘), exp 는 클리어 경험치 (일반 lv x 300 · 보스 lv x 1000).",
           "# floor scene lv hp% def% boss atk% s0 s1 s2 count exp"]
    for floor, scene, lv, hp, dfn, is_boss, atk, sp, count, exp in rows:
        line = f"{floor} {scene} {lv} {hp} {dfn} {'B' if is_boss else '-'} {atk}"
        if sp is not None:
            line += " " + " ".join("-" if x == SPECIES_NONE else str(x) for x in sp)
        # The count and the EXP follow the species; a boss row (no count) writes `-` so
        # the EXP column stays in place — the Rust parser reads `-` as no count.
        line += f" {'-' if count is None else count} {exp}"
        out.append(line)
    return "\n".join(out) + "\n"


def parse(text):
    """The Rust parser's rules, so `--check` verifies what the app would load."""
    rows = []
    for line in text.splitlines():
        line = line.split("#")[0].strip()
        if not line:
            continue
        f = line.split()
        if len(f) not in (7, 10, 11, 12):
            return None
        try:
            sp = None
            if len(f) >= 10:
                sp = tuple(SPECIES_NONE if x == "-" else int(x) for x in f[7:10])
            count = None if len(f) < 11 or f[10] == "-" else int(f[10])
            exp = int(f[11]) if len(f) == 12 else 0
            rows.append((int(f[0]), int(f[1]), None if f[2] == "-" else int(f[2]),
                         int(f[3]), int(f[4]), f[5].upper() == "B", int(f[6]), sp, count, exp))
        except ValueError:
            return None
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--out")
    ap.add_argument("--check", action="store_true",
                    help="regenerate and compare against --out (exit 1 on drift)")
    ap.add_argument("--normal", action="store_true",
                    help="use the NORMAL monster table for the level column (not the tower's)")
    ap.add_argument("--report", action="store_true",
                    help="print each floor's weighed total against its target (the calibration view)")
    a = ap.parse_args(argv)

    jar = open_assets(a.zip)
    rows = build(jar, hard=not a.normal)
    text = render(rows)

    # Every invariant the app will re-check, checked here too, so a broken table
    # cannot be written in the first place.
    assert len(rows) == FLOORS
    assert all(1 <= f <= FLOORS for f, *_ in rows)
    assert all(0 < s <= 0xFF for _, s, *_ in rows), "a scene id must fit one byte"
    assert all(lv is None or 1 <= lv <= LEVEL_MAX for _, _, lv, *_ in rows)
    assert all(100 <= hp <= 2000 and 100 <= d <= 2000 and 100 <= atk <= 2000
               for _, _, _, hp, d, _, atk, _, _, _ in rows), \
        "percentages must be >= 100 and within guardian::PCT_MAX"
    bosses = [r[0] for r in rows if r[5]]
    assert bosses == list(range(BOSS_EVERY, FLOORS + 1, BOSS_EVERY)), bosses
    lv = [r[2] for r in rows]
    assert lv == sorted(lv), "the level curve must never go down"
    assert lv[0] == 60 and lv[69] == 100 and lv[-1] == LEVEL_MAX == 120, \
        "137차: the curve is 60 -> 100 (floor 70 = a hell Lv 100 monster) -> 120"
    assert all(r[2] == level_for(r[0]) for r in rows), "every row carries level_for()"

    # ── 137차: the hp% ramp, the weighed count, EXP on clear ──
    for floor, _scene, _lv, hp, dfn, is_boss, atk, _sp, count, _exp in rows:
        assert (dfn, atk) == (100, 100), f"floor {floor}: def% / atk% stay 100 (§1.3)"
        assert hp == hp_pct_for(floor, is_boss), f"floor {floor}: hp% is the 71..99 ramp"
        if is_boss:
            assert count is None, f"boss floor {floor} carries a count"
        else:
            cap = NAMED_CAP if named_floor(floor) else MAX_MONSTERS
            assert count is not None and COUNT_MIN <= count <= cap, f"floor {floor}: count {count}"
    assert hp_pct_for(70) == 100 and hp_pct_for(71) > 100 and hp_pct_for(99) == HP_RAMP_TOP
    # Every ten-floor group is strictly stronger (higher level) than the one below it,
    # which is what "11~20층은 반드시 1~9층보다 높아야" asks for — by level now.
    groups = {}
    for r in rows:
        groups.setdefault((r[0] - 1) // 10, []).append(r[2])
    gks = sorted(groups)
    # A 60-step ramp over 99 floors climbs ~0.6 a floor, so a group boundary may share a
    # level; the rule is therefore "never lower, and every group STARTS above the last".
    for lo, hi in zip(gks, gks[1:]):
        assert max(groups[lo]) <= min(groups[hi]), f"group {hi} level below group {lo}"
        assert min(groups[hi]) > min(groups[lo]), f"group {hi} does not start above group {lo}"
    assert all(r[9] == exp_for(r[2], r[5]) for r in rows), "the EXP column is exp_for()"
    assert all(0 < r[9] <= 10_000_000 for r in rows), "within guardian::EXP_MAX"

    # 88차 + 137차: the species rules, re-checked on the rows about to be written.
    if any(r[7] is not None for r in rows):
        harmless, fighters, _ = TOWER_ROOMS.species_pools(jar)
        weights = TOWER_ROOMS.species_rows(jar)
        named = set(named_species(jar))
        assert len(named) == 11, "the hard table has %d red-name species, not 11" % len(named)
        prev, uses, last_named = set(), {}, None
        for floor, scene, _lv, _hp, _d, is_boss, _atk, sp, count, _exp in rows:
            live = [x for x in sp if x != SPECIES_NONE]
            assert live and len(set(live)) == len(live), f"floor {floor}: {sp}"
            assert not (prev & set(live)), f"floor {floor} repeats the floor below"
            assert not (set(live) & set(harmless)), f"floor {floor} spawns a harmless critter"
            prev = set(live)
            if is_boss:
                assert sp[0] == BOSS_LADDER[floor // BOSS_EVERY - 1], f"floor {floor}: the ladder's boss"
                assert tuple(live[1:]) == RAID_ESCORTS.get(floor, ()), \
                    f"floor {floor}: 10..70 alone, 80/90/100 the fixed raid escorts"
                assert all(x in fighters for x in live[1:]), f"floor {floor}"
            else:
                assert len(live) == 3, f"floor {floor}: an ordinary floor uses all three slots"
                n_swarm = sum(1 for x in live if x in fighters and weights[x][2] < SWARM_HP)
                assert n_swarm <= (0 if named_floor(floor) else 1), f"floor {floor}: too many swarm species"
                if named_floor(floor):
                    assert sp[2] in named and all(x in fighters for x in sp[:2]), f"floor {floor}"
                    assert sp[2] != last_named, f"floor {floor}: same named species as the named floor below"
                    last_named = sp[2]
                else:
                    assert all(x in fighters for x in live), f"floor {floor} spawns a non-fighter"
                # The weighed count is the SMALLEST that reaches the target (or the cap).
                cap = NAMED_CAP if named_floor(floor) else MAX_MONSTERS
                goal = target_hp(floor)
                assert floor_total(floor, sp, count, weights) >= goal or count == cap, \
                    f"floor {floor}: the kept monsters fall short of the target"
                if count > COUNT_MIN:
                    assert floor_total(floor, sp, count - 1, weights) < goal, \
                        f"floor {floor}: count {count} is not the smallest"
            for x in live:
                if x in fighters:
                    uses[x] = uses.get(x, 0) + 1
        assert rows[-1][7] == (175, 143, 129), "floor 100 is kept as shipped (decision 3)"
        assert max(uses.values()) <= 3, "an ordinary species appears more than three times"
        assert sorted(named) == sorted({r[7][2] for r in rows if named_floor(r[0])}), \
            "every named species appears on some named floor"
        if a.report:
            print("floor  lv hp% count   target    total  (s0 s1 s2)")
            for floor, _s, lvl, hp, _d, is_boss, _atk, sp, count, _e in rows:
                if is_boss:
                    print(f"{floor:5d} {lvl:3d} {hp:3d}  boss                    {sp}")
                    continue
                print(f"{floor:5d} {lvl:3d} {hp:3d} {count:5d} {target_hp(floor):8.0f} "
                      f"{floor_total(floor, sp, count, weights):8d}  {sp}")

    if a.check:
        if not a.out:
            raise SystemExit("--check needs --out (the file to compare against)")
        try:
            on_disk = open(a.out, encoding="utf-8").read()
        except OSError as e:
            print(f"FAIL: cannot read {a.out}: {e}")
            return 1
        if parse(on_disk) != rows:
            print(f"FAIL: {a.out} is not what this script generates (stale or edited)")
            return 1
        print(f"OK: {a.out} matches - {len(rows)} floors, "
              f"{len(set(r[1] for r in rows))} distinct scenes, "
              f"levels {lv[0]}..{lv[-1]}")
        return 0

    if a.out:
        with open(a.out, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
        print(f"wrote {a.out}: {len(rows)} floors, "
              f"{len(set(r[1] for r in rows))} distinct scenes, levels {lv[0]}..{lv[-1]}")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
