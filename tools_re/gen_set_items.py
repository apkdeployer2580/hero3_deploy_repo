"""Generate `wie_android/src/set_items.rs` — which worn (cat, idx) belongs to which set.

142차 (2026-09-30, `LEGACY_ITEMS_REWORK_85_90.md §3`): the three 85제 sets carry a real set
effect — six pieces each: 투구 · 갑옷 · 장갑 · 신발 · 방패 · 세트 장신구 (the shield slot only
opens for spear wielders, `0x4d936`, so the accessory is the piece everyone else can wear).
The host only sees `(cat, idx)` on the character, so the membership is looked up in the ASSET
the way the hell drop table is: every `"` row, by the name `gen_legacy_items` gives the set's
pieces (`SETS`) and accessory (`RW_SET_ACCESSORY`). 142차-C adds the three 90제 sets; their sixth
piece is an existing 90제 accessory.

★ 150차 (2026-10-01, user decision 3): ANY 4 DISTINCT pieces of the six turn the set's effect
on (was 5 — the count is `setbonus::PIECES_FOR_BONUS`, host code). The six-piece table itself
is unchanged; the set accessories keep their own 150차 options (`acc_options.py` — no more
set-themed option overwrite) and are still set pieces. The level check below compares the
piece's TIER (85/90제 — `gen_helldrop_table.legacy_rows` hands the tier since 150차, because a
reworked accessory's 요구레벨 byte is 1).

★★ 154차 (2026-10-02, user decision): a set has FOUR TO SIX pieces — its five equipment pieces
(투구 · 갑옷 · 장갑 · 신발 · 방패) and at most one accessory (`GL.RW_SET_ACCESSORY`): 「세트가 4개일
수도있고 5개일수도있고 6개일 수도있는거야」 · 「4or5or6 이야 7도아니고 3도아니고」 (`PIECES_MIN` ..
`PIECES_MAX`, and never below `setbonus::PIECES_FOR_BONUS` — a set must be able to turn on).
레아의목걸이 moved from 크루세이더 to 붉은마녀, 굴베이그의탈리스만 left the sets (a plain accessory),
크루세이더 is five pieces. The tier check covers the equipment pieces only: accessories are 요구 Lv 1
since 150차, so the 85제 레아 is a piece of the 90제 붉은마녀.

★★ 155차 (2026-10-02, user decision, `LEGACY_ITEMS_REWORK_85_90.md §13.2`): 템플러 lost 이안의맹세 and
코르버스 lost 닉스의펜던트 (both five pieces now), 빙해's accessory is 이안의맹세 (was 클레르의가보 —
「빙해의 검사」 이안; an 85제 accessory on a 90제 set, the 레아 rule), 클레르의가보 and 닉스의펜던트 are
plain accessories. Three set accessories (레갈리스교단반지 · 레아의목걸이 · 이안의맹세), 33 rows.

    python tools_re/gen_set_items.py [--zip <asset.zip>] [--check]

`--check` exits 1 when the checked-in table does not match the asset.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_helldrop_table as HD    # noqa: E402
import gen_legacy_items as GL      # noqa: E402

OUT = os.path.join("wie_android", "src", "set_items.rs")
# The order is the bit order `setbonus.rs` and `wie_util::stats::SET_*` use: the three 85제
# sets (142차), then the three 90제 sets (142차-C).
SET_ORDER = ["템플러", "크루세이더", "레갈리스근위", "코르버스", "붉은마녀", "빙해"]
# 154차: a set is four, five or six pieces (user decision — never three, never seven).
PIECES_MIN, PIECES_MAX = 4, 6
ACCESSORY_CAT = 0x0C
SETBONUS_RS = os.path.join("wie_android", "src", "setbonus.rs")


def set_spec(sname):
    return next(s for s in GL.SETS if s[1] == sname)


def set_accessory(sname):
    """The set's accessory piece, or None (154차 · 155차: 템플러 · 크루세이더 · 코르버스 have none)."""
    return GL.RW_SET_ACCESSORY.get(sname)


def set_level(sname):
    """The tier (85 / 90제) every EQUIPMENT piece of this set must have (`SETS`' old level)."""
    return GL.rw_tier(set_spec(sname)[0])


