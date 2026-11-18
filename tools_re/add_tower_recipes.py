"""Write the 하늘성 조합서 into i15_dat — 78 recipes for the `"` legacy items, each needing
`{닉스코어파편` (138차, TOWER_REWARD_BALANCE_DRAFT.md §5; decisions ⑩ ⑮ ⑱ ⑲ ⑳).

★ 142차: re-costed for the 85 / 90제 rework — see the block above `part_shards` (85제 no
base, 90제 = the same slot's 85제 piece, 전설 = its 133차 base, shards 10 / 20 / 30). The
row ORDER below is still the 133차 one (a 조합서's idx is a save-file identity); the list
that follows is the 133차 costing, kept as the record of what the order was built from.

WHAT IT WRITES. One 조합서 per new legacy item (the 78 `"` rows `add_legacy_items.py` /
`add_legacy_boots.py` put in i0..i12), in LEVEL order:

  * the ENTRY tier — Lv73 set · Lv74 weapons · Lv75 accessories — needs 3 shards and NO
    base (15 rows): a Lv70 white is two faction items and cannot be one base, and the user's
    rule is "저레벨은 파편 3개";
  * every tier above needs the SAME slot one tier down as its base, plus shards on the
    ladder 갑옷 3/6/10/15/26/30 (Lv73/79/85/91/97/99) × 투구·방패 0.93 · 장갑·신발 0.90,
    weapons 3/8/15/30 (Lv74/82/90/99), accessories 3/10/20 (Lv75/85/95) — §5.2, whose
    example 30 / 28 / 27 the part factors reproduce exactly;
  * no second material (⑱ — a random 고대 재료 would make one of them the bottleneck and
    break "Lv97 세트 5부위 = 4런"), success rate 100 (⑮), craft fee 8,000 (the shipped
    top-tier recipes' price — see below).

WHERE. ⑳: over the original NETWORK recipes 0x27..0x58 (50 rows no offline source can give
— `getitem_dat` names 0x00..0x26 only and no drop, shop, cash or hell table holds cat 0x0f)
plus 28 appended = 117 rows. ★★★ 127 IS THE REAL CEILING: the parse loop reads the count
with `ldrsb` (0x53eca / 0x53f86) AND treats the ROW INDEX as a signed byte (0x53ef4/0x53ef8
`lsls/asrs #24` before the `muls`, 0x53f84..0x53f8a the same for the continue test, the
teardown's 0x526f0 `asrs`) — a 128th row would index in front of the table.

⚠ THE FILE IS NOT ENOUGH — `binary_patches.toml` must carry the matching guest bytes, the
same three the 85차 material row needed: count table `0x5e483` 0x59 -> 0x75, the
allocation `0x5373c movs r0,#0xb2 -> #0xea` (`lsls #4` after it: 117 x 32 = 0xea << 4) and
the teardown bound `0x526f2 cmp r3,#0x59 -> #0x75`. SHIP TOGETHER.

HOW THE GAME READS A ROW (disassembly, 138차):

    [u8 len][name][u32 price][u8 len][text][base cat][base idx][mat1 idx][n1][mat2 idx][n2][rate][out cat][out idx]

  * `price` is the CRAFT FEE: the special-craft key (0x22400) refuses when 소지금 < row+8,
    and 조합하기 (0x24fd4) subtracts it (0x250e6) — it is not only a sale price.
  * Both the check (0x224b8..0x2252a) and the craft (0x25096..0x250e0) walk the three
    ingredients as (cat, idx, count) and SKIP one whose idx / cat is 0xff — the base row's
    test is `ldrb [row+0x14]; cmp #0xff; beq` (0x224f8 / 0x250b6), and the painter
    (0x222a0) draws that row like an absent second material. So a no-base row is supported.
  * The ingredients are spent BEFORE the roll (0x2509c..0x250dc); on success the recipe
    itself is spent too — `spend(0x0f, idx, 1)` at 0x25130 — and the output given.
  * `rate` is `rand(1,100) <= rate` (0x250f6); 100 never fails.

The text is display only (the game reads the tail) and GENERATED, never written by hand.
★ 151차 (2026-10-01, user): 「레벨 N 슬롯;[베이스;]닉스코어파편 N개;@<the result's options>」 —
a no-base recipe has no 「베이스 없음」 segment, and the result item's own stat lines follow the
materials, read off the FINAL i0..i12 row `bind` builds (main 방/공 · the two option pairs with
the game's labels `InGame_txt[80 + id]` and its `%` rule `0x6d34` · an accessory's third option
from `acc_options`), so a change to the item tables moves the text with it. The game shows the
text in three places: the list/bag 효과 box prints the FIRST segment, and the 상세보기 body
prints every `;` segment on its own line (`0x3878`, 168 px wrap, 13 px pitch, 7 lines before the
frame — the longest recipe is 6). The `@` (white) before the first option line sets the item's
options apart from the red materials (the name's `"` colour carries over). The 효과 box's second
line — the game's 「<name>의조합설명서」 — becomes 「닉스코어파편 N개」 in the host's sprintk
(`gen_recipe_effects.py` -> `wie_util::recipe_effects`).

THE RESULT (cat, idx) IS NEVER TYPED. `bind` runs the chain's OWN i0..i12 builds (the
handlers `patch_asset.TARGETS` holds before this one) on the pristine jar and finds each
result by name among the `"` rows, so a change to the legacy tables moves the recipes with
it — and a name that does not resolve refuses the build.

CONTRACT: owned by `patch_asset.py` (registered AFTER `add_legacy_items`), fed the PRISTINE
member, verified against the pristine bytes (rows 0x00..0x26 byte-identical). The member
is ciphertext: `h3crypt.decrypt_bytes` / `encrypt_member` with the jar's own `des_dat`,
whose encoder `h3crypt.py --check` proves byte-exact on all eight shipped members.
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dump_item_db as DB          # noqa: E402
import gen_legacy_items as GL      # noqa: E402
import h3crypt                     # noqa: E402
import acc_options as AO           # noqa: E402  (151차: an accessory's third option)

MEMBER = "i15_dat"
FIRST = 0x27               # the first overwritten row (the network recipes are 0x27..0x58)
SHIPPED_ROWS = 0x59        # 89 — the count table byte this tool's rows depend on
TOTAL_ROWS = 0x75          # 117 = 0x27 + 78, under the signed-row ceiling 127
SHARD_IDX = 0x2E           # `{닉스코어의 파편` (cat 0x0e) — add_tower_material.py
SHARD_NAME = "닉스코어파편"   # 151차 (was 「닉스코어의 파편」)
PRICE = 8000               # craft fee (see the header) — the shipped top recipes' own price
RATE = 100                 # decision ⑮
NONE = 0xFF

# The other parts scale the 갑옷 cost (per mille, round half up) — §5.2, kept in 142차.
PART_PER_MILLE = {1: 1000, 0: 930, 11: 930, 2: 900, 3: 900}   # 갑옷 · 투구/방패 · 장갑/신발
ACC_CAT = 12

# The accessory chain (§5.3): 85 <- 75, 95 <- 85, by concept.
ACC_BASE = {
    "이안의맹세": "레갈리스교단반지", "레아의목걸이": "엘지스의증표", "일레느의각인": "율리케의목걸이",
    "클레르의가보": "이안의맹세", "닉스의펜던트": "레아의목걸이", "굴베이그의탈리스만": "일레느의각인",
}
# The Lv99 armour legends' base (§5.3): the Lv97 piece whose concept continues, or the Lv91
# 코르버스 piece where a slot has three legends.
LEGEND_BASE = {
    "멜하르의흉갑": "빙해의갑옷", "율리케의성의": "붉은마녀로브", "광전사의흑갑": "코르버스암살복",
    "예언자의눈": "빙해의투구", "장로회의관": "붉은마녀헬름",
    "집행자의손": "빙해의장갑", "오딘의손길": "붉은마녀장갑", "키드의발톱": "코르버스크로우",
    "바질의발걸음": "붉은마녀신발", "섬광의군화": "빙해의장화",
    "영혼의수호": "왕국의철벽",
}
# The recipe text's slot word — the shipped recipes' own (`레벨 12 라이플`, `레벨 22 다크스톤`)
# and the game's category name for accessories (InGame_txt 76 `장신구`).
SLOT = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", 4: "스피어", 5: "스워드", 6: "나이프",
        7: "건", 8: "라이플", 9: "다크스톤", 10: "홀리스톤", 11: "방패", 12: "장신구"}


# ★★ 142차 (2026-09-30) — the 85 / 90제 rework (`LEGACY_ITEMS_REWORK_85_90.md §6.4`).
# Recipes are re-costed by the RESULT's new tier; the ROW ORDER stays the 133차 one:
# ⛔⛔ a 조합서's (0x0f, idx) is a save-file identity, and the rows were sorted by the old
# 요구레벨 — re-sorting by the new one would hand every owned 조합서 a different product.
# So `lv` below stays the OLD level (the sort key) and `new_lv` is what the text shows.
#
#   85제: no base · shards 갑옷 10 (투구·방패 x0.93 · 장갑·신발 x0.90) · weapons 10 · 악세 6
#   90제 (not a legend): the SAME slot's 85제 piece + shards 20 / 20 / 15
#       sets: 코르버스 <- 레갈리스근위 · 붉은마녀 <- 크루세이더 · 빙해 <- 템플러
#       weapons: hero3 rung <- hero2 rung · 악세: ACC_BASE (the 85제 accessory of the concept)
#   전설: the base it had (a 90제 item) + shards 30
#
# ★ 150차 (2026-10-01): an accessory's 요구레벨 is 1 now (`acc_options.REQ_LV`), so its recipe
# text says 「레벨 1 장신구」 (`new_lv`); its COST still follows its tier (`tier`, 85 / 90제 —
# `ACC_TIER`, `ACC_BASE`). ⛔ The sort key `lv` and so every recipe's idx are unchanged: only the
# nine accessory recipes' TEXT moves (one byte shorter each).
C_ARMOUR_TIER = {85: 10, 90: 20, "legend": 30}
WEAPON_TIER = {85: 10, 90: 20, "legend": 30}
ACC_TIER = {85: 6, 90: 15}
SET90_BASE = {"코르버스": "레갈리스근위", "붉은마녀": "크루세이더", "빙해": "템플러"}


def part_shards(cat, key):
    return (C_ARMOUR_TIER[key] * PART_PER_MILLE[cat] + 500) // 1000


def plan():
    """`[dict(result, base, shards, lv, new_lv, tier, cat)]` — the 78 recipes by NAME, in row
    order. `lv` is the 133차 요구레벨 (the SORT KEY, frozen — see above), `tier` the 142차 85 /
    90제 the cost follows, `new_lv` the 요구레벨 the recipe text prints (= the tier, but 1 for an
    accessory since 150차). Pure: no archive."""
    sets = {s[1]: s[3] for s in GL.SETS}
    set_lv = {s[1]: s[0] for s in GL.SETS}
    rows = []

    def add(result, base, shards, lv, cat):
        tier = GL.rw_tier(lv)
        new_lv = GL.RW_ACC_REQ_LV if cat == ACC_CAT else tier
        rows.append(dict(result=result, base=base, shards=shards, lv=lv,
                         new_lv=new_lv, tier=tier, cat=cat))

    # 85제 sets — no base.
    for sname in ("템플러", "크루세이더", "레갈리스근위"):
        for cat in GL.SET_PARTS:
            add(sets[sname][cat], None, part_shards(cat, 85), set_lv[sname], cat)
    # 90제 sets — the 85제 piece of the same slot.
    for sname, below in SET90_BASE.items():
        for cat in GL.SET_PARTS:
            add(sets[sname][cat], sets[below][cat], part_shards(cat, 90), set_lv[sname], cat)
    # Weapons: hero1 / hero2 rungs are 85제 (no base), the hero3 rung 90제 (<- hero2),
    # the legend <- the hero3 rung.
    legend_weapon = {c: nm for c, nm, *_ in GL.LEGENDS_WEAPON}
    for cat in sorted(GL.WEAPONS):
        names = [n[0] for n in GL.WEAPONS[cat][2]]
        add(names[0], None, WEAPON_TIER[85], GL.WLV[0], cat)
        add(names[1], None, WEAPON_TIER[85], GL.WLV[1], cat)
        add(names[2], names[1], WEAPON_TIER[90], GL.WLV[2], cat)
        add(legend_weapon[cat], names[2], WEAPON_TIER["legend"], GL.LEG_LV, cat)
    # The armour legends. ⛔ By CATEGORY, not by `GL.LEGENDS_ARMOUR`: the shield legend
    # 영혼의수호 (cat 11) is defined in `LEGENDS_WEAPON`, and that list split would drop it.
    for cat, nm, *_ in GL.LEGENDS:
        if cat not in GL.WEAPONS:
            add(nm, LEGEND_BASE[nm], part_shards(cat, "legend"), GL.LEG_LV, cat)
    # Accessories: 85제 no base, 90제 <- ACC_BASE.
    for name, lv, *_ in GL.ACCESSORIES:
        tier = GL.rw_tier(lv)
        add(name, ACC_BASE[name] if tier == 90 else None, ACC_TIER[tier], lv, ACC_CAT)
    rows.sort(key=lambda r: (r["lv"], r["cat"]))      # stable: ties keep the order above
    return rows


def legacy_index(members):
    """`{name: (cat, idx)}` for every `"` row of the final i0..i12 (`members[cat]` bytes)."""
    out = {}
    for cat in range(13):
        for idx, _off, body in DB.records(members[cat]):
            nl = body[0]
            raw = body[1:1 + nl]
            if raw and raw[0] == GL.NEW_MARK:
                name = raw[1:].decode("euc-kr")
                if name in out:
                    raise SystemExit(f"legacy name {name} appears twice")
                out[name] = (cat, idx)
    return out


def resolve(rows, index):
    """The plan with every (cat, idx) filled in — a name that does not resolve refuses."""
    out = []
    for r in rows:
        if r["result"] not in index:
            raise SystemExit(f"recipe result {r['result']} is not a legacy row")
        oc, oi = index[r["result"]]
        if oc != r["cat"]:
            raise SystemExit(f"{r['result']} is in cat {oc}, the plan says {r['cat']}")
        if r["base"] is None:
            bc = bi = NONE
        else:
            if r["base"] not in index:
                raise SystemExit(f"recipe base {r['base']} is not a legacy row")
            bc, bi = index[r["base"]]
        out.append(dict(r, out=(oc, oi), base_at=(bc, bi)))
    return out


def euc(s):
    return s.encode("euc-kr")


# ★ 151차 — the result's options under the materials (see the header).
# The game's `%` rule, `0x6d34(buf, id)`: ids 0..3 · 5..8 · 0x0e..0x12 get a '%', and 150차
# widens it to the host option 0x78 (5초당 HP회복).
PCT_IDS = frozenset({0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x07, 0x08,
                     0x0E, 0x0F, 0x10, 0x11, 0x12, 0x78})
ARMOUR_LIKE = frozenset({0, 1, 2, 3, 11})     # the detail window's 「방」 line; weapons print 「공」
OPTIONS_COLOUR = "@"                         # white — the name's red `"` carries over otherwise
LABEL_BASE = 80                              # InGame_txt[80 + id] = the option's label (「물리방어 +」)


def ingame_labels(blob):
    """`InGame_txt` rows as text (`u16 total · u16 count · u16 offsets[count]` · NUL strings)."""
    count = struct.unpack_from("<H", blob, 2)[0]
    offs = struct.unpack_from(f"<{count}H", blob, 4)
    return [blob[o:blob.index(b"\0", o)].decode("euc-kr") for o in offs]


def option_text(labels, sid, val):
    val = val - 0x100 if val > 0x7F else val     # the pair's value is an i8
    return f"{labels[LABEL_BASE + sid]}{val}{'%' if sid in PCT_IDS else ''}"


def result_options(cat, name, stats, labels):
    """The result item's stat lines, as its own 상세보기 shows them: 「방 206」/「공 1053」 (an
    accessory's 성능 = 기절 저항 only when non-zero — 0 since 150차), the two option pairs, and
    an accessory's third option (`acc_options`, the summary wording that fits one line)."""
    out = []
    if cat == ACC_CAT:
        perf = struct.unpack_from("<H", stats, 10)[0]
        if perf:
            out.append(f"기절 저항 +{perf}")
        pairs = ((stats[14], stats[15]), (stats[16], stats[17]))
    else:
        main = struct.unpack_from("<H", stats, 12)[0]
        out.append(f"{'방' if cat in ARMOUR_LIKE else '공'} {main}")
        pairs = ((stats[16], stats[17]), (stats[18], stats[19]))
    out += [option_text(labels, sid, val) for sid, val in pairs if sid or val]
    if cat == ACC_CAT:
        third = AO.by_name().get(name)
        if third:
            out.append(third[5])
    return out


