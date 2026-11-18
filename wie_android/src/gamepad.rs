//! Gamepad (and hardware-keyboard) input → guest phone keys. 58차 (2026-08-18).
//!
//! ## How gamepad input reaches this app — and why it takes TWO routes
//!
//! The app is a GameActivity: input flows Java `GameActivity` → the crate-bundled
//! `android_native_app_glue` input buffer → `android-activity` → winit. That chain
//! was read end-to-end (android-activity 0.6.1 + winit 0.30.13) before this module
//! was written, and it passes exactly ONE of the three things a gamepad produces:
//!
//! * **Button/D-pad `KeyEvent`s** pass. The glue's `default_key_filter` only drops
//!   volume/camera/zoom, and winit forwards every key as `WindowEvent::KeyboardInput`
//!   (D-pad → `KeyCode::Arrow*`, buttons → `PhysicalKey::Unidentified(Android(96..))`).
//! * **Joystick axis `MotionEvent`s do NOT pass.** The glue's `default_motion_filter`
//!   drops every non-`SOURCE_TOUCHSCREEN` motion event, and android-activity 0.6.1
//!   exposes no way to replace that filter. Even if it did, winit would misread the
//!   survivors: it treats every `MotionAction::Move` as a TOUCH and would hand the
//!   stick's -1..1 axis values to the app as touch coordinates.
//!
//! So sticks are captured in Java instead — `MainActivity.dispatchGenericMotionEvent`
//! (the top of the dispatch chain, before any view) reads the axes and calls a JNI
//! native (`lib.rs::gamepad_jni`) that posts a [`crate::UserEvent`] to the winit
//! thread. Buttons are captured the same way (`dispatchKeyEvent`, gamepad/joystick
//! sources only) so ALL pad input takes one owned path with no dependence on the
//! glue's filters or winit's `Unidentified` fallthrough; the winit `KeyboardInput`
//! route stays as the mapping for REAL keyboards and for D-pad-only devices
//! (TV remotes), whose sources Java deliberately leaves alone.
//!
//! ## What the guest can be sent
//!
//! Hero3 is a feature-phone game: 4-way walk (ONE latched direction — see
//! [`crate::overlay::joy_dir`]), OK, CLR, `*` (map), and number keys, of which
//! `1/3/9/0/#` are the five GUEST quickbar slots (`crate::quickslot_hotkey`, static in
//! the game's own dispatcher) and `7` is the secondary weapon. ★ The HUD has more
//! PLATES than that (`quickpage::PLATES`); a pad button reaches whichever plate is
//! currently sitting in its slot, and `QuickSlot::guest` is what says which
//! (`App::slot_for_hotkey`, `overlay::draw_pad_badges`). There is nothing analog to
//! map an analog stick onto, so the stick is quantised through the SAME
//! deadzone+hysteresis the on-screen joystick uses.
//!
//! Mapping is data, not code: [`button_action`] consults the player's override table
//! (`wie_gamepad.txt`, see [`crate::config::read_gamepad_map`]) before the defaults,
//! so any pad layout is one text file away without a rebuild — and since 2026-10-09 one
//! screen away too: 설정 → 패드 버튼 설정 (`App::padmap_*`, rules in `padmap.rs`) writes
//! that same file and swaps the table in at once ([`set_overrides`]).

use std::collections::HashMap;

use wie_backend::KeyCode;

// ★ The keycode NUMBERS live in `padhud.rs`, which also says where each one sits
// under the thumb. One file for the physical pad, one (this) for what a press does
// to the guest — so the mapping table and the badge table cannot drift apart.
pub use crate::padhud::{
    KEYCODE_BACK, KEYCODE_BUTTON_A, KEYCODE_BUTTON_B, KEYCODE_BUTTON_L1, KEYCODE_BUTTON_L2, KEYCODE_BUTTON_R1, KEYCODE_BUTTON_R2,
    KEYCODE_BUTTON_SELECT, KEYCODE_BUTTON_START, KEYCODE_BUTTON_THUMBL, KEYCODE_BUTTON_THUMBR, KEYCODE_BUTTON_X, KEYCODE_BUTTON_Y,
    KEYCODE_DPAD_CENTER, KEYCODE_DPAD_DOWN, KEYCODE_DPAD_LEFT, KEYCODE_DPAD_RIGHT, KEYCODE_DPAD_UP,
};
use crate::padhud::{PadPos, pos_for_code};

