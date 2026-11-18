#!/usr/bin/env python3
"""아바타 옷장 168차 — costume sets 04..07, drawn as MALE outfits from the shipped hero sheets.

    python tools_re/costume_factory/build.py --check           # rebuild in a temp dir, compare with costumes/
    python tools_re/costume_factory/build.py --out <dir>       # write the sets into <dir> (to look at them)
    python tools_re/costume_factory/build.py --force [04 ..]   # overwrite costumes/NN_* (all four if none named)
    python tools_re/costume_factory/preview.py costumes/07_pool_ritz <out.png>   # worn by a hero, many poses

⛔ 169차: these four sets were REDRAWN pixel by pixel by `tools_re/costume_atelier/` (the user: the 168차
recolours looked awkward). This script is kept as the record of 168차; `--check` now only says so and
`--force` refuses — it would overwrite the new art. `--out <dir>` still draws the old sets somewhere else.

★ The PNGs under `costumes/` are the source of truth (`costumes/README.md`); this script is the record
of how they were drawn. It never writes `costumes/` without `--force`, because once a frame is touched
up by hand a rerun would throw the edit away — `--check` is how to tell whether that has happened.
02 (이안) is the 167차 draft and is not built here. 01 and 03 are retired (set numbers are never reused).

Every input is a SHIPPED sheet in the asset zip (templates, heads, NPC sprites, the cif paper-doll
records) and never a costume sheet, so the output does not depend on itself.
"""
import argparse
import os
import shutil
import sys
import tempfile

import factory as F
import set_halloween as H
import set_kenneth as K
import set_pool as S

ROOT = F.AS.COSTUMES

HALLOWEEN = """# 의상 4 — 헬 엔딩 보상 (main 전용: 바닐라에는 헬이 없어 얻는 길이 없다).
# 168차 (사용자: 남자 캐릭터용으로 다시) — 원작 둥근 투구(h60012)의 실루엣 위에 그린 해골 가면 + 뼈 무늬 검은 옷.
# tools_re/gen_avatar_catalog.py 가 모든 의상의 이 파일을 wie_android/src/avatar_catalog.rs 로 모은다.
# 게임 글꼴이 그리는 글자만(KS X 1001 한글 · ASCII). 이름은 목록 칸 96 px, 설명은 정보 칸 143 px 까지.
set            = 할로윈-해골 가면
acquire        = clear.hell
helmet         = 할로윈-해골 가면
helmet.desc    = 섬뜩하게 웃는 해골 가면
armor          = 할로윈-해골 상의
armor.desc     = 갈비뼈가 그려진 검은 상의
gloves         = 할로윈-해골 장갑
gloves.desc    = 뼈 손이 달린 검은 장갑
shoes          = 할로윈-해골 하의
shoes.desc     = 다리뼈가 그려진 검은 하의
weapon         = 할로윈-뼈다귀
weapon.desc    = 무기처럼 휘두르는 뼈
weapon.spear   = 할로윈-뼈 창
weapon.sword   = 할로윈-뼈다귀
weapon.knife   = 할로윈-해골 손톱
weapon.gun     = 할로윈-뼈 권총
weapon.rifle   = 할로윈-뼈 대포
weapon.dark    = 할로윈-호박불
weapon.holy    = 할로윈-유령불
shield         = 할로윈-갈비뼈방패
shield.desc    = 갈비뼈로 엮은 방패
"""

KENNETH = """# 의상 5 — 하드코어 엔딩 보상. 168차: 게임 속 케네스 NPC(머리 npc0008 · 몸 npc1008 · 다리 npc2008 · 검 npc4008)의
# 붉은 뿔 가면과 연보라 강철 갑옷을 주인공 몸에 맞춰 옮겼다(가면은 눈 위치에 맞춰 얹고, 머리카락이 비치는 자리는 가면 뒤판으로 덮는다).
set            = 케네스
acquire        = clear.hard
helmet         = 케네스의 가면
helmet.desc    = 케네스가 쓰던 붉은 뿔 가면
armor          = 케네스의 갑옷
armor.desc     = 연보라빛 강철 판금 갑옷
gloves         = 케네스의 건틀릿
gloves.desc    = 붉은 손등의 강철 건틀릿
shoes          = 케네스의 각반
shoes.desc     = 강철 각반을 덧댄 바지
weapon         = 케네스의 무기
weapon.desc    = 케네스가 쓰던 무기
weapon.spear   = 케네스의 창
weapon.sword   = 케네스의 장검
weapon.knife   = 케네스의 손톱
weapon.gun     = 케네스의 단총
weapon.rifle   = 케네스의 대포
weapon.dark    = 케네스의 흑마법진
weapon.holy    = 케네스의 은마법진
shield         = 케네스의 방패
shield.desc    = 진홍 문장의 강철 방패
"""


