"""Retune the game's SMAF scores onto the bundled MA-3 tone bank.

`wie_android/assets/hero3_ma3.sf3` carries the game's own Yamaha MA-3 tones, so a
program number in a score now selects a REAL preset instead of falling back (see the
80차 section of CLAUDE.md). These 236 bytes are the instrument choices that were
retuned once that became true: 14 of the 19 `res/snd/bgm*_mf` pick a different
program, and nothing else in the score changes - same length, same events, same tempo.

Provenance: the values are lifted from the player-supplied 「소리복원판」 APK
(`dist/hero3_lgt1.04_vanilla_0.210.0(소리복원판).apk`), which is where the MA-3 bank came
from too. They are a TASTE change and were measured to be one: preset coverage over all
19 scores is identical with and without them (36/38 either way), so this tool is
independent of the bank swap and can be reverted on its own.

⛔ Every write is verify-then-write: a byte that is neither the expected old value nor
the new one aborts the run. The scores are pristine game data and never change, so a
mismatch means the asset is not what this table was built against - not that the table
needs relaxing.

★ ORDER. `patch_asset.py --apply` rebuilds the zip from the pristine `.orig`, which
would drop these; `patch_item_prices.py` edits in place. So this runs LAST:

    python tools_re/patch_asset.py       <zip> --apply
    python tools_re/patch_item_prices.py <zip> --apply
    python tools_re/patch_bgm_ma3.py     <zip> --apply

Usage: python patch_bgm_ma3.py <zip> [--apply | --check]
  --check  exit 1 unless every byte is already the new value (the staleness gate)
  --apply  patch in place, after copying the current zip into asset_backup/
"""

import io
import os
import shutil
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import patch_asset as PA  # noqa: E402  (for `backup_paths` - one source of truth)

