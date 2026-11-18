# Hero3 스킬 버튼 (H5식 탭-발동 스킬 슬롯) feasibility — 분석 결과 (2026-07-24)

> 사용자 요청: H5 레퍼런스(`docs/h5/reference_screenshots/스크린샷 2026-06-18 014831.png`)처럼 동그란 스킬/포션 버튼을 탭하면 스킬/포션이 발동되게. 이게 가능한가?
> 분석 = workflow `wf_9fcbeeaf` (skill-dispatch / skill-cast-fn 2 finder + 독립 adjudicator, 전원 HIGH). guest addr = file + 0x100000.

## 결론
- **포션(아이템) 버튼 = ✅ 이미 됨** (`useItemEffect 0x153e08` 단발 호출). H5식 동그란 아이콘으로 restyle만 하면 끝.
- **스킬 버튼 = ⚠ FRAGILE_COMBO_BOUND** — 아이템처럼 "함수 한 방"으로는 **불가**. 가능은 하지만 **콤보 상태 주입 + 실기기 검증**이 필요한 실험적 작업.

## 왜 스킬은 아이템과 다른가 (CONFIRMED)
- 필드 퀵슬롯 디스패처 `0x143048`: 슬롯 type 바이트로 분기 — type==2=아이템→`0x1430c6`→**`bl 0x153e08` useItemEffect(동기 적용·count 감소까지 in-call)**, type==1=스킬→`0x1430dc`.
- 스킬 브랜치 `0x1430dc`의 **모든 bl 타겟을 전수 확인**: `0x14ad10`(CHAR getter)·`0x199cbc/cc0/cc4`(**전부 `bx lr` no-op 스텁**)·`0x147308`/`0x1470d0`(bool 게이트)·`0x140024`(액션상태 게이트)·`0x1487ec`(UI "사용불가" 피드백). **효과를 적용하는 호출이 하나도 없음.**
- 스킬 브랜치가 하는 일 = 게이트 검사 후 **실시간 콤보/애니 상태머신을 "무장(arm)"**: pending 플래그 `CHAR+0xb4+0x224`=1, 액션 카테고리 `CHAR+0x2c9`(무기-anim 스위치 0xa/b/c/d), current-skill, `*r1=1`("action started") 기록하고 `r0=1` 반환. **데미지/투사체/버프는 이후 프레임**이 `CHAR+0x224/+0x2c9/+0x261`을 읽어 실행.
- ∴ 아이템 = "호출하면 끝". 스킬 = "라이브 상태를 seed하고 돌아가는 루프가 완성해주길 기대".

## 스킬 열거는 가능 (RAM, 아이템 HUD와 동일 방식) — byte 확인됨
- CHAR = `*(sl+0x444)` (라이브 0x1b7b6c). 슬롯 idx `s`(현재 선택 = signed byte `CHAR+0x261`):
  - `type[s]` = signed byte **`CHAR+0x252+s`** (1=스킬, 2=아이템)
  - `skill-ref[s]` = byte **`CHAR+0x25c+s`**: `&0x1f`=스킬 id(1..31), `>>6`=2-bit 카테고리
  - `item-ref[s]` = byte `CHAR+0x257+s` (`>>5`=tab, `&0x1f`=slot)
- HUD는 슬롯 범위를 스캔해 type==1 항목을 스킬 버튼으로. **한계: 이건 "퀵슬롯에 배정된 스킬"만** — "배운 스킬 전체" 카탈로그는 미발견 → 버튼은 배정된 스킬만 노출 가능.

## 유일한 현실적 경로 (route a — 콤보 주입)
탭 시: 퀵슬롯 selector(`CHAR+0x261`)를 검증된 type==1 슬롯으로 세팅 → 필드 키 체인이 쓰는 것과 **같은 디스패처 경로**(`0x142758`/`0x143048`)에 적절한 command code 투입 → **돌아가는 per-frame 필드 루프가 무장된 상태를 소비해 캐스트 완성.** = 동기 `castSkill()` 호출이 아니라 **합성 콤보/커맨드 주입.**

**필수 게이트(탭 전 pre-check, 아니면 no-op)**: `CHAR+0x9e60` 모드 바이트 nonzero AND `0x140024` idle(`CHAR+0x9bb5` top-bit 0x80/0x90 모드 clear) AND selector가 id!=0 인 실제 type==1 슬롯. (범위/코스트 게이트 `0x1470d0`→`0x19c280` = 부분 해독·거절 시 UI 피드백만·crash 아님).

## 잔여 리스크 (crash-safe 미증명)
1. **desync**: 상태머신이 애니 중일 때 임의 탭으로 액션상태 주입 → 애니 겹침/멈춤·hit 프레임 오타이밍·`CHAR+0x2c9` 손상 가능. 위 게이트로 방어 필수.
2. 잘못된 selector/미장착(id 하위5비트=0) → 거절 또는 garbage read.
3. 하류 소비자(`CHAR+0x224/+0x2c9/+0x261` 읽어 hit 적용하는 프레임 핸들러)를 **아직 안 봄** → 외부 무장 캐스트가 진짜 콤보와 동일하게 해소된다는 보장 없음.

## 권장 다음 스텝 (다음 세션)
- **스킬 버튼을 useItemEffect처럼 단일 함수 호출로 출시 금지**(동기 applier 없음, 확정).
- 프로토타입 route a: 탭→selector 세팅+디스패처 경로 구동(2 게이트 pre-check) → **실기기서 여러 스킬/상태에 걸쳐 캐스트 완성(애니 재생·데미지·멈춤 없음) 검증** 후에만 출시. 실험/opt-in 취급(아이템 버튼 검증 방식 그대로).
- 병행 de-risk: 하류 `CHAR+0x224/+0x2c9/+0x261` 소비자 RE로 외부-무장 캐스트가 깨끗이 해소되는지 확인/반증.

원자료 = `wf_9fcbeeaf` journal.
