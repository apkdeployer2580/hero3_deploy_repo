// Standalone self-test of the host-owned cash-shop buff clock.
//
//   rustc --edition 2021 --test buffs_selftest.rs -o tt_buffs.exe && ./tt_buffs.exe
//
// ★ This `include!`s the REAL source, not a copy — so the encodings and the expiry
// rules under test are exactly the ones the app ships. `wie_android/src/buffs.rs`
// must therefore stay dependency-free (same contract as endpoint.rs / tls_buf.rs /
// minimap_render.rs; see CLAUDE.md).
//
// Why standalone at all: `cargo test -p wie_android` dies at startup on this Windows
// host because of the linked winit/android deps.

include!("wie_android/src/buffs.rs");

// The GENERATED Hangul glyph table, included for one reason: `status_lines` now
// returns Korean, and a syllable with no glyph is drawn as NOTHING rather than as a
// visible error. `hangul_covers_every_status_line` below turns that silent hole into
// a test failure, and it can only do that against the real table.
#[path = "wie_android/src/hangul.rs"]
mod hangul;

#[cfg(test)]
mod hangul_coverage {
    use super::*;

    /// Every syllable `status_lines` can emit must exist in the generated table.
    /// `overlay::draw_hangul` skips an unknown character, so a missing glyph is an
    /// invisible hole in the HUD rather than a crash — exactly the kind of defect
    /// that ships. Covers all three items and all three time formats.
    #[test]
    fn hangul_covers_every_status_line() {
        let mut lines = Vec::new();
        for grant in [Grant::Exp(2), Grant::Exp(4), Grant::LevelRelief(RELIEF_LEVELS)] {
            for ms in [7 * 24 * HOUR_MS, 5 * HOUR_MS + 12 * 60_000, 90_000] {
                let mut b = Buffs::default();
                b.apply_timed(grant, 1_000_000, ms);
                lines.extend(b.status_lines(1_000_000));
            }
        }
        assert!(!lines.is_empty());
        for l in &lines {
            assert!(hangul::covers(l), "no glyph for some syllable of {l:?}");
        }
    }

    /// The quick-save toasts went Korean on 2026-08-08, and they are plain literals in
    /// `emulator.rs` — a crate this standalone test cannot link. So they are repeated here on
    /// purpose: the thing being protected is not the spelling but the fact that
    /// `tools_re/gen_hangul_glyphs.py` was RE-RUN after the words changed. Forget that and
    /// every syllable draws as nothing, which on a device looks like the toast is broken
    /// rather than like a missing glyph.
    ///
    /// If you change a toast string in `emulator.rs`, change it in the generator's `WORDS`,
    /// re-run it, and change it here.
    #[test]
    fn hangul_covers_every_save_toast() {
        for toast in ["저장완료", "저장실패", "저장불가"] {
            assert!(hangul::covers(toast), "no glyph for some syllable of {toast:?}");
        }
    }

    // ── 소켓확장 (2026-08-14) ────────────────────────────────────────────────
    /// The item's HOST effect is gone (44차) — it is delivered into the network
    /// warehouse as an orb and resolved by the server — but the recognition stays, so
    /// that a copy left in a bag by the old shop row says where the working one is.
    /// It must still not collide with any other cash arm: the dispatch in emulator.rs
    /// is an if/else chain, so an overlap would silently shadow one of the two.
    #[test]
    fn socket_expand_is_its_own_arm() {
        assert!(is_socket_expand(0x12, 0x1a));
        assert!(!is_socket_expand(0x12, 0x19), "오브상자 is its own arm, not the socket item");
        assert!(!is_socket_expand(0x0e, 0x1a), "wrong category is not the socket item");
        assert_eq!(grant_for(0x12, IDX_SOCKET_EXPAND), None, "not a timed buff");
        assert!(!is_box(0x12, IDX_SOCKET_EXPAND), "not a random box");
        assert!(!is_level_elixir(0x12, IDX_SOCKET_EXPAND), "not the level elixir");
    }

    // ── 오브상자 (2026-08-22) ────────────────────────────────────────────────
    /// ★★ The 오브상자 pays NOTHING from the host. Player: "오브상자 프리미엄 버프 토스트
    /// 없애줘 그거 왜있는거야? 오브상자는 그냥 오브를 얻는 용도인건데?" — and they were
    /// right: the 2026-08-05 buff pool was a stand-in for a capability we did not have
    /// (dropping a real orb), and 44차 built it. The engine now writes one real cat-0x10
    /// orb per unit into the network warehouse, which is what the item's own description
    /// promises. ⛔ Putting `IDX_ORB_BOX` back into `box_reward` re-adds the substitute.
    #[test]
    fn the_orb_box_pays_no_host_reward() {
        assert!(!is_box(0x12, IDX_ORB_BOX), "the 오브상자 is not a host-paid random box");
        for roll in 0..8u64 {
            assert_eq!(box_reward(0x12, IDX_ORB_BOX, roll), None, "roll {roll} paid something");
        }
        assert_eq!(grant_for(0x12, IDX_ORB_BOX), None, "not a timed buff either");
        assert!(is_orb_box(0x12, IDX_ORB_BOX), "…but it is still recognised, for bag copies");
        assert!(!is_orb_box(0x12, IDX_SOCKET_EXPAND), "and it is a DIFFERENT arm from 소켓확장");
        assert!(!is_orb_box(0x10, IDX_ORB_BOX), "wrong category is not the orb box");
        // The two 비밀의상자 are untouched — they never had an item to drop.
        for idx in [IDX_SECRET_BOX_A, IDX_SECRET_BOX_B] {
            assert!(is_box(0x12, idx), "비밀의상자 {idx:#x} is still a host-paid box");
            assert!(box_reward(0x12, idx, 0).is_some());
        }
    }

