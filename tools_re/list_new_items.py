"""List every NEW item row of the shipped asset (legacy 78 · recipes 78 · shard · reaver 8 ·
boss accessories 2) as JSON, and write the grant file for one save slot."""
import io
import json
import os
import struct
import sys
import zipfile

ROOT = r"C:\hero3NativeAPP"
sys.path.insert(0, os.path.join(ROOT, "tools_re"))
os.chdir(ROOT)
import dump_item_db as D  # noqa: E402
import add_tower_recipes as R  # noqa: E402
import gen_legacy_items as GL  # noqa: E402

OUT = sys.argv[1]
z = zipfile.ZipFile(D.DEFAULT_ZIP)
j = zipfile.ZipFile(io.BytesIO(z.read("0002865E.jar")))

# InGame_txt labels: option id -> label[80 + id]
labels = {}
for line in open(os.path.join(ROOT, "tools_re", "ingame_ids.txt"), encoding="utf-8"):
    parts = line.strip().split(None, 2)
    if len(parts) >= 3 and parts[0].isdigit():
        labels[int(parts[0])] = parts[2].split("   (")[0].strip()
OPT_NAME = {v: k for k, v in GL.OPT_ID.items()}


def opt_label(oid):
    if oid == 0:
        return None
    lab = labels.get(80 + oid, f"id{oid:#x}")
    return lab.replace(" +", "").replace(" -", "").strip()


def opt_text(oid, val):
    if oid == 0:
        return "—"
    v = struct.unpack("<b", bytes([val]))[0]
    unit = "%" if oid in GL.PCT_IDS else ""
    sign = "+" if v >= 0 else ""
    return f"{opt_label(oid)} {sign}{v}{unit}"


SLOT = {**D.SLOT, 12: "악세"}
rows = []
cats = {}
for cat in range(0, 13):
    blob = j.read(f"res/dat/i{cat}_dat")
    recs = list(D.records(blob))
    cats[cat] = recs
    for i, _o, b in recs:
        nl = b[0]
        raw = b[1:1 + nl]
        name = D.euc(raw)
        st = b[1 + nl:]
        colour = D.COLOUR.get(raw[0], ("", ""))[0] if raw else ""
        group = None
        if raw and raw[0] == 0x22:
            group = "legacy"
        elif raw and raw[0] == 0x7B and cat in range(4, 12) and "리버" in name:
            group = "reaver"
        elif cat == 12 and i in (0x28, 0x29):
            group = "bossacc"
        if group is None:
            continue
        price = struct.unpack_from("<I", st, 0)[0]
        if cat == 12:
            grade, lv = st[5], st[6]
            main = struct.unpack_from("<H", st, 10)[0]
            sub = None
            o = st[14:18]
            sprite = pal = None
        else:
            sprite, pal, grade, lv = st[4], st[5], st[7], st[8]
            main = struct.unpack_from("<H", st, 12)[0]
            sub = struct.unpack_from("<H", st, 14)[0]
            o = st[16:20]
        rows.append(dict(group=group, cat=cat, idx=i, slot=SLOT[cat], name=name[1:] if colour else name,
                         colour=colour, lv=lv, grade=grade, main=main, sub=sub,
                         opt1=opt_text(o[0], o[1]), opt2=opt_text(o[2], o[3]), price=price,
                         sprite=sprite, palette=pal))

# recipes: plan order == row order from FIRST
plan = R.plan()
recipes = []
for k, r in enumerate(plan):
    recipes.append(dict(group="recipe", cat=15, idx=R.FIRST + k, result=r["result"], base=r["base"],
                        shards=r["shards"], lv=r["lv"], slot=SLOT[r["cat"]], fee=R.PRICE, rate=R.RATE))

# shard
blob = j.read("res/dat/i14_dat")
for i, _o, b in D.records(blob):
    if i == 0x2E:
        nl = b[0]
        shard = dict(group="material", cat=14, idx=i, name=D.euc(b[1:1 + nl])[1:], price=struct.unpack_from("<I", b, 1 + nl)[0])

data = dict(items=rows, recipes=recipes, shard=shard)
json.dump(data, open(os.path.join(OUT, "new_items.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("legacy", sum(r["group"] == "legacy" for r in rows), "reaver", sum(r["group"] == "reaver" for r in rows),
      "bossacc", sum(r["group"] == "bossacc" for r in rows), "recipes", len(recipes))

# grant file: legacy equipment first (cat order), then recipes, then reaver + boss accessories.
# ⛔ NOT the shard: cat 0x0e is a 재료가방 category — the intake hook diverts it into the box,
# so it never shows in the bag and the grant would hand it out on every poll.
lines = ["# 141차 — 신규 아이템 전부를 SLOT 2 (0-based 1) 의 가방에. 한 폴에 한 줄, 가방이 차면 기다린다.", "slot 1"]
order = [r for r in rows if r["group"] == "legacy"] + [dict(cat=r["cat"], idx=r["idx"], name=r["result"] + " 조합서") for r in recipes] \
    + [r for r in rows if r["group"] in ("reaver", "bossacc")]
for r in order:
    lines.append(f"cat {r['cat']:02x} idx {r['idx']:02x}   # {r['name']}")
open(os.path.join(OUT, "wie_grant.txt"), "w", encoding="utf-8", newline="\n").write("\n".join(lines) + "\n")
print("grant rows", len(order))
