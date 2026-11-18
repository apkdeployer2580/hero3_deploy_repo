// Standalone self-test of the gamepad HUD's pure half (59차).
//
//   rustc --edition 2021 --test padhud_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host (the linked
// winit/android deps abort at startup), so this compiles the REAL source — not a
// copy — as its own test crate. `padhud.rs` is dependency-free for exactly this
// reason; if it ever grows a `use crate::...` this file stops building, which is the
// intended alarm.
//
// ★ Unlike `items_selftest.rs` / `landscape_selftest.rs`, this is NOT a hand-written
// mirror. It includes the source, so it cannot drift from it.

include!("wie_android/src/padhud.rs");

// --------------------------------------------------------------------------
// The keycode -> position table
// --------------------------------------------------------------------------

/// The four face buttons are the whole point of this module: Android reports them by
/// POSITION, so 96 is the bottom button on an Xbox pad, a Switch pad and a PS pad
/// alike. If these four ever get "corrected" to the printed letters, the HUD starts
/// lying on two of the three families.
#[test]
fn face_buttons_map_to_positions_not_to_printed_letters() {
    assert_eq!(pos_for_code(KEYCODE_BUTTON_A), Some(PadPos::Face(FacePos::Down)));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_B), Some(PadPos::Face(FacePos::Right)));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_X), Some(PadPos::Face(FacePos::Left)));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_Y), Some(PadPos::Face(FacePos::Up)));
    // And the four are distinct, which is what makes a diamond glyph readable.
    let mut seen = vec![];
    for c in [KEYCODE_BUTTON_A, KEYCODE_BUTTON_B, KEYCODE_BUTTON_X, KEYCODE_BUTTON_Y] {
        let p = pos_for_code(c).unwrap();
        assert!(!seen.contains(&p), "two face codes share a position: {p:?}");
        seen.push(p);
    }
}

#[test]
fn shoulders_carry_both_a_side_and_a_row() {
    assert_eq!(pos_for_code(KEYCODE_BUTTON_L1), Some(PadPos::Shoulder { right: false, lower: false }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_R1), Some(PadPos::Shoulder { right: true, lower: false }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_L2), Some(PadPos::Shoulder { right: false, lower: true }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_R2), Some(PadPos::Shoulder { right: true, lower: true }));
}

#[test]
fn sticks_middles_and_the_hat() {
    assert_eq!(pos_for_code(KEYCODE_BUTTON_THUMBL), Some(PadPos::StickPress { right: false }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_THUMBR), Some(PadPos::StickPress { right: true }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_SELECT), Some(PadPos::Middle { right: false }));
    assert_eq!(pos_for_code(KEYCODE_BUTTON_START), Some(PadPos::Middle { right: true }));
    // Pads that send BACK from the left middle button land in the same picture.
    assert_eq!(pos_for_code(KEYCODE_BACK), Some(PadPos::Middle { right: false }));
    for c in [KEYCODE_DPAD_UP, KEYCODE_DPAD_DOWN, KEYCODE_DPAD_LEFT, KEYCODE_DPAD_RIGHT, KEYCODE_DPAD_CENTER] {
        assert_eq!(pos_for_code(c), Some(PadPos::Dpad));
    }
}

#[test]
fn unknown_codes_have_no_picture() {
    // Volume, letters, and the gap in the BUTTON block (98) are not pad positions.
    for c in [0, 1, 24, 25, 29, 98, 110, 200, -1] {
        assert_eq!(pos_for_code(c), None, "code {c} should have no badge");
    }
}

/// Every position the table can produce needs its own Korean name — a copy-paste
/// duplicate here would put the wrong button in the DIAG legend, which is the one
/// place a confused player looks.
#[test]
fn every_position_has_a_distinct_korean_name() {
    let all = [
        PadPos::Face(FacePos::Up),
        PadPos::Face(FacePos::Down),
        PadPos::Face(FacePos::Left),
        PadPos::Face(FacePos::Right),
        PadPos::Shoulder { right: false, lower: false },
        PadPos::Shoulder { right: false, lower: true },
        PadPos::Shoulder { right: true, lower: false },
        PadPos::Shoulder { right: true, lower: true },
        PadPos::Middle { right: false },
        PadPos::Middle { right: true },
        PadPos::StickPress { right: false },
        PadPos::StickPress { right: true },
        PadPos::Dpad,
    ];
    let mut names: Vec<&str> = all.iter().map(|p| pos_name_ko(*p)).collect();
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), before, "two positions share a Korean name");
    assert!(names.iter().all(|n| !n.is_empty()));
}

// --------------------------------------------------------------------------
// The preference
// --------------------------------------------------------------------------

#[test]
fn pref_tokens_round_trip_and_garbage_falls_back_to_auto() {
    for p in [PadHud::Auto, PadHud::On, PadHud::Off] {
        assert_eq!(PadHud::parse(p.as_token()), p);
    }
    // Whitespace is tolerated (the file is written by us but may be hand-edited).
    assert_eq!(PadHud::parse("  on\n"), PadHud::On);
    // Anything else is AUTO, never a silent ON.
    for s in ["", "ON", "yes", "1", "auto ", "garbage"] {
        assert_eq!(PadHud::parse(s), PadHud::Auto, "{s:?} should parse as AUTO");
    }
    assert_eq!(PadHud::default(), PadHud::Auto);
}

