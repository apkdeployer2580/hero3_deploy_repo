import struct
data = open(r'd:\tmp\client.bin', 'rb').read()
SL_FO = 0xb2c40

# sl offsets used as the post-flush dialog arg (arg0 to 0x175b98) at each send site
sites = {
    'recovery 04/3d @0x70f00':      0x968,
    'money_down 05/3e @0x745ca':    0x9b8,
    'cash_buy 05/42 @0x74e74':      0x9bc,
    'orb_combine 05/06 @0x74fa4':   0x9c0,
    'op41 @0x6367e':                0x7e0,
    'op05 @0x63762':                0x7e4,
    'op04 @0x63984/0x63ac2/0x741c4':0x7ec,
    'op3f @0x6483c':                0x4ac,
    'op07-warn @0x70e9a(err)':      0x964,
    'wh_full? 0x4b4':               0x4b4,
    'saveslot G_saves 0x4a4':       0x4a4,
    'op3f-arr 0x4ac dup':           0x4ac,
}
def decstr(fo):
    end=data.find(b'\x00',fo)
    raw=data[fo:end]
    try: return raw.decode('euc-kr')
    except: return raw.decode('latin1','replace')

for label, off in sites.items():
    wfo = SL_FO + off
    reloc = struct.unpack_from('<I', data, wfo)[0]
    sfo = reloc + 0xeec
    s = decstr(sfo) if 0 <= sfo < len(data) else '??'
    print("%-34s sl+%#05x  reloc=%#08x -> str_fo=%#07x  '%s'" % (label, off, reloc, sfo, s[:40]))
