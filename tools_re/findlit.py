"""Find every Thumb-1 `ldr rX,[pc,#imm]` whose literal word equals one of the given values.
Reports the *instruction* file offset (so you know which function uses the constant)."""
import sys, struct
D = open(r'd:\tmp\client.bin', 'rb').read()

def scan(vals):
    out = []
    for off in range(0, len(D) - 1, 2):
        hw = struct.unpack_from('<H', D, off)[0]
        if (hw & 0xf800) != 0x4800:      # LDR Rd,[PC,#imm8*4]
            continue
        rd = (hw >> 8) & 7
        imm = (hw & 0xff) * 4
        lit = ((off + 4) & ~3) + imm
        if lit + 4 > len(D):
            continue
        w = struct.unpack_from('<I', D, lit)[0]
        if w in vals:
            out.append((off, rd, lit, w))
    return out

if __name__ == '__main__':
    vals = set(int(a, 16) for a in sys.argv[1:])
    for off, rd, lit, w in scan(vals):
        print('insn %#08x  ldr r%d,[pc] -> lit %#08x = %#x' % (off, rd, lit, w))
