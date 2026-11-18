"""What can a player actually GET? Drop tables, and the reachability of every item.

Answers the question "is this item obtainable offline at all", which the shipped
data settles far more sharply than play does.  Three engine mechanisms can hand
the player a piece of equipment, and this tool models all three:

  1. GENERIC drop.  The looting routine rolls into the species' weapon / armour /
     soul intervals and then calls the candidate picker at guest 0x38cf4, which
     scans a category and keeps rows whose LEVEL (mem +0x11 = stat byte +8) is
     inside a window and whose TIER (mem +0x10 = stat byte +7) passes:
         0x38d4a  cmp #0        - one arm keeps ONLY tier 0
         0x38d90  cmp #3 / bhi  - the other keeps tier <= 3
     and the level window itself is clamped at 0x38cf8/0x38d0e to lo <= 40,
     hi <= 59.  So a generic drop can never produce tier >= 4, nor anything
     that asks for level 60+.
  2. NAMED drop.  One (cat, idx) slot per species per difficulty - the boss
     finisher.  Enumerated below; the ceiling across both files is tier 4.
  3. SMITHING.  i15_dat turns one base item plus materials into one output item.
     Chased transitively here, so "reachable" means reachable through any chain.

Shops are not a fourth source of anything new: shop_dat selects by level band
(<= 40) and category, i.e. a subset of 1.  Quest tables carry no tier 4/5 ids.

Usage:
    python tools_re/dump_drop_table.py [--zip <asset.zip>] [--species N]
                                       [--unreachable] [--tier N]
"""
import argparse
import collections
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from h3crypt import open_assets, decrypt_member, records, DEFAULT_ZIP  # noqa: E402

TIER_NAME = {0: "기본", 1: "노랑", 2: "초록", 3: "하늘", 4: "빨강", 5: "흰색", 8: "혼"}
SLOT = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", 4: "스피어", 5: "스워드",
        6: "나이프", 7: "단총", 8: "캐논", 9: "암흑성", 10: "광휘석", 11: "방패",
        12: "혼/악세"}
COLOUR = {0x40: "white", 0x7C: "yellow", 0x24: "cyan", 0x7B: "red",
          0x7D: "gold", 0x27: "green", 0x22: "red2"}
GENERIC_TIER_MAX = 3      # guest 0x38d90
GENERIC_LEVEL_MAX = 59    # guest 0x38d0e
SOUL_CAT = 0x0C           # drop record +0x0a names the accessory outright


def equipment(jar):
    """{(cat, idx): (name, colour, tier, level, price)} for cat 0x00..0x0c."""
    out = {}
    for cat in range(0, 13):
        for i, r in enumerate(records(jar.read("res/dat/i%d_dat" % cat))):
            nl = r[0]
            nm, st = r[1:1 + nl], r[1 + nl:]
            if len(st) < 9:
                continue
            out[(cat, i)] = (nm.decode("euc-kr", "replace"),
                             COLOUR.get(nm[0], "") if nm else "",
                             st[7], st[8], struct.unpack_from("<I", st, 0)[0])
    return out


def species_names(jar):
    return [r[1:1 + r[0]].decode("euc-kr", "replace")
            for r in records(jar.read("res/dat/enemy_dat"))]


def drop_records(jar, hard):
    name = "res/dat/droph_dat" if hard else "res/dat/drop_dat"
    out = []
    for r in records(decrypt_member(jar, name)):
        u = lambda o: struct.unpack_from("<H", r, o)[0]
        out.append(dict(wcap=u(0), wsplit=u(2), acap=u(4), asplit=u(6),
                        scap=u(8), sidx=r[10], ncap=u(11),
                        cat=r[13], idx=r[14], ccap=u(15)))
    return out


def recipes(jar):
    """[(base(cat,idx), out(cat,idx), name, recipe_text)] from i15_dat."""
    out = []
    for r in records(decrypt_member(jar, "res/dat/i15_dat")):
        nl = r[0]
        nm = r[1:1 + nl].decode("euc-kr", "replace")
        o = 1 + nl + 4
        el = r[o]
        o += 1
        txt = r[o:o + el].decode("euc-kr", "replace")
        t = r[o + el:]
        if len(t) >= 9:
            out.append(((t[0], t[1]), (t[-2], t[-1]), nm, txt))
    return out


