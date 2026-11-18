"""Read the game's ENCRYPTED res/dat members (drop / shop / smith / getitem / i15).

Six data files in the LGT archive are ciphertext, so every earlier session that
tried to answer "what does this monster drop" had to give up and read the guest's
per-map copy instead.  They are all readable from the host now.

THE CIPHER (guest 0x2a684 desInit / 0x2ac94 desDecrypt / 0x141c4 resource loader)

  * DES whose tables are the file `res/dat/des_dat` itself, in this order:
        IP(64) FP(64) E(48) P(32) S(8*64) PC1(56) PC2(48)   = 824 B
    Every table is the textbook one EXCEPT S-box 1 index 58, which is 2 instead
    of 3.  The key schedule shifts are the standard 1,1,2,2,... (guest 0x5d45c).
  * Key = the first 8 bytes of the rodata string "0XP@KOE9XZ7JE" (0x5c9f4).
    desInit memcpy's 8 bytes and NUL-terminates, so the trailing 5 are unused.
  * THE DEVIATION THAT DEFEATS A STOCK DES: the two halves are swapped BEFORE
    round 1, as well as by the usual swap after round 16 - L and R trade roles.
    Plain DES with the same key and tables produces garbage.
  * Framing = [16 B MD5 of the plaintext][DES-ECB body].  `decrypt_member`
    verifies that MD5, so a wrong key or a stale asset is an exception, never a
    plausible-looking table.

WHAT THE PLAINTEXT LOOKS LIKE - every one of these is `[u16 LE reclen][body]*`:

  drop_dat / droph_dat  161 records x 17 B, indexed by SPECIES (same order and
                        count as enemy_dat).  Fields, all LE:
                          +0x00 u16 weapon cap      +0x02 u16 weapon split
                          +0x04 u16 armour cap      +0x06 u16 armour split
                          +0x08 u16 soul cap        +0x0a u8  soul idx
                          +0x0b u16 named-item cap  +0x0d u8  cat  +0x0e u8 idx
                          +0x0f u16 consumable cap
                        (the guest's in-memory copy is 0x14 B - it aligns the
                        u16 at +0x0b, which is why older notes say +0x0c.)
                        Anchor: species 29 hard = cat 0x04 idx 0x15 = {레이븐,
                        the drop this repo already knew about from play.
  shop_dat              5 x 8 B: [lv lo][lv hi][category list, 0xff padded]
  smith_dat             80 x 9 B
  getitem_dat           96 x 2 B: (cat, idx) pairs
  i15_dat               89 x consumable-shaped records - NOT an equipment
                        category.  It is the SMITHING RECIPE table: name, price,
                        a Korean recipe string, then a tail whose last two bytes
                        are the (cat, idx) of the equipment the recipe produces
                        and whose first two are the base item consumed.

THE ENCODER (138차, 2026-09-29) — so a member can be WRITTEN, not only read

  The guest decrypts `D(y) = FP(F_rev(S(IP(y))))` (F = the 16 Feistel rounds with the
  usual final swap, S = the extra half swap before round 1, rev = reversed schedule), and
  F_rev inverts F, so the encryption the game will undo is `E(x) = FP(S(F(IP(x))))` —
  forward schedule, the SAME halves swap moved to the end. `encrypt_member` frames it the
  way the asset does: [MD5 of the plaintext][ECB body], the plaintext zero-padded to a
  whole block (the shipped members carry that padding after the last record).
  `--check` proves it on the eight shipped members: `encrypt(decrypt(raw)) == raw`, BYTE
  FOR BYTE — the strongest statement available that a written member is one the guest
  reads exactly as the tool meant (`add_tower_recipes.py` writes i15_dat with it).

Usage:
    python tools_re/h3crypt.py [--zip <asset.zip>] [--out <dir>] [--check]
"""
import argparse
import hashlib
import io
import os
import struct
import sys
import zipfile

SHIFTS = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1]
KEY = b"0XP@KOE9"
DEFAULT_ZIP = os.path.join("android_app", "app", "src", "main", "assets", "hero3_fixed.zip")
DEFAULT_JAR = "0002865E.jar"
ENCRYPTED = ["res/dat/drop_dat", "res/dat/droph_dat", "res/dat/shop_dat",
             "res/dat/shoph_dat", "res/dat/smith_dat", "res/dat/smithh_dat",
             "res/dat/getitem_dat", "res/dat/i15_dat"]


def _bits(data):
    out = []
    for by in data:
        out += [(by >> i) & 1 for i in range(7, -1, -1)]
    return out


def _pack(bits):
    out = bytearray()
    for i in range(0, len(bits), 8):
        v = 0
        for b in bits[i:i + 8]:
            v = (v << 1) | b
        out.append(v)
    return bytes(out)


