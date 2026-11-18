"""LGT binary.mod: 알려진 정적 주소를 참조하는 **코드 위치**를 찾는다.

왜 이게 필요한가 — LGT 는 KTF 와 완전히 다른 컴파일이라 코드 주소가 하나도
이식되지 않는다(MIGRATION_LGT.md §1). 하지만 **데이터 주소는 확보돼 있다**:
CHAR 레코드가 `.bss 0x1503a88` 정적이고 구조체 오프셋은 KTF와 거의 같다(§2.7).
LGT 는 절대 주소 모델이라 정적을 만지는 코드는 그 주소를 **리터럴 풀에 그대로
박아 둔다**. 그래서 "이 필드를 읽는 함수가 어디냐" 는 리터럴 검색으로 답이 난다.

즉 이 스크립트는 KTF 때의 "게스트 코드 주소" 문제를 데이터 주소 문제로 바꾼다.
§4 의 남은 항목(HUD 스킵, quickbind, EXP 배수, 저장 체인) 전부의 공통 진입로다.

usage:
    # CHAR 레코드 안을 가리키는 모든 리터럴
    python tools_re/lgt_statics_xref.py <binary.mod>
    # 특정 필드(CHAR+0x252 = 퀵바 type[0]) 를 가리키는 것만
    python tools_re/lgt_statics_xref.py <binary.mod> --char-off 0x252
    # 임의 주소 범위
    python tools_re/lgt_statics_xref.py <binary.mod> --range 0x150d914 0x150d924
    # 상수 즉시값(immediate) 검색 — movs/cmp/sub rX,#imm8 (HUD 폭 0xaa 같은 것)
    python tools_re/lgt_statics_xref.py <binary.mod> --imm8 0xaa

출력의 `va` = ELF vaddr (lgtdis.py / binary_patches.toml / 프로브 [prof] 와 같은 축).
리터럴은 Thumb `ldr rX,[pc,#imm]` 이 읽는 4바이트 정렬 워드이므로, 보고되는
`loader va` 는 그 워드를 실제로 pc-relative 로 읽는 명령의 주소다(있을 때).
"""
import argparse
import struct
import sys

TEXT_OFF, TEXT_VA = 0x34, 0x1000
# CHAR = .bss 0x1503a88, 레코드 길이는 앱이 읽는 최대 오프셋(+0xa0c0)을 넉넉히 덮는 값.
CHAR_BASE = 0x1503A88
CHAR_LEN = 0xA800


def text_range(elf):
    """(.text 파일오프셋, vaddr, 크기) — 섹션 헤더를 읽지 않고 알려진 배치를 쓴다.

    리비전 둘(01.00.07 = 0x5db85, 01.00.04 = 0x5dcc9)의 .text 크기가 다르므로
    파일 끝까지로 잡고 넘치는 부분은 호출자가 무시한다.
    """
    return TEXT_OFF, TEXT_VA, len(elf) - TEXT_OFF


def find_words(elf, lo, hi):
    """[lo, hi) 안의 값을 갖는 4바이트 정렬 워드를 (va, value) 로 전부 돌려준다."""
    fo0, va0, size = text_range(elf)
    hits = []
    for off in range(fo0, fo0 + size - 3, 4):
        (v,) = struct.unpack_from('<I', elf, off)
        if lo <= v < hi:
            hits.append((off - fo0 + va0, v))
    return hits


def find_loaders(elf, target_va):
    """`ldr rX,[pc,#imm]` 로 `target_va` 워드를 읽는 명령들의 va.

    Thumb-1 T1: 0100 1ddd iiiiiiii, 유효주소 = (pc & ~3) + 4 + imm*4, pc = 명령 va.
    """
    fo0, va0, size = text_range(elf)
    out = []
    for off in range(fo0, fo0 + size - 1, 2):
        (h,) = struct.unpack_from('<H', elf, off)
        if h & 0xF800 != 0x4800:
            continue
        va = off - fo0 + va0
        if ((va & ~3) + 4 + (h & 0xFF) * 4) == target_va:
            out.append(va)
    return out


def find_imm8(elf, imm, mnemonics):
    """movs/cmp/adds/subs rX,#imm8 의 위치. Thumb-1 형식 001 op rrr iiiiiiii."""
    ops = {'movs': 0x2000, 'cmp': 0x2800, 'adds': 0x3000, 'subs': 0x3800}
    want = [ops[m] for m in mnemonics]
    fo0, va0, size = text_range(elf)
    out = []
    for off in range(fo0, fo0 + size - 1, 2):
        (h,) = struct.unpack_from('<H', elf, off)
        for base in want:
            if (h & 0xF800) == base and (h & 0xFF) == imm:
                out.append((off - fo0 + va0, [k for k, v in ops.items() if v == base][0], (h >> 8) & 7))
    return out


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('binary')
    ap.add_argument('--char-off', type=lambda s: int(s, 0), help='CHAR+off 하나만')
    ap.add_argument('--range', nargs=2, type=lambda s: int(s, 0), help='임의 [lo, hi)')
    ap.add_argument('--imm8', type=lambda s: int(s, 0), help='즉시값 검색')
    ap.add_argument('--loaders', action='store_true', help='리터럴을 읽는 ldr 명령까지')
    args = ap.parse_args(argv[1:])

    elf = open(args.binary, 'rb').read()

    if args.imm8 is not None:
        hits = find_imm8(elf, args.imm8, ('movs', 'cmp', 'adds', 'subs'))
        for va, mn, reg in hits:
            print(f'{va:08x}  {mn} r{reg}, #{args.imm8:#x}')
        print(f'-- {len(hits)} immediate(s) == {args.imm8:#x}', file=sys.stderr)
        return

    if args.range:
        lo, hi = args.range
    elif args.char_off is not None:
        lo = CHAR_BASE + args.char_off
        hi = lo + 4
    else:
        lo, hi = CHAR_BASE, CHAR_BASE + CHAR_LEN

    for va, v in find_words(elf, lo, hi):
        tag = f'CHAR+{v - CHAR_BASE:#x}' if CHAR_BASE <= v < CHAR_BASE + CHAR_LEN else f'{v:#x}'
        line = f'literal @{va:08x} = {v:#x}  ({tag})'
        if args.loaders:
            ldrs = find_loaders(elf, va)
            line += '  <- ' + (' '.join(f'{x:#x}' for x in ldrs) if ldrs else 'no pc-rel loader')
        print(line)


if __name__ == '__main__':
    main(sys.argv)
