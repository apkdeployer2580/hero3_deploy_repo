"""Offline paper-doll compositor for hero3: cif records + layer sheets -> PIL images."""
import math

from PIL import Image

import bmview as V
import cif

LAYER_GROUP_PREFIX = {0: 'h000', 1: 'h100', 2: 'h200', 3: 'h300', 6: 'h600', 7: 'h511'}


def load_sheet(prefix, sprite):
    return V.frames(f'res/hero/{prefix}{sprite:02d}_bm')


def transform(img, ent):
    flags = ent['flags']
    if flags & 0x20:
        img = img.transpose(Image.FLIP_LEFT_RIGHT)
    if flags & 0x40:
        img = img.transpose(Image.FLIP_TOP_BOTTOM)
    ex = ent['extra']
    if flags & 0x80 and len(ex) >= 3:
        sx, sy, rot = ex[0], ex[1], ex[2]
        if (sx, sy) != (25, 25) and sx and sy:
            w = max(1, round(img.width * sx / 25))
            h = max(1, round(img.height * sy / 25))
            img = img.resize((w, h), Image.NEAREST)
        if rot:
            img = img.rotate(-rot, resample=Image.NEAREST, expand=True)
    return img


def compose(record, layers, mode='center', size=(110, 110), anchor=(55, 78), skip=()):
    """layers: dict layer -> list of frame images (or None to skip the layer)."""
    canvas = Image.new('RGBA', size, (0, 0, 0, 0))
    ax, ay = anchor
    own_head = layers.get('own_head')      # 171차: the 투구 frames carry the whole head
    for i, ent in enumerate(record):
        L = ent['layer']
        if L in skip:
            continue
        if L == 0 and own_head and any(e['layer'] == 6 for e in record[i + 1:]):
            continue
        frames = layers.get(L)
        if not frames or ent['frame'] >= len(frames):
            continue
        img = transform(frames[ent['frame']], ent)
        if mode == 'center':
            x = ax + ent['dx'] - img.width // 2
            y = ay + ent['dy'] - img.height // 2
        else:
            x = ax + ent['dx']
            y = ay + ent['dy']
        layer = Image.new('RGBA', size, (0, 0, 0, 0))
        layer.paste(img, (x, y))
        canvas = Image.alpha_composite(canvas, layer)
    return canvas


def poses(cif_index, anims=(0,), frames=(0,)):
    r = cif.parse(V.jar().read(f'res/hero/h{cif_index}_cif'))
    out = []
    for a in anims:
        if a >= len(r['anims']):
            continue
        for di, dl in enumerate(r['anims'][a]):
            for fi in frames:
                if fi < len(dl):
                    out.append(((a, di, fi), dl[fi]))
    return out
