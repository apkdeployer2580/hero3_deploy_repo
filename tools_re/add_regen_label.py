"""Append 「5초당 HP회복 +」 to `res/dat/InGame_txt` — the label of 레아의목걸이's option 2 (150차).

WHY. 150차 (2026-10-01, user decision — `LEGACY_ITEMS_REWORK_85_90.md §11`/§12, the table in
`acc_options.py`) gives 레아의목걸이 a NEW option: every 5 s of play in the field the host heals
20 % of the true max HP. The game has no such stat (its own HP회복 0x03 heals max x N / 1000 per
~2 s of standing still), so the row's second pair carries a stat id `addStat` (0x4ee34, gate
`id - 2 <= 0x15`) ignores — `acc_options.REGEN` = 0x78 — and the host reads it. The item detail
window names an option `InGame_txt[80 + id]` (`add_attack_speed_label`'s rule), so id 0x78 needs
a row at index 200 that says what the option is; without it the window would print whatever sits
there (nothing — past the table).

THE ROW INDEX IS A CONTRACT, three ways:
  * `80 + acc_options.REGEN` — the detail window's rule (id 0x78 -> row 200);
  * `vendor/wie/wie_util/src/acc_options.rs` carries `REGEN_STAT` / `REGEN_LABEL_ROW` (generated
    by `gen_acc_options.py`) — the host and the guest hooks read those; `regen_label_agrees`
    reads them back out of the Rust source, the same pattern as
    `add_hell_restart_label.hell_label_agrees` / `patch_asset.stat_ids_agree`;
  * the rows below it: 198 = 공격속도 (id 0x76, `add_attack_speed_label`), 199 = 「헬모드로
    재시작합니다」 (`add_hell_restart_label`; id 0x77 is therefore reserved — a stat labelled
    with the ending line). Ours is the third append, at [`EXPECTED_INDEX`] = 200.

This module owns the `InGame_txt` member of the `patch_asset.py` chain from 150차 on: it runs
`add_hell_restart_label.build` first (which runs `add_attack_speed_label` inside) and appends
ours right after — 200 rows before, 201 after. ⛔ The game appends the value itself, and the
`%` (the widened `0x6d34` rule — a guest hook's job, not the asset's), so the label ends in `+ `
like the game's own option labels (`특수공격 +` …).

FORMAT: `add_attack_speed_label` (u16 length · u16 count · u16 offset[count] · NUL-terminated
EUC-KR strings). Idempotent: fed the PRISTINE file it builds all three rows; fed a table that
already ends with ours it returns it unchanged.
"""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import acc_options as AO             # noqa: E402
import add_attack_speed_label as ASL  # noqa: E402
import add_hell_restart_label as HRL  # noqa: E402

LABEL = AO.REGEN_LABEL                # 「5초당 HP회복 +」
STAT = AO.REGEN                       # 0x78
# 198 stock rows + 공격속도 (198) + 헬모드 (199) => ours is 200 = 80 + 0x78.
EXPECTED_INDEX = HRL.EXPECTED_INDEX + 1
if EXPECTED_INDEX != ASL.LABEL_BASE + STAT or EXPECTED_INDEX != AO.REGEN_LABEL_ROW:
    raise SystemExit(f"add_regen_label: row {EXPECTED_INDEX} != 80 + {STAT:#04x} or != "
                     f"acc_options.REGEN_LABEL_ROW {AO.REGEN_LABEL_ROW} — the three halves disagree")

RUST_CONST = (Path(__file__).resolve().parents[1] / 'vendor' / 'wie' / 'wie_util' / 'src'
              / 'acc_options.rs')
_ROW_RE = re.compile(r'pub const REGEN_LABEL_ROW: u16 = (\d+);')
_STAT_RE = re.compile(r'pub const REGEN_STAT: u8 = (0x[0-9a-fA-F]+|\d+);')


def raw_label() -> bytes:
    return LABEL.encode("euc-kr")


def _check_label() -> None:
    raw = raw_label()  # raises on a syllable the game font cannot draw
    if b";" in raw or b"\x00" in raw:
        raise SystemExit(f"add_regen_label: {LABEL!r} holds a line break or a NUL")
    if raw[:1] in (b"@", b"|", b"$", b"{", b"}", b"'", b'"'):
        raise SystemExit(f"add_regen_label: {LABEL!r} starts with a colour escape")


_check_label()