/// What a mapped control does when pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadAction {
    /// Press/release a guest phone key.
    Key(KeyCode),
    /// Activate host quickbar PLATE `n` (`InputEvent::QuickActivate`).
    ///
    /// ★★ 79차-B — a pad button names a PLATE, not a guest key. It used to send the
    /// plate's digit (`NUM1`/`NUM3`/`NUM9`/`NUM0`/`HASH`), which only worked while a
    /// plate WAS a guest slot. It is not: there are `quickpage::PLATES` plates over five
    /// guest slots and which one a slot holds moves (`quickpage::Window`), so a digit
    /// would fire whatever happened to be seated there. Naming the plate makes the pad
    /// reach all seven — a digit could only ever reach five — and it goes through the
    /// same worker path as a plate tap, which is what admits a plate that is not
    /// currently in the guest's bar and what watches for a real cast.
    ///
    /// ⛔ This is FIELD-only, like the plates themselves. Off the field a plate does
    /// nothing (see `app.rs`), and the digits a window wants are on the pull-out keypad.
    Quick(u8),
    /// Flip the host quickbar page (`InputEvent::QuickPageNext`) — a host feature,
    /// not a guest key, so it needs its own arm.
    QuickPage,
    /// 빠른저장 (59차): the host's own save-now path (`InputEvent::QuickSave`), the
    /// same one the SAVE button sends. A pad-only player otherwise cannot save
    /// without reaching for the screen — and unlike every other action here there is
    /// no guest key that means "save", so it needs its own arm too.
    QuickSave,
}

/// Every keycode this app can draw or bind, in PHYSICAL order — face buttons,
/// shoulders, middles, sticks, hat. [`pos_for_action`] scans it in this order, so a
/// guest key reachable from two codes is badged with the first: OK is on both the
/// bottom face button and the hat's centre, and the face button is the one a player
/// thinks of.
const ALL_CODES: [i32; 18] = [
    KEYCODE_BUTTON_A,
    KEYCODE_BUTTON_B,
    KEYCODE_BUTTON_X,
    KEYCODE_BUTTON_Y,
    KEYCODE_BUTTON_L1,
    KEYCODE_BUTTON_R1,
    KEYCODE_BUTTON_L2,
    KEYCODE_BUTTON_R2,
    KEYCODE_BUTTON_SELECT,
    KEYCODE_BUTTON_START,
    KEYCODE_BUTTON_THUMBL,
    KEYCODE_BUTTON_THUMBR,
    KEYCODE_DPAD_UP,
    KEYCODE_DPAD_DOWN,
    KEYCODE_DPAD_LEFT,
    KEYCODE_DPAD_RIGHT,
    KEYCODE_DPAD_CENTER,
    KEYCODE_BACK,
];

/// The player's override table: read from `wie_gamepad.txt` at start, and replaced by
/// 설정 → 패드 버튼 설정 when the player saves there (2026-10-09).
///
/// A global and not a field on `App` because the DRAWING path needs it: the HUD
/// badges are painted by free functions in `overlay.rs` that have no `App` in hand,
/// exactly like [`crate::stick_mode`] and [`crate::immersive_active`]. One store, so
/// what a button DOES and what its badge SAYS can never disagree.
///
/// ★ A lock, not the `OnceLock` it was until 2026-10-09: the table now changes while the
/// app runs. Every reader and the one writer are on the UI thread, so it is never
/// contended — the lock is there to make the global sound, not to arbitrate.
static OVERRIDES: std::sync::OnceLock<std::sync::RwLock<HashMap<i32, Option<PadAction>>>> = std::sync::OnceLock::new();

fn store() -> &'static std::sync::RwLock<HashMap<i32, Option<PadAction>>> {
    OVERRIDES.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

/// Install the override table — at start from the file, and again whenever the pad
/// screen saves. The next press and the next drawn badge both see it.
pub fn set_overrides(map: HashMap<i32, Option<PadAction>>) {
    *store().write().unwrap_or_else(|e| e.into_inner()) = map;
}

