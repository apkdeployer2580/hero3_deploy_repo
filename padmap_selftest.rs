// Standalone selftest for `wie_android/src/padmap.rs` — the pure half of the gear's
// 「패드 버튼 설정」 screen (2026-10-09). `include!`s the REAL source.
//
//   rustc --edition 2021 --test padmap_selftest.rs -o t.exe; .\t.exe
//
// The pad here is a stand-in with hero3's shape (OK on A + the hat's centre, CLR on B + START
// + BACK, five plates on X / Y / R1 / R2 / L2, one plate and the map unbound) — the rules are
// the file's, not the game's, which is why the same file is shared with hero2.

#[allow(dead_code)]
mod padmap {
    include!("wie_android/src/padmap.rs");
}

use padmap::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Ok,
    Clr,
    Q(u8),
    Map,
}

const A: i32 = 96;
const B: i32 = 97;
const X: i32 = 99;
const Y: i32 = 100;
const L1: i32 = 102;
const R1: i32 = 103;
const L2: i32 = 104;
const R2: i32 = 105;
const L3: i32 = 106;
const R3: i32 = 107;
const START: i32 = 108;
const SELECT: i32 = 109;
const CENTER: i32 = 23;
const BACK: i32 = 4;

const KNOWN: [i32; 18] = [A, B, X, Y, L1, R1, L2, R2, SELECT, START, L3, R3, 19, 20, 21, 22, CENTER, BACK];
const SOFT: [i32; 2] = [CENTER, BACK];
const ESSENTIAL: [Act; 2] = [Act::Ok, Act::Clr];

fn default(code: i32) -> Option<Act> {
    Some(match code {
        A | CENTER => Act::Ok,
        B | START | BACK => Act::Clr,
        X => Act::Q(0),
        Y => Act::Q(1),
        R1 => Act::Q(2),
        R2 => Act::Q(3),
        L2 => Act::Q(4),
        SELECT => Act::Map,
        _ => return None,
    })
}

fn pad() -> Pad<'static, Act> {
    Pad { known: &KNOWN, soft: &SOFT, essential: &ESSENTIAL, default }
}

fn token(a: Option<Act>) -> String {
    match a {
        None => "NONE".into(),
        Some(Act::Ok) => "OK".into(),
        Some(Act::Clr) => "CLR".into(),
        Some(Act::Q(n)) => format!("Q{}", n + 1),
        Some(Act::Map) => "*".into(),
    }
}

#[test]
fn a_conflict_swaps_the_two_buttons() {
    let p = pad();
    let mut ov = HashMap::new();
    assert_eq!(p.assign(&mut ov, Act::Q(0), Y), Assign::Done { displaced: Some(Act::Q(1)), to: Some(X) });
    assert_eq!(p.effective(&ov, Y), Some(Act::Q(0)));
    assert_eq!(p.effective(&ov, X), Some(Act::Q(1)));
    // Nothing else moved.
    assert_eq!(p.effective(&ov, R1), Some(Act::Q(2)));
    assert_eq!(ov.len(), 2);
}

#[test]
fn swapping_back_leaves_nothing_to_store() {
    let p = pad();
    let mut ov = HashMap::new();
    p.assign(&mut ov, Act::Q(0), Y);
    p.assign(&mut ov, Act::Q(0), X);
    assert!(ov.is_empty(), "back to the built-in layout must store nothing: {ov:?}");
}

#[test]
fn pressing_the_button_that_already_does_it_changes_nothing() {
    let p = pad();
    let mut ov = HashMap::new();
    assert_eq!(p.assign(&mut ov, Act::Q(0), X), Assign::Same);
    // A second button that also fires the action counts as "already does it" too.
    assert_eq!(p.assign(&mut ov, Act::Ok, CENTER), Assign::Same);
    assert!(ov.is_empty());
}

/// Only the PRIMARY button takes part in a swap: START and BACK keep cancelling after the
/// player moves CLR to R3, and the swap goes to B.
#[test]
fn only_the_primary_button_swaps() {
    let p = pad();
    let mut ov = HashMap::new();
    assert_eq!(p.assign(&mut ov, Act::Clr, R3), Assign::Done { displaced: None, to: Some(B) });
    assert_eq!(p.effective(&ov, R3), Some(Act::Clr));
    assert_eq!(p.effective(&ov, B), None);
    assert_eq!(p.effective(&ov, START), Some(Act::Clr));
    assert_eq!(p.effective(&ov, BACK), Some(Act::Clr));
}

/// The row and the HUD badge name the button the player chose, not the first built-in one
/// in the pad's physical order (START comes before R3 there).
#[test]
fn a_chosen_button_is_listed_first() {
    let p = pad();
    let mut ov = HashMap::new();
    p.assign(&mut ov, Act::Clr, R3);
    assert_eq!(p.codes_for(&ov, Act::Clr), vec![R3, START, BACK]);
    assert_eq!(p.codes_for(&HashMap::new(), Act::Clr), vec![B, START, BACK]);
    assert_eq!(p.codes_for(&HashMap::new(), Act::Ok), vec![A, CENTER]);
}

#[test]
fn an_action_with_no_button_takes_the_pressed_one_and_its_old_action_loses_it() {
    let p = pad();
    let mut ov = HashMap::new();
    assert!(p.codes_for(&ov, Act::Q(5)).is_empty());
    assert_eq!(p.assign(&mut ov, Act::Q(5), SELECT), Assign::Done { displaced: Some(Act::Map), to: None });
    assert_eq!(p.codes_for(&ov, Act::Q(5)), vec![SELECT]);
    assert!(p.codes_for(&ov, Act::Map).is_empty(), "the map had one button and gave it up");
}

