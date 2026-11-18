"""The game's `_bm` sheets as PIL RGBA frames (tools_re/avatar_sheets.py's codec) + a contact sheet."""
import io
import os
import sys
import zipfile

from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import avatar_sheets as AS  # noqa: E402

ASSET = AS.ASSET
_JAR = None


def jar(path=ASSET):
    global _JAR
    if _JAR is None:
        z = zipfile.ZipFile(path)
        name = [n for n in z.namelist() if n.endswith('.jar')][0]
        _JAR = zipfile.ZipFile(io.BytesIO(z.read(name)))
    return _JAR


def frame_rgba(f):
    img = Image.new('RGBA', (f.cw, f.h), (0, 0, 0, 0))
    px = img.load()
    for y in range(f.h):
        for x in range(f.cw):
            i = f.pixels[y * f.w + x]
            if i == 0 or f.palette[i] == AS.TRANSPARENT_565:
                continue
            r, g, b = AS.to_rgb(f.palette[i])
            px[x, y] = (r, g, b, 255)
    return img


def frames(name):
    return [frame_rgba(f) for f in AS.decode(jar().read(name), name)]


def contact(rows, out, scale=3, pad=4, bg=(60, 64, 72, 255), label=True):
    """rows = [(label, [PIL images])]"""
    lab_w = 70 if label else 0
    width = max(sum(i.width * scale + pad for i in imgs) for _, imgs in rows) + pad + lab_w
    height = sum(max([i.height for i in imgs] + [8]) * scale + pad for _, imgs in rows) + pad
    sheet = Image.new('RGBA', (width, height), bg)
    d = ImageDraw.Draw(sheet)
    y = pad
    for lab, imgs in rows:
        h = max([i.height for i in imgs] + [8]) * scale
        if label:
            d.text((2, y), str(lab), fill=(255, 255, 255, 255))
        x = pad + lab_w
        for img in imgs:
            big = img.resize((img.width * scale, img.height * scale), Image.NEAREST)
            sheet.alpha_composite(big, (x, y))
            x += big.width + pad
        y += h + pad
    sheet.save(out)
    return sheet.size
