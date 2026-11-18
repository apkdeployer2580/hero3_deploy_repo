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