    /// The bag toasts fire for an inert copy of the SHOP ROW left in a bag by a
    /// build older than 65차; since 69차 the real item is delivered straight to the
    /// bag, so the honest instruction is to buy it again.
    ///
    /// ★ 70차: Korean now — the ASCII constraint this test used to enforce died with
    /// 61차 (`hangul.rs` covers every KS X 1001 syllable, so a Korean toast cannot
    /// render blank any more), and an English line in a Korean game read like debug
    /// output.
    #[test]
    fn the_bag_toast_is_korean_and_says_what_to_do() {
        for t in [SOCKET_EXPAND_BAG_NOTICE, ORB_BOX_BAG_NOTICE] {
            assert!(!t.is_empty() && t.chars().count() <= 30, "{t:?} has to fit a toast");
            assert!(t.contains("다시 구매"), "{t:?} must tell the player what to do");
            // ⛔ Never again "창고": nothing is delivered there any more (69차).
            assert!(!t.contains("창고"), "{t:?} is out of date");
        }
        assert_ne!(SOCKET_EXPAND_BAG_NOTICE, ORB_BOX_BAG_NOTICE, "the two say which item it was");
    }

    // ── uncommitted cash use / rollback note (2026-08-14) ────────────────────
    /// Round-trip: what the note says must be exactly what comes back, or the
    /// rollback would silently move an expiry.
    #[test]
    fn baseline_round_trips_exactly() {
        let b = CashBaseline {
            buffs: Buffs {
                exp_x2_until: 1_770_000_000_123,
                exp_x4_until: 0,
                relief_until: 42,
            },
            levelups: 3,
        };
        assert_eq!(parse_baseline(&serialise_baseline(&b)), Some(b));
        let empty = CashBaseline::default();
        assert_eq!(parse_baseline(&serialise_baseline(&empty)), Some(empty));
    }

    /// ★ The policy that separates this file from `wie_buffs.txt`: a baseline is
    /// applied by OVERWRITING live state, so an incomplete note must refuse rather
    /// than degrade to zeros — zeros would wipe a week the player paid for. Every
    /// missing or unparsable key is a refusal.
    #[test]
    fn a_damaged_baseline_refuses_instead_of_zeroing() {
        let whole = "exp_x2 5\nexp_x4 6\nrelief 7\nlevelups 2\n";
        assert!(parse_baseline(whole).is_some());
        for drop in ["exp_x2 5", "exp_x4 6", "relief 7", "levelups 2"] {
            let damaged = whole.replace(&format!("{drop}\n"), "");
            assert_eq!(parse_baseline(&damaged), None, "a note missing {drop:?} must refuse");
        }
        assert_eq!(parse_baseline("exp_x2 5\nexp_x4 x\nrelief 7\nlevelups 2\n"), None);
        assert_eq!(parse_baseline("exp_x2 5\nexp_x4 6\nrelief 7\nlevelups -1\n"), None);
        assert_eq!(parse_baseline(""), None);
        // A truncated write (the app died mid-`fs::write`) is the realistic damage.
        assert_eq!(parse_baseline("exp_x2 5\nexp_x4 6\nrel"), None);
    }

    /// Unknown lines are ignored, not fatal — a future key must not make old builds
    /// refuse to roll back, which would re-open the duplication window.
    #[test]
    fn baseline_ignores_unknown_keys() {
        let text = "# note\nexp_x2 5\nsomething 9\nexp_x4 6\nrelief 7\nlevelups 2\n";
        assert_eq!(
            parse_baseline(text),
            Some(CashBaseline {
                buffs: Buffs { exp_x2_until: 5, exp_x4_until: 6, relief_until: 7 },
                levelups: 2,
            })
        );
    }

    /// The duplication the note exists to close, expressed as state: a buff applied
    /// but never saved must not survive the restart, and the baseline is what makes
    /// that true. (The file plumbing is `config.rs`; this pins the arithmetic.)
    #[test]
    fn rolling_back_undoes_an_uncommitted_use() {
        let now = 1_000_000;
        let committed = CashBaseline { buffs: Buffs::default(), levelups: 0 };
        // Player uses 경험의 보주: the note is armed FIRST, then the effect lands.
        let note = serialise_baseline(&committed);
        let mut live = committed.buffs;
        live.apply(Grant::Exp(4), now);
        assert_eq!(live.exp_multiplier(now), 4, "the effect applies immediately while playing");
        // App dies before the save. Startup restores the note.
        let restored = parse_baseline(&note).expect("the note is intact");
        assert_eq!(restored.buffs.exp_multiplier(now), 1, "no buff without the consumption");
        assert_eq!(restored, committed);
    }

    /// A SECOND uncommitted use must not re-arm the note: `arm_cash_baseline` leaves
    /// an existing one alone, so the rollback still reaches all the way back to the
    /// last committed state rather than to the state after use #1.
    #[test]
    fn the_baseline_reaches_past_a_second_use() {
        let now = 1_000_000;
        let committed = CashBaseline { buffs: Buffs::default(), levelups: 0 };
        let note = serialise_baseline(&committed); // armed once, by use #1
        let mut live = committed.buffs;
        live.apply(Grant::Exp(2), now);
        live.apply(Grant::LevelRelief(RELIEF_LEVELS), now);
        assert!(live.any_live(now));
        let restored = parse_baseline(&note).unwrap();
        assert!(!restored.buffs.any_live(now), "both uses roll back, not just the last");
    }

}
