#!/usr/bin/env python3
"""아바타 옷장 (167차) — the costume SHEETS: `_bm` codec, the indexed-PNG frames the art lives in,
and the checks that keep a costume drawable.

    python tools_re/avatar_sheets.py verify                 # the codec round-trips every shipped res/hero sheet
    python tools_re/avatar_sheets.py check  [costumes/NN_x]  # every costume PNG is a frame the game can place
    python tools_re/avatar_sheets.py export <group> <sprite> <dir>   # a shipped sheet as PNG frames (to trace)
    python tools_re/avatar_sheets.py preview <out.png> [costumes/NN_x ...]   # contact sheet (scratch only)

★ THE PNG IS THE SOURCE (hero2's rule): `costumes/<NN>_<slug>/<part>_<frame>.png`, one indexed PNG per
frame. `add_costumes.py` packs them into `res/hero/h<D><CC><49+NN>_bm` jar members inside the
`patch_asset.py` chain; nothing else writes those members.

    part = helmet (4 frames) · armor (3) · gloves (12) · shoes (18) · shield (1)
         · weapon_<spear|sword|knife|gun|rifle|dark|holy> (6 · 5 · 4 · 6 · 7 · 1 · 1)

★★ A FRAME'S SIZE IS NOT FREE. The paper doll places every layer frame by its index and its size,
and every shipped sheet of a group carries the same sizes frame by frame (167차 survey: 23 torso
sheets, one signature; 13 of the 16 arm sheets; the weapons per category). `FRAME_SIZES` is that
signature; a PNG of any other size is refused. Draw inside the frame, never resize it.

★ Palette: index 0 is transparent (the game's own sentinel `0xf81f`); every other index is an
opaque RGB565 colour (the PNG's 8-bit channels are rounded to 565 — what you see is not exactly
what ships, by at most 4/8/4 levels). At most 255 opaque colours a frame; 15 or fewer pack as 4 bpp.
Semi-transparency does not exist in this format — a PNG alpha below 128 is transparent, above opaque.

Stdlib only (the PNG reader/writer is in here): the asset chain must not need PIL.
"""
import io
import os
import struct
import sys
import zipfile
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COSTUMES = os.path.join(ROOT, 'costumes')
ASSET = os.path.join(ROOT, 'android_app', 'app', 'src', 'main', 'assets', 'hero3_fixed.zip')

TRANSPARENT_565 = 0xF81F
SHEET_BASE = 49
SET_MAX = 50

# group -> (file prefix, part name). Mirrors `wie_util::avatar::{group_name_parts, sheet_member}`.
WEAPON_KINDS = ['spear', 'sword', 'knife', 'gun', 'rifle', 'dark', 'holy']  # categories 4..10
GROUPS = {0: ('h600', 'helmet'), 1: ('h100', 'armor'), 2: ('h200', 'gloves'), 3: ('h300', 'shoes')}
for _i, _k in enumerate(WEAPON_KINDS):
    GROUPS[4 + _i] = (f'h5{4 + _i:02d}', f'weapon_{_k}')
GROUPS[11] = ('h511', 'shield')
PART_GROUP = {part: g for g, (_, part) in GROUPS.items()}

# The shipped signature of every group: the VISIBLE size (cw, h) per frame index — what a PNG of
# that frame must measure. 167차 survey of res/hero, the majority of each group (helmet 17/17 ·
# torso 23/23 · arms 13/16 · legs 13/13 · spear 13/13 · sword 13/14 · claw 13/13 · gun 9/13 ·
# rifle 13/13 · the two stones 10/10 · shield 13/13).
FRAME_SIZES = {
    0: [(21, 28), (23, 28), (21, 28), (18, 26)],
    1: [(12, 10), (12, 10), (11, 11)],
    2: [(12, 14), (12, 15), (8, 9), (16, 9), (9, 11), (12, 12), (15, 14), (14, 11), (10, 9), (10, 10), (8, 9), (11, 14)],
    3: [(22, 19), (24, 19), (17, 18), (16, 21), (20, 13), (22, 19), (26, 18), (17, 17), (17, 18), (22, 14), (25, 15),
        (17, 18), (20, 16), (19, 16), (25, 18), (16, 18), (25, 17), (19, 13)],
    4: [(38, 15), (18, 5), (44, 9), (20, 3), (29, 26), (11, 11)],
    5: [(17, 29), (14, 44), (43, 22), (24, 8), (24, 8)],
    6: [(15, 30), (26, 28), (21, 30), (34, 21)],
    7: [(16, 19), (26, 15), (11, 24), (16, 18), (10, 22), (21, 17)],
    8: [(9, 28), (10, 30), (32, 13), (28, 21), (15, 24), (11, 3), (7, 7)],
    9: [(66, 45)],
    10: [(66, 45)],
    11: [(20, 14)],
}