def reachable(jar, items):
    """(reachable set, named-drop set, soul-drop set).

    The soul slot is its own mechanism and must not be scored with the generic
    rule: the drop record names the accessory by index (+0x0a) instead of
    letting the picker choose, and every cat 0x0c row has level 0, so the
    generic level window would have excluded the whole category.
    """
    named, souls = set(), set()
    for hard in (False, True):
        for d in drop_records(jar, hard):
            if d["cat"] != 0xFF:
                named.add((d["cat"], d["idx"]))
            souls.add((SOUL_CAT, d["sidx"]))
    seed = {k for k, v in items.items()
            if k[0] != SOUL_CAT
            and v[2] <= GENERIC_TIER_MAX and 1 <= v[3] <= GENERIC_LEVEL_MAX}
    seed |= (named | souls) & set(items)
    reach, rec = set(seed), recipes(jar)
    changed = True
    while changed:                       # smithing chains
        changed = False
        for base, made, _, _ in rec:
            if base in reach and made in items and made not in reach:
                reach.add(made)
                changed = True
    return reach, named, souls


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--species", type=int, help="print one species' drop record")
    ap.add_argument("--unreachable", action="store_true",
                    help="list every item no offline path can produce")
    ap.add_argument("--tier", type=int, help="restrict --unreachable to one tier")
    a = ap.parse_args()

    jar = open_assets(a.zip)
    items, names = equipment(jar), species_names(jar)
    reach, named, souls = reachable(jar, items)

    if a.species is not None:
        for hard in (False, True):
            d = drop_records(jar, hard)[a.species]
            tag = "HARD" if hard else "NORM"
            it = items.get((d["cat"], d["idx"]))
            print("%s %-16s weapon<=%-5d(split %-5d) armour<=%-5d(split %-5d) "
                  "soul<=%-5d(idx %2d) named<=%-5d consumable<=%-5d  named=%s"
                  % (tag, names[a.species], d["wcap"], d["wsplit"], d["acap"],
                     d["asplit"], d["scap"], d["sidx"], d["ncap"], d["ccap"],
                     "-" if d["cat"] == 0xFF else "%s (cat 0x%02x idx 0x%02x)"
                     % (it[0] if it else "?", d["cat"], d["idx"])))
        return

    if a.unreachable:
        rows = [(k, v) for k, v in sorted(items.items()) if k not in reach]
        if a.tier is not None:
            rows = [r for r in rows if r[1][2] == a.tier]
        print("오프라인 획득 경로가 없는 장비 %d종" % len(rows))
        for (c, i), (nm, col, t, lv, pr) in rows:
            print("  cat=0x%02x idx=0x%02x  tier=%d %-4s lv=%-3d %-7s %-16s price=%d"
                  % (c, i, t, TIER_NAME.get(t, "?"), lv, col, nm, pr))
        return

    print("=== 지정 드롭(named) 슬롯 ===")
    for hard in (False, True):
        n = sum(1 for d in drop_records(jar, hard) if d["cat"] != 0xFF)
        print("  %-4s %d/161 종이 지정 드롭을 가진다" % ("HARD" if hard else "NORM", n))
    eq = sorted(k for k in named if k in items)
    print("  그중 장비인 것 %d종, 최고 티어 %d"
          % (len(eq), max(items[k][2] for k in eq) if eq else -1))
    for k in eq:
        nm, col, t, lv, _ = items[k]
        print("      cat=0x%02x idx=0x%02x tier=%d lv=%-3d %-7s %s"
              % (k[0], k[1], t, lv, col, nm))

    print("\n=== 티어별 도달 가능성 (일반 드롭 + 지정 드롭 + 조합 연쇄) ===")
    agg = collections.defaultdict(collections.Counter)
    for k, v in items.items():
        a_ = agg["tier %d %s" % (v[2], TIER_NAME.get(v[2], "?"))]
        a_["총"] += 1
        a_["가능" if k in reach else "불가"] += 1
    for k in sorted(agg):
        print("  %-12s %s" % (k, dict(agg[k])))

    print("\n=== 카테고리별 도달 불가 ===")
    for cat in range(0, 13):
        miss = [k for k in items if k[0] == cat and k not in reach]
        if miss:
            print("  cat 0x%02x %-7s %d종" % (cat, SLOT[cat], len(miss)))


if __name__ == "__main__":
    main()
