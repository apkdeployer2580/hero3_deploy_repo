"""하늘성 입장구를 **기존 NPC 둘**에게 심는다 — 리츠는 엔자크(씬 111)의 행인, 케이는 네오 솔티아(씬 5)의
로빈(113차, 2026-09-19). 하드코어에서만 대화가 바뀌고 「예」가 입장이다.

⛔⛔ 이 도구는 2026-09-10 에 통째로 다시 쓰였다. 옛 판은 **새 NPC 를 하나 추가**했고
(수도사, 타일 18,12), 사용자가 원한 것은 그것이 아니었다:

    *"내가원한건 NPC가 추가되는게 아니라 이미 존재하는 NPC에게 대화를 걸면
      하드코어일때만 대화가 달라지면서 대화 종료 후 하늘성으로 들어갈 수있게"*

정본 = `GUARDIAN_CASTLE_FEASIBILITY.md §J`. 그 절이 스크립트 VM 을 전수 해독했고, 이 도구는
그 결과 위에 서 있다. 요지만 옮기면:

★★ **A 레코드의 payload 는 조건 컨테이너일 수 있다** (§J.3). 게임 자신이 kind 5 행 30곳에서 쓴다:

    payload_spec := <byte>                        단순 — B 인덱스 하나
                  | ff <n> <len>[n]  block x n    조건 컨테이너
    block        := (cond)*  <payload byte>       조건은 AND, 마지막 바이트가 B 인덱스

  순서대로 평가하고 **첫 매치가 이긴다** — 코퍼스 346개 컨테이너에서 「무조건 블록이 마지막이
  아닌 경우」가 **0건**인 것이 그 근거다.

★★ **그 세 가지를 가르는 것은 인터프리터 `0x52148` 하나다** (2026-09-11 실측): 스트림에서
  바이트 하나를 i8 로 읽어 ⓐ `0 <= v < count` 면 **그 인덱스의 레코드를 실행**하고(누적 오프셋
  표 `+8`, 개수 `+0x10`), ⓑ `v == -1`(`0xff`) 이면 위의 컨테이너를 돌리고(첫 매치가 이긴다 —
  `0x52228` 루프가 블록마다 `0x52148(.., 1)` 을 부르고 nonzero 에서 멈춘다), ⓒ `-6 <= v <= -2`
  이면 **조건**으로 보고 `0x3beac(-2 - v, stream)` 를 부른다. 그래서 조건 옵코드는 `fa`(index 4)
  ~ `fe`(index 0) 다섯이고, 점프 표는 `0x5d890` 이다.
  ⛔ **`fa` 만이 영구 플래그다** — index 4 = `0x3bff2` 가 `getFlag`(`0x3f571`)를 부르고
  `getFlag(op[0]) == op[1]` 을 비교한다(`[sp+8]+0x56` 이 곧 `op+1` 이다: 두 리터럴이
  `base+0xa0a8` 과 `base+0xa0fd`, 차 `0x55`). `fb` 는 index 3 = `0x3f391` 짜리 **다른 표**이고
  (코퍼스 id 0x00..0xb1 로 `0x23` 이 쓰는 집합과 안 겹친다), `fc` 는 소지금, `fe` 는 아이템
  소지(오퍼랜드 **3**개)다.

★★ **`fa <id> <0|1>` = 영구 플래그 테스트** (§J.2). 게터 `0x3f570` / 세터 `0x3f590` 이
`base+0x9e98 + id` 를 읽고 쓰며, 그 256바이트가 **세이브에 그대로 실린다**
(`deserialiseSave 0x1aa84` / `serialiseSave 0x1affc` 의 `0x100` memcpy). 스크립트가 쓰는 쪽은
**`op 0x23 <id> <value>`** 다. ⇒ 「한번이라도 들었나」는 **호스트 없이** 게임 자신이 기억한다.

⛔ **난이도를 읽는 조건은 없다** (§J.3, 다섯 팔 전수). 그래서 `HARD` 만은 호스트가 난이도
바이트(`base+0xa318`)를 읽어 빈 flag id 에 미러해 준다. 그 한 바이트를 빼면 전부 게임 데이터다.

⛔ **kind 를 바꾸지 말 것.** 우리가 손대는 것은 payload 하나이고, 행인은 원래부터
**kind 4**(스프라이트 있는 말 걸 수 있는 NPC)다. 컨테이너가 kind 4 에서 도는 것은 게임
자신이 증명한다 — 코퍼스에 **kind 4 + 컨테이너가 180행**(그중 스프라이트 있는 것 105행)이고,
`fa` 조건 둘짜리 7바이트 블록(우리와 바이트 모양이 같은 것)이 **44개** 있다(예: `e0030_scn`
A[3] `fa 07 00 fa 33 00 0a`).

⛔ **추가는 언제나 리스트 끝에** — C/B 인덱스는 스크립트가 절대값으로 참조한다. A[10] 은
**제자리 수정**이라(길이만 늘어난다) 인덱스가 하나도 안 움직인다. 도구가 그것을 검사한다.

usage:
    python tools_re/patch_guardian_npc.py <asset.zip> --check    # 적용됐나 (미적용이면 exit 1)
    python tools_re/patch_guardian_npc.py <asset.zip> --apply
    python tools_re/patch_guardian_npc.py <asset.zip> --verify   # 심은 것을 사람이 읽게 출력
"""
import argparse
import io
import os
import shutil
import sys
import zipfile

DEFAULT_ZIP = "android_app/app/src/main/assets/hero3_fixed.zip"

