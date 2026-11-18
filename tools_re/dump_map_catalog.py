"""The map / scene catalogue - what places exist, and what lives in them.

Answers the two questions `GUARDIAN_CASTLE_FEASIBILITY.md` listed as Phase A
items 2 and 7 ("enumerate the map ids", "can the spawn table be changed"), and
it answers them from the shipped assets rather than from play, because BOTH
files are plaintext: `res/map/map<id>_mp` and `res/event/e<scene>_scn` are not
among the eight DES members (see `h3crypt.py`).

THE TWO LEVELS, AND WHY THE SCENE IS THE ONE THAT MATTERS

  A MAP is geometry: a name, a tile grid, a collision grid, decoration objects
  and a list of spawn POINTS.  135 of them.
  A SCENE is a map plus the three monster species that fill those points, plus
  a region byte and an intro-event id.  244 of them.

  So the same map is reused by several scenes with different monsters - the game
  itself already does the "recombine existing assets" trick the tower design
  wants (map 6 is scene 7 with species 84/106/128 and scene 8 with 86/110/132).

  The guest's map-move entry point takes a SCENE id, not a map id:

    loadScene(u16 scene)                                        guest 0x15f28
        name = sprintf("res/event/e%02d%02d_scn", scene/100, scene%100)
        scn  = loadResource(name)                       ; 0x1c080
        *(u16*)(base + 0x9e08) = scene          ; NOT the length - see A.8
        mapId = parseScnHeader(scn)                     ; 0x14284, below
        loadMap(sprintf("res/map/map%d_mp", mapId))     ; 0x15edc -> 0x151bc
        free(scn)

  Its ONLY caller is the warp handler at 0x3f280, which passes the byte at
  `base + 0x9e15`.  That is a FIXED address, not a field of some warp object the
  player stepped on (the handler's prologue sets `r7 = getBase() + 0x9e08`), so a
  warp is a REQUEST anyone can post - and the host posts one by writing two bytes
  rather than calling this function.  Calling it directly loads all the data
  correctly but leaves the world undrawn; see `guest_anchors::SceneWorld`.

THE SCN HEADER - 9 BYTES, AND THREE OF THEM ARE THE MONSTER TABLE

  parseScnHeader (guest 0x14284) reads exactly nine bytes and returns [0]:

    [0] map id            -> *(u32*)(base + 0x9e1c)   (= CHAR + 0x9e44)
    [1] always 0          -> *(u32*)(base + 0x9e34)
    [2] 0 / 1 / 2         -> *(u8 *)(base + 0x9e38)
    [3] [4] [5] species   -> *(u32*)(base + 0x9e3c + i*4)   <-- THE SPAWN TABLE
    [6] region            -> *(u32*)(base + 0x9e48)
    [7] ?                 -> *(u32*)(base + 0x9e54)
    [8] intro event       -> *(u32*)(base + 0x9e58); if != 0xff calls 0x3c2e0

  A map's spawn point stores a TYPE of 0, 1 or 2 - an index into that triple -
  so the species a floor is populated with is THREE WORDS of guest memory, at
  CHAR+0x9e64 / +0x9e68 / +0x9e6c, and nothing else.  0xff = slot unused.
  Species >= 161 is not a row of `enemy_dat` (which has exactly 161) but a boss:
  `res/boss/boss_dat` index `species - 161`, and that file has exactly 15 rows.

THE MAP FILE - the parse order below IS guest 0x151bc, field by field

    [u8  n_theme] [u8 x n_theme]        -> MAP+0x00, MAP+0x04     tile-set ids
    [u8  n]       [u8 x n]              blob 0 (one byte, always 'A')
    [u8  n]       [u8 x n]              blob 1 = THE MAP NAME (ASCII)
    [u8  flag]                          -> MAP+0x0c
    [u8  w] [u8 h]                      -> MAP+0x48, MAP+0x4c ; MAP+0x50/0x52 = *16
    [u8  n_obj]   [u8 x n_obj]          -> MAP+0x28, list at MAP+0x14   object ids
    [u8  n_npc]   [u8 x n_npc]          -> MAP+0x2c, array at MAP+0x20  sprite ids
    [u8  tile[w*h]]                     -> MAP+0x54 / MAP+0x58
    [u8  attr[w*h]]                     -> MAP+0x5c / MAP+0x60   bit 7 = blocked
    [u8  n0] [rec6 x n0]                -> MAP+0x78, list at MAP+0x9c   layer 0
    [u8  n1] [rec6 x n1]                -> MAP+0x7c, list at MAP+0x8c   layer 2
    [u8  n2] [rec6 x n2]                            list at MAP+0xac    layer ?
    [u8  n3] [u8 x n3]                  -> MAP+0x30, list at MAP+0x3c   spawn kinds
    [u8  n4] [[type][x][y] x n4]        -> MAP+0x34 + four parallel arrays
    == end of file

  rec6 = [flags][id][u16 px][u16 py], coordinates in PIXELS (w*16 by h*16).
  `flags & 0x20` picks which table `id` indexes: set -> the sprite list
  (guest 0x149b4), clear -> the object list (guest 0x14d34, which is handed
  MAP+0x14 and MAP+0x28).  The event array (MAP+0x64/0x68, i8[w*h], the one the
  minimap paints as a warp tile) is allocated and ZEROED here and filled at
  runtime - it is not in the file.

  Nothing reads the file cursor after the spawn list, so "parses to exactly EOF"
  is a real check on this whole model, and `--check` runs it on all 135 maps.

Usage:
    python tools_re/dump_map_catalog.py [--zip <asset.zip>] [--check]
                                        [--maps] [--scenes] [--by-level]
                                        [--map N] [--scene N] [--json <file>]

    (Windows: prefix PYTHONUTF8=1, the console is cp949 - same trap as
     hero3_server/selftest.py.)
"""
import argparse
import collections
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from h3crypt import open_assets, records, DEFAULT_ZIP  # noqa: E402

