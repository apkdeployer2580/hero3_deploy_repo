# 브랜치 두 갈래 — `main`(개정판) / `vanilla`(편의성 개선 원본)

2026-08-31 확정. 2026-09-03 **정정 (77차-B)**: 아이템 **테이블은 양쪽이 같다.**
~~갈리는 것은 **드롭뿐**이다.~~ → **2026-09-19 (114차, 사용자 결정)**: 갈리는 것은 **셋** — ① 새 아이템
드롭(리버·검은 눈물·타이탄의 파편) ② 하늘성 입장(하드코어에서도 NPC 대화가 원본 그대로) ③ 레벨 상한
(main 99 / vanilla **70**). 호스트(에뮬·HUD·세이브·터치·**3종 가방**·데미지 32비트·제련 확률 표기)는 둘 다
같은 리메이크다. 셋째 줄 「완전 원본(오리지널)」은 **보류**(같은 날 사용자: *"오리지널은 나중에 구현할게"*).

정본은 이 파일. `CLAUDE.md` 와 `play-release` 스킬은 여기로 보낸다.

## 한 줄

| 브랜치 | 무엇을 싣나 | 누구에게 주나 | APK 이름 |
|---|---|---|---|
| **`main`** (개정판) | 같은 아이템 표 + **리버/보스악세 드롭** + **하늘성** + **레벨 상한 99** + 데미지 32비트(캡 9,999,999) + 3종 가방 | 기본. 일상 작업도 여기 | `hero3_lgt1.04_0.<vc>.0.apk` |
| **`vanilla`** (편의성 개선 원본) | **같은 아이템 표** + 그 드롭 **꺼짐** + 하늘성 **입장 불가**(하드에서도 NPC 대화 원본) + **레벨 상한 70** + 같은 캡 + 같은 3종 가방 | 새 컨텐츠 없이 편의 기능만 원하는 사람 | `hero3_lgt1.04_vanilla_0.<vc>.0.apk` |

`main` 이 기본인 이유: 앞으로 넣을 컨텐츠의 기본 줄이고, vanilla 는 "드롭을 안 켜는" 쪽이라
머지 실수가 싸다.

## 무엇이 갈리나

**아이템 테이블은 같다.** 양쪽 모두:

* `i4_dat`..`i11_dat` 끝의 `{리버…` 8행 (`tools_re/add_reaver_weapons.py`)
* `i12_dat` 끝의 `{타이탄의 파편`·`{검은 눈물` 2행 (`tools_re/add_boss_accessories.py`)
* 게스트 카운트 테이블 (`binary_patches.toml` VA `0x5e478`..`0x5e480`)
* 데미지 **32비트**(플레이어→몬스터 캡 **9,999,999**, 몬스터→플레이어 32767 — 102차; 옛 9999 → 32199 캡을 대체)

**`vanilla` 가 끄는 것 — 셋 (114차, 2026-09-19). 전부 `--cfg hero3_vanilla` 하나에서 갈린다:**

* ★★★ **하늘성 입장 불가** — 호스트 게이트 `wie_android/src/flavor.rs::TOWER`(= `!VANILLA`) 넷: 층 표
  `wie_guardian_floors.txt` 를 **읽지 않는다**(`Run::new(vec![])` → 영원히 `Idle`) · 제어 파일 `wie_guardian.txt`
  를 **읽지 않는다**(그 파일의 `hard 1` 노브가 대화를 바꾼다) · **HARD 미러 `0xc0` 를 언제나 0 으로**(main 세이브가
  들고 온 1 도 지운다 — 그래서 행인·로빈의 조건 컨테이너가 노말/하드 모두 **원본 블록**으로 떨어진다; 에셋은 안
  바꾼다) · ENTER 플래그는 지우기만 하고 `Command::Enter` 를 안 만든다(커서 리셋도 같이 꺼짐).
  종 훅 `tower_species_lgt` 는 양쪽에 설치되지만 런이 없으면 무장되지 않아 무동작이다.
