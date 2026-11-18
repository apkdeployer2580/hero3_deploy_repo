"""하늘성 100층 — can a fully built Lv 99 character clear it?  (refine +10, five sockets filled)

`balance_audit.py` measures the new items with NO orbs and NO refine.  This file adds the two
things it leaves out and asks the end-game question on one concrete character.  Everything is
read from the same sources (the shipped asset, guardian.rs, the floor table), so a balance
change is re-audited by running it again.

What it adds over balance_audit — all read off binary.mod `6a352944…`:

  item power   eff = (main + refine x (2 weapon | 1 other)) x (100 + bonus) / 100        0x528f0
  orb bonus    bonus = power x (f*f + f) / 2, f = filled sockets (<= 5)                  0x52950
               the item's OWN main stat takes `bonus` % whatever the orb is, and the wearer
               ALSO gets addStat(statId, bonus) when statId is 2..0x12 — over all NINE worn
               slots, weapons and accessories included                           0x4f15a..0x4f1a4
  attack       A = eff(weapon) x (100 + 힘|정신 + 물공%|특공%) / 100                     0x4cdb4
  defence      D = (sum armor eff + 10) x (100 + min(물방%|특방%, DEF_PCT_CAP)) / 100 + 체력
                                                                             0x4cf64 / 0x4cfa4
               The game truncated both to i16 (past 32767 the halfword went NEGATIVE — a build
               one option too many was hit for 32767, and an attack past it dealt 1). 160차:
               the `statwide` hooks cap the defence % at 500 (T2) and keep the true value past
               the i16 for combat (T1); `--before` shows the game as it was.
  monster hit  max(1, atk +- rand(0,(mLv+1)/2) - D), x1.5 on a crit, cap 32767            0x38518
               D = 물리방어 when the scene's FIRST species has enemyg_dat[5] & 1, else 특수방어
               An accessory's own power is 저항 (0 on the new ones — refine/orb is all of it).
               (the species row's byte 9 is 0 for every species, so the lookup always lands on
               the first slot's row; a boss id is skipped by the loader and its row stays
               calloc'd zero).  A BOSS's own attack passes no table at all             0x26d7c
               ⇒ every hit on a boss floor reads 특수방어.
  boss skills  a boss record has three attack values (+0x8e / +0x90 / +0x92, picked by its
               state byte 2 / 0xb / 0xc·0xd); 160차 T4 grows all three (only the first before).

Usage:
    python tools_re/tower_clear_audit.py [--zip <asset.zip>] [--vit 110] [--int 200] [--floor 100]
"""
import argparse
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import balance_audit as B                       # noqa: E402
import dump_item_db as D                        # noqa: E402
from h3crypt import records                     # noqa: E402

REFINE_MAX = 10          # rec+0x3c5, the forge's cap
SOCKETS_MAX = 5          # orbcombine::MAX_SOCKETS
I16_MAX = 32767
PIECES_FOR_BONUS = 4     # setbonus::PIECES_FOR_BONUS
BLESSING_PCT = 20        # wie_util::stats::BLESSING_REDUCE_PCT
WITCH_PCT = 20           # WITCH_MAX_HP_PCT (and the two accessories' 20)
DRAIN_CAP_PCT = 20       # a hit's drain is capped at 20 % of max HP (ITEM_BALANCE_AUDIT_156 §4)

SLOTS = ("w1", "w2", 0, 1, 2, 3, "a1", "a2")
SLOT_KO = {"w1": "주무기", "w2": "보조무기", 0: "투구", 1: "갑옷", 2: "장갑", 3: "신발", "a1": "장신구1", "a2": "장신구2"}


def s16(v):
    v &= 0xFFFF
    return v - 0x10000 if v >= 0x8000 else v


def orbs(src):
    out = {}
    dec, _ = D.decode(16, src.dats[16])
    for r in dec:
        t = bytes.fromhex(r["tail"])
        name = r["name"][:-1] if r["name"].endswith("의") else r["name"]
        out[name] = (t[0], B.s8(t[1]))
    return out


