// Gamepad HUD — 59차 (2026-08-19). What the on-screen controls have to say so a
// player can tell WHICH pad button fires which one, on ANY pad.
//
// ── WHY POSITIONS AND NOT LETTERS ────────────────────────────────────────────
//
// Player, 2026-08-19: "제조사별로 패드가 다르니까 이걸 호환가능하게 하도록 위치로
// 표시하는게 나을거같은데". They are right, and the reason is in Android itself:
// the kernel reports face buttons by POSITION (`BTN_SOUTH/EAST/NORTH/WEST`) and the
// generic key layout maps those to `KEYCODE_BUTTON_A/B/Y/X`. So keycode 96 is "the
// BOTTOM face button" on every pad — printed `A` on an Xbox pad, `B` on a Switch pad
// (Nintendo's letters are mirrored), and `✕` with no letter at all on a PlayStation
// pad. A HUD that printed "A" would therefore be wrong on two of the three families
// the player named. A HUD that draws the POSITION is right on all of them.
//
// Left/right and upper/lower are the two things every pad family agrees on, so that
// is the whole vocabulary here: which of the four face positions, which side +
// which row for the shoulders, which of the two middle buttons, which stick.
//
// ── WHAT LIVES HERE ─────────────────────────────────────────────────────────
//
// The PHYSICAL pad: the Android keycodes, where each one sits under the thumb
// ([`pos_for_code`]), and the badge geometry that puts a position glyph on a
// control ([`badge_at`]). What a code DOES to the guest is `gamepad.rs` — one file
// for the pad, one for the game, and the keycode numbers exist once (gamepad.rs
// re-exports them) so the two tables cannot drift apart.
//
// Dependency-free on purpose: `padhud_selftest.rs` at the repo root `include!`s this
// file, which is the only way to test any of it on the Windows host (CLAUDE.md:
// `cargo test -p wie_android` aborts there). Do not add a `use crate::...`.

// Android `KeyEvent` keycodes. API-frozen constants, not bindgen.
//
// ★ These are POSITIONAL despite their names: Android's `generic.kl` maps
// `BTN_SOUTH -> BUTTON_A`, `BTN_EAST -> BUTTON_B`, `BTN_NORTH -> BUTTON_Y`,
// `BTN_WEST -> BUTTON_X`, so 96 is the bottom button and 99 is the left one on every
// pad whose HID descriptor is standard. That is what makes [`pos_for_code`] a
// lookup and not a per-vendor quirk table.
pub const KEYCODE_BACK: i32 = 4;
pub const KEYCODE_DPAD_UP: i32 = 19;
pub const KEYCODE_DPAD_DOWN: i32 = 20;
pub const KEYCODE_DPAD_LEFT: i32 = 21;
pub const KEYCODE_DPAD_RIGHT: i32 = 22;
pub const KEYCODE_DPAD_CENTER: i32 = 23;
pub const KEYCODE_BUTTON_A: i32 = 96;
pub const KEYCODE_BUTTON_B: i32 = 97;
pub const KEYCODE_BUTTON_X: i32 = 99;
pub const KEYCODE_BUTTON_Y: i32 = 100;
pub const KEYCODE_BUTTON_L1: i32 = 102;
pub const KEYCODE_BUTTON_R1: i32 = 103;
pub const KEYCODE_BUTTON_L2: i32 = 104;
pub const KEYCODE_BUTTON_R2: i32 = 105;
pub const KEYCODE_BUTTON_THUMBL: i32 = 106;
pub const KEYCODE_BUTTON_THUMBR: i32 = 107;
pub const KEYCODE_BUTTON_START: i32 = 108;
pub const KEYCODE_BUTTON_SELECT: i32 = 109;

/// Which of the four face positions. `Down` is Xbox `A` / Switch `B` / PS `✕`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FacePos {
    Up,
    Down,
    Left,
    Right,
}

