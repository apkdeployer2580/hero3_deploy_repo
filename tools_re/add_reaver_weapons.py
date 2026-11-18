"""Append the eight `{리버…` drop-only weapons/shield to i4..i11.

WHY. The player asked for red items that are not in the cash shop, required at
Lv 65, and 20 % above the Lv-70 white finals (75차). Those finals already live as
the last `@` row of each weapon file (방패: last white). Colour is a name-prefix
escape (`{` = red); the combat number is the 20-byte stat block. Appending a
row is the same contract as the 16th orb: the file grows, and the signed-byte
count table at VA 0x5e474 must tick with it (`binary_patches.toml`).

★★★ 152차-B (2026-10-02, user): «리버무기의 스펙을 바꾸자 80제로 바꾸고 스펙을 올려줘 … 하드코어와
헬 모두에서 파밍이 가능한 80제 무기로». The rows are now the 80제 rung of the 142차 ladder the
legacy weapons sit on (85제 = 90 % · 90제 = 98 % of the category's 전설 main stat):

  b7 tier = 4 (red, matches `{`) — unchanged
  b8 req  = 80 (was 65)
  b12/b14 main/sub = the category's 전설 row x 0.82 (`REAVER_OF_LEGEND`, `gen_legacy_items.
                     rw_scale` on the 75차 anchor — the same function the 85제/90제 use), e.g.
                     스피어 385 -> 881 (85제 967 · 전설 1074), 방패 77 -> 176 (85제 194)
  b16..19 bonuses  = weapons: 기절 +50 (was +15 — 85제's second options are flat 60) /
                     쿨타임 -25 % (unchanged — it is already 85제's cooldown value);
                     shield: 물리방어 +30 % / 특수방어 +30 % (unchanged)
  everything else copied from the white (price 159,500 / 76,200, sockets 8, icon, set) —
  ⛔ the price is deliberately NOT raised: 152차-B made the reaver a hell farming drop (every
  Lv 68..81 kill, `reaverdrop.rs`) and the sale price (`price >> 2`) would be a gold tap.

⛔ The 75차 numbers are still the FLOOR every legacy weapon is scaled from — `gen_legacy_items.
floor_from_rows` skips these rows and recomputes that floor from the white (`anchor_row`), so
changing this spec does not move the 78 legacy rows.

This module is owned by `patch_asset.py` — `build` is fed the PRISTINE file
and is idempotent if the reaver row (THIS spec) is already last; a row of an older
spec refuses. Do not edit the zip by hand; the next `--apply` would rebuild from
`.orig` and drop a hand edit.
"""
import struct
import sys

sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import dump_item_db as DB  # noqa: E402
import gen_legacy_items as GL  # noqa: E402  (152차-B: the ladder + the frozen 75차 anchor)

MARK = 0x7B  # `{` red
TIER = 4
REQ_LV = 80                # 152차-B: 80제 (was 65)
REAVER_OF_LEGEND = 0.82    # 152차-B: 80제 = 전설 주스탯의 82 % — one 8 %p step below 85제's 90 %
GROWTH = 1.036             # == add_legacy_items.GROWTH — the ladder the 전설 row is computed on

# cat -> display name after the colour escape. 무기 종류 이름 앞에 「리버」. The table lives in
# `gen_legacy_items` (its floor hunt must recognise these rows and cannot import this module).
NAMES = GL.REAVER_NAMES

# Pristine row counts. A different count means we are not looking at the stock file
# (or a previous append already landed). Used as a refuse-to-guess gate.
PRISTINE_N = {4: 30, 5: 30, 6: 30, 7: 30, 8: 30, 9: 30, 10: 30, 11: 26}


def euc(s: str) -> bytes:
    return s.encode("euc-kr")


def raw_name(cat: int) -> bytes:
    return bytes([MARK]) + euc(NAMES[cat])


def _split(body: bytes):
    nl = body[0]
    return body[1:1 + nl], body[1 + nl:]


def _white_best(rows):
    """The strongest `@` white row — same pick CASH_SHOP uses for the 종결."""
    best = None
    for idx, _off, body in rows:
        name, stats = _split(body)
        if not name or name[0] != 0x40:  # `@`
            continue
        if len(stats) < 16:
            continue
        main = struct.unpack_from("<H", stats, 12)[0]
        if best is None or main > best[0]:
            best = (main, idx, body, stats)
    if best is None:
        raise SystemExit("no white `@` row to scale from")
    return best


# Equipment options are EXACTLY two (statId, value) pairs at stats[16..19].
# addStat (0x4ee34) applies only ids 2..0x17; 힘/체력/정신/민첩 are 0x18..0x1b
# and are ignored. Ids (PLAY_FEEDBACK_FIXES.md §40차 table): 0x0c = the stat the
# status screen calls `기절` (CRI; the orb text says 크리티컬), 0x11 = 쿨타임 감소 in
# percent (summed into CHAR+0x124 by 0x4f014; the host's cooldowns read it — §73차 §4).
# 방패: 물리방어 0x07 / 특수방어 0x08 (percent, 149차).
WEAPON_OPTS = (0x0C, 50, 0x11, 25)   # 152차-B: 기절 15 -> 50
ALLSTAT_DEF = (0x07, 30, 0x08, 30)