def attack_kinds(src):
    """species id -> True when its enemyg row says PHYSICAL (byte 5 bit 0)."""
    return [r[5] & 1 == 1 for r in records(src.jar.read("res/dat/enemyg_dat"))]


class Char:
    """The user's character: Lv 99 크레이지암즈, the four stats given outright."""

    def __init__(self, M, cname, plv, st):
        b = next(x for x in M.builds if x.cname == cname)
        self.cname, self.plv, self.st, self.trees, self.passive = cname, plv, st, b.trees, b.passive
        self.spear_class = b.spear_class


class Setup:
    def __init__(self, label, gear, orb, main="w1", refine=REFINE_MAX, filled=SOCKETS_MAX):
        self.label, self.gear, self.orb, self.main, self.refine, self.filled = label, gear, orb, main, refine, filled


class Audit:
    def __init__(self, src, plv, st, cname, before=False):
        self.src = src
        # `before` = the game as 159차 measured it: no defence % cap, both stats folded at i16,
        # a boss's skill attacks left at their spawn values.
        self.before = before
        self.def_cap = None if before else src.def_cap
        self.M = B.Model(src, plv)
        self.orbs = orbs(src)
        self.phys = attack_kinds(src)
        self.c = Char(self.M, cname, plv, st)
        self.set_of = self.M.set_of

    # ── the character sheet ─────────────────────────────────────────────────────────
    def sheet(self, su):
        c, M = self.c, self.M
        s = dict(c.passive)
        host = dict(taken=0, dealt=0, kill_hp=0, hit_hp=0, regen5=0)
        eff, notes = {}, []

        def add(k, v):
            s[k] = s.get(k, 0) + v
        sets = {}
        for slot in SLOTS:
            name = su.gear.get(slot)
            if not name:
                continue
            r = M.by_name[name]
            weapon = 4 <= r["cat"] <= 10
            bonus = 0
            orb = su.orb.get(slot)
            if orb and su.filled:
                sid, power = self.orbs[orb]
                bonus = power * (su.filled * su.filled + su.filled) // 2
                if 2 <= sid <= 0x12:
                    add(sid, bonus)
            eff[slot] = (r["main"] + su.refine * (2 if weapon else 1)) * (100 + bonus) // 100
            for k, v in r["opts"]:
                if k == B.REGEN5:
                    host["regen5"] += v
                else:
                    add(k, v)
            if r["third"]:
                kind, sid, val = r["third"]
                if kind == "Stat":
                    add(sid, val)
                elif kind == "AtkSpeed":
                    add(B.ATKSPD, val)
                elif kind == "Taken":
                    host["taken"] += val
                elif kind == "Dealt":
                    host["dealt"] += val
                elif kind == "KillMaxHp":
                    host["kill_hp"] += val
                elif kind == "HitMaxHp":
                    host["hit_hp"] += val
            key = (r["cat"], r["idx"])
            if key in self.set_of:
                sets.setdefault(self.set_of[key], set()).add(key)
        setfx = {B.SET_NAMES[sid] for sid, pcs in sets.items() if len(pcs) >= PIECES_FOR_BONUS}
        b11 = [M.by_name[su.gear[c_]]["setid"] if su.gear.get(c_) else 0 for c_ in (0, 1, 2, 3)]
        native = len(set(b11)) == 1 and b11[0] > 0
        if native:
            for k in (B.HPMAX, B.PATK, B.MATK, B.PDEF, B.MDEF, B.SPCOST):
                add(k, 20)
        st = c.st
        wname = su.gear[su.main]
        wcat = M.by_name[wname]["cat"]
        phys = wcat in B.PHYS
        a_raw = eff[su.main] * (100 + st[0 if phys else 2] + s.get(B.PATK if phys else B.MATK, 0)) // 100
        armor = sum(eff.get(c_, 0) for c_ in (0, 1, 2, 3))
        pct = lambda k: s.get(k, 0) if self.def_cap is None else min(s.get(k, 0), self.def_cap)
        dp_raw = (armor + 10) * (100 + pct(B.PDEF)) // 100 + st[1]
        ds_raw = (armor + 10) * (100 + pct(B.MDEF)) // 100 + st[1]
        for k, name in ((B.PDEF, "물방%"), (B.MDEF, "특방%")):
            if self.def_cap is not None and s.get(k, 0) > self.def_cap:
                notes.append("%s 합 %d → 상한 %d (%d 버림)" % (name, s.get(k, 0), self.def_cap, s.get(k, 0) - self.def_cap))
        for what, v in (("공격력", a_raw), ("물리방어", dp_raw), ("특수방어", ds_raw)):
            if v > I16_MAX:
                notes.append(("⛔ %s %d 이 i16 을 넘어 %d 로 접힌다" % (what, v, s16(v))) if self.before
                             else ("%s %d — 화면 표시는 32767, 전투는 진짜 값" % (what, v)))
        fold = s16 if self.before else (lambda v: v)
        hp0 = 14 * c.plv + 18 * st[1]
        d24, d25 = B.TREE_D[wcat]
        return dict(
            s=s, eff=eff, setfx=setfx, native=native, notes=notes, phys=phys, wcat=wcat, d24=d24, d25=d25,
            A=fold(a_raw), dp=fold(dp_raw), ds=fold(ds_raw), armor=armor,
            hp=hp0 * (100 + s.get(B.HPMAX, 0)) // 100,
            hp_buffed=hp0 * (100 + s.get(B.HPMAX, 0)) // 100 * (100 + min(100, host["kill_hp"] + host["hit_hp"]
                                                                       + (WITCH_PCT if "붉은마녀" in setfx else 0))) // 100,
            ev=s.get(B.BLOCK, 0) if c.spear_class else st[3] // 3 + s.get(B.DODGE, 0),
            acc=min(s.get(B.ACC, 0), self.src.hit_cap), stun=s.get(B.STUN, 0),
            # an accessory's own power IS 저항 (0x4d204 sums the two worn ones, 0x4cf2c adds the option sum)
            res=s.get(B.RES, 0) + eff.get("a1", 0) + eff.get("a2", 0),
            pierce=max(0, min(100, s.get(B.PIERCE, 0))), drain=s.get(B.DRAIN, 0),
            cd=min(s.get(B.CD, 0), self.src.cd_cap), speed=min(s.get(B.ATKSPD, 0), self.src.max_speed),
            host=host)

    # ── a floor ────────────────────────────────────────────────────────────────────
    def floor_monsters(self, floor):
        mon = self.M.mon
        f = mon.floors[floor]
        out = []
        first = f["species"][0]
        physical = False if first > 160 else self.phys[first]
        for k, sid in enumerate(f["species"]):
            m = mon.tower(mon.sp[sid], floor, self.c.plv, escort=f["boss"] and k > 0)
            m.update(sid=sid, boss=sid > 160, physical=physical)
            if self.before:                    # the forks grew only the first attack value
                m["skill"] = tuple(mon.sp[sid].get("skill", ()))
            m["name"] = m["name"].lstrip("{")
            out.append(m)
        return f, out

    def versus(self, sh, m):
        c = self.c
        d = m["lv"] - c.plv
        clamp = B.Model.clamp
        # player -> monster
        p_hit = clamp(80 + sh["acc"] + sh["d24"] - m["dodge"] - 5 * d, 15, 95)
        p_crit = clamp(sh["d25"] + sh["stun"] - m["res"] - d, 2, 60)
        mdef = m["dp"] if sh["phys"] else m["ds"]
        mult = (1.5 if "빙해" in sh["setfx"] else 1.0) * (1 + sh["host"]["dealt"] / 100)
        out = {}
        for key, k in (("basic", 1.0), ("skill", B.SKILL_MUL)):
            raw = sh["A"] * k - mdef
            if raw <= 1:
                n = cr = 1.0
            else:
                n = raw + mdef * sh["pierce"] / 100
                cr = 1.5 * raw + mdef * sh["pierce"] / 100
            out[key] = p_hit / 100 * ((1 - p_crit / 100) * n + p_crit / 100 * cr) * mult
        if "코르버스" in sh["setfx"]:
            out = {k: v * 4 / 3 for k, v in out.items()}
        # monster -> player
        m_hit = clamp(80 + m["hit"] - sh["ev"] + 5 * d, 15, 95)
        m_crit = clamp(2 * d - sh["res"], 2, 60)
        dfn = sh["dp"] if m["physical"] else sh["ds"]
        cut = (1 - sh["host"]["taken"] / 100) * (1 - (BLESSING_PCT / 100 if "레갈리스근위" in sh["setfx"] else 0))
        hit = lambda atk, mul=2: max(1, int(max(1, min(I16_MAX, (atk - dfn) * mul // 2)) * cut))
        one, crit = hit(m["atk"]), hit(m["atk"], 3)
        # a boss's two skill attacks go through the same function (0x26d7c), special defence
        skills = [(hit(a), hit(a, 3)) for a in m.get("skill", ()) if a]
        return dict(p_hit=p_hit, p_crit=p_crit, basic=out["basic"], skill=out["skill"], m_hit=m_hit, m_crit=m_crit,
                    one=one, crit=crit, skills=skills,
                    exp=m_hit / 100 * ((1 - m_crit / 100) * one + m_crit / 100 * crit))

    # ── reports ────────────────────────────────────────────────────────────────────
    def report(self, su, floor):
        M, sh = self.M, self.sheet(su)
        f, mons = self.floor_monsters(floor)
        spd = M.speed(sh["speed"])
        print("\n### %s" % su.label)
        for slot in SLOTS:
            if su.gear.get(slot):
                print("  %-4s %-8s %s  [%s%s]  성능 %d" % (
                    SLOT_KO[slot], su.gear[slot], B.opt_text(M.by_name[su.gear[slot]]),
                    (su.orb.get(slot) + " x%d" % su.filled) if su.orb.get(slot) and su.filled else "오브 없음",
                    " · 제련 +%d" % su.refine if su.refine else "", sh["eff"][slot]))
        print("  → 공격력 %d · 물리방어 %d · 특수방어 %d · 최대HP %d(처치/피격 버프 %d) · 회피 %d · 명중 %d · 기절 %d · 저항 %d"
              % (sh["A"], sh["dp"], sh["ds"], sh["hp"], sh["hp_buffed"], sh["ev"], sh["acc"], sh["stun"], sh["res"]))
        print("    방패무시 %d%% · 흡수 %d · 쿨타임 -%d%% · 공속 +%d(x%.2f) · 피해감소 %d%% · 세트효과 %s · 원작 4부위 %s"
              % (sh["pierce"], sh["drain"], sh["cd"], sh["speed"], spd, sh["host"]["taken"],
                 "/".join(sorted(sh["setfx"])) or "없음", "켜짐" if sh["native"] else "꺼짐"))
        for n in sh["notes"]:
            print("    " + n)
        print("  %d층 (%s, 플레이어가 맞는 방어 = %s):" % (floor, "보스층" if f["boss"] else "일반층",
                                                  "물리" if mons[0]["physical"] else "특수"))
        n_esc = 8 if f["boss"] else 0
        tot_hp = worst = expect = 0
        attacks = 0.0
        for k, m in enumerate(mons):
            v = self.versus(sh, m)
            cnt = 1 if (f["boss"] and k == 0) else (n_esc // max(1, len(mons) - 1) if f["boss"] else 1)
            tot_hp += m["hp"] * cnt
            worst += v["crit"] * cnt
            expect += v["exp"] * cnt
            attacks += m["hp"] / max(v["basic"], 1) * cnt
            print("    %-8s Lv%d HP %6d 공 %5d x%d | 내가: 적중 %2d%% 크리 %2d%% 기본 %6.0f 스킬 %6.0f (기본 %4.1f타) | 맞을 때: %2d%% · 한 방 %5d(크리 %2d%% %5d) = HP 의 %5.1f%%"
                  % (m["name"], m["lv"], m["hp"], m["atk"], cnt, v["p_hit"], v["p_crit"], v["basic"], v["skill"],
                     m["hp"] / max(v["basic"], 1), v["m_hit"], v["one"], v["m_crit"], v["crit"], 100 * v["one"] / sh["hp"]))
            if v["skills"]:
                print("             보스 스킬 공격(공 %s): %s" % (
                    " · ".join(str(a) for a in m["skill"] if a),
                    " · ".join("한 방 %d(크리 %d) = HP 의 %.0f%%" % (o, c, 100 * o / sh["hp"]) for o, c in v["skills"])))
        heal = min(sh["hp"] * DRAIN_CAP_PCT / 100, self.versus(sh, mons[0])["basic"] * sh["drain"] / 200) if sh["drain"] else 0
        print("    합계: 몬스터 HP %d · 기본 공격 약 %.0f타 · 전원이 동시에 크리로 때리면 %d (HP 의 %.0f%%) · 한 바퀴 기대 피해 %.0f (HP 의 %.1f%%) · 흡수 타격당 +%.0f"
              % (tot_hp, attacks, worst, 100 * worst / sh["hp"], expect, 100 * expect / sh["hp"], heal))
        return sh

    def sweep(self, su, lo=1, hi=100):
        """Every floor: the worst single (crit) hit as a share of max HP, and the floors that one-shot."""
        sh = self.sheet(su)
        bad, worst = [], (0, 0, "")
        for floor in range(lo, hi + 1):
            _, mons = self.floor_monsters(floor)
            for m in mons:
                v = self.versus(sh, m)
                share = v["crit"] / sh["hp"]
                if share > worst[0]:
                    worst = (share, floor, m["name"])
                if v["crit"] >= sh["hp"]:
                    bad.append(floor)
        bad = sorted(set(bad))
        print("  %-34s 최악의 한 방 = %d층 %s, HP 의 %.0f%% · 한 방에 죽는 층 %s"
              % (su.label, worst[1], worst[2], 100 * worst[0], ("%d개 (%d~%d층)" % (len(bad), bad[0], bad[-1])) if bad else "없음"))


def matrix(A, floor):
    """armour set x how the four armour sockets are split — the one decision that settles the floor."""
    W = {"w1": "시즈타이탄주포", "w2": "검은공장의걸작"}
    acc = {"a1": "레아의목걸이", "a2": "이안의맹세"}
    sets = [
        ("붉은마녀 4 + 레아 · 이안", {0: "붉은마녀헬름", 1: "붉은마녀로브", 2: "붉은마녀장갑", 3: "붉은마녀신발", **acc}),
        ("빙해 4 + 레아 · 이안", {0: "빙해의투구", 1: "빙해의갑옷", 2: "빙해의장갑", 3: "빙해의장화", **acc}),
        ("코르버스 4 + 레아 · 이안", {0: "코르버스후드", 1: "코르버스암살복", 2: "코르버스크로우", 3: "코르버스스텝", **acc}),
        ("전설(장로회 · 멜하르 · 오딘 · 섬광) + 레아 · 이안", {0: "장로회의관", 1: "멜하르의흉갑", 2: "오딘의손길", 3: "섬광의군화", **acc}),
        ("전설(예언자 · 광전사 · 오딘 · 섬광) + 율리케 · 일레느",
         {0: "예언자의눈", 1: "광전사의흑갑", 2: "오딘의손길", 3: "섬광의군화", "a1": "율리케의목걸이", "a2": "일레느의각인"}),
    ]
    splits = [("방어 오브 0 (투신 x4)", ("투신",) * 4), ("정령1 금강1", ("정령", "금강", "투신", "투신")),
              ("정령2 금강2", ("정령", "정령", "금강", "금강")), ("정령3 금강1", ("정령", "정령", "정령", "금강")),
              ("정령4", ("정령",) * 4)]
    mon = A.floor_monsters(floor)[1]
    print("\n## %d층 — 방어구 x 방어구 네 칸의 오브 (무기 뇌제 x5 둘 · 장신구 투신 + 흡혈 · 전부 제련 +%d · 오브 x%d)"
          % (floor, REFINE_MAX, SOCKETS_MAX))
    print("칸 = 보스 한 방 → 최대 HP 대비 % (크리 한 방) · 호위 한 방 · 1~100층 중 한 방(크리)에 죽는 층 수")
    print("| 방어구 | 최대 HP | 공격력 | " + " | ".join(n for n, _ in splits) + " |")
    print("|---|---|---|" + "---|" * len(splits))
    for label, gear in sets:
        cells = []
        for _, sp in splits:
            su = Setup(label, {**W, **gear}, {"w1": "뇌제", "w2": "뇌제", 0: sp[0], 1: sp[1], 2: sp[2], 3: sp[3], "a1": "투신", "a2": "흡혈"})
            sh = A.sheet(su)
            v = [A.versus(sh, m) for m in mon]
            dead = set()
            for fl in range(1, 101):
                for m in A.floor_monsters(fl)[1]:
                    if A.versus(sh, m)["crit"] >= sh["hp"]:
                        dead.add(fl)
            cells.append("%d → %.0f%% (%d) · %d · %s%s" % (
                v[0]["one"], 100 * v[0]["one"] / sh["hp"], v[0]["crit"], max(x["one"] for x in v[1:]),
                ("%d층" % len(dead)) if dead else "없음", " ⛔접힘" if any("접힌다" in n for n in sh["notes"]) else ""))
        print("| %s | %d | %d | %s |" % (label, sh["hp"], sh["A"], " | ".join(cells)))


def sockets(A, floor):
    """The balanced layout at 0..5 filled sockets — how much of the answer is the orbs."""
    su0 = setups()[2]
    mon = A.floor_monsters(floor)[1]
    print("\n## 홈 수에 따른 변화 — 「%s」 배치를 홈 f 개만 채웠을 때" % su0.label)
    print("| 홈 | 오브 보너스(세기 15) | 공격력 | 물리 / 특수방어 | 최대 HP | 보스 한 방(크리) | 호위 한 방 | 층 전체(기본 공격) |")
    print("|---|---|---|---|---|---|---|---|")
    for f in range(0, SOCKETS_MAX + 1):
        sh = A.sheet(Setup(su0.label, su0.gear, su0.orb, filled=f))
        v = [A.versus(sh, m) for m in mon]
        hits = mon[0]["hp"] / max(v[0]["basic"], 1) + 4 * sum(m["hp"] / max(x["basic"], 1) for m, x in zip(mon[1:], v[1:]))
        print("| %d | +%d %% | %d | %d / %d | %d | %d (%d) | %d | 약 %.0f타 |" % (
            f, 15 * (f * f + f) // 2, sh["A"], sh["dp"], sh["ds"], sh["hp"], v[0]["one"], v[0]["crit"],
            max(x["one"] for x in v[1:]), hits))


def setups():
    W = {"w1": "시즈타이탄주포", "w2": "검은공장의걸작"}
    witch = {**W, **{0: "붉은마녀헬름", 1: "붉은마녀로브", 2: "붉은마녀장갑", 3: "붉은마녀신발", "a1": "레아의목걸이", "a2": "이안의맹세"}}
    legend = {**W, **{0: "장로회의관", 1: "율리케의성의", 2: "오딘의손길", 3: "바질의발걸음", "a1": "레아의목걸이", "a2": "이안의맹세"}}
    ice = {**W, **{0: "빙해의투구", 1: "빙해의갑옷", 2: "빙해의장갑", 3: "빙해의장화", "a1": "레아의목걸이", "a2": "율리케의목걸이"}}
    none = {}
    attack = {"w1": "뇌제", "w2": "뇌제", 0: "투신", 1: "투신", 2: "속박", 3: "영제", "a1": "투신", "a2": "흡혈"}
    balanced = {"w1": "뇌제", "w2": "뇌제", 0: "정령", 1: "정령", 2: "금강", 3: "금강", "a1": "투신", "a2": "흡혈"}
    armor_m = {"w1": "뇌제", "w2": "뇌제", 0: "정령", 1: "정령", 2: "정령", 3: "금강", "a1": "투신", "a2": "흡혈"}
    wall = {"w1": "정령", "w2": "금강", 0: "정령", 1: "정령", 2: "금강", 3: "금강", "a1": "정령", "a2": "금강"}
    return [
        Setup("A. 붉은마녀 세트 · 제련만(오브 없음)", witch, none, filled=0),
        Setup("B. 붉은마녀 세트 · 공격형 오브(방어 오브 0)", witch, attack),
        Setup("C. 붉은마녀 세트 · 균형 오브(방어구 정령2 금강2)", witch, balanced),
        Setup("D. 전설 방어구 · 균형 오브(방어구 정령2 금강2)", legend, balanced),
        Setup("E. 빙해 세트 · 균형 오브(방어구 정령2 금강2)", ice, balanced),
        Setup("F. 붉은마녀 세트 · 방어구 정령3 금강1", witch, armor_m),
        Setup("G. 붉은마녀 세트 · 8칸 전부 방어 오브(정령4 금강4)", witch, wall),
        Setup("H. 전설 방어구 · 8칸 전부 방어 오브(정령4 금강4)", legend, wall),
    ]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--zip", default=B.DEFAULT_ZIP)
    ap.add_argument("--plv", type=int, default=99)
    ap.add_argument("--vit", type=int, default=110)
    ap.add_argument("--int", dest="int_", type=int, default=200)
    ap.add_argument("--floor", type=int, default=100)
    ap.add_argument("--cls", default="크레이지암즈")
    ap.add_argument("--detail", action="store_true", help="print every setup's full sheet")
    ap.add_argument("--before", action="store_true",
                    help="the game before 160차: no defence %% cap, i16 fold, boss skills at their spawn values")
    a = ap.parse_args(argv)
    src = B.Src(a.zip)
    base = next(b for c, t, b in B.classes(src) if c == a.cls)
    st = [base[0], a.vit, a.int_, base[3]]
    pts = sum((min(v, 99) - b0) + 2 * max(0, v - 99) for v, b0 in zip(st, base))
    A = Audit(src, a.plv, st, a.cls, before=a.before)
    print("# 하늘성 %d층 클리어 점검 — %s Lv %d · 힘 %d 체력 %d 정신 %d 민첩 %d (스탯 포인트 %d / %d)"
          % (a.floor, a.cls, a.plv, st[0], st[1], st[2], st[3], pts, 4 * (a.plv - 1)))
    print("규칙: " + ("160차 이전 — 방어% 상한 없음 · 공격력/방어력 i16 접힘 · 보스 스킬 공격은 스폰값" if a.before else
                    "방어%% 상한 %d · 공격력/방어력 i16 해제 · 보스 스킬 공격도 층 레벨로 성장 (160차)" % src.def_cap))
    print("오브: " + " · ".join("%s(%s %+d → x5 = %+d)" % (n, B.ONAME.get(sid, "성능"), p, p * 15)
                             for n, (sid, p) in A.orbs.items() if n != "소켓확장"))
    sus = setups()
    for su in (sus if a.detail else (sus[0], sus[2])):
        A.report(su, a.floor)
    matrix(A, a.floor)
    sockets(A, a.floor)
    print("\n## 1~100층 전수 — 한 방(크리)이 최대 HP 를 넘는 층")
    for su in sus:
        A.sweep(su)


if __name__ == "__main__":
    main()
