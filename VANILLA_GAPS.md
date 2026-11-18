# 바닐라(`vanilla`)에서 꺼져 있는 것 — 관리 목록

2026-09-20 신설(사용자 지시: *"현재 바닐라에서 안되는 항목들 이제 리스트로 관리할꺼야"*). **정본은 이 파일**이고
`BRANCHES.md` 가 여기로 보낸다. 바닐라 = `main` 트리 + `android_app/flavor.properties` 의 `flavor=vanilla` 한 줄
(→ rustc `--cfg hero3_vanilla`). 그 플래그 하나가 아래 항목을 전부 가른다 — 코드에 다른 갈림은 없다
(`wie_android/src/flavor.rs` 머리말이 같은 목록을 영어로 든다).

⛔ 항목을 더하거나 빼면 **이 표 · `flavor.rs` 머리말 · `flavor::boot_line` 문구**를 같이 고칠 것. 부팅 로그의
`flavor: vanilla — …` 한 줄이 기기에서 플레이버를 가르는 유일한 통로다(같은 패키지, 같은 versionCode 수열).

## 표 (2026-09-20 신설 · V10~V16 = 2026-10-05 163차, vanilla vc442 = main vc441 트리 기준)

| # | 항목 | main | vanilla | 어디서 갈리나 | 근거 회차 | 상태 |
|---|---|---|---|---|---|---|
| V1 | **하늘성 입장** (엔자크 행인 씬 111 `A[10]` · 네오 솔티아 로빈 씬 5 `A[3]` 의 하드 대화 → 입장) | 하드코어에서 대화가 바뀌고 예 → 1층 | **입장 불가** — 노말/하드 모두 **원본 대화** | `flavor::TOWER` 게이트 셋(`emulator.rs`): HARD 미러 `0xc0` 언제나 0(main 세이브의 1 도 지움) · ENTER 플래그는 지우기만 · 입장 확정 커서 리셋 없음 | 114차 (사용자 결정) | 의도된 차이. 에셋(대화 블록)은 양콪 동일 — 하드 블록은 있으나 조건이 영원히 거짓 |
| V2 | **하늘성 층 진행** (층 표 · 제어 파일 · 포크 · 리스폰 차단 · 클리어 EXP · 보스 층 보상) | `wie_guardian_floors.txt` 100층 · `wie_guardian.txt` 트리거 | **전부 무동작** — 층 표·제어 파일을 **읽지 않는다**(`Run::new(vec![])` → 영원히 `Idle`), 폴 블록 자체가 안 돈다 | `flavor::TOWER` — `emulator.rs` `Run::new` · 제어 파일 리더 | 114차 | 의도된 차이. `wie_guardian.txt` 의 `hard 1` 테스트 노브도 같이 죽는다 |
| V3 | **하늘성 종 훅 `tower_species_lgt`** | 층마다 종 교체 | **설치는 되지만 무장되지 않음**(런이 없으니 `TOWER_SPECIES` 가 영영 미설정) | `hook.rs` 양쪽 설치 · `emulator.rs` `Action::Warp` 에서만 무장 | 88차 · 114차 | V2 의 귀결. 훅 자체는 무해(`scene != 로드 중 씬` 이면 통과) |
| V4 | **새 아이템 드롭** — 리버 8종(`{리버…`, cat 0x04..0x0b idx 끝행) · `{검은 눈물`·`{타이탄의 파편`(cat 0x0c idx 0x28/0x29) | 하드 전용 킬당 5 % / 보스별 5 % | **훅 미설치** → 드롭 0 | `hook.rs`: `cfg!(hero3_vanilla) && ReaverDropLgt → continue` (`binary_patches.toml` `pc = 0x38ffd`) | 75차 · 78차-C · 77차-B | 의도된 차이. 세이브에 이미 있는 것은 **로드·착용·표시된다**(테이블은 공유) |
| V5 | **탑 안 드롭 0** (`tower_no_drops` 분기 — 같은 훅 안) | 런 중 재고 드롭·리버 전부 막힘 | 훅이 없으니 분기도 없음 — 어차피 런이 없어 **무관** | 같은 훅 | 2026-09-11 | V2·V4 의 귀결 |
| V6 | **레벨 상한** | **99** (`doLevelUp 0x4d6a8/0x4d6ac` 0x46 → 0x63) | **70** (바이트 원본) | `binary_patches.toml` 다섯 패치 `flavor = "main"` → `patch.rs::resolve_patches` 가 건너뜀. 부팅 로그 `Applied 46 patches` + `not installing the main-only patch at 0x4d6..` 다섯 줄 | 111차 · 114차 | 의도된 차이 |
| V7 | **스탯/스킬 포인트 클램프** | 392 / 58 (`0x4d628` · `0x4d664/0x4d668`) | **276 / 44** (원본) | 위 다섯 패치 중 셋 | 111차 | V6 의 짝. ⛔ 호스트 천장 `items::{LEVEL_CAP 99, STAT_POINT_CAP 392, SKILL_POINT_CAP 58}` 은 양쪽 그대로 — main 의 Lv 71+ 세이브가 vanilla 에서 「캐릭터 없음」이 되지 않게 |
| V8 | **하늘성 보상 재료 `{닉스코어의 파편`**(cat 0x0e idx 0x2e) | 보스 층 클리어 지급 | 에셋(`i14_dat` 47행)·게스트 카운트 3바이트는 **있으나 획득 경로 없음** | 에셋 체인은 양쪽 동일(`patch_asset.TARGETS`) · 지급은 V2 의 `Action::Reward` | 85차 | V2 의 귀결. 표를 빼면 63차식 `Allocation failure` — ⛔ 빼지 말 것 |
| V9 | **하늘성 상태줄·토스트·점멸** (`하늘성 N층 남은 M` · 「하늘성 입장」 · 흑백 점멸) | 런 중 표시 | 런이 없어 **안 뜬다** | `Run::status_line` = `None` when `Idle` | 2026-09-09 §I | V2 의 귀결 |
| V10 | **쿨타임 감소 합 상한 60 %** (158차 P2) | 합 60 % 에서 멈춤 | **원작** — 합 100 % 이상이면 1틱 | toml `cooldown_cap_lgt` `flavor = "main"` · 호스트 쿨타임 링 `items::cd_reduce_cap`(main 60 · vanilla 100, `flavor::REBALANCE`) | 158차 · 163차 (사용자: 「바닐라에서는 쿨감 제한 적용되면안돼」) | 의도된 차이. ★ 원작 **폭풍의 오브**(0x11, 홈 5개 +90 %)로 바닐라도 닿는다 |
| V11 | **명중 합 255 클램프** (158차 P1) | 256+ 를 255 로 | **원작** — u8 로 접힘 | toml `hit_sum_clamp_lgt` `flavor = "main"` | 158차 · 163차 | 의도된 차이(사용자 패치노트상 개정판). 원작 **사신의 오브**(0x09, +120)로 닿는다 |
| V12 | **방어% 합 상한 500** (160차 T2) | 500 에서 멈춤 | **원작** | toml `def_pct_cap_lgt` ×2 `flavor = "main"` | 160차 · 163차 | 의도된 차이. 원작 **금강/정령**(+225 %)으로 닿는다 |
| V13 | **공격력·방어력 i16 해제** (160차 T1) | 32,767 위도 전투에 진짜 값 | **원작**(i16 절단) | toml `stat_wide_lgt` ×4 · `def_read_wide_lgt` · `atk_read_wide_lgt` ×2 `flavor = "main"` | 160차 · 163차 | 원작 장비로는 32,767 에 못 닿아 체감 차이 없음 |
| V14 | **방패무시 작동** (156차) | `raw += def × pct/100` | **원작** — 합산만 하고 안 읽음 | 호스트 `emulator.rs` 가 바닐라에선 `set_pierce(0, false)` (`flavor::REBALANCE`) | 156차 · 163차 | 원작 **직격의 오브**(0x12, +90 %)가 바닐라에서 살아나던 것을 되돌림 |
| V15 | **공격속도 · 장신구 3번째 옵션 훅** (131차 · 150차) | 설치 | **미설치** | toml `atk_speed`/`atk_event`/`atk_recover`×2 · `acc_third_stat`/`acc_summary`×3 `flavor = "main"` | 163차 | 해당 아이템을 바닐라에서 못 얻어 원래 무동작 — 개정판 기능이라 뺐다 |
| V16 | **데미지 상한** | 9,999,999(플레이어→몬스터) · 32,767(몬스터→플레이어) | **원작 9,999 · 9,999** | toml 상한 리터럴 `0x4beac` · `0x385cc` `flavor = "main"` | vc204(리버, 9,999 → 32,199) · 102차 · 163차 | 의도된 차이. ★ 접기 패치는 양쪽 — 바닐라에서 32,768+ 가 원작의 「1」 대신 9,999 |

