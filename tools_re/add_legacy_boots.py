"""Append `"섬광의군화` — the Lv99 legend that carries the 공격속도 stat — to i3_dat.

WHY. 131차/131차-B built the stat (guest hooks + a host that sums the worn
equipment's option pairs) but nothing in the shipped data carries it, so the
only way to feel it was the test knob `wie_attack_speed.txt`. The player asked
for the item itself: *"섬광의 군화 아이템에 공격속도 옵션을 추가해서 넣어줘봐
내가 그걸 장착한뒤 변화를 볼게"* (2026-09-23). `LEGACY_ITEMS_70_99.md §5.4`
already specifies the boots; this module lands that row.

CONTRACT (same as `add_reaver_weapons`): this module is owned by
`patch_asset.py`, `build` is fed the PRISTINE file, and it must be idempotent
when the row is already last. The FILE is not enough — the signed-byte
per-category count table at VA 0x5e474 is what the parse loop believes
(`ldrsb` @ 0x53eca / 0x53f86), so `binary_patches.toml` ticks cat 0x03
(49 pristine + this row + the seven 133차 rows = 57). Equipment tables derive
their allocation from that table (0x5370e), so that byte is the only guest edit.

★★★ 133차-B (2026-09-24, user: "새로 신규 아이템들 전부 드랍되어야하는데") — THE
EXPERIMENT ROW WAS NORMALISED to the §5.4 design. 131차-C had shipped it with
required level 1 and colour `@` so the tester could wear it immediately, and the
docstring said to raise it when the legacy set ships; the set shipped (133차),
and the `@` colour also kept the row OUT of the hell drop pool
(`gen_helldrop_table.py` collects the `"` rows). Now:

* **Required level 99** — the design value. ⚠ An already-granted pair follows
  the DB row (131차-C: stats live in the row), so a sub-99 character can no
  longer equip it.
* **Colour `"` (0x22)** — the legacy tier's colour, proven real in 133차 (the
  escape alphabet at rodata 0x5b2bc carries it; 77 sibling rows wear it).
  ⇒ the hell drop table picks this row up automatically (78 rows).
* **Option 2 is 공격속도, option 1 is the design's 쿨타임-45.** The design's
  option 2 was a placeholder (`SP회복+90`) waiting for exactly this stat.
* **+60 (158차 — was +127, the i8 ceiling).** ⛔⛔ THE OPTION VALUE IS A SIGNED BYTE.
  A first try at +200 stored `0xC8`, which the game's own detail screen printed as
  `힘 +-56` and which the host read as **-56** — `sum_options` then clamped the total
  to 0, so the stat was silently OFF. 127 is the most one option pair can carry;
  more needs a second pair or a second piece of gear. ★ 158차 P3 (2026-10-02) settled
  the balance at **60** (`ITEM_BALANCE_AUDIT_156.md §8` — +127 was +77 % damage for
  every class; 60 is one rung above the +50 boots).

★ THE OPTION LINE READS 「공격속도 +60」 since `add_attack_speed_label.py` appended
that label to `InGame_txt` and the id moved to where it landed (`0x76` = index
198 - 80). ⛔ The two must move together: the detail screen labels an option by
`label[80 + id]`, so a different index needs a different id. The id still has to
sit outside `addStat`'s gate (`0x4ee34`, `id - 2 <= 0x15`) — that is what leaves
the value for the host to read — and `0x76` does.

★★ 150차 (2026-10-01, user decisions 2 and 5 — `LEGACY_ITEMS_REWORK_85_90.md §11`): the row is
cloned from the strongest `@` white boots (an @팬텀/@홀리 set piece), so it carried that set's
**b10 = 6** (the wear-class label the 상세보기 prints as 「솔티안 전용」 — anyone may wear it) and
**b11 = 6/7** (the game's own 4-piece set id read by `0x4f270`: with it, these boots completed
a @팬텀 set). A legend is no set piece: both are 0 now (`FACTION`, `NATIVE_SET_ID`).
"""
import struct
import sys

sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import dump_item_db as DB  # noqa: E402

CAT = 3                    # 신발
MARK = 0x22                # `"` — the legacy tier's colour (133차-B; was `@` for the test)
NAME = "섬광의군화"
TIER = 5                   # b7: the final tier, same as the 77 legacy rows
REQ_LV = 90                # 142차: 최종(전설) = 90제 (was 99 in 133차-B, 1 for the 131차-C experiment)
PERF = 199                 # b12 성능, from §5.4
PRICE = 199_000            # b0..b3
PRISTINE_N = 49            # refuse to guess if the file is not the stock one
FACTION = 0                # b10: no wear-class label (150차 결정 5; the template's was 6)
NATIVE_SET_ID = 0          # b11: a legend belongs to no native set (150차 결정 2)

