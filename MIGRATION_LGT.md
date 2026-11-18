# LGT판 (binary.mod, Ver 01.00.07) 마이그레이션 — canonical

> 2026-08-10 시작. KTF `client.bin`(Ver 01.00.03)에서 리버싱한 모든 게스트 결합
> 기능을 LGT `binary.mod`(Ver 01.00.07, 인게임 표기 "VER 1.0.4")로 옮기는 작업의
> 정본 문서. 진행분은 여기에 append 한다.

## 0. 빌드 식별

| | KTF (기존 타깃) | LGT (마이그레이션 대상) |
|---|---|---|
| 패키지 | `__adf__` + `0103EFD4.jar` | `app_info` + `0002865E.jar` |
| PID / AID | PD007102 / 0103EFD4 | PD119037 / 0002865E |
| Ver | 01.00.03 | 01.00.07 (인게임 "VER 1.0.4") |
| 배급 | 일렉트로닉아츠코리아(유) | (주)핸즈온모바일코리아 (LGT ez-i, LG-LH8000) |
| 게스트 바이너리 | `client.bin64000` 735,760 B raw ARM | `binary.mod` 519,952 B **ELF** (2009-03-10) |
| md5 | `3792a476ae92663e5dcdf320bb388b31` | `6a352944bb43c0789ca9f4b6f0702f4b` |

★ LGT 열은 **01.00.07 기준**이다. 유통되는 LGT판에는 리비전이 하나 더 있고
(01.00.04, 인게임 "VER 1.0.1") 게스트 주소가 다르다 — **§2.6 을 먼저 읽을 것.**
| 주소 모델 | 게스트 = fo + 0x100000, RWPI `sl` = 0x1b2c40 | `.text` va 0x1000 (fo 0x34, 크기 0x5db85), `.data` 0x1400000, `.bss` 0x1500000, **절대 주소** (RWPI 아님) |
| 세이브 DB | `Hero3GameSave_N` / `Hero3SlotSave_N` / `Hero3OptionSave` | 문자열 자체가 없음 — **미리버싱** |

## 1. ★ 핵심 사실: 같은 소스, 완전히 다른 컴파일

KTF 게임로직 영역을 4 KB 간격으로 152개 표본을 떠서 LGT에서 16바이트 정확일치를
찾으면 **0개**다 (8바이트도 2개). BL/리터럴로드를 와일드카드한 퍼지 매칭도 전패.
세이브 DB 이름 문자열조차 다르다. 즉 **KTF 주소는 단 하나도 패턴 이식이 안 되고,
게스트 결합 기능 전부가 LGT 바이너리 신규 RE 대상이다.** (스크래치의
`map_anchors.py` 실험 — 방법론 기록용으로만 의미 있음.)

다만 **게임 의미론은 동일**하다: 블리터 하나를 명령 단위로 대조한 결과 인자
13개의 순서, 포맷 코드(0x0b=4bpp/0x0c=8bpp), 픽셀 산술까지 같았고 자료구조
간접참조 방식만 달랐다(§3). 그래서 "KTF RE 문서 = LGT RE의 지도"는 성립한다.

## 2. 이번 회차(1차)에서 끝난 것 — 전부 프로브 검증 완료

1. **LGT 부팅 확인**: vendored wie의 `LgtEmulator`가 이 게임을 그대로 돌린다.
   타이틀 → 슬롯 → 클래스 선택 → 인트로 → 오프닝 전투 컷신까지 프로브로 실측.
   wie_probe가 아카이브를 자동 감지한다(KTF `__adf__` / LGT `app_info`).
2. **LgtEmulator 호스트 API** (`vendor/wie/wie_lgt/src/emulator.rs`):
   `read_memory`/`write_memory`/`call_guest_function(_out)` — KTF와 동일 계약,
   LGT 클렛 이벤트와 같은 `run_function` 메커니즘. JVM attach 가드는 불필요
   (LGT엔 ARM쪽 AOT 자바가 없음).
3. **LGT 로더에 바이너리 패치 설치** (`vendor/wie/wie_lgt/src/runtime/init.rs`):
   `install_binary_patches(core, binary.mod 전체, 실행 섹션들)`. 해시 규약은
   KTF와 동일하게 "파일 전체 md5".
4. **blit565 LGT 포팅** — 성능 패치의 이식 1호.
   * LGT 블리터 = **`0x2dd7c`** (훅 pc `0x2dd7d`). 발견 방법은 KTF 때와 동일:
     프로브 `--profile 1`로 오프닝 전투를 돌리면 비-idle 리프 PC 상위가
     `0x2e59c/0x2e5b4/0x2e5c2`(~16%) — 이 함수의 내부 루프다.
   * KTF와의 유일한 차이 = 스프라이트 기술자: `+8`(픽셀)/`+0xc`(팔레트)가
     KTF는 할당자 핸들 이중참조(+8바이트 헤더), LGT는 **직접 포인터**
     (팔레트는 엔트리를 바로 가리킴). 픽셀 산술은 동일:
     `pixels = data + 2*palette_len + 9`, 4bpp 행바이트 = ceil(pitch/2),
     홀수 srcX는 LOW 니블 시작.
   * 구현: `blit565::SpriteLayout { KtfHandles, LgtDirect }` + 훅 종류
     `blit565_lgt` + toml 엔트리(해시 `6a352944…`).
   * **검증**: 같은 입력으로 `--hle 0/1` 프레임 4장 바이트 단위 동일,
     호출의 99.5% HLE 처리, 게스트 명령 1.3 M/frame → **0.7 M/frame (-40%)**.
     KTF 회귀도 동일 방식으로 통과(프레임 동일, 훅 100% 처리).
5. **빌드 게이트** (`wie_android/src/guest_build.rs`): 부팅한 아카이브 종류를
   기록하고, KTF가 아니면 `items::resolve_char`(모든 введ스펙션의 단일 관문)가
   `None`, `apply_quickpage_actions`(유일한 무검증 게스트 호출)가 no-op.
   LgtEmulator에 호출 API가 생긴 순간부터 바이트 검증 가드만으로는 확률적
   안전이라 이 게이트가 필요해졌다. blit565는 이 게이트와 무관(해시 키).

## 2.5 2차에서 확보한 것 (정적 앵커 + RE 파이프라인 검증)

### RE 파이프라인 — LGT에서 결정적으로 동작 확인

프로브에 **`--dump-mem base:len[,…]`** 추가(`wie_probe/src/main.rs`): `--dump`
프레임마다 게스트 메모리 구간을 `mem_<frame>_<base>.bin` 으로 저장한다. LGT 통계
영역 기본값 = `--dump-mem 0x1400000:0x604,0x1500000:0x12958` (.data / .bss).

**검증(worked example)**: 클래스 상세 화면에서 입력만 다른 두 결정적 런을 뜨고
.bss 를 diff → **정확히 1워드**만 달라졌다. 즉 "값 하나만 바꾸는 입력 → 단일 워드
격리"가 KTF 때와 똑같이 LGT 에서도 성립한다. 재현:
```
K=620:OK,700:OK,780:OK,860:OK,940:OK
wie_probe --zip <lgt game.zip> --w 240 --h 320 --frames 1120 --ms 20 --keys $K \
  --dump-mem 0x1400000:0x604,0x1500000:0x12958 --out A --dump 1100
wie_probe ... --keys $K,1000:RIGHT,1040:RIGHT --out B --dump 1100
# A/B 의 mem_01100_0x01500000.bin diff → 0x150ea78 한 워드
```
바뀐 워드 = **`.bss 0x150ea78`** (클래스 상세 화면이 들고 있는 표시-데이터 포인터).
값이 `.text` 안의 클래스 테이블(엔트리 `~0x4e14c`, 스트라이드 `0x7d0`)을 가리킨다.
→ 클래스 테이블 위치가 부수적으로 확보됨. (주의: 이 화면의 RIGHT 는 클래스 자체가
아니라 이 포인터를 움직인다 — 화면상 클래스명은 그대로였다.)

### 세이브 DB 이름 (KTF 와 다름 — 세이브 체인 이식의 출발점)

binary.mod 문자열(fo 0x5abe8 근방):
* 게임: **`GameSave`** + `%s_%d` → `GameSave_<slot>` (KTF 의 `Hero3` 접두사 **없음**)
* 슬롯요약: **`SlotSave`** → `SlotSave_<slot>`
* 옵션: **`OptionSave`**
* 포맷 문자열 `%s_%d` (fo 0x5ba48), `%d.mmf`(사운드), 사운드엔진 `Yamaha_MA3`
KTF 의 `save.rs` DB 이름을 이 이름들로 바꾸는 것이 세이브 체인 이식의 1단계다
(단, 레코드 직렬화 함수 주소·레이아웃은 별도 RE 필요).

### 함수/자료구조 이름 힌트 (디버그 문자열, fo 0x5b6xx~0x5cbxx)

`onEventMessageOkKey()`, `pMain->pNpcQueue[objID].moveType`, `shotAttackCnt`,
`Event_freeID` / `Map_freeID` / `freeBossType` / `freeEnemy` / `Hero_Free` /
`freeItemMgr` / `freePopUpMessage` / `ResMgr_FreeGame` / `GMenu_FreeResource`,
`====> frameBuf is NULL`, `===> FileRead/FileWrite Error`, 리소스 경로
`res/dat/{char,enemy,drop,shop,smith,getitem}_dat`, `res/menu/*`. 이 문자열들의
XREF(어느 코드가 참조하나)을 lgtdis 로 따라가면 해당 루틴 진입점을 특정할 수 있다.

### 서버 연동 힌트

사설서버 IP `210.222.18.28` / `210.222.18.8`, 앱키 `hero3lgt_wipi`, SMS 관련
`PHONENUMBER`/`SMS HERO3`. `hero3_server` 연동 이식 때 참고.

## 2.6 3차 — 앱을 실제로 LGT로 전환 (2026-08-10, 기기 실측 완료)

§5의 "zip만 바꾸면 나머지는 자동"을 **실제로 실행**했고, 그 과정에서 §0의 표가
LGT판을 하나로 적었던 것이 틀렸다는 게 드러났다.

### ★ LGT판은 리비전이 둘이다

| | 01.00.04 | 01.00.07 (= §0의 그 빌드) |
|---|---|---|
| app_info `Ver:` | 01.00.04 | 01.00.07 |
| 인게임 표기 | **VER 1.0.1** | VER 1.0.4 |
| `binary.mod` | 520,308 B / `e76fafdd3e9e0057be8446cf7ab35577` | 519,952 B / `6a352944bb43c0789ca9f4b6f0702f4b` |
| `.text` 크기 | 0x5dcc9 | 0x5db85 |
| 블리터 진입점 | **0x2ddf0** (훅 pc 0x2ddf1) | 0x2dd7c (훅 pc 0x2dd7d) |

PID/AID(`PD119037` / `0002865E`)와 패키지 구성은 같다. §1의 "KTF↔LGT는 완전히
다른 컴파일"과 달리 **이 둘 사이는 마이너 리비전**이다: 블리터 함수가 통째로
알아볼 수 있게 남아 있고 `+0x74` 만큼 밀렸을 뿐이다(.text 가 0x144 B 길다).
그래서 재RE 가 아니라 **주소 재확인**으로 끝났고, `binary_patches.toml` 에
01.00.04 용 엔트리를 추가했다(해시 키라 두 엔트리가 서로 간섭하지 않는다).
계약 조항별 대조 근거는 그 toml 주석에 명령 단위로 적어 뒀다.

**교훈**: 성능 패치가 md5 키라서, LGT라고 다 같은 게 아니다. 새 아카이브를 받으면
**먼저 `binary.mod` 의 md5를 찍어라.** 표에 없으면 toml 이 해시-없는 제네릭
엔트리로 폴백해서 게임은 돌지만 blit565 가 안 붙고, 기기 로그의 `blit` 이 `0/0`
으로 남는다.

### 프로브 A/B (두 리비전 모두 통과)

`--w 240 --h 320 --frames 2600 --ms 20 --keys 260:OK,400:OK,700:OK,900:OK,…,2200:OK`
로 `--hle 0` / `--hle 1` 을 돌려 프레임 6장(500/1000/1500/2000/2400/2600) md5 대조:

| | 프레임 일치 | 블리터 호출 | 게스트 명령 (2600프레임) |
|---|---|---|---|
| 01.00.07 | 6/6 동일 | 226,008 / 226,008 HLE, refused 0 | 3.249 G → **2.129 G (-34.5 %)** |
| 01.00.04 | 6/6 동일 | 226,008 / 226,008 HLE, refused 0 | (동일 수순) |

두 리비전의 호출 수가 같은 것은 프로브가 결정적이고 게임 로직이 같기 때문이다.
프레임 해시도 타이틀 화면(버전 문자열이 1.0.1/1.0.4 로 다름)만 빼면 서로 같다.

### 앱 전환 — 무엇을 건드렸나

* `android_app/app/src/main/assets/hero3_fixed.zip` ← LGT 01.00.07 game.zip
  (md5 `07d7dba0d16177b3d7d997ba8bf12f01`, 1,763,788 B). 파일 **이름은 그대로**
  둬야 한다 — `lib.rs::BUNDLED_GAME_ASSET` 가 이 이름을 읽는다.
* 되돌리기: `android_app/asset_backup/hero3_fixed.zip.ktf`
  (md5 `1e7c7432…`, 기존 KTF i18-rename 판)를 같은 자리에 덮으면 끝.
* 코드 변경 **0줄**. 아카이브 감지·게이트·블리터 해시 매칭이 전부 자동이다.
* `build_android.ps1` 의 `-Install` 만 고쳤다: 기기 시리얼이 하드코딩(`<S25_SERIAL>`)
  이라 다른 폰에서는 `device not found` 로 끝났다. 이제
  `$env:ANDROID_SERIAL` → `device` 상태인 유일한 기기 → 기존 상수 순으로 고른다.

### 기기 실측 (Galaxy S21 / SM-G991N, release 서명, versionCode 28)

```
main: loaded asset hero3_fixed.zip (1763788 bytes)
main::app: boot: window 1080x2400 landscape_pref=true -> guest framebuffer 496x223 (immersive=true)
main::emulator: LGT build: KTF guest-address features (…) inactive until ported
wie_core_arm::binary_patches::hook: Hook installed at 0x2dd7d: Blit565Lgt
wie_core_arm::binary_patches::hook: Installed 1 hooks for Hero3 binary.mod (LGT)
main::database: DB open name="OptionSave_11" app_id="PD119037"
main::emulator: … blit 15238/15238, refused [0,0,0,0,0]
```
이용안내 → 타이틀 → 슬롯 → **클래스 선택**까지 진행, 스프라이트 정상. `blit` 이
분모=분자이므로 호스트 블리터가 100 % 처리 중이다.

주의: `blit 0/0` 은 이용안내처럼 **스프라이트를 안 그리는 텍스트 화면**에서도
정상적으로 나온다. 패치 설치 여부는 `blit` 비율이 아니라 부팅 로그의
`Hook installed at …: Blit565Lgt` 한 줄로 판단하는 게 확실하다.

디버그 APK 로는 설치가 안 된다 — 폰에 깔린 것이 릴리스 키 서명이라
`INSTALL_FAILED_UPDATE_INCOMPATIBLE` 이 난다. 지우고 깔면 앱 전용 저장소가
날아가므로(=KTF 세이브·레이아웃·설정 전부), **`-Release -Install` 로 덮어써야**
한다. 세이브는 DB 이름이 갈리므로(KTF `Hero3GameSave_N` / LGT `GameSave_N`)
한 설치 안에서 공존하고, KTF로 되돌려도 그대로 남아 있다.

## 2.7 4차 — CHAR 확보. 관문이 열렸다 (2026-08-10, 기기 실측 완료)

### ★ LGT엔 CHAR 포인터 전역이 없다 — 레코드가 `.bss` 정적이다

**`CHAR = 0x1503a88` (`.bss`, 고정).** §4 표는 이 항목을 "포인터 전역을 찾아라"로
적어 놨지만 전제가 틀렸다. KTF는 레코드를 힙에 할당하고 `*(sl+0x444)`에 포인터를
두지만, **LGT는 레코드 자체를 `.bss`에 컴파일해 넣었고 가리키는 포인터가 없다.**
그래서 "0 → 힙 포인터로 바뀐 전역" 으로 좁히는 접근은 원리적으로 실패한다
(실제로 유일한 후보가 나왔는데 열어 보니 4bpp 스프라이트였다: 첫 바이트 `0b` =
포맷 코드, 뒤가 팔레트).

### 어떻게 찾았나 — 화면의 숫자를 그대로 검색

1. **인게임 메뉴 키 = `CLR`**. LSK/RSK/`*`/`#`/숫자 전부 아니다 (`#` 은 맵 창).
   메뉴 = 상태보기 / INVENTORY / EQUIPMENT / SKILL / QUEST / SYSTEM.
2. 상태보기를 열어 프로브로 프레임을 뜨면 숫자가 전부 보인다: HP 167/167,
   SP 100/100, 힘12 민첩15 체력8 정신12, LV 1, EXP 1/814, ATT1 58 ATT2 63,
   P.DEF/M.DEF 42, CRI/RES 6, ACC 12, DOD 27.
3. `.data`/`.bss`/힙 덤프에서 그 숫자들이 **몰려 있는** 창을 점수화해 찾는다.
   814(EXP-to-next)가 가장 희소해서 앵커로 쓰기 좋다.

### 검증 — 독립 항목 다섯 개가 동시에 맞는다

| 확인 | 결과 |
|---|---|
| 상태창 숫자 6개 (`+0xe2` level, `+0xe4` exp, `+0xe8` exp_next, `+0xec` hp_max, `+0xf2` hp, `+0xf8` sp) | **KTF 오프셋 그대로** 전부 일치 |
| `+0xa0c0` window id | 필드 **3**(`WIN_NONE`) / 대화 진입 런 **4**(`WIN_SCRIPT`) — KTF 상수 그대로 |
| `+0x9e28` 인벤토리 그리드 핸들 | 힙 포인터(`0x40126ab8`), 런 사이 재할당까지 KTF와 동일 |
| `+0x9c6c` selector | 필드 0 / 메뉴·상태창 1 |
| `+0xb8` 이름 | `c4 c9 c0 cc` = CP949 "케이" |

base가 1워드라도 틀리면 이 중 어느 것도 맞지 않는다.

### 유일한 오프셋 차이: 상태 바이트가 `+0x7c` → **`+0x78`**

값 의미는 같다 (4 = 필드 자유이동, 5 = 메뉴/퀵아이템 선택 오버레이). 이 크레이트가
읽는 나머지 오프셋은 전부 그대로 이식된다. 1.0.3→1.0.4 리비전 차이로 앞쪽에
한 워드가 들고 난 것으로 보인다.

### 부수 확보

* 카메라/스크롤 오프셋 = `.bss 0x150d914`(x) / `0x150d918`(y), 두 번째 쌍이
  `0x150d91c`/`0x150d920`. RIGHT로 걸으면 x가 감소(월드가 왼쪽으로 밀림), LEFT면
  증가, DOWN이면 y 감소. `CHAR+0x9e8c` 이기도 하다.
* 프로브로 인게임까지 가는 결정적 키 스크립트: 프레임 260부터 120프레임 간격으로
  `OK` 를 계속 넣으면 ~7800프레임에 필드. 그 뒤 `CLR`→`OK` 로 상태보기.

### 앱 반영 — 게이트를 두 축으로 쪼갰다

`guest_addresses_valid()` 하나가 읽기와 쓰기/호출을 같이 막고 있었는데, **CHAR 위치는
알지만 코드 주소는 아직 모르는** 상태를 한 플래그로 표현할 수 없어서 나눴다
(`wie_android/src/guest_build.rs`):

* `char_anchor()` → `Pointer{global,fallback}`(KTF) / `Static(0x1503a88)`(LGT).
  `items::resolve_char` 가 이걸 쓴다. 정적 쪽은 포인터 널 체크가 불가능하므로
  **내용 타당성**(level 1..70, hp ≤ hp_max, sp ≤ 1000)으로 "캐릭터가 생성됐나"를
  판정한다 — 클래스 선택 전에는 레코드가 0이라 자연히 `None`.
* `state_off()` → 0x7c / 0x78.
* `guest_code_addresses_valid()` (개명) → **게스트 호출·쓰기 전용** 게이트. 14곳에
  명시적으로 박았다: `save_gate`, `backup_target`, `popup_sel_addr`, `popup_relabel`,
  `slot_writes`, `autoloot_to_force`, `hud_y_origin_to_set`, `band_x_patches`,
  `target_box_x_patches`, `band_height_poke`, `hud_bottom_patch`,
  `equip_relief_patch`, `exp_multiplier_patch`, `full_restore_writes`.
  ★ 이 중 코드 패치들은 그동안 **바이트 검증에만** 의존했다(확률적). 이제 확정적이다.

### 기기 실측 (S21, versionCode 29)

```
quickbar probe: char=Some(1503a88) field=true window=Some(3) state=Some(4) selector=Some(0) in_game=true
```
오버레이 HUD가 LGT에서 동작한다: 좌상단 LV 배지 + HP/SP 게이지, 우상단 MAP 프레임,
하단 퀵바 셀. 컷신 중에는 `window=4 field=false` 로 정확히 접힌다.

**예상되고 의도된 것**: 게임 자신의 하단 HUD가 **같이 보인다**(HP/SP 이중 표시).
`hud_bottom_patch` 는 게스트 코드 패치라 게이트가 끄기 때문이다. 이건 §4의 "HUD
스킵/배지" 항목이 아직 안 됐다는 뜻이지 회귀가 아니다.

## 2.8 5차 — 미니맵 + 게임 HUD 스킵. §4의 1·2번 완료 (2026-08-10)

§4 순서대로 1번(미니맵)과 2번(HUD 스킵)을 끝냈다. 둘 다 **프로브 픽셀/바이트 검증
완료**, 기기 실측만 대기. 그 과정에서 §3.4의 도구 구멍을 메우고, KTF↔LGT 사이의
**일반 규칙 하나**를 확정했다 — 이게 이번 회차에서 가장 값나가는 결과다.

### ★ 컨테이너 참조 규약: 게임의 컴파일이 아니라 **캐리어 런타임**의 성질이다

KTF 쪽 RE 문서 전체에 나오는 `*(*(X)) + 8` 이중참조는 게임 소스가 그렇게 쓴 게
아니다. `wie_ktf` 의 `WIPICContext::data_ptr` 가 `*(handle) + 8` 이고, `wie_lgt` 의
그것은 **항등함수**다 (`Ok(memory.0)`). 즉 게임이 `MC_knlCalloc` /
`MC_grpCreateOffScreenFrameBuffer` 로 받아 든 값이 KTF에서는 핸들, LGT에서는 곧바로
데이터 주소다. 그래서 **플래그 하나가 모든 컨테이너를 동시에 해결한다** — 인벤토리
그리드, AREA 객체, 아이템DB, 맵 배열, 프레임버퍼 전부.

`guest_anchors.rs::container_data_off()` = KTF `Some(8)` / LGT `None`.
§2.4의 블리터 포팅(`SpriteLayout::{KtfHandles, LgtDirect}`)이 스프라이트 기술자
하나에 대해 이미 발견했던 것의 **일반형**이다.

⚠ **이게 조용한 버그를 하나 잡았다.** `items.rs` 의 `resolve_grid_base` /
`resolve_area_base` / `item_db_icon` 은 무조건 이중참조를 하고 있었다. LGT에서는
`*(진짜 데이터 포인터)` = 데이터의 첫 워드를 다시 주소로 써서, 그 값이 우연히 힙
범위면 검증을 통과하고 **엉뚱한 바이트를 아이템 레코드로 읽는다**. 없는 기능이
아니라 틀린 값을 보여주는 쪽이라 더 나쁘다. 이제 세 곳 모두 `deref_container()`
하나를 지나간다.

### 공용 앵커 표 — 프로브의 KTF 하드코딩 제거 (§3.4 첫 항목 완료)

새 파일 **`wie_android/src/guest_anchors.rs`** = 의존성 없는 "빌드별 값" 표.
`guest_build.rs` 가 `include!` 해서 원자 상태(어느 빌드가 부팅했나)만 얹고,
`wie_probe` 도 `mod guest_build { include!(…) }` 로 **같은 소스**를 쓴다. 프로브의
`--map-check`/`--char-dump`/`--find-fb`/`--trace-state` 5곳에 박혀 있던
`0x1b3084`/`0x1b7b6c`/`+0x7c` 가 전부 사라졌다. `statics_ranges()` 도 빌드별로
갈라서 `--char-dump` 가 LGT에서 `.data`/`.bss` 를 뜬다.

프로브가 공유하지 **않는** 것 하나: `Static` 앵커의 "캐릭터가 살아 있나" 내용
검사. 그건 `items.rs` 의 CHAR 오프셋으로 만들어져 있고, 진단 도구는 레코드가 죽어
보일 때도 생 주소를 원한다(그게 보통 디버깅 중인 상태다).

표를 고정하는 standalone 테스트: **`anchors_selftest.rs`** (11 tests).
`rustc --edition 2021 --test anchors_selftest.rs -o t.exe; .\t.exe`

### 1번 미니맵 — 픽셀 동일, 두 맵에서

`minimap_render.rs` 는 `MapChain { data_off, noblend_global }` 를 인자로 받게 됐고,
LGT는 `data_off: None`(직접 포인터) + `noblend_global: None`(미발견 = 억제 없음).
**게스트 코드 주소는 한 개도 필요 없었다** — 데이터 체인만 갈리면 되는 항목이라 가장
싸다는 §4의 판단이 맞았다.

| 런 | 맵 | 결과 |
|---|---|---|
| LGT 01.00.07, 필드 | 35x25 (버퍼 70x50) | **0 / 3500 픽셀 불일치** |
| LGT, 문 통과 후 두 번째 맵 | 19x18 (버퍼 38x36) | **0 / 1368 픽셀 불일치** |
| KTF 회귀(기존 세이브) | 35x36 (버퍼 70x72) | **0 / 5040** — 예전 수치 그대로 |

맵창을 한 번도 안 연 프레임에서도 우리 렌더가 이미 성립한다(맵창은 자기 데이터를
갖지 않는다는 전제가 LGT에서도 참). 맵이 바뀌면 `CHAR+0x9c90` 재읽기로 따라온다 —
두 번째 맵은 걸어서 실제로 넘어가 확인했다.

체인(LGT): `MAP = *(CHAR+0x9c90)` / `attr = *(MAP+0x60)` / `event = *(MAP+0x68)`,
`w,h = i32 @MAP+0x48/+0x4c`. KTF와 오프셋이 전부 같고 홉 수만 하나씩 짧다.

부수 확보: `CHAR+0x9c8c` 는 게임 자신이 캐시해 둔 맵 객체 포인터다(KTF에서도 동일:
`+0x9c8c` = 이미 푼 포인터, `+0x9c90` = 핸들). 우리는 캐시를 쓰지 않는다.

### 2번 게임 HUD 하단 블록 스킵 — LGT 사이트 확보

