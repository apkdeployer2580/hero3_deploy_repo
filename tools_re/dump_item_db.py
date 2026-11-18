"""Dump the game's whole item database out of the shipped asset, every category.

WHY THIS EXISTS ALONGSIDE `decode_items.py`: that tool's record model is WRONG, and the
error is load-bearing. It reads the file as "u16 FILE header, then per record
namelen/name/4B/efflen/effect/<T>B trailer". There is no file header: the u16 is a
**per-record length**, and what that tool calls the last 2 bytes of a record's trailer is
the NEXT record's length prefix. Both halves of `rename_cash_items.py` inherited the
mistake, which is how the shipped `dat/i18_dat` came to carry stale length prefixes
(see `hero3_server/CASH_SHOP.md`). Prefer this decoder.

The real layout, one file per item CATEGORY (`iN_dat` = category N), all EUC-KR:

    per record:  [u16 LE reclen][body]        reclen = len(body)

    EQUIPMENT   (i0..i12)   body = [u8 namelen][name][stats]
                            len(stats) = 20, except i12 = 18
    CONSUMABLE  (i13, i14, i16, i17, i18)
                            body = [u8 namelen][name][4B price][u8 efflen][effect][tail]
                            len(tail) is a per-file constant: i13=5 i14=0 i16=4 i17=0 i18=4

    i15 is MX-DES encrypted (key `0EP@KO91`) and is NOT the equipment file — the
    equipment categories are the plaintext i0..i12.

This stride parses all 18 plaintext files byte-exact end to end, which is what says the
record positions are the wire indices: an item id on the wire is `(category, index)`.

The first name byte, when it is one of `@ | $ { } ' "`, is a TEXT COLOUR ESCAPE, not a
cash flag (rodata alphabet 0x5b2bc, RGB table 0x5b268; readers `isEscape` 0x35d8 and
`applyColourEscape` 0x37e4, both in the font layer). Reported as `colour`.

Usage:
    python tools_re/dump_item_db.py [--zip <asset.zip>] [--dat <dir>] [--json <out>]
                                    [--cat N] [--colour white,gold] [--grep 카이져]
"""
import argparse
import io
import json
import os
import sys
import zipfile

DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")

# statlen per equipment category; everything not listed is 20
EQUIP_STATLEN = {12: 18}
EQUIP_CATS = range(0, 13)
ENCRYPTED = {15}

COLOUR = {0x40: ("@", "white"), 0x7C: ("|", "yellow"), 0x24: ("$", "cyan"),
          0x7B: ("{", "red"), 0x7D: ("}", "gold"), 0x27: ("'", "green"),
          0x22: ('"', "red2")}

# What each equipment category equips, read off its own item names.
SLOT = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", 4: "스피어", 5: "스워드",
        6: "나이프", 7: "단총", 8: "캐논", 9: "암흑성", 10: "광휘석", 11: "방패",
        12: "혼/악세", 13: "전투주문서", 14: "재료", 16: "?", 17: "?", 18: "소비"}


def euc(b):
    try:
        return b.decode("euc-kr")
    except Exception:
        return "<" + b.hex() + ">"


def records(blob):
    """Walk the [u16 reclen] framing. Yields (index, offset, body). Stops on the first
    record that does not fit, so a caller can tell a clean parse from a truncated one."""
    p = i = 0
    while p + 2 <= len(blob):
        rl = blob[p] | (blob[p + 1] << 8)
        if rl == 0 or p + 2 + rl > len(blob):
            break
        yield i, p, blob[p + 2:p + 2 + rl]
        p += 2 + rl
        i += 1


def split_name(body):
    nl = body[0]
    nm = body[1:1 + nl]
    mark, colour = COLOUR.get(nm[0], ("", "")) if nm else ("", "")
    return nm, mark, colour, euc(nm[1:] if mark else nm), 1 + nl


