"""Render the game's maps to PNG - one image per map, plus labelled contact sheets.

WHY. "Which map should a tower floor use?" is a question about how a map LOOKS, and
until this existed the only way to see one was to warp there in the emulator. The
first generated arena shipped as a single tile repeated 1444 times precisely because
nobody could look at it (2026-09-12 report: "the maps do not look like real maps, it
is just one tile laid out over and over").

HOW A MAP IS DRAWN, as far as it is decoded:

  * `res/map/theme_<n>_bm` is a tileset: ONE frame, 16 px wide, 16*k tall - a vertical
    strip of k 16x16 tiles. A map's `theme` field lists the strips it loads (1..3 of them).
  * A TILE BYTE DECOMPOSES, and this is exact rather than a best guess:

        slot  = (id >> 6) & 3     which of the map's `theme` entries
        bit5  = (id >> 5) & 1     VERTICAL flip; the horizontal mirror is attr bit 6 (see below)
        index =  id & 0x1f        the tile inside that strip
        id == 0xff                no tile (the void outside a room)

    Scored over every tile of every shipped map - 134,751 tiles, 135 maps - this leaves
    **0 ids unresolved**, and no theme strip in the game holds more than 32 tiles, which
    is exactly why five bits are enough for the index. Earlier models here were guesses
    that happened to render something: flat `id & 0x3f` into the concatenated strips left
    4.75 % unresolved, a 64-slot window 12.13 %, a 32-slot window 9.33 %.
    The decomposition also explains the shape of the id sets a map actually uses -
    GULBEIG_RUIN_2's `theme [33, 22, 37]` with 9 / 22 / 18 tiles uses ids 0x00..0x07 and
    0x20..0x26 (slot 0), **all 22** of 0x40..0x55 (slot 1) and **all 18** of 0x80..0x91
    (slot 2). A flat index cannot produce three runs that land on 0x40 and 0x80.
  * `res/map/obj_<n>_bm` are the DECORATION banks: 44 banks of 5 frames each except bank
    33 which holds 52, so the frames CONCATENATE into ids 0..216 - and 216 is exactly the
    largest id any map uses. A decoration record's `id` is that global index, and a map's
    `obj` / `npc` fields are the id sets it draws (`set(records) == set(obj) | set(npc)`
    holds on 135/135 shipped maps; `npc` ids come from the character art instead, so they
    are skipped here). Off by default (`--decor`) because they hide the floor.
  * collision is a separate layer: `attr[y*w+x] & 0x80`. Tinted red with `--attr`.

Usage:
    python tools_re/render_maps.py                     # every map + contact sheets
    python tools_re/render_maps.py --map 44 --map 120  # just these
    python tools_re/render_maps.py --candidates        # only plausible tower floors
    python tools_re/render_maps.py --attr --spawns     # overlay collision and spawns
"""
import argparse
import io
import os
import sys
import zipfile
from collections import deque

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_map_catalog as MC          # noqa: E402
import gen_h3_icons as ICON            # noqa: E402

from PIL import Image, ImageDraw       # noqa: E402

DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
BLOCKED = 0x80
TILE = 16
MISSING = (26, 26, 30, 255)            # an id this tool cannot resolve

# ★★★ 88차-B (2026-09-12): THERE ARE TWO TRANSFORMS, AND THE MIRROR IS IN `attr`.
#   * tile byte bit 5 = VERTICAL flip: `54` is the north cap run and `74` the south lip,
#     `53`/`73` the two corners — the same art upside down.
#   * attr byte bit 6 (0x40) = HORIZONTAL mirror. Map 71's brazier pair is `46 46` with
#     attr `80 c0`: the same half, mirrored on the right, one symmetric fixture. Every
#     right-hand counterpart in the game's walls is built this way (cap corner, run end,
#     side wall, lip corner, foot, pilaster), and so are the floor's variants.
#   Measured against the game's own frame of scene 128 (map 71): with these two the
#   render is pixel-for-pixel the wall the game draws; with LEFT_RIGHT on bit 5 and no
#   attr mirror (the 87차 model) every brazier came out as two left halves — which is
#   exactly what the generated rooms looked like in vc300/vc301 ("횃불이 반 잘린듯").
TILE_FLIP = Image.FLIP_TOP_BOTTOM
MIRROR = 0x40


def open_jar(zpath):
    z = zipfile.ZipFile(zpath)
    names = [n for n in z.namelist() if n.endswith(".jar")]
    if len(names) != 1:
        raise SystemExit("expected exactly one .jar in %s" % zpath)
    return zipfile.ZipFile(io.BytesIO(z.read(names[0])))


