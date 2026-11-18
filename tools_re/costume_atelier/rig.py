"""Weapon rigs: how each weapon frame is the weapon's straight drawing turned and scaled.

A weapon is drawn ONCE, straight (the shipped sheet's own straight frame: sword 3+4 side by side,
spear 2/3, claw 3, gun 1, rifle 2), and every other frame of its kind is that drawing under an
affine map (angle, scale along/across, mirror) placed so its pixels land where the shipped frames
put theirs — the paper doll was written for those placements, so the grip stays in the hand.

`fit()` finds each map by searching angle/scale/shift until the shipped straight frame, sent
through it, covers the shipped target frame best (averaged over many shipped sheets of the kind);
`RIGS` is the result, checked in so a build never depends on a search. `turn()` renders a straight
drawing through a rig with RotSprite-style supersampling and a clean one-pixel outline after.
"""
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'costume_factory'))
sys.path.insert(0, os.path.dirname(HERE))

from PIL import Image  # noqa: E402

import avatar_sheets as AS  # noqa: E402

# kind -> (straight frame indices drawn side by side, x gap between them)
STRAIGHT = {
    'sword': ([3, 4], 0),
    'spear': ([2], 0),
    'shaft': ([3], 0),       # the spear's own shaft frames 1/3/5
    'knife': ([3], 0),
    'gun': ([1], 0),
    'rifle': ([2], 0),
}
KIND_GROUP = {'sword': 5, 'spear': 4, 'shaft': 4, 'knife': 6, 'gun': 7, 'rifle': 8}
# kind -> target frames rendered from the straight drawing (others are drawn by hand)
TARGETS = {
    'sword': [0, 1, 2],
    'spear': [0, 4],
    'shaft': [1, 5],
    'knife': [0, 1, 2],
    'gun': [0, 3, 5],
    'rifle': [3],
}


def straight_size(kind):
    g = KIND_GROUP[kind]
    idx, gap = STRAIGHT[kind]
    sizes = [AS.FRAME_SIZES[g][i] for i in idx]
    return (sum(w for w, _ in sizes) + gap * (len(sizes) - 1), max(h for _, h in sizes))


def join(frames, gap=0):
    w = sum(f.width for f in frames) + gap * (len(frames) - 1)
    h = max(f.height for f in frames)
    out = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    x = 0
    for f in frames:
        out.alpha_composite(f, (x, 0))
        x += f.width + gap
    return out


def split(img, kind):
    g = KIND_GROUP[kind]
    idx, gap = STRAIGHT[kind]
    out = []
    x = 0
    for i in idx:
        w, h = AS.FRAME_SIZES[g][i]
        out.append(img.crop((x, 0, x + w, h)))
        x += w + gap
    return out


# ── affine sampling ───────────────────────────────────────────────────────────

def _inv(rig):
    """rig = (angle_deg, sx, sy, mirror, tx, ty): straight point p -> R(angle)·diag(sx, sy)·M·p + t."""
    a, sx, sy, mir, tx, ty = rig
    c, s = math.cos(math.radians(a)), math.sin(math.radians(a))

    def back(X, Y):
        x, y = X - tx, Y - ty
        u, v = c * x + s * y, -s * x + c * y
        u, v = u / sx, v / sy
        if mir:
            v = -v
        return u, v
    return back


def turn(src, rig, size, sub=4, outline=None):
    """Render `src` (straight drawing, RGBA) through `rig` into a frame of `size`."""
    back = _inv(rig)
    W, H = size
    out = Image.new('RGBA', (W, H), (0, 0, 0, 0))
    sp = src.load()
    sw, sh = src.size
    from collections import Counter
    for Y in range(H):
        for X in range(W):
            cnt = Counter()
            for j in range(sub):
                for i in range(sub):
                    u, v = back(X + (i + 0.5) / sub, Y + (j + 0.5) / sub)
                    ui, vi = math.floor(u), math.floor(v)
                    if 0 <= ui < sw and 0 <= vi < sh:
                        p = sp[ui, vi]
                        cnt[p[:3] if p[3] >= 128 else None] += 1
                    else:
                        cnt[None] += 1
            col, n = cnt.most_common(1)[0]
            if col is None:
                opaque = sum(v for k, v in cnt.items() if k is not None)
                if opaque * 2 >= sub * sub:   # a tie leans opaque, so thin lines survive the turn
                    col = max(((k, v) for k, v in cnt.items() if k is not None), key=lambda kv: kv[1])[0]
            if col is not None:
                out.putpixel((X, Y), col + (255,))
    if outline is not None:
        out = reline(out, outline)
    return out


def reline(img, dark):
    """Pixels on the silhouette whose colour is not already dark take the outline colour."""
    w, h = img.size
    px = img.load()
    o = img.copy()
    op = o.load()

    def lum(c):
        return 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
    for y in range(h):
        for x in range(w):
            if px[x, y][3] == 0:
                continue
            edge = any(not (0 <= x + dx < w and 0 <= y + dy < h) or px[x + dx, y + dy][3] == 0
                       for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))
            if edge and lum(px[x, y]) > lum(dark) + 30:
                op[x, y] = dark + (255,)
    return o


# ── fitting a rig from the shipped sheets ──────────────────────────────────────

def _mask(img):
    w, h = img.size
    p = img.load()
    return {(x, y) for y in range(h) for x in range(w) if p[x, y][3] >= 128}


def _alpha(src):
    import numpy as np
    return (np.array(src)[:, :, 3] >= 128)


