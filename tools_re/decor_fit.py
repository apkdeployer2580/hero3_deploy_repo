"""Fit the field scroll S = screen - world from decoration blits (blit log) vs the map's
decoration records (world px/py, frame size from the bank)."""
import os, re, sys, io, zipfile
from collections import Counter
ROOT = r"C:\hero3NativeAPP"; sys.path.insert(0, os.path.join(ROOT, "tools_re")); os.chdir(ROOT)
import dump_map_catalog as C
import render_maps as RM
log, scene = sys.argv[1], int(sys.argv[2])
jar = RM.open_jar("android_app/app/src/main/assets/hero3_fixed.zip")
maps, scenes = C.load(jar)
sc = scenes[scene]; mid = sc["map"]; m = maps[mid]
decor=[r for L in m["layers"] for r in L if not r["sprite"]]
print("scene", scene, "map", mid, m["name"], m["w"], "x", m["h"], "decor", len(decor))
art = RM.Art(jar)
recs = []
for r in decor:
    f = art.decor(r["id"])
    if f is None: continue
    recs.append((r["id"], r["px"], r["py"], f.width, f.height, r["flags"]))
blits = []
for line in open(log, encoding="utf-8", errors="replace"):
    mm = re.match(r"\[blitlog\]\s+#(\d+) dst=\((-?\d+),(-?\d+)\) (\d+)x(\d+) spr=(0x[0-9a-f]+) src=\((-?\d+),(-?\d+)\)", line)
    if mm: blits.append(tuple(int(x, 16) if i == 5 else int(x, 10) for i, x in enumerate(mm.groups())))
# unclipped decor blits: src=(0,0) and not touching the frame edge
W, H = 496, 240
dec = [b for b in blits if b[6] == 0 and b[7] == 0 and b[1] > 0 and b[2] > 0 and b[1] + b[3] < W and b[2] + b[4] < H and not (b[3] == 16 and b[4] == 16)]
votes = Counter()
for (_, dx, dy, w, h, spr, _, _) in dec:
    for (gid, px, py, fw, fh, fl) in recs:
        if (fw, fh) == (w, h):
            votes[(dx - px, dy - (py - fh))] += 1
print("top S votes (screen - world):", votes.most_common(6))
S = votes.most_common(1)[0][0]
ok = sum(1 for (_, dx, dy, w, h, *_ ) in dec if any((fw, fh) == (w, h) and (dx - px, dy - (py - fh)) == S for (_, px, py, fw, fh, _) in recs))
print("decor blits explained by S:", ok, "of", len(dec))

# unique-size matches: one record of that (w,h) on the whole map and one such blit on screen
from collections import defaultdict
by_size = defaultdict(list)
for r in recs: by_size[(r[3], r[4])].append(r)
bl_by_size = defaultdict(list)
for b in dec: bl_by_size[(b[3], b[4])].append(b)
print("unique-size pairs:")
for sz, rs in sorted(by_size.items()):
    if len(rs) == 1 and len(bl_by_size.get(sz, [])) == 1:
        (gid, px, py, fw, fh, fl), b = rs[0], bl_by_size[sz][0]
        print(f"  {sz} gid {gid} flags {fl:#x} world ({px},{py}) blit dst ({b[1]},{b[2]}) -> S=({b[1]-px},{b[2]-(py-fh)})  [S if py is top: ({b[1]-px},{b[2]-py})]")
# sizes with exactly 2 records and 2 blits: print both pairings' candidates
for sz, rs in sorted(by_size.items()):
    bs = bl_by_size.get(sz, [])
    if len(rs) == 2 and len(bs) == 2:
        print("  pair2", sz, [(r[1], r[2]) for r in rs], [(b[1], b[2]) for b in bs])
