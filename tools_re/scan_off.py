"""Find every site that materialises a given small constant as a CHAR offset.

Three Thumb-1 idioms the compiler used for CHAR+imm:
  A) movs rX,#imm8 ; lsls rX,rX,#sh          (imm8<<sh == target)
  B) ldr rX,[pc,#..] with literal == target
  C) adds rX,#imm8  (only <=0xff, reported separately)
Prints the *instruction* file offset.
"""
import struct, sys

D = open(r'd:\tmp\client.bin', 'rb').read()
IMAGE = 0x100000


def hw(off):
    return struct.unpack_from('<H', D, off)[0]


def scan(target):
    hits = []
    # A) movs + lsls pair (allow the lsls to be up to 6 halfwords later; the
    #    compiler often schedules an unrelated insn between them)
    for off in range(0, len(D) - 2, 2):
        a = hw(off)
        if (a & 0xf800) != 0x2000:
            continue
        rd = (a >> 8) & 7
        imm = a & 0xff
        for k in range(1, 7):
            p = off + 2 * k
            if p + 2 > len(D):
                break
            b = hw(p)
            if (b & 0xf800) != 0x0000:      # LSLS Rd,Rm,#imm5
                continue
            sh = (b >> 6) & 0x1f
            rm = (b >> 3) & 7
            rd2 = b & 7
            if rm != rd or rd2 != rd or sh == 0:
                continue
            if (imm << sh) == target:
                hits.append(('movs+lsls', off, p, rd))
            break
    # B) pc-relative literal
    for off in range(0, len(D) - 1, 2):
        a = hw(off)
        if (a & 0xf800) != 0x4800:
            continue
        lit = ((off + 4) & ~3) + (a & 0xff) * 4
        if lit + 4 > len(D):
            continue
        if struct.unpack_from('<I', D, lit)[0] == target:
            hits.append(('ldr-lit', off, lit, (a >> 8) & 7))
    return hits


if __name__ == '__main__':
    for a in sys.argv[1:]:
        t = int(a, 16)
        h = scan(t)
        print('=== target %#x : %d sites ===' % (t, len(h)))
        for kind, off, extra, rd in sorted(h, key=lambda x: x[1]):
            print('  fo %#07x guest %#x  %-9s r%d  (%s %#x)'
                  % (off, off + IMAGE, kind, rd, 'lsls@' if kind == 'movs+lsls' else 'lit@', extra))
