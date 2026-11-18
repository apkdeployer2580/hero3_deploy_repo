"""Append the two boss-only `{` accessories to i12_dat (cat 0x0c, 혼/악세).

WHY. The player asked (2026-09-03) for two accessories that drop from ONE boss each
(the options below are that round's — 150차 replaced them, see the ★★ block):
  타이탄의 파편  — 크레이지타이탄(141) only — 명중 +35, 쿨타임 -10 %
                  (2026-09-03 second decision: 물리공격 +30 dropped — the attack getter
                  0x4cdb4 branches on the equipped weapon kind, so 물리공격 (0x05) only
                  reaches ATT1 for 리츠 weapons; 케이 uses 특수공격 (0x06))
  검은 눈물      — 다크니스이라(138) only — 쿨타임 -30 %, SP 회복 +30
The drop side is the `reaver_drop_lgt` guest hook (`wie_core_arm::binary_patches::
reaverdrop::BOSS_ACCESSORIES`); this module is only the item rows.

CATEGORY. cat 0x0c is the game's own accessory slot (반지 · 이어링 · 아뮬렛 · 혼 — 40 stock
rows). Its stat block is the 18-byte variant of the equipment layout (MIGRATION_LGT.md
§2.22: same layout two bytes earlier — tier at b5, main u16 at b10, and the two
(statId, value) option pairs at b14..17). Both rows are cloned from the last white 혼
(`@백발백중의혼`, idx 0x25) with the tier, name and options replaced, so price/icon are
whatever the game already ships for that slot (150차: 성능 and 요구레벨 are set too).

OPTIONS are `addStat` ids (`0x4ee34`, PLAY_FEEDBACK_FIXES.md §40차 table):
  0x05 물리공격 / 0x06 특수공격 (percent, weapon-kind gated)   0x09 명중
  0x0f SP 회복                                                0x11 쿨타임 감소 (%, host)
물리공격/특수공격 (0x05/0x06) are percent bonuses on the weapon's 성능
(`ATT = 성능 * (100 + base + bonus) / 100`, getter 0x4cdb4), chosen by weapon kind.

★★ 150차 (2026-10-01, user decision — `LEGACY_ITEMS_REWORK_85_90.md §11`/§12): the two rows
are two of the eleven reworked accessories, and `acc_options.py` is the one source of truth
for all three of their options — the ROWS below take their two pairs FROM it (타이탄의 파편 =
회피 +15 · 쿨타임 -20 %, 검은 눈물 = 쿨타임 -30 % · SP소모 -30; the thirds 공격속도 +50 /
주는 피해 +25 % live outside the asset, `acc_options.rs`). 성능 (b10..11 — the character's
저항, which the template 백발백중의혼 had at 15) is written to `acc_options.PERF` = 0 and the
요구레벨 (b6) to `acc_options.REQ_LV` = 1 (the template's own value). ⇒ verify allows exactly
four bytes to differ from the template: the tier, b6, b10..11 and the options.

Same contract as the reaver rows: the FILE is not enough — the per-category count byte at
VA 0x5e480 (0x28 → 0x2a, `binary_patches.toml`) is what the parser believes. Ship both.

Owned by `patch_asset.py` (fed the PRISTINE file; idempotent when the rows are already last).
"""
import struct
import sys

sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import acc_options as AO  # noqa: E402  (150차: the 11 accessories' decisions)
import dump_item_db as DB  # noqa: E402

CAT = 12
MARK = 0x7B  # `{` red
TIER = 4
PRISTINE_N = 40
TEMPLATE_IDX = 0x25  # `@백발백중의혼`
STATS_LEN = 18
B_TIER = 5
B_REQ = 6       # 요구레벨 (150차: acc_options.REQ_LV)
B_PERF = 10     # u16 성능 = the character's 저항 (150차: acc_options.PERF = 0)
OPT_OFF = 14


def _raw_opts(name):
    """(statId, value, statId, value) as the four raw bytes, from the decision table."""
    (a, x), (b, y) = AO.options_of(name)
    return (a & 0xFF, x & 0xFF, b & 0xFF, y & 0xFF)


# (name, (statId, value, statId, value)) in append order → idx 0x28, 0x29. ★ 150차: the
# pairs come from `acc_options.py` (the 75차-C values were 명중 35 · 쿨타임 10 / 쿨타임 30 · SP회복 30).
ROWS = [(n, _raw_opts(n)) for n in ("타이탄의 파편", "검은 눈물")]


def euc(s: str) -> bytes:
    return s.encode("euc-kr")


