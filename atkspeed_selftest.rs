// Standalone self-test of 공격속도 / 스킬사용속도 — the host half of the new stat.
//
//   rustc --edition 2021 --test atkspeed_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host, so this compiles the REAL
// source — not a copy. `atkspeed.rs` is dependency-free for exactly that reason.
//
// The GUEST half (the pose-length scaler and its carry) lives in
// `vendor/wie/wie_core_arm/src/binary_patches/atkspeed.rs` and is covered by
// `cargo test -p wie_core_arm`, which does run here.

include!("wie_android/src/atkspeed.rs");

/// The guest's own gate in `addStat` (`0x4ee48`: `subs r3,r4,#2 / cmp r3,#0x15 / bls`).
fn guest_addstat_acts_on(id: u8) -> bool {
    id.wrapping_sub(2) <= 0x15
}

/// The log line's speed estimate: off is 1.00x, it never goes backwards, and it can never
/// claim more than the measured floor allows.
///
/// ★ 2026-09-24 (132차-B): the ceiling here used to be asserted as 2.00x — a MODEL of the
/// hook that nobody had measured. The pinned-monster harness measured 15 -> 5 probe frames
/// (3.00x) at +300 % and +400 %, and 15 -> 8.47 (1.77x) at the +127 % a single option pair can
/// carry. The estimate is now that table (`ATTACK_FRAMES_MEASURED`).
#[test]
fn the_advertised_speed_is_monotonic_and_capped() {
    assert_eq!(speed_x100_at(0), 100, "off is the game's own speed");
    for pct in 1..=MAX_PCT + 50 {
        let x = speed_x100_at(pct);
        assert!(x >= 100, "pct {pct} made it slower");
        assert!(x <= 300, "pct {pct} claims more than 15 -> 5 frames");
        assert!(x >= speed_x100_at(pct - 1), "pct {pct} went backwards");
    }
    // The points a player will actually meet, as measured.
    assert_eq!(speed_x100_at(100), 166);
    assert_eq!(speed_x100_at(127), 177, "the one +127 option pair of @섬광의군화");
    assert_eq!(speed_x100_at(200), 214);
    assert_eq!(speed_x100_at(300), 300, "the ceiling is already reached here");
    assert_eq!(speed_x100_at(MAX_PCT), 300);
}

#[test]
fn the_stat_id_sits_outside_what_the_guest_understands() {
    // Every id the game itself implements, so that picking one of them would have made an
    // item silently add to somebody else's stat.
    for id in 0x02u8..=0x17 {
        assert!(guest_addstat_acts_on(id), "{id:#04x} is one of the game's own 22 arms");
    }
    assert!(!guest_addstat_acts_on(ATTACK_SPEED_STAT));
    assert!(!guest_addstat_acts_on(0x00) && !guest_addstat_acts_on(0x01));
    // ⛔ `0x04` looks free (its jump-table arm is empty) but it is INSIDE the gate and the
    //   label table still names it — it is not ours to take.
    assert!(guest_addstat_acts_on(0x04));
}

#[test]
fn a_character_with_no_such_option_publishes_nothing() {
    // What every existing save looks like: the worn gear's option pairs are all (0, 0),
    // because no shipped equipment row uses them at all.
    let worn = vec![(0u8, 0i8); WORN_SLOTS * 2];
    assert_eq!(sum_options(&worn), 0);
    assert_eq!(effective_pct(sum_options(&worn), None), 0);
}

#[test]
fn only_our_pairs_are_counted_and_the_total_is_bounded() {
    let worn = [
        (0x02, 40),                  // 최대HP
        (ATTACK_SPEED_STAT as u8, 12),
        (0x11, 5),                   // 쿨타임감소%
        (ATTACK_SPEED_STAT as u8, 8),
        (0x00, 0),                   // an empty pair
    ];
    assert_eq!(sum_options(&worn), 20);
    // Saturating, both ends.
    assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, -128), (ATTACK_SPEED_STAT, 10)]), 0);
    assert_eq!(sum_options(&[(ATTACK_SPEED_STAT, 127); 8]), MAX_PCT);
}