* ★★★ **레벨 상한 70** — 111차의 게스트 패치 다섯(`0x4d6a8`·`0x4d6ac`·`0x4d628`·`0x4d664`·`0x4d668`)이
  `binary_patches.toml` 에서 **`flavor = "main"`** 을 달고 있어 vanilla 에서는 설치되지 않는다 → `doLevelUp` 이
  바이트 원본(70 클램프 · 포인트 276/44). 호스트 천장 `items::{LEVEL_CAP 99, STAT_POINT_CAP 392, SKILL_POINT_CAP 58}`
  은 **양쪽 그대로** — 천장은 게스트 클램프보다 느슨해도 되고 좁으면 안 된다(main 에서 만든 Lv 71+ 캐릭터가 vanilla
  에서 「캐릭터 없음」이 된다). 부팅 로그 = `Applied 46 patches`(main 51) + `not installing the main-only patch at 0x4d6..` 다섯 줄.
* **새 아이템 드롭 꺼짐** — `reaver_drop_lgt` (`binary_patches.toml` `pc = 0x38ffd`). 리버 8종·검은 눈물·타이탄의 파편이
  **한 훅**이라 이것 하나로 셋이 다 꺼진다. 원본 보스 드롭은 그대로. 세이브에 이미 있는 것은 로드·착용·표시된다.
* ★ **플레이버 차이를 더하는 규칙**: 게스트 **바이트 패치**는 toml 엔트리에 `flavor = "main"|"vanilla"`(pc 전용, 파서가
  오타를 거절), **훅**은 `hook.rs` 의 kind 매치, **호스트 기능**은 `flavor.rs` 의 이름 있는 상수 — ⛔ `cfg!(hero3_vanilla)`
  를 `wie_android` 다른 곳에서 직접 읽지 말 것. ⛔ 카운트 테이블 바이트에는 절대 태그를 달지 말 것(테이블은 공유).
* 부팅 로그 첫 줄 중 하나가 **`flavor: vanilla — …`** / **`flavor: main — …`** 다(`flavor::boot_line`) — 같은 패키지 ·
  같은 versionCode 수열이라 이 줄이 logcat 에서 플레이버를 가르는 유일한 통로다.
* ★ **78차-C 의 「하드 전용」 게이트도 그 훅 안에 있다** — 그래서 vanilla 에는 드롭 관점의
  영향이 없다. ⚠ 다만 **세이브 수정은 별개다**: 아래 「`vanilla` = `main` + 한 줄」 참조.
  2026-09-03 출고는 **main vc228 · vanilla vc229**(둘 다 같은 소스, 플래그만 다르다).
  **2026-09-19 출고는 main vc353 · vanilla vc354**(main `04db413` = 104차-B~110차-D). ⚠ **`git checkout main -- .` 은
  main 이 지운 파일을 안 지운다** — 99차가 지운 온라인 파일 아홉이 vanilla 에 남아 있었다(빌드엔 무해, `lib.rs` 가 안 부른다).
  재동기 뒤 `git diff main --stat` 이 **flavor + version 두 줄**인지 보고, 남은 파일은 `git rm`.
  **2026-09-13 출고는 main vc310 · vanilla vc311** (main `091d3c2` 트리 = 86차~91차-B + 90차-B 세이브,
  이 절차 그대로 — diff 는 flavor + version 두 줄). vanilla 는 vc277 이후 처음 따라잡았다.
  ⚠ 그때 워크트리의 에셋 zip 이 **vc277 시절 stale**(`8c8b982f…`)이었다 — gitignore 라 `checkout` 이
  안 건드린다. **main 것을 복사한 뒤 여섯 `--check`** 를 돌려야 테이블이 안 갈라진다.
  **2026-09-06 출고는 main vc276 · vanilla vc277**(main `5906747` 트리 = 81차·81차-B 패드 숫자, 이 절차 그대로 — diff 는 flavor 한 줄).
  옛 판: main vc272 · vanilla vc273(main `10c647a` 트리).