class Des:
    """DES parameterised by res/dat/des_dat, with the guest's half-swap."""

    def __init__(self, tbl):
        if len(tbl) != 824:
            raise ValueError("des_dat must be 824 bytes, got %d" % len(tbl))
        self.IP, self.FP = list(tbl[0:64]), list(tbl[64:128])
        self.E, self.P = list(tbl[128:176]), list(tbl[176:208])
        self.S = [list(tbl[208 + 64 * i:208 + 64 * (i + 1)]) for i in range(8)]
        self.PC1, self.PC2 = list(tbl[720:776]), list(tbl[776:824])

    @staticmethod
    def _perm(b, t):
        return [b[x - 1] for x in t]

    def _schedule(self, key):
        k = self._perm(_bits(key), self.PC1)
        c, d, ks = k[:28], k[28:], []
        for s in SHIFTS:
            c, d = c[s:] + c[:s], d[s:] + d[:s]
            ks.append(self._perm(c + d, self.PC2))
        return ks

    def _f(self, r, k):
        x = [a ^ b for a, b in zip(self._perm(r, self.E), k)]
        out = []
        for n in range(8):
            s = x[6 * n:6 * n + 6]
            v = self.S[n][((s[0] << 1 | s[5]) << 4) |
                          (s[1] << 3 | s[2] << 2 | s[3] << 1 | s[4])]
            out += [(v >> 3) & 1, (v >> 2) & 1, (v >> 1) & 1, v & 1]
        return self._perm(out, self.P)

    def ecb_decrypt(self, body, key=KEY):
        ks = self._schedule(key)[::-1]
        out = bytearray()
        for i in range(0, len(body), 8):
            b = self._perm(_bits(body[i:i + 8]), self.IP)
            l, r = b[32:], b[:32]          # halves swapped before round 1
            for k in ks:
                l, r = r, [a ^ b for a, b in zip(l, self._f(r, k))]
            out += _pack(self._perm(r + l, self.FP))
        return bytes(out)

    def ecb_encrypt(self, body, key=KEY):
        """The inverse of `ecb_decrypt`: forward schedule, the halves swap AFTER round 16
        instead of before round 1 (see the module header). `len(body)` must be a whole
        number of 8-byte blocks."""
        if len(body) % 8:
            raise ValueError("DES-ECB body must be a multiple of 8 bytes, got %d" % len(body))
        ks = self._schedule(key)
        out = bytearray()
        for i in range(0, len(body), 8):
            b = self._perm(_bits(body[i:i + 8]), self.IP)
            l, r = b[:32], b[32:]
            for k in ks:
                l, r = r, [a ^ b for a, b in zip(l, self._f(r, k))]
            out += _pack(self._perm(l + r, self.FP))
        return bytes(out)


def open_assets(zip_path=DEFAULT_ZIP, jar=DEFAULT_JAR):
    z = zipfile.ZipFile(zip_path)
    return zipfile.ZipFile(io.BytesIO(z.read(jar)))


def decrypt_member(jar, name):
    """Plaintext of an encrypted res/dat member. Raises if the MD5 header fails."""
    raw = jar.read(name)
    plain = Des(jar.read("res/dat/des_dat")).ecb_decrypt(raw[16:])
    if hashlib.md5(plain).digest() != raw[:16]:
        raise ValueError("%s: MD5 header mismatch - wrong key or a patched asset" % name)
    return plain


def encrypt_member(plain, des_tbl):
    """[16 B MD5][DES-ECB body] of `plain`, zero-padded to a whole block — the framing
    `decrypt_member` verifies (the MD5 covers the PADDED plaintext, as in the shipped
    members). `des_tbl` is the jar's own `res/dat/des_dat`."""
    padded = bytes(plain) + b"\0" * (-len(plain) % 8)
    return hashlib.md5(padded).digest() + Des(des_tbl).ecb_encrypt(padded)


def decrypt_bytes(raw, des_tbl):
    """`decrypt_member` for a member already in hand. Raises if the MD5 header fails."""
    plain = Des(des_tbl).ecb_decrypt(raw[16:])
    if hashlib.md5(plain).digest() != raw[:16]:
        raise ValueError("MD5 header mismatch - wrong key or a patched asset")
    return plain


def records(b):
    """Split a `[u16 LE len][body]*` stream. Trailing cipher padding is dropped."""
    out, o = [], 0
    while o + 2 <= len(b):
        n = struct.unpack_from("<H", b, o)[0]
        if n == 0 or o + 2 + n > len(b):
            break
        out.append(b[o + 2:o + 2 + n])
        o += 2 + n
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zip", default=DEFAULT_ZIP)
    ap.add_argument("--jar", default=DEFAULT_JAR)
    ap.add_argument("--out", help="write each plaintext to this directory")
    ap.add_argument("--check", action="store_true",
                    help="exit 1 unless every encrypted member decrypts and verifies")
    a = ap.parse_args()
    jar = open_assets(a.zip, a.jar)
    des = jar.read("res/dat/des_dat")
    bad = 0
    for name in ENCRYPTED:
        try:
            p = decrypt_member(jar, name)
        except KeyError:
            print("%-24s MISSING" % name)
            continue
        except ValueError as e:
            print("%-24s FAIL  %s" % (name, e))
            bad += 1
            continue
        # ★ 138차: the encoder must reproduce the shipped ciphertext exactly.
        roundtrip = encrypt_member(p, des) == jar.read(name)
        if not roundtrip:
            bad += 1
        print("%-24s ok  %5d B  %3d records  encode %s"
              % (name, len(p), len(records(p)), "round-trips" if roundtrip else "DIFFERS"))
        if a.out:
            os.makedirs(a.out, exist_ok=True)
            with open(os.path.join(a.out, os.path.basename(name) + ".plain"), "wb") as f:
                f.write(p)
    if a.check and bad:
        sys.exit(1)


if __name__ == "__main__":
    main()
