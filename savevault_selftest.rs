//! Standalone selftest for `wie_android/src/savevault.rs`.
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the save-vault policy is kept
//! dependency-free and this file `include!`s **the real source**, not a copy of
//! it. Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2021 --test savevault_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! ★ This is an `include!`, so deleting a function in `savevault.rs` breaks the
//! build here rather than leaving a green mirror behind — the trap
//! `items_selftest.rs` and `landscape_selftest.rs` fell into (CLAUDE.md).

include!("wie_android/src/savevault.rs");

/// A second, outside-in pass over the rotation policy: the module's own tests
/// check the tiers, this one checks the properties that must hold for ANY input,
/// because rotation is the one part of the vault that can destroy data.
#[cfg(test)]
mod properties {
    use super::*;

    /// Deterministic pseudo-random generations — no `rand` dependency, and a fixed
    /// seed so a failure is reproducible.
    fn spread(seed: u64, n: usize) -> Vec<(u64, usize)> {
        let mut s = seed | 1;
        let mut out = Vec::with_capacity(n);
        let mut t = 1_800_000_000_000u64;
        for _ in 0..n {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            t = t.wrapping_add((s >> 40) % (6 * 60 * 60 * 1000)); // 0..6h apart
            let len = if (s >> 33) % 7 == 0 { 0 } else { 1392 + 0x10 };
            out.push((t, len));
        }
        out
    }

    #[test]
    fn rotation_never_deletes_the_newest_generation() {
        for seed in 1..40u64 {
            for n in 1..40usize {
                let gens = spread(seed, n);
                let newest = gens
                    .iter()
                    .enumerate()
                    .max_by_key(|(i, g)| (g.0, *i))
                    .map(|(i, _)| i)
                    .unwrap();
                assert!(
                    !to_delete(&gens, None).contains(&newest),
                    "seed {seed} n {n}: the newest generation was deleted"
                );
            }
        }
    }