def member_name(group: int, set_id: int) -> str:
    if not 1 <= set_id <= SET_MAX:
        raise ValueError(f'set {set_id} outside 1..{SET_MAX}')
    return f'res/hero/{GROUPS[group][0]}{SHEET_BASE + set_id:02d}_bm'


def shipped_member(group: int, sprite: int) -> str:
    return f'res/hero/{GROUPS[group][0]}{sprite:02d}_bm'


# ── the `_bm` codec ────────────────────────────────────────────────────────────

class Frame:
    """One frame: `w` x `h` indices (row-major, padding columns included), `cw` visible columns,
    `palette` = RGB565 values (index 0 = the transparent sentinel), `bpp` 8 or 4."""

    def __init__(self, w, h, cw, palette, pixels, bpp):
        self.w, self.h, self.cw, self.palette, self.pixels, self.bpp = w, h, cw, palette, pixels, bpp

    def size(self):
        return (self.cw, self.h)


def decode(data: bytes, name='<sheet>') -> list:
    count = struct.unpack_from('<H', data, 0)[0]
    off, frames = 2, []
    for i in range(count):
        (flen,) = struct.unpack_from('<I', data, off)
        t = data[off + 4]
        w, h, cw, palcnt = struct.unpack_from('<HHHH', data, off + 5)
        if t not in (0x0B, 0x0C):
            raise ValueError(f'{name} frame {i}: type {t:#x}')
        pal = list(struct.unpack_from(f'<{palcnt}H', data, off + 13))
        po = off + 13 + 2 * palcnt
        n = w * h if t == 0x0C else (w * h + 1) // 2
        if 9 + 2 * palcnt + n != flen:
            raise ValueError(f'{name} frame {i}: length {flen} != 9 + {palcnt}*2 + {n}')
        raw = data[po:po + n]
        if t == 0x0C:
            px = list(raw)
        else:
            px = []
            for k in range(w * h):
                b = raw[k // 2]
                px.append(b >> 4 if k % 2 == 0 else b & 0x0F)
        frames.append(Frame(w, h, cw, pal, px, 8 if t == 0x0C else 4))
        off = po + n
    if off != len(data):
        raise ValueError(f'{name}: consumed {off} of {len(data)} bytes')
    return frames


def encode(frames: list) -> bytes:
    out = bytearray(struct.pack('<H', len(frames)))
    for f in frames:
        if f.bpp == 8:
            raw = bytes(f.pixels)
            t = 0x0C
        else:
            raw = bytearray((len(f.pixels) + 1) // 2)
            for k, v in enumerate(f.pixels):
                raw[k // 2] |= (v << 4) if k % 2 == 0 else v
            raw = bytes(raw)
            t = 0x0B
        body = struct.pack('<BHHHH', t, f.w, f.h, f.cw, len(f.palette)) + struct.pack(f'<{len(f.palette)}H', *f.palette) + raw
        out += struct.pack('<I', len(body)) + body
    return bytes(out)


def to_rgb(v: int):
    r, g, b = (v >> 11) & 0x1F, (v >> 5) & 0x3F, v & 0x1F
    return (r * 255 // 31, g * 255 // 63, b * 255 // 31)


def to_565(rgb) -> int:
    r, g, b = rgb
    return ((r * 31 + 127) // 255) << 11 | ((g * 63 + 127) // 255) << 5 | ((b * 31 + 127) // 255)


# ── indexed PNG (stdlib) ───────────────────────────────────────────────────────

def _chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xFFFFFFFF)


def write_png(path: str, w: int, h: int, indices: list, palette_rgb: list) -> None:
    """An 8-bit indexed PNG; index 0 is transparent (tRNS), the rest opaque."""
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        raw += bytes(indices[y * w:(y + 1) * w])
    plte = b''.join(bytes(c) for c in palette_rgb)
    png = (b'\x89PNG\r\n\x1a\n' + _chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 3, 0, 0, 0))
           + _chunk(b'PLTE', plte) + _chunk(b'tRNS', b'\x00') + _chunk(b'IDAT', zlib.compress(bytes(raw), 9))
           + _chunk(b'IEND', b''))
    with open(path, 'wb') as f:
        f.write(png)


def _unfilter(data: bytes, w: int, h: int, bpp_bytes: int, stride: int) -> bytes:
    out = bytearray()
    prev = bytearray(stride)
    pos = 0
    for _ in range(h):
        ft = data[pos]
        line = bytearray(data[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        for i in range(stride):
            a = line[i - bpp_bytes] if i >= bpp_bytes else 0
            b = prev[i]
            c = prev[i - bpp_bytes] if i >= bpp_bytes else 0
            if ft == 1:
                line[i] = (line[i] + a) & 0xFF
            elif ft == 2:
                line[i] = (line[i] + b) & 0xFF
            elif ft == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif ft == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 0xFF
            elif ft != 0:
                raise ValueError(f'PNG filter {ft}')
        out += line
        prev = line
    return bytes(out)


def read_png(path: str):
    """`(w, h, rgba rows)` — every pixel as (r, g, b, a). Indexed (1/2/4/8 bit), grey, RGB and
    RGBA PNGs are read; 16-bit and interlaced ones are refused (no editor needs them here)."""
    with open(path, 'rb') as f:
        data = f.read()
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError(f'{path}: not a PNG')
    pos, idat, plte, trns = 8, bytearray(), None, None
    w = h = depth = ctype = None
    while pos < len(data):
        (n,) = struct.unpack_from('>I', data, pos)
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if kind == b'IHDR':
            w, h, depth, ctype, _comp, _filt, interlace = struct.unpack('>IIBBBBB', body)
            if interlace:
                raise ValueError(f'{path}: interlaced PNG')
        elif kind == b'PLTE':
            plte = [tuple(body[i:i + 3]) for i in range(0, len(body), 3)]
        elif kind == b'tRNS':
            trns = body
        elif kind == b'IDAT':
            idat += body
        elif kind == b'IEND':
            break
    if depth == 16:
        raise ValueError(f'{path}: 16-bit PNG')
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    bits = depth * channels
    stride = (w * bits + 7) // 8
    raw = _unfilter(zlib.decompress(bytes(idat)), w, h, max(1, bits // 8), stride)
    rows = []
    for y in range(h):
        line = raw[y * stride:(y + 1) * stride]
        row = []
        for x in range(w):
            if bits < 8:
                byte = line[(x * bits) // 8]
                shift = 8 - bits - (x * bits) % 8
                v = (byte >> shift) & ((1 << bits) - 1)
                vals = (v,)
            else:
                k = x * (bits // 8)
                vals = tuple(line[k:k + channels])
            if ctype == 3:
                i = vals[0]
                r, g, b = plte[i]
                a = trns[i] if trns is not None and i < len(trns) else 255
            elif ctype == 0:
                g0 = vals[0] * (255 // ((1 << depth) - 1))
                r = g = b = g0
                a = 255
            elif ctype == 4:
                r = g = b = vals[0]
                a = vals[1]
            elif ctype == 2:
                r, g, b = vals
                a = 255
                if trns is not None and len(trns) == 6 and (r, g, b) == struct.unpack('>HHH', trns):
                    a = 0
            else:
                r, g, b, a = vals
            row.append((r, g, b, a))
        rows.append(row)
    return w, h, rows


# ── frames <-> PNG ─────────────────────────────────────────────────────────────

def frame_to_png(path: str, f: Frame) -> None:
    pal = [(255, 0, 255)] + [to_rgb(v) for v in f.palette[1:]]
    idx = [f.pixels[y * f.w + x] for y in range(f.h) for x in range(f.cw)]
    write_png(path, f.cw, f.h, idx, pal)


def png_to_frame(path: str) -> Frame:
    """A PNG -> a frame: opaque colours in first-seen order after the transparent index 0."""
    w, h, rows = read_png(path)
    colours, idx = [], []
    for row in rows:
        for (r, g, b, a) in row:
            if a < 128:
                idx.append(0)
                continue
            c = to_565((r, g, b))
            if c == TRANSPARENT_565:
                c ^= 1  # never collide with the sentinel
            if c not in colours:
                colours.append(c)
            idx.append(1 + colours.index(c))
    if len(colours) > 255:
        raise ValueError(f'{path}: {len(colours)} opaque colours - a frame holds 255')
    pal = [TRANSPARENT_565] + colours
    bpp = 4 if len(pal) <= 16 else 8
    # Every shipped frame stores an EVEN row width (1085 of them: an odd visible width carries one
    # transparent padding column) — the stored layout is copied, not reasoned about.
    sw = w + (w & 1)
    if sw != w:
        idx = [v for y in range(h) for v in idx[y * w:(y + 1) * w] + [0]]
    return Frame(sw, h, w, pal, idx, bpp)


# ── a costume folder ───────────────────────────────────────────────────────────

def sheets_of(folder: str) -> dict:
    """`{group: [Frame]}` for a costume folder, every frame checked against the group's shipped
    signature. Raises on the first problem — a sheet the game cannot place must never ship."""
    parts = {}
    for fn in sorted(os.listdir(folder)):
        if not fn.endswith('.png'):
            continue
        part, _, n = fn[:-4].rpartition('_')
        if part not in PART_GROUP or not n.isdigit():
            raise ValueError(f'{os.path.relpath(os.path.join(folder, fn), ROOT)}: not <part>_<frame>.png '
                             f'(part one of {", ".join(sorted(PART_GROUP))})')
        parts.setdefault(part, {})[int(n)] = os.path.join(folder, fn)
    out = {}
    for part, files in parts.items():
        g = PART_GROUP[part]
        want = FRAME_SIZES[g]
        if sorted(files) != list(range(len(want))):
            raise ValueError(f'{os.path.relpath(folder, ROOT)}: {part} has frames {sorted(files)}, the game draws '
                             f'{len(want)} (0..{len(want) - 1})')
        frames = []
        for i, size in enumerate(want):
            f = png_to_frame(files[i])
            if f.size() != size:
                raise ValueError(f'{os.path.relpath(files[i], ROOT)}: {f.size()[0]}x{f.size()[1]}, the game places '
                                 f'this frame as {size[0]}x{size[1]} - draw inside the frame, do not resize it')
            frames.append(f)
        out[g] = frames
    return out


def set_id_of(folder: str) -> int:
    base = os.path.basename(os.path.normpath(folder))
    head = base.split('_', 1)[0]
    if not (len(head) == 2 and head.isdigit()):
        raise ValueError(f'{folder}: folder must be NN_<slug>')
    return int(head)


def costume_folders() -> list:
    if not os.path.isdir(COSTUMES):
        return []
    return [os.path.join(COSTUMES, d) for d in sorted(os.listdir(COSTUMES)) if os.path.isdir(os.path.join(COSTUMES, d))]


def members() -> dict:
    """`{jar member: _bm bytes}` for every costume sheet in `costumes/`."""
    out = {}
    for folder in costume_folders():
        sid = set_id_of(folder)
        for g, frames in sorted(sheets_of(folder).items()):
            out[member_name(g, sid)] = encode(frames)
    return out


# ── the jar ────────────────────────────────────────────────────────────────────

def jar_of(zpath: str = ASSET) -> zipfile.ZipFile:
    z = zipfile.ZipFile(zpath)
    jars = [n for n in z.namelist() if n.lower().endswith('.jar')]
    return zipfile.ZipFile(io.BytesIO(z.read(jars[0])))


def cmd_verify() -> int:
    j = jar_of()
    n = 0
    for name in sorted(j.namelist()):
        if name.startswith('res/hero/h') and name.endswith('_bm'):
            data = j.read(name)
            if encode(decode(data, name)) != data:
                print(f'FAIL: {name} does not round-trip')
                return 1
            n += 1
    print(f'OK: {n} shipped res/hero sheets round-trip through the codec byte for byte')
    return 0


def cmd_check(folders) -> int:
    bad = 0
    for folder in folders or costume_folders():
        try:
            sheets = sheets_of(folder)
        except ValueError as e:
            print(f'FAIL: {e}')
            bad += 1
            continue
        parts = ', '.join(f'{GROUPS[g][1]}({len(fr)})' for g, fr in sorted(sheets.items())) or 'no frames yet'
        print(f'OK: {os.path.relpath(folder, ROOT)} - {parts}')
    return 1 if bad else 0


def cmd_export(group: int, sprite: int, out: str) -> int:
    frames = decode(jar_of().read(shipped_member(group, sprite)))
    os.makedirs(out, exist_ok=True)
    part = GROUPS[group][1]
    for i, f in enumerate(frames):
        frame_to_png(os.path.join(out, f'{part}_{i}.png'), f)
    print(f'wrote {len(frames)} frame(s) of {shipped_member(group, sprite)} to {out}')
    return 0


def main(argv) -> int:
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0
    if argv[0] == 'verify':
        return cmd_verify()
    if argv[0] == 'check':
        return cmd_check(argv[1:])
    if argv[0] == 'export' and len(argv) == 4:
        return cmd_export(int(argv[1]), int(argv[2]), argv[3])
    if argv[0] == 'preview' and len(argv) >= 2:
        import avatar_preview  # noqa: F401  (PIL — scratch previews only)
        return avatar_preview.main(argv[1], argv[2:] or costume_folders())
    print(__doc__)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
