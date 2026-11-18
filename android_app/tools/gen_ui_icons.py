"""Pack the overlay's UI button icons into `ui_icons.bin` for wie_android.

Source art lives in `android_app/tools/ui_icons/*.png` (author-supplied). Output is
`wie_android/src/ui_icons.bin`, pulled in with `include_bytes!`.

──────────────────────────────────────────────────────────────────────────────
★ WHY THIS STORES ALPHA ONLY, NOT COLOUR

Both supplied icons are black silhouettes on transparency. The overlay draws every
control as a translucent dark disc with a single light foreground colour
(`overlay::LABEL`), and it needs the same glyph in several states — normal, pressed,
dimmed-when-unavailable. Baking the source RGB in would give us a black-on-black
icon that is invisible on the plate, and would need one copy per state.

So we keep only the **coverage** (the alpha channel) and let the draw call pick the
colour. One asset, every state, and it matches the hand-drawn glyphs in overlay.rs
instead of fighting them.

That also means the author can hand over art in any colour: only the shape matters.

★ WHY IT IS CROPPED AND PRE-SCALED HERE

The source files are 981x982 and 4000x4000. Scaling 16 megapixels down to ~90px at
draw time, per frame, on the UI thread is not an option — and the crop matters just
as much: `minimap_toggle.png` has wide empty margins, so an uncropped icon would
draw at roughly half the visual size of the hand-drawn glyphs next to it.

Cropping to the alpha bounding box makes every icon's *ink* fill the cell, so icons
authored with different padding still come out the same visual weight.

Usage:  python android_app/tools/gen_ui_icons.py
"""
from __future__ import annotations

import pathlib
import struct

from PIL import Image

HERE = pathlib.Path(__file__).resolve().parent
SRC = HERE / "ui_icons"
OUT = HERE.parent.parent / "wie_android" / "src" / "ui_icons.bin"

# Cell size. The plates are ~98 px on a 1080p device and the glyph is inset, so ~90
# is already 1:1; 96 leaves headroom for a larger plate without resampling artifacts.
CELL = 96

# Name -> file. The name is what `ui_icons.rs` looks up, so it is part of the API and
# must stay in sync with the `Icon` enum there.
ICONS = [
    ("quick_page", "quick_page.png"),
    ("minimap_toggle", "minimap_toggle.png"),
]

# Below this the pixel is treated as empty when computing the bounding box. Not 0:
# the 4000px source has a faint anti-aliased halo whose stray pixels would inflate
# the box and shrink the visible glyph.
BBOX_ALPHA_FLOOR = 8


def mask_of(path: pathlib.Path) -> tuple[int, int, bytes]:
    """-> (w, h, alpha bytes) — cropped to the inked area and fitted into CELL."""
    im = Image.open(path).convert("RGBA")
    alpha = im.getchannel("A")

    box = alpha.point(lambda a: 255 if a >= BBOX_ALPHA_FLOOR else 0).getbbox()
    if box is None:
        raise SystemExit(f"{path.name}: fully transparent, nothing to pack")
    alpha = alpha.crop(box)

    # Fit inside CELL preserving aspect, then centre on a CELL x CELL field so the
    # Rust side can blit without knowing anything about the source proportions.
    w, h = alpha.size
    scale = min(CELL / w, CELL / h)
    fitted = alpha.resize((max(1, round(w * scale)), max(1, round(h * scale))), Image.LANCZOS)
    cell = Image.new("L", (CELL, CELL), 0)
    cell.paste(fitted, ((CELL - fitted.width) // 2, (CELL - fitted.height) // 2))
    return CELL, CELL, cell.tobytes()


def main() -> None:
    blob = bytearray(b"H3UI")
    blob.append(len(ICONS))
    for name, filename in ICONS:
        path = SRC / filename
        if not path.exists():
            raise SystemExit(f"missing source art: {path}")
        w, h, data = mask_of(path)
        raw = name.encode("ascii")
        blob.append(len(raw))
        blob += raw
        blob += struct.pack("<BB", w, h)
        blob += data
        ink = sum(1 for b in data if b > 128)
        print(f"{name}: {path.name} -> {w}x{h}, {ink} inked px ({100 * ink // (w * h)}% of cell)")

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(blob)
    print(f"wrote {OUT} ({len(blob)} bytes)")


if __name__ == "__main__":
    main()
