"""Name Hero3's network requests by correlating send sites with UI strings.

Each online operation shows a status message ("돈 내리기 중입니다.", "캐시템
구입중입니다.", ...) from the same function that sends the request. So:

  string file-offset
    -> relocated pointer word value (offset - 0xeec)
    -> the .data slot holding that word
    -> sl-relative offset (slot - 0x b2c40)
    -> code sites loading that sl offset (literal pool `ldr r3,[pc]; add r3,sl`)
    -> nearest flush() site  ==  the request that operation sends

This gives human names to entries in packet_catalog.txt without needing to
trigger every feature on the device.
"""
import struct
import sys

DATA = open(r'd:\tmp\client.bin', 'rb').read()
RELOC_DELTA = 0xEEC
SL_FO = 0xB2C40
FLUSH = 0x7E890


def s23(v):
    return v - (1 << 23) if v & (1 << 22) else v


def bl_target(fo):
    hw1 = struct.unpack_from('<H', DATA, fo)[0]
    hw2 = struct.unpack_from('<H', DATA, fo + 2)[0]
    if (hw1 & 0xF800) != 0xF000 or (hw2 & 0xF800) != 0xF800:
        return None
    return fo + 4 + s23(((hw1 & 0x7FF) << 12) | ((hw2 & 0x7FF) << 1))


FLUSH_SITES = sorted(i for i in range(0, len(DATA) - 3, 2) if bl_target(i) == FLUSH)

# Korean status strings that accompany a network operation (file offsets from the
# EUC-KR string dump of the 0xaa700..0xab200 region).
TARGETS = {
    0x0AA740: '슬롯 정보 서버에 저장중입니다',
    0x0AA880: '서버 응답 기다리는중',
    0x0AA898: '금액을 설정하세요',
    0x0AA8AC: '삭제하였습니다',
    0x0AA8BC: '돈 내리기 중입니다',
    0x0AA8D0: '캐시템 구입중입니다',
    0x0AA910: '오브 결합중입니다',
    0x0A9FE4: '싱글 창고가 가득찼습니다',
    0x0AA000: '인벤토리로 내리면 다시 네트워크로 올릴 수 없습니다',
    0x0AA800: '서버와 접속할 수 없습니다',
    0x0AA830: '서버와 연결이 종료되었습니다',
}


def data_slots_for(str_fo):
    """Find .data words holding the relocated pointer to this string."""
    want = str_fo - RELOC_DELTA
    pat = struct.pack('<I', want)
    out = []
    start = 0
    while True:
        i = DATA.find(pat, start)
        if i < 0:
            break
        if i % 4 == 0:
            out.append(i)
        start = i + 1
    return out


def literal_sites_for(sl_off):
    """Find literal-pool words equal to this sl offset (how code addresses it)."""
    pat = struct.pack('<I', sl_off)
    out = []
    start = 0
    while True:
        i = DATA.find(pat, start)
        if i < 0:
            break
        if i % 4 == 0:
            out.append(i)
        start = i + 1
    return out


def nearest_flush(fo, window=0x600):
    """Flush sites within `window` bytes after `fo` (same function, usually)."""
    return [f for f in FLUSH_SITES if 0 <= f - fo < window]


def main():
    print('Hero3 network request naming (string -> send site)')
    print('=' * 72)
    for str_fo, label in sorted(TARGETS.items()):
        slots = data_slots_for(str_fo)
        print(f'\n{label}   (string @ {str_fo:#08x})')
        if not slots:
            print('   no .data pointer found (string may be used via a different mechanism)')
            continue
        for slot in slots:
            sl_off = slot - SL_FO
            print(f'   data slot {slot:#08x} = sl+{sl_off:#x}')
            lits = literal_sites_for(sl_off & 0xFFFFFFFF)
            # literal pools sit just after the code that uses them
            near = []
            for lit in lits:
                for f in nearest_flush(max(0, lit - 0x600), 0xC00):
                    if abs(f - lit) < 0x600:
                        near.append((lit, f))
            if not near:
                print('      (no flush site within range of any literal use)')
            for lit, f in sorted(set(near)):
                print(f'      literal @{lit:#08x}  ->  flush @{f:#08x}')


if __name__ == '__main__':
    sys.stdout.reconfigure(encoding='utf-8')
    main()
