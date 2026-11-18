//! Standalone selftest for `wie_android/src/refine.rs` — the 제련 확률 readout
//! (2026-08-26 player request: "제련시 제련확률을 표시해주고싶어 … 제련석을 올릴때마다
//! 확률도 마찬가지로 계산되어서 최신화 되어야해").
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the arithmetic is kept
//! dependency-free and this file `include!`s **the real source**, not a copy of it.
//! Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2021 --test refine_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! ★ This is an `include!`, so deleting a function in `refine.rs` breaks the build here
//! rather than leaving a green mirror behind — the trap `items_selftest.rs` and
//! `landscape_selftest.rs` fell into (CLAUDE.md).

include!("wie_android/src/refine.rs");

/// A second, outside-in pass. The module's own tests check the published table row by
/// row; these re-derive the answer from the guest's algorithm itself — by SIMULATING
/// all 100 dice faces — so a transcription error in the closed form cannot pass both.
#[cfg(test)]
mod simulated {
    use super::*;

    /// The guest's three rolls, enumerated exhaustively. `rand(1,100)` is uniform and
    /// inclusive, so "probability" here is literally "how many of the 100 faces".
    ///
    /// Returns (success, destroy, reset) counted in hundred-thousandths, which is the
    /// finest the three nested 1-in-100 draws can produce (100 * 100 * 100 outcomes,
    /// scaled to a rate).
    fn brute_force(level: i8, stones: u8, streak: u8) -> (u32, u32, u32) {
        let t1 = success_threshold(level, stones, streak);
        let t2 = doom_threshold(level, stones, streak);
        let (mut success, mut destroy, mut reset) = (0u32, 0u32, 0u32);
        for roll1 in 1..=100i32 {
            if roll1 <= t1 {
                success += 100 * 100;
                continue;
            }
            for roll2 in 1..=100i32 {
                if roll2 > t2 {
                    continue; // 제련이 실패하였습니다 — the item is untouched
                }
                for roll3 in 1..=100i32 {
                    if roll3 > 50 { reset += 1 } else { destroy += 1 }
                }
            }
        }
        (success, destroy, reset)
    }

    /// Everything the player can actually be looking at: every level, every stone count
    /// the spinner can reach, every streak the guest can hold. 10 * 99 * 3 = 2,970 cases,
    /// each one a million simulated dice — the whole reachable input space, exhaustively.
    #[test]
    fn evaluate_agrees_with_the_dice() {
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                for stones in STONE_MIN..=STONE_MAX {
                    let Refine::Ready(o) = evaluate(0x04, level, stones, streak) else {
                        panic!("+{level} B={stones} C={streak} should be refinable");
                    };
                    let (success, destroy, reset) = brute_force(level, stones, streak);
                    // 1,000,000 outcomes -> tenths of a percent is a divide by 1000.
                    assert_eq!(
                        u32::from(o.success),
                        success / 1000,
                        "success at +{level} B={stones} C={streak}"
                    );
                    assert_eq!(
                        u32::from(o.reset),
                        u32::from(o.destroy),
                        "the 50:50 split at +{level} B={stones} C={streak}"
                    );
                    // The dice give an exact rate; `evaluate` rounds it to a tenth. They
                    // may differ by at most that rounding (half a tenth = 500 here).
                    let exact_destroy = destroy / 1000;
                    assert!(
                        u32::from(o.destroy).abs_diff(exact_destroy) <= 1,
                        "destroy at +{level} B={stones} C={streak}: printed {} vs dice {exact_destroy}",
                        o.destroy
                    );
                    assert!(
                        (destroy * 2).abs_diff(reset * 2) == 0,
                        "destroy and reset must be the same count"
                    );
                }
            }
        }
    }

    /// The two facts the readout states as advice, re-checked against the dice rather
    /// than against the formula: at [`guaranteed_stones`] the simulation never once
    /// reaches roll 2.
    #[test]
    fn the_advertised_safe_count_never_rolls_for_doom() {
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                let need = guaranteed_stones(level, streak);
                let (success, destroy, reset) = brute_force(level, need, streak);
                assert_eq!(success, 1_000_000, "+{level} C={streak}: not certain at {need}");
                assert_eq!(destroy, 0);
                assert_eq!(reset, 0);
            }
        }
    }
}

