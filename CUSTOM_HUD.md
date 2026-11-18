# Hero3 커스텀 HUD (오버레이) — 진행 상태 + 다음 세션 TODO

> **★★★ 2026-07-30 — 사용자 실기기 플레이 피드백 6건 전부 수정. canonical = [`PLAY_FEEDBACK_FIXES.md`](PLAY_FEEDBACK_FIXES.md).**
> 이 문서의 HUD 관련 서술 중 **다음 두 가지는 그 문서가 정정**한다:
> ① **HP/SP 게이지 오프셋이 뒤바뀌어 있었다** (빨간 바가 SP 를 그림). 정답 =
>    HP `(s16)CHAR+0xf2` / max HP `(s32)CHAR+0xec`(필드다, 상수 1000 아님) / SP `CHAR+0xf8` 0..1000.
> ② **필드 판별자는 `CHAR+0x7c` 가 아니다** — 게임 창이 열려도 안 바뀐다. 창 id `CHAR+0xa0c0`
>    (3=창없음/8=맵/14=저장/15·16=인벤)와 함께 봐야 한다. 그래서 예전엔 메뉴 위에 HUD·마스크가
>    그려지고 게이지가 0으로 보였다.
> 그 외 신설: 메뉴 모드 1/3/7/9 키, 풀 키패드 토글, 단일방향 조이스틱, 미니맵 자동갱신 증거화,
> 몰입 논리폭 캡 496(게임 팝업 u8 절단 회피).

> **★★★★ 2026-08-03 — 이 문서의 미니맵 서술은 전부 폐기되었다.**
> 아래에 나오는 "게임은 맵창을 열 때만 오프스크린 버퍼를 칠하므로 호스트가 `*` 를 두 번 눌러
> 재-페인트시킨다 / `LAST_OFFSCREEN_COPY` 로 현재 버퍼를 식별한다 / 번쩍임은 프레임 드롭으로
> 은폐한다" 는 **전제 자체가 불필요했다**. 맵창이 칠할 때 쓰는 두 배열(`attr`·`event`)은 필드의
> 충돌·워프 판정이 매 프레임 읽는 **상시 유효 데이터**라, 호스트가 아무 때나 같은 그림을 직접
> 그릴 수 있다. 맵창은 이제 **한 번도 열지 않는다**. 디컴파일 결과·픽셀 대조 검증·삭제 목록은
> [`PLAY_FEEDBACK_FIXES.md`](PLAY_FEEDBACK_FIXES.md) **13차**.

> 2026-07-24. H5식 커스텀 HUD: 좌 조이스틱 + 우측 원형 스킬/포션 버튼(디코드 아이콘) + 좌상단 HP/SP/EXP 바 + 게임 자체 HUD 마스크. **전부 오버레이(게임 바이너리 패치 0).** 전부 미커밋.
> **★★ 2026-07-28 세션 = Path A 결착 + 진짜 원인(`_bm` 디코더 2바이트 버그) 발견·수정.** 바로 아래 섹션이 canonical.

## ★★★★★ 2026-07-28 — Path A 종결: `CHAR+0x9bf8` = s{N}_bm 이었고, 진짜 범인은 **`convert_bm_v2` 디코더 버그**

**1. `CHAR+0x9bf8` setter 발견 → 리소스 확정 (정적 disasm, HIGH)**
- 찾은 법: literal 스캔이 실패한 이유 = 이 필드는 **`CHAR+0xb4` 베이스 + 리터럴 `0x9b44`** 로 써진다(0xb4+0x9b44=0x9bf8). CHAR 심볼릭 추적 스캐너(`scratchpad/scan_char.py`: `bl 0x14ad10` 반환값을 base 심볼로 잡고 const/copy/sp-spill 전파)로 전 바이너리 스캔 → **store 사이트 = guest `0x140448`** (형제 `0x140452` = `CHAR+0x9bfc`).
- 그 함수가 하는 일: `alloc(0x20)` → `CHAR+0x9bf8` 저장 → `CHAR+0x9bfc = *(mgr)+8`(이미지 헤더 배열 캐시) → **`i=0,1` 루프**로 `*(sl+0x328)` = **`"/skill/s4_bm"`** 이름 템플릿 + 시트 id `(int8)*(CHAR+0xd1+i)`(코드상 `[sp+0x34]=CHAR+0xb4`, `+0x1d+i`) 를 `0x194b98` 에 넘겨 2장 로드.
- `0x194b98` 독립 확인: 이름을 **뒤에서 `'_'`** 까지 잘라 suffix(`_bm`), **앞에서 첫 숫자 전까지** prefix(`/skill/s`) 로 분해 → 나머지 인덱스 인자가 음수(caller 가 `-1,-1` 전달)면 포맷 `sl+0xccc` = **`"%s%d%s"`** 로 재조립 → **`/skill/s{sheetId}_bm`**. (2자리 s10 용 `>9` 분기도 존재.)
- ∴ **HUD 스킬 아이콘 소스 = 우리가 이미 쓰는 `/skill/s{N}_bm` 바로 그것**. 핸들만 다른 **두 번째 2-슬롯 매니저**(`0x9bf0` = 클래스시트 매니저, `0x9bf8` = HUD 전용 온필드 2영웅 매니저)일 뿐.
- ⛔ **RETRACT(직전 세션)**: "게임 HUD 는 s{N}_bm 을 안 그린다 / 소스가 다른 이미지" = **오답**. 핸들 상이 ≠ 리소스 상이.

**2. 블릿 크기 = `15×15` (직전 세션 "15×9" **RETRACT**)**
- `0x14a51a` 가 `0x10f984(ctx, x, y, r3=15, s0=15, img16B, srcX=cell*15, srcY=0, s3=9)` 호출.
- 래퍼 `0x10f984` → `0x11a568` 인자 매핑을 따라가면 `r3`/`s0` 이 폭·높이 슬롯이고 `9` 는 **맨 뒤 플래그 인자**. 결정적 근거 = **자매 래퍼 `0x10f9ac`**(=이미지 전체 그리기)가 **그 두 슬롯을 이미지 헤더의 `img[4]`/`img[2]`(=자기 w/h)로 채운다** → 두 슬롯 = w,h 확정. 게다가 **아이템 브랜치(`0x14a3b4`)도 똑같이 9 를 넘기면서 풀사이즈로 그린다** → 9=높이 해석은 모순.
- ∴ crop = **cell*15 부터 15×15** = 현 오버레이와 동일. 매핑(sheet,cell)·crop 전부 이미 맞았음.