def turn_mask(alpha, rig, size, sub=2):
    """Fast opaque mask of `turn` (numpy): a target pixel is opaque when half its samples are."""
    import numpy as np
    a, sx, sy, mir, tx, ty = rig
    c, s = math.cos(math.radians(a)), math.sin(math.radians(a))
    W, H = size
    off = (np.arange(sub) + 0.5) / sub
    X = (np.arange(W)[None, :, None, None] + off[None, None, None, :])
    Y = (np.arange(H)[:, None, None, None] + off[None, None, :, None])
    x, y = X - tx, Y - ty
    u = (c * x + s * y) / sx
    v = (-s * x + c * y) / sy
    if mir:
        v = -v
    ui, vi = np.floor(u).astype(int), np.floor(v).astype(int)
    sh, sw = alpha.shape
    ok = (ui >= 0) & (ui < sw) & (vi >= 0) & (vi < sh)
    val = np.zeros(ok.shape, bool)
    val[ok] = alpha[vi[ok], ui[ok]]
    return val.reshape(H, W, sub * sub).mean(axis=2) >= 0.5


def _score(src, rig, target_mask, size):
    import numpy as np
    m = turn_mask(src, rig, size)
    inter = np.logical_and(m, target_mask).sum()
    union = np.logical_or(m, target_mask).sum()
    return inter / union if union else 0.0


def fit(kind, target, sheets, angles=None):
    """Search the rig mapping each sheet's straight frame onto its frame `target`."""
    import bmview as V
    g = KIND_GROUP[kind]
    size = AS.FRAME_SIZES[g][target]
    pairs = []
    for name in sheets:
        fr = V.frames(name)
        idx, gap = STRAIGHT[kind]
        pairs.append((_alpha(join([fr[i] for i in idx], gap)), _alpha(fr[target])))
    best = (0, None)
    angles = angles if angles is not None else range(-180, 180, 4)
    for mir in (False, True):
        for a in angles:
            for s in (0.5, 0.65, 0.8, 0.9, 1.0):
                rig = _center(pairs[0][0].shape[::-1], (a, s, 1.0, mir, 0, 0), size)
                sc = sum(_score(src, rig, tm, size) for src, tm in pairs) / len(pairs)
                if sc > best[0]:
                    best = (sc, rig)
    # refine
    sc0, rig = best
    for step_a, step_s, step_t in ((2, 0.05, 2), (1, 0.02, 1), (0.5, 0.01, 1)):
        improved = True
        while improved:
            improved = False
            a, sx, sy, mir, tx, ty = rig
            for cand in ((a + step_a, sx, sy, mir, tx, ty), (a - step_a, sx, sy, mir, tx, ty),
                         (a, sx + step_s, sy, mir, tx, ty), (a, sx - step_s, sy, mir, tx, ty),
                         (a, sx, sy + step_s, mir, tx, ty), (a, sx, sy - step_s, mir, tx, ty),
                         (a, sx, sy, mir, tx + step_t, ty), (a, sx, sy, mir, tx - step_t, ty),
                         (a, sx, sy, mir, tx, ty + step_t), (a, sx, sy, mir, tx, ty - step_t)):
                sc = sum(_score(src, cand, tm, size) for src, tm in pairs) / len(pairs)
                if sc > sc0 + 1e-9:
                    sc0, rig, improved = sc, cand, True
    return sc0, rig


def _center(src_size, rig, size):
    """Shift a rig so the straight drawing's centre lands on the frame's centre."""
    a, sx, sy, mir, _, _ = rig
    c, s = math.cos(math.radians(a)), math.sin(math.radians(a))
    cx, cy = src_size[0] / 2, src_size[1] / 2
    if mir:
        cy = -cy
    X = c * cx * sx - s * cy * sy
    Y = s * cx * sx + c * cy * sy
    return (a, sx, sy, mir, size[0] / 2 - X, size[1] / 2 - Y)


# fitted by fit() over every shipped sheet of the kind whose frame sizes match the signature
# (angle, scale along, scale across, mirror, shift x, shift y)
RIGS = {
    ('sword', 0): (-57, 0.59, 1.07, True, 5.106, 27.685),
    ('sword', 1): (79, 0.88, 1.01, True, -0.69, 3.423),
    ('sword', 2): (20, 0.94, 1.1, False, 0.315, -0.967),
    ('spear', 0): (-16, 1.08, 1.16, True, -0.907, 17.89),
    ('spear', 4): (-43, 0.92, 1.13, False, -1.56, 22.28),
    ('shaft', 1): (-15, 0.9, 0.9, False, -0.065, 3.539),
    ('shaft', 5): (-42, 0.8, 1.0, False, -1.297, 9.978),
    ('knife', 0): (-68, 0.88, 0.98, False, -7.967, 25.253),
    ('knife', 1): (-23.5, 0.88, 0.95, False, -5.248, 10.631),
    ('knife', 2): (-96, 0.9, 1.0, False, 1.657, 31.314),
    ('gun', 0): (39, 0.88, 0.97, False, 2.943, -3.445),
    ('gun', 3): (-39.5, 0.9, 1.0, False, -5.784, 12.775),
    ('gun', 5): (-20, 1.0, 1.02, False, -4.281, 7.899),
    ('rifle', 3): (21.5, 1.0, 1.0, False, 2.027, -1.946),
}


def frames_for(kind, straight):
    """Every rigged frame of `kind` from a straight drawing -> {frame index: image}."""
    g = KIND_GROUP[kind]
    out = {}
    for (k, t), r in RIGS.items():
        if k == kind:
            out[t] = turn(straight, r, AS.FRAME_SIZES[g][t])
    return out
