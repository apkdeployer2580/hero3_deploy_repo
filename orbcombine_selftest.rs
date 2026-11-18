//! Standalone selftest for `wie_android/src/orbcombine.rs` — the in-game 오브결합 /
//! 소켓확장 model (2026-08-25 player request: "네트워크창고 말고 인게임에서 결합").
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the socket rules and the list
//! geometry are kept dependency-free and this file `include!`s **the real source**,
//! not a copy of it. Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2021 --test orbcombine_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! ★ This is an `include!`, so deleting a function in `orbcombine.rs` breaks the build
//! here rather than leaving a green mirror behind — the trap `items_selftest.rs` and
//! `landscape_selftest.rs` fell into (CLAUDE.md).

include!("wie_android/src/orbcombine.rs");

/// A second, outside-in pass: the module's own tests check each rule, these check the
/// properties that must hold for EVERY input — because the output of this module is a
/// write into a live character's save.
#[cfg(test)]
mod properties {
    use super::*;

    /// Every (cat, sockets, filled, kind) combination there is, small enough to run
    /// exhaustively — which is the point: a socket write that lands on the wrong pair is
    /// not something a sampled test should be trusted to catch.
    fn every_cell() -> Vec<Cell> {
        let mut out = Vec::new();
        for cat in [0x00u16, 0x01, 0x04, 0x0c, 0x0d, 0x10, 0x12] {
            for sockets in 0..=6u8 {
                for filled in -2i8..=6 {
                    for orb in [-1i8, 0, 7] {
                    for equipped in [false, true] {
                    out.push(Cell {
                        tab: 0,
                        slot: 0,
                        key: 0xabcd_ef01_2345_6789,
                        cat,
                        idx: 3,
                        count: 1,
                        refine: 0,
                        sockets,
                        orb,
                        filled,
                        equipped,
                        name: Vec::new(),
                    });
                    }
                    }
                }
            }
        }
        out
    }

    /// ★★ THE PROPERTY THIS WHOLE FILE EXISTS FOR: a write is emitted only for a pair the
    /// rules accept, it never touches a byte outside the six the save carries, and it can
    /// never push either counter out of range.
    #[test]
    fn no_write_can_produce_an_out_of_range_record() {
        let rec = 0x1234_0000u32;
        // ★ A WORN item must never produce a write, whatever else is true of it — the
        // character's attack cache is not something the host may refresh.
        assert!(every_cell().iter().any(|c| c.equipped), "the corpus must contain worn cells");
        for c in every_cell().iter().filter(|c| c.equipped) {
            for (kind, orb) in [(Kind::Socket, 0x07u16), (Kind::Expand, IDX_SOCKET_EXPAND)] {
                assert!(writes(rec, c, kind, orb).is_empty(), "worn: {c:?} {kind:?}");
                // `Worn` when it is equipment at all; a worn NON-equipment cell is refused
                // one clause earlier, which is equally fine.
                let want = if c.is_equipment() { Refusal::Worn } else { Refusal::NotEquipment };
                assert_eq!(eligible(c, kind), Err(want), "worn: {c:?} {kind:?}");
            }
        }
        for c in every_cell() {
            // ★ The orb index is swept, not fixed: it is the byte the game indexes its
            // table with, unchecked, on every launch.
            for (kind, orb) in [
                (Kind::Socket, 0x00u16),
                (Kind::Socket, 0x07),
                (Kind::Socket, IDX_ORB_MAX),
                (Kind::Socket, IDX_SOCKET_EXPAND),
                (Kind::Socket, 0x10),
                (Kind::Socket, 0x30),
                (Kind::Socket, 0xffff),
                (Kind::Expand, IDX_SOCKET_EXPAND),
            ] {
                let w = writes(rec, &c, kind, orb);
                if kind == Kind::Socket && orb > IDX_ORB_MAX {
                    assert!(w.is_empty(), "{c:?} orb {orb:#x} is outside the table");
                    continue;
                }
                if eligible(&c, kind).is_err() {
                    assert!(w.is_empty(), "{c:?} {kind:?} is ineligible but produced {w:?}");
                    continue;
                }
                assert!(!w.is_empty(), "{c:?} {kind:?} is eligible but wrote nothing");
                for (addr, v) in &w {
                    let off = addr - rec;
                    assert!(
                        [REC_SOCKETS, REC_ORB, REC_FILLED].contains(&off),
                        "{c:?} {kind:?} wrote {off:#x}, which the save does not carry"
                    );
                    if off == REC_SOCKETS {
                        assert!(*v >= 1 && *v <= MAX_SOCKETS, "{c:?}: {v} sockets");
                    }
                    if off == REC_FILLED {
                        assert!(*v as i8 >= 0 && *v <= c.sockets, "{c:?}: filled {v} of {}", c.sockets);
                        assert!(*v <= MAX_SOCKETS);
                    }
                    if off == REC_ORB {
                        assert!(u16::from(*v) <= IDX_ORB_MAX, "{c:?}: orb id {v:#x} is outside the table");
                    }
                }
            }
        }
    }

