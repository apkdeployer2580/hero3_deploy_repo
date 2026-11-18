"""Generate the 하늘성 floors: four rooms (maps 135..138) and four scenes (243..246).

    python tools_re/gen_tower_rooms.py <asset.zip> [--apply | --check]
    python tools_re/gen_tower_rooms.py <asset.zip> --preview <dir>   # PNGs, no writes

★★★ 88차 (2026-09-12) — THREE THINGS CHANGED, and each undoes a 87차 assumption:

  1. A TORCH IS TWO COLUMNS WIDE — AND THE RIGHT COLUMN IS THE LEFT ONE **MIRRORED**.
     87차 placed the brazier stack `40 43 46 49` as ONE column and the player saw "횃불이
     반 잘린". 88차's first pass put the pair in (`40 40 / 43 43 / 46 46 / 49 49`, rows
     1..4 of the census) and the player still saw two left halves — because the mirror is
     NOT in the tile byte. ★★★ `attr` bit 6 (0x40) is a horizontal mirror: map 71's pair
     carries attr `80 c0`. The same bit builds every right-hand counterpart of the game's
     walls (cap corner `53`, run end `42..4b`, foot `4e`, side wall `55`, lip corner `73`,
     pilaster `50`) and the floor's mirrored variants, and tile-byte bit 5 is a VERTICAL
     flip (`54` cap / `74` lip). Measured pixel-for-pixel against the game's own frame of
     scene 128 (`tools_re/render_maps.py` renders it identically now). `put()` takes a
     `mirror` flag and every right-hand piece below sets it. On top of the tiles the game
     hangs decoration **149** (16x7 base lip, a normal + `flags 0x80` mirrored pair) and an
     animated flame SPRITE (layer 2, `flags 0x20 id 5` → sprite bank `npc [5]`), which is
     what `npc [5]` on maps 71/72 is FOR; both are placed here too.
  2. ONE SCENE PER ROOM, and the SPECIES COME FROM THE HOST. 87차 baked a species triple
     into each of twelve scenes (three count tiers x four rooms), so a floor's monsters
     were a property of which scene it used — and with three halls rotating, floors 1/4/7
     were the same line-up ("1,2,3 층 몬스터가 4,5,6 에도 똑같이"). The host now overrides
     the triple as the scene loads (`tower_species_lgt` hook on `parseScnHeader`, knob
     `wie_util::stats::TOWER_SPECIES`), and the floor table carries `s0 s1 s2` per floor —
     so every floor is its own roll and a scene only needs to name its MAP. The triple in
     the scene header is the FALLBACK (an app without the hook still spawns something).
     The count tier survives as which slots the host leaves at `0xff`.
  3. NO HARMLESS CRITTERS AT ALL (player decision 2026-09-12, second pass: "박스같은건
     넣으면 안돼 몬스터만 넣어야해"). The attackless set — the nineteen green-`'` critters,
     boxes included — is measured from the species table (`attackless()` below) and
     EXCLUDED; every floor draws from `attackers()`. 88차's first pass had put them on
     floors 1~9 in 54-strong swarm halls (maps 139..141, scenes 247..249); those are now
     RETIRED from the archive like 87차's tier scenes. Nothing in a map says which species
     it can host — 88차 warped species 6 / 85 / 43 (never spawned on this tileset) into
     map 135 and they rendered and moved, so the 87차 "npc-list hosting" pool was a
     coincidence, not a constraint.

WHAT THE PLAYER ASKED FOR (2026-09-12)

  "Map071 에서 정 가운데 기둥이 있고 네모낳게 둘러싼 형태의 맵 1개, 똑같은 map071 에서
   정가운데 + 각 1시,5시,7시,11시 방향에 기둥이 있는 형태의 맵 1개, 똑같은 map071 에서
   정가운데 기둥없음 + 각 1시,5시,7시,11시 방향에 기둥이 있는 형태의 맵 1개, 총 3개를
   돌려가면서 1~9층 사용하고 보스방은 Map072 에서 가운데 영역에 모든 장애물 다 없애고
   정사각형으로 뻥 뚫린형태 + 모든 벽에 1칸 간격으로 횃불이 심어져 있는 맵 이렇게 4개의
   맵으로 1층~100층까지 모두 구성할꺼야 … 몬스터는 랜덤으로 가져오면 돼"

  So: three square halls that differ only in where their pillars stand, plus one bigger
  square hall with nothing in it and torches all the way round, and random monsters.

WHY THIS REPLACES `gen_tower_arena.py`

  That tool built ONE 40x40 hall (86차) by copying map 44's whole tile layer. It shipped
  as vc298 and the verdict was that 90 identical floors are monotonous. This tool builds
  rooms to a SHAPE instead - which is only safe because the tile encoding is now exact
  rather than a guess (see below), so a placed tile id draws what it drew in the donor.

THE TILE ENCODING IS SOLVED, AND THAT IS WHAT MAKES THIS BUILDABLE

  A tile byte decomposes as `slot = (id >> 6) & 3` (which of the map's `theme` entries),
  `bit5 = (id >> 5) & 1` (a transform) and `index = id & 0x1f` (the tile in that strip);
  `0xff` is the void. Over every tile of every shipped map - 134,751 tiles, 135 maps -
  that leaves **0 ids unresolved**, and no theme strip holds more than 32 tiles, which
  is why five bits are enough. `tools_re/render_maps.py` carries the scoring; the earlier
  flat `id & 0x3f` model left 4.75 % unresolved and would have made placed ids a lottery.

  ⛔ THE 86차 TOOL'S REASONING NO LONGER APPLIES. Its header says "inventing art on top
  of a model that is 88 % understood is how you get a floor that repeats" - true then.
  The model is now 100 %, and every id this tool writes is one the donor map uses in the
  SAME role (a wall column stays a wall column, floor stays floor), asserted in `verify`.

WHERE EVERY BYTE COMES FROM - nothing here is invented art

  * WALL VOCABULARY: map 71 `GULBEIG_RUIN_1`. Censused by walking every floor tile and
    reading the blocked run above / below / beside it, so these are the stacks the map's
    own author used, with their counts:

        north wall, nearest-the-floor first      count in map 71
          49 46 43 40 54                              12   <- carries a lit brazier
          4A 47 41 41 54 / 4A 47 44 41 54             11   <- plain masonry
          4A 47 50 50 54                              12   <- pilaster
          4E 4B 48 45 42                              14   <- the run's two ends
        below a floor tile:  51 73 FF (x8) / 74 ...        <- the south lip
        beside a floor tile: 55 55 (x28)                   <- the side walls

    Tile **0x46 is the flame**: of map 71's 31 tiles it has by far the most bright-warm
    ink (45 px of r>150 and r-b>70; the next is 8), and `40 43 46 49` are each used
    exactly 12 times - one count for one fixture. That is the "횃불" the request names,
    and it is a wall tile, not a decoration.
  * FLOOR: a 2D window copied out of whichever map in the `[33, 22]` theme family has
    the emptiest one of the size wanted - measured per room, not pinned, because the
    best 20x20 window and the best 24x24 window are not on the same map (20 -> map 106
    `GULBEIG_RUIN_6`, 24 -> map 105 `GULBEIG_RUIN_5`). The 20x20 mix is fourteen
    different ids (`04 36 05 01 37 35 03 33 06 38 32 02 07 00`), which is the whole
    point: a floor of one id reads as a texture, and that was 86차-B's complaint.
    No window is perfectly open, so the cells that are not floor are filled from the
    NEAREST floor cell of the same window - that closes the holes without inventing a
    tile and without flattening the pattern.
  * PILLAR: map 72's own free-standing pillar, copied stamp, attrs and all. It occurs
    twice there, at tiles (6,14) and (6,22), as a 3x3 arch with **only the centre
    blocked** (attr `00 00 40 / 00 80 40 / 40 00 00` — the right column and the
    bottom-left are mirrored halves) plus the statue sprite (decoration id 162) drawn as
    two mirrored halves:

          91 80 91          decoration: {flags 00, id 162, px = cx*16,      py = (cy+1)*16}
          90 83 90   <- blocked                 {flags 80, id 162, px = cx*16 + 16, py = same}
          82 85 82

  * TORCH INTERVAL: a brazier is a PAIR of columns (88차 — see the header). The halls
    keep map 71's own rhythm `TT PP TT PP` (`torch_step` 4), and the boss hall's "1칸
    간격으로 횃불" is `TT P TT P` (`torch_step` 3): one plain column between fixtures.
  * THE OTHER THREE WALLS: in this projection only the north wall shows its face, so
    there is no torch TILE for the sides or the lip. The boss hall hangs decoration
    **id 10** there instead - the game's own lit lamp (7x17, a warm flame in a housing)
    - at the same one-tile interval. A decoration record's `id` is a global index over
    the concatenated `obj_<n>_bm` banks (5 frames each), so id 10 is bank 2 frame 0.

THE ROOM FRAME, which is the only thing composed rather than copied

      x:            0            1 .. INNER              INNER+1
      row 0         53           54 x INNER               53          cap
      rows 1..4     42/45/48/4B  <column stacks>          42/45/48/4B wall face
      row 5         4E           floor                    4E          side wall, top
      rows 6..h-3   55           floor                    55          side wall
      row h-2       73           74 x INNER               73          south lip
      row h-1       FF           FF                       FF          void

  `INNER` is the side of the square walkable room, so the halls are a true square as the
  request asks ("네모낳게", "정사각형"). Every id above appears in map 71 in this role.

MONSTERS - the species are the HOST'S, the count is the MAP'S

  A scene names its map and its monsters in separate header bytes (`header[0]` = map,
  `header[3..6]` = a species TRIPLE), and a map's spawn records are `(type, x, y)` where
  `type` 0/1/2 picks one of those three slots; `0xff` in a slot spawns nothing.

  * SPECIES: the floor table (`gen_guardian_floors.py`) rolls a triple PER FLOOR and the
    host writes it over the scene's own as the scene loads. The triple baked in here is
    only what an app without that hook would see. `attackless()` / `attackers()` are the
    two pools and both generators share them, so "which species is harmless" has one
    definition — and the harmless ones are never used. Any species renders on any map
    (88차 measurement, header).
  * COUNT: a hall has 12 points (4 per type), the boss hall 1 + 4 + 4. The host picks
    how many slots are live, so a hall is worth 4 / 8 / 12.
  * ⚠ Measured in passing: 54 monsters (the retired swarm halls) is above anything the
    game ships (max 32, map 87) and still renders and ticks headlessly at +18 % guest
    instructions per frame — the loader's loops are 8-bit and the array is sized by
    the count. Kept here in case a big room is ever wanted again.

⛔ SCENE IDS ARE A ONE-BYTE BUDGET. The game ships 0..243 and the warp destination is a
single byte, so 244..255 are the only free ones - twelve. This tool uses 243..246 (one
per room) and REMOVES the retired 247..254 from the archive so nothing stale can be warped to.
"""
import argparse
import io
import os
import random
import shutil
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_map_catalog as MC          # noqa: E402  (the map / scene parser)
from patch_guardian_npc import build_scn, parse_scn  # noqa: E402

