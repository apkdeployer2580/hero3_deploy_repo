"""Find all Thumb BL call sites targeting a given function, and show the
immediate loaded into r0 just before the call (the argument, when it is a
`movs r0,#imm`).

Classic Thumb BL is a halfword pair:
    hw1 = 0xF000 | imm11_hi     (offset bits 22:12)
    hw2 = 0xF800 | imm11_lo     (offset bits 11:1)
    target = (addr + 4) + sign_extend((hi << 12) | (lo << 1), 23)
"""
import struct, sys

data = open(r'd:\tmp\client.bin', 'rb').read()
target = int(sys.argv[1], 16)

def s23(v):
    return v - (1 << 23) if v & (1 << 22) else v

hits = []
for i in range(0, len(data) - 3, 2):
    hw1 = struct.unpack_from('<H', data, i)[0]
    if (hw1 & 0xf800) != 0xf000:
        continue
    hw2 = struct.unpack_from('<H', data, i + 2)[0]
    if (hw2 & 0xf800) != 0xf800:
        continue
    off = s23((((hw1 & 0x7ff) << 12) | ((hw2 & 0x7ff) << 1)))
    if (i + 4 + off) == target:
        hits.append(i)

print("callers of %#x: %d" % (target, len(hits)))
for i in hits:
    # look back a few halfwords for `movs r0,#imm8` (0x20xx)
    arg = None
    for back in range(2, 20, 2):
        if i - back < 0:
            break
        hw = struct.unpack_from('<H', data, i - back)[0]
        if (hw & 0xff00) == 0x2000:
            arg = hw & 0xff
            break
        # stop if we hit another BL/branch
        if (hw & 0xf800) in (0xf000, 0xe000):
            break
    print("  call @ %#08x   r0 = %s" % (i, ('%d (%#x)' % (arg, arg)) if arg is not None else '?'))
