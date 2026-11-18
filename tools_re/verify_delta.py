import struct, re
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)

vals = {}
for i in range(0, n - 3, 4):
    v = struct.unpack_from('<I', data, i)[0]
    vals.setdefault(v, []).append(i)

# a broad set of obvious string starts (asciiz, preceded by NUL)
strs = []
for m in re.finditer(rb'[\x20-\x7e]{6,}\x00', data):
    off = m.start()
    if off > 0 and data[off - 1] == 0:
        strs.append(off)
print("candidate asciiz strings:", len(strs))

for delta in (0x0eec, -9468, -9484, 0xbbb7665c - (1 << 32)):
    hit = sum(1 for s in strs if (s - delta) & 0xffffffff in vals)
    print("delta 0x%08x -> %d/%d strings referenced" % (delta & 0xffffffff, hit, len(strs)))

print()
for delta in (0x0eec,):
    for t, name in [(0x0ab1c0, 'MSG_5min'), (0x0ab1fc, 'IP1'), (0x0ab20c, 'BILLSOCK'),
                    (0x0ab218, 'IP2'), (0x0ab228, 'readSockErr'), (0x0a9d50, 'NETWORK')]:
        v = (t - delta) & 0xffffffff
        print("%-12s str@0x%06x val=0x%08x litpool@%s" % (
            name, t, v, [hex(x) for x in vals.get(v, [])]))