def pieces_for_bonus():
    """`setbonus::PIECES_FOR_BONUS` — a set smaller than that could never turn on."""
    import re
    with open(SETBONUS_RS, encoding="utf-8") as f:
        m = re.search(r"pub const PIECES_FOR_BONUS: u8 = (\d+);", f.read())
    if not m:
        raise SystemExit(f"{SETBONUS_RS}: no PIECES_FOR_BONUS")
    return int(m.group(1))


def members():
    """{name: set id} for every piece of every set (four to six each, 154차)."""
    unknown = sorted(set(GL.RW_SET_ACCESSORY) - set(SET_ORDER))
    if unknown:
        raise SystemExit(f"RW_SET_ACCESSORY names no such set: {unknown}")
    out = {}
    for sid, sname in enumerate(SET_ORDER):
        spec = set_spec(sname)
        names = [spec[3][cat] for cat in GL.SET_PARTS]
        acc = set_accessory(sname)
        if acc:
            names.append(acc)
        for name in names:
            if name in out:
                raise SystemExit(f"{name} is a piece of {SET_ORDER[out[name]]} and {sname}")
            out[name] = sid
    return out


def rows(jar):
    want = members()
    got = []
    for cat, idx, tier, name, _main in HD.legacy_rows(jar):
        if name in want:
            got.append((cat, idx, want[name], name, tier))
    missing = sorted(set(want) - {g[3] for g in got})
    if missing:
        raise SystemExit(f"set pieces not in the asset: {missing} — is the chain applied?")
    if len(got) != len(want):
        raise SystemExit(f"{len(got)} set rows in the asset for {len(want)} piece names — a name is on two rows")
    floor = max(PIECES_MIN, pieces_for_bonus())
    for sid, sname in enumerate(SET_ORDER):
        n = sum(1 for g in got if g[2] == sid)
        if not floor <= n <= PIECES_MAX:
            raise SystemExit(f"{sname} has {n} pieces — a set is {floor}..{PIECES_MAX} (154차)")
    for cat, idx, sid, name, tier in got:
        if cat == ACCESSORY_CAT:
            continue  # 154차: an accessory is 요구 Lv 1 — any tier may be any set's piece (레아 → 붉은마녀)
        want_tier = set_level(SET_ORDER[sid])
        if tier != want_tier:
            raise SystemExit(f"{name} is {tier}제, a {SET_ORDER[sid]} piece must be {want_tier}제")
    return sorted(got)


def render(got) -> str:
    lines = [
        "// GENERATED by tools_re/gen_set_items.py — do not edit. `--check` gates it.",
        "// The set pieces: (cat, idx, set) with 85제 (142차) 0 = 템플러 · 1 = 크루세이더 ·",
        "// 2 = 레갈리스근위 and 90제 (142차-C) 3 = 코르버스 · 4 = 붉은마녀 · 5 = 빙해 — the bit",
        "// order of `setbonus::SETS` / `wie_util::stats::SET_*`. Any 4 distinct pieces of a set",
        "// turn its effect on (150차 — was 5; `setbonus::PIECES_FOR_BONUS`). A set is 4..6 pieces",
        "// (154차 · 155차): 템플러 · 크루세이더 · 코르버스 have no accessory (five pieces), 레아의목걸이 is",
        "// 붉은마녀's, 이안의맹세 is 빙해's; 굴베이그의탈리스만 · 클레르의가보 · 닉스의펜던트 are no set piece.",
        "#[rustfmt::skip]",
        "pub const SET_ITEMS: &[(u16, u16, u8)] = &[",
    ]
    for cat, idx, sid, name, _req in got:
        lines.append(f"    ({cat:#04x}, {idx:#04x}, {sid}), // {SET_ORDER[sid]} · {name}")
    lines.append("];")
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=HD.DEFAULT_ZIP if hasattr(HD, "DEFAULT_ZIP") else GL.DEFAULT_ZIP)
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()
    text = render(rows(HD.open_jar(a.zip)))
    if a.check:
        if not os.path.exists(OUT):
            raise SystemExit(f"STALE: {OUT} does not exist — run this tool without --check")
        with open(OUT, encoding="utf-8") as f:
            have = f.read().replace("\r\n", "\n")
        if have != text:
            raise SystemExit(f"STALE: {OUT} does not match the asset — run this tool without --check")
        print(f"OK: {OUT} matches the asset")
        return
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {OUT}: {text.count('),')} pieces")


if __name__ == "__main__":
    main()