# basename -> [(offset, old program, new program)]
PATCHES = {
    "bgm3_mf": [
        (0x0089,  80,  25),
        (0x0230,  80,  25),
        (0x026e,  80,  25),
        (0x0d7d,  25,  24),
        (0x0df0,  15,  12),
        (0x0ee7,  15,  12),
        (0x0fcd,  15,  12),
        (0x12d3,  25,  24),
        (0x1340,  15,  12),
        (0x1437,  15,  12),
        (0x14c1,  25,  24),
        (0x1526,  15,  12),
        (0x1620,  15,  12),
        (0x1704,  15,  12),
        (0x1a27,  25,  24),
        (0x1a97,  15,  12),
        (0x1b34,  15,  12),
    ],
    "bgm4_mf": [
        (0x00cb,  15,  13),
        (0x014f,  15,  13),
    ],
    "bgm5_mf": [
        (0x09f9,  90,  91),
        (0x0d15,  90,  91),
        (0x0d79,  90,  91),
    ],
    "bgm7_mf": [
        (0x06fd,  86,  87),
        (0x0737,  86,  87),
        (0x0750,  86,  87),
        (0x077f,  86,  87),
        (0x07bb,  86,  87),
        (0x07d9,  86,  87),
        (0x0809,  86,  87),
        (0x0821,  86,  87),
        (0x082b,  86,  87),
        (0x0843,  86,  87),
        (0x086a,  86,  87),
        (0x0881,  86,  87),
        (0x08cd,  86,  87),
        (0x08eb,  86,  87),
        (0x0916,  86,  87),
        (0x0949,  86,  87),
        (0x097f,  86,  87),
        (0x09a6,  86,  87),
        (0x09e1,  86,  87),
        (0x09f9,  86,  87),
        (0x0a03,  86,  87),
        (0x0a26,  86,  87),
        (0x0a4e,  86,  87),
        (0x0a53,  86,  87),
        (0x0a9c,  86,  87),
        (0x0ad9,  86,  87),
        (0x0aed,  86,  87),
        (0x0b1f,  86,  87),
        (0x0b50,  86,  87),
        (0x0b6e,  86,  87),
        (0x0ba3,  86,  87),
        (0x0bb6,  86,  87),
        (0x0bc0,  86,  87),
        (0x0bdd,  86,  87),
        (0x0bff,  86,  87),
        (0x0c16,  86,  87),
        (0x0c7a,  86,  87),
        (0x0c8d,  86,  87),
        (0x0ca3,  86,  87),
        (0x0ccb,  86,  87),
        (0x0ced,  86,  87),
        (0x0cf2,  86,  87),
        (0x0d1f,  86,  87),
        (0x0d3c,  86,  87),
        (0x0d41,  86,  87),
        (0x0d5c,  86,  87),
        (0x0d79,  86,  87),
        (0x0d7e,  86,  87),
        (0x0dc7,  86,  87),
        (0x0e01,  86,  87),
        (0x0e15,  86,  87),
        (0x0e48,  86,  87),
        (0x0e83,  86,  87),
        (0x0e9c,  86,  87),
        (0x0ecb,  86,  87),
        (0x0ee7,  86,  87),
        (0x0eec,  86,  87),
        (0x0f03,  86,  87),
        (0x0f2e,  86,  87),
        (0x0f40,  86,  87),
        (0x0f8f,  86,  87),
        (0x0fad,  86,  87),
        (0x0fd4,  86,  87),
        (0x100b,  86,  87),
        (0x1042,  86,  87),
        (0x105f,  86,  87),
        (0x109e,  86,  87),
        (0x10b0,  86,  87),
        (0x10b5,  86,  87),
        (0x10d7,  86,  87),
        (0x10f9,  86,  87),
        (0x10fe,  86,  87),
        (0x1147,  86,  87),
        (0x1184,  86,  87),
        (0x1198,  86,  87),
        (0x11cf,  86,  87),
        (0x11ff,  86,  87),
        (0x1218,  86,  87),
        (0x124c,  86,  87),
        (0x1263,  86,  87),
        (0x1268,  86,  87),
        (0x1289,  86,  87),
        (0x12aa,  86,  87),
        (0x12bc,  86,  87),
        (0x131f,  86,  87),
        (0x1336,  86,  87),
        (0x1348,  86,  87),
        (0x1370,  86,  87),
        (0x1392,  86,  87),
        (0x1397,  86,  87),
        (0x13bf,  86,  87),
        (0x13d6,  86,  87),
        (0x13db,  86,  87),
        (0x13f7,  86,  87),
        (0x140e,  86,  87),
        (0x1413,  86,  87),
    ],
    "bgm8_mf": [
        (0x00aa,  80,  26),
        (0x018d,  80,  26),
        (0x01b8,  80,  26),
    ],
    "bgm10_mf": [
        (0x00cb,  78,  23),
        (0x0268,  78,  23),
    ],
    "bgm11_mf": [
        (0x00cb,  89,  30),
        (0x01f5,  89,  30),
    ],
    "bgm12_mf": [
        (0x00aa,  11,   7),
        (0x00cb,  38,  16),
        (0x0120,  11,   7),
        (0x0156,  38,  16),
        (0x0169,  38,  16),
    ],
    "bgm13_mf": [
        (0x00aa,  15,  14),
        (0x0153,  15,  14),
        (0x016f,  15,  14),
        (0x01ad,  15,  14),
        (0x045c,  27,  30),
        (0x0634,  27,  30),
        (0x06ac,  27,  30),
        (0x0a08,  27,  30),
        (0x0bc8,  27,  30),
        (0x0c40,  27,  30),
        (0x0f6c,  27,  30),
        (0x1144,  27,  30),
        (0x11bc,  27,  30),
        (0x1518,  27,  30),
        (0x16d8,  27,  30),
        (0x1750,  27,  30),
        (0x1a7c,  27,  30),
        (0x1c54,  27,  30),
        (0x1ccc,  27,  30),
        (0x2028,  27,  30),
        (0x21ec,  27,  30),
        (0x2266,  27,  30),
    ],
    "bgm14_mf": [
        (0x00aa,  58,  22),
        (0x00cb,  11,  12),
        (0x0350,  58,  22),
        (0x0394,  58,  22),
        (0x03a7,  11,  12),
        (0x03f1,  27,  29),
        (0x0435,  31,  33),
        (0x0439,  31,  33),
        (0x044e,  27,  29),
        (0x04cc,  27,  29),
        (0x04f2,  75,  74),
        (0x04f7,  75,  74),
        (0x04fc,  31,  33),
        (0x052d,  27,  29),
        (0x056a,  31,  33),
        (0x056e,  28,  32),
        (0x0595,  27,  29),
        (0x05a8,  31,  33),
        (0x05ac,  31,  33),
        (0x0606,  27,  29),
        (0x0696,  31,  33),
        (0x069b,  31,  33),
        (0x0756,  27,  29),
        (0x07c4,  31,  33),
        (0x07c8,  31,  33),
        (0x07e9,  27,  29),
        (0x08b8,  27,  29),
        (0x08ea,  75,  74),
        (0x08ee,  75,  74),
        (0x08f2,  31,  33),
        (0x092e,  27,  29),
        (0x098f,  31,  33),
        (0x0993,  28,  32),
        (0x09c9,  27,  29),
        (0x09eb,  31,  33),
        (0x09ef,  31,  33),
        (0x0a70,  27,  29),
        (0x0b56,  27,  29),
        (0x0b81,  75,  74),
        (0x0b86,  75,  74),
        (0x0ba7,  75,  74),
        (0x0bdf,  31,  33),
        (0x0c3c,  27,  29),
        (0x0c57,  75,  74),
        (0x0c8b,  27,  29),
        (0x0e8f,  21,  20),
        (0x0f2e,  21,  20),
        (0x0f81,  21,  20),
        (0x103e,  27,  29),
        (0x1117,  21,  20),
        (0x11b8,  27,  29),
    ],
    "bgm15_mf": [
        (0x021f,  15,  14),
        (0x03dc,  15,  14),
        (0x059b,  15,  14),
        (0x0782,  15,  14),
        (0x0981,  15,  14),
        (0x0b8c,  15,  14),
        (0x0de3,  15,  14),
        (0x0efa,  15,  14),
        (0x10b5,  15,  14),
    ],
    "bgm16_mf": [
        (0x0089,  49,  20),
        (0x010c,  43,  19),
        (0x01bd,  49,  20),
        (0x01d5,  49,  20),
        (0x01fb,  43,  19),
        (0x023d,  70,  69),
        (0x03de,  70,  69),
        (0x0406,  70,  69),
        (0x05b2,  70,  69),
        (0x0ca7,  70,  69),
        (0x0f99,  70,  69),
        (0x0fc1,  70,  69),
        (0x12d9,  70,  69),
    ],
    "bgm17_mf": [
        (0x0089, 114,  32),
        (0x00aa,  54,  21),
        (0x010d, 109,  31),
        (0x014a, 114,  32),
        (0x0162,  54,  21),
        (0x017a, 114,  32),
        (0x01b3, 109,  31),
    ],
    "bgm18_mf": [
        (0x010b,   8,   3),
        (0x012c,   6,   1),
        (0x01cc,   8,   3),
        (0x0258,   6,   1),
    ],
}