def pool(hero):
    who = {'kei': '케이', 'ritz': '리츠'}[hero]
    glasses = {'kei': '파란 렌즈의 선글라스', 'ritz': '멋을 낸 검은 선글라스'}[hero]
    trunks = {'kei': '꽃무늬 파란 수영복과 샌들', 'ritz': '줄무늬 빨간 수영복과 샌들'}[hero]
    weapons = {
        'kei': ['weapon.spear   = 수영장 파라솔', 'weapon.sword   = 수영장 풍선 검', 'weapon.knife   = 수영장 물갈퀴',
                'weapon.dark    = 수영장 물보라', 'weapon.holy    = 수영장 비눗방울'],
        'ritz': ['weapon.spear   = 수영장 파라솔', 'weapon.sword   = 수영장 풍선 검', 'weapon.knife   = 수영장 물갈퀴',
                 'weapon.gun     = 수영장 물총', 'weapon.rifle   = 수영장 대형 물총'],
    }[hero]
    n = {'kei': 6, 'ritz': 7}[hero]
    return f"""# 의상 {n} — 기본 지급 · {who} 전용 (hero = {hero}: 다른 주인공의 옷장에는 안 나온다).
# 168차: 선글라스(그 주인공의 눈 위치에 맞춤) · 꽃목걸이를 건 맨몸 · 맨팔 · 수영복과 샌들 · 물놀이 장난감 무기.
# 무기는 {who}의 직업들이 쓰는 종류만 그렸다(char_dat 의 직업별 두 무기 트리).
set            = 수영장 파티 {who}
acquire        = starter
hero           = {hero}
helmet         = 수영장 선글라스
helmet.desc    = {glasses}
armor          = 수영장 꽃목걸이
armor.desc     = 맨몸에 건 꽃목걸이
gloves         = 수영장 맨팔
gloves.desc    = 소매 없이 드러낸 팔
shoes          = 수영장 수영복
shoes.desc     = {trunks}
weapon         = 수영장 장난감
weapon.desc    = 물놀이 장난감 무기
""" + '\n'.join(weapons) + """
shield         = 수영장 비치볼
shield.desc    = 알록달록한 비치볼
"""


SETS = {
    '04': ('04_halloween', lambda: (H.build(), HALLOWEEN)),
    '05': ('05_kenneth', lambda: (K.build(), KENNETH)),
    '06': ('06_pool_kei', lambda: (S.build('kei'), pool('kei'))),
    '07': ('07_pool_ritz', lambda: (S.build('ritz'), pool('ritz'))),
}


SUPERSEDED = {'04', '05', '06', '07'}   # 169차: tools_re/costume_atelier/sets/*.py draws these now


def differences(built, shipped):
    """What differs between two set folders: frames by PIXELS (not PNG bytes), costume.txt by text
    (line endings aside — git's autocrlf checks it out as CRLF on this machine)."""
    bad = []
    for n in sorted(set(os.listdir(built)) | set(os.listdir(shipped))):
        a, b = os.path.join(built, n), os.path.join(shipped, n)
        if not os.path.exists(b):
            bad.append(f'{n}: built but not in costumes/')
        elif not os.path.exists(a):
            bad.append(f'{n}: in costumes/ but not built')
        elif n.endswith('.png'):
            if F.AS.read_png(a) != F.AS.read_png(b):
                bad.append(f'{n}: pixels differ')
        elif open(a, 'rb').read().replace(b'\r\n', b'\n') != open(b, 'rb').read().replace(b'\r\n', b'\n'):
            bad.append(f'{n}: differs')
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('sets', nargs='*', help='04 05 06 07 (default: all four)')
    how = ap.add_mutually_exclusive_group(required=True)
    how.add_argument('--check', action='store_true')
    how.add_argument('--out')
    how.add_argument('--force', action='store_true')
    a = ap.parse_args()
    keys = a.sets or sorted(SETS)
    for k in keys:
        if k not in SETS:
            sys.exit(f'unknown set {k} (have {" ".join(sorted(SETS))})')
    if a.force and any(k in SUPERSEDED for k in keys):
        sys.exit('refusing --force: 04-07 are drawn by tools_re/costume_atelier since 169차 '
                 '(this would overwrite the new art) - use tools_re/costume_atelier/build.py')
    if a.check:
        for k in keys:
            print(f'{SETS[k][0]}: superseded (169차) - checked by tools_re/costume_atelier/build.py --check')
        return 0
    tmp = None
    stale = False
    try:
        for k in keys:
            folder, make = SETS[k]
            parts, txt = make()
            dst = os.path.join(tmp or a.out or ROOT, folder)
            F.write_set(dst, parts, txt)
            if not a.check:
                print(f'{folder}: written to {dst}')
                continue
            bad = differences(dst, os.path.join(ROOT, folder))
            print(f'{folder}: {"OK" if not bad else "STALE"}')
            for b in bad[:12]:
                print('   ', b)
            stale |= bool(bad)
    finally:
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
    return 1 if stale else 0


if __name__ == '__main__':
    sys.exit(main())
