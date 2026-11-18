"""Append the `"` legacy rows (LEGACY_ITEMS_70_99.md §5) to i0..i12 — the hell-mode drop table.

WHY. 133차 opens 헬모드 (difficulty byte 2, after a hardcore clear), and its drops are
the Lv 73~99 legacy items the draft designed: six armour sets, the hero1→hero2→hero3
weapon ladder, the Lv99 legends and nine accessories. `gen_legacy_items.py` owns every
number (floors re-derived from the asset, growth 1.036, option ids); this module only
turns its `planned_rows` into record bytes. **77 rows, not 78** — `섬광의군화` already
shipped (131차-C, i3 idx 0x31) and a file must not hold two rows of one name.

CONTRACT (same as `add_reaver_weapons`): owned by `patch_asset.py`, fed the PRISTINE
member, idempotent when the rows are already last. ⚠ THE FILE IS NOT ENOUGH — the
signed-byte per-category count table at VA 0x5e474 is what the parse loop believes
(`ldrsb` @ 0x53eca / 0x53f86) and the 13 equipment tables derive their allocation from
it (0x5370e), so `binary_patches.toml` must carry the matching counts. SHIP TOGETHER.

★★ CHAINED, NOT PARALLEL. Several of these members already have an owner
(`add_reaver_weapons` i4..i11, `add_legacy_boots` i3, `add_boss_accessories` i12), and
`patch_asset.TARGETS` is one handler per member — so `handlers(prev)` WRAPS whatever
was registered before it: build = legacy(inner(src)), and verify replays the inner
build (builds are pure functions of the pristine bytes) before checking our append.
⛔ `patch_asset.py` must update TARGETS with this module LAST.

★ THE FLOOR IS DERIVED FROM THE POST-INNER MEMBER, so the weapon floors were the
`{리버` rows the inner handler just appended (the draft's floors — Lv70 whites would put
a Lv73 row below a Lv65 one). Our own rows are excluded from the floor hunt: every
`"` row, and `섬광의군화` by name (the Lv1/성능199 experiment row would otherwise become
the 신발 floor and inflate that whole column).
★★ 152차-B: the reaver rows moved to 80제, so the weapon floor is now their 75차 ANCHOR
(white × 1.2, recomputed by `gen_legacy_items.anchor_row`), not their bytes — the legacy
ladder is byte-identical to what shipped before (see `gen_legacy_items.REAVER_NAMES`).

★ COLOUR `"` (0x22) IS REAL: the escape alphabet at rodata 0x5b2bc is
`40 7c 24 7b 7d 27 22`, read only by the font layer (`isEscape` 0x35d8 /
`applyColourEscape` 0x37e4, RGB table 0x5b268) — the 7th, never used by shipped items.

★★ 150차 (2026-10-01, user decisions — `LEGACY_ITEMS_REWORK_85_90.md §11`/§12):
  * accessories (18 B, i12): b6 요구레벨 = the plan's `req` (`acc_options.REQ_LV` = 1),
    b10..11 성능 (= the character's 저항) = the plan's `main` (`acc_options.PERF` = 0), b14..17 =
    the decision table's two pairs. The plan's `lv` stays the TIER (85/90제) — it is not
    written anywhere any more; `gen_legacy_items.legacy_tiers` carries it downstream.
  * armour (20 B, i0..i3): b10 (the wear-class label the 상세보기 prints as 「솔티안 전용」 —
    the @팬텀 template's 6) = 0, b11 (the game's own 4-piece set id, `0x4f270`) = the plan's
    `set_id` (8..13 on the 24 set pieces, 0 on the legends). Shields and weapons keep their
    template's b10/b11, which are 0 — `verify_cat` insists they stay 0.
"""
import struct
import sys

sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import dump_item_db as DB          # noqa: E402
import gen_legacy_items as GL      # noqa: E402
import acc_options as AO           # noqa: E402  (150차 accessory decisions)

MARK = GL.NEW_MARK   # 0x22 `"`
TIER = 5             # b7 (equipment) / b5 (accessory) — the white/final tier, colour is the marker
SOCKETS = 8          # b9 — the shipped maximum (§7.5)
GROWTH = 1.036
ACC_CAT = 12
ARMOUR_CATS = GL.RW_ARMOUR_CATS   # 투구·갑옷·장갑·신발 — the rows that carry b10 = 0 / b11 = set id
B_FACTION, B_SET_ID = 10, 11      # 20-byte equipment row: wear-class label · native set id (150차)

# Rows the handler expects to FIND (pristine + the inner handler's appends). A different
# count means the chain order changed or the file is not the stock one — refuse to guess.
EXPECT_BASE = {0: 48, 1: 54, 2: 48, 3: 50, 4: 31, 5: 31, 6: 31, 7: 31,
               8: 31, 9: 31, 10: 31, 11: 27, 12: 42}
# …and the rows it appends (LEGACY_ITEMS_70_99.md §7.1, minus the shipped boots).
EXPECT_ADD = {0: 8, 1: 9, 2: 9, 3: 7, 4: 4, 5: 4, 6: 4, 7: 4, 8: 4, 9: 4, 10: 4,
              11: 7, 12: 9}


