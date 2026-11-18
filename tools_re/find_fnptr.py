import struct, sys
data = open(r'd:\tmp\client.bin', 'rb').read()
n = len(data)
RELOC_DELTA = 0xeec

# render fn file offsets we want to locate in fnptr tables
fns = {
    0x729d4: 'render_conn_terminated(998)',
    0x72b18: 'render_WAIT(9a8)',
    0x72cbc: 'render_msg(a3ac)',
}
if len(sys.argv) > 1:
    fns = {int(a,16): 'arg' for a in sys.argv[1:]}

for fo, label in fns.items():
    guest = 0x100000 + fo
    cands = {
        'fo-delta (fo-0xeec)': fo - RELOC_DELTA,
        'fo-delta|1': (fo - RELOC_DELTA) | 1,
        'guest|1': guest | 1,
        'guest': guest,
        'fo': fo,
        'fo|1': fo | 1,
    }
    print("=== %s  fo=%#x guest=%#x ===" % (label, fo, guest))
    for cname, cval in cands.items():
        pat = struct.pack('<I', cval & 0xffffffff)
        start = 0; hits = []
        while True:
            i = data.find(pat, start)
            if i < 0: break
            if i % 4 == 0: hits.append(i)
            start = i + 1
        if hits:
            print("   %-22s = %#010x  @ %s" % (cname, cval & 0xffffffff, ' '.join('%#x'%h for h in hits)))