**LGT 필드 HUD 렌더러 = `0x22a4`..`0x3050`** (`push {r4-r7,lr}` / `sub sp,#0xd4` …
`add sp,#0xd4; pop {r4-r7}; pop {r0}; bx r0`), 하단 블록 시작 = **`0x2864`**
(`ldr r5,[sp,#0x94]`, 원본 halfword `0x9d25`), 패치 = **`b 0x304a`** = `0xe3f1`.

**찾는 법이 §4가 적어 놓은 것과 다르다.** §4는 "맵창 열린 프로파일과 필드
프로파일의 차집합"을 제안했는데 **그 방법은 성립하지 않는다**: LGT에서 맵창을 열어도
게임의 하단 HUD는 계속 그려진다(프레임으로 확인). 실제로 먹힌 방법은 **산술 서명
검색**이다. 두 빌드 모두 패널 원점을 `((s16)screen_w - 0xaa)/2 + 2` / `screen_h -
0x18` 로 계산하므로, ELF 전체에서 `subs rX,#0xaa` 를 뽑으면 14개고 그중 뒤에
`asrs #1` + `adds #2` 가 붙은 것은 **하나**다. KTF 0x149f7a 와 명령 단위로 같은 소스다.

**검증 — `--poke` 로 A/B (프로브는 결정적)**:
* 사이트의 halfword 가 정적 디스어셈이 예고한 `0x9d25` 로 읽혔다.
* 하단 패널이 사라지고 **그 자리로 월드가 비쳐 보인다**(연못과 수련잎이 이어진다).
  검게 덮는 게 아니라 그리기를 건너뛰는 것이 맞다는 KTF 결론이 LGT에서도 그대로.
* 짧은 런(250프레임) + 긴 런(3000프레임: 걷기 → 메뉴 열기 → 맵창 2회 → 걷기)에서
  **CHAR 레코드 45 KB 가 A/B 바이트 동일**, `.data` 동일.
* 다른 데서 갈린 바이트는 `.bss 0x1500000`/`0x1500004` 둘뿐이고, 이 둘을 pc-relative
  로 읽는 명령이 **전부 스킵 구간 안**(`0x2f64`,`0x2f78`,`0x2fa8`,`0x2ffc`)이다 =
  HUD 전용 blink 카운터. KTF 채택 노트가 `0x14a2a0`/`0x14a2bc`/`0x14a2d0` 에 대해
  적은 것과 같은 성질이다. (짧은 런에서 `0x150ec58` 한 워드도 갈렸는데 긴 런에서는
  수렴했다 — 블록이 캐시하는 값이고 주변이 화면 크기 전역이다: `0x150ec88` = 240,
  `0x150ec8c` = 320.)

**앱 반영은 §3.6 규율대로 항목별로만 열었다**: `guest_code_addresses_valid()` 는
LGT에서 여전히 false 이고, `items::hud_bottom_patch` 만 새 표
`guest_build::hud_bottom_patch() -> Option<(site, orig, branch)>` 를 쓴다.
이게 **첫 번째 게스트 코드 패치의 항목별 분기**이고, 남은 항목들도 같은 모양으로
붙이면 된다. 쓰기 전 원본 halfword 검사는 그대로 남아 있다.

§4의 나머지(밴드/타겟박스/hud_y 미세 조정)는 §4.5대로 LGT에서 게이트가 꺼 놓은 채
게임 기본 배치로 둔다. 배지(`+0x18a`/`+0x18c`)는 LGT 미검증 — 레벨 1이라 값이 0이라
확인할 수단이 아직 없다(0을 그리는 것은 무해).

### 5번 EXP 배수 — 나눗셈 상수 확보. 장비레벨제한은 사이트만 확보

**LGT `addExpFromKill` = `0x4d388`, 나눗셈 상수 `movs r1,#0x64` = `0x4d3cc`.**
(halfword `0x2164` — KTF와 **같은 명령**이라 주소만 빌드별이다.)

찾는 법 = **상수 다발 스캔**. 공식이 `(enemyLv*13 + 130) * enemy[0xb4] *
(clamp(lvDiff,±9)*2 + 10) / 100` 이므로 `#0xd`·`#0x82`·`#0xa`·`#0x64` 가 한 창(0x38)
안에 다 있고 `muls` 가 2개 이상인 지점을 찾으면 **바이너리 전체에서 1곳**이다. 그
20개 명령이 KTF 0x14650c 와 하나씩 대응한다.

★ **산술로 교차검증됐다 — 패턴 일치보다 강한 증거다.** 이 함수는 이미지 안에서 호출
지점이 딱 하나고(함수 포인터로 불리므로 유일한 참조가 `0x3a6b0` 의 리터럴
`0x4d389`), 그 호출자는 `r0 = 접근자(); r0 += 0x8c; addExpFromKill(r0, enemy);` 뒤에
`checkLevelUp`(`0x4d714`)을 부른다. **접근자 `0x1068` 이 돌려주는 값이
`0x1503ab0` = CHAR+0x28** 이므로 `+0x8c` 는 **CHAR+0xb4** = 이 크레이트가 문서화해 둔
스탯 서브구조체 `H` 이고, 함수 안의 `[r4,#0x30]` 은 **CHAR+0xe4** = `items::EXP_OFF`
그대로다. 사이트가 틀렸다면 이 일치가 나올 수 없다. 덤으로 §2.7의 CHAR 베이스
`0x1503a88` 이 값 검색과 **독립적으로** 코드 산술로 재확인됐다.

라이브 확인: `--poke 7850:0x4d3cc:3221` 을 걸면 이전 바이트가 **`64 21`**
(`movs r1,#0x64`)로 읽혔다 = 정적 판독이 실행 중인 메모리와 일치.

⚠ **남은 미검증 한 가지 — 효과 자체**. 나눗셈 상수를 50으로 바꾼 런과 안 바꾼 런을
3,800프레임 배회시켰지만 **전투가 한 번도 안 걸려서** EXP가 둘 다 1/814 그대로였다
(CHAR도 바이트 동일 = 패치가 다른 부작용을 내지 않는다는 것만 확인됨). "적을 잡으면
증가분이 정확히 2배" 는 아직 관측하지 못했다. 필드에서 전투를 유발하는 결정적 키
스크립트가 필요하고, 그건 §4 5번 남은 칸의 검증 런과 같은 문제다.

앱: `guest_build::exp_divisor_site()` (항목별 분기), `items::exp_multiplier_patch` 의
`guest_code_addresses_valid()` 게이트 해제. `movs r1,#imm8` 이고 imm8이 100을 나누는
경우에만 쓰는 형태 검사는 그대로 남아 있다. 상수 하나를 바꾸는 패치이고 형태 검사가
막아 주므로, 최악의 경우는 "캐시템이 안 듣는다"이지 손상이 아니다.

**장비레벨제한 -5 는 사이트만 확보하고 이식은 보류했다.** LGT 체크 =
`0x477cc`..`0x477dc` (`showMessage(0x67)` 의 유일한 호출자, `showMessage` = `0x46b7c`):
```
477cc adds r4,#0xba ; 477ce movs r3,#0 ; 477d0 ldrsh r1,[r4,r3]   ; 플레이어 레벨
477d2 movs r3,#2    ; 477d4 ldrsb r3,[r2,r3]                       ; 아이템 요구 레벨
477d6 cmp r1,r3     ; 477d8 bge 0x477e2 (통과)
477da movs r0,#0x67 ; 477dc bl 0x46b7c
```
KTF는 요구레벨을 비교 레지스터로 **옮기는** `adds r1,r3,#0` 이 있어서 그걸
`subs r1,r3,#N` 으로 바꾸면 끝이었지만, LGT는 **읽은 바이트를 바로 비교**해서 그
한 halfword 트릭의 대응물이 없다. 자연스러운 등가물은 `movs r3,#2` +
`ldrsb r3,[r2,r3]` (4바이트)를 `ldrb r3,[r2,#2]`(`0x7893`) + `subs r3,#N`(`0x3b00|N`)
으로 바꾸는 것이다 — 요구레벨이 N보다 작으면 음수가 되고 부호 있는 `cmp`/`bge` 가
통과시키므로 의미가 정확히 "-N 레벨"이다. 다만 (a) 앱의 패치 배관이 halfword 하나만
싣게 돼 있고 (b) 검증에 캐릭터 + 레벨 제한 장비가 필요하다. **정적 판독만으로
켜지 않는다** — 다음 회차.

### 새 도구 (다음 항목들이 그대로 쓸 것)

* **`wie_probe --map-hunt 1`** — 맵 체인을 주소가 아니라 **구조로** 찾는다. 정적
  영역과 CHAR 레코드의 모든 워드에 두 규약을 다 시도해서, `+0x48/+0x4c` 가 그럴듯한
  타일 크기이고 `+0x60/+0x68` 이 `w*h` 바이트 배열로 풀리는 후보만 남긴다. LGT에서
  후보 2개(둘 다 같은 맵)로 끝났다. 워프(event>0) 타일 좌표도 찍어 준다 — 두 번째
  맵으로 걸어갈 목표가 이걸로 나온다.
  ⚠ 규약이 틀려도 "그럴듯"할 수 있다: KTF의 같은 맵 객체가 `direct` 로는
  818 blocked/21 events, `handle+8` 로는 828/12 를 내고 **후자만 진짜다**. 심판은
  `--map-check` 의 픽셀 대조다.
* **`wie_probe --poke frame:addr:hexbytes`** — 프레임 시작에 게스트 메모리 쓰기,
  이전 바이트를 로그로 남긴다. 게스트 코드 패치 후보를 **앱에 넣기 전에** 검증하는
  작업대. 위 A/B 가 이것 하나로 됐다.
* **`tools_re/lgt_statics_xref.py`** — 알려진 정적 주소를 참조하는 코드를 찾는다.
  `--range lo hi [--loaders]` 로 리터럴과 그걸 읽는 `ldr rX,[pc,#imm]` 을, `--imm8 N`
  으로 `movs/cmp/adds/subs rX,#N` 을 뽑는다. HUD 사이트는 `--imm8 0xaa` 로 나왔고,
  blink 카운터가 HUD 전용임은 `--range` 로 증명했다.

### LGT 코드젠에 대해 알게 된 것

* **긴 호출은 `bx rN` 베니어**를 거친다: `0x5a6e8`부터 `bx r0`,`bx r1`,`bx r2`,… 가
  2바이트 간격으로 늘어서 있다(`__call_via_rN`). 그래서 `bl #0x5a6fc` 는 "r5 로
  간접 호출"이라는 뜻이다 — 프로파일러가 이 주소를 자주 가리키면 그건 함수가 아니라
  베니어다.
* **CHAR 는 함수 인자로 돌아다닌다.** ELF 전체에서 CHAR 레코드 범위를 가리키는
  리터럴은 22개뿐이고 `0x1503a88` 자체는 3곳(`0x2a620`,`0x2a8bc`,`0x2ac84`)에만
  있다. HUD 렌더러는 `0x22aa` 에서 접근자를 호출해 `[sp,#0xac]` 에 담아 쓴다.
  `.data`/`.bss` 어디에도 CHAR 주소를 담은 포인터 전역은 **없다**(§2.7 재확인).
* ⚠ **CHAR 오프셋이 전부 이식되는 것은 아니다 — 새 반례**: HUD 렌더러가 쓰는
  `+0xa384`(KTF) 가 LGT에서는 **`+0xa370`** 이다(0x14 차이). 상태 바이트
  `0x7c`→`0x78`(4)와 크기가 다르니 단일 시프트가 아니다. 앱이 쓰는 오프셋들
  (≤ `0xa0c0`)은 §2.7에서 실측으로 확인된 것뿐이고, **새 오프셋은 매번 라이브로
  확인해야 한다**는 §4 공통주의가 이걸로 다시 증명됐다.

### 정정 — 인게임 맵창 키는 `*` 다, `#` 이 아니다

§2.7/§3.2 가 "`#` = 맵 창"이라고 적었지만 틀렸다. 필드에서 후보 키 10개를 100프레임
간격으로 넣고 `CHAR+0xa0c0`(window id)을 추적한 결과 **`*` 만 window 3 → 8** 로
바꾼다. `#` 은 필드에서 무반응이다. KTF와 같은 키다(`items.rs` 의 `WIN_MAP` 주석이
원래 "`*` 이 토글한다"고 적고 있었다). 맵창은 다음 아무 키에서 닫힌다.

## 2.9 기기 실측 체크리스트 (5차분, versionCode 30)

5차 변경은 전부 프로브에서 검증됐지만 **기기 실측은 대기 상태**다. 아래가 확인할
항목과 "무엇을 보면 되는가"다. 로그는 `adb logcat -s main:V main::emulator:V
main::database:V wie_core_arm::binary_patches::hook:V`.

| 확인 | 어디서 보나 | 기대 |
|---|---|---|
| LGT로 부팅했나 | 부팅 로그 | `LGT build: data-side features active … containers = direct pointers` |
| 성능 패치 설치 | 부팅 로그 | `Hook installed at 0x2dd7d: Blit565Lgt` (★ `blit` 비율이 아니라 이 줄로 판단 — §2.6) |
| **미니맵** | 우상단 MAP 프레임 / 30초 로그 | 프레임이 채워진다. 로그 `quickbar probe: … minimap=Some((35, 25))` 처럼 **타일 크기가 찍히면** 체인이 풀린 것 (`None` = 안 풀림) |
| **게임 HUD 스킵** | 화면 하단 | 게임 자신의 퀵바 칸 + HP/SP 바가 **사라지고** 그 자리로 월드가 보인다. 로그 `game bottom HUD: skipping its draw at 0x2864 (write ok = true)` |
| HP/SP 이중 표시 해소 | 화면 | 오버레이 게이지만 남는다 (§2.7 말미의 하자가 이것) |
| 인벤토리/퀵바 값 | 퀵바 셀의 수량·아이콘 | `deref_container` 수정 전에는 LGT에서 **틀린 값**이 나왔을 수 있다. 이제 맞아야 한다 |
| EXP 캐시템 | 로그 | 캐시템을 쓸 때 `exp multiplier: 0x4d3cc <- …`. **효과(2배)는 적을 잡아 봐야** 확인된다 — 프로브에서 전투를 못 만들어 미검증인 유일한 항목 |

### 세이브에 대해 — 무엇이 남고 무엇이 안 보이는가

덮어쓰기 설치(같은 서명 키, 같은 `applicationId`)이므로 **앱 전용 저장소는 그대로
남는다**: 레이아웃 에디터 배치, 설정, `wie_buffs.txt`, 그리고 **DB 레코드 전부**.

다만 KTF 세이브와 LGT 세이브는 **DB 이름이 갈린다**(KTF `Hero3GameSave_N` /
LGT `GameSave_N`, §2.5). 그래서:
* 지금 실려 있는 LGT 판에서는 **LGT 세이브만 슬롯에 보인다**. 이전에 LGT로 플레이한
  세이브가 있으면 그대로 이어진다.
* KTF 시절 세이브는 **지워진 게 아니라 안 보이는 것**이다. `asset_backup/
  hero3_fixed.zip.ktf` 를 되돌려 깔면 다시 보인다.
* 확인용 로그: `main::database: DB open name="GameSave_0" app_id="PD119037"`
  (KTF였다면 `Hero3GameSave_0` / `PD007102`).

⚠ **디버그 APK 로는 설치가 안 된다** — 폰의 것이 릴리스 키 서명이라
`INSTALL_FAILED_UPDATE_INCOMPATIBLE`. 지우고 깔면 앱 전용 저장소가 날아가므로
(= 세이브·레이아웃·설정 전부) 반드시 `-Release -Install` 로 덮어쓴다.

## 2.10 6차 — 배관·테스트 정비 (2026-08-10, 원격 리눅스 세션)

이 회차는 **원격 클라우드(리눅스) 세션**이라 근본 제약이 있었다: 게임 아카이브와
`binary.mod` 는 라이선스 문제로 저장소에 없고(gitignore가 `hero3_fixed.zip*` 를
막는다) 오직 로컬 머신(`D:/tmp/lgt/…`)에만 있다. 즉 **프로브 런·라이브 RE·`--poke`
A/B 는 전부 불가**. §4의 3(저장 체인)·4(quickbind)·6(스크래치)과 모든 "검증 런"
칸은 바이너리가 있는 로컬 세션 몫이다. 이 세션은 그 제약 안에서 §4 5번의 남은
코드 작업과 도구 정비를 끝냈다.

### 5번 잔여 (a) 완료 — equip-relief 배관이 빌드별 shape 을 싣는다

§2.8 말미가 적어 둔 "halfword 하나만 싣는 구조를 4바이트로 넓히기"가 끝났다:

* `guest_anchors.rs::EquipReliefSite` — `MoveHalfword { site }`(KTF `0x13d0aa`,
  기존 동작 그대로) / `LoadPair { site }`(LGT용 2-halfword). 표 함수
  `equip_relief_site()`, 래퍼는 `guest_build`. 이 패치도 이제
  `guest_code_addresses_valid()` 가 아니라 **항목별 표**를 지난다(§3.6 규율,
  `hud_bottom_patch`·`exp_divisor_site` 와 같은 모양).
* `buffs::equip_relief_pair(levels)` — LGT 인코더. 원본 `[0x2302, 0x56d3]`
  (`movs r3,#2; ldrsb r3,[r2,r3]`) ↔ 패치 `[0x7893, 0x3b00|N]`
  (`ldrb r3,[r2,#2]; subs r3,#N`). 상한은 KTF imm3 와 같은
  `RELIEF_LEVELS_MAX`(7) — 두 빌드가 같은 아이템에 다른 효과를 내지 않게.
* `items::equip_relief_patch` → `Option<(u32, Vec<u8>)>` (한 halfword든 두 개든
  싣는다). LoadPair 경로의 판정은 순수 함수 `equip_relief_pair_patch` 로 떼어
  단위테스트가 지킨다(원본→패치, 패치→복원, 이미 맞으면 무쓰기, 모르는 바이트 거부).

★ **LGT arm 은 여전히 `None` 이다** — §2.8의 규율("정적 판독만으로 켜지 않는다")
그대로. 켜는 것 = `guest_anchors.rs` 의 arm 하나를
`Some(EquipReliefSite::LoadPair { site: 0x4_77d2 })` 로 + `anchors_selftest.rs` 의
규율 테스트 한 줄 갱신. 그 전에 필요한 검증 런(캐릭터 + 레벨 제한 장비,
`--poke frame:0x477d2:93783b05` A/B)은 §4 5번의 계획대로 EXP 효과·배지 검증과
같은 런에 묶는 것이 맞다.

### §3.4 사본 구멍 절반 메움

`items_selftest.rs` 에 `deref_container` 미러(컨테이너 규약을 파라미터로 받는
형태)를 넣고 `resolve_grid_base` 가 그걸 지나게 했다. **양 규약 테스트** 포함:
LGT 직접 포인터 데이터에 KTF 이중참조를 돌리면 "그럴듯하게 풀리는" §2.8의 조용한
버그 모양 자체를 테스트가 고정한다. 여전히 사본이다(`include!` 아님) — 드리프트
위험은 남지만, 이 버그 클래스는 이제 사본 쪽에도 감시가 있다.

### ★ `cargo test` 는 Windows 호스트만 죽는다 — 리눅스에서는 그냥 돈다

`cargo test -p wie_android` 가 리눅스 호스트에서 정상 동작한다(59 tests,
`libasound2-dev` 만 있으면 됨). CLAUDE.md의 "시작하자마자 죽는다"는 **Windows
호스트 한정**이다. 그리고 첫 실제 실행이 잠재 버그를 하나 잡았다:
`gauges_use_the_corrected_hp_sp_offsets` 가 **19차의 배지 필드 추가 이후 줄곧
깨져 있었다** — `read_stats` 가 stat/skill 포인트를 읽게 됐는데 테스트 시드가
그 두 필드를 안 심어서 `None` 이 나왔다. 테스트 시드 수정(+배지 값 검증 추가).

### 셀프테스트 카운트 (이 회차 이후)

standalone 178 + anchors 13 (items 43, buffs 14, anchors 13 — 각 +1, anchors는
5차의 EXP 표 테스트가 카운트 갱신을 빠뜨려 실제로는 12였던 것도 이번에 정정).

## 2.11 커버리지 매트릭스 + 조용한 버그 감사 (6차, 2026-08-10) — canonical 누락 방지표

> "KTF(~vc24)에서 하던 것이 LGT에서 빠짐없이 진행되는가"를 한 눈에 보는 표.
> 새 회차마다 **여기부터 확인**하고, 상태가 바뀌면 이 표를 고친다. 근거는
> 코드 전수 감사(6차): `wie_android` 의 모든 게스트 read/write/call 지점을 나열하고
> 각각의 게이트·오프셋·실패모드를 대조했다.

### 게이트 아키텍처 (감사 결론 — 견고함)

* **게스트 쓰기·호출은 전부 LGT에서 꺼진다.** `guest_code_addresses_valid()` 는
  LGT에서 `false` 이고, 예외로 항목별 표(`hud_bottom_patch`/`exp_divisor_site`)만
  실측된 LGT 주소를 돌려준다. `equip_relief_site()` 의 LGT arm 은 §2.8 규율대로 아직
  `None`.
* **게스트 읽기는 열려 있고, CHAR 앵커·상태 바이트·컨테이너 홉이 전부 build-aware
  다** (`char_anchor()`/`state_off()`/`container_data_off()`). 6차 전수 grep 으로
  하드코딩된 `0x7c`·이중참조가 LGT 경로로 새지 않음을 확인.
* **유일한 KTF 절대주소 읽기** = `POPUP_HANDLES_SLOT 0x1b2df4`(sl-상대, `popup_view`
  /`cash_use_pending`). LGT에서 이 주소는 매핑 밖이거나, `사용하기` EUC-KR 9바이트
  **정확일치** 라벨 검사에서 걸러진다 → **fail-safe(absent)**, 틀린 값 아님.
* 알려진 LGT shift `+0xa384→+0xa370` 은 게이트된 `band_height_poke`(KTF 전용)에만
  있어 LGT로 새지 않는다.

### 매트릭스

범례: ✅ LGT 활성·검증 / 🔶 LGT 활성·**기기검증 대기(조용한 버그 감시 대상)** /
⛔ LGT 게이트 OFF(게스트 코드 미RE) / ➖ 캐리어 무관(이식 불필요)