# ── 대상 둘 (113차, 2026-09-19) ──────────────────────────────────────────────
#
# ★ 리츠는 사막마을 엔자크(씬 111)의 행인, 케이는 네오 솔티아(씬 5)의 **로빈**이다 — 사용자
#   결정: *"케이일때는 네오솔티아의 로빈 이라는 npc로 하드코어일때만 대화가 바뀌며 하늘성에
#   입장 가능하게"*. 두 대상 모두 **기존 NPC 의 A 행 payload 만** 바꾼다(kind 불변).
#
# ★★ 로빈의 A 행은 행인과 달리 이미 **조건 컨테이너**다(`ff 02 0c 0f …`, 스토리 플래그
#   `fb 9e/9f` 로 갈린다 — 그의 대사는 장이 진행되면 바뀐다). 그래서 하드 블록 둘을 그 컨테이너의
#   **맨 앞**에 넣는다: 첫 매치가 이기므로 하드에서는 진행도와 무관하게 탑 대화가 나오고, 노말
#   (HARD=0)에서는 두 블록이 모두 떨어져 원본 블록들이 그대로 평가된다.
# ★ 세 flag id 는 두 대상이 **공유**한다(한 캐릭터는 한 직업이라 충돌이 없고, HEARD 는 「설명을
#   한 번 들었다」는 뜻이라 어느 NPC 에게 들었든 같다).
#
# 색 코드는 원본 규약을 따른다: `|` = 주인공 이름색, `$` = NPC 이름색, `@` = 흰 본문 (§2.21).
# ★ 두 대사의 전 글자가 KS X 1001 안에 있음을 확인했다 — 게임 폰트는 그 밖을 **조용히** 버린다.
RITZ = "|[리츠]@ "
PASSERBY = "$[행인]@ "
KEI = "|[케이]@ "
ROBIN = "$[로빈]@ "
YES = "예"
NO = "아니오"

# (사용자 문구 그대로, 2026-09-10)
PASSERBY_LINES = [
    PASSERBY + ".. (무섭다...)혹시 하늘성에 대해 알고 계시나요?",
    RITZ + "들어본적 없는데.",
    PASSERBY + "과거 에레보스 대륙이 멸망하고 솔티아가 하늘로 올라갔을 때 어떤 종교집단이 "
               "그 솔티아로 오르기 위한 하늘성이라는 탑을 지었다고 해요.",
    PASSERBY + "제가 예전에 신드라프에서 잠시 생활했을 때 이 목걸이를 전재산을 주고 샀는데 "
               "얼마전에 놀라운 걸 발견했어요!",
    RITZ + "별거 없어보이는데.",
    PASSERBY + "이 목걸이를 손에 꽉쥐고 눈을 감으면 그 하늘성으로 의식을 잠시 이동할 수있어요. "
               "이미 몬스터들로 가득차서 전 아무것도 못하고 죽었지만...",
    PASSERBY + "실제로 죽는건 아니니까 걱정안해도 돼요! 아마 탑을 오를수록 몬스터들이 "
               "가지고있는 전리품의 가치도 올라가겠죠?",
    PASSERBY + "한번 해보시겠어요?",
]
# (사용자 문구 그대로, 2026-09-19 — 「살떄」는 오타로 보고 「살때」)
ROBIN_LINES = [
    ROBIN + "하늘성. 들어봤어? 옛날 에레보스 대륙이 멸망하고 솔티아가 하늘로 올라왔을 때, "
            "어떤 종교집단이 이 솔티아로 오르려고 지었다는 탑이야.",
    KEI + "그런 게 있을리가...",
    ROBIN + "있어! 예전에 실바나르에서 잠깐 살때 이 목걸이를 샀는데, 이걸 꽉 쥐고 눈을 감으면 "
            "의식만 잠깐 그 탑으로 갈 수 있어.",
    KEI + "(...그냥 꿈꾼거 아냐?)",
    ROBIN + "근데 몬스터가 바글바글해서 난 아무것도 못 하고 죽었지만... 진짜 죽는 건 아니니까 걱정 마!",
    ROBIN + "위로 올라갈수록 몬스터들이 들고 있는 물건도 굉장해질 거야. 대장장이도 못 만드는 물건이 있을지도 몰라.",
    ROBIN + "케이, 네가 한번 가 볼래?",
]

