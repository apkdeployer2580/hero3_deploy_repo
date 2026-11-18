"""Extract every client->server request shape from Hero3's client.bin.

The serializer is fully identified (Track C session 1):
    0x17e150 put_u8(v)        0x17e184 put_bytes(src,n)
    0x17e1c4 put_u32_le(v)    0x17e204 put_u32_be(v)
    0x17e890 flush()          -> writes [u32 LE total_len][payload] and sends

So every send site is a straight-line run of put_* calls terminated by flush().
Walk back from each flush, disassemble, and reconstruct the payload template:
constant bytes where the argument is an immediate, `??` where it is dynamic.

This is a static complement to on-device capture: it enumerates requests we have
not triggered yet (복구, 캐시샵, 오브 결합, 창고 입출고 ...).
"""
import io
import struct
import sys

from capstone import CS_ARCH_ARM, CS_MODE_LITTLE_ENDIAN, CS_MODE_THUMB, Cs

DATA = open(r'd:\tmp\client.bin', 'rb').read()
IMAGE_BASE = 0x100000

PUT = {
    0x7E150: ('u8', 1),
    0x7E184: ('bytes', None),
    0x7E1C4: ('u32le', 4),
    0x7E204: ('u32be', 4),
}
FLUSH = 0x7E890
SESSION_INIT = 0x75CDC

md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_LITTLE_ENDIAN)


def s23(v):
    return v - (1 << 23) if v & (1 << 22) else v


def bl_target(fo):
    hw1 = struct.unpack_from('<H', DATA, fo)[0]
    hw2 = struct.unpack_from('<H', DATA, fo + 2)[0]
    if (hw1 & 0xF800) != 0xF000 or (hw2 & 0xF800) != 0xF800:
        return None
    off = s23(((hw1 & 0x7FF) << 12) | ((hw2 & 0x7FF) << 1))
    return fo + 4 + off


def find_calls(target):
    out = []
    for i in range(0, len(DATA) - 3, 2):
        if bl_target(i) == target:
            out.append(i)
    return out


def func_start(fo, limit=0x400):
    """Scan back for a plausible Thumb prologue (push {..., lr})."""
    best = max(0, fo - limit)
    for i in range(fo & ~1, best, -2):
        hw = struct.unpack_from('<H', DATA, i)[0]
        if (hw & 0xFE00) == 0xB400 and (hw & 0x0100):  # push {..., lr}
            return i
    return best


def trace(flush_fo):
    """Reconstruct the payload emitted on the straight-line path into `flush`.

    A linear sweep of the whole function is over-inclusive: it unions put_*
    calls from mutually exclusive branches, producing templates that no single
    message ever matches. So the sequence is RESET at every branch, leaving only
    the final basic block. That under-reports messages assembled across several
    blocks (loops, conditionals) but never invents a byte that is not really
    emitted on the path that reaches this flush.
    """
    start = func_start(flush_fo)
    code = DATA[start:flush_fo + 4]
    seq = []
    r0 = None
    r0_known = False
    mode = None
    truncated = False
    for ins in md.disasm(code, IMAGE_BASE + start):
        m, ops = ins.mnemonic, ins.op_str
        if m in ('movs', 'mov') and ops.startswith('r0, #'):
            r0 = int(ops.split('#')[1], 0)
            r0_known = True
            continue
        if m == 'bl':
            tgt = int(ops.lstrip('#'), 0) - IMAGE_BASE
            if tgt in PUT:
                kind, _ = PUT[tgt]
                seq.append((kind, r0 if r0_known else None))
            elif tgt == SESSION_INIT:
                mode = r0 if r0_known else None
            r0_known = False
            continue
        # Branch => new basic block. Anything collected so far may belong to a
        # path that does not reach this flush.
        if m.startswith('b') and m not in ('bl', 'blx') or m == 'pop' and 'pc' in ops:
            if seq:
                truncated = True
            seq = []
            r0_known = False
            continue
        if ops.startswith('r0'):
            r0_known = False
    return start, mode, seq, truncated


def render(seq):
    parts = []
    for kind, val in seq:
        if kind == 'u8':
            parts.append(f'{val:02x}' if val is not None else '??')
        elif kind == 'u32le':
            parts.append(f'{val:08x}(LE)' if val is not None else '????????(LE)')
        elif kind == 'u32be':
            parts.append(f'{val:08x}(BE)' if val is not None else '????????(BE)')
        else:
            parts.append('<bytes>')
    return ' '.join(parts) if parts else '(none detected)'


def main():
    sites = find_calls(FLUSH)
    out = io.StringIO()
    out.write(f'Hero3 client->server request catalog - {len(sites)} send sites\n')
    out.write('=' * 78 + '\n')
    out.write('template: constant bytes shown in hex, ?? = runtime value\n')
    out.write('only the final basic block before flush is shown (branch-safe);\n')
    out.write('"[+earlier blocks]" means more bytes are emitted upstream on some path.\n\n')
    rows = []
    for fo in sites:
        start, mode, seq, truncated = trace(fo)
        rows.append((fo, start, mode, seq))
        out.write(f'flush @ {fo:#08x}  (fn ~{start:#08x})')
        if mode is not None:
            out.write(f'  [session_init mode={mode}]')
        if truncated:
            out.write('  [+earlier blocks]')
        out.write('\n    ' + render(seq) + '\n')
    out.write('\n')

    # group by leading constant byte = likely opcode/category
    out.write('grouped by first constant byte:\n')
    groups = {}
    for fo, _s, _m, seq in rows:
        lead = seq[0][1] if seq and seq[0][0] == 'u8' and seq[0][1] is not None else None
        groups.setdefault(lead, []).append((fo, seq))
    for lead in sorted(groups, key=lambda x: (x is None, x)):
        label = f'{lead:#04x}' if lead is not None else 'dynamic/unknown'
        out.write(f'  {label}: {len(groups[lead])} site(s)\n')
        for fo, seq in groups[lead]:
            out.write(f'      {fo:#08x}  {render(seq)}\n')

    text = out.getvalue()
    print(text)
    with open('packet_catalog.txt', 'w', encoding='utf-8') as f:
        f.write(text)


if __name__ == '__main__':
    main()