**같은 것(갈리지 않음 — 확인용)**: 3종 가방(오브·재료·소비 + 귀환서 팝업 통과 · 부활의서 대여) · 데미지 32비트 접기(캡은 V16) · **흡수 32,767+ 수정(163차)** · 최대 HP 게이지 훅(142차 — 바닐라에선 원작 장비로 무동작, 회복 합 32,767 접힘 사망을 막는다; 뺄지는 사용자 결정 대기) · 제련 확률 표기 ·
터치 메뉴 전부 · 퀵슬롯 7칸 · 세이브 금고 · 오프라인 창고/캐시샵 · 아이템 테이블(리버·보스악세·닉스코어 행 포함) · 상점 상태 바이트 게이트 ·
119차 스틱 걷기 · 에셋 zip(md5 동일).

## 운영 규칙

* **출고 짝**: main `-Release` 뒤 곧바로 vanilla 를 같은 트리로 굽는다(`BRANCHES.md` 절차: 워크트리 `.claude/worktrees/vanilla-build` 에 `git checkout main -- .` → `flavor=vanilla` 되돌림 →
  에셋 zip·`version.properties`·`local.properties` 복사 → `-Release` → APK 를 main 의 `dist/` 로 → main `version.properties` 를 그 번호로 정렬). ⛔ `git checkout main -- .` 은 main 이 지운 파일을
  안 지운다 — 뒤에 `git diff main --stat` 로 flavor(+version) 두 줄인지 확인.
