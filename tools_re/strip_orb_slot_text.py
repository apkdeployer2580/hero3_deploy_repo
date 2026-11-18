"""Drop the "…에 결합." sentence from every orb's effect text in the game's item DB.

WHY. The orbs (category 0x10 = `i16_dat`) each end their effect text with a slot
advisory — `방어구에 결합.` / `무기/장갑에 결합.` / `신발에 결합.` … — but that advisory
is **not backed by any code**. 39차 established there is no body-part gate anywhere:
the client checks only `cmp r3,#0xc` (category <= equipment max) at 0x7608, and the
server's `_orb_combine` checks only `orb[8] == WH_CAT_ORB and equip[8] <= WH_CAT_EQUIP_MAX`.
So every orb already fits every equipment slot, on both sides.

40차 recorded the user's decision: **keep it that way and delete the advisory**, rather
than build a gate the original game never had. This tool is that deletion. It is the
zero-risk half of play-feedback item 2 — text only, no instance bytes, no save layout.

★★★ 2026-08-18 (56차) — THE ADVISORY TEXT GOES, BUT ITS `;` MUST STAY. READ THIS
BEFORE TOUCHING `strip_advisory`.

Deleting the whole `;<슬롯>에 결합.` segment — which is what this tool did from 43차 to
55차 — removed the **only `;` in every orb's effect string**, and that broke the
one-line description the game shows under the focused warehouse/shop/bag cell. Player
report (2026-08-18): "네트워크창고에서 오브의 효과가 빈값으로 나오는 오브들이 간혹있어.
상세보기로 들어가면 나오는데."

The cat-0x10 arm of the focused-item panel measures that one line by **scanning for `;`
with no NUL test and no bound** (LGT `0x1179c..0x117c8`, network warehouse; the same
idiom sits in seven more screens — `0xf958` 캐시샵, `0x10958` 창고, `0x11aac`, `0x202b0`,
`0x204b4`, `0x225f0`, `0x238dc`, `0x41680`):

    0x1179a  str  r2,[r3,#0x10]      ; row[0x10] = row[0x0c]   (rewind to the head)
    0x117b4  ldr  r3,[r0,#0x10]
    0x117b8  ldrb r3,[r3,r2]         ; c = effect[i]
    0x117ba  cmp  r3,#0x3b           ; ';'  ← the ONLY exit test
    0x117bc  beq  0x117ca
    0x117c0  adds r3,#1              ; len++            (u16, no bound)

`len` is then handed to the text drawer `0x3878` as a signed 16-bit count
(`0x11860 lsls #0x10 / asrs #0x10`), and that drawer bails out immediately when the
count is <= 0 (`0x39ea cmp r6,r0 / blt`, r6 = 0). With no `;` in the string the scan
runs off the end of the `malloc(efflen+1)` buffer and stops at the first 0x3b byte it
finds **in the heap**, so:

  * a 0x3b within ~32 KB  -> len positive -> the drawer stops at the string's own NUL
    (`0x3a06 cmp r5,#0 / beq`) and the line looks perfectly normal;
  * the first 0x3b >= 0x8000 bytes out -> len reads back NEGATIVE -> nothing is drawn
    at all = the empty effect line the player saw.

Which of the two happens depends on what is sitting behind the item-DB strings in the
guest heap, i.e. on allocation history — hence "간헐적", hence "오브들이" (only some of
them, and not the same ones every session). 상세보기 was unaffected because the popup
draws the whole string, NUL-bounded, instead of trusting that count.

Every one of the 15 stock orbs shipped with exactly one `;` (verified against the
pristine `hero3_fixed.zip.orig` and the KTF backup), so the game's own data satisfied
the invariant the renderer assumes and nobody ever hit this. **The invariant is: an orb
effect string ALWAYS contains a `;`.** `strip_advisory` therefore keeps the separator and
drops only the sentence after it, `ensure_line_break` enforces the invariant for any
future table, and `verify` fails the build if a row ever ships without one. A bare
trailing `;` is invisible: the panel draws the bytes before it (exactly the original
first line), and the multi-line popup just advances y with nothing to print.

⛔ Do NOT "clean up" the trailing `;`. It is load-bearing, and there is no code fix that
is cheaper — the unbounded scan is duplicated across eight screens.

The record model is `dump_item_db`'s (the correct one; see that file's docstring on why
`decode_items.py`'s was wrong and load-bearing):

    per record: [u16 LE reclen][u8 namelen][name][4B price][u8 efflen][effect][4B tail]

`;` is the game's own line break inside effect text, so the advisory is always the final
`;`-separated segment. Both spellings occur in the shipped file — 0x0c (흡혈의) has no
closing period — so the match tolerates a missing `.`.

★ 2026-08-14 — THIS TOOL ALSO APPENDS A 16TH ORB ROW: `소켓확장` (idx 0x0f).

That row is not an orb. It is the 소켓확장 cash item wearing an orb's clothes, so that the
game's OWN 오브결합 flow can be used to pick which piece of equipment gets the extra socket
(the player's ask: "네트워크창고에서 소켓확장 아이템을 사용하면 **오브처럼** 소켓확장을
시킬 아이템을 선택할 수 있게 해야해"). The client offers that flow on exactly one condition —
the warehouse row's category byte is 0x10 (`0x7674 cmp r3,#0x10`, the sole gate; the menu-row
handler at 0x724e never re-reads it) — and the row's NAME and EFFECT come from this file via
`dbPtr[cat][idx]`. So a row here is what makes it read "소켓확장" instead of an orb's name.

⚠ THE ROW COUNT IS CODE, NOT DATA. Appending here alone is a NO-OP that also corrupts the
heap if anything ever indexes 0x0f: the loader bounds itself with a hardcoded const table
(`ldrsb` @ guest 0x53eca / 0x53f86 off the table at VA 0x5e474) and the orb table is a
hardcoded `alloc(0xb4<<1)` = 360 B = exactly 15 x 0x18 (@0x53748). Three guest bytes must
change WITH this file, and they live in `vendor/wie/data/binary_patches.toml` as
`[[entry.patch]]` items on the LGT `6a352944…` entry:

    VA 0x5e484  0f -> 10   row count for cat 0x10
    VA 0x53748  b4 -> c0   alloc immediate: 0xc0<<1 = 384 = 16 x 0x18
    VA 0x52744  0f -> 10   teardown bound (else the 16th row's strings leak)

They are patches rather than edits to `binary.mod` on purpose: the hook table is keyed by
that file's MD5, so editing it would silently unhook `blit565` / `blend565` / `blitlut565`
and cost a third to two-thirds of the guest-instruction budget.

★★★ 72차 (2026-08-29) — THE PRICE IS 4, AND IT IS NOT COSMETIC. READ THIS BEFORE
CHANGING IT BACK.

Player report: *"소켓확장이 버릴 수 없는 것과 창고에 넣을 수 없는 점 … 영웅서기4,5와 다르게
인벤토리가 아이템 종류별로 나눠져있지 않아서 평소에는 창고에 넣어둬야 하는데"*. Both
refusals turned out to be THIS FIELD, and nothing else:

  * 버리기 — the field-inventory drop arm `0x475bc` refuses when the focused item's cached
    SELL PRICE `CHAR+0xacac` is 0, and that cache is written as `price == 0 ? 0 : price>>2`.
    The message is 「버릴수 없는 아이템입니다.」 (`InGame_txt[0xa5]`).
  * 창고 올리기 — the deposit handler calls `canDeposit(cat, idx)` at `0x6d70`, whose
    cat-0x10 arm (`0x6dc0`) returns `ldrb [orbTable + idx*0x18 + 8]` = **the low byte of the
    price**. The message is 「해당 아이템은 올릴 수 없습니다.」 (`0x5bce8`).
    ⚠ CLAUDE.md described that gate as a "거래가능" flag. There is no such flag — it is the
    price, read one byte wide, which is also why five ordinary equipment rows whose price is
    a multiple of 256 cannot be deposited either (블러디고글/다크써클릿/라이트닝기어 70400,
    가죽건틀릿 6400, 밴디드워커 19200). Original-game behaviour; left alone.

4 is the smallest value that satisfies BOTH: the low byte is `0x04` (deposit passes) and
`4 >> 2 = 1` (drop passes). 1..3 would fix only the deposit.

── THE TRADE-OFF THIS ACCEPTS ───────────────────────────────────────────────
The old note here read: *"PRICE 0 IS DELIBERATE … the cash shop hands this item out for
free — a sellable row would be an infinite gold press."* That is still literally true, and
the press is now open: buy 소켓확장, sell it for **1 gold**. It is accepted because 38차
already made exactly that trade for the OTHER 41 cash rows — `patch_item_prices.py` sets
every `}`-marked cash item to price 4 for the same 1-gold sale — so this row was the odd
one out rather than the protected one. One gold per shop round-trip is slower than any
monster in the game.

⛔ If it is ever reverted, revert the PLAYER-FACING claim with it: 소켓확장 goes back to
being undroppable and unstorable, and the report above comes back.

Usage:
    python tools_re/strip_orb_slot_text.py --in <i16_dat> --out <i16_dat> [--verify]
(normally driven by `patch_asset.py`, which feeds it the PRISTINE file)
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_item_db as DB  # noqa: E402  (the CORRECT record model)

# The record count of the pristine LGT i16_dat; a different count means we are not
# looking at the file this tool was written against, so refuse to touch it.
EXPECTED_RECORDS = 15
# Trailing segment to drop: "<slots>에 결합" with an optional closing period. Anchored,
# so an effect that merely mentions 결합 mid-sentence is left alone.
ADVISORY = re.compile(r"^[^;]*에 결합\.?$")
#: The game's own line break inside effect text — and the byte the focused-cell panel
#: scans for with no NUL test (see the module docstring). Every orb row must carry one.
LINE_BREAK = ";"

# ── the appended 소켓확장 row (see the module docstring for why it exists) ──────
#: Its index. MUST equal `EXPECTED_RECORDS` — it is the row right after the last real
#: orb — and must match `hero3_protocol.ORB_IDX_SOCKET_EXPAND` and the three guest
#: patches. `_new_orb_record`'s random roll stays bounded by ORB_INDEX_COUNT = 15, so
#: the game can never drop one.
SOCKET_ROW_IDX = EXPECTED_RECORDS
SOCKET_ROW_NAME = "소켓확장"
#: `;` is the game's own line break inside effect text.
#:
#: ★ 71차-B (TODO `AN`, 사용자 결정 2026-08-26): the second segment used to read
#: `창고에서 장비를 선택.` and that is now a LIE — 69차 moved 오브결합 into the in-game bag
#: (`orbcombine.rs`, popup tag 0x64 row 2) and 70차 moved the cash delivery there too, so a
#: player who follows the old sentence opens the network warehouse and finds nothing. The
#: first segment and the `;` are untouched, so 56차's `;` invariant and the `ldrsb` length
#: cap are both unaffected.
#: ⚠ 71차's handoff note said this would make the row 2 B shorter — it does NOT. `창고` and
#: `가방` are both two Hangul syllables = 4 B in EUC-KR, so the record's length is unchanged
#: (measured). Do not "correct" the 44차 guest constants to match a shrink that never happened;
#: they are sized for the record COUNT (16), and per-record length is carried by its own u16.
SOCKET_ROW_EFFECT = "장비의 오브 홈을 하나 늘림.;가방에서 장비를 선택."
#: 4 = the minimum that opens BOTH the drop gate and the deposit gate — see the docstring.
#: Every real orb here is 1200, and 4 is what 38차 gave the other 41 cash rows.
SOCKET_ROW_PRICE = 4
#: The 4-byte tail, copied verbatim from row 0x00 (투신의). Byte 0 is the stat type the
#: equipment detail line turns into a label (`0x50 + type`) and it is gated to 2..0x12 at
#: guest 0x4f184 — 0x02 is inside that window. Nothing here is ever used in practice
#: (the server converts this row into a socket increment and never stamps it as a
#: socketed orb), but a row that would render sanely if it ever were is the cheap option.
#:
#: ★ 71차 correction to the sentence above: byte 0 is the STAT TYPE ID and it does more
#: than label a line — `0x4f014` feeds it to `addStat(statId, bonus)` for the wearer, so
#: an orb really does raise the stat its text names. The 2..0x12 window and 0x02 being
#: inside it are both still correct, so this row is unaffected either way.
#: See `PLAY_FEEDBACK_FIXES.md §71차 §4`.
SOCKET_ROW_TAIL = bytes([0x02, 0x02, 0x02, 0x05])


# ── description overrides (71차) ─────────────────────────────────────────────
#
# ★★ 2026-08-26, player instruction: "뇌제의 오브 게임내 설명을 바꿔줘 장비의 성능을 강화로".
#
# WHY IT WAS WRONG. 71차 read the two sites that consume a socketed orb and they disagree
# with this one row's text:
#
#   bonus = power * (f*f + f) / 2            ; power = orbRow[0x15], f = filled sockets
#   (1) the ITEM's own number, guest 0x528f0:
#         rec[+0x3ce] = (100 + bonus) * (dbRow[0x16] + mult*refine) / 100
#       — computed WITHOUT looking at the stat type, so EVERY orb raises the stat the
#       piece of equipment happens to have. Weapon -> 공격력; armour/gloves -> 방어력.
#   (2) a NAMED STAT LINE on the wearer, guest 0x4f014 -> addStat(statId, bonus) (0x4ee34):
#         gated on `(u8)(statId - 2) <= 0x10`, i.e. statId 2..0x12.
#
# 뇌제의 carries statId **0x00**, which is OUTSIDE that window — so it is the one orb that
# adds no named stat line of its own, and path (1) is all it does.
#
# ⚠ Path (1) is NOT cosmetic, and saying so would be the easy mistake here: `rec+0x3ce` is
# the number the character's ATT/DEF are built FROM (weapon getters `0x4cdb4`/`0x4ce64`,
# armour sum `0x4d014`, all called inside `0x4f014`). So 뇌제의 in a weapon really does
# raise 공격력 and in armour really does raise 방어력 — it just does it through the piece of
# equipment instead of as its own line. That is precisely why "무기의 공격력을 강화" reads
# false in every non-weapon slot, which is what the player hit: "장갑에 적용했을떄 성능+x%
# 라고 뜨면 공격력이 올라가는건지 방어력에 성능이 올라가는건지 모르겠어".
# "장비의 성능을 강화" is true in every slot, and 성능 is the game's own label for that
# number (`성    능  %d`, rodata 0x5bfd0).
#
# ⛔ This is a TEXT change only. The stat type and power bytes are the record's 4-byte tail
# and `build()` copies them through untouched — see `verify`, which fails if a tail moves.
#
# The override replaces the FIRST `;`-separated segment, which is the one-line description
# the focused-cell panel draws, and keeps the `;` and everything after it. See the module
# docstring for why that separator is load-bearing.
#
# ⛔ Keyed by index AND name. An index alone would silently rewrite whatever row happens to
# sit at 0x02 if this tool were ever pointed at another table (the KTF `i16_dat`, a future
# revision), and rewriting the wrong orb's text is invisible until a player reads it.
DESC_OVERRIDE = {
    0x02: ("뇌제의", "장비의 성능을 강화."),
}


def apply_override(idx: int, name: str, effect: str) -> str:
    """Swap in the override's one-line description, keeping the `;` and everything after."""
    hit = DESC_OVERRIDE.get(idx)
    if hit is None:
        return effect
    want_name, new = hit
    if name != want_name:
        raise SystemExit(
            f"record {idx:#x} is {name!r} but DESC_OVERRIDE expects {want_name!r} — "
            "refusing to rewrite a row this table did not mean")
    if LINE_BREAK in new:
        raise SystemExit(f"DESC_OVERRIDE[{idx:#04x}] must not contain {LINE_BREAK!r}")
    _head, sep, rest = effect.partition(LINE_BREAK)
    return new + (sep + rest if sep else LINE_BREAK)


