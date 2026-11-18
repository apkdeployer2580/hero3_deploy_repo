import struct, sys
data = open(r'd:\tmp\client.bin', 'rb').read()
SL_FO = 0xb2c40

# Which sl-relative literal reaches a given data slot?
for label, fo in [('readSocket_err_ptr', 0xb36a8), ('BILLSOCK_ptr', 0xb369c),
                  ('IP1_ptr', 0xb3690), ('IP2_ptr', 0xb36a0), ('MSG5min_ptr', 0xb3670)]:
    print("%-20s data@%#x -> sl+%#x" % (label, fo, fo - SL_FO))

target = int(sys.argv[1], 16) if len(sys.argv) > 1 else 0xa68
print("\nsearching literal %#x ..." % target)
pat = struct.pack('<I', target)
start = 0
while True:
    i = data.find(pat, start)
    if i < 0:
        break
    if i % 4 == 0:
        print("  literal word @ %#08x" % i)
    start = i + 1
