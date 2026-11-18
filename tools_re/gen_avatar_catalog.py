#!/usr/bin/env python3
"""아바타 옷장 (167차) — `costumes/*/costume.txt` -> `wie_android/src/avatar_catalog.rs`.

    python tools_re/gen_avatar_catalog.py            # 다시 쓴다
    python tools_re/gen_avatar_catalog.py --check    # 디스크의 파일이 지금 원천과 같은지 (다르면 exit 1)

한 폴더 = 한 세트. 폴더 이름 `NN_<slug>` 의 NN 이 세트 번호(1..50)이고, 그 세트의 시트는 게임 파일 이름에서
스프라이트 `49 + NN` 로 실린다(`wie_util::avatar::sheet_sprite`). costume.txt 의 키:

    set = <세트 이름>
    acquire = starter | clear.normal | clear.hard | clear.hell
    hero = kei | ritz                                                   (없으면 두 주인공 모두 — 168차)
    head = own                                                          (171차: 투구 시트가 얼굴까지 머리 전체를 그린다 —
                                                                         게임은 그 세트를 입은 동안 주인공 자신의 머리를 안 그린다)
    helmet / armor / gloves / shoes / weapon / shield = <조각 이름>      (없는 칸 = 그 세트에 조각이 없다)
    <칸>.desc = <한 줄 설명>
    weapon.<spear|sword|knife|gun|rifle|dark|holy> = <그 무기 종류에서의 이름>   (없으면 weapon)

⛔ 이름·설명은 게임 글꼴이 그리는 글자만 — KS X 1001 한글과 ASCII. 벗어난 글자는 게임 글꼴이 조용히 건너뛰므로
   여기서 거절한다. 폭: 조각 이름 ≤ 96 px(목록 칸 — 오른쪽 끝의 착용 표시 E 를 비켜서) · 세트 이름과 설명 ≤ 143 px(정보 칸), 게임 그리기 규칙
   (`gamefont::game_layout`: 한글 11 · 공백 5 · `.`/`,` 4 · 그 밖의 ASCII 8).
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "costumes")
OUT = os.path.join(ROOT, "wie_android", "src", "avatar_catalog.rs")

SLOTS = ["helmet", "armor", "gloves", "shoes", "weapon", "shield"]
WEAPON_KINDS = ["spear", "sword", "knife", "gun", "rifle", "dark", "holy"]  # categories 4..=10
ACQUIRE = {"starter": "Acquire::Starter", "clear.normal": "Acquire::Clear(0)",
           "clear.hard": "Acquire::Clear(1)", "clear.hell": "Acquire::Clear(2)"}
HERO = {"kei": "Some(Hero::Kei)", "ritz": "Some(Hero::Ritz)"}
SET_MAX = 50
ROW_PX = 96
INFO_PX = 143


def width(s):
    w = 0
    for ch in s:
        if ch == " ":
            w += 5
        elif ch in ".,":
            w += 4
        elif ord(ch) < 0x80:
            w += 8
        else:
            w += 11
    return w


def drawable(s):
    for ch in s:
        if ord(ch) < 0x80:
            if not (0x20 <= ord(ch) < 0x7f):
                return False
            continue
        try:
            b = ch.encode("euc_kr")
        except UnicodeEncodeError:
            return False
        # KS X 1001 Hangul syllables are rows 0xb0..0xc8; the game font has those and ASCII only.
        if not (len(b) == 2 and 0xb0 <= b[0] <= 0xc8):
            return False
    return True


def parse(path):
    kv = {}
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            if "=" not in line:
                raise SystemExit(f"{path}:{n}: not `key = value`")
            k, v = (p.strip() for p in line.split("=", 1))
            if k in kv:
                raise SystemExit(f"{path}:{n}: {k} twice")
            kv[k] = v
    return kv


def load():
    sets = []
    for d in sorted(os.listdir(SRC)):
        full = os.path.join(SRC, d)
        if not os.path.isdir(full):
            continue
        m = re.fullmatch(r"(\d\d)_([a-z0-9_]+)", d)
        if not m:
            raise SystemExit(f"costumes/{d}: folder must be NN_<slug>")
        sid = int(m.group(1))
        if not 1 <= sid <= SET_MAX:
            raise SystemExit(f"costumes/{d}: set number {sid} outside 1..{SET_MAX}")
        txt = os.path.join(full, "costume.txt")
        if not os.path.exists(txt):
            raise SystemExit(f"costumes/{d}: no costume.txt")
        kv = parse(txt)
        known = {"set", "acquire", "hero", "head"} | set(SLOTS) | {s + ".desc" for s in SLOTS} | {"weapon." + w for w in WEAPON_KINDS}
        for k in kv:
            if k not in known:
                raise SystemExit(f"costumes/{d}/costume.txt: unknown key {k}")
        if kv.get("acquire") not in ACQUIRE:
            raise SystemExit(f"costumes/{d}/costume.txt: acquire must be one of {sorted(ACQUIRE)}")
        if "hero" in kv and kv["hero"] not in HERO:
            raise SystemExit(f"costumes/{d}/costume.txt: hero must be one of {sorted(HERO)}")
        if "head" in kv and kv["head"] != "own":
            raise SystemExit(f"costumes/{d}/costume.txt: head must be `own` (or absent)")
        if "head" in kv and not kv.get("helmet"):
            raise SystemExit(f"costumes/{d}/costume.txt: head = own needs a helmet piece")
        name = kv.get("set", "")
        if not name:
            raise SystemExit(f"costumes/{d}/costume.txt: no set name")
        pieces = [kv.get(s, "") for s in SLOTS]
        descs = [kv.get(s + ".desc", "") for s in SLOTS]
        weapons = [kv.get("weapon." + w, "") for w in WEAPON_KINDS]
        if any(weapons) and not pieces[4]:
            raise SystemExit(f"costumes/{d}/costume.txt: weapon.<kind> names need a `weapon` default")
        for s, p, ds in zip(SLOTS, pieces, descs):
            if ds and not p:
                raise SystemExit(f"costumes/{d}/costume.txt: {s}.desc without {s}")
        for label, text, limit in ([("set", name, INFO_PX)]
                                   + [(s, p, ROW_PX) for s, p in zip(SLOTS, pieces)]
                                   + [("weapon." + w, n, ROW_PX) for w, n in zip(WEAPON_KINDS, weapons)]
                                   + [(s + ".desc", t, INFO_PX) for s, t in zip(SLOTS, descs)]):
            if not text:
                continue
            if not drawable(text):
                raise SystemExit(f"costumes/{d}/costume.txt: {label} {text!r} has a glyph the game font cannot draw")
            if width(text) > limit:
                raise SystemExit(f"costumes/{d}/costume.txt: {label} {text!r} is {width(text)} px, the box holds {limit}")
        sets.append(dict(id=sid, folder=d, name=name, acquire=ACQUIRE[kv["acquire"]], hero=HERO.get(kv.get("hero"), "None"),
                         pieces=pieces, descs=descs, weapons=weapons, own_head="head" in kv))
    ids = [s["id"] for s in sets]
    if len(set(ids)) != len(ids):
        raise SystemExit(f"two folders share a set number: {ids}")
    return sets


def rs_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render(sets):
    out = []
    w = out.append
    w("// GENERATED by `tools_re/gen_avatar_catalog.py` from `costumes/*/costume.txt` — do not edit.")
    w("// Re-run the tool after changing a costume.txt; `--check` fails when this file is stale.")
    w("//")
    w("// ★ 167차 아바타 옷장: one row per costume SET. A set's sheets ride in the jar under sprite")
    w("// `49 + id` (`wie_util::avatar::sheet_sprite`); a piece listed here but without its sheet in the")
    w("// jar is simply not offered (the AVATAR page lists only what the published masks say exists).")
    w("// Dependency-free: `avatar_selftest.rs` `include!`s this file.")
    w("")
    w("/// How a set is obtained.")
    w("#[derive(Clone, Copy, Debug, PartialEq, Eq)]")
    w("pub enum Acquire {")
    w("    /// Every character owns it from the start.")
    w("    Starter,")
    w("    /// The ending clear at this difficulty (0 노말 · 1 하드코어 · 2 헬) — the byte the 2회차")
    w("    /// transition found (`wie_util::stats::ending_transition`).")
    w("    Clear(u8),")
    w("}")
    w("")
    w("/// ★ 168차: the hero a set is made for (`CHAR+0xb8`, `items::char_hero`) — such a set is offered")
    w("/// only in that hero's wardrobe.")
    w("#[derive(Clone, Copy, Debug, PartialEq, Eq)]")
    w("pub enum Hero {")
    w("    Ritz,")
    w("    Kei,")
    w("}")
    w("")
    w("/// One costume set.")
    w("#[derive(Clone, Copy, Debug)]")
    w("pub struct SetDef {")
    w("    /// The set number, 1..=50 — also the save's worn byte value.")
    w("    pub id: u8,")
    w("    pub name: &'static str,")
    w("    pub acquire: Acquire,")
    w("    /// `None` = both heroes; `Some(h)` = only `h`'s wardrobe offers it.")
    w("    pub hero: Option<Hero>,")
    w("    /// Per avatar slot (투구 · 갑옷 · 장갑 · 신발 · 무기 · 방패): the piece's name, `\"\"` = no piece.")
    w("    pub pieces: [&'static str; 6],")
    w("    /// Per avatar slot: the one-line description.")
    w("    pub descs: [&'static str; 6],")
    w("    /// The weapon's name per category 스피어 · 스워드 · 나이프 · 건 · 라이플 · 다크스톤 · 홀리스톤")
    w("    /// (4..=10); `\"\"` = `pieces[4]`.")
    w("    pub weapons: [&'static str; 7],")
    w("    /// ★ 171차 (`head = own`): the 투구 sheet draws the whole head, the hero's own face included,")
    w("    /// so the game leaves the hero's head out while the 투구 slot shows it")
    w("    /// (`wie_util::stats::AVATAR_OWN_HEAD`, the painter hook `handle_head_paint_lgt`).")
    w("    pub own_head: bool,")
    w("}")
    w("")
    w("#[rustfmt::skip]")
    w("pub static SETS: &[SetDef] = &[")
    for s in sets:
        w(f"    // costumes/{s['folder']}")
        w("    SetDef {")
        w(f"        id: {s['id']},")
        w(f"        name: {rs_str(s['name'])},")
        w(f"        acquire: {s['acquire']},")
        w(f"        hero: {s['hero']},")
        w("        pieces: [" + ", ".join(rs_str(p) for p in s["pieces"]) + "],")
        w("        descs: [" + ", ".join(rs_str(p) for p in s["descs"]) + "],")
        w("        weapons: [" + ", ".join(rs_str(p) for p in s["weapons"]) + "],")
        w(f"        own_head: {'true' if s['own_head'] else 'false'},")
        w("    },")
    w("];")
    w("")
    return "\n".join(out)


def main():
    text = render(load())
    if "--check" in sys.argv[1:]:
        try:
            with open(OUT, encoding="utf-8", newline="") as f:
                have = f.read().replace("\r\n", "\n")
        except FileNotFoundError:
            have = None
        if have != text:
            print(f"STALE: {os.path.relpath(OUT, ROOT)} — re-run tools_re/gen_avatar_catalog.py")
            return 1
        print(f"OK: {os.path.relpath(OUT, ROOT)} matches costumes/")
        return 0
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"wrote {os.path.relpath(OUT, ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