#[test]
fn the_row_cycles_through_all_three_states() {
    let mut p = PadHud::Auto;
    let mut seen = vec![p];
    for _ in 0..2 {
        p = p.next();
        seen.push(p);
    }
    assert_eq!(seen, vec![PadHud::Auto, PadHud::On, PadHud::Off]);
    assert_eq!(p.next(), PadHud::Auto, "the cycle must close");
}

/// AUTO follows the last input source; ON and OFF ignore it. This is the whole
/// contract the HUD gate rests on.
#[test]
fn auto_follows_the_last_input_source_and_the_others_do_not() {
    assert!(!PadHud::Auto.active(false));
    assert!(PadHud::Auto.active(true));
    assert!(PadHud::On.active(false));
    assert!(PadHud::On.active(true));
    assert!(!PadHud::Off.active(false));
    assert!(!PadHud::Off.active(true));
}

// --------------------------------------------------------------------------
// Badge geometry
// --------------------------------------------------------------------------

#[test]
fn a_badge_sits_on_the_lower_left_rim_of_its_control() {
    // A 100x100 control in the middle of a big window: nothing clamps.
    let b = badge_at(500, 500, 100, 100, 2000, 1000).unwrap();
    // Down-left of the centre (550, 550).
    assert!(b.cx < 550.0, "badge must be LEFT of the control centre");
    assert!(b.cy > 550.0, "badge must be BELOW the control centre");
    // On the rim: the centre-to-centre distance is about the control's radius.
    let d = ((b.cx - 550.0).powi(2) + (b.cy - 550.0).powi(2)).sqrt();
    assert!(d > 40.0 && d < 60.0, "badge centre {d} px from control centre (rad 50)");
    // And it is a fraction of the control, not a second control.
    assert!(b.r >= 12.0 && b.r < 50.0, "badge radius {}", b.r);
}

#[test]
fn a_badge_is_never_bigger_than_the_control_it_marks() {
    // A tiny control: the pixel floor must not produce a badge larger than the face.
    for s in 1..40u32 {
        let b = badge_at(200, 200, s, s, 1000, 1000).unwrap();
        let rad = f64::from(s) / 2.0;
        assert!(b.r <= rad + 0.001, "control {s}px -> badge r {} > control r {rad}", b.r);
        assert!(b.r > 0.0);
    }
}

#[test]
fn a_badge_scales_with_its_control() {
    let small = badge_at(0, 0, 60, 60, 4000, 4000).unwrap();
    let big = badge_at(0, 0, 240, 240, 4000, 4000).unwrap();
    assert!(big.r > small.r, "a bigger control gets a bigger badge");
}

/// A control parked in a corner still shows its whole badge: the clamp pulls it
/// inside instead of letting it fall off the screen (which is what the layout editor
/// makes possible — a player can drag a plate to the very edge).
#[test]
fn a_badge_is_clamped_inside_the_window() {
    for (x, y, w, h, win_w, win_h) in [
        (0u32, 0u32, 100u32, 100u32, 2000u32, 1000u32),
        (1900, 900, 100, 100, 2000, 1000),
        (0, 900, 100, 100, 2000, 1000),
        (1900, 0, 100, 100, 2000, 1000),
    ] {
        let b = badge_at(x, y, w, h, win_w, win_h).unwrap();
        assert!(b.cx - b.r >= -0.001, "badge off the left edge: {b:?}");
        assert!(b.cy - b.r >= -0.001, "badge off the top edge: {b:?}");
        assert!(b.cx + b.r <= f64::from(win_w) + 0.001, "badge off the right edge: {b:?}");
        assert!(b.cy + b.r <= f64::from(win_h) + 0.001, "badge off the bottom edge: {b:?}");
    }
}

#[test]
fn a_zero_sized_control_or_window_has_no_badge() {
    assert_eq!(badge_at(10, 10, 0, 50, 1000, 1000), None);
    assert_eq!(badge_at(10, 10, 50, 0, 1000, 1000), None);
    assert_eq!(badge_at(10, 10, 50, 50, 0, 1000), None);
    assert_eq!(badge_at(10, 10, 50, 50, 1000, 0), None);
}

/// A window smaller than the badge itself must still produce finite, on-screen
/// numbers — `App::new` builds a layout at placeholder sizes before the real window
/// arrives, and every geometry helper in this HUD has to survive that.
#[test]
fn a_window_smaller_than_the_badge_still_yields_sane_numbers() {
    let b = badge_at(0, 0, 8, 8, 10, 10).unwrap();
    assert!(b.r > 0.0 && b.r.is_finite());
    assert!(b.cx >= 0.0 && b.cy >= 0.0 && b.cx.is_finite() && b.cy.is_finite());
}
