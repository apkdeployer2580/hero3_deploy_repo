import struct, io
data = open(r'd:\tmp\client.bin','rb').read()
SL_FO=0xb2c40; DELTA=0xeec
slots=[('05_14 (57568) post-send',0x4a8),('05_3f (575f8) post-send',0x4ac),
('05_3d#1 (57c1a)',0x4b4),('05_3d#2 (57c1a)',0x4b8),('05_40 withdraw (57c1a)',0x4bc),
('05_41 err type0xe',0x7d4),('05_41 err type0xf',0x7d8),('05_41 err type0x11',0x7dc),
('05_41 (63584) post-send',0x7e0),('05_05 (636c2) post-send',0x7e4),
('05_05 err',0x7e8),('05_04 (636c2) post-send',0x7ec),('05_04 err full',0x7f0),
('05_04 err irreversible',0x7f4)]
def kr(fo,ml=90):
    e=fo
    while e<len(data) and data[e]!=0 and e-fo<ml: e+=1
    try: return data[fo:e].decode('euc-kr')
    except: return data[fo:e].decode('latin1')
out=io.open(r'd:\tmp\wie_app\tools_re\dis\slots_clean.txt','w',encoding='utf-8')
for lab,V in slots:
    W=struct.unpack_from('<I',data,SL_FO+V)[0]
    s=kr(W+DELTA)
    out.write(u"%-28s V=%#06x -> str@%#x : %s\n"%(lab,V,W+DELTA,s))
out.close()
print("ok")
