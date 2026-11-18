"""Resilient Thumb disassembler for Hero3 client.bin (skips undecodable halfwords).

Usage: python h3dis2.py <start_fo_hex> <end_fo_hex> [outfile]
Prints FILE OFFSETS. Annotates [sl,#imm] slots, pc-rel literals, and BL targets.
"""
import sys, struct, io, re
from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_LITTLE_ENDIAN

DATA = open(r'd:\tmp\client.bin', 'rb').read()
IMAGE_BASE = 0x100000
SL_FO = 0xb2c40
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


def slot_note(text):
    notes = []
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
                note += ' (reloc-> %#x%s)' % (tgt, ' %r' % s if s else '')
            notes.append(note)
    return notes


def line_for(ins):
    fo = ins.address - IMAGE_BASE
    line = '%06x  %-8s %s' % (fo, ins.mnemonic, ins.op_str)
    if ins.mnemonic == 'ldr' and '[pc' in ins.op_str:
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
    if ins.mnemonic in ('bl', 'blx', 'b', 'bne', 'beq', 'bgt', 'blt', 'bge', 'ble',
                        'bhi', 'bls', 'bcs', 'bcc', 'bmi', 'bpl'):
        m = re.match(r'^#(0x[0-9a-f]+|-?\d+)$', ins.op_str.strip())
        if m:
            line += '  ; -> fo %#x' % (int(m.group(1), 0) - IMAGE_BASE)
    for n in slot_note(ins.op_str):
        line += '  ; ' + n
    return line


def run(start, end):
    out = []
    pc = start
    while pc < end:
        got = False
        for ins in md.disasm(DATA[pc:end], IMAGE_BASE + pc, count=1):
            out.append(line_for(ins))
            pc += ins.size
            got = True
            break
        if not got:
            hw = struct.unpack_from('<H', DATA, pc)[0]
            out.append('%06x  .hw      %#06x' % (pc, hw))
            pc += 2
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