/// The one way a remap could lock the player out: giving an UNBOUND action the only real OK
/// button. The hat's centre does not count — on most pads the d-pad is an axis and that code
/// never arrives.
#[test]
fn an_essential_action_cannot_be_stranded() {
    let p = pad();
    let mut ov = HashMap::new();
    assert_eq!(p.assign(&mut ov, Act::Q(5), A), Assign::Refused { lost: Act::Ok });
    assert!(ov.is_empty(), "a refusal changes nothing");
    // CLR has START, a real button, as well as B — taking B is fine.
    assert_eq!(p.assign(&mut ov, Act::Q(5), B), Assign::Done { displaced: Some(Act::Clr), to: None });
    assert_eq!(p.codes_for(&ov, Act::Clr), vec![START, BACK]);
    // …and now START is the last real one.
    assert_eq!(p.assign(&mut ov, Act::Q(6), START), Assign::Refused { lost: Act::Clr });
}

/// A soft code counts once the PLAYER put the action there: then it is a real button on
/// their pad.
#[test]
fn a_soft_code_the_player_chose_counts() {
    let p = pad();
    let mut ov = HashMap::new();
    // Some pads send BACK from the button where others send B. The player binds OK to it…
    assert!(matches!(p.assign(&mut ov, Act::Ok, BACK), Assign::Done { .. }));
    // …and can then give A away: OK still has BACK, which they chose.
    assert!(matches!(p.assign(&mut ov, Act::Q(5), A), Assign::Done { .. }));
    assert_eq!(p.codes_for(&ov, Act::Ok)[0], BACK);
}

#[test]
fn a_swap_never_strands_anything() {
    // Every assignment of every action to every capturable code, from the built-in layout:
    // whatever is displaced lands somewhere unless the action had no button to give.
    let p = pad();
    let actions = [Act::Ok, Act::Clr, Act::Q(0), Act::Q(1), Act::Q(2), Act::Q(3), Act::Q(4), Act::Q(5), Act::Map];
    for &action in &actions {
        for &code in KNOWN.iter().chain([110, 188].iter()) {
            let mut ov = HashMap::new();
            let had = !p.codes_for(&ov, action).is_empty();
            match p.assign(&mut ov, action, code) {
                Assign::Done { displaced, to } => {
                    assert_eq!(p.codes_for(&ov, action)[0], code, "{action:?} <- {code}");
                    if had {
                        assert!(to.is_some());
                        if let Some(d) = displaced {
                            assert!(!p.codes_for(&ov, d).is_empty(), "{d:?} lost every button");
                        }
                    }
                    for e in ESSENTIAL {
                        assert!(!p.codes_for(&ov, e).is_empty(), "{e:?} stranded by {action:?} <- {code}");
                    }
                }
                Assign::Same | Assign::Refused { .. } => assert!(ov.is_empty()),
                Assign::NotCapturable => assert!((19..=22).contains(&code)),
            }
        }
    }
}

#[test]
fn directions_are_not_capturable_and_the_hat_centre_is() {
    for code in 19..=22 {
        assert!(!capturable(code));
        let mut ov = HashMap::new();
        assert_eq!(pad().assign(&mut ov, Act::Q(0), code), Assign::NotCapturable);
    }
    assert!(capturable(CENTER));
    assert!(capturable(A));
    // A vendor key the app has no position for is still a button.
    assert!(capturable(188));
    assert!(!capturable(0));
    assert!(!capturable(-1));
}

#[test]
fn an_unknown_button_is_caught_and_listed_after_the_known_ones() {
    let p = pad();
    let mut ov = HashMap::new();
    assert_eq!(p.assign(&mut ov, Act::Map, 188), Assign::Done { displaced: None, to: Some(SELECT) });
    assert_eq!(p.codes_for(&ov, Act::Map), vec![188]);
    assert_eq!(p.effective(&ov, SELECT), None);
}

#[test]
fn the_file_holds_only_what_differs_and_is_sorted() {
    let p = pad();
    let mut ov = HashMap::new();
    p.assign(&mut ov, Act::Q(0), Y);
    p.assign(&mut ov, Act::Clr, R3);
    // An entry equal to the built-in layout is never written, even if it got into the map.
    ov.insert(A, Some(Act::Ok));
    let text = p.serialize(&ov, "# head\n", token);
    assert_eq!(text, "# head\n97=NONE\n99=Q2\n100=Q1\n107=CLR\n");
}

#[test]
fn the_prompt_counts_down_in_whole_seconds() {
    assert_eq!(seconds_left(0), Some(5));
    assert_eq!(seconds_left(1), Some(5));
    assert_eq!(seconds_left(1_000), Some(4));
    assert_eq!(seconds_left(4_001), Some(1));
    assert_eq!(seconds_left(4_999), Some(1));
    assert_eq!(seconds_left(5_000), None);
    assert_eq!(seconds_left(60_000), None);
}

#[test]
fn a_row_describes_its_buttons_in_one_line() {
    fn name(c: i32) -> String {
        format!("b{c}")
    }
    assert_eq!(describe(&[], name), "없음");
    assert_eq!(describe(&[96], name), "b96");
    assert_eq!(describe(&[97, 108, 4], name), "b97 외 2");
}