    /// A combine is never a no-op that still spends the orb: at least one of the bytes it
    /// writes must actually change.
    ///
    /// ★ At least one, not every one: socketing the SAME KIND of orb again writes the same
    /// orb id back, which is the game's own amplify behaviour (`orbStat * (f*f+f)/2` grows
    /// with the count, not with the variety — the 43차-B rule). The FILLED byte still moves.
    #[test]
    fn an_accepted_combine_always_changes_something() {
        let rec = 0;
        for c in every_cell() {
            for (kind, orb) in [(Kind::Socket, 0x07u16), (Kind::Expand, IDX_SOCKET_EXPAND)] {
                let w = writes(rec, &c, kind, orb);
                if w.is_empty() {
                    continue;
                }
                let changed = w.iter().any(|(addr, v)| {
                    let before = match *addr {
                        a if a == REC_SOCKETS => c.sockets,
                        a if a == REC_FILLED => c.filled_u8(),
                        a if a == REC_ORB => c.orb as u8,
                        _ => unreachable!(),
                    };
                    before != *v
                });
                assert!(changed, "{c:?} {kind:?} wrote {w:?} and changed nothing");
            }
        }
    }

    /// Applying a combine to its own result must eventually refuse — i.e. repeated
    /// combining fills the item and then stops, rather than running away.
    #[test]
    fn repeated_combining_converges() {
        for start in 0..=MAX_SOCKETS {
            let mut c = Cell {
                tab: 0,
                slot: 0,
                key: 0xabcd_ef01_2345_6789,
                cat: 0x04,
                idx: 3,
                count: 1,
                refine: 0,
                sockets: start,
                orb: -1,
                filled: 0,
                equipped: false,
                name: Vec::new(),
            };
            // Expand until it refuses — must stop at exactly MAX_SOCKETS.
            let mut n = 0;
            while eligible(&c, Kind::Expand).is_ok() {
                for (addr, v) in writes(0, &c, Kind::Expand, IDX_SOCKET_EXPAND) {
                    assert_eq!(addr, REC_SOCKETS);
                    c.sockets = v;
                }
                n += 1;
                assert!(n <= MAX_SOCKETS, "expansion ran away from {start}");
            }
            assert_eq!(c.sockets, MAX_SOCKETS, "started at {start}");
            // Then fill every socket — must stop at exactly `sockets` fills.
            let mut n = 0;
            while eligible(&c, Kind::Socket).is_ok() {
                for (addr, v) in writes(0, &c, Kind::Socket, 0x03) {
                    match addr {
                        a if a == REC_ORB => c.orb = v as i8,
                        a if a == REC_FILLED => c.filled = v as i8,
                        _ => unreachable!(),
                    }
                }
                n += 1;
                assert!(n <= MAX_SOCKETS, "filling ran away");
            }
            assert_eq!(c.filled, MAX_SOCKETS as i8);
            assert_eq!(c.orb, 0x03, "the LAST orb socketed is the item's orb — the 43차-B rule");
        }
    }

