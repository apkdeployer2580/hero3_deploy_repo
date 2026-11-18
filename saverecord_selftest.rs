//! Standalone selftest for `wie_android/src/saverecord.rs` — the host's reading of the
//! game's own save-record format (MD5 header + swapped-halves DES body).
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the module is dependency-free and
//! this file `include!`s **the real source**. Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2021 --test saverecord_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! The three fixtures below are records the GUEST wrote (wie_probe `--host-save 1750`
//! on the shipped LGT archive, 2026-09-13): a 1408-byte `GameSave_11_0`, its 32-byte
//! `SlotSave_11_0`, and the 32-byte `OptionSave_11`. They are the ground truth this
//! module is measured against — a record the game itself produced must validate, and
//! any single byte flipped must not.

include!("wie_android/src/saverecord.rs");

mod fixtures {
    include!("wie_android/src/saverecord_fixtures.rs");
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // ── MD5: RFC 1321 test suite ──────────────────────────────────────────────
    #[test]
    fn md5_matches_rfc_1321() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(hex(&md5(b"abcdefghijklmnopqrstuvwxyz")), "c3fcd3d76192e4007dfb496cca67e13b");
        assert_eq!(
            hex(&md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        // Lengths straddling the padding boundary (55 / 56 / 64 bytes).
        assert_eq!(hex(&md5(&[b'x'; 55])), hex(&md5(&vec![b'x'; 55])));
        assert_eq!(md5(&[0u8; 56]).len(), 16);
        assert_eq!(md5(&[0u8; 64]).len(), 16);
    }

    // ── the table ─────────────────────────────────────────────────────────────
    #[test]
    fn the_rebuilt_table_is_the_textbook_plus_one_byte() {
        let t = game_table();
        assert_eq!(t.len(), TABLE_LEN);
        assert_eq!(t[TWEAK_OFFSET], TWEAK_VALUE, "S-box 1 entry 58 is the asset's deviation");
        // Measured md5 of the shipped `res/dat/des_dat` (2026-09-13).
        assert_eq!(hex(&md5(&t)), "be7470ffa34cac05d0c1056b697abf75", "the rebuilt table must be byte-identical to the asset");
        assert!(Des::new(&t, &KEY).is_some());
    }

    #[test]
    fn a_bad_table_gives_no_opinion_rather_than_a_verdict() {
        assert_eq!(check(&GAMESAVE_1408, &[0u8; 100]), Verdict::NoTable);
        let mut t = game_table();
        t[0] = 200; // an IP index past 64 would index out of range
        assert_eq!(check(&GAMESAVE_1408, &t), Verdict::NoTable);
        assert!(Des::new(&t, &KEY).is_none());
    }

    // ── the records the guest wrote ──────────────────────────────────────────
    #[test]
    fn the_guests_own_records_validate() {
        let t = game_table();
        assert_eq!(check(&GAMESAVE_1408, &t), Verdict::Valid { plain_len: 1392 });
        assert_eq!(check(&SLOTSAVE_32, &t), Verdict::Valid { plain_len: 16 });
        assert_eq!(check(&OPTIONSAVE_32, &t), Verdict::Valid { plain_len: 16 });
    }

    /// The plaintext heads measured with `tools_re/h3crypt.py` on the same bytes — so the
    /// Rust cipher is checked against an independent implementation, not just against
    /// its own MD5.
    #[test]
    fn the_plaintext_agrees_with_the_python_reference() {
        let t = game_table();
        let p = plaintext(&GAMESAVE_1408, &t).unwrap();
        assert_eq!(hex(&p[..24]), "050101000000000001010101000000000001010100000000");
        let s = plaintext(&SLOTSAVE_32, &t).unwrap();
        assert_eq!(hex(&s), "000000000c4400000501000000000000");
        let o = plaintext(&OPTIONSAVE_32, &t).unwrap();
        assert_eq!(hex(&o), "030308010201013c3c143c0000000000");
    }

    #[test]
    fn every_single_byte_flip_is_caught() {
        let t = game_table();
        let des = Des::new(&t, &KEY).unwrap();
        // Every byte of the summary, and a stride through the payload.
        for i in 0..SLOTSAVE_32.len() {
            let mut r = SLOTSAVE_32;
            r[i] ^= 0x01;
            assert_eq!(check_with(&r, &des), Verdict::Md5Mismatch, "summary byte {i}");
        }
        for i in (0..GAMESAVE_1408.len()).step_by(37) {
            let mut r = GAMESAVE_1408;
            r[i] ^= 0x80;
            assert_eq!(check_with(&r, &des), Verdict::Md5Mismatch, "payload byte {i}");
        }
    }

    #[test]
    fn a_truncated_or_padded_record_is_not_the_record() {
        let t = game_table();
        assert_eq!(check(&GAMESAVE_1408[..1400], &t), Verdict::Md5Mismatch);
        assert_eq!(check(&GAMESAVE_1408[..16], &t), Verdict::Md5Mismatch);
        assert_eq!(check(&SLOTSAVE_32[..24], &t), Verdict::Md5Mismatch);
        let mut longer = GAMESAVE_1408.to_vec();
        longer.push(0);
        assert_eq!(check(&longer, &t), Verdict::Md5Mismatch, "a plaintext tail byte is still covered by the MD5");
        assert_eq!(check(&[], &t), Verdict::TooShort);
        assert_eq!(check(&[0u8; 15], &t), Verdict::TooShort);
        assert_eq!(check(&[0u8; 16], &t), Verdict::Md5Mismatch, "an all-zero header is not the MD5 of nothing");
    }

    /// The writer leaves `len % 8` trailing bytes as plaintext. A record built that way
    /// must validate — the rule is honoured even though shipped records are whole blocks.
    #[test]
    fn a_plaintext_tail_is_part_of_the_digest() {
        let t = game_table();
        let des = Des::new(&t, &KEY).unwrap();
        // Take the real summary's plaintext, append three plaintext bytes, re-frame.
        let plain = plaintext(&SLOTSAVE_32, &t).unwrap();
        let mut ext = plain.clone();
        ext.extend_from_slice(b"abc");
        let mut rec = md5(&ext).to_vec();
        rec.extend_from_slice(&SLOTSAVE_32[16..]); // the two encrypted blocks, unchanged
        rec.extend_from_slice(b"abc");
        assert_eq!(check_with(&rec, &des), Verdict::Valid { plain_len: 19 });
        assert_eq!(des.decrypt_body(&rec[16..]), ext);
    }

    #[test]
    fn hex_head_is_bounded() {
        assert_eq!(hex_head(&[0xde, 0xad], 8), "dead");
        assert_eq!(hex_head(&[0xde, 0xad, 0xbe], 2), "dead…");
        assert_eq!(hex_head(&[], 4), "");
    }
}