/// A copy of the table in force — the pad screen edits a copy and installs it on 저장.
pub fn overrides_snapshot() -> HashMap<i32, Option<PadAction>> {
    store().read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn with_overrides<T>(f: impl FnOnce(&HashMap<i32, Option<PadAction>>) -> T) -> T {
    f(&store().read().unwrap_or_else(|e| e.into_inner()))
}

/// Does any button do something other than the built-in layout says? (The settings row's
/// 「사용자 배치」, and what keeps the pad rows on the panel for a player who changed them.)
pub fn remapped() -> bool {
    with_overrides(|ov| ov.iter().any(|(code, over)| *over != default_button_action(*code)))
}

/// Default button layout. Chosen around how the game is actually played: A confirms
/// and attacks (OK), B cancels and opens/closes the menu (CLR — the game uses one key
/// for both), the four remaining face/shoulder buttons are quickbar slots 1-4 in HUD
/// order, the shoulders carry the MENU keys (LB `7`, RB `9`, LT `#`, RT `0`),
/// SELECT the map (`*`),
/// START the menu, R3 flips the quickbar page and L3 saves.
pub fn default_button_action(code: i32) -> Option<PadAction> {
    Some(match code {
        KEYCODE_DPAD_UP => PadAction::Key(KeyCode::UP),
        KEYCODE_DPAD_DOWN => PadAction::Key(KeyCode::DOWN),
        KEYCODE_DPAD_LEFT => PadAction::Key(KeyCode::LEFT),
        KEYCODE_DPAD_RIGHT => PadAction::Key(KeyCode::RIGHT),
        KEYCODE_DPAD_CENTER | KEYCODE_BUTTON_A => PadAction::Key(KeyCode::OK),
        // BACK here is the gamepad-source BACK some pads send for B/SELECT — the
        // phone's own back key is not routed through this table (see module docs).
        KEYCODE_BUTTON_B | KEYCODE_BUTTON_START | KEYCODE_BACK => PadAction::Key(KeyCode::CLEAR),
        KEYCODE_BUTTON_X => PadAction::Quick(0),
        KEYCODE_BUTTON_Y => PadAction::Quick(1),
        // ★ 2026-08-20, the player's own assignment: "메뉴오픈시 좌측상단 7번이 LB,
        // 9번 RB, #은 LT". Those three were the keys a MENU needed (page left / page
        // right / confirm-register), so they went on the shoulders where a thumb does
        // not have to leave the stick.
        //
        // ★★ 79차-B: the five that were quickslot DIGITS are now `Quick(0..=4)` — the
        // plate, not the key (see `PadAction::Quick`). The seats are unchanged, so a
        // player's muscle memory is: X/Y = plates 1·2, R1 = plate 3, R2 = plate 4,
        // L2 = plate 5. L1 keeps `7`, which is the SECONDARY WEAPON and a real guest
        // key, not a plate.
        //
        // Plates 6 and 7 have NO default button: a standard pad has no seat left that
        // is not already something. They are one line of `wie_gamepad.txt` away
        // (`<keycode>=Q6`), and that is the point of the token — a digit could never
        // have addressed them at all.
        KEYCODE_BUTTON_L1 => PadAction::Key(KeyCode::NUM7),
        KEYCODE_BUTTON_R1 => PadAction::Quick(2),
        // ★ SWAPPED 2026-08-20 (player request: "게임패드에서 7번과 #의 받는 게임패드
        // 입력키를 서로 변경해줘"). L2 was `#` (quickslot 5 / 등록) and R2 was `7`
        // (보조무기); they are now the other way round. Nothing else needs touching —
        // the on-screen badges are a REVERSE lookup of this table
        // (`overlay::draw_pad_badges` -> `gamepad::pos_for_key`), so they follow, and a
        // player who had already remapped either key in `wie_gamepad.txt` keeps their
        // own binding (the file is consulted first).
        KEYCODE_BUTTON_L2 => PadAction::Quick(4),
        KEYCODE_BUTTON_R2 => PadAction::Quick(3),
        KEYCODE_BUTTON_SELECT => PadAction::Key(KeyCode::STAR),
        KEYCODE_BUTTON_THUMBR => PadAction::QuickPage,
        // ★ 59차: the LEFT stick's press-in was the one control on a standard pad that
        // nothing used. 빠른저장 is the host feature a pad-only player most needs and
        // cannot otherwise reach (the SAVE button is touch), and pressing a stick in is
        // hard to do by accident — which matters for a key that writes to the save file.
        KEYCODE_BUTTON_THUMBL => PadAction::QuickSave,
        _ => return None,
    })
}

/// The guest DIGIT a plate button sends while a window is open (81차, player request
/// 2026-09-06: *"게임패드일때만 L1, R1 등 패드버튼으로 1,3,7,9,# 동작하게"*).
///
/// On the field a pad button names a PLATE (79차-B). Off the field a plate means
/// nothing — the touch plates stay that way — but a pad has no pull-out keypad to
/// reach for, so the buttons fall back to the digits they sent BEFORE 79차-B: the
/// game's own «1»«3» tab badges, «9» next page and `0`/`#`. This is a FIXED table
/// keyed by the plate number, not by what happens to be seated in the guest's bar,
/// which is what keeps menu navigation predictable (the 79차-B ⛔ is about routing
/// through residency, not about a constant table). Plates 6·7 have no digit: `None`.
///
/// Default seats: X = `1`, Y = `3`, R1 = `9`, R2 = `0`, L2 = `#`; L1 is `7` outright.
pub fn menu_digit(plate: u8) -> Option<KeyCode> {
    Some(match plate {
        0 => KeyCode::NUM1,
        1 => KeyCode::NUM3,
        2 => KeyCode::NUM9,
        3 => KeyCode::NUM0,
        4 => KeyCode::HASH,
        _ => return None,
    })
}

/// Resolve an Android keycode to its action: the player's `wie_gamepad.txt` override
/// first, the built-in layout second.
pub fn button_action(code: i32) -> Option<PadAction> {
    with_overrides(|ov| button_action_in(code, ov))
}

/// [`button_action`] against an explicit table — the testable half. An override to
/// `NONE` (stored as no entry — see [`parse_map_line`]) cannot be expressed by the
/// built-in table, so overrides map to `Option<PadAction>`: `Some(None)` =
/// explicitly unmapped.
pub fn button_action_in(code: i32, overrides: &HashMap<i32, Option<PadAction>>) -> Option<PadAction> {
    match overrides.get(&code) {
        Some(over) => *over,
        None => default_button_action(code),
    }
}

/// Which pad position fires `action`, under the override table in force — the reverse
/// of [`button_action`], and the whole basis of the HUD badges (59차).
///
/// Reverse and not a second table: a `wie_gamepad.txt` remap then shows up on the HUD
/// for free, and a badge can never claim a button that does something else.
///
/// ★ 2026-10-09: a button the PLAYER chose comes first (`padmap::Pad::codes_for`). The
/// old scan in physical order badged CLR on START after the player moved it to R3,
/// because START sits earlier in [`ALL_CODES`] and still cancels by default.
pub fn pos_for_action(action: PadAction) -> Option<PadPos> {
    with_overrides(|ov| pos_for_action_in(ov, action))
}

/// [`pos_for_action`] under an explicit table — the pad screen badges its DRAFT, the table
/// being edited, not the one in force.
pub fn pos_for_action_in(ov: &HashMap<i32, Option<PadAction>>, action: PadAction) -> Option<PadPos> {
    pad().codes_for(ov, action).into_iter().find_map(pos_for_code)
}

// ── 패드 버튼 설정 (2026-10-09) — what this game hands `padmap.rs` ──────────────────

/// Codes that are a second way to send something rather than a button of their own on
/// most pads: the hat's centre (OK) and the gamepad-source BACK some pads send for B or
/// SELECT (CLR). See `padmap::Pad::soft`.
const SOFT_CODES: [i32; 2] = [KEYCODE_DPAD_CENTER, KEYCODE_BACK];

/// The two the game cannot be played without — a remap may never leave either with no
/// button (`padmap::Pad::assign`).
const ESSENTIAL: [PadAction; 2] = [PadAction::Key(KeyCode::OK), PadAction::Key(KeyCode::CLEAR)];

/// This game's pad, for `padmap.rs`.
pub fn pad() -> crate::padmap::Pad<'static, PadAction> {
    crate::padmap::Pad {
        known: &ALL_CODES,
        soft: &SOFT_CODES,
        essential: &ESSENTIAL,
        default: default_button_action,
    }
}

