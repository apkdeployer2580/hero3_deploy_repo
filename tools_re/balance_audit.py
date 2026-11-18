"""156차-B — the new items' balance against the hell and 하늘성 monsters, re-measurable.

Every number in ITEM_BALANCE_AUDIT_156.md comes out of this file. It reads the SHIPPED asset
(species, bosses, item rows, skill trees, classes), the floor table, and the constants the
forks use straight from the Rust sources (guardian.rs · atkspeed.rs), so a later balance
change is re-audited by running it again — nothing here is a copied number.

The combat formulas are the game's own, RE'd on binary.mod (HELL_COMBAT_BALANCE.md §1-§5,
ITEM_BALANCE_AUDIT_156.md §1):

  attack   A   = weapon main x (100 + 힘|정신 + 물공%|특공%) / 100            0x4cdb4 / 0x4ce64
  hit          = max(1, A x skill - mon_def) (+ mon_def x min(방패무시%,100) after the x1.5 crit) dmgwide
  player def   = (Σ helm/armor/gloves/boots/shield main + 10) x (100 + min(물방%|특방%, DEF_PCT_CAP))/100 + 체력
               (160차 T2: the two getters cap the summed % at 500 — `statwide`)
  monster hit  = max(1, mon_atk - player_def), x1.5 on a monster crit                0x38518
  max HP       = (14 L + 18 체력) x (100 + HP%) / 100                                  0x49000
  player hit % = clamp(80 + min(명중, HIT_SUM_CAP) + d24 - mon_dodge + 5(pLv - mLv), 15, 95)   0x4bf60
               (158차 P1: the byte store at 0x4f7ca is clamped at 255 by `statcap`; it WRAPPED before)
  player crit% = clamp(d25 + 기절 - mon_res + (pLv - mLv), 2, 60)                      0x4bed8
  monster hit% = clamp(80 + mon_hit - 회피 + 5(mLv - pLv), 15, 95)                     0x4bfca
               회피 = 블록 for a class whose FIRST tree is 창술, else 민첩/3 + 회피      0x4d2ec
  monster crit = clamp(2(mLv - pLv) - 저항, 2, 60)
  hell fork    = level 38..67 -> 68..100, HP/ATK/DEF grow per level, DEF x HELL_DEF_PCT,
                 hit/dodge/res rewritten against the player's level (guardian.rs)
  tower fork   = floor level + growth + the row's %s; hit/dodge/res the HELL way against the
                 player's level (158차 P8 — the species' own fields until then)
  cooldown     = desc x (100 - min(쿨타임, COOLDOWN_REDUCE_CAP_PCT))/100              0x4bbd6
               (capped at 70 by `statcap`; a sum >= 100 % was a one-tick cooldown)
  기습         = the first hit on a full monster x AMBUSH_MULT (158차 P4: 2, was 3)
  native set   = helm/armor/gloves/boots b11 equal and > 0: HP/물공/특공/물방/특방 +20, SP소모 +20

Usage:
    python tools_re/balance_audit.py [--zip <asset.zip>] [--plv 99]
                                     [monsters|options|items|sets|showcase|all]
"""
import argparse
import copy
import io
import os
import re
import statistics
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from h3crypt import records                     # noqa: E402
import dump_item_db as D                        # noqa: E402
import acc_options as AO                        # noqa: E402

