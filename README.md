# hero3NativeAPP

피처폰(WIPI, LGT) RPG **영웅서기3 — 대지의 성흔**을 안드로이드에서 돌리는 네이티브 앱.

에뮬레이터 코어는 [dlunch/wie](https://github.com/dlunch/wie) 포크(`vendor/wie`, `vendor/arm32_cpu`)이고,
그 위에 `wie_android/` 가 터치 HUD · 레이아웃 편집기 · 미니맵 · 세이브 금고 · 앱 내장 창고/캐시샵을 얹는다.
앱은 100 % 오프라인이다 — 소켓을 열지 않는다.

## 빌드

게임 에셋(`android_app/app/src/main/assets/hero3_fixed.zip`)은 저장소에 들어 있다. 받은 그대로:

```powershell
.\build_android.ps1            # 디버그 APK → android_app\app\build\outputs\apk\debug\app-debug.apk
.\build_android.ps1 -Release   # 서명본. 키스토어는 저장소 밖에 둔다 (스크립트 머리말의 HERO3_KEYSTORE*)
```

필요한 것: Windows · Android SDK + NDK 25.1.8937393 · JDK 17+ · Rust(`aarch64-linux-android` 타깃, `cargo-ndk`).

## 구성

| 경로 | 내용 |
|---|---|
| `wie_android/` | 안드로이드 호스트 (Rust) — HUD, 입력, 오디오, 세이브, 게스트 훅의 호스트 절반 |
| `android_app/` | Gradle 프로젝트 (GameActivity) |
| `vendor/` | 에뮬레이터 코어 포크와 패치된 의존성 |
| `wie_probe/` | 헤드리스 실행기 — 기기 없이 게스트를 돌려 실측한다 |
| `tools_re/` | 게스트 리버싱 · 에셋 도구 |
| `costumes/` | 아바타 의상 시트 원본(PNG) |
| `*_selftest.rs` | 의존성 없는 셀프테스트 (`rustc --edition 2021 --test <파일> -o t.exe`) |

## 문서

* 플레이버(main / vanilla): [`BRANCHES.md`](BRANCHES.md) · [`VANILLA_GAPS.md`](VANILLA_GAPS.md)
* LGT 판 리버싱: [`MIGRATION_LGT.md`](MIGRATION_LGT.md)
* 기능별 조사·설계 문서는 저장소 루트의 `*.md`

## 라이선스

이 프로젝트의 코드에는 라이선스를 부여하지 않는다 — [`LICENSE`](LICENSE). 게임과 그 파생물은 원 권리자의 것이고,
`vendor/` 와 제3자 에셋은 각자의 라이선스를 따른다(같은 파일에 목록이 있다).

<!-- release-section -->
## 최신 릴리스 (APK)

**[최신 APK 받기 → Releases](https://github.com/apkdeployer2580/hero3_deploy_repo/releases/latest)**

| 판 | 파일 | versionCode |
|---|---|---|
| 일반판 | `hero3_lgt1.04_0.459.0.apk` | 459 |
| vanilla 판 | `hero3_lgt1.04_vanilla_0.460.0.apk` | 460 |

일반판은 리버·보스 장신구 드랍, 하늘성, 레벨 상한 99 가 들어간 개정판이고, vanilla 판은 그 셋이 없는 편의성 개선 원본(레벨 상한 70)입니다. 두 판은 같은 앱이라 한 폰에 하나만 설치됩니다. 쓰던 판을 받으세요.

**이미 설치해서 하던 분 — 세이브를 그대로 이어 하려면**

* 앱을 **지우지 말고** 받은 APK 를 그대로 설치하세요. 업데이트로 덮어 설치되고 세이브가 그대로 남습니다.
* ⚠ 앱을 지우면 세이브도 함께 지워지고 되살릴 수 없습니다.
* 「앱이 설치되지 않았습니다」가 뜨면 서명이 다른 APK(예: 이 소스로 직접 빌드한 APK)가 깔려 있는 것입니다. 그 위에는 공식 APK 를 덮어 설치할 수 없고, 지우면 세이브가 사라집니다.
* 공식 APK 의 서명 인증서 SHA-256: `6dd5550c75f0611f8e5653c4704e2e24c2d4e9e9e8911f5207a6fb5847f589e5`
