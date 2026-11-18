"""Google Play 스토어 등록정보용 그래픽 자산 생성.

`gen_app_icon.py` 와 **같은 원작 아트에서** 만든다(발명 0): vendored
`hero3_title_logo_src.png`(타이틀 로고 crop) + `hero3_title_ember_src.png`
(원작 타이틀 배경의 세로 ember 그라데이션 1xN 스트립). 런처 아이콘과 스토어 아이콘이
따로 놀지 않도록 헬퍼도 그대로 재사용한다.

산출 (android_app/play/):
  icon_512.png            512x512   Play "앱 아이콘". 필수.
  feature_1024x500.png   1024x500   Play "그래픽 이미지". 필수. **알파 없음**(Play 요구).

Play 규격 (2026-05 기준 — Console 이 최종):
  * 앱 아이콘   = 512x512 PNG(32bit), <= 1 MB. Play 가 알아서 둥글게 마스킹하므로
                  모서리를 직접 깎지 않는다.
  * 그래픽 이미지 = 1024x500 PNG/JPEG, **투명도 불가**, <= 15 MB. 기기 프레임/스크린샷
                  콜라주는 지양(정책), 로고+배경이 안전하다.

실행: python android_app/tools/gen_play_assets.py
"""
from __future__ import annotations

import pathlib

from PIL import Image

import gen_app_icon as base

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent / "play"

ICON_PX = 512
FEATURE_W, FEATURE_H = 1024, 500
# 로고가 캔버스에서 차지할 폭(밝은 bbox 기준). 아이콘은 런처 legacy 와 같은 0.82,
# 그래픽 이미지는 가로가 길어 여백이 많으므로 조금 작게 잡아 숨통을 준다.
ICON_LOGO_FRAC = 0.82
FEATURE_LOGO_FRAC = 0.46


def place(canvas: Image.Image, logo: Image.Image, frac_of_width: float) -> None:
    """로고의 밝은 bbox 폭이 캔버스 폭의 `frac` 이 되도록 중앙 합성."""
    bright_w = base.BRIGHT_BBOX_IN_SRC[2] - base.BRIGHT_BBOX_IN_SRC[0]
    scale = canvas.width * frac_of_width / bright_w
    w = max(1, round(logo.width * scale))
    h = max(1, round(logo.height * scale))
    canvas.alpha_composite(logo.resize((w, h), Image.LANCZOS), ((canvas.width - w) // 2, (canvas.height - h) // 2))


def main() -> None:
    logo_rgb, ember = base.load_sources()
    logo = base.feather(logo_rgb)
    OUT.mkdir(parents=True, exist_ok=True)

    # --- 512x512 아이콘 ------------------------------------------------------
    # 런처 legacy 와 같은 배경 확대율을 써서, 금빛 띠가 로고 뒤에 오도록 맞춘다.
    icon = base.zoomed_background(ember, ICON_PX, ICON_LOGO_FRAC)
    place(icon, logo, ICON_LOGO_FRAC)
    icon_path = OUT / "icon_512.png"
    icon.save(icon_path)

    # --- 1024x500 그래픽 이미지 ---------------------------------------------
    # ember 는 세로 그라데이션이므로 높이에 맞춰 늘리고 가로로 채운다.
    #
    # ★ 배경 높이는 캔버스가 아니라 **로고 배율**에서 유도해야 한다. 아이콘의 확대율을
    # 그대로 가져다 쓰니 그라데이션 행이 로고 행과 어긋나서, 로고 crop 의 페더 테두리와
    # 배경 밝기가 달라지고 **로고 주위에 사각 패치가 보였다**(실측). 원본에서 로고 밝은
    # bbox 폭이 캔버스의 SAFE/CANVAS 일 때 ember 한 장이 캔버스를 정확히 덮으므로,
    # 폭 비율 f 로 놓으면 필요한 ember 높이는 W * f / (SAFE/CANVAS) 이다.
    tall = max(FEATURE_H, round(FEATURE_W * FEATURE_LOGO_FRAC / (base.SAFE_DP / base.CANVAS_DP)))
    strip = ember.resize((FEATURE_W, tall), Image.LANCZOS).convert("RGBA")
    off = (tall - FEATURE_H) // 2
    feature = strip.crop((0, off, FEATURE_W, off + FEATURE_H))
    place(feature, logo, FEATURE_LOGO_FRAC)
    # ★ 알파 제거: Play 는 그래픽 이미지에 투명도를 허용하지 않는다. 페더 테두리가
    # 남아 있으므로 검정 위에 합성해 완전 불투명 RGB 로 낸다.
    flat = Image.new("RGB", feature.size, (0, 0, 0))
    flat.paste(feature, mask=feature.split()[3])
    feature_path = OUT / "feature_1024x500.png"
    flat.save(feature_path)

    for p in (icon_path, feature_path):
        im = Image.open(p)
        print(f"  {p.name:<24} {im.size[0]}x{im.size[1]} {im.mode:<5} {p.stat().st_size / 1024:7.1f} KB")
    print(f"\n산출 위치: {OUT}")


if __name__ == "__main__":
    main()