def euc(s: str) -> bytes:
    return s.encode("euc-kr")


def _split(body: bytes):
    nl = body[0]
    return body[1:1 + nl], body[1 + nl:]


def _floor_row(cat: int, recs):
    """The strongest surviving row — `gen_legacy_items.load_floor`'s pick, on one member (the
    one function both use, `floor_from_rows`). ★ 152차-B: for a weapon cat that is the 75차
    리버 ANCHOR, not the reaver row the inner handler just appended (which is 80제 now) — so the
    legacy ladder did not move when the reaver did."""
    best = GL.floor_from_rows(cat, (_split(body) for _idx, _off, body in recs))
    if best is None:
        raise SystemExit(f"i{cat}_dat: no floor row to scale from")
    return best


def _template(cat: int, recs) -> bytes:
    """The stat block every field we do not set is copied from.

    Equipment: the floor row itself (its own non-stat bytes came from the strongest
    white when the inner handler cloned it). Accessories: the strongest `@` row with
    the 혼 flag CLEAR (b7) — the legacy nine are 이어링/아뮬렛-side rows (§7.5), and a
    혼 template would put them in the wrong sub-slot."""
    best = None
    for _idx, _off, body in recs:
        raw, stats = _split(body)
        if not raw:
            continue
        if cat == ACC_CAT:
            # Any colour: i12 has no white non-혼 row at all. The 혼 flag (b7) must be
            # clear — the nine legacy rows are 이어링/아뮬렛-side (§7.5).
            if raw[0] == MARK or len(stats) != 18 or stats[7] != 0:
                continue
            key = struct.unpack_from("<H", stats, 10)[0]
        else:
            if len(stats) < 20:
                continue
            mark = GL.COLOUR.get(raw[0], "")
            name = (raw[1:] if mark else raw).decode("euc-kr", "replace")
            if raw[0] == MARK or name in GL.FLOOR_EXCLUDE_NAMES:
                continue
            key = struct.unpack_from("<H", stats, 12)[0]
        if best is None or key > best[0]:
            best = (key, stats)
    if best is None:
        raise SystemExit(f"i{cat}_dat: no template row")
    return best[1]


def _pair(p) -> tuple:
    """An option pair as the two raw bytes the row holds (the value is an i8)."""
    return p[0] & 0xFF, p[1] & 0xFF


def _equip_stats(cat: int, template: bytes, row: dict) -> bytes:
    out = bytearray(template)
    struct.pack_into("<I", out, 0, row["price"])
    out[7] = TIER
    out[8] = row["req"]
    out[9] = SOCKETS
    if cat in ARMOUR_CATS:
        # ★ 150차: no wear-class label (결정 5), and the set's OWN native id (결정 2) — the
        # template's 6/6 made every legacy armour piece one @팬텀 set.
        out[B_FACTION] = GL.RW_FACTION_NONE
        out[B_SET_ID] = row["set_id"]
    elif row["set_id"]:
        raise SystemExit(f"i{cat}_dat: {row['name']} carries set id {row['set_id']} — only armour rows have one")
    struct.pack_into("<H", out, 12, row["main"])
    struct.pack_into("<H", out, 14, row["sub"])
    out[16], out[17] = _pair(row["o1"])
    out[18], out[19] = _pair(row["o2"])
    return bytes(out)


def _acc_stats(template: bytes, row: dict) -> bytes:
    # 18-byte variant: b0..3 price · b5 tier · b6 요구레벨 · b7 혼 flag (0) ·
    # b10,11 성능 · b14..17 the two option pairs. Verified against the stock rows
    # (반지 idx 0: price 1000 @ b0..3, perf u16 @ b10) and `add_boss_accessories`.
    # ★ 150차: 요구레벨 = the plan's `req` (1) and 성능 = its `main` (0), not the tier.
    out = bytearray(template)
    struct.pack_into("<I", out, 0, row["price"])
    out[5] = TIER
    out[6] = row["req"]
    out[7] = 0
    struct.pack_into("<H", out, 10, row["main"])
    out[14], out[15] = _pair(row["o1"])
    out[16], out[17] = _pair(row["o2"])
    return bytes(out)


def make_row(cat: int, template: bytes, row: dict) -> bytes:
    name = bytes([MARK]) + euc(row["name"])
    stats = _acc_stats(template, row) if cat == ACC_CAT else _equip_stats(cat, template, row)
    body = bytes([len(name)]) + name + stats
    return len(body).to_bytes(2, "little") + body


def _ours(recs, cat: int, plan) -> bool:
    if len(recs) < len(plan):
        return False
    tail = recs[-len(plan):]
    return all(_split(body)[0] == bytes([MARK]) + euc(p["name"])
               for p, (_i, _o, body) in zip(plan, tail))


