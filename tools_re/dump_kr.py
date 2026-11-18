import re, sys, io
data = open(r'd:\tmp\client.bin', 'rb').read()

start = int(sys.argv[1], 16)
end = int(sys.argv[2], 16)
outpath = sys.argv[3]

# runs of: euckr pair | printable ascii  -> decode as euc-kr
pat = re.compile(rb'(?:[\xa1-\xfe][\xa1-\xfe]|[\x20-\x7e])+')
res = []
for m in pat.finditer(data[start:end]):
    b = m.group()
    if len(b) < 3:
        continue
    try:
        s = b.decode('euc-kr')
    except Exception:
        s = b.decode('latin1')
    if s.strip() == '':
        continue
    res.append((start + m.start(), len(b), s))

with io.open(outpath, 'w', encoding='utf-8') as f:
    for o, n, s in res:
        f.write("0x%06x [%3d] %s\n" % (o, n, s))
print("wrote", outpath, len(res), "runs")