def spec(cat: int, white_stats: bytes):
    """152차-B: the 80제 `(main, sub)` of a category — its 전설 row x `REAVER_OF_LEGEND`, from the
    75차 anchor cut from the strongest white (the legacy ladder's own floor)."""
    m, s, _price = GL.rw_scale(GL.anchor_row(cat, white_stats), REAVER_OF_LEGEND, GROWTH)
    return m, s


def _scale_stats(stats: bytes, cat: int) -> bytes:
    out = bytearray(stats)
    if len(out) < 20:
        raise SystemExit(f"equipment stats are {len(out)} B, want 20")
    out[7] = TIER
    out[8] = REQ_LV
    main, sub = spec(cat, stats)
    struct.pack_into("<H", out, 12, main)
    struct.pack_into("<H", out, 14, sub)
    a, av, b, bv = ALLSTAT_DEF if cat == 11 else WEAPON_OPTS
    out[16], out[17], out[18], out[19] = a, av, b, bv
    return bytes(out)


def make_row(cat: int, white_stats: bytes) -> bytes:
    name = raw_name(cat)
    body = bytes([len(name)]) + name + _scale_stats(white_stats, cat)
    return len(body).to_bytes(2, "little") + body


def already_ours(body: bytes, cat: int) -> bool:
    name, _ = _split(body)
    return name == raw_name(cat)


def build_cat(cat: int, src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"i{cat}_dat does not parse byte-exact ({consumed}/{len(src)}) — "
            "feed the pristine file (patch_asset.py rebuilds from .orig)")
    if already_ours(recs[-1][2], cat):
        # Idempotent only for THIS spec: a 75차 row (Lv65, white x 1.2) must not survive a
        # rebuild by being mistaken for ours.
        _main, _idx, _body, stats = _white_best(recs[:-1])
        want = make_row(cat, stats)
        if src[recs[-1][1]:] != want:
            raise SystemExit(
                f"i{cat}_dat: the last row is a reaver of an older spec — "
                "rebuild from the pristine file (patch_asset.py --apply)")
        return src
    if len(recs) != PRISTINE_N[cat]:
        raise SystemExit(
            f"i{cat}_dat: expected {PRISTINE_N[cat]} rows, got {len(recs)} — refusing")
    _main, _idx, _body, stats = _white_best(recs)
    return src + make_row(cat, stats)


def verify_cat(cat: int, before: bytes, after: bytes) -> None:
    b, a = list(DB.records(before)), list(DB.records(after))
    if already_ours(b[-1][2], cat):
        if before != after:
            raise SystemExit(f"i{cat}_dat: reaver row already last but bytes changed")
        return
    if len(a) != len(b) + 1:
        raise SystemExit(f"i{cat}_dat: wanted +1 row, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if b[i][2] != a[i][2]:
            raise SystemExit(f"i{cat}_dat: row {i:#x} changed — only the append is allowed")
    name, stats = _split(a[-1][2])
    if name != raw_name(cat):
        raise SystemExit(f"i{cat}_dat: last name {name!r} != {raw_name(cat)!r}")
    if stats[7] != TIER or stats[8] != REQ_LV:
        raise SystemExit(f"i{cat}_dat: last row tier/req {stats[7]}/{stats[8]}")
    want = ALLSTAT_DEF if cat == 11 else WEAPON_OPTS
    got = (stats[16], stats[17], stats[18], stats[19])
    if got != want:
        raise SystemExit(f"i{cat}_dat: last row bonuses {got} != {want}")
    _wm, _wi, _wb, white = _white_best(b)
    main, sub = struct.unpack_from("<H", stats, 12)[0], struct.unpack_from("<H", stats, 14)[0]
    if (main, sub) != spec(cat, white):
        raise SystemExit(f"i{cat}_dat: last row main/sub {main}/{sub} != the 80제 spec {spec(cat, white)}")
    print(f"VERIFY OK: i{cat}_dat +1 `{NAMES[cat]}` idx=0x{len(b):02x} "
          f"ATK/DEF={main}/{sub} Lv{REQ_LV} "
          f"opt {got[0]:02x}+{got[1]}/{got[2]:02x}+{got[3]}")


class Handler:
    """One category, same shape `patch_asset.TARGETS` already expects."""

    def __init__(self, cat: int):
        self.cat = cat
        self.__name__ = f"add_reaver_i{cat}"

    def build(self, src: bytes) -> bytes:
        return build_cat(self.cat, src)

    def verify(self, before: bytes, after: bytes) -> None:
        verify_cat(self.cat, before, after)


HANDLERS = {f"i{cat}_dat": Handler(cat) for cat in NAMES}