/// Properties that must hold for EVERY input, because this readout is the number a
/// player will bet an irreplaceable item on.
#[cfg(test)]
mod properties {
    use super::*;

    /// ⛔⛔ A readout that says "safe" when it is not is worse than no readout — and the
    /// trap is NOT the obvious one. `destroy` reaching 0 does **not** mean the item is
    /// safe: at `+8` with 53 stones the true chance is `0.02 %`, which rounds to a
    /// printed `0.0`. This pins the distinction that [`Odds::safe`] and [`pct_cell`]
    /// exist to keep.
    #[test]
    fn safe_means_impossible_not_merely_small() {
        let mut rounds_to_zero_but_possible = 0u32;
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                for stones in STONE_MIN..=STONE_MAX {
                    let Refine::Ready(o) = evaluate(0x04, level, stones, streak) else {
                        continue;
                    };
                    // `safe` is the strong claim, so it must be earned.
                    if o.safe {
                        assert_eq!(o.doom, 0, "+{level} B={stones}: safe but breakable");
                        assert_eq!(o.destroy, 0);
                        assert_eq!(o.reset, 0);
                        assert!(
                            o.success == FULL || o.doom_if_fail == 0,
                            "+{level} B={stones}: safe without a reason"
                        );
                        assert_eq!(pct_cell(o.destroy, o.safe), "0");
                    }
                    if o.success == FULL {
                        assert!(o.safe, "+{level} B={stones}: certain but not marked safe");
                    }
                    // …and the weak case must never be printed as a flat zero.
                    if o.destroy == 0 && !o.safe {
                        rounds_to_zero_but_possible += 1;
                        assert_eq!(
                            pct_cell(o.destroy, o.safe),
                            "<0.1",
                            "+{level} B={stones}: 0.0 % printed for a real chance"
                        );
                    }
                }
            }
        }
        // The case is real and reachable — if this ever goes to zero the test above has
        // stopped testing anything.
        assert!(rounds_to_zero_but_possible > 0, "the rounding case vanished");
    }

    /// The three headline numbers are a partition: what is left over after success and
    /// doom is a plain failure, and it can never be negative.
    #[test]
    fn the_outcomes_partition_certainty() {
        for level in 0..MAX_LEVEL {
            for streak in [0u8, 1, 3] {
                for stones in STONE_MIN..=STONE_MAX {
                    let Refine::Ready(o) = evaluate(0x04, level, stones, streak) else {
                        continue;
                    };
                    let accounted = u32::from(o.success) + u32::from(o.destroy) + u32::from(o.reset);
                    assert!(
                        accounted <= u32::from(FULL) + 1,
                        "+{level} B={stones}: outcomes sum to {accounted}"
                    );
                }
            }
        }
    }

    /// Cost does not depend on the stones — the player cannot buy a cheaper attempt,
    /// only a safer one. (`base+0x418` is written from `n` alone.)
    #[test]
    fn cost_tracks_level_only() {
        for level in 0..MAX_LEVEL {
            let want = cost(level);
            for stones in STONE_MIN..=STONE_MAX {
                for streak in [0u8, 1, 3] {
                    let Refine::Ready(o) = evaluate(0x04, level, stones, streak) else {
                        continue;
                    };
                    assert_eq!(o.cost, want, "+{level} B={stones} C={streak}");
                }
            }
        }
        // The run the player is really asking about.
        let run: u32 = (0..MAX_LEVEL).map(cost).sum();
        assert_eq!(run, 115_500);
    }

    /// Every string the readout can print must exist in the game's own font, which is
    /// KS X 1001 완성형 only (CLAUDE.md: cp949 extensions are silently skipped).
    /// Checked here as a Hangul-syllable-block range test — `gamefont`'s own vocabulary
    /// test in `gamefont_selftest.rs` is what proves the glyphs are actually there.
    #[test]
    fn readout_vocabulary_is_plain_hangul() {
        for word in ["성공", "파괴", "확정", "제련도", "제련할", "수", "없습니다", "개"] {
            for ch in word.chars() {
                assert!(
                    ('\u{ac00}'..='\u{d7a3}').contains(&ch),
                    "{word}: {ch} is not a Hangul syllable"
                );
            }
        }
    }
}
