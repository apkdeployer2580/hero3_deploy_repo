import struct
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)

vals = {}
for i in range(0, n - 3, 4):
    v = struct.unpack_from('<I', data, i)[0]
    vals.setdefault(v, []).append(i)
S = set(vals)

targets = [0x0ab1c0, 0x0ab1fc, 0x0ab20c, 0x0ab218, 0x0ab228, 0x0a9d50]

sets = []
for t in targets:
    sets.append(set(t - v for v in S))

inter = sets[0]
for s in sets[1:]:
    inter &= s
print("deltas matching ALL %d targets: %d" % (len(targets), len(inter)))
cands = sorted(inter, key=lambda d: abs(d))[:20]
for d in cands:
    print("  delta 0x%08x (%d)" % (d & 0xffffffff, d))

# also 5-of-6
from collections import Counter
c = Counter()
for s in sets:
    for d in s:
        c[d] += 1
best = [(d, k) for d, k in c.items() if k >= len(targets) - 1]
best.sort(key=lambda x: (-x[1], abs(x[0])))
print("\ntop deltas by target-hit-count:")
for d, k in best[:15]:
    print("  delta 0x%08x hits %d" % (d & 0xffffffff, k))
