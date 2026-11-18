"""Append 「헬모드로 재시작합니다」 to `res/dat/InGame_txt` — the ending window's line after a
hardcore clear (140차, 2026-09-29).

WHY. Ending window 18 paints `InGame_txt` row 170 「하드코어 모드로;재시작합니다」 through the
common-widget installer `0x46b7c(0x13)` (`0x46cec movs r3,#0xaa`), whatever difficulty the
clear was on. Since 133차 a HARDCORE clear restarts in 헬 (difficulty byte 2), so the player
read 「하드코어 모드로 재시작합니다」 and then got 헬 — the user's *"뭔가 안맞아"*. Normal's
next playthrough IS 하드코어, so row 170 must stay as it is; the 헬 wording is a SECOND row,
and the guest hook `ending_label_lgt` (`wie_core_arm::binary_patches::endinglabel`) picks it
when the byte is not 0.

THE ROW INDEX IS A CONTRACT. The hook loads it with an 8-bit `movs`, so the row must sit
below 256 and the Rust side carries the number by hand (`endinglabel::HELL_RESTART_LABEL`).
This module owns the `InGame_txt` member of the `patch_asset.py` chain: it runs
`add_attack_speed_label.build` first (row 198 = 공격속도, whose index fixes a stat id — see
that module) and appends ours right after, at [`EXPECTED_INDEX`] = 199. `patch_asset.py
--check` reads the Rust constant out of `endinglabel.rs` (`hell_label_agrees`) and refuses a
zip whose row is anywhere else.

FORMAT: see `add_attack_speed_label` (u16 length · u16 count · u16 offset[count] · NUL-terminated
EUC-KR strings). `;` is the game's own line break inside a message — ours is ONE line on purpose:
the widget centres every line, and split at `;` the shorter 「헬모드로」 sat visibly off to the right
(probe frame, 140차); at 11 syllables it fits the box's ~143-px line.

Idempotent: fed the PRISTINE file it builds both rows; fed a table that already ends with
ours it returns it unchanged (`patch_asset` calls `build` on the pristine bytes only, but
`verify` is written for either).
"""
import re
import sys
from pathlib import Path

import add_attack_speed_label as ASL

LABEL = "헬모드로 재시작합니다"
# 198 stock rows + the 공격속도 row (198) => ours is 199. Mirrored in
# `vendor/wie/wie_core_arm/src/binary_patches/endinglabel.rs::HELL_RESTART_LABEL`.
EXPECTED_INDEX = ASL.PRISTINE_N + 1
# The game's own row this one stands in for after a hardcore clear.
HARDCORE_INDEX = 170

RUST_CONST = Path(__file__).resolve().parents[1] / 'vendor' / 'wie' / 'wie_core_arm' / 'src' / 'binary_patches' / 'endinglabel.rs'
_RUST_RE = re.compile(r'pub const HELL_RESTART_LABEL: u32 = (\d+);')


def raw_label() -> bytes:
    return LABEL.encode("euc-kr")


def label_index(blob: bytes) -> int:
    """The row our line sits at in this table, or a SystemExit naming the member."""
    rows = ASL._parse(blob)
    for i, r in enumerate(rows):
        if r == raw_label():
            return i
    raise SystemExit("InGame_txt: the 헬모드 restart label is not in this table")


def rust_index() -> int:
    """`HELL_RESTART_LABEL` as the hook source carries it — the other half of the contract."""
    try:
        src = RUST_CONST.read_text(encoding='utf-8')
    except OSError as e:
        raise SystemExit(f"cannot read {RUST_CONST}: {e}")
    m = _RUST_RE.search(src)
    if not m:
        raise SystemExit(f"{RUST_CONST}: no `pub const HELL_RESTART_LABEL: u32 = N;`")
    return int(m.group(1))


def hell_label_agrees(ingame_txt: bytes) -> str:
    """`''` when the table's row and the hook's constant say the same index, else why."""
    try:
        idx = label_index(ingame_txt)
    except SystemExit as e:
        return str(e)
    want = rust_index()
    if idx != want:
        return (f'the 헬모드 restart label sits at InGame_txt row {idx} but endinglabel.rs '
                f'HELL_RESTART_LABEL is {want} — the ending window would paint another line')
    if idx != EXPECTED_INDEX:
        return f'the 헬모드 restart label sits at row {idx}, not the planned {EXPECTED_INDEX}'
    return ''


def build(src: bytes) -> bytes:
    rows = ASL._parse(src)
    if rows and rows[-1] == raw_label():
        return src
    with_speed = ASL.build(src)  # refuses a table that is not the stock one (or already +1)
    rows = ASL._parse(with_speed)
    if rows[HARDCORE_INDEX] != "하드코어 모드로;재시작합니다".encode("euc-kr"):
        raise SystemExit(f"InGame_txt: row {HARDCORE_INDEX} is not the 하드코어 restart line — refusing")
    out = ASL._build(rows + [raw_label()])
    if label_index(out) != EXPECTED_INDEX:
        raise SystemExit(f"InGame_txt: the 헬모드 row landed at {label_index(out)}, wanted {EXPECTED_INDEX}")
    return out


def verify(before: bytes, after: bytes) -> None:
    b, a = ASL._parse(before), ASL._parse(after)
    if b and b[-1] == raw_label():
        if before != after:
            raise SystemExit("InGame_txt: the 헬모드 label was already last but bytes changed")
        return
    # Everything the 공격속도 tool guarantees, on the table as it stands before OUR append
    # (a no-op when `before` already carried that row — then ours is the only addition).
    ASL.verify(before, ASL._build(a[:-1]))
    added = 1 if (b and b[-1] == ASL.raw_label()) else 2
    if len(a) != len(b) + added:
        raise SystemExit(f"InGame_txt: wanted +{added} row(s), {len(b)} -> {len(a)}")
    if a[-1] != raw_label():
        raise SystemExit(f"InGame_txt: last row {a[-1]!r} != {raw_label()!r}")
    if a[HARDCORE_INDEX] != b[HARDCORE_INDEX]:
        raise SystemExit(f"InGame_txt: row {HARDCORE_INDEX} (the 하드코어 line) changed — it must stay for a normal clear")
    why = hell_label_agrees(after)
    if why:
        raise SystemExit(f"InGame_txt: {why}")
    print(f"VERIFY OK: InGame_txt +1 `{LABEL}` idx={len(a) - 1} (hook HELL_RESTART_LABEL = {rust_index()})")


HANDLERS = {"InGame_txt": sys.modules[__name__]}