MAP_PREFIX = "res/map/map"
MAP_SUFFIX = "_mp"
SCN_PREFIX = "res/event/e"
SCN_SUFFIX = "_scn"
ATTR_BLOCKED = 0x80
SPECIES_NONE = 0xFF
BOSS_BASE = 161          # species >= this indexes res/boss/boss_dat instead
OBJ_ID_IS_SPRITE = 0x20  # rec6 flags bit 5 -> id indexes the sprite list


def parse_map(d):
    """The guest's own parse order (0x151bc). Raises on a short file."""
    c = 0

    def u8():
        nonlocal c
        v = d[c]
        c += 1
        return v

    def blob():
        nonlocal c
        n = u8()
        b = d[c:c + n]
        c += n
        if len(b) != n:
            raise ValueError("truncated blob")
        return b

    theme = blob()
    tag = blob()
    name = blob()
    flag = u8()
    w = u8()
    h = u8()
    obj = blob()
    npc = blob()
    tile = d[c:c + w * h]
    c += w * h
    attr = d[c:c + w * h]
    c += w * h
    if len(tile) != w * h or len(attr) != w * h:
        raise ValueError("truncated layer")
    layers = []
    for _ in range(3):
        n = u8()
        recs = []
        for _ in range(n):
            r = d[c:c + 6]
            c += 6
            if len(r) != 6:
                raise ValueError("truncated rec6")
            recs.append(dict(flags=r[0], id=r[1],
                             px=r[2] | (r[3] << 8), py=r[4] | (r[5] << 8),
                             sprite=bool(r[0] & OBJ_ID_IS_SPRITE)))
        layers.append(recs)
    kinds = list(blob())
    n = u8()
    spawns = []
    for _ in range(n):
        r = d[c:c + 3]
        c += 3
        if len(r) != 3:
            raise ValueError("truncated spawn")
        spawns.append((r[0], r[1], r[2]))
    return dict(theme=list(theme), tag=list(tag),
                name=name.decode("ascii", "replace"), flag=flag, w=w, h=h,
                obj=list(obj), npc=list(npc), tile=tile, attr=attr,
                layers=layers, kinds=kinds, spawns=spawns, end=c, size=len(d))