const _: () = assert!(crate::quickpage::PLATES == 7, "REMAP_ROWS lists seven plates");

/// The functions of 설정 → 패드 버튼 설정, in the order the DIAG legend lists them. Every
/// action a button can be given — the walk is not here (`padmap::capturable`).
///
/// ★ Second form (2026-10-09): the screen offers each of these ON THE IN-GAME HUD — the
/// control that does it in play, lit where it is played (`overlay::padmap_scene`); a
/// function with no control on screen (a hidden plate) is a cell in the screen's window.
/// The index into this table is what a target carries ([`remap_row`]).
///
/// ⛔ A plate's label is its number as the HUD prints it (`Q1`..`Q7` on the plates), and
/// off the field the first five also send the menu digits `1·3·9·0·#` (81차,
/// [`menu_digit`]) — they follow the plate, so moving 퀵슬롯 1 moves «1» with it.
pub const REMAP_ROWS: [(PadAction, &str); 13] = [
    (PadAction::Key(KeyCode::OK), "확인 / 공격"),
    (PadAction::Key(KeyCode::CLEAR), "취소 / 메뉴"),
    (PadAction::Quick(0), "퀵슬롯 1"),
    (PadAction::Quick(1), "퀵슬롯 2"),
    (PadAction::Quick(2), "퀵슬롯 3"),
    (PadAction::Quick(3), "퀵슬롯 4"),
    (PadAction::Quick(4), "퀵슬롯 5"),
    (PadAction::Quick(5), "퀵슬롯 6"),
    (PadAction::Quick(6), "퀵슬롯 7"),
    (PadAction::Key(KeyCode::NUM7), "보조무기 (7)"),
    (PadAction::Key(KeyCode::STAR), "지도 (*)"),
    (PadAction::QuickPage, "퀵바 페이지"),
    (PadAction::QuickSave, "빠른저장"),
];

/// The [`REMAP_ROWS`] index of an action — what the pad screen's targets carry.
pub fn remap_row(action: PadAction) -> Option<usize> {
    REMAP_ROWS.iter().position(|(a, _)| *a == action)
}

/// The `wie_gamepad.txt` token for an action — [`parse_map_line`] read backwards.
pub fn token(action: Option<PadAction>) -> String {
    let Some(action) = action else {
        return "NONE".to_string();
    };
    match action {
        PadAction::Quick(n) => format!("Q{}", n + 1),
        PadAction::QuickPage => "PAGE".to_string(),
        PadAction::QuickSave => "SAVE".to_string(),
        PadAction::Key(k) => match k {
            KeyCode::UP => "UP",
            KeyCode::DOWN => "DOWN",
            KeyCode::LEFT => "LEFT",
            KeyCode::RIGHT => "RIGHT",
            KeyCode::OK => "OK",
            KeyCode::CLEAR => "CLR",
            KeyCode::NUM0 => "0",
            KeyCode::NUM1 => "1",
            KeyCode::NUM2 => "2",
            KeyCode::NUM3 => "3",
            KeyCode::NUM4 => "4",
            KeyCode::NUM5 => "5",
            KeyCode::NUM6 => "6",
            KeyCode::NUM7 => "7",
            KeyCode::NUM8 => "8",
            KeyCode::NUM9 => "9",
            KeyCode::HASH => "#",
            KeyCode::STAR => "*",
            // Keys `parse_map_line` has no token for are never bound by this app — the
            // screen offers only `REMAP_ROWS`. Written as switched off rather than as a
            // line the reader would skip, which would quietly bring the default back.
            _ => "NONE",
        }
        .to_string(),
    }
}