DEFAULT_ZIP = os.path.join(ROOT, "android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
GUARDIAN_RS = os.path.join(ROOT, "wie_android", "src", "guardian.rs")
ATKSPEED_RS = os.path.join(ROOT, "wie_android", "src", "atkspeed.rs")
WIE_UTIL_RS = os.path.join(ROOT, "vendor", "wie", "wie_util", "src", "lib.rs")   # stats::{HIT_SUM_CAP, COOLDOWN_REDUCE_CAP_PCT, AMBUSH_MULT, DEF_PCT_CAP}
SET_ITEMS_RS = os.path.join(ROOT, "wie_android", "src", "set_items.rs")
FLOORS_TXT = os.path.join(ROOT, "wie_guardian_floors.txt")

# option ids (addStat 0x4ee34, jump table 0x5e1cc) + the host's two
HPMAX, HPREGEN, PATK, MATK, PDEF, MDEF, ACC, DODGE, BLOCK, STUN, RES, SPCOST, SPREGEN, DRAIN, CD, PIERCE = (
    2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18)
ATKSPD, REGEN5 = 0x76, 0x78
ONAME = {HPMAX: "HP%", HPREGEN: "HP회복", PATK: "물공%", MATK: "특공%", PDEF: "물방%", MDEF: "특방%",
         ACC: "명중", DODGE: "회피", BLOCK: "블록", STUN: "기절", RES: "저항", SPCOST: "SP소모",
         SPREGEN: "SP회복", DRAIN: "HP흡수", CD: "쿨타임", PIERCE: "방패무시", ATKSPD: "공속",
         REGEN5: "5초HP%", 4: "SP+"}
WEAPON = {4: "창", 5: "검", 6: "단도", 7: "총", 8: "캐논", 9: "암흑성", 10: "광휘석"}
PHYS = {4, 5, 6}
TREE_D = {4: (20, 1), 5: (60, 1), 6: (20, 1), 7: (0, 20), 8: (0, 1), 9: (20, 1), 10: (20, 1)}  # d24/d25
SET_NAMES = ["템플러", "크루세이더", "레갈리스근위", "코르버스", "붉은마녀", "빙해"]
SLOTNAME = {0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", 11: "방패", 12: "장신구"}
SKILL_MUL = 3.45            # a 245 % skill at Lv 5 (섬광 · 선풍 · 연사 · 업화 · 파동 …)
HELD_VIT, HELD_AGI = 60, 40  # the reference stat build: 체력 60 · 민첩 40 · the rest into the weapon's stat


# ── sources ──────────────────────────────────────────────────────────────────────────

def rust_const(path, name):
    m = re.search(r"pub const %s\s*:\s*[^=]+=\s*([^;]+);" % re.escape(name), open(path, encoding="utf-8").read())
    if not m:
        raise SystemExit("%s: no `pub const %s`" % (path, name))
    return m.group(1).strip()


def num(s):
    return float(s) if "." in s else int(s, 0)


class Src:
    def __init__(self, zip_path):
        z = zipfile.ZipFile(zip_path)
        jars = [n for n in z.namelist() if n.endswith(".jar")]
        self.jar = zipfile.ZipFile(io.BytesIO(z.read(jars[0])))
        self.dats = D.load_dats(zip_path, None)
        g = {n: num(rust_const(GUARDIAN_RS, n)) for n in (
            "HP_GROWTH", "ATK_GROWTH", "DEF_GROWTH", "HELL_DEF_PCT", "HELL_HIT_BASE", "HELL_HIT_PER_LV",
            "HELL_DODGE_BASE", "HELL_DODGE_PER_LV", "HELL_RES_BASE", "HELL_RES_PER_LV", "HELL_HIT_CLAMP",
            "HELL_DODGE_CLAMP", "HELL_SPAWN_LV_MIN", "HELL_SPAWN_LV_MAX", "HELL_LV_MIN", "HELL_LV_MAX",
            "I16_MAX")}
        self.k = g
        # 158차: the three caps/multipliers the hooks apply, from the shared crate.
        self.hit_cap = int(rust_const(WIE_UTIL_RS, "HIT_SUM_CAP"), 0)
        self.cd_cap = int(rust_const(WIE_UTIL_RS, "COOLDOWN_REDUCE_CAP_PCT"), 0)
        self.ambush = int(rust_const(WIE_UTIL_RS, "AMBUSH_MULT"), 0)
        self.def_cap = int(rust_const(WIE_UTIL_RS, "DEF_PCT_CAP"), 0)      # 160차 T2
        frames = rust_const(ATKSPEED_RS, "ATTACK_FRAMES_MEASURED")
        self.frames = [(int(a), int(b) / 100) for a, b in re.findall(r"\((\d+),\s*(\d+)\)", frames)]
        self.max_speed = int(rust_const(ATKSPEED_RS, "MAX_PCT"))
        self.set_items = [(int(c, 16), int(i, 16), int(s)) for c, i, s in
                          re.findall(r"\((0x[0-9a-f]+),\s*(0x[0-9a-f]+),\s*(\d)\)", open(SET_ITEMS_RS, encoding="utf-8").read())]


def u16(b, o):
    return int.from_bytes(b[o:o + 2], "little")


def s8(v):
    return v - 256 if v >= 128 else v


def species(src):
    """id -> stats. 0..160 = enemyh_dat (21-byte tail, species loader 0x377xx),
    161.. = bossh_dat (25-byte tail, boss loader 0x25bxx)."""
    out = {}
    for i, r in enumerate(records(src.jar.read("res/dat/enemyh_dat"))):
        nl = r[0]
        t = r[1 + nl:]
        name = r[1:1 + nl].decode("euc-kr", "replace").rstrip("@")
        out[i] = dict(id=i, name=name, lv=t[0], hp=u16(t, 5), atk=u16(t, 7), dp=u16(t, 9), ds=u16(t, 11),
                      hit=s8(t[13]), dodge=s8(t[14]), res=t[16], attackless=u16(t, 17) == 0,
                      named=name.startswith("{"), boss=False)
    for i, r in enumerate(records(src.jar.read("res/boss/bossh_dat"))):
        nl = r[0]
        t = r[1 + nl:]
        # a boss carries THREE attack values (record +0x8e / +0x90 / +0x92, picked by its state
        # byte): the plain hit and two skill attacks. 160차 T4 grows all three.
        out[161 + i] = dict(id=161 + i, name=r[1:1 + nl].decode("euc-kr", "replace"), lv=t[0], hp=u16(t, 2),
                            atk=u16(t, 4), skill=(u16(t, 6), u16(t, 8)), dp=u16(t, 10), ds=u16(t, 12), hit=s8(t[14]),
                            dodge=s8(t[15]), res=s8(t[19]), attackless=False, named=False, boss=True)
    return out


def classes(src):
    out = []
    for r in records(src.jar.read("res/dat/char_dat")):
        nl = r[0]
        if nl == 0:
            continue
        cl = r[1 + nl]
        cname = r[2 + nl:2 + nl + cl].decode("euc-kr", "replace")
        t = r[2 + nl + cl:]
        if not cname:
            continue
        out.append((cname, (t[1], t[2]), (t[3], t[4], t[5], t[6])))
    return out


def tri(n):
    return n * (n + 1) // 2


def skill_passives(src):
    """tree -> {stat id: value at skill level 5} over the tree's kind-3 rows."""
    out = {}
    for tree in range(4, 11):
        acc = {}
        for i, r in enumerate(records(src.jar.read("res/skill/s%d_dat" % tree))):
            if i < 7:
                continue
            nl = r[0]
            t = r[1 + nl:]
            kind, dl = t[0], t[1]
            d = t[2 + dl:]
            if kind != 3:
                continue
            for k in range(3):
                o = 14 + 5 * k
                sid = d[o]
                if sid == 0x7f:
                    continue
                base = int.from_bytes(d[o + 1:o + 3], "little", signed=True)
                gr = int.from_bytes(d[o + 3:o + 5], "little", signed=True)
                acc[sid] = acc.get(sid, 0) + base + gr * tri(5)
        out[tree] = acc
    return out


def item_rows(src):
    rows = []
    for cat in range(13):
        decoded, _ = D.decode(cat, src.dats[cat])
        for r in decoded:
            st = bytes.fromhex(r["stats"])
            d = dict(cat=cat, idx=r["idx"], name=r["name"], mark=r["marker"])
            if cat < 12:
                d.update(grade=st[7], lv=st[8], setid=st[11], main=u16(st, 12),
                         opts=[(st[16], s8(st[17])), (st[18], s8(st[19]))])
            else:
                d.update(grade=st[5], lv=st[6], setid=0, main=u16(st, 10),
                         opts=[(st[14], s8(st[15])), (st[16], s8(st[17]))])
            d["opts"] = [o for o in d["opts"] if o[0] != 0]
            rows.append(d)
    third = {}
    for name, _mark, _o1, _o2, (kind, sid, val), *_ in AO.ACC:
        third[name] = (kind, sid, val)
    for d in rows:
        d["third"] = third.get(d["name"])
    return rows


def floors():
    out = {}
    for line in open(FLOORS_TXT, encoding="utf-8"):
        if line.startswith("#") or not line.strip():
            continue
        p = line.split()
        out[int(p[0])] = dict(floor=int(p[0]), lv=int(p[2]), hp=int(p[3]), dfn=int(p[4]), boss=p[5] == "B",
                              atk=int(p[6]), species=[int(x) for x in p[7:10] if x != "-"])
    return out


# ── monsters ─────────────────────────────────────────────────────────────────────────

class Monsters:
    def __init__(self, src):
        self.k = src.k
        self.sp = species(src)
        self.floors = floors()

    def grow(self, v, g, d, pct=100):
        x = max(1, round(v * g ** d))
        return x * max(pct, 1) // 100

    def clamp_stat(self, v):
        return max(1, min(self.k["I16_MAX"], v))

    def hell_level(self, lv0):
        k = self.k
        lo, hi = k["HELL_SPAWN_LV_MIN"], k["HELL_SPAWN_LV_MAX"]
        d = min(max(lv0, lo), hi) - lo
        span_in, span_out = hi - lo, k["HELL_LV_MAX"] - k["HELL_LV_MIN"]
        return k["HELL_LV_MIN"] + (d * span_out + span_in // 2) // span_in

    def at(self, s, lv, hp=100, dfn=100, atk=100):
        d = lv - s["lv"]
        k = self.k
        return dict(lv=lv, hp=self.grow(s["hp"], k["HP_GROWTH"], d, hp),
                    atk=self.clamp_stat(self.grow(s["atk"], k["ATK_GROWTH"], d, atk)),
                    skill=tuple(0 if a == 0 else self.clamp_stat(self.grow(a, k["ATK_GROWTH"], d, atk)) for a in s.get("skill", ())),
                    dp=self.clamp_stat(self.grow(s["dp"], k["DEF_GROWTH"], d, dfn)),
                    ds=self.clamp_stat(self.grow(s["ds"], k["DEF_GROWTH"], d, dfn)))

    def rolls(self, lv, plv):
        """The fork's three roll fields for a monster at `lv` against a player at `plv`
        (guardian.rs hell_hit / hell_dodge / hell_res — hell since 156차, the tower since 158차 P8)."""
        k = self.k
        d = lv - plv
        return dict(hit=max(-k["HELL_HIT_CLAMP"], min(k["HELL_HIT_CLAMP"], k["HELL_HIT_BASE"] - k["HELL_HIT_PER_LV"] * d)),
                    dodge=max(-k["HELL_DODGE_CLAMP"], min(k["HELL_DODGE_CLAMP"], k["HELL_DODGE_BASE"] - k["HELL_DODGE_PER_LV"] * d)),
                    res=max(-k["HELL_HIT_CLAMP"], min(k["HELL_HIT_CLAMP"], k["HELL_RES_BASE"] + k["HELL_RES_PER_LV"] * (plv - lv))))

    def hell(self, s, plv):
        k = self.k
        lv = self.hell_level(s["lv"])
        m = self.at(s, lv, 100, k["HELL_DEF_PCT"], 100)
        m.update(name=s["name"], content="hell", **self.rolls(lv, plv))
        return m

    def tower(self, s, floor, plv, escort=False):
        row = self.floors[floor - 1] if escort and floor - 1 in self.floors else self.floors[floor]
        m = self.at(s, row["lv"], row["hp"], row["dfn"], row["atk"])
        # 158차 P8: the tower's rolls are forked like hell's (the species' own fields before).
        m.update(name=s["name"], content="tower", **self.rolls(row["lv"], plv))
        return m

    def reference(self, plv):
        sp = self.sp
        normals = sorted([s for s in sp.values() if not s["boss"] and not s["attackless"] and not s["named"]
                          and self.hell_level(s["lv"]) >= 98], key=lambda s: self.hell(s, plv)["hp"])
        pick = normals[len(normals) // 2]
        out = []
        m = self.hell(pick, plv); m["label"] = "헬 Lv%d 일반(%s)" % (m["lv"], pick["name"]); out.append(m)
        m = self.hell(sp[52], plv); m["label"] = "헬 Lv%d 네임드(%s)" % (m["lv"], sp[52]["name"].lstrip("{")); out.append(m)
        for floor, sid, what in ((71, 118, "일반"), (85, 132, "일반"), (99, 138, "네임드"), (100, 175, "보스")):
            m = self.tower(sp[sid], floor, plv)
            m["label"] = "탑 %d층 %s(%s)" % (floor, what, sp[sid]["name"].lstrip("{"))
            out.append(m)
        return out


# ── the player ───────────────────────────────────────────────────────────────────────

class Build:
    def __init__(self, cname, trees, base, wcat, passives, plv):
        self.cname, self.trees, self.wcat, self.plv = cname, trees, wcat, plv
        self.phys = wcat in PHYS
        self.main_idx = 0 if self.phys else 2
        self.d24, self.d25 = TREE_D[wcat]
        self.spear_class = trees[0] == 4          # 0x4d2ec: evasion = 블록
        self.shield = 4 in trees                   # the shield slot opens with a spear
        self.passive = {}
        for t in trees:
            for k, v in passives[t].items():
                self.passive[k] = self.passive.get(k, 0) + v
        st, pts = list(base), 4 * (plv - 1)        # stat points; above 99 a point costs 2 (0x4f810)

        def raise_to(i, target):
            nonlocal pts
            while st[i] < target:
                cost = 1 if st[i] < 99 else 2
                if pts < cost:
                    return
                pts -= cost
                st[i] += 1
        raise_to(1, HELD_VIT)
        raise_to(3, HELD_AGI)
        raise_to(self.main_idx, 10 ** 6)
        self.st = st

    def label(self):
        return "%s/%s" % (self.cname, WEAPON[self.wcat])


class Model:
    def __init__(self, src, plv):
        self.src, self.plv = src, plv
        self.rows = item_rows(src)
        self.by_name = {r["name"]: r for r in self.rows}
        self.leg = [r for r in self.rows if r["mark"] == '"']
        self.neutral = {}
        for cat in range(12):
            r90 = sorted([r for r in self.leg if r["cat"] == cat and r["lv"] == 90], key=lambda r: r["main"])
            self.neutral[cat] = r90[0]["main"] if r90 else 0
        pas = skill_passives(src)
        self.builds = [Build(c, t, b, w, pas, plv) for c, t, b in classes(src) for w in t]
        self.mon = Monsters(src)
        self.ref = self.mon.reference(plv)
        self.set_of = {(c, i): s for c, i, s in src.set_items}

    # gear: dict slot -> {main, opts, third, setid}; slots w, 0..3 armor, 11 shield, a1/a2
    def baseline(self, b):
        g = {"w": dict(main=self.neutral[b.wcat], opts=[])}
        for c in (0, 1, 2, 3):
            g[c] = dict(main=self.neutral[c], opts=[], setid=0)
        g[11] = dict(main=self.neutral[11] if b.shield else 0, opts=[])
        g["a1"], g["a2"] = dict(opts=[]), dict(opts=[])
        return g

    def put(self, b, g, r, acc_slot="a1", keep_set=False):
        g = copy.deepcopy(g)
        cat = r["cat"]
        if 4 <= cat <= 10:
            if cat != b.wcat:
                return None
            g["w"] = dict(main=r["main"], opts=list(r["opts"]))
        elif cat == 11:
            if not b.shield:
                return None
            g[11] = dict(main=r["main"], opts=list(r["opts"]))
        elif cat == 12:
            g[acc_slot] = dict(opts=list(r["opts"]), third=r["third"])
        else:
            g[cat] = dict(main=r["main"], opts=list(r["opts"]), setid=r["setid"] if keep_set else 0)
        return g

    def effective(self, b, g, setfx=(), extra=None):
        s = dict(b.passive)
        host = dict(taken=0, dealt=0)

        def add(k, v):
            s[k] = s.get(k, 0) + v
        for it in g.values():
            for k, v in it.get("opts", []):
                add(k, v)
            t = it.get("third")
            if t:
                kind, sid, val = t
                if kind == "Stat":
                    add(sid, val)
                elif kind == "AtkSpeed":
                    add(ATKSPD, val)
                elif kind == "Taken":
                    host["taken"] += val
                elif kind == "Dealt":
                    host["dealt"] += val
        sids = [g[c].get("setid", 0) for c in (0, 1, 2, 3)]
        native = len(set(sids)) == 1 and sids[0] > 0
        if native:
            for k in (HPMAX, PATK, MATK, PDEF, MDEF, SPCOST):
                add(k, 20)
        for k, v in (extra or {}).items():
            add(k, v)
        st = b.st
        A = g["w"]["main"] * (100 + st[b.main_idx] + s.get(PATK if b.phys else MATK, 0)) / 100
        armor = sum(g[c]["main"] for c in (0, 1, 2, 3, 11))
        return dict(A=A, native=native, setfx=set(setfx), raw=s, taken=host["taken"], dealt=host["dealt"],
                    # 160차 T2: the getters cap the summed defence % at DEF_PCT_CAP
                    defp=(armor + 10) * (100 + min(s.get(PDEF, 0), self.src.def_cap)) / 100 + st[1],
                    defs=(armor + 10) * (100 + min(s.get(MDEF, 0), self.src.def_cap)) / 100 + st[1],
                    hp=(14 * b.plv + 18 * st[1]) * (100 + s.get(HPMAX, 0)) / 100,
                    # 158차 P1: the byte at stat+0x2c is clamped at HIT_SUM_CAP (it wrapped mod 256 before)
                    acc_raw=s.get(ACC, 0), acc=min(s.get(ACC, 0), self.src.hit_cap),
                    ev=s.get(BLOCK, 0) if b.spear_class else st[3] // 3 + s.get(DODGE, 0),
                    stun=s.get(STUN, 0), res=s.get(RES, 0), drain=s.get(DRAIN, 0),
                    # items::shield_pierce_pct clamps 0..=100 and stats::pierce_bonus caps at the whole defence
                    pierce=max(0, min(100, s.get(PIERCE, 0))), pierce_raw=s.get(PIERCE, 0),
                    # 158차 P2: the cast path reads the sum through COOLDOWN_REDUCE_CAP_PCT
                    cd_raw=s.get(CD, 0), cd=min(s.get(CD, 0), self.src.cd_cap),
                    spcost=s.get(SPCOST, 0), speed=min(s.get(ATKSPD, 0), self.src.max_speed))

    def speed(self, pct):
        t = self.src.frames
        pct = max(0, min(pct, self.src.max_speed))
        for (p0, f0), (p1, f1) in zip(t, t[1:]):
            if pct <= p1:
                return t[0][1] / (f0 - (f0 - f1) * (pct - p0) / (p1 - p0))
        return t[0][1] / t[-1][1]

    @staticmethod
    def clamp(v, lo, hi):
        return max(lo, min(hi, v))

    def offense(self, b, e, m):
        d = m["lv"] - b.plv
        hell = m["content"] == "hell"
        if hell:
            # the fork's fields cancel the level terms (guardian.rs hell_rates)
            p_hit = self.clamp(80 + e["acc"] + b.d24 - m["dodge"] - 5 * d, 15, 95)
            p_crit = self.clamp(b.d25 + e["stun"] - m["res"] - d, 2, 60)
        else:
            p_hit = self.clamp(80 + e["acc"] + b.d24 - m["dodge"] - 5 * d, 15, 95)
            p_crit = self.clamp(b.d25 + e["stun"] - m["res"] - d, 2, 60)
        mdef = m["dp"] if b.phys else m["ds"]
        out = dict(p_hit=p_hit, p_crit=p_crit)
        for key, k in (("basic", 1.0), ("skill", SKILL_MUL)):
            raw = e["A"] * k - mdef
            if raw <= 1:
                n = c = 1.0
            else:
                n = raw + mdef * e["pierce"] / 100
                c = 1.5 * raw + mdef * e["pierce"] / 100
            landed = (1 - p_crit / 100) * n + p_crit / 100 * c
            exp = p_hit / 100 * landed * (1 + e["dealt"] / 100)
            if "빙해" in e["setfx"]:
                exp *= 1.5
            if "코르버스" in e["setfx"]:
                exp *= 4 / 3                     # the second half of every kill at x2
            if "크루세이더" in e["setfx"]:
                N = m["hp"] / max(exp, 1)        # attacks per kill; the first landed hit x AMBUSH saves (AMBUSH-1) hits
                saved = (self.src.ambush - 1) * landed / max(exp, 1)
                exp *= N / max(N - saved, 1)
            if key == "basic":
                exp *= self.speed(e["speed"])
            out[key] = exp
        return out

    def defense(self, b, e, m):
        d = m["lv"] - b.plv
        m_hit = self.clamp(80 + m["hit"] - e["ev"] + 5 * d, 15, 95)
        m_crit = self.clamp(2 * d - e["res"], 2, 60)
        taken = max(1.0, m["atk"] - (e["defp"] + e["defs"]) / 2) * (1 - e["taken"] / 100)
        if "레갈리스근위" in e["setfx"]:
            taken *= 0.8
        exp = m_hit / 100 * taken * (1 + m_crit / 100 * 0.5)
        return dict(m_hit=m_hit, m_crit=m_crit, onehit=taken, hp=e["hp"], ehp=e["hp"] / max(exp, 1e-9),
                    hits_to_die=e["hp"] / taken)

    def run(self, b, g, setfx=(), extra=None):
        e = self.effective(b, g, setfx, extra)
        return e, [(self.offense(b, e, m), self.defense(b, e, m)) for m in self.ref]


# ── reports ──────────────────────────────────────────────────────────────────────────

def med(xs):
    return statistics.median(xs) if xs else float("nan")


def span(xs):
    xs = sorted(xs)
    return "%+.0f%% (%+.0f..%+.0f)" % (med(xs), xs[0], xs[-1]) if xs else "-"


def pct(a, b):
    return (a / b - 1) * 100 if b else 0.0


def report_monsters(M):
    mon = M.mon
    print("## 몬스터 기준표 (플레이어 Lv %d)" % M.plv)
    print("헬 일반 종(무해 · 네임드 제외), 헬 레벨 구간별 — HP / 공격 / 물리방어:")
    for lo, hi in ((68, 78), (79, 85), (86, 92), (93, 100)):
        ms = [mon.hell(s, M.plv) for s in mon.sp.values() if not s["boss"] and not s["attackless"]
              and not s["named"] and lo <= mon.hell_level(s["lv"]) <= hi]
        if ms:
            f = lambda key: "%d..%d (중앙 %d)" % (min(m[key] for m in ms), max(m[key] for m in ms), med([m[key] for m in ms]))
            print("  Lv %3d~%3d %2d종: HP %s · 공격 %s · 방어 %s" % (lo, hi, len(ms), f("hp"), f("atk"), f("dp")))
    print("헬 네임드(`{`):")
    for s in sorted([s for s in mon.sp.values() if s["named"]], key=lambda s: (s["lv"], s["id"])):
        m = mon.hell(s, M.plv)
        print("  %-10s Lv %2d→%3d  HP %6d · 공격 %5d · 방어 %4d/%4d" % (s["name"].lstrip("{"), s["lv"], m["lv"], m["hp"], m["atk"], m["dp"], m["ds"]))
    print("하늘성 주요 층(명중 · 회피 · 저항 = 158차 P8 포크값 — 층 레벨 vs 플레이어 Lv %d):" % M.plv)
    for fl in (1, 30, 50, 70, 71, 80, 85, 90, 95, 99, 100):
        f = mon.floors[fl]
        parts = []
        for k, sid in enumerate(f["species"]):
            m = mon.tower(mon.sp[sid], fl, M.plv, escort=f["boss"] and k > 0)
            parts.append("%s Lv%d HP %d 공 %d 방 %d 명중 %d 회피 %d 저항 %d" % (
                mon.sp[sid]["name"].lstrip("{"), m["lv"], m["hp"], m["atk"], m["dp"], m["hit"], m["dodge"], m["res"]))
        print("  %3d층 %s: %s" % (fl, "보스" if f["boss"] else "    ", " | ".join(parts)))
    print("\n기준 몬스터:")
    for m in M.ref:
        print("  %-26s Lv %3d HP %6d 공격 %5d 방어 %4d/%4d 명중 %4d 회피 %4d 저항 %4d" % (
            m["label"], m["lv"], m["hp"], m["atk"], m["dp"], m["ds"], m["hit"], m["dodge"], m["res"]))
    print("\n기준 빌드(중립 90제 장비 · 옵션 0 · 패시브 Lv5) — 적중/크리/피격/죽기까지 맞는 수, 빌드 중앙(최소..최대):")
    for i, m in enumerate(M.ref):
        rows = [M.run(b, M.baseline(b))[1][i] for b in M.builds]
        f = lambda xs, p="%.0f": (p + " (" + p + ".." + p + ")") % (med(xs), min(xs), max(xs))
        print("  %-26s 적중 %s · 크리 %s · 피격 %s · 맞는 수 %s" % (
            m["label"], f([r[0]["p_hit"] for r in rows]), f([r[0]["p_crit"] for r in rows]),
            f([r[1]["m_hit"] for r in rows]), f([r[1]["hits_to_die"] for r in rows], "%.2f")))


def report_options(M):
    print("\n## 옵션 가치표 — 중립 장비에 옵션 하나를 +N 했을 때, 20 빌드 중앙(최소..최대)")
    cols = [(0, "헬 일반"), (1, "헬 네임드"), (3, "탑 85층"), (4, "탑 99층 네임드"), (5, "탑 100층 보스")]
    for amount in (30, 60, 90):
        print("\n+%d | %s" % (amount, " | ".join(n for _, n in cols)))
        for k in (ACC, STUN, PATK, MATK, PIERCE, ATKSPD):
            acc = {i: [] for i, _ in cols}
            for b in M.builds:
                if (k == PATK and not b.phys) or (k == MATK and b.phys):
                    continue
                _, base = M.run(b, M.baseline(b))
                _, new = M.run(b, M.baseline(b), extra={k: amount})
                for i, _ in cols:
                    acc[i].append(pct(new[i][0]["basic"], base[i][0]["basic"]))
            print("  %-6s 화력 | %s" % (ONAME[k], " | ".join(span(acc[i]) for i, _ in cols)))
        for k in (HPMAX, PDEF, MDEF, DODGE, BLOCK, RES):
            acc = {i: [] for i, _ in cols}
            for b in M.builds:
                if (k == BLOCK and not b.spear_class) or (k == DODGE and b.spear_class):
                    continue
                _, base = M.run(b, M.baseline(b))
                _, new = M.run(b, M.baseline(b), extra={k: amount})
                for i, _ in cols:
                    acc[i].append(pct(new[i][1]["ehp"], base[i][1]["ehp"]))
            print("  %-6s 생존 | %s" % (ONAME[k], " | ".join(span(acc[i]) for i, _ in cols)))


def opt_text(r):
    s = ["%s%+d" % (ONAME.get(k, hex(k)), v) for k, v in r["opts"]]
    if r["third"]:
        kind, sid, val = r["third"]
        s.append("[3]%s%+d" % (ONAME.get(sid, kind) if kind == "Stat" else
                               {"AtkSpeed": "공속", "Taken": "피해감소%", "Dealt": "주는피해%",
                                "KillMaxHp": "처치최대HP%", "HitMaxHp": "피격최대HP%"}[kind], val))
    return " ".join(s)


ACC_85 = ("율리케의목걸이", "레갈리스교단반지", "엘지스의증표", "이안의맹세", "레아의목걸이", "일레느의각인")


def tier_of(M, r):
    """85 · 90 · 전설 · 리버80 · 보스 — off the row itself (the legacy rows' 등급 is 5 for all)."""
    if r["name"].startswith("리버"):
        return "리버80"
    if r["mark"] == "{":
        return "보스"
    if r["cat"] == 12:
        return "85" if r["name"] in ACC_85 else "90"
    if r["lv"] == 85:
        return "85"
    top = max(x["main"] for x in M.leg if x["cat"] == r["cat"])
    return "전설" if r["main"] == top else "90"


def item_deltas(M, r, acc_slot="a1"):
    per = []
    for b in M.builds:
        g0 = M.baseline(b)
        g1 = M.put(b, g0, r, acc_slot)
        if g1 is None:
            continue
        _, base = M.run(b, g0)
        e1, new = M.run(b, g1)
        per.append((b, e1, [(pct(n[0]["basic"], o[0]["basic"]), pct(n[1]["ehp"], o[1]["ehp"])) for o, n in zip(base, new)]))
    return per


def report_items(M):
    print("\n## 아이템별 — 중립 90제(옵션 0) 같은 칸 대비, 쓸 수 있는 빌드의 중앙(최소..최대)")
    rows = M.leg + [r for r in M.rows if r["mark"] == "{" and (r["name"].startswith("리버") or r["name"] in ("타이탄의 파편", "검은 눈물"))]
    order = [0, 1, 2, 3, 11, 12, 4, 5, 6, 7, 8, 9, 10]
    for cat in order:
        print("\n### %s" % (SLOTNAME.get(cat) or WEAPON[cat]))
        print("| 아이템 | 제 | 주스탯 | 옵션 | 헬 일반 화력 | 헬 일반 생존 | 헬 네임드 화력 | 탑 85층 화력 | 탑 99층 네임드 화력 | 탑 99층 네임드 생존 |")
        print("|---|---|---|---|---|---|---|---|---|---|")
        for r in rows:
            if r["cat"] != cat:
                continue
            per = item_deltas(M, r)
            if not per:
                continue
            col = lambda i, w: span([p[2][i][w] for p in per])
            tier = tier_of(M, r)
            sname = SET_NAMES[M.set_of[(cat, r["idx"])]] if (cat, r["idx"]) in M.set_of else ""
            print("| %s%s | %s | %d | %s | %s | %s | %s | %s | %s | %s |" % (
                r["name"], " (%s)" % sname if sname else "", tier, r["main"], opt_text(r),
                col(0, 0), col(0, 1), col(1, 0), col(3, 0), col(4, 0), col(4, 1)))


def set_gear(M, b, sid):
    g = M.baseline(b)
    for r in M.leg:
        if (r["cat"], r["idx"]) in M.set_of and M.set_of[(r["cat"], r["idx"])] == sid:
            if r["cat"] in (0, 1, 2, 3):
                g = M.put(b, g, r, keep_set=True)
            elif r["cat"] == 11 and b.shield:
                g = M.put(b, g, r)
            elif r["cat"] == 12:
                g = M.put(b, g, r, "a1")
    return g


def report_sets(M):
    print("\n## 세트 6종 — 방어구 4부위(원작 4부위 +20% 포함) + 방패(창 직업) + 세트 장신구 + 세트 효과, 중립 대비")
    print("| 세트 | 헬 일반 화력 | 헬 일반 생존 | 헬 네임드 화력 | 헬 네임드 생존 | 탑 99층 네임드 화력 | 탑 99층 네임드 생존 | 효과의 몫(헬 일반) | 원작 +20%의 몫(헬 일반 생존) |")
    print("|---|---|---|---|---|---|---|---|---|")
    for sid, sname in enumerate(SET_NAMES):
        acc = {k: [] for k in ("h0d", "h0e", "h1d", "h1e", "t4d", "t4e", "fxd", "fxe", "nat")}
        for b in M.builds:
            _, base = M.run(b, M.baseline(b))
            g = set_gear(M, b, sid)
            _, full = M.run(b, g, (sname,))
            _, nofx = M.run(b, g, ())
            g2 = copy.deepcopy(g)
            for c in (0, 1, 2, 3):
                g2[c]["setid"] = 0
            _, nonat = M.run(b, g2, ())
            acc["h0d"].append(pct(full[0][0]["basic"], base[0][0]["basic"]))
            acc["h0e"].append(pct(full[0][1]["ehp"], base[0][1]["ehp"]))
            acc["h1d"].append(pct(full[1][0]["basic"], base[1][0]["basic"]))
            acc["h1e"].append(pct(full[1][1]["ehp"], base[1][1]["ehp"]))
            acc["t4d"].append(pct(full[4][0]["basic"], base[4][0]["basic"]))
            acc["t4e"].append(pct(full[4][1]["ehp"], base[4][1]["ehp"]))
            acc["fxd"].append(pct(full[0][0]["basic"], nofx[0][0]["basic"]))
            acc["fxe"].append(pct(full[0][1]["ehp"], nofx[0][1]["ehp"]))
            acc["nat"].append(pct(nofx[0][1]["ehp"], nonat[0][1]["ehp"]))
        f = lambda k: "%+.0f%%" % med(acc[k])
        print("| %s | %s | %s | %s | %s | %s | %s | 화력 %s · 생존 %s | %s |" % (
            sname, f("h0d"), f("h0e"), f("h1d"), f("h1e"), f("t4d"), f("t4e"), f("fxd"), f("fxe"), f("nat")))
    print("\n세트 방어구 4부위의 옵션 합:")
    for sid, sname in enumerate(SET_NAMES):
        tot = {}
        for r in M.leg:
            if (r["cat"], r["idx"]) in M.set_of and M.set_of[(r["cat"], r["idx"])] == sid and r["cat"] in (0, 1, 2, 3):
                for k, v in r["opts"]:
                    tot[k] = tot.get(k, 0) + v
        print("  %-8s %s" % (sname, " ".join("%s%+d" % (ONAME.get(k, hex(k)), v) for k, v in sorted(tot.items()))))


SHOWCASE = [
    ("데스나이트", 5, ["빙해의투구", "빙해의갑옷", "빙해의장갑", "빙해의장화", "빙해의대검", "클레르의가보", "타이탄의 파편"], ("빙해",), "공격 최대(빙해 + 전설 검 + 장신구 둘)"),
    ("크레이지암즈", 7, ["코르버스후드", "코르버스암살복", "코르버스크로우", "코르버스스텝", "검은공장의걸작"], ("코르버스",), "명중 쌓기 — u8 접힘"),
    ("디스럽터", 7, ["예언자의눈", "코르버스암살복", "오딘의손길", "섬광의군화", "검은공장의걸작", "검은 눈물", "일레느의각인"], (), "쿨타임 · SP소모 무한 스킬"),
    ("가디언나이트", 4, ["붉은마녀헬름", "붉은마녀로브", "붉은마녀장갑", "붉은마녀신발", "리파이너의가호", "레아의목걸이", "엘지스의증표"], ("붉은마녀",), "생존 최대(HP + 블록)"),
    ("버서커", 6, ["크루세이더후드", "크루세이더코트", "크루세이더핸드", "크루세이더워커", "더글라스의밀검", "엘지스의증표", "타이탄의 파편"], ("크루세이더",), "85제 + 보스 장신구만"),
]


def report_showcase(M):
    print("\n## 조합의 천장")
    for cname, wcat, names, fx, note in SHOWCASE:
        b = next(x for x in M.builds if x.cname == cname and x.wcat == wcat)
        g = M.baseline(b)
        slots = iter(["a1", "a2"])
        for n in names:
            r = M.by_name[n]
            g = M.put(b, g, r, next(slots) if r["cat"] == 12 else "a1", keep_set=True)
        _, base = M.run(b, M.baseline(b))
        e, new = M.run(b, g, fx)
        print("\n### %s — %s" % (b.label(), note))
        print("%s" % " + ".join(names))
        print("원작 4부위 %s · 명중 합 %d → 굴림이 읽는 값 %d · 쿨타임 %d%%(유효 %d%%) · SP소모 %d · 공속 %+d(×%.2f) · 방패무시 %d%%(유효 %d%%) · 흡수 %d · 최대HP %.0f · 회피/블록 %d" % (
            "켜짐" if e["native"] else "꺼짐", e["acc_raw"], e["acc"], e["cd_raw"], e["cd"], e["spcost"], e["speed"], M.speed(e["speed"]),
            e["pierce_raw"], e["pierce"], e["drain"], e["hp"], e["ev"]))
        print("| 몬스터 | 적중 | 크리 | 공격당 피해 | 피격 확률 | 한 방 | 생존 |")
        print("|---|---|---|---|---|---|---|")
        for m, (o0, d0), (o1, d1) in zip(M.ref, base, new):
            print("| %s | %d→%d%% | %d→%d%% | %.0f→%.0f (%+.0f%%) | %d→%d%% | %.0f→%.0f / HP %.0f→%.0f | %+.0f%% |" % (
                m["label"], o0["p_hit"], o1["p_hit"], o0["p_crit"], o1["p_crit"], o0["basic"], o1["basic"],
                pct(o1["basic"], o0["basic"]), d0["m_hit"], d1["m_hit"], d0["onehit"], d1["onehit"], d0["hp"], d1["hp"],
                pct(d1["ehp"], d0["ehp"])))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--plv", type=int, default=99)
    ap.add_argument("what", nargs="?", default="all", choices=["monsters", "options", "items", "sets", "showcase", "all"])
    a = ap.parse_args(argv)
    M = Model(Src(a.zip), a.plv)
    if a.what in ("monsters", "all"):
        report_monsters(M)
    if a.what in ("options", "all"):
        report_options(M)
    if a.what in ("items", "all"):
        report_items(M)
    if a.what in ("sets", "all"):
        report_sets(M)
    if a.what in ("showcase", "all"):
        report_showcase(M)


if __name__ == "__main__":
    main()