def scn_list(d, o):
    """One length-prefixed record list: `u8 count`, `count` length bytes, bodies.

    The offsets table the game builds is the PREFIX SUM of those length bytes -
    guest 0x51ee8 accumulates `scn[cursor]` into a running offset and stores it as
    the next entry, advancing the cursor one byte per record. So the record bodies
    are variable-length and back to back, and nothing in the file is aligned.
    """
    n = d[o]
    o += 1
    lens = list(d[o:o + n])
    o += n
    recs = []
    for L in lens:
        if o + L > len(d):
            raise ValueError("truncated scn record")
        recs.append(d[o:o + L])
        o += L
    return recs, o


def parse_scn(d):
    """The 9-byte header plus THREE record lists - and nothing else.

    ★ 244/244 scn files end at exactly EOF under this model, which is the same
    proof the map parser gets (see the module header). The three lists are:

      A  placement / triggers.  7-byte NPC rows are `x y ? ff ? kind payload`;
         `kind 3` is "talk" and `payload` indexes C. `kind 4` indexes B.
      B  event scripts - BYTECODE (opcodes 0x02..0x36 seen; `0x34 xx` is a common
         two-byte one). The dialogue CHOICE opcode has not been identified yet.
      C  strings, NUL-terminated EUC-KR with the game's colour codes
         (`@` white, `}` orange, `$` sky, `|`, `{`) - see MIGRATION_LGT.md 2.21.
         26,426 hangul runs live here and ZERO in A or B, which is what identifies
         this list rather than a guess.
    """
    o = 9
    lists = []
    for _ in range(3):
        recs, o = scn_list(d, o)
        lists.append(recs)
    return dict(map=d[0], b1=d[1], b2=d[2], species=[d[3], d[4], d[5]],
                region=d[6], b7=d[7], intro=d[8],
                place=lists[0], script=lists[1], text=lists[2],
                end=o, size=len(d))


def load(jar):
    maps, scenes = {}, {}
    for n in jar.namelist():
        if n.startswith(MAP_PREFIX) and n.endswith(MAP_SUFFIX):
            maps[int(n[len(MAP_PREFIX):-len(MAP_SUFFIX)])] = parse_map(jar.read(n))
        elif n.startswith(SCN_PREFIX) and n.endswith(SCN_SUFFIX):
            scenes[int(n[len(SCN_PREFIX):-len(SCN_SUFFIX)])] = parse_scn(jar.read(n))
    return maps, scenes


def creature_table(jar, hard):
    """species id -> (name, level). 0..160 from enemy_dat, 161.. from boss_dat."""
    out = {}
    en = "res/dat/enemyh_dat" if hard else "res/dat/enemy_dat"
    bo = "res/boss/bossh_dat" if hard else "res/boss/boss_dat"
    for i, r in enumerate(records(jar.read(en))):
        nl = r[0]
        out[i] = (r[1:1 + nl].decode("euc-kr", "replace"), r[1 + nl])
    for i, r in enumerate(records(jar.read(bo))):
        nl = r[0]
        out[BOSS_BASE + i] = ("[보스] " + r[1:1 + nl].decode("euc-kr", "replace"),
                              r[1 + nl])
    return out


def species_of(scene, maps):
    """The species actually reachable on a scene: its triple, restricted to the
    slots its map really spawns."""
    used = {t for t, _, _ in maps[scene["map"]]["spawns"]}
    used |= {k for k in maps[scene["map"]]["kinds"]}
    return [s for i, s in enumerate(scene["species"])
            if s != SPECIES_NONE and i in used]


def scene_level(scene, maps, crea):
    got = [crea[s][1] for s in species_of(scene, maps) if s in crea]
    return max(got) if got else None


