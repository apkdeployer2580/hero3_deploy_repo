"""Regenerate the hell-drop table the guest hook rolls from.

The 133차 hell mode (difficulty byte 2, opened by clearing hardcore's ending) drops
the `"` legacy rows that `add_legacy_items.py` appends to i0..i12. The drop side is
one extra roll inside the `reaver_drop_lgt` hook (`wie_core_arm::binary_patches::
helldrop`), and a hook cannot read the item DB — it needs the `(cat, idx, 등급)`
triples at compile time. This tool derives them FROM THE SHIPPED ASSET, so the table
can never drift from the rows the player actually sees: a row is in the pool exactly
when its name starts with the `"` colour escape (0x22), the one colour the stock data
never uses and `add_legacy_items` always does.

★ 150차 (2026-10-01): the third value is the TIER (85제 / 90제, `gen_legacy_items.
legacy_tiers`), no longer the asset's 요구레벨 byte — the reworked accessories carry 요구 Lv 1
(`acc_options.REQ_LV`) while staying 85/90제 pieces, and the two pools, the tower rewards and
the set table all mean the tier. It is CROSS-CHECKED against the asset (equipment b8 == the
tier — the equipment 요구레벨 IS the tier; accessory b6 == 1), so a table and an asset that
disagree about a row fail here. For every row that shipped before 150차 the value is the
same number it was, so `HELL_ITEMS` did not change.

134차 added two more things the hook needs and cannot see:

  * `HELL_NAMED_SPECIES` — the species `enemyh_dat` names in red (`{` colour escape),
    hell's named bosses below `boss_dat` (161+, named by number in the hook). The user
    decision «93~100 네임드보스 급 들에서만 드랍» needs this set at compile time.
  * a cross-check of the ladder constants: the hard table's spawn-level range (the
    ladder's INPUT — `guardian::HELL_SPAWN_LV_MIN/MAX`) against the asset, the record
    count against `helldrop::BOSS_DAT_BASE`, and `HELL_LV_MIN/MAX` equal in both crates
    (the host computes the level, the hook gates on it; they must agree).

152차 (2026-10-02) added the two other things a hell kill now drops (user decision: Lv 79..85
rolls «신규아이템의 모든 조합서», Lv 79+ rolls one 닉스코어파편, each at 1 %):

  * `HELL_RECIPES` — every `i15_dat` 조합서 whose tail MAKES a `HELL_ITEMS` row, `(0x0f, idx,
    the result's 등급)`, in record order. It is the same list `gen_guardian_rewards.py` pays
    from (that tool calls `recipe_rows` here), and it must be a BIJECTION onto the legacy rows:
    one recipe per new item, none missing — `--check` refuses anything else.
  * `HELL_SHARD` — the tower material's `(cat, idx)`, looked up by name (`material_idx`).

Usage:
    python tools_re/gen_helldrop_table.py [--zip <asset.zip>] [--check]

`--check` exits 1 when the checked-in Rust table does not match the asset or one of
the constants disagrees — same contract as `gen_cash_catalog.py` / `gen_bagbox_names.py`.
"""
import argparse
import collections
import io
import os
import re
import struct
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import acc_options as AO  # noqa: E402
import dump_item_db as DB  # noqa: E402
import gen_legacy_items as GL  # noqa: E402
from h3crypt import decrypt_member, records  # noqa: E402

DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
OUT = os.path.join("vendor", "wie", "wie_core_arm", "src", "binary_patches", "helldrop_table.rs")
HOOK_RS = os.path.join("vendor", "wie", "wie_core_arm", "src", "binary_patches", "helldrop.rs")
HOST_RS = os.path.join("wie_android", "src", "guardian.rs")
NEW_MARK = 0x22   # `"`
NAMED_MARK = 0x7B  # `{` — the game's red-name escape on its elite species
ACC_CAT = 12       # 18-byte records: 요구레벨 at b6, not b8
TIERS = (GL.RW_CONCEPT, GL.RW_FINAL)   # 85제 · 90제 — the only values the third column holds
ENEMY_HARD = "res/dat/enemyh_dat"
RECIPE_CAT = 0x0F        # 조합서 (`i15_dat`, DES-encrypted)
RECIPE_TAIL = 9          # [base cat][base idx][mat1 idx][n1][mat2 idx][n2][rate][out cat][out idx]
MATERIAL_CAT = 0x0E      # 재료
MATERIAL_NAME = "닉스코어파편"   # 151차 name — `add_tower_material.NAME`


