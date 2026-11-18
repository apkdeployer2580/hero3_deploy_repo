"""Append `공격속도 +` to `res/dat/InGame_txt` so the 공격속도 option has a real label.

WHY. The item detail screen labels an option by indexing this table with the option's
stat id — `index = 80 + id` in `tools_re/ingame_ids.txt` numbering (성능+ 80, 물리공격+ 85
= id 0x05, SP회복+ 95 = 0x0f, 쿨타임- 97 = 0x11). 공격속도 is a stat the game does not
have, so it had to pick an id `addStat` (0x4ee34, gate `id - 2 <= 0x15`) ignores — and any
such id lands on somebody else's word. The player saw 「힘 +127」 (id 0x18 -> index 104 =
힘 +), which is this table telling the truth about a row that was never meant to exist.

⛔ DO NOT OVERWRITE INDEX 104. That string is the real 힘, used by the stat-point screen.
The fix is to grow the table and move the stat id to the new row.

FORMAT (measured, and the file checks out byte-exact):

```text
u16 total length   (== len(file))
u16 count
u16 offset[count]  (from the start of the file; offset[0] == 4 + count*2)
NUL-terminated EUC-KR strings, in offset order
```

So appending is: count += 1, every offset += 2 (one more slot), one new offset, the string
at the end, and the length word re-stamped. `ATTACK_SPEED_STAT` must equal
`new index - LABEL_BASE`; [`stat_id`] computes it so the two can never drift.

This module is owned by `patch_asset.py` — `build` is fed the PRISTINE file and must be
idempotent when the row is already last.
"""
import struct
import sys

LABEL = "공격속도 +"
LABEL_BASE = 80        # index = LABEL_BASE + stat id (성능 + is id 0 at index 80)
PRISTINE_N = 198       # refuse to guess if this is not the stock table


def _parse(blob: bytes):
    total, count = struct.unpack_from("<HH", blob, 0)
    if total != len(blob):
        raise SystemExit(f"InGame_txt: length word {total} != file {len(blob)}")
    offs = list(struct.unpack_from(f"<{count}H", blob, 4))
    if offs and offs[0] != 4 + count * 2:
        raise SystemExit(f"InGame_txt: first offset {offs[0]} != header {4 + count * 2}")
    out = []
    for o in offs:
        end = blob.index(b"\x00", o)
        out.append(blob[o:end])
    return out


def _build(rows) -> bytes:
    count = len(rows)
    head = 4 + count * 2
    offs, body, at = [], bytearray(), head
    for r in rows:
        offs.append(at)
        body += r + b"\x00"
        at += len(r) + 1
    blob = bytearray(struct.pack("<HH", head + len(body), count))
    for o in offs:
        blob += struct.pack("<H", o)
    return bytes(blob + body)


def raw_label() -> bytes:
    return LABEL.encode("euc-kr")


def stat_id(blob: bytes) -> int:
    """The id an item must carry for this table to label it 공격속도."""
    rows = _parse(blob)
    for i, r in enumerate(rows):
        if r == raw_label():
            return i - LABEL_BASE
    raise SystemExit("InGame_txt: the 공격속도 label is not in this table")


def build(src: bytes) -> bytes:
    rows = _parse(src)
    if rows and rows[-1] == raw_label():
        return src
    if len(rows) != PRISTINE_N:
        raise SystemExit(f"InGame_txt: expected {PRISTINE_N} rows, got {len(rows)} — refusing")
    return _build(rows + [raw_label()])


def verify(before: bytes, after: bytes) -> None:
    b, a = _parse(before), _parse(after)
    if b and b[-1] == raw_label():
        if before != after:
            raise SystemExit("InGame_txt: the label was already last but bytes changed")
        return
    if len(a) != len(b) + 1:
        raise SystemExit(f"InGame_txt: wanted +1 row, {len(b)} -> {len(a)}")
    for i in range(len(b)):
        if a[i] != b[i]:
            raise SystemExit(f"InGame_txt: row {i} changed — only the append is allowed")
    if a[-1] != raw_label():
        raise SystemExit(f"InGame_txt: last row {a[-1]!r} != {raw_label()!r}")
    sid = stat_id(after)
    # The whole point of the new id: the game's own `addStat` must keep ignoring it.
    if (sid - 2) <= 0x15:
        raise SystemExit(f"InGame_txt: stat id {sid:#04x} is inside addStat's gate")
    if not (0 <= sid <= 127):
        raise SystemExit(f"InGame_txt: stat id {sid} does not fit the option byte (i8)")
    print(f"VERIFY OK: InGame_txt +1 `{LABEL}` idx={len(a) - 1} => stat id {sid:#04x} ({sid})")


HANDLERS = {"InGame_txt": sys.modules[__name__]}