/// Where a pad control sits, in the only terms every pad family shares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadPos {
    /// One of the four face buttons (drawn as a diamond of four dots, the bound one lit).
    Face(FacePos),
    /// A shoulder button or trigger: which side, and which of the two rows.
    /// `lower` = the trigger (L2/LT/ZL), `!lower` = the bumper (L1/LB/L).
    Shoulder { right: bool, lower: bool },
    /// The two small middle buttons: SELECT/back/`−`/SHARE (left) and START/`+`/OPTIONS (right).
    Middle { right: bool },
    /// Pressing a stick in (L3 / R3).
    StickPress { right: bool },
    /// The d-pad / hat as a whole.
    Dpad,
}

/// Where on the pad this keycode is. `None` = a code this HUD has no picture for
/// (which is also every code the mapping table ignores).
///
/// The order of the arms is the physical layout, not the numeric order, so that a
/// reader can check it against a pad in their hand.
pub fn pos_for_code(code: i32) -> Option<PadPos> {
    Some(match code {
        KEYCODE_DPAD_UP | KEYCODE_DPAD_DOWN | KEYCODE_DPAD_LEFT | KEYCODE_DPAD_RIGHT => PadPos::Dpad,
        // The hat's press-in. Distinct from the four directions so a remap of it can
        // be drawn, even though nothing binds it by default.
        KEYCODE_DPAD_CENTER => PadPos::Dpad,
        KEYCODE_BUTTON_A => PadPos::Face(FacePos::Down),
        KEYCODE_BUTTON_B => PadPos::Face(FacePos::Right),
        KEYCODE_BUTTON_X => PadPos::Face(FacePos::Left),
        KEYCODE_BUTTON_Y => PadPos::Face(FacePos::Up),
        KEYCODE_BUTTON_L1 => PadPos::Shoulder { right: false, lower: false },
        KEYCODE_BUTTON_R1 => PadPos::Shoulder { right: true, lower: false },
        KEYCODE_BUTTON_L2 => PadPos::Shoulder { right: false, lower: true },
        KEYCODE_BUTTON_R2 => PadPos::Shoulder { right: true, lower: true },
        KEYCODE_BUTTON_THUMBL => PadPos::StickPress { right: false },
        KEYCODE_BUTTON_THUMBR => PadPos::StickPress { right: true },
        // Some pads send BACK from the left middle button instead of SELECT.
        KEYCODE_BUTTON_SELECT | KEYCODE_BACK => PadPos::Middle { right: false },
        KEYCODE_BUTTON_START => PadPos::Middle { right: true },
        _ => return None,
    })
}

/// Korean name of a position, for the DIAG legend (a Java dialog, so it can hold
/// Hangul — the overlay's own font is a 5x7 ASCII bitmap and cannot).
///
/// Deliberately says the POSITION and then, in brackets, the label the most common
/// pad prints there: the position is the truth, the letter is the hint.
pub fn pos_name_ko(p: PadPos) -> &'static str {
    match p {
        PadPos::Face(FacePos::Up) => "면버튼 위 (Xbox Y)",
        PadPos::Face(FacePos::Down) => "면버튼 아래 (Xbox A)",
        PadPos::Face(FacePos::Left) => "면버튼 왼쪽 (Xbox X)",
        PadPos::Face(FacePos::Right) => "면버튼 오른쪽 (Xbox B)",
        PadPos::Shoulder { right: false, lower: false } => "왼쪽 어깨 위 (L1/LB)",
        PadPos::Shoulder { right: false, lower: true } => "왼쪽 트리거 (L2/LT)",
        PadPos::Shoulder { right: true, lower: false } => "오른쪽 어깨 위 (R1/RB)",
        PadPos::Shoulder { right: true, lower: true } => "오른쪽 트리거 (R2/RT)",
        PadPos::Middle { right: false } => "가운데 왼쪽 (SELECT)",
        PadPos::Middle { right: true } => "가운데 오른쪽 (START)",
        PadPos::StickPress { right: false } => "왼스틱 누르기 (L3)",
        PadPos::StickPress { right: true } => "오른스틱 누르기 (R3)",
        PadPos::Dpad => "십자키 / 왼스틱",
    }
}