def build_cat(cat: int, src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(f"i{cat}_dat does not parse byte-exact ({consumed}/{len(src)})")
    floor = _floor_row(cat, recs)
    plan = GL.planned_rows(cat, floor, GROWTH)
    if len(plan) != EXPECT_ADD[cat]:
        raise SystemExit(f"i{cat}_dat: planner made {len(plan)} rows, expected {EXPECT_ADD[cat]}")
    if _ours(recs, cat, plan):
        return src
    if len(recs) != EXPECT_BASE[cat]:
        raise SystemExit(
            f"i{cat}_dat: expected {EXPECT_BASE[cat]} rows before the legacy append, "
            f"got {len(recs)} — refusing (chain order? not the stock file?)")
    template = _template(cat, recs)
    out = src
    for row in plan:
        out += make_row(cat, template, row)
    return out


def verify_cat(cat: int, before: bytes, after: bytes) -> None:
    b, a = list(DB.records(before)), list(DB.records(after))
    floor = _floor_row(cat, b)
    plan = GL.planned_rows(cat, floor, GROWTH)
    if _ours(b, cat, plan):
        if before != after:
            raise SystemExit(f"i{cat}_dat: legacy rows already last but bytes changed")
        return
    if len(a) != len(b) + len(plan):
        raise SystemExit(f"i{cat}_dat: wanted +{len(plan)} rows, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if b[i][2] != a[i][2]:
            raise SystemExit(f"i{cat}_dat: row {i:#x} changed — only the append is allowed")
    set_rows = 0
    for j, row in enumerate(plan):
        name, stats = _split(a[len(b) + j][2])
        if name != bytes([MARK]) + euc(row["name"]):
            raise SystemExit(f"i{cat}_dat: row {len(b) + j:#x} name {name!r} != {row['name']!r}")
        if cat == ACC_CAT:
            # ★ 150차: 요구레벨 1 · 성능 0 · the decision table's pairs (acc_options).
            if (row["req"], row["main"]) != (AO.REQ_LV, AO.PERF):
                raise SystemExit(f"i{cat}_dat: the plan gives {row['name']} 요구Lv {row['req']} 성능 {row['main']}, "
                                 f"the decision is {AO.REQ_LV} / {AO.PERF}")
            if (tuple(row["o1"]), tuple(row["o2"])) != tuple(tuple(p) for p in AO.options_of(row["name"])):
                raise SystemExit(f"i{cat}_dat: the plan's options of {row['name']} are not acc_options'")
            got = (stats[5], stats[6], stats[7], struct.unpack_from("<H", stats, 10)[0],
                   struct.unpack_from("<I", stats, 0)[0],
                   (stats[14], stats[15]), (stats[16], stats[17]))
            want = (TIER, row["req"], 0, row["main"], row["price"], _pair(row["o1"]), _pair(row["o2"]))
        else:
            got = (stats[7], stats[8], stats[9], struct.unpack_from("<H", stats, 12)[0],
                   struct.unpack_from("<H", stats, 14)[0],
                   struct.unpack_from("<I", stats, 0)[0],
                   (stats[16], stats[17]), (stats[18], stats[19]),
                   stats[B_FACTION], stats[B_SET_ID])
            # Armour: no wear-class label and the set's own id (150차). Shields / weapons: the
            # template's bytes, which are 0 and must stay 0 (결정 2 「방패·무기는 0」).
            want = (TIER, row["req"], SOCKETS, row["main"], row["sub"], row["price"],
                    _pair(row["o1"]), _pair(row["o2"]),
                    GL.RW_FACTION_NONE if cat in ARMOUR_CATS else 0, row["set_id"])
            set_rows += 1 if row["set_id"] else 0
        if got != want:
            raise SystemExit(f"i{cat}_dat: row {len(b) + j:#x} ({row['name']}) {got} != {want}")
        if row["price"] % 256 == 0:
            raise SystemExit(f"i{cat}_dat: {row['name']} price {row['price']} — low byte 0 (R8)")
    extra = (f", native set id on {set_rows}" if cat in ARMOUR_CATS else
             f", 요구Lv {AO.REQ_LV} · 성능 {AO.PERF}" if cat == ACC_CAT else "")
    print(f"VERIFY OK: i{cat}_dat +{len(plan)} legacy row(s), idx 0x{len(b):02x}..0x{len(a) - 1:02x}{extra}")


class Handler:
    """One member, wrapping whatever handler was registered before this module."""

    def __init__(self, cat: int, inner):
        self.cat = cat
        self.inner = inner
        tail = f" after {getattr(inner, '__name__', type(inner).__name__)}" if inner else ""
        self.__name__ = f"add_legacy_i{cat}{tail}"

    def build(self, src: bytes) -> bytes:
        mid = self.inner.build(src) if self.inner else src
        return build_cat(self.cat, mid)

    def verify(self, before: bytes, after: bytes) -> None:
        mid = self.inner.build(before) if self.inner else before
        if self.inner:
            self.inner.verify(before, mid)
        verify_cat(self.cat, mid, after)


def handlers(prev: dict) -> dict:
    """{member: Handler} for i0..i12, chaining any handler `prev` already has."""
    return {f"i{cat}_dat": Handler(cat, prev.get(f"i{cat}_dat"))
            for cat in range(13)}
