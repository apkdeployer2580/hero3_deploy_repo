import struct
data = open(r'd:\tmp\client.bin','rb').read()
SL = 0xb2c40  # file offset of RWPI static base
def rd32(fo): return struct.unpack_from('<I', data, fo)[0]
def kr(fo, n=48):
    out=b''
    i=fo
    while i < len(data) and len(out) < n:
        b=data[i]
        if b==0: break
        out+=bytes([b]); i+=1
    return out
offs = [0x18,0x16c,0x480,0x484,0x488,0x48c,0x490,0x494,0x498,0x49c,0x444]
for O in offs:
    slot_fo = SL + O
    word = rd32(slot_fo)
    tgt = (word + 0xeec) & 0xffffffff
    tgt_fo = tgt  # target is a file offset already (guest = fo+0x100000)
    raw = data[tgt_fo:tgt_fo+16].hex() if 0 <= tgt_fo < len(data) else 'OOB'
    s = kr(tgt_fo) if 0 <= tgt_fo < len(data) else b''
    try:
        dec = s.decode('euc-kr')
    except Exception:
        dec = repr(s)
    print("sl+0x%03x  slot_fo=0x%06x  word=0x%08x  tgt_fo=0x%06x  bytes=%s  ascii=%r  euckr=%r" % (O, slot_fo, word, tgt_fo, raw, s, dec))
