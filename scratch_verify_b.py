import struct
f=open('d:/tmp/client.bin','rb').read()
GB=0x100000

def u16(o): return struct.unpack_from('<H',f,o)[0]
def u32(o): return struct.unpack_from('<I',f,o)[0]

# 1) resolve pc-relative literals in path B prologue
# 0x4a6ac : ldr r1,[pc,#0x1e8]
for site,imm in ((0x4a6ac,0x1e8),(0x4a6c4,0x1d4),(0x4a704,0x198),(0x4a716,0x18c),
                 (0x49d78,0x1c0),(0x49db8,0x16c),(0x49dc6,0x168)):
    pc=(site+GB+4)&~3
    lit=pc+imm
    fo=lit-GB
    raw=u32(fo)
    print('site %06x  lit@guest %06x (fo %06x) raw=%08x  raw+0xeec=%08x -> guest %08x'
          %(site,lit,fo,raw,(raw+0xeec)&0xffffffff,((raw+0xeec)&0xffffffff)+GB))

print()
# 2) find every BL / B(T2) / Bcc that targets 0x14a6aa, and any branch landing inside 0x14a6aa..0x14a780
targets={}
lo,hi=0x14a6aa,0x14a790
for o in range(0,len(f)-4,2):
    h1=u16(o)
    g=o+GB
    t=None
    if (h1&0xf800)==0xf000:
        h2=u16(o+2)
        if (h2&0xf800) in (0xf800,0xe800):   # BL / BLX
            off=((h1&0x7ff)<<12)|((h2&0x7ff)<<1)
            if off&0x400000: off-=0x800000
            t=g+4+off
            kind='BL'
        else: continue
    elif (h1&0xf800)==0xe000:                # B T2
        off=(h1&0x7ff)<<1
        if off&0x800: off-=0x1000
        t=g+4+off; kind='B'
    elif (h1&0xf000)==0xd000 and (h1&0x0f00)!=0x0f00 and (h1&0x0f00)!=0x0e00:
        off=(h1&0xff)<<1
        if off&0x100: off-=0x200
        t=g+4+off; kind='Bcc'
    else: continue
    if t is not None and lo<=t<hi:
        targets.setdefault(t,[]).append((g,kind))

for t in sorted(targets):
    print('target %06x  <- %s'%(t,', '.join('%06x(%s)'%(a,k) for a,k in targets[t])))
