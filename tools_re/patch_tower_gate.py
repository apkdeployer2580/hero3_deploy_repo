# -*- coding: utf-8 -*-
"""하늘성: 층으로 쓰는 씬의 A행 스크립트를 **탑에 있는 동안만** 끈다.

── 왜 이것이 필요한가 (2026-09-11, 제보: "몬스터 잡았더니 갑자기 퀘스트가 진행됐어")

층 조건은 사용자가 정했다(2026-09-11): *"맵이 나눠져 있으면 안돼 … 걸어서 갈수 없는곳에
몬스터가 있다거나 … 몹 리젠과 아이템도 나오면 안되고 기존에 있던 상호작용하는것들도 있으면
안돼"*. `gen_guardian_floors.py` 는 그것을 **씬을 고르는 것**으로 풀려고 했고, 「나가는 문이
없는 씬」 24개를 찾았다고 했다. 그 24개는 **없었다** — `exit_rows` 의 컨테이너 워커가 A행
맨 앞의 컨테이너 **하나만** 읽고 멈췄기 때문이다. 보스 아레나의 A행은 컨테이너가 **둘**이고,
둘째가 보스를 잡은 뒤의 스크립트를 고른다(대사 + 워프). 그래서 층 표가 「비우면 스토리가
도는 방」으로 가득 찼다.

워커를 고치고 다시 세면 조건을 통과하는 씬은 **0개**다. 게임의 방은 전부 A행을 갖고 있다.
⇒ 씬을 고르는 것으로는 풀 수 없고, **스크립트가 돌지 않게** 해야 한다.

── 어떻게

게임 자신의 조건 기계를 쓴다. A행의 payload 는 조건 컨테이너일 수 있고
(`ff <n> <len>[n]` + 블록 n개, 블록 = `(cond)* <payload>`, 순서대로 평가해 첫 매치가 이긴다),
조건 `fa <id> <val>` 은 영구 flag 배열(`base+0x9e98`)을 읽는다. 그래서 **모든 블록 앞에
`fa TOWER 00` 을 한 줄 붙이면** 그 행은 「TOWER 플래그가 0일 때만」 움직인다.

  * 탑 밖(플래그 0) — 조건이 늘 참이므로 **선택되는 payload 집합이 원본과 똑같다**.
    이 도구가 그것을 씬마다 다시 계산해서 증명한다(`verify`).
  * 탑 안(플래그 1) — **퀘스트를 진행시키는 블록만** 매치하지 않는다: 대사(`0x34`),
    워프(`0x28`), 영구 플래그 쓰기(`0x23`). 나머지 블록은 그대로 돈다.

⛔⛔ **전부 막으면 화면이 검게 나온다** (2026-09-11, 기기 실측 vc291). 씬의 A행 중 하나는
진입할 때 도는 준비 스크립트이고(보스 아레나에서는 `1f <종> …` = 등장, ops `{1f,2a,29}`),
그것까지 막으면 게임이 페이드를 안 걷어서 HUD 만 뜨고 월드가 안 그려진다. 그래서 게이트는
**블록 단위**이고 판정은 그 블록이 고르는 B 레코드의 옵코드다.

몬스터는 영향이 없다. 스폰은 **맵의 스폰 지점 + scn 헤더의 종 세 워드**가 만들고
(`GUARDIAN_CASTLE_FEASIBILITY.md §B`), 그것은 A행을 거치지 않는다.

⛔ 이 게이트가 곧 「문이 없다」이기도 하다 — 출구 워프는 전부 A행이 고르는 B 레코드 안에
있으므로, A행이 아무것도 안 고르면 나갈 문도 없다. `gen_guardian_floors.py` 의 문 검사는
그래서 **게이트가 심어졌는지**를 보는 검사로 바뀐다.

⛔ 에셋 도구 순서에서 이것이 **다섯째이자 마지막**이다 (`patch_guardian_npc.py` 다음).
`patch_asset.py` 는 pristine 에서 재빌드하므로 앞의 것들을 전부 날린다.
"""

import argparse
import io
import os
import shutil
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from patch_guardian_npc import parse_scn, build_scn, flag_ids_in_use  # noqa: E402
import gen_guardian_floors as G  # noqa: E402  (module level is safe: that side imports us lazily)

# ⛔ 이 id 는 `patch_guardian_npc.py` 의 0xC0/0xC1/0xC2 옆자리이고, 이 도구가 코퍼스 전수로
#    비어 있음을 확인한 뒤에만 적용한다. 바꾸려면 wie_android/src/items.rs 의
#    `GUARDIAN_FLAG_TOWER` 와 **함께** 바꿔라.
FLAG_TOWER = 0xC3