/// A button as the pad screen names it: its POSITION, in the DIAG legend's words
/// (`padhud::pos_name_ko`) — except the two codes that share a position with something
/// else there, and a button the app has no position for, which is named by its keycode so
/// the player can still tell two of them apart.
///
/// ⛔ Not `pos_name_ko` itself: that is drawn by a Java dialog, and its `(Xbox Y)` has
/// LOWERCASE letters — the game font's lowercase cells are symbols (`gamefont_selftest`'s
/// drawer test), so `box` would come out as garbage on this screen. Same words, uppercase.
pub fn button_name(code: i32) -> String {
    use crate::padhud::{FacePos, PadPos};
    let name = match code {
        KEYCODE_DPAD_CENTER => "십자키 가운데",
        KEYCODE_BACK => "뒤로 (BACK)",
        _ => match pos_for_code(code) {
            Some(PadPos::Face(FacePos::Up)) => "면버튼 위 (XBOX Y)",
            Some(PadPos::Face(FacePos::Down)) => "면버튼 아래 (XBOX A)",
            Some(PadPos::Face(FacePos::Left)) => "면버튼 왼쪽 (XBOX X)",
            Some(PadPos::Face(FacePos::Right)) => "면버튼 오른쪽 (XBOX B)",
            Some(PadPos::Shoulder { right: false, lower: false }) => "왼쪽 어깨 (L1/LB)",
            Some(PadPos::Shoulder { right: false, lower: true }) => "왼쪽 트리거 (L2/LT)",
            Some(PadPos::Shoulder { right: true, lower: false }) => "오른쪽 어깨 (R1/RB)",
            Some(PadPos::Shoulder { right: true, lower: true }) => "오른쪽 트리거 (R2/RT)",
            Some(PadPos::Middle { right: false }) => "가운데 왼쪽 (SELECT)",
            Some(PadPos::Middle { right: true }) => "가운데 오른쪽 (START)",
            Some(PadPos::StickPress { right: false }) => "왼스틱 누르기 (L3)",
            Some(PadPos::StickPress { right: true }) => "오른스틱 누르기 (R3)",
            Some(PadPos::Dpad) => "십자키",
            None => return format!("버튼 {code}"),
        },
    };
    name.to_string()
}

/// The header `wie_gamepad.txt` is written with. Comment lines only — the reader skips
/// them (`parse_map_line`).
pub const MAP_FILE_HEADER: &str = "# 설정 → 패드 버튼 설정 이 쓴 파일 (기본 배치와 다른 버튼만).\n# <android keycode>=<UP DOWN LEFT RIGHT OK CLR 0-9 # * Q1-Q7 PAGE SAVE NONE>\n";

/// Which pad position sends this guest key. Convenience over [`pos_for_action`] for
/// the drawing path, which has a `KeyCode` in hand for every control it paints.
pub fn pos_for_key(key: KeyCode) -> Option<PadPos> {
    pos_for_action(PadAction::Key(key))
}

/// The pad half of the DIAG dialog (59차): every bound position, in Korean, with what
/// it does. A player who cannot tell which button is "X" on their pad reads this.
///
/// Built from the same reverse lookup the badges use, so it describes the CONFIGURED
/// pad (`wie_gamepad.txt` included), not the defaults.
pub fn legend() -> String {
    let mut out = String::from("게임패드 (위치 = 모든 패드 공통)\n");
    let mut rows: Vec<(PadAction, String)> = vec![
        (PadAction::Key(KeyCode::OK), "확인 / 공격".into()),
        (PadAction::Key(KeyCode::CLEAR), "취소 / 메뉴".into()),
    ];
    // ★ A plate the player switched off in the layout editor (2026-09-10) says so
    // here. The mapping is unchanged — `Quick(n)` is always plate n — so the button is
    // still listed against its seat; it simply does nothing on the field while it is
    // off, and a DIAG dump that did not mention it would be lying to the next report.
    let hidden = crate::quick_hidden();
    rows.extend((0..crate::quickpage::PLATES as u8).map(|n| {
        let off = if crate::quickpage::plate_hidden(hidden, n) { " (숨김)" } else { "" };
        (PadAction::Quick(n), format!("퀵슬롯 {}{off}", n + 1))
    }));
    rows.extend([
        (PadAction::Key(KeyCode::NUM7), "보조무기 (7)".into()),
        (PadAction::Key(KeyCode::STAR), "지도 (*)".into()),
        (PadAction::QuickPage, "퀵바 페이지".into()),
        (PadAction::QuickSave, "빠른저장".into()),
        (PadAction::Key(KeyCode::UP), "이동 / 커서".into()),
    ]);
    for (action, what) in rows {
        match pos_for_action(action) {
            Some(p) => out.push_str(&format!("  {} = {what}\n", crate::padhud::pos_name_ko(p))),
            // A remap — or simply a pad with fewer buttons than there are plates — can
            // leave an action unreachable; say so rather than omit the row.
            None => out.push_str(&format!("  (없음) = {what}\n")),
        }
    }
    out.push_str("버튼 바꾸기: 설정 → 패드 버튼 설정 (화면의 버튼을 누르고 패드 버튼을 누른다 — 겹치면 서로 바뀐다).\n");
    out.push_str("퀵슬롯 6·7 은 기본 버튼이 없다 — 그 화면에서 붙인다.\n");
    out.push_str("메뉴/창에서는 퀵슬롯 버튼 1~5 가 숫자 1·3·9·0·# 이 된다 (6·7 은 무동작).\n");
    out.push_str("숨긴 퀵슬롯은 필드에서 무동작이다 (메뉴 숫자키는 그대로) — 설정 → 화면 배치 → HIDE/SHOW.\n");
    out
}