def check(maps, scenes, crea):
    fail = collections.Counter()
    for mid, m in maps.items():
        if m["end"] != m["size"]:
            fail["map does not parse to EOF"] += 1
        for t, x, y in m["spawns"]:
            if t > 2:
                fail["spawn type is not 0/1/2"] += 1
            if x >= m["w"] or y >= m["h"]:
                fail["spawn outside the tile grid"] += 1
        for k in m["kinds"]:
            if k > 2:
                fail["spawn kind is not 0/1/2"] += 1
        for li, recs in enumerate(m["layers"]):
            for r in recs:
                if r["px"] > m["w"] * 16 or r["py"] > m["h"] * 16:
                    fail["object outside the pixel grid"] += 1
                table = m["npc"] if r["sprite"] else m["obj"]
                if r["id"] not in table:
                    fail["object id not in its table"] += 1
    for sid, s in scenes.items():
        if s["map"] not in maps:
            fail["scene names a missing map"] += 1
        for sp in s["species"]:
            if sp != SPECIES_NONE and sp not in crea:
                fail["scene names an unknown species"] += 1
    return fail


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--maps", action="store_true")
    ap.add_argument("--scenes", action="store_true")
    ap.add_argument("--by-level", action="store_true")
    ap.add_argument("--map", type=int)
    ap.add_argument("--scene", type=int)
    ap.add_argument("--hard", action="store_true",
                    help="use enemyh_dat / bossh_dat levels")
    ap.add_argument("--json")
    a = ap.parse_args(argv)

    jar = open_assets(a.zip)
    maps, scenes = load(jar)
    crea = creature_table(jar, a.hard)

    if a.check or not any((a.maps, a.scenes, a.by_level, a.json,
                           a.map is not None, a.scene is not None)):
        fail = check(maps, scenes, crea)
        eof = sum(1 for m in maps.values() if m["end"] == m["size"])
        seof = sum(1 for s_ in scenes.values() if s_["end"] == s_["size"])
        print(f"maps {len(maps)}  scenes {len(scenes)}  "
              f"creatures {len(crea)} (161 enemy + {len(crea) - BOSS_BASE} boss)")
        print(f"maps that parse to exactly EOF: {eof}/{len(maps)}")
        # ⛔ The scn container model stands or falls on this line. Anything but
        # 244/244 means the three-list model is wrong, NOT that one file is odd.
        print(f"scenes that parse to exactly EOF: {seof}/{len(scenes)}")
        if seof != len(scenes):
            fail["scn_eof"] = len(scenes) - seof
        print("consistency failures: " + (str(dict(fail)) if fail else "NONE"))
        withmon = sum(1 for m in maps.values() if m["spawns"])
        print(f"maps carrying spawn points: {withmon}/{len(maps)}")
        reuse = collections.Counter(s["map"] for s in scenes.values())
        multi = sorted(((n, m) for m, n in reuse.items() if n > 1), reverse=True)
        print(f"maps reused by more than one scene: {len(multi)}"
              + (f"  (most: map {multi[0][1]} x{multi[0][0]})" if multi else ""))

    if a.maps:
        print(f"\n{'map':>4} {'name':24s} {'size':>7} {'blk%':>5} "
              f"{'obj':>4} {'npc':>4} {'spawn':>5} scenes")
        by_map = collections.defaultdict(list)
        for sid, s in scenes.items():
            by_map[s["map"]].append(sid)
        for mid in sorted(maps):
            m = maps[mid]
            blk = sum(1 for b in m["attr"] if b & ATTR_BLOCKED)
            n = len(m["attr"]) or 1
            ss = sorted(by_map.get(mid, []))
            print(f"{mid:>4} {m['name']:24s} {m['w']:>3}x{m['h']:<3} "
                  f"{100 * blk // n:>4}% {len(m['obj']):>4} {len(m['npc']):>4} "
                  f"{len(m['spawns']):>5} "
                  + (",".join(str(x) for x in ss[:8]) + ("…" if len(ss) > 8 else "")))

    if a.scenes or a.by_level:
        rows = []
        for sid in sorted(scenes):
            s = scenes[sid]
            m = maps[s["map"]]
            sp = species_of(s, maps)
            rows.append((scene_level(s, maps, crea), sid, s, m, sp))
        if a.by_level:
            rows = [r for r in rows if r[0] is not None]
            rows.sort(key=lambda r: (r[0], r[1]))
        print(f"\n{'lv':>3} {'scn':>4} {'map':>4} {'name':22s} {'sp':>5} "
              f"{'rgn':>4} intro  monsters")
        for lv, sid, s, m, sp in rows:
            mon = " · ".join(f"{crea[x][0]}({crea[x][1]})" for x in sp if x in crea)
            print(f"{(lv if lv is not None else -1):>3} {sid:>4} {s['map']:>4} "
                  f"{m['name'][:22]:22s} {len(m['spawns']):>5} {s['region']:>4} "
                  f"{('-' if s['intro'] == 0xff else str(s['intro'])):>5}  {mon}")

    if a.map is not None:
        m = maps[a.map]
        print(f"\nmap {a.map} = {m['name']}  {m['w']}x{m['h']} tiles "
              f"({m['w'] * 16}x{m['h'] * 16} px)  file {m['size']} B")
        print(f"  theme {m['theme']}  tag {m['tag']}  flag {m['flag']}")
        print(f"  objects {m['obj']}")
        print(f"  sprites {m['npc']}")
        for li, recs in enumerate(m["layers"]):
            print(f"  layer{li}: {len(recs)} placement(s)")
            for r in recs[:12]:
                kind = "sprite" if r["sprite"] else "object"
                print(f"     flags {r['flags']:#04x} {kind} {r['id']:>3} "
                      f"at px ({r['px']},{r['py']}) = tile "
                      f"({r['px'] // 16},{r['py'] // 16})")
            if len(recs) > 12:
                print(f"     … {len(recs) - 12} more")
        print(f"  spawn kinds {m['kinds']}")
        print(f"  spawn points ({len(m['spawns'])}): "
              + ", ".join(f"t{t}@({x},{y})" for t, x, y in m["spawns"][:24]))
        used = sorted({s for s, sc in scenes.items() if sc["map"] == a.map})
        print(f"  used by scenes: {used}")
        rows = []
        for y in range(m["h"]):
            rows.append("".join("#" if m["attr"][y * m["w"] + x] & ATTR_BLOCKED
                                else "." for x in range(m["w"])))
        print("  collision:")
        for r in rows:
            print("    " + r)

    if a.scene is not None:
        s = scenes[a.scene]
        m = maps[s["map"]]
        print(f"\nscene {a.scene}: map {s['map']} = {m['name']}  "
              f"{m['w']}x{m['h']}  scn file {s['size']} B")
        print(f"  region {s['region']}  b1 {s['b1']}  b2 {s['b2']}  "
              f"b7 {s['b7']}  intro "
              + ("-" if s["intro"] == 0xff else str(s["intro"])))
        for i, sp in enumerate(s["species"]):
            if sp == SPECIES_NONE:
                print(f"  species[{i}] = -")
                continue
            nm, lv = crea.get(sp, ("?", 0))
            n = sum(1 for t, _, _ in m["spawns"] if t == i)
            print(f"  species[{i}] = {sp:>3} {nm}  lv {lv}  "
                  f"({n} spawn point(s))")

    if a.json:
        out = dict(
            maps={str(k): dict(name=v["name"], w=v["w"], h=v["h"],
                               theme=v["theme"], objects=v["obj"],
                               sprites=v["npc"], kinds=v["kinds"],
                               spawns=v["spawns"],
                               blocked=sum(1 for b in v["attr"]
                                           if b & ATTR_BLOCKED),
                               placements=[[r["flags"], r["id"], r["px"],
                                            r["py"]] for L in v["layers"]
                                           for r in L])
                  for k, v in sorted(maps.items())},
            scenes={str(k): dict(map=v["map"], species=v["species"],
                                 region=v["region"], b2=v["b2"], b7=v["b7"],
                                 intro=v["intro"],
                                 level=scene_level(v, maps, crea))
                    for k, v in sorted(scenes.items())},
            creatures={str(k): dict(name=v[0], level=v[1])
                       for k, v in sorted(crea.items())})
        with open(a.json, "w", encoding="utf-8") as f:
            json.dump(out, f, ensure_ascii=False, indent=1)
        print(f"\nwrote {a.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
