import struct, sys
data = open(r'd:\tmp\client.bin', 'rb').read()
RELOC_DELTA = 0xeec
SL_FO = 0xb2c40

def cstr(fo, maxlen=48):
    end = data.find(b'\x00', fo, fo+maxlen)
    if end < 0: return None
    raw = data[fo:end]
    if not raw: return None
    try: return raw.decode('euc-kr')
    except: return None

start = int(sys.argv[1],16)
end = int(sys.argv[2],16)
for a in range(start, end, 4):
    w = struct.unpack_from('<I', data, a)[0]
    note = ''
    # try as code fnptr: (fo-0xeec)|1
    fo = (w & ~1) + RELOC_DELTA
    if 0 < fo < len(data) and (w & 1):
        note = '-> fn fo=%#x (guest %#x)' % (fo, 0x100000+fo)
    else:
        # try as data ptr: fo-0xeec -> string
        t = w + RELOC_DELTA
        if 0 < t < len(data):
            s = cstr(t)
            if s: note = '-> data %#x %r' % (t, s)
    print('%#08x (sl+%#x): %#010x  %s' % (a, a-SL_FO, w, note))