/// One `wie_gamepad.txt` line: `<android keycode>=<TOKEN>`, `#`-comments allowed
/// (only at line START — the game keys `#`/`*` appear on the right of `=`).
/// Tokens are the same set `wie_backend::KeyCode::parse` knows (`UP DOWN LEFT RIGHT
/// OK CLR 0`..`9 # *`) plus `Q1`..`Q<PLATES>` (activate that quickbar PLATE — 79차-B,
/// and the only way to reach the plates past the fifth), `PAGE` (quickbar page flip),
/// `SAVE` (빠른저장) and `NONE` (swallow the button).
///
/// ⛔ `Q<n>` is ONE-BASED, to match the `Q1`..`Q7` the layout editor prints on the
/// plates. Everything inside the app is zero-based; this is the one place a player
/// reads, so it uses the label they can see. Returns `None` for blanks/comments/garbage — a config typo must never
/// panic the app, which is why this does NOT call `KeyCode::parse` (it
/// `unimplemented!`s on unknown tokens).
pub fn parse_map_line(line: &str) -> Option<(i32, Option<PadAction>)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (code, token) = line.split_once('=')?;
    let code: i32 = code.trim().parse().ok()?;
    let action = match token.trim() {
        "UP" => Some(PadAction::Key(KeyCode::UP)),
        "DOWN" => Some(PadAction::Key(KeyCode::DOWN)),
        "LEFT" => Some(PadAction::Key(KeyCode::LEFT)),
        "RIGHT" => Some(PadAction::Key(KeyCode::RIGHT)),
        "OK" => Some(PadAction::Key(KeyCode::OK)),
        "CLR" => Some(PadAction::Key(KeyCode::CLEAR)),
        "0" => Some(PadAction::Key(KeyCode::NUM0)),
        "1" => Some(PadAction::Key(KeyCode::NUM1)),
        "2" => Some(PadAction::Key(KeyCode::NUM2)),
        "3" => Some(PadAction::Key(KeyCode::NUM3)),
        "4" => Some(PadAction::Key(KeyCode::NUM4)),
        "5" => Some(PadAction::Key(KeyCode::NUM5)),
        "6" => Some(PadAction::Key(KeyCode::NUM6)),
        "7" => Some(PadAction::Key(KeyCode::NUM7)),
        "8" => Some(PadAction::Key(KeyCode::NUM8)),
        "9" => Some(PadAction::Key(KeyCode::NUM9)),
        "#" => Some(PadAction::Key(KeyCode::HASH)),
        "*" => Some(PadAction::Key(KeyCode::STAR)),
        "PAGE" => Some(PadAction::QuickPage),
        "SAVE" => Some(PadAction::QuickSave),
        "NONE" => None,
        t => match t.strip_prefix(['Q', 'q']).and_then(|n| n.parse::<u8>().ok()) {
            // One-based on the way in, zero-based everywhere after.
            Some(n) if (1..=crate::quickpage::PLATES as u8).contains(&n) => Some(PadAction::Quick(n - 1)),
            _ => return None,
        },
    };
    Some((code, action))
}

/// Analog-trigger digital edge with hysteresis. Many Bluetooth pads (Xbox, PS)
/// never send `BUTTON_L2/R2` KeyEvents — the triggers exist ONLY as analog axes —
/// so the axes are thresholded here and then routed through the SAME button table
/// as a synthetic `KEYCODE_BUTTON_L2/R2`, which keeps `wie_gamepad.txt` remaps
/// working for them. Engage above 50 %, release below 25 %: a trigger resting
/// right on one threshold cannot chatter.
pub fn trigger_held(was_held: bool, value: f32) -> bool {
    const ON: f32 = 0.5;
    const OFF: f32 = 0.25;
    if was_held { value > OFF } else { value >= ON }
}