COND_FA = 0xFA
CONTAINER = 0xFF
GATE = bytes([COND_FA, FLAG_TOWER, 0x00])

# 탑에서 돌면 안 되는 옵코드. `GUARDIAN_CASTLE_FEASIBILITY.md §J.1` 의 확정 뜻에서:
#   0x28 워프            — 방에서 나가는 문이자 스토리 점프
#   0x34 0x35 대사       — C[] 문자열 출력 (`0x3c77c`)
#   0x2c 0x2d 0x2e 0x36 — 같은 함수를 부르는 대사 변형 (화자/초상 파라미터)
#   0x23 setFlag         — 영구 플래그 쓰기 = 퀘스트 진행 그 자체 (⛔ `23 ff` 는 예외 —
#                          로드마다 되돌아가는 진입 표시다, `forbidden_records`)
#   0x22                 — 다른 플래그 표(`fb` 조건이 읽는 것, getter 0x3f391) 쓰기
#   0x17 보상            — 소지금 += v (하드는 +10000) — 탑의 보상은 층 단위로 따로 준다 (§3.4)
# ★ 2026-09-11 코드리뷰 실측(VM 모델): 옛 셋(28·34·23)으로는 층 팔레트 안에서 대사 변형이 28씬 ·
#   금 보상이 8씬 · 0x22 가 13씬에서 탑 안에 살아 있었다 — 「상호작용 없음」(2026-09-11 사용자 결정)
#   에 어긋난다. 넓힌 비용은 준비 씬 140 → 139 (보스 씬 하나).
# ⛔ 이 집합을 넓힐 때는 반드시 프로브로 다시 볼 것: 진입 스크립트가 통째로 막히면 월드가
#    안 그려진다 — `tower_ready` 가 그것을 fail closed 로 걸러 주지만, 걸러진 만큼 팔레트가 준다.
FORBIDDEN_OPS = frozenset((0x28, 0x34, 0x35, 0x2C, 0x2D, 0x2E, 0x36, 0x23, 0x22, 0x17))

# A행 헤더 6바이트: `x y ? ff ? kind`, 그다음이 payload 다.
ROW_HEAD = 6
# 헤더 안의 kind 바이트 자리와, 진입(자동 실행) 행의 kind. kind 는 트리거 발자국이다
# (3 = 밟기 · 4 = 말 걸기 · 5/6 = 타일 트리거, `GUARDIAN_CASTLE_FEASIBILITY.md §J.4`) — 0 은
# 발자국이 없다 = 씬에 들어설 때 도는 행이고, 코퍼스에서 전부 (0,0) 에 앉아 있다.
ROW_KIND = 5
KIND_ENTRY = 0
# 레코드 길이는 u8 이다 (리스트가 `count | length[count] | 본문`).
REC_MAX = 0xFF


def forbidden_records(B):
    """탑에서 돌면 안 되는 B 레코드의 인덱스 집합. 못 읽는 레코드는 **금지 쪽**으로 센다.

    ★★★ 예외 하나: `23 ff <v>` — 모든 씬의 페이드인 레코드(`B[0] = 23 ff 01 33 00 08`)가 세우는
    **로드마다 되돌아가는 진입 표시**다(`gen_guardian_floors.FLAG_ENTRY_MARK`). 퀘스트 플래그가
    아니고, 이것을 막으면 그 씬은 탑 안에서 검은 화면이다(vc293 의 12개 층). 다른 id 의 `0x23`
    은 그대로 금지다.
    """
    out = set()
    for i, rec in enumerate(B):
        ops, args = [], []
        if G.walk_script(rec, ops, args) != len(rec):
            out.add(i)                       # 못 읽는 것은 fail closed
            continue
        for op, a in zip(ops, args):
            if op not in FORBIDDEN_OPS:
                continue
            if op == G.OP_SETFLAG and a[:1] == bytes([G.FLAG_ENTRY_MARK]):
                continue                     # 진입 표시 — 위 docstring
            out.add(i)
            break
    return out


def _decide_tower(tower):
    """`evaluate` 용 조건 판정: 우리 플래그만 안다. 게임의 조건은 **어느 쪽으로도 갈 수 있다**
    (그래야 「탑 밖에서 고르는 집합이 원본과 같다」가 모든 플래그 상태에 대해 성립한다)."""
    def decide(op, args):
        if op == G.COND_FLAG and args[:1] == bytes([FLAG_TOWER]):
            return args[1] == tower
        return None
    return decide