| # | KTF 기능 | LGT 상태 | 근거 / 실패모드 |
|---|---|---|---|
| 1 | blit565 성능 패치 | ✅ | §2.6, 두 리비전 프레임 바이트 동일 (md5 키) |
| 2 | 미니맵 렌더 | ✅ | §2.8 프로브 0/3500·0/1368 + **7차 기기 실측** `minimap=Some((18,17))` |
| 3 | 게임 하단 HUD 스킵 | ✅ | §2.8 `--poke` A/B + **7차 기기 실측** `skipping its draw at 0x2864 (write ok = true)` |
| 4 | 오버레이 HP/SP/LV/EXP 게이지 | ✅ | 오프셋 `0xe2..0xf8` §2.7 실측 검증 |
| 5 | 오버레이 **퀵바 셀**(종류/수량/아이콘) | ✅ | §2.16 프로브 바인딩 A/B 로 `0x252`=종류(2=아이템)·`0x257`=인벤슬롯 검증. §2.13 아이템DB(`grid+0x38`, stride `0x18`, 아이콘 `+0x14`=프레임 17..28) 검증. `0x25c`/`0x26c`(스킬)만 스킬 바인딩 런 대기 |
| 6 | 오버레이 **배지**(스탯/스킬 포인트) | 🔶 | `0x18a/0x18c` **LGT 미검증**. Lv1=0이라 무해하나 Lv>1 미검증(§2.8) |
| 7 | 보조무기 아이콘(키7) | 🔶 | `secondary_weapon_icon`: EQUIP6 `0x513/0x514/0x524` **LGT 미검증** |
| 8 | no-save 지역서 SAVE/미니맵 숨김 | 🔶 | `can_save_here`: AREA `+0x11d` 미검증, **fail-open**(틀리면 "저장가능"=안전쪽) |
| 9 | EXP x2/x4 캐시템 | 🔶 | §2.8 사이트+산술, §2.12 트리거 확보로 **비로소 도달 가능**. 효과(적처치 2배)는 기기 대기 |
| 10 | 장비레벨제한 -5 | 🔶 | §2.12: LGT arm 개방 = `LevelBias{0x477ce}`. §2.8의 `0x477d2` 안은 **부호 버그**라 폐기. 효과는 기기 대기 |
| 11 | 빠른저장/자동저장 4단 체인 | ⛔(절반 확보) | §2.13: `writeSaveRecord 0x1a264` / `writeDbRecord 0x1a1d4` / 이름 문자열 / 슬롯 `CHAR+0xa410` 확보. 남은 것 = `serialiseSave`·`slotSummary` + 스크래치. **버튼은 이제 안 그려진다** (`save_chain_supported()`) |
| 12 | 복구 백업(사설서버 04 3d) | ⛔ | §2.13 에서 `readDbRecord 0x1a0e8` / `readSaveRecord 0x1a198` 후보 확보. 11번에 의존 |
| 13 | quickbind(팝업 퀵슬롯등록) | 🔶 | §2.16: 게스트 함수 **불필요**. `PopupCommit::SyntheticOk`(프로브 A/B 검증) + 데이터 쓰기 개방. ★ 51차(PLAY_FEEDBACK_FIXES.md): **스킬만 되고 소모품이 안 되던 원인 = 관문 6의 KTF 선점 바이트(`0x9c89`/`0xa220`)** — `popup_preempt_offsets()` 빌드별 표로 분리, LGT 는 빈 리스트(합성 OK 는 게임 라우터를 타므로 게임이 알아서 거른다). 기기 대기 |
| 14 | quickpage 페이지 복원 | ⛔ | `apply_quickpage_actions` 안의 `BindSkill` 만 `SKILL_BIND_FN` 게스트 호출이 필요 — `BindItem`/`Clear`(=`slot_writes`)는 이제 검증된 오프셋뿐이라 분리하면 열린다 |
| 15 | 오토루팅 강제 ON | ⛔ | `autoloot_to_force` 게이트(게스트 쓰기 `0x2d7`) |
| 16 | 풀회복(박스 보상) | 🔶 | §2.13: 게이트 뗐다. 빌드별 주소를 안 쓰고 검증된 `0xf2`/`0xf8` 에만 쓴다. 트리거(#17)도 열려 실제 발동은 기기 대기 |
| 17 | 캐시템 감지→호스트 버프 | 🔶 | §2.12: `popup_handle[8]` = `.bss 0x1512934` 확보, 팝업 클러스터 오프셋 전부 검증. 읽기만 열었고 라벨 9바이트 정확일치가 관문이라 fail-safe. **실제 발동은 기기 대기** |
| 18 | 스킬 쿨다운 감시 | 🔶 | `skill_cast_active`: `0x9bb5/0x2c9` 미검증. 표시 전용(틀려도 쿨다운 표시만) |
| 19 | 밴드/타겟박스/hud_y 미세조정 | ➖ | §4.5 LGT는 게임 기본 배치. 게이트 OFF가 의도된 정상 |
| 20 | 진동(크리티컬) | ➖ | 호스트가 `sd004_mf` 재생 감지. KTF 주소 안 씀 |
| 21 | 가로형 | ➖ | §4.5 해상도 적응, 캐리어 무관 |
| 22 | 버프 만료시계/상태줄 | ➖ | 호스트 절대시각(`wie_buffs.txt`), 캐리어 무관 |
| 23 | 사설서버 창고/복구/상점 연동(네트워크) | ➖/⛔ | 네트워킹은 호스트지만, 백업 업로드는 12번에 의존 |

### 🔶 = 조용한 버그 감시 대상 (5·6·7·8·18) — 다음 기기/프로브 세션의 1순위

이 다섯은 **LGT에서 켜져 있고**, CHAR 오프셋이 **LGT 미검증**이라, 오프셋이
`+0xa384→+0xa370` 처럼 밀렸다면 **없는 게 아니라 틀린 값**을 낼 수 있다 — 정확히
사용자가 경계한 조용한 버그 클래스다. 다만:
* 8·18 은 **안전쪽으로 실패**한다(8=fail-open, 18=쿨다운 표시만).
* 5·6·7 은 **plausibility 가드**가 있으나(종류 1/2만, 배지 캡, 아이콘 프레임≤28)
  약해서, 우연히 그럴듯한 바이트가 통과하면 틀린 값이 보일 수 있다.
* **왜 지금 오프셋을 안 바꾸나**: 이 환경(원격 리눅스)엔 게임 바이너리/프로브가
  없어 오프셋 검증이 불가하다. 검증 없이 값을 바꾸거나 기능을 끄는 것은 둘 다
  근거 없는 변경이다(전자는 새 조용한 버그, 후자는 회귀).
* **검증은 명령 하나로 끝나게 만들어 뒀다 (6차 후속)**: `wie_probe --verify-offsets 1`
  이 이 표의 다섯 항목을 **dump 프레임마다 라벨링·디코드해서** 찍는다 — 검증된 기준값
  (게이지/윈도우/상태)도 같이 찍어 base가 맞는지 상태창 숫자로 교차확인할 수 있다.
  로컬 세션 절차:
  ```
  # §3.2 키 스크립트로 인게임 진입 후 상태창(CLR->상태보기)까지, 그 프레임을 dump
  wie_probe --zip <game.zip> --w 240 --h 320 --frames 8600 --ms 20 --keys <§3.2 $k> \
    --db <새 디렉터리> --dump 8200,8400 --verify-offsets 1 --out <dir>
  # 출력 [verify] 의 REF lv/hp/sp/exp 가 상태창과 일치하면 base OK →
  # #5 qb / #6 badge / #7 equip / #8 area / #18 cast 값을 화면과 대조.
  ```
  오프셋이 밀렸다면 REF는 맞는데 🔶 값만 화면과 어긋난다 → 그 항목만 §3.3(a) 값 검색으로
  새 오프셋을 찾아 `guest_anchors.rs`/`items.rs` 에 빌드별로 넣는다(6차 equip-relief
  가 본보기). 이 다섯은 §4 5번의 결합 런(전투+레벨업+장비)에서 상태창을 함께 뜨면 전부
  같은 런으로 닫힌다. **수동 오프셋 산술을 하지 말 것** — 그 자체가 조용한 버그
  원천이라 도구로 뺐다.

## 2.12 7차 — LGT 전면 전환 + 기기 실측. 캐시버프 트리거 확보 (2026-08-11, 로컬 Windows)

> 이 회차의 한 줄 요약: **§2.9 기기 체크리스트가 전부 닫혔고**, §2.11 매트릭스의
> ⛔ 넷(#9·#10·#16·#17)이 실은 **한 가지 주소**에 걸려 있었다는 것을 찾아 그 주소를
> 확보했다. 이제 LGT가 이 앱의 정식 타깃이다.

### 이 머신의 출발 상태 — 문서가 낡아 있었다

* `android_app/app/src/main/assets/hero3_fixed.zip` 가 **KTF 였다** (`__adf__` +
  `0103EFD4.jar`). §2.6 의 "APK가 싣고 있는 것은 LGT"는 다른 머신 얘기였다.
* `D:\tmp` · `<keystore>` 둘 다 **없다**. §3.1 의 경로 표 전부 무효.
* `%USERPROFILE%\.gradle\gradle.properties` 가 **없어서** `-Release` 가 "서버 주소
  없음"으로 하드에러 났다. `<keystore>` 의 `D:/` 경로를
  `<keystore>` 로 고쳐 재작성. 인증서 SHA-256 은 문서에 적힌
  `6dd5550c…` 와 일치 → 기존 설치를 **덮어쓸 수 있다**(세이브 유지).

### 전환

| | |
|---|---|
| 새 LGT 아카이브 | `<keystore>` (zip md5 `049ff7be…`) |
| 안의 `binary.mod` | 519,952 B, md5 **`6a352944…`** = **01.00.07** → `binary_patches.toml` 엔트리 있음 |
| 되돌릴 KTF | `android_app/asset_backup/hero3_fixed.zip.ktf` (1,800,713 B) |
| LGT 원본(패치 전) | `android_app/asset_backup/hero3_lgt.zip.orig` |
| 실제 실린 것 | 위 원본에 `patch_asset.py` 를 적용한 **1,764,382 B** (★ 10차까지 실제로는 미적용 1,763,788 B 였다 — §2.21) |

★ **`patch_asset.py` 가 KTF 전용이었다**: 멤버 경로를 `dat/i18_dat` 로 하드코딩하고
`sys.path` 에 죽은 `d:\tmp\…` 를 넣고 있었다. LGT는 `res/dat/i18_dat` 다. 이제
**basename 으로 찾는다**. LGT에서 그대로 동작: 27 레코드, `0x11`/`0x12`/`0x18` 만
변경(경험의비석·경험의보주·초월의문장), 나머지 전 멤버 바이트 동일, `binary.mod`
md5 불변(= blit565 매칭 유지).
⚠ `--apply` 는 `<zip>.orig` 를 **옆에** 떨군다. Gradle 은 `assets/` 를 통째로
패키징하므로 **assets 안에서 --apply 하지 말 것** — 백업 파일이 APK에 실린다.
`asset_backup/` 사본에서 돌리고 결과만 복사한다(이 회차가 그렇게 했다).

### 프로브 재검증 — 실제로 실리는 zip 그대로

`--frames 8600 --ms 20`, §3.2 키 스크립트, `--hle 0` / `--hle 1`:

| | |
|---|---|
| 프레임 md5 (2000/4000/6000/8200/8400) | **5/5 바이트 동일** |
| 블리터 호출 | 1,200,941 / 1,202,076 HLE = **99.906 %**, refused geom 1,135, fmt/mem/null **0** |
| 게스트 명령 (8500프레임) | 13.95 G → **8.79 G (-37.0 %)** |
| CHAR | `0x1503a88`, REF lv 1 / exp 1·814 / hp 167·167 / sp 1000 — §2.7 그대로 |
| 맵 체인 | 필드에서 `35x25` — §2.8 수치 그대로 |

### 기기 실측 (SM-G960N, release, versionCode 31/32) — §2.9 체크리스트 전부 닫힘

```
main: loaded asset hero3_fixed.zip (1765437 bytes)
main::app: boot: window 1080x2220 landscape_pref=true -> guest framebuffer 492x239
main::emulator: LGT build: data-side features active …; containers = direct pointers
wie_core_arm::binary_patches::hook: Hook installed at 0x2dd7d: Blit565Lgt
wie_core_arm::binary_patches::hook: Installed 1 hooks for Hero3 binary.mod (LGT)
main::database: DB open name="OptionSave_11" app_id="PD119037"
main::emulator: game bottom HUD: skipping its draw at 0x2864 (write ok = true)   ← 기기 최초
main::emulator: quickbar probe: … minimap=Some((18, 17))                          ← 기기 최초
main::emulator: area gate: can_save=Some(true)
```
5차의 **미니맵**과 **하단 HUD 스킵** 둘 다 기기에서 확인됐다(§2.8이 "기기 대기"로
남겨 둔 항목).

⚠ **실사용 신호 하나**: `save: queued Manual retry expired unfulfilled` — 호스트
SAVE 버튼을 눌렀는데 아무 일도 안 일어난다. 저장 4단 체인이 LGT 미이식(§4-3)이라
예상된 동작이지만, **UI 는 버튼을 보여 준다**. §4-3 이 끝날 때까지 LGT에서 SAVE
버튼을 숨기는 편이 정직하다(게임 자신의 메뉴 → SYSTEM 저장은 정상 동작).

### ★ 새 진단 도구 — `items::offset_audit`, ~5초마다 기기 로그

`--verify-offsets` 는 프로브가 **도달할 수 없는 상태**(레벨업·퀵슬롯 등록·보조무기
장착·아이템 팝업)를 못 만든다. 사람은 몇 분이면 다 만든다. 그래서 같은 감사를
기기 로그로 옮겼다: 검증된 REF 블록 + 🔶 다섯 + 팝업 클러스터를 한 줄에 찍는다.
읽기 전용이고 전부 bounds-checked. 매트릭스에 🔶 가 없어지면 이 블록을 지운다.

기기 첫 출력(필드, 레벨 1):
```
offset audit (LGT) CHAR=0x1503a88 | REF lv=1 exp=1/814 hp=166/166 sp=1000 win=3 state=4 sel=0
 | #6 badge stat=0 skill=0 | #5 qb0..4 전부 0 | #7 equip present=1 tab=0 slot=3
 | #8 area=0x4d85f404 can_save=1 | #18 cast mode=2 cat=0
 | #17 pop page=0 open=0 rows=0 sel=0 tag=0 tab=0 handles=None      ← 이 None 이 병목이었다
```

### ★★ 진짜 병목은 #17 하나였다 — 그리고 주소를 찾았다

**버프를 부여하는 경로가 앱 전체에 딱 하나다**: `emulator.rs` 의 OK 키다운 핸들러가
`items::cash_use_pending` 을 묻고, 그것만이 `buffs::grant_for`/`box_reward` 로
간다. 그 함수는 팝업의 강조 행 라벨이 `사용하기` 인지를 KTF 의 `sl` 상대 슬롯
`0x1b2df4` 를 거쳐 확인한다. **LGT에서 그 주소는 매핑조차 안 된다**(위 로그의
`handles=None`, 기기 실측). 결과:

* #9 EXP 2·4배 — 사이트(`0x4d3cc`)가 맞아도 **요청이 안 온다**
* #10 장비레벨제한 — 같음
* #16 풀회복(박스 보상) — 트리거가 같은 곳
* #17 자체

즉 §2.11 이 ⛔ 로 적은 넷이 **주소 하나**에 걸려 있었다. §4 는 이걸 "quickbind"
항목 뒤쪽에 묻어 뒀는데, 실은 **가장 값나가는 단일 항목**이었다.

**`popup_handle[8]` = `.bss 0x1512934`** (LGT 01.00.07).

찾는 법 = **§3.3(b) 를 워드 하나에서 런으로 넓힌 것**. `free_popup` 이 핸들을 전부
NULL 로 만드니까, 팝업이 열려 있을 때 N 워드가 모두 그럴듯한 힙 포인터이고 닫히면
같은 워드가 모두 0 인 구간을 찾으면 된다. 이 서명은 아주 구체적이라 `.bss` 전체
19,030 워드에서 **후보가 정확히 하나** 나왔다(길이 8):

```
프로브 키: … 8100:CLR, 8250:DOWN, 8400:OK, 8550:OK   → 인벤토리 → 첫 아이템 → 팝업
           8800:CLR                                   → 팝업 닫기
--dump 8700,8950 --dump-mem 0x1500000:0x12958
0x01512934  len=8  0x488422b4 0x488422a4 0x488428c4 … 0x48842874   (닫히면 전부 0)
```

**역참조 결과 = 화면에 있던 그 두 줄**, EUC-KR, **직접 포인터**(할당자 헤더 없음):
```
[0] 0x488422b4 → bb f3 bc bc ba b8 b1 e2 00  = 상세보기
[1] 0x488422a4 → b9 f6 b8 ae b1 e2 00        = 버리기
[2..7]                                        = 할당됐지만 0 (팝업이 2줄이었다)
```
홉 수는 새 발견이 아니라 `container_data_off()` 의 따름정리다 — 인벤토리 그리드·
AREA·아이템DB와 같은 규약.

### 같은 두 덤프가 팝업 클러스터 오프셋을 공짜로 검증했다

diff 에 `0x150d70c` 가 걸렸는데 그게 `CHAR+0x9c84` 다. 바이트를 그대로 읽으면
**화면과 정확히 일치**한다:

| 오프셋 | 필드 | 열림 | 닫힘 | 화면 |
|---|---|---|---|---|
| `+0x9c6c` | selector | 1 | 1 | 메뉴 오버레이 있음 |
| `+0x9c6e` | menu page | **2** | 2 | `PAGE_ITEM` = 인벤토리 ✅ |
| `+0x9c71` | inv tab | 0 | 0 | `« 1 … 3 »` 첫 탭 ✅ |
| `+0x9c72` | inv cursor[0] | 0 | 0 | 첫 칸 강조 ✅ |
| `+0x9c84` | popup open | **1** | **0** | ✅ |
| `+0x9c85` | popup rows | **2** | 0 | 상세보기/버리기 = 2줄 ✅ |
| `+0x9c86` | popup sel | 0 | 0 | 0행 강조 ✅ |
| `+0x9c87` | popup tag | **100 (0x64)** | 100 | ⚠ KTF `TAG_ITEM` 은 **0x66** |

★ **태그가 다르다**. `cash_use_pending` 은 태그를 안 보므로(그 이유가 주석에 적혀
있다 — 태그는 동적이라 추측이 된다) 영향 없지만, `popup_view`(퀵슬롯등록 재작성)는
게이트 4에서 태그를 요구하므로 **LGT에서 항상 `None`** 이다. §4-4 를 할 때 태그
상수도 빌드별로 갈라야 한다.

인벤토리 그리드 레이아웃도 같은 방식으로 검증됐다(`grid = *(CHAR+0x9e28)` =
`0x40126ab8`, §2.7 의 기기 실측값과 **같다**):
슬롯 stride `0x3c`, `+0x3b6` cat / `+0x3b8` idx / `+0x3ba` count →
`(0x12, 0, 5)`·`(0x12, 4, 5)` = **포션 5개**, 장비 5칸 count 1, 빈칸 `0xff/0xff`.
`CASH_CAT = 0x12` 가 LGT에서도 그대로다.

아이템DB 핸들도 풀린다: `ITEMDB_HANDLE_OFF` 는 **CHAR 기준이 아니라 그리드 기준**이고
(`item_db_icon` 은 `resolve_grid_base()` 위에 `+0x38` 을 얹는다), `grid+0x38` =
`0x4012d260` — 그럴듯한 힙 포인터다. (초고에서 `CHAR+0x38` 을 읽고 "0 이라 안 풀린다"고
적었는데 **베이스를 잘못 잡은 것**이었다. 정정 완료.)

### 앱 반영 — §3.6 규율대로 항목별

* `guest_anchors.rs::PopupHandles { Pointer{global}, Static(addr) }` +
  `GuestBuild::popup_handles()` — KTF `Pointer{0x1b2df4}` / LGT `Static(0x1512934)`.
  `CharAnchor` 와 **같은 모양의 불일치**다(포인터 전역 vs `.bss` 정적).
* `items::popup_handles` 빌드 인식화, `items::popup_label_addr` 는 이제
  `deref_container` 한 줄 — 홉 수가 캐리어 규약에서 자동으로 나온다.
* 쓰기 경로는 **여전히 잠겨 있다**: `popup_relabel`/`popup_sel_addr`/
  `apply_quickpage_actions` 는 `guest_code_addresses_valid()` 게이트 그대로.
  즉 이번에 열린 것은 **읽기**뿐이다.
* 결과: **#17 활성 → #9 EXP 2·4배가 LGT에서 실제로 동작 가능**해졌다
  (트리거 + 사이트 둘 다 있음).

### ★ 장비레벨제한 — §2.8 의 4바이트 패치는 **부호 버그**였다. 사이트를 바꿨다

§2.8/6차가 준비한 LGT 릴리프는 `0x477d2` 의
`movs r3,#2; ldrsb r3,[r2,r3]` → `ldrb r3,[r2,#2]; subs r3,#N` 이다. 산술은 맞지만
**`ldrsb`(부호) 를 `ldrb`(무부호) 로 바꾼다.** 요구레벨 바이트가 ≥ `0x80` 인 아이템이
하나라도 있으면(C `signed char` 필드에서 `-1` = "제한 없음" 은 아주 흔한 인코딩)
동작이 **"착용 가능" → "착용 불가"** 로 뒤집힌다. 아이템 데이터를 그 값으로 검사한
적이 없었다. 정확히 이 마이그레이션이 계속 잡아내는 조용한 버그 클래스다.

그래서 **비교의 반대쪽**을 패치한다 — `0x477ce`:
```
477cc  adds  r4,#0xba
477ce  movs  r3,#0        ┐  →  ldrh r1,[r4,#0]   (0x8821)
477d0  ldrsh r1,[r4,r3]   ┘  →  adds r1,#N        (0x3100|N)
477d2  movs  r3,#2           (건드리지 않음 — ldrsb 유지)
477d4  ldrsb r3,[r2,r3]
477d6  cmp   r1,r3
477d8  bge   0x477e2
```
**등가성이 증명된다** (그럴듯한 게 아니라):
* `r4` = 접근자(`CHAR+0x28`, §2.8) `+0xba` = **`CHAR+0xe2` = `items::LEVEL_OFF`** —
  HUD 가 `lv` 로 읽고 `--verify-offsets` 가 `lv=Some(1)` 로 찍은 바로 그 halfword.
* 레벨은 게임 자신이 70에서 캡한다(`LEVEL_CAP`, 레벨업 루틴의 `cmp #70`) 그리고
  `resolve_char` 가 `1..=70` 밖이면 레코드를 거부한다. 필드가 가질 수 있는 **모든**
  값에서 `ldrh` = `ldrsh` 다. 그래서 이 재작성은 비교만 바꾼다.

**라이브 확인** (`wie_probe --poke`, 결정적):
```
[poke] f=7850 0x477ce: 0023e15e -> 21880531 (ok)     ← 정적 판독과 일치
[poke] f=7850 0x477d2: 0223d356 -> 0223d356 (ok)     ← §2.8 판독도 라이브 최초 확인
프레임 md5 8200/8400 = 무패치 런과 동일
```

앱: `EquipReliefSite::LevelBias { site }` (구 `LoadPair` 대체),
`buffs::equip_relief_level_bias` (`[0x2300,0x5EE1]` ↔ `[0x8821,0x3100|N]`),
`items::equip_relief_bias_patch`. **LGT arm 을 열었다** (`Some(LevelBias{0x4_77ce})`).
`anchors_selftest` 에 `assert_ne!(site, 0x4_77d2)` 를 박았다 — 옛 계획의 주소로
"되돌리는" 편집이 고침처럼 보이면서 실은 회귀이기 때문이다.

### 셀프테스트

standalone **192** (anchors 13 → **14**, 팝업 앵커 테스트 추가).
`cargo ndk -t arm64-v8a --platform 26 check` 통과.

### 이 머신에서 `wie_probe` 를 빌드하려면

`cargo build --release -p wie_probe` 가 **`error calling dlltool 'dlltool.exe'`** 로
죽는다. 호스트가 `x86_64-pc-windows-gnu` 라서 `build_android.ps1` 의 dlltool 심
(§헤더)이 **호스트 빌드에도** 필요하다. 그 스크립트의 심 블록만 떼어 dot-source 한
뒤 cargo 를 돌리면 된다. 그리고 PowerShell 5.1 에서 `cargo … 2>&1` +
`$ErrorActionPreference='Stop'` 조합은 진행 로그를 **종료성 에러**로 만든다 —
리다이렉트하지 말 것.

## 2.13 7차 후속 — 저장 체인 절반 + 남은 게이트 정리 (2026-08-11)

### ★ 프로브가 LGT에서 실제로 저장했다 — 그게 모든 것의 출발점이었다

§4-3 이 제안한 그대로: 인게임 → `CLR` → **DOWN ×5** → `OK`(SYSTEM) → `OK` 연타.
`--db` 디렉터리에 레코드가 떨어진다.

```
$k += '8100:CLR','8200:DOWN','8260:DOWN','8320:DOWN','8380:DOWN','8440:DOWN','8520:OK'
$k += '8660:OK','8800:OK','8940:OK','9080:OK'      # frames 9200
→ PD119037\db\GameSave_11_0\1   1408 B
  PD119037\db\SlotSave_11_0\1     32 B
  PD119037\db\OptionSave_11\1     32 B
```
* **1408 B** = KTF와 **같은 크기**(CLAUDE.md 부록의 정정값 그대로).
* 레코드는 **암호화돼 있다**. `0x40..0x47` 과 `0x48..0x4f` 가 같은 8바이트
  (`79 b1 d9 42 0f eb 0c a7`) → **8바이트 블록 ECB**(1408 = 176 블록). 그래서 파일
  바이트로 `.bss` 를 역검색해도 안 나온다(평문 버퍼가 아니라 암호문이고, 그건 힙에 있다).

### ★★ 정적 XREF 가 함수 두 개를 바로 줬다 — 값 검색보다 빨랐다

`%s_%d` (va **`0x5ca14`**) 를 참조하는 코드는 **세 곳뿐**이다
(`lgt_statics_xref.py --range 0x5ca10 0x5ca30 --loaders`). 그 셋을 디스어셈하면 끝난다.

**`0x1a1d4` = `writeDbRecord(name, buf, len)`**
```
r6 = alloc(len + 0x10)
memcpy(r6,        <16바이트 헤더 전역>, 0x10)
memcpy(r6 + 0x10, buf,                  len)
<WIPI DB write>(name, r6, len + 0x10)
free(r6); free(buf)
```
★ **KTF 문서의 "디스크 파일 = 16 + 페이로드"가 여기서 증명된다.** SlotSave 파일이
32 B = `0x10` 헤더 + `SLOT_RECORD_LEN 0x10`, GameSave 가 1408 = `0x10` + 1392.
호스트가 부를 것은 이 함수가 **아니라** 아래 래퍼다(KTF도 그렇다).

**`0x1a264` = `writeSaveRecord(baseName, buf, len)`** — KTF `FN_WRITE_RECORD 0x1951b0`
의 대응물:
```
memset(sp, 0, 0x1e)
r0 = 접근자()                      ; bl 0x5a6f4, 리터럴 0x1069 → 0x1068 (§2.8: CHAR+0x28)
sprintf(sp, "%s_%d", baseName, *(r0 + 0xa3e8))
writeDbRecord(sp, buf, len)        ; = 0x1a1d4
```
**`0x1a198` = `readSaveRecord(baseName, slot)`** → **`0x1a0e8` = `readDbRecord(name)`**
(#12 복구 백업의 `FN_LOAD_SAVE_RECORD` 후보), `0x1a2b8` 은 같은 모양의 세 번째
래퍼(삭제/존재확인 계열).

### 확보된 상수 — 전부 서로를 교차검증한다

| | LGT | KTF | 근거 |
|---|---|---|---|
| 레코드 기본 이름 | **`"GameSave_11"` @ `0x5bbb4`** / **`"SlotSave_11"` @ `0x5bbc0`** | `sl+0x138` / `sl+0x13c` 의 `"./Hero3GameSave"` | 문자열 자체에 `_11` 이 들어 있고 `%s_%d` 로 슬롯이 붙어 **`GameSave_11_0`** — 프로브가 떨어뜨린 파일 이름과 정확히 일치 |
| 저장 슬롯 번호 | **`*(CHAR + 0xa410)`** | `*(CHAR + 0xa3f8)` | 접근자 `0x1068` = `CHAR+0x28`, `+0xa3e8` → `CHAR+0xa410`. 슬롯 0 이었고 파일이 `..._0` 로 떨어졌다 |
| 레코드 헤더 | `0x10` 바이트, 쓰기 함수가 붙인다 | 동일 | `0x1a1d4` 의 두 `memcpy` |
| `sl` / `"./"` 접두사 | **없다** (절대 주소 문자열) | 있다 | §0 주소 모델 |

⚠ **`SAVE_SLOT_OFF` 는 이식되지 않는 오프셋 세 번째 사례다** (`0x7c→0x78`,
`0xa384→0xa370` 에 이어 `0xa3f8→0xa410`). §4 공통주의가 또 맞았다.

### 남은 것 — 정확히 두 함수 + 스크래치

* **`serialiseSave(u32* outLen) -> buf`** 와 **`slotSummary() -> buf16`**.
  찾는 법이 이미 좁혀져 있다: `"GameSave_11"`(`0x5bbb4`) 과 `"SlotSave_11"`(`0x5bbc0`)
  을 **둘 다 가까이서** 로드하는 사이트가 곧 저장/불러오기 루틴이다 —
  `0xc018`/`0xc01e`, `0x25170`/`0x25180`, `0x254b0`/`0x254c0`, `0x3b248`/`0x3b258`,
  `0x481fa`/`0x4820a`. 이 다섯 중 `0x1a264` 를 두 번(게임+슬롯) 부르는 것이 저장
  루틴이고, 그 안에서 `0x1a264` 의 2번째 인자를 만드는 호출이 두 함수다.
* **out-param 스크래치**. KTF는 게임 자신의 **도달 불가 코드**를 썼고 그 증명이
  근거였다. LGT에서 같은 증명을 하기 전에는 아무 데도 쓰지 말 것(§4-3). HUD 스킵이
  건너뛰는 `0x2864..0x304a` 는 "우리 패치가 건너뛴다"일 뿐이고 **도달 불가 증명이
  아니다** — 혼동하지 말 것.

### 이번 회차에 실제로 켜고 끈 것

* **#16 풀회복 개방.** `items::full_restore_writes` 의
  `guest_code_addresses_valid()` 게이트를 **뗐다**. 이 함수는 호출도 안 하고
  빌드별 주소도 안 쓴다 — `HP_OFF 0xf2` / `SP_OFF 0xf8` 에만 쓰고, 그 둘은 §2.7·§2.12
  에서 상태창 숫자로 검증된 게이지 오프셋이다. 값도 게임 자신의 것
  (`*(CHAR+0xec)`, `AddSP` 클램프 상수). 남는 보호는 원래부터 있던 것 —
  `resolve_char` 가 살아 있는 레코드(레벨 1..70, hp ≤ hp_max, sp ≤ 1000)가 아니면
  거부한다. 블랭킷 게이트가 설명하지 못하는 경우를 숨기고 있던 것이다.
* **#5 아이콘 = 문제 없음.** `ITEMDB_HANDLE_OFF` 는 **그리드 기준**이고
  `grid+0x38` = `0x4012d260` 로 풀린다. 그 표를 뜨면 stride `0x18`, `+0x14` 아이콘
  바이트가 idx 0..11 에서 **17..28** — `item_bm` 의 0..28 범위 안이다. 매트릭스 #5의
  아이콘 절반은 이미 맞다.
* ★ **저장 버튼을 없앴다 — 저장 체인이 없는 빌드에서.** §2.12 가 실사용으로 잡은
  `save: queued Manual retry expired unfulfilled` 의 처리다.
  `guest_build::save_chain_supported()` 하나를 두고 세 곳에 꽂았다:
  `app::quicksave_zone`(필드 버튼이 **아예 안 그려진다** — no-save AREA 와 같은 규칙),
  `emulator::InputEvent::QuickSave`(설정 메뉴 행은 남으므로 **`저장미지원` 토스트**로
  이유를 말한다. 3초 뒤 "저장불가"로 장소 탓을 하는 게 아니다),
  그리고 자동저장 타이머·캐시템 저장 재시도(둘 다 성공할 수 없으므로 **arm 하지
  않는다**). `guest_code_addresses_valid()` 와 지금은 답이 같지만 **질문이 다르다** —
  UI가 물어야 하는 건 "이 버튼이 뭔가 할 수 있나"다. §4-3 이 끝나면 이 함수만 바꾼다.
* **#15 오토루팅은 보류가 맞다** (§4 주석 갱신): `CHAR+0x2d7` 은 게스트 **쓰기**라
  오프셋이 밀렸으면 엉뚱한 바이트를 1로 만든다. 그 바이트를 세우는 것은 캐시템
  `오토루팅(싱글)`(cat `0x12` idx `0x0d`)뿐이고 시작 인벤토리에 없다.

### 셀프테스트 / 빌드

standalone **192**, `cargo ndk check` 통과, 릴리스 서명 빌드 통과.

## 2.14 8차 — 네트워크가 LGT에서 **에뮬레이터를 죽이고 있었다** (2026-08-11)

### ★★ 원인: 사설서버가 아니라 WIPI net 슬롯 매핑이었다

사용자 보고: "KTF일 때는 접속이 됐는데 LGT에서는 네트워크가 안 된다."
**서버도 SSH도 문제가 아니다.** Lightsail 인스턴스는 Running, 정적 IP `<hero3-server>`
그대로, 그리고 빌드는 `libmain.so` 안에서 호스트 문자열과 인증서 핀을 **검증한 뒤**에만
성공한다(`build_android.ps1` §2b). 원인은 캐리어 런타임에 있었다.

`wie_lgt/src/runtime/svc_ids.rs` 는 net 슬롯을 **딱 세 개**만 이름 붙였다:
`Connect 0x258` / `Close 0x259` / `SocketClose 0x25e`. 그리고
`WIPICSvcId::try_from` 의 `_ =>` 는 **`FatalError`** 다. `handle_wipic_svc` 가
`try_from(id)?` 로 그걸 그대로 전파하므로 —

> **게임이 소켓을 만들려는 순간 에뮬레이터가 죽는다.**

그게 `MC_netConnect` 콜백이 성공을 보고한 **직후 첫 호출**이다(`net.rs` 의 recon 주석이
KTF에서 관측한 순서 그대로). 그래서 창고·복구·캐시샵이 LGT에서 "안 되는" 게 아니라
**시도하는 순간 끝났다.** KTF에서는 같은 코드가 동작한다 — `wie_ktf` 의 메서드 표가
net 블록 34칸을 전부 등록하기 때문이다.

### 고친 방법 — 포팅이 아니라 매핑이다

`wie_wipi_c::api::net` 의 구현들은 **캐리어 무관**이다(`Platform::network()` 를 쓰고
KTF 고유 요소가 없다). 그래서 LGT에 필요한 것은 슬롯 번호 매핑뿐이었다.

`wie_lgt/src/runtime/wipi_c.rs`:
* `NET_SVC_BASE = 0x258`, `NET_SVC_SLOTS = 34`.
* `handle_wipic_svc` 가 이 범위를 **`try_from` 보다 먼저** 범위로 디스패치한다 —
  범위 안의 id 가 `try_from` 에 닿으면 안 된다(그게 FatalError 다).
* `net_slot_body(slot)` 가 `wie_ktf` 의 표와 **위치까지 동일**하게 바디를 돌려준다:
  0 connect / 1 close / 2 socket / 6 socket_close / 13 set_read_cb / 30 connect_named /
  31 send / 32 recv / 33 close_fd = VERIFIED, 나머지는 `trace_stub`(로그 남기고
  게임을 계속 걷게 하는 값).

★ **베이스가 `0x258` 이라는 것은 가정이 아니라 증명이다**: 기존에 이름 붙은 세 슬롯이
`0x258+0`, `0x258+1`, `0x258+**6**` 이고, KTF 표에서 인덱스 6 이 정확히
`net::socket_close` 다. 인접하지 않은 점 하나가 끼어 있어서 우연일 수 없다.

### 다음 세션이 볼 것

기기에서 창고/복구/캐시샵을 눌러 `NETTRACE` 로그를 뜬다. LGT 게임이 KTF와 **다른
슬롯**을 쓴다면 `MC_netUnk` 스텁이 그 번호를 찍는다(그게 `trace_stub` 의 존재 이유).
그리고 §2.5 가 기록한 LGT의 앱키 `hero3lgt_wipi` / 서버 IP `210.222.18.28` 때문에
**첫 패킷의 모양이 KTF와 다를 수 있다** — 그건 `hero3_server` 쪽 대조가 필요하다.

## 2.16 8차 — 퀵슬롯등록 이식 완료. 게스트 함수는 **하나도 필요 없었다** (2026-08-11)

> ★ **§2.15 의 진단이 두 군데 틀렸다. 이 절이 정정본이다.** 실측이 §2.15의 추측을
> 두 번 뒤집었고, 결과적으로 RE 단계가 통째로 사라졌다.

### 정정 1 — 팝업 태그는 빌드 차이가 아니다

§2.15 는 "LGT 태그가 `0x64` 라서 `popup_view` 게이트 4가 막힌다"고 적었다. 틀렸다.
그 `0x64` 는 §2.12에서 **무기**(핸드나이프) 팝업을 뜬 값이고, 그 팝업은 애초에
`상세보기 / 버리기` **두 줄**짜리 = 등록 행이 없는 다른 종류다.

포션(cat `0x12`)으로 다시 뜨니:

| 오프셋 | 값 | 뜻 |
|---|---|---|
| `+0x9c6e` page | **2** | `PAGE_ITEM` |
| `+0x9c84` open | 1 | 열림 |
| `+0x9c85` rows | **8** | `ROWS_ITEM` **그대로** |
| `+0x9c87` tag | **0x66** | `TAG_ITEM` **그대로** |
| `+0x9c71/0x9c72` | 0 / 6 | 탭 0, 커서 = 포션 슬롯 6 |

화면도 같은 말을 한다: `사용하기 / 1키등록 / 3키등록 / 9키등록 / 0키등록 / #키등록 /
상세보기 / 버리기` — 8줄, row 1 이 정확히 `LABEL_KEY1`(`1키등록`).
**즉 `popup_view` 의 10개 관문이 LGT에서 전부 통과한다.** 진짜로 막고 있던 것은
`popup_relabel` / `popup_sel_addr` 안의 `guest_code_addresses_valid()` 블랭킷 게이트
하나였다 (그리고 그 앞에 §2.12 전까지는 `popup_handles` 가 `None` 이었던 것).

### 정정 2 — 커밋에 `popup_key` 가 필요 없다

§2.15 는 "남은 것은 게스트 함수 하나(`popup_key`)"라고 적었다. 그것도 틀렸다.
`emulator.rs` 의 row 0 경로가 **이미 `OK` 를 합성해서** 게임이 처리하게 한다. 그렇다면
행 번호만 써 두면 **같은 합성이 아무 행이나 커밋할 수 있다.**

**프로브 A/B (결정적, 두 런은 poke 하나만 다르다)** — 포션 팝업을 띄우고 `OK` 전에
`CHAR+0x9c86` 을 1(`1키등록`)로 poke:

| | A (poke 없음, row 0) | B (`--poke 8790:0x150d70e:01`) |
|---|---|---|
| `CHAR+0x252` qb type[0] | 0 | **2** = 아이템 |
| `CHAR+0x257` qb itemref[0] | 0 | **6** = 커서가 있던 포션 슬롯 |
| 팝업 | 닫힘(아이템 소비됨) | 닫힘 |

게임 자신의 팝업 디스패처가 키가 도착할 때 그 바이트를 읽는다는 뜻이고, 그 뒤의
전부(종류·참조 기록, 기존 바인딩 정리, 팝업 정리)는 **게임 코드**다.
★ 덤으로 **§2.11 매트릭스 #5의 퀵바 오프셋이 이 런에서 검증됐다** — `0x252`가 종류,
`0x257`이 인벤 슬롯 참조, KTF 문서 그대로.

### 앱 반영 — §3.6 규율대로 항목별 표

* `guest_anchors.rs::PopupCommit { GuestCall{fn_addr,select_key}, SyntheticOk }` +
  `GuestBuild::popup_commit()`. KTF는 측정된 호출을 **그대로 유지**한다(현장 검증된
  경로이고, 두 방식이 한 가지 점에서 다르다 — 아래).
* `items::popup_relabel` / `items::popup_sel_addr` 의 게이트를
  `guest_code_addresses_valid()` → **`popup_commit().is_some()`** 으로 바꿨다. 물어야 할
  질문이 "KTF냐"가 아니라 **"제안한 행을 실행할 수 있느냐"** 이기 때문이다. 제안만 하고
  실행이 안 되는 조합이 출하되면 안 되고, 그걸 정확히 이 표가 답한다.
* `items::POPUP_KEY_FN` / `POPUP_KEY_SELECT` 상수는 표로 옮겨 없앴다.
* ★ **`items::popup_count_restore`** — 합성 OK 경로 **전용**. 우리 재작성이
  `+0x9c85` 를 4로 줄여 놓는데 커밋할 행은 최대 5(플레이 4 → 게스트 행 5)다. 직접
  호출은 그 사이에 게스트 코드가 안 돌지만, **합성 키는 다음 tick 에 배달되므로** 그
  틈에 게임의 커서 클램프가 행 번호를 줄인 카운트 안으로 끌어당길 수 있다. 커밋 직전
  네이티브 카운트(8/7)를 되돌려 그 창을 없앤다. 팝업은 곧 닫히고, 카운트 바이트는
  이 기능이 이미 소유한 바이트다.
* `anchors_selftest.rs` 에 "LGT arm 은 주소를 싣지 않는다"를 박았다 — 키 입력은
  낡을 수 없지만 호출 주소는 낡는다.

### 아직 남은 것

**quickpage 페이지 복원(#14)** 은 그대로 게이트다: `apply_quickpage_actions` 의
`BindSkill` 이 `SKILL_BIND_FN`(게스트 호출)을 쓴다. `BindItem`/`Clear` 쪽
(`slot_writes`)은 이제 검증된 오프셋만 쓰므로 이식 가능하지만, 한 함수 안에 섞여 있어
분리가 필요하다. 다음 회차 몫.

## 2.15 (초판, §2.16이 정정) 퀵슬롯등록이 LGT에서 안 뜨는 이유

사용자 보고: 스킬을 누르면 여전히 `포인트투자 / 1키등록 / 3키등록 / 9키등록 / 0키등록 /
#키등록 / 등록취소`(7행)가 뜬다. `포인트투자 / 퀵슬롯등록 / 등록취소`(3행)로 바뀌어야 한다.

### 직접 원인 — `popup_view` 가 4번 관문에서 막힌다

`items::popup_view` 의 게이트 4가 `CHAR+0x9c87`(팝업 태그)이 KTF의 `TAG_ITEM 0x66` /
`TAG_SKILL 0x67` 이기를 요구한다. **LGT의 아이템 팝업 태그는 `0x64` 로 실측됐다**
(§2.12의 열림/닫힘 `.bss` diff). 그래서 `popup_view` 가 항상 `None` 이고 재작성이
아예 시작되지 않는다. 스킬 팝업 태그는 아직 안 재 봤다(아이템이 `0x64` 이니 `0x65` 가
유력하지만 **재 보고 넣을 것**).

### 이미 LGT에서 검증된 것 — 재작성에 필요한 데이터는 전부 있다

| 필요 | LGT 상태 |
|---|---|
| 라벨 버퍼 주소 | ✅ `popup_handle[8]` = `.bss 0x1512934`, 직접 포인터 (§2.12) |
| 팝업 열림/행수/선택 | ✅ `+0x9c84`/`+0x9c85`/`+0x9c86` 화면과 일치 (§2.12) |
| 메뉴 페이지 / 인벤 탭·커서 | ✅ `+0x9c6e`=2, `+0x9c71`/`+0x9c72` (§2.12) |
| 네이티브 행 수 | ✅ 스킬 팝업이 **7행** = KTF `ROWS_SKILL` 그대로 (사용자 보고) |
| 라벨 문자열 출처 | ✅ `binary.mod` 에 **없다** — 에셋 `res/dat/InGame_txt` 의 id 조회다. 즉 우리 방식(버퍼 덮어쓰기)이 LGT에서도 성립한다 |

### 남은 것 = `popup_key(kind, key)` 의 LGT 주소 **하나**

커밋 경로를 따라가면(`emulator.rs` `PopupConfirm` → `PopupBindSlot` →
`popup_commit`) 필요한 게스트 **호출**은 `POPUP_KEY_FN` 뿐이다(KTF `0x13a86c`,
선택 인자 `-5`). `popup_sel_addr`(`CHAR+0x9c86` 쓰기)와 `popup_relabel`(라벨 버퍼 +
행수 바이트 쓰기)은 **데이터 쓰기**이고 그 오프셋은 위 표대로 전부 검증됐다.
`SKILL_BIND_FN` 은 이 경로에 없다 — 그건 quickpage 페이지 복원 전용이다.

★ **그래서 순서를 지켜야 한다.** 태그만 빌드별로 갈라 놓고 데이터 쓰기를 열면 목록은
`퀵슬롯등록` 으로 바뀌는데 **선택해도 아무 일이 안 난다** — 지금의 "틀렸지만 동작하는"
목록보다 나쁘다. 그래서 이번 회차는 `popup_key` 를 찾기 전까지 아무것도 켜지 않았다.

**찾는 법 (다음 세션)**: 팝업이 열린 상태에서 키를 넣는 결정적 프로브 런
(§2.12의 `8100:CLR,8250:DOWN,8400:OK,8550:OK` 뒤에 `1`/`3`/`OK` 를 넣는다) →
`--profile 1` 의 `pc<-lr` 쌍에서 팝업 처리 함수를 좁히고, `CHAR+0x9c86`(선택 바이트)을
쓰는 코드를 `lgt_statics_xref.py` 로 교차확인한다. KTF의 "점프테이블이 `key+0x10` 으로
인덱스된다"는 성질이 같은 컴파일러 관용구로 남아 있을 가능성이 높다.

### 이번 회차에 되돌린 것 — SAVE 버튼

7차 후속이 저장 체인 미이식을 이유로 필드 SAVE 버튼을 숨겼는데, **되돌렸다.**
이미 출하된 기능을 캐리어 포팅이 안 끝났다는 이유로 치우는 것은 잘못된 교환이다
(사용자 지시, 2026-08-11). `guest_build::save_chain_supported()` 는 남았지만 이제
**진단 전용**이다 — 아무것도 숨기거나 끄지 않고, 거절의 원인이 "장소"인지 "미이식"인지
로그로 구분해 주기만 한다. 자동저장·캐시템 저장 재시도도 원래대로 arm 한다.
정답은 §4-3 을 끝내는 것이고, §2.13 이 그 절반을 해 뒀다.

## 2.17 프레임 드랍 — 측정했다. 훅이 안 걸린 **알파 블렌드 루프**가 21 %다 (2026-08-11)

사용자 보고: "메인메뉴부터 인게임까지 렉이 조금씩 계속 걸리고 게임이 너무 느리다."

### 프로파일 (LGT, 필드 8,900프레임, `--profile 1`)

```
[prof] 13,844,000 samples, 3283 distinct leaf PCs
[prof]  39.60%  pc=0x00000008            <- WIPI 타이머 idle (일 아님)
[prof]   0.62%  pc=0x0002b7f0
[prof]   0.62%  pc=0x0002b7d8   ... 약 35개 PC 가 0x2b790..0x2b816 에 균등 분포
```
0x86 바이트(≈67 명령) 구간에 **명령마다 균등한** 샘플 = 언롤 없는 타이트 루프이고,
합치면 전체 샘플의 **약 21 %**다. **`0x2dd7c` 블리터(blit565_lgt)와는 별개 구간**이고
훅이 없다.

### 그 루프가 무엇인가 — RGB565 **픽셀 단위 알파 블렌드**

```
0x2b790  ldrh r2,[r6]            ; 목적지 픽셀
         R = (p & 0xf800)>>8 , G = (p & 0x07e0)>>3 , B = (p & 0x1f)<<3
         채널마다  (src*alpha + dst*(255-alpha)) >> 8       ; alpha = [sp,#0x24]
                   src R/G/B  = [sp,#0x78] / [sp,#0x7c] / [sp,#0x80]
         RGB565 재조립
0x2b810  strh r1,[r6] ; r6 += 2
0x2b816  bne  0x2b790
```
= **반투명 사각형 채우기(`blendRect`)**. 픽셀당 게스트 명령 **약 44개**.
메뉴 패널·대화 바·HUD 패널·페이드가 전부 이걸 쓰니 "메인메뉴부터 인게임까지 조금씩
계속"이 정확히 설명된다. 240×320 전체 페이드 한 장 = **프레임당 3.4 M 명령**
(필드 프레임 전체가 약 1.0 M인 것과 비교하라).

### `blend565_lgt` 훅 스펙 — 함수 전체를 해독했다. 구현은 기계적이다

**함수 = `0x2b700` `blendRect(ctx, x, y, w, h, …, alpha, R, G, B)`**
(`push {r4-r7,lr}; sub sp,#0x5c`). 앞부분이 하는 일:
`[sp+0x74]` 의 알파를 0..255로 zero-extend(0이면 즉시 리턴) → `0x2ad6c` 로 `sp+0x2c` 에
그래픽 컨텍스트를 채움 → 음수 x/y 보정 + `[sp+0x34]`(폭)·`[sp+0x38]`(높이)로 클리핑 →
행 시작 포인터 = `stride*y + bpp*x + base` → 전역 `+0x82` 바이트가 **1(RGB565)** 인지
확인(아니면 `0x2b846` 의 다른 포맷 경로).

**훅 지점 = `0x2b786`**(바깥 루프의 첫 명령 `ldr r4,[sp,#0x28]`; `0x2b836` 이 여기로
돌아온다). 그 직전 `0x2b784` 가 `str r4,[sp,#0x20]` 로 남은 행수를 이미 써 놨으므로
훅이 프레임에서 **전부 읽을 수 있다**:

| 슬롯 | 뜻 |
|---|---|
| `[sp+0x20]` | 남은 행 수 (h) |
| `[sp+0x28]` | 행당 픽셀 수 (클리핑된 w) |
| `[sp+0x1c]` | 현재 행의 **바이트** 포인터 |
| `[sp+0x18]` | 행 스트라이드 (**바이트**) |
| `[sp+0x24]` | 알파 0..255 |
| `[sp+0x78]` / `[sp+0x7c]` / `[sp+0x80]` | 소스 R / G / B |

**픽셀 연산** (내부 루프 `0x2b790`..`0x2b816` 그대로):
`R=(p&0xf800)>>8, G=(p&0x07e0)>>3, B=(p&0x1f)<<3` →
채널마다 `(src*alpha + dst*(255-alpha)) >> 8` → `R&0xf8<<8 | G&0xfc<<3 | B>>3`
(코드가 쓰는 마스크: `r1 &= [pc리터럴]`, `r3 = (ip<<3) & 0x7e0`, `r2 = (x>>0xb) & 0x1f`).

**탈출 = `0x2b8d6`** — `0x2b71a`/`0x2b776`/`0x2b844`/`0x2b848` 이 모두 분기하는 공통
리턴 지점. `0x2b838`~`0x2b842` 의 스필백은 컴파일러가 스택을 여분 레지스터로 쓴 것이라
건너뛰어도 된다(같은 슬롯에서 로드한 값을 되돌려 쓰는 것뿐이다).

### ★★ 구현 완료 + 측정 (2026-08-11)

`vendor/wie/wie_core_arm/src/binary_patches/blend565.rs` + `HookKind::Blend565Lgt
{ exit_pc }` + toml 엔트리(`pc = 0x2b787`, `exit_pc = 0x2b8d7`, 01.00.07 에만).

같은 8,600프레임 결정적 런, `--hle 1`, 프레임 픽셀 **5/5 완전 동일**:

| 프레임 | 훅 없음 | 훅 있음 | 감소 |
|---|---|---|---|
| 2000 | 1.774 G | 0.919 G | **-48.2 %** |
| 4000 | 3.186 G | 1.586 G | **-50.2 %** |
| 6000 | 7.306 G | 2.045 G | **-72.0 %** |
| 8000 | 8.651 G | 3.043 G | **-64.8 %** |
| 8500 | 8.793 G | **3.185 G** | **-63.8 %** |

* 프레임당 게스트 명령 **1.0 M → 0.4 M**
* 가상시계 배율 **1.86x → 7.46x** — 실시간 대비 여유가 **4배**가 됐다
* `svc` 는 5,640,894 → 5,647,559 로 사실상 동일 = 훅이 SVC 폭풍을 만들지 않는다
  (호출 자체는 드물고, 한 번이 엄청난 양의 작업을 대신한다)

★ 6000프레임 지점의 -72 %가 이 항목의 정체를 그대로 보여 준다: 그 구간이 컷신·대사
바가 많은 곳이고, 반투명 사각형이 화면을 많이 덮을수록 절약이 커진다. 사용자가
"메인메뉴부터 인게임까지 조금씩 계속"이라고 한 그 패턴이다.

**안전장치**: 훅이 바꿔치기한 것은 2바이트 `ldr r4,[sp,#0x28]` 이라, 거절할 때는 그
로드를 **직접 수행하고** 2바이트 뒤로 재개한다(blit565의 `resume_past_push` 와 같은
규율). 거절 조건 = 말이 안 되는 프레임(행/폭 0 또는 4096 초과, 스트라이드가 행 폭보다
작음, 알파 범위 밖, 널 포인터). 설치 시점에는 halfword 가 `0x9c0a` 인지 확인하고
아니면 **설치를 거부**한다. 그래서 최악이 "예전과 같음"이다.

**픽셀 산술은 게스트의 것을 그대로 옮겼다** — 특히 빨강은 누산기를 **시프트하지 않고**
pc 리터럴 `0xfffff800` 로 마스크한 뒤 `strh` 가 16비트로 자르는 것에 의존한다. 이게
게스트가 하는 방식이고, `(acc>>11 & 0x1f) << 11` 로 "정리"하면 같은 값이지만 두 코드가
갈라질 수 있으므로 일부러 게스트 형태로 뒀다.

### ★★ 그리고 같이 발견한 것 — 라이브러리 훅 6개가 **한 번도 설치된 적이 없다**

`wie_core_arm::install_binary_patches` 는 **either/or** 다:
```rust
if 해시 일치 엔트리 { return install_entry(...); }   // ← 여기서 return
if 해시 없는 엔트리 { return install_entry(...); }
```
Hero3는 두 캐리어 모두 해시 엔트리를 갖고 있으므로, `binary_patches.toml` 의 제네릭
엔트리에 있는 **`memcpy` / `memset` / `strcpy` / `strlen` / `inline_copy` /
`reg_inline_copy` 여섯 개가 KTF에서도 LGT에서도 설치되지 않는다.** 기기 로그의
`Installed **1** hooks` 가 그 증거였는데 아무도 그 숫자를 의심하지 않았다.
(§3.5가 LGT의 `0x5a4fa~0x5a51e` 를 "제네릭 패턴 훅 후보"로 적어 둔 것도 이 때문에
영원히 후보로 남아 있었다.)

⚠ **`install_entry` 를 두 번 부르는 것으로는 고칠 수 없다 — 시도해서 확인했다**:
그 함수는 끝에서 자기가 만든 레지스트리로 `register_svc_handler(BINARY_PATCH_SVC, …)`
를 호출하므로 두 번째 호출이 **첫 프레임 전에**
`FatalError("SVC handler already registered for 128")` 로 죽는다.

**고쳤다 — `install_entries(&[(&Entry, bool)])`**: 모든 엔트리의 훅을 **먼저 resolve**
하고, 패치는 **합쳐진 훅 목록**에 대해 검사·설치하고, 디스패처는 **마지막에 한 번만**
등록한다. 세 가지가 함께여야만 성립한다 — (1) 디스패처 1개, (2) 패치↔훅 SVC 겹침을
**전체** 기준으로 봐야 제네릭 패치가 게임 전용 훅을 덮지 않는다, (3) PC 충돌 시
**먼저 온 것(=해시 엔트리)** 이 이긴다. `install_entry` 는 이제 1개짜리 래퍼다.

⚠⚠ **그런데 LGT에서는 이득이 0 이었다 — 측정값이다.** 같은 8,600프레임 런에서
`instr=8,793,014,905` / `svc=5,640,894` / `batches=13,533,404` 가 **리팩터 전과
완전히 동일**했고 크래시도 없다. 즉 제네릭 엔트리의 여섯 패턴은 **LGT 코드젠에 하나도
매칭되지 않는다** — 그 패턴들은 KTF 컴파일러의 관용구(R7 프레임 상대 다운카운터 등)로
쓰여 있고, LGT는 §2.8이 적어 둔 대로 스택 스필 위주 + `bx rN` 베니어의 다른 코드젠이다.

그래도 리팩터는 남긴다: 구조적으로 옳고(회귀 0으로 검증됨), §3.5가 후보로 적어 둔
LGT stdlib 핫스팟(`0x5a4fa~0x5a51e`)에 **LGT용 패턴을 새로 쓰면 그때부터 바로 붙는다**.
지금까지는 패턴을 써도 설치될 자리가 없었다.

★ **따라서 프레임 드랍의 실제 해결은 아래 `blend565_lgt` 훅이다.** 위 두 항목 중
성능을 실제로 되돌리는 것은 이쪽이다.

### 참고 — 기기가 갤럭시 S9 다

지금 테스트 중인 것은 **SM-G960N(2018)** 이고, 4차·5차의 실측은 S21(SM-G991N)에서
했다. 인터프리터 바운드 워크로드라 기기 세대 차이가 그대로 프레임에 나온다. 위 두
항목을 고친 뒤에도 남는 차이는 이것일 수 있으므로, 기기 로그의 30초 게이지
(`emu throughput` / `guest fps` / `blit` 비율)를 **S9 기준으로 새로 적어 둘 것**.

## 2.18 (역사 — §2.19가 5건 전부 처리) 다음 세션 인수인계 — 미완 5건 (2026-08-11 말)

> ★ **9차(§2.19)가 이 5건을 전부 진행했다** — (1') 네트워크·(3) letterbox·(5) 저장
> 체인은 **원인 확정 + 수정 + 프로브 검증까지 완료**, (2) 아이콘은 의심 지점 전부
> 무혐의 판정(기기 진단 라인 추가), (4) 크리티컬은 훅 후보 함수까지 확보. 이 칸은
> 그 근거의 출발점으로만 남긴다. **다음 세션은 §2.19부터 읽어라.**

> 기기 = **SM-G960N (S9)**, 설치본 **versionCode 41**. S25 Ultra(<S25_SERIAL>)는
> 다른 테스트 중이므로 **건드리지 말 것** (vc24 그대로 둔다).
> 아래 5건은 전부 "원인 미상"이 아니라 **다음 한 수가 정해진 상태**다.

### (1') 네트워크 — ★ 호스트 쪽은 다 끝났다. TLS 까지 붙는다. 남은 것은 **패킷/서버 대조**

> (1)은 이 항목의 초기 진단이고, 그 뒤 네 번의 실기기 왕복으로 아래까지 왔다.
> **다음 세션은 이 칸부터 읽어라.** 설치본 **vc44**.

LGT 의 net 슬롯 대응이 **전부 채워졌다** (전부 실기기 로그 근거):

| 슬롯 | 기능 | KTF 대비 |
|---|---|---|
| 0 | `MC_netConnect` | 같음 |
| 1 | `MC_netClose` | 같음 |
| 2 | `socket(af, type)` | **SVC id 가 `0x7d0` 그룹** (`0x7d0 + k` = KTF 슬롯 `2 + k`) |
| 3 | **connect** (`net::connect_lgt`) | KTF는 슬롯 **30** |
| 4 | **send** | KTF는 슬롯 **31** |
| 5 | **recv** | KTF는 슬롯 **32** |
| 6 | `close(fd)` | 같음 |

그리고 두 가지 구조적 수정:
* **모르는 WIPI SVC id 는 더 이상 치명적 오류가 아니다** — 로깅 스텁이다. 이게 없었으면
  위 표를 얻는 데 슬롯 하나당 빌드 왕복 한 번이 필요했다. LGT 는 SVC **id 그룹**
  (`0x258`/`0x7d0`/`901`/`904`)과 **슬롯 배치**가 둘 다 KTF와 다르다.
* **`AndroidNetwork::send`/`recv` 가 대기 중 fd 를 그 자리에서 promote 한다** — 게임은
  소켓을 둘 연달아 열고, fd 3 의 완료 콜백이 돌자마자 **fd 4 로** 로그인 패킷을 쓴다
  (fd 4 의 완료 태스크가 첫 폴링에 닿기 전). 캐리어 무관한 실제 경쟁 상태였다.

**실기기에서 확인된 사실**:
```
tls: validating "<hero3-server>" against a pinned certificate   x2
tls: handshake complete ("TLSv1_3")                            x2
net: fd 3 connected (TLS)
slot31 socket_send(fd=4, len=7)  payload: 07 00 00 00 01 01 04
```
서버는 살아 있다(이 PC 에서 `<hero3-server>` **TCP OPEN**), 핀·호스트는
`libmain.so` 안에서 빌드가 검증한다. **그런데도 창고는 무한 로딩이다.**

★ **다음 수 = 패킷 레벨 대조.** 호스트 소켓 계층은 더 볼 것이 없다.
1. `hero3_server` 로그를 켜고 위 첫 패킷 `07 00 00 00 01 01 04` 이 도착하는지, 서버가
   무엇을 돌려주는지 본다. `hero3_server/protocol.py` 의 프레임 규약(`MAX_FRAME` 1016)과
   대조.
2 §2.5 가 기록한 LGT 앱키 **`hero3lgt_wipi`** 와 서버 IP `210.222.18.28` 은 KTF와
   다르다. 서버가 KTF 클라이언트 기준으로 로그인/핸드셰이크를 파싱하면 LGT 패킷을
   거절할 수 있다 — 그러면 로그에는 send/recv 가 **정상 동작**하는 모습이 찍히므로
   호스트 문제와 바로 구분된다.
3. 그리고 슬롯 **901 / 904** 의 정체를 아직 모른다(`(port=8091, 1, .data 문자열, …)` /
   `(.text 문자열, 1, .data, .data, …)`). 지금은 스텁이 0을 돌려주고 게임이 그대로
   진행하지만, 이 둘이 세션 키/앱키를 세팅하는 호출이면 그 값이 서버에 안 전달된다.
   `0x14005e4`/`0x14005f4`/`0x5c734` 의 문자열을 읽는 것이 그 확인이다(프로브
   `--dump-mem 0x1400000:0x604` + binary.mod 직접 읽기).

### (1) (초기 진단, (1')이 정정·확장) 남은 것은 슬롯 3 구현

세 번의 왕복으로 여기까지 왔다: `0x258`(600) 그룹 매핑 → 소켓 그룹이 **`0x7d0`(2000)**
이라는 것 확보(`0x7d0 + k` = KTF 슬롯 `2 + k`) → 모르는 SVC id 를 **치명적 오류에서
로깅 스텁으로** 바꿔서 한 번의 실행이 전부를 나열하게 만듦. vc41 실기기 로그:

```
slot0   MC_netConnect(cb=0x18901)                          -> 0, callback pending
slot2   socket(af=2, type=1)                               -> fd 3
slot901 MC_wipicUnknown(0x1f9b, 0x1, 0x14005e4, 0x0, 0x18901, 0x150ea80) -> 0
slot904 MC_wipicUnknown(0x5c734, 0x1, 0x14005e4, 0x14005f4, 0x18901, 0x150ea80) -> 0
slot3   MC_netSocketConnect?(0x3, 0x0, 0x0, 0x18b35, 0xf, 0x150ea80) -> 0
slot0   MC_netConnect callback(result=0 SUCCESS, param=0x0)
        …같은 순서로 fd 4 재시도 → slot6 close(4) → slot1 MC_netClose → close_all dropped 2
```

★★ **LGT는 KTF의 슬롯 30(`connect_named`)을 쓰지 않는다. 슬롯 3으로 접속한다.**
그 슬롯은 지금 `net::trace_stub(3, "MC_netSocketConnect?", 0)` 이라 **아무것도 하지 않고
0을 돌려준다** → 소켓이 실제로 연결되지 않고 게임은 응답을 기다린다 = 사용자가 보는
"로딩이 계속 뜬다". (그 전 버전들에서는 여기 오기 전에 죽었다.)

**다음 수**:
1. `0x14005e4` / `0x14005f4` / `0x5c734` 의 문자열을 읽어라 — `0x1f9b` = **8091**(포트)
   이고 `0x14005e4` 는 `.data` 문자열, `0x5c734` 는 `.text` 문자열이다. §2.5가 기록한
   LGT 서버 `210.222.18.28` / 앱키 `hero3lgt_wipi` 가 여기 나올 것이다. 프로브
   `--dump-mem 0x1400000:0x604` 로 이미 뜰 수 있고, `.text` 쪽은 binary.mod 를 그냥 읽으면
   된다.
2. **901 / 904 의 정체를 그 문자열로 확정**하라. 901 = `(port, 1, host, 0, cb, param)`,
   904 = `(imageStr, 1, host, host2, cb, param)` 모양이다. 목적지를 세팅하는 호출로 보인다.
3. **슬롯 3 을 실제 connect 로 구현**하라. `wie_wipi_c::api::net::connect_named` 가 이미
   `Platform::network()` 로 진짜 소켓을 열고 콜백을 쏘는 로직을 갖고 있으니, LGT의 인자
   모양(fd, …, cb, param)에 맞춘 얇은 어댑터를 만들어 슬롯 3 에 붙이는 것이 최단경로다.
   목적지는 `wie_android` 의 사설서버 리다이렉션이 이미 처리한다.
4. 그 뒤 send/recv 가 어느 슬롯인지도 같은 로그가 알려줄 것이다(스텁이 찍는다).

### (2) 퀵슬롯 아이콘이 "네모박스" — 그라데이션이 아니라 **아틀라스 셀** 문제다

★ **초기 진단이 틀렸다.** `overlay.rs::blit_icon` 의 테두리 그라데이션(`edge_band` 로
사각형 네 변을 스무스스텝 페이드, 2026-08-05 작업)은 **살아 있고 조건이 없다**. 그리고
일반 퀵바(`draw_quickbar`)와 몰입형 플레이트 **두 경로 모두** `blit_icon` 을 부른다.
그래서 그라데이션이 빠진 게 아니라 **가져오는 셀이 틀렸다**.

근거: 기기 로그 `#5 qb0 t=Some(1) i=Some(0) s=Some(45) cell=Some(5)` — 슬롯·셀은 맞다.
`items::skill_icon` 은 시트 id 를 **`CHAR+0xd1`(`SKILL_SHEET_OFF`)** 에서 읽고
`atlas = sheet_id - 4` 가 `<= 6` 이어야 한다. **`0xd1` 은 LGT 미검증 오프셋이다**
(§2.7이 검증한 것은 `0xe2..0xf8`, `0x9c6c`, `0x9e28`, `0xa0c0` 뿐).

**다음 수**: `--verify-offsets` 에 `CHAR+0xd1`(+`0xd2`)을 추가하고, 스킬을 퀵슬롯에 묶은
상태로 프로브를 떠서 시트 id 가 4..10 인지 확인. 아이템 쪽은 `item_db_icon` 이
`grid+0x38` 표에서 `+0x14` 를 읽는데 그 표는 §2.13에서 이미 검증됐다(프레임 17..28).

### (3) 메인메뉴 로고가 잘려/반복되어 보인다 — 코드는 있다, 게이트를 찾아야 한다

KTF 때 한 작업(가운데 로고 확대 + 양끝 그라데이션 + 화면 양쪽의 잘린 반복 제거)의
구현이 **`overlay.rs:2072 letterbox_shell`** 에 그대로 있다 — 주석에 "FEATHERED
gradient that starts INSIDE the artwork", "its rectangle simply STOPS — a razor-straight
vertical edge" 라고 그 문제를 적어 뒀다. 호출은 `app.rs:751`
(`if self.shell { overlay::letterbox_shell(...) }`).

**다음 수**: LGT에서 `items::shell_state` 가 타이틀/슬롯 화면을 `true` 로 보는지 확인.
그게 CHAR 상태 바이트 기반이면 LGT의 `state_off()` 는 이미 갈라져 있으니(0x78), 그보다
**셸 판정에 쓰는 상태값 자체**가 LGT에서 다를 가능성이 크다. 기기 로그의
`quickbar probe: … state=Some(N)` 을 타이틀 화면에서 찍어 보면 바로 나온다.

### (4) 크리티컬 발생 시 프레임 드랍 — **진동이 아니다** (사용자 실측)

환경설정에서 크리티컬을 off 로 해도 진동은 나는데 렉은 없다 → 원인은 진동/사운드가
아니라 **크리티컬 이펙트의 렌더링**이다. §2.17의 `blend565_lgt` 훅으로 필드 프레임은
-64% 가 됐지만 이 이펙트는 그 훅이 잡는 경로가 아니다.

**다음 수**: 이펙트가 화면에 있는 순간을 `--profile 1` 로 떠라. §2.17에서 블렌드 루프를
찾은 것과 **완전히 같은 방법**이다(균등 분포한 PC 뭉치를 찾아 디스어셈). 전투를 만드는
결정적 키 스크립트가 필요한데, §4-5가 요구하는 그 런과 같은 것이므로 한 번에 묶어라.

### (5) 호스트 SAVE(빠른저장/자동저장) — §4-3 절반 남음

§2.13이 `writeSaveRecord 0x1a264` / `writeDbRecord 0x1a1d4` / 이름 문자열 / 슬롯
`CHAR+0xa410` / 레코드 1408 B(8바이트 블록 ECB)를 확보했다. **남은 것은
`serialiseSave(u32* outLen)` 와 `slotSummary()` 두 함수 + out-param 스크래치.**
후보 사이트는 두 이름 문자열을 **함께** 로드하는 다섯 곳: `0xc018`/`0xc01e`,
`0x25170`/`0x25180`, `0x254b0`/`0x254c0`, `0x3b248`/`0x3b258`, `0x481fa`/`0x4820a`.
★ 버튼은 **숨기지 말 것** — 한 번 숨겼다가 사용자 지시로 되돌렸다(§2.16 말미).

### 그 밖에 남은 것

* **버프 표시**: 기기 로그가 `cash buffs: none live` 다 = 호스트 버프가 부여된 적이 없다.
  쓰신 아이템이 캐시 3종(`0x12`의 `0x11`/`0x12`/`0x18`)이 아니었을 가능성과, "HP바 우측"이
  호스트 상태줄이 아니라 **게임 자신의 상단 버프 밴드**(LGT에서 게이트 OFF)일 가능성이
  갈리지 않았다. 사용자에게 ① 어떤 아이템 ② KTF에서 어떤 모양이었는지 확인 필요.
* **`blend565_lgt` 는 LGT 01.00.07 에만** 붙어 있다. 01.00.04(`.text` 가 0x144 길다)와
  **KTF 는 아직 안 봤다** — KTF에도 같은 함수가 있을 것이므로 거기서도 큰 이득이 예상된다.
* **quickpage 페이지 복원(#14)**: `apply_quickpage_actions` 안의 `BindSkill` 만
  `SKILL_BIND_FN` 게스트 호출이 필요하다. `BindItem`/`Clear`(=`slot_writes`)는 §2.16에서
  검증된 오프셋만 쓰므로 분리하면 열린다.
* **오토루팅(#15)**: `CHAR+0x2d7` 게스트 쓰기, 검증 전에는 손대지 말 것(§4).

## 2.19 ★ 9차 — §2.18의 5건 전부 진행. 저장 체인 완주, 창고 무한로딩 원인 확정 (2026-08-11, 원격 리눅스)

> 이 회차는 **기기 없이** 진행했다(원격 리눅스, 프로브 + 정적 RE + 서버 코드).
> 에셋은 사용자가 첨부한 zip(md5 `049ff7be…` — §3.1의 정본 01.00.07과 동일)을 썼다.
> ★ **이후 픽셀/화면 검증은 전부 가로(496x240)로 한다** — 사용자 지시. 앱은
> 가로판으로만 완성한다. 이 회차의 letterbox·저장 E2E 실측도 496x240 이다.

### (1') 네트워크 — 창고 무한 로딩의 범인은 **서버의 핸드셰이크 파싱**이었다

§2.18-(1')의 "다음 수" 세 개를 전부 실행했다:

1. ★★ **`hero3_protocol.py` 가 `01 01 02`(KTF)만 핸드셰이크로 인정하고 있었다.**
   LGT 클라이언트는 `07 00 00 00 | 01 01 04` 를 보낸다(캐리어 바이트 `0x04`) →
   서버가 **무응답** → 핸드셰이크는 유일한 unsolicited 프레임이므로 클라이언트는
   WAIT에서 영원히 대기 = 사용자가 본 "창고 무한 로딩". 기기 로그의 "send/recv 는
   정상 동작"과 정확히 부합한다(§2.18-(1')이 예측한 그 시나리오).
   수정 = `HELLO_CARRIERS = {0x02: KTF, 0x04: LGT}` 로 수락 + selftest 2건
   (`01 01 04` 수락 / `01 01 03` 거절). `tls_e2e_test.py` 포함 서버 테스트 전부 PASS.
2. ★★ **슬롯 901/904의 정체를 정적으로 확정했다 — 기기 왕복 0회.**
   비결 = `.data`(0x1400000, 0x604 B)가 버퍼가 아니라 **16바이트 ARM 임포트 스텁
   표**라는 발견: 각 스텁 = `str lr,[sp,#-4]!; bl <resolver>; .word 테이블; .word
   인덱스`. 전체 덤프로 게임이 쓰는 WIPI 임포트 **64개 전 목록**이 나왔다(§2.18-(1')
   이 기기 런으로 얻으려던 것). `0x14005e4` = (0x1fb, **901**), `0x14005f4` =
   (0x1fb, **904**), `0x1400584` = 603 = `0x258+3`(슬롯3 connect — 교차검증).
   콜백 `0x18900` 디스어셈으로 계약 확정:
   * **901 = `htons`** — `901(8091)` 반환을 u16으로 잘라(`lsls#16;lsrs#16`) 슬롯3의
     `port` 인자로 넘긴다. 실인자는 r0 하나뿐(r1..r3은 stale).
   * **904 = `inet_addr`** — `904("210.222.18.28" @ .text 0x5c734)` 반환이 슬롯3의
     `addr` 인자. `connect_named` 가 `to_le_bytes()`로 옥텟을 읽으므로 network-order
     LE 워드가 정확히 맞물린다. 보조 경로 = `"210.222.18.8"`(0x5c744) + 포트
     0x8de(2270), 분기 플래그 `.bss 0x150ea74`.
   구현 = `wie_wipi_c::api::net::{htons, inet_addr}` + `wie_lgt` 디스패치(900번대
   그룹, 901/904만 named). 지금까지 스텁이 0을 돌려줘 슬롯3이 `(addr=0, port=0)`을
   받고 있었다(vc41 로그의 `0x0, 0x0` 설명 완료 — 리다이렉션 덕에 연결은 됐었다).
3. **남은 것**: 실기기에서 창고 열어 로그인 시퀀스가 서버 로그에 찍히는지 확인
   (`<- handshake 01 01 04 (LGT)` 가 첫 줄로 보여야 한다).

### (2) 퀵슬롯 아이콘 — 의심 지점 전부 무혐의. 남은 것은 기기 로그 한 줄

§2.18-(2)의 "다음 수"보다 멀리 갔다 — 네 겹을 전부 검증했고 **전부 정상**이다:
* **오프셋 0xd1은 LGT에서도 맞다 (정적 증명)**: 시트 로더 함수 `0x4957c`(템플릿
  `"res/skill/s4_bm"` @ `0x5e2e0` 사용)가 `ldrsb [base+i, #0x1d]` 로 시트 id를
  읽는데, base = 게터 `0x1068`(=`CHAR+0x28`) `+0x8c` 이므로 `0x28+0x8c+0x1d` =
  **`CHAR+0xd1+i` — KTF와 동일**.
* **런타임 값도 유효 (프로브 실측)**: 인게임 f=8200에서 `sheet[0]=6, sheet[1]=5`
  (4..=10 범위 안, REF 블록 lv=1/exp=1/814/hp=167 §2.7 기준값 일치).
* **아틀라스도 무혐의**: LGT 에셋(res/comm/item_bm + res/skill/s4..s10_bm)으로
  `gen_h3_icons.py` 를 돌려 체크인된 `icons.bin` 과 대조 — **82,998 B 바이트 동일**.
* **그리기 경로에 빌드 게이트 없음** (`draw_quickbar`/`draw_skill_buttons`).
따라서 "네모박스"는 이 넷 밖에 있다. 진단 라인을 심었다: `offset_audit` 에
**`#5b sheet=a/b`**, 프로브 `--verify-offsets` 에 같은 줄. 기기에서 아이콘이 박스로
보이는 순간의 `#5b` 값이 다음 한 수다(4..=10이면 blit 쪽, 아니면 시트 로드 타이밍).

### (3) 메인메뉴 로고 letterbox — 원인 = `shell_state` 의 liveness 게이트

`items::shell_state` 가 `resolve_char()?` 를 통과해야 true 인데, LGT(`Static` 앵커)의
`resolve_char` 는 **살아있는 캐릭터(레벨 1..70)** 를 요구한다. 셸 화면(타이틀/슬롯)은
정확히 캐릭터가 없는 곳이라 **LGT에서 letterbox 가 한 번도 그려진 적이 없다**
(KTF는 `Pointer` 앵커라 이 게이트가 없었다). 프로브 실측(240x320·496x240 둘 다):
이용안내/EA로고 = state 2, **타이틀→슬롯→클래스 = state 3**(레코드는 제로),
인게임 = 4, 메뉴 = 5 — `== 3` 판정 자체는 그대로 이식된다.
수정 = `items::char_base_raw()`(liveness 없이 앵커 주소만) 추가, `shell_state` 가
그걸 쓴다. 데이터 읽기는 전부 기존 `resolve_char` 유지.

### (4) 크리티컬 이펙트 렉 — 훅 후보 함수 확보: **테이블 블렌드 스프라이트 블리터 `0x2e8c4`**

§2.17과 같은 방법(차집합 프로파일). 인트로 전투가 **프레임 ~3000..4200** 에 있다
(OK 연타 스크립트 그대로, 496x240 몽타주로 확인). `--frames 2950`(전투 전) vs
`--frames 4200`(전투 포함) 프로파일 비교 → 전투 구간에서 뭉치
**`0x2ea38..0x2eba6`** 가 표본 배증(균등 분포 = 픽셀 루프).
* 함수 진입 = **`0x2e8c4`** — blit565(`0x2dd7c..0x2e8c2`) **바로 다음**. `_bm` 타입
  `0x0b` 검사가 있는 **팔레트 스프라이트** 블리터인데, blit565와 달리 **목적지
  픽셀을 읽어**(ldrh) 채널 분해 후 **32-스트라이드 룩업 테이블로 블렌드**한다
  (`table[(a+b)*32+c]` 모양, 픽셀당 목적지 읽기 1 + LUT 3). 테이블 = `.bss
  0x150eccc` / `0x150f8cc`(런타임 생성), 알파 인덱스 변환표 리터럴 `0x2ec64`.
* 반투명 이펙트(크리티컬 플래시/스킬 이펙트)가 이 경로다 — `blend565`(사각형)와
  `blit565`(불투명 스프라이트)가 못 잡는 세 번째 픽셀 루프.
* **다음 수** = blit565 때와 동일: 인자 계약 디스어셈 → 호스트 구현 →
  `--poke`/프레임 해시 A/B(픽셀 동일) → `binary_patches` 해시 엔트리.
  재현 런 = 이 회차의 인트로 전투 스크립트(결정적, 기기 불필요).

### (5) ★★ 저장 체인 — **완주했다.** 프로브가 호스트 체인으로 저장하고 게임이 그걸 로드한다

남았던 두 함수를 §2.13의 "다음 수" 그대로 찾았다: 두 이름 문자열을 함께 로드하는
다섯 사이트 중 **`0x2515e..0x25186` 이 저장 루틴 본체**고, 리터럴 풀이 계약 전체를
한 곳에서 준다:

```
0x2515e  outLen=0 (스택); r0=&outLen
0x25166  bx [0x25200]=0x1ad89 → serialiseSave(&outLen) -> buf
0x25172  bx [0x25204]=0x1a265 → writeSaveRecord([0x25208]="GameSave_11", buf, outLen)
0x25178  bx [0x2520c]=0x1a6d1 → slotSummary() -> buf16
0x25182  bx [0x25204]          → writeSaveRecord([0x25210]="SlotSave_11", buf16, 0x10)
```

* **`serialiseSave = 0x1ad88`**, **`slotSummary = 0x1a6d0`** (+§2.13의
  `writeSaveRecord 0x1a264`, 슬롯 `CHAR+0xa410`, 이름 `.text 0x5bbb4/0x5bbc0`).
* ★ **스크래치 = `.bss` 꼬리 `0x1512f00`** — KTF의 죽은코드보다 성질이 강하다.
  로더(`core.load`)가 섹션을 페이지 단위로 매핑하므로 선언된 `.bss` 끝(`0x1512958`)
  뒤 `0x1513000` 까지가 **매핑돼 있지만 ELF 밖**이다. 3중 증명: ① 정적 — `.text`/
  `.data` 어디에도 `0x1512958` 이상을 가리키는 리터럴이 없다(최대 `0x1512954`,
  리터럴+imm5 최대 도달 `0x15129d0`; 스크래치는 그보다 +0x530) ② 매핑 — `--poke`
  쓰기/읽기 정상 ③ 라이브 — 센티널이 부팅+7,800프레임 플레이+**게임 자신의 저장**
  을 그대로 통과(바이트 동일).
* **앱 배관** = §3.6 규율대로 per-item 표: `guest_anchors::SaveChain`(+`SaveNames`
  — KTF는 sl 포인터, LGT는 `.text` 주소 직접) / `save::Chain`(selftest 미러) /
  `save_gate`·`backup_target`·`save_scratch`·`emulator::save_begin` 전환 /
  `save_chain_supported()` 는 이제 표를 따른다 → **LGT에서 SAVE UI 복귀**.
  `save::plausible_ptr` 하한 `0x10_0000→0x1000`(LGT 절대 주소 모델).
  ⚠ backup(`FN_LOAD_SAVE_RECORD`)은 **여전히 KTF 전용** — `0x1a198`은 후보일 뿐.
* ★★ **vendored 수정 — `wie_lgt` 에 `JvmThreadGuard` 이식.** 첫 E2E에서
  serialise는 성공했는데 `writeSaveRecord` 의 WIPI DB 쓰기가
  `find_calling_class` 의 `threads.get(&id).unwrap()` 패닉으로 죽었다. KTF의
  `db2c704`(26차 부근) 와 동일 원인 — LGT emulator의 "JVM 어태치 불필요" 주석이
  DB 경로에 대해 틀렸던 것. `call_guest_function{,_out}` 둘 다 가드 적용.
* ★★ **E2E 검증 (기기 불필요, 가로 496x240)**: 새 프로브 플래그 **`--host-save
  <frame>`** 이 앱의 실제 스텝 머신(`save.rs` include!)을 그대로 몬다.
  ① 필드 f=8000에서 4단계 완주(`len=1392` — 1408-0x10 정확) →
  `GameSave_11_0`(1408 B)+`SlotSave_11_0`(32 B) 생성 ② **같은 `--db` 로 재부팅 →
  슬롯 화면에 캐릭터 요약(네오솔티아/버서커/LV1/PLAY TIME)과 "OK: 이어하기" →
  로드되어 f=1400에 인게임**(생성 경로면 ~7800프레임 필요하므로 로드 확정).
  호스트가 쓴 레코드를 게임이 읽고, 슬롯 요약까지 게임이 그린다 — §4-3 종결.

### 그 밖에 이 회차가 남긴 것

* 프로브 `--verify-offsets` 에 `#5b sheet[0]/sheet[1]` 줄, `items::offset_audit` 에
  같은 `#5b` 줄(기기 5초 주기).
* 셀프테스트: standalone **194**(anchors 16 — SaveChain 표 검증 추가, save 11 —
  두 체인 모두 스텝 머신 통과), `cargo test -p wie_android` 59, `hero3_server`
  selftest + `tls_e2e_test.py` 전부 PASS. `cargo fmt`/`clippy`(vendor) 통과
  (경고는 전부 기존 것).
* ⚠ 이 회차의 코드 변경 중 기기 확인 대기: 창고 로그인 시퀀스(서버 반응),
  letterbox 실화면, SAVE 버튼 복귀 후 실저장, `#5b` 시트 값.

## 2.20 ★ 10차 — 플레이 피드백 5건. HUD 두 위젯 이식 + 세 번째 픽셀 루프 훅 (2026-08-11, 로컬 Windows)

> 이 회차의 입력은 사용자 기기 테스트 피드백 5건이고, 전부 **LGT 실기기**에서 나온 것이다.
> 이 머신 정정: 릴리스 키스토어는 `<keystore>` 가 아니라 **`<keystore>`**
> 이고(§3.1 표 갱신), LGT `game.zip` 원본도 `<keystore>` 에 없다 — `binary.mod` 는
> **APK 에셋**(`android_app/app/src/main/assets/hero3_fixed.zip` → `0002865E.jar`)에서 꺼냈다
> (md5 `6a352944…`, 정본과 동일). 기기 = **SM-G960N**(adb `<S9_SERIAL>`), 연결 확인됨.

### (1) 메뉴 D-패드 화살표 — 호스트만, 게스트 무관

`overlay.rs` 세 곳: `dpad_arms` 가 각 화살표의 **크로스축만** 7/6 으로 늘리고(수직 화살표는
가로로, 수평 화살표는 세로로 — 삼각형의 **밑변**이 놓인 축이 그쪽이다), `left_column` 의
기본 span 이 `5/4 → 11/8`, `draw_triangle_face` 의 라운딩이 `0.10 → 0.20`(짧은 변 기준).

★ 같이 고친 버그: `draw_triangle_face` 가 `bx = hw, by = hh*TIP` 를 **방향과 무관하게** 썼다.
정사각형 셀에서는 같은 값이라 지금까지 문제가 없었지만, 크로스축을 늘린 순간 LEFT/RIGHT
화살표가 **가늘고 긴 창**이 된다. 이제 `along`/`perp` 를 방향별로 나눈다.

라운딩 상한은 **내접원 반지름**이다(0.27~0.30 × 짧은 변). 0.20 이면 침식비 k ≈ 0.3 —
"훨씬 더 둥근" 삼각형이면서 방향은 남는다. 바운딩 사각형은 그대로이므로 `placement` 의
충돌 검사는 하나도 건드릴 필요가 없다(넓어진 중앙 행/열이 들어가는 곳은 **비어 있는 코너
셀**뿐이다).

### (2)(3) ★★ 게임 자신의 상단 HUD 두 위젯 — LGT 사이트 확보. §4.5의 "안 한다"를 뒤집었다

§4.5 는 "밴드/타겟박스/hud_y 미세 조정은 LGT에서 굳이 안 한다 — 게이트가 꺼서 게임 기본
배치로 동작하고 그게 정상으로 보인다"고 적어 뒀다. **실기기가 그걸 반증했다**: 게임 기본
배치는 우리 오버레이와 **같은 자리**를 쓴다 — 버프 밴드는 좌상단 HP바 위에 겹치고,
타겟 몬스터 박스는 우상단 미니맵 **뒤에** 깔린다. 사용자 보고 그대로다.

호스트 배관은 이미 전부 있었다(`overlay::band_x_shift` / `target_box_shift`,
`app.rs::relayout` 의 packed 아톰, `emulator.rs` 30프레임 poke 패스). 막고 있던 것은
`items.rs` 두 함수의 `guest_code_addresses_valid()` 게이트 하나뿐이었다.

**렌더러 = `0x22a4`**(§2.8 이 하단 블록 사이트 `0x2864` 로 이미 확보한 그 함수). `0x22a4`
부터 `0x2870` 까지 디스어셈해서 두 위젯을 산술로 식별했다:

**버프 밴드** (루프 `0x2326`..`0x2482`, 가드 = `(s16)*(CHAR+0x16c) > 0` = 버프 개수,
카운터 = `[sp,#0xa0]` 를 u8 로):

| 사이트 | 명령 | 의미 |
|---|---|---|
| `0x2372` | `adds r1,#1` | 프레임 `x = w*i + 1` |
| `0x23ec` | `adds r5,#3` | 아이콘 `x = w*i + 3` (프레임 +2) |
| `0x2450` | `adds r1,#1` | 잔여틱 숫자 `x = w*i + w/2 + 1` |

`w` = `ldrsh [LAY,#0x30]` = ui_bm 프레임3 폭(20) — KTF와 동일. ⚠ 중간의
`0x237c movs r3,#0x14` 는 **20바이트 버프 레코드 stride** 다(`muls r3,r5,r3`, r5 = i),
좌표가 아니다 — KTF `0x149c48` 이 적어 둔 그 함정이 여기도 있다.

**타겟 몬스터 박스** — KTF는 12사이트인데 **LGT는 10**이고, 그 차이가 구조적이다.
선택자(`0x2484`, KTF와 같은 바이트)가 경로 A(`0x2498..0x2608`)/경로 B(`0x2660..0x27e2`)를
가르는 것은 같지만, **HP바 채우기 두 개가 공유**된다 — 경로 A는 `0x2608 b 0x27e4`,
경로 B는 `0x27e2` 에서 같은 `0x27e4` 로 흘러 들어간다.

```
   경로 A                 경로 B                 공유 (0x27e4~)
   0x252a subs r1,#2      0x2706 subs r1,#2      0x282e adds r1,#0x1f
   0x2560 adds r1,#1      0x273c adds r1,#1      0x285a adds r1,#0x1f
   0x259a subs r1,#0x2c   0x2776 subs r1,#0x2c
   0x25f4 adds r1,#0xf    0x27d0 adds r1,#0xf
```
`W` = `(s16)[sp,#0x94]`(= `ldr r3,[ctx,#0x44]` @ `0x22f2`), `FW` = `ldrsh [[r7],#0x40]`(85).
모든 사이트가 `W - FW + K` 를 **각자 다시 계산**하므로 KTF와 마찬가지로 공유 소스는 없다.
상수 다중집합은 두 빌드가 **동일**하다(`-2 / +1 / -44 / +15 / +31 / +31`) — 같은 C 다.
`TARGET_BOX_SHIFT_MAX = 211`(= 255-44)도 그대로 성립한다.

플래그 안전성도 확인했다: 열 사이트 전부 직후에 **또 다른 플래그 세팅 ALU** 또는 `bl` 이
와서, 조건분기가 오기 전에 플래그가 다시 쓰인다 → ADDS↔SUBS 재인코딩이 안전하다.

**검증** (세 겹):
1. **정적** — 열 halfword 전부 표의 `orig` 와 바이트 일치(`0x3902/0x3101/0x392c/0x310f/0x311f`).
2. **라이브** — 프로브 `--poke` 10발(shift 60)의 읽기-되돌림 로그가 정확히 그 원래 값을 찍었다:
   `0x252a: 0239 -> 3e39 (ok)` … 열 줄 전부 ok.
3. **호스트 산술** — `landscape_selftest` 57 tests(미니맵 dodge/겹침 없음 포함) 통과.
   실측 shift: 496x240 → band_x=153, box_shift=128 / 2340x1080 → band_x=110, box_shift=65
   (전부 u8·211 인코딩 안).
⚠ **화면 검증은 기기 몫이다**: 인트로 전투(f=3000..4200)에는 **타겟 박스가 안 뜨고**(poke A/B
프레임 7/7 완전 동일 = 그 블록에 도달하지 않음) 버프도 안 걸린다. 몬스터를 타겟한 상태와
버프를 쓴 상태는 프로브로 만들 수 없다.

**앱 반영** — §3.6 규율대로 per-item 표: `guest_anchors::GuestBuild::{band_x_sites,
target_box_x_sites}`(`&'static [(addr, opcode, delta)]`, 빈 슬라이스 = 미확보),
`items::{band_x_patches, target_box_x_patches}` 가 그 표를 돌린다. 상수 배열이 있던 자리에는
**게임에 대한 설명만** 남겼다(주소는 빌드별, 이유는 빌드 무관).

부수 확보(적어만 둔다, 아직 안 씀):
* **hud_y 원점 = `.bss 0x1503a50`** 직접 정적(KTF의 `*(*(sl+0x428))` 이중참조가 아니다).
  밴드 프레임 `+2`, 아이콘 `+5`, 숫자 `+h-7`, 타겟박스 `+2/+5/+6/+11` 이 전부 여기서 온다.
  `hud_y_shift()` 가 0을 돌려주므로 지금은 필요가 없다.
* ⚠ **§2.6의 `+0xa384 → +0xa370` 은 기준이 다르다**: `0xa370` 은 접근자 반환값
  (`0x1068` = `CHAR+0x28`) 기준이므로 CHAR 기준으로는 **`+0xa398`** 이다(리터럴 `0x262c` =
  `0xa370`, 로더 `0x232c`). `band_height_poke` 는 KTF 전용으로 남겨 뒀으니 지금 영향은 없지만,
  LGT에서 밴드 높이를 만질 일이 생기면 **`0xa398` 로 라이브 확인부터** 할 것.

### (4) ★ 퀵슬롯 페이지 버튼 — 게이트 하나가 기능 전체를 삼키고 있었다

`apply_quickpage_actions` 첫 줄이 `if !guest_code_addresses_valid() { return; }` 였다.
그래서 LGT에서 탭은 **스토어만 뒤집고**(페이지 인덱스·파일·UI 이벤트는 전부 정상) 게스트
퀵바는 그대로 = 사용자가 본 "동작을 안 한다". 게스트 CALL 이 필요한 것은 `BindSkill`
**하나**뿐이고, `Clear`/`BindItem` 은 §2.16 이 LGT에서 검증한 네 오프셋에 대한 데이터 쓰기다.

* `GuestBuild::quickbar_writes_ok()` — KTF/LGT true, SKT false. 근거는 §2.16 의 팝업 A/B
  (게임 자신의 바인더가 `CHAR+0x252`=2, `CHAR+0x257`=6 을 쓰는 것을 관측). `slot_writes` 가
  이 게이트를 쓴다.
* `GuestBuild::skill_bind_fn()` — KTF `0x145c48|1`, LGT **None**. LGT 정적 탐색 두 경로 모두
  빈손이다: 퀵바 배열을 가리키는 리터럴이 `.text`/`.data` 에 **없고**(`getter() + imm` 으로
  접근한다), 게임 자신의 바인더는 §2.16 이 이미 합성 `OK` 로 몰 수 있다고 보인 팝업
  디스패처 안에 있다.
* ★ **그래서 `None` 인 빌드는 스킬도 raw-write 로 복원한다.** `Entry::Skill` 에
  **`sref`(게임이 써 놨던 스킬-ref 바이트 그대로)** 를 추가했다. 모듈 헤더의 "바이트가 아니라
  아이덴티티를 저장한다" 규칙과 충돌하지 않는다 — 그 규칙은 **아이덴티티**에 대한 것이고
  (파티가 바뀌면 `(cat,cell)` 이 남의 스킬을 가리킨다) 그건 `class` 스탬프가 지킨다.
  `skill_bind` 가 추가로 하는 일은 (a) 습득 검사 (b) `unbind_ref` 중복제거 (c) `cell` →
  ref 패킹인데, (a) 캡처 시점에 묶여 있었고 게임에 unlearn 이 없다, (b) 모든 플랜이 5칸을
  먼저 비운다, (c) **`sref` 가 그 패킹 결과 바이트 자체**다.
* 파일 포맷은 **append-only**: `s <p> <s> <cat> <cell> <class> [<sref>]`. 6필드(구버전) 도
  그대로 읽고, 그 엔트리는 `skill_bind` 가 있는 빌드에서만 복원되며 없으면 이유를 로그로
  남기고 칸을 비운다. `quickpage_selftest` 에 왕복 테스트 추가(16 tests).

### (5) ★★ 크리티컬 이펙트 프레임드랍 — 세 번째 픽셀 루프를 훅했다. **픽셀 7/7 동일, 명령 -24 %**

§2.19-(4)가 잡아 둔 후보 함수 **`0x2e8c4`** 를 완전히 해독해서 호스트로 옮겼다
(`vendor/wie/wie_core_arm/src/binary_patches/blitlut565.rs`, `HookKind::BlitLut565Lgt`).

**정체** = `blit565`(불투명 스프라이트)와 `blend565`(반투명 **사각형**)가 못 잡던 것 —
**반투명 스프라이트**. 인자는 `blit565` 의 13개 + 2개(스프라이트별/호출별 블렌드 레벨)이고
인덱스도 순서도 같다(그 자체가 인자 해독의 교차검증이다). 4bpp 전용(`cmp r3,#0xb`, 아니면
`0x3064a` 로 떠난다), 인덱스 0 투명, 팔레트 RGB565, `pixels = data + 2*pal + 9`,
`rowBytes = ceil(pitch/2)`, 홀수 srcX 는 **LOW 니블**부터 — 전부 `blit565` 와 동일.
다른 것은 **목적지 픽셀을 읽어** 32-stride 바이트 LUT 두 단으로 블렌드한다는 점이다:

```
sR=si>>11, sG=(si&0x7e0)>>6, sB=si&0x1f     ; ★ 초록을 5비트로 반으로 접는다
dR=di>>11, dG=(di&0x7e0)>>6, dB=di&0x1f     ;   → 32x32 표 하나가 세 채널을 다 본다
r1=T[t][sR][dR]  g1=T[t][sG][dG]  b1=T[t][sB][dB]
a > 0xfe ?  R,G,B = r1,g1,b1                 ; 0xff = 2단 생략
         :  R=U[a][r1][dR]  G=U[a][g1][dG]  B=U[a][b1][dB]
store (R<<11) | ((G*2)<<5 & 0x7e0) | (B & 0x1f)
```
`t` 는 인자 자체가 아니라 **`alpha_map[인자]`** (`.data` 바이트 표). 표 주소 셋은 리터럴
풀에서 읽었다: `0x2ec64` → `alpha_map = 0x14001bf`, `0x2ec68` → `T = .bss 0x150f8cc`,
`0x2ec6c` → `U = .bss 0x150eccc`. 빌드별이라 **toml 필드**로 넘긴다(하드코딩 안 함).

★ **표 읽기가 절대 밖으로 안 나가는 증명**: 한 plane = 32*32 = 1024 B 이고 1단은
`s*32 + d`(둘 다 5비트)라 구조적으로 in-plane. 2단은 **1단 결과**로 인덱스하는데 그건 표
바이트라서 5비트라는 보장이 없다 → 그래서 **호출마다 plane 1 KB 를 훑어 31 초과 엔트리가
있으면 거절**한다. 그러면 2단 인덱스가 `<= 31*32+31` 로 증명된다. 스프라이트 한 장 값에
비하면 1 KB는 공짜다.

★ **초록의 최하위 비트는 원래 사라진다** — 이 함수의 성질이고 우리 이식의 손실이 아니다.
양쪽 초록을 반으로 접어 표를 보고 결과를 두 배로 쓰므로 6비트 초록 필드는 **항상 짝수**다.
유닛테스트 3개가 그 마스크(`0xffdf`)를 기준으로 항등 plane 왕복을 검증한다.

**거절 조건**(전부 `blit565` 와 같은 규율 — 거절 시 밀어낸 `push {r4-r7,lr}` 를 직접 수행하고
2바이트 뒤로 재개하므로 최악이 "예전과 같음"): transform ≠ 0(게스트는 transform 별로 루프
사본을 통째로 갖고 있고 `0x2ec70` 이후는 안 봤다), 4bpp 아님, 음수 원점, 행이 목적지
stride 를 넘음, 표 plane 을 못 읽음. 쓰기는 **모든 읽기 뒤**에만 일어난다.

**A/B 측정** (496x240 가로, 인트로 전투 포함 4,300프레임, OK 연타 결정적 스크립트,
`--hle 1` 고정, 차이는 **toml 엔트리 유무 하나**):

| | 훅 없음 | 훅 있음 | 감소 |
|---|---|---|---|
| f=4300 누적 명령 | 2,030,331,028 | **1,541,433,009** | **-24.1 %** |
| f=3800→4000 구간 | 114.4 M | 81.1 M | -29.1 % |
| f=4000→4200 구간 | 116.2 M | 80.3 M | -30.9 % |
| f=4200→4300 구간 | 45.3 M | 27.2 M | **-40.0 %** |
| 가상시계 | 4.17x | **5.27x** | +26 % |

* **덤프 프레임 7/7 바이트 완전 동일**(3000/3200/3400/3600/3800/4000/4200). 훅 없는 런에서는
  게스트가 같은 픽셀을 그렸다는 뜻이므로, 이 비교가 **이 훅만** 고립시킨다.
* 이 함수 호출 수 = **25,355**(= 956,530 - 931,175). 그 중 host 가 그린 것 16,479(transform 0,
  전부 4bpp, fmt 거절 0), 나머지 8,876 은 transform ≠ 0 거절(t1=505, t2=653, **t4+=7,718**).
  → **다음 최적화 후보**: 이 함수의 transform 4~8(전치) 사본이 자기 호출의 30 %다.
  `blit565` 도 전치는 거절하므로 둘 다 같이 남은 항목이다.
* `--hle 0`(스프라이트 훅 둘 다 무력화) 대비도 7/7 동일 — 즉 blit565 회귀도 없다.

### 셀프테스트 / 빌드

standalone **198**(anchors 16→**19**: band_x/target_box 표 + quickbar 게이트 3건 추가,
quickpage 15→**16**: sref 컬럼 왕복), `cargo test -p wie_core_arm blitlut` **3**,
`cargo ndk check` 경고 0(기존 것 2개뿐), `cargo clippy -p wie_core_arm` 새 경고 0.

⚠ **`cargo fmt` 를 루트 워크스페이스에 돌리지 말 것** — 이 저장소의 `wie_android`/`wie_probe`
는 fmt-clean 이 아니다(HEAD 에서 24개 파일이 재포맷된다). 이 회차에 한 번 돌렸다가 전부
되돌리고 편집을 수동 재적용했다. vendor 워크스페이스는 `vendor/wie/AGENTS.md` 가 요구하는
대로 fmt 한다(그쪽도 기존 드리프트 4개 파일은 건드리지 않았다).

### 기기 확인 대기 (이 회차 전부)

1. 화살표 크기/둥글기 — 메뉴 열고 눈으로.
2. **버프 밴드가 HP/SP/EXP 바 오른쪽**에 오는지 — 캐시버프 또는 게임 자체 버프를 걸고.
   로그 `buff band x: 0x2372 <- N` 세 줄이 먼저 떠야 한다.
3. **몬스터 HP/이름 박스가 미니맵 왼쪽**인지 — 몬스터 타겟 상태. 로그
   `target box x: 0x252a <- 0x...` 열 줄.
4. 퀵슬롯 페이지 버튼 — 1↔2 전환, 스킬/아이템이 실제로 갈리는지. 로그
   `quickpage: now on page N (M action(s))`.
5. 크리티컬 이펙트 렉 — 체감. 30초 게이지의 `blit` 비율/`instr` 도 같이 볼 것.

## 2.21 ★ 11차 — 캐시샵 카탈로그 21종 → 58종. "마커 = 캐시" 가 애초에 틀렸다 (2026-08-11)

사용자: "명왕 이라는 문구를 가진 캐시템 존재하지 않아? 그런 캐시아이템들이 엄청나게 많은데 그
모든 항목들을 그럼 네트워크 창고 캐시샵에 구현해놔야 해." → 결정 **B**(정통 `}` + 초록 `'` 전부).

정본 = **`hero3_server/CASH_SHOP.md` 머리말**(정정 3건 + 전체 표). 여기는 게스트/에셋 쪽 근거만.

### (1) 이름 앞 바이트는 **텍스트 색상 이스케이프**다 — 캐시 플래그가 아니다

클라이언트 rodata **`0x5b2bc`** = `40 7c 24 7b 7d 27 22` (`@ | $ { } ' "`) 가 이스케이프
알파벳이고 **`0x5b268`** 이 7×RGB 표다. 이 알파벳을 읽는 코드는 바이너리 전체에 **두 곳,
둘 다 폰트 레이어**: `isEscape()` `0x35d8`(0폭 처리) / `applyColourEscape()` `0x37e4`
(펜 색 → `.bss 0x150e75c`, 글리프 루프는 그리지 않고 전진). `}` = 금색(219,154,26),
`'` = 초록(120,196,55). `.text` 에 0x7d/0x7b/0x7c 와 비교하는 명령은 **없다**.
→ **"`}` = 캐시템" 은 사설서버의 표기 관습**이다. 데이터가 그 관습을 반증한다(금색을
이벤트투구가, 초록을 명왕 세트가 단다).

### (2) 장비 DB 는 i15 가 아니라 **평문 i0..i12** 다

`CASH_SHOP.md` 의 "cat 0x0f(i15) = EQUIPMENT, 캐시 0종" 은 **파일 배정이 틀렸다**.
i15 = **레시피** 표이고(클라의 `05 41` 이 cat 0x0f 를 거부하는 이유), 장비는 평문
`i0..i12` = cat 0x00..0x0c 다(클라 자신의 게이트가 `cat <= 0x0c`, 게스트 `0x176d10`).
2026-07-23 이 "DES 노이즈" 로 기각한 **16이라는 수는 맞았고 파일이 틀렸다** — 공허 세트
12 + 이벤트투구 4 = 16 이 평문 장비 카테고리에 있었다.

### (3) ★★ 장비를 팔려면 20바이트 중 **네 바이트**를 고쳐야 했다

구입 지급의 in-bag 레코드는 **05 3f 목록 행**에서 온다(게스트 `0x1580a6` 가
`P+0x320 + row*0x14` 에서 20B 복사). 그리고 장비 카테고리에서는 그 중 **네 바이트가 검증
없이 인스턴스로 복사된다**:

| 와이어 | 인스턴스 | 뜻 | writer |
|---|---|---|---|
| `rec[0x10]` | `+0x3c6` | 총 소켓 수 | `0x176f96` |
| `rec[0x11]` | `+0x3c5` | 제련도 | `0x176f5c` |
| `rec[0x12]` | `+0x3c9` | 박힌 소켓 수 | `0x177002` |
| `rec[0x13]` | `+0x3c8` | 박힌 오브 id | `0x176fcc` |

기존 `_cash_item_record` 는 이 꼬리를 `ff` 로 채웠고, 제련도는 **부호확장해서 읽힌다**
(`0x1771a8` `ldrb; lsls #0x18; asrs #0x18`) → **-1**. 게다가 세이브 직렬화가 네 바이트를
전부 기록한다(`0x195c5e`/`0x195c9c`/`0x195cda`/`0x195d18`) → **세이브에 영구 손상**.
정답은 추측이 아니라 **게임 자신의 신규장비 초기화값**이다: 제련도 0(`0x14e4aa`), 총 소켓
rand(1,5)(`0x14e526`), 박힌 0(`0x14e594`), 오브 id -1(`0x14e560`). 그래서 샵 행은
소켓 1 / 제련도 0 / 박힘 0 / 오브 0xff 를 싣는다. 비장비 카테고리(0x0d..0x12)는
`+0x3c4..+0x3ce` 를 아예 건드리지 않으므로 기존 `ff*8` 그대로 둔다.

★ 부수 정정 둘:
* **05 40 지급 게이트는 `payload[0xa]` = 되돌려준 키의 첫 바이트 = 아이템 index** 다
  (`CASH_SHOP.md` 이 amount 라고 적었던 것). → **index 0 인 행은 사도 안 온다.**
  셀프테스트가 카탈로그 전체에 대해 검사한다.
* 캐시 구입 지급은 클라가 **키를 재생성**한다(`0x580ae`, WIPI iface+0x70) — 서버의 합성
  serial 은 가방에 안 남는다. 반면 창고 내리기(05 04)는 20B 를 그대로 복사하므로 그쪽은
  서버가 키 유일성을 지켜야 한다.

### (4) 페이저가 2페이지 전용이라 **인덱스 32 이후를 잃고 있었다**

`05 3f` 커서는 오프셋이 아니라 `anchor ± 1` 이고 `anchor = min(A,N) ≤ 16` 이라 **0..17**
밖으로 못 나간다. 2페이지는 커서로 구분됐지만 4페이지는 0페이지의 next 와 1페이지의 next 가
**둘 다 17** 이라 원리적으로 불가능하다 → 페이지 번호를 **세션 상태**로 옮기고 커서를 방향으로
읽는다(`cash_page_window`). 리셋 = **`05 14`**(기기 실측 오픈 순서 `05 3d → 05 14 → 05 3f
cursor=0`). 마지막 창이 **1행**이면 그 prev 커서가 0(=오픈과 구분 불가)이라 창을 겹쳐 잡는다.

### 앱/에셋 쪽

★ **LGT 에셋에 캐시템 이름 패치가 한 번도 안 들어가 있었다.** 실린 zip 은 1,763,788 B 로
`럭키스톤 / 네트워크 재료 드랍 2배` 원본 그대로였고, 호스트는 그 아이템으로 **EXP 2배**를
준다(HUD 버프 이름은 이미 `경험의비석`). `patch_asset.py --apply` 로 0x11/0x12/0x18 세 항목을
고쳐 **1,764,382 B**, 원본은 `hero3_fixed.zip.orig`. `binary.mod` 는 바이트 동일 → 바이너리
패치 해시 엔트리 영향 없음(도구가 다른 멤버 전부 동일성을 검증한다).

### 부수 확보 — 캐시샵 화면은 **서버 응답이 여는 것**이고, 호스트에는 recv 타임아웃이 없다

병렬 조사(6갈래)가 세 가지를 더 확정했다:

* **3탭 화면(SINGLE/NETWORK/CASH) = 스크린 상태 3**, 그리고 `.text` 전체에서 그 상태를
  세우는 곳은 **둘뿐**이다 — `0x13c38`(**`05 3f` 응답 핸들러 꼬리**)와 `0x0aafa`(DETAIL
  복귀). 행 수 `module+0x30c` 의 writer 도 `0x13b3e` 하나, 같은 핸들러 안이다.
  → **캐시샵 화면은 서버의 `05 3f` 응답 없이는 존재조차 하지 않는다.** 탭 전환은 로컬
  (키 핸들러 `0xa9bc`, `0xaa5c`=키1 / `0xaa7a`=키3) 이지만 그 핸들러는 상태 3에서만 돈다.
* **`05 3f` 요청 빌더 = `0x78de..0x790c`** — class 5 / op 0x3f / 12바이트 바디, 커서 =
  `*(P+0x308) + 1`(저장된 anchor 를 읽어 +1 하고 **되쓴 뒤** 바디에 넣는다). 카테고리·탭
  필드는 **없다** → `CASH_SHOP.md` 가 "다음 세션 아이디어"로 남겨둔 항목이 닫혔고, N페이지
  페이저가 커서를 방향으로 읽는 근거가 정적으로도 확인됐다. 단 **찾은 것은 forward 빌더
  하나뿐**이라 prev 가 요청을 보내는지는 미확정이다.
* ⚠ **호스트 recv 에는 폴 상한이 없다** — `wie_wipi_c/src/api/net.rs` 의 `ReadWaiter` 루프는
  무한이고(connect 는 `MAX_POLLS=240`, `network.rs` 의 8초 상한도 connect 전용), 그래서
  서버가 한 프레임을 안 보내면 "요청중입니다" 모달이 **영구히** 남는다. 창고·캐시샵의 모든
  무한로딩 증상이 이 한 줄로 설명된다.
* 부수 버그(현재 무해): net slot 0 `connect` 가 즉시 0을 반환하면서 콜백까지 spawn 해
  게스트가 `0x18900` 을 두 번 타고 **소켓이 fd 3, fd 4 두 개** 생긴다. `network.rs` 의
  대기 fd promote 가 우회하고 있어 동작에는 영향이 없다(9차에 그걸 넣은 이유이기도 하다).

### 검증

`hero3_server/selftest.py` **158 PASS**(+6: 장비 카탈로그·tag 0x11 충돌·인스턴스 4바이트·
index 0·페이저 크기 1..80 프로퍼티), `tls_e2e_test.py` ALL PASS. 실기기 대기 = 장비 구입
지급이 실제로 정상 아이템으로 들어오는지(초록 20행이 의심되면 `HERO3_CASH_TOP_TIER=0`).

## 2.22 ★ 12차 — 최종티어(`@`흰색) 48행 + 오토루팅이 **한 번도 켜진 적 없었다** (2026-08-13)

사용자 질문 두 개에서 출발했다. ① "카이져스톰 이라는 아이템이 있는지 확인해줘. 최종무기들을
캐시샵에서 팔았었다고 하는데" ② "오토루팅 인게임 default 처리가 안되어있어."

### (1) `@`흰색이 사다리의 끝이다 — §2.21의 초록 20행은 top-tier가 아니었다

**카이져스톰은 실존한다**: `i7_dat`(cat `0x07`, 단총) idx **`0x1d`**, 요구 Lv 70, 317/47,
원 159,500. 그리고 혼자가 아니다 — 12개 장비 카테고리 **전부**에 `@`흰색 등급이 있고
캐시샵은 그걸 **하나도 팔지 않고 있었다**.

20바이트 스탯 블록을 13개 파일 전체로 디코드해서 확정한 레이아웃:

| 오프셋 | 뜻 |
|---|---|
| `b0..3` | u32 원-가격 |
| `b4` | 아이콘/스프라이트 id |
| `b5` | 세트/효과 id (`0xff` = 없음) |
| **`b7`** | **티어** |
| **`b8`** | **요구 레벨** |
| `b9..b11` | 세트/외형 |
| `b12,13` | u16 주스탯 (무기=공격력, 방어구=방어력) |
| `b14,15` | u16 보조 (무기만) |
| `b16..19` | (스탯id, 값) 보너스 2쌍 |

★ **`b7` 이 이름의 색 이스케이프와 12슬롯 전체에서 1:1 로 맞는다**: 무색 0 · `|`노랑 1 ·
`'`초록 **2** · `$`하늘 3 · `{`빨강 4 · `@`흰색 **5**. (`}`금색 공허는 티어 4 — 금색은
텍스트 색이고 등급이 아니다.) `i12`(악세, 18바이트)는 같은 레이아웃이 2바이트 앞이라
티어가 `b5` 다. 즉 **초록은 하늘·빨강보다 아래**고, §2.21이 "top rung" 이라 부른 20행은
그때까지 디코드된 마지막 등급이었을 뿐이다.

### (2) 왜 최종템이 카테고리마다 **두 번** 있는가

12개 파일 전부가 사다리 끝을 다시 발행하는 **두 번째 블록**을 뒤에 달고 있고, 규칙이 12곳에서
동일한 순수 상향이다:
* `{`빨강 티어 4 였던 레코드가 `@`흰색 티어 5로, 레벨↑·위력 약 1.7배로 재등장
* 이미 `@`흰색이던 레코드는 **같은 레벨·같은 가격에 주스탯만 약 1.7배**로 재등장

예) cat `0x07`: `0x16 {템페스트` Lv64 201/30 → `0x1c @템페스트` Lv64 287/43,
그리고 `0x17 @카이져스톰` Lv70 226/33 원159,500 → **`0x1d @카이져스톰` Lv70 317/47 원159,500.**

그래서 **높은 인덱스가 정본**이고 낮은 쪽은 모든 항목에서 열등하다(더 싸지도, 레벨이 낮지도
않다). 샵은 **이름당 한 행**만, 강한 쪽을 싣는다 — §2.21이 이벤트투구 4스킨에 내린 것과 같은
판단이고 이유도 같다(같은 이름에 스탯만 나쁜 두 번째 행은 버그로 읽힌다). 열등한 쌍둥이는
`FINAL_TIER_VARIANT_ITEMS` 에 남겨 두어(기본 OFF) DB 레코드는 하나도 도달 불가가 아니다.

**`FINAL_TIER_EQUIP_ITEMS` = 48행** (투구 5·갑옷 7·장갑 5·신발 5·무기 7종×3·방패 2·악세 3).
카탈로그 56 → **104행 / 7페이지**. 지급은 새로 할 일이 없다 — 전부 `cat <= 0x0c` 라
§2.21-(3)의 소켓/제련도 4바이트가 이미 적용된다. 재생성은
`python tools_re/dump_item_db.py --colour white`.

### (3) ★★ 오토루팅은 **출고본에서 한 번도 강제된 적이 없다** — 그리고 캐시템도 빠져 있었다

`items::autoloot_to_force` 는 첫 줄이 `if !guest_code_addresses_valid() { return None; }`
였고 그 플래그는 **KTF 전용**이다. APK 는 LGT 에셋을 싣는다 → `emulator.rs` 의 30프레임
강제 루프가 **실기기에서 그 바이트를 쓴 적이 단 한 번도 없다**. 그런데 서버는 "호스트가
상시 ON 으로 강제하니 이미 가진 것을 파는 셈"이라며 `오토루팅`(cat `0x12` idx `0x0d`)을
**delist** 해 놨다. 호스트 기능 하나를 두 곳이 서로 있다고 가정해서 **플레이어는 둘 다 못
받고 있었다.**

처리: **캐시템 재등재**(`CASH_SHOP_DELISTED` 에서 `0x0d` 제거, `AUTOLOOT_IDX` 로 명시).
게임 자신의 검증된 `useItem` arm 이 자기 플래그를 세우므로 위험이 0이고 **오늘 동작한다**.
그리고 게이트를 §3.6 규율대로 항목별로 쪼갰다 — `guest_anchors::autoloot_off()` =
KTF `Some(0x2d7)` / LGT `None`. LGT 오프셋이 확정되면 그 표 한 줄로 호스트 강제가 되살아난다.
`emulator.rs` 는 부팅 때 한 번 **강제 가능 여부를 로그로 말한다**(이번 회귀가 조용히 지나간
이유가 그것이므로).

### (4) KTF `useItem` 은 이제 표까지 검증됐다

`cmp #0x10` @`0x153f02` → 17엔트리 점프테이블 `0x1a923c`(엔트리는 **테이블 자신에 대한 상대
오프셋**, 베이스는 `sl + 0xffff65fc`). 읽어 보니 이 저장소가 인용해 온 arm 이 전부 재현된다:
`[0x07] 오브원석 → 0x15403e` · `[0x0a] 비밀의상자`/`[0x0b] 부활의서 → 0x154542`(소비만 =
클라 효과 없음) · `[0x0c] 컵라면 → 0x1540cc` · `[0x0e] 여행가방 → 0x154106` · 그리고
**`[0x0d] 오토루팅 → 0x1540f6`**. 그 arm 5줄이 `CHAR+0x2d7` 의 출처다.

### (5) ★★ LGT 오토루팅 = **`CHAR+0x2d8` 에 `0x81`** — 기기 실측. 세 번 틀린 뒤에

정적 RE 는 후보에서 멈췄고, 그 후보는 물론이고 **그 뒤 두 개도 틀렸다.** 결정한 것은
디스어셈이 아니라 측정이고, 결정적으로 **감시 창을 레코드 전체로 넓힌 뒤에야** 나왔다.

**틀린 답 셋 (전부 기기가 배제)**

| 후보 | 왜 그럴듯했나 | 어떻게 죽었나 |
|---|---|---|
| `0x2dd` | CHAR 게터로 도달하는 그 구역의 **유일한 `strb #1`** | 두 시행 내내 **한 번도 변하지 않았다** |
| `0x2d7` | **KTF와 같은 오프셋**에, 아이템 사용 순간 **정확히 `0x01`** | 호스트가 1을 써도 **아무 일도 안 일어났다** |
| `0x2d5` | 사용 직후 `0 -> 1` | 곧 **`1 -> 2`** 로 올라감 = `0x2d4` 와 한 쌍인 **u16 틱 카운터** |

`0x2d7` 이 특히 위험했다 — 오프셋·값·타이밍이 **동시에** 맞았다. 우연 세 개였다.

**맞는 답** (S25 `<S25_SERIAL>`, vc52, `items::flag_window_changes` 가 `CHAR+0x0..0xa800`
전체 감시):

```
03:47:16.427  +0x9c84=1 +0x9c85=3 +0x9c87=0x69      아이템 팝업 열림 (tag 105)
03:47:33.490  CHAR+0x2d8 (0x1503d60)  0x00 -> 0x81  <== 1분 뒤에도 0x81 그대로
              +0x9c84 1->0  +0x9c85 3->0            팝업 닫힘 = 사용 확정
```

43 KB 전체에서 사용 시점의 **영속 `0 -> non-zero` 는 이것 하나**였고, 같은 런이
`+0x2d5=0 +0x2d7=0 +0x2dd=0` 을 찍으며 옛 후보 셋을 그 자리에서 퇴출시켰다.

★ **값이 `1` 이 아니라 `0x81` 이다.** KTF arm 은 리터럴 `1` 을 쓰고 LGT arm 은 `0x81` 을 쓴다.
그래서 앵커가 **(오프셋, 값) 쌍**이 됐다 — `guest_anchors::autoloot_flag()`. 오프셋만 들고
있었다면 `0x2d8` 에 `1` 을 써서 **또 조용히 실패**했을 것이다(비트필드일 가능성이 크다).
호스트는 **바이트가 정확히 0 일 때만** 쓴다: 이미 non-zero 면 게임이나 앞선 쓰기가 그 바이트를
소유하고 있으므로 통째로 덮지 않는다.

★ **캐시템은 다시 제외했다 — 이번엔 측정 근거로.** 아침에 되살린 이유가 "호스트가 한다고
가정하고 delist" 였는데, 이제 그 가정이 **실측**이 됐다(vc53 이 캐릭터 생성부터 쓰고, 사용자가
구매 없이 자동습득을 확인). 카탈로그 105 → **104행 / 7페이지**.
⚠ 남은 위험 하나: 호스트는 **바이트가 0일 때만** 쓴다(비트필드로 보여 통째로 덮지 않는다).
세이브가 그 자리에 0이 아닌 다른 값을 복원하면 호스트도 손을 안 대고 캐시템도 없으니 켤 방법이
없다. 증상은 "옛 세이브를 불러오면 오토루팅이 안 된다"이고, 되돌리기는 `CASH_SHOP_DELISTED`
에서 `AUTOLOOT_IDX` 를 빼는 한 줄이다.

### 방법론 — §2.11 의 남은 🔶 를 이걸로 닫아라

1. **창을 좁히지 마라.** 첫 시행은 KTF 앵커가 `0x2d7` 이라서 `0x280..0x300` 만 봤고, 그 창
   안의 유일한 후보가 답처럼 보였다. **예상한 답 주변의 창은 확인이나 오도만 하고 발견을 못
   한다.** 43 KB 를 초당 두 번 읽는 건 memcpy 다 — 좁힐 이유가 없다.
2. **카운터를 자동 묵음 처리하라.** 매 샘플 연속으로 변하는 바이트는 카운터다
   (`FLAGWATCH_COUNTER_RUN = 3`). 이걸 안 넣으면 `0x2ba`/`0x2d4` 가 신호를 덮는다 — 첫 시행이
   그래서 읽기 어려웠다.
3. **게임 자신의 아이템으로 필드를 세워라.** 그 바이트를 쓰는 코드를 찾는 것보다 그 바이트를
   쓰게 **만드는** 것이 훨씬 싸다.
4. **한 순간에 변했다는 것은 증거가 아니다.** 블록 쓰기가 이웃을 통째로 움직인다
   (`0x28b..0x29d` 의 `0xff` 연속이 그 예). **호스트가 그 값을 써서 효과가 재현되는지**까지
   확인해야 답이다.

### (5b) 정적 탐색이 도달하지 못한 경로 — 다시 하지 않게 남기는 것

* **게임 자신의 베이스는 `CHAR+0x28`** 이다. 접근자 썽크 `0x1068`(`ldr r0,[pc,#0]; bx lr`)
  이 **`0x1503ab0`** 을 돌려주고, 호출은 `ldr rN,[pc]=0x1069; bl <bx rN 베니어>` 로 **647
  군데**다. 즉 게임 오프셋 `k` = 이 크레이트의 `k + 0x28`. 이게 §2.11 오프셋 **12개를 독립
  재확인**한다: 레벨 `0xba`·EXP `0xbc`·maxHP `0xc4`·HP `0xca`·SP `0xd0`·스탯/스킬포인트
  `0x162`/`0x164`·팝업 군집 `0x9c44..0x9c5f`·그리드 핸들 `0x9e00`·window id `0xa098`.
  ★ 특히 **팝업 TAG 바이트의 쓰기 6곳이 정확히 100..105 를 저장한다** — §2.13이 실측한
  `0x64` 다.
* 647곳을 국소 추상해석으로 풀면 `game+0x200..0x300` 전체에서 `strb #1` 이 **딱 하나**:
  **`0x51152` → `game+0x2b5` = `0x1503d65` = 이 크레이트 기준 `CHAR+0x2dd`**. arm 모양은
  KTF와 판박이(게터 → 리터럴 오프셋 → `movs #1` → `strb` → 수량 차감 꼬리로 `b`).
* **그런데 그 arm 은 `0x508fc` 디스패처의 `[0x05]`** 다(경계 `cmp #0x10`, 테이블 `0x5e224`,
  꼬리 `0x51248`). 그리고 그 디스패처는 **useItem 이 아니다**: `[0x00]..[0x03]` 이 각기 다른
  arm 인데 cat `0x12` 의 포션/하이포션/미라클포션/엘릭서는 하나를 공유해야 하고, 레코드가
  KTF `useItem` 이 인덱싱하는 `0x3c4`/`0x3c` 인벤토리 그리드가 아니라 **id `+0x7c`·수량
  `+0x80` 의 `0x14` stride 배열**이다. 스킬/액션 사용으로 보인다.
* 따라서 **`CHAR+0x2dd` 는 후보였고, (5)의 실측이 그것을 반증했다** — 그 바이트는 시행 내내
  변하지 않았다. `anchors_selftest.rs` 가 표에 `0x2dd` 가 들어가 있지 **않음을** 단정한다.
* **LGT의 진짜 `useItem` 은 아직 미특정이다.** 오토루팅은 측정으로 닫혔지만, 다른 캐시템
  arm(창고열쇠·오브상자 등)을 건드릴 일이 생기면 그 디스패처가 필요하다. 찾는 표식 = 키가
  `그리드 + tab*0x3c4 + slot*0x3c + 0x3b8` 의 u16 인 디스패처(KTF `0x153ed0..0x153f02` 가 그
  모양).
* ★ **일반화할 방법론**: `items::flag_window_changes`(레코드 플래그 창의 변화만 보고, 매 샘플
  변하는 바이트는 카운터로 자동 묵음) + 게임 자신의 아이템으로 그 필드를 세우고 지켜보기.
  §2.11 에 남은 🔶 오프셋들도 같은 방법으로 닫는 게 디스어셈보다 빠르다.
* 도구: `tools_re/dump_item_db.py`(전 카테고리 디코더, `--colour white`).
  ★ 기존 `decode_items.py`/`rename_cash_items.py` 의 레코드 모델은 **틀렸다** — `u16` 은
  파일 헤더가 아니라 **레코드마다 붙는 길이**다. 그 오해가 출고된 `i18_dat` 에 낡은 길이
  접두를 남겼다(§검증 참조).

### 검증

`hero3_server/selftest.py` **192 PASS**(+10: 최종티어 48행·카이져스톰 `0x1d` 등재·약본
`0x17` 제외·변종 표 분리·이미 팔던 행과 비충돌·오토루팅 재등재·페이저 1..160 프로퍼티),
`tls_e2e_test.py` ALL PASS. standalone **199 PASS**(`anchors_selftest.rs` +1 = 20).
`cargo ndk check` 통과.

**서버 배포 완료** (2026-08-12 18:05 UTC, `<hero3-server>`): 백업 →`.py`/`.md` 10개만 업로드
→ **서버에서** selftest/tls_e2e ALL PASS → install → restart. 로컬↔서버 md5 10개 전부 일치,
재시작 후 error 0건, `hero3.db` 무변경. 페이저를 클라이언트와 같은 커서 규약(`A±1`)으로
실주행해 **7페이지 105/105 행 도달**(오토루팅 재제외 후 104/104), 카이져스톰 `(0x07,0x1d)`·오토루팅 `(0x12,0x0d)` 도달 확인.

**기기 실측** (S25 `<S25_SERIAL>`, vc49 → vc53):
* 오토루팅 오프셋을 **세 번 틀린 뒤 확정**: `CHAR+0x2d8 <- 0x81` ((5)). vc51 이
  `0x2d7 <- 1` 을 실제로 썼는데(`write ok = true`) 자동습득이 **안 됐다** — 그게 `0x2d7` 을
  죽인 증거다. vc53 배너 = `auto-loot forced ON (CHAR+0x2d8 <- 0x81)`.
* ★ **무한로딩 발견·수정** ((6)): 네트워크 진입 후 `CLR` 로 나올 때 "슬롯정보 서버에
  저장중입니다" 무한 대기.
* 부수 확인: `blit 1515/1710` → 창고에서 `345192/354498` (블리터 훅 89~97 % 적용 — 타이틀
  화면의 `0/0` 은 스프라이트를 아직 안 그려서였다).

실기기 남은 것 = 최종티어 48행의 **구입·지급**(7페이지 스크롤 포함), 그리고 §2.22-(4) 의
`i18_dat` 프레이밍이 화면에 실제로 드러나는지.

## 2.23 ★ 무한로딩 — 서버가 **같은 연결의 두 번째 `04 3d` 에 ack 를 안 보냈다** (2026-08-13)

사용자 보고: "네트워크 한번 접속한 뒤에 다시 CLR로 뒤로나가면 슬롯정보 서버에 저장중입니다
뜨면서 무한로딩. 그리고 네트워크 창고는 왜 이렇게 프레임드랍이 심한거야?"

**두 증상이 같은 원인이다.** 서버 로그가 그대로 말한다:

```
RX #5  [04 3d …] -> STORED                       -> TX #5 [04 3d 01 00 …]  이어서 05 3d  ✅
RX #9  [04 3d …] -> STORED (repeat on this conn) -> (없음)                 이어서 멈춤     ❌
그 뒤로 RX [00 0a] 하트비트만 5초마다 무한
```

2026-08-12 에 "브라우징 중에 저장 완료 박스가 계속 뜬다"는 불만을 처리하면서 **연결의 첫
업로드만 ack** 하도록 바꿨는데, 클라이언트는 그 ack 에 **매번** 블로킹한다. 그게 슬롯 저장
모달을 넘기는 프레임이라 안 주면 화면이 죽는다. 코드 주석이 이 실패 모드를 **미리 적어
뒀다** — "a `04 3d (repeat…)` with no `05 3d` behind it. Then this becomes ack always" —
그리고 그 조건이 그대로 성립했다.

★ 페이지 넘김과 `CLR` 이탈은 **와이어에서 구분이 안 된다**(둘 다 같은 `05 3d`/`05 14`/`05 3f`
뒤에 정확히 `04 3d`). 그래서 영리하게 가를 방법이 없고 **항상 ack** 가 답이다. 박스가 다시
보이는 것은 받아들일 트레이드다 — 스스로 사라지는 토스트가 죽은 화면보다 낫다. 다만 텍스트는
반대 방향으로 쓴다: **빈 텍스트가 OK 를 요구**하게 만드므로, 반복 ack 에는 짧은 **비어 있지 않은**
문구(`동기화`)를 넣어 저절로 닫히게 했다.

### 프레임드랍도 같은 뿌리였다

창고에서의 실측 (`emu` 로그):

```
guest 6.9 fps (타이틀 21~23)   svc 11,100,615 (초당 약 26,000)
in-tick 22% of wall, sleeping 77%      blit 345192/354498 (97 %)
```

호스트는 77 % 놀고 있는데 게스트가 6.9 fps다 — 에뮬레이션 속도 문제가 아니라 **오지 않는 ack 를
기다리며 `recv` 가 초당 2.6만 번 헛도는 것**이다. `CASH_SHOP.md` 가 이미 적어 둔 함정
(`ReadWaiter` 에 poll cap 이 없다 — connect 는 240 폴로 제한, recv 는 무제한)이 여기서 비용으로
드러났다. `refused` 두 번째 카운터가 계속 자라는 것도 같은 얘기다.

⚠ **poll cap 자체는 아직 안 넣었다.** ack 가 항상 나가면 대기가 안 생기므로 이번 증상은
사라지지만, 서버가 답을 못 주는 다른 경우에 같은 스핀이 다시 난다. `wie_wipi_c/src/api/net.rs`
의 recv 대기에 상한을 두는 것이 남은 작업이다.

### 검증

`hero3_server/selftest.py` **192 PASS** (+6: `04 3d` 3연속 전부 ack / ack 텍스트 비어 있지 않음).
서버 배포 완료, 로컬↔서버 md5 일치.

## 2.24 ★ 창고 프레임드랍의 진범은 **네트워크가 아니라 프레임 페이싱**이었다 (2026-08-13, 13차)

사용자: "여전히 네트워크 창고에 접속하는 순간부터 페이지 이동 등 렉이 너무 심해."

§2.23 은 이 증상을 "오지 않는 ack 를 기다리며 `recv` 가 초당 2.6만 번 헛돎"으로 적었다.
**그 서술은 틀렸다.** 이번에 SVC 를 종류별로 세는 계측을 넣고 기기에서 재보니 `recv` 폴은
**초당 0회**였고, 초당 2.6만 번 돌던 것은 `MC_grpFillRect`(WIPIC id **`0xd3`**) 였다.
§2.23 의 숫자(`svc 11,100,615`)는 맞았지만 **귀속이 틀렸다** — SVC 총계는 종류를 말해주지
않는데, LGT 는 C 표준 라이브러리까지 SVC 라서 총계만으로는 `memcpy` 와 소켓 syscall 을
구분할 수 없다. 그래서 §2.23 이 남긴 "recv poll cap" 숙제는 이 증상의 원인이 아니다
(cap 대신 2 ms 게이트를 넣어 두긴 했다 — 아래 (3)).

### 계측이 먼저다 — 이번에 추가한 세 줄

기기에서 30초마다 나오는 기존 `emu throughput` / `emu loop` 밑에 붙는다:
```
emu svc/s by category [init 0 wipic 12251 stdlib 8174 hook 300] hot ids/s: 0xd3=11023 0x414=8002 …
emu frame timing: guest set 7/s timers (min 244 ms, mean 244 ms, max 244 ms); pump napped 54/s for 14.5 ms avg
emu net: recv polls 0/s, absorbed 0% by the 2 ms gate, socket 0 ms/s
```
* `svc/s by category` = `wie_util::stats::add_svc_detail` (SVC 진입 한 곳에서 `r12` 의 id 를
  같이 센다 — 모든 디스패치 경로가 여기를 지난다). **id → 이름은 `wie_lgt/src/runtime/svc_ids.rs`.**
* `frame timing` = 게임 자신이 요청한 `MC_knlSetTimer` 값의 min/mean/max + 이벤트 펌프가
  실제로 잔 시간. **이 한 줄이 "느린 게 우리냐 게임이냐"를 즉시 가른다.**

### (1) ★★ 두 곳에서 프레임을 16 ms 격자에 가두고 있었다

게스트는 프레임 타이머를 **20 ms** 로 걸어 두는데, 그 타이머를 두 계층이 각각 16 ms 로
반올림했다:

1. `wie_midp/.../net/wie/event_queue.rs` 의 이벤트 펌프 — 큐가 비면 **무조건 `sleep(16)`**.
   20 ms 짜리 타이머는 16 ms 기상에 안 걸리니 **32 ms** 에야 실행된다.
2. `wie_android/src/emulator.rs` 의 워커 루프 — 틱이 짧게 끝나면 `FRAME_PACE(16 ms)` 를
   **채우도록** 잤다. 게스트가 3 ms 뒤에 깨워달라 해도 16 ms 뒤에 본다.

두 격자가 곱해져서 20 ms 타이머가 실측 **45 ms** 가 됐다. 즉 **게임 전체가 설계 속도의 44 %
로 돌고 있었다** — 창고만의 문제가 아니라 필드·전투·메뉴 전부.

수정: 펌프는 **대기 중 타이머의 가장 이른 `due` 까지만** 자고(상한은 그대로 16 ms, 하한
1 ms), 워커 루프는 `Emulator::next_wakeup_ms()`(→ `Executor::next_wakeup_in`)로 **게스트가
깨워달라고 한 시점을 넘겨 자지 않는다**. 둘 다 "예전보다 오래 자는 일은 없다"는 방향이라
느려질 수 있는 경로가 없다.

실측 (S25, 타이틀 화면): **22 fps → 69 fps**, 14.6 → 56.2 M inst/s, 호스트 루프 61 → 134/s,
in-tick 27 % → 52 %. 오프라인 패널 화면(FORCE) = **53 fps, fillRect 40,400/s**.

### (2) `MC_grpFillRect` 는 자기 픽셀만 쓴다 — 밴드 왕복이 없어졌다

`graphics::fill_rect` 는 `canvas_rows` 로 **밴드 전체**(`width x h`)를 게스트 메모리에서
읽고, 그리고, 다시 통째로 써 넣었다. `drawImage` 라면 맞는 모양이지만 **fill 은 아래 픽셀이
필요 없다**. 496x240x16bpp 에서 호출당 ~1 KB 읽기 + ~1 KB 쓰기 + 1 KB 할당·제로화를
**실제로 바꾸는 16바이트**를 위해 지불했고, 패널 화면은 프레임당 fillRect 가 **1,650~2,450개**다.

`FrameBuffer::fill_rect_direct` 가 클램프 후 자기 행만 써 넣는다. `ImageBufferCanvas::fill_rect`
와 `clamp_rows` 의 산술을 그대로 복제했고(`w as u32` 인데 `h.max(0)` 인 **비대칭까지** 그대로),
16bpp 는 `from_color(to_color(p)) == p` 가 **65536 값 전수 확인**이라 색 왕복이 항등함수다.
테스트 `direct_fill_matches_band_canvas` 가 14 케이스(1x1, 전체화면, 음수 원점, 양쪽 경계
초과, 0/음수 extent)로 **밴드 캔버스와 바이트 동일**을 못 박는다.

### (3) recv 는 진범이 아니었지만, 게이트는 넣어 뒀다

`AndroidNetwork::recv` 는 "지금은 없다"는 답을 **2 ms** 동안 재사용한다(`RECV_EMPTY_REUSE`).
이미 복호화된 평문은 절대 붙잡지 않는다(게이트는 `TlsStream::buffered()` 가 빈 것만 본다).
그리고 `socket_recv` 의 **호출마다 1 KB `Vec` 할당**을 스택 버퍼로 바꿨다. 이번 증상의
원인은 아니지만, 서버가 답을 못 주는 경우에 §2.23 이 걱정한 스핀이 다시 나면 여기서 흡수된다.

### (4) 남은 것 = **게임 자신의 244 ms** (에뮬레이터가 더 할 게 없다)

네트워크 세션의 **모달 박스 / SLOT 선택** 상태에서 게임은 프레임 타이머를 **244 ms**
(min=mean=max, 프레임당 1회)로 건다. 다른 화면은 전부 20 ms 다. 그래서 그 상태의 4~7 fps 는
**게임의 설계 속도**이고, 그 순간 호스트는 **78 % 놀고 있다**. 창고 목록 자체는 20 ms 라
빠르다(S9 실측 33.9 fps → 이번 수정 이후로는 더). 더 빠르게 하려면 게스트 상수를 패치해야
하는데, 그건 게임 동작을 바꾸는 일이라 **사용자 승인 사항**으로 남긴다. 후속 작업:
`movs r1, #244` 로 후보를 좁혀 `MC_knlSetTimer` 호출 직전 사이트를 확정할 것
(§3.3 의 정적 XREF 경로 — 단, 파일 오프셋과 `lgtdis.py` 의 vaddr 매핑이 어긋나 있어 그것부터
맞춰야 한다).

### 검증

`wie_wipi_c` 유닛 **16 PASS**(신규 `direct_fill_matches_band_canvas` 포함), vendor 워크스페이스
유닛 전부 PASS(사전 실패인 `wie_lgt --doc` 2건과 호스트에서 못 돌던 `wie_cli`/`wie_ktf_dump`
바이너리 제외), standalone **199 PASS**, `cargo ndk check` 통과, `cd vendor/wie; cargo fmt`.
기기 = S25 `<S25_SERIAL>` vc54→vc56, 공용 서버 `<hero3-server>` 로 로그인·창고 접속 정상,
패널 픽셀 육안 동일.

⚠ **S9(`<S9_SERIAL>`) 실측 대기**: 측정 도중 USB 에서 빠졌다. 저사양 기준기에서
(1)+(2)의 효과가 가장 크다 — 그 기기의 창고 모달 프레임은 호스트 작업 **112 ms**(in-tick
55 %)였고 그 대부분이 프레임당 2,450 번의 밴드 왕복이었다.

## 3. 작업대 — 다음 세션이 그대로 쓸 것

### 3.1 파일이 어디 있나

★ **10차 정정 — 이 표는 머신마다 틀렸던 이력이 있다.** 7차가 `D:/tmp/...` 를 지웠고,
10차 머신에는 **`<keystore>` 도 없다.** 그래서 이 표에서 항상 참인 항목은 **저장소 안에
있는 것들뿐**이고, 밖에 있는 것은 매 세션 `Test-Path` 로 확인하고 여기를 고칠 것.

| | 경로 |
|---|---|
| **RE 대상 `binary.mod` 를 꺼내는 정본 경로** | `android_app/app/src/main/assets/hero3_fixed.zip` → `0002865E.jar` → `binary.mod` (2단 zip). 저장소 안이므로 **항상 있다** |
| 그 `binary.mod` | 519,952 B, md5 **`6a352944…`** (= 아래 원본과 동일 바이트) |
| **실제 APK에 들어간 것** | 위 그 zip = LGT 01.00.07 game.zip + `patch_asset.py` (1,763,788 B) |
| ~~LGT game.zip 원본~~ | ~~`<keystore>`~~ — **10차 머신에 없음**. `binary.mod` 만 필요하면 위 경로로 충분하다 |
| 되돌릴 KTF 에셋 | `android_app/asset_backup/hero3_fixed.zip.ktf` (저장소 안, gitignore) |
| 릴리스 키 | **`<keystore>`** (10차 실측; `~/.gradle/gradle.properties` 의 `hero3.keystore` 가 정답이다 — 하드코딩된 경로를 믿지 말고 그 프로퍼티를 읽어라) |
| 서명·서버 설정 | `%USERPROFILE%\.gradle\gradle.properties` |
| LGT 01.00.04 (다른 리비전, 비교용) | **없음** — 필요하면 다시 구해야 한다 |

### 3.2 프로브를 인게임까지 모는 결정적 스크립트

```powershell
$k = @(); for ($f=260; $f -le 7800; $f += 120) { $k += "${f}:OK" }   # → ~7800프레임에 필드
$k += '8100:CLR','8250:OK'                                          # → 메뉴 → 상태보기
$MEM = '0x1400000:0x604,0x1500000:0x12958,0x40000000:0x800000'      # .data / .bss / 힙 8MB
.\target\release\wie_probe.exe --zip D:/tmp/lgt/upload/x/game_07.zip --w 240 --h 320 `
  --db D:/tmp/lgt/dbX --frames 8600 --ms 20 --keys ($k -join ',') --hle 1 `
  --out D:/tmp/lgt/X --dump 8200,8400 --dump-mem $MEM
python tools_re/probe_png.py D:/tmp/lgt/X --montage D:/tmp/lgt/X.png
```
* **키 배치** (5차에서 `*`/`#` 정정): `CLR` = 인게임 메뉴, **`*` = 맵 창**,
  `1·3·7·9` = 퀵슬롯, OK/방향키. `#`·LSK·RSK·`0`·`2`·`4`·`5`·`6`·`8` 은 필드에서
  **무반응**. 맵창은 다음 아무 키에서 닫힌다. 확인 방법 = `--trace-state 1` 이
  찍는 `window=` (`CHAR+0xa0c0`: 3 = 창 없음, 4 = 스크립트, 8 = 맵).
* 메뉴 = 상태보기 / INVENTORY / EQUIPMENT / SKILL / QUEST / SYSTEM (저장은 SYSTEM).
* `--db` 는 **매번 새 디렉터리**로. 세이브가 남으면 다음 런이 결정적이지 않다.
* ~~**방향키는 아이소메트릭이다**: 화면 UP = 타일 `(-1,-1)` … 한 타일 ≈ 27프레임~~
  ★ **61차 실측으로 정정**: 이 바이너리(6a352944)에서 방향키는 **4방향 축 이동**이다 —
  RIGHT = 타일 `(+1,0)`, DOWN = `(0,+1)`, ~4프레임/타일 (`wie_probe --walk-cal`,
  4방향 216프레임씩 걷기 시리즈). 타일 = 16×16 px, 플레이어 월드픽셀 = 오브젝트 +0x10
  (u16 x,y), 렌더 번역 = `clamp(cam2)` — 전부 `PLAY_FEEDBACK_FIXES.md §61차`.
  이 항목의 원 서술이 어떤 조건에서 관측됐는지는 재현하지 못했다.

### 3.3 주소를 찾는 두 가지 방법 — 둘 다 LGT에서 검증됨

**(a) 값 검색** — 화면이 숫자를 보여주는 것이면 이게 가장 빠르다. 상태창/인벤토리를
열고 프레임을 뜬 뒤, 그 숫자들이 **몰려 있는** 메모리 창을 점수화해 찾는다. 희소한
값(EXP-to-next 814 같은)을 앵커로 쓰고 흔한 값(6, 8, 12)은 확인용으로만. CHAR가
이렇게 나왔다(§2.7).

**(b) 단일-워드 격리** — 화면에 숫자가 없으면, **입력만 다른 두 결정적 런**을 떠서
`.data`/`.bss`/힙을 워드 단위로 diff 한다. 2차에서 클래스 상세 화면으로,
4차에서 카메라 좌표로 성립을 확인했다. 주의점 둘:
* 힙 diff는 대부분 **프레임버퍼/타일 픽셀**이다(값이 `0x93889388` 처럼 16비트
  반복이면 픽셀이다). 게임 상태는 `.bss` 에 있을 확률이 높다 — LGT는 절대 주소
  모델이라 KTF가 힙에 두는 것도 정적으로 갖고 있다.
* 두 런이 **같은 화면에서 끝나면** diff가 비는 게 정상이다. 걷기 실험에서 3걸음과
  13걸음이 같은 프레임 해시를 낸 적이 있다(벽에 막힘) — 프레임 md5를 먼저 비교해서
  두 런이 실제로 갈렸는지 확인하고 diff 하라.

### 3.4 도구의 구멍

* ~~`wie_probe` 가 KTF CHAR를 하드코딩한다~~ → **5차에서 메웠다**(§2.8). 프로브가
  `wie_android/src/guest_anchors.rs` 를 `include!` 하므로 앱과 같은 앵커를 쓴다.
  `--map-check`/`--char-dump`/`--find-fb`/`--trace-state` 전부 빌드 인식.
* `items_selftest.rs` 는 `include!` 가 아니라 **items.rs 로직의 사본**이다. items.rs
  를 고쳐도 자동으로 따라오지 않는다. ~~5차에 추가한 `deref_container()`(빌드별 홉 수)
  도 그 사본에는 없다~~ → **6차에서 미러 + 양 규약 테스트를 넣었다**(§2.10). 다만
  여전히 사본이므로 LGT 경로의 최종 심판은 프로브 `--map-check` 다.
  (`endpoint`/`tls_buf`/`minimap_render`/`buffs`/`save`/`quickpage`/`backup`/
  `guest_anchors` 만 진짜 `include!`.)
* ~~`cargo test -p wie_android` 는 어디서도 못 돈다~~ → **Windows 호스트 한정**이다.
  리눅스에서는 `libasound2-dev` 만 깔면 59 tests 가 그냥 돈다(§2.10). 원격/CI
  세션에서는 standalone 셀프테스트에 더해 이걸 돌릴 것.

### 3.5 디스어셈 / 코드젠

* `python tools_re/lgtdis.py <binary.mod> <시작va> <끝va>` (capstone). 프로파일러와
  toml의 pc = ELF vaddr 그대로.
* 섹션: `.text` va 0x1000 (fo 0x34, 01.00.07은 크기 0x5db85), `.data` 0x1400000
  (0x604), `.bss` 0x1500000 (0x12958). **절대 주소** — RWPI `sl` 없음.
* 블리터 하나가 완전히 리버싱돼 있다(01.00.07 `0x2dd7c`..`0x2e8c2`, 에필로그
  `add sp,#0x54; pop {r4-r7}; pop {r0}; bx r0`). LGT 코드젠의 관용구(스택 스필
  위주, `ip` 활용, 조기반환을 `bl 에필로그`로)를 익히는 표본으로 쓸 것.
* stdlib류 핫스팟 `0x5a4fa~0x5a51e`(memcpy류 추정)는 제네릭 패턴 훅 후보.
* **`0x5a6e8` 부터는 `bx rN` 베니어 표**다(`bx r0`,`bx r1`,… 2바이트 간격). `bl` 이
  이쪽을 가리키면 함수가 아니라 레지스터 간접호출이다 — 프로파일 해석 시 주의.
* **정적 주소 XREF: `python tools_re/lgt_statics_xref.py <binary.mod> …`**
  (`--range lo hi [--loaders]` / `--char-off off` / `--imm8 N`). 5차에서 HUD 사이트를
  `--imm8 0xaa` 로 찾고, blink 카운터가 HUD 전용임을 `--range` 로 증명했다. 데이터
  주소는 확보돼 있으니(§2.7) 이게 "이 필드를 만지는 코드가 어디냐"의 표준 경로다.
* **`wie_probe --verify-offsets 1`** (6차 추가) — §2.11 매트릭스의 🔶 다섯 오프셋 +
  검증 기준 필드를 dump 프레임마다 라벨링·디코드 출력. 빌드 인식은 공유
  `guest_anchors` 를 쓰므로 KTF/LGT 자동. 매핑 밖 읽기는 전부 `None`(비패닉).
  조용한 버그 감시 대상을 명령 하나로 검증하는 표준 경로다(§2.11에 절차).
* 디버그 문자열(fo 0x5b6xx~0x5cbxx)의 XREF를 따라가는 방법은 아직 안 써 봤다.
  `Hero_Free` / `pMain->pNpcQueue[objID].moveType` / `ResMgr_FreeGame` /
  `res/dat/{char,enemy,drop,shop,smith,getitem}_dat` — 코드 주소가 필요한 §4의
  뒤쪽 항목들(저장 체인, HUD 그리기)에서 값 검색이 막히면 이 경로로 갈 것.

### 3.6 게이트 규율 — 무엇을 언제 여는가

`wie_android/src/guest_build.rs` 가 둘로 나뉘어 있다(§2.7):

* `char_anchor()` / `state_off()` / `container_data_off()` / `minimap_noblend_global()`
  — **데이터**. LGT 값이 들어가 있고 읽기는 열려 있다.
* `hud_bottom_patch()` — **항목별로 푼 첫 게스트 코드 패치**(5차). `(site, orig,
  branch)` 를 빌드별로 돌려주고, LGT 값이 실측으로 들어가 있다.
* `guest_code_addresses_valid()` — **아직 안 찾은 코드 주소**. LGT는 여전히 `false`.

§4의 항목 하나를 끝낼 때마다 그 항목의 주소만 빌드별로 갈라 넣고, 그 항목을 쓰는
함수의 게이트만 푼다. **`guest_code_addresses_valid()` 를 통째로 true 로 만들지 말 것**
— 그 순간 아직 안 찾은 주소들까지 같이 열린다. 항목별로 갈라야 하면
`char_anchor()`/`hud_bottom_patch()` 처럼 "빌드별 값을 돌려주는 함수"를 하나 더
만드는 게 맞다(전자는 데이터, 후자가 코드 패치의 본보기다). 그 표는 전부
`guest_anchors.rs` 에 있고 `anchors_selftest.rs` 가 지킨다.

## 4. 남은 이식 대상 — 순차 진행 계획

CHAR(§2.7)가 열린 뒤라 순서에 **의존관계**가 생겼다. 아래 번호대로 가면 앞 항목의
결과가 뒤 항목의 도구가 된다. 각 항목은 "필요한 것 → 찾는 법 → 검증 → 앱 반영"
네 칸으로 적어 뒀다. KTF 주소는 **지도이지 정답이 아니다**(§1).

> ★ **7차(§2.12)가 이 순서를 다시 짰다.** 1·2·5번은 코드가 끝났고, 이 표에 없던 항목
> (캐시버프 트리거 = §2.11 #17)이 실은 **#9·#10·#16 을 동시에 막고 있던 단일 병목**
> 이었다 — 그것도 §2.12에서 확보했다. 남은 순서:
>
> 1. **기기 플레이 검증** (코드 작업 0). `items::offset_audit` 이 ~5초마다 찍으므로,
>    사람이 ① 캐시템 사용 ② 적 처치(EXP 2배) ③ 레벨업(배지) ④ 보조무기 장착
>    ⑤ 퀵슬롯 등록 ⑥ 보스방 진입(no-save) 만 하면 🔶 다섯 + #9 + #10 + #17 이
>    **전부 한 세션에서 닫힌다.** 프로브로는 도달할 수 없는 상태들이라 이게 최단경로다.
> 2. **3번 저장 체인** — 이제 가장 값나가는 미이식 항목이다. LGT에서 SAVE 버튼이
>    **눌리는데 아무 일도 안 하는** 것이 실사용으로 확인됐다(§2.12). 그 전에
>    "LGT에서는 SAVE 버튼을 숨긴다"를 넣는 편이 정직하다.
> 3. **6번 스크래치** (3번의 부산물) → **4번 quickbind**. 4번은 데이터 오프셋이
>    §2.12에서 검증됐고(`0x9c6e`/`0x9c84`/`0x9c85`/`0x9c86`/`0x9c71`/`0x9c72`),
>    **팝업 태그가 KTF `0x66` → LGT `0x64`** 라는 것도 나왔다 — `popup_view` 게이트 4가
>    이것 때문에 LGT에서 항상 `None` 이므로 태그 상수부터 빌드별로 갈라야 한다.
> 4. **#15 오토루팅**은 `CHAR+0x2d7` 을 **검증 없이 쓰면 안 된다** — 게스트 쓰기라
>    오프셋이 밀렸으면 엉뚱한 바이트를 1로 만든다. 그 바이트를 세우는 것은 캐시템
>    `오토루팅(싱글)`(cat `0x12` idx `0x0d`) 뿐이고 시작 인벤토리에 없으므로, 검증에는
>    상점까지 가는 런이나 `useItem` 점프테이블 RE 가 필요하다. **보류가 맞다.**

### 1. 미니맵 — ✅ 완료 (5차, §2.8). 기기 실측만 대기

체인 = `MAP = *(CHAR+0x9c90)` / `attr = *(MAP+0x60)` / `event = *(MAP+0x68)`
(KTF와 오프셋 동일, 홉 하나씩 짧음). `--map-check` 로 두 맵에서 **0/3500, 0/1368**,
KTF 회귀 **0/5040**. 게스트 코드 주소는 실제로 하나도 필요하지 않았다.

### 2. HUD 스킵 — ✅ 하단 블록 완료 (5차, §2.8). 배지는 미검증

패치 = `0x2864` 의 `0x9d25` → `b 0x304a` (`0xe3f1`), 렌더러 `0x22a4`..`0x3050`.
`--poke` A/B 로 패널 소멸 + CHAR 45 KB 바이트 동일. `guest_build::hud_bottom_patch()`
로 항목별 분기.

남은 것:
* **배지** `+0x18a`(스탯) / `+0x18c`(스킬 포인트) — LGT 미검증. 레벨 1에서는 둘 다 0
  이라 확인할 방법이 없다. 레벨업 한 번 시키는 프로브 런(적 처치)이 필요하고, 그건
  §4 5번(EXP)의 런과 같이 하면 공짜다.
* **밴드/타겟박스/hud_y 미세 조정** — §4.5대로 LGT에서는 굳이 안 한다. 게이트가 꺼서
  게임 기본(가운데 정렬) 배치로 동작하고, 그게 LGT에서 정상으로 보인다.

### 3. 저장 4단 체인 — 세이브가 걸린 항목이라 가장 조심스럽다

> ★ **§2.13(7차 후속)이 절반을 끝냈다. 이 칸을 읽기 전에 §2.13 을 읽을 것.**
> 확보: `writeSaveRecord = 0x1a264`(KTF `FN_WRITE_RECORD` 대응), `writeDbRecord =
> 0x1a1d4`(0x10 헤더 부착), `readDbRecord = 0x1a0e8` / `readSaveRecord = 0x1a198`,
> 이름 문자열 `"GameSave_11"` @ `0x5bbb4` · `"SlotSave_11"` @ `0x5bbc0`, 슬롯 번호
> `*(CHAR+0xa410)`, 레코드 1408 B(암호화, 8바이트 블록 ECB).
> **남은 것 = `serialiseSave(u32* outLen)` · `slotSummary()` 두 함수 + 스크래치.**
> 다음 수: 두 이름 문자열을 **함께** 로드하는 다섯 사이트(§2.13)를 디스어셈.

* **필요**: `serializeSave(u32* outLen)` / 암호화 / 기록 / `loadSaveRecord` 네 함수의
  LGT 주소 + out-param 스크래치로 쓸 **도달 불가능한 게스트 메모리**.
* **이미 확보**: DB 이름이 `GameSave_<slot>` / `SlotSave_<slot>` / `OptionSave`
  (§2.5), 그리고 인게임 저장 경로 = 메뉴(`CLR`) → **SYSTEM** (§2.13의 키 스크립트가
  프로브로 실제 저장을 만든다).
* **찾는 법**: 프로브로 실제로 저장시키면 `--db` 디렉터리에 `PD119037/db/GameSave_0`
  이 떨어진다. 그 **레코드 바이트를 `.bss`/힙에서 역검색**하면 직렬화 버퍼와
  그 소스가 잡힌다(§3.3(a)의 값 검색을 바이트열로 확장한 것). 저장 직전/직후
  프로파일 차집합으로 함수 진입점을 좁힌다.
* **검증**: 호스트가 부른 결과 레코드가 게임이 저장한 것과 **바이트 동일**.
  KTF 때 세운 기준을 그대로 쓴다.
* **주의**: 스크래치 주소는 KTF에서 "죽은 코드 영역"을 썼다(도달 불가 증명 포함).
  LGT에서 같은 증명을 다시 하기 전에는 **아무 데나 쓰면 안 된다.** 검증 없이는
  `.bss` 끝의 미사용 영역이라도 쓰지 말 것.
* **앱**: `save_gate` / `backup_target` 게이트 해제 + `save.rs` 의 DB 이름 분기.

### 4. quickbind (skill_bind / popup_key)

* **필요**: `skill_bind` 와 `popup_key` 두 함수의 LGT 주소. 퀵바 **데이터** 오프셋
  (`+0x252` 계열)은 CHAR 확보로 이미 읽을 수 있다 — 다만 아직 **미검증**이다
  (지금은 아무것도 안 묶여 있어 전부 0이라 확인이 안 됐다).
* **찾는 법**: 먼저 데이터부터. 인게임에서 스킬을 1번에 묶은 런과 안 묶은 런을
  §3.3(b)로 diff → `+0x252` 대가 맞는지 확정. 그 다음 그 바이트를 쓰는 코드를
  프로파일/트레이스로 찾으면 `skill_bind` 다.
* **앱**: `slot_writes` / `popup_sel_addr` / `popup_relabel` / `apply_quickpage_actions`.

### 5. EXP 2·4배 — ✅ 사이트 확보·이식 완료 (5차) / 장비레벨제한 — 사이트만

EXP: `addExpFromKill` = `0x4d388`, 나눗셈 상수 = **`0x4d3cc`** (§2.8). 상수 다발
스캔으로 바이너리 전체 1곳, 산술 교차검증까지 됨. `guest_build::exp_divisor_site()`.

장비레벨제한: 체크는 `0x477cc`..`0x477dc` 로 확보했지만 **LGT 코드젠에 패치할 move가
없다**. 4바이트 등가 패치(`ldrb r3,[r2,#2]` + `subs r3,#N`)를 §2.8에 적어 뒀다.
남은 일 두 가지:
* ~~배관을 4바이트로 넓히기~~ → **6차 완료**(§2.10): `EquipReliefSite::LoadPair` +
  `buffs::equip_relief_pair` + `items::equip_relief_patch` 의 `Vec<u8>` 화. LGT arm
  은 규율대로 `None` — 켜는 것은 `guest_anchors.rs` 한 arm + `anchors_selftest.rs`
  한 줄.
* **검증 런** (로컬 세션): 캐릭터 + 레벨 제한 장비가 필요하다. 상점까지 가는 결정적
  키 스크립트를 만들면 EXP 검증(적 처치)과 2번의 배지 검증(레벨업)도 같은 런에서
  끝난다 — 이 세 개는 묶어서 하는 것이 맞다. 릴리프의 A/B 는
  `--poke frame:0x477d2:93783b05` (이전 바이트가 `02 23 d3 56` 으로 읽혀야 한다).
  ★ 이 런에서 상태창·인벤토리를 함께 뜨면 **§2.11 매트릭스의 🔶 다섯 개
  (퀵바·배지·보조무기·no-save·쿨다운) 오프셋 검증도 전부 같이 끝난다** — §3.3(a)
  값 검색으로 화면 숫자를 CHAR 오프셋과 대조. 조용한 버그 감시 대상을 한 런으로 닫는
  것이 목표.

### 6. 스크래치(죽은 코드 영역)

3번의 부산물. 저장 체인을 리버싱하고 나면 KTF 때와 같은 기법(도달 불가 증명)을
LGT 코드에 재적용한다. 3번 없이 단독으로 할 이유가 없다.

### 이미 캐리어 무관인 것 (할 일 없음)

* **진동(크리티컬)**: `sd004_mf` 재생 경로는 호스트 감지 — KTF 주소 `0x147a74` 는
  안 쓴다.
* **가로형**: §4.5.
* **blit565 성능 패치**: §2.6, 두 리비전 모두 검증 완료.

### 공통 주의

CHAR 구조체 오프셋은 **대부분** 이식되지만 전부는 아니다 — 상태 바이트가
`+0x7c` → `+0x78` 로 움직인 실례가 있다(§2.7). 새 오프셋을 쓸 때마다 라이브로
확인하고, 확인한 것만 코드에 넣을 것.

## 4.5 가로형(landscape)은 이식 대상이 아니다 — 실측 완료 (2026-08-10)

`LANDSCAPE_FEASIBILITY.md`의 KTF 결론("게임은 해상도 적응적, 게스트 패치 불필요")이
LGT에서도 성립한다. 프로브 `--w 496 --h 240`으로 오프닝 전투까지 실측:
월드가 **폭 전체로** 그려지고, 대사 바 stretch, SKIP 우하단 고정, 타이틀 아트
가운데 정렬 — KTF 측정표와 동일. 가로형 = 호스트가 프레임버퍼를 창 크기로
잡는 것(`lib.rs::init_logical`, 캐리어 무관) + 게임 소스의 적응 렌더링이므로
재리버싱이 필요 없다.

⚠ **10차 정정 (§2.20)**: "미세 조정은 LGT에서 굳이 안 한다 — 게이트가 꺼서 게임 기본
배치로 동작하고 그게 정상으로 보인다"는 이 칸의 원래 결론은 **실기기가 반증했다**. 게임
기본 배치는 우리 오버레이와 같은 자리를 쓴다(버프 밴드 ↔ 좌상단 HP바, 타겟 몬스터 박스 ↔
우상단 미니맵). `band_x`(3사이트)·`target_box`(10사이트)는 §2.20 에서 이식했고, 남은 것은
`hud_y` 원점(`.bss 0x1503a50`, 지금 shift 0이라 불필요)과 밴드 높이(`CHAR+0xa398`, 라이브
확인 전)뿐이다.

## 5. 앱에서 LGT로 뛰우려면

APK 에셋의 game zip을 LGT판(`app_info` 포함 zip)으로 바꾸면 나머지는 자동:
아카이브 감지 → LgtEmulator 부팅 → blit565_lgt 설치 → KTF 전용 기능은 게이트가
끄고 부팅 로그에 `LGT build: …` 경고 1줄. **실제로 해봤고 그대로 됐다 — §2.6.**

~~화면은 240x320 (랜드스케이프 트랙은 KTF 전용 좌표계라 LGT에선 세로가 안전).~~
정정(2026-08-10 기기 실측): 가로형도 그냥 된다. 저장된 `landscape_pref=true` 로
게스트 프레임버퍼 **496x223** 이 잡힌 채 부팅했고 화면 배치는 정상이었다. §4.5 가
말한 대로 해상도 적응 렌더링은 LGT에서도 성립하고, KTF 좌표계에 묶인 것은 HUD
스킵/배지 미세 조정뿐인데 그건 게이트가 이미 꺼 놓는다.