class Art:
    """Tile strips and decoration banks, decoded once and shared."""

    def __init__(self, jar):
        self.jar = jar
        self._themes = {}
        self._objs = {}

    def theme(self, n):
        if n not in self._themes:
            cw, h, px = ICON.decode_bm_bytes(self.jar.read("res/map/theme_%d_bm" % n))[0]
            out = []
            for i in range(h // TILE):
                out.append(Image.frombytes("RGBA", (TILE, TILE), bytes(
                    b for y in range(TILE) for x in range(TILE)
                    for b in ((px[(i * TILE + y) * cw + x] >> 16) & 0xFF,
                              (px[(i * TILE + y) * cw + x] >> 8) & 0xFF,
                              px[(i * TILE + y) * cw + x] & 0xFF,
                              (px[(i * TILE + y) * cw + x] >> 24) & 0xFF))))
            self._themes[n] = out
        return self._themes[n]

    def tile(self, theme, v, attr=0):
        """The 16x16 image a tile byte draws (with its attr's mirror), or `None` for the
        void / an unknown id."""
        if v == 0xFF:
            return None
        slot, flip, idx = (v >> 6) & 3, (v >> 5) & 1, v & 0x1F
        if slot >= len(theme):
            return None
        strip = self.theme(theme[slot])
        if idx >= len(strip):
            return None
        t = strip[idx]
        if flip:
            t = t.transpose(TILE_FLIP)
        if attr & MIRROR:
            t = t.transpose(Image.FLIP_LEFT_RIGHT)
        return t

    def decor(self, gid):
        """The frame global decoration id `gid` draws, or `None` past the end.

        The banks concatenate: bank 0's five frames are ids 0..4, bank 1's are 5..9, and
        so on. 44 banks, 5 frames each but 52 in bank 33, so ids run 0..216 - and 216 is
        the largest id any shipped map uses. Banks 34..43 repeat bank 33's art and no map
        reaches them.
        """
        if not self._objs:
            n = 0
            while True:
                try:
                    raw = self.jar.read("res/map/obj_%d_bm" % n)
                except KeyError:
                    break
                for cw, h, px in ICON.decode_bm_bytes(raw):
                    self._objs[len(self._objs)] = Image.frombytes("RGBA", (cw, h), bytes(
                        b for y in range(h) for x in range(cw)
                        for b in ((px[y * cw + x] >> 16) & 0xFF, (px[y * cw + x] >> 8) & 0xFF,
                                  px[y * cw + x] & 0xFF, (px[y * cw + x] >> 24) & 0xFF)))
                n += 1
        return self._objs.get(gid)


def walk_regions(m):
    """Sizes of the 4-connected walkable regions, biggest first."""
    w, h, a = m["w"], m["h"], m["attr"]
    seen, out = set(), []
    for y in range(h):
        for x in range(w):
            if a[y * w + x] & BLOCKED or (x, y) in seen:
                continue
            q, reg = deque([(x, y)]), {(x, y)}
            while q:
                cx, cy = q.popleft()
                for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    nx, ny = cx + dx, cy + dy
                    if (0 <= nx < w and 0 <= ny < h and not (a[ny * w + nx] & BLOCKED)
                            and (nx, ny) not in reg):
                        reg.add((nx, ny))
                        q.append((nx, ny))
            seen |= reg
            out.append(len(reg))
    return sorted(out, reverse=True)


def render(m, art, attr=False, spawns=False, decor=False):
    w, h = m["w"], m["h"]
    im = Image.new("RGBA", (w * TILE, h * TILE), (0, 0, 0, 255))
    missing = 0
    for y in range(h):
        for x in range(w):
            v = m["tile"][y * w + x]
            t = art.tile(m["theme"], v, m["attr"][y * w + x])
            if t is not None:
                im.paste(t, (x * TILE, y * TILE), t)
            elif v != 0xFF:            # 0xff IS the void; only a bad id is "missing"
                missing += 1
                ImageDraw.Draw(im).rectangle(
                    [x * TILE, y * TILE, x * TILE + TILE - 1, y * TILE + TILE - 1], fill=MISSING)
    if decor:
        # A record's `id` is the GLOBAL decoration index (see the header); `flags & 0x80`
        # mirrors it, which is how the game builds a symmetric object out of two halves.
        npcs = set(m["npc"])
        for layer in m["layers"]:
            for r in layer:
                if r["id"] in npcs:
                    continue           # character art, a different source
                f = art.decor(r["id"])
                if f is None:
                    continue
                if r["flags"] & 0x80:
                    f = f.transpose(Image.FLIP_LEFT_RIGHT)
                im.paste(f, (r["px"], r["py"] - f.height), f)
    ov = Image.new("RGBA", im.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(ov)
    if attr:
        for y in range(h):
            for x in range(w):
                if m["attr"][y * w + x] & BLOCKED:
                    d.rectangle([x * TILE, y * TILE, x * TILE + TILE - 1, y * TILE + TILE - 1],
                                fill=(210, 40, 40, 70))
    if spawns:
        for (t, x, y) in m["spawns"]:
            c = [(255, 90, 90, 255), (90, 240, 140, 255), (130, 180, 255, 255)][t % 3]
            d.ellipse([x * TILE + 3, y * TILE + 3, x * TILE + TILE - 4, y * TILE + TILE - 4],
                      fill=c, outline=(0, 0, 0, 255))
    return Image.alpha_composite(im, ov), missing


def label(m, mid, regions, scene_ids):
    reg = regions[0] if regions else 0
    return ("map %d  %s  %dx%d\nwalk %d (max region %d)  spawns %d  decor %d  tiles %d\nscenes %s"
            % (mid, m["name"] or "(no name)", m["w"], m["h"],
               sum(1 for i in range(m["w"] * m["h"]) if not (m["attr"][i] & BLOCKED)),
               reg, len(m["spawns"]), sum(len(x) for x in m["layers"]),
               len(set(m["tile"])), scene_ids or "-"))


def sheet(items, cols, cell_w, cell_h, pad=6):
    rows = (len(items) + cols - 1) // cols
    W = cols * (cell_w + pad) + pad
    H = rows * (cell_h + pad + 34) + pad
    out = Image.new("RGB", (W, H), (18, 18, 22))
    d = ImageDraw.Draw(out)
    for i, (im, text) in enumerate(items):
        cx = pad + (i % cols) * (cell_w + pad)
        cy = pad + (i // cols) * (cell_h + pad + 34)
        th = im.copy()
        th.thumbnail((cell_w, cell_h), Image.NEAREST)
        out.paste(th.convert("RGB"), (cx + (cell_w - th.width) // 2, cy + (cell_h - th.height) // 2))
        d.rectangle([cx, cy, cx + cell_w, cy + cell_h], outline=(70, 70, 80))
        for k, line in enumerate(text.split("\n")):
            d.text((cx + 2, cy + cell_h + 2 + k * 10), line, fill=(220, 220, 210))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--out", default="map_gallery")
    ap.add_argument("--map", type=int, action="append")
    ap.add_argument("--candidates", action="store_true",
                    help="only maps a scene with monsters uses, or fully open ones")
    ap.add_argument("--attr", action="store_true", help="tint blocked tiles red")
    ap.add_argument("--spawns", action="store_true", help="mark spawn points")
    ap.add_argument("--decor", action="store_true", help="also paste decorations")
    ap.add_argument("--cols", type=int, default=3)
    ap.add_argument("--cell", type=int, default=460)
    ap.add_argument("--per-sheet", type=int, default=9)
    a = ap.parse_args()

    jar = open_jar(a.zip)
    maps, scenes = MC.load(jar)
    art = Art(jar)
    by_map = {}
    for sid, sc in scenes.items():
        by_map.setdefault(sc["map"], []).append(sid)

    ids = sorted(a.map) if a.map else sorted(maps)
    if a.candidates and not a.map:
        keep = []
        for mid in ids:
            m = maps[mid]
            has_mon = any(any(s != 0xFF for s in scenes[s2]["species"]) for s2 in by_map.get(mid, []))
            open_all = all(not (v & BLOCKED) for v in m["attr"])
            if has_mon or open_all:
                keep.append(mid)
        ids = keep

    os.makedirs(a.out, exist_ok=True)
    items, rows = [], []
    for mid in ids:
        m = maps[mid]
        im, missing = render(m, art, a.attr, a.spawns, a.decor)
        im.convert("RGB").save(os.path.join(a.out, "map%03d.png" % mid))
        regions = walk_regions(m)
        items.append((im, label(m, mid, regions, sorted(by_map.get(mid, [])))))
        rows.append((mid, m["name"] or "", m["w"], m["h"], regions[0] if regions else 0,
                     len(set(m["tile"])), len(m["spawns"]), missing))

    for i in range(0, len(items), a.per_sheet):
        chunk = items[i:i + a.per_sheet]
        s = sheet(chunk, a.cols, a.cell, a.cell)
        p = os.path.join(a.out, "sheet%02d.png" % (i // a.per_sheet + 1))
        s.save(p)
        print("wrote %s (%d maps)" % (p, len(chunk)))

    rows.sort(key=lambda r: -r[4])
    print("\n%-5s %-20s %-8s %-9s %-7s %-7s %s" % ("map", "name", "size", "region", "tileids", "spawns", "unresolved"))
    for mid, nm, w, h, reg, ntile, nsp, miss in rows[:24]:
        print("%-5d %-20s %-8s %-9d %-7d %-7d %d" % (mid, nm[:20], "%dx%d" % (w, h), reg, ntile, nsp, miss))
    print("\n%d map(s) -> %s" % (len(ids), a.out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
