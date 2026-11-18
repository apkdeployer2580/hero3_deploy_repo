"""Rewrite cash-item names/descriptions/icons in the game's own item DB (`i18_dat`).

Why this is legitimate and not a "guest binary patch": item names and effect text do
NOT live in the client binary — they live in `dat/i18_dat` inside the game JAR, and the
JAR we ship is `android_app/app/src/main/assets/hero3_fixed.zip`, an archive this
project already rebuilds. `i18_dat` is PLAINTEXT EUC-KR (only `i15_dat` is encrypted).

★ 2026-08-13 REWRITTEN ON THE CORRECT RECORD MODEL. The previous version (via
`decode_items.py`) read the file as "u16 FILE header + records with a 6-byte tail";
the u16 is really a PER-RECORD length prefix and the tail is 4 bytes — the mistaken
model swallowed the next record's length into the previous record's tail, so every
record after the first renamed one shipped with a stale length prefix
(`hero3_server/CASH_SHOP.md` 2026-08-13 §(4)). This version uses `dump_item_db`'s
model, which parses all 18 plaintext files byte-exact to the end:

    per record: [u16 LE reclen][u8 namelen][name][4B price][u8 efflen][effect][4B tail]
    tail = [icon u8][useArm u8][param u16]   (icon: 상자류=0x11, 문장류=0x1a, 컵라면=0x15;
                                              useArm ≥ 0x10 = the consume-only tail)

The items renamed here have no client-side effect of their own — their use falls
through to the consume-only tail — and the Android host supplies the effect
(`wie_android/src/buffs.rs`, `emulator.rs`), so their TEXT has to match:

    0x11 럭키스톤   (네트워크 재료 드랍 2배, 1일)   -> 경험의비석 / 경험치 2배
    0x12 럭키쥬엘   (네트워크 재료 드랍 2배, 1주일) -> 경험의보주 / 경험치 4배
    0x15 신의의문장 (길드스킬 발동확률 — 길드 없음)  -> 천상의 영약 / 즉시 레벨 1 상승
                                                    + 아이콘 = 컵라면(0x15) [사용자 지정]
    0x18 초월의문장 (장비레벨제한 무시, 1주일)      -> same name, "레벨제한 5 감소"

A leading `}` (0x7d) is kept on every renamed name — it is the gold TEXT COLOUR the
other cash rows carry (not a cash flag; see CASH_SHOP.md 2026-08-11 §(1)).
`;` is the game's line break inside effect text.

Usage:
    python tools_re/rename_cash_items.py --in <i18_dat> --out <i18_dat> [--verify]
(normally driven by `patch_asset.py`, which feeds it the PRISTINE file)
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_item_db as DB  # noqa: E402  (the CORRECT record model)

CASH = 0x7D  # '}' gold colour escape, kept as the first name byte

# idx -> (new name without the marker | None, new effect | None, new icon byte | None
#         [, marker byte | None to drop it, price u32])
#
# ★ The 4- and 5-element form (2026-09-14, the BAG BOXES): a row may DROP the `}` marker
# and set its price. The three delisted no-op rows become the bag-item cells that open the
# host's 오브가방 / 재료가방 / 소비가방 (`BAG_TYPES_FEASIBILITY.md §3.4`). No marker, so
# `patch_item_prices.py` (which only touches `}` + price-0 rows) leaves them; price 0, so
# the game's own gates refuse 버리기 / 올리기 / 판매 on the cell (72차: the sell-price
# cache `price >> 2` gates all three). Icon 0x0d = the CARD frame the cash shop's 전투카드
# rows draw (user, 2026-09-14: "가방아이콘 캐시샵의 카드아이콘 사용해줘"). The 열기 verb is the host's (`bagbox.rs::LABEL_OPEN`); the useItem arm never runs.
RENAMES = {
    0x11: ("경험의비석", "경험치 2배;(사용후 1주일간)", None),
    0x12: ("경험의보주", "경험치 4배;(사용후 1주일간)", None),
    0x15: ("천상의 영약", "즉시 레벨 1 상승;(최대 70)", 0x15),  # icon 0x15 = 컵라면
    0x16: ("오브가방", "모든 오브를 보관하는 가방", 0x0d, None, 0),
    0x17: ("재료가방", "모든 재료를 보관하는 가방", 0x0d, None, 0),
    0x18: (None, "장비레벨제한 5감소;(사용후 1주일간)", None),
    0x19: ("소비가방", "모든 소비품을 보관하는 가방", 0x0d, None, 0),
}


def _plan(i):
    """(name, effect, icon, marker, price) with the short forms filled in: marker `}`
    kept, price untouched (`None`)."""
    row = RENAMES[i]
    name, eff, icon = row[0], row[1], row[2]
    marker = row[3] if len(row) > 3 else bytes([CASH])
    price = row[4] if len(row) > 4 else None
    return name, eff, icon, marker, price
# The record count of the pristine LGT i18_dat; a different count means we are not
# looking at the file this table was written against, so refuse to touch it.
EXPECTED_RECORDS = 27


def _fields(body: bytes):
    """(name, price4, effect, tail4) of one record body, on the correct model."""
    nl = body[0]
    name = body[1:1 + nl]
    price = body[1 + nl:5 + nl]
    el = body[5 + nl]
    eff = body[6 + nl:6 + nl + el]
    tail = body[6 + nl + el:]
    return name, price, eff, tail


def build(src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"input does not parse byte-exact ({consumed}/{len(src)} bytes) — this is "
            "the stale-length-prefix damage the old tool caused; feed the PRISTINE "
            "i18_dat (patch_asset.py falls back to <zip>.orig automatically)")
    if len(recs) != EXPECTED_RECORDS:
        raise SystemExit(f"expected {EXPECTED_RECORDS} records, got {len(recs)} — refusing")
    out = bytearray()
    for i, _off, body in recs:
        name, price, eff, tail = _fields(body)
        if len(tail) != 4:
            raise SystemExit(f"record {i:#x}: tail is {len(tail)}B, want 4 — model mismatch")
        if i in RENAMES:
            new_name, new_eff, new_icon, marker, new_price = _plan(i)
            if name[:1] != bytes([CASH]):
                raise SystemExit(f"record {i:#x} is not '}}'-marked ({name!r}) — refusing")
            if new_name is not None:
                name = (marker or b"") + new_name.encode("euc-kr")
            if new_eff is not None:
                eff = new_eff.encode("euc-kr")
            if new_icon is not None:
                tail = bytes([new_icon]) + tail[1:]
            if new_price is not None:
                price = int(new_price).to_bytes(4, "little")
        for blob in (name, eff):
            if not 0 < len(blob) <= 0xFF:
                raise SystemExit(f"record {i:#x}: {blob!r} does not fit a u8 length")
        body = bytes([len(name)]) + name + price + bytes([len(eff)]) + eff + tail
        out += len(body).to_bytes(2, "little") + body
    return bytes(out)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--in", dest="src", required=True)
    ap.add_argument("--out", dest="dst", required=True)
    ap.add_argument("--verify", action="store_true")
    a = ap.parse_args()
    src = open(a.src, "rb").read()
    dst = build(src)
    open(a.dst, "wb").write(dst)
    print(f"wrote {a.dst}  {len(src)} -> {len(dst)} bytes")
    if a.verify:
        verify(src, dst)


def verify(src: bytes, dst: bytes) -> None:
    """Byte-exact round-trip: only the RENAMES records changed, and as intended."""
    before = list(DB.records(src))
    after = list(DB.records(dst))
    consumed = after[-1][1] + 2 + len(after[-1][2]) if after else 0
    if consumed != len(dst):
        raise SystemExit(f"VERIFY FAILED: output has trailing bytes ({consumed}/{len(dst)})")
    if len(after) != len(before):
        raise SystemExit(f"VERIFY FAILED: {len(before)} records in, {len(after)} out")
    for (i, _ob, b), (_, _nb, x) in zip(before, after):
        if i in RENAMES:
            new_name, new_eff, new_icon, marker, new_price = _plan(i)
            name, _, eff, tail = _fields(x)
            if new_name is not None and name != (marker or b"") + new_name.encode("euc-kr"):
                raise SystemExit(f"VERIFY FAILED: record {i:#x} name is {name!r}")
            if new_eff is not None and eff != new_eff.encode("euc-kr"):
                raise SystemExit(f"VERIFY FAILED: record {i:#x} effect is {eff!r}")
            if new_icon is not None and tail[0] != new_icon:
                raise SystemExit(f"VERIFY FAILED: record {i:#x} icon is {tail[0]:#x}")
            # Everything the table does not name must survive: price, use arm, param.
            _, bp, _, bt = _fields(b)
            _, xp, _, xt = _fields(x)
            want_price = bp if new_price is None else int(new_price).to_bytes(4, "little")
            if xp != want_price or bt[1:] != xt[1:]:
                raise SystemExit(f"VERIFY FAILED: record {i:#x} price/arm/param changed")
        elif b != x:
            raise SystemExit(f"VERIFY FAILED: untouched record {i:#x} changed")
    print(f"VERIFY OK: {len(after)} records round-trip byte-exact, only "
          f"{sorted(hex(k) for k in RENAMES)} changed")
    for i, _off, body in after:
        if i in RENAMES:
            name, _, eff, tail = _fields(body)
            shown = name[1:] if name[:1] == bytes([CASH]) else name
            print("  %#04x  %-14s | %-28s | icon %#04x" % (
                i, shown.decode("euc-kr"), eff.decode("euc-kr"), tail[0]))


if __name__ == "__main__":
    main()