def decode(cat, blob):
    rows = []
    end = 0
    for idx, off, body in records(blob):
        nm, mark, colour, name, used = split_name(body)
        row = {"idx": idx, "name": name, "marker": mark, "colour": colour,
               "raw_name": euc(nm)}
        if cat in EQUIP_CATS:
            row["stats"] = body[used:].hex()
            row["statlen"] = len(body) - used
        else:
            price = body[used:used + 4]
            el = body[used + 4]
            eff = body[used + 5:used + 5 + el]
            row["price"] = price.hex()
            row["effect"] = euc(eff)
            row["tail"] = body[used + 5 + el:].hex()
        rows.append(row)
        end = off + 2 + len(body)
    return rows, end


def load_dats(zip_path, dat_dir):
    """Return {cat: bytes}. Reads the asset zip -> game jar -> res/dat/iN_dat, or a
    directory of already-extracted files."""
    out = {}
    if dat_dir:
        for cat in list(EQUIP_CATS) + [13, 14, 15, 16, 17, 18]:
            f = os.path.join(dat_dir, "i%d_dat" % cat)
            if os.path.exists(f):
                out[cat] = open(f, "rb").read()
        return out
    z = zipfile.ZipFile(zip_path)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit("expected exactly one .jar in %s, found %r" % (zip_path, jars))
    j = zipfile.ZipFile(io.BytesIO(z.read(jars[0])))
    for n in j.namelist():
        base = os.path.basename(n)
        if base.startswith("i") and base.endswith("_dat"):
            mid = base[1:-4]
            if mid.isdigit():
                out[int(mid)] = j.read(n)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--dat", help="directory of extracted iN_dat files (skips the zip)")
    ap.add_argument("--json", help="also write the whole decode here")
    ap.add_argument("--cat", type=lambda s: int(s, 0), action="append",
                    help="only this category (repeatable)")
    ap.add_argument("--colour", help="only these colours, comma separated "
                                     "(white,gold,green,red,cyan,yellow,plain)")
    ap.add_argument("--grep", help="only rows whose name contains this")
    a = ap.parse_args()

    want_cat = set(a.cat) if a.cat else None
    want_col = set(a.colour.split(",")) if a.colour else None
    dats = load_dats(a.zip, a.dat)
    doc = {}

    for cat in sorted(dats):
        blob = dats[cat]
        if cat in ENCRYPTED:
            doc["i%d" % cat] = {"cat": cat, "kind": "encrypted", "size": len(blob), "rows": []}
            continue
        rows, end = decode(cat, blob)
        doc["i%d" % cat] = {"cat": cat, "kind": "equip" if cat in EQUIP_CATS else "consumable",
                            "slot": SLOT.get(cat, "?"), "rows": rows,
                            "parsed": end, "size": len(blob), "clean": end == len(blob)}

    if a.json:
        json.dump(doc, open(a.json, "w", encoding="utf-8"), ensure_ascii=False, indent=1)

    for key in sorted(doc, key=lambda k: doc[k]["cat"]):
        v = doc[key]
        if want_cat is not None and v["cat"] not in want_cat:
            continue
        if v["kind"] == "encrypted":
            print("=== %-4s cat=0x%02x  MX-DES ENCRYPTED, %d bytes" % (key, v["cat"], v["size"]))
            continue
        state = "OK" if v["clean"] else "** TRUNCATED %d/%d — record framing is broken **" % (
            v["parsed"], v["size"])
        print("=== %-4s cat=0x%02x %-11s %-8s rec=%-3d %s" % (
            key, v["cat"], v["kind"], v["slot"], len(v["rows"]), state))
        for r in v["rows"]:
            if want_col and (r["colour"] or "plain") not in want_col:
                continue
            if a.grep and a.grep not in r["name"]:
                continue
            print("  0x%02x %1s%-7s %-18s %s" % (
                r["idx"], r["marker"], r["colour"], r["name"],
                r.get("effect", r.get("stats", ""))))

    bad = [k for k, v in doc.items() if v["kind"] != "encrypted" and not v["clean"]]
    if bad:
        print("\n!! %s did not parse end-to-end — see this file's docstring on stale "
              "record lengths." % ", ".join(sorted(bad)), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