**3. ★ 진짜 원인 = repo `_bm` 디코더의 2바이트 오프셋 버그 (CRACKED)**
`d:\testrepo\tools\converter\convert_bm_v2.py` 는 9바이트 프레임 헤더 뒤의 `0xf81f` 를 **"프레임 경계 마커"** 로 보고 팔레트를 2바이트 뒤부터 읽는다. 실제로는 **그 `0xf81f` 가 palette[0](투명 센티널)이고 `palcnt` 가 그걸 포함**한다. 결과 = 모든 인덱스 k 가 palette[k+1] 로 해석 + 픽셀 스트림 2바이트 밀림 → **색 노이즈**(+ 옛 docstring 이 "정렬 이슈"로 넘긴 2바이트 underrun).

확정 포맷(자체 검증 assert 로 강제):
```
file :  u16 frame_count                     (Hero4 컨테이너는 u32)
frame:  u32 frame_len
        u8 type(0x0b=4bpp | 0x0c=8bpp), u16 w, u16 h, u16 cw, u16 palcnt
        u16 palette[palcnt]  (RGB565 LE, [0]=0xf81f=투명)
        u8  pixels[...]      (dense, row stride = w)
        frame_len == 9 + palcnt*2 + (w*h | (w*h+1)/2)
```
`w`=바이트 정렬된 행 폭, `cw`=실제 보이는 폭(4bpp 는 행이 바이트 패딩), index 0 = 투명.

**증거(전수)**: Hero3 JAR `_bm` **479/479** 파일이 바이트 단위로 **정확히 소진**(item_bm 6426/6426, 29/29 프레임). Hero4 `work/h4/extracted` 이미지 파일 **420개** 도 동일 규칙으로 exact(프리픽스만 u32). 렌더 결과 = 물약/반지/두루마리, 화살/조준경/활 등 **알아볼 수 있는 아트**(직전엔 컬러 노이즈).
- ⛔ **RETRACT(직전 세션의 2차 자가정정)**: "디코드는 정상" = **오답**. 최초 진단("디코드 손상")이 맞았고 **위치만 틀렸다**(팔레트 offset). 그 오판 때문에 s{N}_bm 이 소스가 아니라는 2차 오답까지 연쇄됐음.
- ⚠ **cross-game 영향**: `convert_bm_v2` 로 구운 Hero3/Hero4 `_bm` 자산 **전부** 같은 결함을 안고 있다. repo 툴 수정 + 재베이크는 **별건 대형 작업**(테스트/라운드 문서 동반) → **사용자 결정 대기**. 이번 세션은 HUD 아틀라스만 자체 디코더로 해결(리스크 0).

**4. 이번 세션 구현 (전부 컴파일·빌드 완료)**
- `tools_re/gen_h3_icons.py` = **자체 정확 디코더 탑재**(repo convert_bm_v2 의존 제거) + 프레임마다 포맷 assert. 발명이던 **`SKILL_ICON_W=12` crop·`SKILL_DARK_KEY=48` 다크키 제거** — 게임과 같은 **raw 15×15 셀**(원형 클립은 `blit_icon` 의 disc 클리핑이 담당 → H5 룩 유지하면서 픽셀은 원작 그대로).
- `wie_android/src/icons.bin` 재생성(82,998B; items 29×15×15 + s4~s10 각 9셀).
- `items.rs::skill_icon` = 게임과 **동일한 2-way 선택**으로 교정: `onfield = ((ref as i8)>>6) != 0`, `sheetId = u8@CHAR+0xd1+onfield`. (기존 `member=ref>>6` 0..3 직접 인덱싱은 member 2/3 에서 `CHAR+0xd3/0xd4` 를 읽어 게임과 어긋남 — 실제 게임은 온필드 2장만 로드.)
- 검증: `items_selftest.rs` **8/8 PASS**(ground truth 일심/장탄 + 빈칸/잘못된 시트/2시트 clamp/범위밖 셀). ※ `cargo test -p wie_android` 는 winit/android 링크 때문에 **Windows 호스트에서 기동 즉시 STATUS_ACCESS_VIOLATION** = 기존 문제(그래서 standalone selftest 사용).
- 빌드: `cargo ndk`(arm64-v8a) + strip + `:app:assembleDebug` → **APK 48,991,518 B**.
- ⏳ **미검증 = 실기기**: 이번 세션에는 인증된 폰이 안 붙어 있었다(`adb devices` = `<PHONE_SERIAL> unauthorized`, S25 `<S25_SERIAL>` 부재) → 설치/스크린샷 못 함. **사용자 게이트**.

### ▶▶ 다음 세션 첫 행동
1. **실기기 확인**(1순위): APK 설치 → SLOT3 필드에서 퀵바 스킬 2칸이 이제 **알아볼 수 있는 스킬 아트**(s8c5 일심 / s7c5 장탄)로 뜨는지 + 포션 아이콘 선명도. 아니면 스크린샷 회신.
2. **cross-game 결정**: repo `convert_bm_v2` 수정 + Hero3/Hero4 자산 재베이크 진행 여부(라운드 문서 + 테스트 동반). 위 §3 참조.
3. (미착수 유지) 아래 "가로 몰입 레이아웃" 목표.

<details><summary>▼ 2026-07-27 Path A 원본 기록 (위에서 결착·일부 RETRACT)</summary>

## 2026-07-27(계속) — Path A: 게임 실제 하단-HUD 스킬아이콘 draw 루틴 RE (사용자 "A로 진행" → 1·2 실행 시도 → **재정정**)
> **★★ 2회 자가정정 정리 (정직)**:
> - (last-round 주장) "s{N}_bm 디코드 손상이 원인" = **RETRACT**. 실측 재검(s7/s8 full strip dark-key 렌더)으로 **디코드는 정상**(s8=coherent 상세 아이콘, s7=대부분 구조적·일부 busy=원작 추상아트). idx0(transparent)=35뿐·bg=idx1(near-black 0,0,8)이나 gen_h3_icons 의 dark-key(<48)가 이미 제거 → 노이즈 아님.
> - (그렇다면) **게임 HUD 는 s{N}_bm 상세 아이콘을 안 그린다**: 게임 HUD=단순 flat(녹색디스크+흰X/파랑디스크+흰대각선) ≠ s{N}_bm 멀티컬러 상세아트(칼날/링/버스트) → **소스가 다른 이미지**. disasm 도 소스=**`CHAR+0x9bf8`**(별도 핸들)이 s{N}_bm 시트매니저 **`CHAR+0x9bf0`**(setup fn `0x140c7c`: alloc 0x14B → `0x199cc8(.., CHAR+0xd1, 2)`)과 **다름**을 확인 → 워크플로 L2 "not s{N}_bm" **재확인(맞음)**.
> - ∴ **1(디코드 수정)·2(15×9 crop) = s{N}_bm 대상이라 부적합**(s{N}_bm 은 HUD 소스가 아니므로 고쳐도 게임과 안 맞음). **실행 보류**(잘못된 변경 회피).
>
> **★ 미해결 blocker = `CHAR+0x9bf8` 의 정확한 JAR 리소스**: heap-alloc 핸들이라 정적 추적 난이도 高 + **세션 usage limit** 로 워크플로 L1/L4(loader) 사망 → 미확정. setup fn `0x140c7c` 은 `CHAR+0x9bf0`(s{N}_bm) 만 세팅·`0x9bf8` 은 별도 코드(미발견). literal 0x9bf8 write-site 부재(read 2곳만).