# b16..b19 = EXACTLY two (statId, i8) pairs.
#   0x11 = 쿨타임 감소 %  (the design's option 1)
#   0x76 = 공격속도       (`wie_android/src/atkspeed.rs::ATTACK_SPEED_STAT`; the guest
#                          ignores it, the host sums it — that is the whole trick. It was
#                          0x18 until 131차-D put its label at InGame_txt row 198.)
ATTACK_SPEED_STAT = 0x76  # = InGame_txt index 198 - 80; see add_attack_speed_label.py
# ★ 158차 P3 (2026-10-02, user «전부 권장대로»): 공격속도 127 → 60. +127 alone was +77 % damage
#   for every class in every content (attack 15 → 8.47 frames); the other boots are 0..+46 %.
#   60 = +42 %, one rung above the two 공속 +50 boots (크루세이더워커 · 빙해의장화). The 쿨타임
#   -45 stays — P2 (cooldown cap 60 %, `statcap`) is what bounds it now. DB row: an already
#   owned pair follows from its next load (`ITEM_BALANCE_AUDIT_156.md §0 2 · §8 P3`).
OPTS = (0x11, 45, ATTACK_SPEED_STAT, 60)


def raw_name() -> bytes:
    return bytes([MARK]) + NAME.encode("euc-kr")


def _split(body: bytes):
    nl = body[0]
    return body[1:1 + nl], body[1 + nl:]


def _white_best(rows):
    """The strongest `@` white row — the template every field we do not set comes from."""
    best = None
    for _idx, _off, body in rows:
        name, stats = _split(body)
        if not name or name[0] != 0x40:
            continue
        if len(stats) < 20:
            continue
        perf = struct.unpack_from("<H", stats, 12)[0]
        if best is None or perf > best[0]:
            best = (perf, stats)
    if best is None:
        raise SystemExit("i3_dat: no white `@` row to copy from")
    return best[1]


def _stats(template: bytes) -> bytes:
    out = bytearray(template)
    if len(out) < 20:
        raise SystemExit(f"i3_dat: equipment stats are {len(out)} B, want 20")
    struct.pack_into("<I", out, 0, PRICE)
    out[7] = TIER
    out[8] = REQ_LV
    out[10] = FACTION
    out[11] = NATIVE_SET_ID
    struct.pack_into("<H", out, 12, PERF)
    out[16], out[17], out[18], out[19] = OPTS
    return bytes(out)


def make_row(template: bytes) -> bytes:
    name = raw_name()
    body = bytes([len(name)]) + name + _stats(template)
    return len(body).to_bytes(2, "little") + body


def already_ours(body: bytes) -> bool:
    name, _ = _split(body)
    return name == raw_name()


def build(src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"i3_dat does not parse byte-exact ({consumed}/{len(src)}) — "
            "feed the pristine file (patch_asset.py rebuilds from .orig)")
    if already_ours(recs[-1][2]):
        return src
    if len(recs) != PRISTINE_N:
        raise SystemExit(f"i3_dat: expected {PRISTINE_N} rows, got {len(recs)} — refusing")
    return src + make_row(_white_best(recs))


def verify(before: bytes, after: bytes) -> None:
    b, a = list(DB.records(before)), list(DB.records(after))
    if already_ours(b[-1][2]):
        if before != after:
            raise SystemExit("i3_dat: the boots row was already last but bytes changed")
        return
    if len(a) != len(b) + 1:
        raise SystemExit(f"i3_dat: wanted +1 row, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if b[i][2] != a[i][2]:
            raise SystemExit(f"i3_dat: row {i:#x} changed — only the append is allowed")
    name, stats = _split(a[-1][2])
    if name != raw_name():
        raise SystemExit(f"i3_dat: last name {name!r} != {raw_name()!r}")
    if stats[7] != TIER or stats[8] != REQ_LV:
        raise SystemExit(f"i3_dat: last row tier/req {stats[7]}/{stats[8]}")
    got = (stats[16], stats[17], stats[18], stats[19])
    if got != OPTS:
        raise SystemExit(f"i3_dat: last row options {got} != {OPTS}")
    if struct.unpack_from("<H", stats, 12)[0] != PERF:
        raise SystemExit("i3_dat: last row 성능 is not the designed one")
    if (stats[10], stats[11]) != (FACTION, NATIVE_SET_ID):
        raise SystemExit(f"i3_dat: last row b10/b11 {stats[10]}/{stats[11]} != {FACTION}/{NATIVE_SET_ID} "
                         f"(a wear-class label or a native set id on a legend)")
    print(f"VERIFY OK: i3_dat +1 `{NAME}` idx=0x{len(b):02x} 성능={PERF} Lv{REQ_LV} "
          f"opt {got[0]:02x}+{got[1]} / {got[2]:02x}+{got[3]} (0x76 = 공격속도) b10/b11 0/0")


HANDLERS = {"i3_dat": sys.modules[__name__]}
