import struct, sys
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)

# targets: literal-pool word addresses we care about
targets = {
    0xb3670: 'MSG_5min_disconnect',
    0xb3690: 'IP_210.222.18.28',
    0xb369c: 'BILLSOCK',
    0xb36a0: 'IP_211.239.150.70',
    0xb36a8: 'readSocket_CB_SET_ERROR',
    0xb33d4: 'NETWORK',
}
if len(sys.argv) > 1:
    targets = {int(a, 16): 'arg' for a in sys.argv[1:]}

print("=== THUMB  LDR Rd,[PC,#imm8*4] ===")
for i in range(0, n - 1, 2):
    h = struct.unpack_from('<H', data, i)[0]
    if 0x4800 <= h <= 0x4fff:
        rd = (h >> 8) & 7
        imm = (h & 0xff) * 4
        tgt = ((i + 4) & ~3) + imm
        if tgt in targets:
            print("  0x%06x  ldr r%d, =0x%06x   ; %s" % (i, rd, tgt, targets[tgt]))

print("=== ARM  LDR Rd,[PC,#imm12] ===")
for i in range(0, n - 3, 4):
    w = struct.unpack_from('<I', data, i)[0]
    if (w & 0x0f7f0000) == 0x051f0000:  # ldr rD,[pc,#-imm] / 0x059f = +imm
        pass
    if (w & 0x0f7f0000) in (0x051f0000, 0x059f0000):
        rd = (w >> 12) & 0xf
        imm = w & 0xfff
        up = (w >> 23) & 1
        tgt = i + 8 + (imm if up else -imm)
        if tgt in targets:
            print("  0x%06x  ldr r%d, =0x%06x   ; %s" % (i, rd, tgt, targets[tgt]))
