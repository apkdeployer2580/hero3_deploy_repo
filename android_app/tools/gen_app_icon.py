"""
WIE 네이티브 앱 런처 아이콘/이름 = 원작 영웅서기3 로고로 교체.

사용자 요청: "로고는 영웅서기3 어플 로고 그대로 사용하고싶고 이름도 영웅서기3 로 하고싶어".
교체 전 상태 = `res/` 에 mipmap 이 아예 없어 android:icon 미선언 → 안드로이드 기본
플레이스홀더 아이콘(사용자가 말한 "이상한 로고") + android:label="WIE Hero".

로고 소스 (발명 0 — 원작 아트만 사용):
  1순위 `d:/testrepo/android/app/src/main/assets/ui/title_bg.png` (530x735)
        = 실제 원작 부팅 타이틀 스크린샷(피처폰 "NO IC" 잡티만 in-paint 제거된 것,
          testrepo `tools/converter/gen_h3_title_bg.py` 산출). 사용 가능한 원작
          영웅서기3 로고 중 최고 해상도.
  2순위 이 스크립트 옆 `hero3_title_logo_src.png` (1순위에서 잘라 vendored — testrepo
        없이도 재생성 가능하도록).
  ※ 원작 패키지의 `Hero3/middle.icon`(20x20 KTF 메뉴 아이콘)·`big.icon`(120x110 배너)도
    같은 로고지만 해상도가 훨씬 낮고 big.icon 은 "전체이용가" 심의 배지가 박혀 있어
    런처 아이콘 소스로는 부적합 → 타이틀 아트를 채택.

산출 (안드로이드 표준 adaptive icon, minSdk 26 이므로 전 기기가 adaptive 경로):
  res/mipmap-anydpi-v26/ic_launcher.xml      adaptive (bg=ember 그라데이션, fg=로고)
  res/mipmap-anydpi-v26/ic_launcher_round.xml
  res/mipmap-{m,h,x,xx,xxx}hdpi/ic_launcher_background.png   108/162/216/324/432 px
  res/mipmap-{m,h,x,xx,xxx}hdpi/ic_launcher_foreground.png   108/162/216/324/432 px
  res/mipmap-{m,h,x,xx,xxx}hdpi/ic_launcher.png              48/72/96/144/192 px (legacy)
  res/mipmap-{m,h,x,xx,xxx}hdpi/ic_launcher_round.png        동일 크기 원형 (legacy)

기하: adaptive 캔버스 108dp 중 실제 보이는 영역 72dp, 권장 safe zone 66dp.
로고의 **밝은 픽셀 bbox 폭**을 캔버스의 66/108 (=safe zone) 에 맞춘다 → 어떤 런처
마스크(원형/스퀴클)에서도 금빛 로고가 잘리지 않는다.

배경 레이어 = 원작 타이틀의 **ember 세로 그라데이션**(위 검정 → 중앙 금갈색 띠 → 아래
검정)을 스크린샷 좌측 마진(x 1..30, 로고/심의배지/"게임시작"/EA 푸터가 전혀 걸치지
않는 순수 배경 열)에서 행별로 실측 샘플링해 만든다. 행 정렬은 crop 중심 = 캔버스
중심이므로, 전경 crop 의 페더 테두리가 같은 행의 배경 밝기와 일치 → 이음선/헤일로 0.
단색 배경(#2D1103, crop 테두리 평균)으로 먼저 만들어 보니 아트 중앙 띠보다 어두워
테두리 링이 보였다 → 그라데이션으로 교체.

실행: python android_app/tools/gen_app_icon.py
"""
from __future__ import annotations

import pathlib
import sys

from PIL import Image, ImageDraw

HERE = pathlib.Path(__file__).resolve().parent
RES = HERE.parent / "app" / "src" / "main" / "res"

# 로고 소스 후보 (앞이 우선).
TITLE_BG = pathlib.Path("d:/testrepo/android/app/src/main/assets/ui/title_bg.png")
VENDORED = HERE / "hero3_title_logo_src.png"
VENDORED_EMBER = HERE / "hero3_title_ember_src.png"   # 1 x N 세로 그라데이션 스트립.

# ember 그라데이션 샘플링 열 (좌측 마진). 우측 마진 x 499..528 은 y 8..100 에서
# "전체이용가" 심의 배지(초록)에 오염되어 있어 쓰지 않는다 (실측 확인).
EMBER_COLS = (1, 31)

