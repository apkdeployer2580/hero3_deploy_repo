"""Headless regression: invisible arrivals and skill 3 freezing with facing zero.

Run after building wie_probe: python tools_re/tower_arrival_selftest.py
Only isolated probe memory is changed; no device or save database is used.
"""

import argparse
from pathlib import Path
import re
import struct
import subprocess


ROOT = Path(__file__).resolve().parents[1]
CHAR = 0x1503A88
BOOT_KEYS = (
    "260:OK,400:OK,460:OK,520:OK,580:OK,640:OK,700:HASH,760:OK,"
    "820:HASH,880:OK,940:HASH,1000:OK,1060:HASH,1120:OK,1180:HASH,"
    "1240:OK,1300:HASH,1360:OK,1420:HASH,1480:OK,1540:HASH,1600:OK"
)


def poke(frame, offset, data):
    return f"{frame}:{CHAR + offset:x}:{data}"


def read_char(folder, frame):
    data = (folder / f"mem_{frame:05d}_0x{CHAR:08x}.bin").read_bytes()
    assert len(data) == 0xA600, "incomplete character snapshot"
    return data


def word(data, offset, fmt="<H"):
    return struct.unpack_from(fmt, data, offset)[0]


def position(log, frame):
    match = re.search(
        rf"\[player\] f={frame} .*?pos=\(Some\((\d+)\),Some\((\d+)\)\)", log
    )
    assert match, f"missing position at frame {frame}"
    return tuple(map(int, match.groups()))


def run(args, name, scene, facing, death=False):
    folder = args.out / name
    folder.mkdir(parents=True, exist_ok=True)
    frames = [1715, 1725, 1750, 1850, 1900]
    warps = f"1700:{scene}:{facing}"
    keys = BOOT_KEYS + ",1720:NUM1,1860:DOWN:30"
    pokes = [
        poke(1690, 0x9F83, "01"),
        poke(1690, 0x252, "01"),
        poke(1690, 0x25C, "03"),
        poke(1690, 0x26C, "02"),
        poke(1690, 0x1B8, "01"),
    ]
    if death:
        frames += [1962, 2015, 2150]
        warps += f",2000:0:{facing}"
        keys += ",2100:DOWN:20"
        pokes += [poke(1960, 0xF2, "0000")]
        pokes += [
            poke(1990, 0xF2, "a700"),
            poke(1990, 0xF8, "e8030000"),
            poke(1990, 0x9BC8, "0000"),
            poke(1990, 0xA1F8, "0000"),
            poke(1990, 0x9BB5, "00"),
            poke(1990, 0x9BB6, "00"),
        ]
    command = [
        str(args.probe), "--zip", str(args.zip), "--out", str(folder),
        "--frames", str(max(frames) + 1), "--keys", keys,
        "--host-warp", warps, "--tower-species", f"{scene}:-,-,-",
        "--dump", ",".join(map(str, frames)),
        "--player", ",".join(map(str, frames)),
        "--dump-mem", f"{CHAR:x}:a600", "--poke", ",".join(pokes),
    ]
    result = subprocess.run(command, capture_output=True)
    log = (result.stdout + result.stderr).decode("utf-8", errors="replace")
    (folder / "probe.log").write_text(log, encoding="utf-8")
    assert result.returncode == 0 and "[probe] done" in log, folder
    assert "tick error" not in log.lower(), folder
    before = read_char(folder, 1715)
    casting = read_char(folder, 1725)
    after = read_char(folder, 1850)
    assert word(before, 0x9E30) == scene
    assert before[0x9BB4] == facing
    assert word(casting, 0xF8, "<I") < word(before, 0xF8, "<I"), "skill not cast"
    assert casting[0x9BB6] == 2, "skill animation did not start"
    if facing == 0:
        assert after[0x9BB6] == 2, "old freeze no longer reproduced"
        assert read_char(folder, 1900)[0x9BB6] == 2, "direction input freed old freeze"
        assert position(log, 1850) == position(log, 1900)
    else:
        assert after[0x9BB6] == 0, "skill did not finish"
        assert position(log, 1850) != position(log, 1900), "movement still blocked"
    if death:
        dying = read_char(folder, 1962)
        returned = read_char(folder, 2015)
        assert word(dying, 0xF2) == 0 and dying[0x9BB6] == 16
        assert word(returned, 0x9E30) == 0
        assert word(returned, 0xF2) == 167
        assert returned[0x9BB4] == facing and returned[0x9BB6] == 0
        assert position(log, 2015) != position(log, 2150), "return movement blocked"
    print(f"PASS {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, default=ROOT / "target/release/wie_probe.exe")
    parser.add_argument("--zip", type=Path, default=ROOT / "android_app/app/src/main/assets/hero3_fixed.zip")
    parser.add_argument("--out", type=Path, default=ROOT / "target/tower-arrival-regression")
    args = parser.parse_args()
    source = (ROOT / "wie_android/src/guardian.rs").read_text(encoding="utf-8")
    facing = int(re.search(r"pub const WARP_ENTRY: u8 = (\d+);", source)[1])
    assert 1 <= facing <= 4, "arrival must have a valid sprite direction"
    run(args, "old-freeze", 243, 0)
    run(args, "floor-and-death-return", 243, facing, death=True)
    run(args, "next-room", 244, facing)
    run(args, "boss-room", 246, facing)


if __name__ == "__main__":
    main()
