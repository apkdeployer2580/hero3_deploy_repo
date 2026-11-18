"""Identify the InGame_txt string-id consumer(s).

Heuristic: for every `movs rX,#imm8` (imm8 < 196 = the id count) that is followed
within a short window by a `bl target`, tally (target -> set of ids). The real
getText / showMessage will collect a large, dense set of ids; unrelated callees
collect a couple of accidental hits.
"""
import struct
import sys
from collections import defaultdict

D = open(r'd:\tmp\client.bin', 'rb').read()
IMAGE = 0x100000
NIDS = 196


def hw(o):
    return struct.unpack_from('<H', D, o)[0]


def bl_target(o):
    """Decode a Thumb BL/BLX pair at o -> guest target, or None."""
    if o + 4 > len(D):
        return None
    h1, h2 = hw(o), hw(o + 2)
    if (h1 & 0xf800) != 0xf000:
        return None
    if (h2 & 0xf800) == 0xf800:
        blx = False
    elif (h2 & 0xf800) == 0xe800:
        blx = True
    else:
        return None
    off = ((h1 & 0x7ff) << 12) | ((h2 & 0x7ff) << 1)
    if (h1 >> 10) & 1:
        off -= 1 << 23
    t = IMAGE + o + 4 + off
    return (t & ~3) if blx else t


calls = defaultdict(set)
sites = defaultdict(list)
for o in range(0, len(D) - 2, 2):
    a = hw(o)
    if (a & 0xf800) != 0x2000:
        continue
    rd, imm = (a >> 8) & 7, a & 0xff
    if imm >= NIDS:
        continue
    for k in range(1, 9):
        p = o + 2 * k
        t = bl_target(p)
        if t is None:
            continue
        calls[t].add(imm)
        sites[t].append((o, imm, rd))
        break

top = sorted(calls.items(), key=lambda kv: -len(kv[1]))[:12]
for t, ids in top:
    print('callee %#x : %d distinct ids' % (t, len(ids)))

if len(sys.argv) > 1:
    want = int(sys.argv[1], 16)
    for t, ids in top:
        hits = [(o, i, r) for (o, i, r) in sites[t] if i == want]
        if hits:
            print('--- callee %#x sites passing id %#x ---' % (t, want))
            for o, i, r in hits:
                print('   fo %#07x guest %#x  movs r%d,#%#x' % (o, o + IMAGE, r, i))
