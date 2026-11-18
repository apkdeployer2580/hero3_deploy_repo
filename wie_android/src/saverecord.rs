// The game's own save-record format, readable by the host — MD5 header + DES body.
//
// ── WHY THE HOST HAS TO BE ABLE TO READ A RECORD (90차, 2026-09-13) ─────────────
// A slot row reads 「빈슬롯」 when `loadSaveRecord` (LGT `0x1a0e8`) returns 0, and the
// only test that function applies to a record that exists is
//
//     MD5(descramble(record[16..])) == record[0..16]
//
// Nothing else — no length field, no magic, no version. So "the game cannot read this
// record" is a property of the BYTES, and until this file existed the host could not
// evaluate it: every `stream_write` was stored verbatim and every `stream_read` handed
// back verbatim, and a record the guest itself could not read back was written
// "successfully", logged as `DB set … len=32`, and discovered by the player as an empty
// row after the next launch. Eight rounds of slot-pin policy (67차..78차) could not find
// that, because none of them could tell a good record from a bad one.
//
// With this module the host CAN. `check` answers exactly what the guest's loader would
// answer, so (a) `Database::set` refuses to replace a readable record with an
// unreadable one and says so, (b) boot classifies every character record as ok / MD5 BAD
// instead of "N bytes", and (c) the vault can offer and auto-apply only generations the
// game will actually read. See `vault.rs` for the three call sites.
//
// ── THE CIPHER (guest `0x2a684` desInit · `0x2acfc` encrypt · `0x2ac94` decrypt) ─
// * DES whose permutation tables are the asset `res/dat/des_dat` (824 B):
//   `IP(64) FP(64) E(48) P(32) S(8×64) PC1(56) PC2(48)`. Every table is the textbook
//   one EXCEPT one byte: S-box 1 entry 58 is 2, not 3 (`tools_re/h3crypt.py` found
//   it; re-measured 2026-09-13 as the only difference in 824 bytes). [`game_table`]
//   rebuilds the asset from the textbook tables plus that one byte, so nothing from the
//   asset is committed here.
// * Key = the first 8 bytes of the rodata string `0XP@KOE9XZ7JE` (`0x5c9f4`).
// * The two halves are swapped BEFORE round 1 as well as after round 16 — plain DES
//   with the same tables produces garbage.
// * The writer (`0x1a1d4`) encrypts `len / 8` whole blocks IN PLACE and leaves the
//   `len % 8` tail as plaintext (`0x2acfc`: `nblocks = len >> 3`); the MD5 covers all
//   `len` plaintext bytes. Records seen so far are whole blocks (1392, 16), but the
//   rule is honoured so an odd length still validates.
// * The 16-byte summary `slotSummary` writes has the same framing (32 B on disk), and
//   so does `OptionSave_11`.
//
// Verified against records the guest itself produced on 2026-09-13 (wie_probe
// `--host-save`, LGT 01.00.07): GameSave 1408 B, SlotSave 32 B, OptionSave 32 B all
// validate, and `saverecord_selftest.rs` carries those bytes as fixtures.
//
// Dependency-free on purpose: `saverecord_selftest.rs` `include!`s this file. **Do not
// add imports here.** The MD5 is written out rather than pulled from a crate for the
// same reason.

/// Bytes in `res/dat/des_dat`.
pub const TABLE_LEN: usize = 824;
/// The writer's header: MD5 of the plaintext.
pub const HEADER_LEN: usize = 16;
/// The guest's DES key (`0x5c9f4`, first 8 bytes of `0XP@KOE9XZ7JE`).
pub const KEY: [u8; 8] = *b"0XP@KOE9";

// ── the tables ───────────────────────────────────────────────────────────────

const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6, 64, 56, 48, 40, 32, 24, 16, 8, 57,
    49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61, 53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];
const FP: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30, 37, 5, 45, 13, 53, 21, 61, 29, 36,
    4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27, 34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];
const E: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18, 19, 20, 21, 20, 21, 22, 23, 24, 25,
    24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];
const P: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19, 13, 30, 6, 22, 11, 4, 25,
];
const S: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6,
        2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1, 10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4,
        13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15,
        3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11,
        7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 1, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13,
        7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8,
        12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3,
        7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12,
        14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];
const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60, 52, 44, 36, 63, 55, 47, 39, 31, 23,
    15, 7, 62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 28, 20, 12, 4,
];
const PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52, 31, 37, 47, 55, 30, 40, 51, 45, 33,
    48, 44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];