* 구현: `flavor.properties` 의 `flavor=vanilla` → `build_android.ps1` 이 rustc `--cfg hero3_vanilla` →
  `hook.rs` 가 `ReaverDropLgt` 를 설치하지 않고, `patch.rs::resolve_patches` 가 `flavor = "main"` 패치를 건너뛰고,
  `wie_android::flavor::TOWER` 가 false 다. 세 크레이트(`wie_core_arm`·`wie_util`·`wie_android`)의 `Cargo.toml` 이
  `[lints.rust] unexpected_cfgs … check-cfg = ['cfg(hero3_vanilla)']` 로 그 cfg 를 선언한다.
* 세이브에 이미 있는 리버/보스악세는 vanilla 에서도 **로드·착용·표시** 된다.

⛔ **테이블을 빼서 vanilla 를 만들지 말 것.** 카운트와 파일 행이 어긋나면 63차처럼
`Allocation failure` → 게임시작 검은 화면이다 (vanilla vc218 + main 세이브, §77차).

소켓확장·영약·캐시샵 최종티어는 갈라지기 **전에** 양쪽 플레이의 일부가 됐다.
테이블·카운트는 의도적으로 같고, 훅만 이쪽에서 끈다.

## ★★★ `vanilla` = `main` + `flavor.properties` 한 줄 (2026-09-03 정정)

~~**`main` → `vanilla` 로 호스트/테이블을 통째 머지하지 말 것** — 드롭 훅 커밋이 같이 들어온다.~~
**그 문구는 무효다.** 테이블을 *빼서* vanilla 를 만들던 시절 이야기이고, 77차-B 가 그 전제를
없앴다 — 지금 갈리는 것은 **`flavor.properties` 한 줄뿐**이고 드롭은 코드의 유무가 아니라
**빌드 플래그**로 꺼진다(`--cfg hero3_vanilla` → `hook.rs` 가 `ReaverDropLgt` 만 `continue`).

⛔⛔ **그 낡은 문구의 대가는 「회차마다 cherry-pick 을 손으로 쌓는 것」이고, 하나가 빠져 있었다**
(2026-09-03 발견): **77차 `on_slot_list`/`SLOT_LIST_SCREENS`** — 슬롯 목록(온라인 2 · 게임시작 9)에서
호스트가 요약 수리를 돌리면 게임이 같은 OK 에서 `readSaveRecord` 를 하는 중이라 월드 초기화가
물린다(「세이브 슬롯 누르면 게임시작이 멈춘다」). `origin/vanilla` 에서 `on_slot_list` 출현 **0회**.

⚠ **정정**: 같은 날 한 세션이 「78차 `275b071` 도 빠져 있다」고 적었는데 **틀렸다** — 그것은
**stale 한 로컬 `vanilla`** 를 본 것이고, `origin/vanilla` 에는 `6e3130d` 로 이미 cherry-pick 되어
있었다(`target_occupied` 출현 3회, 출고본 vc225 에 포함). 커밋 `482792d` 의 메시지가 그 상태를
정확히 적어 두었다 — 「생성핀 cherry-pick, 77차 §2 는 아직 없다」.
★ 교훈: **브랜치 상태를 판정하기 전에 `git fetch` 를 먼저 할 것.** 로컬 브랜치 ref 는 다른
체크아웃/세션이 밀어 둔 것을 모른다.

★ cherry-pick 을 쌓는 방식 자체가 이 사고의 원인이다 — 회차가 늘 때마다 사람이 「이번 것은
vanilla 에도 필요한가」를 판단해야 하고, 77차가 그 판단에서 빠졌다. 아래 절차는 그 판단을
아예 없앤다.

