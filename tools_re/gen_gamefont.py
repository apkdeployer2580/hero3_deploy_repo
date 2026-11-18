#!/usr/bin/env python3
"""Emit the GAME'S OWN font as a Rust table, so the overlay can draw Korean.

Why
---
The overlay has always drawn with a 5x7 ASCII bitmap, which is why every settings
row is labelled in English, and with a hand-made 12x12 Gulim set (`hangul.rs`) whose
CLOSED syllable list makes any new Korean string a silent blank unless someone
remembers to regenerate it. Both problems disappear if the overlay draws with the
font the game itself uses: it is the right typeface by definition, and it covers all
2350 KS X 1001 syllables.

What the game ships (`res/font/` inside the jar)
------------------------------------------------
  korb   7920 B  360 glyphs x 22 B, 16x11 px, 2 B per row.
                 [0:160] 초성  8 sets x 20    (3520 B)
                 [160:248] 중성 4 sets x 22   (1936 B)
                 [248:360] 종성 4 sets x 28   (2464 B)
                 The split is the loader's own memcpy sizes (guest 0x3ec2..0x3f00).
  eng7   1045 B  95 glyphs x 11 B, 8x11 px, 1 B per row, starting at ' '.
  eng5   1045 B  the same shape, a narrower face.
  table  4700 B  2350 x u16 BE: Wansung index -> Johab code.
  etc     199 B  four tables, in FILE order (the loader's four memcpys,
                 guest 0x3f7e..0x3fda, are what pins this down):
                   [0:93]    remap: 초성 31, 중성 31, 종성 31   -> 0x1500072
                   [93:137]  초성 벌[종성유무*22 + 중성]         -> 0x1500046
                   [137:177] 중성 벌[종성유무*20 + 초성]         -> 0x150001e
                   [177:199] 종성 벌[중성]                       -> 0x1500008

Composition (guest 0x3a5a..0x3b30, read instruction by instruction)
-------------------------------------------------------------------
    johab = table[wansung_index]
    cho  = remap[      (johab >> 10) & 0x1f]
    jung = remap[0x1f + ((johab >> 5) & 0x1f)]
    jong = remap[0x3e + ( johab       & 0x1f)]
    hasJong = jong != 0
    glyph  = korb_cho [440 * choBul [hasJong*22 + jung] + 22 * cho]     (copy)
    glyph |= korb_jung[484 * jungBul[hasJong*20 + cho ] + 22 * jung]    (or)
    glyph |= korb_jong[616 * jongBul[jung]             + 22 * jong]     (or)

Usage
-----
    python tools_re/gen_gamefont.py            # writes wie_android/src/gamefont_data.rs
    python tools_re/gen_gamefont.py --check    # fail if stale

The data is baked rather than read at runtime on purpose: the overlay draws before
the guest has opened anything, a missing/short resource would have to be handled on
every draw, and 14 KB in .rodata costs nothing next to that.
"""

from __future__ import annotations

import argparse
import io
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSET = ROOT / "android_app" / "app" / "src" / "main" / "assets" / "hero3_fixed.zip"
OUT = ROOT / "wie_android" / "src" / "gamefont_data.rs"

WANT = ["korb", "eng7", "table", "etc"]


def read_font_files() -> dict[str, bytes]:
    z = zipfile.ZipFile(ASSET)
    jar_name = next(n for n in z.namelist() if n.lower().endswith(".jar"))
    jar = zipfile.ZipFile(io.BytesIO(z.read(jar_name)))
    return {n: jar.read(f"res/font/{n}") for n in WANT}


def ksx1001_table() -> list[int]:
    """The 2350 KS X 1001 Hangul syllables, in the order the game's `table` is
    indexed by, as `unicode - 0xAC00`.

    Derived from the codec rather than typed out: index i is EUC-KR
    `(0xB0 + i//94, 0xA1 + i%94)`, which is exactly the arithmetic the game does.
    """
    out = []
    for i in range(2350):
        row, col = divmod(i, 94)
        out.append(ord(bytes([0xB0 + row, 0xA1 + col]).decode("euc-kr")) - 0xAC00)
    if out != sorted(out):
        raise SystemExit("KS X 1001 hangul are not in Unicode order — the binary search would be wrong")
    return out


def rust_bytes(name: str, data: bytes, doc: str) -> str:
    cells = []
    for i in range(0, len(data), 16):
        cells.append("    " + " ".join(f"0x{b:02x}," for b in data[i : i + 16]))
    body = "\n".join(cells)
    return f"/// {doc}\npub const {name}: [u8; {len(data)}] = [\n{body}\n];\n"


def render(files: dict[str, bytes]) -> str:
    expect = {"korb": 7920, "eng7": 1045, "table": 4700, "etc": 199}
    for k, n in expect.items():
        if len(files[k]) != n:
            raise SystemExit(f"{k} is {len(files[k])} bytes, expected {n} — the asset changed; re-read the format notes")
    parts = [
        rust_bytes("KORB", files["korb"], "`res/font/korb` — 360 jamo glyphs, 22 B each (16x11, 2 B/row)."),
        rust_bytes("ENG7", files["eng7"], "`res/font/eng7` — 95 ASCII glyphs from ' ', 11 B each (8x11)."),
        rust_bytes("TABLE", files["table"], "`res/font/table` — 2350 x u16 BE, Wansung index -> Johab."),
        rust_bytes("ETC", files["etc"], "`res/font/etc` — remap + the three 벌 tables (see the module docs)."),
    ]
    ks = ksx1001_table()
    rows = ["    " + " ".join(f"0x{v:04x}," for v in ks[i : i + 12]) for i in range(0, len(ks), 12)]
    parts.append(
        "/// The 2350 KS X 1001 Hangul syllables as `unicode - 0xAC00`, in the order the\n"
        "/// game's `table` file is indexed by. Ascending, so a binary search gives the\n"
        "/// index directly.\n"
        f"pub const KSX1001: [u16; {len(ks)}] = [\n" + "\n".join(rows) + "\n];\n\n"
        "/// Position of `ch` in [`KSX1001`], or `None` when the standard omits it.\n"
        "pub fn ksx1001_index(ch: char) -> Option<usize> {\n"
        "    let v = u16::try_from((ch as u32).checked_sub(0xac00)?).ok()?;\n"
        "    KSX1001.binary_search(&v).ok()\n"
        "}\n"
    )
    return (
        "// @generated by tools_re/gen_gamefont.py — DO NOT EDIT BY HAND.\n"
        "//\n"
        "// The GAME'S OWN font, lifted from `res/font/` inside the shipped asset so the\n"
        "// overlay can draw Korean in the game's typeface. Regenerate (and re-run\n"
        "// `--check`) whenever the asset changes; `gamefont.rs` documents the format and\n"
        "// the composition rules, both read out of the guest's own loader and renderer.\n"
        "\n" + "\n".join(parts)
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="fail if the generated file is stale")
    args = ap.parse_args()

    text = render(read_font_files())
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current != text:
            print(f"STALE: {OUT} does not match the asset. Run: python tools_re/gen_gamefont.py")
            return 1
        print(f"OK: {OUT.name} matches the asset")
        return 0

    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({len(text)} chars)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