/// Hardware keyboard → guest keys, for the winit `KeyboardInput` route (real
/// keyboards, and any device whose events Java's gamepad-source check leaves
/// alone). Arrows walk, Enter/Space confirm, Escape/Backspace = CLR, the digit
/// rows are the phone digits, and NumpadMultiply is the `*` map key.
pub fn keyboard_action(key: winit::keyboard::KeyCode) -> Option<PadAction> {
    use winit::keyboard::KeyCode as K;
    Some(PadAction::Key(match key {
        K::ArrowUp => KeyCode::UP,
        K::ArrowDown => KeyCode::DOWN,
        K::ArrowLeft => KeyCode::LEFT,
        K::ArrowRight => KeyCode::RIGHT,
        K::Enter | K::NumpadEnter | K::Space => KeyCode::OK,
        K::Escape | K::Backspace => KeyCode::CLEAR,
        K::Digit0 | K::Numpad0 => KeyCode::NUM0,
        K::Digit1 | K::Numpad1 => KeyCode::NUM1,
        K::Digit2 | K::Numpad2 => KeyCode::NUM2,
        K::Digit3 | K::Numpad3 => KeyCode::NUM3,
        K::Digit4 | K::Numpad4 => KeyCode::NUM4,
        K::Digit5 | K::Numpad5 => KeyCode::NUM5,
        K::Digit6 | K::Numpad6 => KeyCode::NUM6,
        K::Digit7 | K::Numpad7 => KeyCode::NUM7,
        K::Digit8 | K::Numpad8 => KeyCode::NUM8,
        K::Digit9 | K::Numpad9 => KeyCode::NUM9,
        K::NumpadMultiply => KeyCode::STAR,
        _ => return None,
    }))
}