★ **그래서 vanilla 는 main 을 통째로 받아 오고 그 한 줄만 되돌린다** — 머지가 아니다.
`git merge main` 은 **충돌 10건**이 나고 그중 셋이 `emulator.rs`/`savevault.rs`/`vault.rs`,
즉 세이브 정책 파일 전부다(2026-09-03 `merge-tree` 실측). 손으로 푸는 순간 71차-E 의 사고
(「판정을 다른 파일에 흩뜨렸다」)를 재현할 자리가 생긴다. 충돌 0건짜리 절차는 이것이다:

```powershell
git checkout vanilla
git checkout main -- .                                   # main 트리를 통째로
git checkout HEAD -- android_app/flavor.properties       # 그 한 줄만 vanilla 로
git commit
```

★ `flavor.properties` 는 **merge base 이후 vanilla 만 건드렸다** → 설령 머지해도 git 이
`flavor=vanilla` 를 자동으로 지킨다(실측). 드롭이 다시 켜질 위험은 **그 파일이 `flavor=` 로
돌아갈 때뿐**이므로, vanilla 를 건드린 뒤에는 항상 그 줄을 눈으로 확인할 것.

## 이름 규칙

파일 이름은 `android_app/flavor.properties` 의 `flavor=` 가 정한다.
`name_apk.ps1` 이 읽고, `-Flavor` 로 덮을 수 있다.

```
hero3_lgt1.04_0.<vc>.0.apk            ← main  (flavor 빈 칸)
hero3_lgt1.04_vanilla_0.<vc>.0.apk    ← vanilla
```

`lgt1.04` 와 `0.<vc>.0` 규칙은 `play-release` 스킬 §1-b 그대로다. 토큰만 한 칸 끼운다.
⛔ `vanilla` 빌드에 토큰을 빼지 말 것 — `dist/` 에서 어느 줄인지 안 보인다.

## 설치 신원은 하나다

둘 다 `applicationId = com.hero3remake.app`, 같은 릴리스 키. 기기에 깔면 **서로
덮어쓴다**. 세이브는 유지되고, **테이블이 같으므로** 줄을 갈아타도 월드가 열린다.

그래서 **`versionCode` 는 두 브랜치가 하나의 수열**이다. 어느 줄을 구워도
`android_app/version.properties` 가 오른다. 다른 줄은 그 숫자 **이상으로** 맞춘 뒤
다음 `-Release` 를 돌린다 — 안 맞추면 두 줄이 같은 코드를 내고 한쪽 설치가 거절된다.

```
vanilla 에서 210 을 냈다  →  main 의 version.properties 도 210
다음 main -Release        →  211
```

## 작업 흐름

일상은 **`main`**.

| 하는 일 | 어디에 |
|---|---|
| 새 아이템 **행·카운트·캡** | **양쪽 같이** (테이블이 같아야 한다). `main` 에 넣고 vanilla 에 같은 패치·에셋을 옮긴다 |
| 그 아이템의 **드롭** | **`main` 만** (`reaver_drop_lgt` 또는 그 후손). vanilla 는 `--cfg hero3_vanilla` 가 훅을 건너뛴다 |
| **하늘성** 코드·층 표·입장 NPC 에셋 | `main` 에 넣는다(에셋·훅은 양쪽에 실린다). vanilla 는 `flavor::TOWER` 가 통째로 끈다 — 새 진입 경로를 만들면 **그 게이트를 같이 물을 것** |
| **레벨/포인트 상한**처럼 vanilla 가 원본을 지켜야 하는 게스트 바이트 | toml 엔트리에 **`flavor = "main"`**. 호스트 천장은 양쪽 공통(느슨한 쪽) |
| 에뮬·HUD·세이브·터치·오디오·서버 | `main` 에 넣고, **vanilla 를 위 절차로 main 트리에 다시 맞춘다** |

