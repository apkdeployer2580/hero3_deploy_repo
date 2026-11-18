import struct, sys
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)
SL_FO = 0xb2c40

# targets = sl-offsets to find as literal-pool words
targets = [int(a, 16) for a in sys.argv[1:]] or [0x994, 0x998, 0x9a8]

for t in targets:
    pat = struct.pack('<I', t)
    hits = []
    start = 0
    while True:
        i = data.find(pat, start)
        if i < 0:
            break
        if i % 2 == 0:  # literal pools are >=2 aligned; usually 4
            hits.append(i)
        start = i + 1
    print("sl+%#x (data slot %#x)  literal-word hits: %d" % (t, SL_FO + t, len(hits)))
    for i in hits:
        aligned = " word4" if i % 4 == 0 else " halfword"
        print("    lit@%#08x%s" % (i, aligned))