def _members(zbytes):
    z = zipfile.ZipFile(io.BytesIO(zbytes))
    jars = [n for n in z.namelist() if n.lower().endswith(".jar")]
    if len(jars) != 1:
        raise SystemExit(f"expected exactly one jar, got {jars}")
    jb = z.read(jars[0])
    jz = zipfile.ZipFile(io.BytesIO(jb))
    found = {}
    for name in jz.namelist():
        base = name.rsplit("/", 1)[-1]
        if base in PATCHES:
            found[base] = name
    missing = set(PATCHES) - set(found)
    if missing:
        raise SystemExit(f"{sorted(missing)} not in the jar")
    return z, jars[0], jb, jz, found


def _state(blob, rows, who):
    """(applied, pristine) counts; aborts on a byte that is neither."""
    applied = pristine = 0
    for off, old, new in rows:
        if off >= len(blob):
            raise SystemExit(f"{who}: offset {off:#x} past end ({len(blob)} bytes)")
        b = blob[off]
        if b == new:
            applied += 1
        elif b == old:
            pristine += 1
        else:
            raise SystemExit(
                f"{who}: {off:#x} is {b}, expected {old} (pristine) or {new} (applied) "
                f"- this asset is not what the table was built against"
            )
    return applied, pristine


def _patched(blob, rows):
    out = bytearray(blob)
    for off, _old, new in rows:
        out[off] = new
    return bytes(out)


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    zpath = sys.argv[1]
    apply_it = "--apply" in sys.argv
    checking = "--check" in sys.argv
    if apply_it and checking:
        raise SystemExit("--apply and --check are exclusive")

    with open(zpath, "rb") as source:
        zbytes = source.read()
    _z, jar, jb, jz, found = _members(zbytes)

    total_applied = total_pristine = 0
    per_file = {}
    for base, rows in PATCHES.items():
        blob = jz.read(found[base])
        a, p = _state(blob, rows, base)
        per_file[base] = (a, p, len(rows))
        total_applied += a
        total_pristine += p

    total = sum(len(r) for r in PATCHES.values())
    for base in sorted(PATCHES, key=lambda n: int(n[3:-3])):
        a, p, n = per_file[base]
        mark = "applied" if a == n else ("pristine" if p == n else f"MIXED {a}/{n}")
        print(f"  {base:<10} {n:>3} byte(s)  {mark}")
    print(f"{zpath}: {total_applied}/{total} applied, {total_pristine} pristine")

    if checking:
        if total_applied != total:
            raise SystemExit(
                f"CHECK FAILED: {total - total_applied} of {total} MA-3 score bytes are "
                f"not applied - re-run with --apply"
            )
        print(f"CHECK OK: {zpath} carries the MA-3 score retune")
        return
    if not apply_it:
        print("(dry run - pass --apply to write)")
        return
    if total_applied == total:
        print("nothing to do")
        return

    bak = os.path.join(os.path.dirname(PA.backup_paths(zpath)[0]),
                       os.path.basename(zpath) + ".prebgm")
    os.makedirs(os.path.dirname(bak), exist_ok=True)
    if not os.path.exists(bak):
        shutil.copy2(zpath, bak)
        print(f"backup -> {bak}")

    newjar = PA.rebuild(jb, {found[b]: (lambda rows: lambda d: _patched(d, rows))(r)
                             for b, r in PATCHES.items()})
    newzip = PA.rebuild(zbytes, {jar: lambda _d: newjar})

    # Nothing but the scores may move.
    old_j = zipfile.ZipFile(io.BytesIO(jb))
    new_j = zipfile.ZipFile(io.BytesIO(newjar))
    changed = [n for n in old_j.namelist() if old_j.read(n) != new_j.read(n)]
    want = sorted(found[b] for b, (_, pristine, _) in per_file.items() if pristine)
    if sorted(changed) != want:
        raise SystemExit(f"refusing to write: jar members changed = {sorted(changed)}")
    print(f"  jar: {len(old_j.namelist())} members, only the {len(want)} scores differ")

    tmp = zpath + ".tmp"
    with open(tmp, "wb") as f:
        f.write(newzip)
    os.replace(tmp, zpath)
    print(f"APPLIED: {zpath} ({len(newzip)} bytes)")


if __name__ == "__main__":
    main()