TARGETS = [
    dict(
        name="엔자크 행인 (리츠)",
        # 마을 씬. 사막마을 엔자크 = 맵 60 ENZARK 35x36, intro 3.
        scene=111,
        # 손댈 A 행 = **A[10]** — 제보의 그 행인이다.
        #
        # ⛔⛔ 2026-09-11 정정: 옛 판은 **A[1] 을 고쳤고 그것은 행인이 아니었다.** A[1] 은
        #    `1a 00 00 ff 00 05 08` = 타일 (26,0) · **스프라이트 없음**(`ff`) · payload 8 →
        #    `B[8] = 28 77 1f 1b 00` 이고, `28` 은 **워프**다(A[3..7] 도 전부 스프라이트 없음 +
        #    `28 7x ..` = 마을 건물 문이다). 즉 A[1] 은 **맵 위쪽 끝의 출구 타일**이었다.
        # ★ 행인을 특정하는 근거 둘 (둘 다 코퍼스 실측, `--verify` 가 다시 찍는다):
        #    ① `C[8]`(`|[리츠]@ ... 뭘 보나.`)과 `C[9]`(`$[행인]@ .. (무섭다...)`)를 **둘 다 찍는
        #       B 레코드는 `B[16]` 하나뿐**이고(`36 08` + `34 09`), 그 B[16] 을 가리키는 A 행은
        #       **A[10] 하나뿐**이다.
        #    ② A[10] 은 스프라이트 `0x2a` 를 갖고 kind 4 다 — 말을 걸 수 있는 NPC 의 모양이고
        #       (같은 씬의 A[8]·A[9]·A[11..14] 가 모두 그렇다), 타일 (32,19)는 실기기 스크린샷의
        #       화면 오른쪽에 서 있는 그 NPC 위치와 맞는다.
        row=10,
        header=bytes([0x20, 0x13, 0x05, 0x2A, 0x00, 0x04]),   # x=32 y=19 · 스프라이트 0x2a · kind 4
        original_payload=bytes([0x10]),                         # 단순 payload: B[16]
        # 행인의 여는 대사 = C[8] `|[리츠]@ ... 뭘 보나.` 를 우리 긴 스크립트에서 재사용한다.
        # ⛔ 이것을 original_payload 로 쓰지 말 것 — 그것은 **B 인덱스**라 `34 <그것>` 은
        #    엉뚱한 C 레코드를 찍는다(2026-09-11 에 실제로 한 번 그렇게 나갔다).
        opening_c=0x08,
        lines=PASSERBY_LINES,
        again=PASSERBY + "하늘성으로 다시 들어가시겠어요?",
    ),
    dict(
        name="네오 솔티아 로빈 (케이)",
        # 케이의 마을. `e0005_scn`: |A|=5 |B|=22 |C|=87, intro 없음(헤더 [8] = 0xff).
        scene=5,
        # 로빈 = **A[3]** (타일 8,8 · 스프라이트 0x26 · kind 4). 근거: 원본 대사 세 줄
        # C[39] `난 전쟁따윈 안해…` · C[40] `그거 알아? 대장장이들도…` · C[41] `그 조합서를 모아…` 를
        # 찍는 B 레코드는 `B[12]`(`34 27 34 28 34 29`) 하나뿐이고, B[12] 를 payload 로 갖는 블록은
        # A[3] 의 컨테이너 안(`fb 9f 00 0c`)에만 있다.
        row=3,
        header=bytes([0x08, 0x08, 0x9E, 0x26, 0x01, 0x04]),
        # 이미 컨테이너다: `ff 02 <12> <15>` + 블록 둘 (`fb 9e 00` 조건 → 중첩 컨테이너, 그 밖).
        original_payload=bytes.fromhex(
            "ff 02 0c 0f"
            "fb 9e 00 ff 02 04 01 fb 9f 00 0c 10"
            "ff 03 04 05 01 fb 9f 00 0d fe 11 0a 01 0f 0e"),
        # 여는 대사 = 원본 C[39] `$[로빈]@ 난 전쟁따윈 안해. 모험가가 되어 온 세상을 돌거야.`
        opening_c=39,
        lines=ROBIN_LINES,
        again=ROBIN + "하늘성으로 다시 가 볼래?",
    ),
]