def open_jar(zip_path):
    z = zipfile.ZipFile(zip_path)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit(f"expected exactly one jar in {zip_path}")
    return zipfile.ZipFile(io.BytesIO(z.read(jars[0])))


def legacy_rows(jar):
    """`[(cat, idx, tier, name, main)]` for every `"` row, in (cat, idx) order — `tier` is the
    85/90제 of `gen_legacy_items.legacy_tiers`, cross-checked against the row's own bytes
    (equipment 요구레벨 b8 == tier · accessory 요구레벨 b6 == `acc_options.REQ_LV`). Refuses a
    `"` row the tier table does not know, and a table row the asset does not carry."""
    dats = {}
    for n in jar.namelist():
        b = os.path.basename(n)
        if b.startswith("i") and b.endswith("_dat") and b[1:-4].isdigit():
            dats[int(b[1:-4])] = jar.read(n)
    tiers = GL.legacy_tiers()
    out, seen = [], set()
    for cat in range(0, 13):
        for idx, _off, body in DB.records(dats[cat]):
            nl = body[0]
            raw, stats = body[1:1 + nl], body[1 + nl:]
            if not raw or raw[0] != NEW_MARK:
                continue
            name = raw[1:].decode("euc-kr", "replace")
            tier = tiers.get((cat, name))
            if tier is None:
                raise SystemExit(f"i{cat}_dat 0x{idx:02x} `\"{name}` is not a legacy row gen_legacy_items knows")
            if tier not in TIERS:
                raise SystemExit(f"{name}: tier {tier} is not one of {TIERS}")
            req = stats[6] if cat == ACC_CAT else stats[8]
            want_req = AO.REQ_LV if cat == ACC_CAT else tier
            if req != want_req:
                raise SystemExit(
                    f"i{cat}_dat 0x{idx:02x} {name}: 요구레벨 byte {req}, a {tier}제 "
                    f"{'accessory' if cat == ACC_CAT else 'piece'} must carry {want_req} — "
                    "run the asset chain (patch_asset.py --apply …) or fix the tables")
            main = struct.unpack_from("<H", stats, 10 if cat == ACC_CAT else 12)[0]
            out.append((cat, idx, tier, name, main))
            seen.add((cat, name))
    missing = sorted(set(tiers) - seen)
    if out and missing:
        raise SystemExit(f"{len(missing)} legacy row(s) of the tier table are not in the asset: {missing[:5]}")
    return out


def recipe_results(jar, legacy):
    """`[(idx, (out cat, out idx), result tier, result name)]` for every 조합서 whose tail makes a
    row of `legacy` (any rows whose first four fields are `(cat, idx, tier, name)`), in record
    order."""
    by_item = {(row[0], row[1]): (row[2], row[3]) for row in legacy}
    out = []
    for i, r in enumerate(records(decrypt_member(jar, f"res/dat/i{RECIPE_CAT}_dat"))):
        if len(r) < RECIPE_TAIL:
            raise SystemExit(f"i{RECIPE_CAT}_dat: record {i} is {len(r)} B, not >= {RECIPE_TAIL}")
        t = r[-RECIPE_TAIL:]
        made = by_item.get((t[7], t[8]))
        if made is not None:
            out.append((i, (t[7], t[8]), made[0], made[1]))
    return out


def recipe_rows(jar, legacy):
    """`[(0x0f, idx, result tier, result name)]` — the one definition both the hell drop
    (`HELL_RECIPES`) and the tower's exit reward (`gen_guardian_rewards.RECIPES`) use."""
    return [(RECIPE_CAT, i, tier, name) for i, _made, tier, name in recipe_results(jar, legacy)]


