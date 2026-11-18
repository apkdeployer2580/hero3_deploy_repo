"""Regenerate `vendor/wie/wie_util/src/acc_options.rs` — the reworked accessories' THIRD option.

150차 (2026-10-01). The decision table is `tools_re/acc_options.py` (all three options of the
11 accessories); the asset rows carry the first two, and this tool carries the third to the
two crates that need it at compile time:

  * the guest — `acc_third_stat_lgt` feeds a `Stat` third to the game's own `addStat` inside
    the wear recompute, and `acc_summary_lgt` writes `summary_euc` over the bag summary
    box's first line ("레벨1 장신구/저항0");
  * the host — the effects (`stats::AccEffects`, the 공격속도 sum) and the 상세보기 text.

(cat, idx) is resolved BY NAME in the shipped asset's `i12_dat` (the colour escape stripped),
so a table can never point at a row that moved. `--check` also proves the asset rows carry
exactly the decisions: 요구 Lv 1 (b6), 성능 0 (b10..11), the two option pairs (b14..17), the
tier byte untouched (b5: `"` 5 · `{` 4), and that `InGame_txt` row 200 = 80 + 0x78 names
레아's 「5초당 HP회복 +」 in agreement with the generated constants (`add_regen_label`) —
run it AFTER the asset chain (`patch_asset.py --apply` …).

Usage:
    python tools_re/gen_acc_options.py [--zip <asset.zip>] [--check]
"""
import argparse
import io
import os
import struct
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import acc_options as AO  # noqa: E402
import add_regen_label as RGL  # noqa: E402
import dump_item_db as DB  # noqa: E402

DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
OUT = os.path.join("vendor", "wie", "wie_util", "src", "acc_options.rs")
ACC_CAT = 12
TIER_BYTE = {'"': 5, "{": 4}   # b5 — `add_legacy_items.TIER` / `add_boss_accessories.TIER`


def open_jar(zip_path):
    z = zipfile.ZipFile(zip_path)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit(f"expected exactly one jar in {zip_path}")
    return zipfile.ZipFile(io.BytesIO(z.read(jars[0])))


def accessory_rows(jar):
    """{name (escape stripped): (idx, colour escape, stats bytes)} for cat 0x0c."""
    blob = None
    for n in jar.namelist():
        if os.path.basename(n) == "i12_dat":
            blob = jar.read(n)
    if blob is None:
        raise SystemExit("i12_dat not found in the asset jar")
    out = {}
    for idx, _off, body in DB.records(blob):
        nl = body[0]
        raw, stats = body[1:1 + nl], body[1 + nl:]
        if not raw:
            continue
        colour = chr(raw[0]) if raw[0] in (0x22, 0x7B) else ""
        name = (raw[1:] if colour else raw).decode("euc-kr", "replace")
        out.setdefault(name, []).append((idx, colour, bytes(stats)))
    return out


def resolve(rows):
    """[(idx, entry)] sorted by idx; raises when a name is missing, ambiguous or re-coloured."""
    got = []
    for a in AO.ACC:
        name, colour = a[0], a[1]
        hits = rows.get(name, [])
        if len(hits) != 1:
            raise SystemExit(f"{name}: {len(hits)} row(s) of that name in i12_dat (need exactly 1)")
        idx, have_colour, _stats = hits[0]
        if have_colour != colour:
            raise SystemExit(f"{name}: the asset row is coloured {have_colour!r}, the table says {colour!r}")
        got.append((idx, a))
    got.sort(key=lambda t: t[0])
    return got