def socket_row() -> bytes:
    """The full on-disk record (length prefix included) for the 소켓확장 row."""
    name = SOCKET_ROW_NAME.encode("euc-kr")
    eff = SOCKET_ROW_EFFECT.encode("euc-kr")
    body = (bytes([len(name)]) + name + SOCKET_ROW_PRICE.to_bytes(4, "little")
            + bytes([len(eff)]) + eff + SOCKET_ROW_TAIL)
    return len(body).to_bytes(2, "little") + body


def _fields(body: bytes):
    """(name, price4, effect, tail4) of one record body."""
    nl = body[0]
    name = body[1:1 + nl]
    price = body[1 + nl:5 + nl]
    el = body[5 + nl]
    eff = body[6 + nl:6 + nl + el]
    tail = body[6 + nl + el:]
    return name, price, eff, tail


def strip_advisory(effect: str) -> str:
    """Drop the final `<slots>에 결합.` segment but KEEP the `;` that introduced it.

    Keeping the separator is not cosmetic — see the module docstring. The panel that
    renders the focused cell's one-line description finds the end of that line by
    scanning for `;` with no terminator check, so a `;`-less orb string makes it read
    past the buffer and, about as often as not, produce a negative length and draw
    nothing at all.
    """
    if LINE_BREAK not in effect:
        return effect
    head, _, last = effect.rpartition(LINE_BREAK)
    return head + LINE_BREAK if ADVISORY.match(last) else effect


