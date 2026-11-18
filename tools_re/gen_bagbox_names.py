#!/usr/bin/env python3
"""Emit the names/effects of the BAG-BOX categories as a Rust table.

The host's 오브가방·재료가방·소비가방 windows list stacks the host stores as `(cat, idx,
count)` only — a box never holds a bag record, so there is no baked name to read back
(`BAG_TYPES_FEASIBILITY.md §3.4`). The name and the one-line effect come from the game's
own item DB (`dat/i*_dat`), decoded by `dump_item_db.py`'s record model, and baked here
so the window needs no guest read at all.

    python tools_re/gen_bagbox_names.py            # writes wie_android/src/bagbox_names.rs
    python tools_re/gen_bagbox_names.py --check    # fail (exit 1) if the table is stale

Run it AFTER the asset chain (`patch_asset.py` renames rows in cat 0x12 and adds the
tower material to cat 0x0e), and re-run `--check` before a release build.

121차 (2026-09-21): the same file also carries `FORGE_BASES` — the box-category items a
forge recipe consumes as its BASE row (`smith_dat` / `smithh_dat` / the `i15_dat` tails),
so a crafting window can have them pushed back into the bag alongside the 재료가방.

125차 (2026-09-22): and `RECIPE_NEEDS` / `SMITH_NEEDS` — per 조합서 (`i15_dat` record = the
recipe's item idx) and per smith (`smith_dat` + `smithh_dat`, five rows each), the
box-category items that recipe / those rows consume (mat1, mat2, base). The window edge
pushes the box back into the bag "as much as fits" (`bagbox::forge_plan`), and these
tables are what lets it put the stacks THIS visit can use ahead of the rest — a recipe's
materials are typically the small stacks that a biggest-first order pushed off the end.
"""

from __future__ import annotations

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import dump_item_db as db  # noqa: E402
import h3crypt  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "wie_android", "src", "bagbox_names.rs")

# The box categories, in the order the boxes are numbered (orb · material · consumable).
CATS = [0x10, 0x0E, 0x0F, 0x12, 0x0D]

# The forge's recipe tables. `smith_dat` / `smithh_dat` are 80 x 9 B (the 일반조합 list —
# five rows per smith, normal / hard), and every `i15_dat` record (a 조합서, the 대장간's
# recipe) ends in the same 9 B. One record shape, confirmed against the have/need
# painter `0x21688` (loader of the `%s = %c%d / %d` literal `0x5d02c`):
#
#     [base cat][base idx][mat1 idx][n1][mat2 idx][n2][rate][out cat][out idx]
#
#   row 1 = countOf(0x0e, rec[2]) / rec[3]      row 2 = countOf(0x0e, rec[4]) / rec[5]
#   row 3 = countOf(rec[0], rec[1]) / 1         (0xff = no such row)
#
# The two material rows are cat 0x0e BY CONSTRUCTION, so the 재료가방 push-back already
# covers them. The BASE row's category is whatever the table says — equipment for most
# recipes (never boxed), but 포도주 (0x12, 0x05) for 로얄윈터하츠 (smith record 4, 케이's
# quest) and 바람피리 (0x0e, 0x09) for 그리폰의피리 (i15 record 18). A base that lives in a
# box is invisible to the forge unless the window edge materialises it too.
FORGE_TABLES = ["res/dat/smith_dat", "res/dat/smithh_dat", "res/dat/i15_dat"]
FORGE_REC = 9


def rows(zip_path):
    dats = db.load_dats(zip_path, None)
    out = []
    for cat in CATS:
        blob = dats.get(cat)
        if blob is None:
            raise SystemExit("i%d_dat missing from %s" % (cat, zip_path))
        if cat in db.ENCRYPTED:
            # i15 (조합서) is MX-DES like the drop tables; its plaintext carries cipher
            # padding after the last record, so only the parse START has to be clean.
            blob = h3crypt.decrypt_member(h3crypt.open_assets(zip_path), "res/dat/i%d_dat" % cat)
            decoded, end = db.decode(cat, blob)
            if not decoded:
                raise SystemExit("i%d_dat: nothing parsed after decrypt" % cat)
        else:
            decoded, end = db.decode(cat, blob)
            if end != len(blob):
                raise SystemExit("i%d_dat: parsed %d of %d bytes" % (cat, end, len(blob)))
        for r in decoded:
            effect = r["effect"].split(";")[0].strip()
            # The 4-byte LE price. Its LOW BYTE is what the client's `canDeposit`
            # (`0x6d70`) reads for every box category, so the host's own 올리기 can
            # apply the same refusal (96차, `bagbox::deposit_refusal`).
            price = int.from_bytes(bytes.fromhex(r["price"]), "little")
            out.append((cat, r["idx"], r["marker"], r["name"], effect, price))
    return out


