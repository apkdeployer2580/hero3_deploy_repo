"""LGT binary.mod (ELF) Thumb 디스어셈블러 — h3dis.py 의 LGT 판.

KTF client.bin 은 raw 바이너리(게스트 = fo + 0x100000)지만 LGT binary.mod 는
ELF 다: .text 가 파일오프셋 0x34, vaddr 0x1000 에 로드된다 (readelf -S 로 확인).
그래서 여기서는 **vaddr 로 지정**한다 — 프로브 프로파일러([prof] pc=...)와
binary_patches.toml 의 pc 가 쓰는 주소가 바로 이 vaddr 다.

usage:
    python tools_re/lgtdis.py <binary.mod> <시작va> <끝va> [출력파일]

예 (블리터 확인):
    python tools_re/lgtdis.py d:/tmp/binary_lgt.mod 0x2dd7c 0x2e8c4
"""
import sys

import capstone

TEXT_OFF, TEXT_VA = 0x34, 0x1000


def main(argv):
    elf = open(argv[1], 'rb').read()
    va0, va1 = int(argv[2], 16), int(argv[3], 16)
    out = open(argv[4], 'w') if len(argv) > 4 else sys.stdout

    md = capstone.Cs(capstone.CS_ARCH_ARM, capstone.CS_MODE_THUMB)
    md.skipdata = True

    fo = va0 - TEXT_VA + TEXT_OFF
    code = elf[fo:fo + (va1 - va0)]
    for i in md.disasm(code, va0):
        b = ' '.join(f'{x:02x}' for x in i.bytes)
        print(f'{i.address:08x}  {b:<12} {i.mnemonic:<8} {i.op_str}', file=out)


if __name__ == '__main__':
    main(sys.argv)
