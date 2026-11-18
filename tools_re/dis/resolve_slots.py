import struct
data = open(r'd:\tmp\client.bin','rb').read()
SL_FO = 0xb2c40
DELTA = 0xeec

# (label, V offset from sl) collected from warehouse send funcs
slots = [
 ('57568/05_14 after',       0x4a8),
 ('575f8/05_3f after',       0x4ac),
 ('57c1a/05_3d(1) status',   0x4b4),
 ('57c1a/05_3d(2) status',   0x4b8),
 ('57c1a withdraw status',   0x4bc),
 ('575f8/05_03 gridbase',    0x18),
 ('575f8/05_3d net-ptr',     0x484),
 ('63584 val err 0xe/0x1d',  0x7d4),
 ('63584 val err 0xf',       0x7d8),
 ('63584 val err 0x11',      0x7dc),
 ('63584/05_41 status',      0x7e0),
 ('636c2/05_05 status',      0x7e4),
 ('636c2 err e8',            0x7e8),
 ('636c2/05_04 status',      0x7ec),
 ('636c2 err f0',            0x7f0),
 ('636c2 err f4',            0x7f4),
]

def resolve(V):
    slot_fo = SL_FO + V
    if slot_fo+4 > len(data): return None
    W = struct.unpack_from('<I', data, slot_fo)[0]
    return slot_fo, W

def kr_at(fo, maxlen=64):
    # read until 0 byte, decode euc-kr
    end = fo
    while end < len(data) and data[end] != 0 and end-fo < maxlen:
        end += 1
    b = data[fo:end]
    try:
        return b.decode('euc-kr')
    except Exception:
        return repr(b)

for label, V in slots:
    r = resolve(V)
    if not r:
        print(f"{V:#06x} {label}: OUT OF RANGE"); continue
    slot_fo, W = r
    # try two interpretations:
    # (a) W is a direct pointer: string fo = W + DELTA
    # (b) W is a pointer to a pointer (global var)
    tgt_a = W + DELTA
    sA = kr_at(tgt_a) if 0 <= tgt_a < len(data) else 'oob'
    # (b) deref: *(W+DELTA) is another word -> +DELTA again
    line = f"V={V:#06x} slot@{slot_fo:#x} W={W:#010x}  strA@{tgt_a:#x}='{sA}'"
    # also if W points into data region as ptr-to-ptr
    if 0 <= tgt_a+0 < len(data)-4:
        W2 = struct.unpack_from('<I', data, tgt_a)[0]
        tgt_b = W2 + DELTA
        if 0 <= tgt_b < len(data):
            sB = kr_at(tgt_b)
            line += f"  | derefB@{tgt_b:#x}='{sB}'"
    print(label)
    print("   ", line)
