import struct
data = open(r"d:/tmp/client.bin","rb").read()
def read_kr(fo, maxlen=80):
    b=data[fo:fo+maxlen]; e=b.find(b"\x00")
    if e>=0: b=b[:e]
    try: return b.decode("euc-kr")
    except: return repr(b)
# decode the cat1/cat4 slot strings properly
for imm,fo in [("0x48c",0x0a9600),("0x490",0x0a9618),("0x498",0x0a9638)]:
    print(imm, "=>", read_kr(fo))
print("----- search feature strings -----")
targets = ["서버 응답","기다리","저장중","돈 내리기","캐시템","오브 결합","창고가 가득","무작동","연결이 종료","접속할 수 없"]
for t in targets:
    tb=t.encode("euc-kr")
    i=0
    hits=[]
    while True:
        j=data.find(tb,i)
        if j<0: break
        hits.append(j); i=j+1
    for h in hits:
        # guest addr = fo + 0x100000; also print the value a reloc word would hold (fo-0xeec)
        print(f"  {t!r:20s} fo=0x{h:06x} guest=0x{h+0x100000:06x} relocword=0x{(h-0xeec)&0xffffffff:08x} => {read_kr(h)!r}")