def _decide_entry(op, args):
    """`tower_ready` 용: 우리 플래그는 1(탑 안), `fa ff 00` 은 참(로더가 로드마다 되돌리는 진입
    표시 — `gen_guardian_floors.FLAG_ENTRY_MARK`), 나머지 게임 조건은 미지."""
    if op == G.COND_FLAG and args[:1] == bytes([FLAG_TOWER]):
        return args[1] == 1
    if op == G.COND_FLAG and args[:1] == bytes([G.FLAG_ENTRY_MARK]):
        return args[1] == 0
    return None


def gate_stream(stream, bad, top):
    """스트림을 게이트가 걸린 형태로 다시 만든다. `(bytes, 건 수, 이 스트림이 금지 레코드를 직접
    실행하나)` 또는 못 읽으면 `None`.

    규칙 — VM 의미(`gen_guardian_floors.evaluate` 머리말) 위에서:
    * 블록(컨테이너의 원소)이 금지 레코드를 **직접** 실행하면 그 블록 앞에 `fa c3 00` 을 붙인다.
      탑 안에서는 그 블록이 「아무것도 안 돌았다」가 되어 **다음 형제 블록**(게임 자신의 else —
      보통 「이미 본 방」의 평상시 스크립트)이 돈다. 탑 밖에서는 항상 참인 조건 하나가 더 붙을 뿐
      이라 바이트 동작이 같다. 중첩 컨테이너는 같은 규칙으로 재귀한다(옛 판은 최상위만 봤다).
    * 최상위 스트림의 금지 payload 는 블록이 없으므로 한 블록짜리 컨테이너로 감싼다.
    """
    out = bytearray()
    touched = 0
    direct_bad = False
    i = 0
    while i < len(stream):
        b = stream[i]
        if b == CONTAINER:
            got = G.container_at(stream, i)
            if got is None:
                return None
            blocks, i = got
            newblocks = []
            for blk in blocks:
                g = gate_stream(blk, bad, False)
                if g is None:
                    return None
                nb, t, has_bad = g
                if has_bad:
                    nb = GATE + nb
                    t += 1
                newblocks.append(nb)
                touched += t
            lens = [len(x) for x in newblocks]
            if any(L > 0xFF for L in lens):
                return None                  # 길이 바이트는 u8
            out += bytes([CONTAINER, len(newblocks)]) + bytes(lens)
            for nb in newblocks:
                out += nb
            continue
        if b in G.COND_LEN:
            n = G.COND_LEN[b]
            if i + n > len(stream):
                return None
            out += bytes(stream[i:i + n])
            i += n
            continue
        if b >= 0xF0:
            return None
        if b in bad:
            if top:
                block = GATE + bytes([b])
                out += bytes([CONTAINER, 1, len(block)]) + block
                touched += 1
            else:
                direct_bad = True
                out += bytes([b])
        else:
            out += bytes([b])
        i += 1
    return bytes(out), touched, direct_bad


def selectable(tail, tower):
    """이 payload 부분이 **어떤 플래그 상태에서든** 실행할 수 있는 B 인덱스 집합 (`tower` 는 우리
    플래그 값), 그리고 정확히 읽혔는지. 모델 = `gen_guardian_floors.evaluate` (VM `0x52148`)."""
    got = G.evaluate(bytes(tail), _decide_tower(tower))
    if got is None:
        return [], False
    return sorted(got[0]), True


def gate_scene(raw):
    """씬 하나를 게이트가 걸린 바이트로. `(new, note)` 또는 `(raw, None)` (변경 없음/불가)."""
    head, lists = parse_scn(raw)
    A, B, C = lists
    bad = forbidden_records(B)
    newA = []
    touched = 0
    for row in A:
        if len(row) < ROW_HEAD + 1:
            newA.append(row)
            continue
        tail = row[ROW_HEAD:]
        if GATE in tail:
            newA.append(row)                 # 이미 걸려 있다
            continue
        got = gate_stream(tail, bad, True)
        if got is None:
            return raw, None                 # 한 행이라도 못 읽으면 이 씬은 통째로 거절
        gated, n, _ = got
        # ⛔ 불변식 1: 탑 밖에서는 고르는 것이 원본과 **정확히 같아야** 한다.
        before, ok0 = selectable(tail, 0)
        after, ok1 = selectable(gated, 0)
        if not (ok0 and ok1) or before != after:
            return raw, None
        # ⛔ 불변식 2: 탑 안에서는 금지된 레코드를 **하나도** 고르면 안 된다.
        #    (금지되지 않은 것은 그대로 돈다 — 진입 스크립트가 그것이다.)
        inside, ok2 = selectable(gated, 1)
        if not ok2 or (set(inside) & bad):
            return raw, None
        newrow = row[:ROW_HEAD] + gated
        if len(newrow) > REC_MAX:
            return raw, None
        newA.append(newrow)
        touched += n
    if not touched:
        return raw, None
    return build_scn(head, [newA, B, C]), "%d block(s) gated" % touched


