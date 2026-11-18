# WIE + 모바일 터치 컨트롤 — 빌드 & 배포 안내

> 이 문서는 **웹(wasm) 빌드**용이다. **안드로이드 APK** 의 서명·버전·배포는
> [`RELEASE_APK.md`](RELEASE_APK.md) 를 볼 것.

dlunch/wie_app 프론트엔드에 **모바일 터치 컨트롤(조이스틱/피처폰 키패드, 좌우 반전 포함)**을 얹은 포크.
Hero3/4 같은 WIPI 원작을 폰에서 손가락으로 조작할 수 있게 함. 에뮬레이터 코어는 그대로(dlunch/wie).

## 컨트롤 레이아웃 (게임 화면 우상단 ⚙️ → "🎮 조작"에서 선택, 선택은 브라우저에 저장됨)

| 값 | 설명 |
|---|---|
| `조이스틱 + 버튼` | 좌=가상 조이스틱, 우=공격(OK)/스킬(1·2·3)/메뉴(#)·*·CLR |
| `조이스틱 + 버튼 (왼손)` | 위와 동일, 좌우 반전 |
| `피처폰 키패드` | 좌=방향키+OK(옛 피처폰), 우=숫자 키패드(1-9·*·0·#·CLR) |
| `피처폰 키패드 (왼손)` | 위와 동일, 좌우 반전 |

> 키 매핑: 터치는 게임이 이해하는 피처폰 키코드(UP/DOWN/LEFT/RIGHT/OK/0-9/#/*/CLR)로 그대로 전달됨.
> "공격=OK, 스킬=1/2/3, 메뉴=#"는 기본 추정값 — 게임마다 실제 키가 다를 수 있으니, 돌려보고 맞는 키로 바꾸면 됨
> (index.html의 `.actions` 버튼 `data-key` 값만 고치면 됨).

---

## 배포 (무료) — ★ 서버 빌드/특수 헤더 불필요

이 앱은 **단일 스레드 + 2D 캔버스**라 SharedArrayBuffer/COOP·COEP 헤더가 **필요 없음**.
배포물 = **미리 빌드된 정적 폴더 `wie_web/dist/`** 하나. 서버에서 빌드할 필요 없이 그 폴더만 올리면 됨.
(호스트에서 소스 빌드하면 Rust/wasm-pack이 없어 실패하니, 반드시 **이미 빌드된 `dist/`를 올릴 것**.)

### 방법 1 — Netlify Drop (제일 쉬움, CLI 불필요)
1. 브라우저로 https://app.netlify.com/drop 접속
2. **`d:\tmp\wie_app\wie_web\dist` 폴더를 드래그&드롭**
3. 바로 URL 생성됨. (계정 로그인하면 URL 유지·관리 가능)

### 방법 2 — Vercel (CLI, 최초 1회 로그인)
```powershell
cd d:\tmp\wie_app\wie_web\dist
npx vercel deploy --prod        # 최초 실행 시 브라우저 로그인(계정 연결) 1회
```
정적 폴더로 자동 인식되어 배포됨. (레포 루트의 `vercel.json`은 git 연동 시 미리 빌드된 dist를 서빙하는 설정.)

### 방법 3 — Cloudflare Pages (CLI, 최초 1회 로그인)
```powershell
cd d:\tmp\wie_app
npx wrangler pages deploy wie_web/dist --project-name=wie-touch
# 최초 실행 시 wrangler login(브라우저 로그인) 1회
```

### 방법 4 — GitHub Pages
`wie_web/dist/` 내용을 새 저장소의 `gh-pages` 브랜치에 push → Settings → Pages 활성화.
(헤더 제약이 없는 앱이라 GitHub Pages도 그대로 동작.)

> 어느 방법이든 **"계정 로그인/연결" 1회**만 사용자가 하면 됨. 빌드·산출물은 이미 준비됨.

---

## ⚠ 저작권

배포에는 **에뮬레이터 + 터치 UI만** 올릴 것. **게임 파일(hero3_fixed.zip 등)은 절대 함께 올리지 말 것**
(한빛/EA 저작물 공개 배포가 됨). wie.dlunch.net과 동일하게 **사용자가 자기 게임 zip을 그 자리에서 업로드**하는 방식이므로,
배포물 자체엔 게임이 들어있지 않음 → 문제 없음.

---

## 다시 빌드하려면 (UI 수정 후)

```powershell
# 이 PC 기준 (Rust=windows-gnu + wasm-pack 설치돼 있음)
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;$env:PATH"
$env:RUSTFLAGS = "-Cdlltool=$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-dlltool.exe"
cd d:\tmp\wie_app
npm run build:prod      # 산출물 = wie_web/dist/  (첫 릴리스 빌드는 wasm 컴파일로 ~10분)
```
- UI 파일: `wie_web/src/html/index.html`, `wie_web/src/css/style.css`, `wie_web/src/ts/index.ts`
- 게임/에뮬 코어는 안 건드림(git dep = dlunch/wie).