#[test]
fn the_override_file_is_one_line_and_fails_to_nothing() {
    assert_eq!(parse_override("pct 60"), Some(60));
    assert_eq!(parse_override("pct 0"), Some(0));
    assert_eq!(parse_override("junk\npct 30\n"), Some(30));
    assert_eq!(parse_override("rate 100\nas 141\n"), None, "a reaver file arms nothing here");
    assert_eq!(parse_override("pct"), None);
    // The override replaces the equipment sum outright — that is what makes it a test knob
    // and not a bonus on top.
    assert_eq!(effective_pct(25, Some(0)), 0);
    assert_eq!(effective_pct(25, Some(100)), 100);
}

// ── the held attack key (132차-B) ─────────────────────────────────────────────

const IDLE: HoldView = HoldView { idle: true, ui: false, combo_open: None };
/// Acting with nothing to chain: the combo is full, or it is the follow-through (state 4).
const BUSY: HoldView = HoldView { idle: false, ui: false, combo_open: None };
const UI: HoldView = HoldView { idle: false, ui: true, combo_open: None };
/// The swing on `step`, with no next step queued yet.
const fn swing(step: i8) -> HoldView {
    HoldView { idle: false, ui: false, combo_open: Some(step) }
}

/// Drive a hold one worker iteration at a time (16 ms apart, like `FRAME_PACE`) and return
/// the iterations on which it pressed.
fn presses(views: &[HoldView]) -> Vec<usize> {
    let mut hold = AttackHold::new(0, 0);
    let mut out = vec![];
    for (i, v) in views.iter().enumerate() {
        if hold.step(i as u64 * 16, i as u64, *v) {
            out.push(i);
        }
    }
    out
}

/// ⛔ vc401: the iteration that delivered the physical press answered it with a second
/// keydown, because the guest had not run a tick on the first one yet.
#[test]
fn a_hold_never_presses_in_the_iteration_of_the_press() {
    let mut hold = AttackHold::new(1_000, 7);
    assert!(!hold.step(1_000, 7, IDLE), "same iteration, stale idle");
    assert!(!hold.step(5_000, 7, IDLE), "still the same iteration, however late");
    assert!(hold.step(5_016, 8, IDLE), "the next iteration may");
}

/// The feature itself: attack, back to idle, attack again — for as long as the thumb stays.
#[test]
fn a_held_key_re_presses_every_time_the_action_ends() {
    // 0 = the press (queued), then the swing runs for a few iterations, then idle.
    let v = [IDLE, BUSY, BUSY, BUSY, IDLE, BUSY, BUSY, IDLE, IDLE, BUSY];
    // 4: first idle after the swing. 7: idle again (and 33 ms since 4). 8: 16 ms after 7
    // is under ATTACK_REPEAT_MS, so no second press on a still-idle frame.
    assert_eq!(presses(&v), vec![4, 7]);
}

/// A REFUSED press (the game ignored it) is retried, but never faster than the floor.
#[test]
fn a_refused_press_is_retried_at_the_repeat_floor() {
    let v = [IDLE; 12];
    let p = presses(&v);
    assert_eq!(p.first(), Some(&3), "0 is the press, 1 and 2 are under 33 ms");
    for w in p.windows(2) {
        assert!((w[1] - w[0]) as u64 * 16 >= ATTACK_REPEAT_MS, "{p:?} re-presses too fast");
    }
}

/// ⛔⛔ THE REGRESSION: the last page of a dialogue. The thumb goes down while the page is
/// up (the press is what closes it), the field comes back a tick later, and the thumb is
/// still down. Measured in the real game: an OK three frames after the close re-opens the
/// dialogue. The hold must stay silent until the key is released.
#[test]
fn a_hold_that_began_on_a_dialogue_never_presses() {
    let mut v = vec![UI];
    v.extend([IDLE; 40]);
    assert_eq!(presses(&v), Vec::<usize>::new());
}

