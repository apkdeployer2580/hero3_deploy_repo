"""Scan client.bin for Thumb BL/BLX targeting a given guest address."""
import sys, struct
D = open(r'd:\tmp\client.bin', 'rb').read()
IMAGE = 0x100000

def scan(target_guest):
    res = []
    for off in range(0, len(D) - 3, 2):
        h1 = struct.unpack_from('<H', D, off)[0]
        h2 = struct.unpack_from('<H', D, off + 2)[0]
        if (h1 & 0xf800) != 0xf000:
            continue
        if (h2 & 0xf800) == 0xf800:
            blx = False
        elif (h2 & 0xf800) == 0xe800:
            blx = True
        else:
            continue
        s = (h1 >> 10) & 1
        imm11 = h1 & 0x7ff
        imm11b = h2 & 0x7ff
        off23 = (imm11 << 12) | (imm11b << 1)
        if s:
            off23 -= (1 << 23)
        pc = IMAGE + off + 4
        tgt = pc + off23
        if blx:
            tgt &= ~3
        if tgt == target_guest:
            res.append((off, 'blx' if blx else 'bl'))
    return res

if __name__ == '__main__':
    for a in sys.argv[1:]:
        t = int(a, 16)
        r = scan(t)
        print('target %#x : %d callers' % (t, len(r)))
        for off, k in r:
            print('   file %#06x  %s  (guest %#x)' % (off, k, off + IMAGE))