def row_problems(rows):
    """What the asset rows get wrong against the decisions (empty = they carry them).

    The 18-byte accessory stats (the record body after the name): b0..3 원가 · b4 sprite ·
    b5 tier · b6 요구 Lv · b7 · b8 · b9 · b10..11 성능 (u16) · b12..13 보조 · b14..17 the two
    `(statId, i8)` option pairs."""
    problems = []
    for a in AO.ACC:
        name, o1, o2 = a[0], a[2], a[3]
        idx, col, st = rows[name][0]
        if len(st) != 18:
            problems.append(f"{name} (0x{idx:02x}): stats are {len(st)} bytes, not 18")
            continue
        if st[5] != TIER_BYTE.get(col):
            problems.append(f"{name} (0x{idx:02x}): tier byte {st[5]}, want {TIER_BYTE.get(col)} ({col} row)")
        req = st[6]
        perf = struct.unpack_from("<H", st, 10)[0]
        p1 = (st[14], struct.unpack_from("b", st, 15)[0])
        p2 = (st[16], struct.unpack_from("b", st, 17)[0])
        if req != AO.REQ_LV:
            problems.append(f"{name} (0x{idx:02x}): 요구 Lv {req}, want {AO.REQ_LV}")
        if perf != AO.PERF:
            problems.append(f"{name} (0x{idx:02x}): 성능 {perf}, want {AO.PERF}")
        if p1 != tuple(o1):
            problems.append(f"{name} (0x{idx:02x}): option 1 {p1}, want {tuple(o1)}")
        if p2 != tuple(o2):
            problems.append(f"{name} (0x{idx:02x}): option 2 {p2}, want {tuple(o2)}")
    return problems


def label_problems(jar):
    """InGame_txt row 200 (= 80 + `acc_options.REGEN`) must say 「5초당 HP회복 +」 and agree with the
    generated `REGEN_LABEL_ROW` / `REGEN_STAT` — `add_regen_label.regen_label_agrees`."""
    names = [n for n in jar.namelist() if n.rsplit("/", 1)[-1] == "InGame_txt"]
    if len(names) != 1:
        return [f"InGame_txt: {len(names)} member(s) of that name in the jar"]
    why = RGL.regen_label_agrees(jar.read(names[0]))
    if not why:
        return []
    return [why if why.startswith("InGame_txt") else f"InGame_txt: {why}"]


def euc_literal(text):
    return 'b"' + "".join(f"\\x{b:02x}" for b in text.encode("euc-kr")) + '"'


def third_expr(kind, sid):
    return f"Third::Stat(0x{sid:02x})" if kind == AO.STAT else f"Third::{kind}"