def forge_bases(zip_path):
    """Sorted `(cat, idx)` of every recipe BASE that lives in a bag box."""
    jar = h3crypt.open_assets(zip_path)
    out = set()
    for name in FORGE_TABLES:
        for i, r in enumerate(h3crypt.records(h3crypt.decrypt_member(jar, name))):
            t = r[-FORGE_REC:]
            if len(t) != FORGE_REC:
                raise SystemExit("%s: record %d is %d B, not >= %d" % (name, i, len(r), FORGE_REC))
            cat, idx = t[0], t[1]
            if cat != 0xFF and cat in CATS:
                out.add((cat, idx))
    return sorted(out)


# `smith_dat` / `smithh_dat` are 80 records = 16 smiths x 5 rows: the smith window's
# installer `0x1feb8(mode)` has `0x1fdc0` copy records `(mode-1)*5 .. +5` into the 일반조합
# list (`mode` = the byte `CHAR+0xac90` the NPC script leaves for window 13; 0 reads the
# same rows as 1, past 16 reads none).
SMITHS = 16
SMITH_ROWS = 5


def needs_of(tail):
    """The box-category `(cat, idx)` a recipe tail consumes: mat1, mat2, then the base.
    `0xff` = no such row; an equipment base is never boxed and is left out; duplicates
    fold to the first mention."""
    out = []
    for cat, idx in ((0x0E, tail[2]), (0x0E, tail[4]), (tail[0], tail[1])):
        if cat != 0xFF and idx != 0xFF and cat in CATS and (cat, idx) not in out:
            out.append((cat, idx))
    return out