**게임 필드 하단-HUD 퀵바 draw 루틴 = guest `0x149b40`~`0x14a6c0`** (vtable dispatch·BL caller 0). 슬롯 0..4 루프, `type[slot]` (`CHAR+0x252`, base `CHAR+0xb4`) 로 분기: skill(==1)→`0x14a51a`, item(==2)→`0x14a3b4`, empty→셀 배경만.

**스킬 아이콘 draw (skill 브랜치 `0x14a51a`) — 정적 disasm 확정(HIGH)**:
- 소스 이미지 = 핸들 **`CHAR+0x9bf8`** → `[sp+0xe4] = *(*(CHAR+0x9bf8))+8` (16B 이미지-헤더 2개 배열). **s4..s10 클래스시트(=`CHAR+0x9bf0`, 스킬 메뉴 `0x13420c` 가 sheet-helper `0x19c280`+`CHAR+0xd1`로 쓰는 것)와는 다른 핸들** — HUD 는 sheet-helper/`CHAR+0xd1` **미사용**(브랜치 내 `0x19c280` 호출 0).
- 프레임 선택 = **`r7 = (skill-ref[slot]>>6 != 0) ? 1 : 0`** (`CHAR+0x25c+slot` 상위2비트가 nonzero? → 헤더 index 0/1). **현 오버레이의 `sheetIdArray[0xd1+member]`(7시트) 선택과 다름** — HUD 는 **온필드 2 영웅 시트 중 택1**(2-value). SLOT3 에선 양쪽이 우연히 일치(member0→s7, member1→s8).
- 셀 = **`cell = cell-array[slot]` (`CHAR+0x26c+slot`)**, **`srcX = cell*15`**, blit **15×9 px** (15px 셀의 상단 9행만). 현 오버레이는 15×15 전체 crop.
- 검증(SLOT3 ground truth): slot0 ref0x6a→r7=1→시트[1], slot1 ref0x2d→r7=0→시트[0], cell 둘 다 5. = **s8c5(일심)/s7c5(장탄)** — **현 오버레이 (sheet,cell) 결과와 동일**. ∴ 매핑은 원인 아님.

**★ 다음(사용자 결정 대기 = 3-way fork)**:
- **(A) 진짜 decoded**: `CHAR+0x9bf8` 리소스 확정 → 디코드 → (frame=ref>>6!=0, cell=cell-array, 15×9) 슬라이스. 확정법 = ①세션limit 해제 후 loader RE(0x9bf8 setup fn·0x199cc? 계열 재추적) 또는 ②device RAM dump(`*(*(CHAR+0x9bf8))` 포인터 → 이미지 헤더 dump → w/h/데이터로 리소스 매칭). 사용자 선호(decoded)나 현재 정적 gated.
- **(B) 라이브 크롭(보장)**: HUD draw 스크린 rect 는 이제 grounded(하단 strip y≥288·5셀) → 게임이 이미 그린 셀을 프레임버퍼서 크롭=**픽셀 정확 일치**·리소스ID 불요. 원래 접근(핑크테두리=trim). 사용자가 이전에 decoded 선호했으나 현재 A가 gated면 최속.
- **(C) 현행 s{N}_bm 유지**: 현 오버레이의 상세 s{N}_bm 아이콘(일심/장탄 실아트)=정확한 스킬아트·H5식 리치. 게임 tiny-HUD 와 "다르"지만 더 풍부. "게임과 정확 일치" 불요면 현행 OK(dark-key/crop 미세조정만).

**★ 오답 회피 기록**: 1(디코드수정)·2(15×9 crop) = s{N}_bm 대상 → **HUD 소스 아님이라 부적합**. 실행 안 함.

### ▶▶ 다음 세션 첫 행동 (사용자 결정 2026-07-27 = **A: decoded 방식 확정**)
> 목표 = **`CHAR+0x9bf8` 핸들의 실제 이미지 리소스 특정** → 디코드 → HUD draw 와 동일하게 `frame=(skill-ref>>6!=0)?1:0` · `cell=cell-array[slot]` · **15×9** 슬라이스로 오버레이 아이콘 교체. 두 접근(A-1 우선 → 실패 시 A-2):

**A-1 (자율·우선, 세션 usage limit 리셋 후 = 한국시간 20:40+)** — loader RE 로 `CHAR+0x9bf8` 세팅 코드 찾기:
- 기지식: setup fn **`0x140c7c`** = `CHAR+0x9bf0`(s{N}_bm 시트매니저)+`0x9bf4` 만 세팅(`alloc 0x14B` → `0x199cc8(*(0x9bf0)+0xc, CHAR+0xd1, 2)`). **`0x9bf8` 은 여기서 안 씀 → 형제 setup fn 이 별도 존재**.
- 시작점: ① `0x199cbc/0x199cc0/0x199cc4/0x199cc8` (sheet-manager helper 계열) **전 caller** `find_callers.py` → `CHAR+0x9bf8` store 하는 fn 식별. ② literal `0x9bf8` write-site 부재 → 핸들은 base `0x9bf0` (여러 site) 또는 computed offset(`movs#0x9b;lsls#8`+add)으로 write 될 수 있음 → 그 패턴 스캔. ③ 리소스명 = sl name-ptr 테이블(`sl+0x328`=`/skill/s4_bm`, 인접 idx = EUC-KR 디버그문자열; snd 는 idx14+) 에서 매칭 — 스킬아이콘 전용 시트명 재확인(comm/ui_bm·inmenu_bm 도 후보).
- 워크플로 재개 가능: `Workflow({scriptPath:"...h3-hud-quickbar-icon-locate-wf_ae81094a-1e8.js", resumeFromRunId:"wf_ae81094a-1e8"})` — L2/L3 캐시 재사용, L1/L4 만 재실행. (단 스크립트의 L1/L4 프롬프트를 "0x9bf8 setter 찾기"로 Edit 후 재개 권장.)