def item_rows(members):
    """`{(cat, idx): (name without its colour escape, stat bytes)}` of the final i0..i12."""
    out = {}
    for cat in range(13):
        for idx, _off, body in DB.records(members[cat]):
            _nm, _mark, _colour, name, used = DB.split_name(body)
            out[(cat, idx)] = (name, body[used:])
    return out


def text_segments(r):
    """The recipe text's `;` segments — see the header."""
    segs = [f"레벨 {r['new_lv']} {SLOT[r['cat']]}"]
    if r["base"] is not None:
        segs.append(r["base"])
    segs.append(f"{SHARD_NAME} {r['shards']}개")
    opts = list(r.get("opts", ()))
    if opts:
        opts[0] = OPTIONS_COLOUR + opts[0]
    return segs + opts


def record(r):
    """One i15 record body (no length prefix)."""
    name = euc('"' + r["result"])
    text = euc(";".join(text_segments(r)))
    if len(name) > 0xFF or len(text) > 0xFF:
        raise SystemExit(f"{r['result']}: a field is over 255 bytes")
    bc, bi = r["base_at"]
    oc, oi = r["out"]
    tail = bytes([bc, bi, SHARD_IDX, r["shards"], NONE, 0, RATE, oc, oi])
    return bytes([len(name)]) + name + struct.pack("<I", PRICE) + bytes([len(text)]) + text + tail


