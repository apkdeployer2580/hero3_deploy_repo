"""Bake the whole Hero3 item database AND where each item comes from into one HTML page.

The page is the item codex at https://hero3-item-codex.vercel.app (Vercel project
`hero3-item-codex`, linked from `out/hero3-item-codex/.vercel/project.json`). It is the
3편 counterpart of hero5's codex and hero2's drop table.

WHERE THE NUMBERS COME FROM - nothing below is typed in by hand:

  * the item database - `res/dat/i0_dat`..`i18_dat` of the shipped asset
    (`assets/hero3_fixed.zip` -> game jar), `i15_dat` (조합서) DES-decrypted by `h3crypt`;
  * the option labels - the game's own `res/dat/InGame_txt` row `80 + statId` (the detail
    window prints exactly that), and its `%` rule (`0x6d34`: ids 2 3 5..8 0x0e..0x12, plus
    0x78 since the 150차 patch at `0x6d38`);
  * the stock drops - `drop_dat` / `droph_dat` (161 x 17 B per species, DES) and the roll
    count `enemy_dat` / `enemyh_dat` tail[19]; the loot routine `0x38e70` rolls
    `rand(0, 0xffff)` that many times against the record's cumulative caps (weapon <
    armour < soul < named < material, a value EQUAL to a cap is empty-handed - 134차 §5.1);
  * the app's own drops - read out of the Rust sources that ship them:
    `binary_patches/{helldrop,reaverdrop,helldrop_table}.rs` (hell bands, reaver, boss
    accessories), `wie_android/src/guardian.rs` (`REWARD_TIERS`, the hell ladder),
    `wie_android/src/{set_items,setbonus}.rs` (sets), `tools_re/acc_options.py` (the
    accessories' third option, which lives outside the asset), `cash_catalog.rs`.

EQUIPMENT, `i0`..`i11`, 20-byte stat block (MIGRATION_LGT.md 2.22-(1)):

    b0..3  u32 shop price        b4     sprite id     b5   palette (0xff = own colours)
    b7     tier 0..5             b8     level requirement
    b12,13 u16 main stat (weapon = attack, armour = defence)
    b14,15 u16 secondary (weapons only)
    b16..19  two (statId, value) option pairs

`i12` (accessories) is the same block two bytes shorter: tier `b5`, level `b6`,
main `b10`, secondary `b12`, pairs `b14..17`. The shift is checked, not assumed
(`verify()`): the reaver rows, the boss accessories and the eleven reworked accessories
were written by tools whose own tables are independent of this decode.

Usage:
    python tools_re/gen_item_codex.py [--zip <asset.zip>] [--out <dir>] [--check]
"""
import argparse
import base64
import collections
import io
import json
import os
import re
import struct
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)

import acc_options as AO  # noqa: E402
import add_attack_speed_label as ASL  # noqa: E402
import gen_legacy_items as GL  # noqa: E402
import h3crypt  # noqa: E402

DEFAULT_ZIP = os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
DEFAULT_OUT = os.path.join(ROOT, "out", "hero3-item-codex")
VERSION_FILE = os.path.join(ROOT, "android_app", "version.properties")

BP = os.path.join(ROOT, "vendor", "wie", "wie_core_arm", "src", "binary_patches")
HELLDROP_RS = os.path.join(BP, "helldrop.rs")
HELLTABLE_RS = os.path.join(BP, "helldrop_table.rs")
REAVER_RS = os.path.join(BP, "reaverdrop.rs")
GUARDIAN_RS = os.path.join(ROOT, "wie_android", "src", "guardian.rs")
SET_ITEMS_RS = os.path.join(ROOT, "wie_android", "src", "set_items.rs")
SETBONUS_RS = os.path.join(ROOT, "wie_android", "src", "setbonus.rs")
CASH_RS = os.path.join(ROOT, "wie_android", "src", "cash_catalog.rs")

# The release the page's 「신규」 tags are measured against (개정판 v0.389.0 = 129차).
BASE_RELEASE = "0.389.0"

EQUIP_CATS = list(range(0, 12))          # i0..i11, 20-byte block
ACC_CAT = 12                             # i12, 18-byte block
CONSUM_CATS = [13, 14, 16, 17, 18]
RECIPE_CAT = 15
MAT_CAT = 14

# The game's own colour-escape table: alphabet at .text 0x5b2bc, RGB triples (u32 R,G,B)
# at 0x5b268, same order.
COLOUR = {
    "@": ("white", "#FFFFFF"), "|": ("yellow", "#D8D926"), "$": ("cyan", "#2EBCFF"),
    "{": ("red", "#DF3939"), "}": ("gold", "#DB9A1A"), "'": ("green", "#78C437"),
    '"': ("red2", "#FF0000"),
}
COLOUR_KO = {"white": "흰색", "yellow": "노랑", "cyan": "하늘",
             "red": "빨강", "gold": "금색", "green": "초록",
             "red2": "빨강2", "": "무색"}

# tier -> the name colour it is 1:1 with across all twelve slots (2.22-(1)). Tier 5 has two:
# the stock legend `@` and, since 133차, the new 85/90제 rows `"`.
TIER_COLOUR = {0: "", 1: "yellow", 2: "green", 3: "cyan", 4: "red", 5: "white"}

SLOT = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발",
        4: "스피어", 5: "스워드", 6: "나이프",
        7: "단총", 8: "캐논", 9: "암흑성", 10: "광휘석",
        11: "방패", 12: "혼·악세"}

MELEE = {4, 5, 6}
RANGED = {7, 8, 9, 10}

# The detail window's `%` rule (`0x6d34`), and 0x78 (5초당 HP회복) since the 150차 patch.
PCT_IDS = {0x02, 0x03, 0x05, 0x06, 0x07, 0x08, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x78}
LABEL_IDS = list(range(0x00, 0x18)) + [0x76, 0x78]
# `InGame_txt` row 80 + 0x1c is an unrelated word; the one scroll that carries 0x1c is the
# revive scroll and its own text says so.
EXTRA_LABEL = {0x1C: "전투불능 회복"}
# `addStat 0x4ee34` takes statId 2..0x17 and 0x04 is an empty arm: an EQUIPMENT option
# carrying 0x01 or 0x04 adds nothing (0x00 feeds the item's own 성능). 0x76 / 0x78 are the
# host's own stats (131차-D 공격속도 · 150차 5초당 HP회복).
NO_ARM = {0x01, 0x04}
HOST_STATS = {0x76, 0x78}

SCROLL_TARGET = {0: "적 1", 1: "적 전체", 2: "자신", 3: "아군 1", 4: "아군 전체"}
ORB_FUSE = [1, 2, 3, 4, 5]

# The material picker `0x38e28` (134차 §5.1): rand(0,9) 0~1 -> idx 9..13, 2~5 -> 0..2,
# 6~9 -> 3..8.
MAT_GROUPS = [((9, 13), 2), ((0, 2), 4), ((3, 8), 4)]
GENERIC_TIER_MAX = 3      # guest 0x38d90
GENERIC_LEVEL_MAX = 59    # guest 0x38d0e


# -- asset plumbing ----------------------------------------------------------

def open_jar(zip_path):
    z = zipfile.ZipFile(zip_path)
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit("expected exactly one .jar in %s, found %r" % (zip_path, jars))
    return zipfile.ZipFile(io.BytesIO(z.read(jars[0])))


