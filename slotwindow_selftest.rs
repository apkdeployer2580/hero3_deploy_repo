//! Standalone selftest for `wie_android/src/slotwindow.rs` — the save-slot WINDOW
//! (130차: twelve real slots behind the game's three-row list).
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the window's rules are kept
//! dependency-free and this file `include!`s **the real source**, not a copy of it.
//! Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2021 --test slotwindow_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! ★ This is an `include!`, so deleting a function in `slotwindow.rs` breaks the build
//! here rather than leaving a green mirror behind.

include!("wie_android/src/slotwindow.rs");

/// Outside-in: the properties that hold for EVERY window, because a wrong map here is a
/// character written into someone else's slot.
#[cfg(test)]
mod properties {
    use super::*;

    #[test]
    fn every_real_slot_is_reachable_and_no_two_rows_share_one() {
        let mut hits = [0u32; TOTAL as usize];
        for first in 0..=FIRST_MAX {
            let rows: Vec<u8> = (0..VISIBLE).map(|g| real_slot(first, g).unwrap()).collect();
            assert!(rows.windows(2).all(|w| w[1] == w[0] + 1), "{first}: contiguous rows");
            for r in rows {
                hits[r as usize] += 1;
            }
        }
        assert!(hits.iter().all(|h| *h > 0), "every real slot shows in some window: {hits:?}");
        assert_eq!(hits[0], 1, "slot 0 is only in window 0");
        assert_eq!(hits[TOTAL as usize - 1], 1, "the last slot is only in the last window");
    }

    #[test]
    fn a_renamed_record_always_lands_on_a_real_slot_the_window_shows() {
        for first in 1..=FIRST_MAX {
            for g in 0..VISIBLE {
                for family in ["GameSave_11_", "SlotSave_11_"] {
                    let name = format!("{family}{g}");
                    let renamed = remap_name(first, &name).expect("a guest row is renamed");
                    let tail: u8 = renamed.rsplit('_').next().unwrap().parse().unwrap();
                    assert_eq!(guest_slot(first, tail), Some(g), "{name} -> {renamed}");
                    assert!(renamed.starts_with(family));
                    // Renaming is not idempotent by construction — the RESULT is a
                    // real-slot name, which the map must leave alone (it is one digit
                    // only when it happens to be `first + g <= 2`, and then it is a row).
                    if tail > GUEST_SLOT_MAX {
                        assert_eq!(remap_name(first, &renamed), None, "{renamed}: a real-slot name is not a row");
                    }
                }
            }
        }
    }

    #[test]
    fn the_host_key_arm_never_forwards_and_never_leaves_the_rows() {
        for first in 0..=FIRST_MAX {
            for cursor in 0..VISIBLE {
                for up in [true, false] {
                    let (f, c) = match on_key(first, cursor, up).expect("a row/window pair is always ours") {
                        KeyMove::Move { cursor: c } => (first, c),
                        KeyMove::Slide { first: f } => (f, cursor),
                        KeyMove::Wrap { first: f, cursor: c } => (f, c),
                    };
                    assert!(f <= FIRST_MAX && c <= GUEST_SLOT_MAX, "({first},{cursor},{up}) -> ({f},{c})");
                    // One press moves the real slot by exactly one, except at the wrap.
                    let before = i32::from(real_slot(first, cursor).unwrap());
                    let after = i32::from(real_slot(f, c).unwrap());
                    let step = if up { -1 } else { 1 };
                    let wrapped = (after - before).abs() == i32::from(TOTAL) - 1;
                    assert!(after - before == step || wrapped, "({first},{cursor},{up}): {before} -> {after}");
                }
            }
        }
    }

    #[test]
    fn row_writes_never_overlap_and_stay_inside_the_arrays() {
        let s = Summary { playtime_ms: 1, class: 2, level: 3, extra: 4 };
        let mut spans: Vec<(u32, u32)> = Vec::new();
        for g in 0..VISIBLE {
            for (a, b) in row_writes(0x1000_0000, g, Some(&s)) {
                spans.push((a, a + b.len() as u32));
            }
        }
        for (i, (a0, a1)) in spans.iter().enumerate() {
            for (j, (b0, b1)) in spans.iter().enumerate() {
                if i != j {
                    assert!(a1 <= b0 || b1 <= a0, "writes {i} and {j} overlap");
                }
            }
        }
        // The packed block `base+0xa3ec .. base+0xa3fb` is exactly 15 bytes of three rows.
        let lo = spans.iter().filter(|(a, _)| *a >= 0x1000_0000 + OCCUPIED_OFF).map(|(a, _)| *a).min().unwrap();
        let hi = spans.iter().filter(|(a, _)| *a >= 0x1000_0000 + OCCUPIED_OFF).map(|(_, b)| *b).max().unwrap();
        assert_eq!((lo, hi), (0x1000_0000 + OCCUPIED_OFF, 0x1000_0000 + LEVEL_OFF + 3), "nothing outside the guest's own 15 bytes");
    }
}