def forge_needs(zip_path):
    """`(recipe_needs, smith_needs)`: `[(recipe idx, [needs])]` for every `i15_dat` record
    and `[[needs]]` for each of the 16 smiths (normal and hard rows folded together — the
    game picks the table by the difficulty byte, and a union costs at most a stack)."""
    jar = h3crypt.open_assets(zip_path)
    recipes = []
    for i, r in enumerate(h3crypt.records(h3crypt.decrypt_member(jar, "res/dat/i15_dat"))):
        t = r[-FORGE_REC:]
        if len(t) != FORGE_REC:
            raise SystemExit("i15_dat: record %d is %d B, not >= %d" % (i, len(r), FORGE_REC))
        recipes.append((i, needs_of(t)))
    smiths = [[] for _ in range(SMITHS)]
    for name in ["res/dat/smith_dat", "res/dat/smithh_dat"]:
        recs = h3crypt.records(h3crypt.decrypt_member(jar, name))
        if len(recs) != SMITHS * SMITH_ROWS:
            raise SystemExit("%s: %d records, expected %d" % (name, len(recs), SMITHS * SMITH_ROWS))
        for i, r in enumerate(recs):
            for p in needs_of(r[-FORGE_REC:]):
                if p not in smiths[i // SMITH_ROWS]:
                    smiths[i // SMITH_ROWS].append(p)
    return recipes, smiths


def rs_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render(table, bases, recipe_needs, smith_needs):
    lines = [
        "// @generated by tools_re/gen_bagbox_names.py — DO NOT EDIT BY HAND.",
        "//",
        "// Names and one-line effects of every item in the BAG-BOX categories (오브 0x10 ·",
        "// 재료 0x0e/0x0f · 소비 0x12/0x0d), decoded from the shipped `dat/i*_dat` with",
        "// `dump_item_db.py`'s record model. The host's box windows list stacks stored as",
        "// `(cat, idx, count)` only, so this is where their text comes from — no guest read.",
        "// The colour marker (`}` gold, `@` white, …) is kept apart from the name so a",
        "// window can tint the row the way the game tints the bag cell. The price is the",
        "// DB's 4-byte field as a number: the client's `canDeposit` (`0x6d70`) refuses to",
        "// deposit an item whose LOW price byte is 0, and the host's 올리기 does the same.",
        "//",
        "// Plain `//`, never `//!`: this file is `include!`d by `bagbox_selftest.rs`.",
        "// Every table carries `#[rustfmt::skip]`: `--check` compares BYTES, and a repo-wide",
        "// rustfmt (c5d0971) once reflowed this file and left that gate red for good.",
        "",
        "/// `(cat, idx, colour marker, name, first effect line, price)`.",
        "#[rustfmt::skip]",
        "pub const NAMES: &[(u16, u16, &str, &str, &str, u32)] = &[",
    ]
    for cat, idx, mark, name, effect, price in table:
        lines.append(f"    (0x{cat:02x}, 0x{idx:02x}, {rs_str(mark)}, {rs_str(name)}, {rs_str(effect)}, {price}),")
    lines.append("];")
    lines.append("")
    lines += [
        "/// The box-category items a FORGE RECIPE consumes as its base row (121차): every",
        "/// `smith_dat` / `smithh_dat` record and every `i15_dat` (조합서) tail is",
        "/// `[base cat][base idx][mat1 idx][n1][mat2 idx][n2][rate][out cat][out idx]`, and",
        "/// the forge's have/need painter (`0x21688`) counts the base with",
        "/// `countOf(rec[0], rec[1])` against a need of 1. The two material rows are cat 0x0e",
        "/// by construction; the base is whatever the table says. Only the bases that live in",
        "/// a bag box are listed — an equipment base is in the bag already.",
        "#[allow(dead_code)]",
        "#[rustfmt::skip]",
        "pub const FORGE_BASES: &[(u16, u16)] = &[",
    ]
    for cat, idx in bases:
        lines.append(f"    (0x{cat:02x}, 0x{idx:02x}),")
    lines.append("];")
    lines.append("")
    lines += [
        "/// Per 조합서 (125차): `(recipe idx, the box-category items its tail consumes — mat1,",
        "/// mat2, base)`. The recipe idx is the `i15_dat` record number = the cat-0x0f item's",
        "/// idx, so a recipe stack in the 재료가방 finds its needs by `(0x0f, idx)`. Equipment",
        "/// bases are left out (never boxed); `0xff` rows are skipped.",
        "#[allow(dead_code)]",
        "#[rustfmt::skip]",
        "pub const RECIPE_NEEDS: &[(u16, &[(u16, u16)])] = &[",
    ]
    for idx, needs in recipe_needs:
        body = ", ".join(f"(0x{c:02x}, 0x{i:02x})" for c, i in needs)
        lines.append(f"    (0x{idx:02x}, &[{body}]),")
    lines.append("];")
    lines.append("")
    lines += [
        "/// Per smith (125차): the box-category items the smith's five 일반조합 rows consume",
        "/// (`smith_dat` and `smithh_dat` folded together), indexed by `smith id - 1` — the id is",
        "/// the byte `CHAR+0xac90` window 13's installer `0x1feb8(mode)` receives (`0x1fdc0` copies",
        "/// rows `(mode-1)*5 .. +5`; 0 reads the same rows as 1).",
        "#[allow(dead_code)]",
        "#[rustfmt::skip]",
        "pub const SMITH_NEEDS: &[&[(u16, u16)]] = &[",
    ]
    for needs in smith_needs:
        body = ", ".join(f"(0x{c:02x}, 0x{i:02x})" for c, i in needs)
        lines.append(f"    &[{body}],")
    lines.append("];")
    lines.append("")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=os.path.join(ROOT, db.DEFAULT_ZIP))
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    recipe_needs, smith_needs = forge_needs(a.zip)
    text = render(rows(a.zip), forge_bases(a.zip), recipe_needs, smith_needs)
    if a.check:
        cur = open(OUT, encoding="utf-8").read() if os.path.exists(OUT) else ""
        if cur != text:
            print("STALE: %s does not match the asset — rerun without --check" % OUT)
            return 1
        print("OK: %s is current" % OUT)
        return 0
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print("wrote %s (%d rows)" % (OUT, text.count("\n    (")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