* **바닐라는 어느 기기에도 안 깐다**(사용자 결정 2026-09-19, TODO `DC`) — 파일로만. 확인이 필요하면 S9 에 깔고 부팅 로그 `flavor: vanilla` 만 보고 main 으로 되돌린다.
* **검증 셋**: `libmain.so` 에 `not installing the reaver drop hook` 가 **1**(main 0) · 에셋 zip md5 가 main 과 동일 · 부팅 시 `Applied 58 patches`(main 65) · `not installing the main-only hook` **19줄**(163차부터; 목록은 `wie_core_arm` 테스트 `the_main_only_sites_are_exactly_the_kaejeongpan_balance` 가 고정).
* 항목이 늘면(예: 오리지널 플레이버, 바닐라 전용 밸런스) 여기 V-번호를 잇고 `flavor.rs` 에 이름 있는 상수를 더한다 — ⛔ `cfg!(hero3_vanilla)` 를 `wie_android` 다른 곳에서 직접 읽지 말 것.

## 이력

| 날짜 | vanilla | main 트리 | 메모 |
|---|---|---|---|
| 2026-10-10 | **vc458** | `67f0731` + 미커밋 173~176차 − 쿨타임 상한 60→70 변경분(main vc457 트리 그대로; 60→70 은 vc457 에도 없다) | 사용자 지시 *"바닐라에도 적용해줘. 지금 작업한 쿨타임 상한 60 에서 70으로 올린거는 빼고 적용하면 돼. 바닐라에는 쿨타임 상한 없어"*. 165~176차(3종 가방 흡입 · 아바타 옷장 167~176 · 패드 버튼 설정 173·174)는 플레이버 무관이라 그대로 · V1~V16 무변경(toml · `hook.rs` · `flavor.rs` = HEAD) · 할로윈 세트(04, 헬 엔딩 보상)는 바닐라에서 얻을 길 없음 · 에셋 `2d711bec…` main 과 동일 · md5 `c03ba1aa…` · 28,279,674 B · `.so` 대조(vanilla / main vc457) = `flavor: vanilla` **1/0** · `not installing the reaver drop hook` **1/0** · `not installing the ending label hook` **1/0** · `floor(s) loaded` **0/2** · `costume sheet(s)` · `wie_avatar_grant.txt` · `padmap` 1/1 · 내장 toml 「쿨타임 감소 60 %」 1/1 · 헤드리스 부팅 안 함 · 기기 미설치 · `vanilla` 브랜치 미갱신(트리 미커밋) |
| 2026-10-05 | **vc442** | 미커밋 트리(163차 + 164차) = main vc441 과 같은 트리 | 163차 사용자 지시 *"바닐라에서는 쿨감 제한 적용되면안돼. 신규템으로 인한 밸런스 관련 패치했던거 적용되면 안되는거야"* → **V10~V16 신설**(훅 19 · 상한 리터럴 2 · 호스트 `REBALANCE`). 흡수 수정(양쪽) · 164차 포션(양쪽)도 실림 · 에셋 `00cdaef1…` main 과 동일 · md5 `056b6925…` · `.so` 대조(vanilla / main) = `flavor: vanilla` **1/0** · `not installing the reaver drop hook` **1/0** · `개정판 balance OFF` **1/0** · `floor(s) loaded` **0/2** · `AbsorbLgt` 1/1 · 헤드리스 바닐라 프로브 = `Applied 58 patches` · `not installing the main-only hook` 19줄 · 데미지 9,999 · 흡수 정상 · 기기 미설치 · `vanilla` 브랜치 미갱신(트리 미커밋) |
| 2026-10-03 | **vc440** | `e673fc9` (162차 · 161차 포함 = main vc439 와 같은 트리) | 사용자 지시 *"바닐라에도 적용해"*. 162차(드롭 수명 6 → 60 초 패치 둘 · 루팅 30칸 상한 훅)는 플레이버 무관 · 161차 슬롯 포커스도 함께 · V1~V9 무변경(게이트 셋 이 회차 무변경) · 에셋 `00cdaef1…` main 과 동일 · `.so` 대조(vanilla / main) = `flavor: vanilla` **1/0** · `not installing the reaver drop hook` **1/0** · `not installing the ending label hook` **1/0** · `floor(s) loaded` **0/2** · `LootSpawnCapLgt` · `loot: the field already holds` **1/1** · `100% OFFLINE` 1/1 · md5 `0d05d1f0…` · 기기 미설치(파일만) · `vanilla` 브랜치 `d434da4`(트리 + flavor 한 줄) |
| 2026-10-03 | **vc437** | 160차 커밋(main) — T1 i16 해제 · T2 방어% 500 · T4 보스 스킬 포크 | 사용자 지시 *"바닐라에도 반영"*. 160차 훅 아홉은 **양쪽 설치**(상한 아래 · i16 안 = 바이트 동일 — 신규 아이템 없이는 닿지 않는 수치) · V1~V9 무변경(게이트 셋 이 회차 무변경 확인) · 에셋 `00cdaef1…` main 과 동일 · `.so` 대조(vanilla / main) = `flavor: vanilla` **1/0** · `flavor: main` 0/1 · `not installing the reaver drop hook` **1/0** · `not installing the ending label hook` **1/0** · `floor(s) loaded` **0/2**(탑 로더 미링크) · 160차 훅 넷 `StatWideLgt`·`DefPctCapLgt`·`AtkReadWideLgt`·`DefReadWideLgt` **1/1** · 158차 `HitSumClampLgt`·`CooldownCapLgt` 1/1 · `100% OFFLINE` 1/1. md5 `c9a18c0c…` · 27,990,349 B · 브랜치 `c0258cb`. 워크트리 `vanilla-build` 에 `read-tree` 로 받아 구움 · 기기 미설치 |
| 2026-10-03 | **vc434** | `cef52be` + 작업 트리의 미커밋 157차 · 158차 (= main vc433 과 같은 트리) | 사용자 지시 *"바닐라에도 적용해줘"*. 158차는 전부 플레이버 무관(명중 255 클램프 · 쿨타임 상한 60 % 훅 둘 · 아이템 행 열넷 · 기습 ×2 · 소비가방 loan) — 탑 굴림 포크(P8)만 V2 의 귀결로 vanilla 에선 돌 일이 없다. `dist/hero3_lgt1.04_vanilla_0.434.0.apk` md5 `be3971b9…`, 27,979,949 B · `.so`: `flavor: vanilla` 1 · `not installing the reaver drop hook` 1 · `not installing the ending label hook` 1 · `HitSumClampLgt`/`CooldownCapLgt` 1 · `floor(s) loaded` 0 · 에셋 `00cdaef1…` = main 과 동일. 어느 기기에도 안 깔았다(파일만). ⚠ `vanilla` 브랜치(`973f779`, vc390)에는 아직 없다 — 트리가 미커밋이라 BRANCHES.md 절차는 main 커밋 뒤 |
| 2026-09-20 | **vc372** | `150f467` (118차-B) + 작업 트리의 미커밋 119차 7파일 | 이 목록 신설. 사용자 지시로 다른 세션의 119차 WIP 도 실었다(main vc371 과 같은 내용) |
| 2026-09-19 | vc363 | `8036efe` (116차) | |
| 2026-09-19 | vc361 · vc359 · vc354 | 115차 · 114차 · 110차-D | 114차에 플레이버 게이트 셋 확정 |
| 2026-09-13 | vc311 | `091d3c2` (91차-B) | |
| 2026-09-06 | vc277 | `5906747` (81차-B) | |
