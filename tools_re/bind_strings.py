import struct
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)

strings = {
    'money_down(돈 내리기)': 0xaa8bc,
    'cash_buy(캐시템)':      0xaa8d0,
    'orb_combine(오브 결합)':0xaa910,
    'wh_full(싱글 창고)':    0xa96e8,
    'no_reupload':           0xaa012,
    'saving_slot':           0xaa740,
}

# Build index of thumb ldr r,[pc,#imm] -> target word fo
ldrs = []
for i in range(0, n - 1, 2):
    h = struct.unpack_from('<H', data, i)[0]
    if 0x4800 <= h <= 0x4fff:
        imm = (h & 0xff) * 4
        tgt = ((i + 4) & ~3) + imm
        ldrs.append((i, tgt))

for label, sfo in strings.items():
    reloc = sfo - 0xeec
    pat = struct.pack('<I', reloc)
    print("== %s  string_fo=%#x reloc=%#x ==" % (label, sfo, reloc))
    start = 0
    wordaddrs = []
    while True:
        j = data.find(pat, start)
        if j < 0:
            break
        if j % 4 == 0:
            wordaddrs.append(j)
        start = j + 1
    if not wordaddrs:
        print("   (no literal-pool word holds reloc)")
    for W in wordaddrs:
        refs = [hex(i) for (i, t) in ldrs if t == W]
        print("   litword@%#08x  referenced-by-ldr: %s" % (W, refs if refs else 'none'))
