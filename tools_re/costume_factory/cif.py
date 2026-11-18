"""Parser for res/hero/h<N>_cif (paper-doll composition records).

layout: N, list[N], A, then A animations: D (directions), D x { F (frames), F x record }
record: count, count x entry; entry = 4 bytes, or 9 when b0 & 0x80 (extended: 5 more bytes).
entry b0: low nibble = layer, 0x20 = flip h, 0x40 = flip v(?), 0x80 = extended.
"""
import struct


def _entry(d, p):
    b0 = d[p]
    L = 9 if b0 & 0x80 else 4
    e = d[p:p + L]
    ent = dict(layer=b0 & 0x0f, flags=b0 & 0xf0, frame=e[1],
               dx=struct.unpack('b', e[2:3])[0], dy=struct.unpack('b', e[3:4])[0],
               extra=bytes(e[4:]))
    return ent, p + L


def parse(d):
    p = 0
    n = d[p]; p += 1
    ids = list(d[p:p + n]); p += n
    a = d[p]; p += 1
    anims = []
    for _ in range(a):
        dirs = d[p]; p += 1
        dl = []
        for _ in range(dirs):
            f = d[p]; p += 1
            frames = []
            for _ in range(f):
                cnt = d[p]; p += 1
                ents = []
                for _ in range(cnt):
                    ent, p = _entry(d, p)
                    ents.append(ent)
                frames.append(ents)
            dl.append(frames)
        anims.append(dl)
    return dict(ids=ids, anims=anims, consumed=p, size=len(d))