def material_idx(jar):
    """The 닉스코어파편 row's idx in `i14_dat`, found by name (a drifted row count fails here)."""
    for idx, _off, body in DB.records(jar.read(f"res/dat/i{MATERIAL_CAT}_dat")):
        nl = body[0]
        if DB.euc(body[1:1 + nl]).lstrip("@|${}'\"") == MATERIAL_NAME:
            return idx
    raise SystemExit(
        f"i{MATERIAL_CAT}_dat has no `{MATERIAL_NAME}` row — run "
        "tools_re/patch_asset.py --apply first (add_tower_material appends it)")


def check_recipes(rows, results):
    """152차: the hell recipe pool must be exactly one 조합서 per new item — a bijection."""
    problems = []
    made = collections.Counter(out for _i, out, _t, _n in results)
    dup = sorted(k for k, n in made.items() if n > 1)
    if dup:
        problems.append(f"legacy rows made by more than one 조합서: {[f'{c:#x}/{i:#x}' for c, i in dup[:4]]}")
    missing = sorted((cat, idx) for cat, idx, *_ in rows if (cat, idx) not in made)
    if missing:
        problems.append(f"{len(missing)} legacy row(s) have no 조합서: {[f'{c:#x}/{i:#x}' for c, i in missing[:5]]}")
    return problems


def hard_species(jar):
    """`[(id, name, level, named)]` off the HARD table — the one hell spawns from."""
    out = []
    for i, r in enumerate(records(jar.read(ENEMY_HARD))):
        nl = r[0]
        raw = r[1:1 + nl]
        named = bool(raw) and raw[0] == NAMED_MARK
        name = (raw[1:] if named else raw).decode("euc-kr", "replace").rstrip("@")
        out.append((i, name, r[1 + nl], named))
    return out


def render(rows, species, recipes, shard) -> str:
    named = [(i, n) for i, n, _lv, is_named in species if is_named]
    lines = [
        "//! GENERATED by `python tools_re/gen_helldrop_table.py` — DO NOT EDIT BY HAND.",
        "//!",
        "//! Every `\\\"`-coloured legacy row in the shipped asset (`add_legacy_items.py`),",
        "//! as `(cat, idx, 등급)` — the pool the hell-mode kill roll picks from",
        "//! (`super::helldrop`). 등급 = the 85제 / 90제 tier (`gen_legacy_items.legacy_tiers`,",
        "//! cross-checked against the asset); since 150차 an accessory's 요구레벨 is 1, so the",
        "//! comment shows both. Regenerate after any asset change; `--check` gates it.",
        "",
        f"pub const HELL_ITEMS: [(u16, u16, u8); {len(rows)}] = [",
    ]
    for cat, idx, tier, name, main in rows:
        req = AO.REQ_LV if cat == ACC_CAT else tier
        lines.append(f"    (0x{cat:02x}, 0x{idx:02x}, {tier}), // {name} ({tier}제 · Lv{req}, {main})")
    lines.append("];")
    lines += [
        "",
        "/// The species the game itself names in red (`{` colour escape in `res/dat/enemyh_dat`,",
        "/// the table hell spawns from) — hell's named bosses below `boss_dat`. Sorted by id.",
        f"pub const HELL_NAMED_SPECIES: [u8; {len(named)}] = [",
    ]
    for i, n in named:
        lines.append(f"    {i}, // {n}")
    lines.append("];")
    lines += [
        "",
        "/// 152차: every 조합서 (`i15_dat`, cat 0x0f) whose tail MAKES a `HELL_ITEMS` row, as `(cat, idx,",
        "/// 등급 of the result)` in record order — one per new item (`--check` asserts the bijection).",
        "/// The same list the tower's exit reward pays from (`gen_guardian_rewards.RECIPES`). A hell",
        "/// kill at live Lv `RECIPE_LV_MIN..=RECIPE_LV_MAX` rolls one of these, uniformly.",
        f"pub const HELL_RECIPES: [(u16, u16, u8); {len(recipes)}] = [",
    ]
    for cat, idx, tier, name in recipes:
        lines.append(f"    (0x{cat:02x}, 0x{idx:02x}, {tier}), // {name} 조합서 ({tier}제)")
    lines.append("];")
    lines += [
        "",
        f"/// 152차: {MATERIAL_NAME} (`add_tower_material.py`, `i{MATERIAL_CAT}_dat` idx 0x{shard:02x}) — a hell",
        "/// kill at live Lv `SHARD_LV_MIN`+ rolls one.",
        f"pub const HELL_SHARD: (u16, u16) = (0x{MATERIAL_CAT:02x}, 0x{shard:02x});",
    ]
    return "\n".join(lines) + "\n"