def records(plain):
    """The record bodies of a decrypted member, by `h3crypt.records` (trailing padding
    dropped)."""
    return h3crypt.records(plain)


def pack(bodies):
    return b"".join(struct.pack("<H", len(b)) + b for b in bodies)


class Handler:
    """The i15_dat member. `bind` must run first (patch_asset calls it with the jar)."""

    __name__ = "add_tower_recipes"

    def __init__(self, targets):
        self.targets = targets
        self.des = None
        self.rows = None

    def bind(self, jar_bytes):
        import io
        import zipfile
        j = zipfile.ZipFile(io.BytesIO(jar_bytes))

        def member(base):
            names = [n for n in j.namelist() if n.rsplit("/", 1)[-1] == base]
            if len(names) != 1:
                raise SystemExit(f"expected exactly one {base} in the jar, got {names}")
            return j.read(names[0])

        self.des = member("des_dat")
        final = {}
        for cat in range(13):
            member_bytes = member(f"i{cat}_dat")
            mod = self.targets.get(f"i{cat}_dat")
            final[cat] = mod.build(member_bytes) if mod else member_bytes
        # ★ 151차: the result's options come from these same final rows, labelled with the
        # final InGame_txt (the chain appends rows 198..200 — 공격속도 · 헬모드 · 5초당 HP회복).
        txt = member("InGame_txt")
        mod = self.targets.get("InGame_txt")
        labels = ingame_labels(mod.build(txt) if mod else txt)
        items = item_rows(final)
        rows = []
        for r in resolve(plan(), legacy_index(final)):
            name, stats = items[r["out"]]
            if name != r["result"]:
                raise SystemExit(f"{r['result']}: row {r['out']} is named {name}")
            rows.append(dict(r, opts=result_options(r["out"][0], name, stats, labels)))
        self.rows = rows

    def build(self, src):
        if self.rows is None:
            raise SystemExit("add_tower_recipes: bind(jar) was not called")
        plain = h3crypt.decrypt_bytes(src, self.des)
        recs = records(plain)
        if len(recs) != SHIPPED_ROWS:
            raise SystemExit(f"i15_dat has {len(recs)} rows, expected the shipped {SHIPPED_ROWS}")
        bodies = recs[:FIRST] + [record(r) for r in self.rows]
        if len(bodies) != TOTAL_ROWS:
            raise SystemExit(f"{len(bodies)} rows, expected {TOTAL_ROWS}")
        return h3crypt.encrypt_member(pack(bodies), self.des)

    def verify(self, before, after):
        a = records(h3crypt.decrypt_bytes(after, self.des))
        b = records(h3crypt.decrypt_bytes(before, self.des))
        if len(a) != TOTAL_ROWS:
            raise SystemExit(f"VERIFY: {len(a)} rows, expected {TOTAL_ROWS}")
        if a[:FIRST] != b[:FIRST]:
            raise SystemExit("VERIFY: a row below 0x27 changed")
        want = [record(r) for r in self.rows]
        if a[FIRST:] != want:
            raise SystemExit("VERIFY: the recipe rows are not the plan")
        for r, body in zip(self.rows, a[FIRST:]):
            t = body[-9:]
            if t[2] != SHARD_IDX or t[4] != NONE or t[6] != RATE or (t[7], t[8]) != r["out"]:
                raise SystemExit(f"VERIFY: {r['result']} tail {t.hex()}")
        print(f"VERIFY OK: i15_dat {len(b)} -> {len(a)} rows, 0x{FIRST:02x}..0x{TOTAL_ROWS - 1:02x} "
              f"= {len(want)} 하늘성 recipes ({sum(1 for r in self.rows if r['base'] is None)} without a base)")


def handlers(targets):
    """{member: handler}. ⛔ Register AFTER `add_legacy_items` — `bind` replays the i0..i12
    handlers in `targets` to find where the results land."""
    return {MEMBER: Handler(targets)}


def main():
    rows = plan()
    print(f"{len(rows)} recipes")
    for r in rows:
        print(f"  {r['tier']}제 Lv{r['new_lv']:2d} (rank {r['lv']:2d}) {SLOT[r['cat']]:<4} {r['result']:<10} <- {r['base'] or '(none)':<10} x{r['shards']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
