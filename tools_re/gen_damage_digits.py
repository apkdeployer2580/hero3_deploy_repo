#!/usr/bin/env python3
"""Lift the game's DAMAGE DIGITS out of the shipped asset into `damage_digits.rs`.

145차 (2026-09-30), user: *"HP SP바 밑에 하늘성 층수와 남은 몬스터 표시되고있던거를 중앙
상단 … 데미지 폰트에 색상만 다르게 해서 층수와 남은 몬스터를 표시"*. The overlay draws the
tower banner with the same digits the game pops over a hit monster.

What the game ships (`res/comm/number_bm`, the `_bm` sprite format `gen_h3_icons.py`
documents): four 4bpp frames, each ten digit cells side by side —

    frame 0  200x20  big   digits, blue  palette
    frame 1  140x13  small digits, blue  palette
    frame 2  200x20  big   digits, orange palette
    frame 3  140x13  small digits, orange palette

The four frames share their PIXELS: frames 2/3 are frames 0/1 with a different palette,
index for index (asserted below). The indices mean the same in both:

    0  transparent (the 0xf81f sentinel)
    1  dark  fill   (bottom of the glyph)   blue #0030ac · orange #cd1000
    2  mid   fill                           blue #0099e6 · orange #ff6900
    3  light fill   (top of the glyph)      blue #29eae6 · orange #ffaa31
    4  the 1-px OUTLINE around every glyph, near-white #fffaff in both

So "the damage font in another colour" is a palette of our own for 1..3 with the
outline kept — which is exactly what this file stores: the index maps, not colours.

The game's number renderer (`0x1e6c`) advances by the frame width / 10 per digit (20 /
14) — the cells below are that advance, glyph padding included.

Usage:  python tools_re/gen_damage_digits.py [--check]
"""
import argparse
import io
import struct
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSET = ROOT / "android_app" / "app" / "src" / "main" / "assets" / "hero3_fixed.zip"
OUT = ROOT / "wie_android" / "src" / "damage_digits.rs"
MEMBER = "res/comm/number_bm"


def read_member() -> bytes:
    z = zipfile.ZipFile(ASSET)
    jar_name = next(n for n in z.namelist() if n.lower().endswith(".jar"))
    jar = zipfile.ZipFile(io.BytesIO(z.read(jar_name)))
    return jar.read(MEMBER)


def rgb565(v: int) -> tuple[int, int, int]:
    return ((v >> 11) & 31) * 255 // 31, ((v >> 5) & 63) * 255 // 63, (v & 31) * 255 // 31


def decode(d: bytes) -> list[tuple[int, int, list[tuple[int, int, int]], list[int]]]:
    """[(cw, h, palette_rgb, indices row-major)] — the `_bm` layout, 4bpp frames only."""
    (count,) = struct.unpack_from("<H", d, 0)
    off = 2
    frames = []
    for _ in range(count):
        (flen,) = struct.unpack_from("<I", d, off)
        typ = d[off + 4]
        w, h, cw, palcnt = struct.unpack_from("<HHHH", d, off + 5)
        if typ != 0x0B:
            raise SystemExit(f"{MEMBER}: expected 4bpp frames (0x0b), found {typ:#x}")
        pal = [rgb565(v) for v in struct.unpack_from(f"<{palcnt}H", d, off + 13)]
        px = d[off + 13 + palcnt * 2 : off + 4 + flen]
        stride = (w + 1) // 2
        if len(px) != stride * h or flen != 9 + palcnt * 2 + stride * h:
            raise SystemExit(f"{MEMBER}: frame byte count does not add up")
        idx = []
        for y in range(h):
            row = px[y * stride : (y + 1) * stride]
            for x in range(cw):
                b = row[x // 2]
                idx.append((b >> 4) & 15 if x % 2 == 0 else b & 15)
        frames.append((cw, h, pal, idx))
        off += 4 + flen
    if off != len(d):
        raise SystemExit(f"{MEMBER}: {len(d) - off} trailing byte(s)")
    return frames


def luma(c: tuple[int, int, int]) -> float:
    r, g, b = c
    return 0.299 * r + 0.587 * g + 0.114 * b


def check_shape(frames) -> None:
    if len(frames) != 4:
        raise SystemExit(f"{MEMBER}: expected 4 frames, found {len(frames)}")
    for a, b in ((0, 2), (1, 3)):
        if frames[a][:2] != frames[b][:2] or frames[a][3] != frames[b][3]:
            raise SystemExit(f"{MEMBER}: frames {a} and {b} do not share their pixels — the recolour premise fails")
    for f in (0, 1, 2, 3):
        cw, h, pal, idx = frames[f]
        if cw % 10:
            raise SystemExit(f"{MEMBER}: frame {f} is {cw} wide, not ten cells")
        if len(pal) != 5 or max(idx) > 4:
            raise SystemExit(f"{MEMBER}: frame {f} is not the 5-entry palette this file assumes")
        # 1 < 2 < 3 in brightness, and 4 is the near-white outline.
        l1, l2, l3, l4 = (luma(pal[i]) for i in (1, 2, 3, 4))
        if not (l1 < l2 < l3 < l4) or min(pal[4]) < 240:
            raise SystemExit(f"{MEMBER}: frame {f}'s palette is not dark < mid < light < white: {pal}")


def cells(frame) -> list[list[str]]:
    cw, h, _pal, idx = frame
    step = cw // 10
    out = []
    for d in range(10):
        rows = []
        for y in range(h):
            rows.append("".join("." if (v := idx[y * cw + d * step + x]) == 0 else str(v) for x in range(step)))
        out.append(rows)
    return out


def emit_set(name: str, doc: str, digits: list[list[str]], w: int, h: int) -> str:
    lines = [
        f"/// {doc}",
        f"pub const {name}_W: usize = {w};",
        f"pub const {name}_H: usize = {h};",
        f"pub const {name}: [[&str; {name}_H]; 10] = [",
    ]
    for d, rows in enumerate(digits):
        lines.append(f"    // {d}")
        lines.append("    [")
        lines.extend(f'        "{r}",' for r in rows)
        lines.append("    ],")
    lines.append("];")
    return "\n".join(lines) + "\n"


def render(d: bytes) -> str:
    frames = decode(d)
    check_shape(frames)
    big, small = frames[0], frames[1]
    head = (
        "// @generated by tools_re/gen_damage_digits.py — DO NOT EDIT BY HAND.\n"
        "//\n"
        "// The GAME'S OWN damage digits, lifted from `res/comm/number_bm` inside the shipped\n"
        "// asset (145차: the tower banner draws with them). Regenerate (and re-run `--check`)\n"
        "// whenever the asset changes; the generator documents the format.\n"
        "//\n"
        "// One string per pixel row: `.` transparent, `1` dark / `2` mid / `3` light fill\n"
        "// (the game's three shades, bottom to top), `4` the white outline. The cell width is\n"
        "// the advance the game's number renderer `0x1e6c` uses.\n"
        "\n"
    )
    return (
        head
        + emit_set("BIG", "The big set (frames 0/2): 20x20 cells.", cells(big), big[0] // 10, big[1])
        + "\n"
        + emit_set("SMALL", "The small set (frames 1/3): 14x13 cells.", cells(small), small[0] // 10, small[1])
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="fail if the generated file is stale")
    args = ap.parse_args()

    text = render(read_member())
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current.replace("\r\n", "\n") != text:
            print(f"STALE: {OUT} does not match the asset. Run: python tools_re/gen_damage_digits.py")
            return 1
        print(f"OK: {OUT.name} matches the asset")
        return 0

    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({len(text)} chars)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
