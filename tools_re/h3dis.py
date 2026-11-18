"""Thumb disassembler for Hero3 client.bin with WIPI-aware annotation.

Address model (established this session):
  guest address = 0x100000 (wie IMAGE_BASE) + file offset
  RWPI static base sb/sl = 0x1b2c40  (= image + 0xb2c40), from the crash dump
  relocated data words hold  (file_offset - 0xeec); +0xeec recovers the target
"""
import os, sys, struct, io
from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_LITTLE_ENDIAN

# `d:\tmp` is the first machine's scratch dir; set H3_CLIENT_BIN to the extracted
# `client.bin64000` anywhere else. Same bytes either way (sha1 3d008576...).
CLIENT_BIN = os.environ.get('H3_CLIENT_BIN', r'd:\tmp\client.bin')
DATA = open(CLIENT_BIN, 'rb').read()
IMAGE_BASE = 0x100000
SL_FO = 0xb2c40          # static base, as a file offset
RELOC_DELTA = 0xeec

md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_LITTLE_ENDIAN)
md.detail = False


def cstr(fo, maxlen=64):
    end = DATA.find(b'\x00', fo, fo + maxlen)
    if end < 0:
        return None
    raw = DATA[fo:end]
    if not raw:
        return None
    try:
        s = raw.decode('euc-kr')
    except Exception:
        s = raw.decode('latin1')
    if all(0x20 <= b < 0x7f or b >= 0x80 for b in raw):
        return s
    return None


def annotate(text):
    """Resolve `[sl, #imm]` data slots and PC-relative literals."""
    notes = []
    import re
    m = re.search(r'\[(?:sl|r10|sb|r9), #(0x[0-9a-f]+|\d+)\]', text)
    if m:
        imm = int(m.group(1), 0)
        slot = SL_FO + imm
        if 0 <= slot < len(DATA) - 3:
            word = struct.unpack_from('<I', DATA, slot)[0]
            note = 'sl+%#x = data@%#x -> %#010x' % (imm, slot, word)
            tgt = word + RELOC_DELTA
            if 0 < tgt < len(DATA):
                s = cstr(tgt)
                if s:
                    note += ' (reloc-> %#x %r)' % (tgt, s)
                else:
                    note += ' (reloc-> %#x)' % tgt
            notes.append(note)
    return notes


def run(start, end):
    code = DATA[start:end]
    out = []
    for ins in md.disasm(code, IMAGE_BASE + start):
        fo = ins.address - IMAGE_BASE
        line = '%06x  %-8s %s %s' % (fo, ins.mnemonic, ins.op_str, '')
        # PC-relative literal
        if ins.mnemonic == 'ldr' and '[pc' in ins.op_str:
            import re
            m = re.search(r'#(0x[0-9a-f]+|-?\d+)', ins.op_str.split('[pc')[1])
            if m:
                lit = ((fo + 4) & ~3) + int(m.group(1), 0)
                if 0 <= lit < len(DATA) - 3:
                    word = struct.unpack_from('<I', DATA, lit)[0]
                    line += '  ; lit@%#x = %#010x' % (lit, word)
                    t = word + RELOC_DELTA
                    s = cstr(t) if 0 < t < len(DATA) else None
                    if s:
                        line += ' (reloc-> %#x %r)' % (t, s)
        for n in annotate(ins.op_str):
            line += '  ; ' + n
        out.append(line)
    return out


if __name__ == '__main__':
    start = int(sys.argv[1], 16)
    end = int(sys.argv[2], 16)
    dst = sys.argv[3] if len(sys.argv) > 3 else None
    lines = run(start, end)
    if dst:
        with io.open(dst, 'w', encoding='utf-8') as f:
            f.write('\n'.join(lines) + '\n')
        print('wrote', dst, len(lines), 'insns')
    else:
        print('\n'.join(lines))