/// The same for a hold that TALKS: the first OK opens the NPC's dialogue, which spoils the
/// hold before it can press again — and a UI that comes and goes in between still counts.
#[test]
fn a_hold_that_opened_anything_but_an_attack_is_spoiled_until_release() {
    let v = [IDLE, UI, UI, IDLE, IDLE, IDLE, IDLE];
    assert_eq!(presses(&v), Vec::<usize>::new());

    // Attacks first, THEN a box (a quest message after a kill): pressing stops for good.
    let v = [IDLE, BUSY, BUSY, IDLE, BUSY, UI, IDLE, IDLE, IDLE, IDLE];
    let p = presses(&v);
    assert_eq!(p, vec![3]);
    let mut hold = AttackHold::new(0, 0);
    for (i, view) in v.iter().enumerate() {
        hold.step(i as u64 * 16, i as u64, *view);
    }
    assert!(hold.spoiled());

    // A fresh hold (the key was released and pressed again) is clean.
    let mut hold = AttackHold::new(10_000, 100);
    assert!(!hold.spoiled());
    assert!(hold.step(10_050, 101, IDLE));
}

/// ★★★ 132차-C — THE COMBO. Mashing OK makes the game append a step per press while the swing
/// runs (`[2]` -> `[2,3]` -> `[2,3,4]` = 공격 1·2·3, then the 장전). The idle-only rule gave
/// "공격1, 장전, 공격1, 장전". A hold must press once per step while the swing has room.
#[test]
fn a_held_key_chains_the_combo_once_per_step() {
    let v = [
        IDLE,     // 0: the physical press (never answered in its own iteration)
        swing(0), // 1: attack 1 running, room for attack 2 -> chain
        swing(0), // 2: same step, already chained -> nothing
        swing(1), // 3: attack 2 running -> chain attack 3
        swing(1), // 4
        swing(2), // 5: attack 3 -> one press (the game refuses it: the combo is full)
        swing(2), // 6: …and it is not retried on the same step
        BUSY,     // 7: the 장전 / follow-through
        BUSY,     // 8
        IDLE,     // 9: back to idle -> a new combo
        swing(0), // 10: and step 0 chains again
    ];
    assert_eq!(presses(&v), vec![1, 3, 5, 9, 10]);
}

/// ⛔ The combo press is NOT under the 33 ms idle floor: at the top of the stat a pose is one
/// tick (~20 ms), so each step must get its press in the very next iteration.
#[test]
fn chaining_keeps_up_with_one_tick_poses() {
    let v = [IDLE, swing(0), swing(1), swing(2), BUSY, IDLE];
    // 1, 2, 3 are 16 ms apart. 5 is idle, and 80 ms after the idle floor was armed.
    assert_eq!(presses(&v), vec![1, 2, 3, 5]);
}

/// The combo press obeys the same two protections as the idle one.
#[test]
fn a_spoiled_or_brand_new_hold_does_not_chain() {
    // Rule 2: in the iteration of the physical press, not even a combo press.
    let mut hold = AttackHold::new(0, 0);
    assert!(!hold.step(0, 0, swing(0)));
    assert!(hold.step(16, 1, swing(0)), "the next iteration may");
    // Rule 1: a hold that saw a UI never chains, whatever the script says.
    let v = [IDLE, UI, swing(0), swing(1), IDLE];
    assert_eq!(presses(&v), Vec::<usize>::new());
}

/// Busy frames never press, however long the attack key has been held.
#[test]
fn nothing_is_pressed_while_the_player_is_still_acting() {
    let v = [IDLE, BUSY, BUSY, BUSY, BUSY, BUSY, BUSY, BUSY];
    assert_eq!(presses(&v), Vec::<usize>::new());
}

/// The measured table itself must be a table the interpolation can walk: percents rising,
/// lengths never rising, and the two ends where the rest of the file says they are.
#[test]
fn the_measured_table_is_well_formed() {
    let t = ATTACK_FRAMES_MEASURED;
    assert_eq!(t[0], (0, 1500), "0 % is the unhooked attack");
    assert_eq!(t[t.len() - 1].0, MAX_PCT, "the table must reach the cap");
    for w in t.windows(2) {
        assert!(w[0].0 < w[1].0, "percents must rise: {w:?}");
        assert!(w[0].1 >= w[1].1, "a bigger bonus may not make the attack longer: {w:?}");
    }
    // Past the cap the hook clamps, and so does the estimate.
    assert_eq!(attack_frames_x100(MAX_PCT + 1000), attack_frames_x100(MAX_PCT));
}