def targets(jar):
    """층으로 쓸 수 있는 씬: 몬스터가 있고, 걸어서 닿고, 레벨을 읽을 수 있는 것."""
    maps, scenes = G.load(jar)
    crea = G.creature_table(jar, True)
    stranded = G.unreachable_monsters(jar, maps, scenes)
    out = []
    for sid, sc in sorted(scenes.items()):
        if sid > 0xFF:
            continue                          # 워프 목적지는 1바이트
        if not G.species_of(sc, maps):
            continue
        if stranded.get(sid, True):
            continue
        if G.scene_level(sc, maps, crea) is None:
            continue
        out.append(sid)
    return out


def member(sid):
    return "res/event/e%02d%02d_scn" % (sid // 100, sid % 100)


def open_jar(zpath):
    z = zipfile.ZipFile(zpath)
    names = [n for n in z.namelist() if n.endswith(".jar")]
    if len(names) != 1:
        raise SystemExit("expected exactly one .jar in %s, found %d" % (zpath, len(names)))
    return z, names[0]


def plan(zpath):
    z, jarname = open_jar(zpath)
    jar = zipfile.ZipFile(io.BytesIO(z.read(jarname)))
    used = flag_ids_in_use(jar)
    if FLAG_TOWER in used:
        raise SystemExit(
            "REFUSING: flag id 0x%02x is already used by the game. 다른 id 를 고르고 "
            "wie_android/src/items.rs 의 GUARDIAN_FLAG_TOWER 와 함께 바꿔라." % FLAG_TOWER)
    todo, already, refused = {}, [], []
    for sid in targets(jar):
        name = member(sid)
        try:
            raw = jar.read(name)
        except KeyError:
            continue
        new, note = gate_scene(raw)
        if note is None:
            (already if _is_gated(raw) else refused).append(sid)
            continue
        todo[name] = new
    return z, jarname, jar, todo, already, refused


def _is_gated(raw):
    """탑 안(플래그 1)에서 이 씬이 **금지된 레코드를 하나도 고를 수 없나**.

    ⛔ 이것이 진짜 불변식이다 — "모든 행에 GATE 바이트가 있나"가 아니다. 게이트는 필요한
    블록에만 붙으므로 (진입 스크립트는 그대로 돌아야 한다), 붙은 자리를 세는 검사는
    금지 레코드가 없어서 아무것도 안 붙은 씬을 「안 걸렸다」고 오판한다.
    """
    try:
        _, (A, B, _C) = parse_scn(raw)
    except Exception:
        return False
    if not A:
        return False
    bad = forbidden_records(B)
    for row in A:
        if len(row) < ROW_HEAD + 1:
            continue
        inside, ok = selectable(row[ROW_HEAD:], 1)
        if not ok or (set(inside) & bad):
            return False
    return True


def tower_ready(raw):
    """층으로 쓸 수 있나 = 게이트가 걸렸고, **진입 때 페이드인이 확실히 돈다**.

    ⛔⛔ 두 번째 조건이 없으면 화면이 검게 나온다 (2026-09-11, 프로브 실측). 모든 씬의 진입 행은
    `fa ff 00 … 00` 으로 B[0] = `23 ff 01 33 00 08`(진입 표시 + 8프레임 페이드인)을 돌리는데,
    그것이 안 돌면 게임 로직은 다 도는데(몬스터 수도 맞다) 월드가 안 그려진다 — 씬 7 실측
    비검정 115,327 → 1,989, 씬 13/51 실측 118,349 → 2,569 (검은 세계 + HUD 만).
    ⛔⛔⛔ 코드리뷰 2026-09-11 정정 둘: ① 옛 판은 「아무 A행이나 무언가 고르면 준비됨」이라 kind 5/6
    트리거 행만 살아 있는 씬을 통과시켰고 — 그래서 vc293 층 표의 12개 층(씬 13·14·15·51·53·55·57·
    59·63, 그리고 페이드인이 게이트된 158)이 검은 화면이었다. ② 옛 워커가 중첩 컨테이너 뒤의 꼬리
    payload 를 못 봐서 씬 9/121 의 B[0] 는 **우연히** 살아남았다. 지금 판정 = VM 모델
    (`gen_guardian_floors.evaluate`)로 kind-0 행을 돌려 페이드인 레코드가 **CERTAIN**(어떤 게임
    플래그 상태에서도 돈다)일 때만 층이다. 게임 플래그 뒤의 else 로만 페이드인하는 씬(31 류)은 그
    플레이어의 진행도에 따라 검을 수 있으므로 **fail closed** 로 뺀다.
    """
    if not _is_gated(raw):
        return False
    try:
        _, (A, B, _C) = parse_scn(raw)
    except Exception:
        return False
    fade = G.fade_in_records(B)
    if not fade:
        return False
    for row in A:
        if len(row) < ROW_HEAD + 1 or row[ROW_KIND] != KIND_ENTRY:
            continue
        got = G.evaluate(bytes(row[ROW_HEAD:]), _decide_entry)
        if got is None:
            continue
        reach, _ran = got
        if any(reach.get(k, G.NEVER) == G.CERTAIN for k in fade):
            return True
    return False


def rewrite(zpath, apply):
    z, jarname, jar, todo, already, refused = plan(zpath)
    print("tower gate: flag 0x%02x, %d scene(s) to patch, %d already gated, %d refused%s"
          % (FLAG_TOWER, len(todo), len(already), len(refused),
             (" " + str(refused[:12])) if refused else ""))
    if not todo:
        return 0
    if not apply:
        print("(dry run - pass --apply to write)")
        return 0

    backup = os.path.join("android_app/asset_backup", os.path.basename(zpath) + ".pretower")
    if not os.path.exists(backup):
        os.makedirs(os.path.dirname(backup), exist_ok=True)
        shutil.copyfile(zpath, backup)
        print("backup -> %s" % backup)

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as out:
        for it in jar.infolist():
            out.writestr(it, todo.get(it.filename) or jar.read(it.filename))
    newjar = buf.getvalue()

    tmp = zpath + ".new"
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as out:
        for it in z.infolist():
            out.writestr(it, newjar if it.filename == jarname else z.read(it.filename))
    z.close()
    os.replace(tmp, zpath)

    z2, jn2 = open_jar(zpath)
    back = zipfile.ZipFile(io.BytesIO(z2.read(jn2)))
    for name, want in todo.items():
        if back.read(name) != want:
            raise SystemExit("write-back verification FAILED for %s" % name)
    print("%s: %d scene(s) written and verified" % (zpath, len(todo)))
    return 0


def check(zpath, verbose):
    z, jarname = open_jar(zpath)
    jar = zipfile.ZipFile(io.BytesIO(z.read(jarname)))
    bad, ungatable = [], []
    n = 0
    tgt = targets(jar)                       # 한 번만 — 아래 -v 출력도 이걸 쓴다
    for sid in tgt:
        try:
            raw = jar.read(member(sid))
        except KeyError:
            continue
        n += 1
        if not _is_gated(raw):
            # 심을 수 없는 씬은 오류가 아니라 **층에서 빠지는** 씬이다. 그것을 층 표에서
            # 빼는 것은 `gen_guardian_floors.py` 의 게이트 조건이 하고, 여기서는 "심을 수
            # 있었는데 안 심겼다"만 실패로 본다.
            if gate_scene(raw)[1] is None:
                ungatable.append(sid)
            else:
                bad.append(sid)
    if bad:
        print("tower gate: NOT APPLIED to %d of %d floor scene(s): %s"
              % (len(bad), n, bad[:20]))
        return 1
    print("tower gate: %d of %d floor scene(s) gated on flag 0x%02x (%d cannot be gated and are "
          "therefore not floors%s)"
          % (n - len(ungatable), n, FLAG_TOWER, len(ungatable),
             (": " + str(ungatable[:12])) if ungatable else ""))
    if verbose:
        print("  scenes:", tgt)
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("zip", nargs="?", default="android_app/app/src/main/assets/hero3_fixed.zip")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    a = ap.parse_args(argv)
    if a.check:
        return check(a.zip, a.verbose)
    return rewrite(a.zip, a.apply)


if __name__ == "__main__":
    raise SystemExit(main())