def const_in(path, name, ty):
    text = open(path, encoding="utf-8").read()
    m = re.search(rf"pub const {re.escape(name)}: {ty} = (-?\d+);", text)
    if not m:
        raise SystemExit(f"{path}: `pub const {name}: {ty} = …;` not found")
    return int(m.group(1))


def check_constants(species):
    lv = [s[2] for s in species]
    lo, hi = min(lv), max(lv)
    problems = []
    got = const_in(HOST_RS, "HELL_SPAWN_LV_MIN", "i8")
    if got != lo:
        problems.append(f"guardian::HELL_SPAWN_LV_MIN = {got} but the hard table's lowest spawn is {lo}")
    got = const_in(HOST_RS, "HELL_SPAWN_LV_MAX", "i8")
    if got != hi:
        problems.append(f"guardian::HELL_SPAWN_LV_MAX = {got} but the hard table's highest spawn is {hi}")
    got = const_in(HOOK_RS, "BOSS_DAT_BASE", "u8")
    if got != len(species):
        problems.append(f"helldrop::BOSS_DAT_BASE = {got} but enemyh_dat has {len(species)} species")
    for name in ("HELL_LV_MIN", "HELL_LV_MAX"):
        a, b = const_in(HOST_RS, name, "i8"), const_in(HOOK_RS, name, "i8")
        if a != b:
            problems.append(f"{name}: guardian.rs says {a}, helldrop.rs says {b} — the host computes it, the hook gates on it")
    return problems, lo, hi


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    jar = open_jar(a.zip)
    rows = legacy_rows(jar)
    if not rows:
        raise SystemExit(f"{a.zip}: no `\"` legacy rows — run the asset chain first")
    species = hard_species(jar)
    reqs = sorted({r[2] for r in rows})
    named = [i for i, _n, _lv, is_named in species if is_named]
    results = recipe_results(jar, rows)
    recipes = [(RECIPE_CAT, i, tier, name) for i, _made, tier, name in results]
    shard = material_idx(jar)
    text = render(rows, species, recipes, shard)
    problems, lo, hi = check_constants(species)
    problems += check_recipes(rows, results)
    if a.check:
        try:
            have = open(OUT, encoding="utf-8", newline="").read()
        except FileNotFoundError:
            raise SystemExit(f"STALE: {OUT} does not exist — run this tool without --check")
        if have.replace("\r\n", "\n") != text:
            raise SystemExit(f"STALE: {OUT} does not match {a.zip} — rerun this tool")
        if problems:
            raise SystemExit("DISAGREE:\n  " + "\n  ".join(problems))
        print(f"CHECK OK: {OUT} matches the asset ({len(rows)} rows, 등급 {reqs}; {len(recipes)} 조합서 "
              f"0x{recipes[0][1]:02x}..0x{recipes[-1][1]:02x}; shard 0x{MATERIAL_CAT:02x}/0x{shard:02x}; "
              f"{len(named)} named species {named}; hard spawn levels {lo}..{hi}, {len(species)} species)")
        return
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {len(rows)} rows, 등급 {reqs}; {len(recipes)} 조합서; shard idx 0x{shard:02x}; "
          f"{len(named)} named species {named}; hard spawn levels {lo}..{hi}, {len(species)} species")
    if problems:
        print("⚠ the asset or the Rust constants disagree (fix them, then --check):\n  " + "\n  ".join(problems))
        sys.exit(1)


if __name__ == "__main__":
    main()
