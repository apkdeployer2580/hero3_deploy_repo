"""Pack Hero3 quickbar icons into a binary atlas for the wie_android overlay HUD.

Decodes /comm/item_bm (29 consumable/scroll icons) + /skill/s4_bm..s10_bm (7
per-class skill sheets, each a 135x15 strip = 9 cells of 15x15) and writes
`icons.bin`:

  magic "H3IC" (4)
  item_count       u16 LE
  item icons[item_count]:  w u8, h u8, px[w*h] u32 LE (0xAARRGGBB; a=0 transparent)
  skill_sheet_count u8            (sheets s4..s10 in order = classes 0..6)
  per sheet:  cell_count u8, cells[cell_count]: w u8, h u8, px[w*h] u32 LE

Usage:  python gen_h3_icons.py
Output: d:\\tmp\\wie_app\\wie_android\\src\\icons.bin   (for include_bytes!)

──────────────────────────────────────────────────────────────────────────────
★ 2026-07-28 — self-contained `_bm` decoder (replaces the repo's convert_bm_v2).

The repo decoder (`d:\\testrepo\\tools\\converter\\convert_bm_v2.py`) treats the
0xf81f halfword after the 9-byte frame header as a *frame boundary marker* and
starts the palette 2 bytes later.  That is wrong: **0xf81f IS palette[0]** (the
transparent sentinel) and `palcnt` counts it.  Consequences of the old reading:
every pixel index k resolved to palette[k+1] and the pixel stream was read 2
bytes late -> scrambled colours (the "노이즈 아이콘" this HUD kept hitting) and a
2-byte underrun the old docstring waved off as "정렬 이슈".

Proof = exact byte arithmetic, asserted below for every frame:

    file:  u16 frame_count
    frame: u32 frame_len, u8 type(0x0b=4bpp|0x0c=8bpp), u16 w, u16 h,
           u16 cw, u16 palcnt, u16 palette[palcnt] (RGB565 LE), pixels[...]
    frame_len == 9 + palcnt*2 + pixel_bytes      (pixel_bytes = w*h or (w*h+1)//2)
    sum(4 + frame_len) + 2 == filesize

  item_bm 6426/6426 bytes, 29/29 frames; s4..s10_bm 2514..2544 bytes, 1 frame
  each — all exact.  Rendered output is coherent art (potions/rings/scrolls,
  arrows/crosshairs/bows) instead of colour noise.

`w` is the byte-padded row width (4bpp rows are byte aligned); `cw` is the real
visible width, so frames are emitted cw-wide.  Index 0 = transparent.

This is a cross-game finding — every Hero3/Hero4 `_bm` bake made with
convert_bm_v2 carries the same defect.  Fixing the repo tool + re-baking is a
separate, larger job (see CUSTOM_HUD.md).
"""
import os
import struct

JAR = r"d:\tmp\h3_jar_inspect"
OUT = r"d:\tmp\wie_app\wie_android\src\icons.bin"
SKILL_STRIDE = 15  # per-icon column stride within a class strip


# ── _bm decoder ──────────────────────────────────────────────────────────────

def rgb565_to_argb(v):
    """RGB565 -> 0xAARRGGBB; the 0xf81f sentinel (palette[0]) is transparent."""
    if v == 0xF81F:
        return 0
    r, g, b = (v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F
    return 0xFF000000 | (r * 255 // 31) << 16 | (g * 255 // 63) << 8 | (b * 255 // 31)


def decode_bm(path):
    """-> [(cw, h, [u32 px])] for every frame, with the format asserted exactly."""
    return decode_bm_bytes(open(path, "rb").read(), path)


def decode_bm_bytes(d, path="<bytes>"):
    """The same decode, straight off a jar member — no temp file needed.

    Callers that already hold the member (the map tools read `res/map/theme_*_bm`
    out of the jar) should use this; `decode_bm` is the file-path wrapper.
    """
    count = struct.unpack_from("<H", d, 0)[0]
    off = 2
    frames = []
    for i in range(count):
        (flen,) = struct.unpack_from("<I", d, off)
        off += 4
        t = d[off]
        w, h, cw, palcnt = struct.unpack_from("<HHHH", d, off + 1)
        assert t in (0x0B, 0x0C), f"{path} frame {i}: bad type {t:#x}"
        assert 1 <= cw <= w, f"{path} frame {i}: cw {cw} > w {w}"
        pal = [rgb565_to_argb(struct.unpack_from("<H", d, off + 9 + 2 * j)[0])
               for j in range(palcnt)]
        po = off + 9 + palcnt * 2
        nbytes = w * h if t == 0x0C else (w * h + 1) // 2
        assert 9 + palcnt * 2 + nbytes == flen, \
            f"{path} frame {i}: len {flen} != 9+{palcnt}*2+{nbytes}"
        px = [0] * (cw * h)
        for k in range(w * h):
            x, y = k % w, k // w
            if x >= cw:
                continue  # row padding
            idx = d[po + k] if t == 0x0C else (
                (d[po + k // 2] >> 4) if k % 2 == 0 else (d[po + k // 2] & 0x0F))
            if idx and idx < palcnt:
                px[y * cw + x] = pal[idx]
        frames.append((cw, h, px))
        off = po + nbytes
    assert off == len(d), f"{path}: consumed {off} of {len(d)} bytes"
    return frames


def pack_icon(w, h, px):
    out = bytearray([w, h])
    for v in px:
        out += struct.pack("<I", v)
    return out


def main():
    blob = bytearray(b"H3IC")

    # --- items: comm/item_bm, one frame per icon ---
    items = decode_bm(os.path.join(JAR, "comm", "item_bm"))
    blob += struct.pack("<H", len(items))
    for w, h, px in items:
        blob += pack_icon(w, h, px)
    print(f"items: {len(items)} icons ({items[0][0]}x{items[0][1]})")

    # --- skills: s4..s10, each one 135x15 strip sliced into 15px cells ---
    #     The game HUD blits exactly this: srcX = cell*15, srcY = 0, 15x15
    #     (guest 0x14a51a; the `9` there is a trailing flag, not a height —
    #     the item branch passes the same 9 while drawing full-size icons).
    sheets = [f"s{n}_bm" for n in range(4, 11)]
    blob += bytes([len(sheets)])
    for sh in sheets:
        frames = decode_bm(os.path.join(JAR, "skill", sh))
        assert len(frames) == 1, f"{sh}: expected 1 strip frame, got {len(frames)}"
        sw, shh, spx = frames[0]
        ncell = max(1, sw // SKILL_STRIDE)
        blob += bytes([ncell])
        for c in range(ncell):
            cw = min(SKILL_STRIDE, sw - c * SKILL_STRIDE)
            cell = [spx[y * sw + c * SKILL_STRIDE + x]
                    for y in range(shh) for x in range(cw)]
            blob += pack_icon(cw, shh, cell)
        print(f"{sh}: {sw}x{shh} -> {ncell} cells")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "wb") as fp:
        fp.write(blob)
    print(f"wrote {OUT} ({len(blob)} bytes)")


if __name__ == "__main__":
    main()