# 하위 호환 별칭(다른 도구·문서가 이름으로 참조한다).
SCENE = TARGETS[0]["scene"]
SCN_MEMBER = "res/event/e%02d%02d_scn" % (SCENE // 100, SCENE % 100)
NPC_ROW = TARGETS[0]["row"]
NPC_HEADER = TARGETS[0]["header"]
NPC_ORIGINAL_PAYLOAD = TARGETS[0]["original_payload"][0]
OPENING_LINE_C = TARGETS[0]["opening_c"]
LINES = PASSERBY_LINES
AGAIN = TARGETS[0]["again"]

# ── 영구 플래그 id ────────────────────────────────────────────────────────────
# §J.2: 스크립트가 `fa` 로 읽거나 `op 0x23` 으로 쓰는 id 를 전수로 모으면 105개이고, 남는 가장
# 긴 연속 구간이 **0x3d..0xc8** 이다. 그 구간이 정확히 0x3d 에서 시작하는 것이 코드와 맞물린다 —
# `0x46a00` 의 루프가 지우는 범위가 **0x33..0x3c** 다(데이터 경계와 코드 경계가 한 칸도 안 어긋난다).
# ⛔ 세 id 는 `guardian_flag_ids_are_free` 셀프테스트가 코퍼스 전수로 못 박는다.
FLAG_HARD = 0xC0    # 호스트가 난이도를 미러한다 (게임은 절대 안 만진다)
FLAG_HEARD = 0xC1   # 하드에서 설명을 한 번 들었다 — 스크립트가 세우고 세이브가 나른다
FLAG_ENTER = 0xC2   # 「예」를 골랐다 — 호스트가 폴링하고 소비 후 0 으로 되돌린다
# (0xC3 = 탑 봉인 — `patch_tower_gate.py` 의 것. 여기서는 안 쓴다.)
FLAG_WAKE = 0xC4    # ★ 143차: 100층 완주로 돌아왔다 — 호스트가 복귀 워프 직전에 세우고, 입구 마을의
                    #   진입 스크립트가 「의식이 몽롱하다...」를 말하며 스스로 0 으로 되돌린다
FLAG_IDS = (FLAG_HARD, FLAG_HEARD, FLAG_ENTER, FLAG_WAKE)

# ── 옵코드 (§J.1) ────────────────────────────────────────────────────────────
OP_SAY = 0x34       # 34 <C index>          대사 출력 (0x3c77c)
OP_SETFLAG = 0x23   # 23 <id> <value>       setFlag  (0x3f590)
OP_WAIT = 0x31      # 31 <i8 프레임>        스테퍼 대기 (0x3de5a: ldrsb → [r7+0x50]) — 컷신의 `19 06 31 0a 19 00`
OP_CHOICE = 0xFF    # ff <p> <n> (label delta) x n  + 분기 본문
CHOICE_P = 0x00     # 실측: 진짜 선택지 사이트는 전부 p=0
COND_FLAG = 0xFA    # fa <id> <0|1>         영구 플래그 테스트

# ── 143차: 100층 완주 뒤의 한 줄 (사용자 문구 그대로, 2026-09-30) ─────────────
#
# 사용자: *"화면이 그라데이션으로 점점 하얀색으로 … 다시 그라데이션으로 기존(하늘성에 입장하기 전)
# 상태로 돌아오고서 대화창으로 "의식이 몽롱하다..." 라고 뜨게"*. 흰 화면은 호스트가 그리고
# (`overlay::draw_wake_veil`), 대사는 **게임 자신의 대화창**이 말한다 — 입구 마을 둘(111·5)의
# **진입 행 A[0]** 이 로드마다 도는 블록(`fa ff 00 …`) 꼬리에 중첩 컨테이너 하나를 붙인다:
#
#     … <원본 블록> ff 01 04 | fa c4 01 <B_wake>        FLAG_WAKE 가 1 일 때만
#     B_wake = 23 c4 00 | 31 WAKE_WAIT | 34 <C_wake>     스스로 지우고 · 기다리고 · 말한다
#
# ★ 진입 행은 **로드 때 한 번만** 평가된다(프로브 실측 2026-09-30: 로드 뒤 0xff 를 0 으로 되돌려도
#   B[0] 의 페이드가 다시 안 돈다) ⇒ 호스트는 복귀 워프를 **쏘기 전에** FLAG_WAKE 를 세워야 한다.
# ★ 대기가 있는 이유: 호스트의 흰 막이 걷히는 동안 대사가 이미 떠 있으면 「돌아오고서 뜨게」가
#   아니다. 프로브 실측(2026-09-30, 씬 111): 대기 없음 = 로드 뒤 35프레임에 창이 뜨고,
#   `31 0a` = +20프레임, `31 7f 31 2d` = +344프레임 ⇒ **한 단위 = 게임 2프레임(≈ 40 ms)**.
#   오퍼랜드는 i8(`ldrsb`)이라 한 번에 127 단위까지.
#   70 단위 ≈ 2.8 초 + 로드 뒤 0.7 초 = 3.5 초 — 호스트의 막(착지 + 0.7 초 유지 + 1.8 초 걷힘)이
#   다 걷힌 뒤다. 게임이 느린 기기에서는 대사가 더 늦게 뜰 뿐, 먼저 뜨지는 않는다.
# ★ 화자 없는 줄이다(게임의 서술 줄 `장로님도, 레아님도 없는;…` 과 같은 모양) — 리츠/케이 둘 다에게
#   맞고, 사용자 문구에 화자가 없다. 프로브 프레임으로 게임 대화창에 그대로 뜨는 것을 확인했다.
# ⛔ 행 순서·길이 규칙은 NPC 패치와 같다: 추가는 리스트 끝에만, A[0] 은 제자리에서 **길어지기만**.
WAKE_LINE = "의식이 몽롱하다..."
WAKE_WAIT = (70,)          # 스크립트 대기 단위 (한 단위 = 게임 2프레임)
ENTRY_ROW = 0
ENTRY_HEADER = bytes([0x00, 0x00, 0x00, 0xFF, 0x00, 0x00])  # x=0 y=0 · 스프라이트 없음 · kind 0

# ── 대사는 위 TARGETS 에 있다 (113차부터 두 벌) ──────────────────────────



def parse_scn(d):
    """헤더 9바이트 + 리스트 셋. 남는 바이트가 있으면 모델이 틀린 것이므로 던진다."""
    o = 9
    lists = []
    for _ in range(3):
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
        lists.append(recs)
    if o != len(d):
        raise ValueError("scn does not end at EOF (%d of %d)" % (o, len(d)))
    return d[:9], lists


def build_scn(head, lists):
    out = bytearray(head)
    for recs in lists:
        if len(recs) > 255:
            raise ValueError("list count byte is u8")
        out.append(len(recs))
        for r in recs:
            if len(r) > 255:
                raise ValueError("record length byte is u8")
            out.append(len(r))
        for r in recs:
            out += r
    return bytes(out)


def string_record(s):
    return s.encode("euc-kr") + b"\x00"


def scn_member(scene):
    return "res/event/e%02d%02d_scn" % (scene // 100, scene % 100)


def split_container(payload):
    """A payload spec as a list of blocks. A simple one-byte payload is one block."""
    if payload[0] != OP_CHOICE:
        if len(payload) != 1:
            raise ValueError("a simple payload is one byte, got %s" % payload.hex(" "))
        return [bytes(payload)]
    n = payload[1]
    lens = list(payload[2:2 + n])
    o = 2 + n
    blocks = []
    for L in lens:
        blocks.append(bytes(payload[o:o + L]))
        o += L
    if o != len(payload):
        raise ValueError("the container does not consume the payload exactly")
    return blocks


def join_container(blocks):
    if len(blocks) > 255 or any(len(b) > 255 for b in blocks):
        raise ValueError("container counts/lengths are u8")
    return bytes([OP_CHOICE, len(blocks)] + [len(b) for b in blocks]) + b"".join(blocks)


def already_applied(t, lists):
    return string_record(t["again"]) in lists[2]


def _u8(name, v):
    if not 0 <= v <= 0xFF:
        raise ValueError("%s no longer fits in one byte: %d" % (name, v))
    return v


def patched(t, d):
    """(새 scn 바이트, 사람이 읽을 요약) for one target. 이미 적용돼 있으면 (원본, None)."""
    head, lists = parse_scn(d)
    A, B, C = [list(x) for x in lists]
    if already_applied(t, lists):
        return d, None

    row = A[t["row"]]
    if row[:6] != t["header"] or bytes(row[6:]) != t["original_payload"]:
        raise ValueError(
            "%s: A[%d] is not the pristine row (got %s, want %s %s).\n"
            "  이 도구는 pristine 씬 %d 를 요구한다 — `patch_asset.py --apply` 로 에셋을 다시\n"
            "  만든 뒤(그것이 .orig 에서 재빌드한다) 도구 순서대로 다시 돌려라."
            % (t["name"], t["row"], row.hex(" "), t["header"].hex(" "), t["original_payload"].hex(" "), t["scene"]))

    # ── C 추가 (리스트 끝에만) ────────────────────────────────────────────────
    c0 = len(C)
    C += [string_record(x) for x in t["lines"]]           # c0 .. c0+len-1
    i_yes = _u8("yes row", c0 + len(t["lines"]))
    i_no = _u8("no row", i_yes + 1)
    i_again = _u8("again row", i_no + 1)
    C += [string_record(YES), string_record(NO), string_record(t["again"])]

    # ── B 추가: 하드 첫 대화 / 하드 재방문 ────────────────────────────────────
    # 「예」 분기만 ENTER 를 세운다. ⛔ 분기 밖에 두면 아니오를 골라도 입장한다.
    yes_body = bytes([OP_SETFLAG, FLAG_ENTER, 1])
    choice = bytes([OP_CHOICE, CHOICE_P, 2, i_yes, len(yes_body), i_no, 0]) + yes_body

    long_body = bytearray()
    long_body += bytes([OP_SAY, t["opening_c"]])             # 원본 여는 대사를 재사용
    for k in range(len(t["lines"])):
        long_body += bytes([OP_SAY, _u8("line", c0 + k)])
    long_body += bytes([OP_SETFLAG, FLAG_HEARD, 1])          # 들었다고 기록 (세이브에 남는다)
    long_body += choice

    short_body = bytes([OP_SAY, i_again]) + choice

    i_long = _u8("long script", len(B))
    B.append(bytes(long_body))
    i_short = _u8("short script", len(B))
    B.append(short_body)

    # ── A[row] 제자리 수정: 하드 블록 둘을 컨테이너 **맨 앞**에 ────────────────
    # 단순 payload 는 무조건 블록 하나짜리 컨테이너가 되고, 이미 컨테이너인 행(로빈)은 원본
    # 블록들이 그대로 뒤따른다 — 첫 매치가 이기므로 노말에서는 원본이 그대로 평가된다.
    blk_first = bytes([COND_FLAG, FLAG_HARD, 1, COND_FLAG, FLAG_HEARD, 0, i_long])
    blk_again = bytes([COND_FLAG, FLAG_HARD, 1, COND_FLAG, FLAG_HEARD, 1, i_short])
    spec = join_container([blk_first, blk_again] + split_container(t["original_payload"]))
    A[t["row"]] = t["header"] + spec

    new = build_scn(head, [A, B, C])

    # ── verify-then-write ────────────────────────────────────────────────────
    # 새 파일을 우리 파서로 되읽어 ① EOF 에서 끝나고 ② A[row] 말고는 **전부 바이트 동일**이며
    # ③ 기존 인덱스가 하나도 안 움직였는지 확인한다.
    _, back = parse_scn(new)
    old_A, old_B, old_C = lists
    for i, r in enumerate(old_A):
        if i == t["row"]:
            continue
        if back[0][i] != r:
            raise ValueError("A[%d] moved or changed" % i)
    if back[0][t["row"]][:6] != t["header"]:
        raise ValueError("the NPC row header changed")
    if split_container(back[0][t["row"]][6:])[2:] != split_container(t["original_payload"]):
        raise ValueError("the original blocks did not survive behind ours")
    for name, old, cur in (("B", old_B, back[1]), ("C", old_C, back[2])):
        if cur[:len(old)] != list(old):
            raise ValueError("list %s: an existing record moved" % name)
    if back[1][i_long] != bytes(long_body) or back[1][i_short] != short_body:
        raise ValueError("appended scripts did not round-trip")

    note = (
        "%s: scene %d - 기존 NPC A[%d] (타일 %d,%d, kind 4) 의 payload 앞에 하드 블록 둘\n"
        "  하드 & 처음   -> B[%d]  (원본 C[%d] + 새 %d줄 + setFlag 0x%02x + 예/아니오)\n"
        "  하드 & 들었음 -> B[%d]  (C[%d] + 예/아니오)\n"
        "  그 외(노말)   -> 원본 블록 %d개 그대로\n"
        "  flag id: HARD 0x%02x (호스트가 미러) · HEARD 0x%02x · ENTER 0x%02x\n"
        "  A %d행(길이 %d->%d) · B %d->%d · C %d->%d · %d -> %d bytes"
        % (t["name"], t["scene"], t["row"], t["header"][0], t["header"][1],
           i_long, t["opening_c"], len(t["lines"]), FLAG_HEARD,
           i_short, i_again, len(split_container(t["original_payload"])),
           FLAG_HARD, FLAG_HEARD, FLAG_ENTER,
           len(A), len(row), len(A[t["row"]]), len(old_B), len(B), len(old_C), len(C),
           len(d), len(new)))
    return new, note


def wake_record(i_c):
    """B_wake: 스스로 지우고 · 기다리고 · 말한다. 지우기가 **먼저**다 — 대사 도중 세이브가 떠도
    그 플래그가 세이브에 1 로 실리지 않게."""
    body = bytearray([OP_SETFLAG, FLAG_WAKE, 0])
    for n in WAKE_WAIT:
        body += bytes([OP_WAIT, _u8("wake wait", n)])
    body += bytes([OP_SAY, _u8("wake line", i_c)])
    return bytes(body)


def wake_block(i_wake):
    """진입 블록 꼬리에 붙는 중첩 컨테이너: FLAG_WAKE 가 1 이면 B_wake."""
    return join_container([bytes([COND_FLAG, FLAG_WAKE, 1, _u8("wake script", i_wake)])])


def wake_applied(lists):
    return string_record(WAKE_LINE) in lists[2]


def _has_wake_tail(row):
    tail = wake_block(0)
    return len(row) >= len(tail) and row[-len(tail):-1] == tail[:-1]


def _is_wake_record(r):
    """B_wake 의 모양(인덱스만 다르다): `23 c4 00` + `31 n`… + `34 <c>`."""
    try:
        return r == wake_record(r[-1])
    except (IndexError, ValueError):
        return False


def patched_wake(t, d):
    """★ 143차: (새 scn 바이트, 요약) — 대상 씬의 진입 행에 「의식이 몽롱하다...」를 심는다.
    이미 있으면 (원본, None). NPC 패치와 독립이라 어느 쪽이 먼저여도 된다."""
    head, lists = parse_scn(d)
    A, B, C = [list(x) for x in lists]
    if wake_applied(lists):
        return d, None
    row = A[ENTRY_ROW]
    if row[:6] != ENTRY_HEADER:
        raise ValueError("%s: A[%d] is not the entry row (got %s)" % (t["name"], ENTRY_ROW, row.hex(" ")))
    blocks = split_container(row[6:])
    # 진입 행은 블록 하나이고 그 블록은 「이 로드에서 아직 안 돌았다」(`fa ff 00`)로 시작한다(§L.2).
    # ⛔ 그 모양이 아니면 거절한다 — 우리 꼬리가 로드마다 한 번이 아니라 엉뚱한 때 돌게 된다.
    if len(blocks) != 1 or blocks[0][:3] != bytes([COND_FLAG, 0xFF, 0x00]):
        raise ValueError("%s: A[%d] is not a one-block `fa ff 00` entry row (%s)" % (t["name"], ENTRY_ROW, row.hex(" ")))
    i_c = _u8("wake line", len(C))
    C.append(string_record(WAKE_LINE))
    i_wake = _u8("wake script", len(B))
    # ⛔ B 인덱스가 조건/컨테이너 바이트(0xfa..0xff)와 겹치면 실행기가 레코드가 아니라 조건으로 읽는다.
    if i_wake >= 0xFA:
        raise ValueError("B index 0x%02x would read as a condition byte" % i_wake)
    B.append(wake_record(i_c))
    A[ENTRY_ROW] = ENTRY_HEADER + join_container([blocks[0] + wake_block(i_wake)])
    new = build_scn(head, [A, B, C])

    # verify-then-write: 진입 행 말고는 전부 바이트 동일, 원본 블록이 앞에 그대로, 인덱스 무변동.
    _, back = parse_scn(new)
    old_A, old_B, old_C = lists
    for i, r in enumerate(old_A):
        if i != ENTRY_ROW and back[0][i] != r:
            raise ValueError("A[%d] moved or changed" % i)
    nb = split_container(back[0][ENTRY_ROW][6:])
    if len(nb) != 1 or not nb[0].startswith(blocks[0]) or nb[0][len(blocks[0]):] != wake_block(i_wake):
        raise ValueError("the entry block did not survive in front of ours")
    for name, old, cur in (("B", old_B, back[1]), ("C", old_C, back[2])):
        if cur[:len(old)] != list(old):
            raise ValueError("list %s: an existing record moved" % name)
    if back[1][i_wake] != wake_record(i_c) or back[2][i_c] != string_record(WAKE_LINE):
        raise ValueError("the wake script did not round-trip")
    note = (
        "%s: scene %d - 진입 행 A[%d] 꼬리에 `fa %02x 01 -> B[%d]` (%s, 대기 %d 단위 · C[%d])\n"
        "  A[%d] 길이 %d->%d · B %d->%d · C %d->%d · %d -> %d bytes"
        % (t["name"], t["scene"], ENTRY_ROW, FLAG_WAKE, i_wake, WAKE_LINE, sum(WAKE_WAIT), i_c,
           ENTRY_ROW, len(row), len(A[ENTRY_ROW]), len(old_B), len(B), len(old_C), len(C), len(d), len(new)))
    return new, note


# ── flag id 자유도 검사 (§J.2) ───────────────────────────────────────────────
# 게임이 쓰는 id 를 **전수로** 모은다: A 레코드의 `fa <id>` 와 B 스크립트의 `op 0x23 <id>`.
# ⛔ 이 검사가 이 도구의 진짜 안전장치다 — 남의 id 를 쓰면 퀘스트가 깨지고, 그 배열은
#    세이브가 나르므로 되돌릴 수도 없다.
# ★ 옵코드/조건 길이 표는 여기 없다 — `gen_guardian_floors.OPLEN` / `COND_LEN` 하나뿐이다
#   (코드리뷰 2026-09-11: 여기 있던 사본 둘은 아무도 안 읽었고 그중 `_ARGC[0xFC]` 는 워커들과
#   어긋나 있었다).
# `0x46a00` 의 루프가 지우는 고정 범위. 스크립트에는 안 나오지만 코드가 쓴다.
_CODE_CLEARED = range(0x33, 0x3D)


def flag_ids_in_use(jar):
    """게임이 읽거나 쓰는 영구 flag id 전부. 파싱이 안 되는 레코드는 **보수적으로** 훑는다."""
    used = set(_CODE_CLEARED)
    for name in jar.namelist():
        if not (name.startswith("res/event/") and name.endswith("_scn")):
            continue
        try:
            _, (A, B, C) = parse_scn(jar.read(name))
        except Exception:
            continue
        # ★ 113차: 한 대상이 이미 심긴 zip 에서 다른 대상을 심을 때, 우리 자신의 행/스크립트가
        #   「게임이 쓰는 id」로 읽히면 안 된다. 우리 것은 모양으로 안다 — 대상 행 자체, 그리고
        #   `23 c1 01`/`23 c2 01` 을 담은 B 레코드(이 검사가 pristine 코퍼스에서 먼저 통과했으므로
        #   게임 레코드에는 그 바이트열이 없다).
        ours_rows = {t["row"] for t in TARGETS if scn_member(t["scene"]) == name}
        ours_marks = (bytes([OP_SETFLAG, FLAG_HEARD, 1]), bytes([OP_SETFLAG, FLAG_ENTER, 1]))
        is_target = any(scn_member(t["scene"]) == name for t in TARGETS)
        for ai, r in enumerate(A):       # 어떤 0xfa 뒤의 바이트든 id 후보로 본다
            if ai in ours_rows and any(m in b for b in B for m in ours_marks):
                continue
            # ★ 143차: 우리 진입 꼬리(`ff 01 04 fa c4 01 <i>`)는 게임 것이 아니다 — 그 7바이트만 뗀다.
            if is_target and ai == ENTRY_ROW and _has_wake_tail(r):
                r = r[:-len(wake_block(0))]
            for i in range(len(r) - 1):
                if r[i] == 0xFA:
                    used.add(r[i + 1])
        for r in B:                      # 어떤 0x23 뒤의 바이트든
            if any(m in r for m in ours_marks) or (is_target and _is_wake_record(r)):
                continue
            for i in range(len(r) - 1):
                if r[i] == OP_SETFLAG:
                    used.add(r[i + 1])
    return used


def assert_ids_free(jar):
    used = flag_ids_in_use(jar)
    clash = [i for i in FLAG_IDS if i in used]
    if clash:
        raise SystemExit(
            "REFUSING: flag id(s) %s are already used by the game.\n"
            "  다른 id 를 고르고 `FLAG_*` 와 wie_android/src/items.rs 의 `GUARDIAN_FLAG_*` 를\n"
            "  **함께** 바꿔라. 지금 비어 있는 가장 긴 구간을 보려면 이 함수의 결과를 참고하라."
            % ["0x%02x" % i for i in clash])
    return used


def open_jar(zpath):
    z = zipfile.ZipFile(zpath)
    names = [n for n in z.namelist() if n.endswith(".jar")]
    if len(names) != 1:
        raise SystemExit("expected exactly one .jar in %s, found %d" % (zpath, len(names)))
    return z, names[0]


def rewrite(zpath, apply):
    z, jarname = open_jar(zpath)
    jar_bytes = z.read(jarname)
    jar = zipfile.ZipFile(io.BytesIO(jar_bytes))
    changes = {}
    for t in TARGETS:
        member = scn_member(t["scene"])
        # 두 패치는 서로 독립이다(NPC 행 · 진입 행), 그래서 빠진 쪽만 얹는다.
        new, note = patched(t, jar.read(member))
        if note is None:
            print("%s: scene %d already carries the 하늘성 entrance dialogue (%s)" % (zpath, t["scene"], t["name"]))
        new, wake_note = patched_wake(t, new)
        if wake_note is None:
            print("%s: scene %d already carries the 하늘성 wake line (%s)" % (zpath, t["scene"], t["name"]))
        notes = [n for n in (note, wake_note) if n]
        if notes:
            changes[member] = (new, "\n".join(notes))
    if not changes:
        return 0
    # ⛔ 게임이 쓰는 id 를 덮으면 퀘스트가 깨지고, 그 배열은 세이브가 나르므로 되돌릴 수 없다.
    used = assert_ids_free(jar)
    print("flag ids: %d in use across the corpus; ours (%s) are free"
          % (len(used), ", ".join("0x%02x" % i for i in FLAG_IDS)))
    for member, (_, note) in changes.items():
        print(note)
    if not apply:
        print("(dry run - pass --apply to write)")
        return 0

    # 백업은 asset_backup/ 으로 — ⛔ assets/ 안에 두면 gradle 이 APK 에 그대로 싣는다(71차-B).
    backup = os.path.join("android_app/asset_backup", os.path.basename(zpath) + ".prenpc")
    if not os.path.exists(backup):
        os.makedirs(os.path.dirname(backup), exist_ok=True)
        shutil.copyfile(zpath, backup)
        print("backup -> %s" % backup)

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as out:
        for it in jar.infolist():
            data = changes[it.filename][0] if it.filename in changes else jar.read(it.filename)
            out.writestr(it, data)
    newjar = buf.getvalue()

    tmp = zpath + ".new"
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as out:
        for it in z.infolist():
            data = newjar if it.filename == jarname else z.read(it.filename)
            out.writestr(it, data)
    z.close()
    os.replace(tmp, zpath)
    # 그리고 되읽어 확인한다.
    z2, jn2 = open_jar(zpath)
    j2 = zipfile.ZipFile(io.BytesIO(z2.read(jn2)))
    for member, (new, _) in changes.items():
        if j2.read(member) != new:
            raise SystemExit("write-back verification FAILED for %s" % member)
    print("%s: written and verified (%d scene(s))" % (zpath, len(changes)))
    return 0


def check_target(t, jar, zpath, verbose):
    head, lists = parse_scn(jar.read(scn_member(t["scene"])))
    A, B, C = lists
    if not already_applied(t, lists):
        print("CHECK FAILED: %s scene %d does not carry the 하늘성 entrance dialogue for %s (run --apply)"
              % (zpath, t["scene"], t["name"]))
        return 1
    row = A[t["row"]]
    if row[:6] != t["header"]:
        print("CHECK FAILED: %s: the NPC row header changed" % t["name"])
        return 1
    try:
        blocks = split_container(row[6:])
    except ValueError as e:
        print("CHECK FAILED: %s: %s" % (t["name"], e))
        return 1
    if len(blocks) < 3 or blocks[0][:6] != bytes([COND_FLAG, FLAG_HARD, 1, COND_FLAG, FLAG_HEARD, 0]) \
            or blocks[1][:6] != bytes([COND_FLAG, FLAG_HARD, 1, COND_FLAG, FLAG_HEARD, 1]):
        print("CHECK FAILED: %s: A[%d] does not start with the two hard blocks" % (t["name"], t["row"]))
        return 1
    if blocks[2:] != split_container(t["original_payload"]):
        print("CHECK FAILED: %s: the original blocks behind ours changed" % t["name"])
        return 1
    i_long, i_short = blocks[0][-1], blocks[1][-1]
    if i_long >= len(B) or i_short >= len(B):
        print("CHECK FAILED: %s: a branch points past the end of list B" % t["name"])
        return 1
    if bytes([OP_SETFLAG, FLAG_ENTER, 1]) not in B[i_long] or bytes([OP_SETFLAG, FLAG_ENTER, 1]) not in B[i_short]:
        print("CHECK FAILED: %s: a script never sets the ENTER flag" % t["name"])
        return 1
    print("CHECK OK: %s scene %d (%s): A[%d] -> B[%d]/B[%d] + %d original block(s)"
          % (zpath, t["scene"], t["name"], t["row"], i_long, i_short, len(blocks) - 2))
    # ★ 143차: 진입 행의 「의식이 몽롱하다...」.
    if not wake_applied(lists):
        print("CHECK FAILED: %s scene %d does not carry the 하늘성 wake line for %s (run --apply)"
              % (zpath, t["scene"], t["name"]))
        return 1
    entry = A[ENTRY_ROW]
    try:
        eb = split_container(entry[6:])
    except ValueError as e:
        print("CHECK FAILED: %s: entry row: %s" % (t["name"], e))
        return 1
    if entry[:6] != ENTRY_HEADER or len(eb) != 1 or eb[0][:3] != bytes([COND_FLAG, 0xFF, 0x00]) \
            or not _has_wake_tail(eb[0]):
        print("CHECK FAILED: %s: A[%d] does not end in the wake container" % (t["name"], ENTRY_ROW))
        return 1
    i_wake = eb[0][-1]
    i_c = list(C).index(string_record(WAKE_LINE))
    if i_wake >= len(B) or B[i_wake] != wake_record(i_c):
        print("CHECK FAILED: %s: the wake container does not point at the wake script" % t["name"])
        return 1
    print("CHECK OK: %s scene %d (%s): entry A[%d] -> fa %02x 01 -> B[%d] -> C[%d] %s"
          % (zpath, t["scene"], t["name"], ENTRY_ROW, FLAG_WAKE, i_wake, i_c, WAKE_LINE))
    if verbose:
        print("  A[%d] = %s" % (t["row"], row.hex(" ")))
        for i, b in enumerate(blocks):
            print("     block %d = %s" % (i, b.hex(" ")))
        for i in (i_long, i_short):
            print("  B[%2d] = %s" % (i, B[i].hex(" ")))
        for i, r in enumerate(C):
            if i == t["opening_c"] or i >= len(C) - (len(t["lines"]) + 4):
                print("  C[%2d] = %s" % (i, r.rstrip(b"\x00").decode("euc-kr")))
        print("  A[%d] = %s" % (ENTRY_ROW, entry.hex(" ")))
        print("  B[%2d] = %s" % (i_wake, B[i_wake].hex(" ")))
    return 0


def check(zpath, verbose):
    z, jarname = open_jar(zpath)
    jar = zipfile.ZipFile(io.BytesIO(z.read(jarname)))
    rc = 0
    for t in TARGETS:
        rc |= check_target(t, jar, zpath, verbose)
    return rc


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("zip", nargs="?", default=DEFAULT_ZIP)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--verify", action="store_true")
    a = ap.parse_args(argv)
    if a.check or a.verify:
        return check(a.zip, a.verify)
    return rewrite(a.zip, a.apply)


if __name__ == "__main__":
    sys.exit(main())