**A-2 (fallback, device RAM dump)** — SLOT3 세이브 + **필드 자유이동(state==4)** 에서 포인터 체인 dump:
- 체인: `CHAR = *(0x1b3084)`(≈`0x1b7b6c`) → `P1 = *(CHAR+0x9bf8)` → `P2 = *(P1)` → 헤더배열 `= P2+8` (16B 이미지헤더 ×2 = frame0/1; 각 헤더에 w/h/dataptr/palptr). w/h 로 JAR `_bm` 매칭.
- 2-pass dump_trigger(줄="addr_hex len_hex"): **pass1** `1b3084 4`(→CHAR 확인) + `1c1764 16`(=CHAR+0x9bf0~0x9c00, 3 핸들; CHAR=0x1b7b6c 가정) → 출력서 P1=*(CHAR+0x9bf8) 읽기. **pass2** `<P1> 4`(→P2) + `<P2> 40`(헤더 2개) → w/h/dataptr → **pass3** `<dataptr> <w*h+pal>` 로 픽셀+팔레트 dump → JAR `_bm` 바이트 매칭.
- adb(§`project_wie_native_saveload`): `MSYS_NO_PATHCONV=1 adb push d:/tmp/dump_trigger /data/local/tmp/` → `adb shell run-as net.wie.emulator cp /data/local/tmp/dump_trigger files/` → 2초 → `adb exec-out run-as net.wie.emulator cat files/ram_dump.bin`(region = u32 addr+u32 len+bytes). ⚠ 자동 tap 중 폰 개인알림(예 NAVER 2단계) 뜨면 누르지 말고 사용자에게 알리고 중단.
- 리소스 특정 후 = 그 `_bm` 을 `gen_h3_icons.py` 에 소스로 추가(convert_bm_v2 디코드) → `icons.bin` 재생성 → `items::skill_icon` 을 (frame=ref>>6!=0, cell=cell-array, 15×9) 로 교체 → 빌드 → device 재검증(사용자 게이트).

**RE 산출(scratchpad)**: `f_14axxx_hud2.txt`/`f_14a_branches.txt`/`f_14a_prologue.txt`/`f_14a_e4setup.txt`(HUD draw), `f_143048_dispatch.txt`(데이터모델), `f_1428cc_hudchain.txt`+`f_142758_top.txt`(cast-anim=HUD아님), `f_1533a8_itemicon.txt`(인벤=HUD아님), `f_134388_caller.txt`(스킬메뉴). 워크플로 `wf_ae81094a-1e8`(L2/L3 완료 HIGH·L1/L4 session-limit 사망).

</details>

## ★★★ 가로 몰입 레이아웃 — **2026-07-28 구현 완료** (상세 = [`LANDSCAPE_FEASIBILITY.md`](LANDSCAPE_FEASIBILITY.md))
> 사용자 가설("해상도 fix 를 풀면 꽉 찰 것") = **맞았다**. 헤드리스 러너 `wie_probe` 로 실측하니
> 게임은 **해상도 적응적**(필드는 맵을 더 넓게 그리고, 타이틀/HUD/미니맵은 가운데 정렬, 대사바는
> 폭 전체) → **게스트 패치 0** 으로 가로 100% 채움 달성. 2026-07-24 "HARD/LARGE 필요" 판정은 반박됨.
>
> 구현: 첫 `resumed` 에서 창 aspect 로 게스트 프레임버퍼 확정(`init_logical`, base_h 기본 240 →
> 2340×1080 이면 520×240) → 1:1 fill(왜곡 0) · 양옆 컨트롤 컬럼 제거 · 우측 반투명 2×4 키
> (`1 3 / 5 9 / 0 # / OK CLR`) · 그 외 전 영역 조이스틱 · 게임 자체 HUD 는 실측 **176×24 logical**
> 박스만 마스킹하고 **그 안에 우리 퀵바를 배치** · 게이지는 logical 스케일. 설정에 **FULL/ZOOM**
> 토글(재시작 시 적용). `landscape_selftest.rs` **10/10 PASS**, APK ~49 MB.
> **★ 실기기 검증 완료(SM-S938N 2340×1080)**: `-> guest framebuffer 520x240`, 타이틀/SLOT/필드 전부
> 100% 채움·조이스틱 이동 OK·크래시 0. 기기에서 결함 2건 잡아 수정 — ① 한글 라벨("취소")은
> `draw_glyph` 가 5×7 ASCII 폰트라 **빈 칸으로 렌더** → `CLR` 로 교체 ② **회전 레이스**(첫
> `resumed` 가 회전 전 세로 크기를 보고 → 두 번째 실행부터 몰입 모드 무효화) → 저장된 가로 설정 +
> 창 long/short 로 판단하도록 수정, 3회 연속 확인. ZOOM 240=완전 채움 / 320=시야↑ 대신 좁은 맵에서
> 좌우 띠(예측과 일치).

<details><summary>▼ 2026-07-27 원본 목표 기술 (구현으로 대체됨 — 2·3번 항목의 stretch/crop 딜레마는 "게임이 실제로 더 넓게 그린다"로 해소)</summary>

## 다음 세션 목표 — 가로 몰입(full-immersion) 레이아웃 (사용자 확정 2026-07-27, 최우선)
> 이전 "최대 확대"(양옆 컨트롤 컬럼 유지)를 **대체**하는 사용자 확정 목표.

**사용자가 원하는 가로모드 최종 형태:**
1. **양옆 컨트롤 컬럼(LSK/RSK/CLR/OK + 숫자패드) 전부 제거.**
2. **게임 화면이 가로·세로 모두 화면을 꽉 채움** (현재는 세로만 참). 게임=240×320(3:4)이므로 16:9를 꽉 채우려면 **stretch(왜곡) 또는 cover(상하 crop)** 중 택1 필요 → **다음 세션 첫 결정** (사용자는 "꽉 채움"을 원함 = 이전 "왜곡 없는 최대확대"에서 방침 전환).
3. **반투명(translucent) 버튼**을 화면 **우측에** 오버레이: **`1, 3, 5, 9, 0, #, 취소`** (7개, 축소 세트). 게임 위에 반투명으로 떠 있음.
4. **반투명 버튼 영역을 제외한 화면 전체 = 조이스틱 존** — 아무 데나 눌러 이동.