def records(blob):
    """The [u16 reclen][body] framing. Raises if it does not consume the file exactly."""
    p = i = 0
    out = []
    while p + 2 <= len(blob):
        rl = blob[p] | (blob[p + 1] << 8)
        if rl == 0 or p + 2 + rl > len(blob):
            break
        out.append((i, blob[p + 2:p + 2 + rl]))
        p += 2 + rl
        i += 1
    if p != len(blob):
        raise SystemExit("record framing broke at %d of %d bytes" % (p, len(blob)))
    return out


def euc(b):
    return b.decode("euc-kr", "replace")


def split_name(body):
    nl = body[0]
    raw = body[1:1 + nl]
    mark = chr(raw[0]) if raw and chr(raw[0]) in COLOUR else ""
    name = euc(raw[1:] if mark else raw)
    return mark, name, 1 + nl


def clean(text):
    """Game text minus colour escapes, `;` line breaks shown as ` · `."""
    t = "".join(ch for ch in text if ch not in "@|${}'\"")
    parts = [p.strip() for p in t.split(";")]
    return " · ".join(p for p in parts if p)


# -- Rust sources: the app's own drop rules ---------------------------------------

def rs_text(path):
    with open(path, encoding="utf-8") as fp:
        return fp.read()


def rs_const(src, name):
    m = re.search(r"pub const %s:\s*[^=]+=\s*([^;]+);" % re.escape(name), src)
    if not m:
        raise SystemExit("constant %s not found" % name)
    return m.group(1).strip()


def rs_int(src, name):
    return int(rs_const(src, name).replace("_", ""), 0)


def rs_tuples(text):
    """Every `(a, b, c)` integer tuple in a slice of Rust source, in order."""
    out = []
    for m in re.finditer(r"\(([0-9xa-fA-F_,\s]+)\)", text):
        parts = [p.strip() for p in m.group(1).split(",") if p.strip()]
        out.append(tuple(int(p.replace("_", ""), 0) for p in parts))
    return out


def rs_array(src, name):
    """The body of `pub const NAME: [...] = [ ... ];`, comments stripped."""
    m = re.search(r"pub const %s:[^=]+=\s*&?\[(.*?)\];" % re.escape(name), src, re.S)
    if not m:
        raise SystemExit("array %s not found" % name)
    return re.sub(r"//[^\n]*", "", m.group(1))


def load_rules():
    hd = rs_text(HELLDROP_RS)
    ht = rs_text(HELLTABLE_RS)
    rv = rs_text(REAVER_RS)
    gd = rs_text(GUARDIAN_RS)
    hell = {k: rs_int(hd, k) for k in (
        "RATE_PCT", "HELL_LV_MIN", "HELL_LV_MAX", "RECIPE_LV_MIN", "RECIPE_LV_MAX",
        "ITEM_LV_MIN", "SHARD_LV_MIN", "NAMED_LV_MIN", "BOSS_TIER_REQ", "BOSS_DAT_BASE")}
    host = {k: rs_int(gd, k) for k in (
        "HELL_SPAWN_LV_MIN", "HELL_SPAWN_LV_MAX", "HELL_LV_MIN", "HELL_LV_MAX", "ENDGAME_MIN_LV")}
    for k in ("HELL_LV_MIN", "HELL_LV_MAX"):
        if host[k] != hell[k]:
            raise SystemExit("guardian.rs %s %d != helldrop.rs %d" % (k, host[k], hell[k]))
    items = rs_tuples(rs_array(ht, "HELL_ITEMS"))
    recipes = rs_tuples(rs_array(ht, "HELL_RECIPES"))
    named = [int(x, 0) for x in re.findall(r"\b(0x[0-9a-fA-F]+|\d+)\b", rs_array(ht, "HELL_NAMED_SPECIES"))]
    shard = rs_tuples(rs_const(ht, "HELL_SHARD"))[0]
    reaver = {
        "rate": rs_int(rv, "RATE_PCT"),
        "bosses": [int(x) for x in re.findall(r"\d+", rs_array(rv, "BOSS_SPECIES"))],
        "items": rs_tuples(rs_array(rv, "REAVER_ITEMS")),
        "hell_max": rs_int(rv, "REAVER_HELL_LV_MAX"),
        "acc": rs_tuples(rs_array(rv, "BOSS_ACCESSORIES")),
    }
    tiers = []
    for m in re.finditer(r"RewardTier \{([^}]*)\}", rs_array(gd, "REWARD_TIERS")):
        body = m.group(1)

        def f(key, body=body):
            mm = re.search(r"\b%s:\s*(Some\(\([^)]*\)\)|\([^)]*\)|[^,\s}]+)" % key, body)
            if not mm:
                raise SystemExit("REWARD_TIERS: no %s in %r" % (key, body))
            return mm.group(1).strip()

        rec = tuple(int(x) for x in re.findall(r"\d+", f("recipe")))
        leg = f("legacy")
        leg = tuple(int(x) for x in re.findall(r"\d+", leg)) if leg.startswith("Some") else None
        tiers.append({"boss": int(f("boss")), "shards": int(f("shards")), "recipe": rec,
                      "legacy": leg, "endgame": int(f("endgame_pct"))})
    return {"hell": hell, "host": host, "items": items, "recipes": recipes, "named": named,
            "shard": shard, "reaver": reaver, "tower": tiers}


def load_sets():
    pieces = {(c, i): s for c, i, s in rs_tuples(rs_array(rs_text(SET_ITEMS_RS), "SET_ITEMS"))}
    src = rs_text(SETBONUS_RS)
    need = rs_int(src, "PIECES_FOR_BONUS")
    body = rs_array(src, "SETS")
    sets = [list(m) for m in re.findall(
        r'name:\s*"([^"]*)",\s*bonus:\s*"([^"]*)",\s*text:\s*"([^"]*)"', body)]
    if len(sets) != rs_int(src, "SET_COUNT"):
        raise SystemExit("setbonus.rs: %d SETS parsed, SET_COUNT says otherwise" % len(sets))
    return pieces, sets, need


def load_cash():
    return {(int(c, 16), int(i, 16)) for c, i in
            re.findall(r"// cat 0x([0-9a-f]+) idx 0x([0-9a-f]+)", rs_text(CASH_RS))}