BLOCKED = 0x80
VOID = 0xFF

WALL_MAP = 71                          # GULBEIG_RUIN_1 - the wall vocabulary
PILLAR_MAP = 72                        # GULBEIG_RUIN_2 - the 3x3 statue niche
# The floor comes from whichever map in this theme family has the emptiest window of the
# size we need - see `floor_patch`. `[33, 22]` is the GULBEIG / GUARDIAN_CAVE stone set,
# eleven maps, and slot 0 / slot 1 ids mean the same thing in every one of them.
FAMILY_THEME = [33, 22]
FRAME_MAP = 72                         # theme / tag / flag
DONOR_SCENE = 202                      # GUARDIAN_CAVE_2 - region 14, b1/b2, b7

# --- the wall vocabulary, all of it map 71's (see the header census) -----------------
CAP_CORNER = 0x53
CAP_RUN = 0x54
END_COL = (0x42, 0x45, 0x48, 0x4B)     # rows 1..4 at the run's two ends
END_FOOT = 0x4E                        # row 5 at those ends
SIDE = 0x55                            # the side walls below that
FOOT_CORNER = 0x73
FOOT_RUN = 0x74
TORCH_COL = (0x40, 0x43, 0x46, 0x49)   # 0x46 is the flame - 45 warm px, used 12x
PLAIN_COL = (0x41, 0x44, 0x47, 0x4A)

# ★★★ 89차 - THE TWO CORNER STAMPS. `(map, diagonal column x, cap row)`; the side wall
# is the column one step further from the room. These are the only two places in the
# shipped game where a north wall run turns straight into a long side wall, and they are
# exact mirror images of each other (`verify` asserts that too, so a bad reading of
# either one cannot pass unnoticed).
WEST_CORNER_SRC = (72, 1, 11)          # GULBEIG_RUIN_2, diagonal x=1 / side wall x=0
EAST_CORNER_SRC = (71, 28, 0)          # GULBEIG_RUIN_1, diagonal x=28 / side wall x=29
CORNER_ROWS = 6                        # cap + the four diagonal rows + the foot