# title_bg.png (530x735) 기준 실측 좌표.
#   금빛/흰색 밝은 픽셀 bbox = rows 233..506, cols 58..463  (심의 배지·"게임시작"·EA
#   푸터를 마스크한 뒤 luma>330 으로 측정) → 검 끝 ~ 검 밑동 + "영웅서기 대지의 성흔".
#   crop 은 여기에 10px 여유를 준 사각형.
LOGO_CROP = (48, 223, 474, 517)          # (l, t, r, b) → 426x294
BRIGHT_BBOX_IN_SRC = (58, 233, 464, 507)  # 밝은 로고 자체의 bbox (crop 좌표 아님)

# adaptive icon 규격 (dp).
CANVAS_DP = 108
SAFE_DP = 66          # 로고 폭을 여기에 맞춘다.
LEGACY_DP = 48        # 옛 런처용 정사각 아이콘 기준 크기.

DENSITIES = {         # 이름 -> dp 당 px 배율
    "mdpi": 1,
    "hdpi": 1.5,
    "xhdpi": 2,
    "xxhdpi": 3,
    "xxxhdpi": 4,
}

FEATHER_FRAC = 0.09   # crop 테두리 알파 페이드 폭 (crop 짧은 변 대비).


def canvas_span_in_src() -> int:
    """adaptive 캔버스 108dp 에 해당하는 원본 px 폭 (로고 66dp 기준 역산)."""
    bright_w = BRIGHT_BBOX_IN_SRC[2] - BRIGHT_BBOX_IN_SRC[0]
    return round(bright_w * CANVAS_DP / SAFE_DP)


def load_sources() -> tuple[Image.Image, Image.Image]:
    """(로고 crop RGB, ember 세로 스트립 RGB 1 x span) 을 돌려준다."""
    span = canvas_span_in_src()
    if TITLE_BG.is_file():
        src = Image.open(TITLE_BG).convert("RGB")
        if src.size != (530, 735):
            sys.exit(f"[FAIL] {TITLE_BG} 크기 {src.size} != (530, 735) — 실측 좌표 무효")
        logo = src.crop(LOGO_CROP)

        # crop 중심행을 캔버스 중심에 두고 span 행만큼 좌측 마진을 행평균한다.
        cy = (LOGO_CROP[1] + LOGO_CROP[3]) // 2
        top = cy - span // 2
        if top < 0 or top + span > src.height:
            sys.exit(f"[FAIL] ember 샘플 행 {top}..{top + span} 이 원본 높이 {src.height} 를 벗어남")
        margin = src.crop((EMBER_COLS[0], top, EMBER_COLS[1], top + span))
        ember = margin.resize((1, span), Image.BOX)   # 열 방향 평균 = 행별 대표색

        if not VENDORED.is_file():
            logo.save(VENDORED)
            print(f"  vendored 로고 기록: {VENDORED}")
        if not VENDORED_EMBER.is_file():
            ember.save(VENDORED_EMBER)
            print(f"  vendored ember 기록: {VENDORED_EMBER}")
        return logo, ember

    if VENDORED.is_file() and VENDORED_EMBER.is_file():
        print(f"  [경고] {TITLE_BG} 없음 → vendored 소스 사용")
        return Image.open(VENDORED).convert("RGB"), Image.open(VENDORED_EMBER).convert("RGB")
    sys.exit(f"[FAIL] 로고 소스 없음: {TITLE_BG} / {VENDORED}+{VENDORED_EMBER}")


def background(ember: Image.Image, canvas_px: int) -> Image.Image:
    """ember 스트립 → 정사각 full-bleed 배경 (가로 균일·세로 그라데이션)."""
    return ember.resize((canvas_px, canvas_px), Image.LANCZOS).convert("RGBA")


def feather(im: Image.Image) -> Image.Image:
    """RGB crop → 테두리를 투명으로 페이드한 RGBA."""
    w, h = im.size
    out = im.convert("RGBA")
    span = max(1, round(min(w, h) * FEATHER_FRAC))
    alpha = Image.new("L", (w, h), 255)
    ap = alpha.load()
    for y in range(h):
        dy = min(y, h - 1 - y)
        for x in range(w):
            d = min(x, w - 1 - x, dy)
            if d < span:
                ap[x, y] = round(255 * d / span)
    out.putalpha(alpha)
    return out


