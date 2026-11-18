#!/usr/bin/env python3
"""Generate the overlay's Hangul bitmap font, as Rust source.

WHY THIS EXISTS
---------------
`overlay::draw_glyph` is a 5x7 ASCII bitmap font, so every host-drawn string had
been ASCII — which is why the cash-buff status lines said "EXP X2 6D 23H" instead
of naming the item.  The player asked for the real Korean names (2026-08-06).

Until 60차 this shipped ONLY the syllables of the closed WORDS list below (~40
glyphs), which was the right size for toasts — and the wrong size for the hero5-
style touch HUD (61차): once the host draws item names, skill names and window
labels read out of GUEST memory, the text is no longer a closed set the build can
know. So the shipped set is now the full KS X 1001 완성형 table (2,350 syllables)
— exactly the syllables CP949 can encode in its base plane, and the game's own
strings are CP949, so every Hangul string that can come OUT of the game is
coverable by construction. (CP949's extension syllables outside KS X 1001 cannot
appear in the game data either: the game shipped as EUC-KR-safe text in 2006.)

The output stays a Rust source file of const bitmaps, not a runtime asset: it
must never fail to load, and keeping it in the crate means the standalone
selftests can `include!` it like everything else. 2,350 glyphs x 24 bytes is
~68 KB of data — noise next to the embedded icon atlases.

WORDS below is kept as the PREVIEW + regression list: the ASCII art of exactly
those syllables is printed for eyeballing (2,350 would scroll the terminal into
uselessness), and every WORDS syllable is asserted to be in the shipped set.

Re-run:
    python tools_re/gen_hangul_glyphs.py
and check the ASCII-art preview it prints before committing the result.
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

# The overlay's known Korean words — the PREVIEW/regression set (see module doc).
WORDS = [
    "경험의비석",  # cash item, EXP x2
    "경험의보주",  # cash item, EXP x4
    "초월의문장",  # cash item, equip level requirement -5
    "일",  # time units, from buffs::fmt_time_left
    "시간",
    "분",
    "저장완료",  # quick-save toast (player 2026-08-08: "Saved! 말고 저장완료")
    "저장실패",  # ...and its two failure twins, so the set cannot end up half-Korean
    "저장불가",
    # ★ 59차: the settings-row toasts. 57차 shipped the HEAT SAVER toast without
    # adding its syllables, so it drew as ": ()" on the device — a missing glyph is
    # silently BLANK (`overlay::draw_hangul` returns early). With the full KS X 1001
    # set that failure mode is gone for game-derived text; keep adding new UI words
    # here anyway so the preview shows them and the selftest pins them.
    "발열절감",  # HEAT SAVER row (57차)
    "재시작후적용",  # ...and its "applies at next start" tail
    "켬",  # on / off, shared by both toggles
    "끔",
    "게임패드",  # PAD HUD row (59차)
    "자동",
    "항상",
    # ★ 60차: the NET row (offline warehouse / cash shop) and the save-vault notice.
    "창고",  # NET row
    "오프라인",
    "서버",
    "세이브복원가능",  # the save vault's boot notice (60차)
    "이용할수없는기능",  # '#' on the slot screen, now that 복구 is off (60차)
]

# 12x12. Cell size unchanged from the closed-set era so no draw call moves.
# Font choice is per-machine: Gulim on Windows (hinted screen face, the original
# choice), NanumGothicCoding on Linux — evaluated 2026-08-19 by rendering
# 험/경/발/절/획/슬/롯/창/퀵 at 12 px thresholded against Baekmuk Dotum/Gulim,
# NanumGothic and UnDotum; NanumGothicCoding was the only one that kept every
# stroke. Both faces are legible at 12; they differ in flavour, not fidelity.
# NanumGothicCoding is OFL-licensed (SIL Open Font License), which permits this
# bitmap derivative.
W = H = 12
FONT_CANDIDATES = [
    (r"C:\Windows\Fonts\gulim.ttc", 12),
    ("/usr/share/fonts/truetype/nanum/NanumGothicCoding.ttf", 12),
    (r"C:\Windows\Fonts\malgun.ttf", 12),
    (r"C:\Windows\Fonts\batang.ttc", 12),
]
OUT = Path(__file__).resolve().parent.parent / "wie_android" / "src" / "hangul.rs"


def load_font():
    for path, size in FONT_CANDIDATES:
        if Path(path).exists():
            return ImageFont.truetype(path, size), Path(path).name, size
    sys.exit("no Korean font found; edit FONT_CANDIDATES")


def ksx1001_syllables():
    """The 2,350 완성형 syllables: EUC-KR lead 0xB0..0xC8, trail 0xA1..0xFE."""
    out = []
    for lead in range(0xB0, 0xC9):
        for trail in range(0xA1, 0xFF):
            out.append(bytes([lead, trail]).decode("euc-kr"))
    assert len(out) == 2350
    # KS X 1001 dictionary order IS Unicode code-point order for this block —
    # asserted rather than trusted, because glyph() binary-searches the table.
    assert out == sorted(out)
    return out


def render(font, ch):
    """1-bit W x H bitmap of `ch`, centred in the cell.

    Rendered DIRECTLY at the target size rather than oversampled-and-downscaled:
    a hinted screen face puts its strokes on the pixel grid on purpose, and
    resampling averages that back into grey which then thresholds to mush.
    """
    img = Image.new("L", (W * 2, H * 2), 0)
    ImageDraw.Draw(img).text((0, 0), ch, font=font, fill=255)
    bbox = img.getbbox()
    if bbox:
        img = img.crop(bbox)
    cell = Image.new("L", (W, H), 0)
    cell.paste(img, (max((W - img.width) // 2, 0), max((H - img.height) // 2, 0)))
    rows = []
    for y in range(H):
        bits = 0
        for x in range(W):
            if cell.getpixel((x, y)) >= 110:
                bits |= 1 << (W - 1 - x)
        rows.append(bits)
    return rows


def main():
    font, font_name, font_size = load_font()
    syllables = ksx1001_syllables()
    in_set = set(syllables)
    for w in WORDS:
        for ch in w:
            assert ch in in_set, f"WORDS syllable {ch!r} outside KS X 1001?!"
    glyphs = [(ch, render(font, ch)) for ch in syllables]

    preview = sorted({ch for w in WORDS for ch in w})
    for ch in preview:
        rows = dict(glyphs)[ch]
        print(f"--- {ch} ---")
        for bits in rows:
            print("".join("#" if bits & (1 << (W - 1 - x)) else "." for x in range(W)))

    blank = sum(1 for _, rows in glyphs if not any(rows))
    if blank:
        sys.exit(f"{blank} syllables rendered BLANK — wrong font path?")

    lines = [
        "//! Hangul glyphs: the full KS X 1001 완성형 table (2,350 syllables).",
        "//!",
        "//! GENERATED by `tools_re/gen_hangul_glyphs.py` — do not hand-edit; re-run the",
        f"//! tool. Source font: {font_name} at {font_size} px (OFL for the Nanum faces),",
        "//! thresholded to 1-bit cells.",
        "//!",
        "//! Until 60차 this held only the ~40 syllables of the overlay's own toasts.",
        "//! The hero5-style HUD (61차) draws item/skill names read out of guest memory,",
        "//! and the game's strings are CP949 — so shipping exactly the KS X 1001 set",
        "//! makes every game-derived Hangul string coverable BY CONSTRUCTION, and a",
        "//! missing-glyph blank (57차's `\": ()\"` toast) impossible for game text.",
        "",
        "/// Glyph cell size. Taller than the 5x7 ASCII cell, so mixed text is",
        "/// BOTTOM-aligned by the caller.",
        f"pub const HANGUL_W: u32 = {W};",
        f"pub const HANGUL_H: u32 = {H};",
        "",
        "/// Bitmap rows, MSB = leftmost pixel of the cell. SORTED by char (the generator",
        "/// emits KS X 1001 in EUC-KR order, which is Unicode order for this block) —",
        "/// [`glyph`] binary-searches it.",
        f"const GLYPHS: [(char, [u16; {H}]); {len(glyphs)}] = [",
    ]
    for ch, rows in glyphs:
        body = ", ".join(f"0b{bits:0{W}b}" for bits in rows)
        lines.append(f"    ('{ch}', [{body}]),")
    lines += [
        "];",
        "",
        "/// Rows for `ch`, or `None` when it is not one of the syllables we ship.",
        f"pub fn glyph(ch: char) -> Option<&'static [u16; {H}]> {{",
        "    GLYPHS.binary_search_by_key(&ch, |(c, _)| *c).ok().map(|i| &GLYPHS[i].1)",
        "}",
        "",
        "/// Whether every Hangul character in `s` has a glyph — the check the selftest",
        "/// uses so a new word can never ship with missing syllables.",
        "///",
        "/// `allow(dead_code)`: its only caller is `buffs_selftest.rs`, which `include!`s",
        "/// this file rather than linking the crate, so the crate build cannot see the use.",
        "#[allow(dead_code)]",
        "pub fn covers(s: &str) -> bool {",
        "    s.chars().filter(|c| !c.is_ascii()).all(|c| glyph(c).is_some())",
        "}",
        "",
    ]
    OUT.write_text("\n".join(lines), encoding="utf-8")
    print(f"\nwrote {OUT} ({len(glyphs)} glyphs)")


if __name__ == "__main__":
    main()