def label_index(blob: bytes) -> int:
    """The row our line sits at in this table, or a SystemExit naming the member."""
    for i, r in enumerate(ASL._parse(blob)):
        if r == raw_label():
            return i
    raise SystemExit("InGame_txt: the 5초당 HP회복 label is not in this table")


def rust_consts() -> tuple:
    """`(REGEN_LABEL_ROW, REGEN_STAT)` as `acc_options.rs` carries them — the other half."""
    try:
        src = RUST_CONST.read_text(encoding='utf-8')
    except OSError as e:
        raise SystemExit(f"cannot read {RUST_CONST}: {e}")
    row, stat = _ROW_RE.search(src), _STAT_RE.search(src)
    if not row or not stat:
        raise SystemExit(f"{RUST_CONST}: no `pub const REGEN_LABEL_ROW: u16 = N;` / "
                         f"`pub const REGEN_STAT: u8 = 0xNN;` (rerun tools_re/gen_acc_options.py)")
    return int(row.group(1)), int(stat.group(1), 0)


def regen_label_agrees(ingame_txt: bytes) -> str:
    """`''` when the table's row, the Rust constants and the stat id all say the same, else why."""
    try:
        idx = label_index(ingame_txt)
    except SystemExit as e:
        return str(e)
    row, stat = rust_consts()
    if idx != row:
        return (f'the 5초당 HP회복 label sits at InGame_txt row {idx} but acc_options.rs '
                f'REGEN_LABEL_ROW is {row} — the detail window would name the option with another line')
    if stat != STAT or row != ASL.LABEL_BASE + stat:
        return (f'acc_options.rs says REGEN_STAT {stat:#04x} / REGEN_LABEL_ROW {row}, but the label '
                f'rule is row = 80 + id and acc_options.py says {STAT:#04x}')
    if idx != EXPECTED_INDEX:
        return f'the 5초당 HP회복 label sits at row {idx}, not the planned {EXPECTED_INDEX}'
    if (stat - 2) <= 0x15 or not (0 <= stat <= 127):
        return f'stat id {stat:#04x} is inside addStat\'s gate or does not fit the option byte (i8)'
    return ''


def build(src: bytes) -> bytes:
    rows = ASL._parse(src)
    if rows and rows[-1] == raw_label():
        return src
    with_hell = HRL.build(src)  # refuses a table that is not the stock one (or a partial one)
    rows = ASL._parse(with_hell)
    if len(rows) != EXPECTED_INDEX:
        raise SystemExit(f"InGame_txt: {len(rows)} rows after the 헬모드 label, expected {EXPECTED_INDEX} — refusing")
    if rows[-1] != HRL.raw_label():
        raise SystemExit("InGame_txt: the last row is not the 헬모드 label — refusing")
    out = ASL._build(rows + [raw_label()])
    if label_index(out) != EXPECTED_INDEX:
        raise SystemExit(f"InGame_txt: the 5초당 HP회복 row landed at {label_index(out)}, wanted {EXPECTED_INDEX}")
    return out


def verify(before: bytes, after: bytes) -> None:
    b, a = ASL._parse(before), ASL._parse(after)
    if b and b[-1] == raw_label():
        if before != after:
            raise SystemExit("InGame_txt: the 5초당 HP회복 label was already last but bytes changed")
        return
    # Everything the 헬모드 tool (and, inside it, the 공격속도 tool) guarantees, on the table as
    # it stands before OUR append.
    HRL.verify(before, ASL._build(a[:-1]))
    if b and b[-1] == HRL.raw_label():
        added = 1
    elif b and b[-1] == ASL.raw_label():
        added = 2
    else:
        added = 3
    if len(a) != len(b) + added:
        raise SystemExit(f"InGame_txt: wanted +{added} row(s), {len(b)} -> {len(a)}")
    if a[-1] != raw_label():
        raise SystemExit(f"InGame_txt: last row {a[-1]!r} != {raw_label()!r}")
    if a[:-1] != ASL._parse(HRL.build(before)):
        raise SystemExit("InGame_txt: a row below the 5초당 HP회복 label is not the 헬모드 tool's")
    why = regen_label_agrees(after)
    if why:
        raise SystemExit(f"InGame_txt: {why}")
    row, stat = rust_consts()
    print(f"VERIFY OK: InGame_txt +1 `{LABEL}` idx={len(a) - 1} => stat id {STAT:#04x} "
          f"(acc_options.rs REGEN_LABEL_ROW {row} · REGEN_STAT {stat:#04x})")


HANDLERS = {"InGame_txt": sys.modules[__name__]}