/// 게임패드 HUD preference (`wie_pad_hud.txt`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PadHud {
    /// The last input source wins: a pad press shows the pad HUD, a screen touch
    /// brings the touch HUD back. Default, and the reason there is no timeout —
    /// every switch is something the player just did, so nothing changes on its own.
    #[default]
    Auto,
    /// Always the pad HUD (walk controls hidden even if the pad is put down).
    On,
    /// Never the pad HUD.
    Off,
}

impl PadHud {
    /// File token. Anything else parses back to [`PadHud::Auto`] — a corrupt or
    /// hand-edited file must not change behaviour silently.
    pub fn as_token(self) -> &'static str {
        match self {
            PadHud::Auto => "auto",
            PadHud::On => "on",
            PadHud::Off => "off",
        }
    }

    pub fn parse(s: &str) -> PadHud {
        match s.trim() {
            "on" => PadHud::On,
            "off" => PadHud::Off,
            _ => PadHud::Auto,
        }
    }

    /// The settings row cycles AUTO → ON → OFF → AUTO.
    pub fn next(self) -> PadHud {
        match self {
            PadHud::Auto => PadHud::On,
            PadHud::On => PadHud::Off,
            PadHud::Off => PadHud::Auto,
        }
    }

    /// Is the pad HUD showing? `pad_last` = the most recent input came from a pad.
    pub fn active(self, pad_last: bool) -> bool {
        match self {
            PadHud::Auto => pad_last,
            PadHud::On => true,
            PadHud::Off => false,
        }
    }
}

/// A position badge: a small disc stuck on a control, holding the position glyph.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Badge {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
}

/// Fraction of the control's inscribed radius the badge occupies, and how far out
/// its centre sits. 0.74 puts the badge centre ON the rim, so it reads as stuck to
/// the control (an app-icon badge) instead of floating beside it, and leaves the
/// middle of the face — where the skill art is — alone.
const BADGE_R: f64 = 0.36;
const BADGE_OUT: f64 = 0.74;
/// Never smaller than this many pixels of radius, or the glyph inside stops being a
/// glyph. Menu keys on a 1080-tall window come out around r=35, i.e. a 12.6 px badge,
/// which is why the floor is where it is rather than higher.
const BADGE_MIN: f64 = 12.0;

/// Badge for the control occupying `rect`, placed at its LOWER-LEFT rim and clamped
/// inside the window.
///
/// Lower-left because of how this HUD is arranged: the skill plates climb an arc
/// up-and-LEFT from the action disc and the menu keys run down the left edge, so the
/// down-left diagonal is the one direction that is empty for every control in both
/// sets. A badge that lands on the game is fine (the whole overlay does); a badge
/// that lands on the NEXT control's badge is not.
pub fn badge_at(x: u32, y: u32, w: u32, h: u32, win_w: u32, win_h: u32) -> Option<Badge> {
    if w == 0 || h == 0 || win_w == 0 || win_h == 0 {
        return None;
    }
    let rad = f64::from(w.min(h)) / 2.0;
    // The floor can exceed the control (a tiny control gets a proportionally huge
    // badge), so cap at 55 % of it: a badge must never be mistaken for the control.
    let r = (rad * BADGE_R).max(BADGE_MIN.min(rad * 0.55)).min(rad);
    let cx = f64::from(x) + f64::from(w) / 2.0 - rad * BADGE_OUT;
    let cy = f64::from(y) + f64::from(h) / 2.0 + rad * BADGE_OUT;
    // Clamp so a control at the screen edge still shows its whole badge. `max` before
    // `min` so a window narrower than the badge yields the centre, not a negative.
    let cx = cx.max(r).min(f64::from(win_w) - r).max(0.0);
    let cy = cy.max(r).min(f64::from(win_h) - r).max(0.0);
    Some(Badge { cx, cy, r })
}