def raw_name(i: int) -> bytes:
    return bytes([MARK]) + euc(ROWS[i][0])


def _split(body: bytes):
    nl = body[0]
    return body[1:1 + nl], body[1 + nl:]


def _stats_from_template(template_stats: bytes, opts) -> bytes:
    out = bytearray(template_stats)
    if len(out) != STATS_LEN:
        raise SystemExit(f"i12 stats are {len(out)} B, want {STATS_LEN}")
    out[B_TIER] = TIER
    out[B_REQ] = AO.REQ_LV
    struct.pack_into("<H", out, B_PERF, AO.PERF)
    out[OPT_OFF], out[OPT_OFF + 1], out[OPT_OFF + 2], out[OPT_OFF + 3] = opts
    return bytes(out)


def make_row(i: int, template_stats: bytes) -> bytes:
    name = raw_name(i)
    body = bytes([len(name)]) + name + _stats_from_template(template_stats, ROWS[i][1])
    return len(body).to_bytes(2, "little") + body


def already_ours(recs) -> bool:
    if len(recs) < len(ROWS):
        return False
    tail = recs[-len(ROWS):]
    return all(_split(body)[0] == raw_name(i) for i, (_idx, _off, body) in enumerate(tail))


def build(src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"i12_dat does not parse byte-exact ({consumed}/{len(src)}) — "
            "feed the pristine file (patch_asset.py rebuilds from .orig)")
    if already_ours(recs):
        return src
    if len(recs) != PRISTINE_N:
        raise SystemExit(f"i12_dat: expected {PRISTINE_N} rows, got {len(recs)} — refusing")
    _name, stats = _split(recs[TEMPLATE_IDX][2])
    out = src
    for i in range(len(ROWS)):
        out += make_row(i, stats)
    return out


def verify(before: bytes, after: bytes) -> None:
    b, a = list(DB.records(before)), list(DB.records(after))
    if already_ours(b):
        if before != after:
            raise SystemExit("i12_dat: accessory rows already last but bytes changed")
        return
    if len(a) != len(b) + len(ROWS):
        raise SystemExit(f"i12_dat: wanted +{len(ROWS)} rows, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if b[i][2] != a[i][2]:
            raise SystemExit(f"i12_dat: row {i:#x} changed — only the append is allowed")
    _tname, tstats = _split(b[TEMPLATE_IDX][2])
    for i, (name_txt, opts) in enumerate(ROWS):
        name, stats = _split(a[len(b) + i][2])
        if name != raw_name(i):
            raise SystemExit(f"i12_dat: row {len(b) + i:#x} name {name!r} != {raw_name(i)!r}")
        if len(stats) != STATS_LEN or stats[B_TIER] != TIER:
            raise SystemExit(f"i12_dat: row {len(b) + i:#x} stats/tier {len(stats)}/{stats[B_TIER]}")
        got = tuple(stats[OPT_OFF:OPT_OFF + 4])
        if got != opts or opts != _raw_opts(name_txt):
            raise SystemExit(f"i12_dat: row {len(b) + i:#x} options {got} != {opts} (acc_options {_raw_opts(name_txt)})")
        # ★ 150차: 요구레벨 and 성능 are the decisions, not the template's (its 성능 was 15).
        req, perf = stats[B_REQ], struct.unpack_from("<H", stats, B_PERF)[0]
        if (req, perf) != (AO.REQ_LV, AO.PERF):
            raise SystemExit(f"i12_dat: row {len(b) + i:#x} 요구Lv/성능 {req}/{perf} != {AO.REQ_LV}/{AO.PERF}")
        # Everything but tier, 요구레벨, 성능 and options is the template's.
        for k in range(STATS_LEN):
            if k in (B_TIER, B_REQ, B_PERF, B_PERF + 1) or OPT_OFF <= k < OPT_OFF + 4:
                continue
            if stats[k] != tstats[k]:
                raise SystemExit(f"i12_dat: row {len(b) + i:#x} byte {k} differs from the template")
        print(f"VERIFY OK: i12_dat +1 `{name_txt}` idx=0x{len(b) + i:02x} "
              f"요구Lv {req} 성능 {perf} tier {TIER} "
              f"opt {got[0]:02x}+{got[1]}/{got[2]:02x}+{got[3]}")


class Handler:
    __name__ = "add_boss_accessories"

    def build(self, src: bytes) -> bytes:
        return build(src)

    def verify(self, before: bytes, after: bytes) -> None:
        verify(before, after)


HANDLERS = {"i12_dat": Handler()}
