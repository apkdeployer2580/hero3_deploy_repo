import struct
f=open('d:/tmp/client.bin','rb').read()
GB=0x100000
def u16(o): return struct.unpack_from('<H',f,o)[0]
def u32(o): return struct.unpack_from('<I',f,o)[0]

# A) any data word anywhere pointing into 0x14a6aa..0x14a790 (raw, thumb, or reloc-encoded)
lo,hi=0x14a6aa,0x14a790
hits=[]
for o in range(0,len(f)-4):
    w=u32(o)
    for cand,tag in ((w,'raw'),(w&~1,'thumb') if w&1 else (None,None),
                     ((w+0xeec+GB)&0xffffffff,'reloc')):
        if cand is None: continue
        if lo<=cand<hi:
            hits.append((o,w,tag,cand))
print('data-word refs into region:',len(hits))
for h in hits[:40]: print('  fo %06x word=%08x (%s) -> %06x'%h)

# B) find writes to [sp,#0x100] and [sp,#0x118] inside the HUD fn
print()
for o in range(0x49b30,0x4a790,2):
    h=u16(o)
    # str rN,[sp,#imm8*4]  = 1001 0 Rd imm8
    if (h&0xf800)==0x9000:
        imm=(h&0xff)*4; rd=(h>>8)&7
        if imm in (0x100,0x114,0x118):
            print('STR r%d,[sp,#0x%x] @ fo %06x guest %06x'%(rd,imm,o,o+GB))
