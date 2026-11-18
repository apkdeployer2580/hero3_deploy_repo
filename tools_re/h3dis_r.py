"""Resilient wrapper around h3dis.run(): resumes past undecodable halfwords."""
import sys, io, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import h3dis

def run_res(start, end):
    out = []
    fo = start
    while fo < end:
        lines = h3dis.run(fo, end)
        if lines:
            out.extend(lines)
            last = int(lines[-1].split()[0], 16)
            # advance past last decoded insn: figure width by re-disasm of 1
            nxt = last + 2
            # try 4-byte wide (bl/blx/ldr.w)
            import struct
            hw = struct.unpack_from('<H', h3dis.DATA, last)[0]
            if (hw & 0xf800) in (0xf000, 0xf800, 0xe800):
                nxt = last + 4
            if nxt <= fo:
                nxt = fo + 2
            fo = nxt
        else:
            out.append('%06x  <undecodable %02x%02x>' % (fo, h3dis.DATA[fo+1], h3dis.DATA[fo]))
            fo += 2
    return out

if __name__ == '__main__':
    s = int(sys.argv[1], 16); e = int(sys.argv[2], 16)
    lines = run_res(s, e)
    dst = sys.argv[3] if len(sys.argv) > 3 else None
    if dst:
        io.open(dst, 'w', encoding='utf-8').write('\n'.join(lines) + '\n')
        print('wrote', dst, len(lines))
    else:
        print('\n'.join(lines))