/// Resolve a winit `PhysicalKey` (the `KeyboardInput` route) to an action: real
/// keyboard codes via [`keyboard_action`], and anything winit could not identify —
/// which on Android is precisely the gamepad buttons, relayed as raw keycodes —
/// via the same table the JNI route uses. Belt-and-braces: with the Java dispatch
/// hooks in place, gamepad events normally never reach winit at all.
pub fn winit_key_action(key: &winit::keyboard::PhysicalKey) -> Option<PadAction> {
    use winit::keyboard::{NativeKeyCode, PhysicalKey};
    match key {
        PhysicalKey::Code(c) => keyboard_action(*c),
        PhysicalKey::Unidentified(NativeKeyCode::Android(sc)) => button_action(*sc as i32),
        PhysicalKey::Unidentified(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::joy_dir;

    #[test]
    fn the_default_layout_covers_confirm_cancel_and_the_first_five_plates() {
        assert_eq!(default_button_action(KEYCODE_BUTTON_A), Some(PadAction::Key(KeyCode::OK)));
        assert_eq!(default_button_action(KEYCODE_BUTTON_B), Some(PadAction::Key(KeyCode::CLEAR)));
        // ★ 79차-B: PLATES, not the digits that used to stand in for them.
        for plate in 0..5u8 {
            assert!(
                [
                    KEYCODE_BUTTON_X,
                    KEYCODE_BUTTON_Y,
                    KEYCODE_BUTTON_R1,
                    KEYCODE_BUTTON_R2,
                    KEYCODE_BUTTON_L2
                ]
                .iter()
                .any(|c| default_button_action(*c) == Some(PadAction::Quick(plate))),
                "plate {plate} is not reachable from the default pad layout"
            );
        }
        // The secondary weapon is a real guest key and stays one — it is not a plate.
        assert_eq!(default_button_action(KEYCODE_BUTTON_L1), Some(PadAction::Key(KeyCode::NUM7)));
    }

    /// 81차: with a window open the first five plate buttons are the digits they were
    /// before 79차-B, in the same seats (X=1 Y=3 R1=9 R2=0 L2=#) — and the plates past
    /// the fifth stay silent, because there is no sixth digit for them to have been.
    #[test]
    fn plate_buttons_fall_back_to_their_old_digits_off_the_field() {
        let seats = [
            (KEYCODE_BUTTON_X, KeyCode::NUM1),
            (KEYCODE_BUTTON_Y, KeyCode::NUM3),
            (KEYCODE_BUTTON_R1, KeyCode::NUM9),
            (KEYCODE_BUTTON_R2, KeyCode::NUM0),
            (KEYCODE_BUTTON_L2, KeyCode::HASH),
        ];
        for (code, digit) in seats {
            let Some(PadAction::Quick(plate)) = default_button_action(code) else {
                panic!("{code} should be a plate by default");
            };
            assert_eq!(menu_digit(plate), Some(digit), "plate {plate} (code {code})");
        }
        for plate in 5..crate::quickpage::PLATES as u8 {
            assert_eq!(menu_digit(plate), None);
        }
        // L1 stays `7` on both sides, so page-left and page-right are a pair again.
        assert_eq!(default_button_action(KEYCODE_BUTTON_L1), Some(PadAction::Key(KeyCode::NUM7)));
    }

    /// ...and the plates past the fifth are deliberately UNBOUND by default: a standard
    /// pad has no seat left that is not already something. This is the test that says
    /// the gap is a decision, so nobody "fixes" it by stealing SELECT or a stick press.
    #[test]
    fn the_plates_past_the_fifth_have_no_default_button() {
        for plate in 5..crate::quickpage::PLATES as u8 {
            assert!(
                ALL_CODES.iter().all(|c| default_button_action(*c) != Some(PadAction::Quick(plate))),
                "plate {plate} should need a wie_gamepad.txt line, not a default seat"
            );
        }
    }

    #[test]
    fn overrides_win_and_none_unmaps() {
        let mut over = HashMap::new();
        over.insert(KEYCODE_BUTTON_A, Some(PadAction::Key(KeyCode::NUM5)));
        over.insert(KEYCODE_BUTTON_B, None);
        assert_eq!(button_action_in(KEYCODE_BUTTON_A, &over), Some(PadAction::Key(KeyCode::NUM5)));
        assert_eq!(button_action_in(KEYCODE_BUTTON_B, &over), None);
        // Untouched codes still resolve through the defaults.
        assert_eq!(button_action_in(KEYCODE_BUTTON_SELECT, &over), Some(PadAction::Key(KeyCode::STAR)));
    }

    #[test]
    fn map_lines_parse_and_garbage_is_ignored() {
        assert_eq!(parse_map_line("96=OK"), Some((96, Some(PadAction::Key(KeyCode::OK)))));
        assert_eq!(parse_map_line(" 107 = PAGE "), Some((107, Some(PadAction::QuickPage))));
        assert_eq!(parse_map_line("104=#"), Some((104, Some(PadAction::Key(KeyCode::HASH)))));
        assert_eq!(parse_map_line("97=NONE"), Some((97, None)));
        assert_eq!(parse_map_line("# comment"), None);
        assert_eq!(parse_map_line(""), None);
        assert_eq!(parse_map_line("96=WHAT"), None);
        assert_eq!(parse_map_line("abc=OK"), None);
    }

    /// The `Q<n>` token — the only way to reach a plate past the fifth, and the one
    /// place in the app that is ONE-BASED (it matches the `Q1`..`Q7` the layout editor
    /// prints on the plates).
    #[test]
    fn plate_tokens_are_one_based_and_bounded() {
        assert_eq!(parse_map_line("96=Q1"), Some((96, Some(PadAction::Quick(0)))));
        assert_eq!(parse_map_line(" 97 = q7 "), Some((97, Some(PadAction::Quick(6)))));
        // Zero is not a plate, and neither is one past the row.
        assert_eq!(parse_map_line("96=Q0"), None);
        assert_eq!(parse_map_line(&format!("96=Q{}", crate::quickpage::PLATES + 1)), None);
        // Not a number, and not a plate token at all.
        assert_eq!(parse_map_line("96=QX"), None);
        assert_eq!(parse_map_line("96=Q"), None);
    }

    /// The stick reuses the on-screen joystick's quantiser: engage past 30 % of
    /// full deflection, keep until 15 %, and a diagonal must beat the held axis by
    /// 1.2x to turn — so a thumb resting near a diagonal cannot flutter.
    #[test]
    fn stick_quantise_engages_holds_and_releases_like_the_touch_stick() {
        // Center: nothing.
        assert_eq!(joy_dir(0.0, 0.0, 1.0, None), None);
        // Below the engage deadzone: still nothing.
        assert_eq!(joy_dir(0.2, 0.0, 1.0, None), None);
        // Full right: walk right.
        assert_eq!(joy_dir(1.0, 0.0, 1.0, None), Some(KeyCode::RIGHT));
        // Held right, decayed to 20 %: keep walking (release threshold is 15 %).
        assert_eq!(joy_dir(0.2, 0.0, 1.0, Some(KeyCode::RIGHT)), Some(KeyCode::RIGHT));
        // Held right, a shallow diagonal: not a decisive turn — keep right.
        assert_eq!(joy_dir(0.8, 0.85, 1.0, Some(KeyCode::RIGHT)), Some(KeyCode::RIGHT));
        // Held right, stick slammed down: decisive turn.
        assert_eq!(joy_dir(0.1, 1.0, 1.0, Some(KeyCode::RIGHT)), Some(KeyCode::DOWN));
        // Back to center: release.
        assert_eq!(joy_dir(0.0, 0.0, 1.0, Some(KeyCode::DOWN)), None);
        // Hat up (digital -1): UP.
        assert_eq!(joy_dir(0.0, -1.0, 1.0, None), Some(KeyCode::UP));
    }

    #[test]
    fn trigger_hysteresis_cannot_chatter_on_a_threshold() {
        assert!(!trigger_held(false, 0.4)); // below engage
        assert!(trigger_held(false, 0.5)); // engages
        assert!(trigger_held(true, 0.3)); // held through the dip
        assert!(!trigger_held(true, 0.2)); // releases
        assert!(!trigger_held(false, 0.3)); // and stays released until 0.5 again
    }
}