/// Key-schedule rotations (guest `0x5d45c`) — the standard ones.
const SHIFTS: [usize; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

/// Offset of the one byte where the asset departs from the textbook: S-box 1, entry 58
/// (`208 + 58`).
pub const TWEAK_OFFSET: usize = 208 + 58;
/// The asset's value there (the textbook says 3).
pub const TWEAK_VALUE: u8 = 2;

/// `res/dat/des_dat`, rebuilt from the textbook tables plus the one measured deviation.
///
/// The host compares this with the asset at boot (`vault::install_table`) and adopts the
/// asset's bytes if they ever differ, so a re-patched archive cannot silently make every
/// record look broken.
pub fn game_table() -> [u8; TABLE_LEN] {
    let mut t = [0u8; TABLE_LEN];
    let mut i = 0;
    for src in [&IP[..], &FP[..], &E[..], &P[..]] {
        t[i..i + src.len()].copy_from_slice(src);
        i += src.len();
    }
    for sbox in &S {
        t[i..i + 64].copy_from_slice(sbox);
        i += 64;
    }
    for src in [&PC1[..], &PC2[..]] {
        t[i..i + src.len()].copy_from_slice(src);
        i += src.len();
    }
    debug_assert_eq!(i, TABLE_LEN);
    t[TWEAK_OFFSET] = TWEAK_VALUE;
    t
}

// ── DES, bit-per-byte, parameterised by the table ─────────────────────────────
//
// Speed is irrelevant here (a save is 174 blocks, a summary 2; both run once per
// write) and a bit-per-byte transliteration of the guest's own routine is the form
// easiest to check against it.

/// A key schedule, ready to decrypt with the guest's swapped-halves DES.
pub struct Des {
    ip: [u8; 64],
    fp: [u8; 64],
    e: [u8; 48],
    p: [u8; 32],
    s: [[u8; 64]; 8],
    /// Subkeys in DECRYPTION order (K16 first).
    ks: [[u8; 48]; 16],
}

fn bits(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 8);
    for &b in data {
        for i in (0..8).rev() {
            out.push((b >> i) & 1);
        }
    }
    out
}

fn pack(bits: &[u8]) -> Vec<u8> {
    bits.chunks(8).map(|c| c.iter().fold(0u8, |v, &b| (v << 1) | b)).collect()
}

fn perm(src: &[u8], table: &[u8]) -> Vec<u8> {
    table.iter().map(|&x| src[usize::from(x) - 1]).collect()
}

impl Des {
    /// `None` when the table is not 824 bytes or a permutation entry is out of range.
    pub fn new(table: &[u8], key: &[u8; 8]) -> Option<Self> {
        if table.len() != TABLE_LEN {
            return None;
        }
        let ip: [u8; 64] = table[0..64].try_into().ok()?;
        let fp: [u8; 64] = table[64..128].try_into().ok()?;
        let e: [u8; 48] = table[128..176].try_into().ok()?;
        let p: [u8; 32] = table[176..208].try_into().ok()?;
        let mut s = [[0u8; 64]; 8];
        for (n, sbox) in s.iter_mut().enumerate() {
            sbox.copy_from_slice(&table[208 + 64 * n..208 + 64 * (n + 1)]);
        }
        let pc1: [u8; 56] = table[720..776].try_into().ok()?;
        let pc2: [u8; 48] = table[776..824].try_into().ok()?;
        // Every permutation index must address its source width, or `perm` panics.
        let in_range = |t: &[u8], width: usize| t.iter().all(|&x| (1..=width as u8).contains(&x));
        if !(in_range(&ip, 64) && in_range(&fp, 64) && in_range(&e, 32) && in_range(&p, 32) && in_range(&pc1, 64) && in_range(&pc2, 56)) {
            return None;
        }
        if s.iter().flatten().any(|&v| v > 15) {
            return None;
        }
        // Key schedule, then reversed: the guest's decrypt walks K16..K1 (the "half
        // swap" it performs on the subkey block at `0x2ac94` is exactly this reversal).
        let k = perm(&bits(key), &pc1);
        let (mut c, mut d) = (k[..28].to_vec(), k[28..].to_vec());
        let mut ks = [[0u8; 48]; 16];
        for (round, &sh) in SHIFTS.iter().enumerate() {
            c.rotate_left(sh);
            d.rotate_left(sh);
            let cd: Vec<u8> = c.iter().chain(d.iter()).copied().collect();
            ks[15 - round].copy_from_slice(&perm(&cd, &pc2));
        }
        Some(Self { ip, fp, e, p, s, ks })
    }