def ensure_line_break(effect: str) -> str:
    """The renderer's invariant, applied constructively: the string carries a `;`.

    A no-op for today's table (every stock row has one and `strip_advisory` preserves
    it). It exists so a future edit that produces a single-segment effect cannot
    reintroduce the 2026-08-18 empty-description bug silently.
    """
    return effect if LINE_BREAK in effect else effect + LINE_BREAK


def build(src: bytes) -> bytes:
    recs = list(DB.records(src))
    consumed = recs[-1][1] + 2 + len(recs[-1][2]) if recs else 0
    if consumed != len(src):
        raise SystemExit(
            f"input does not parse byte-exact ({consumed}/{len(src)} bytes) — feed the "
            "PRISTINE i16_dat (patch_asset.py falls back to <zip>.orig automatically)")
    if len(recs) != EXPECTED_RECORDS:
        raise SystemExit(f"expected {EXPECTED_RECORDS} records, got {len(recs)} — refusing")

    out = bytearray()
    for i, _off, body in recs:
        name, price, eff, tail = _fields(body)
        if len(tail) != 4:
            raise SystemExit(f"record {i:#x}: tail is {len(tail)}B, want 4 — model mismatch")
        text = eff.decode("euc-kr")
        stripped = apply_override(i, name.decode("euc-kr"), ensure_line_break(strip_advisory(text)))
        if stripped != text:
            eff = stripped.encode("euc-kr")
        if LINE_BREAK not in stripped:
            raise SystemExit(f"record {i:#x}: effect {stripped!r} has no {LINE_BREAK!r}")
        if not 0 < len(eff) <= 0xFF:
            raise SystemExit(f"record {i:#x}: effect {eff!r} does not fit a u8 length")
        body = bytes([len(name)]) + name + price + bytes([len(eff)]) + eff + tail
        out += len(body).to_bytes(2, "little") + body
    out += socket_row()
    return bytes(out)