def corner_stamp(jar, map_id, dx, row0):
    """`[((tile, mirror, blocked) diagonal, (…) side wall)]`, `CORNER_ROWS` rows of it.

    Read out of the donor rather than written down here: the point of the stamp is that
    nobody gets to retype it.
    """
    m = MC.parse_map(jar.read(map_member(map_id)))
    w, sx = m["w"], dx + (1 if dx > m["w"] // 2 else -1)

    def cell(x, y):
        t = m["tile"][y * w + x]
        if t == VOID:
            return (VOID, False, True)   # nothing is drawn, so the mirror bit is noise
        return (t, bool(m["attr"][y * w + x] & MIRROR),
                bool(m["attr"][y * w + x] & BLOCKED))

    return [(cell(dx, row0 + k), cell(sx, row0 + k)) for k in range(CORNER_ROWS)]


def mirror_stamp(stamp):
    """The same corner seen from the other side: every mirror bit flips."""
    return [tuple((t, mir if t == VOID else not mir, blk) for (t, mir, blk) in row)
            for row in stamp]


# --- map 72's free-standing pillar, copied stamp and all ------------------------------
PILLAR_TILES = ((0x91, 0x80, 0x91),
                (0x90, 0x83, 0x90),
                (0x82, 0x85, 0x82))
# attr of the same stamp: the centre is blocked, and the right column / bottom-left are
# the mirrored halves (map 72, both pillars).
PILLAR_ATTR = ((0x00, 0x00, 0x40),
               (0x00, 0x80, 0x40),
               (0x40, 0x00, 0x00))
PILLAR_STATUE = 162                    # the mirrored-pair statue that stands in it
LAMP = 10                              # the game's lit lamp, for walls with no torch face
TORCH_BASE = 149                       # the brazier's base lip, 16x7, drawn as a mirrored pair
WALL_SHADOW = 150                      # the diagonal shadow at the foot of a side wall
FLAME_SPRITE = 5                       # layer-2 record `flags 0x20 id 5`: the animated flame
FLAME_FLAGS = 0x20
MIRROR = 0x40                          # attr bit 6: draw this tile mirrored
LAMP_STEP = 2                          # a lamp is one tile wide, so "1칸 간격" is every other tile

FLOOR_IDS = frozenset(list(range(0x00, 0x08)) + list(range(0x20, 0x27)))

# Balance knobs. ⛔ An open hall changes what a monster count means: 86차 measured a
# Lv70 character with 1214 HP dying in seven seconds to 18 monsters in a 38x38 hall
# because nothing splits the room. These halls are smaller and carry pillars, but the
# per-floor ramp still comes from `gen_guardian_floors.tiers` - this is only how many
# things converge at once.
HALL_SPAWNS = 20                       # the host keeps the floor's `count` (1..20) and parks the rest
BOSS_ADDS = 4                          # boss hall: 1 boss + 4 of each add slot
SPECIES_MIN_LEVEL = 40                 # on the HARD table, which is what the tower runs
SPECIES_SEED = 0x48414C4C              # "HALL" - the fallback roll is deterministic per build

SCENES_PER_ROOM = 1                    # 88차: the host picks species, so one scene per MAP
FIRST_SCENE = 243                      # the 86차 arena's orphan; 244.. follow
# 87차's three-tier scenes (250..254) and 88차's swarm-hall scenes (247..249) and maps
# (139..141). They are DELETED from the archive, not left behind: a table that still
# named one would warp into a room nothing maintains — so `present()` requires them gone.
RETIRED_SCENES = tuple(range(247, 255))
RETIRED_MAPS = (139, 140, 141)


class Room(object):
    def __init__(self, map_id, name, inner, pillars, torch_step, lamps, npc, spawns):
        self.map_id = map_id
        self.name = name
        self.inner = inner
        self.pillars = pillars         # "none" | "centre" | "diagonals" | "both"
        self.torch_step = torch_step   # north wall: a brazier PAIR every N columns
        self.lamps = lamps             # hang lamps on the other three walls
        self.npc = npc                 # the map's sprite-bank list (copied from map 71 / 72)
        self.spawns = spawns           # spawn points; the boss hall ignores this

    @property
    def boss(self):
        return self.pillars == "none" and self.lamps

    @property
    def swarm(self):
        """A big-room hall (>= 50 points). None is shipped since 88차's second pass."""
        return not self.boss and self.spawns >= 50

    @property
    def w(self):
        return self.inner + 2

    @property
    def h(self):
        return self.inner + 7          # 5 wall rows on top, lip + void below


# ⛔ Map 135 is REUSED, not orphaned: it held the 86차 arena, and leaving that behind
# would keep a map in the archive that nothing reaches.
# ★ `npc` is what the donor maps declare (map 71 -> `[5]`, map 72 -> `[0]`); it is NOT a
# species pool (88차 — any species renders on any map). Kept so the map parses as the
# donors do.
ROOMS = (
# ★ `npc [5]` on every room: that bank is the brazier FLAME sprite (map 72 declares [5]
# too — 87차's `(0,)` for the throne was a misreading; nothing needs bank 0).
    Room(135, b"GUARDIAN_HALL_A", 20, "centre", 4, False, (5,), HALL_SPAWNS),
    Room(136, b"GUARDIAN_HALL_B", 20, "both", 4, False, (5,), HALL_SPAWNS),
    Room(137, b"GUARDIAN_HALL_C", 20, "diagonals", 4, False, (5,), HALL_SPAWNS),
    Room(138, b"GUARDIAN_THRONE", 24, "none", 3, True, (5,), 1 + 2 * BOSS_ADDS),
)

# The entry row and the record it runs, byte for byte what 232 of the game's 244 scenes
# do: a `kind 0` row whose payload is a container holding `fa ff 00` (the loader's entry
# flag is still 0) and then record 0, which raises that flag and walks the fade in.
#
# ⛔ THIS IS WHY THE ROOMS NEED NO SCENE GATE. `patch_tower_gate` exists because the
# game's own rooms run dialogue, quests and warps from their A rows; these scenes have
# exactly one A row and one B record, so there is nothing to gate. And the fade-in must
# be there: block it and the floor renders as a black world with a HUD (86차 measured
# 118,349 non-black pixels -> 2,569).
ENTRY_ROW = bytes([0x00, 0x00, 0x00, 0xFF, 0x00, 0x00,   # x y ? ff ? kind=0
                   0xFF, 0x01, 0x04,                     # container: 1 block, 4 bytes
                   0xFA, 0xFF, 0x00,                     # cond: flag 0xff == 0
                   0x00])                                # payload: record 0
FADE_IN = bytes([0x23, 0xFF, 0x01, 0x33, 0x00, 0x08])    # setFlag(0xff,1); fade in 8f


def _blob(b):
    if len(b) > 255:
        raise SystemExit("blob longer than a length byte")
    return bytes([len(b)]) + bytes(b)


def map_member(map_id):
    return "res/map/map%d_mp" % map_id


def scene_member(scene_id):
    return "res/event/e%02d%02d_scn" % (scene_id // 100, scene_id % 100)


# ---------------------------------------------------------------------------- geometry

def pillar_centres(room):
    """Blocked-centre tiles of the room's pillars, in the request's clock order."""
    inner = room.inner
    cx, cy = 1 + (inner - 1) // 2, 5 + (inner - 1) // 2
    # Far enough out that the four diagonals read as corners of the room rather than a
    # cluster, and >= 2 from the walls so the 3x3 stamp always fits.
    d = max(3, inner // 3 - 1)
    diag = [(cx + d, cy - d),          # 1 o'clock
            (cx + d, cy + d),          # 5 o'clock
            (cx - d, cy + d),          # 7 o'clock
            (cx - d, cy - d)]          # 11 o'clock
    if room.pillars == "none":
        return []
    if room.pillars == "centre":
        return [(cx, cy)]
    if room.pillars == "diagonals":
        return diag
    if room.pillars == "both":
        return [(cx, cy)] + diag
    raise SystemExit("unknown pillar layout %r" % room.pillars)


def floor_patch(jar, side):
    """A `side x side` block of a real floor, holes closed from the nearest floor cell.

    Returns `(rows, donor_map, substituted)`. The window is the one with the most
    pure-floor cells across the WHOLE theme family, not one hand-picked map: the family
    is every map whose `theme` starts `[33, 22]` (so a floor id drawn from it means the
    same thing in our rooms), and the search is over every window position in each.
    ⛔ Do not pin one donor - the best 20x20 window and the best 24x24 window are not on
    the same map, and pinning map 106 left a 24x24 boss floor a third substituted.
    """
    maps, _scenes = MC.load(jar)
    # ⛔ Never our own rooms, present or retired: a generated floor as its own donor is
    # circular, and a retired map that is still in the archive at `--apply` time but
    # gone by `--check` time makes the two disagree (88차-B, that exact trip).
    ours = [r.map_id for r in ROOMS] + list(RETIRED_MAPS)
    fam = sorted(mid for mid, m in maps.items()
                 if list(m["theme"])[:2] == FAMILY_THEME and mid not in ours)
    best = None
    for mid in fam:
        m = maps[mid]
        w, h, t, a = m["w"], m["h"], m["tile"], m["attr"]
        if w < side or h < side:
            continue

        def pure(x, y, t=t, a=a, w=w):
            return not (a[y * w + x] & BLOCKED) and t[y * w + x] in FLOOR_IDS

        for y0 in range(h - side + 1):
            for x0 in range(w - side + 1):
                n = sum(1 for yy in range(side) for xx in range(side) if pure(x0 + xx, y0 + yy))
                if best is None or n > best[0]:
                    best = (n, mid, x0, y0)
    if best is None:
        raise SystemExit("no map in the theme family is %dx%d or bigger" % (side, side))
    _n, mid, x0, y0 = best
    m = maps[mid]
    w, t, a = m["w"], m["tile"], m["attr"]
    # Each cell carries its mirror bit along with the id: the donor's floor is a mix of
    # a tile and its mirror image, and dropping the bit flattens that (88차-B).
    good = [[(t[(y0 + yy) * w + x0 + xx], a[(y0 + yy) * w + x0 + xx] & MIRROR)
             if (not (a[(y0 + yy) * w + x0 + xx] & BLOCKED)
                 and t[(y0 + yy) * w + x0 + xx] in FLOOR_IDS) else None
             for xx in range(side)] for yy in range(side)]
    subs = 0
    rows = []
    for yy in range(side):
        row = []
        for xx in range(side):
            v = good[yy][xx]
            if v is None:
                subs += 1
                v = _nearest(good, xx, yy, side)
            row.append(v)
        rows.append(row)
    return rows, mid, subs


def _nearest(good, x, y, side):
    """The value of the closest cell that IS floor. Rings out from (x, y)."""
    for r in range(1, side * 2):
        best = None
        for yy in range(max(0, y - r), min(side, y + r + 1)):
            for xx in range(max(0, x - r), min(side, x + r + 1)):
                if max(abs(xx - x), abs(yy - y)) != r or good[yy][xx] is None:
                    continue
                dist = (xx - x) ** 2 + (yy - y) ** 2
                if best is None or dist < best[0]:
                    best = (dist, good[yy][xx])
        if best is not None:
            return best[1]
    raise SystemExit("map %d window has no floor tile at all" % FLOOR_MAP)


def torch_at(room, x, run=None):
    """Does run column `x` (1-based from the run's first column) carry a brazier?

    ⛔ `run` is the run's length, and an incomplete trailing pair is NOT a brazier: the
    fixture is two columns and half of one is the very bug 88차 fixed. 89차 shortened
    every run by two columns (the corners took them), which put the boss hall's 22-wide
    run one column past its last whole pair.


    ★★ A brazier is TWO columns (88차). Map 71 draws `40 43 46 49` only as an adjacent
    pair — the fixture is a twin brazier on a shared base, and one column of it is half
    a torch, which is exactly what the player reported ("횃불이 반 잘린듯"). So the
    period is `torch_step` and the first TWO columns of every period are torch columns:
    step 4 = `TT PP` (map 71's own rhythm), step 3 = `TT P` (the boss hall's "1칸 간격").
    ⛔ Do not go below 3: `TT TT` is a wall of fire, not torches at an interval.
    """
    if room.torch_step < 3:
        raise SystemExit("torch_step %d would put pairs edge to edge" % room.torch_step)
    phase = (x - 1) % room.torch_step
    if phase not in (0, 1):
        return False
    return run is None or x - phase + 1 <= run


def spawn_points(room, blocked):
    """`(type, x, y)`, evenly around the room and never on a pillar.

    BY ANGLE, not by walking a list of free tiles. The first version took every n-th
    entry of a ring built side-by-side, and because the stride and the four-sided
    interleave shared a factor every point landed on the same wall - twelve monsters in
    one corner. One point per equal angle cannot do that.

    The radius is a fraction of the room, so the points ring the player rather than
    starting on top of them (the host lands the player mid-room -
    `guardian::landing_tile`). A point that falls on a pillar walks inwards until it is
    free. The type cycles, so each species slot gets the same share.

    The boss hall is different: its single type-0 point IS the boss and it stands north
    of centre in front of the torch wall, with the adds ringed behind it.
    """
    import math
    inner = room.inner
    cx, cy = 1 + (inner - 1) // 2, 5 + (inner - 1) // 2
    lo, hi = 1, inner                      # the floor's own bounds
    ylo, yhi = 5, 5 + inner - 1

    def place(angle, radius):
        for step in range(24):             # walk inwards, 4 % of the radius at a time
            f = 1.0 - step * 0.04
            x = min(max(int(round(cx + radius * f * math.cos(angle))), lo), hi)
            y = min(max(int(round(cy + radius * f * math.sin(angle))), ylo), yhi)
            if not blocked(x, y):
                return (x, y)
        return None

    def ring(n, radius, phase=0.0):
        out, seen = [], set()
        for i in range(n):
            p = place(2.0 * math.pi * i / n + phase, radius)
            k = 0
            while p in seen and k < 6:      # nudge off a taken tile
                k += 1
                p = place(2.0 * math.pi * i / n + phase + 0.13 * k, radius - k)
            if p is None or p in seen:
                continue
            seen.add(p)
            out.append(p)
        return out

    if room.boss:
        boss_at = place(-math.pi / 2.0, inner * 0.34)          # due north of centre
        adds = [p for p in ring(2 * BOSS_ADDS, inner * 0.42, 0.4) if p != boss_at]
        if boss_at is None or len(adds) < 2 * BOSS_ADDS:
            raise SystemExit("map %d: placed %d add(s), wanted %d"
                             % (room.map_id, len(adds), 2 * BOSS_ADDS))
        return [(0, boss_at[0], boss_at[1])] + [(1 + i % 2, x, y)
                                                for i, (x, y) in enumerate(adds)]
    if room.swarm:
        # 54 points do not fit on one circle of a 20-wide room (its circumference is
        # ~50 tiles), so a swarm is three concentric rings, outer to inner, each phased
        # so the rings do not line up into spokes. The innermost ring still keeps a
        # tile or two off the player's landing spot (`guardian::landing_tile` puts them
        # at the room's centre).
        picked, seen = [], set()
        for n, frac, phase in ((26, 0.44, 0.0), (20, 0.32, 0.17), (14, 0.20, 0.31)):
            for p in ring(n, inner * frac, phase):
                if p not in seen:
                    seen.add(p)
                    picked.append(p)
        if len(picked) < room.spawns:
            raise SystemExit("map %d: placed %d swarm point(s), wanted %d"
                             % (room.map_id, len(picked), room.spawns))
        picked = picked[:room.spawns]
    else:
        picked = ring(room.spawns, inner * 0.40)
        if len(picked) < room.spawns:
            raise SystemExit("map %d: placed %d spawn point(s), wanted %d"
                             % (room.map_id, len(picked), room.spawns))
    return [(i % 3, x, y) for i, (x, y) in enumerate(picked)]


def build_map(jar, room, patch):
    """The room's `map<id>_mp`: our frame, the donor's tiles, the donor's pillar."""
    frame = MC.parse_map(jar.read(map_member(FRAME_MAP)))
    w, h, inner = room.w, room.h, room.inner
    west, east = 1, w - 2              # the run-end diagonal columns (89차)
    tile = [VOID] * (w * h)
    attr = [BLOCKED] * (w * h)

    def put(x, y, t, blocked=True, mirror=False):
        tile[y * w + x] = t
        attr[y * w + x] = (BLOCKED if blocked else 0) | (MIRROR if mirror else 0)

    decor0, decor1, sprites = [], [], []
    # ─ the north wall ────────────────────────────────────────────────────────────────
    # cap, then a column per run column, then the two run-end diagonals. Every piece on
    # the RIGHT is the left piece with the mirror bit, as map 71 builds it.
    # ★★★ 89차: THE RUN SITS ONE COLUMN IN FROM THE MAP EDGE. `west`/`east` are the
    # run-end diagonals and the side wall is the column OUTSIDE each of them - see the
    # corner-stamp block below for why.
    put(west, 0, CAP_CORNER)
    put(east, 0, CAP_CORNER, mirror=True)
    run = east - west - 1              # face columns between the two diagonals
    for i, x in enumerate(range(west + 1, east), start=1):
        put(x, 0, CAP_RUN)
        torch = torch_at(room, i, run)
        col = TORCH_COL if torch else PLAIN_COL
        # The second column of a pair is the first mirrored (map 71: attr `80 c0` on all
        # four brazier rows; the plain pair `41 41 / 44 44 / 47 47 / 4a 4a` mirrors its
        # lower two rows). `torch_at` numbers pairs from the run's first column.
        second = (i - 1) % room.torch_step == 1
        for k in range(4):
            put(x, 1 + k, col[k], mirror=second and (torch or k >= 2))
        if torch and second:
            # The fixture's base lip (a mirrored pair, 16x7, anchored 7 px below the wall)
            # and its animated flame, centred between the two columns.
            base_y = 5 * 16 + 7
            decor0.append((0x00, TORCH_BASE, (x - 1) * 16 + 8, base_y))
            decor0.append((0x80, TORCH_BASE, (x - 1) * 16 + 24, base_y))
            sprites.append((FLAME_FLAGS, FLAME_SPRITE, x * 16, 7 * 16))

    # ─ the two corners, lifted from the donors instead of composed (89차) ─────────────
    #
    # ★★★ THE SIDE WALL IS ITS OWN COLUMN AND IT STANDS OUTSIDE THE DIAGONAL. Up to
    # vc304 the run-end diagonal `42 45 48 4b` / `4e` and the `55` side wall shared one
    # column, so the wall face jumped inwards by the diagonal's whole width the moment
    # it turned the corner - the player's "상단에서 좌우 연결부 -> 좌우 벽 이 부분이 좀
    # 어색한거같아 … 좌우를 한칸 씩 넓혀야 띄워야될거같아" (2026-09-12). The game NEVER
    # draws that: it puts the side wall one column further out and lets the diagonal's
    # column become FLOOR below the foot. Both donors carry the stamp, and the two are
    # exact mirror images of each other - which is the cross-check:
    #
    #        map 72 x=1/x=0, rows 11..16      map 71 x=28/x=29, rows 0..5
    #          diag  side                       diag  side
    #     r0   53    ff                         53*   ff        <- the run's cap corner;
    #     r1   42*   53                         42    53*          the side wall's own
    #     r2   45*   55                         45    55*          corner is ONE ROW
    #     r3   48*   55                         48    55*          LOWER, with void above
    #     r4   4b*   55                         4b    55*
    #     r5   4e*   55                         4e    55*
    #     r6   floor 55                         01.   55*       <- floor under the foot
    #
    # ⛔ So the foot `4e` carries the DIAGONAL's polarity (west mirrored, east plain),
    # not the side wall's. vc304 had it the other way because the player judged it in a
    # configuration the game never draws - the foot sitting directly on `55`. `verify`
    # now asserts both columns against the donor maps byte for byte, so this cannot
    # drift back into a guess (TODO CF: "원본 스탬프가 답이지 극성 추측이 아니다").
    for k in range(4):
        put(west, 1 + k, END_COL[k], mirror=True)
        put(east, 1 + k, END_COL[k])
    put(west, 5, END_FOOT, mirror=True)
    put(east, 5, END_FOOT)
    # the side walls: void at the cap row, their own corner one row lower, then down.
    put(0, 1, CAP_CORNER)
    put(w - 1, 1, CAP_CORNER, mirror=True)
    for y in range(2, h - 2):
        put(0, y, SIDE)
        put(w - 1, y, SIDE, mirror=True)
    # the shadow each side wall throws at its foot (map 71: id 150 at `x*16+8, last wall
    # row`, normal on a left-facing wall and mirrored on a right-facing one)
    decor0.append((0x00, WALL_SHADOW, 0 * 16 + 8, (h - 3) * 16))
    decor0.append((0x80, WALL_SHADOW, (w - 1) * 16 + 8, (h - 3) * 16))
    # south lip, then the void row
    put(0, h - 2, FOOT_CORNER)
    put(w - 1, h - 2, FOOT_CORNER, mirror=True)
    for x in range(1, w - 1):
        put(x, h - 2, FOOT_RUN)
    for x in range(w):
        put(x, h - 1, VOID)
    # The floor, mirror bits and all. It runs the full `inner` width from row 6 down -
    # including the two columns the diagonals stand in - and stops short of them on row
    # 5, which is the notch the corner stamp cuts. Cells the frame already wrote are
    # skipped rather than reasoned about.
    for yy in range(inner):
        for xx in range(inner):
            x, y = 1 + xx, 5 + yy
            if tile[y * w + x] != VOID:
                continue
            t, mir = patch[yy][xx]
            put(x, y, t, blocked=False, mirror=bool(mir))

    # the pillars, stamp (with its attrs) and statue both
    for (px, py) in pillar_centres(room):
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                a = PILLAR_ATTR[dy + 1][dx + 1]
                put(px + dx, py + dy, PILLAR_TILES[dy + 1][dx + 1],
                    blocked=bool(a & BLOCKED), mirror=bool(a & MIRROR))
        for (flags, ox) in ((0x00, 0), (0x80, 16)):
            decor1.append((flags, PILLAR_STATUE, px * 16 + ox, (py + 1) * 16))

    # lamps on the three walls whose faces this projection does not show
    if room.lamps:
        # ★ 89차: the side wall now starts at row 2 (it used to start at row 6, under the
        # diagonal), so the lamps start higher or the boss hall has four bare rows of
        # wall at each corner. Same parity as before, so the rhythm does not shift.
        for y in range(4, h - 2, LAMP_STEP):
            decor0.append((0x00, LAMP, 0 * 16 + 4, (y + 1) * 16))
            decor0.append((0x00, LAMP, (w - 1) * 16 + 4, (y + 1) * 16))
        for x in range(1, w - 1, LAMP_STEP):
            decor0.append((0x00, LAMP, x * 16 + 4, (h - 1) * 16))

    spawns = spawn_points(room, lambda x, y: bool(attr[y * w + x] & BLOCKED))
    kinds = sorted({t for (t, _x, _y) in spawns})
    # `obj` lists the DECORATION ids (banks of `obj_<n>_bm`); the flame is a sprite from
    # the `npc` list and is not an obj id (map 71: obj [149, 150], npc [5]).
    obj = sorted({r[1] for r in decor0 + decor1})

    out = bytearray()
    out += _blob(frame["theme"])        # [33, 22, 37] - map 72's, so slot 2 exists
    out += _blob(frame["tag"])
    out += _blob(room.name)
    out.append(frame["flag"])
    out.append(w)
    out.append(h)
    out += _blob(obj)                   # the decoration ids this map draws
    out += _blob(room.npc)              # the sprite banks its species need
    out += bytes(tile)
    out += bytes(attr)
    for layer in (decor0, decor1, sprites):
        out.append(len(layer))
        for (flags, gid, px, py) in layer:
            out += bytes([flags, gid, px & 0xFF, (px >> 8) & 0xFF, py & 0xFF, (py >> 8) & 0xFF])
    out += _blob(kinds)
    out.append(len(spawns))
    for (t, x, y) in spawns:
        out += bytes([t, x, y])
    return bytes(out)


# ---------------------------------------------------------------------------- monsters

# ── the species pools (88차) ───────────────────────────────────────────────────────
#
# ★★★ "공격능력이 없는 몬스터" IS A FIELD, NOT A GUESS. Every `enemyh_dat` row ends in a
# 21-byte stat tail; the row parser (`0x376c0`..`0x37830`) copies `tail[17..18]` as the
# u16 at species-table `+0x1e`, and the spawn copies it to the live record at `+0xb4`
# (measured: 도적's record reads `0a 00` there on the normal table, `'박스`'s reads
# `00 00`). It is ZERO for exactly nineteen species — 90/92 `'쥐`, 102 `'박쥐`, 145..152
# `'박스`, 153..160 (`'쥐 '벌 '유령 '먼지`) — and those are precisely the green-`'`
# critters the player watched not attacking. Everything else is non-zero (3..0x30) and
# fights. The prefix is only a cross-check; the field is the rule.
ATTACKLESS_OFF = 17

# The `{`-prefixed mini-bosses (Lv 43..67, HP 16k..28k): 29 카이저골렘, 35/36 로커, 38/39,
# 52 시바, 133 커브스쿠툼, 135, 136 커브이라(x3 HP), 138 다크니스이라, 139, 141 크레이지타이탄,
# 142, 144. They are the reaver-drop bosses in the wild; twelve of them at a floor's HP
# multiplier would be a wall, so the ordinary pool leaves them out and they stay boss
# territory (`gen_guardian_floors` decides what a boss floor spawns).
ELITE_HP = 10000


def species_rows(jar, hard=True):
    """`{species: (name, level, hp, atk, tail)}` for 0..160 off the HARD stat table."""
    from h3crypt import records
    out = {}
    en = "res/dat/enemyh_dat" if hard else "res/dat/enemy_dat"
    for i, r in enumerate(records(jar.read(en))):
        nl = r[0]
        name = r[1:1 + nl].decode("euc-kr", "replace")
        t = r[1 + nl:]
        hp = int.from_bytes(t[5:7], "little", signed=True)
        atk = int.from_bytes(t[7:9], "little", signed=True)
        out[i] = (name, t[0], hp, atk, bytes(t))
    return out


def attackless(rows):
    """Species that never attack: `tail[17..18] == 0`. Sorted."""
    return sorted(s for s, (_n, _lv, _hp, _atk, t) in rows.items()
                  if int.from_bytes(t[ATTACKLESS_OFF:ATTACKLESS_OFF + 2], "little") == 0)


def attackers(rows, min_level=SPECIES_MIN_LEVEL):
    """Ordinary fighting species at hard level >= `min_level`, elites excluded. Sorted."""
    harmless = set(attackless(rows))
    return sorted(s for s, (_n, lv, hp, _atk, _t) in rows.items()
                  if s not in harmless and lv >= min_level and hp < ELITE_HP)


def species_pools(jar, hard=True):
    """`(attackless, attackers, creature_table)` — the two pools both generators use."""
    rows = species_rows(jar, hard)
    return attackless(rows), attackers(rows), MC.creature_table(jar, hard)


def scene_plan():
    """`[(scene_id, room)]` in the order this tool writes them. Pure - no archive."""
    plan, sid = [], FIRST_SCENE
    for room in ROOMS:
        for _i in range(SCENES_PER_ROOM):
            plan.append((sid, room))
            sid += 1
    if sid - 1 > 255:
        raise SystemExit("scene ids ran past one byte (%d) - the warp destination is a byte" % (sid - 1))
    return plan


def hall_scenes():
    """`[scene, ...]` for the three ordinary halls, in rotation order. Pure."""
    return [s for s, r in scene_plan() if not r.boss and not r.swarm]


def swarm_scenes():
    """`[scene, ...]` for the three swarm halls, in rotation order. Pure."""
    return [s for s, r in scene_plan() if r.swarm]


def boss_scene():
    """The boss hall's scene. Pure."""
    return [s for s, r in scene_plan() if r.boss][0]


# The boss every `npc [0]` arena of the shipped game carries (멜페토). Only the FALLBACK
# header uses it — the floor table names the real boss per floor.
FALLBACK_BOSS = 164


def build_scenes(jar):
    """`{member: bytes}` for the seven scenes, each with a FALLBACK species triple.

    The host overrides the triple as the scene loads (see the header), so what is baked
    here only matters to an app without the hook. It is still a sane floor: fighters in
    a hall, a boss with two adds in the throne. Never a harmless critter.
    """
    harmless, fighters, _table = species_pools(jar)
    if len(fighters) < 3:
        raise SystemExit("species pool too small (%d fighters)" % len(fighters))
    donor, _ = parse_scn(jar.read(scene_member(DONOR_SCENE)))
    rng = random.Random(SPECIES_SEED)
    out = {}
    for sid, room in scene_plan():
        if room.boss:
            trio = [FALLBACK_BOSS] + rng.sample(fighters, 2)
        elif room.swarm:
            trio = rng.sample(harmless, 3)
        else:
            trio = rng.sample(fighters, 3)
        h = bytearray(donor[:9])
        h[0] = room.map_id
        h[3], h[4], h[5] = trio
        h[8] = 0xFF                    # no intro cutscene
        out[scene_member(sid)] = build_scn(bytes(h), [[ENTRY_ROW], [FADE_IN], []])
    return out


# ------------------------------------------------------------------------------ verify

def regions(m):
    """Sizes of the 4-connected walkable regions, biggest first."""
    from collections import deque
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
            out.append((len(reg), reg))
    out.sort(key=lambda kv: -kv[0])
    return out


def verify(jar):
    """Re-parse what we wrote and assert every property the tower depends on."""
    # The donor maps are the source of truth for the vocabulary. If a donor ever
    # changes, these assertions fail rather than the rooms silently drawing rubble.
    wall = MC.parse_map(jar.read(map_member(WALL_MAP)))
    ww, wh = wall["w"], wall["h"]
    have = set()
    for y in range(5, wh):
        for x in range(ww):
            if not (wall["attr"][y * ww + x] & BLOCKED) and (wall["attr"][(y - 1) * ww + x] & BLOCKED):
                have.add(tuple(wall["tile"][(y - k) * ww + x] for k in range(1, 6)))
    for col, what in ((TORCH_COL, "brazier"), (PLAIN_COL, "plain")):
        want = (col[3], col[2], col[1], col[0], CAP_RUN)
        if want not in have:
            raise SystemExit("map %d no longer carries the %s wall column %s"
                             % (WALL_MAP, what, [hex(v) for v in want]))
    west_stamp = corner_stamp(jar, *WEST_CORNER_SRC)
    east_stamp = corner_stamp(jar, *EAST_CORNER_SRC)
    if west_stamp != mirror_stamp(east_stamp):
        raise SystemExit("the two donor corners are no longer mirror images of each other")
    pil = MC.parse_map(jar.read(map_member(PILLAR_MAP)))
    pw = pil["w"]
    found = 0
    for y in range(1, pil["h"] - 1):
        for x in range(1, pw - 1):
            block = tuple(tuple(pil["tile"][(y + dy) * pw + x + dx] for dx in (-1, 0, 1))
                          for dy in (-1, 0, 1))
            if block == PILLAR_TILES:
                found += 1
                if not (pil["attr"][y * pw + x] & BLOCKED):
                    raise SystemExit("map %d pillar at (%d,%d) is not blocked" % (PILLAR_MAP, x, y))
    if found < 1:
        raise SystemExit("map %d no longer contains the 3x3 pillar stamp" % PILLAR_MAP)

    rooms = []
    for room in ROOMS:
        m = MC.parse_map(jar.read(map_member(room.map_id)))
        pillar_cells = {(px + dx, py + dy) for (px, py) in pillar_centres(room)
                        for dy in (-1, 0, 1) for dx in (-1, 0, 1)}
        if m["end"] != m["size"]:
            raise SystemExit("map %d does not parse to EOF (%d of %d)"
                             % (room.map_id, m["end"], m["size"]))
        if (m["w"], m["h"]) != (room.w, room.h):
            raise SystemExit("map %d is %dx%d, wanted %dx%d"
                             % (room.map_id, m["w"], m["h"], room.w, room.h))
        w, a = m["w"], m["attr"]
        # Every tile id must be one the game can draw: slot < len(theme) and the index
        # inside that strip. 0xff is the void. This is the check that the solved
        # encoding buys us - a mistyped id cannot reach the device.
        for v in set(m["tile"]):
            if v == VOID:
                continue
            if ((v >> 6) & 3) >= len(m["theme"]):
                raise SystemExit("map %d tile %#02x names theme slot %d of %d"
                                 % (room.map_id, v, (v >> 6) & 3, len(m["theme"])))
        walk = sum(1 for i in range(m["w"] * m["h"]) if not (a[i] & BLOCKED))
        # `inner x inner`, less the two tiles on row 5 that the corner diagonals stand
        # in (89차 - the side wall moved out one column and the diagonals took row 5).
        want_walk = room.inner * room.inner - 2 - len(pillar_centres(room))
        if walk != want_walk:
            raise SystemExit("map %d has %d walkable tile(s), wanted %d"
                             % (room.map_id, walk, want_walk))
        # ★ 88차: every brazier on the north wall is a PAIR — the flame tile 0x46 at row
        # 3 must have a twin to exactly one side, never stand alone. That is the whole of
        # the "횃불이 반 잘린" fix, and this is what keeps it fixed.
        flames = [x for x in range(w) if m["tile"][3 * w + x] == TORCH_COL[2]]
        for x in flames:
            twins = [t for t in (x - 1, x + 1) if t in flames]
            if len(twins) != 1:
                raise SystemExit("map %d: the brazier at column %d is not one half of a pair"
                                 % (room.map_id, x))
            # ★ 88차-B: the pair is a LEFT half and a MIRRORED left half — attr `80 c0`
            # on every brazier row, exactly as map 71 carries it.
            left = min(x, twins[0])
            for k in range(1, 5):
                if a[k * w + left] & MIRROR or not (a[k * w + left + 1] & MIRROR):
                    raise SystemExit("map %d: brazier at %d,%d is not `80 c0` mirrored"
                                     % (room.map_id, left, k))
        if not flames:
            raise SystemExit("map %d has no brazier on its north wall" % room.map_id)
        pairs = len(flames) // 2
        bases = [r for r in m["layers"][0] if r["id"] == TORCH_BASE]
        flame_sprites = [r for r in m["layers"][2] if r["id"] == FLAME_SPRITE and r["flags"] & FLAME_FLAGS]
        if len(bases) != 2 * pairs or len(flame_sprites) != pairs:
            raise SystemExit("map %d: %d brazier(s) but %d base lip(s) and %d flame sprite(s)"
                             % (room.map_id, pairs, len(bases), len(flame_sprites)))
        # ★★★ 89차: THE TWO CORNERS ARE THE DONORS' OWN BYTES. Not a polarity rule that
        # can be argued about - the six rows of `(diagonal, side wall)` are compared
        # against map 72's west corner and map 71's east corner exactly as they stand
        # there, which is what TODO CF asked for. Anything that drifts fails here.
        for stamp, dx, sx in ((west_stamp, 1, 0), (east_stamp, w - 2, w - 1)):
            got = [((m["tile"][y * w + dx], bool(a[y * w + dx] & MIRROR),
                     bool(a[y * w + dx] & BLOCKED)),
                    (m["tile"][y * w + sx], bool(a[y * w + sx] & MIRROR),
                     bool(a[y * w + sx] & BLOCKED))) for y in range(6)]
            if got != stamp:
                raise SystemExit(
                    "map %d: corner at x=%d is not the donor's stamp"
                    "\n  wanted %s" "\n  got    %s"
                    % (room.map_id, dx, stamp, got))
        # and below the stamp: the side wall runs to the lip, the diagonal's column is
        # floor. That is the widening the player asked for, so it is asserted too.
        for y in range(6, m["h"] - 2):
            for sx, mir in ((0, False), (w - 1, True)):
                if m["tile"][y * w + sx] != SIDE or bool(a[y * w + sx] & MIRROR) != mir:
                    raise SystemExit("map %d: side wall at (%d,%d) is not %#02x"
                                     % (room.map_id, sx, y, SIDE))
            for dx in (1, w - 2):
                if a[y * w + dx] & BLOCKED and (dx, y) not in pillar_cells:
                    raise SystemExit("map %d: (%d,%d) is walled - the diagonal's column"
                                     " must be floor below its foot" % (room.map_id, dx, y))
        want_spawns = 1 + 2 * BOSS_ADDS if room.boss else room.spawns
        if len(m["spawns"]) != want_spawns:
            raise SystemExit("map %d has %d spawn point(s), wanted %d"
                             % (room.map_id, len(m["spawns"]), want_spawns))
        per_type = [sum(1 for (t, _x, _y) in m["spawns"] if t == k) for k in range(3)]
        if room.swarm and min(per_type) * 3 < 50:
            raise SystemExit("map %d: a swarm with all three slots live is %d < 50"
                             % (room.map_id, min(per_type) * 3))
        if len(set(m["tile"])) < 8:
            raise SystemExit("map %d uses only %d distinct tile id(s) - a floor of one id"
                             " reads as a texture, which is the bug 86차-B fixed"
                             % (room.map_id, len(set(m["tile"]))))
        regs = regions(m)
        if len(regs) != 1:
            raise SystemExit("map %d walkable area is in %d pieces - a floor must be one room"
                             % (room.map_id, len(regs)))
        biggest = regs[0][1]
        for (t, x, y) in m["spawns"]:
            if a[y * w + x] & BLOCKED:
                raise SystemExit("map %d spawn (%d,%d) is on a wall" % (room.map_id, x, y))
            if (x, y) not in biggest:
                raise SystemExit("map %d spawn (%d,%d) is not reachable" % (room.map_id, x, y))
            if t not in m["kinds"]:
                raise SystemExit("map %d spawn type %d missing from kinds" % (room.map_id, t))
        if list(m["npc"]) != list(room.npc):
            raise SystemExit("map %d npc %s, wanted %s" % (room.map_id, m["npc"], list(room.npc)))
        rooms.append(m)

    harmless, fighters, _table = species_pools(jar)
    scenes = []
    for sid, room in scene_plan():
        head, lists = parse_scn(jar.read(scene_member(sid)))
        if head[0] != room.map_id:
            raise SystemExit("scene %d points at map %d, wanted %d" % (sid, head[0], room.map_id))
        if lists[0] != [ENTRY_ROW] or lists[1] != [FADE_IN] or lists[2]:
            raise SystemExit("scene %d scripts are not the minimal pair" % sid)
        live = [s for s in head[3:6] if s != MC.SPECIES_NONE]
        if len(live) != 3:
            raise SystemExit("scene %d fallback triple has %d live species, wanted 3" % (sid, len(live)))
        if room.boss and not any(s >= MC.BOSS_BASE for s in live):
            raise SystemExit("boss scene %d has no boss species (%s)" % (sid, live))
        if room.swarm and any(s not in harmless for s in live):
            raise SystemExit("swarm scene %d fallback carries a fighter (%s)" % (sid, live))
        if not room.boss and not room.swarm and any(s not in fighters for s in live):
            raise SystemExit("hall scene %d fallback carries a non-fighter (%s)" % (sid, live))
        scenes.append((sid, head))
    for sid in RETIRED_SCENES:
        if scene_member(sid) in jar.namelist():
            raise SystemExit("retired scene %d is still in the archive" % sid)
    for mid in RETIRED_MAPS:
        if map_member(mid) in jar.namelist():
            raise SystemExit("retired map %d is still in the archive" % mid)
    return rooms, scenes


# ------------------------------------------------------------------------------- chain

def open_jar(zpath):
    z = zipfile.ZipFile(zpath)
    names = [n for n in z.namelist() if n.endswith(".jar")]
    if len(names) != 1:
        raise SystemExit("expected exactly one .jar in %s, found %d" % (zpath, len(names)))
    return z, names[0]


def build(zpath):
    z, jarname = open_jar(zpath)
    jar = zipfile.ZipFile(io.BytesIO(z.read(jarname)))
    want, subs = {}, {}
    for room in ROOMS:
        patch, donor, n = floor_patch(jar, room.inner)
        subs[room.map_id] = (donor, n)
        want[map_member(room.map_id)] = build_map(jar, room, patch)
    want.update(build_scenes(jar))
    return z, jarname, jar, want, subs


def present(jar):
    """True when this archive already carries the rooms (used by the floor table)."""
    try:
        for room in ROOMS:
            jar.read(map_member(room.map_id))
        for sid, _room in scene_plan():
            jar.read(scene_member(sid))
    except KeyError:
        return False
    names = jar.namelist()
    return (not any(scene_member(sid) in names for sid in RETIRED_SCENES)
            and not any(map_member(mid) in names for mid in RETIRED_MAPS))


def _summary(want, subs):
    for room in ROOMS:
        m = MC.parse_map(want[map_member(room.map_id)])
        print("  map %3d %-16s %2dx%-2d  %3d walkable  %2d pillar(s)  %2d tile ids"
              "  %2d decor  %2d spawns  (floor = map %d, %d cell(s) filled)"
              % (room.map_id, room.name.decode(), m["w"], m["h"],
                 sum(1 for i in range(m["w"] * m["h"]) if not (m["attr"][i] & BLOCKED)),
                 len(pillar_centres(room)), len(set(m["tile"])),
                 sum(len(x) for x in m["layers"]), len(m["spawns"]),
                 subs[room.map_id][0], subs[room.map_id][1]))
    for sid, room in scene_plan():
        head, _ = parse_scn(want[scene_member(sid)])
        kind = "boss" if room.boss else "swarm" if room.swarm else "hall"
        print("  scene %3d -> map %3d  %-5s fallback species %s" % (sid, head[0], kind, list(head[3:6])))


def rewrite(zpath, apply):
    z, jarname, jar, want, subs = build(zpath)
    have = {}
    for name in want:
        try:
            have[name] = jar.read(name)
        except KeyError:
            have[name] = None
    retired = [scene_member(sid) for sid in RETIRED_SCENES if scene_member(sid) in jar.namelist()]
    retired += [map_member(mid) for mid in RETIRED_MAPS if map_member(mid) in jar.namelist()]
    if all(have[n] == want[n] for n in want) and not retired:
        print("%s: the tower rooms are already present (%d maps, %d scenes)"
              % (zpath, len(ROOMS), len(scene_plan())))
        return 0
    if retired:
        print("retiring %d stale scene(s): %s" % (len(retired), retired))
    print("tower rooms: %d map(s) + %d scene(s)" % (len(ROOMS), len(scene_plan())))
    _summary(want, subs)
    if not apply:
        print("(dry run - pass --apply to write)")
        return 0

    backup = os.path.join("android_app/asset_backup", os.path.basename(zpath) + ".prerooms")
    if not os.path.exists(backup):
        os.makedirs(os.path.dirname(backup), exist_ok=True)
        shutil.copyfile(zpath, backup)
        print("backup -> %s" % backup)

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as out:
        for it in jar.infolist():
            if it.filename in retired:
                continue
            out.writestr(it, want.get(it.filename) or jar.read(it.filename))
        for name, data in sorted(want.items()):
            if name not in jar.namelist():
                out.writestr(name, data)
    newjar = buf.getvalue()

    tmp = zpath + ".new"
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as out:
        for it in z.infolist():
            out.writestr(it, newjar if it.filename == jarname else z.read(it.filename))
    z.close()
    # ⛔ `os.replace` can lose to a Windows file lock (`PermissionError: WinError 5`)
    # and the traceback alone gets swallowed by a pipe - 84차-B shipped a half-patched
    # archive that way. Retry, and the chain rule is `--apply` then `--check` again.
    for attempt in range(8):
        try:
            os.replace(tmp, zpath)
            break
        except PermissionError:
            if attempt == 7:
                raise
            import time
            time.sleep(0.4)

    z2, jn2 = open_jar(zpath)
    back = zipfile.ZipFile(io.BytesIO(z2.read(jn2)))
    for name, data in want.items():
        if back.read(name) != data:
            raise SystemExit("write-back verification FAILED for %s" % name)
    verify(back)
    print("%s: written and verified" % zpath)
    return 0


def check(zpath):
    z, jarname, jar, want, subs = build(zpath)
    missing = [n for n in want if n not in jar.namelist()]
    if missing:
        print("NOT APPLIED: %s is missing %s" % (zpath, missing[:4]))
        return 1
    bad = [n for n in want if jar.read(n) != want[n]]
    if bad:
        print("NOT APPLIED: %s differs from what this script generates: %s" % (zpath, bad[:4]))
        return 1
    rooms, scenes = verify(jar)
    print("CHECK OK: %s carries the tower rooms" % zpath)
    _summary(want, subs)
    return 0


def preview(zpath, outdir):
    """Render the rooms to PNG so a human can look before anything is written."""
    import render_maps as RM
    from PIL import Image
    z, jarname, jar, want, subs = build(zpath)
    _summary(want, subs)
    os.makedirs(outdir, exist_ok=True)
    art = RM.Art(jar)
    cells = []
    for room in ROOMS:
        m = MC.parse_map(want[map_member(room.map_id)])
        im, miss = RM.render(m, art, attr=False, spawns=True, decor=True)
        im.convert("RGB").save(os.path.join(outdir, "room%d.png" % room.map_id))
        plain, _ = RM.render(m, art, attr=True, spawns=False, decor=False)
        plain.convert("RGB").save(os.path.join(outdir, "room%d_attr.png" % room.map_id))
        cells.append((im, "map %d %s %dx%d  spawns %d  unresolved %d"
                      % (room.map_id, room.name.decode(), m["w"], m["h"], len(m["spawns"]), miss)))
        if miss:
            raise SystemExit("map %d has %d unresolvable tile id(s)" % (room.map_id, miss))
    sheet = RM.sheet(cells, 2, 460, 460)
    sheet.save(os.path.join(outdir, "rooms.png"))
    print("preview -> %s" % outdir)
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("zip")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--preview")
    a = ap.parse_args(argv)
    if a.preview:
        return preview(a.zip, a.preview)
    return check(a.zip) if a.check else rewrite(a.zip, a.apply)


if __name__ == "__main__":
    raise SystemExit(main())