    /// The viewport is always consistent, for every list length and every viewport
    /// height — a selection drawn off screen is a dialog the player cannot use, and a
    /// scroll position past either end is a window with nothing in it.
    #[test]
    fn the_scroll_viewport_is_always_consistent() {
        const ROW_H: u32 = 94;
        for len in 0..40usize {
            for rows in 0..12u32 {
                let view = rows * ROW_H;
                for sel in 0..len.max(1) {
                    // Wherever the drag left it, a key brings the selection into view.
                    for from in [0.0, 37.0, 1e9, -1e9] {
                        let px = scroll_to_show(from, sel, len, ROW_H, view);
                        assert!((0.0..=scroll_span(len, ROW_H, view)).contains(&px), "len {len} rows {rows} sel {sel}: {px} is off the span");
                        if len == 0 {
                            assert_eq!(px, 0.0);
                            continue;
                        }
                        // A zero-height viewport shows nothing, so "which row is under
                        // it" has no answer; the span assertion above is all it owes.
                        if view == 0 {
                            continue;
                        }
                        let top = sel as f64 * f64::from(ROW_H);
                        // A row TALLER than the viewport cannot be wholly shown; the rule
                        // is then "its top edge", which is what `scroll_to_show` returns.
                        if view >= ROW_H {
                            assert!(px <= top, "len {len} rows {rows} sel {sel}: top cut off");
                            assert!(top + f64::from(ROW_H) <= px + f64::from(view), "len {len} rows {rows} sel {sel}: bottom cut off");
                        }
                        // …and the row a tap on its first pixel names is that same row.
                        assert_eq!(row_at(px, top - px, len, ROW_H), Some(sel), "len {len} rows {rows} sel {sel}");
                    }
                }
                // The partial row at the top edge never starts outside the panel.
                for px in [0.0, 1.0, f64::from(ROW_H) / 2.0, scroll_span(len, ROW_H, view)] {
                    let (i, off) = first_visible(px, ROW_H);
                    assert!(off < ROW_H, "len {len} rows {rows}: offset {off} is a whole row");
                    assert!(i as f64 * f64::from(ROW_H) <= px);
                }
            }
        }
    }

    /// Stepping the selection can never leave the list, whatever the step.
    #[test]
    fn stepping_never_leaves_the_list() {
        for len in 0..20usize {
            for sel in 0..len.max(1) {
                for step in [-100i32, -10, -1, 0, 1, 10, 100] {
                    let n = step_sel(sel, len, step);
                    if len == 0 {
                        assert_eq!(n, 0);
                    } else {
                        assert!(n < len, "len {len} sel {sel} step {step} -> {n}");
                    }
                }
            }
        }
    }

    /// ⛔ 소켓확장 and a real orb must never be confused. The pseudo-orb has no stat of
    /// its own, so socketing it would set the item's orb id to 0x0f and read an entry
    /// past the 15-orb table — the exact wild read the 16th-row asset patch exists to
    /// prevent (`binary_patches.toml`).
    #[test]
    fn the_pseudo_orb_never_becomes_an_items_orb() {
        for c in every_cell() {
            for (addr, v) in writes(0, &c, kind_of(IDX_SOCKET_EXPAND), IDX_SOCKET_EXPAND) {
                assert_ne!(addr, REC_ORB, "{c:?}: 소켓확장 was socketed as an orb");
                assert_ne!(v, IDX_SOCKET_EXPAND as u8);
            }
        }
    }

    /// Every player-facing string has to be renderable by the game font, which covers KS
    /// X 1001 only. A cp949 extension syllable would draw as a gap.
    #[test]
    fn every_message_is_inside_the_game_fonts_repertoire() {
        let mut all: Vec<String> = vec![NO_TARGET.into()];
        all.push(FAILED.into());
        for k in [Kind::Socket, Kind::Expand] {
            all.push(title(k).into());
            all.push(done_text(k).into());
        }
        for r in [Refusal::NotEquipment, Refusal::NoSockets, Refusal::NoRoom, Refusal::SocketCap, Refusal::Corrupt] {
            all.push(r.text().into());
        }
        for s in &all {
            assert!(!s.is_empty());
            for ch in s.chars() {
                if (ch as u32) < 0x80 {
                    continue;
                }
                // KS X 1001 완성형 covers AC00..D7A3 only partially; the practical test the
                // repo uses is "the game itself ships this syllable". Keep the check to the
                // Hangul syllable block plus the punctuation the game uses, and rely on
                // `gamefont_selftest.rs`'s vocabulary list for the exact coverage.
                assert!(
                    ('\u{AC00}'..='\u{D7A3}').contains(&ch) || ch == '·',
                    "{s:?} contains {ch:?}, which is not a Hangul syllable"
                );
            }
        }
    }
}