⛔⛔ **~~드롭이 안 섞인 커밋만 vanilla 에 cherry-pick~~ 하지 말 것** (2026-09-03 폐기). 그 방식은
회차마다 사람이 「이번 것은 vanilla 에도 필요한가」를 판단하게 만들고, **77차가 그 판단에서
빠져서 vanilla 에 게임시작 멈춤이 남아 있었다.** 드롭은 커밋의 유무가 아니라 **빌드 플래그**로
꺼지므로 골라 옮길 이유가 없다 — 위 「`vanilla` = `main` + 한 줄」 절차를 쓸 것.
⛔ `git merge main` 은 여전히 하지 말 것. 이유가 바뀌었다: 드롭 훅이 아니라 **충돌 10건**,
그중 셋이 세이브 정책 파일 전부다.

에셋 zip 은 gitignore 다. 줄을 갈아탈 때 **같은 도구로 다시 굽는다** (양쪽 `patch_asset.py` 가
리버·보스악세 행을 묶는다):

```powershell
python tools_re/patch_asset.py android_app/app/src/main/assets/hero3_fixed.zip --apply
python tools_re/patch_item_prices.py android_app/app/src/main/assets/hero3_fixed.zip --apply
python tools_re/patch_asset.py android_app/app/src/main/assets/hero3_fixed.zip --check
```

`--check` 가 i16 과 i12(보스악세) 를 본다. 카운트 패치 없이 행만 있으면 63차다.

## 출고

```powershell
# 어느 줄이든 (99차부터 HERO3_SERVER_* 는 읽지 않는다 — 100 % 오프라인)
.\build_android.ps1 -Release
.\.claude\skills\play-release\name_apk.ps1    # flavor.properties 를 읽는다
```

건네기 전에 파일 이름에 `vanilla` 유무와 `versionCode` 를 본다. 기기 확인은 항상
`adb devices` → `dumpsys package com.hero3remake.app | grep versionCode`.
이름만으로는 깔린 줄이 안 보인다 — 같은 패키지다.
`vanilla` 부팅 로그(114차부터):
* **`flavor: vanilla — 하늘성 OFF …`** 한 줄(main 은 `flavor: main — …`).
* `Hook installed at 0x38ffd: ReaverDropLgt` 가 **없고** 대신 `vanilla: not installing the reaver drop hook at 0x38ffd`.
* **`Applied 46 patches`**(main **51**) + `not installing the main-only patch at 0x4d6a8/0x4d6ac/0x4d628/0x4d664/0x4d668` 다섯 줄;
  `Patch applied at 0x4d6..` 는 **0줄**. `Patch applied at 0x5e478` / `0x5e480` / `0x5e482` 는 **있어야** 한다(테이블).
* `guardian: … floor(s) loaded` 가 **없다**(층 표를 push 해 둔 기기에서도) · `guardian: hard-mode mirror … -> 1` 이 **없다**.

★ **기기 없이 APK 만으로 확인하는 법** (2026-09-03 사용): `lib/arm64-v8a/libmain.so` 안에
문자열 **`not installing the reaver drop hook`** 이 있으면 cfg 가 실제로 켜진 것이다 —
⛔ **이 머신에는 `strings` 바이너리가 없다**(2026-09-13). 쓰면 양쪽 다 0 이 나와서 **vanilla 가
main 처럼 보인다**. `grep -c "not installing the reaver drop hook" <libmain.so>` 로 바이너리를 직접
읽을 것 — 실측 main **0** / vanilla **1**. 검산: ~~`grep -c "<hero3-server>"`~~ → 99차부터 그 문자열은
어느 쪽에도 없다(서버 목적지 삭제). 대신 `grep -c "100% OFFLINE"` 가 양쪽 **1** 이어야
그 방법이 동작한 것이다(안 그러면 검사 자체가 무효다). —
main 빌드에서는 `cfg!(hero3_vanilla)` 가 컴파일 타임 `false` 라 그 블록이 통째로 제거되어
**없다**. 그리고 `assets/hero3_fixed.zip` 의 jar 멤버(1,290 + 7 = 1,297개)가 main 것과
**바이트 동일**해야 한다 — 하나라도 다르면 테이블이 갈라진 것이다.