    #[test]
    fn rotation_indices_are_unique_valid_and_bounded() {
        for seed in 1..40u64 {
            for n in 0..40usize {
                let gens = spread(seed, n);
                let del = to_delete(&gens, None);
                let mut sorted = del.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), del.len(), "seed {seed} n {n}: duplicate index");
                assert!(del.iter().all(|i| *i < n), "seed {seed} n {n}: index out of range");
                let kept = n - del.len();
                assert!(kept <= KEEP_MAX, "seed {seed} n {n}: kept {kept} over the ceiling");
                assert!(kept >= n.min(KEEP_NEWEST), "seed {seed} n {n}: kept fewer than the guaranteed newest");
            }
        }
    }

    /// Rotation must converge: applying it, dropping the survivors back in and
    /// applying it again must not keep eating generations forever.
    #[test]
    fn rotation_is_idempotent_once_the_vault_is_trimmed() {
        for seed in 1..40u64 {
            let gens = spread(seed, 30);
            let del = to_delete(&gens, None);
            let kept: Vec<(u64, usize)> = gens
                .iter()
                .enumerate()
                .filter(|(i, _)| !del.contains(i))
                .map(|(_, g)| *g)
                .collect();
            assert!(
                to_delete(&kept, None).is_empty(),
                "seed {seed}: a second pass deleted more from an already-trimmed vault"
            );
        }
    }

    /// The vault must not grow without bound even if every write is different,
    /// which is the shape of a guest stuck in a save loop.
    #[test]
    fn a_write_loop_cannot_grow_the_vault_past_the_ceiling() {
        let mut gens: Vec<(u64, usize)> = Vec::new();
        let mut t = 1_800_000_000_000u64;
        for _ in 0..500 {
            t += MIN_GAP_MS;
            gens.push((t, 1408));
            let del = to_delete(&gens, None);
            gens = gens
                .iter()
                .enumerate()
                .filter(|(i, _)| !del.contains(i))
                .map(|(_, g)| *g)
                .collect();
            assert!(gens.len() <= KEEP_MAX);
        }
        assert!(gens.len() <= KEEP_MAX);
    }

    /// ★★★ 72차 — THE TEST THE 2026-08-29 REPORT SHOULD HAVE FAILED.
    ///
    /// The loop above asserts only that the vault does not GROW. It said nothing about
    /// how far back the survivors reach, so it stayed green while a 제련 grind collapsed
    /// the whole recent tier to sixty seconds — six seats spent [`MIN_GAP_MS`] apart —
    /// and the player was left with nothing between "one minute ago" and "four days ago".
    /// That is the sentence in the report, and this is the assertion that forbids it.
    #[test]
    fn a_write_loop_cannot_collapse_the_history_to_the_last_minute() {
        // Two hours of a refine grind: a full save pair every 10 s is the fastest the
        // vault can be made to take a copy.
        let mut gens: Vec<(u64, usize)> = Vec::new();
        let mut t = 1_800_000_000_000u64;
        for _ in 0..720 {
            t += MIN_GAP_MS;
            gens.push((t, 1408));
            let del = to_delete(&gens, None);
            gens = gens.iter().enumerate().filter(|(i, _)| !del.contains(i)).map(|(_, g)| *g).collect();
            assert!(gens.len() <= KEEP_MAX, "the ceiling still holds");
        }
        let newest = gens.iter().map(|g| g.0).max().unwrap();
        let deepest = newest - gens.iter().map(|g| g.0).min().unwrap();
        assert!(
            deepest >= 60 * 60_000,
            "a two-hour grind must leave a restore point at least an hour deep; deepest was {deepest} ms across {} generations",
            gens.len()
        );
        // …and the dense head is still dense: the player who notices immediately needs
        // the last few saves, not just the ladder.
        let within_a_minute = gens.iter().filter(|g| newest - g.0 <= 60_000).count();
        assert!(within_a_minute >= 6, "the newest six must survive too, got {within_a_minute}");
    }

    /// Every rung of the ladder is actually occupied when the history is long enough to
    /// fill it — a rung that silently never matches is a rung that protects nothing.
    #[test]
    fn every_age_rung_keeps_a_generation() {
        let mut gens: Vec<(u64, usize)> = Vec::new();
        let mut t = 1_800_000_000_000u64;
        for _ in 0..5000 {
            t += 10_000;
            gens.push((t, 1408));
            let del = to_delete(&gens, None);
            gens = gens.iter().enumerate().filter(|(i, _)| !del.contains(i)).map(|(_, g)| *g).collect();
        }
        let newest = gens.iter().map(|g| g.0).max().unwrap();
        // The run is 50 000 s (~13.9 h) long, so every rung up to 12 h is reachable. A
        // band can be momentarily empty just after its occupant graduates, so the property
        // asserted is the one that matters: SOMETHING at least this old is still on disk.
        for rung in SPREAD_MS {
            assert!(
                gens.iter().any(|g| newest - g.0 >= rung),
                "no survivor at least {rung} ms old; survivor ages: {:?}",
                gens.iter().map(|g| newest - g.0).collect::<Vec<_>>()
            );
        }
    }

    /// F9: the host must never hand the guest a character record it would `memcpy`
    /// negatively. `loadSaveRecord 0x1a0e8` computes `total - 0x10` with no lower bound.
    #[test]
    fn a_record_the_guest_would_subtract_below_zero_is_never_presented() {
        for len in 1..RECORD_HEADER {
            assert!(!record_len_safe("GameSave_11_0", len), "{len} B GameSave must be hidden");
            assert!(!record_len_safe("SlotSave_11_2", len), "{len} B SlotSave must be hidden");
        }
        // 0 is the guest's own clean "empty" branch, and >= 0x10 is arithmetically safe.
        assert!(record_len_safe("GameSave_11_0", 0));
        assert!(record_len_safe("SlotSave_11_0", 0));
        assert!(record_len_safe("SlotSave_11_0", RECORD_HEADER));
        assert!(record_len_safe("GameSave_11_0", 1408));
        // Anything that is not a character record is none of our business.
        for len in 0..40 {
            assert!(record_len_safe("OptionSave_11", len));
            assert!(record_len_safe("Hero3Net", len));
        }
    }

    /// An unreadable record must read as OCCUPIED, never as empty: `delete_verdict`'s
    /// `target_len == 0` arm ends in `fs::remove_file`.
    #[test]
    fn an_unreadable_record_is_never_mistaken_for_an_empty_one() {
        assert_eq!(target_len(Ok(1408)), 1408);
        assert_eq!(target_len(Err(true)), 0, "NotFound really is absent");
        assert_eq!(target_len(Err(false)), UNREADABLE_LEN, "any other error must not look empty");
        // The two verdicts must both refuse on it, with and without a pin.
        assert_eq!(write_verdict(None, 1, target_len(Err(false)), None), Verdict::Refuse);
        assert_eq!(delete_verdict(None, 1, target_len(Err(false))), Verdict::Refuse);
        assert_eq!(delete_verdict(Some(0), 1, target_len(Err(false))), Verdict::Refuse);
        // …and the pinned character's own slot is still its own to overwrite.
        assert_eq!(delete_verdict(Some(1), 1, target_len(Err(false))), Verdict::Allow);
    }

    /// The restore's summary partner is chosen by VALIDITY, not by proximity.
    #[test]
    fn a_summary_partner_must_be_the_right_size_and_from_the_same_save() {
        let t = 1_800_000_000_000u64;
        assert!(summary_pairs(t, t, SUMMARY_LEN), "same instant, right size");
        assert!(summary_pairs(t, t + 40, SUMMARY_LEN), "a few ms apart is the normal case");
        assert!(summary_pairs(t, t - PAIR_WINDOW_MS, SUMMARY_LEN), "the window is inclusive");
        assert!(!summary_pairs(t, t + PAIR_WINDOW_MS + 1, SUMMARY_LEN), "just outside the window");
        assert!(!summary_pairs(t, t + 4 * 24 * 3_600_000, SUMMARY_LEN), "four days apart is not a pair");
        for bad in [0, 1, RECORD_HEADER, SUMMARY_LEN - 1, SUMMARY_LEN + 1, 1408] {
            assert!(!summary_pairs(t, t, bad), "{bad} is not a slot summary");
        }
    }

    /// What the whole vault exists for: after a wipe, the pre-image of the wipe is
    /// still on disk and is what a restore would offer.
    #[test]
    fn the_pre_image_of_a_wipe_is_what_restore_offers() {
        // A good record, then the wipe destroys it. The vault captured the good
        // bytes on the way out (should_vault says yes), so the generation list is:
        let gens = [(1_800_000_000_000u64, 1392 + 0x10)];
        assert!(should_vault(&[1u8; 1408], None, 0, 1_800_000_000_000, MIN_GAP_MS));
        assert!(plausible_game_record(gens[0].1), "…and its length marks it as a real record");
        assert!(to_delete(&gens, None).is_empty(), "the only copy of the character must never be rotated out");
    }

    /// ★★ A DELETE IS NEVER RATE-LIMITED (2026-08-25).
    ///
    /// The limit stops a guest writing in a loop from turning the vault into a write
    /// amplifier. A delete is not one of a series — it is the last thing that ever
    /// happens to that record, and the copy declined here can never be taken again.
    /// The window was real: the warehouse rewrites a save on every deposit and every
    /// page turn, so a 초기화 landing within ten seconds of one of those found the
    /// limit armed and left NOTHING behind.
    #[test]
    fn a_deletion_is_vaulted_even_inside_the_rate_limit_window() {
        let old = [7u8; 1408];
        let now = 1_800_000_000_000u64;
        let just_now = now - 1; // a vault write one millisecond ago
        assert!(!should_vault(&old, Some(&[8u8; 1408]), just_now, now, MIN_GAP_MS), "an OVERWRITE still waits its turn");
        assert!(should_vault(&old, None, just_now, now, MIN_GAP_MS), "★ a DELETE never waits");
        // The one refusal that still applies: there is nothing to keep.
        assert!(!should_vault(&[], None, 0, now, MIN_GAP_MS), "an empty record has nothing to lose");
        // And a delete of a record identical to some other record is still a delete —
        // the identical-content shortcut is an OVERWRITE rule and must not leak here.
        assert!(should_vault(&old, None, now, now, MIN_GAP_MS));
    }

    // ── the restore pin (66차 제보: 복원이 복구 슬롯을 한 칸 밀어낸다) ────────────
    //
    // A restore used to cost three seats — the pre-image capture, the rotate that
    // capture triggers (which could delete the generation being restored), and the
    // byte-identical duplicate the first save afterwards wrote. These properties pin
    // the policy half; the I/O half (`vault.rs`) has no harness on this host.

    /// Whatever the corpus and whichever generation is pinned, rotation deletes
    /// neither the pin nor the newest generation.
    #[test]
    fn rotation_with_a_pin_never_deletes_the_pin_or_the_newest() {
        for seed in [1u64, 7, 99, 1234, 65537] {
            for n in [1usize, 2, 7, 16, 40, 120] {
                let gens = spread(seed, n);
                let newest = (0..n).max_by_key(|i| gens[*i].0).unwrap();
                for pin_at in [0usize, n / 2, n - 1] {
                    let dead = to_delete(&gens, Some(gens[pin_at]));
                    assert!(!dead.contains(&newest), "seed {seed} n {n}: the newest was deleted");
                    // The pin is named by (ms, len), so the index that is force-kept is
                    // the FIRST match — which is what the caller will find on disk too.
                    let kept_pin = gens.iter().position(|g| *g == gens[pin_at]).unwrap();
                    assert!(!dead.contains(&kept_pin), "seed {seed} n {n} pin {pin_at}: the pin was deleted");
                }
            }
        }
    }

    /// Rotation with a pin still converges: feed the survivors back with the same pin
    /// and nothing more is deleted.
    #[test]
    fn rotation_with_a_pin_is_still_idempotent() {
        for seed in [3u64, 11, 4242] {
            let gens = spread(seed, 60);
            let pin = gens[7];
            let dead = to_delete(&gens, Some(pin));
            let left: Vec<(u64, usize)> =
                gens.iter().enumerate().filter(|(i, _)| !dead.contains(i)).map(|(_, g)| *g).collect();
            assert!(to_delete(&left, Some(pin)).is_empty(), "seed {seed}: a second pass deleted more");
        }
    }

    /// ★ The property the report is actually about: a save loop (the warehouse rewrites
    /// the record on every deposit and every page turn) must never age the restored
    /// generation out, and must still respect the ceiling.
    #[test]
    fn a_write_loop_cannot_age_out_the_restored_generation() {
        let pin = (1_800_000_000_000u64, 1408);
        let mut gens: Vec<(u64, usize)> = vec![pin];
        let mut t = pin.0;
        for _ in 0..500 {
            t += MIN_GAP_MS;
            gens.push((t, 1408));
            let del = to_delete(&gens, Some(pin));
            gens = gens.iter().enumerate().filter(|(i, _)| !del.contains(i)).map(|(_, g)| *g).collect();
            assert!(gens.len() <= KEEP_MAX, "the pin must not raise the ceiling");
            assert!(gens.contains(&pin), "the restored generation was rotated out");
        }
    }

    /// End to end, as the player experiences it: restore the OLDEST generation of a
    /// full vault, then save once. Exactly ONE seat is spent (the pre-restore
    /// pre-image), the restored generation is still there, and the duplicate the first
    /// save would have written is declined.
    #[test]
    fn the_restore_scenario_costs_exactly_one_seat() {
        let restored_bytes = [3u8; 1408];
        let mut gens: Vec<(u64, usize)> = (0..KEEP_MAX as u64).map(|i| (1_800_000_000_000 + i * 60_000, 1408)).collect();
        let pin = gens[0];
        let before = gens.len();

        // 1) restore pins the source, then captures the pre-restore bytes as a new
        //    generation and rotates.
        gens.push((1_800_000_000_000 + KEEP_MAX as u64 * 60_000, 1400));
        let del = to_delete(&gens, Some(pin));
        gens = gens.iter().enumerate().filter(|(i, _)| !del.contains(i)).map(|(_, g)| *g).collect();
        assert!(gens.contains(&pin), "the generation just restored must still be there");

        // 2) the first save afterwards: its pre-image IS the restored bytes, so the pin
        //    declines the duplicate and no generation is spent.
        assert!(
            duplicates_pin(&restored_bytes, Some(&restored_bytes)),
            "the first save after a restore must not vault a byte-identical copy"
        );
        // …and the same bytes with no pin WOULD have been vaulted, which is the bug.
        assert!(should_vault(&restored_bytes, Some(&[4u8; 1408]), 0, 1, MIN_GAP_MS));
        assert!(gens.len() <= before, "one seat in, one seat out — never more");
    }

    /// R2 is an OVERWRITE rule. A delete whose pre-image happens to equal the pinned
    /// generation is still vaulted, because a delete can never be re-taken.
    #[test]
    fn a_delete_is_still_vaulted_even_when_it_equals_the_pin() {
        let bytes = [9u8; 1408];
        assert!(duplicates_pin(&bytes, Some(&bytes)), "precondition: it does match the pin");
        assert!(should_vault(&bytes, None, 0, 1_800_000_000_000, MIN_GAP_MS), "a delete is never declined");
    }
}