**feasibility / 구현 노트 (Claude 답변="가능할 수도 있다" → 계속 구현·테스트 중):**
- **반투명**: softbuffer 는 **알파 블렌딩 없음**(불투명만) → 버튼을 반투명으로 그리려면 **오버레이에서 수동 알파 블렌드**(bg 픽셀 read → 버튼색과 ~50% 혼합 → write) 구현 필요. 가능(per-pixel RMW).
- **게임 꽉 채움**: `app.rs redraw` 의 게임 blit 은 `layout.game` rect 로 스케일 → rect 를 전체 창으로 하고 stretch(비율 무시) 또는 cover(비율 유지+crop) 로. `overlay::layout()` 가로 분기 재설계.
- **조이스틱=전체−버튼**: 현재 조이스틱 존=좌측 영역 → "전체 창에서 버튼 rect 만 제외"로 변경. hit-test 는 버튼 우선, 아니면 조이스틱. 가능.
- 버튼 세트 축소(1/3/5/9/0/#/취소): `build_*` 재구성. "취소"=CLEAR(CLR) 또는 별도.
- ⚠ 세로모드는 현행 유지(이 목표는 가로 전용). ⚠ 원작 240×320 하드코딩(`LANDSCAPE_FEASIBILITY.md`)과 별개 — 이건 **오버레이/스케일 레이어**라 게임 바이너리 무관(가능).

</details>

## ★★★★★ 2026-07-28(계속) — 레퍼런스 UI 재설계 (사용자 스크린샷 기준)

사용자 제공 레퍼런스(모바일 액션RPG HUD)대로 가로 몰입 오버레이를 재구성. **떠 있는 숫자 키패드를
없애고 스킬 버튼으로** 바꾸는 것이 요지였다.

| 요소 | 구현 |
|---|---|
| **스킬/아이템 버튼** | 우하단 **액션 디스크 왼쪽-위 아크**에 5칸(퀵슬롯 1·3·9·0·#). 반투명 사각 플레이트 + 림(스킬=골드/아이템=스틸/빈칸=흐린 회색) + **디코드된 아이콘**(icons.bin) + 아이템은 **보유 수량** 우하단 표기. 빈 칸은 핫키 문자만 |
| **액션 버튼** | 우하단 큰 반투명 원(2중 골드 링) = `OK` |
| **HP/MP** | 좌상단 **원형 초상 플레이트**(레벨 숫자 내부) + 오른쪽에 HP(빨강)/SP(파랑) 캡슐, **현재값 숫자를 바 안에 우측정렬** |
| **EXP** | **최하단 중앙**으로 이동. 게임 자체 HUD 를 덮는 띠 안에 폭 62% 앰버 바 |
| 하단 띠 | 가운데 176×24 logical 박스는 불투명(게임 HUD 은폐 필수) + **좌우는 반투명 틴트** → 떠 있는 검은 사각형이 아니라 **연속된 하단 HUD 스트립**으로 보이게 |
| 조이스틱 | 좌하단 **상시 표시**(레퍼런스처럼). 드래그하면 손가락 위치에 라이브 스틱 생성(기존 동작 유지) |
| 상단 | `MAP`(`*`) `MENU`(LSK) `SEL`(RSK) `BACK`(CLR) 텍스트 버튼 — 스킬 5칸이 덮지 않는 키 |

### ★ 이 라운드에서 잡은 결함 3건

1. **5×7 오버레이 폰트에 알파벳이 없었다.** 테이블은 `0-9 * # O K C L R S E T X P` **14 글리프**뿐이고
   나머지는 전부 `_ => [0; 7]`(빈 칸). "취소" 가 안 보인 것도, `map/menu/sel/back` 이 빈 플레이트로
   나온 것도 **같은 원인**이다. → **A-Z 전체 + `- + / . %` 추가**하고 `glyph()` 이
   `to_ascii_uppercase()` 로 **대소문자 접기**를 하게 수정. (앞으로 라벨은 이 집합 안에서만.)
2. **스킬 아크가 화면 밖으로 나가 바닥에 일렬로 clamp 됐다.** 화면 좌표는 y 가 아래로 자라고
   `cy = acy - r*sin` 이라 **90°=위 / 180°=왼쪽**인데 188°~276°(아래-왼쪽~아래)로 잡았다.
   → **104°~196°** 로 교정.
3. **인접 슬롯이 겹쳤다(기기에서 육안 확인 + selftest 가 잡음).** 사각형은 **현(chord) ≥ side 로 부족**하다
   — 대각 오프셋이면 |dx|,|dy| 둘 다 side 미만일 수 있다. → 각 간격에서 필요한 반경을
   `chord ≥ side*1.485` 로 **역산**하고 slot 크기를 `win_h/9` 로 줄임. selftest 가 전 창 크기에서
   비겹침을 단정.

### 검증
* `landscape_selftest.rs` **11/11 PASS** (아크 비겹침·액션 디스크 미침범·상단 행 비겹침·조이스틱 홈이
  액션 디스크와 분리·회전 레이스 가드 포함).
* **실기기(SM-S938N)**: `MAP/MENU/SEL/BACK` 라벨 정상, HP `850` / SP `331` 숫자 표시, 초상 레벨 `7`,
  EXP 앰버 바가 하단 스트립에서 채워짐, **SLOT3(퀵슬롯 배정된 세이브)에서 디코드된 스킬 아이콘 2개 +
  포션 아이콘 + 수량 `14`** 가 플레이트에 정상 렌더(로그 `types=[1,1,2,0,0]`). 크래시 0.
  스크린샷 = `d:/tmp/probe/dev_ui_slot3.png`.
* ⏳ **결함 3(아크 비겹침) 수정본은 빌드/테스트만 완료 — 기기 재설치 전에 USB 인증이 끊겨 미설치.**
  다음 세션 첫 행동 = 그 APK 설치 후 아크 간격 육안 확인.

### 남은 갭 (레퍼런스 대비, 정직)
* **초상 얼굴 아트 없음** — 원형 플레이트에 레벨 숫자만. 레퍼런스처럼 얼굴을 넣으려면 JAR
  `hero/h*_bm` 프레임을 `icons.bin` 에 초상 섹션으로 추가해야 함(atlas 포맷 확장).
* 레퍼런스의 `DASH`/재정렬 버튼 등 게임에 없는 기능은 만들지 않았다(발명 회피).
* 좁은 실내 맵에서는 좌우 검은 여백이 크다 → 설정 **ZOOM** 을 낮추면 채움률↑(§LANDSCAPE_FEASIBILITY).

## ★★★★★★ 2026-07-28 (3차 UI 패스 — 사용자 피드백)

| 요청 | 구현 |
|---|---|
| OK 를 **둘러싸는 링** | 우측 캐스케이드 폐기 → **액션 디스크 중심 링**(`ring_slots`): 6석 등거리(seat0=CLR, seat1~5=퀵슬롯 0~4), 2340×1080 에서 `side=108 · R=336 · 138° wrap`. 반경은 ① 디스크 클리어 ② **인접 정사각형 비겹침**(`chord ≥ side*1.485` — 대각 오프셋 때문에 `chord ≥ side` 로는 부족) 을 만족하는 최소값, 화면을 벗어나면 plate 를 10% 씩 줄여 재시도 |
| HP/MP **숫자 제거** | 바 안 값 텍스트 삭제(채움 자체가 readout). 바 높이도 `h/22`→`h/26` |
| 레벨 원 **축소** | `h/11`(98px) → `h/16`(67px) |
| EXP 바 **축소** | 폭 62%→**34%**, 높이 `band/3`→`band/6` |
| MENU/SEL **제거** | 삭제. ⚠ 그 결과 **LSK/RSK 는 몰입 모드에서 미바인딩** — 게임 일부 상태의 자체 메뉴가 이 키를 쓰므로 필요해지면 되살려야 함(정직 표기) |
| 미니맵 **맵 이동 시 갱신** | 게임은 **맵 창을 열 때만** 오프스크린 버퍼를 칠한다 → 호스트가 **스스로 `*` 를 두 번 눌러**(open→close) 게스트에게 재-페인트를 시킨다. 트리거 = 버퍼가 없거나 읽히지 않음 + **필드 자유이동 상태** + 5초 쿨다운. 약 0.4초 게임 맵창이 번쩍임(그 비용으로 자동 최신화) |

**미니맵 파이프라인 정리**: `MC_grpCreateOffScreenFrameBuffer(w,h)` 1회 → `w*h` 번 1×1 `FillRect` → `CopyFrameBuffer` (WIPI 트레이스). 핸들이 게스트 구조체에 안 들려 있어 **형상 스캔**으로 찾는다(`{w,h,bpl==w*2,bpp==16}` + 화면 크기 제외). 기기에서 두 번 헛발질 후 원인 확정:
* `Emulator::read_memory` 는 **매핑 안 된 구간에 걸치면 전체를 빈 값으로 반환** → 1MB 통짜 읽기가 조용히 실패 → **64KB 청크**로 변경
* 힙 앵커(인벤토리 그리드 핸들)는 `0x4040_0000`, 프레임버퍼는 `0x4906_xxxx` = **145MB 위** → ±24MB 창으로는 도달 불가 → **앵커 위쪽으로 넓게**(256MB, 스텝당 12MB) 스윕 → 약 7초 내 발견
* 기기 확인: `minimap framebuffer found: 0x4906884c 38x30` → 패널에 약 5배 확대 표시 성공

**검증**: `landscape_selftest.rs` **13/13 PASS** — 링 등거리성·디스크 외부·wrap>90°·전 컨트롤 비겹침/화면내·CLR=seat0·미니맵 패널 우상단·조이스틱 반경 ≤ 짧은변/10. APK 49,023,846 B 빌드 완료.
⏳ **3차 패스는 기기 미확인** — 설치 직전 USB 인증이 끊겼다(2차 패스까지는 기기 확인 완료).

## ★★★★★★★ 2026-07-28 (4차 — 미니맵 자동 갱신 완성 + 키 역할 실측)

### 미니맵: 번쩍임 은폐 + 맵 전환 감지 (사용자 선택 = (b) 은폐 + 맵 id 감지)

측정으로 드러난 제약 3개(전부 `wie_probe` 실측):
1. 게임은 per-map 오프스크린 버퍼를 **맵 창을 열 때만** 만들고 칠한다. 창을 한 번도 안 열면
   프레임버퍼 레코드가 **0개**.
2. **이전 맵의 버퍼를 해제하지 않는다** → 방문한 맵 수만큼 후보가 공존. 모양(shape) 스캔만으로는
   "지금 어느 맵인지" 알 수 없다. ← 사용자가 겪은 "맵 이동해도 안 갱신" 의 진짜 원인.
3. 맵 전환에도 `state`(CHAR+0x7c)는 4로 유지 → 상태 전이는 맵 변경 신호가 못 된다.

해결(3단):
* **현재 버퍼 식별 = 게스트의 copy-to-screen 후킹.** `MC_grpCopyFrameBuffer(screen, .., src, ..)` 는
  맵 창이 열려 있는 동안만 실행되고, 그 `src` 가 곧 **현재 맵의 버퍼**다. vendored wie 에
  관측용 슬롯 `wie_backend::LAST_OFFSCREEN_COPY` + `note_offscreen_copy()` 를 추가해
  `copy_frame_buffer` 가 기록하고, 앱이 읽는다. → 힙 스캔 전면 삭제(스캔은 청크/앵커 두 함정까지
  겪고도 "어느 맵인지"를 못 풀었다).
* **맵 변경 감지 = 맵 서명.** 전 CHAR/전역 차분(같은 맵 2샘플 vs 다른 맵 1샘플)에서 맵 id·맵 크기·
  맵 이름은 **부재**로 확인됐고, 대신 **`CHAR+0x9bcc..+0x9bd4`** 가 **걷기에는 바이트 단위로 불변,
  맵 로드 때만 통째로 변하는** per-map 리소스 포인터임을 확정 → 8바이트 해시를 서명으로 쓴다.
  (정직: 이것은 디코드된 map id 가 아니라 **변경 감지용 서명**이다.)
* **번쩍임 은폐.** 갱신은 게스트 맵 창을 실제로 열고 닫아야 하므로, 그 구간 동안
  `AndroidScreen::paint` 가 프레임을 **드롭**한다(+정착 8프레임) → 화면은 직전 필드 프레임을 그대로
  유지하고 사용자는 창을 못 본다. 5초 쿨다운으로 루프 방지.

**실측 증명** (`wie_probe`, 앱과 같은 vendored 코어):

| 프레임 | 위치 | 서명(CHAR+0x9bcc) | 마지막 copy |
|---|---|---|---|
| 700 | 마을 | `8c97874a8097874a` | None |
| 860 | 마을(맵창 후) | 동일 | `(0x4904a260, 46x48)` |
| 2100 | 실내(걸어서 이동) | **`8c75864a8075864a`** ← 변경 | 46x48 (stale) |
| 2300 | 실내(맵창 후) | 동일 | **`(0x4904a260, 24x22)`** ← 새 맵 버퍼 |

★ 같은 WIPI **핸들이 재사용**되고 **크기만 바뀐다** → 크기를 매 틱 후크에서 다시 읽는 현 구현이 맞다.

**기기 확인**: 부팅→세이브 로드 후 **탭 없이** 패널이 채워짐(`minimap: hidden auto-refresh` →
`minimap buffer: 0x4904a260 38x30`). 초기 구현이 비어 보였던 원인은 **KTF 간접 핸들 규약**:
`data_ptr(handle) = *(handle) + 8` 인데 `+8` 을 빼먹어 헤더 검증이 실패하고 있었다(인벤토리 그리드의
`GRID_DATA_ADJ` 와 같은 규약).

### ★ 자동 갱신을 고정 타이밍 → **증거 기반**으로 교체 (사용자 재보고: "여전히 맵을 한 번 눌러야 갱신됨")

증상: 자동 갱신이 **산발적으로 실패**(기기 로그에서 1차 시도 실패 → 8초 뒤 2차에서 성공). 원인은
`open → 14프레임 대기 → close` **고정 창**: 게스트가 버퍼를 다 칠하기 전에 창을 닫아버리는 라운드가
생긴다(프로브에서 짧은 간격도 성공하므로 타이밍 자체가 아니라 **라운드마다 흔들리는 것**이 문제).

교체: **도색을 실제로 본 뒤 닫는 상태기계** — `PressOpen → AwaitPaint → PressClose → Settle`.
`AwaitPaint` 는 매 프레임 copy-to-screen 후크를 확인해 **paint 가 관측되면 즉시** 닫고, 최대 80프레임
(~1.3초)까지만 기다린다. 실패한 라운드는 **2초 뒤 재시도**(성공 시 8초). 어떤 경로로도 화면이 얼어붙지
않게 **180프레임 하드캡**을 두고, 캡에 걸리면 키를 떼고 suppression 을 해제한다.

### 키 역할 실측 (사용자 질문: MENU/SEL 은 뭐고 LSK/RSK 는 뭐냐)

`LSK/RSK` = **Left/Right Soft Key** = 피처폰 화면 아래 두 개의 소프트키(WIPI
`KeyCode::LEFT_SOFT_KEY` / `RIGHT_SOFT_KEY`). 앞서 붙였던 `MENU`/`SEL` 라벨은 **내 추측**이었고,
프로브로 실제 동작을 확인했다:

| 키 | 실제 동작 (필드/메뉴) |
|---|---|
| **CLEAR (`CLR`)** | **메인 메뉴 열기** (상태보기/INVENTORY/EQUIPMENT/SKILL/QUEST/SYSTEM) **+ 뒤로가기** |
| **OK** | **확인/선택** (메뉴에서 항목 진입) + 필드 액션 |
| `5` | 확인/선택 대체 |
| **LSK / RSK** | 필드·메뉴에서 **눈에 보이는 동작 없음** |

→ 그래서 MENU/SEL 제거는 **기능 손실이 아니다**(라벨이 애초에 틀렸다). 현재 버튼 세트
(OK 디스크 + CLR + 퀵슬롯 5 + MAP 패널)로 **필드·메뉴·상태창까지 전부 조작 가능**함을 확인.

## ★ 신규 도구 — `wie_probe` (헤드리스 호스트 러너, 2026-07-28)
창/오디오 없이 **호스트에서 게임을 부팅**해 프레임을 덤프하고 WIPI draw-call 을 트레이스한다.
이번 가로 트랙의 모든 판단이 여기서 나왔고, 앞으로 RE/회귀 확인에 그대로 쓸 수 있다.
```
cargo build -p wie_probe
./target/debug/wie_probe --zip d:/tmp/hero3_fixed.zip --w 514 --h 240 --frames 700 --ms 20 \
    --db d:/tmp/probe/db --out d:/tmp/probe/x --dump 700 --keys 260:OK,300:OK,360:OK
python tools_re/probe_png.py d:/tmp/probe/x --montage d:/tmp/probe/x.png
RUST_LOG=wie_wipi_c=debug ./target/debug/wie_probe ... 2> trace.log
```
* `--ms` = 프레임당 가상 시계 진행(ms). **필수** — 게스트가 50 ms WIPI 타이머를 걸고 idle 하므로
  실시간 시계로는 헤드리스에서 타이머가 안 터져 화면이 영원히 검정이다(이번 세션 첫 함정).
* `--db <dir>` = `<dir>/PD007102/db/<name>/<record_id>` (Android DB 와 **동일 레이아웃**) →
  디바이스 백업(`d:\tmp\wie_saves\save2\{GameSave,SlotSave,OptionSave}.bin` → `Hero3GameSave_0/1`,
  `Hero3SlotSave_0/1`, `Hero3OptionSave/1`)을 그대로 넣으면 **세이브 로드해서 필드까지** 들어간다.
* 타이틀 통과 키 스크립트 = `--keys 260:OK,300:OK,360:OK`, 미니맵 = `*`(STAR).

## 현재 상태 (SLOT3 세이브=스킬 key1·3[id 10·13]/포션 key9 로 device 검증)
- ✅ **좌상단 HP/SP/EXP 바 + 레벨** — `overlay::draw_stats`. **★ 2026-07-27 둥근 캡슐 게이지로 재디자인**(얇게·rounded caps·`capsule_bar`).
- ✅ **게임 하단 HUD 제거** — `overlay::mask_game_hud`(하단 띠 + 좌상단 2스킬 박스를 black 으로 덮음).
- ✅ **아이템 아이콘** — 디코드된 item_bm 프레임(숫자 없음), 아이템 DB 라이브 조회. 버튼 9 포션 렌더 확인. **★ blit 알파-aware 바이linear 로 교체**(nearest 확대 뭉개짐 해소). **★ 2026-07-28 디코더 수정으로 색 정확해짐**(cw=15 crop).
- ✅ **스킬 아이콘 셀** — **★ 2026-07-27 RE 완료(wf_af12b0ab, 메인루프 disasm 재검증)**: cell 은 id 로 계산 불가한 **런타임 상태** → `items::skill_cell` 이 게임과 동일하게 라이브 읽음(아래 레시피). 기존 `min(id,8)` clamp 버그 제거.
- ✅ **스킬 아이콘 아트** — **★ 2026-07-28**: `_bm` 디코더 팔레트 오프셋 버그 수정 → 노이즈 아닌 실제 스킬 아트. 발명이던 12px crop/다크키 제거, 게임과 같은 raw 15×15. 시트 선택도 게임과 동일한 2-way(§ 최상단 섹션).

## 확정 RE 레시피 (구현됨)
- **CHAR** = `*(0x1b3084)` (=*(sl+0x444), sl=0x1b2c40); 라이브 0x1b7b6c. 힙=0x4000_0000+.
- **퀵바 5슬롯** (CHAR-relative stride1): type u8 @CHAR+0x252+s (1=skill,2=item) · item-ref u8 @+0x257+s (tab=>>5,slot=&0x1f) · skill-ref u8 @+0x25c+s (id=&0x1f, cat=>>6) · current @+0x261. 슬롯→핫키 [1,3,9,0,#].
- **스탯** (gauge 0x14a040): HP u16 @CHAR+0xf8 (max=상수 1000) · SP u16 @+0xf2 / maxSP u32 @+0xec · EXP u32 @+0xe4 / next u32 @+0xe8 · Level u16 @+0xe2.
- **아이템 아이콘** (icon-draw 0x1533a8, HIGH): cat==0x12 이면 frame = 라이브 `itemDB=*(*(grid_base+0x38))+8; rec=itemDB+idx*0x18; frame=u8@rec+0x14`. grid_base=`*(*(CHAR+0x9e28))+8`. frame=raw 0-based item_bm(29프레임). i18_dat 오프라인 확인: 포션0x12·하이포션0x13·미라클0x14·엘릭서0x15…오브원석0x1c (전부 <29).
- **스킬 시트/셀** — ⚠ **canonical = 최상단 2026-07-28 섹션**(게임 draw `0x14a51a`, 소스 `CHAR+0x9bf8` = **/skill/s{N}_bm**[setter `0x140448`], selector `(ref as i8)>>6 != 0` → 온필드 2시트 중 택1, **srcX=cell*15, 15×15**). 아래 2·3 의 "member 0..3 직접 인덱싱"과 상단 옛 블록의 "15×9"는 **정정됨**. 아래는 이전 RAM-dump 유도(SLOT3 에선 결과 동일):
  1. `cell = int8 @ CHAR+0x26c+slot` — **base-skill 스트립 칸(0..8) 그 자체** (bind writer 0x145cf0 이 직접 저장). `<0`=빈칸. (구 "레코드 인덱스→sk 체인→+0x173" 모델은 **오답=스킬 PICKER draw** 였음. 실제 HUD 은 cell=idx 직접.)
  2. `member = skill-ref[s] >> 6` (CHAR+0x25c+s 상위 2비트 = **파티 멤버 0..3**. ★ 카테고리 아님 — 장탄=dat-cat 2 인데 ref>>6=0 → 멤버가 맞음).
  3. `sheetId = u8 @ CHAR+0xd1 + member` (멤버0→7=s7, 멤버1→8=s8, …), **atlas sheet = sheetId−4** (s4..s10→0..6).
  4. blit `srcX = cell*15` (s{N}_bm 135×15 = 9칸).
  - **ground truth 검증**: 슬롯0 ref 0x6a idx 5 → 멤버1/s8/cell5 = **일심**; 슬롯1 ref 0x2d idx 5 → 멤버0/s7/cell5 = **장탄**. (s7=사격/연사/난사/곡예/저격/**장탄**/조준/냉정/속공 · s8 cell5=일심.)
  - 구현 = `items::skill_icon(read,char,slot)`. helper 0x19c280 = sheet 검증(4..10)·PICKER 경로. RAM dump 로 idx[0]=idx[1]=5·sheet[0xd1]=7·[0xd2]=8 확인.

## 직전 세션 변경 (2026-07-27 · 전부 구현+컴파일 완료 · APK ~48.9MB · device 재검증 대기)
사용자 피드백 4건 처리:
1. **아이콘 뭉개짐 (스킬+포션)** → ✅ `overlay::blit_icon` 을 **알파-aware 바이linear** 로 교체(`sample_bilinear`). nearest 1.5×radius 확대의 uneven 블록(뭉개짐) 해소·엣지 texel 은 **opaque 이웃만** 가중평균→실루엣 클린(다크 헤일로 0). 스킬+포션 둘 다 개선.
2. **스킬 프레임 오류** → ✅ RE(wf_af12b0ab)로 cell=런타임 상태 확정 → `items::skill_cell` 이 게임과 동일 라이브 읽기(위 레시피). 기존 `min(id,8)`(id10·13 둘 다 cell8 로 collapse) 버그 제거. 셀프테스트 5/5 PASS.
3. **HP/MP/EXP 바** → ✅ `overlay::draw_stats` 재디자인: 얇은 **둥근 캡슐 게이지**(`capsule_bar`, rounded caps·높이 game.h/60 clamp 5..14·색코딩 HP빨강/SP파랑/EXP앰버·thin rim). 각진 두꺼운 사각+border 제거.
4. **화면 꽉 채우기** (사용자 선택=**최대 확대**, 왜곡 없음) → ✅ `overlay::layout()`: 세로=게임 **풀-width**(구 60% cap→70%, 세로 letterbox 제거)·가로=게임 **풀-height**(구 60%width+2pad cap 제거, height-fit 최대)·컨트롤이 남는 양옆/아래 마진을 채움(검정 최소화). ⚠ **정직 한계**: 3:4 게임은 16:9 가로를 왜곡 없이 100% 채울 수 없음(=height-fit 이 최대). 진짜 와이드=별도 LARGE 트랙(`LANDSCAPE_FEASIBILITY.md`).

**device 재검증 게이트**(빌드는 됐으나 폰 실측 필요): (a) 바이linear 아이콘 룩(부드러움 OK? crisp 선호면 상수 스위치 용이) (b) skill_cell 포인터체인이 필드서 라이브 resolve 되는지(SLOT3 스킬 key1·3 → 올바른 셀?) (c) 캡슐 바 크기/위치 (d) 최대확대 레이아웃 여백 느낌.

## 확정 RE 레시피 요약 (구현됨) — 상단 "확정 RE 레시피" 참조.

## 파일 (미커밋)
- `wie_android/src/{lib.rs(QuickSlot/IconRef/Stats/UserEvent::Hud/quickslot_hotkey), atlas.rs(icons.bin 파서), items.rs(scan_quickbar/read_stats/item_db_icon/★skill_cell+skill_sheet/rd_i8), emulator.rs(Hud publish), overlay.rs(draw_quickbar/★blit_icon+sample_bilinear/★draw_stats+capsule_bar/mask_game_hud/★layout), app.rs}`
- `wie_android/src/icons.bin`(**82,998B** ★2026-07-28 재생성, include_bytes!) — `tools_re/gen_h3_icons.py`가 item_bm(29)+s4..s10_bm(7×9칸)을 **자체 정확 디코더**로 생성(repo convert_bm_v2 미사용 — 그 툴은 팔레트 2바이트 버그).
- `items_selftest.rs` — standalone 로직 테스트(`rustc --edition 2021 --test items_selftest.rs -o t.exe && ./t.exe`) **8/8 PASS**. `cargo test -p wie_android` 는 호스트에서 크래시(기존 이슈).
- 빌드=`build_android.ps1`(★ ps `$ErrorActionPreference=Stop` 이 cargo-ndk stderr "Building arm64-v8a" 를 오탐→cargo 단계서 abort. 회피=Bash/PS 로 `cargo ndk build`+strip+`gradlew :app:assembleDebug` 수동, 각 단계 `$LASTEXITCODE` 체크·`2>&1` 지양). APK=`android_app/app/build/outputs/apk/debug/app-debug.apk`. SLOT3 네비=title 2×OK→slot→joystick down-swipe×2(1=1DOWN)→OK.
- device dump=`push d:/tmp/dump_trigger("addr len")`→`run-as net.wie.emulator cp files/dump_trigger`→`exec-out run-as cat files/ram_dump.bin`.

원자료 RE workflow: wf_481f13bd(HUD/stat/mask), wf_9add52f0(id→frame), **wf_af12b0ab(스킬 셀=런타임 idx@0x26c→sk체인→cell@sk+idx*0x38+0x173·메인루프 disasm 재검증·2026-07-27)**. (구 wf_18ef675e=org spend limit 로 중단·wf_af12b0ab 가 대체·완료.)
