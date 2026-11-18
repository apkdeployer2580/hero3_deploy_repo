"""Append the tower-only material `{닉스코어파편` to i14 (cat 0x0e, 재료).

★ 151차 (2026-10-01): renamed 「닉스코어의 파편」 -> 「닉스코어파편」 (user). Nothing keys on
the name but the tools (`add_tower_recipes`, `gen_guardian_rewards`, `gen_bagbox_names`
find the row by it); the game and every save address the row as (0x0e, 0x2e).

WHY. The player's reward design for 하늘성 (2026-09-11) is: only boss floors
(every 10th) pay, floors 1..20 pay a level-appropriate ordinary weapon/armour, and
from floor 21 up they pay THIS material, more of it the deeper the floor — with a
0.1 % jackpot of a 리버 weapon or a tier-4 red instead. The material is meant to
be spent on a forge recipe the player will add later, so it has to be a real row
in the game's own material table rather than a host-side token.

The name does not exist anywhere in the shipped data (`닉스` only ever occurs
inside `피닉스`), so this is a NEW row, and that makes it the same four-part
contract as the 16th orb (44차) and the eight 리버 weapons (75차):

    1. this file appends record idx 0x2e to `res/dat/i14_dat`
    2. count table   VA 0x5e482  (= 0x5e474 + cat)   0x2e -> 0x2f
    3. allocation    VA 0x53730  `movs r0,#0xe6`     0xe6 -> 0xeb
    4. teardown bound VA 0x526a6 `cmp r3,#0x2e`      0x2e -> 0x2f

⚠ ALL FOUR SHIP TOGETHER. 2 is what the parse loop believes (`ldrsb` @ 0x53eca /
0x53f86); 3 is this category's hardcoded backing allocation — `0xe6 << 2` = 920 =
46 x 0x14, and 47 x 0x14 = 940 = `0xeb << 2` — because only the THIRTEEN equipment
tables derive their size from the count table (0x5370e); 4 frees the new row's two
strings on teardown. Serving idx 0x2e against any subset of these is a wild read
past the end of a 920-byte allocation on the first material list draw.

Why a row is safe to add at all: nothing picks material indices at random. Every
reader of the count table (0x52588 0x525c8 0x5370a 0x53a10 0x53b92 0x53c76 0x53d3c
0x53ea6 0x53f7e 0x5403e 0x54138 — the complete xref) is the item-DB parse/alloc/
teardown family, the generic drop selector (0x38cf4) only walks EQUIPMENT by
요구레벨/등급, and consumable drops name an explicit `(cat, idx)` in the drop
record. So idx 0x2e appears only where we hand it over, or in a recipe.

★ PRICE 1800 is deliberate, not decoration. `price == 0` makes an item
undroppable AND undepositable (the sell-price cache `CHAR+0xacac` = `price >> 2`
gates both — 72차), so a zero-priced reward would be stuck in the bag forever.
1800 also sits one step above the deepest stock material ladder
(900 / 1200 / 1500 tiers, 1300 문장), which is where a tower drop belongs.

This module is owned by `patch_asset.py` — `build` is fed the PRISTINE file and
must be idempotent if our row is already last.
"""
import sys

sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import dump_item_db as DB  # noqa: E402

CAT = 0x0E
MEMBER = "i14_dat"
MARK = 0x7B          # `{` red — the colour our own additions use (리버, 보스 악세)
NAME = "닉스코어파편"
EFFECT = "하늘성의 결정. 장비 조합에 사용된다"
PRICE = 1800
TAIL_LEN = 0         # i14 records carry no trailer (dump_item_db's table)

# Pristine row count. Anything else means this is not the stock file (or a previous
# append already landed), and we refuse to guess.
PRISTINE_N = 46
NEW_IDX = PRISTINE_N  # 0x2e

# The three guest bytes that must ship with this row, mirrored here so the
# selftest and `--check` can assert the pair without parsing the toml.
GUEST_PATCHES = (
    (0x5E482, 0x2E, 0x2F),   # count table entry for cat 0x0e
    (0x53730, 0xE6, 0xEB),   # backing allocation: (0xe6<<2)=920 -> (0xeb<<2)=940
    (0x526A6, 0x2E, 0x2F),   # teardown bound `cmp r3,#0x2e`
)


def euc(s: str) -> bytes:
    return s.encode("euc-kr")


def raw_name() -> bytes:
    return bytes([MARK]) + euc(NAME)


def make_row() -> bytes:
    """One `[u16 reclen][body]` record in the consumable layout."""
    name = raw_name()
    eff = euc(EFFECT)
    body = (bytes([len(name)]) + name
            + PRICE.to_bytes(4, "little")
            + bytes([len(eff)]) + eff
            + bytes(TAIL_LEN))
    return len(body).to_bytes(2, "little") + body


def already_ours(body: bytes) -> bool:
    nl = body[0]
    return body[1:1 + nl] == raw_name()


def build(src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"{MEMBER} does not parse byte-exact ({consumed}/{len(src)}) — "
            "feed the pristine file (patch_asset.py rebuilds from .orig)")
    if already_ours(recs[-1][2]):
        return src
    if len(recs) != PRISTINE_N:
        raise SystemExit(
            f"{MEMBER}: expected {PRISTINE_N} rows, got {len(recs)} — refusing")
    return src + make_row()


def verify(before: bytes, after: bytes) -> None:
    b, a = list(DB.records(before)), list(DB.records(after))
    if already_ours(b[-1][2]):
        if before != after:
            raise SystemExit(f"{MEMBER}: our row already last but bytes changed")
        print(f"VERIFY OK: {MEMBER} already carries `{NAME}` at idx {NEW_IDX:#04x}")
        return
    if len(a) != len(b) + 1:
        raise SystemExit(f"{MEMBER}: wanted +1 row, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if b[i][2] != a[i][2]:
            raise SystemExit(f"{MEMBER}: row {i:#x} changed — only the append is allowed")
    body = a[-1][2]
    nl = body[0]
    name, rest = body[1:1 + nl], body[1 + nl:]
    if name != raw_name():
        raise SystemExit(f"{MEMBER}: last name {name!r} != {raw_name()!r}")
    price = int.from_bytes(rest[:4], "little")
    if price != PRICE:
        raise SystemExit(f"{MEMBER}: price {price} != {PRICE}")
    if price == 0:
        raise SystemExit(f"{MEMBER}: price 0 would make the reward undroppable")
    el = rest[4]
    eff = rest[5:5 + el]
    if eff != euc(EFFECT):
        raise SystemExit(f"{MEMBER}: effect {eff!r} != {euc(EFFECT)!r}")
    if len(rest) != 5 + el + TAIL_LEN:
        raise SystemExit(f"{MEMBER}: trailer is {len(rest) - 5 - el} B, want {TAIL_LEN}")
    print(f"VERIFY OK: {MEMBER} +1 `{NAME}` idx={NEW_IDX:#04x} price={PRICE} "
          f"(sells for {PRICE >> 2}) — needs guest bytes "
          + " ".join(f"{va:#x}:{old:02x}->{new:02x}" for va, old, new in GUEST_PATCHES))


HANDLERS = {MEMBER: sys.modules[__name__]}
