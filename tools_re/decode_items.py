"""Decode Hero3 item-DB category files (JAR resource dat/iN_dat).

One file per item CATEGORY: file `iN_dat` holds category byte N (decimal), and on
the online wire an item id = (category = record[8], index = record[9]). The client
resolves an item's name / effect / 원-price / icon from these files by that id —
NOT from client.bin — so the private server only needs the right (cat, idx) pairs.

Two on-disk formats (RE wf_74d95519, 2026-07-23, validated against the device-
confirmed cat-0x12 cash catalog):

  CONSUMABLE / SCROLL  (i0..i14, i16..i18 — PLAINTEXT EUC-KR):
    u16 LE header, then per record:
      namelen u8, name[namelen], 4B field, efflen u8, effect[efflen], TRAILER[T]
    T is a per-file constant (6 for i18 consumables, 7 for i13 scrolls); this tool
    auto-detects it. A leading byte 0x7d ('}') in the NAME marks a CASH item.
    Record position N = wire index N.

  EQUIPMENT  (i15 — MX-DES ENCRYPTED, key 0EP@KO91):
    16B preamble, then per record:
      2B pre-field, namelen u8, name[namelen], 4B field, efflen u8, effect, 9B trailer
    Names lead with 0x7c '|' / 0x7b '{' / 0x40 '@' / 0x27 quote = RARITY/CRAFT
    tier color codes (effects are craft recipes), NOT cash. Equipment has 0 '}'
    items (the 0x7d bytes in the raw ENCRYPTED file are DES cipher noise).

Usage:  python decode_items.py [i18_dat i13_dat i15_dat ...]
        (default = all three anchors). Set PYTHONIOENCODING=utf-8 for names.
"""
import os
import sys

# --- locations (override via env if the layout moves) ---
DAT_DIR = os.environ.get("H3_DAT_DIR", r"d:\tmp\h3_jar_inspect\dat")
H5_DES_DIR = os.environ.get("H5_DES_DIR", r"d:\testrepo\tools")   # provides mx_des_decrypt
MX_KEY = b"0EP@KO91"                                              # Hero3 DES key (rodata 13-char, first 8)
ENCRYPTED_FILES = {"i15_dat"}                                    # equipment category is MX-DES encrypted

CASH_MARKER = 0x7d           # '}' = cash/premium marker (consumables & scrolls)
EQUIP_MARKERS = {0x27, 0x40, 0x7b, 0x7c}   # ' @ { |  = equipment rarity/craft tiers


def mx_decrypt(raw: bytes) -> bytes:
    """Decrypt an MX-DES .dat resource (equipment). Falls back to a cached plain file."""
    try:
        sys.path.insert(0, H5_DES_DIR)
        from h5_des import mx_des_decrypt  # type: ignore
        return mx_des_decrypt(raw, MX_KEY)
    except Exception as e:
        cached = os.path.join(r"d:\testrepo\work\h3\decrypted", "i15_dat.0EP@KO91.plain")
        if os.path.exists(cached):
            return open(cached, "rb").read()
        raise RuntimeError(f"mx_des_decrypt unavailable ({e}) and no cached plain at {cached}")


def _valid_name(b: bytes, allow_markers: set) -> bool:
    """A length-prefixed name = optional leading marker byte, then >=2 EUC-KR
    double-byte chars (lead/trail 0xa1-0xfe) and/or ASCII printable."""
    if not b:
        return False
    i = 1 if b[0] in allow_markers else 0
    dbl = 0
    while i < len(b):
        c = b[i]
        if 0xa1 <= c <= 0xfe:
            if i + 1 >= len(b) or not (0xa1 <= b[i + 1] <= 0xfe):
                return False
            i += 2
            dbl += 1
        elif 0x20 <= c <= 0x7e:
            i += 1
        else:
            return False
    return dbl >= 2


def _parse(d: bytes, header: int, prefix: int, trailer: int, markers: set):
    """Parse records with a fixed layout: [header] then per record
    [prefix bytes][namelen][name][4B][efflen][effect][trailer]. None on misparse."""
    recs = []
    off = header
    n = len(d)
    while off < n:
        if all(x == 0 for x in d[off:off + max(1, prefix + 1)]) and off + 16 >= n:
            break  # zero padding tail
        p = off + prefix
        if p >= n:
            return None
        namelen = d[p]
        s = p + 1
        if namelen == 0 or s + namelen > n:
            if all(x == 0 for x in d[off:]):
                break
            return None
        name = d[s:s + namelen]
        if not _valid_name(name, markers):
            if all(x == 0 for x in d[off:]):
                break
            return None
        q = s + namelen
        if q + 5 > n:
            return None
        price = int.from_bytes(d[q:q + 4], "little")
        q += 4
        efflen = d[q]
        q += 1
        if q + efflen > n:
            return None
        effect = d[q:q + efflen]
        q += efflen
        trail = d[q:q + trailer]
        recs.append({"off": off, "name": name, "price": price, "effect": effect, "trail": trail})
        off = q + trailer
    return recs


def decode(fn: str):
    path = os.path.join(DAT_DIR, fn)
    d = open(path, "rb").read()
    encrypted = fn in ENCRYPTED_FILES
    if encrypted:
        d = mx_decrypt(d)
    # equipment (encrypted) uses a 16B preamble + 2B pre-field + 9B trailer and
    # rarity markers; consumables/scrolls use a 2B header + 0 pre-field + rarity=cash.
    if encrypted:
        cfgs = [(16, 2, 9, EQUIP_MARKERS | {CASH_MARKER})]
    else:
        cfgs = [(2, 0, T, {CASH_MARKER}) for T in range(4, 12)]
    best = None
    for header, prefix, trailer, markers in cfgs:
        recs = _parse(d, header, prefix, trailer, markers)
        if recs and (best is None or len(recs) > len(best[1])):
            best = ((header, prefix, trailer), recs)
    return d, encrypted, best


def report(fn: str):
    d, encrypted, best = decode(fn)
    tag = "MX-DES decrypted" if encrypted else "plaintext"
    if not best:
        print(f"\n===== {fn} ({tag}, len={len(d)}) : UNPARSEABLE =====")
        return
    (hdr, pre, tr), recs = best
    cash = [(i, r) for i, r in enumerate(recs) if r["name"][:1] == bytes([CASH_MARKER])]
    print(f"\n===== {fn} ({tag}, len={len(d)}) : {len(recs)} records "
          f"[header={hdr} pre={pre} trailer={tr}] : {len(cash)} cash ('}}') items =====")
    for i, r in cash:
        try:
            nm = r["name"][1:].decode("euc-kr")
        except Exception:
            nm = repr(r["name"])
        try:
            ef = r["effect"].decode("euc-kr")
        except Exception:
            ef = repr(r["effect"])
        print(f"  idx 0x{i:02x}({i:>2})  {nm:<16}  | {ef}")
    if encrypted:
        from collections import Counter
        mk = Counter(chr(r["name"][0]) if r["name"][0] in (EQUIP_MARKERS | {CASH_MARKER}) else "-"
                     for r in recs)
        print(f"  (equipment rarity markers: {dict(mk)})  cash items = {len(cash)}")


if __name__ == "__main__":
    files = sys.argv[1:] or ["i18_dat", "i13_dat", "i15_dat"]
    for f in files:
        report(f)