def hell_level(lv0, host):
    lo, hi = host["HELL_SPAWN_LV_MIN"], host["HELL_SPAWN_LV_MAX"]
    d = min(max(lv0, lo), hi) - lo
    span_in = hi - lo
    span_out = host["HELL_LV_MAX"] - host["HELL_LV_MIN"]
    return host["HELL_LV_MIN"] + (d * span_out + span_in // 2) // span_in


def app_version():
    try:
        with open(VERSION_FILE, encoding="ascii") as fp:
            m = re.search(r"^versionCode=(\d+)", fp.read(), re.M)
        return "0.%s.0" % m.group(1) if m else "?"
    except OSError:
        return "?"


# -- `_bm` sprite decode (same arithmetic as gen_h3_icons.py, asserted per frame) --

def rgb565(v):
    if v == 0xF81F:
        return None                      # palette[0] is the transparent sentinel
    r, g, b = (v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F
    return (r * 255 // 31, g * 255 // 63, b * 255 // 31)


def decode_bm(d):
    (count,) = struct.unpack_from("<H", d, 0)
    off = 2
    frames = []
    for i in range(count):
        (flen,) = struct.unpack_from("<I", d, off)
        off += 4
        t = d[off]
        w, h, cw, palcnt = struct.unpack_from("<HHHH", d, off + 1)
        assert t in (0x0B, 0x0C), "frame %d: bad type %#x" % (i, t)
        assert 1 <= cw <= w, "frame %d: cw %d > w %d" % (i, cw, w)
        pal = [rgb565(struct.unpack_from("<H", d, off + 9 + 2 * j)[0]) for j in range(palcnt)]
        po = off + 9 + palcnt * 2
        nbytes = w * h if t == 0x0C else (w * h + 1) // 2
        assert 9 + palcnt * 2 + nbytes == flen, "frame %d: length mismatch" % i
        px = [None] * (cw * h)
        for k in range(w * h):
            x, y = k % w, k // w
            if x >= cw:
                continue                 # row padding, not picture
            idx = d[po + k] if t == 0x0C else (
                (d[po + k // 2] >> 4) if k % 2 == 0 else (d[po + k // 2] & 0x0F))
            if idx and idx < palcnt:
                px[y * cw + x] = pal[idx]
        frames.append((cw, h, px))
        off = po + nbytes
    assert off == len(d), "consumed %d of %d bytes" % (off, len(d))
    return frames


def frames_to_datauris(frames, scale=4):
    from PIL import Image
    out = []
    for w, h, px in frames:
        im = Image.new("RGBA", (w, h))
        im.putdata([(0, 0, 0, 0) if p is None else (p[0], p[1], p[2], 255) for p in px])
        im = im.resize((w * scale, h * scale), Image.NEAREST)
        buf = io.BytesIO()
        im.save(buf, "PNG", optimize=True)
        out.append("data:image/png;base64," + base64.b64encode(buf.getvalue()).decode())
    return out


# -- field decode ------------------------------------------------------------

def opts(pairs):
    return [{"i": i, "v": v - 256 if v > 127 else v} for i, v in pairs if i or v]


def equip_rows(cat, blob):
    rows = []
    for idx, body in records(blob):
        mark, name, used = split_name(body)
        b = body[used:]
        acc = cat == ACC_CAT
        want = 18 if acc else 20
        if len(b) != want:
            raise SystemExit("i%d idx %d: stat block is %d bytes, expected %d"
                             % (cat, idx, len(b), want))
        s = 2 if acc else 0              # the accessory block is the same, two bytes early
        (price,) = struct.unpack_from("<I", b, 0)
        (main,) = struct.unpack_from("<H", b, 12 - s)
        (sub,) = struct.unpack_from("<H", b, 14 - s)
        rows.append({
            "cat": cat, "idx": idx, "n": name, "mk": mark, "t": b[7 - s], "lv": b[8 - s],
            "price": price, "main": main, "sub": sub,
            "o": opts([(b[16 - s], b[17 - s]), (b[18 - s], b[19 - s])]),
        })
    return rows


def consum_rows(cat, blob, taillen):
    rows = []
    for idx, body in records(blob):
        mark, name, used = split_name(body)
        (price,) = struct.unpack_from("<I", body, used)
        el = body[used + 4]
        eff = euc(body[used + 5:used + 5 + el])
        tail = body[used + 5 + el:]
        if len(tail) != taillen:
            raise SystemExit("i%d idx %d: trailer is %d bytes, expected %d"
                             % (cat, idx, len(tail), taillen))
        r = {"cat": cat, "idx": idx, "n": name, "mk": mark, "price": price, "eff": eff}
        if cat == 13:
            (val,) = struct.unpack_from("<h", tail, 2)
            r.update(tgt=tail[0], i=tail[1], v=val, dur=tail[4])
        elif cat == 16:
            r.update(i=tail[0], pw=tail[1], sx=int("홈을 하나 늘림" in eff))
        elif cat == 18:
            (val,) = struct.unpack_from("<H", tail, 2)
            r.update(ico=tail[0], use=tail[1], v=val)
        rows.append(r)
    return rows


def recipe_rows(jar):
    """`i15_dat` (DES): name, fee (the price field — 138차: the smith's commission), text, and
    the 9-byte tail [base cat][base idx][mat1 idx][n1][mat2 idx][n2][rate][out cat][out idx]."""
    out = []
    for idx, r in enumerate(h3crypt.records(h3crypt.decrypt_member(jar, "res/dat/i15_dat"))):
        mark, name, used = split_name(r)
        (fee,) = struct.unpack_from("<I", r, used)
        el = r[used + 4]
        text = euc(r[used + 5:used + 5 + el])
        t = r[used + 5 + el:]
        if len(t) < 9:
            raise SystemExit("i15_dat %d: tail %d B, expected >= 9" % (idx, len(t)))
        t = t[-9:]
        out.append({"idx": idx, "n": name, "mk": mark, "fee": fee, "txt": clean(text),
                    "base": None if t[0] == 0xFF else [t[0], t[1]],
                    "mats": [[t[2], t[3]]] + ([[t[4], t[5]]] if t[4] != 0xFF else []),
                    "rate": t[6], "out": [t[7], t[8]]})
    return out


def icon_frame(cat, idx, row):
    if cat == 0x12:
        return row["ico"]
    if cat == 0x0E and idx == 3:
        return 0x1C
    return cat


def ingame_labels(jar):
    rows = ASL._parse(jar.read("res/dat/InGame_txt"))
    out = {}
    for i in LABEL_IDS:
        lab = euc(rows[80 + i]).strip()
        out[i] = [lab, 1 if i in PCT_IDS else 0]
    for i, lab in EXTRA_LABEL.items():
        out[i] = [lab, 0]
    return out


# -- monsters and their stock drops ----------------------------------------------------

def species_table(jar, hard):
    """[(name, named, level, rolls, colour mark)] for the 161 field species of one difficulty.
    The name carries the same colour escapes as items: `{` red = the game's named elites,
    `'` green = the harmless critters."""
    out = []
    for r in h3crypt.records(jar.read("res/dat/enemyh_dat" if hard else "res/dat/enemy_dat")):
        nl = r[0]
        raw = r[1:1 + nl]
        mark = chr(raw[0]) if raw and chr(raw[0]) in COLOUR else ""
        name = euc(raw[1:] if mark else raw).rstrip("@").strip()
        t = r[1 + nl:]
        out.append((name, mark == "{", t[0], t[19], mark))
    return out


def drop_table(jar, hard):
    out = []
    for r in h3crypt.records(h3crypt.decrypt_member(jar, "res/dat/droph_dat" if hard else "res/dat/drop_dat")):
        u = lambda o: struct.unpack_from("<H", r, o)[0]  # noqa: E731
        out.append({"w": u(0), "a": u(4), "s": u(8), "sidx": r[10], "n": u(11),
                    "cat": r[13], "idx": r[14], "c": u(15)})
    return out


def per_roll(d):
    """Per-roll chances of the five intervals. A draw EQUAL to a cap is empty-handed."""
    caps = [d["w"], d["a"], d["s"], d["n"], d["c"]]
    out, prev = [], None
    for c in caps:
        width = c if prev is None else c - prev - 1
        out.append(max(0, width) / 65536.0)
        prev = c if prev is None else max(prev, c)
    return out


def per_kill(p, rolls):
    return 0.0 if rolls <= 0 else 1.0 - (1.0 - p) ** rolls


# -- self-check: the decode must reproduce independently recorded values -----

VERIFY = [
    # (cat, name, tier, level, main, sub, price, [(statId, value)])
    # 152차-B: the reavers are 80제 (add_reaver_weapons.py) — stun +50 / cooldown 25.
    (4, "리버스피어", 4, 80, 881, 353, 159500, [(0x0C, 50), (0x11, 25)]),
    (11, "리버쉴드", 4, 80, 176, 0, 76200, [(0x07, 30), (0x08, 30)]),
    # 2.22-(1)/(2): the pistol ladder's top rung, higher index = canonical.
    (7, "카이져스톰", 5, 70, 317, 47, 159500, None),
]


def verify(equip):
    bad = []
    for cat, name, t, lv, main, sub, price, o in VERIFY:
        hits = [r for r in equip if r["cat"] == cat and r["n"] == name]
        if not hits:
            bad.append("%s (cat %d) is not in the decode at all" % (name, cat))
            continue
        r = max(hits, key=lambda r: r["idx"])
        got = (r["t"], r["lv"], r["main"], r["sub"], r["price"])
        if got != (t, lv, main, sub, price):
            bad.append("%s: tier/lv/main/sub/price %r != recorded %r"
                       % (name, got, (t, lv, main, sub, price)))
        if o is not None:
            mine = [(p["i"], p["v"]) for p in r["o"]]
            if mine != o:
                bad.append("%s: options %r != recorded %r" % (name, mine, o))
    # The eleven reworked accessories (150차): acc_options.py is the source the asset
    # tools wrote them from — the decode must read back both pairs, 성능 0 and 요구 Lv 1.
    acc = {r["n"]: r for r in equip if r["cat"] == ACC_CAT}
    for name, _mk, o1, o2, *_ in AO.ACC:
        r = acc.get(name)
        if r is None:
            bad.append("accessory %s is not in i12" % name)
            continue
        mine = [(p["i"], p["v"]) for p in r["o"]]
        if mine != [o1, o2] or r["main"] != AO.PERF or r["lv"] != AO.REQ_LV:
            bad.append("accessory %s: options %r main %d lv %d != acc_options %r"
                       % (name, mine, r["main"], r["lv"], [o1, o2]))
    for r in equip:
        want = TIER_COLOUR.get(r["t"])
        have = COLOUR.get(r["mk"], ("", ""))[0]
        if want is None:
            bad.append("cat %d %s: tier %d is off the 0..5 ladder" % (r["cat"], r["n"], r["t"]))
        elif have != want and not (r["mk"] == "}" and r["t"] == 4) \
                and not (r["mk"] == '"' and r["t"] == 5):
            bad.append("cat %d %s: tier %d wants colour %r, name carries %r"
                       % (r["cat"], r["n"], r["t"], want, have or "none"))
    return bad


def verify_drops(names_h, drops_h):
    """The drop model against two values recorded independently (134차 §5.1 · NETWORK_ONLY
    §0): 크레이지타이탄 hard per roll = 30 · 30 · 2 · 0.5 · 37.5 %, and 카이저골렘 hard names
    `{레이븐` (cat 0x04 idx 0x15)."""
    bad = []
    want = [30.0, 30.0, 2.0, 0.5, 37.5]
    got = [round(p * 100, 1) for p in per_roll(drops_h[141])]
    if names_h[141][0] != "크레이지타이탄" or got != want:
        bad.append("species 141 %s per-roll %r != recorded %r" % (names_h[141][0], got, want))
    d = drops_h[29]
    if names_h[29][0] != "카이저골렘" or (d["cat"], d["idx"]) != (0x04, 0x15):
        bad.append("species 29 %s names %#x/%#x, recorded 0x04/0x15"
                   % (names_h[29][0], d["cat"], d["idx"]))
    return bad


# -- sources -------------------------------------------------------------------

def pct_text(p):
    v = p * 100
    if v >= 10:
        return "%.0f%%" % v
    if v >= 1:
        return "%.1f%%" % v
    if v >= 0.1:
        return "%.2f%%" % v
    return "%.3f%%" % v


def build(zip_path, want_check):
    jar = open_jar(zip_path)

    def dat(n):
        return jar.read("res/dat/%s" % n)

    equip = []
    for cat in EQUIP_CATS + [ACC_CAT]:
        equip += equip_rows(cat, dat("i%d_dat" % cat))

    taillen = {13: 5, 14: 0, 16: 4, 17: 0, 18: 4}
    groups = {c: consum_rows(c, dat("i%d_dat" % c), taillen[c]) for c in CONSUM_CATS}
    sep = {}
    for c, rows in groups.items():
        keep = [r for r in rows if not r["n"].startswith("(아래부터")]
        sep[c] = len(rows) - len(keep)
        groups[c] = keep
    recipes = recipe_rows(jar)

    rules = load_rules()
    set_of, sets, set_need = load_sets()
    cash = load_cash()
    tiers = GL.legacy_tiers()
    third = {a[0]: a[6] for a in AO.ACC}

    names_n, names_h = species_table(jar, False), species_table(jar, True)
    drops_n, drops_h = drop_table(jar, False), drop_table(jar, True)
    if len(names_n) != len(drops_n) or len(names_h) != len(drops_h):
        raise SystemExit("species / drop record counts disagree")

    bad = verify(equip) + verify_drops(names_h, drops_h)

    # name lookup for every (cat, idx) the page may mention
    by_key = {}
    for r in equip:
        by_key[(r["cat"], r["idx"])] = r
    for c, rows in groups.items():
        for r in rows:
            by_key[(c, r["idx"])] = r
    for r in recipes:
        by_key[(RECIPE_CAT, r["idx"])] = {"n": r["n"] + " 조합서", "mk": r["mk"], "cat": RECIPE_CAT, "idx": r["idx"]}

    def label(key):
        r = by_key.get(tuple(key))
        return [r["n"], r["mk"]] if r else ["cat %02x/%02x" % tuple(key), ""]

    # ---- the new (since BASE_RELEASE) and the changed rows
    legacy_keys = {(c, i) for c, i, _t in rules["items"]}
    legacy_tier = {(c, i): t for c, i, t in rules["items"]}
    hell_recipe_keys = {(c, i) for c, i, _t in rules["recipes"]}
    recipe_tier = {(c, i): t for c, i, t in rules["recipes"]}
    quotes = {(r["cat"], r["idx"]) for r in equip if r["mk"] == '"'}
    if quotes != legacy_keys:
        bad.append("HELL_ITEMS (%d) != the asset's \" rows (%d)" % (len(legacy_keys), len(quotes)))
    for r in equip:
        k = (r["cat"], r["idx"])
        if k in legacy_keys and tiers.get((r["cat"], r["n"])) != legacy_tier[k]:
            bad.append("%s: HELL_ITEMS tier %d, gen_legacy_items %r"
                       % (r["n"], legacy_tier[k], tiers.get((r["cat"], r["n"]))))
    for k in hell_recipe_keys:
        r = recipes[k[1]]
        if tuple(r["out"]) not in legacy_keys:
            bad.append("조합서 %#x makes %r, not a new item" % (k[1], r["out"]))
    if bad:
        for m in bad:
            print("VERIFY: " + m, file=sys.stderr)
        raise SystemExit("the decode no longer reproduces recorded values")

    changed = {tuple(k) for k in rules["reaver"]["items"]} | {(c, i) for _s, c, i in rules["reaver"]["acc"]}

    # ---- stock drops: per species, per difficulty
    src = collections.defaultdict(list)          # (cat, idx) -> [[kind, text, title, weight]]
    stock = collections.defaultdict(list)        # (cat, idx) -> [(pct, species, mode)]
    mobs = []
    host = rules["host"]
    hell = rules["hell"]
    rv = rules["reaver"]
    named_set = set(rules["named"])
    acc_of = {s: (c, i) for s, c, i in rv["acc"]}
    n85 = sum(1 for _c, _i, t in rules["items"] if t < hell["BOSS_TIER_REQ"])
    n90 = len(rules["items"]) - n85
    # ★ 156차 (user 3): the pools keep their tiers, but an accessory is 요구 Lv 1 — the pages say
    # 「신규 장신구」 for it and count 85제/90제 over equipment only.
    a85 = sum(1 for c, _i, t in rules["items"] if c == ACC_CAT and t < hell["BOSS_TIER_REQ"])
    a90 = sum(1 for c, _i, t in rules["items"] if c == ACC_CAT) - a85
    e85, e90 = n85 - a85, n90 - a90

    for sid in range(len(names_h)):
        mname, named, lv_h, rolls_h, mmark = names_h[sid]
        _n2, _nn, lv_n, rolls_n, _m2 = names_n[sid]
        mob = {"id": sid, "n": mname, "mk": mmark, "nm": int(named), "lv": [lv_n, lv_h],
               "hl": hell_level(lv_h, host), "r": [rolls_n, rolls_h], "d": [], "x": []}
        for mode, (dr, rolls) in enumerate(((drops_n[sid], rolls_n), (drops_h[sid], rolls_h))):
            pw, pa, ps, pn, pc = per_roll(dr)
            cell = {"w": per_kill(pw, rolls), "a": per_kill(pa, rolls), "c": per_kill(pc, rolls)}
            if ps > 0:
                k = (ACC_CAT, dr["sidx"])
                cell["s"] = label(k) + [per_kill(ps, rolls), list(k)]
                stock[k].append((per_kill(ps, rolls), mname, mode, "혼"))
            if pn > 0 and dr["cat"] != 0xFF:
                k = (dr["cat"], dr["idx"])
                cell["nd"] = label(k) + [per_kill(pn, rolls), list(k)]
                stock[k].append((per_kill(pn, rolls), mname, mode, "지정"))
            mob["d"].append(cell)
        # the app's own drops (once per kill, independent of the roll count)
        hl = mob["hl"]
        if sid in rv["bosses"]:
            mob["x"].append(["hard", "리버 무기 %d%%" % rv["rate"], "하드코어: 리버 무기 8종 중 하나"])
        if sid in acc_of:
            nm_ = label(acc_of[sid])[0]
            mob["x"].append(["hard", "%s %d%%" % (nm_, rv["rate"]), "하드코어·헬 공통"])
        if hell["HELL_LV_MIN"] <= hl <= rv["hell_max"]:
            mob["x"].append(["hell", "리버 무기 %d%%" % rv["rate"], "헬: 리버 무기 8종 중 하나"])
        if hell["RECIPE_LV_MIN"] <= hl <= hell["RECIPE_LV_MAX"]:
            mob["x"].append(["hell", "신규 조합서 %d%%" % hell["RATE_PCT"], "헬: 신규 조합서 %d종 중 하나" % len(rules["recipes"])])
        if hl >= hell["ITEM_LV_MIN"]:
            boss_tier = (named or sid in named_set) and hl >= hell["NAMED_LV_MIN"]
            if boss_tier:
                mob["x"].append(["hell", "신규 90제 %d%%" % hell["RATE_PCT"],
                                 "헬 네임드: 90제 풀 %d종(장비 %d · 신규 장신구 %d) 중 하나" % (n90, e90, a90)])
            else:
                mob["x"].append(["hell", "신규 85제 %d%%" % hell["RATE_PCT"],
                                 "헬: 85제 풀 %d종(장비 %d · 신규 장신구 %d) 중 하나" % (n85, e85, a85)])
        if hl >= hell["SHARD_LV_MIN"]:
            mob["x"].append(["hell", "닉스코어파편 %d%%" % hell["RATE_PCT"], "헬: 닉스코어파편 1개"])
        mobs.append(mob)

    # stock drops -> item sources (best first)
    for k, lst in stock.items():
        lst.sort(key=lambda x: -x[0])
        best = lst[0]
        mode_ko = ["노말", "하드"]
        title = " / ".join("%s %s %s" % (nm_, mode_ko[m], pct_text(p)) for p, nm_, m, _w in lst)
        text = "%s(%s) %s" % (best[1], mode_ko[best[2]], pct_text(best[0]))
        if len(lst) > 1:
            text += " 외 %d" % (len(lst) - 1)
        src[k].append(["drop", text, "몬스터 처치 시 (한 마리당): " + title])

    # generic drops (picker 0x38cf4) and the common material slot (0x38e28)
    for r in equip:
        if r["cat"] != ACC_CAT and r["t"] <= GENERIC_TIER_MAX and 1 <= r["lv"] <= GENERIC_LEVEL_MAX:
            src[(r["cat"], r["idx"])].append(["gen", "일반 드랍", "몬스터의 무기·방어구 칸에서 레벨대에 맞는 등급 3 이하 장비가 무작위로 나온다"])
    for (lo, hi), share in MAT_GROUPS:
        for i in range(lo, hi + 1):
            src[(MAT_CAT, i)].append(["gen", "공통 재료", "모든 몬스터의 재료 칸 — 재료가 나오면 %d/10 확률로 이 묶음(%d종) 중 하나" % (share, hi - lo + 1)])

    # smithing, scripted gets, cash shop
    for r in recipes:
        out = tuple(r["out"])
        src[out].append(["smith", "조합", "대장장이 특수조합: %s 조합서" % r["n"]])
    for c, i in [(x[0], x[1]) for x in h3crypt.records(h3crypt.decrypt_member(jar, "res/dat/getitem_dat"))]:
        k = (c, i)
        if not any(s[0] == "event" for s in src[k]):
            src[k].append(["event", "퀘스트·이벤트", "퀘스트·이벤트 진행 중 지급"])
    for k in cash:
        src[k].append(["cash", "캐시샵", "인게임 캐시샵(네트워크 메뉴) 판매"])

    # the app's own drops: hard bosses, hell bands, the tower exit reward
    for k in rv["items"]:
        k = tuple(k)
        src[k].append(["hard", "하드 보스 8종 %d%%" % rv["rate"],
                       "하드코어: 보스 8종(%s) 처치마다 %d%% 로 리버 8종 중 하나" % (
                           ", ".join(names_h[s][0] for s in rv["bosses"]), rv["rate"])])
        src[k].append(["hell", "헬 Lv%d~%d %d%%" % (hell["HELL_LV_MIN"], rv["hell_max"], rv["rate"]),
                       "헬: Lv%d~%d 몬스터 처치마다 %d%% 로 리버 8종 중 하나" % (hell["HELL_LV_MIN"], rv["hell_max"], rv["rate"])])
    for s, (c, i) in acc_of.items():
        src[(c, i)].append(["hard", "%s %d%%" % (names_h[s][0], rv["rate"]),
                            "하드코어·헬: %s 처치마다 %d%%" % (names_h[s][0], rv["rate"])])
    for c, i, t in rules["items"]:
        if t >= hell["BOSS_TIER_REQ"]:
            src[(c, i)].append(["hell", "헬 네임드 Lv%d+ %d%%" % (hell["NAMED_LV_MIN"], hell["RATE_PCT"]),
                                "헬: 네임드(붉은 이름) 몬스터 Lv%d 이상 처치마다 %d%% 로 90제 풀 %d종(장비 %d · 신규 장신구 %d) 중 하나" % (hell["NAMED_LV_MIN"], hell["RATE_PCT"], n90, e90, a90)])
        else:
            src[(c, i)].append(["hell", "헬 Lv%d+ %d%%" % (hell["ITEM_LV_MIN"], hell["RATE_PCT"]),
                                "헬: Lv%d 이상 몬스터 처치마다 %d%% 로 85제 풀 %d종(장비 %d · 신규 장신구 %d) 중 하나 (네임드 Lv%d+ 제외)" % (hell["ITEM_LV_MIN"], hell["RATE_PCT"], n85, e85, a85, hell["NAMED_LV_MIN"])])
    for c, i, t in rules["recipes"]:
        src[(c, i)].append(["hell", "헬 Lv%d~%d %d%%" % (hell["RECIPE_LV_MIN"], hell["RECIPE_LV_MAX"], hell["RATE_PCT"]),
                            "헬: Lv%d~%d 몬스터 처치마다 %d%% 로 신규 조합서 %d종 중 하나" % (hell["RECIPE_LV_MIN"], hell["RECIPE_LV_MAX"], hell["RATE_PCT"], len(rules["recipes"]))])
    shard = tuple(rules["shard"])
    src[shard].append(["hell", "헬 Lv%d+ %d%%" % (hell["SHARD_LV_MIN"], hell["RATE_PCT"]),
                       "헬: Lv%d 이상 몬스터 처치마다 %d%% 로 1개" % (hell["SHARD_LV_MIN"], hell["RATE_PCT"])])

    tw = rules["tower"]

    def floors(pred):
        f = [t["boss"] for t in tw if pred(t)]
        return f

    def span(f):
        if not f:
            return ""
        return "%d층" % f[0] if len(f) == 1 else "%d~%d층" % (f[0], f[-1])

    src[shard].append(["tower", "하늘성 보상 %d~%d개" % (tw[0]["shards"], tw[-1]["shards"]),
                       "하늘성 퇴장 보상: 클리어한 최고 보스층에 따라 " + ", ".join("%d층 %d개" % (t["boss"], t["shards"]) for t in tw)])
    for c, i, t in rules["recipes"]:
        f = floors(lambda x, t=t: x["recipe"][0] <= t <= x["recipe"][1])
        src[(c, i)].append(["tower", "하늘성 %s 보상" % span(f), "하늘성 퇴장 보상: 최고 보스층 %s — 조합서 하나 (%d제 조합서 중 무작위)" % (span(f), t)])
    eg = [t for t in tw if t["endgame"]]
    for c, i, t in rules["items"]:
        f = floors(lambda x, t=t: x["legacy"] and x["legacy"][0] <= t <= x["legacy"][1])
        if f:
            src[(c, i)].append(["tower", "하늘성 %d층+ 보상" % f[0],"하늘성 퇴장 보상: 최고 보스층 %d층 이상 — %d제 실물 하나 (무작위)" % (f[0], t)])
        if t >= host["ENDGAME_MIN_LV"] and eg:
            src[(c, i)].append(["tower", "하늘성 %s 확률" % "/".join(str(x["boss"]) for x in eg),
                                "하늘성 퇴장 보상: 최고 보스층 " + ", ".join("%d층 %d%%" % (x["boss"], x["endgame"]) for x in eg) + " 확률로 90제 하나 추가"])

    # ---- decorate rows
    icons = frames_to_datauris(decode_bm(jar.read("res/comm/item_bm")))
    for r in equip:
        k = (r["cat"], r["idx"])
        r["ico"] = icon_frame(r["cat"], r["idx"], r)
        if k in legacy_keys:
            r["new"] = 1
            r["lg"] = legacy_tier[k]
        if k in changed:
            r["chg"] = 1
        if k in set_of:
            r["sid"] = set_of[k]
        if r["cat"] == ACC_CAT and r["n"] in third:
            r["o3"] = third[r["n"]]
        r["src"] = src.get(k, [])
    for c, rows in groups.items():
        for r in rows:
            r["ico"] = icon_frame(c, r["idx"], r)
            r["src"] = src.get((c, r["idx"]), [])
    for r in recipes:
        k = (RECIPE_CAT, r["idx"])
        o = by_key.get(tuple(r["out"]))
        r["on"] = [o["n"], o.get("mk", ""), o.get("t", 0)] if o else ["?", "", 0]
        r["oi"] = o.get("ico", r["out"][0]) if o else r["out"][0]
        r["bn"] = label(r["base"]) if r["base"] else None
        r["mn"] = [label((MAT_CAT, m[0])) + [m[1]] for m in r["mats"]]
        r["src"] = src.get(k, [])
        if k in hell_recipe_keys:
            r["new"] = 1
            r["lg"] = recipe_tier[k]
        r["ico"] = RECIPE_CAT

    for cat in set(r["cat"] for r in equip):
        seen = {}
        for r in equip:
            if r["cat"] == cat:
                seen.setdefault(r["n"], []).append(r)
        for name, rs in seen.items():
            if len(rs) > 1:
                top = max(rs, key=lambda r: r["idx"])
                for r in rs:
                    r["tw"] = 2 if r is top else 1

    tcol = {}
    for t, c in TIER_COLOUR.items():
        hx, ch = "#B9BCC6", ""
        for m, (nm, h) in COLOUR.items():
            if nm == c:
                hx, ch = h, m
        tcol["%d" % t] = [COLOUR_KO[c], hx, ch]

    hell_bands = {
        "lv": [hell["HELL_LV_MIN"], hell["HELL_LV_MAX"]],
        "spawn": [host["HELL_SPAWN_LV_MIN"], host["HELL_SPAWN_LV_MAX"]],
        "rate": hell["RATE_PCT"], "recipe": [hell["RECIPE_LV_MIN"], hell["RECIPE_LV_MAX"]],
        "item": hell["ITEM_LV_MIN"], "shard": hell["SHARD_LV_MIN"], "named": hell["NAMED_LV_MIN"],
        "reaver": [hell["HELL_LV_MIN"], rv["hell_max"]], "reaver_rate": rv["rate"],
        "n85": n85, "n90": n90, "e85": e85, "e90": e90, "a85": a85, "a90": a90, "nrec": len(rules["recipes"]),
        "bosses": [names_h[s][0] for s in rv["bosses"]],
        "acc": [[names_h[s][0], label((c, i))[0]] for s, c, i in rv["acc"]],
        "named_species": [names_h[s][0] for s in rules["named"]],
    }

    data = {
        "ver": app_version(), "base": BASE_RELEASE,
        "equip": equip,
        "orbs": groups[16], "scrolls": groups[13], "consum": groups[18],
        "mats": groups[14], "recipes": recipes, "misc": groups[17], "mobs": mobs,
        "icons": icons,
        "stat": {"%d" % k: v for k, v in ingame_labels(jar).items()},
        "noarm": sorted(NO_ARM), "host": sorted(HOST_STATS),
        "slot": {"%d" % k: v for k, v in SLOT.items()},
        "tcol": tcol,
        "colour": {m: [nm, hx] for m, (nm, hx) in COLOUR.items()},
        "tgt": {"%d" % k: v for k, v in SCROLL_TARGET.items()},
        "fuse": ORB_FUSE,
        "melee": sorted(MELEE), "ranged": sorted(RANGED),
        "sep": {"%d" % k: v for k, v in sep.items()},
        "sets": sets, "setneed": set_need,
        "setinfo": [[sum(1 for v in set_of.values() if v == i),
                     max([r["lg"] for r in equip if r.get("sid") == i] or [0])] for i in range(len(sets))],
        "hell": hell_bands, "tower": tw,
        "news": {"items": len(legacy_keys), "i85": n85, "i90": n90, "e85": e85, "e90": e90, "acc": a85 + a90,
                 "recipes": len(hell_recipe_keys)},
    }
    other = sum(len(v) for v in groups.values())
    print("decode OK - %d equipment + %d other + %d 조합서 = %d items, %d species, %d icons; "
          "new since %s: %d items + %d 조합서"
          % (len(equip), other, len(recipes), len(equip) + other + len(recipes), len(mobs),
             len(icons), BASE_RELEASE, len(legacy_keys), len(hell_recipe_keys)))
    return None if want_check else data


# -- the plain new-items page (`new/index.html`) ---------------------------------------
#
# The user (2026-10-02): «신규아이템 보기가 너무힘든데 세트별로 묶어서 그냥 다른 구사여미 필요없고
# 효과랑 드랍 테이블만 적어줘». One static page, no script: the drop table once at the top, then
# the 78 rows grouped by set (`set_items.rs`), the rest by tier and kind.

SET_SLOT_ORDER = {0: 0, 1: 1, 2: 2, 3: 3, 11: 4, 12: 5}
# ★ 155차 (user 2, 2026-10-02): an accessory is 요구 Lv 1 since 150차, so 85제/90제 means nothing on
# it — the page calls every new accessory 「신규 장신구」 (row label, one section, no tier tag) and
# counts the tiers over equipment only. The drop POOLS still split by tier (`legacy_tiers`) — the
# drop table says so, with the accessory split per pool; that is the fact, not a page choice.
SLOT_WORD = {**SLOT, 12: "신규 장신구"}
ACC_WORD = SLOT_WORD[ACC_CAT]


def stat_text(stat, i, v):
    lab, pct = stat.get("%d" % i, ["id 0x%x" % i, 0])
    if not v:
        return re.sub(r"\s*[+-]$", "", lab)
    lab = lab if lab.endswith(("+", "-")) else lab + " +"
    return "%s%d%s" % (lab, v, "%" if pct else "")


def item_effects(stat, r):
    parts = []
    if r["main"]:
        parts.append(("공 %d" if 4 <= r["cat"] <= 10 else "방 %d") % r["main"])
    parts += [stat_text(stat, p["i"], p["v"]) for p in r["o"]]
    if r.get("o3"):
        parts.append(r["o3"])
    return " · ".join(parts)


def new_items_body(data):
    from html import escape as E
    stat, hell, tower = data["stat"], data["hell"], data["tower"]
    host = {"HELL_SPAWN_LV_MIN": hell["spawn"][0], "HELL_SPAWN_LV_MAX": hell["spawn"][1],
            "HELL_LV_MIN": hell["lv"][0], "HELL_LV_MAX": hell["lv"][1]}

    def first_spawn(want):
        return next(lv for lv in range(hell["spawn"][0], hell["spawn"][1] + 1)
                    if hell_level(lv, host) >= want)

    news = data["news"]
    made = {tuple(r["out"]): r for r in data["recipes"] if r.get("new")}
    rows = [r for r in data["equip"] if r.get("new")]

    def floors(pred):
        f = [t["boss"] for t in tower if pred(t)]
        return "%d~%d층" % (f[0], f[-1]) if len(f) > 1 else "%d층" % f[0]

    legacy_from = next(t["boss"] for t in tower if t["legacy"])
    endgame = " · ".join("%d층 %d%%" % (t["boss"], t["endgame"]) for t in tower if t["endgame"])
    rec85 = floors(lambda t: t["recipe"][0] <= 85 <= t["recipe"][1])
    rec90 = floors(lambda t: t["recipe"][0] <= 90 <= t["recipe"][1])
    rate = hell["rate"]
    # 155차: the tiers count equipment only; the accessories are one 「신규 장신구」 group.
    acc_rows = [r for r in rows if r["cat"] == ACC_CAT]
    e85 = sum(1 for r in rows if r["cat"] != ACC_CAT and r["lg"] == 85)
    e90 = sum(1 for r in rows if r["cat"] != ACC_CAT and r["lg"] == 90)
    a85 = sum(1 for r in acc_rows if r["lg"] == 85)
    a90 = sum(1 for r in acc_rows if r["lg"] == 90)
    if ((e85 + a85, e90 + a90, e85 + e90 + len(acc_rows)) != (news["i85"], news["i90"], news["items"])
            or (e85, e90, len(acc_rows)) != (news["e85"], news["e90"], news["acc"])):
        raise SystemExit("new-items page: tier counts %d+%d / %d+%d do not add up to %s"
                         % (e85, a85, e90, a90, news))
    out = []
    out.append("<h1>신규 아이템</h1>")
    # 155차 (user 6): no 「전체 도감」 link on this page.
    out.append('<p class="meta">영웅서기3 개정판 v%s · %d종 (85제 %d · 90제 %d · %s %d)</p>'
               % (E(data["ver"]), news["items"], e85, e90, E(ACC_WORD), len(acc_rows)))

    out.append("<h2>드랍표</h2>")
    out.append('<div class="tbl"><table class="drops"><thead><tr><th>입수처</th><th>85제 (%d종)</th>'
               '<th>90제 (%d종)</th></tr></thead><tbody>' % (news["i85"], news["i90"]))
    pool85 = "%d종(장비 %d · %s %d) 중 하나" % (news["i85"], e85, ACC_WORD, a85)
    pool90 = "%d종(장비 %d · %s %d) 중 하나" % (news["i90"], e90, ACC_WORD, a90)
    out.append("<tr><td>헬모드 처치</td><td data-l=\"85제\">Lv%d 이상 몬스터 %d%% · %s</td>"
               "<td data-l=\"90제\">네임드 Lv%d 이상 %d%% · %s</td></tr>"
               % (hell["item"], rate, E(pool85), hell["named"], rate, E(pool90)))
    out.append("<tr><td>하늘성 퇴장 보상</td><td data-l=\"85제\">최고 보스층 %d층 이상이면 1개</td><td data-l=\"90제\">최고 보스층 %s 확률로 1개</td></tr>"
               % (legacy_from, endgame))
    out.append("<tr><td>특수조합</td><td data-l=\"85제\">조합서 + 닉스코어파편</td><td data-l=\"90제\">조합서 + 베이스 장비 + 닉스코어파편</td></tr>")
    out.append("<tr><td>조합서</td><td data-l=\"85제\">헬 Lv%d~%d 처치 %d%% · 하늘성 %s 퇴장 보상</td>"
               "<td data-l=\"90제\">헬 Lv%d~%d 처치 %d%% · 하늘성 %s 퇴장 보상</td></tr>"
               % (hell["recipe"][0], hell["recipe"][1], rate, rec85, hell["recipe"][0], hell["recipe"][1], rate, rec90))
    out.append('<tr><td>닉스코어파편</td><td colspan="2">헬 Lv%d 이상 처치 %d%% · 하늘성 퇴장 보상 %d~%d개</td></tr>'
               % (hell["shard"], rate, tower[0]["shards"], tower[-1]["shards"]))
    out.append('<tr><td>%s</td><td colspan="2">%d종 — %d종은 Lv%d 이상 몬스터 풀(85제 열) · %d종은 네임드 Lv%d 이상 풀(90제 열) · '
               '하늘성 퇴장 보상도 같은 띠. 전부 요구 Lv1</td></tr>'
               % (E(ACC_WORD), len(acc_rows), a85, hell["item"], a90, hell["named"]))
    out.append("</tbody></table></div>")
    out.append('<p class="note">헬모드에서는 하드코어 몬스터가 Lv%d~%d 로 올라간다 (하드 Lv%d 이상 = 헬 Lv%d 이상, '
               '하드 Lv%d 이상 = 헬 Lv%d 이상). 네임드는 붉은 이름 몬스터와 보스.</p>'
               % (hell["lv"][0], hell["lv"][1], first_spawn(hell["item"]), hell["item"],
                  first_spawn(hell["named"]), hell["named"]))

    def table(items, group_tier):
        h = ['<div class="tbl"><table class="items"><thead><tr><th>아이템</th><th>효과</th><th>조합</th></tr></thead><tbody>']
        for r in items:
            rc = made.get((r["cat"], r["idx"]))
            mats = []
            if rc:
                if rc["bn"]:
                    mats.append("%s +" % E(rc["bn"][0]))
                mats += ["%s %d개" % (E(m[0]), m[2]) for m in rc["mn"]]
            tag = ('<span class="tag">%d제</span>' % r["lg"]
                   if group_tier is not None and r["lg"] != group_tier and r["cat"] != ACC_CAT else "")
            h.append('<tr><td class="nm"><b>%s</b>%s<span>%s · Lv%d</span></td><td class="fx">%s</td><td class="mk">%s</td></tr>'
                     % (E(r["n"]), tag, E(SLOT_WORD[r["cat"]]), r["lv"], E(item_effects(stat, r)),
                        " ".join(mats) or "&mdash;"))
        h.append("</tbody></table></div>")
        return "".join(h)

    by_set = collections.defaultdict(list)
    rest = collections.defaultdict(list)
    for r in rows:
        if "sid" in r:
            by_set[r["sid"]].append(r)
        elif r["cat"] == ACC_CAT:
            rest[(None, ACC_WORD)].append(r)   # 155차: one section, no tier
        else:
            kind = "무기" if 4 <= r["cat"] <= 10 else "방어구 · 방패"
            rest[(r["lg"], kind)].append(r)
    for v in by_set.values():
        v.sort(key=lambda r: (SET_SLOT_ORDER.get(r["cat"], 9), r["idx"]))
    for v in rest.values():
        v.sort(key=lambda r: (r["cat"], r["idx"]))

    def set_tier(items):
        """A set's tier is its EQUIPMENT's (a set accessory is 요구 Lv 1 and may be the other tier —
        레아의목걸이 in 붉은마녀, 이안의맹세 in 빙해)."""
        return max(r["lg"] for r in items if r["cat"] != ACC_CAT)

    def set_section(sid):
        name, bonus, text = data["sets"][sid]
        items = by_set[sid]
        tier = set_tier(items)
        return ('<h2>%s 세트<small>%d제 · %d부위</small></h2>'
                '<p class="setfx">%d부위 이상 착용: <b>%s</b> · %s</p>%s'
                % (E(name), tier, len(items), data["setneed"], E(bonus), E(text), table(items, tier)))

    def rest_section(tier, kind):
        items = rest.get((tier, kind))
        if not items:
            return ""
        if tier is None:
            label = "%s (세트 없음)" % kind
        else:
            label = "%d제 %s" % (tier, kind) + (" (세트 없음)" if kind != "무기" else "")
        return '<h2>%s<small>%d종</small></h2>%s' % (E(label), len(items), table(items, tier))

    for tier in (85, 90):
        sids = sorted(s for s, v in by_set.items() if set_tier(v) == tier)
        for s in sids:
            out.append(set_section(s))
        for kind in ("무기", "방어구 · 방패"):
            out.append(rest_section(tier, kind))
    out.append(rest_section(None, ACC_WORD))   # 155차: the accessories close the page, as one group
    shown = sum(len(v) for v in by_set.values()) + sum(len(v) for v in rest.values())
    if shown != news["items"]:
        raise SystemExit("new-items page lists %d rows, the data has %d" % (shown, news["items"]))
    return "\n".join(x for x in out if x)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--out", default=DEFAULT_OUT)
    ap.add_argument("--ver", help="the version label to print (default: android_app/version.properties)")
    ap.add_argument("--check", action="store_true", help="decode and verify only; write nothing")
    a = ap.parse_args()

    data = build(a.zip, a.check)
    if data is None:
        return 0
    if a.ver:
        data["ver"] = a.ver

    tpl = open(os.path.join(HERE, "item_codex.html"), encoding="utf-8").read()
    blob = json.dumps(data, ensure_ascii=False, separators=(",", ":"))
    if "/*DATA*/" not in tpl:
        raise SystemExit("template has no /*DATA*/ slot")
    # `</` inside a JSON string would close the <script> element early.
    blob = blob.replace("</", "<\\/")
    html = tpl.replace("/*DATA*/", blob).replace("{{VER}}", data["ver"]).replace("{{BASE}}", data["base"])

    os.makedirs(a.out, exist_ok=True)
    path = os.path.join(a.out, "index.html")
    with open(path, "w", encoding="utf-8", newline="\n") as fp:
        fp.write(html)
    print("wrote %s (%.1f KB)" % (path, os.path.getsize(path) / 1024.0))

    ntpl = open(os.path.join(HERE, "item_codex_new.html"), encoding="utf-8").read()
    page = (ntpl.replace("{{BODY}}", new_items_body(data)).replace("{{VER}}", data["ver"])
            .replace("{{N}}", str(data["news"]["items"])))
    os.makedirs(os.path.join(a.out, "new"), exist_ok=True)
    npath = os.path.join(a.out, "new", "index.html")
    with open(npath, "w", encoding="utf-8", newline="\n") as fp:
        fp.write(page)
    print("wrote %s (%.1f KB)" % (npath, os.path.getsize(npath) / 1024.0))
    return 0


if __name__ == "__main__":
    sys.exit(main())