    /// The Feistel function on a 32-bit half (as 32 bit-bytes).
    fn f(&self, r: &[u8], k: &[u8; 48]) -> Vec<u8> {
        let x: Vec<u8> = perm(r, &self.e).iter().zip(k.iter()).map(|(a, b)| a ^ b).collect();
        let mut out = Vec::with_capacity(32);
        for n in 0..8 {
            let s = &x[6 * n..6 * n + 6];
            let row = (s[0] << 1) | s[5];
            let col = (s[1] << 3) | (s[2] << 2) | (s[3] << 1) | s[4];
            let v = self.s[n][usize::from((row << 4) | col)];
            out.extend_from_slice(&[(v >> 3) & 1, (v >> 2) & 1, (v >> 1) & 1, v & 1]);
        }
        perm(&out, &self.p)
    }

    /// Decrypt one 8-byte block the way the guest does — halves swapped before round 1.
    pub fn decrypt_block(&self, block: &[u8; 8]) -> [u8; 8] {
        let b = perm(&bits(block), &self.ip);
        let (mut l, mut r) = (b[32..].to_vec(), b[..32].to_vec());
        for k in &self.ks {
            let fr = self.f(&r, k);
            let nr: Vec<u8> = l.iter().zip(fr.iter()).map(|(a, b)| a ^ b).collect();
            l = r;
            r = nr;
        }
        let rl: Vec<u8> = r.iter().chain(l.iter()).copied().collect();
        let out = pack(&perm(&rl, &self.fp));
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&out);
        arr
    }

    /// Descramble a record body: whole blocks are decrypted, the `len % 8` tail is
    /// plaintext (the writer never touched it).
    pub fn decrypt_body(&self, body: &[u8]) -> Vec<u8> {
        let whole = body.len() / 8 * 8;
        let mut out = Vec::with_capacity(body.len());
        for block in body[..whole].chunks_exact(8) {
            let mut b = [0u8; 8];
            b.copy_from_slice(block);
            out.extend_from_slice(&self.decrypt_block(&b));
        }
        out.extend_from_slice(&body[whole..]);
        out
    }
}

// ── MD5 (RFC 1321), written out so this file stays dependency-free ─────────────

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4,
    11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1,
    0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453,
    0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942,
    0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05,
    0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d,
    0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// MD5 of `data`.
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

// ── the verdict ───────────────────────────────────────────────────────────────

/// What the guest's loader would make of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The MD5 matches: the game will load it. `plain_len` = bytes after the header.
    Valid { plain_len: usize },
    /// Shorter than the 16-byte header (the guest reads a 0..15-byte record as a
    /// negative memcpy — `savevault::record_len_safe` hides those already).
    TooShort,
    /// The header does not match the descrambled body: the game reads this as 빈슬롯.
    Md5Mismatch,
    /// The table handed in is not a usable `des_dat`: no opinion.
    NoTable,
}

impl Verdict {
    #[allow(dead_code)] // selftest + future callers; `vault::classify` matches directly
    pub fn is_valid(self) -> bool {
        matches!(self, Verdict::Valid { .. })
    }
}

/// Judge `record` with `table` (824 bytes of `des_dat`).
pub fn check(record: &[u8], table: &[u8]) -> Verdict {
    let Some(des) = Des::new(table, &KEY) else {
        return Verdict::NoTable;
    };
    check_with(record, &des)
}

/// [`check`] with a prepared schedule — for a caller judging many records.
pub fn check_with(record: &[u8], des: &Des) -> Verdict {
    if record.len() < HEADER_LEN {
        return Verdict::TooShort;
    }
    let plain = des.decrypt_body(&record[HEADER_LEN..]);
    if md5(&plain) == record[..HEADER_LEN] {
        Verdict::Valid { plain_len: plain.len() }
    } else {
        Verdict::Md5Mismatch
    }
}

/// The plaintext of a valid record, or `None`. Not on any shipping path yet — it is
/// what a host-side summary rebuild would start from, and the selftest uses it to compare
/// against the Python reference.
#[allow(dead_code)]
pub fn plaintext(record: &[u8], table: &[u8]) -> Option<Vec<u8>> {
    let des = Des::new(table, &KEY)?;
    if record.len() < HEADER_LEN {
        return None;
    }
    let plain = des.decrypt_body(&record[HEADER_LEN..]);
    (md5(&plain) == record[..HEADER_LEN]).then_some(plain)
}

/// Hex of the first `n` bytes, for the refusal line — enough to recognise a zeroed
/// buffer, a shifted record or a stale header at a glance.
pub fn hex_head(bytes: &[u8], n: usize) -> String {
    let mut s = String::with_capacity(n * 2 + 1);
    for b in bytes.iter().take(n) {
        s.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > n {
        s.push('…');
    }
    s
}
