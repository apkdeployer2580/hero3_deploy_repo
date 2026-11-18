"""Small helpers every set script uses."""
from engine import Mat, Grid, dress, overlay, shade, paint, split  # noqa: F401
import bodies
import garments
import rig as R

import avatar_sheets as AS  # noqa: E402  (rig put tools_re on sys.path)


def family(keys, ramp, z=2, fam=None, tones=None):
    """One material per key, each pinned to a tone of `ramp` — draw a part pixel by pixel.
    keys='012345' -> tones 0..5 in order (or give `tones`, e.g. keys='LM', tones=(1, 3))."""
    fam = fam or ('f_' + keys)
    tones = tones if tones is not None else range(len(keys))
    return {k: Mat(ramp, z=z, flat=t, fam=fam) for k, t in zip(keys, tones)}


def cuff(g, key, above, new):
    """Pixels of `key` whose upper neighbour is one of `above` become `new` (a boot's top rim)."""
    for y in range(1, g.h):
        for x in range(g.w):
            if g.keys[y][x] == key and g.keys[y - 1][x] in above:
                g.keys[y][x] = new
    return g


def legs(mapping, pal, overlays=(), under=frozenset(), post=None):
    out = []
    for i in range(18):
        ov = [o.get(i) if isinstance(o, dict) else o for o in overlays]
        ov = [o for o in ov if o]
        g = dress(bodies.legs()[i], mapping, ov, under=under)
        if post:
            post(g, i)
        out.append(shade(g, pal, f'legs{i}'))
    return out


def arms(mapping, pal, overlays=(), post=None):
    out = []
    for i in range(12):
        ov = [o.get(i) if isinstance(o, dict) else o for o in overlays]
        ov = [o for o in ov if o]
        g = dress(bodies.arms()[i], mapping, ov)
        if post:
            post(g, i)
        out.append(shade(g, pal, f'arms{i}'))
    return out


def frames(blocks, pal, part):
    """Hand-drawn frames (a list of blocks) at the part's fixed sizes."""
    g = R.AS.PART_GROUP[part]
    sizes = R.AS.FRAME_SIZES[g]
    if len(blocks) != len(sizes):
        raise ValueError(f'{part}: {len(blocks)} frames, want {len(sizes)}')
    return [paint(b, pal, s, f'{part}{i}') for i, (b, s) in enumerate(zip(blocks, sizes))]


def straight(block, pal, kind):
    """A weapon's straight drawing (its kind's straight canvas size)."""
    return paint(block, pal, R.straight_size(kind), f'{kind}-straight')


def weapon(kind, straight_img, hand=None, outline=None):
    """Every frame of a weapon kind from its straight drawing (+ hand-drawn frames by index).

    kinds: sword (5 frames: 0-2 turned, 3-4 = the straight drawing split) · spear (6: head 2 and
    shaft 3 straight, 0/4 turned head, 1/5 turned shaft — pass shaft=… via hand['shaft']) ·
    knife (4: 3 straight, 0-2 turned) · gun (6: 1 straight, 0/3/5 turned, 2/4 hand) ·
    rifle (7: 2 straight, 3 turned, 0/1 hand, 4-6 the shipped effect frames)."""
    hand = dict(hand or {})
    g = R.KIND_GROUP[kind]
    n = len(R.AS.FRAME_SIZES[g])
    out = [None] * n
    for t, img in R.frames_for(kind, straight_img).items():
        out[t] = img if outline is None else R.reline(img, outline)
    for t, img in zip(R.STRAIGHT[kind][0], R.split(straight_img, kind)):
        out[t] = img
    if kind == 'spear':
        sh = hand.pop('shaft')
        for t, img in R.frames_for('shaft', sh).items():
            out[t] = img if outline is None else R.reline(img, outline)
        out[3] = sh
    if kind == 'rifle':
        import bmview as V
        ship = V.frames('res/hero/h50800_bm')
        for t in (4, 5, 6):
            out[t] = hand.pop(t, None) or ship[t]
    for t, img in hand.items():
        out[t] = img
    missing = [i for i, f in enumerate(out) if f is None]
    if missing:
        raise ValueError(f'{kind}: frames {missing} not drawn')
    for i, (f, s) in enumerate(zip(out, R.AS.FRAME_SIZES[g])):
        if f.size != tuple(s):
            raise ValueError(f'{kind} frame {i}: {f.size} != {tuple(s)}')
    return out


def hair_pal(ramp, z=2, extra=None):
    """The helmet palette for hair drawn with region separators: '0' outline, '1' a separator line
    between locks (tone 1, its own family so each lock shades on its own), 'h' a lock's pixels
    (shaded automatically), '2'..'5' a pixel pinned to that tone (strand highlights)."""
    pal = {
        '0': Mat(ramp, z=z, flat=0, fam='hair'),
        '1': Mat(ramp, z=z, flat=1, fam='sep'),
        'h': Mat(ramp, z=z, fam='hair'),
        '2': Mat(ramp, z=z, flat=2, fam='hair'),
        '3': Mat(ramp, z=z, flat=3, fam='hair'),
        '4': Mat(ramp, z=z, flat=4, fam='hair'),
        '5': Mat(ramp, z=z, flat=5, fam='hair'),
    }
    pal.update(extra or {})
    return pal


def circle(prims, pal):
    """A dark/holy stone frame (the magic circle) -> [one 66x45 frame]. See circles.py."""
    import circles
    return [circles.render(prims, pal)]


def flip(block):
    """A text block mirrored left-right (the '|' tone half too)."""
    rows, trows = split(block)
    w = max(len(r) for r in rows)
    out = [r.ljust(w, '.')[::-1] for r in rows]
    if trows is not None:
        tw = max(len(r) for r in trows)
        out = [a + ' | ' + b.ljust(tw, '.')[::-1] for a, b in zip(out, trows)]
    return chr(10).join(out)