def verify(src: bytes, dst: bytes) -> None:
    """Byte-exact round-trip: names/prices/tails untouched, only the advisory gone —
    plus exactly one appended row, the 소켓확장 one."""
    before = list(DB.records(src))
    after = list(DB.records(dst))
    consumed = after[-1][1] + 2 + len(after[-1][2]) if after else 0
    if consumed != len(dst):
        raise SystemExit(f"VERIFY FAILED: output has trailing bytes ({consumed}/{len(dst)})")
    if len(after) != len(before) + 1:
        raise SystemExit(f"VERIFY FAILED: {len(before)} records in, {len(after)} out "
                         f"(want {len(before) + 1} — the 15 orbs plus 소켓확장)")

    # The appended row, checked as bytes rather than as fields: it is the thing the
    # three guest patches are sized for, so a silent drift here is a heap overrun.
    i, off, body = after[-1]
    if i != SOCKET_ROW_IDX:
        raise SystemExit(f"VERIFY FAILED: appended row is idx {i:#x}, want {SOCKET_ROW_IDX:#x}")
    got = len(body).to_bytes(2, "little") + body
    if got != socket_row():
        raise SystemExit(f"VERIFY FAILED: appended row bytes are {got.hex(' ')}, "
                         f"want {socket_row().hex(' ')}")
    n, _p, e, t = _fields(body)
    if n.decode("euc-kr") != SOCKET_ROW_NAME or t != SOCKET_ROW_TAIL:
        raise SystemExit("VERIFY FAILED: appended row does not decode to 소켓확장")

    changed = 0
    for (i, _ob, b), (_, _nb, x) in zip(before, after):
        bn, bp, be, bt = _fields(b)
        xn, xp, xe, xt = _fields(x)
        if (bn, bp, bt) != (xn, xp, xt):
            raise SystemExit(f"VERIFY FAILED: record {i:#x} name/price/tail changed")
        want = apply_override(i, bn.decode("euc-kr"), ensure_line_break(strip_advisory(be.decode("euc-kr"))))
        if xe.decode("euc-kr") != want:
            raise SystemExit(f"VERIFY FAILED: record {i:#x} effect is {xe!r}, want {want!r}")
        if xe != be:
            changed += 1
        if "에 결합" in xe.decode("euc-kr"):
            raise SystemExit(f"VERIFY FAILED: record {i:#x} still advises a slot: {xe!r}")

    # ★ Every override actually landed. Without this a typo'd key (an index that is not in
    # the table, or one whose text already matched) would be a silent no-op and the shipped
    # asset would keep the wrong sentence — the 43차/44차 failure mode, one layer down.
    for idx, (want_name, new) in DESC_OVERRIDE.items():
        row = next((b for i, _o, b in after if i == idx), None)
        if row is None:
            raise SystemExit(f"VERIFY FAILED: DESC_OVERRIDE has {idx:#04x} but there is no such record")
        got_name = _fields(row)[0].decode("euc-kr")
        if got_name != want_name:
            raise SystemExit(f"VERIFY FAILED: record {idx:#x} is {got_name!r}, override expects {want_name!r}")
        got = _fields(row)[2].decode("euc-kr")
        if not got.startswith(new + LINE_BREAK) and got != new:
            raise SystemExit(f"VERIFY FAILED: record {idx:#x} effect is {got!r}, "
                             f"override wanted it to start with {new + LINE_BREAK!r}")

    # ★ The renderer invariant, checked on EVERY output row including the appended one:
    # a `;`-less orb effect makes the focused-cell panel scan past the string and, when
    # the length it computes reads back negative, draw nothing (module docstring, 56차).
    for i, _off, body in after:
        _, _, eff, _ = _fields(body)
        cut = eff.find(LINE_BREAK.encode())
        if cut < 0:
            raise SystemExit(
                f"VERIFY FAILED: record {i:#x} effect {eff!r} carries no {LINE_BREAK!r} — "
                "the focused-cell description would come out empty at random")
        if cut == 0:
            raise SystemExit(
                f"VERIFY FAILED: record {i:#x} effect {eff!r} starts with {LINE_BREAK!r} — "
                "the panel's line length would be 0 and it would draw nothing")
        # `ldrsb` reads both length prefixes (guest 0x53ef6 name, 0x53f3c effect), so a
        # 0x80+ length is a NEGATIVE malloc argument in the loader, not a long string.
        if len(eff) > 0x7F:
            raise SystemExit(f"VERIFY FAILED: record {i:#x} effect is {len(eff)}B; the "
                             "loader reads the length with ldrsb, so 0x7f is the ceiling")
    print(f"VERIFY OK: {len(after)} orb records round-trip byte-exact, "
          f"{changed} advisories removed, all carry a {LINE_BREAK!r}")
    for i, _off, body in after:
        _, _, eff, _ = _fields(body)
        print("  %#04x  %s" % (i, eff.decode("euc-kr")))


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


if __name__ == "__main__":
    main()