def render(resolved):
    L = [
        "//! GENERATED by `python tools_re/gen_acc_options.py` — DO NOT EDIT BY HAND.",
        "//!",
        "//! 150차 (2026-10-01): the THIRD option of the 11 reworked accessories (the nine `\"`",
        "//! legacy rows and the two `{` boss rows of cat 0x0c). The asset rows carry only two",
        "//! `(statId, i8)` pairs — the parser reads exactly two, so a third cannot ride in the",
        "//! row — and `tools_re/acc_options.py` is the one source of truth for all three.",
        "//!",
        "//! `Stat` thirds are fed to the game's own `addStat 0x4ee34` inside the wear recompute",
        "//! (`wie_core_arm` `acc_third_stat_lgt`), so every recompute — wear, preview, load —",
        "//! sees them; the others are host/hook effects ([`crate::stats::AccEffects`], the",
        "//! 공격속도 sum). `summary_euc` is what the bag summary box's first line shows",
        "//! (`acc_summary_lgt`), ≤ 134 px in the game font; `detail` is the 상세보기 line.",
        "",
        "/// What a third option does.",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub enum Third {",
        "    /// An ordinary game stat: an `addStat` id (0x02, 0x03, 0x05..=0x12) and its value.",
        "    Stat(u8),",
        "    /// 공격속도 +N — the host stat 0x76, summed with the worn option pairs.",
        "    AtkSpeed,",
        "    /// 받는 피해 -N % ([`crate::stats::AccEffects::taken_pct`]).",
        "    Taken,",
        "    /// 주는 피해 +N % ([`crate::stats::AccEffects::dealt_pct`]).",
        "    Dealt,",
        "    /// 적 처치 시 20초간 최대 HP +N % ([`crate::stats::AccEffects::kill_max_hp_pct`]).",
        "    KillMaxHp,",
        "    /// 피격 시 20초간 최대 HP +N % ([`crate::stats::AccEffects::hit_max_hp_pct`]).",
        "    HitMaxHp,",
        "}",
        "",
        "/// One accessory's third option.",
        "pub struct AccThird {",
        "    pub cat: u8,",
        "    pub idx: u8,",
        "    /// The row's name without its colour escape (logs).",
        "    pub name: &'static str,",
        "    pub third: Third,",
        "    pub value: i8,",
        "    /// The bag summary box's first line (UTF-8).",
        "    pub summary: &'static str,",
        "    /// The same in EUC-KR, no NUL — what the summary hook writes.",
        "    pub summary_euc: &'static [u8],",
        "    /// The 상세보기 line (may be wider than the summary box; the host wraps it).",
        "    pub detail: &'static str,",
        "}",
        "",
        f"pub const ACC_THIRDS: [AccThird; {len(resolved)}] = [",
    ]
    for idx, (name, _colour, _o1, _o2, (kind, sid, value), summary, detail) in resolved:
        L.append("    AccThird {")
        L.append(f"        cat: 0x{ACC_CAT:02x},")
        L.append(f"        idx: 0x{idx:02x},")
        L.append(f'        name: "{name}",')
        L.append(f"        third: {third_expr(kind, sid)},")
        L.append(f"        value: {value},")
        L.append(f'        summary: "{summary}",')
        L.append(f"        summary_euc: {euc_literal(summary)},")
        L.append(f'        detail: "{detail}",')
        L.append("    },")
    L += [
        "];",
        "",
        "/// The third option of `(cat, idx)`, if that item has one.",
        "pub fn third_of(cat: u16, idx: u16) -> Option<&'static AccThird> {",
        "    ACC_THIRDS.iter().find(|t| u16::from(t.cat) == cat && u16::from(t.idx) == idx)",
        "}",
        "",
        "/// ★ 150차 — 레아의목걸이's option 2 「5초당 HP회복 +N%」: a stat id `addStat` ignores",
        "/// (`id - 2 > 0x15` returns), so the game itself does nothing with it; the host heals",
        "/// N % of the true max HP every [`REGEN_PERIOD_MS`] of play clock in the field. The",
        "/// detail window names it `InGame_txt[80 + id]` = row [`REGEN_LABEL_ROW`]",
        "/// (`tools_re/add_regen_label.py`), and the game's `%` rule is widened to it.",
        f"pub const REGEN_STAT: u8 = 0x{AO.REGEN:02x};",
        f"pub const REGEN_LABEL_ROW: u16 = {AO.REGEN_LABEL_ROW};",
        f"pub const REGEN_PERIOD_MS: u64 = {AO.REGEN_PERIOD_MS};",
        "",
        "/// The ids a `Stat` third may carry — `addStat`'s additive arms (0x04 is a no-op,",
        "/// 0x13.. are mode switches). The hook refuses anything else.",
        "pub const fn guest_stat(id: u8) -> bool {",
        "    matches!(id, 0x02 | 0x03 | 0x05..=0x12)",
        "}",
    ]
    return "\n".join(L) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    jar = open_jar(a.zip)
    rows = accessory_rows(jar)
    resolved = resolve(rows)
    text = render(resolved)
    problems = row_problems(rows) + label_problems(jar)
    if a.check:
        try:
            have = open(OUT, encoding="utf-8", newline="").read()
        except FileNotFoundError:
            raise SystemExit(f"STALE: {OUT} does not exist — run this tool without --check")
        if have.replace("\r\n", "\n") != text:
            raise SystemExit(f"STALE: {OUT} does not match tools_re/acc_options.py + {a.zip} — rerun this tool")
        if problems:
            raise SystemExit("ASSET ROWS DISAGREE with tools_re/acc_options.py (run the asset chain):\n  "
                             + "\n  ".join(problems))
        print(f"CHECK OK: {OUT} matches ({len(resolved)} accessories, idx "
              f"{', '.join(f'0x{i:02x}' for i, _ in resolved)}); the asset rows carry the decisions "
              f"(요구Lv {AO.REQ_LV} · 성능 {AO.PERF} · two pairs · tier byte) and InGame_txt row "
              f"{AO.REGEN_LABEL_ROW} names option 0x{AO.REGEN:02x}")
        return
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {len(resolved)} accessories")
    if problems:
        print("⚠ the asset rows do not carry the decisions yet (run the asset chain, then --check):\n  "
              + "\n  ".join(problems))


if __name__ == "__main__":
    main()