def foreground(logo_rgba: Image.Image, canvas_px: int) -> Image.Image:
    """로고를 adaptive foreground 캔버스에 배치 (밝은 bbox 폭 = safe zone)."""
    bright_w = BRIGHT_BBOX_IN_SRC[2] - BRIGHT_BBOX_IN_SRC[0]
    target_bright = canvas_px * SAFE_DP / CANVAS_DP
    scale = target_bright / bright_w
    w = max(1, round(logo_rgba.width * scale))
    h = max(1, round(logo_rgba.height * scale))
    scaled = logo_rgba.resize((w, h), Image.LANCZOS)
    fg = Image.new("RGBA", (canvas_px, canvas_px), (0, 0, 0, 0))
    fg.alpha_composite(scaled, ((canvas_px - w) // 2, (canvas_px - h) // 2))
    return fg


def zoomed_background(ember: Image.Image, size: int, logo_frac: float) -> Image.Image:
    """ember 배경을 로고 상대크기에 맞춰 확대 후 center-crop.

    adaptive 는 로고 폭이 캔버스의 SAFE/CANVAS 이므로 그라데이션 띠가 정확히 로고
    행에 맞는다. legacy 는 로고가 캔버스 대비 더 크므로 배경도 같은 배율로 확대해야
    금빛 띠가 로고 뒤에 온다.
    """
    zoom = logo_frac / (SAFE_DP / CANVAS_DP)
    big = max(size, round(size * zoom))
    plate = background(ember, big)
    off = (big - size) // 2
    return plate.crop((off, off, off + size, off + size))


def shape_mask(size: int, round_mask: bool) -> Image.Image:
    """정사각(라운드코너)/원형 마스크 (4x 슈퍼샘플 안티에일리어싱)."""
    m = Image.new("L", (size * 4, size * 4), 0)
    d = ImageDraw.Draw(m)
    if round_mask:
        d.ellipse((0, 0, size * 4 - 1, size * 4 - 1), fill=255)
    else:
        d.rounded_rectangle((0, 0, size * 4 - 1, size * 4 - 1), radius=round(size * 4 * 0.14), fill=255)
    return m.resize((size, size), Image.LANCZOS)


def legacy(logo_rgba: Image.Image, size: int, ember: Image.Image, round_mask: bool) -> Image.Image:
    """옛 런처용 정사각/원형 비트맵 (ember 배경 + 로고 합성 후 도형 마스크)."""
    # 정사각은 좌우 장식이 모서리에 닿지 않게, 원형은 원 안에 들어오게 실측 조정.
    logo_frac = 0.72 if round_mask else 0.82
    plate = zoomed_background(ember, size, logo_frac)

    bright_w = BRIGHT_BBOX_IN_SRC[2] - BRIGHT_BBOX_IN_SRC[0]
    scale = size * logo_frac / bright_w
    w = max(1, round(logo_rgba.width * scale))
    h = max(1, round(logo_rgba.height * scale))
    plate.alpha_composite(logo_rgba.resize((w, h), Image.LANCZOS), ((size - w) // 2, (size - h) // 2))

    plate.putalpha(shape_mask(size, round_mask))
    return plate


ADAPTIVE_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- 원작 영웅서기3 타이틀 로고. tools/gen_app_icon.py 산출 — 직접 편집하지 말 것. -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
</adaptive-icon>
"""


def main() -> None:
    print("영웅서기3 런처 아이콘 생성")
    logo_rgb, ember = load_sources()
    span = canvas_span_in_src()
    print(f"  로고 crop {logo_rgb.size} / 캔버스 = 원본 {span}px / ember 스트립 {ember.size}")
    logo = feather(logo_rgb)

    anydpi = RES / "mipmap-anydpi-v26"
    anydpi.mkdir(parents=True, exist_ok=True)
    for name in ("ic_launcher.xml", "ic_launcher_round.xml"):
        (anydpi / name).write_text(ADAPTIVE_XML, encoding="utf-8")

    # 단색 배경으로 만들었던 이전 산출물 정리.
    stale = RES / "values" / "ic_launcher_colors.xml"
    if stale.is_file():
        stale.unlink()
        print(f"  제거: {stale.name} (단색 배경 폐기)")

    for dens, mult in DENSITIES.items():
        d = RES / f"mipmap-{dens}"
        d.mkdir(parents=True, exist_ok=True)
        canvas = round(CANVAS_DP * mult)
        background(ember, canvas).convert("RGB").save(d / "ic_launcher_background.png")
        foreground(logo, canvas).save(d / "ic_launcher_foreground.png")
        legacy_px = round(LEGACY_DP * mult)
        legacy(logo, legacy_px, ember, False).save(d / "ic_launcher.png")
        legacy(logo, legacy_px, ember, True).save(d / "ic_launcher_round.png")
        print(f"  {dens:8s} adaptive {canvas}px / legacy {legacy_px}px")

    print(f"완료 → {RES}")


if __name__ == "__main__":
    main()
