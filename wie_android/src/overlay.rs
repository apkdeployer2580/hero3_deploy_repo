//! On-screen touch controls + settings gear + settings modal, drawn directly
//! into the softbuffer framebuffer. Orientation-aware: portrait places the game
//! on top with controls below; landscape centres the game with the D-pad on the
//! left and the keypad on the right.

use std::collections::HashSet;

use wie_backend::KeyCode;

use crate::{CONTENT_H, CONTENT_W, IconRef, QuickSlot, SlotKind, Stats, atlas, ui_icons};

const BTN: u32 = 0xff2c_3040;
const BTN_DPAD: u32 = 0xff26_3548;
const BTN_PRESSED: u32 = 0xff4a_6cff;
const BORDER: u32 = 0xff70_80a0;
const LABEL: u32 = 0xffff_ffff;
const PANEL: u32 = 0xff1a_1c26;
/// Quickbar action buttons — skills blue, items green, empty dim; pressed = bright.
const QB_SKILL: u32 = 0xff2a_4a8a;
const QB_ITEM: u32 = 0xff1f_5a3a;
const QB_EMPTY: u32 = 0xff2a_2e3a;
const QB_PRESSED: u32 = 0xff4a_6cff;
// (The capsule-gauge palette that sat here — BAR_TRACK/RIM/HP/SP/EXP — went with the
// capsules in 141차; the status block's colours are hero5's, in `hud_view.rs`.)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub(crate) fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    fn contains(&self, px: f64, py: f64) -> bool {
        let (xi, yi) = (px as i64, py as i64);
        xi >= self.x as i64 && xi < (self.x + self.w) as i64 && yi >= self.y as i64 && yi < (self.y + self.h) as i64
    }
    /// Half-open overlap test — what keeps [`crate::placement`]'s controls off the
    /// OBSTACLES (controls may overlap each other freely since 2026-08-07).
    /// Touching edges do NOT count as overlapping.
    pub(crate) fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    /// Grown by `n` on every side, clipped at the window origin. Used to keep a
    /// visible gap between a control and the things it must NOT touch (the game's own
    /// HUD bands, our menu chrome, the gauge tap zone).
    pub(crate) fn grown(&self, n: u32) -> Rect {
        Rect::new(self.x.saturating_sub(n), self.y.saturating_sub(n), self.w + n * 2, self.h + n * 2)
    }
}

/// The standard control margin for a window — one place, so every producer agrees.
pub(crate) fn pad_of(win_w: u32, win_h: u32) -> u32 {
    (win_w.min(win_h) / 110).max(3)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Label {
    Text(&'static str),
    Arrow(Dir),
}

/// How a button paints its background.
///
/// ★ 2026-07-30 player request: "the 1/3/7/9/0/#/CANCEL keys and the 1/3/7/9 keys
/// that appear with the menu open are square — make them ALL round". So every
/// immersive on-screen key is [`Face::Round`]; only the opaque portrait layouts
/// (which have no game showing through) keep [`Face::Solid`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Face {
    /// Opaque disc — non-immersive portrait D-pad / keypad layouts, which sit on a
    /// black control column rather than on the game, so their face is opaque.
    /// (Round too: the player said "make them ALL round", and those layouts hold a
    /// third copy of the same 1/3/7/9/0/#/CLR keys.)
    Solid,
    /// Translucent anti-aliased disc. softbuffer has no alpha channel, so the
    /// blend is done per pixel here.
    Round,
    /// The big bottom-right ACTION disc: [`Face::Round`] plus an inner gold ring so
    /// the one key that matters most still stands out from its neighbours.
    Action,
    /// Paint nothing — the widget draws its own face later (the minimap panel).
    None,
    /// A translucent HEXAGON for the arrow pointing `Dir`, inked inside the carried
    /// rect, with a small arrow glyph in its middle.
    ///
    /// ★ 91차 (2026-09-13), player: "이제 삼각형이 아니라 각각 동서남북이 육각형 모형이
    /// 되는거야." It replaces the 2026-08-09 triangle (`Face::Triangle`), which was
    /// itself the replacement of a round key with a triangle glyph — the BUTTON is a
    /// hexagon now, and since six identical corners say nothing about direction, the
    /// glyph is back inside it.
    ///
    /// ★ It carries its own rect because the arrows are the one control whose TOUCH
    /// TARGET is deliberately larger than its ink (2026-08-17, "표시되는 영역보다 조금
    /// 더 큰범위를 눌러도 동작하게"): `Button::rect` is [`Arm::hit`], this is
    /// [`Arm::draw`]. Every other face inks its whole button.
    Hexagon(Dir, Rect),
}

pub struct Button {
    rect: Rect,
    pub key: KeyCode,
    label: Label,
    dpad: bool,
    face: Face,
    /// The touch target when it is NOT `rect`: a D-pad arrow owns a diagonal
    /// QUARTER of its square (91차-B), and a quarter is not a rectangle. `None` for
    /// every other button — `rect` is the target. See [`Wedge`].
    wedge: Option<Wedge>,
}

impl Button {
    /// Does a touch at `(x, y)` belong to this button? The one place the two target
    /// shapes are told apart, so `hit` and the slide-between-keys path agree.
    fn contains(&self, x: f64, y: f64) -> bool {
        match self.wedge {
            Some(w) => w.contains(x, y),
            None => self.rect.contains(x, y),
        }
    }
}

pub struct Layout {
    pub game: Rect,
    pub buttons: Vec<Button>,
    pub gear: Rect,
    /// The floating-joystick zone (left control area) when joystick mode is on;
    /// `None` in classic D-pad mode. A touch here (that misses the corner
    /// buttons) drives the joystick.
    pub joystick: Option<Rect>,
}

/// What the overlay is showing. The emulator worker tells us whether the
/// real-time field has control or one of the game's OWN windows (main menu,
/// inventory, dialogue, shop, …) is open — the controls a player needs differ
/// completely, and drawing the field HUD over a game window is what produced the
/// "HP/MP bars read 0 in the menu" report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Real-time field free-roam: skill plates + gauges + minimap panel.
    #[default]
    Field,
    /// A game window is open: menu-navigation keys on the LEFT, the quickbar
    /// plates still on the right.
    Menu,
    /// The game's SCRIPT window (id 4) is playing — a story scene or NPC dialogue.
    /// Since 64차 NOTHING of ours is drawn over it: skipping is the invisible hit
    /// target on the game's own `# SKIP` lettering, advancing is the screen-wide OK
    /// fallback, and only a dialogue CHOICE brings the arrows up (2026-08-03's OK+`#`
    /// pair and 63차's skip disc are both gone). Placement-wise this is NOT a third
    /// layout: it borrows the MENU set's seats (see `placement::ids_for`), so the
    /// choice arrows sit wherever the player put the menu D-pad.
    Cutscene,
}

/// The keys the game's own windows need and which the immersive layout otherwise
/// has NO seat for. Player-reported: `1`/`3` move between menus (tabs), `7`/`9`
/// page within one, and `#` (added 2026-08-03) confirms / registers in the game's
/// own context popups — without it the "1·3·7·9 에 등록" list could be opened but
/// not answered.
///
/// ★ These are drawn on the LEFT since 2026-08-03. They used to take the same
/// bottom-right ring seats the quickbar uses on the field, which meant the two sets
/// were mutually exclusive; the player asked for both at once — nav keys under the
/// left thumb, quickbar plates under the right.
// The always-on `1 3 7 9 #` menu-key block is GONE (79차-B, 2026-09-04).
//
// It was a KEYPAD, and this app stopped being driven by a keypad: 66차 made the game's
// own «1» «3» «7» «9» badges tappable where it draws them (found by pixel scan, so it
// works on screens with no anchor at all), and the menu rows, the inventory cells, the
// item grids and the game's own buttons are all direct taps now. Five permanent digit
// keys on the left were a second, less legible way to do the same thing.
//
// ⛔ Nothing became unreachable at the time: the PULL-OUT keypad (the `123` toggle)
// still carried all twelve keys plus both soft keys. ★ 2026-09-18 the player asked for
// that toggle to go too ("상단 톱니바퀴 옆에 123버튼 이제 제거해도 돼"): every screen the
// game has is tapped directly now (66차~80차), so the keypad had become a leftover. The
// digits 2/4/5/6/8/0/`*` and the two soft keys are no longer on the touch HUD at all —
// a gamepad still sends them (`gamepad.rs`). If a window ever turns out to want one of
// them, add the tap target for that screen; do not bring a keypad back.

/// The game's own `# SKIP` cutscene hint, in LOGICAL guest px — see [`game_skip_hint`]
/// for the probe measurement these three numbers come from. ⛔ Do not "tidy" them into
/// window px: the game re-anchors its own chrome to whatever framebuffer we hand it.
const SKIP_HINT_W: f64 = 32.0;
const SKIP_HINT_H: f64 = 9.0;
const SKIP_HINT_MARGIN: f64 = 2.0;

/// A live view of the active joystick, for rendering (base at `center`, thumb at
/// `thumb`, both in window pixels).
#[derive(Clone, Copy)]
pub struct JoyView {
    pub center: (f64, f64),
    pub thumb: (f64, f64),
}

/// ★ `SetPortrait` .. `CycleBaseH` are RETAINED but unreachable: their settings rows
/// were removed on player request 2026-07-31 (the modal is LAYOUT + SERVER only). The
/// features themselves still work from their persisted config files, and putting a row
/// back is one line in [`settings_items`] — so this is a deliberate, reversible
/// amputation of the UI, not a deletion of working code.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum SettingsAction {
    SetPortrait,
    SetLandscape,
    SetDpad,
    SetJoystick,
    /// Toggle landscape immersive (full-screen) mode — applies on next start.
    ToggleImmersive,
    /// Cycle the immersive logical height (zoom vs fill) — applies on next start.
    CycleBaseH,
    /// Open the layout editor: drag a control to move it, two fingers to resize it
    /// (player request 2026-07-31).
    EditLayout,
    /// Cycle the 조작 방식 (player request 2026-08-16; three-way since 2026-09-22):
    /// 자유 스틱 = the floating stick that spawns under the finger → 십자키 = the
    /// four-arrow D-pad pinned bottom-left with no stick at all → 고정 스틱 = the
    /// stick with its base pinned to that seat. Takes effect at once — see
    /// [`crate::StickMode`].
    ToggleTouchType,
    /// Cycle the effect-bus trim (2026-08-27). The music/effect balance is one fixed
    /// number applied at every slider step, and a level that reads as "effects too loud"
    /// on an S25 Ultra reads as "effects inaudible" on a Fold 6 — so the number the ear
    /// picks is the number the ear can change. See [`crate::audio_mix::SFX_LEVELS`].
    CycleSfxLevel,
    // ⛔ `EditServer` / `EditAccount` / `ToggleNetMode` (the private server's address,
    // account code and OFFLINE / SERVER / AUTO source) WERE HERE and are gone (99차,
    // 2026-09-16 — 100 % offline). The panel has no row that can reach a server.
    /// Save the game NOW, without walking the menu (player request 2026-08-04:
    /// "빠른저장 버튼"). The worker owns the whole thing — it re-reads the guest's own
    /// "may I save here" byte and drives the game's four save primitives.
    QuickSave,
    /// Show the on-device diagnostics dialog (2026-08-07): audio pipeline status,
    /// app/OS/device versions, window + logical sizes — with a copy button. Exists
    /// because remote testers cannot run adb, and "소리가 안 나요" from another
    /// phone is undebuggable without the init lines it shows.
    Diagnostics,
    /// 발열 절감 (57차): half-resolution render (hardware-upscaled) + lite synth,
    /// for low-spec devices that run hot (S9). Applies at next start — see
    /// `config::read_heat_saver`.
    ToggleHeatSaver,
    /// 게임패드 전용 HUD (59차): cycle AUTO -> ON -> OFF. Applies AT ONCE (the layout
    /// is rebuilt from the preference every relayout) — see `crate::pad_hud_active`.
    TogglePadHud,
    /// 패드 버튼 설정 (2026-10-09): open the remap screen — pick an action, press the
    /// button that should do it (`App::padmap_open`, rules in `padmap.rs`).
    PadMap,
    Close,
}

/// See [`SettingsAction`]: the first six are retained for rows that were removed.
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum SKind {
    Portrait,
    Landscape,
    Dpad,
    Joystick,
    Immersive,
    BaseH,
    EditLayout,
    TouchType,
    Save,
    Diag,
    HeatSaver,
    SfxLevel,
    PadHud,
    PadMap,
    Close,
}

pub struct SettingsItem {
    rect: Rect,
    pub action: SettingsAction,
    kind: SKind,
    /// A row that is drawn but cannot be pressed in the current configuration.
    /// Swallows the tap rather than closing the modal, and is drawn dimmed with a
    /// reason — a full-width button that silently no-ops reads as a broken app.
    enabled: bool,
}

// --------------------------------------------------------------------------
// Layout
// --------------------------------------------------------------------------

// ── regions the GAME owns (never draw overlay chrome there) ──────────────────
//
// Both are drawn by the game's own field HUD, in GUEST framebuffer pixels
// [RE: workflow wf_477f0b16 QA/QB, both CONFIRMED HIGH]:
//
//  * TOP-LEFT — the active buff / status list. Up to 3 slots of `/comm/ui_bm`
//    frame 3 (20x25) at `x = 20*i + 1, y = 2`, each with its remaining-ticks
//    number at y = 18. Absolute top-left coordinates, NOT screen-relative.
//    Rect (1, 2, 60, 25) at 3 slots. The verifier of the companion question read
//    the SAME band as the party portrait row (19 px stride, level digit centred at
//    idx*19+10), i.e. up to 4 entries out to x ~= 76 — the two readings disagree on
//    what the band IS but agree on where it is, so reserve the wider (0, 0, 80, 28)
//    and be correct either way.
//  * TOP-RIGHT — the target monster's name + level + HP bar (`ui_bm` frame 4,
//    85x26) at `x = screen_w - 87, y = 2`. RIGHT-EDGE anchored, so it tracks the
//    widened framebuffer. Reserve (screen_w - 90, 0, 90, 30).
//
// Player-reported (2026-07-30): our minimap panel covered the monster box, and the
// gauges + gear + keypad toggle covered the buff list.
// Kept (with the selftest mirror) because the Y0 shift below is derived from it:
// how far the game's band has to move is a function of its own height.
#[allow(dead_code)]
const GUEST_BUFFLIST_W: u32 = 80;
#[allow(dead_code)]
const GUEST_BUFFLIST_H: u32 = 28;
// Documentation + selftest mirror: the overlay no longer dodges these bands (the
// game's own band is moved below our chrome instead), but the geometry is what the
// shift is derived from and what the tests assert against.
#[allow(dead_code)]
const GUEST_TARGETBOX_W: u32 = 90;
#[allow(dead_code)]
const GUEST_TARGETBOX_H: u32 = 30;

/// Map a length in GUEST pixels to window pixels along x / y of `game`.
fn gx(game: Rect, n: u32) -> u32 {
    let (lw, _) = crate::logical();
    n * game.w / lw.max(1)
}
fn gy(game: Rect, n: u32) -> u32 {
    let (_, lh) = crate::logical();
    n * game.h / lh.max(1)
}

/// How far DOWN (in GUEST px) the game's own top HUD band has to be pushed so it
/// clears our gauge block. The band's y is `Y0 + const` for a y-origin the game
/// reads from a global, and the host writes that global — so instead of our HUD
/// dodging the game's, the game's band moves below ours (player request
/// 2026-07-30: "gauges at the extreme top-left, put the buff list under them").
///
/// Derived from our own chrome, so it follows any restyle: the band has to clear
/// BOTH the gauge block (top-left) and the minimap panel (top-right), because the
/// game draws its buff list under the former and its target-monster box under the
/// latter.
pub fn hud_y_shift(_game: Rect) -> u32 {
    // 0 = leave the game's own y-origin alone. The band used to be pushed DOWN to
    // clear our gauges; now it is pushed RIGHT instead (see `band_x_shift`), which
    // is where the player wants it — on the same row as the gauges, in the strip the
    // gear and keypad toggle used to occupy.
    0
}

/// Height (GUEST px) we would like the game's buff band to have.
///
/// ★ 2026-08-05: the ORIGINAL 25, i.e. "do not shrink the band". It used to ask for
/// our HP+SP+EXP stack height so the two would read as one row, which on a tall
/// phone came out below the natural height and was clamped to `items::BAND_H_MIN`
/// (20) — exactly tall enough for the hardcoded 15x15 icon at y+5 but NOT for the
/// frame's own bottom border row, so every buff icon looked shaved at the bottom
/// (player report: "버프 항목의 아이콘 하단 살짝 잘린것처럼 보이는데"). At 25 the
/// full frame fits and nothing is cropped; `items::band_height_poke` also restores
/// a previously shrunk record back to 25 by the same read-compare-write pass.
pub fn band_h_shift(_game: Rect) -> u32 {
    25
}

/// Where the game's own buff band should START, in GUEST px: just right of our
/// gauge block. The band's x is `cell_w * i + K` with K a one-byte immediate in the
/// renderer, so the host can move it (see `items::band_x_patch`).
pub fn band_x_shift(game: Rect) -> u32 {
    if game.w == 0 {
        return 1;
    }
    let b = stats_block(game);
    let right = (b.bars.x + b.bars.w).saturating_sub(game.x);
    let (lw, _) = crate::logical();
    // window px -> guest px, + a little air; the immediate is a u8.
    ((right * lw.max(1) / game.w) + 3).min(200).max(1)
}

/// Window rect of the game's own buff/status list (top-left), at its SHIFTED
/// position (see [`hud_y_shift`]).
pub(crate) fn game_buff_box(game: Rect) -> Rect {
    Rect::new(
        game.x,
        game.y + gy(game, hud_y_shift(game)),
        gx(game, GUEST_BUFFLIST_W),
        gy(game, GUEST_BUFFLIST_H),
    )
}

/// Window rect of the game's own target-monster box (top-right, right-anchored).
/// Its y comes from the SAME y-origin global as the buff list, so the shift moves
/// this too.
pub(crate) fn game_target_box(game: Rect) -> Rect {
    let w = gx(game, GUEST_TARGETBOX_W);
    Rect::new(
        (game.x + game.w).saturating_sub(w),
        game.y + gy(game, hud_y_shift(game)),
        w,
        gy(game, GUEST_TARGETBOX_H),
    )
}

/// Largest logical-aspect rect fitting inside `area`, centered. Uses the LIVE
/// guest size (not the 240x320 constants) so an immersive wide frame is never
/// letterboxed with the portrait ratio during a rotation.
fn contain(area: Rect) -> Rect {
    let (lw, lh) = crate::logical();
    let scale = (area.w as f32 / lw as f32).min(area.h as f32 / lh as f32).max(0.0);
    let w = (lw as f32 * scale) as u32;
    let h = (lh as f32 * scale) as u32;
    Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h)
}

/// Landscape **immersive** layout: the game frame is the WHOLE window (the guest
/// already runs at an aspect-matched logical size, so filling it is 1:1 and
/// distortion-free), there are NO side control columns, and the controls are
/// translucent overlays drawn on top of the game:
///
/// * a right-edge 2x4 grid of keys — `1 3 / 5 9 / 0 # / OK CLR`
///   (the label is ASCII "CLR", not "취소": `draw_glyph` is a 5x7 ASCII bitmap
///   font, so a Hangul label renders BLANK — device-confirmed 2026-07-28.)
/// * everything outside those buttons (and the gear) is the joystick zone, so a
///   touch anywhere on the field starts a stick there.
///
/// `OK` is included even though the user's 7-key sketch omitted it: dialogue
/// advance / menu confirm has no other binding, so without it the mode would be
/// unusable. Flagged rather than silently dropped.
/// Bounding square of the big bottom-right ACTION disc (the reference layout's
/// primary button). Shared by the layout and the skill cluster so they cannot
/// drift apart.
/// Centre of the whole bottom-right action cluster (disc + ring + secondary
/// button). FIXED, i.e. derived from the cluster's nominal radius rather than from
/// the disc's own size — shrinking the disc must not drag this toward the corner,
/// or the ring gets squeezed off-screen.
fn action_centre(win_w: u32, win_h: u32, pad: u32) -> (u32, u32) {
    let nominal = (win_h / 6).clamp(56, 190);
    (win_w.saturating_sub(pad * 3 + nominal), win_h.saturating_sub(pad * 3 + nominal))
}

/// Bounding square of the ACTION disc (key OK). Smaller than the cluster slot it
/// sits in: the player asked for OK and the secondary-weapon button to be a
/// same-size pair side by side (2026-07-30), and the space for the second one comes
/// out of the disc.
/// ★ BUILT-IN geometry only. The player's own size/position for OK is applied by
/// [`crate::placement`] AFTER this, never inside it — that is what stops a resized
/// OK from dragging the skill ring around (`ring_slots` derives its radius from the
/// disc, so a disc that grew with the player's knob used to shrink every plate:
/// measured 98 px -> 52 px at the old +40 % knob).
pub(crate) fn action_disc(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let (cx, cy) = action_centre(win_w, win_h, pad);
    let r = (win_h / 8).clamp(48, 140);
    Rect::new(cx.saturating_sub(r), cy.saturating_sub(r), r * 2, r * 2)
}

/// The persistent joystick home (centre + radius): the STICK seat's inscribed
/// circle. This is also the radius of the LIVE stick's base disc.
///
/// ★ The stick is PLACEABLE since 2026-07-31. A left-handed player moves the buttons
/// to the left, and the hint used to stay bottom-LEFT underneath them — measured:
/// after a FLIP the OK disc covered it outright, and because controls win the hit
/// test the stick could no longer be started there. So it moves with the layout.
///
/// ★ And RESIZABLE since 2026-08-16, which is why the radius comes out of the
/// RESOLVED rect rather than out of the built-in seat. Reading the built-in radius
/// here was what made the old size knob a no-op.
///
/// ★★★ 91차-B (2026-09-13): the seat is the ONE walk control's square — the square
/// the menu arrows are cut from (`stick_seat` / `dpad_arms`) — and the stick IS that
/// square. Player: "화살표 전체가 조이스틱 전체고 조이스틱 전체가 화살표 전체인 느낌".
/// The first 91차 cut drew a small disc at the square's centre; that read as "a stick in
/// the middle of the arrows", which is exactly what the player said it must not be.
///
/// ★★★ 2026-09-20 the radius came in one notch, from the square's inscribed circle to
/// [`stick_base_radius`] — the circle through the four hexagons' centres ("화살표
/// 크기보다 조이스틱이 조금 더 작아야해"). Same centre, same one knob; only the rim moved.
///
/// What the ring does NOT set any more is the walk-start distance — see
/// [`joy_engage_radius`]: a 312 px ring with the old 30 % engage rule would need a
/// 94 px thumb travel (7 mm) before the character moved. The thumb pins INSIDE the
/// base ([`joy_radius`]), so the reason it stops is drawn: it is the cap in its well.
pub fn joy_home(win_w: u32, win_h: u32) -> (f64, f64, f64) {
    let seat = crate::placement::rect_of(crate::placement::CtrlId::Stick, Mode::Field, win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h));
    (
        f64::from(seat.x) + f64::from(seat.w) / 2.0,
        f64::from(seat.y) + f64::from(seat.h) / 2.0,
        stick_base_radius(&seat),
    )
}

/// The thumb disc's radius as a fraction of the base's. The thumb's CENTRE travels at
/// most `1 - JOY_THUMB` of the base radius ([`joy_radius`]), so the thumb never leaves
/// the base — the pin has a drawn reason.
pub const JOY_THUMB: f64 = 0.44;

/// The stick's pre-91차-B radius: `win_h / 12`, clamped. It is no longer drawn
/// anywhere — it survives as the WALK-START basis ([`stick_engage_radius`]), so that
/// the base disc could grow to the arrows' size without the deadzone growing with it.
fn joy_r_legacy(win_h: u32) -> f64 {
    f64::from((win_h / 12).clamp(34, 96))
}

/// The deadzone basis for a resolved STICK seat: the legacy radius scaled by the
/// seat's size percent. `joy_dir` starts walking at 30 % of this and stops at 15 %,
/// so at a 100 % seat the character moves after the same 27 px it always did, and
/// the player's one knob still makes a bigger stick start walking later.
pub(crate) fn stick_engage_radius(seat: &Rect, win_w: u32, win_h: u32) -> f64 {
    let b = stick_seat(win_w, win_h);
    let base = f64::from(b.w.min(b.h).max(1));
    joy_r_legacy(win_h) * f64::from(seat.w.min(seat.h)) / base
}

/// Angular span of the control ring, in screen degrees (`cy = acy - r*sin`, so 90
/// is UP and 180 is LEFT). The action disc sits in the bottom-right corner, so the
/// reachable arc runs from just right-of-top, over the top, down to just below-left
/// — that is the widest wrap the corner allows.
const RING_A0: f64 = 66.0;
/// Was 204: the last seat ('#') ended up hard against the bottom edge on device,
/// so the arc now stops higher (player request 2026-07-30).
const RING_A1: f64 = 195.0;

/// Widest angular PITCH the ring will use, in degrees: the six-seat ring's.
///
/// It only ever binds when plates are hidden (2026-09-10) — the full eight-seat ring's
/// own pitch is smaller. Six seats is the FIVE-PLATE ring that shipped from 2026-07-30
/// until 79차, i.e. the widest spacing any player has actually had, and at 1080 tall it
/// is the 98 px plate 79차-E measured.
const RING_STEP_MAX: f64 = (RING_A1 - RING_A0) / 5.0;

/// Ring geometry around the action disc: `n` equal steps from [`RING_A0`] toward
/// [`RING_A1`] — reaching it exactly when the pitch is not capped by
/// [`RING_STEP_MAX`] — at a radius that (a) clears the disc and (b) keeps ADJACENT
/// square plates apart. Squares need more than "chord >= side": a diagonal offset can
/// leave both |dx| and |dy| under `side`, hence the 1.485 factor.
///
/// The plate size shrinks until every position is fully on-screen, so the ring
/// adapts to odd aspect ratios instead of clipping (selftest asserts it).
fn ring_slots(win_w: u32, win_h: u32, n: usize, pad: u32) -> Vec<Rect> {
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let a = action_disc(win_w, win_h, pad);
    let ar = f64::from(a.w) / 2.0;
    let (acx, acy) = {
        let (cx, cy) = action_centre(win_w, win_h, pad);
        (f64::from(cx), f64::from(cy))
    };
    // The ring must also clear the secondary-weapon button, which sits between the
    // disc and the ring on the disc's centre row.
    let clear = acx - f64::from(secondary_button(win_w, win_h).x);
    // ★★ The pitch is `arc / (n - 1)` as it always was, but CAPPED (2026-09-10, when
    // plates became hideable). Both halves are measured, not chosen:
    //
    //  * Uncapped, a short ring re-spreads over the WHOLE arc, and with one plate
    //    visible that puts it at `RING_A1` — the arc's far end, up and LEFT of the
    //    action disc, as far from the thumb as the ring can reach. It also drags CLR
    //    out from between that plate and OK (the landscape selftest caught exactly
    //    that, at 2340x1080 with one plate left).
    //  * Pinned to the full ring's pitch instead, the survivors never grow at all: the
    //    pitch is what sets the radius (`need` below is inversely proportional to
    //    `sin(step/2)`), so keeping it tight keeps the ring far out and the plate small.
    //    Measured at 2340x1080: six seats came out 86 px, the same as eight.
    //
    // The cap is the pitch of the SIX-SEAT ring — the five-plate arrangement that
    // shipped from 2026-07-30 until 79차 (2026-09-04) — so hiding down to five plates
    // reproduces that layout exactly (98 px at 1080, the number 79차-E measured), and
    // hiding further keeps that pitch and simply ends the arc earlier. ⛔ Do not raise
    // it: every degree past this is a plate further from the thumb than any shipped
    // layout has ever put one.
    let step = if n > 1 {
        ((RING_A1 - RING_A0) / (n - 1) as f64).min(RING_STEP_MAX)
    } else {
        0.0
    };
    // Lay the ring out at `side`; `None` = some seat would leave the window.
    let try_side = |side: f64| -> Option<Vec<Rect>> {
        let need = if step > 0.0 {
            side * 1.485 / (2.0 * (step.to_radians() / 2.0).sin())
        } else {
            0.0
        };
        let radius = need.max(ar + side * 0.75).max(clear + side / 2.0 + f64::from(pad * 2));
        let mut seats = Vec::with_capacity(n);
        for i in 0..n {
            let ang = (RING_A0 + step * i as f64).to_radians();
            let cx = acx + radius * ang.cos();
            let cy = acy - radius * ang.sin();
            let (x0, y0) = (cx - side / 2.0, cy - side / 2.0);
            if x0 < 0.0 || y0 < 0.0 || x0 + side > f64::from(win_w) || y0 + side > f64::from(win_h) {
                return None;
            }
            seats.push(Rect::new(x0 as u32, y0 as u32, side as u32, side as u32));
        }
        Some(seats)
    };
    // ★★ The LARGEST plate that fits, not the first 10 % step that does (2026-09-04,
    // player: *"퀵슬롯 버튼 크기를 전부 조금씩만 키워줘"*). Going from 5 plates to 7 made the
    // ring crowded enough to need a shrink, and the old `side *= 0.9` loop then
    // overshot: on the S25 (2340x1080) the largest side that fits is **87**, and the
    // loop handed back **79** — the missing 9 % was quantisation, not geometry.
    // The base stays the ceiling, so a ring that already fits is UNCHANGED (six seats
    // still measure 98 px there).
    let base = f64::from((win_h / 11).clamp(40, 120));
    if let Some(seats) = try_side(base) {
        return seats;
    }
    // Bisect: `lo` always fits (or is degenerate), `hi` never does. 14 rounds put the
    // answer inside a pixel for any window this runs on.
    let (mut lo, mut hi) = (0.0_f64, base);
    let mut best: Option<Vec<Rect>> = None;
    for _ in 0..14 {
        let mid = (lo + hi) / 2.0;
        match try_side(mid) {
            Some(seats) => {
                best = Some(seats);
                lo = mid;
            }
            None => hi = mid,
        }
    }
    if let Some(seats) = best {
        return seats;
    }
    // ★ Never return an EMPTY ring. The old code fell out of the loop with `out`
    // cleared, and every consumer took that literally: `cancel_key` degraded to a
    // 1x1 CLR at pixel (0,0) and the whole quickbar disappeared — with the game's
    // own bottom HUD patched off, that leaves a field with no controls at all.
    // Clamp the last attempt into the window instead.
    out.clear();
    let side = lo.max(16.0);
    for i in 0..n {
        let ang = (RING_A0 + step * i as f64).to_radians();
        let radius = (ar + side * 0.75).max(clear + side / 2.0 + f64::from(pad * 2));
        let cx = (acx + radius * ang.cos()).clamp(side / 2.0, f64::from(win_w) - side / 2.0);
        let cy = (acy - radius * ang.sin()).clamp(side / 2.0, f64::from(win_h) - side / 2.0);
        out.push(Rect::new((cx - side / 2.0) as u32, (cy - side / 2.0) as u32, side as u32, side as u32));
    }
    out
}

/// One LIVE ring seat: 0 = CLR, then one per VISIBLE plate in plate order (see
/// [`ring_seats_live`]). The single accessor [`crate::placement`] uses for its
/// built-ins — through [`plate_seat`] for the plates themselves, since a seat index is
/// no longer the same thing as a plate number.
pub(crate) fn ring_seat(win_w: u32, win_h: u32, pad: u32, i: usize) -> Rect {
    seat_or_corner(ring_all(win_w, win_h, pad), win_w, win_h, pad, i)
}

/// Seat `i` of `ring`, or the bottom-right corner fallback. Shared by the live and the
/// full ring so the out-of-range answer cannot differ between them.
fn seat_or_corner(ring: Vec<Rect>, win_w: u32, win_h: u32, pad: u32, i: usize) -> Rect {
    ring.get(i).copied().unwrap_or_else(|| {
        let s = (win_h / 11).clamp(40, 120);
        Rect::new(win_w.saturating_sub(s + pad), win_h.saturating_sub(s + pad), s, s)
    })
}

/// BUILT-IN rect of the CANCEL disc — **between the first quickbar plate and the OK
/// disc, hugging the right edge**, and derived from the clearance it needs rather than
/// picked.
///
/// ★★★ 80차-H. Player, 2026-09-05: *"늘어난 스킬 퀵슬롯 때문에 취소키랑 약간 겹치는데 이거
/// 취소키를 살짝 ok쪽으로 옮겨서 안겹치게 할수있어? 너무 많이 이동시키지는 말고 Q1과 OK키
/// 사이에서 아주 조금 우측에 붙는느낌인거야"*.
///
/// ── WHY IT OVERLAPPED, and it is NOT "the ring got crowded" ────────────────────────
/// CLR has been secondary-button sized since 2026-08-07 ("취소버튼의 크기가 너무 작아서")
/// while sitting on a PLATE-sized ring seat — at 2220x1080 that is a **162 px** disc on an
/// 86 px seat. Seat 0 is the ring's rightmost, so the big disc runs off the right edge and
/// `builtin`'s own `.min(win_w - d)` clamp shoves it back LEFT, straight into plate 1:
/// ```text
///   seat 0 centre 2176  ->  disc x 2095..2257, clamped to 2058..2220  (centre 2139)
///   plate 1 centre (2051, 472) r 43   vs   CLR centre (2139, 505) r 81
///   centres 94 px apart, radii sum 124  ->  30 px of overlap
/// ```
/// ⛔ So do NOT "fix" this by rotating it further along the ring: the arc runs UP from
/// there, i.e. toward plate 1, and any angle that keeps the disc on screen is closer to
/// the plate, not further. The disc has to come off the ring and go DOWN, which is also
/// what the player asked for — down is toward OK.
///
/// The rule: put the centre as far right as the window allows, then take it down from
/// plate 1 by exactly the distance that makes the two circles clear by `pad * 2`. One
/// number (the gap), and the position follows from the sizes — so a bigger CLR, a bigger
/// plate or a ninth seat all move it on their own.
pub(crate) fn cancel_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let d = secondary_button(win_w, win_h)
        .w
        .max(ring_seat(win_w, win_h, pad, 0).w)
        .min(win_w.max(1))
        .min(win_h.max(1));
    let r = f64::from(d) / 2.0;
    let q = ring_seat(win_w, win_h, pad, 1);
    let (qcx, qcy) = (f64::from(q.x) + f64::from(q.w) / 2.0, f64::from(q.y) + f64::from(q.h) / 2.0);
    // Circle-to-circle, because both are drawn as discs: a rect test would call them
    // clear while the strokes still touch.
    let need = r + f64::from(q.w.min(q.h)) / 2.0 + f64::from(pad * 2);
    let cx = (f64::from(win_w) - f64::from(pad * 2) - r).max(r);
    let dx = (cx - qcx).abs();
    // BELOW the plate — toward OK. `dx` already buys part of the distance.
    let dy = (need * need - dx * dx).max(0.0).sqrt();
    let cy = (qcy + dy).clamp(r, f64::from(win_h) - r);
    Rect::new((cx - r).max(0.0) as u32, (cy - r).max(0.0) as u32, d, d)
}

/// BUILT-IN seat of the PAGE-FLIP button: one plate-pitch LEFT of the leftmost
/// quickbar plate, level with it — "보조공격 버튼 좌측 스킬버튼의 좌측" (player,
/// 2026-08-04).
///
/// ★ This replaces `ring_seat(.., <plate count> + 1)`, which asked for a seat that
/// DOES NOT EXIST: the ring has `RING_SEATS` seats (0 = CLR, then one per plate), so
/// the seat one past the end fell straight into `ring_seat`'s bottom-right-corner
/// fallback.
/// That is the whole of the "페이지 버튼이 구석에 박혀 있다" report — not a layout
/// preference, an out-of-range index quietly answered with a fallback rect.
/// ★ 2026-08-06 (player: "페이지 아이콘이 너무 크고 지금 위치보다 더 내려가야해. 더 아래
/// 스킬버튼 좌측에 존재해야해"): SMALLER than a plate, and dropped half a plate so it
/// reads as *below*-left of the lowest plate rather than level with it. The anchor is
/// now the LOWEST plate explicitly (`max y`) instead of the leftmost (`min x`) — on the
/// current arc those are the same seat, but saying which one we mean is what keeps this
/// correct if the arc is ever re-spaced.
pub(crate) fn quickpage_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    below_left_of_lowest_plate(win_w, win_h, pad)
}

/// How far the PAGE button's icon spans across its disc, as a multiple of the disc
/// radius. The shared default in [`draw_mask_button_spanned`] is 1.5 (= 75% of the button).
///
/// ★ 2026-08-06, and it is a correction of a misreading: "페이지 아이콘이 너무 크고"
/// was read as "the BUTTON is too big" and the button was shrunk to 70%. The player
/// meant the icon INSIDE it — "버튼크기는 지금이랑 동일하게 해도 돼". So the button is
/// plate-sized again like every other control, and only the glyph shrinks.
pub const QUICKPAGE_ICON_SPAN: f64 = 1.05;
/// The GAME'S OWN `# SKIP` hint during a cutscene, as a WINDOW-px touch target.
///
/// ★ 63차, player: "현재 컷슬이 나오면 우신최하단 #버튼이 나오고있는데 이거 혹시
/// 우신하단 skip 이라는 문구 클릭했을때 skip 되도록 변경할 수 있는지". That `#` is
/// **not one of ours** — the GAME paints `# SKIP` in its own bottom-right corner, and
/// until now tapping it only advanced the page, because the corner fell through to the
/// screen-wide OK (`tap_anywhere_key`). This rect puts a hit target on the game's own
/// lettering so the word does what it says.
///
/// The geometry is MEASURED, not guessed — `wie_probe` in the opening cutscene at four
/// logical sizes (496x240, 496x228, 560x240, 440x220), two dumped frames each:
///
/// ```text
///   496x240 -> x 462..493  y 229..237     560x240 -> x 526..557  y 229..237
///   496x228 -> x 462..493  y 217..225     440x220 -> x 406..437  y 209..217
/// ```
///
/// Every one is **32 x 9 logical px with a 2 px margin from the right and bottom
/// edges** — i.e. corner-anchored and resolution-INDEPENDENT, which is the same
/// adaptive-render property §4.5 of `MIGRATION_LGT.md` measured for the rest of the
/// game's own chrome. So the ink is derived, never hardcoded in window px.
///
/// The TARGET is then grown past the ink, exactly like the D-pad arrows
/// (`Arm::hit` vs `Arm::draw`, player 2026-08-17 "표시되는 영역보다 조금 더
/// 큼범위를 눌러도 동작하게"): 9 logical px is ~43 window px on a 1080-tall
/// screen, well under a thumb. It stays pinned to the corner and clamped inside
/// `game`, so it can only ever eat screen-wide-OK area in the one corner the player
/// is aiming at.
pub(crate) fn game_skip_hint(game: Rect) -> Rect {
    let (lw, lh) = crate::logical();
    game_skip_hint_at(game, lw, lh)
}

/// [`game_skip_hint`] with the logical size passed in — the pure half, so
/// `landscape_selftest.rs` can mirror it and assert the probe numbers directly
/// instead of asserting a rect it derived the same way the code did.
pub(crate) fn game_skip_hint_at(game: Rect, lw: u32, lh: u32) -> Rect {
    if game.w == 0 || game.h == 0 || lw == 0 || lh == 0 {
        return Rect::new(0, 0, 0, 0);
    }
    // ★ The target spans FROM the ink's top-left corner OUT to the window's true
    // bottom-right corner, rather than being the ink's size shifted in by a margin.
    // That is not style: deriving the margin separately rounded the target's left edge
    // one pixel INSIDE the lettering —
    // `landscape_selftest::the_skip_target_covers_the_game_s_own_lettering` caught it at
    // 2340x1080 (target x 2180 vs ink x 2179). Spanning to the corner makes "the target
    // contains the ink" true BY CONSTRUCTION, and it hands the player the 2-px margin
    // too, which is exactly where a thumb aiming at a corner lands.
    let ink = game_skip_ink(game, lw, lh);
    let want_w = (game.x + game.w).saturating_sub(ink.x);
    let want_h = (game.y + game.h).saturating_sub(ink.y);
    // Thumb floor: a fraction of the SHORT side, so the target is the same physical
    // size in portrait and landscape and at every density.
    let floor = (game.w.min(game.h) / 9).max(24);
    let w = want_w.max(floor).min(game.w);
    let h = want_h.max(floor).min(game.h);
    Rect::new((game.x + game.w).saturating_sub(w), (game.y + game.h).saturating_sub(h), w, h)
}

/// Where the game's own `# SKIP` lettering actually lands, in WINDOW px — the ink the
/// target has to cover. Origin FLOORED and size CEILED, so a fractional scale can only
/// make this rect bigger than the real glyphs, never smaller.
pub(crate) fn game_skip_ink(game: Rect, lw: u32, lh: u32) -> Rect {
    let sx = f64::from(game.w) / f64::from(lw);
    let sy = f64::from(game.h) / f64::from(lh);
    let x = f64::from(game.x) + (f64::from(lw) - SKIP_HINT_MARGIN - SKIP_HINT_W) * sx;
    let y = f64::from(game.y) + (f64::from(lh) - SKIP_HINT_MARGIN - SKIP_HINT_H) * sy;
    Rect::new(x as u32, y as u32, (SKIP_HINT_W * sx).ceil() as u32, (SKIP_HINT_H * sy).ceil() as u32)
}

/// BUILT-IN seat of the QUICK-SAVE button: the next step along the same row as the
/// page button.
///
/// It is a placeable control rather than a settings row because the settings row was
/// UNREACHABLE BY CONSTRUCTION (2026-08-04): the gear that opens the panel exists
/// only OFF the field, and `items::save_gate` — the game's own "may I save here" byte
/// — passes only ON the field. Every press could therefore only ever answer
/// `CANNOT SAVE HERE`. The panel row stays for the layouts that have no placeable
/// controls at all; this is the one a player on the field can actually hit.
/// ★ 2026-08-06 (player: "페이지 버튼 좌측 save버튼은 우측 상단 몬스터 이름 및 HP표시하는
/// 박스의 좌측으로 옮겨줘"): it leaves the plate row entirely and goes to the TOP-RIGHT,
/// immediately left of the box the GAME draws for the targeted monster.
///
/// The anchor is [`game_target_box_at`] — where that box will ACTUALLY be drawn, after
/// the host has poked its x to dodge the minimap panel — so this tracks the dodge
/// instead of assuming the box is still right-anchored. The BUILT-IN minimap rect is
/// used deliberately: this function is itself a built-in seat (the value a placement
/// falls back to), and reading the player's resolved minimap placement here would make
/// one built-in depend on the placement resolver that is asking for it.
pub(crate) fn quicksave_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    // ★ 141차 (player: *"save버튼 가로위치는 두고 스크린샷처럼 버튼을 화면상단에 붙이고"*):
    // the HORIZONTAL position is the one it has had since 2026-08-06 — left of
    // whichever of the game's target box and the minimap reaches further left — and the
    // button now sits ON THE TOP EDGE as a flat word (`draw_top_label`), a plate wide and
    // `TOP_LABEL_H_PCT` of a plate tall.
    let side = ring_seat(win_w, win_h, pad, 1).w;
    let game = Rect::new(0, 0, win_w, win_h);
    let map = minimap_panel_builtin(win_w, win_h);
    let target = game_target_box_at(game, map);
    let block_x = target.x.min(map.x);
    let h = top_label_h(side);
    let x = block_x.saturating_sub(side + pad * 2).min(win_w.saturating_sub(side));
    // ★ 2026-09-30: FLUSH with the top edge (y = 0) — the box hangs from it like a tab
    // (`draw_top_label` borders only the three other sides).
    Rect::new(x, 0, side, h.min(win_h))
}

/// A top-row word's height as a share of a plate's side (141차).
pub const TOP_LABEL_H_PCT: u32 = 60;

fn top_label_h(side: u32) -> u32 {
    (side * TOP_LABEL_H_PCT / 100).max(1)
}

/// The gap between two neighbouring top-row tabs (SAVE · MAP · EXIT): HALF A TAB wide.
/// User 2026-09-30: *"save와 map, exit … 각 버튼별 간격을 조금 더 벌려주고"* — it was
/// `pad * 2` (18 px at 1080 tall against an 86 px tab, the row read as one strip); now
/// 43 px. Taken from the tab's own width so it scales with it. ⛔ One function for every
/// gap in the row — MAP (`minimap_toggle_seat`) and EXIT (`exit_tab_rect`) both use it.
pub(crate) fn top_tab_gap(tab_w: u32) -> u32 {
    (tab_w / 2).max(1)
}

/// BUILT-IN seat of the MAP word (`crate::placement::CtrlId::MinimapToggle`, 141차): on
/// the top row, one gap ([`top_tab_gap`]) LEFT of SAVE, the same size. It used to sit in
/// the minimap's corner and appear only while the view was hidden; now it is a toggle on
/// the row the reference screenshot puts its words on.
pub(crate) fn minimap_toggle_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let save = quicksave_seat(win_w, win_h, pad);
    Rect::new(save.x.saturating_sub(save.w + top_tab_gap(save.w)), save.y, save.w, save.h)
}

/// ★ 143차: the EXIT tab (user 2026-09-30: *"하늘성안으로 들어갔을떄는 map버튼 왼쪽에 exit
/// 버튼을 둬서"*) — one gap ([`top_tab_gap`]) LEFT of wherever the MAP tab is (the
/// RESOLVED rect, so a MAP the player moved in the editor takes EXIT with it), the same
/// size. A MAP parked against the left edge gets it on its right instead, so the tab is
/// never off screen.
///
/// ⛔ Not a placeable control of its own: it exists only in the tower, where SAVE and MAP
/// are both hidden (user decision 2026-09-08, 「탑에서만 숨긴다」), so it always stands in
/// the row they left — exactly where the player's eye already goes for them.
pub fn exit_tab_rect(win_w: u32, map: Rect) -> Rect {
    let gap = top_tab_gap(map.w);
    let x = if map.x >= map.w + gap {
        map.x - map.w - gap
    } else {
        (map.x + map.w + gap).min(win_w.saturating_sub(map.w))
    };
    Rect::new(x, map.y, map.w, map.h)
}

/// ★ 143차 — THE WAKE's veil: the whole window washed toward white by `a` (0 = untouched,
/// 255 = solid white). Drawn LAST, over the game, the HUD and every box — it is the screen
/// itself going white, not a widget (the same rule as the entrance flash).
///
/// One pass, `c + (255 - c) * a / 255` per channel: at 2340x1080 that is the cost of one
/// extra blit for the three or four seconds the wake lasts, and nothing otherwise.
pub fn draw_wake_veil(buf: &mut [u32], a: u8) {
    match a {
        0 => {}
        255 => buf.fill(0xffff_ffff),
        _ => {
            let a = u32::from(a);
            let k = 255 - a;
            for p in buf.iter_mut() {
                let c = *p;
                let r = (c >> 16) & 0xff;
                let g = (c >> 8) & 0xff;
                let b = c & 0xff;
                let r = (r * k + 255 * a + 127) / 255;
                let g = (g * k + 255 * a + 127) / 255;
                let b = (b * k + 255 * a + 127) / 255;
                *p = 0xff00_0000 | (r << 16) | (g << 8) | b;
            }
        }
    }
}

/// The wake's veil as a function of time: `from` → `to` (0.0 clear … 1.0 white) over
/// `ms`, eased both ways (smoothstep) so the white SWELLS in and ebbs out — the "의식이
/// 돌아오는듯한" feel the user asked for — rather than ramping linearly. `ms == 0` = `to`.
pub fn wake_veil_alpha(from: f32, to: f32, elapsed_ms: u64, ms: u32) -> u8 {
    let t = if ms == 0 { 1.0 } else { (elapsed_ms as f32 / ms as f32).clamp(0.0, 1.0) };
    let e = t * t * (3.0 - 2.0 * t);
    let v = from + (to - from) * e;
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Fill of a top-row tab (2026-09-30): dark, translucent — the reference darkens the world
/// behind each word.
const TOP_TAB_FILL: u32 = 0xff0c_1018;
const TOP_TAB_FILL_A: u32 = 170;
/// Its border: the reference's amber bar (measured over black: `#845d18` at ~65 %).
const TOP_TAB_RIM: u32 = 0xffcb_8f25;
const TOP_TAB_RIM_DIM: u32 = 0xff6a_6a6a;
const TOP_TAB_RIM_A: u32 = 230;

/// ★ 146차: how far below a top-row tab its TAP area reaches, as a share of the tab's own
/// height (300 = the tab plus two more tab-heights under it).
pub const TOP_TAB_HALO_H_PCT: u32 = 300;

/// ★ 146차 — the TAP area of a top-row tab (SAVE · MAP · EXIT), much bigger than the box it
/// draws. User 2026-10-01: *"save버튼과 map버튼, exit 버튼의 실제 클릭영역을 대폭 증가시켜줘.
/// 지금 실제 버튼크기와 클릭되는 영역이 같으니까 누르기가 너무힘들어"*. The box stays the
/// size it is; the halo reaches half a gap ([`top_tab_gap`]) out on each side — so two
/// neighbours' halos MEET in the middle of the gap and never overlap — and down to
/// [`TOP_TAB_HALO_H_PCT`] of the tab's height. Clipped to the window.
///
/// ⛔ The halo is a LOW-priority target: `app.rs` tests it only after every real control
/// (plates, the page button, the minimap panel, the stats block, the HUD keys, the game's
/// own confirm) has had its turn, so the extra area can never steal a tap from something
/// the player can see there. The box itself keeps its old place in the chain.
pub fn top_tab_halo(r: Rect, win_w: u32, win_h: u32) -> Rect {
    if r.w == 0 || r.h == 0 {
        return r;
    }
    let side = top_tab_gap(r.w) / 2;
    let x0 = r.x.saturating_sub(side);
    let x1 = (r.x + r.w + side).min(win_w);
    let y1 = (r.y + r.h * TOP_TAB_HALO_H_PCT / 100).min(win_h);
    Rect::new(x0, r.y, x1.saturating_sub(x0), y1.saturating_sub(r.y))
}

/// ★ 146차: the radius of a tab's two bottom corners (user 2026-10-01: *"버튼 모서리부분 조금
/// 둥글게 변경해줘 지금 너무 뾰족해"*) — about a fifth of the tab's height, at least two
/// border widths so the rim visibly bends. The top corners stay square: that side IS the
/// screen's edge, and a rounded top would lift the tab off it.
fn top_tab_radius(r: Rect, t: u32) -> f64 {
    let rad = (r.h / 5).max(t * 2);
    f64::from(rad.min(r.w / 2).min(r.h))
}

/// Distance from pixel centre `(fx, fy)` (tab-local) INTO the tab's shape — its left,
/// right and bottom sides and the two rounded bottom corners (radius `rad`). The top is
/// open. Negative = outside.
fn top_tab_depth(fx: f64, fy: f64, w: f64, h: f64, rad: f64) -> f64 {
    let straight = fx.min(w - fx).min(h - fy);
    if fy <= h - rad {
        return straight;
    }
    let cx = if fx < rad {
        rad
    } else if fx > w - rad {
        w - rad
    } else {
        return straight;
    };
    let (dx, dy) = (fx - cx, fy - (h - rad));
    rad - (dx * dx + dy * dy).sqrt()
}

/// A top-row TAB (141차 word, box since 2026-09-30): a dark translucent rectangle hanging from the
/// top edge, bordered on its left, right and bottom sides only — the top side is the
/// screen's edge (player 2026-09-30: *"상단에 붙어있으니 맨윗줄테두리는없고 아래3변만
/// 테두리가 있는 직사각형 박스형태"*, after the reference `Screenshot_20260929_023746_5`).
/// The word is the game's own font in white with a dark shadow, centred in the box
/// above the bottom border. `dim` = the MAP tab while the view is hidden (grey border
/// and word). ★ 146차: the two bottom corners are rounded ([`top_tab_radius`], anti-aliased
/// fill and rim), and the rect is NO LONGER the hit area — the tap target is the much
/// bigger [`top_tab_halo`] around it.
pub fn draw_top_label(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, text: &str, dim: bool) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let t = (r.h / 14).max(2).min(r.w / 4).min(r.h / 4).max(1);
    let rim = if dim { TOP_TAB_RIM_DIM } else { TOP_TAB_RIM };
    let rad = top_tab_radius(r, t);
    let (w, h, tf) = (f64::from(r.w), f64::from(r.h), f64::from(t));
    for y in r.y..(r.y + r.h).min(win_h) {
        let fy = f64::from(y - r.y) + 0.5;
        let row = (y * win_w) as usize;
        for x in r.x..(r.x + r.w).min(win_w) {
            let d = top_tab_depth(f64::from(x - r.x) + 0.5, fy, w, h, rad);
            let cov = (d + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let i = row + x as usize;
            blend_px(buf, i, TOP_TAB_FILL, (f64::from(TOP_TAB_FILL_A) * cov).round() as u32);
            let on_rim = (tf - d + 0.5).clamp(0.0, 1.0) * cov;
            if on_rim > 0.0 {
                blend_px(buf, i, rim, (f64::from(TOP_TAB_RIM_A) * on_rim).round() as u32);
            }
        }
    }
    let inner = Rect::new(r.x + t, r.y, r.w.saturating_sub(t * 2), r.h.saturating_sub(t));
    let scale = game_fit_scale(text, inner.w * 9 / 10, inner.h * 8 / 10);
    let tw = game_text_w(text, scale);
    let x = inner.x + inner.w.saturating_sub(tw) / 2;
    let y = inner.y + inner.h.saturating_sub(crate::gamefont::KOR_H as u32 * scale) / 2;
    let (ink, shadow) = if dim { (0xff9a_9a9a, 0xff10_1010) } else { (LABEL, 0xff08_0a10) };
    let d = (scale / 2).max(1);
    draw_game_text(buf, win_w, win_h, x + d, y + d, scale, text, shadow);
    draw_game_text(buf, win_w, win_h, x, y, scale, text, ink);
}

/// One small control tucked BELOW-LEFT of the lowest quickbar plate.
///
/// The STICK valve is kept from the row layout this replaces, and for the same measured
/// reason: `potion_button` was caught by the selftest at 1080x2340 doing exactly this —
/// "a row extending left along the bottom edge runs into the stick's home on a narrow
/// screen" — and a control under the walking thumb steals steering. Built-ins are the
/// rect a rejected placement falls BACK to, so an overlapping built-in has no way out.
/// It turns the other way here: with no room to the left the button goes straight DOWN
/// from the plate, which is the direction the player asked for in the first place.
fn below_left_of_lowest_plate(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let ring = ring_all(win_w, win_h, pad);
    let base = ring
        .get(1..)
        .unwrap_or_default()
        .iter()
        .copied()
        // ★ The LIVE seat count, not `RING_SEATS`: with plates switched off the ring is
        // shorter, and asking for a seat past its end is exactly the out-of-range index
        // quietly answered with the corner fallback that this function exists to fix.
        .max_by_key(|r| r.y)
        .unwrap_or_else(|| ring_seat(win_w, win_h, pad, ring_seats_live().saturating_sub(1)));
    // Plate-sized: the player asked for the ICON inside it to shrink, not the button
    // (see `QUICKPAGE_ICON_SPAN`).
    let side = base.w;
    // ★ 91차: the STICK seat is the arrows' square too (touch type B walks with them
    // on the field), so the whole square is what the page button must stay clear of.
    let seat = stick_seat(win_w, win_h);
    let stick_right = seat.x + seat.w + pad * 2;
    // Half a plate lower than the plate's own top = clearly "under" it, not "level".
    let y = (base.y + base.h / 2).min(win_h.saturating_sub(side));
    let x = base.x.saturating_sub(side + pad * 2);
    if x >= stick_right {
        Rect::new(x, y, side, side)
    } else {
        Rect::new(base.x, (base.y + base.h + pad).min(win_h.saturating_sub(side)), side, side)
    }
}

/// BUILT-IN bounding square of the ONE walk control, [`crate::placement::CtrlId::Stick`]
/// — bottom-left, inset `pad * 4` from both edges.
///
/// ★★★ 91차 (2026-09-13) — the stick and the four-arrow D-pad are ONE control. Player:
/// "인게임 조이스틱이랑 메뉴열렸을때 화살표를 그냥 stick이라는 하나의 옵션으로 관리하고
/// 싶어". Until then `CtrlId::Dpad` was a second control, and since the layout editor
/// shows the FIELD set filtered by the live touch type, a TYPE A player saw only the
/// stick disc and had NO way to resize the menu arrows once 83차 removed the MENU page
/// ("현재 설정창에서 인게임 메뉴가 열렸을떄 화살표 크기는 조절이 불가능한데"). Now this
/// square is what every role is cut from:
///
///  * the MENU arrows (and a dialogue CHOICE's arrows) — `dpad_arms` tiles it,
///  * TOUCH TYPE B's field arrows — the same tiling,
///  * TOUCH TYPE A's stick home — a disc at the square's CENTRE whose rim runs through
///    the four hexagons' centres ([`joy_home`] / [`stick_base_radius`]).
///
/// ★ SIZE: `2 x` the old three-cell key column (`3 * cell + 2 * pad`), up from `7/4`
/// — "누르기가 좀 어렵다는 말이 있어서 버튼 크기를 좀 늘리는게 나을것 같아". At 1080 tall
/// that is 546 -> 624 px. The left inset went from `win_w / 26` (85 px) to `pad * 4`
/// (36 px, the same as the bottom inset), which is what keeps the grown square clear
/// of the game's own menu panel: on an 18:9 window the panel's left edge is at 684 px
/// and the old inset would have put the square's right band under it.
///
/// ⛔ The history behind the numbers: 2026-08-09 5/4 ("버튼 크기도 조금더 크게해줘"),
/// 2026-08-11 11/8 ("default 크기를 조금만 더 키워줘"), 2026-08-17 7/4 ("높이랑 밑변
/// 둘다 좀 더 늘려줘"), 91차 2. The square is the only free variable — `dpad_arms`
/// explains why the arm RATIOS cannot give height and base at once.
///
/// ★★ 80차-F: BOTTOM-ANCHORED. 79차-B removed the `1 3 7 9 #` key block that used to
/// sit above the arrows and let the square "centre on its own", which walked it into
/// the vertical middle of the screen; the player asked for the seat they had
/// (2026-09-05: "좌측에 화살표 나오는것도 원래 좌측하단쪽에 위치했엇는데 … 갑자기 좌측
/// 중앙정렬로 바뀌었어"). ⛔ Do not give the menu arrows an anchor of their own again —
/// two anchors means the arrows jump when a window opens. Since 91차 there is not even
/// a second function to give one to.
///
/// Clamped into the window for short screens; the player moves and resizes it in the
/// layout editor like any other control (`placement::Table` wins over this).
pub(crate) fn stick_seat(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let cell = (win_h / 11).clamp(40, 120);
    let span = (cell * 3 + pad * 2) * 2;
    let span = span.min(win_h.saturating_sub(pad * 4).max(1)).min(win_w.max(1));
    let x = (pad * 4).min(win_w.saturating_sub(span));
    let y = win_h.saturating_sub(span + pad * 4);
    Rect::new(x, y, span, span)
}

/// One arrow of the D-pad: where the hexagon is INKED and where a touch counts.
pub(crate) struct Arm {
    /// The hexagon's bounding box. Its vertex on the `dir` axis points OUTWARD.
    pub draw: Rect,
    /// The touch target — the arrow's diagonal QUARTER of the square. Always a
    /// superset of the hexagon, and disjoint from every other arm's.
    pub hit: Wedge,
    pub dir: Dir,
}

/// A quarter of the D-pad square cut by BOTH diagonals — one arrow's touch target
/// (91차-B). The four quarters tile the square exactly: a point is in the quarter
/// whose axis it is closest to, ties on a diagonal go to UP / DOWN, and a small
/// DEAD DISC at the centre belongs to nobody (a thumb resting in the middle must not
/// press anything).
///
/// ★ Why quarters and not the 2026-08-17 horizontal BANDS: a band is only as tall as
/// its share of the square, so the arrows had to be short to be wide, and the
/// player's "뚱뚱한 육각형 … 크기 더 늘려줘" needed both. A quarter is `s/2` deep AND
/// `s` wide at the rim, so a hexagon 0.44 wide and 0.34 tall fits in every one of
/// them at once — and sliding a thumb from one arrow to the next crosses a line, not
/// a dead strip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Wedge {
    pub square: Rect,
    pub dir: Dir,
}

impl Wedge {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let sq = &self.square;
        if !sq.contains(x, y) {
            return false;
        }
        let cx = f64::from(sq.x) + f64::from(sq.w) / 2.0;
        let cy = f64::from(sq.y) + f64::from(sq.h) / 2.0;
        let (dx, dy) = (x - cx, y - cy);
        let dead = f64::from(sq.w.min(sq.h)) * f64::from(DEAD_PCT) / 100.0;
        if dx * dx + dy * dy < dead * dead {
            return false;
        }
        match self.dir {
            Dir::Up => -dy >= dx.abs(),
            Dir::Down => dy >= dx.abs(),
            Dir::Left => -dx > dy.abs(),
            Dir::Right => dx > dy.abs(),
        }
    }
}

/// Length of an arrow's hexagon ALONG its axis, as a percent of the square's side.
///
/// ★ 91차-B (2026-09-13), player: "화살표 크기 더 늘려줘야해. 정육각형이 될필요없어
/// 뚱뚱한 육각형인게 더 좋아." The three numbers below are what fits a diagonal
/// quarter: with the tip at `inset` from the rim and the flat sides ending `TIP` short
/// of the length, the far corners sit `inset + LEN - TIP` in from the rim, where the
/// quarter is still `2 * (50 - inset - LEN + TIP)` % wide — `0.484` of the side at
/// these values, so a `0.44` hexagon clears the diagonal by ~0.02 s on each corner.
pub(crate) const ARM_LEN_PCT: u32 = 34;
/// Width of the hexagon ACROSS its axis, as a percent of the side. Wider than it is
/// long — the "뚱뚱한" — and 1.57x the 91차 regular-ish hexagon's 0.28.
pub(crate) const ARM_WIDTH_PCT: u32 = 44;
/// Each tip's share of the hexagon's length. 30 % leaves the two flat sides 40 % of
/// the length; a regular hexagon's 25 % would give the fat shape a roof too shallow to
/// read as pointed.
pub(crate) const HEX_TIP_PCT: u32 = 30;
/// Radius of the dead disc at the square's centre, as a percent of the side. The
/// hexagons' inner tips sit at `50 - inset - LEN` = 14 % from the centre, so 8 %
/// leaves clear ground between the disc and every arrow.
pub(crate) const DEAD_PCT: u32 = 8;

/// The four arrows inside a D-pad bounding square, in draw / hit-test order: up, down,
/// left, right.
///
/// ★ ONE control, four buttons. The player asked for the arrows to be sized and moved
/// **together** ("화살표는 한개씩 크기조절 및 위치 조정하는게 아니라 조이스틱처럼 한번에
/// 4개의 화살표를 모두 조절하는 방식"), so `placement` knows only [`crate::placement::
/// CtrlId::Stick`] — a single square — and this derives the four targets from whatever
/// square that resolves to. Nothing else in the layout system, the editor or the
/// on-disk format has to know there are four of them.
///
/// ── GEOMETRY, 91차-B (2026-09-13) ───────────────────────────────────────────────
/// **INK** — each arm is a FAT HEXAGON: [`ARM_WIDTH_PCT`] of the side across its axis,
/// [`ARM_LEN_PCT`] along it, tips of [`HEX_TIP_PCT`] of the length at both ends (the
/// outer one at `inset` from the rim, the inner one pointing at the centre). All four
/// are congruent by rotation. At a 624 px square that is 275 x 212 px, against the
/// 176 x 188 of 91차's first cut and the 229 x 140 triangles before it — ~65 % more
/// ink than the first cut.
///
/// **HIT** — the square cut by its two diagonals into four [`Wedge`]s, plus a dead
/// disc at the centre. Every pixel of the square belongs to exactly one arrow (or to
/// the disc), the quarters share no pixel — the "겹치는건 안돼" requirement holds by
/// construction — and each hexagon lies inside its own quarter, which is what makes
/// the ink a subset of the target:
/// ```text
///   +---------------+
///   |\     UP      /|
///   | \    /\     / |   quarter = the axis a point is nearest to
///   |  \  |  |   /  |   ties on a diagonal -> UP / DOWN
///   | L  \|__|/  R  |   centre disc       -> nobody
///   |     /  \      |
///   |    /    \     |
///   |   /      \    |
///   |  /  DOWN  \   |
///   | /          \  |
///   +---------------+
/// ```
pub(crate) fn dpad_arms(r: Rect) -> [Arm; 4] {
    let s = r.w.min(r.h);
    let inset = (s / 50).max(1);
    let len = (s * ARM_LEN_PCT / 100).max(3);
    let wid = (s * ARM_WIDTH_PCT / 100).max(3);
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let hit = |dir| Wedge { square: r, dir };
    [
        Arm {
            draw: Rect::new(cx.saturating_sub(wid / 2), r.y + inset, wid, len),
            hit: hit(Dir::Up),
            dir: Dir::Up,
        },
        Arm {
            draw: Rect::new(cx.saturating_sub(wid / 2), (r.y + r.h).saturating_sub(inset + len), wid, len),
            hit: hit(Dir::Down),
            dir: Dir::Down,
        },
        Arm {
            draw: Rect::new(r.x + inset, cy.saturating_sub(wid / 2), len, wid),
            hit: hit(Dir::Left),
            dir: Dir::Left,
        },
        Arm {
            draw: Rect::new((r.x + r.w).saturating_sub(inset + len), cy.saturating_sub(wid / 2), len, wid),
            hit: hit(Dir::Right),
            dir: Dir::Right,
        },
    ]
}

/// Radius of the STICK's base disc inside a resolved seat: the circle through the four
/// arrow hexagons' CENTRES.
///
/// ★★★ 2026-09-20, player: "화살표의 크기랑 조이스틱 크기를 똑같게 해놨는데 화살표 크기보다
/// 조이스틱이 조금 더 작아야해. 화살 육각형들의 각 중심점을 원으로 연결한게 조이스틱크기라고
/// 생각하면돼". 91차-B made the base the seat's INSCRIBED circle, which put its rim out at
/// the hexagons' outer tips — the disc and the arrows ended at the same place and read as
/// one blob. Through the centres the arrows keep their outer half OUTSIDE the disc, which
/// is what makes the two shapes tell themselves apart.
///
/// ⛔ Derived from [`ARM_LEN_PCT`] and the same `inset` [`dpad_arms`] uses, never from a
/// separate percentage of the side: two numbers for one circle is how the base and the
/// arrows drift apart. At a 624 px seat that is `312 - 12 - 106` = **194 px** (0.62 of the
/// old 312). A non-square seat has no such circle at all, so the SHORTER axis wins and the
/// disc stays inside every arm centre.
pub(crate) fn stick_base_radius(seat: &Rect) -> f64 {
    let s = seat.w.min(seat.h);
    let inset = f64::from((s / 50).max(1));
    let half_len = f64::from((s * ARM_LEN_PCT / 100).max(3)) / 2.0;
    let v = f64::from(seat.h) / 2.0 - inset - half_len;
    let h = f64::from(seat.w) / 2.0 - inset - half_len;
    v.min(h).max(1.0)
}

/// The six corners of an arm's hexagon, clockwise from the OUTWARD tip, for a
/// bounding box `r` and a pointing axis `dir`. The two vertices on the axis are the
/// tips, each [`HEX_TIP_PCT`] of the box's length on that axis; the two flat sides
/// are what is left between them.
pub(crate) fn hexagon(r: Rect, dir: Dir) -> [(f64, f64); 6] {
    let (x0, y0) = (f64::from(r.x), f64::from(r.y));
    let (x1, y1) = (x0 + f64::from(r.w), y0 + f64::from(r.h));
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let tip = f64::from(HEX_TIP_PCT) / 100.0;
    match dir {
        Dir::Up | Dir::Down => {
            let t = (y1 - y0) * tip;
            [(cx, y0), (x1, y0 + t), (x1, y1 - t), (cx, y1), (x0, y1 - t), (x0, y0 + t)]
        }
        Dir::Left | Dir::Right => {
            let t = (x1 - x0) * tip;
            [(x1, cy), (x1 - t, y1), (x0 + t, y1), (x0, cy), (x0 + t, y0), (x1 - t, y0)]
        }
    }
}

/// Ring position 0 (nearest the disc's right side) is the CANCEL key; the rest are
/// the quickbar plates. Keeping them on ONE ring is what gives the reference look of
/// the action button being encircled by its skills.
fn ring_all(win_w: u32, win_h: u32, pad: u32) -> Vec<Rect> {
    ring_slots(win_w, win_h, ring_seats_live(), pad)
}

/// Seats the ring actually lays out right now: CLR plus one per VISIBLE plate.
///
/// ★★ Player request 2026-09-10 ("퀵슬롯이 너무많아서 불편하다는 유저도있어서"): a plate
/// switched off in the layout editor gives its SEAT back, so the survivors spread
/// further along the arc — up to [`RING_STEP_MAX`] — and grow: `ring_slots` takes the
/// largest side that fits, and a wider pitch pulls the ring in and lets that side be
/// bigger. Two ceilings bound it, so a ring that already fits is unchanged: the pitch
/// cap and `ring_slots`'s own `win_h / 11` base size.
///
/// Everything derived from the ring follows for free, which is the point of deriving
/// them: `cancel_seat` re-measures its clearance from the FIRST VISIBLE plate,
/// `quickpage_seat` from the lowest visible one, `quicksave_seat` takes its size from
/// the same plate. ⛔ Do not go back to a literal seat count here — see the
/// `quickpage_seat` note about the page button that ended up in the corner.
fn ring_seats_live() -> usize {
    1 + crate::quickpage::visible_count(crate::quick_hidden())
}

/// The ring as it would be with every plate switched on.
///
/// For the one control that is SIZED from a ring seat without being on the ring (the
/// minimap toggle): a player who switches two plates off has not asked for an
/// unrelated top-right button to change size, and "why did the minimap toggle grow?"
/// is a puzzle nobody needs. ⛔ Not for anything that sits ON the ring.
fn ring_all_full(win_w: u32, win_h: u32, pad: u32) -> Vec<Rect> {
    ring_slots(win_w, win_h, RING_SEATS, pad)
}

/// [`ring_seat`] on the full ring — see [`ring_all_full`].
pub(crate) fn ring_seat_full(win_w: u32, win_h: u32, pad: u32, i: usize) -> Rect {
    seat_or_corner(ring_all_full(win_w, win_h, pad), win_w, win_h, pad, i)
}

/// BUILT-IN rect of one quickbar PLATE, hidden set applied.
///
/// A visible plate takes its place in the live ring; a HIDDEN one is answered with its
/// seat on the full ring. That fallback is never drawn or hit-tested — `quick_buttons`
/// zeroes hidden plates and `placement::shown` keeps them out of the editor — but it
/// has to be a real rect all the same: `placement::accepts` rejects a zero-sized
/// candidate, `Place::from_rect` divides a stored placement by it, and the resolver
/// wants one built-in per id in `ids_for`. Deterministic and off the live ring is what
/// makes it harmless.
pub(crate) fn plate_seat(win_w: u32, win_h: u32, pad: u32, plate: u8) -> Rect {
    match crate::quickpage::visible_index(crate::quick_hidden(), plate) {
        Some(seat) => ring_seat(win_w, win_h, pad, seat + 1),
        None => ring_seat_full(win_w, win_h, pad, usize::from(plate) + 1),
    }
}

/// Seats on the action ring: CLR + every quickbar plate.
///
/// ★ Derived from [`crate::quickpage::PLATES`] rather than written out (2026-09-04,
/// when it went from 5 plates to 7). `ring_slots` spreads `n` seats over a FIXED arc
/// and shrinks the seat until they fit, so adding plates makes each one a little
/// smaller and moves nothing else — and every seat stays individually placeable
/// through `placement`. A literal here is how the page button ended up in the
/// bottom-right corner once already (see `quickpage_seat`).
const RING_SEATS: usize = 1 + crate::quickpage::PLATES;

/// The SECONDARY-WEAPON button (field key `7`), added on player request
/// (2026-07-30): "7 is the secondary-weapon attack and there is no button for it".
///
/// IMMEDIATELY left of the action disc, level with it, one gap apart — the player
/// asked for the two to read as a pair (2026-07-30; the first version sat outside
/// the skill ring, i.e. further left than the `#`/`9` plates, which was awkward).
/// Still larger than a skill plate and smaller than the disc. `ring_slots` pushes
/// the ring out to clear it, so this must NOT call back into the ring.
pub fn secondary_button(win_w: u32, win_h: u32) -> Rect {
    let pad = (win_w.min(win_h) / 110).max(3);
    let a = action_disc(win_w, win_h, pad);
    let d = (a.w * 3 / 5).max(56);
    Rect::new(a.x.saturating_sub(pad * 2 + d), (a.y + a.h / 2).saturating_sub(d / 2), d, d)
}

/// BUILT-IN minimap geometry: the EXTREME top-right corner (player request
/// 2026-07-30 — "the minimap has to be at the very top-right, and the monster
/// name/HP/LV box to its left"). Grown twice on request (win_h/5 -> /4 -> this).
///
/// Pure function of the window — the player's own position and size are applied on
/// top by [`crate::placement`], never in here.
pub(crate) fn minimap_panel_builtin(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let side = (win_h * 26 / 100).clamp(120, 280);
    Rect::new(win_w.saturating_sub(pad * 2 + side), pad * 2, side, side)
}

/// The minimap panel as the player actually sees it. Tapping it presses `*`, which
/// is what makes the game (re)paint the offscreen minimap buffer we mirror here.
pub fn minimap_panel(win_w: u32, win_h: u32) -> Rect {
    crate::placement::rect_of(crate::placement::CtrlId::Minimap, Mode::Field, win_w, win_h).unwrap_or_else(|| minimap_panel_builtin(win_w, win_h))
}

/// How far LEFT (in GUEST px) the game's own target-monster box has to move so it
/// does not end up underneath our minimap panel.
///
/// ★ Rewritten 2026-07-31 for free placement. This used to invert the panel's
/// right-edge anchor unconditionally — correct only while the panel was nailed to
/// the top-right corner. Dragged to the left half of the screen the old formula
/// asked for ~476 guest px, clamped to [`crate::items::TARGET_BOX_SHIFT_MAX`], and
/// parked the game's own monster box in the middle of the field, dodging something
/// that was no longer there. It is now an OVERLAP QUERY: the box moves only when
/// the panel actually covers it, and only by as much as it takes.
pub fn target_box_shift(game: Rect) -> u32 {
    target_box_shift_for(game, minimap_panel(game.w, game.h))
}

fn target_box_shift_for(game: Rect, map: Rect) -> u32 {
    if game.w == 0 || !crate::immersive_active() || game.w <= game.h {
        return 0;
    }
    let base = game_target_box(game);
    if !base.intersects(&map) {
        return 0;
    }
    // Move it left until its right edge clears the panel's left edge, + a small gap.
    let need = (base.x + base.w).saturating_sub(map.x);
    let (lw, _) = crate::logical();
    // The binding guest site is `subs r1,#44`, so the immediate tops out at 255-44.
    (need * lw.max(1) / game.w + 2).min(crate::items::TARGET_BOX_SHIFT_MAX)
}

/// The game's target-monster box where it will ACTUALLY be drawn, i.e. after the
/// host has poked its anchor to dodge `map`.
pub(crate) fn game_target_box_at(game: Rect, map: Rect) -> Rect {
    let base = game_target_box(game);
    let dx = gx(game, target_box_shift_for(game, map));
    Rect::new(base.x.saturating_sub(dx), base.y, base.w, base.h)
}

/// Can the game's box still get out of the way of a panel at `map`? False when the
/// dodge the panel demands no longer encodes, in which case the game's own monster
/// name and HP bar would be drawn underneath our panel — which is why the editor
/// refuses such a placement instead of letting the player discover it in a fight.
pub(crate) fn target_box_can_dodge(win_w: u32, win_h: u32, map: Rect) -> bool {
    let game = Rect::new(0, 0, win_w, win_h);
    if !crate::immersive_active() || win_w <= win_h {
        return true;
    }
    !game_target_box_at(game, map).intersects(&map)
}

/// The settings gear + the `123` keypad toggle, as a pure function of the window.
///
/// ★ 2026-07-30: TOP-CENTRE, nudged slightly RIGHT of the middle (player request).
/// The old spot was beside the gauge block on the left, which the player kept
/// meeting when they did not want it. `123` is a touch larger than the gear
/// ("아주 조금만 크게").
///
/// The settings gear in MENU mode (immersive). Extracted from `layout()` so
/// [`crate::placement`] can treat it as a no-go area: it is hit-tested BEFORE the
/// button list, so a control parked on it is a control that silently opens the settings
/// instead of doing its job. In the field `layout()` zeroes it.
///
/// ★ The `123` keypad toggle that used to sit right of the gear was removed on
/// 2026-09-18 (player request). The gear keeps the exact seat it had beside it — the
/// spacing terms below are that row's geometry, kept so nothing on screen moved.
pub(crate) fn gear_rect(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let gs = (win_w.min(win_h) / 22).clamp(30, 64);
    let ts = (gs * 6 / 5).min(win_h / 8);
    let gap = pad * 2;
    let total = gs + gap + ts;
    let x0 = (win_w / 2 + win_w / 14).min(win_w.saturating_sub(total + pad)) - total / 2;
    let row_y = pad * 2;
    Rect::new(x0, row_y + (ts.saturating_sub(gs)) / 2, gs, gs)
}

fn layout_immersive(win_w: u32, win_h: u32, _pad: u32, mode: Mode, choice: bool) -> (Rect, Vec<Button>, Option<Rect>) {
    let game = Rect::new(0, 0, win_w, win_h);
    let mut buttons = Vec::new();
    // ★ Every rect below comes from the placement resolver, which is the built-in
    // geometry unless the player has moved or resized that control in the layout
    // editor — and which guarantees the whole set stays disjoint and on screen.
    let placed = crate::placement::resolve(mode, win_w, win_h);
    let at = |id: crate::placement::CtrlId| {
        placed
            .iter()
            .find(|(c, _)| *c == id)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| Rect::new(0, 0, 0, 0))
    };
    // Big ACTION disc, bottom-right. NOT in a cutscene (player request 2026-08-16:
    // "컷신 일때는 … 확인버튼이 노출되면 안돼" — 2026-08-03's "OK and #" pair is
    // superseded; since 64차 a scene draws NO disc of ours at all).
    // ★ NEVER in the script window, choice or not (2026-08-17, player: "선택지가 있을떄
    // 화살표있고, OK버튼은 없고"). Confirming is what the whole screen already does
    // (`tap_anywhere_key`), so a disc would be a second copy of a control that is
    // already the size of the window.
    if mode != Mode::Cutscene {
        buttons.push(Button {
            rect: at(crate::placement::CtrlId::Ok),
            key: KeyCode::OK,
            label: Label::Text(""),
            dpad: false,
            face: Face::Action,
            wedge: None,
        });
    }
    // CANCEL beside the first skill plate. NOT in a cutscene: CLR does nothing during
    // a script, and a scene should be watched, not overlaid (2026-08-16). The seat
    // used to be reused there for the `#` skip disc — removed in 64차; the game's own
    // `# SKIP` lettering is the skip control now.
    //
    // ★ A "취소" button for a dialogue CHOICE was tried on 2026-08-17 and REVERTED the
    //   same day, by the player's own reasoning: the guest has no cancel to call. The
    //   game's own choice widget advertises ONE key — it draws an `OK` bar under the
    //   rows and nothing else — and declining is a ROW in the list ("지금은 별로"), not
    //   a key. Faking it host-side would mean writing the cursor index
    //   (`CHAR+0xa284`) to "the row that means no" and synthesising OK, and NOTHING
    //   tells us which row that is: it is last in the 의뢰 list, but a list may have no
    //   decline row at all, and guessing wrong ACCEPTS a quest the player refused.
    //   A button that cannot do what its label says is worse than no button.
    if mode != Mode::Cutscene {
        buttons.push(Button {
            rect: at(crate::placement::CtrlId::Cancel),
            key: KeyCode::CLEAR,
            label: Label::Text("CLR"),
            dpad: false,
            face: Face::Round,
            wedge: None,
        });
    }
    // Menu mode: the navigation keys 1/3/7/9/# down the LEFT edge, while the
    // quickbar plates stay in their ring seats on the right (drawn separately, from
    // `quick_buttons`). Both sets at once is the 2026-08-03 request; before that the
    // ring seats were shared, so opening a window took the plates away.
    //
    // ★ What those right-hand plates DO changes with the mode and cannot not: OFF the
    // field they send the quickslot hotkeys 1/3/9/0/# BY PLATE INDEX, and it is the
    // WINDOW's key handler that receives them, not the field dispatcher — so they
    // navigate the window instead of casting. That is the game's own key routing; the
    // buttons are still the fastest way to send those digits with the right thumb.
    // ★★ 79차: on the FIELD the same plates go through the worker instead
    // (`InputEvent::QuickActivate`), because a plate is no longer a guest slot — there
    // are `quickpage::PLATES` of them over five slots. Plates past the fifth therefore
    // have no digit here and do nothing off the field, which is the right answer:
    // which digit a plate sends must not depend on what is resident in it.
    // ★ 2026-08-09: the four-arrow D-pad, under the menu key block. A FIXED control —
    // unlike the floating stick, it is exactly where it is drawn — for the one
    // thing the stick is bad at: stepping a menu cursor one row at a time. The
    // stick latches a direction and auto-repeats it, which is right for walking
    // and wrong for choosing.
    //
    // ★ On the FIELD too under 십자키 since 2026-08-16 (player: "화살표만 나와야해"
    // — the pinned stick of the first TYPE B round went then, and came back as its
    // own 고정 스틱 choice on 2026-09-22 — see `crate::StickMode`). Same control,
    // same bottom-left seat; holding an arm walks (the key stays down for the life of
    // the touch, and sliding between arms turns — `app.rs` slide_touches), which is
    // exactly what the pinned stick does.
    //
    // ★★★ 91차: the square they are cut from is `CtrlId::Stick`'s — the ONE walk
    // control. `CtrlId::Dpad` is gone; the stick disc (the two stick modes, field) and
    // these arrows (menu / choice / 십자키 field) share a seat and a size knob. See
    // `stick_seat`.
    //
    // Pushed AFTER the menu keys so a player who drags a key on top of an arm
    // keeps the key (first match wins — see `hit`), and BEFORE the joystick zone
    // is consulted at all (`app.rs` hit-tests `layout.buttons` first), which is
    // what stops an arm tap from being read as a walk. NEVER in a plain cutscene:
    // a scene draws no controls of ours (2026-08-16, and 64차 took the skip disc too).
    //
    // `dpad: true` gives them the same tint as the classic layout's D-pad: these
    // are the same four keys, so they should not read as a different kind of
    // control.
    // ★ 2026-08-17, in TWO steps, and the second one is the rule:
    //
    //   (a) 54차 removed the arrows from the script window entirely, and that made the
    //       game's own dialogue CHOICES unanswerable — the player could not pick 예/
    //       아니오 on a quest ("선택지를 못 골랐다"). Window id 4 is BOTH story narration
    //       and NPC dialogue (`items::cutscene_open`'s own doc, and device-confirmed:
    //       the quest guide reads `win=4` exactly like the prologue).
    //   (b) So they are told apart by a different byte instead —
    //       `items::script_choice_open` (`CHAR+0xa0dc`, 1 only while a choice list is
    //       up) — which is what `choice` carries here. Player's rule, verbatim:
    //       "컷신일때는 안나오고 인게임 내에서 대화에 선택지가 있을때는 나오고".
    //
    // A cutscene with no choice therefore has NO arrows (the 54차 look is intact), and
    // the moment a choice appears they are there at full strength, on the MENU seat —
    // which is where `placement` puts a cutscene's controls anyway (Cutscene aliases
    // Menu), i.e. the same arrows the player already uses to walk a menu.
    //
    // ★ 59차: and NOT in 게임패드 HUD mode, anywhere — field, menu or choice (player,
    // 2026-08-19: "패드모드일때 화살표는 인게임과 메뉴 전부 없애도 될것같고"). The pad's
    // own hat/stick walks and steps the cursor through the same quantiser
    // (`gamepad::gamepad_axes` -> `joy_dir`), so on-screen arrows would be a second
    // copy of a control the player's thumb is already on. Under the default AUTO
    // preference a screen touch brings them straight back — see `crate::pad_hud_active`.
    if !crate::pad_hud_active()
        && (mode == Mode::Menu || (mode == Mode::Cutscene && choice) || (mode == Mode::Field && crate::stick_mode().field_arrows()))
    {
        for arm in dpad_arms(at(crate::placement::CtrlId::Stick)) {
            buttons.push(Button {
                // ★ The QUARTER, not the hexagon: a touch anywhere in the arrow's
                // diagonal quarter of the square counts, corners included (2026-08-17's
                // "표시되는 영역보다 조금 더 큰범위" rule, on the 91차-B partition). The
                // ink rect rides along in the face; `rect` is the whole square for the
                // callers that want a bounding box, `wedge` is the real target.
                rect: arm.hit.square,
                wedge: Some(arm.hit),
                key: match arm.dir {
                    Dir::Up => KeyCode::UP,
                    Dir::Down => KeyCode::DOWN,
                    Dir::Left => KeyCode::LEFT,
                    Dir::Right => KeyCode::RIGHT,
                },
                // The BUTTON is the hexagon and it paints its own glyph. `Label` is
                // unused for this face and is only here because `Button` requires one.
                label: Label::Text(""),
                dpad: true,
                face: Face::Hexagon(arm.dir, arm.draw),
            });
        }
    }
    // Cutscene: the only control of ours is the hit target over the GAME'S OWN
    // `# SKIP` lettering in the bottom-right corner (63차, player: "우측하단 skip
    // 이라는 문구 클릭했을때 skip 되도록"). `Face::None` draws NOTHING: the game
    // already painted the words, and anything over them would hide the very thing
    // being tapped. The key was always the skip — the game advertises it itself,
    // `res/menu/helpbody_txt` reads "이벤트 스킵: #".
    //
    // ★ 64차 (2026-08-21): the translucent skip DISC that used to sit on the CANCEL
    // seat is GONE — player: "우측하단 '#skip'을 눌러서 skip하는게 가능해져서 따로
    // 기존 skip버튼(기존 취소 or #버튼 위치)는 필요없을것 같아. 제거해줘." The game's
    // own lettering is the one skip control now, during a CHOICE too: what `#` does
    // with a choice box open was never confirmed (§55차-D), so the disc's
    // choice-state `#` was a button of unknown effect — nothing confirmed is lost
    // with it. (`Face::Skip`, its faint label ink and `MENU_KEY_HASH` left with the
    // disc.)
    //
    // ★ …and NOT while a pad is driving (2026-08-20, the rule the disc followed and
    // the arrows still do): `#` on the pad skips, so a scene overlay would be a
    // second copy of a control the player's thumb is already on.
    if mode == Mode::Cutscene && !crate::pad_hud_active() {
        buttons.push(Button {
            rect: game_skip_hint(game),
            key: KeyCode::HASH,
            label: Label::Text(""),
            dpad: false,
            face: Face::None,
            wedge: None,
        });
    }
    // Field-only buttons: the secondary-weapon attack (key `7`) and the minimap
    // panel, which doubles as the `*` key (that is what repaints it). Off-field
    // neither is drawn, and leaving them hit-testable would fire their key on a
    // blind tap (a source of the "map opened by itself" report).
    if mode == Mode::Field {
        buttons.push(Button {
            rect: at(crate::placement::CtrlId::Secondary),
            key: KeyCode::NUM7,
            // Face is drawn by `draw_secondary` (weapon icon, or a dimmed "7").
            label: Label::Text(""),
            dpad: false,
            face: Face::Round,
            wedge: None,
        });
        // ★ The minimap panel is deliberately NOT a Button. As one it fired STAR on
        // touch-DOWN and — worse — any touch that had not been adopted by the joystick
        // acquired it just by SLIDING over the corner, which is the player-reported
        // "the minimap keeps getting clicked by itself". app.rs gives it real
        // tap semantics (down and up inside, little movement) instead.
    }
    // LSK/RSK and 2/4/5/6/8/0/* live on the pull-out keypad (toggle beside the
    // gear) rather than permanently on screen.
    //
    // ★ The stick zone is the WHOLE WINDOW under 자유 스틱 (a touch that misses
    // every control starts a stick right there) — and NOTHING under 십자키: the
    // second 2026-08-16 round removed the pinned stick outright ("조이스틱은 터치타입
    // B에서 히든처리 되어야해"); 십자키 walks with the D-pad arrows pushed above,
    // which are ordinary buttons and need no zone.
    //
    // ★★ And the SEAT's square under 고정 스틱 (2026-09-22): the base is drawn there
    // and never moves, so only a touch on it may start a drag — anywhere else on the
    // field must stay what it was under 십자키, i.e. nothing. `App::joy_start` takes
    // the drag's centre from that same seat (`joy_home`). The square, not the disc:
    // the arrows' corners are part of the drawn control (`stick_base_radius` runs
    // through the hexagons' centres, inside the square), and a zone smaller than what
    // is drawn is a control that ignores some of its own taps.
    //
    // ★ FIELD ONLY since 2026-08-17 (player: "메뉴 열렸을때 좌측하단에 조이스틱
    // placeholder 없애줘"). The zone carries the idle HOME HINT with it, so off-field
    // it drew a stick under the menu's own D-pad — two walking controls on a screen
    // where walking does not exist. A cutscene was already excluded; a menu is the
    // same argument, and the menu D-pad (pushed above, in EVERY touch type) is what
    // steps a cursor.
    //
    // ★ Dropping the ZONE and not just the drawing is deliberate: a zone that outlives
    // what is drawn is an invisible control, which is the rule `App::stats_tap_zone`
    // states — here it would swallow menu taps into a stick nobody can see.
    //
    // ★ And none in 게임패드 HUD mode (59차), for the reason the arrows go too: the
    // stick is on the pad. Dropping the ZONE and not just the drawing matters here as
    // much as it does for touch type B — a live zone with nothing drawn is an
    // invisible control, and it would eat the very tap that switches AUTO back to the
    // touch HUD.
    let zone = if crate::pad_hud_active() || mode != Mode::Field {
        None
    } else {
        stick_zone_on_field(crate::stick_mode(), win_w, win_h)
    };
    (game, buttons, zone)
}

/// The FIELD's stick zone for a 조작 방식: the whole window (자유 스틱), the resolved
/// STICK seat (고정 스틱), or none (십자키 — the arrows are buttons). The pad-HUD
/// and off-field exclusions live in the caller; this is only the per-mode rule, kept
/// on its own so the selftest mirror can pin it.
pub fn stick_zone_on_field(mode: crate::StickMode, win_w: u32, win_h: u32) -> Option<Rect> {
    match mode {
        crate::StickMode::Float => Some(Rect::new(0, 0, win_w, win_h)),
        crate::StickMode::Dpad => None,
        crate::StickMode::Pinned => {
            Some(crate::placement::rect_of(crate::placement::CtrlId::Stick, Mode::Field, win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h)))
        }
    }
}

pub fn layout(win_w: u32, win_h: u32, joystick: bool, mode: Mode, choice: bool) -> Layout {
    let landscape = win_w > win_h;
    let pad = (win_w.min(win_h) / 110).max(3);
    let mut buttons = Vec::new();
    let game;
    let mut joy_zone = None;

    // Landscape + immersive: the whole window is the game, controls float on top.
    if landscape && crate::immersive_active() {
        let (g, b, jz) = layout_immersive(win_w, win_h, pad, mode, choice);
        // Immersive: the gear is SMALL and sits immediately right of the player-gauge
        // block, centred on it. Player request (2026-07-30): it was big and on the
        // left, where it covered the game's own active-buff list and was itself
        // painted over by the gauges. (The `123` keypad toggle that shared this row
        // was removed 2026-09-18.)
        // ★ FIELD: no gear. The player asked for that strip back (2026-07-30) — the
        // game's own buff band is moved into it instead. The gear stays available on
        // every NON-field screen (title, menus), which is where settings are actually
        // used: orientation applies at once, and immersive / ZOOM apply at the next
        // start anyway.
        // ★ Cutscene joins the field in having NO chrome: a script draws none of our
        // controls at all since 64차 (2026-08-16 dropped OK/arrows, 64차 the skip
        // disc), and a gear there would be the one thing on a screen meant to hold
        // none. Settings stay reachable from every menu screen, including the title,
        // which is where they are actually used.
        let gear = if mode == Mode::Menu {
            gear_rect(win_w, win_h)
        } else {
            Rect::new(0, 0, 0, 0)
        };
        return Layout {
            game: g,
            buttons: b,
            gear,
            joystick: jz,
        };
    }

    // Build the left control region either as a D-pad or a floating joystick.
    let mut build_left = |buttons: &mut Vec<Button>, region: Rect| {
        if joystick {
            joy_zone = Some(build_joystick(buttons, region, pad));
        } else {
            build_dpad(buttons, region, pad);
        }
    };

    if !landscape {
        // Portrait "최대 확대": the game fills the FULL WIDTH (its maximal
        // non-distorted size), controls take the strip below (>= 30% of height).
        // Was capped at 60% height, which left a thin side letterbox on tall
        // phones; filling width removes it.
        let game_area_h = (win_w * CONTENT_H / CONTENT_W).min(win_h * 70 / 100);
        game = contain(Rect::new(0, 0, win_w, game_area_h));
        let strip = Rect::new(0, game_area_h, win_w, win_h - game_area_h);
        let lw = strip.w * 42 / 100;
        build_left(&mut buttons, Rect::new(strip.x, strip.y, lw, strip.h));
        let kx = strip.x + strip.w * 44 / 100;
        build_keypad(&mut buttons, Rect::new(kx, strip.y, strip.x + strip.w - kx, strip.h), pad);
    } else {
        // Landscape "최대 확대": the game fills the FULL HEIGHT (its maximal
        // non-distorted size — a 3:4 game cannot fill a 16:9 screen without
        // distortion), centred, and the controls fill the side margins so they
        // are never left black. Was capped at 60% width + 2*pad tall margin.
        let game_h = win_h;
        let game_w = (game_h * CONTENT_W / CONTENT_H).min(win_w);
        let gx = (win_w - game_w) / 2;
        game = contain(Rect::new(gx, 0, game_w, game_h));
        build_left(&mut buttons, Rect::new(0, 0, gx, win_h));
        build_keypad(&mut buttons, Rect::new(gx + game_w, 0, win_w - (gx + game_w), win_h), pad);
    }

    let gs = (win_w.min(win_h) / 11).clamp(60, 150);
    let gear = Rect::new(win_w.saturating_sub(gs + pad * 3), pad * 3, gs, gs);
    Layout {
        game,
        buttons,
        gear,
        joystick: joy_zone,
    }
}

/// Floating joystick: LSK/RSK/CLR/OK at the four corners of the region, and the
/// whole region is the joystick zone (a touch that misses the corners spawns the
/// stick at that point). Returns the joystick zone rect.
fn build_joystick(out: &mut Vec<Button>, region: Rect, pad: u32) -> Rect {
    let cw = region.w / 3;
    let ch = region.h / 3;
    let corner = |c: u32, r: u32| {
        Rect::new(
            region.x + c * cw + pad,
            region.y + r * ch + pad,
            cw.saturating_sub(2 * pad),
            ch.saturating_sub(2 * pad),
        )
    };
    out.push(Button {
        rect: corner(0, 0),
        key: KeyCode::LEFT_SOFT_KEY,
        label: Label::Text("LSK"),
        dpad: false,
        face: Face::Solid,
        wedge: None,
    });
    out.push(Button {
        rect: corner(2, 0),
        key: KeyCode::RIGHT_SOFT_KEY,
        label: Label::Text("RSK"),
        dpad: false,
        face: Face::Solid,
        wedge: None,
    });
    out.push(Button {
        rect: corner(0, 2),
        key: KeyCode::CLEAR,
        label: Label::Text("CLR"),
        dpad: false,
        face: Face::Solid,
        wedge: None,
    });
    out.push(Button {
        rect: corner(2, 2),
        key: KeyCode::OK,
        label: Label::Text("OK"),
        dpad: false,
        face: Face::Solid,
        wedge: None,
    });
    region
}

/// Max drag (px) for full joystick deflection.
///
/// ★ Takes the WINDOW size, not just the zone rect (2026-08-16). It used to be
/// `joy_radius(zone)` and passed `zone.w / zone.h` into [`joy_home`] as if they were
/// the window's — which was only ever correct because the immersive zone happened to
/// BE the whole window. A resizable stick makes `joy_home` resolve a placement
/// against whatever it is handed, so the old form would have silently mis-scaled
/// both the deflection radius and the stick's own seat. (The first TOUCH TYPE B —
/// the pinned stick, since replaced by the field D-pad — also shrank the zone to the
/// stick's disc, which is what surfaced the bug.)
pub fn joy_radius(zone: &Rect, win_w: u32, win_h: u32) -> f64 {
    if crate::immersive_active() {
        // ★ 91차-B: the thumb's PIN radius — as far as the thumb's centre may go from
        // the base's centre, which is the base radius less the thumb's own, so the
        // thumb disc stays inside the base. Deriving this from the (window-sized) zone
        // gave ~450 px once; the base is `stick_base_radius` of the placed seat now.
        return joy_home(win_w, win_h).2 * (1.0 - JOY_THUMB);
    }
    (zone.w.min(zone.h) as f64) * 0.42
}

/// The deadzone basis `joy_dir` is given — how far a thumb travels before walking
/// starts (30 %) and how close it may come back before it stops (15 %).
///
/// ★ 91차-B split this from [`joy_radius`]: the pin radius follows the base disc,
/// which is now the whole arrows' square, while the walk-start distance stays what
/// the player has been walking with since 2026-07-30 ([`stick_engage_radius`]). The
/// classic (non-immersive) layouts keep the two equal, as they always were.
pub fn joy_engage_radius(zone: &Rect, win_w: u32, win_h: u32) -> f64 {
    if crate::immersive_active() {
        let seat = crate::placement::rect_of(crate::placement::CtrlId::Stick, Mode::Field, win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h));
        return stick_engage_radius(&seat, win_w, win_h);
    }
    joy_radius(zone, win_w, win_h)
}

/// The ONE direction key a joystick deflection `(dx, dy)` maps to, where `(dx, dy)`
/// is measured from the FIXED centre the stick spawned at.
///
/// ★ The centre is fixed as of 2026-08-03. It used to TRAIL the finger (a
/// `joy_follow` that kept the base `radius` behind once the drag exceeded `radius`),
/// which was itself a fix for "walking up, then sliding left without lifting, keeps
/// walking up". The player tested that on device and reported the cure as worse than
/// the disease: the base visibly ran across the screen under the thumb and steering
/// became unpredictable ("조이스틱 자체가 따라와서 이동이 좀 불편해"). Now: touch down
/// = centre, lift and touch again = new centre, and nothing moves in between.
///
/// The trade-off is real and worth stating. With a fixed centre the direction is the
/// angle of the WHOLE vector from the spawn point, so a thumb that has wandered far
/// past the ring needs a proportionally large sideways move to turn. In normal play
/// it does not arise — the thumb disc pins at the ring edge, which is the feedback
/// that says "you are already at full tilt" — and how far the vector can grow is
/// bounded by how far the player chooses to drag.
///
/// The mapping itself is 4-way, dominant axis, with hysteresis.
///
/// Why one key and not the old 8-way two-key set: the game walks in ONE latched
/// direction, so holding UP+LEFT through a diagonal slide does not walk diagonally
/// — it keeps walking UP and drops LEFT on the floor, and the later keyup(UP)
/// leaves the guest with no fresh keydown to latch. Emitting exactly one direction
/// means every turn is a clean keyup(old) + keydown(new), which the guest acts on.
///
/// `active` = the direction currently held; it is KEPT unless the challenging axis
/// beats it by [`TURN_RATIO`], so a thumb resting near a diagonal cannot flutter
/// between two directions.
pub fn joy_dir(dx: f64, dy: f64, radius: f64, active: Option<KeyCode>) -> Option<KeyCode> {
    /// Deflection needed (as a fraction of `radius`) to start / to keep walking.
    const DEAD_ENGAGE: f64 = 0.30;
    const DEAD_KEEP: f64 = 0.15;
    /// How far the challenging axis must beat the held one to take the turn.
    const TURN_RATIO: f64 = 1.2;

    let dist = (dx * dx + dy * dy).sqrt();
    let dead = radius * if active.is_some() { DEAD_KEEP } else { DEAD_ENGAGE };
    if dist <= dead || dist == 0.0 {
        return None;
    }
    // Signed deflection along each key's own direction.
    let comp = |k: KeyCode| match k {
        KeyCode::UP => -dy,
        KeyCode::DOWN => dy,
        KeyCode::LEFT => -dx,
        _ => dx,
    };
    let cand = if dx.abs() >= dy.abs() {
        if dx >= 0.0 { KeyCode::RIGHT } else { KeyCode::LEFT }
    } else if dy >= 0.0 {
        KeyCode::DOWN
    } else {
        KeyCode::UP
    };
    if let Some(cur) = active
        && cur != cand
        && comp(cur) > 0.0
        && comp(cand) < comp(cur) * TURN_RATIO
    {
        return Some(cur); // not a decisive turn — keep walking the same way
    }
    Some(cand)
}

fn build_dpad(out: &mut Vec<Button>, region: Rect, pad: u32) {
    let cw = region.w / 3;
    let ch = region.h / 3;
    let cell = |c: u32, r: u32| -> Rect {
        Rect::new(
            region.x + c * cw + pad,
            region.y + r * ch + pad,
            cw.saturating_sub(2 * pad),
            ch.saturating_sub(2 * pad),
        )
    };
    let mut push = |c, r, key, label, dpad| {
        out.push(Button {
            rect: cell(c, r),
            key,
            label,
            dpad,
            face: Face::Solid,
            wedge: None,
        })
    };
    push(0, 0, KeyCode::LEFT_SOFT_KEY, Label::Text("LSK"), false);
    push(1, 0, KeyCode::UP, Label::Arrow(Dir::Up), true);
    push(2, 0, KeyCode::RIGHT_SOFT_KEY, Label::Text("RSK"), false);
    push(0, 1, KeyCode::LEFT, Label::Arrow(Dir::Left), true);
    push(1, 1, KeyCode::OK, Label::Text("OK"), true);
    push(2, 1, KeyCode::RIGHT, Label::Arrow(Dir::Right), true);
    push(0, 2, KeyCode::CLEAR, Label::Text("CLR"), false);
    push(1, 2, KeyCode::DOWN, Label::Arrow(Dir::Down), true);
    push(2, 2, KeyCode::OK, Label::Text("OK"), true);
}

fn build_keypad(out: &mut Vec<Button>, region: Rect, pad: u32) {
    let cw = region.w / 3;
    let ch = region.h / 4;
    let keys: [[(&'static str, KeyCode); 3]; 4] = [
        [("1", KeyCode::NUM1), ("2", KeyCode::NUM2), ("3", KeyCode::NUM3)],
        [("4", KeyCode::NUM4), ("5", KeyCode::NUM5), ("6", KeyCode::NUM6)],
        [("7", KeyCode::NUM7), ("8", KeyCode::NUM8), ("9", KeyCode::NUM9)],
        [("*", KeyCode::STAR), ("0", KeyCode::NUM0), ("#", KeyCode::HASH)],
    ];
    for (r, row) in keys.iter().enumerate() {
        for (c, (txt, key)) in row.iter().enumerate() {
            let rect = Rect::new(
                region.x + c as u32 * cw + pad,
                region.y + r as u32 * ch + pad,
                cw.saturating_sub(2 * pad),
                ch.saturating_sub(2 * pad),
            );
            out.push(Button {
                rect,
                key: *key,
                label: Label::Text(txt),
                dpad: false,
                face: Face::Solid,
                wedge: None,
            });
        }
    }
}

pub fn hit(buttons: &[Button], x: f64, y: f64) -> Option<KeyCode> {
    buttons.iter().find(|b| b.contains(x, y)).map(|b| b.key)
}

/// Quickbar action-button rects: a top-to-bottom column of round slots along the
/// RIGHT edge of the game rect (H5-style), starting below the top-right gear.
/// One rect per quickbar slot passed.
pub fn quick_buttons(game: Rect, gear: Rect, n: usize, mode: Mode) -> Vec<Rect> {
    let mut out = Vec::new();
    if n == 0 || game.w == 0 || game.h == 0 || mode == Mode::Cutscene {
        return out;
    }
    if crate::immersive_active() && game.w > game.h {
        // Immersive (reference-style HUD): the slots are SKILL BUTTONS on an arc
        // up-and-left of the big action disc, not a keypad column — and each seat is
        // individually placeable, so they come from the resolver like every other
        // control instead of from the ring formula directly.
        //
        // Resolved for the CURRENT mode, not always for the field: since 2026-08-03
        // the plates stay on screen with a game window open, and the editor keeps a
        // separate arrangement per mode, so asking the field resolver here would draw
        // them where the player put their FIELD plates while hit-testing somewhere
        // else.
        let placed = crate::placement::resolve(mode, game.w, game.h);
        let hidden = crate::quick_hidden();
        return (0..n.min(crate::quickpage::PLATES) as u8)
            .map(|i| {
                // ★★ A HIDDEN plate is a ZERO-SIZED rect AT ITS OWN INDEX, never a
                // dropped entry (player request 2026-09-10). `Rect::contains` is
                // half-open so it can never be hit, `draw_pick_highlight` and
                // `padhud::badge_at` already skip zero rects, and the tap falls through
                // to the arm below the plates — the joystick on the field, the game's
                // own panel in a menu.
                //
                // ⛔ Do NOT filter these out. `app.rs` addresses `self.quickbar`, the
                // cooldown vector, `quick_press` and the pad badges BY THIS INDEX
                // (`quickbar[i].slot == i`), so a shorter vector makes every later plate
                // fire, draw and badge as its neighbour.
                if crate::quickpage::plate_hidden(hidden, i) {
                    return Rect::new(0, 0, 0, 0);
                }
                placed
                    .iter()
                    .find(|(c, _)| *c == crate::placement::CtrlId::Quick(i))
                    .map(|(_, r)| *r)
                    .unwrap_or_else(|| Rect::new(0, 0, 0, 0))
            })
            .collect();
    }
    if mode != Mode::Field {
        // Non-immersive layouts already put a full keypad on screen; the plates are a
        // field-HUD extra there, not a second way to press the same keys.
        return out;
    }
    let s = (game.w / 6).clamp(28, 84).min(game.h / 6);
    let gap = (s / 6).max(3);
    let x = game.x + game.w.saturating_sub(s + gap);
    let mut y = (gear.y + gear.h + gap).max(game.y + gap);
    let hidden = crate::quick_hidden();
    for i in 0..n {
        // Same rule as the immersive path: a hidden plate keeps its INDEX and gets no
        // rect, and it does not advance `y` — so the column closes up instead of
        // leaving a hole, and the indices still line up with `self.quickbar`.
        if crate::quickpage::plate_hidden(hidden, i as u8) {
            out.push(Rect::new(0, 0, 0, 0));
            continue;
        }
        if y + s > game.y + game.h {
            break;
        }
        out.push(Rect::new(x, y, s, s));
        y += s + gap;
    }
    out
}

/// The key-'7' (secondary weapon) button's face: the EQUIPPED secondary weapon's
/// icon, or a dimmed "7" when nothing is equipped — which is exactly when the game
/// itself refuses the key, so the button reads as unavailable rather than broken.
pub fn draw_secondary(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, icon: Option<IconRef>) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let at = atlas::atlas();
    let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
    let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
    let rad = f64::from(r.w.min(r.h)) / 2.0;
    let ic = icon.and_then(|i| match i {
        IconRef::Item(f) => at.item(f),
        IconRef::Skill(sheet, cell) => at.skill(sheet, cell),
    });
    match ic {
        Some(ic) => blit_icon(buf, win_w, win_h, cx, cy, rad * 0.9, ic),
        None => {
            blend_disc_soft(buf, win_w, win_h, cx, cy, rad, 0xff0a_0c12, 90, (rad * 0.09).max(1.2));
            draw_centered_text(buf, win_w, win_h, label_box(r), "7", LABEL);
        }
    }
}

/// Live cash-buff status lines, drawn under the unspent-point badges while a game
/// window (menu) is open — player request 2026-08-05: "내가 어떤 캐시템이 현재
/// 적용중인지 he mp바 하단에 글자로 보여줄거야 ... 스탯 및 스킬 + 아래로".
///
/// ★ 2026-08-06 — KOREAN item names, at a smaller size (player: "초월의 문장, 경험의
/// 보주 등등 현재 버프 적용되고있는거 한글로 표시했으면 좋겠어. 그리고 글자크기도 살짝
/// 줄여줘"). Until now this was ASCII-only because `draw_glyph` is a 5x7 ASCII font;
/// [`crate::hangul`] supplies 12x12 bitmaps for the CLOSED set of syllables these
/// lines can contain, which is what makes shipping Korean here possible without a
/// font. The strings are composed by `crate::buffs::status_lines` on the worker (it
/// owns the clock).
///
/// The scale drops from `game.h / 200` to `game.h / 300` — on a 1080-tall window that
/// is the ASCII run going 35 px -> 21 px tall. The Hangul cell stays FULL-WIDTH at the
/// same scale (12s against the Latin 7s), which is not an inconsistency: a Hangul
/// syllable fills its em where a Latin capital is about 70% of it, so equal scales are
/// what makes the two look the same size.
pub fn draw_buff_lines(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, lines: &[String]) {
    if game.w == 0 || game.h == 0 || lines.is_empty() {
        return;
    }
    let b = stats_block(game);
    let s = (game.h / 300).clamp(2, 4);
    let x = b.bars.x;
    let line_h = crate::hangul::HANGUL_H * s;
    // Under the whole block (ring, level plate and badges all reach its bottom row).
    let bb = stats_block_box(game);
    let mut y = bb.y + bb.h + (2 * s).max(4);
    for line in lines {
        let tw = mixed_text_w(line, s);
        // Dim pad behind the glyphs so they stay readable over the game's window art.
        blend_rect(
            buf,
            win_w,
            win_h,
            x.saturating_sub(s),
            y.saturating_sub(s),
            tw + 2 * s,
            line_h + 2 * s,
            0xff05_070c,
            140,
        );
        draw_mixed_text(buf, win_w, win_h, x, y, s, line, LABEL);
        y += line_h + (2 * s).max(4);
    }
}

/// How long a side note stays at full strength, and how long it then takes to fade out.
pub const SIDE_NOTE_HOLD_MS: u128 = 2200;
pub const SIDE_NOTE_FADE_MS: u128 = 500;

/// A side note's strength `elapsed_ms` after it was raised: 255 while it holds, falling
/// to 1 over the fade, `None` once it is gone (the caller drops it).
pub fn side_note_alpha(elapsed_ms: u128) -> Option<u8> {
    let end = SIDE_NOTE_HOLD_MS + SIDE_NOTE_FADE_MS;
    if elapsed_ms < SIDE_NOTE_HOLD_MS {
        Some(255)
    } else if elapsed_ms < end {
        Some(((end - elapsed_ms) * 255 / SIDE_NOTE_FADE_MS).clamp(1, 255) as u8)
    } else {
        None
    }
}

/// The NON-MODAL notes (`UserEvent::SideNote` — the SAVE button's answers and 「하늘성
/// 입장」): user 2026-09-30, *"팝업형태가 아니라 좌측(HP,SP바 아래쪽 화면 좌측끝
/// 중앙부분)에 … 안내문구로만"*. At the LEFT EDGE — the HP/SP block's own left margin —
/// and centred on the game rect's vertical middle, oldest on top. Each is the game's font
/// in white over a dark pad with the top tabs' amber bar down its left side, at `a`
/// (255 = full); nothing here takes input, so the player keeps moving while it reads.
pub fn draw_side_notes(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, notes: &[(&str, u8)]) {
    if game.w == 0 || game.h == 0 || notes.is_empty() {
        return;
    }
    let s = (game.h / 330).clamp(2, 4);
    let inner = (2 * s).max(4);
    let accent = s.max(2);
    let row_h = crate::gamefont::KOR_H as u32 * s + inner * 2;
    let gap = inner;
    let n = notes.len() as u32;
    let total = n * row_h + (n - 1) * gap;
    let x = stats_block_box(game).x;
    let mut y = (game.y + game.h / 2).saturating_sub(total / 2);
    for &(text, a) in notes {
        let a = u32::from(a);
        let tw = game_text_w(text, s);
        blend_rect(buf, win_w, win_h, x, y, accent + tw + inner * 2, row_h, TOP_TAB_FILL, TOP_TAB_FILL_A * a / 255);
        blend_rect(buf, win_w, win_h, x, y, accent, row_h, TOP_TAB_RIM, TOP_TAB_RIM_A * a / 255);
        let (tx, ty) = (x + accent + inner, y + inner);
        let d = (s / 2).max(1);
        draw_game_text_blend(buf, win_w, win_h, tx + d, ty + d, s, text, 0xff08_0a10, a);
        draw_game_text_blend(buf, win_w, win_h, tx, ty, s, text, LABEL, a);
        y += row_h + gap;
    }
}

/// ★ 145차 · 147차: the tower banner's inks — one row per [`crate::tower_hud::Ink`] group,
/// each `[dark, mid, light, outline]` (the digit table's shades 1..=4). The outline is the
/// damage digits' own near-white (`#fffaff`, kept as the game draws it) in every group,
/// so all three still read as the game's damage font; only the fill changes.
/// 147차 (user 2026-10-01: *"현재 층 색과, 남은 몬스터에서 숫자부분만 색 다르게 … 3개의
/// 색상으로 … 어울리는 색상으로"*) — the label keeps 145차's GOLD, the two things the user
/// named change: the floor is SKY BLUE (하늘성 — and gold's complement) and the count
/// CORAL RED (the enemies; a crimson → red → salmon ramp, kept off the game's orange
/// damage digits `#cd1000/#ff6900/#ffaa31` so it never reads as a hit). Blue · gold · red
/// is one primary triad, and each ramp keeps dark < mid < light as the digits' own do.
/// Chosen on a render of the real banner over a tower frame at the S9's ×4.5 against five
/// other triads (sky/gold/mint · aqua/gold/rose · violet/gold/coral · gold/sky/coral ·
/// sky/gold/red-orange).
const TOWER_HUD_INK: [[u32; 4]; crate::tower_hud::INK_GROUPS] = [
    // Ink::Floor — sky blue
    [0xff1a_55b0, 0xff3d_97f0, 0xffa6_dbff, 0xffff_faff],
    // Ink::Label — gold (145차's, unchanged)
    [0xffa8_6a00, 0xfff0_b000, 0xffff_e860, 0xffff_faff],
    // Ink::Count — coral red
    [0xffb0_152a, 0xfff0_4248, 0xffff_9f8c, 0xffff_faff],
];

/// ★ 145차 · 147차 — the 하늘성 BANNER: 「N층」 (+ 「보스」) over 「남은 몬스터 M」 (or 진입 ·
/// 클리어 · 완주 · 이탈), in the game's damage digits, centred on the game rect with its top
/// [`crate::tower_hud::TOP_FRACTION`] of the way down (145차 user 2026-09-30: *"중앙
/// 상단(너무 위쪽에 붙이진말고 적당히 10%정도 떨어진 위치정도)에 데미지 폰트에 색상만
/// 다르게"*; 147차 2026-10-01: *"폰트 조금 줄여주고 … 위치 조금만 위로"* — 10 % → 7 %).
/// Composed in the game's logical pixels by [`crate::tower_hud::banner`] and drawn at
/// [`crate::tower_hud::pixel_scale`] — the game→window scale (`game.h / logical_h`) times
/// [`crate::tower_hud::SCALE`] (0.8: until 147차 its digits were exactly the size of the
/// damage numbers the game pops over a monster, and that read as too big). Each pixel
/// takes its group's ink ([`TOWER_HUD_INK`]). Pixel edges are rounded one by one, as the
/// game's own fractional upscale is (see [`draw_game_text_frac`]). Transparent between the
/// glyphs — no pad: the world stays visible around the words, like the damage numbers.
pub fn draw_tower_hud(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, logical_h: u32, hud: &crate::guardian::TowerHud) {
    if game.w == 0 || game.h == 0 || logical_h == 0 {
        return;
    }
    use crate::guardian::TowerPhase;
    use crate::tower_hud::Status;
    let status = match hud.phase {
        TowerPhase::Fighting { left } => Status::Left(left),
        TowerPhase::Entering => Status::Word("진입"),
        TowerPhase::Cleared => Status::Word("클리어"),
        TowerPhase::Completed => Status::Word("완주"),
        TowerPhase::Left => Status::Word("이탈"),
    };
    let glyphs = |s: &str| crate::gamefont::layout(s).into_iter().map(|c| (c.rows, c.advance)).collect::<Vec<_>>();
    let b = crate::tower_hud::banner(hud.floor, hud.boss, status, &glyphs);
    let s = crate::tower_hud::pixel_scale(game.h, logical_h);
    let (ox, oy) = crate::tower_hud::place(game.x, game.y, game.w, game.h, s, b.w);
    for y in 0..b.h {
        let y0 = (oy + y as f64 * s).round().max(0.0) as u32;
        let y1 = (oy + (y + 1) as f64 * s).round().max(0.0) as u32;
        for x in 0..b.w {
            let v = b.get(x, y);
            if v == 0 {
                continue;
            }
            let group = crate::tower_hud::group_of(v).min(crate::tower_hud::INK_GROUPS - 1);
            let shade = usize::from(crate::tower_hud::shade_of(v)) - 1;
            let x0 = (ox + x as f64 * s).round().max(0.0) as u32;
            let x1 = (ox + (x + 1) as f64 * s).round().max(0.0) as u32;
            fill(buf, win_w, win_h, x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0), TOWER_HUD_INK[group][shade]);
        }
    }
}

/// Rendered width of a mixed Hangul/ASCII string at `scale`.
fn mixed_text_w(text: &str, scale: u32) -> u32 {
    text.chars()
        .map(|ch| {
            if ch.is_ascii() {
                6 * scale
            } else {
                (crate::hangul::HANGUL_W + 1) * scale
            }
        })
        .sum()
}

/// Draw a string that mixes Hangul (12x12, [`crate::hangul`]) and ASCII (5x7,
/// [`draw_glyph`]).
///
/// `y` is the TOP of the Hangul cell; the shorter ASCII glyphs are dropped to sit on
/// the same baseline instead of hanging from the same top, because a line reading
/// "경험의보주 6일 23시간" with the digits floating above the syllables is worse than
/// no Korean at all. A character with no glyph in either font falls through to
/// `draw_glyph`, which has its own unknown-character box.
fn draw_mixed_text(buf: &mut [u32], win_w: u32, win_h: u32, mut x: u32, y: u32, scale: u32, text: &str, color: u32) {
    let baseline_drop = (crate::hangul::HANGUL_H - 7) * scale;
    for ch in text.chars() {
        if ch.is_ascii() {
            draw_glyph(buf, win_w, win_h, x, y + baseline_drop, scale, ch, color);
            x += 6 * scale;
        } else {
            draw_hangul(buf, win_w, win_h, x, y, scale, ch, color);
            x += (crate::hangul::HANGUL_W + 1) * scale;
        }
    }
}

fn draw_hangul(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, scale: u32, ch: char, color: u32) {
    let Some(rows) = crate::hangul::glyph(ch) else {
        return;
    };
    let w = crate::hangul::HANGUL_W;
    for (r, bits) in rows.iter().enumerate() {
        for c in 0..w {
            if bits & (1 << (w - 1 - c)) != 0 {
                fill(buf, win_w, win_h, x + c * scale, y + r as u32 * scale, scale, scale, color);
            }
        }
    }
}


/// Draw a round control whose face is a UI icon COVERAGE MASK, tinted `color`.
///
/// Separate from `blit_icon`: that samples a decoded game icon, which carries its own
/// colour and a 1-bit silhouette. These masks carry only shape (see `ui_icons.rs`), so
/// the colour is the caller's and comes from the same `LABEL` palette as the hand-drawn
/// glyphs — which is the whole reason the packer throws the source RGB away.
///
/// Bilinear on the coverage, for the same reason `blit_icon` is bilinear: a 96px mask
/// scaled to a ~90px disc at a non-integer factor looks blocky under nearest.
///
/// The colour is NOT a parameter: it is `LABEL`, the same foreground every other control
/// uses, and the palette belongs to this module. A caller that could pick the colour is a
/// caller that can make this button look like it belongs to a different HUD.
///
/// `icon_span` is the icon's span across the disc, as a multiple of the disc RADIUS
/// (1.5 = the shared default, i.e. 75% of the button).
///
/// Split out so one control can carry a smaller glyph without every control changing —
/// the disc, its rim and its pressed state all stay identical, which is what keeps it
/// reading as one of the set. See [`QUICKPAGE_ICON_SPAN`].
pub fn draw_mask_button_spanned(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, icon: ui_icons::Icon, pressed: bool, icon_span: f64) {
    let color = LABEL;
    if r.w == 0 || r.h == 0 {
        return;
    }
    let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
    let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
    let rad = f64::from(r.w.min(r.h)) / 2.0;
    // Same face as every other round control, so this reads as one of them.
    blend_disc_soft(
        buf,
        win_w,
        win_h,
        cx,
        cy,
        rad,
        0xff0a_0c12,
        if pressed { 150 } else { 90 },
        (rad * 0.09).max(1.2),
    );
    let Some(m) = ui_icons::icons().get(icon) else {
        return; // Face stays a bare disc: a missing asset must not blank the control.
    };
    // Inset so the disc's rim stays visible; 1.5 matches `blit_icon`'s box_side ratio.
    let side = rad * icon_span;
    if side < 1.0 || m.w == 0 || m.h == 0 {
        return;
    }
    let dw = side.round().max(1.0) as u32;
    let (mw, mh) = (f64::from(m.w), f64::from(m.h));
    let ox = (cx - side / 2.0).round().max(0.0) as u32;
    let oy = (cy - side / 2.0).round().max(0.0) as u32;
    for py in 0..dw {
        for px in 0..dw {
            let (x, y) = (ox + px, oy + py);
            if x >= win_w || y >= win_h {
                continue;
            }
            let sx = (f64::from(px) + 0.5) / f64::from(dw) * mw - 0.5;
            let sy = (f64::from(py) + 0.5) / f64::from(dw) * mh - 0.5;
            let a = sample_coverage(m, sx, sy);
            if a > 0 {
                blend_px(buf, (y * win_w + x) as usize, color, u32::from(a));
            }
        }
    }
}

/// Bilinear coverage from a mask at fractional `(sx, sy)`, clamped at the edges.
fn sample_coverage(m: &ui_icons::Mask, sx: f64, sy: f64) -> u8 {
    let x0 = sx.floor();
    let y0 = sy.floor();
    let fx = sx - x0;
    let fy = sy - y0;
    let cl = |v: f64, hi: u32| -> u32 { v.max(0.0).min(f64::from(hi.saturating_sub(1))) as u32 };
    let (xi, yi) = (cl(x0, m.w), cl(y0, m.h));
    let (xj, yj) = (cl(x0 + 1.0, m.w), cl(y0 + 1.0, m.h));
    let p = |x: u32, y: u32| f64::from(m.coverage(x, y));
    let top = p(xi, yi) * (1.0 - fx) + p(xj, yi) * fx;
    let bot = p(xi, yj) * (1.0 - fx) + p(xj, yj) * fx;
    (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
}

/// Index of the quickbar button under `(x, y)`, if any.
pub fn quick_hit(rects: &[Rect], x: f64, y: f64) -> Option<usize> {
    rects.iter().position(|r| r.contains(x, y))
}

// Skill cooldowns (2026-08-05, reworked 2026-08-07 / 73차) are host-owned.
// Worn-gear percent (`addStat(0x11)` at CHAR+0x124) is folded into `dur` at
// insert, so this fill and the tap-block share the reduced length.
// Combat is gated by SP + cast delay only. A plate fill exists ONLY for a skill
// with a positive length in the builtin table / `wie_cooldowns.txt` (missing or
// 0 = nothing drawn). It starts once the WORKER confirms the cast
// (`UserEvent::SkillCast`). A tap on a still-running plate is swallowed.
// Draw: dark veil on the unrecovered part, pressed-blue fill growing from the
// BOTTOM (`frac` 0→1) — "아래서부터 색감이 차오르는 방식".

/// The rising cooldown fill: a dark veil over the not-yet-recovered part of the
/// disc, and a coloured fill that grows from the BOTTOM as `frac` (elapsed /
/// total, 0..1) approaches 1 — "아래서부터 색감이 차오르는 방식".
fn draw_cooldown_overlay(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, rad: f64, frac: f64) {
    const VEIL: u32 = 0xff05_070c; // not recovered yet: darkened
    const FILL: u32 = 0xff4a_6cff; // recovered from the bottom up: the pressed blue
    let frac = frac.clamp(0.0, 1.0);
    if rad < 1.0 || frac >= 1.0 {
        return;
    }
    let fill_top = cy + rad - 2.0 * rad * frac;
    let x0 = (cx - rad).max(0.0) as u32;
    let x1 = ((cx + rad).max(0.0) as u32 + 1).min(win_w);
    let y0 = (cy - rad).max(0.0) as u32;
    let y1 = ((cy + rad).max(0.0) as u32 + 1).min(win_h);
    for y in y0..y1 {
        let dy = f64::from(y) + 0.5 - cy;
        let row = (y * win_w) as usize;
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - cx;
            if dx * dx + dy * dy > rad * rad {
                continue;
            }
            if f64::from(y) + 0.5 >= fill_top {
                blend_px(buf, row + x as usize, FILL, 96);
            } else {
                blend_px(buf, row + x as usize, VEIL, 120);
            }
        }
    }
}

/// Draw the quickbar as ROUND action buttons (H5-style): filled disc + rim,
/// coloured by kind. Occupied slots show the DECODED skill/potion icon (from the
/// embedded atlas, number-free) + for items the live stack count; empty slots
/// show the hotkey char as a placeholder.
///
/// `cooldowns[i]` = the slot's cooldown progress (elapsed fraction 0..1); `>= 1`
/// or missing = ready, nothing drawn. Index-aligned with `rects`.
pub fn draw_quickbar(buf: &mut [u32], win_w: u32, win_h: u32, rects: &[Rect], slots: &[QuickSlot], pressed: Option<usize>, cooldowns: &[f64]) {
    if crate::immersive_active() {
        draw_skill_buttons(buf, win_w, win_h, rects, slots, pressed, cooldowns);
        return;
    }
    let at = atlas::atlas();
    for (i, r) in rects.iter().enumerate() {
        let Some(slot) = slots.get(i) else {
            break;
        };
        // ⛔ A ZERO-SIZED rect is a plate the player has switched off (see
        // `quick_buttons`), and it must draw NOTHING. Every disc/icon helper below
        // already bails on a tiny radius, but the item STACK COUNT block does not —
        // it picks its own scale from `(r.h / 20).max(2)` and would print the digits
        // at the rect's corner, i.e. in the top-left of the screen.
        if r.w == 0 || r.h == 0 {
            continue;
        }
        let base = if pressed == Some(i) {
            QB_PRESSED
        } else {
            match slot.kind {
                SlotKind::Skill => QB_SKILL,
                SlotKind::Item => QB_ITEM,
                SlotKind::Empty => QB_EMPTY,
            }
        };
        let cx = r.x as f64 + r.w as f64 / 2.0;
        let cy = r.y as f64 + r.h as f64 / 2.0;
        let rad = (r.w.min(r.h) as f64) / 2.0;
        fill_disc(buf, win_w, win_h, cx, cy, rad, base);
        blend_ring_soft(buf, win_w, win_h, cx, cy, rad - 1.0, 2.0, BORDER, 255);

        let icon = slot.icon.and_then(|ic| match ic {
            IconRef::Item(f) => at.item(f),
            IconRef::Skill(sheet, cell) => at.skill(sheet, cell),
        });
        if let Some(ic) = icon {
            blit_icon(buf, win_w, win_h, cx, cy, rad, ic);
            if matches!(slot.kind, SlotKind::Item) {
                // Our own clean stack count (bottom-right), since the atlas icon has none.
                let cnt = format!("{}", slot.count.min(99));
                let bot = Rect::new(r.x, r.y + r.h * 3 / 5, r.w, r.h * 2 / 5);
                draw_centered_text(buf, win_w, win_h, bot, &cnt, LABEL);
            }
        }
        // No icon = an empty slot, and it stays blank. Same reasoning as the immersive
        // plates above: the hotkey character is the mechanism, not the label.
        if let Some(f) = cooldowns.get(i).copied().filter(|f| *f < 1.0) {
            draw_cooldown_overlay(buf, win_w, win_h, cx, cy, rad, f);
        }
    }
}

/// Top-right minimap panel: the game's own offscreen minimap buffer, nearest-
/// neighbour enlarged to fill the panel. Until the player opens the map once the
/// game has not painted that buffer, so the panel shows a `MAP` hint instead
/// (tapping it sends `*`, which is what triggers the paint).
#[allow(clippy::too_many_arguments)]
pub fn draw_minimap(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, mm: Option<&crate::Minimap>, marker: Option<(f32, f32)>, mobs: &[(f32, f32)]) {
    if !crate::immersive_active() || game.w == 0 {
        return;
    }
    draw_minimap_in(buf, win_w, win_h, minimap_panel(win_w, win_h), mm, true, marker, mobs);
}

// `map_overlay_panel` / `draw_map_overlay` were REMOVED 2026-08-04 (player request:
// "가운데 확대해서 표시하는 기능은 제거하고"). They painted a big centred copy of the
// mirrored minimap over the game's own map window — the 6차/7차 answer to "the guest
// draws its map at 1:1, which is tiny on a wide framebuffer".
//
// The show/hide toggle replaced the need for it: the minimap is now a panel you travel
// with, so there is nothing to blow up. Deleted rather than left dead — the panel
// geometry and the 82% figure are recoverable from git history, and carrying an unused
// full-screen draw path invites someone to call it again.

// hero2's hero5-style view (fixed zoom, follows the player, exits as blue bars, the
// monsters as red squares) — its own dependency-free file, shared with hero2 verbatim
// (141차, 2026-09-30).
include!("minimap_view.rs");

// The view's tile classes are the worker's (`minimap_render.rs`), repeated because
// neither file may name the other (both are `include!`d into dependency-free crates).
// A drift would draw floor as wall or exits as nothing — silently — so it is a build
// error instead.
const _: () = assert!(MM_WALL == crate::items::MM_TILE_WALL && MM_FLOOR == crate::items::MM_TILE_FLOOR && MM_EXIT == crate::items::MM_TILE_EXIT);

/// Shared painter: the view (hero5's way: no panel, no frame — the walkable floor
/// darkened and outlined, exits as blue bars, the player teal, the monsters red).
/// `hint` = draw a "MAP" placeholder when there is no map yet.
#[allow(clippy::too_many_arguments)]
fn draw_minimap_in(buf: &mut [u32], win_w: u32, win_h: u32, p: Rect, mm: Option<&crate::Minimap>, hint: bool, marker: Option<(f32, f32)>, mobs: &[(f32, f32)]) {
    if p.w == 0 || p.h == 0 {
        return;
    }
    if let Some(crate::Minimap { w, h, cells }) = mm
        && mm_draw_panel(buf, win_w, win_h, (p.x, p.y, p.w, p.h), u32::from(*w), u32::from(*h), cells, marker, mobs)
    {
        return;
    }
    // No map (yet): the old translucent panel and its hint.
    let t = (p.w / 40).max(2);
    blend_rect(buf, win_w, win_h, p.x, p.y, p.w, p.h, MM_BACKDROP, 75);
    blend_rect(buf, win_w, win_h, p.x, p.y, p.w, t, MM_FRAME, 140);
    blend_rect(buf, win_w, win_h, p.x, p.y + p.h.saturating_sub(t), p.w, t, MM_FRAME, 140);
    blend_rect(buf, win_w, win_h, p.x, p.y, t, p.h, MM_FRAME, 140);
    blend_rect(buf, win_w, win_h, p.x + p.w.saturating_sub(t), p.y, t, p.h, MM_FRAME, 140);
    if hint {
        draw_centered_text(buf, win_w, win_h, p, "MAP", LABEL);
    }
}

/// Reference-style skill buttons — ROUND (player request 2026-07-30: "make the
/// 1/3/7/9/0/# keys all round"). A translucent disc with a bright anti-aliased rim
/// (gold for a skill, steel for an item, dim when empty), the decoded icon feathered
/// into the disc, and the live stack count for items. Empty slots stay dim with just
/// the hotkey character so the layout never jumps.
fn draw_skill_buttons(buf: &mut [u32], win_w: u32, win_h: u32, rects: &[Rect], slots: &[QuickSlot], pressed: Option<usize>, cooldowns: &[f64]) {
    const PLATE: u32 = 0xff10_1420;
    const RIM_SKILL: u32 = 0xffd8_aa38;
    const RIM_ITEM: u32 = 0xff9d_b4d0;
    const RIM_EMPTY: u32 = 0xff4a_5568;
    let at = atlas::atlas();
    for (i, r) in rects.iter().enumerate() {
        let Some(slot) = slots.get(i) else {
            break;
        };
        // ⛔ A ZERO-SIZED rect is a plate the player has switched off (see
        // `quick_buttons`), and it must draw NOTHING. Every disc/icon helper below
        // already bails on a tiny radius, but the item STACK COUNT block does not —
        // it picks its own scale from `(r.h / 20).max(2)` and would print the digits
        // at the rect's corner, i.e. in the top-left of the screen.
        if r.w == 0 || r.h == 0 {
            continue;
        }
        let down = pressed == Some(i);
        let empty = matches!(slot.kind, SlotKind::Empty);
        let rim = match slot.kind {
            SlotKind::Skill => RIM_SKILL,
            SlotKind::Item => RIM_ITEM,
            SlotKind::Empty => RIM_EMPTY,
        };
        let plate = if down { BTN_PRESSED } else { PLATE };
        // User feedback: more see-through. Empty slots almost vanish.
        let a = if empty { 70 } else { 108 };
        let ra = if empty { 120 } else { 185 };
        let (cx, cy, rad) = draw_round_face(buf, win_w, win_h, *r, plate, rim, a, ra);

        let icon = slot.icon.and_then(|ic| match ic {
            IconRef::Item(f) => at.item(f),
            IconRef::Skill(sheet, cell) => at.skill(sheet, cell),
        });
        match icon {
            Some(ic) => {
                // `blit_icon` fits the art to the inscribed circle and fades its
                // outer band out, so an icon that overflows dissolves at the rim
                // instead of being cut by a hard arc.
                blit_icon(buf, win_w, win_h, cx, cy, rad * 0.88, ic);
                if matches!(slot.kind, SlotKind::Item) && slot.count > 0 {
                    // Bottom-CENTRE inside the disc: the old bottom-right corner is
                    // outside a circle inscribed in the same rect.
                    let cnt = format!("{}", slot.count.min(99));
                    let cs = (r.h / 20).max(2);
                    let tw = cnt.len() as u32 * 6 * cs;
                    let ty = (cy + rad * 0.42) as u32;
                    let tx = (cx as u32).saturating_sub(tw / 2);
                    // Dim pad behind the digits so they read over bright icon art.
                    blend_disc_soft(
                        buf,
                        win_w,
                        win_h,
                        cx,
                        ty as f64 + 3.5 * f64::from(cs),
                        f64::from(tw.max(6 * cs)) * 0.72,
                        0xff05_070c,
                        150,
                        f64::from(cs) * 1.5,
                    );
                    draw_text(buf, win_w, win_h, tx, ty, cs, &cnt, LABEL);
                }
            }
            // ★ An empty slot draws NOTHING but its dim rim (2026-08-03). It used to
            // print the slot's hotkey character — 1 / 3 / 9 / 0 / # — which was honest
            // when these plates were understood as "a second way to press that number
            // key", and is a lie now that they are quickbar slots: "이제 우측에 스킬
            // 퀵바는 숫자로써의 역할이 아니야. 퀵바로써의 역할인거야."
            //
            // The hotkey itself is untouched — `quickslot_hotkey` is still what a press
            // SENDS (that mapping is baked into the game's field dispatcher). It simply
            // is not the plate's identity any more, so it is not its label.
            None => {}
        }
        // Cooldown fill LAST, over the icon: the whole face reads as recovering.
        if let Some(f) = cooldowns.get(i).copied().filter(|f| *f < 1.0) {
            draw_cooldown_overlay(buf, win_w, win_h, cx, cy, rad, f);
        }
    }
}

/// Ring every quickbar plate in gold: "the game is asking which plate — tap one".
///
/// Drawn as a THICK OUTER RING rather than a filled tint so the icon underneath stays
/// readable: the player is choosing between five things they identify by their icons,
/// and a wash over all five is exactly the wrong moment to hide them.
///
/// This is the visible half of the 퀵바등록 flow (`items::popup_view`). The other half
/// is that a tap now REGISTERS instead of activating — see `App::window_event`.
pub fn draw_pick_highlight(buf: &mut [u32], win_w: u32, win_h: u32, rects: &[Rect]) {
    const PICK: u32 = 0xffe0_9a3c; // the action disc's gold — "this is the one to touch"
    for r in rects {
        if r.w == 0 || r.h == 0 {
            continue;
        }
        let rad = f64::from(r.w.min(r.h)) / 2.0;
        let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
        let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
        blend_ring_soft(buf, win_w, win_h, cx, cy, rad - 1.0, (rad / 7.0).max(2.5), PICK, 255);
        blend_ring_soft(buf, win_w, win_h, cx, cy, rad * 0.74, (rad / 14.0).max(1.5), PICK, 150);
    }
}

/// Alpha-aware bilinear blit of an atlas icon into `dst` (aspect preserved,
/// centred). The disc-clipped variant is [`blit_icon`], which every key now uses
/// (2026-07-30: all round) - this stays for any future rectangular face.
#[allow(dead_code)]
// ── the resampled-icon memo (72차) ───────────────────────────────────────────
//
// ★★★ WHY. Player, 2026-08-29: *"오브및 소켓결합 가방 창을 열었을때 살짝 프레임드랍이
// 걸리고 렉이걸리는듯한 느낌이 들어."* Measured (rustc -O, desktop): the picker's five
// 15x15 item icons, rescaled to 85x85 on a 2340x1080 screen, cost **0.914 ms per frame** —
// more than rasterising the entire 1180x950 panel behind them (0.597 ms), and for scale a
// `fill` over the whole 2.53 M-pixel window is 0.141 ms. The quickbar pays the same tax
// permanently, in every mode, because `draw_skill_buttons` re-scales every plate too.
//
// All of it is recomputation: [`sample_bilinear`] is a pure function of
// `(icon, dw, dh)` — 4 bounds-checked texel fetches, 12 f64 multiplies, a weight
// normalisation with 3 divides and 3 rounds, per destination pixel — and none of those
// three inputs changes while a window is open.
//
// So the RESAMPLED COLOUR GRID is memoised and everything else stays live. That split is
// deliberate: `blit_icon`'s coverage depends on the disc centre and radius, and its feather
// ramp is cheap, so caching the final pixels would need a fatter key for no extra win.
//
// ⛔ Identity is the icon's ADDRESS. Sound only because every call site takes its icon from
// `atlas::atlas()`, a `OnceLock<Atlas>` — the four are `overlay.rs:1720`, `:1998`, `:2127`
// and `:4495`. If an icon is ever built on the fly, key it on content instead.
// ⛔ The key must carry `dw`/`dh`, not just the icon: a rotation or a HEAT SAVER toggle
// changes them and a stale grid would draw at the wrong size.
type ScaledKey = (usize, u32, u32);
static SCALED_ICONS: std::sync::Mutex<Option<std::collections::HashMap<ScaledKey, std::sync::Arc<Vec<u32>>>>> = std::sync::Mutex::new(None);
/// Distinct `(icon, dw, dh)` tuples to hold. One layout needs ~16; the cap is a guard
/// against a resize loop, and overflowing simply clears and refills.
const SCALED_ICONS_MAX: usize = 64;
/// `sample_bilinear` returns a 24-bit colour, so 0 is a legitimate pixel (pure black) and
/// cannot double as "transparent". The stored word carries an explicit present bit.
const SCALED_PRESENT: u32 = 0x8000_0000;

/// `ic` resampled to `dw x dh`, computed once. Entry `y*dw + x` is
/// [`SCALED_PRESENT`] | rgb, or 0 where the icon is transparent.
fn scaled_icon(ic: &atlas::Icon, dw: u32, dh: u32) -> std::sync::Arc<Vec<u32>> {
    let key: ScaledKey = (ic as *const atlas::Icon as usize, dw, dh);
    if let Ok(g) = SCALED_ICONS.lock()
        && let Some(hit) = g.as_ref().and_then(|m| m.get(&key).cloned())
    {
        return hit;
    }
    let (iw, ih) = (f64::from(ic.w), f64::from(ic.h));
    let mut px = Vec::with_capacity((dw * dh) as usize);
    for py in 0..dh {
        for pxi in 0..dw {
            let sx = (f64::from(pxi) + 0.5) / f64::from(dw) * iw - 0.5;
            let sy = (f64::from(py) + 0.5) / f64::from(dh) * ih - 0.5;
            px.push(sample_bilinear(ic, sx, sy).map_or(0, |c| SCALED_PRESENT | c));
        }
    }
    let out = std::sync::Arc::new(px);
    if let Ok(mut g) = SCALED_ICONS.lock() {
        let m = g.get_or_insert_with(std::collections::HashMap::new);
        if m.len() >= SCALED_ICONS_MAX {
            m.clear();
        }
        m.insert(key, out.clone());
    }
    out
}

fn blit_icon_rect(buf: &mut [u32], win_w: u32, win_h: u32, dst: Rect, ic: &atlas::Icon) {
    if ic.w == 0 || ic.h == 0 || dst.w == 0 || dst.h == 0 {
        return;
    }
    let (iw, ih) = (f64::from(ic.w), f64::from(ic.h));
    let sc = (f64::from(dst.w) / iw).min(f64::from(dst.h) / ih);
    let dw = (iw * sc).round() as u32;
    let dh = (ih * sc).round() as u32;
    if dw == 0 || dh == 0 {
        return;
    }
    let ox = dst.x + (dst.w.saturating_sub(dw)) / 2;
    let oy = dst.y + (dst.h.saturating_sub(dh)) / 2;
    let grid = scaled_icon(ic, dw, dh);
    for py in 0..dh {
        for px in 0..dw {
            let (x, y) = (ox + px, oy + py);
            if x >= win_w || y >= win_h {
                continue;
            }
            let s = grid[(py * dw + px) as usize];
            if s & SCALED_PRESENT != 0 {
                buf[(y * win_w + x) as usize] = s & 0x00ff_ffff;
            }
        }
    }
}

/// Scale a decoded atlas icon into the button's inscribed circle with alpha-aware
/// BILINEAR sampling (not nearest). The old nearest path stretched a ~15px icon to
/// a big disc at a non-integer factor, so source texels mapped to uneven 4/5px
/// blocks — the "뭉개짐" (mushy/blocky) look. Bilinear gives a smooth, H5-style
/// icon; edge texels blend only their OPAQUE neighbours (weight-normalised) so the
/// 1-bit-alpha silhouette stays clean with no dark halo, and the disc colour still
/// shows through transparent areas.
fn blit_icon(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, rad: f64, ic: &atlas::Icon) {
    if ic.w == 0 || ic.h == 0 || rad < 3.0 {
        return;
    }
    let box_side = rad * 1.5; // icon box; slightly < diameter so a rim shows
    let (iw, ih) = (f64::from(ic.w), f64::from(ic.h));
    if box_side < 1.0 || iw < 1.0 || ih < 1.0 {
        return;
    }
    // Fit the icon into the box preserving aspect (skill cells and item frames
    // are both 15x15), so nothing stretches; then centre it on the disc. The
    // cells are full-bleed square art, so the disc clip below is what gives the
    // round H5 look — no colour keying is applied to the game's own pixels.
    let sc = (box_side / iw).min(box_side / ih);
    let dw = (iw * sc).round() as u32;
    let dh = (ih * sc).round() as u32;
    if dw == 0 || dh == 0 {
        return;
    }
    let dx0 = (cx - dw as f64 / 2.0) as i64;
    let dy0 = (cy - dh as f64 / 2.0) as i64;
    // Player request (2026-07-30): "it's fine if the icon is clipped by the circle —
    // fade the icon into the circle so it looks natural". So instead of a hard
    // `d > rad -> skip` clip (which leaves a staircase arc across the artwork), the
    // icon's alpha ramps to 0 over the outer `feather` band. Icons that fit well
    // inside the disc are untouched; ones that overflow dissolve at the rim.
    let feather = (rad * 0.20).max(2.0);
    let inner = (rad - feather).max(0.0);
    // ★ 2026-08-05: the icon's OWN rectangular border also gets a gradient. The
    // source cells are full-bleed 15x15 squares, and a square ending in a hard edge
    // inside a round button is the "사각형의 스킬아이콘 ... 살짝 어색한" look the
    // player reported. Alpha now ramps up over the outer `edge_band` of the SQUARE
    // as well (per-axis distance to the nearest icon edge), so the artwork melts
    // into the disc on all four sides instead of only where the circle clips it.
    let edge_band = (f64::from(dw.min(dh)) * 0.16).max(1.5);
    let grid = scaled_icon(ic, dw, dh);
    for oy in 0..dh {
        for ox in 0..dw {
            let px = dx0 + ox as i64;
            let py = dy0 + oy as i64;
            if px < 0 || py < 0 || px as u32 >= win_w || py as u32 >= win_h {
                continue;
            }
            let (ddx, ddy) = (px as f64 + 0.5 - cx, py as f64 + 0.5 - cy);
            let d = (ddx * ddx + ddy * ddy).sqrt();
            if d >= rad {
                continue;
            }
            let cov = if d <= inner { 1.0 } else { (rad - d) / feather };
            let mut cov = cov * cov * (3.0 - 2.0 * cov); // smoothstep
            // Distance (px) from the destination pixel's centre to the nearest edge
            // of the icon rectangle, smoothstepped over `edge_band`.
            let ex = (ox as f64 + 0.5).min(f64::from(dw) - ox as f64 - 0.5);
            let ey = (oy as f64 + 0.5).min(f64::from(dh) - oy as f64 - 0.5);
            let e = (ex.min(ey) / edge_band).clamp(0.0, 1.0);
            cov *= e * e * (3.0 - 2.0 * e);
            // The resampled colour is memoised (see `scaled_icon`); only the coverage
            // above is per-destination and it is cheap.
            let s = grid[(oy * dw + ox) as usize];
            if s & SCALED_PRESENT != 0 {
                let c = s & 0x00ff_ffff;
                let a = (255.0 * cov) as u32;
                if a >= 254 {
                    buf[(py as u32 * win_w + px as u32) as usize] = 0xff00_0000 | (c & 0x00ff_ffff);
                } else if a > 0 {
                    blend_px(buf, (py as u32 * win_w + px as u32) as usize, c, a);
                }
            }
        }
    }
}

/// Alpha-aware bilinear sample of `ic` at fractional source coords `(fx, fy)`.
/// Only opaque (alpha != 0) texels contribute, weighted by bilinear coverage and
/// re-normalised by the opaque weight; returns `None` (transparent) when opaque
/// coverage < half so the icon keeps a crisp silhouette without edge darkening.
fn sample_bilinear(ic: &atlas::Icon, fx: f64, fy: f64) -> Option<u32> {
    let (w, h) = (i64::from(ic.w), i64::from(ic.h));
    let x0 = fx.floor();
    let y0 = fy.floor();
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0i, y0i) = (x0 as i64, y0 as i64);
    let texel = |xi: i64, yi: i64| -> Option<(f64, f64, f64)> {
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            return None;
        }
        let c = ic.px[(yi * w + xi) as usize];
        if (c >> 24) == 0 {
            return None;
        }
        Some((f64::from((c >> 16) & 0xff), f64::from((c >> 8) & 0xff), f64::from(c & 0xff)))
    };
    let corners = [
        (texel(x0i, y0i), (1.0 - tx) * (1.0 - ty)),
        (texel(x0i + 1, y0i), tx * (1.0 - ty)),
        (texel(x0i, y0i + 1), (1.0 - tx) * ty),
        (texel(x0i + 1, y0i + 1), tx * ty),
    ];
    let (mut r, mut g, mut b, mut wsum) = (0.0, 0.0, 0.0, 0.0);
    for (c, wgt) in corners {
        if let Some((cr, cg, cb)) = c {
            r += cr * wgt;
            g += cg * wgt;
            b += cb * wgt;
            wsum += wgt;
        }
    }
    if wsum < 0.5 {
        return None;
    }
    let r = (r / wsum).round() as u32;
    let g = (g / wsum).round() as u32;
    let b = (b / wsum).round() as u32;
    Some((r << 16) | (g << 8) | b)
}

/// Paint over the game's own field HUD so our custom HUD replaces it: the
/// bottom full-width strip (quickbar window + HP/SP/EXP bars) and the top-left
/// 2-skill box. Regions are content-frame fractions (240x320) mapped into the
/// game rect; opaque black to blend with the letterbox. Device-tunable.
pub fn mask_game_hud(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect) {
    let mask = 0xff00_0000u32;
    let (lw, lh) = crate::logical();
    if crate::immersive_active() {
        // Measured with wie_probe at logical widths 240/300/426/514/640: the game's
        // own field HUD is a FIXED **176 x 24 logical px** box, horizontally
        // CENTRED and bottom-anchored (the run length never changed and its centre
        // tracked the screen centre to within 1 px). So mask exactly that box —
        // masking the whole width would black out live world pixels either side.
        const HUD_W: u32 = 176;
        const HUD_H: u32 = 24;
        // ★ 2026-07-30: in immersive mode we no longer mask anything.
        //
        // The game paints its own HUD (HP/SP/EXP bars + the 5 quickbar cells) into a
        // fixed 176x24 guest box, bottom-centre. Hiding it means painting that box
        // OPAQUE — nothing can restore the world pixels underneath — and the player
        // reported that black band as wrong twice: first as a wide translucent tint,
        // then as the box itself. Since our EXP bar has moved up to the HP/SP stack
        // there is nothing of ours left in there, so the honest option is to leave
        // the game's own strip visible (it is the original art, and it doubles as the
        // quickbar readout) instead of covering the world with black.
        //
        // `game_hud_box` still reports the region for layout code that must avoid it.
        let _ = (mask, HUD_W, HUD_H, lw, lh);
        return;
    }
    // Bottom HUD strip: content y >= 288 (of 320).
    let strip_y = game.y + game.h * 288 / 320;
    fill(
        buf,
        win_w,
        win_h,
        game.x,
        strip_y,
        game.w,
        (game.y + game.h).saturating_sub(strip_y),
        mask,
    );
    // Top-left 2-skill box: content x<48, y<28.
    fill(buf, win_w, win_h, game.x, game.y, game.w * 48 / 240, game.h * 28 / 320, mask);
}

/// Original content width the game's out-of-world screens are laid out for.
const SHELL_ART_W: u32 = CONTENT_W; // 240

/// Letterbox the title / slot-select screens.
///
/// Those screens are 240-px art. Given a wider framebuffer the game fills the extra
/// width by MIRRORING its background outwards (measured on a device capture: the
/// frame is non-black across the full logical width and the two halves mirror about
/// the centre) — so nothing is missing, but the edges show headless copies of the
/// artwork and read as "the title is cut off" (player-reported 2026-07-30).
/// Blacking those bands out makes it a clean centred title instead.
///
/// Only the shell state is affected; the field and the game's windows are untouched.
pub fn letterbox_shell(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect) {
    if !crate::immersive_active() || game.w == 0 {
        return;
    }
    let (lw, _) = crate::logical();
    if lw <= SHELL_ART_W {
        return;
    }
    let art = game.w * SHELL_ART_W / lw;
    let side = (game.w - art) / 2;
    if side == 0 {
        return;
    }
    // FEATHERED gradient that starts INSIDE the artwork.
    //
    // What the player is actually seeing (2026-07-30, third pass): the hard edge is
    // the ARTWORK'S OWN. The title screen is authored for a 240-px-wide portrait
    // screen; landscape has width to spare, so the game paints the art in the middle
    // and its rectangle simply STOPS — a razor-straight vertical edge. A gradient
    // that only starts outside that edge cannot soften it, because the discontinuity
    // is at the edge itself.
    //
    // So the ramp straddles the boundary: alpha is already non-zero a little way
    // INSIDE the art (`FEATHER`), reaching full dark at the screen edge. The art's
    // own outer strip fades out instead of ending, so there is no line anywhere.
    // Eased (t^2) so the darkening is imperceptible where it begins.
    // Measured on device (row-luminance profile of a real capture): OUTSIDE the art
    // the game's own output is already black at most rows. So darkening the outside
    // achieves nothing — the visible razor edge is the ARTWORK'S OWN last column
    // meeting that black. The ramp therefore has to run the other way: transparent
    // some way INSIDE the art, reaching FULL dark AT (and just past) its edge, so the
    // art's outer strip dissolves into the surrounding black.
    // `f_in` = how far the fade reaches into the art, `f_out` = how far past the
    // nominal edge it takes to become fully opaque. The overshoot matters: the art is
    // a couple of logical px wider than the nominal 240, and stopping exactly at 240
    // would itself cut a fresh hard edge.
    let f_in = (art * 10 / 100).max(2);
    let f_out = (art * 4 / 100).max(1);
    let art_left = game.x + side;
    let art_right = game.x + side + art;
    let span = f_in + f_out;
    // Left: alpha 0 at art_left + f_in -> full at art_left - f_out, then solid.
    edge_ramp(buf, win_w, win_h, game, art_left.saturating_sub(f_out), art_left + f_in, false, span);
    fill(buf, win_w, win_h, game.x, game.y, side.saturating_sub(f_out), game.h, 0xff00_0000);
    // Right: mirrored.
    edge_ramp(buf, win_w, win_h, game, art_right.saturating_sub(f_in), art_right + f_out, true, span);
    let solid_x = art_right + f_out;
    fill(
        buf,
        win_w,
        win_h,
        solid_x,
        game.y,
        (game.x + game.w).saturating_sub(solid_x),
        game.h,
        0xff00_0000,
    );
}

/// One column-wise alpha ramp over `[from, to)`, eased `t^2`, black.
/// `invert` = the dark end is on the RIGHT (mirrored side).
#[allow(clippy::too_many_arguments)]
fn edge_ramp(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, from: u32, to: u32, invert: bool, span: u32) {
    let span = f64::from(span.max(1));
    for x in from..to.min(game.x + game.w) {
        let d = if invert { x.saturating_sub(from) } else { to.saturating_sub(x) };
        let t = (f64::from(d) / span).clamp(0.0, 1.0);
        let a = (255.0 * t * t) as u32;
        if a > 0 {
            blend_rect(buf, win_w, win_h, x, game.y, 1, game.h, 0xff00_0000, a);
        }
    }
}

// hero2's STATUS HUD (hero5-style portrait ring · HP/SP bars · level plate, and the
// EXP bar at the bottom centre) — its own dependency-free file, shared with hero2
// verbatim (141차, 2026-09-30).
include!("hud_view.rs");

/// The status block's geometry for this game rect (`hud_view::HvGeom` — one pixel of it
/// is the game's own pixel, so it scales with the game, not with the window).
fn status_geom(game: Rect) -> HvGeom {
    let (lw, lh) = crate::logical();
    HvGeom::new((game.x, game.y, game.w, game.h), lw, lh)
}

/// A `hud_view` `(x, y, w, h)` as a [`Rect`].
fn status_rect(r: (u32, u32, u32, u32)) -> Rect {
    Rect::new(r.0, r.1, r.2, r.3)
}

/// Geometry of the player-gauge block — since 141차 (2026-09-30) hero5's, as hero2 has
/// it (player: *"hero2 에 좌측상단 캐릭터 외형, HP바, SP바, 중앙하단 경험치바, 미니맵
/// 디자인이 변경되었어. 마찬가지로 hero3에도 적용하고싶어"*): the portrait RING, the HP/SP
/// bars beside it, the level plate under it. `hud_view.rs` owns the numbers; this is the
/// same block in `Rect`s for the callers that place things around it. ONE source of
/// truth, so the controls that sit next to it cannot drift on top of it — which is
/// exactly what happened on the first device build.
///
/// ⚠ This block still DRIVES the guest poke that moves the game's own buff band to
/// start right of it ([`band_x_shift`]).
pub struct StatsBlock {
    /// The HP and SP bars as they show right of the ring — what the buff band and
    /// buff lines line up against. (The ring itself is inside [`stats_block_box`];
    /// EXP left the block: it is the thin bar at the bottom centre now —
    /// [`draw_exp_bar`].)
    pub bars: Rect,
}

/// Player-gauge block: extreme TOP-LEFT with a small margin (player request
/// 2026-07-30 — an earlier version pushed it right, to dodge the game's own buff
/// band, and it read as "floating in the middle"; the band is moved DOWN instead,
/// see `crate::items::hud_y_origin`). HP, SP and EXP are one stack: the EXP bar
/// used to live in the game's bottom HUD box, which meant masking that box black.
pub fn stats_block(game: Rect) -> StatsBlock {
    let g = status_geom(game);
    StatsBlock { bars: status_rect(g.bars()) }
}

/// The gauge block as a TAP TARGET: tapping the level plate / HP-SP-EXP bars opens
/// the game's main menu (player request 2026-08-03 — "제일 좌측 상단에 Lv과 HP
/// 표시하는곳 클릭하면 메뉴가 열리게").
///
/// It sends `CLEAR`, which IS the game's open-main-menu key on the field (and its
/// back key everywhere else) — device-probed, see `CUSTOM_HUD.md` §키 역할 실측. So
/// this is not a new capability, it is a second, bigger target for a key the CLR
/// button already sends: the block is ~490x82 px against a ~98 px disc, and it is
/// where a player's eye already is when they think "menu".
///
/// GROWN by EXACTLY ONE pad on every side: the drawn bars are thin, and a target you
/// have to hit precisely is worse than no target. It stays a thin top-left strip, so
/// it does not eat the area a steering thumb uses.
///
/// ★ ONE pad, not two, and this is a correctness bound rather than taste. This zone
/// is hit-tested BEFORE `layout.buttons` (app.rs), so every pixel it owns is a pixel
/// the control underneath never gets — and the only thing keeping a control out from
/// under it is [`crate::placement::obstacles_fixed`], which reserves the UNGROWN
/// [`stats_block_box`]. `accepts` there admits a candidate as soon as
/// `cand.grown(pad)` misses that rect, i.e. a control is legal at exactly `pad` of
/// clearance. With half-open intersection, growing by `pad` puts this zone's edge
/// precisely ON the nearest legal control edge and therefore never inside it; a
/// second pad reaches `pad` past it.
///
/// That is not theoretical: a `pad`-wide annulus of legal-but-swallowed placements
/// was found by an adversarial review of this change (2026-08-03) and brute-force
/// confirmed — over a thousand accepted positions for the OK disc alone at
/// 2340x1080, all of them ones the editor's "slide along the obstacle" drag actively
/// encourages, where tapping the disc's outer sliver opened the main menu instead of
/// attacking. `the_gauge_tap_zone_never_shadows_a_legal_control` in the selftest pins
/// the rule so the growth cannot creep back.
///
/// ★ Field only, and `app.rs` treats it as a deliberate TAP (down and up inside,
/// little movement) rather than a button — the same rule the minimap panel uses, and
/// for the same reason: the immersive joystick zone is the whole window, so a region
/// that fired on touch-down would steal the start of a walk.
pub fn stats_tap_box(game: Rect) -> Rect {
    // The same `pad` the resolver uses: the zone only exists in landscape immersive,
    // where `game` IS the whole window, so the two agree by construction.
    let pad = pad_of(game.w.max(1), game.h.max(1));
    stats_block_box(game).grown(pad)
}

// ── unspent stat / skill point badges ────────────────────────────────────────
//
// ★ NOT invented art: these are the GAME'S OWN two badges, lifted pixel-for-pixel
// out of JAR `/comm/ui_bm` **frame 6** (a 14x7 strip = two 7x7 cells). The game
// draws them at the end of its bottom-HUD block — `0x14a372` blits cell srcX 0 when
// `(s16)*(CHAR+0x18a) > 0` and `0x14a32e` blits cell srcX 7 when `*(CHAR+0x18c) != 0`
// — both at `baseX + 0x99 / +0xa2, baseY - 1`, i.e. the RIGHT END of that block, just
// above it. That block is exactly what `items::hud_bottom_patch` branches over, so
// the badges disappeared with it; this redraws them beside our own gauges, which is
// the same place relative to the HP/SP bars.
//
// Index 0 of a `_bm` palette is the 0xf81f transparent sentinel, which is why the
// four corners are 0 here.
/// Red `+` = unspent STAT points. `/comm/ui_bm` frame 6, cell at srcX 0.
/// Rim 0x940000, fill 0xff0000, glyph 0xfffaff.
const POINT_BADGE_STAT: [u32; 49] = [
    0x0000_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0x0000_0000,
    0xff94_0000,
    0xffff_0000,
    0xffff_0000,
    0xffff_faff,
    0xffff_0000,
    0xffff_0000,
    0xff94_0000,
    0xff94_0000,
    0xffff_0000,
    0xffff_0000,
    0xffff_faff,
    0xffff_0000,
    0xffff_0000,
    0xff94_0000,
    0xff94_0000,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xff94_0000,
    0xff94_0000,
    0xffff_0000,
    0xffff_0000,
    0xffff_faff,
    0xffff_0000,
    0xffff_0000,
    0xff94_0000,
    0xff94_0000,
    0xffff_0000,
    0xffff_0000,
    0xffff_faff,
    0xffff_0000,
    0xffff_0000,
    0xff94_0000,
    0x0000_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0xff94_0000,
    0x0000_0000,
];
/// Blue `+` = unspent SKILL points. `/comm/ui_bm` frame 6, cell at srcX 7.
/// Rim 0x000094, fill 0x0061ff, glyph 0xfffaff.
const POINT_BADGE_SKILL: [u32; 49] = [
    0x0000_0000,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0x0000_0000,
    0xff00_0094,
    0xff00_61ff,
    0xff00_61ff,
    0xffff_faff,
    0xff00_61ff,
    0xff00_61ff,
    0xff00_0094,
    0xff00_0094,
    0xff00_61ff,
    0xff00_61ff,
    0xffff_faff,
    0xff00_61ff,
    0xff00_61ff,
    0xff00_0094,
    0xff00_0094,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xffff_faff,
    0xff00_0094,
    0xff00_0094,
    0xff00_61ff,
    0xff00_61ff,
    0xffff_faff,
    0xff00_61ff,
    0xff00_61ff,
    0xff00_0094,
    0xff00_0094,
    0xff00_61ff,
    0xff00_61ff,
    0xffff_faff,
    0xff00_61ff,
    0xff00_61ff,
    0xff00_0094,
    0x0000_0000,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0xff00_0094,
    0x0000_0000,
];
/// Source cell size of both badges (`ui_bm` frame 6 is 14x7 = 2 x 7x7).
pub const BADGE_SRC: u32 = 7;

/// Lines the game-style message box holds (`draw_native_notice`: a 57-px body at a 13-px
/// pitch, first baseline 7 px down).
pub const NOTICE_LINES: usize = 4;
/// Logical px of text one line of that box can carry: the 164-px panel less its outline
/// and ring on both sides (2 + 4 each) and the 4-px text inset on both.
pub const NOTICE_LINE_W: usize = 164 - 2 * (2 + 4) - 2 * 4;

/// Break `text` into lines no wider than the message box, at spaces when there is one
/// and mid-word when a single word is wider than the box. Measured in the game's own
/// font (`gamefont::width`), which is what draws it.
///
/// ★ 140차: moved here from `bagbox_host` — every host notice goes through the box now
/// (the toast is gone), so the UI wraps whatever it is handed ([`wrap_notice_text`]).
pub fn wrap_notice(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if word.is_empty() {
            continue;
        }
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if crate::gamefont::width(&candidate) <= NOTICE_LINE_W {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        // A word wider than the box on its own: split it by character.
        let mut piece = String::new();
        for ch in word.chars() {
            let mut next = piece.clone();
            next.push(ch);
            if crate::gamefont::width(&next) > NOTICE_LINE_W && !piece.is_empty() {
                lines.push(std::mem::take(&mut piece));
            }
            piece.push(ch);
        }
        line = piece;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` re-flowed for the message box: every `'\n'` the sender put in is kept, each
/// resulting line is wrapped to the box's width, and the whole is cut to the
/// [`NOTICE_LINES`] the body can hold. Idempotent on text that already fits, so a sender
/// that wrapped for itself (`bagbox_host`'s 상세보기) loses nothing.
pub fn wrap_notice_text(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        if raw.trim().is_empty() {
            lines.push(String::new());
        } else {
            lines.extend(wrap_notice(raw));
        }
    }
    lines.truncate(NOTICE_LINES);
    lines.join("\n")
}

/// Blink period of the badges, in ms per half-cycle: 250 on / 250 off = 2 Hz.
///
/// The original toggles its flag on 49 of every 50 HUD frames (`(counter+1) % 50`
/// gates the toggle at guest 0x14a2a2), i.e. it blinks at the frame rate — on a
/// feature phone that reads as a 50 %-brightness flicker, on a 60 fps phone it is
/// just noise. Player choice (2026-08-04): a readable ~2 Hz instead.
///
/// ★ Driven by HOST ELAPSED TIME, never by the guest's own blink flag
/// (`*(u8*)*(sl+0x434)`): the counter that flips it lives at guest 0x14a296, which
/// is INSIDE the range `items::hud_bottom_patch` branches over, so it never advances
/// — reading it would pin the badges permanently on or permanently off.
pub const BADGE_BLINK_MS: u128 = 250;

/// Is the badge visible this instant? `ms` = host milliseconds since app start.
pub fn badge_blink_on(ms: u128) -> bool {
    (ms / BADGE_BLINK_MS) % 2 == 0
}

/// Where the two badges go: side by side (the original's own pairing, 7 px cell +
/// 2 px gap, scaled) BELOW the HP/SP/EXP bar stack, left-aligned with it.
///
/// ★ Moved from the bars' right side 2026-08-05 (player request: "he mp바 우측에
/// 깜빡이며 표시가되는데 그걸 우측이아닌 하단에 표시되도록"). Below the stack the
/// badges also stop competing with the game's own buff band, whose x starts just
/// right of our bars (`band_x_shift`).
pub struct PointBadges {
    pub stat: Rect,
    pub skill: Rect,
}

pub fn point_badges(game: Rect) -> PointBadges {
    // One hud pixel per art pixel: the scale is a whole number by construction, which
    // matters — 7 is odd, and a fractional scale smears the `+`'s one-pixel arms.
    // Beside the level plate, under the SP bar (141차; before that under the old
    // capsule stack, 2026-08-05).
    let g = status_geom(game);
    PointBadges { stat: status_rect(g.badge(0)), skill: status_rect(g.badge(1)) }
}

/// The gauge block as ONE rect (plate + bars + both badges), for the no-go set: it is
/// drawn after the controls, so anything placed under it just disappears behind it.
///
/// ★ The badges are included UNCONDITIONALLY, even though they only draw while points
/// are unspent. Reserving space that is usually empty is the safe direction: the
/// alternative is a control that is legal at level 1 and then gets covered the first
/// time the player levels up. This also widens [`stats_tap_box`] by the same amount,
/// which keeps that function's "exactly one pad of growth" argument intact — both the
/// reserved rect and the tap zone move together — and is a bonus for the player,
/// since tapping a badge now opens the menu the points are spent in.
pub(crate) fn stats_block_box(game: Rect) -> Rect {
    status_rect(status_geom(game).bbox())
}

/// Nearest-neighbour blit of a 7x7 badge into `r`, skipping the transparent corners.
/// No blending: every non-transparent source pixel is fully opaque (the `_bm` palette
/// has one transparent entry and no alpha), which is also what softbuffer wants.
fn blit_badge(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, src: &[u32; 49]) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let x1 = (r.x + r.w).min(win_w);
    let y1 = (r.y + r.h).min(win_h);
    for py in r.y..y1 {
        let sy = ((py - r.y) * BADGE_SRC / r.h).min(BADGE_SRC - 1);
        for px in r.x..x1 {
            let sx = ((px - r.x) * BADGE_SRC / r.w).min(BADGE_SRC - 1);
            let c = src[(sy * BADGE_SRC + sx) as usize];
            if c != 0 {
                buf[(py * win_w + px) as usize] = c;
            }
        }
    }
}

/// Draw whichever badges have unspent points, if the blink says "on".
fn draw_point_badges(buf: &mut [u32], win_w: u32, win_h: u32, stat: Rect, skill: Rect, s: &Stats, blink: bool) {
    if !blink {
        return;
    }
    // `> 0` for both: the game tests stat points with `ble` (signed) and skill points
    // with `beq`, and `read_stats` already folded a negative stat count to 0.
    if s.stat_points > 0 {
        blit_badge(buf, win_w, win_h, stat, &POINT_BADGE_STAT);
    }
    if s.skill_points > 0 {
        blit_badge(buf, win_w, win_h, skill, &POINT_BADGE_SKILL);
    }
}

/// Window rect of the game's own (masked) HUD box in immersive mode — the region
/// our overlay must not fight with. `None` outside immersive mode.
/// No longer masked (see `mask_game_hud`) — kept because it documents the one
/// region of the frame the game owns at the bottom.
#[allow(dead_code)]
pub fn game_hud_box(game: Rect) -> Option<Rect> {
    if !crate::immersive_active() {
        return None;
    }
    let (lw, lh) = crate::logical();
    let bw = (game.w * 176 / lw.max(1)).min(game.w);
    let bh = (game.h * 24 / lh.max(1)).max(2);
    Some(Rect::new(game.x + (game.w - bw) / 2, (game.y + game.h).saturating_sub(bh), bw, bh))
}

/// Draw the player gauges (HP / SP / EXP) + level at the top-left of the game
/// rect — the custom replacement for the masked game HUD. Thin rounded capsule
/// (pill) gauges, game-gauge style: rounded ends, slim, colour-coded, with a fill
/// that grows from the left. (Was: thick square bars with a hard border.)
/// Draw the player's status block at the top-left of the game rect — the custom
/// replacement for the game's own bottom HUD.
///
/// ★★ 141차 (2026-09-30): hero5's block, as hero2 draws it — the character's own head in
/// a gold ring (`portrait`: the game's composition, captured off the frame — helmet,
/// hair palette and all), HP and SP in a silver-framed block with their values, the
/// level on a plate under the ring. The EXP bar left the block for the bottom centre
/// ([`draw_exp_bar`]). One layout for both orientations: its pixel is the game's pixel
/// (`hud_view::hv_unit`). The drawing is `hud_view.rs`.
/// (Was: a LV disc and three thin capsules — HP · SP · EXP.)
///
/// SP is the game's 0..1000 word, shown as the 0..100 the game's own skill costs and
/// SP bar use (129차: the bar draws `(CHAR+0xf8)/10` over 100).
// ★ 153차: the ring's picture is `portrait.rs`'s face square, cut to the ring's own size
// so it lands one to one (the ring centres what it is given).
const _: () = assert!(crate::items::FACE_SIZE == HV_RING);
pub fn draw_stats(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, s: &Stats, blink: bool, portrait: Option<&crate::Portrait>) {
    if game.w == 0 || game.h == 0 {
        return;
    }
    let g = status_geom(game);
    let head = portrait.filter(|p| p.w > 0 && p.h > 0 && p.px.len() == usize::from(p.w) * usize::from(p.h)).map(|p| HvIcon { w: u32::from(p.w), h: u32::from(p.h), px: &p.px });
    let hv = HvStats {
        hp: u32::from(s.hp),
        hp_max: u32::from(s.hp_max),
        sp: u32::from(s.sp) / 10,
        sp_max: (u32::from(s.sp_max) / 10).max(1),
        level: u32::from(s.level),
    };
    hv_draw_status(buf, win_w, win_h, &g, &hv, head);
    // The game's own unspent-point badges, beside the level plate.
    let pb = point_badges(game);
    draw_point_badges(buf, win_w, win_h, pb.stat, pb.skill, s, blink);
}

/// The EXP bar at the bottom centre of the game rect (141차 — hero5's place for it; it
/// was the third capsule under HP/SP). Drawn BEFORE the controls, unlike the block at
/// the top-left: it is not an obstacle to them, so a control the player parks over the
/// bottom edge covers it rather than the other way round.
pub fn draw_exp_bar(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, s: &Stats) {
    if game.w == 0 || game.h == 0 {
        return;
    }
    let (lw, lh) = crate::logical();
    let e = HvExp::new((game.x, game.y, game.w, game.h), lw, lh);
    hv_draw_exp(buf, win_w, win_h, &e, s.exp.min(s.exp_next), s.exp_next.max(1));
}

/// Reference-style status block: a round portrait plate with the level inside,
/// HP and SP bars beside it carrying their CURRENT VALUE as text, and the EXP
/// bar moved to a thin strip at the bottom centre (inside the band the game's
/// own HUD already occupies, so it costs no extra field pixels).


pub fn gear_hit(gear: &Rect, x: f64, y: f64) -> bool {
    gear.contains(x, y)
}

/// Whether `(x, y)` is inside a plain UI rect (keypad toggle, panels). A
/// zero-sized rect never hits, which is how the disabled toggle is expressed.
pub fn rect_hit(r: &Rect, x: f64, y: f64) -> bool {
    r.contains(x, y)
}

/// Whether `(x, y)` falls in the joystick zone (joystick mode only).
pub fn in_joystick_zone(layout: &Layout, x: f64, y: f64) -> bool {
    layout.joystick.as_ref().is_some_and(|z| z.contains(x, y))
}

/// The key a touch that hit NOTHING sends: `OK` during a CUTSCENE, nothing anywhere
/// else. Player request 2026-08-17, immediately after the previous day's "컷신에서는
/// `#` 하나만" shipped and left the script with no way to advance: "컷신에서 다음
/// 대화로 넘어갈 수가 없어 … `#`을 제외한 어디를 눌러도 대화가 넘어가게 …
/// 화면전체가 OK버튼인 느낌이야. 이 정책은 컷신일때만 작동해야 해."
///
/// ★ It is a FALLBACK, not a rect: `App` runs it as the last arm of the same
/// hit-test chain the `#` sits in, so "everywhere except the controls" needs no
/// geometry at all and can never drift from what is drawn. That also keeps the `#`
/// working as the skip — the corner target over the game's own `# SKIP` lettering
/// is tested first and wins its own pixels (the disc it used to share this chain
/// with left in 64차).
///
/// ★ Cutscene ONLY. On the field the same fallback would fire the game's attack key
/// on every stray touch of open ground, and in a menu it would confirm whatever the
/// cursor happens to sit on.
pub fn tap_anywhere_key(mode: Mode) -> Option<KeyCode> {
    (mode == Mode::Cutscene).then_some(KeyCode::OK)
}

// --------------------------------------------------------------------------
// Settings modal
// --------------------------------------------------------------------------

fn settings_panel(win_w: u32, win_h: u32) -> Rect {
    // Lower bound must never exceed the upper bound (tiny placeholder dims in
    // App::new would otherwise make clamp panic with min > max).
    // Grown 56 % -> 72 % of the height for the two size-knob rows (2026-07-31).
    let pw = (win_w * 74 / 100).clamp(360.min(win_w), win_w.max(1));
    let ph = (win_h * 72 / 100).clamp(340.min(win_h), win_h.max(1));
    Rect::new((win_w - pw) / 2, (win_h - ph) / 2, pw, ph)
}

/// Height of the modal's HEADER strip (the "SET" title and the close box). Shared
/// by the layout and the drawing, so the title can never be sized onto row 1.
fn settings_head(p: Rect) -> u32 {
    let close_s = (p.w / 8).clamp(56, 110);
    (p.h / 30 + close_s + p.h / 40).max(p.h * 18 / 100)
}

/// The settings modal:
///
/// ```text
///   SET                                   [X]
///   [        화면 배치               ]       the layout editor (2026-07-31)
///   [        진단 정보 / 세이브 복원  ]       diagnostics + the save vault (60차)
///   [        조작 방식 · 발열 절감 · 효과음 · (패드 표시) ]
/// ```
///
/// ★★★ 99차 (2026-09-16): the ACCOUNT, NET (창고 연결) and 서버 주소 rows are GONE — the
/// game is 100 % offline (user decision: *"상단톱니바퀴에서도 이제 네트워크창고 온라인으로
/// 접속하는 선택지 없애버릴꺼야"*). There is no server to address, no account to carry and
/// no source to choose; the warehouse and the cash shop are always the in-app engine.
///
/// ★ Trimmed twice on player request. 2026-07-31 ("layout edit 외 나머지는 다
/// 지워도 될 것 같아") removed PORTRAIT/LANDSCAPE, D-PAD/JOYSTICK, FULL and ZOOM —
/// one-time setup knobs whose persisted values still work, so putting a row back is
/// one line; `config::read_landscape` DEFAULTS to landscape for the same reason.
///
/// 2026-08-05 removed two more: SAVE moved OUT of the modal onto the on-field
/// SAVE NOW button (`crate::placement::CtrlId::QuickSave`, 20차) — the modal row
/// could never succeed anyway, since the gear exists only OFF the field while the
/// game's save gate passes only ON it — and SERVER went entirely ("설정 버튼
/// 눌렀을때 사설 서버 정보 나오는 항목 제거"); 99차 then removed the server machinery
/// itself, so that row can no longer come back.
///
/// The header height is RESERVED rather than assumed: an earlier layout put row 1 at
/// 20 % of the panel, which the "SET" title (up to scale 22 = 154 px) reached into.
pub fn settings_items(win_w: u32, win_h: u32) -> Vec<SettingsItem> {
    let p = settings_panel(win_w, win_h);
    let close_s = (p.w / 8).clamp(56, 110);
    let close = Rect::new(p.x + p.w - close_s - p.w / 30, p.y + p.h / 30, close_s, close_s);

    let head = settings_head(p);
    let rows_h = p.h.saturating_sub(head + p.h / 24).max(3);
    // ★ TWO MORE ROWS when there is a gamepad (59차; the second 2026-10-09) — and only then.
    // A row that configures the pad can do nothing for a player with no pad, so they appear
    // the moment one is used and stay for the session; they also have to stay visible while
    // either setting is anything but the default, or a player who changed one could not
    // change it back without first picking up the pad.
    let pad_row = crate::pad_seen()
        || crate::pad_hud_pref() != crate::padhud::PadHud::Auto
        || crate::gamepad::remapped();
    // ★ FIVE base rows since 99차 (2026-09-16): 화면 배치 · 진단 정보 · 조작 방식 · 발열 절감 ·
    // 효과음. The ACCOUNT, NET and 서버 주소 rows went with the online features. The
    // 효과음 row sits with HEAT SAVER because both are per-DEVICE tuning, and it is
    // unconditional — the player who needs it is exactly the one whose phone is too
    // quiet to discover it.
    let rows = 4 + 2 * u32::from(pad_row);
    let pitch = rows_h / rows;
    let bh = (pitch * 70 / 100).max(1);
    let row = |i: u32| p.y + head + i * pitch + (pitch - bh) / 2;

    let bw = p.w * 78 / 100;
    let x0 = p.x + (p.w - bw) / 2;

    let mut items = vec![
        SettingsItem {
            rect: close,
            action: SettingsAction::Close,
            kind: SKind::Close,
            enabled: true,
        },
        SettingsItem {
            rect: Rect::new(x0, row(0), bw, bh),
            action: SettingsAction::EditLayout,
            kind: SKind::EditLayout,
            // Only the landscape-immersive HUD has placeable controls; every other
            // layout family builds a fixed D-pad + keypad from the screen edges and
            // would ignore a stored placement entirely.
            enabled: layout_editor_available(win_w, win_h),
        },
        SettingsItem {
            rect: Rect::new(x0, row(1), bw, bh),
            action: SettingsAction::Diagnostics,
            kind: SKind::Diag,
            // Always available: it is precisely the broken configurations (no
            // audio, wrong sizes) that need this row reachable.
            enabled: true,
        },
        SettingsItem {
            rect: Rect::new(x0, row(2), bw, bh),
            action: SettingsAction::ToggleHeatSaver,
            kind: SKind::HeatSaver,
            // Always available: heat is a per-device problem and every layout
            // family pays the same render cost.
            enabled: true,
        },
    ];
    items.push(SettingsItem {
        rect: Rect::new(x0, row(3), bw, bh),
        action: SettingsAction::CycleSfxLevel,
        kind: SKind::SfxLevel,
        // Always available: a speaker that is too quiet for the effect bus is a
        // per-device fact no layout family or feature flag can predict.
        enabled: true,
    });
    if pad_row {
        items.push(SettingsItem {
            rect: Rect::new(x0, row(4), bw, bh),
            action: SettingsAction::TogglePadHud,
            kind: SKind::PadHud,
            // Always available once it is here at all: the pad HUD is not tied to a
            // layout family (it hides walk controls and badges the rest, which every
            // family has).
            enabled: true,
        });
        items.push(SettingsItem {
            rect: Rect::new(x0, row(5), bw, bh),
            action: SettingsAction::PadMap,
            kind: SKind::PadMap,
            // Always available: what a button does is not tied to a layout family either.
            enabled: true,
        });
    }
    items
}

/// Is the layout editor usable at this window? Shared by the row's enabled state
/// and by `App::enter_editor`, so the button cannot say one thing and the handler do
/// another.
pub fn layout_editor_available(win_w: u32, win_h: u32) -> bool {
    crate::immersive_active() && win_w > win_h
}

pub fn settings_hit(items: &[SettingsItem], win_w: u32, win_h: u32, x: f64, y: f64) -> Option<SettingsAction> {
    if let Some(item) = items.iter().find(|i| i.rect.contains(x, y)) {
        // A disabled row swallows the tap: falling through would hand the point to
        // the "outside the panel closes it" rule below and shut the modal.
        return item.enabled.then_some(item.action);
    }
    // Tap outside the panel closes the settings.
    if !settings_panel(win_w, win_h).contains(x, y) {
        return Some(SettingsAction::Close);
    }
    None
}

// --------------------------------------------------------------------------
// Layout editor (player request 2026-07-31)
// --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditAction {
    /// Mirror the whole layout left<->right — a one-tap left-handed layout.
    ///
    /// ★ "the whole layout", not "this mode's": there is one arrangement since
    /// 2026-09-10 (see `crate::placement::Table`), and the GAME / MENU page switch that
    /// used to sit ahead of this button is gone with it.
    Flip,
    /// Switch the SELECTED quickbar plate off (player request 2026-09-10: "스킬 퀵슬롯을
    /// 선택 후 비활성화 처리 … 숨김처리를 하는거야"). No-op unless a plate is selected,
    /// and refused for the last visible one — see [`crate::quickpage::hide_plate`].
    HidePlate,
    /// Switch every hidden plate back on ("다시 숨김처리한거 되돌릴수도있고").
    ///
    /// All of them at once rather than one at a time, and that is a consequence of the
    /// editor showing the layout AS IT WILL BE: a hidden plate has no seat on the ring
    /// and nothing to tap. Restoring the row and switching off the ones you still do
    /// not want is two taps more and needs no ghost furniture.
    ShowPlates,
    /// Put every control back to the built-in layout.
    Reset,
    /// Discard everything done since the editor was opened, and close.
    Cancel,
    /// Keep the changes and close.
    Save,
}

pub struct EditItem {
    pub rect: Rect,
    pub action: EditAction,
}

/// The editor's own chrome: one compact row across the TOP band.
///
/// ★ Moved to the TOP and made smaller 2026-08-03 (player: "설정에서 save, 취소 등등
/// 버튼들이 가운데 쪽에 몰려있는데 상단으로 옮겨주고 크기도 조금 줄여줘 — 드래그하는
/// 과정에서 자꾸 설정 버튼을 누르게 된다"). The chrome is hit-tested BEFORE the
/// controls, and dragging happens across the middle and lower screen where the
/// controls live, so a block sitting in the bottom-centre was directly in the path of
/// every long drag. The top band is the one strip a drag does not cross: our gauges
/// own its left end, the minimap panel owns its right end, and the middle was empty.
/// (It was on the left edge before 2026-07-31 and in the bottom-centre after — both
/// were in the way of something the player wanted to do.)
///
/// The span is DERIVED, not centred: from the right edge of our gauge block to the
/// left edge of the minimap's built-in corner. Both are pure functions of the window,
/// which they have to be — this function is itself an obstacle in
/// [`crate::placement::obstacles`], so reading a PLACED rect here would recurse.
/// Keeping the row inside that band is what stops the chrome from burying a control:
/// anything under it could never be picked up again, and the tap meant to grab it
/// would hit RESET instead.
///
/// Labels are ASCII: `draw_glyph` is a 5x7 ASCII bitmap font, so Korean would render as
/// blank boxes. FLIP = 좌우 반전, HIDE / SHOW = 퀵슬롯 숨김 / 되돌리기.
///
/// ★★ GAME / 메뉴오픈 used to lead this row. Removed 2026-09-10 with the second seat
/// table (`crate::placement::Table`): the editor now always shows the ONE arrangement,
/// which is also what a game window gets.
pub fn edit_items(win_w: u32, win_h: u32) -> Vec<EditItem> {
    let pad = pad_of(win_w, win_h);
    let gap = pad * 2;
    let screen = Rect::new(0, 0, win_w, win_h);
    let gauges = stats_block_box(screen);
    let left = (gauges.x + gauges.w) + gap;
    let right = minimap_panel_builtin(win_w, win_h).x.saturating_sub(gap).max(left + 6);
    let avail = right - left;
    let cell_w = |cols: u32| ((avail + gap) / cols).saturating_sub(gap);
    // One row of six where the band can hold six tappable buttons, two rows of three
    // where it cannot (small windows). Never wider than the band.
    //
    // ★ Eight for one day, 2026-09-10: HIDE / SHOW joined the row that morning and
    // GAME / MENU left it that afternoon. The ROW COUNT is what matters and it is
    // unchanged either way (6/3 and 8/4 both mean "one row or two"), which is what keeps
    // this block's height — and therefore its footprint in `placement::obstacles` — where
    // it has been since 2026-08-03.
    let cols: u32 = if cell_w(6) >= 56 { 6 } else { 3 };
    let w = cell_w(cols).clamp(40, 170);
    let h = (win_h / 18).clamp(26, 64);
    let span = w * cols + gap * (cols - 1);
    let x0 = (left + avail.saturating_sub(span) / 2).min(win_w.saturating_sub(span));
    let y0 = pad * 2;
    let cell = |i: u32| Rect::new(x0 + (i % cols) * (w + gap), y0 + (i / cols) * (h + gap), w, h);
    vec![
        EditItem {
            rect: cell(0),
            action: EditAction::Flip,
        },
        // ★ HIDE / SHOW after FLIP, not at the end: RESET / CANCEL / SAVE keep their
        // order and stay at the right-hand end where the player's thumb already looks
        // for them, and CANCEL keeps its own colour next to SAVE.
        EditItem {
            rect: cell(1),
            action: EditAction::HidePlate,
        },
        EditItem {
            rect: cell(2),
            action: EditAction::ShowPlates,
        },
        EditItem {
            rect: cell(3),
            action: EditAction::Reset,
        },
        EditItem {
            rect: cell(4),
            action: EditAction::Cancel,
        },
        EditItem {
            rect: cell(5),
            action: EditAction::Save,
        },
    ]
}

pub fn edit_hit(items: &[EditItem], x: f64, y: f64) -> Option<EditAction> {
    items.iter().find(|i| i.rect.contains(x, y)).map(|i| i.action)
}

/// Draw the editor: every control of `mode` as a labelled box, the selected one
/// highlighted, the game's own HUD widgets marked as no-go areas, and the chrome.
pub fn draw_layout_editor(buf: &mut [u32], win_w: u32, win_h: u32, mode: Mode, sel: Option<crate::placement::CtrlId>, items: &[EditItem]) {
    const SEL: u32 = 0xffe0_9a3c; // the action disc's gold — "this is the one"
    const BLOCKED: u32 = 0xffc0_3a3a;
    // Dim the live frame so the boxes read as an overlay, not as the game.
    for px in buf.iter_mut() {
        let v = *px;
        let r = ((v >> 16) & 0xff) * 45 / 100;
        let g = ((v >> 8) & 0xff) * 45 / 100;
        let b = (v & 0xff) * 45 / 100;
        *px = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }

    let placed = crate::placement::resolve(mode, win_w, win_h);
    // The two widgets the GAME draws itself. Shown so the refusal to place a control
    // there reads as a rule rather than as a bug.
    let map = placed
        .iter()
        .find(|(c, _)| *c == crate::placement::CtrlId::Minimap)
        .map(|(_, r)| *r)
        .unwrap_or_else(|| Rect::new(0, 0, 0, 0));
    for o in crate::placement::obstacles(mode, win_w, win_h, map) {
        blend_rect(buf, win_w, win_h, o.x, o.y, o.w, o.h, BLOCKED, 60);
        stroke(buf, win_w, win_h, (o.x, o.y, o.w, o.h), BLOCKED);
    }

    for (id, r) in &placed {
        // A quickbar plate the player has switched off is skipped outright — the editor
        // must offer exactly what the HUD will draw. `app.rs` filters its hit-test with
        // the same predicate, so a hidden one cannot be selected blind either. (Until
        // 91차 this also hid whichever of stick / D-pad the live touch type did not use;
        // they are one control now, shown under both types.)
        if !crate::placement::shown(mode, *id) {
            continue;
        }
        let on = sel == Some(*id);
        let colour = if on { SEL } else { BORDER };
        // ★ Draw each control as the SHAPE IT WILL BE (player request 2026-07-31): every
        // key is a disc in play (`Face::Round`/`Action`), and only the minimap is a
        // square panel. Outlining a disc as a box made the editor look like it was
        // arranging something else.
        if id.is_round() {
            let rad = f64::from(r.w.min(r.h)) / 2.0;
            let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
            let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
            blend_disc_soft(
                buf,
                win_w,
                win_h,
                cx,
                cy,
                rad,
                if on { SEL } else { PANEL },
                if on { 70 } else { 120 },
                (rad * 0.08).max(1.0),
            );
            blend_ring_soft(buf, win_w, win_h, cx, cy, rad - 1.0, 2.0, colour, 255);
            if on {
                // Inner ring too: unmistakable at a glance on a busy field, where a
                // one-pixel colour change is not.
                blend_ring_soft(buf, win_w, win_h, cx, cy, rad * 0.8, (rad / 16.0).max(1.2), colour, 200);
            }
        } else {
            blend_rect(
                buf,
                win_w,
                win_h,
                r.x,
                r.y,
                r.w,
                r.h,
                if on { SEL } else { PANEL },
                if on { 70 } else { 120 },
            );
            stroke(buf, win_w, win_h, (r.x, r.y, r.w, r.h), colour);
            if on {
                let i = (r.w.min(r.h) / 12).max(2);
                stroke(
                    buf,
                    win_w,
                    win_h,
                    (r.x + i, r.y + i, r.w.saturating_sub(i * 2), r.h.saturating_sub(i * 2)),
                    colour,
                );
            }
        }
        // ★★ 91차: the STICK box shows everything the one knob governs — the four
        // hexagon arrows it is cut into (menu / choice, and the field under 십자키)
        // and, under either stick mode, the stick's base: the circle through the four
        // hexagons' centres (`stick_base_radius`). Without this a stick player pinching
        // the box could not see the arrows they were sizing, which is the report that
        // merged the two controls in the first place.
        if *id == crate::placement::CtrlId::Stick {
            if crate::stick_mode().stick_on_field() {
                let rad = stick_base_radius(r);
                let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
                let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
                blend_disc(buf, win_w, win_h, cx, cy, rad, 0xff1a_2436, 50);
                blend_ring_soft(buf, win_w, win_h, cx, cy, rad - 1.5, 3.0, colour, 200);
            }
            for arm in dpad_arms(*r) {
                draw_hex_face(buf, win_w, win_h, arm.draw, arm.dir, BTN_DPAD, colour, 70, 150);
            }
        }
        let label = id.label();
        // ★ The STICK's caption lives in the DEAD CENTRE between the four arrows, not
        // across the whole square: fitted to a 624 px box the letters would be 85 px
        // tall and bury the very arrows and disc the box is previewing.
        let lr = if *id == crate::placement::CtrlId::Stick {
            // The hole between the four inner tips: `50 - inset - ARM_LEN` from the
            // centre each way (~0.14 of the side).
            let s = r.w.min(r.h);
            let hole = (s.saturating_sub(s / 50 * 2 + s * ARM_LEN_PCT / 50)).max(1);
            Rect::new(r.x + (r.w - hole.min(r.w)) / 2, r.y + (r.h - hole.min(r.h)) / 2, hole, hole)
        } else {
            *r
        };
        if on {
            // Name over live size percent, both fitted — a 98 px plate cannot hold
            // two lines placed by eye.
            let base = crate::placement::builtin(*id, mode, win_w, win_h);
            let pct = crate::placement::Place::from_rect(*r, base, win_w, win_h).pct;
            // Every control shows a live percent since 2026-08-16 — the stick used to
            // say "MOVE" here because a pinch could not change it.
            let txt = format!("{pct}%");
            let (s, y1, y2) = two_line(lr, label.len().max(txt.len()) as u32 + 1);
            draw_centered_text_in(buf, win_w, win_h, lr.x, y1, lr.w, &label, LABEL, s);
            draw_centered_text_in(buf, win_w, win_h, lr.x, y2, lr.w, &txt, SEL, s);
        } else {
            let s = fit_scale(lr, label.len() as u32 + 1);
            draw_centered_text_in(buf, win_w, win_h, lr.x, lr.y + lr.h.saturating_sub(7 * s) / 2, lr.w, &label, LABEL, s);
        }
    }

    for item in items {
        let r = item.rect;
        // ★ HIDE / SHOW say on their face whether they can do anything right now — the
        // settings panel's own rule for a row that cannot be pressed ("a full-width
        // button that silently no-ops reads as a broken app"). HIDE needs a selected
        // plate that `quickpage::hide_plate` will accept (i.e. not the last visible
        // one); SHOW needs something to restore.
        let dim = match item.action {
            EditAction::HidePlate => !matches!(
                sel,
                Some(crate::placement::CtrlId::Quick(i))
                    if crate::quickpage::hide_plate(crate::quick_hidden(), i).is_some()
            ),
            EditAction::ShowPlates => crate::quick_hidden() == crate::quickpage::SHOW_ALL,
            _ => false,
        };
        // ★ No "you are on this page" colour any more: nothing in this row is a MODE
        // TAB since 2026-09-10, so no button can be the active one. Every caption is a
        // verb, and the only state a face shows is "this one cannot do anything now".
        let face = match item.action {
            _ if dim => PANEL,
            // Leaving without keeping the changes is the one destructive exit; give it
            // its own colour so it is not tapped by mistake next to SAVE.
            EditAction::Cancel | EditAction::Reset => 0xff3a_2430,
            _ => BTN,
        };
        draw_button_face(buf, win_w, win_h, r, face);
        // Single line: the block is compact now, and every caption is one short word.
        let label = match item.action {
            EditAction::Flip => "FLIP",
            // HIDE = 선택한 퀵슬롯 숨김, SHOW = 숨긴 것 전부 복원. ASCII like every
            // other caption here: this row is drawn with `draw_centered_text`, a 5x7
            // ASCII bitmap font, so Korean would come out as blank boxes.
            EditAction::HidePlate => "HIDE",
            EditAction::ShowPlates => "SHOW",
            EditAction::Reset => "RESET",
            EditAction::Cancel => "CANCEL",
            EditAction::Save => "SAVE",
        };
        let s = fit_scale(r, label.len() as u32 + 1);
        let ink = if dim { BORDER } else { LABEL };
        draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h.saturating_sub(7 * s) / 2, r.w, label, ink, s);
    }

    // One-line instructions, just BELOW the chrome row (which now sits at the top —
    // above it there is nothing but the screen edge). ★ `=` is NOT in the 5x7 font
    // (device-caught: it rendered as blank space, so the line read "DRAG  MOVE  2
    // FINGERS  SIZE"). The font has `- + / . %` + letters. Held to the chrome's OWN
    // span, not the window's: centred on the window it ran out under the ring plates
    // and read as scribbled over them (device-caught).
    let pad = pad_of(win_w, win_h);
    let (hx, hw) = match (items.iter().map(|i| i.rect.x).min(), items.iter().map(|i| i.rect.x + i.rect.w).max()) {
        (Some(a), Some(b)) => (a, b.saturating_sub(a)),
        _ => (0, win_w),
    };
    let hint = "TAP - DRAG - PINCH";
    let s = ((win_h / 90).clamp(2, 6)).min((hw * 96 / 100 / (hint.len() as u32 * 6)).max(1));
    let hint_y = items
        .iter()
        .map(|i| i.rect.y + i.rect.h)
        .max()
        .map(|b| b + pad * 2)
        .unwrap_or_else(|| win_h.saturating_sub(pad * 3 + 7 * s));
    draw_centered_text_in(buf, win_w, win_h, hx, hint_y, hw, hint, LABEL, s);
}

// --------------------------------------------------------------------------
// Drawing
// --------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn draw_controls(buf: &mut [u32], win_w: u32, win_h: u32, layout: &Layout, pressed: &HashSet<KeyCode>, joy: Option<JoyView>) {
    for b in &layout.buttons {
        let base = if pressed.contains(&b.key) {
            BTN_PRESSED
        } else if b.dpad {
            BTN_DPAD
        } else {
            BTN
        };
        match b.face {
            // The big ACTION disc keeps its inner gold ring: it is the one key that
            // should read as "the" button. Every other key gets the standard face.
            Face::Action => {
                let (cx, cy, rad) = draw_round_face(buf, win_w, win_h, b.rect, base, 0xffe0_9a3c, 95, 180);
                blend_ring_soft(buf, win_w, win_h, cx, cy, rad * 0.72, (rad / 16.0).max(1.4), 0xffe0_9a3c, 140);
            }
            Face::Round => {
                draw_round_face(buf, win_w, win_h, b.rect, base, BORDER, 92, 165);
            }
            Face::Solid => {
                let rad = f64::from(b.rect.w.min(b.rect.h)) / 2.0;
                let cx = f64::from(b.rect.x) + f64::from(b.rect.w) / 2.0;
                let cy = f64::from(b.rect.y) + f64::from(b.rect.h) / 2.0;
                fill_disc(buf, win_w, win_h, cx, cy, rad, base);
                blend_ring_soft(buf, win_w, win_h, cx, cy, rad - 1.0, 2.0, BORDER, 255);
            }
            // The INK rect, not the button's — the arrows are hit-tested over a wider
            // band than they are drawn in (see `dpad_arms`).
            Face::Hexagon(dir, ink) => {
                draw_hex_face(buf, win_w, win_h, ink, dir, base, BORDER, 92, 165);
                draw_hex_glyph(buf, win_w, win_h, ink, dir, LABEL, 215);
            }
            Face::None => {}
        }
        if b.face == Face::None {
            continue; // the widget paints its own face and label (minimap panel)
        }
        if matches!(b.face, Face::Hexagon(..)) {
            continue; // the face painted its own glyph
        }
        // A round face has to size its label against the circle's INSCRIBED SQUARE,
        // not the button rect: "CLR" sized to the full rect overhangs the disc on
        // both sides (device-confirmed 2026-07-30).
        let lbl = label_box(b.rect);
        match b.label {
            Label::Text(t) => draw_centered_text(buf, win_w, win_h, lbl, t, LABEL),
            Label::Arrow(dir) => {
                let s = (b.rect.w.min(b.rect.h) / 4).max(6);
                draw_arrow(buf, win_w, win_h, b.rect.x + b.rect.w / 2, b.rect.y + b.rect.h / 2, s, dir, LABEL);
            }
        }
    }
    if let Some(zone) = layout.joystick {
        draw_joystick(buf, win_w, win_h, &zone, joy);
    }
    draw_gear(buf, win_w, win_h, layout.gear);
}

/// The STICK's four arrows on their own, for drawing OVER a host dialog.
///
/// ★★ 2026-09-15, user: *"가방창이 좌측 화살표위로 올라와있는데 화살표가 더 위여야돼.
/// 가방창열렸을때 화살표 동작해야해"*. The box windows are the first host dialog the player
/// drives with the arrows (a long list needs them), so the arrows have to be both visible
/// and live while one is up — `App` routes the taps, this draws them last.
///
/// ⛔ Hexagon faces ONLY. Everything else in `layout.buttons` — the plates, OK, CLR, the
/// gear — stays under the window where it belongs; drawing the whole HUD on top would
/// bury the list it is meant to sit beside.
pub fn draw_direction_controls(buf: &mut [u32], win_w: u32, win_h: u32, layout: &Layout, pressed: &HashSet<KeyCode>) {
    for b in &layout.buttons {
        let Face::Hexagon(dir, ink) = b.face else { continue };
        let base = if pressed.contains(&b.key) { BTN_PRESSED } else { BTN_DPAD };
        draw_hex_face(buf, win_w, win_h, ink, dir, base, BORDER, 92, 165);
        draw_hex_glyph(buf, win_w, win_h, ink, dir, LABEL, 215);
    }
}

fn draw_joystick(buf: &mut [u32], win_w: u32, win_h: u32, zone: &Rect, joy: Option<JoyView>) {
    // Immersive (reference-style): the stick has a PERSISTENT home bottom-left so
    // the HUD reads like the reference screenshot; a drag still spawns the live
    // stick wherever the finger lands.
    if crate::immersive_active() {
        // ★ 91차-B: the base is the STICK seat's inscribed circle — the whole arrows'
        // square — and the LIVE stick spawns under the finger at that same size. The
        // walk-start distance is NOT this radius (`joy_engage_radius`), so the bigger
        // base does not bring a dead feel with it.
        //
        // The IDLE stick is only a hint about where to put your thumb, so it is
        // drawn far fainter than the live one (player request 2026-07-30 — the old
        // idle placeholder read as a real control). While a finger is down the
        // stick brightens to its previous strength so the deflection stays legible.
        let (hx, hy, hr) = joy_home(win_w, win_h);
        let live = joy.is_some();
        // Under 자유 스틱 the idle stick is only a HINT about where to put your thumb —
        // a touch anywhere walks — so it is deliberately faint (player request
        // 2026-07-30: the old placeholder read as a real control). Under 고정 스틱
        // (2026-09-22) it IS the control: the only place a walk can start, so it is
        // drawn at the strength of the arrows it replaces (`draw_hex_face` 70/150) —
        // a faint hint there would be an invisible control. Never reached under 십자키
        // (the layout publishes no zone), nor in a cutscene (no zone either).
        let pinned = crate::stick_mode().stick_is_pinned();
        let (base_a, ring_a, thumb_a) = if live {
            (55, 90, 105)
        } else if pinned {
            (40, 90, 80)
        } else {
            (18, 34, 40)
        };
        // 고정 스틱 never moves the base (`App::joy_start` centres the drag on the
        // seat), so `v.center` and `(hx, hy)` are the same point there.
        let (cx, cy) = joy.map_or((hx, hy), |v| v.center);
        blend_disc(buf, win_w, win_h, cx, cy, hr, 0xff1a_2436, base_a);
        blend_ring(buf, win_w, win_h, cx, cy, hr, BORDER, ring_a);
        // The thumb is pinned INSIDE the base by `App` (`joy_radius`), so drawn at its
        // own radius it never crosses the ring.
        let (tx, ty) = joy.map_or((hx, hy), |v| v.thumb);
        blend_disc(buf, win_w, win_h, tx, ty, hr * JOY_THUMB, 0xff9f_b6d4, thumb_a);
        return;
    }
    // On-demand floating stick: nothing is drawn until a finger is down, then the
    // base renders centred at the touch point (so "first press = centre", with no
    // fixed idle base that reads as a stick pinned to one spot).
    let Some(view) = joy else {
        return;
    };
    let r = joy_radius(zone, win_w, win_h);
    let (cx, cy) = view.center;
    // Base disc (opaque — softbuffer does not alpha-blend) + rim.
    fill_disc(buf, win_w, win_h, cx, cy, r, 0xff26_406a);
    ring(buf, win_w, win_h, cx, cy, r, BORDER);
    // Thumb at the finger.
    let (tx, ty) = view.thumb;
    let tr = r * 0.42;
    fill_disc(buf, win_w, win_h, tx, ty, tr, BTN_PRESSED);
    ring(buf, win_w, win_h, tx, ty, tr, BORDER);
}

fn fill_disc(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, color: u32) {
    if r <= 0.0 || (color >> 24) == 0 {
        return;
    }
    let r2 = r * r;
    let x0 = (cx - r).max(0.0) as u32;
    let x1 = ((cx + r) as u32).min(win_w.saturating_sub(1));
    let y0 = (cy - r).max(0.0) as u32;
    let y1 = ((cy + r) as u32).min(win_h.saturating_sub(1));
    for y in y0..=y1 {
        let dy = y as f64 + 0.5 - cy;
        for x in x0..=x1 {
            let dx = x as f64 + 0.5 - cx;
            if dx * dx + dy * dy <= r2 {
                buf[(y * win_w + x) as usize] = color;
            }
        }
    }
}

fn ring(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, color: u32) {
    if r <= 1.0 {
        return;
    }
    let n = ((2.0 * std::f64::consts::PI * r) as u32).max(24);
    let thick = (r / 22.0).max(1.5);
    for i in 0..n {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        let (s, c) = a.sin_cos();
        let mut t = -thick;
        while t <= thick {
            let px = cx + c * (r + t);
            let py = cy + s * (r + t);
            if px >= 0.0 && py >= 0.0 && (px as u32) < win_w && (py as u32) < win_h {
                buf[(py as u32 * win_w + px as u32) as usize] = color;
            }
            t += 1.0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_settings(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    items: &[SettingsItem],
    landscape_active: bool,
    joystick_active: bool,
    immersive_pref: bool,
    base_h: u32,
    // 발열 절감 preference (57차) — drives the HEAT SAVER row's face + state line.
    heat_saver: bool,
) {
    // Dim everything behind the modal (read + darken each pixel).
    for px in buf.iter_mut() {
        let v = *px;
        let r = ((v >> 16) & 0xff) * 4 / 10;
        let g = ((v >> 8) & 0xff) * 4 / 10;
        let b = (v & 0xff) * 4 / 10;
        *px = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }

    // ── the GAME'S own window chrome (60차) ──────────────────────────────────
    //
    // Player: "상단 톱니바퀴로 나오는 setting 창을 좀더 영웅서기 스럽게 디자인을
    // 바꿀수있어? 폰트도 영웅서기에서 사용하는 폰트, 문구도 한글로 하는거지".
    //
    // So the panel is built the way the game builds ITS windows — a parchment body
    // inside a near-black frame with a cream highlight line, a brown title band, and
    // rows shaped like the 이름/효과 fields — and every label goes through
    // `crate::gamefont`, which IS the typeface the game loads (`res/font/korb`).
    // Nothing here is a lookalike: the colours are read off the game's own windows
    // and the glyphs are the game's own bitmaps.
    let p = settings_panel(win_w, win_h);
    let edge = (p.w / 90).clamp(2, 5);
    fill(buf, win_w, win_h, p.x, p.y, p.w, p.h, G_FRAME);
    fill(
        buf,
        win_w,
        win_h,
        p.x + edge,
        p.y + edge,
        p.w.saturating_sub(edge * 2),
        p.h.saturating_sub(edge * 2),
        G_PANEL,
    );
    stroke(
        buf,
        win_w,
        win_h,
        (p.x + edge, p.y + edge, p.w.saturating_sub(edge * 2), p.h.saturating_sub(edge * 2)),
        G_EDGE,
    );
    // The game's window body is not flat — it is a checker, which is what makes its
    // panels read as cloth rather than as a rectangle. Same trick here. ★ The pitch
    // and the contrast were both raised on the player's own look ("체커 질감도 더
    // 줘야할거같아"): at device resolution a 2-px cell with a 5 % lift is invisible.
    checker(
        buf,
        win_w,
        win_h,
        Rect::new(p.x + edge, p.y + edge, p.w.saturating_sub(edge * 2), p.h.saturating_sub(edge * 2)),
        G_PANEL_ALT,
    );

    // Title band: the game's dark-brown scroll bar with cream text in it.
    let head = settings_head(p);
    let band_h = head.saturating_sub(edge * 2).max(1);
    fill(
        buf,
        win_w,
        win_h,
        p.x + edge * 2,
        p.y + edge * 2,
        p.w.saturating_sub(edge * 4),
        band_h,
        G_ROW,
    );
    stroke(
        buf,
        win_w,
        win_h,
        (p.x + edge * 2, p.y + edge * 2, p.w.saturating_sub(edge * 4), band_h),
        G_FRAME,
    );
    let ts = game_fit_scale("설정", p.w / 3, band_h * 70 / 100);
    draw_game_text_centered(
        buf,
        win_w,
        win_h,
        p.x,
        p.y + edge * 2 + band_h.saturating_sub(crate::gamefont::KOR_H as u32 * ts) / 2,
        p.w,
        ts,
        "설정",
        G_TITLE,
    );

    for item in items {
        let r = item.rect;
        // Every row is the same plate; only the words and the state differ. Keeping
        // one drawing path is what stops the rows drifting apart visually as they are
        // added — this panel has grown from three rows to seven.
        let (title, sub, state) = match item.kind {
            SKind::Close => {
                draw_row(buf, win_w, win_h, r, "닫기", "", RowState::Normal);
                continue;
            }
            SKind::EditLayout => {
                let custom = crate::placement::is_customised();
                let st = if !item.enabled {
                    RowState::Disabled
                } else if custom {
                    RowState::Active
                } else {
                    RowState::Normal
                };
                // Unavailable outside landscape full-screen — say so on the row
                // itself, or it reads as a dead control.
                let sub = if !item.enabled {
                    "가로 전용"
                } else if custom {
                    "사용자 배치"
                } else {
                    "편집"
                };
                ("화면 배치", sub, st)
            }
            // Diagnostics — and, since 60차, the way to the save vault's restore.
            SKind::Diag => ("진단 정보", "세이브 복원", RowState::Normal),
            // ★ 2026-08-27 — the effect bus's own trim, because one global number cannot
            // be right on both an S25 Ultra and a Fold 6. `기본` is what every build
            // before this one sounded like. See `audio_mix::SFX_LEVELS`.
            SKind::SfxLevel => {
                let lv = crate::sfx_level();
                let st = if lv == crate::audio_mix::SFX_LEVEL_DEFAULT {
                    RowState::Normal
                } else {
                    RowState::Active
                };
                ("효과음 크기", crate::audio_mix::sfx_level_label(lv), st)
            }
            SKind::TouchType => {
                let m = crate::stick_mode();
                // Highlighted whenever it is not the shipped default: the panel's
                // standing job is to answer "have I changed anything?" at a glance.
                let st = if !item.enabled {
                    RowState::Disabled
                } else if m != crate::StickMode::Float {
                    RowState::Active
                } else {
                    RowState::Normal
                };
                let sub = if !item.enabled { "가로 전용" } else { m.label() };
                ("조작 방식", sub, st)
            }
            // 발열 절감 (57차). Highlighted while ON: the panel's standing job is to
            // answer "have I changed anything?" at a glance.
            SKind::HeatSaver => (
                "발열 절감",
                if heat_saver { "켬" } else { "끔" },
                if heat_saver { RowState::Active } else { RowState::Normal },
            ),
            // 게임패드 HUD (59차). AUTO is the default and is NOT highlighted even
            // though it may be showing the pad HUD right now — the highlight is about
            // the SETTING, not the current screen.
            SKind::PadHud => {
                let pref = crate::pad_hud_pref();
                let sub = match pref {
                    crate::padhud::PadHud::Auto => "자동",
                    crate::padhud::PadHud::On => "항상 켬",
                    crate::padhud::PadHud::Off => "끔",
                };
                let st = if pref == crate::padhud::PadHud::Auto {
                    RowState::Normal
                } else {
                    RowState::Active
                };
                ("패드 표시", sub, st)
            }
            // 패드 버튼 설정 (2026-10-09). Highlighted while the table differs from the
            // built-in layout — the panel's standing "have I changed anything?" answer.
            SKind::PadMap => {
                if crate::gamepad::remapped() {
                    ("패드 버튼 설정", "사용자 배치", RowState::Active)
                } else {
                    ("패드 버튼 설정", "기본", RowState::Normal)
                }
            }
            // Rows that were removed from the panel but whose actions are retained
            // (see `SettingsAction`). They never reach here; naming them keeps the
            // match exhaustive without a catch-all that would hide a new kind.
            SKind::Save => ("빠른 저장", "지금", RowState::Active),
            SKind::Portrait => ("세로", "", RowState::Normal),
            SKind::Landscape => ("가로", "", RowState::Normal),
            SKind::Dpad => ("십자키", "", RowState::Normal),
            SKind::Joystick => ("스틱", "", RowState::Normal),
            SKind::Immersive => ("전체 화면", if immersive_pref { "켬" } else { "끔" }, RowState::Normal),
            SKind::BaseH => ("화면 높이", "", RowState::Normal),
        };
        // Retained for the rows above that are no longer built into the panel.
        let _ = (landscape_active, joystick_active, base_h);
        draw_row(buf, win_w, win_h, r, title, sub, state);
    }
}

/// What a tap on the quantity dialog does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtyHit {
    /// Change the count by this much, clamped into 1..=`max`.
    Step(i32),
    /// Confirm. The caller latches the count and hands the guest an `OK`, which is
    /// what answers the game's own 예/아니오 underneath.
    Ok,
}

// ── the NATIVE window replica (2026-08-25, 70차) ─────────────────────────────
//
// The player rejected the first replica ("임의디자인이 아닌 기존의 클라이언트에서
// 사용하던 디자인 그대로 사용해야함") and attached screenshots of the game's own
// 개수를 설정하세요 window and its message box. Everything below is measured from
// those screenshots (2340x1080 = logical x4.5): the panel is 164 logical px wide,
// built as [2px outline][4px light ring][1px shine][body][dark strip][1px shine]
// [16px OK band][2px outline], with the light OK band running flush to the outline
// at the sides — the ring exists only around the body. Colours are the sampled
// dominants of each band. The spinner is a single ▲ / value box / ▼ column at the
// right edge, exactly where the game puts its own; the value is `%02d` in the
// game's ASCII font; the dark strip is where the game prints the price, and the
// callers put their own line there (최대 N …) right-aligned the same way.
//
// The ±10 coarse buttons of the first replica are GONE from the face — the native
// window has no such buttons. What replaces them is what the game itself does:
// ▲/▼ AUTO-REPEAT while held (`app.rs::qty_hold`), and the LEFT/RIGHT keys still
// step by ten invisibly (68차-B), which no pixel needs to advertise.

/// Native-window palette, sampled from the player's own device screenshots
/// (JPEG dominants; the guest renders RGB565 so ±1 LSB is noise).
const N_OUTLINE: u32 = 0xff21_0d0f;
const N_RING: u32 = 0xffbd_b2ae;
const N_SHINE: u32 = 0xfff6_ebe5;
const N_BODY: u32 = 0xff7b_5852;
const N_DARK: u32 = 0xff4b_3122;
const N_TEXT: u32 = 0xffff_fefd;

// ── the font's two vertical extents, and why the replica has to know them ────
//
// Every cell of the game's font is `KOR_H` = 11 rows, but the two faces fill it
// very differently (measured off `gamefont_data`, not guessed): a Hangul syllable
// inks rows 0..11, while the ASCII face inks rows **0..7** for capitals and digits
// and reserves 7..11 for the four descenders (g p q j). So centring a number on
// `KOR_H` puts it against the TOP of its box with four blank rows underneath — which
// is exactly what the player saw twice over (2026-08-26):
//   "숫자가 박스 정중앙에 없고 살짝 박스 위쪽으로 튀어나와서 … 살짝 아래로"
//   "최대 99라고 적혀있는부분에서 숫자 99도 살짝 아래로 … 영역에서 살짝 튀어나와있어"
/// Rows a digit or capital actually inks, for centring a NUMBER in a box.
const N_CAP_H: f64 = 7.0;
/// How far ASCII drops to sit on a Hangul syllable's baseline, in font px.
///
/// ⛔ For the replica's OWN strings only. A descender would be pushed past row 11 and
/// clipped; every line drawn this way is digits, capitals and Hangul.
const N_ASCII_DROP: f64 = 3.0;

/// [`draw_game_text_frac`] with the ASCII dropped onto the Hangul baseline, so a
/// mixed line (`최대 99`) reads as one line instead of numbers riding high.
fn draw_native_text(buf: &mut [u32], win_w: u32, win_h: u32, x: f64, y: f64, s: f64, text: &str, color: u32) -> f64 {
    let mut pen = x;
    let mut b = [0u8; 4];
    for ch in text.chars() {
        let dy = if ch.is_ascii() { N_ASCII_DROP * s } else { 0.0 };
        pen = draw_game_text_frac(buf, win_w, win_h, pen, y + dy, s, ch.encode_utf8(&mut b), color);
    }
    pen
}

/// Logical geometry shared by both native replicas (all in the game's own px).
const NP_W: f64 = 164.0; // panel width
const NP_OUTLINE: f64 = 2.0;
const NP_RING: f64 = 4.0;
const NP_OK_H: f64 = 16.0;
/// The whole panel sits 5 logical px above centre, like the game's own modals.
const NP_RAISE: f64 = 5.0;

/// One native panel, computed in window px from the game's logical scale.
struct NativePanel {
    panel: Rect,
    /// The `N_BODY` face (inside outline + ring + shine).
    body: Rect,
    /// The dark full-width strip under the body; zero-height when `strip_l == 0`.
    strip: Rect,
    /// The light OK band (flush to the outline).
    ok: Rect,
    /// Window px per logical px.
    s: f64,
}

fn native_panel(win_w: u32, win_h: u32, body_l: f64, strip_l: f64) -> NativePanel {
    native_panel_ex(win_w, win_h, body_l, strip_l, NP_OK_H)
}

/// [`native_panel`] with the OK band's height as a parameter.
///
/// * 71차-N: `ok_l = 0.0` builds the same box with NO OK band - the shape a box the
/// player cannot answer has to have. `draw_native_panel` skips the band when its
/// height comes out zero, so one number covers both the geometry and the paint.
fn native_panel_ex(win_w: u32, win_h: u32, body_l: f64, strip_l: f64, ok_l: f64) -> NativePanel {
    let (_, lh) = crate::logical();
    let s = f64::from(win_h) / f64::from(lh.max(1));
    // [outline][ring][shine] body [strip] [shine] [ok band][outline]
    let total_l = NP_OUTLINE + NP_RING + 1.0 + body_l + strip_l + 1.0 + ok_l + NP_OUTLINE;
    let w = (NP_W * s).round() as u32;
    let h = (total_l * s).round() as u32;
    let x0 = (win_w.saturating_sub(w)) / 2;
    let y0_l = ((f64::from(lh) - total_l) / 2.0 - NP_RAISE).max(0.0);
    let y0 = (y0_l * s).round() as u32;
    // Band edges from the panel top, each rounded independently so a fractional
    // scale cannot accumulate into a visibly uneven frame.
    let e = |l: f64| (y0_l * s + l * s).round() as u32;
    let body_y = e(NP_OUTLINE + NP_RING + 1.0);
    let strip_y = e(NP_OUTLINE + NP_RING + 1.0 + body_l);
    let ok_y = e(total_l - NP_OUTLINE - ok_l);
    let inset = (NP_OUTLINE + NP_RING) * s;
    let bx = (f64::from(x0) + inset).round() as u32;
    let bw = ((NP_W - (NP_OUTLINE + NP_RING) * 2.0) * s).round() as u32;
    NativePanel {
        panel: Rect::new(x0, y0, w, h),
        body: Rect::new(bx, body_y, bw, strip_y.saturating_sub(body_y)),
        strip: Rect::new(bx, strip_y, bw, e(NP_OUTLINE + NP_RING + 1.0 + body_l + strip_l).saturating_sub(strip_y)),
        ok: Rect::new(
            (f64::from(x0) + NP_OUTLINE * s).round() as u32,
            ok_y,
            ((NP_W - NP_OUTLINE * 2.0) * s).round() as u32,
            e(total_l - NP_OUTLINE).saturating_sub(ok_y),
        ),
        s,
    }
}

/// Paint the frame: outline, ring-plus-OK-band (one light fill — the band is flush
/// to the outline), the shine halo, then the body and strip faces.
fn draw_native_panel(buf: &mut [u32], win_w: u32, win_h: u32, np: &NativePanel) {
    let p = np.panel;
    let o = (NP_OUTLINE * np.s).round() as u32;
    fill(buf, win_w, win_h, p.x, p.y, p.w, p.h, N_OUTLINE);
    fill(
        buf,
        win_w,
        win_h,
        p.x + o,
        p.y + o,
        p.w.saturating_sub(o * 2),
        p.h.saturating_sub(o * 2),
        N_RING,
    );
    // The shine: a 1-logical-px halo around the body+strip column, which is exactly
    // the bright seam the screenshots show above the body and above the OK band.
    let sh = np.s.round().max(1.0) as u32;
    let top = np.body.y.saturating_sub(sh);
    let bot = np.strip.y + np.strip.h + sh;
    fill(
        buf,
        win_w,
        win_h,
        np.body.x.saturating_sub(sh),
        top,
        np.body.w + sh * 2,
        bot.saturating_sub(top),
        N_SHINE,
    );
    fill(buf, win_w, win_h, np.body.x, np.body.y, np.body.w, np.body.h, N_BODY);
    if np.strip.h > 0 {
        fill(buf, win_w, win_h, np.strip.x, np.strip.y, np.strip.w, np.strip.h, N_DARK);
    }
    // * 71차-N: a box the player cannot answer has NO OK band (`native_panel_ex` with
    // `ok_l = 0.0`). Drawing one invites the tap the modal then swallows - which is
    // exactly how the 저장하고 있습니다 box came to read as broken rather than busy.
    if np.ok.h == 0 {
        return;
    }
    fill(buf, win_w, win_h, np.ok.x, np.ok.y, np.ok.w, np.ok.h, N_RING);
    // "OK" — white with the game's dark outline, centred on the band.
    let s = np.s;
    let tw = game_text_w_frac("OK", s);
    let tx = f64::from(np.ok.x) + (f64::from(np.ok.w) - tw) / 2.0;
    // Centred on the CAP height, not the cell — see `N_CAP_H`.
    let ty = f64::from(np.ok.y) + (f64::from(np.ok.h) - N_CAP_H * s) / 2.0;
    let d = s.round().max(1.0);
    for (ox, oy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)] {
        draw_game_text_frac(buf, win_w, win_h, tx + ox, ty + oy, s, "OK", N_DARK);
    }
    draw_game_text_frac(buf, win_w, win_h, tx, ty, s, "OK", N_TEXT);
}

/// The dialog's boxes, in the order [`qty_hit`] tests them.
pub struct QtyRects {
    pub panel: Rect,
    /// `+1`, around the ▲ above the value box.
    pub up: Rect,
    pub value: Rect,
    /// `-1`, around the ▼ below it.
    pub down: Rect,
    pub ok: Rect,
}

/// Native measurements of the spinner column (logical px from the PANEL's origin):
/// ▲ centre x 144, value box x 133..157 y 40.5..50.5, ▼ below — all read off the
/// player's screenshot at x4.5.
const NQ_BODY_H: f64 = 53.0;
const NQ_STRIP_H: f64 = 11.0;
const NQ_COL_CX: f64 = 144.0;
const NQ_BOX_W: f64 = 24.0;
const NQ_BOX_H: f64 = 10.0;
const NQ_BOX_TOP: f64 = 40.5;
const NQ_TITLE_X: f64 = 31.0;
const NQ_TITLE_TOP: f64 = 18.0;
/// Half-height of a spinner arrow, in logical px.
const NQ_ARROW_R: f64 = 3.2;
/// Clear air between an arrow's tip and the value box (player request 2026-08-26:
/// "박스와 위아래 화살표사이 간격 조금만 더 벌려야하고"). The first cut left 0.3 px,
/// which reads as the arrows touching the frame.
const NQ_ARROW_GAP: f64 = 1.6;
/// Arrow centres, derived from the box so the two gaps stay equal if the box moves.
const NQ_ARROW_UP_CY: f64 = NQ_BOX_TOP - NQ_ARROW_R - NQ_ARROW_GAP;
const NQ_ARROW_DOWN_CY: f64 = NQ_BOX_TOP + NQ_BOX_H + NQ_ARROW_R + NQ_ARROW_GAP;

pub fn qty_rects(win_w: u32, win_h: u32) -> QtyRects {
    let np = native_panel(win_w, win_h, NQ_BODY_H, NQ_STRIP_H);
    let s = np.s;
    let px = |l: f64| (l * s).round() as u32;
    let bx = np.panel.x + px(NQ_COL_CX - NQ_BOX_W / 2.0);
    let by = np.panel.y + px(NQ_BOX_TOP);
    let bw = px(NQ_BOX_W);
    let bh = px(NQ_BOX_H);
    // The arrows' HIT boxes span the full spinner column above/below the value box,
    // wider than their ink — the same grown-target rule the D-pad uses.
    QtyRects {
        panel: np.panel,
        up: Rect::new(bx, np.body.y, bw, by.saturating_sub(np.body.y)),
        value: Rect::new(bx, by, bw, bh),
        down: Rect::new(bx, by + bh, bw, (np.body.y + np.body.h).saturating_sub(by + bh)),
        ok: np.ok,
    }
}

/// What a tap at `(px, py)` asks the dialog for.
///
/// ⛔ Returns `Some` for a tap anywhere on the PANEL, even one that hits no button: the
/// dialog is drawn over the game's own confirm box, so a tap that falls through would
/// reach the guest and answer 예/아니오 by itself. `Step(0)` is that "swallow it" answer
/// and the caller clamps it to a no-op.
pub fn qty_hit(win_w: u32, win_h: u32, px: f64, py: f64) -> Option<QtyHit> {
    let r = qty_rects(win_w, win_h);
    if r.ok.contains(px, py) {
        return Some(QtyHit::Ok);
    }
    for (box_, step) in [(&r.up, 1), (&r.down, -1)] {
        if box_.contains(px, py) {
            return Some(QtyHit::Step(step));
        }
    }
    r.panel.contains(px, py).then_some(QtyHit::Step(0))
}

/// Draw the dialog. `chosen` is the live count, `max` its ceiling, and `info` the
/// line for the dark strip — right-aligned white on dark, where the game's own
/// window prints the price.
pub fn draw_qty_dialog(buf: &mut [u32], win_w: u32, win_h: u32, title: &str, chosen: u8, max: u8, info: Option<&str>) {
    let np = native_panel(win_w, win_h, NQ_BODY_H, NQ_STRIP_H);
    draw_native_panel(buf, win_w, win_h, &np);
    let s = np.s;
    let px = |l: f64| l * s;
    let (p_x, p_y) = (f64::from(np.panel.x), f64::from(np.panel.y));

    draw_native_text(buf, win_w, win_h, p_x + px(NQ_TITLE_X), p_y + px(NQ_TITLE_TOP), s, title, N_TEXT);

    // The spinner column: ▲, the white-rimmed value box, ▼. The arrows dim at the
    // ends the way the game's own do.
    let r = qty_rects(win_w, win_h);
    let a = (px(NQ_ARROW_R)).round().max(2.0) as u32;
    let acx = np.panel.x + (px(NQ_COL_CX)).round() as u32;
    let up_ink = if chosen >= max { N_BODY } else { N_DARK };
    let down_ink = if chosen <= 1 { N_BODY } else { N_DARK };
    draw_arrow(
        buf,
        win_w,
        win_h,
        acx,
        np.panel.y + (px(NQ_ARROW_UP_CY)).round() as u32,
        a,
        Dir::Up,
        up_ink,
    );
    draw_arrow(
        buf,
        win_w,
        win_h,
        acx,
        np.panel.y + (px(NQ_ARROW_DOWN_CY)).round() as u32,
        a,
        Dir::Down,
        down_ink,
    );
    let vb = r.value;
    fill(buf, win_w, win_h, vb.x, vb.y, vb.w, vb.h, N_TEXT);
    let bi = s.round().max(1.0) as u32;
    fill(
        buf,
        win_w,
        win_h,
        vb.x + bi,
        vb.y + bi,
        vb.w.saturating_sub(bi * 2),
        vb.h.saturating_sub(bi * 2),
        N_DARK,
    );
    let digits = format!("{chosen:02}");
    let dw = game_text_w_frac(&digits, s);
    draw_game_text_frac(
        buf,
        win_w,
        win_h,
        f64::from(vb.x) + (f64::from(vb.w) - dw) / 2.0,
        // ★ On the CAP height (`N_CAP_H`), so the number sits in the middle of the box
        // instead of half a pixel above its top edge.
        f64::from(vb.y) + (f64::from(vb.h) - N_CAP_H * s) / 2.0,
        s,
        &digits,
        N_TEXT,
    );

    // The dark strip: the caller's line, right-aligned where the game prints the price.
    if let Some(info) = info
        && !info.is_empty()
        && np.strip.h > 0
    {
        let tw = game_text_w_frac(info, s);
        let tx = f64::from(np.strip.x) + f64::from(np.strip.w) - tw - px(6.0);
        let ty = f64::from(np.strip.y) + (f64::from(np.strip.h) - crate::gamefont::KOR_H as f64 * s) / 2.0;
        // `최대 99` is MIXED, so the digits ride three rows high unless they are dropped
        // onto the Hangul baseline — see `N_ASCII_DROP`.
        draw_native_text(buf, win_w, win_h, tx, ty, s, info, N_TEXT);
    }
}

// ── the native MESSAGE BOX replica (70차) ────────────────────────────────────
//
// Same panel family as the quantity window, without the dark strip: the game's
// "버릴수 없는 아이템입니다." box from the player's screenshot — body with the
// text at the top left, light OK band. Used for the in-place notices the game
// itself would show (e.g. picking a WORN item in the combine picker).

const NN_BODY_H: f64 = 57.0;
const NN_TEXT_X: f64 = 4.0;
const NN_TEXT_TOP: f64 = 7.0;
/// Line pitch of the game's own message text.
const NN_LINE_H: f64 = 13.0;

/// `true` = the OK band (or any key-equivalent area) was pressed; `false` = the tap
/// landed on the panel and must be swallowed. `None` = outside the panel.
pub fn notice_hit(win_w: u32, win_h: u32, px: f64, py: f64) -> Option<bool> {
    let np = native_panel(win_w, win_h, NN_BODY_H, 0.0);
    if np.ok.contains(px, py) {
        return Some(true);
    }
    np.panel.contains(px, py).then_some(false)
}

/// Draw the game-style message box with `text` ('\n' breaks lines).
/// The same box with NO OK button - for a wait the player cannot dismiss.
///
/// * 71차-N (player, 2026-08-27): "네트워크 접속 종료되고 저장중입니다 팝업에 OK버튼
/// 없애야해. 로딩이 끝나고 정상적으로 팝업이 닫혀야만 사용자가 뭔가 할수있어야해."
/// The save-lock box has always swallowed every key and every tap - that is its whole
/// job, holding the slot list still until the disconnect save lands - but it was drawn
/// with the same OK band as an answerable message, so it read as BROKEN rather than as
/// BUSY. Now it has no band at all and closes itself when the write settles.
pub fn draw_native_wait(buf: &mut [u32], win_w: u32, win_h: u32, text: &str) {
    let np = native_panel_ex(win_w, win_h, NN_BODY_H, 0.0, 0.0);
    draw_native_panel(buf, win_w, win_h, &np);
    let s = np.s;
    let mut y = f64::from(np.panel.y) + NN_TEXT_TOP * s + (NP_OUTLINE + NP_RING + 1.0) * s;
    for line in text.split('\n') {
        draw_game_text_frac(buf, win_w, win_h, f64::from(np.body.x) + NN_TEXT_X * s, y, s, line, N_TEXT);
        y += NN_LINE_H * s;
    }
}

pub fn draw_native_notice(buf: &mut [u32], win_w: u32, win_h: u32, text: &str) {
    let np = native_panel(win_w, win_h, NN_BODY_H, 0.0);
    draw_native_panel(buf, win_w, win_h, &np);
    let s = np.s;
    let mut y = f64::from(np.panel.y) + NN_TEXT_TOP * s + (NP_OUTLINE + NP_RING + 1.0) * s;
    for line in text.split('\n') {
        draw_game_text_frac(buf, win_w, win_h, f64::from(np.body.x) + NN_TEXT_X * s, y, s, line, N_TEXT);
        y += NN_LINE_H * s;
    }
}

// ── ★ 143차: the native YES / NO box ──────────────────────────────────────────
//
// The game's own confirm (「저장하시겠습니까?」, system → 세이브) is the message box above
// plus two cells under the question: the SELECTED cell is filled `N_DARK` with white
// lettering, the other has no box at all and dim lettering (`#a5827b`, the 80차-G
// measurement). Measured off a probe frame (2026-09-30, 496x240): cells 52 x 14 logical,
// 8 apart, centred in the body, their tops 28 below the body's — and the light OK band
// underneath, as on every native box. Used for 「하늘성에서 나가시겠습니까?」.

const NC_CELL_W: f64 = 52.0;
const NC_CELL_H: f64 = 14.0;
const NC_CELL_GAP: f64 = 8.0;
const NC_CELL_TOP: f64 = 28.0;
const NC_DIM: u32 = 0xffa5_827b;

/// Where the two answers are: `[예, 아니오]`, in window px.
fn confirm_cells(np: &NativePanel) -> [Rect; 2] {
    let s = np.s;
    let body_cx = f64::from(np.body.x) + f64::from(np.body.w) / 2.0;
    let y = (f64::from(np.body.y) + NC_CELL_TOP * s).round() as u32;
    let w = (NC_CELL_W * s).round() as u32;
    let h = (NC_CELL_H * s).round() as u32;
    let x0 = (body_cx - (NC_CELL_GAP / 2.0 + NC_CELL_W) * s).round() as u32;
    let x1 = (body_cx + NC_CELL_GAP / 2.0 * s).round() as u32;
    [Rect::new(x0, y, w, h), Rect::new(x1, y, w, h)]
}

/// A tap on the yes/no box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfirmHit {
    /// Cell 0 = 예, 1 = 아니오.
    Cell(u8),
    /// The OK band: answer with the selected cell.
    Ok,
    /// Somewhere else on the panel: swallowed (it is a modal).
    Panel,
}

/// What a tap at `(x, y)` means to the yes/no box, `None` = outside the panel.
pub fn confirm_hit(win_w: u32, win_h: u32, x: f64, y: f64) -> Option<ConfirmHit> {
    let np = native_panel(win_w, win_h, NN_BODY_H, 0.0);
    if np.ok.contains(x, y) {
        return Some(ConfirmHit::Ok);
    }
    // The cells' hit boxes are a little taller than their faces — a thumb aimed at a
    // 14-logical-px word lands around it, and nothing else shares that band.
    let pad = (4.0 * np.s).round() as u32;
    for (i, c) in confirm_cells(&np).iter().enumerate() {
        let r = Rect::new(c.x.saturating_sub(pad), c.y.saturating_sub(pad), c.w + pad * 2, c.h + pad * 2);
        if r.contains(x, y) {
            return Some(ConfirmHit::Cell(i as u8));
        }
    }
    np.panel.contains(x, y).then_some(ConfirmHit::Panel)
}

/// Draw the game-style yes/no box: `text` on top ('\n' breaks lines), 예 / 아니오 under it
/// with `sel` (0 = 예) filled, the OK band at the bottom.
pub fn draw_native_confirm(buf: &mut [u32], win_w: u32, win_h: u32, text: &str, sel: u8) {
    let np = native_panel(win_w, win_h, NN_BODY_H, 0.0);
    draw_native_panel(buf, win_w, win_h, &np);
    let s = np.s;
    let mut y = f64::from(np.panel.y) + NN_TEXT_TOP * s + (NP_OUTLINE + NP_RING + 1.0) * s;
    for line in text.split('\n') {
        draw_game_text_frac(buf, win_w, win_h, f64::from(np.body.x) + NN_TEXT_X * s, y, s, line, N_TEXT);
        y += NN_LINE_H * s;
    }
    for (i, (c, word)) in confirm_cells(&np).iter().zip(["예", "아니오"]).enumerate() {
        let on = usize::from(sel) == i;
        if on {
            fill(buf, win_w, win_h, c.x, c.y, c.w, c.h, N_DARK);
        }
        let tw = game_text_w_frac(word, s);
        let tx = f64::from(c.x) + (f64::from(c.w) - tw) / 2.0;
        let ty = f64::from(c.y) + (f64::from(c.h) - crate::gamefont::KOR_H as f64 * s) / 2.0;
        draw_game_text_frac(buf, win_w, win_h, tx, ty, s, word, if on { N_TEXT } else { NC_DIM });
    }
}

// ── 제련 확률 바 (71차-I) ────────────────────────────────────────────────────
//
// Player, 2026-08-26: *"제련시 제련확률을 표시해주고싶어 … 제련석을 올릴때마다 확률도
// 마찬가지로 계산되어서 최신화 되어야해."*
//
// The smith's own window has nowhere to put this. Its interior was measured off the
// player's screenshot and every band is spoken for — title 0..15, item name 20..34,
// spinner 41..55, 제련석/비용 63..77, 이름/효과 85..129, the money strip 134..145 and
// the `OK : 제련하기` footer 145..160 — and the two that LOOK empty are not: the 효과
// box's second line is where a longer effect string wraps, and the money strip carries
// the player's gold, which is exactly what gates the attempt (`0x215cc` →
// `제련 비용이 부족합니다.`). So the bar goes UNDER the window, where the game draws
// nothing at all, in the game's own chrome so it still reads as part of it.
//
// ⛔ No right angles: the game never draws one (71차-G). Same 2-px 45° chamfer, same
// `#181818`-family outline and cream bevel as every box the game owns.

/// ★★ THE POSITION KNOB — the one line to edit to move the readout.
///
/// `(dx, dy)` is the bar's top-CENTRE in the GAME's own logical px, measured from the
/// top-left corner of the 176x160 menu panel ([`crate::menutouch::panel_origin`]).
/// Panel-relative and in logical px, so one pair of numbers means the same thing on
/// every device and at every logical resolution the app runs at.
///
/// ```text
///   ( 88,  164)   centred, 4 px under the panel        <- default
///   ( 88,  -21)   centred, 4 px above it
///   ( 88,  147)   over the game's own `OK : 제련하기` footer
///   (264,   84)   off to the right of the panel, vertically centred on it
/// ```
///
/// The width is not here because it is not a choice: the bar is exactly as wide as its
/// text, so it never looks padded and never truncates.
const REFINE_BAR: (i32, i32) = (88, 164);

/// Bar height in logical px: outline 1 + bevel 1 + pad 1 + [`crate::gamefont::KOR_H`]
/// 11 + pad 1 + bevel 1 + outline 1.
const RB_H: i32 = 17;
/// Logical px between a label and its number, and between one field and the next.
const RB_LABEL_GAP: usize = 3;
const RB_FIELD_GAP: usize = 11;
/// Logical px of body either side of the text.
const RB_PAD_X: f64 = 4.0;
/// The 45° corner cut, in logical px — the game's own (71차-G).
const RB_CUT: f64 = 2.0;

// Sampled from the player's own 제련 window (2026-08-26 screenshot), so the bar is the
// window's palette and not a new one: the title bar's brown, the 이름/효과 label gold,
// and the white its 제련석 / 비용 sub-rows print their numbers in.
const RB_BODY: u32 = 0xff31_1c10;
const RB_LABEL: u32 = 0xffd6_9631;
const RB_VALUE: u32 = 0xffff_ffff;
/// 성공 100 % — the game's own item-name cream (`#debe7b`, sampled from the 이름 field).
/// ⛔ Not a new colour: this bar invents none.
const RB_GOOD: u32 = 0xffde_be7b;
/// 파괴 at or above [`crate::refine::DANGER_AT`]. This app's OWN gauge red, sampled from
/// the HP bar in the same screenshot — the colour it already uses for "this is what is
/// at stake".
const RB_DANGER: u32 = 0xffe0_3a3a;

/// Width of one readout in logical px, and the per-field pen offsets.
fn refine_bar_metrics(fields: &[crate::refine::Field]) -> (usize, Vec<(usize, usize)>) {
    let mut pen = 0usize;
    let mut spans = Vec::with_capacity(fields.len());
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            pen += RB_FIELD_GAP;
        }
        let label_w = crate::gamefont::width(&f.label);
        let value_at = pen + label_w + if f.label.is_empty() { 0 } else { RB_LABEL_GAP };
        spans.push((pen, value_at));
        pen = value_at + crate::gamefont::width(&f.value);
    }
    (pen, spans)
}

// ── 세이브 슬롯 창 (130차) — the slot lists' ▲/▼ ─────────────────────────────────
//
// The game's slot list is three rows at panel `dx 32..145 · dy 22/39/56 · h 13`
// (`menutouch::SLOT_ROW_*`). Its side margins `dx 0..31` and `dx 146..175` over the rows'
// band are the checkered parchment and nothing else (probe frame, `SAVE_SLOTS_FEASIBILITY.md
// §5.5`), and the strip under the third row (`dy 70..78`) is parchment too, above the
// portrait band. The arrows go in the RIGHT margin, one per half of the rows' band.
// Everything is in the game's own logical px and scaled through the game rect, like the
// 제련 odds bar. (The `n~m / 12` readout that sat under the rows was removed at the
// user's request, 130차 §1-B.)
//
// ⛔ Only the worker knows the window; the UI draws what it was told (`slot_window`).

// ── 136차: the 「정렬」 button on the money strip ─────────────────────────────

/// The label's ink: the panel's own dark brown (the footer band `#311c10`), which reads
/// on the inventory's grey checker and the online panels' parchment alike.
const SORT_INK: u32 = 0xff31_1c10;

/// Draw 「정렬」 at the left end of the panel's money strip — the three bag screens the
/// worker names (`bagsort::SortScreen`), in the game's own font and panel px, scaled
/// through the game rect like every other panel-relative overlay.
///
/// On the 창고 panel the game itself prints a `✱` hint there (`*` = gold deposit, retired
/// 136차 — the touch HUD has had no `*` since 102차-C); it is painted over with the
/// strip's own solid colour first (`menutouch::WH_STAR_HINT`, measured on the probe).
pub fn draw_sort_button(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, screen: crate::bagsort::SortScreen) {
    let (lw, lh) = crate::logical();
    if game.w == 0 || game.h == 0 || lw == 0 || lh == 0 {
        return;
    }
    let Some((ox, oy)) = crate::menutouch::panel_origin(lw as i32, lh as i32) else {
        return;
    };
    let sx = f64::from(game.w) / f64::from(lw);
    let sy = f64::from(game.h) / f64::from(lh);
    let s = sx.min(sy);
    let px = |dx: f64| f64::from(game.x) + (f64::from(ox) + dx) * sx;
    let py = |dy: f64| f64::from(game.y) + (f64::from(oy) + dy) * sy;
    if screen == crate::bagsort::SortScreen::OnlineWarehouse {
        let b = crate::menutouch::WH_STAR_HINT;
        let (x0, y0) = (px(f64::from(b.dx)).round() as u32, py(f64::from(b.dy)).round() as u32);
        let (x1, y1) = (px(f64::from(b.dx + b.w)).round() as u32, py(f64::from(b.dy + b.h)).round() as u32);
        fill(buf, win_w, win_h, x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0), 0xff00_0000 | crate::menutouch::ONLINE_STRIP_BG);
    }
    // Centred in the 13-px strip: the glyphs are `gamefont::KOR_H` (11) tall.
    let (top, bot) = crate::menutouch::MONEY_STRIP_DY;
    let text_y = py(f64::from(top) + (f64::from(bot - top) - crate::gamefont::KOR_H as f64) / 2.0);
    draw_native_text(buf, win_w, win_h, px(f64::from(crate::menutouch::SORT_BUTTON.dx) + 4.0), text_y, s, "정렬", SORT_INK);
}

/// The ▲ box in panel px: `(dx, dy, w, h)`.
pub const SLOT_ARROW_UP: (i32, i32, i32, i32) = (146, 20, 30, 25);
/// The ▼ box in panel px.
pub const SLOT_ARROW_DOWN: (i32, i32, i32, i32) = (146, 45, 30, 25);
/// The game's own dark selected-row fill — the arrows read as part of the list.
const SLOT_ARROW_INK: u32 = 0xff4a_3121;
/// Alpha of an arrow that cannot slide further (the window is at that end).
const SLOT_ARROW_DIM: u32 = 72;

fn slot_arrow_contains(b: (i32, i32, i32, i32), dx: i32, dy: i32) -> bool {
    dx >= b.0 && dx < b.0 + b.2 && dy >= b.1 && dy < b.1 + b.3
}

/// Which arrow a panel-relative tap landed on: `-1` = ▲, `+1` = ▼, `None` = neither.
pub fn slot_window_hit(dx: i32, dy: i32) -> Option<i8> {
    if slot_arrow_contains(SLOT_ARROW_UP, dx, dy) {
        Some(-1)
    } else if slot_arrow_contains(SLOT_ARROW_DOWN, dx, dy) {
        Some(1)
    } else {
        None
    }
}

/// A filled triangle, alpha-blended. Scanline over the bounding box with a half-plane
/// test per pixel — the glyph is at most a few hundred pixels, so simplicity wins.
fn fill_tri(buf: &mut [u32], win_w: u32, win_h: u32, v: [(f64, f64); 3], color: u32, a: u32) {
    let (x0, x1) = (v.iter().map(|p| p.0).fold(f64::MAX, f64::min), v.iter().map(|p| p.0).fold(f64::MIN, f64::max));
    let (y0, y1) = (v.iter().map(|p| p.1).fold(f64::MAX, f64::min), v.iter().map(|p| p.1).fold(f64::MIN, f64::max));
    let edge = |p: (f64, f64), q: (f64, f64), x: f64, y: f64| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let area = edge(v[0], v[1], v[2].0, v[2].1);
    if area == 0.0 {
        return;
    }
    let (xs, xe) = (x0.floor().max(0.0) as u32, (x1.ceil().max(0.0) as u32).min(win_w));
    let (ys, ye) = (y0.floor().max(0.0) as u32, (y1.ceil().max(0.0) as u32).min(win_h));
    for y in ys..ye {
        for x in xs..xe {
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let w0 = edge(v[0], v[1], cx, cy) / area;
            let w1 = edge(v[1], v[2], cx, cy) / area;
            let w2 = edge(v[2], v[0], cx, cy) / area;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                blend_px(buf, (y * win_w + x) as usize, color, a);
            }
        }
    }
}

/// The save-slot window's controls over the game's slot list: ▲ and ▼ in the right
/// margin, dimmed at the end they cannot pass. `game` is the guest frame's window rect;
/// everything is placed in panel px. (A `n~m / total` readout under the rows was tried
/// and removed at the user's request — the row labels already carry the real numbers.)
pub fn draw_slot_window(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, first: u8, total: u8) {
    let (lw, lh) = crate::logical();
    if game.w == 0 || game.h == 0 || lw == 0 || lh == 0 || total < 3 {
        return;
    }
    let Some((ox, oy)) = crate::menutouch::panel_origin(lw as i32, lh as i32) else {
        return;
    };
    let sx = f64::from(game.w) / f64::from(lw);
    let sy = f64::from(game.h) / f64::from(lh);
    let px = |dx: f64| f64::from(game.x) + (f64::from(ox) + dx) * sx;
    let py = |dy: f64| f64::from(game.y) + (f64::from(oy) + dy) * sy;
    let first_max = total - 3;
    // ▲: apex near the top of its box, base 8 logical px below, 14 wide.
    for (b, up, dim) in [(SLOT_ARROW_UP, true, first == 0), (SLOT_ARROW_DOWN, false, first >= first_max)] {
        let cx = f64::from(b.0) + f64::from(b.2) / 2.0;
        let cy = f64::from(b.1) + f64::from(b.3) / 2.0;
        let (half_w, half_h) = (7.0, 4.5);
        let v = if up {
            [(px(cx), py(cy - half_h)), (px(cx - half_w), py(cy + half_h)), (px(cx + half_w), py(cy + half_h))]
        } else {
            [(px(cx - half_w), py(cy - half_h)), (px(cx + half_w), py(cy - half_h)), (px(cx), py(cy + half_h))]
        };
        fill_tri(buf, win_w, win_h, v, SLOT_ARROW_INK, if dim { SLOT_ARROW_DIM } else { 255 });
    }
}

/// Copy a rectangle of LOGICAL pixels onto the window through the game blit's own mapping —
/// destination row `dy` samples logical row `dy * lh / g.h`, destination column `dx` samples
/// `lut[dx]` (`app::sx_lut`) — so every pixel lands exactly where the game frame's blit put
/// that logical pixel, and the copy is indistinguishable from the game having drawn it.
///
/// `src` is `src_w` wide and holds logical pixel `(src_org.0 + c, src_org.1 + r)` at
/// `r * src_w + c`; `rect = (x, y, w, h)` is the logical rectangle to copy, which must lie
/// inside both `src` and the logical frame. Draws NOTHING on any mismatch — a patch that
/// cannot be placed exactly is not placed.
///
/// Two callers: the 117차-D reveal (the game's own message box shown THROUGH the box window,
/// `src` = the whole frame) and the 150차 상세보기 body (`src` = `itemnote::compose`'s patch).
#[allow(clippy::too_many_arguments)]
pub fn blit_logical_rect(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    g: Rect,
    lut: &[u32],
    logical: (u32, u32),
    src: &[u32],
    src_w: u32,
    src_org: (u32, u32),
    rect: (u32, u32, u32, u32),
) {
    let (lw, lh) = logical;
    let (x0, y0, w, h) = rect;
    let src_h = if src_w == 0 { 0 } else { src.len() as u32 / src_w };
    let ok = lw > 0
        && lh > 0
        && g.w > 0
        && g.h > 0
        && g.x.saturating_add(g.w) <= win_w
        && g.y.saturating_add(g.h) <= win_h
        && lut.len() == g.w as usize
        && buf.len() >= (win_w * win_h) as usize
        && w > 0
        && h > 0
        && x0.saturating_add(w) <= lw
        && y0.saturating_add(h) <= lh
        && x0 >= src_org.0
        && y0 >= src_org.1
        && x0 - src_org.0 + w <= src_w
        && y0 - src_org.1 + h <= src_h;
    if !ok {
        return;
    }
    let (x1, y1) = (x0 + w - 1, y0 + h - 1);
    // The LUT is monotone, so the destination columns inside the rectangle are one run.
    let dx0 = lut.partition_point(|&sx| sx < x0);
    let dx1 = lut.partition_point(|&sx| sx <= x1);
    if dx1 <= dx0 {
        return;
    }
    for dy in 0..g.h {
        let sy = ((u64::from(dy) * u64::from(lh)) / u64::from(g.h)) as u32;
        if sy < y0 || sy > y1 {
            continue;
        }
        let src_row = ((sy - src_org.1) * src_w) as usize;
        let dst_row = ((g.y + dy) * win_w + g.x) as usize;
        for (dx, &sx) in lut[dx0..dx1].iter().enumerate() {
            buf[dst_row + dx0 + dx] = src[src_row + (sx - src_org.0) as usize];
        }
    }
}

/// Draw the live 제련 odds under the smith's window.
///
/// `game` is the rect the guest framebuffer occupies, so everything here is placed in
/// the game's own coordinates and follows it through letterboxing and every logical
/// resolution. Draws nothing at all when the bar cannot be placed — there is no
/// truncated or clipped fallback, because a clipped percentage is a wrong percentage.
pub fn draw_refine_odds(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, r: &crate::refine::Refine) {
    let (lw, lh) = crate::logical();
    if game.w == 0 || game.h == 0 || lw == 0 || lh == 0 {
        return;
    }
    let Some((ox, oy)) = crate::menutouch::panel_origin(lw as i32, lh as i32) else {
        return;
    };
    let fields = crate::refine::readout(r);
    if fields.is_empty() {
        return;
    }
    let (content_w, spans) = refine_bar_metrics(&fields);
    let bar_w = content_w as f64 + (RB_PAD_X + 2.0) * 2.0;
    // Placed in LOGICAL px first, then clamped into the game's own frame — a bar hanging
    // off the edge would be the one thing worse than no bar.
    if bar_w > f64::from(lw) || RB_H > lh as i32 {
        return;
    }
    let bx_l = (f64::from(ox + REFINE_BAR.0) - bar_w / 2.0).clamp(0.0, f64::from(lw) - bar_w);
    let by_l = f64::from((oy + REFINE_BAR.1).clamp(0, lh as i32 - RB_H));

    let sx = f64::from(game.w) / f64::from(lw);
    let sy = f64::from(game.h) / f64::from(lh);
    // ONE scale for anything with a shape (the chamfer, the glyphs), so a window whose
    // two axes disagree by a rounding step cannot produce a lopsided corner.
    let s = sx.min(sy);
    let px = |lx: f64| (f64::from(game.x) + lx * sx).round() as u32;
    let py = |ly: f64| (f64::from(game.y) + ly * sy).round() as u32;
    let bar = Rect::new(px(bx_l), py(by_l), (bar_w * sx).round() as u32, (f64::from(RB_H) * sy).round() as u32);
    let edge = (s.round() as u32).max(1);
    let cut = (RB_CUT * s).round() as u32;
    fill_chamfered(buf, win_w, win_h, bar, cut, N_OUTLINE);
    fill_chamfered(buf, win_w, win_h, deflate(bar, edge), cut.saturating_sub(edge).max(1), N_RING);
    fill_chamfered(buf, win_w, win_h, deflate(bar, edge * 2), cut.saturating_sub(edge * 2).max(1), RB_BODY);

    // The text sits on the body's own top edge plus one pad row. `draw_native_text`
    // drops the ASCII onto the Hangul baseline, which is what keeps `성공 50%` reading
    // as one line instead of the digits riding high (the same fix 70차's quantity window
    // needed twice).
    let text_x = f64::from(game.x) + (bx_l + RB_PAD_X + 2.0) * sx;
    let text_y = f64::from(game.y) + (by_l + 3.0) * sy;
    for (f, (label_at, value_at)) in fields.iter().zip(spans) {
        let tone = match f.tone {
            crate::refine::Tone::Good => RB_GOOD,
            crate::refine::Tone::Danger => RB_DANGER,
            crate::refine::Tone::Normal => RB_VALUE,
        };
        if !f.label.is_empty() {
            draw_native_text(buf, win_w, win_h, text_x + label_at as f64 * sx, text_y, s, &f.label, RB_LABEL);
        }
        draw_native_text(buf, win_w, win_h, text_x + value_at as f64 * sx, text_y, s, &f.value, tone);
    }
}

// ── in-game 오브결합 / 소켓확장 target picker ────────────────────────────────
//
// The game has no picker of its own for this: its 오브결합 flow lives entirely on the
// network-warehouse screen and walks the wire-row array (`orbcombine.rs`'s header note).
// So the host draws the list, in the game's own window style, over the inventory.

/// Rows the viewport shows at once. The scroll position is PIXELS
/// (`orbcombine::scroll_span` and friends), so this is not a window index any more — it
/// is simply what the list height is divided by.
///
/// ★★ 2026-09-14: 5 → 7, at the player's request (*"각 열별 높이값 낮출꾼고"*). The
/// rows carry two lines, and the NAME is what has to stay readable — 71차-D bought that
/// legibility and 71차-N refused to give it back. So the name's share of a row went UP as
/// the row came down (`ROW_NAME_PCT`), which holds it at font scale 4 on a 1080-tall
/// screen where the old 5-row layout had scale 5: one step smaller, two more rows, and a
/// drag for the rest.
/// ⛔ Do not push this past ~8 without re-checking `ROW_NAME_PCT * row_h >= 44`
/// (`gamefont::KOR_H` is 11, so scale 4 needs 44 px) — below that the names drop to
/// scale 3 and the complaint 71차-D answered comes straight back.
pub const ORB_ROWS: usize = 7;

/// The name's share of a row's height, and the detail line's. They do not add to 100:
/// the remainder is the inset above, between and below them.
///
/// ★ 2026-09-15, user: *"아이템 명을 조금줄이고 설명을 조금더 키워줘"*. 52/28 → 44/36.
/// ⛔ The name is still the one with the floor: `game_fit_scale` steps down a whole font
/// size at a time, so 44 % of a 1080-screen row is what keeps it at scale 4 (`KOR_H` 11,
/// so scale 4 needs 44 px). Do not take it below that without re-measuring.
///
/// ★★ 2026-09-16, user: *"이름 밑에 설명있는거 폰트크기 조금만 늘려줘 너무 글씨가 작아"*.
/// 36 → 38, and the reason it read so small is a ONE-PIXEL miss: on the 1080-tall screen
/// both devices run, `row_h` is 91, so 36 % is 32 px — and scale 3 needs `KOR_H * 3` = 33.
/// The detail was landing on scale 2 while the name sat at 3, a whole step apart. 38 % is
/// 34 px, which clears it. Measured, not guessed: the width term never binds here (`tw` is
/// ~1042 px against a longest detail of ~300), and a sweep of every window from 480x360 to
/// 3200x1600 says the name/detail lines overlap ONLY on windows ≤ 380 px tall — the same
/// 274 sizes, with the same worst gap, as the 36 % this replaces. Nothing real regressed.
/// ⛔ Re-measure before raising it again: at 38 % the gap between the two lines is already
/// down to 7 px on a 1080 screen (it was 18), and the next step up would collide.
const ROW_NAME_PCT: u32 = 44;
const ROW_DETAIL_PCT: u32 = 38;

/// A drag has to move this many pixels before it stops being a tap. Scaled off the row
/// height so it means the same thing on every screen.
pub const ORB_DRAG_SLOP_PCT: u32 = 25;

/// What a tap on the list dialog does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbHit {
    /// The row at this ABSOLUTE index (not a visible-row number: the viewport scrolls by
    /// pixels). Tapping a row selects it; tapping the row that is already selected
    /// confirms it — the same 1탭 포커스 / 2탭 실행 grammar 66차 gave the menus.
    Row(usize),
    /// Inside the list, but on the empty tail past the last row. Starts a drag like a row
    /// does, and confirms nothing when the finger comes up.
    List,
    Ok,
    Cancel,
    /// A tap on the panel that hit no control. ⛔ Still `Some`: the dialog is drawn over
    /// the game's inventory, and a tap falling through would move the game's cell cursor.
    Swallow,
}

/// The dialog's boxes. The list is a VIEWPORT now — rows are computed from the scroll
/// position with [`orb_row_rect`] rather than being a fixed array.
pub struct OrbRects {
    pub panel: Rect,
    pub title: Rect,
    /// The scrolling viewport: what [`orb_row_rect`] is clipped to.
    pub list: Rect,
    pub row_h: u32,
    /// The scroll indicator's track, down the right edge INSIDE the list. Drawn only —
    /// the whole list is the drag target, so there is no thin bar to hit.
    pub bar: Rect,
    /// The whole band under the list. The BUTTONS are shorter than this (2026-09-15) but
    /// the band is what gets repainted to clip the list, so it must stay a full row tall —
    /// see the clip-by-repaint note in [`draw_orb_dialog`].
    pub band: Rect,
    pub ok: Rect,
    pub cancel: Rect,
}

/// Geometry of the list dialog. Centred like every other game window, and sized as a
/// fraction of the window so it works at every logical resolution the app runs at.
///
/// ★★ 71차-D — THE PANEL IS MUCH BIGGER, and the reason is the CLAMPS, not the
/// percentages (player 2026-08-26: *"가방 창을 조금 많이 키웠으면 좋겠어 글자가 너무 작아서
/// 안보여"*). The old `46% clamp(320,760) x 78% clamp(200,520)` hit its ceiling on every
/// phone this ships to: a 2340x1080 S25 asked for 1076x842 and got **760x520** — a third
/// of the screen's width.
///
/// ★★ 71차-N — THE WIDTH CAME BACK DOWN, THE HEIGHT DID NOT (player, 2026-08-27:
/// *"가방 창 너무커진거같아 조금만 줄여줘 좌측에화살표까지 다 가려버리고있어"*).
/// `56% clamp(320,1180)` puts it at x 580..1760 on an S25, which is 210 px of clearance
/// back on each side.
///
/// ⛔ The HEIGHT is still deliberately untouched. Row height is what sets the font, and
/// 2026-09-14 lowered the row by fitting MORE rows in the same height — not by shrinking
/// the panel. Shaving the height too would take the names below font scale 4.
pub fn orb_rects(win_w: u32, win_h: u32) -> OrbRects {
    let w = (win_w * 56 / 100).clamp(320, 1180);
    let h = (win_h * 88 / 100).clamp(200, 1000);
    let panel = Rect::new((win_w.saturating_sub(w)) / 2, (win_h.saturating_sub(h)) / 2, w, h);

    let edge = (h / 40).clamp(2, 5);
    let pad = (h / 34).clamp(3, 10);
    let ix = panel.x + edge * 2 + pad;
    let iw = w.saturating_sub((edge * 2 + pad) * 2);
    let iy = panel.y + edge * 2 + pad;
    let ih = h.saturating_sub((edge * 2 + pad) * 2);

    let title_h = ih * 15 / 100;
    let btn_h = ih * 15 / 100;
    let list_h = ih.saturating_sub(title_h + btn_h);
    let row_h = (list_h / ORB_ROWS as u32).max(1);
    // The viewport is a WHOLE number of rows tall. The leftover (< one row) is given to
    // the button band, so the list's bottom edge is exactly where a row ends and the
    // clip-by-repaint in `draw_orb_dialog` has a full button band to hide the overflow in.
    let view_h = row_h * ORB_ROWS as u32;

    let ly = iy + title_h;
    let bar_w = (iw / 90).clamp(2, 6);
    let half = (iw.saturating_sub(pad)) / 2;
    let by = ly + view_h;
    let band_h = ih.saturating_sub(title_h + view_h);
    // ★ 2026-09-15, user: *"선택 취소 버튼 높이와 글자크기 더 줄여줘"*. The BUTTONS shrank;
    // the BAND did not. ⛔ That separation is load-bearing: the band is the opaque strip
    // the list is clipped against (`draw_orb_dialog`), and it has to stay at least one row
    // tall or a scrolled row spills out of the bottom of the window. The buttons are
    // bottom-aligned inside it, which also puts a little air between them and the list.
    let btn_h = (band_h * 68 / 100).max(1);
    let bty = by + band_h.saturating_sub(btn_h);
    OrbRects {
        panel,
        title: Rect::new(ix, iy, iw, title_h),
        list: Rect::new(ix, ly, iw, view_h),
        row_h,
        bar: Rect::new(ix + iw.saturating_sub(bar_w), ly, bar_w, view_h),
        band: Rect::new(ix, by, iw, band_h),
        ok: Rect::new(ix, bty, half, btn_h),
        cancel: Rect::new(ix + iw - half, bty, half, btn_h),
    }
}

/// Where row `i` sits at scroll position `scroll`. It may hang off the top or the bottom
/// of [`OrbRects::list`] by up to one row — the dialog draws the whole row and then
/// repaints the title and button bands over the overflow.
///
/// ⛔ Returns `None` for a row entirely outside the viewport rather than a clamped rect:
/// a clamped chamfered plate would draw its corner cut in the middle of the list.
pub fn orb_row_rect(r: &OrbRects, scroll: f64, i: usize) -> Option<Rect> {
    let top = i as f64 * f64::from(r.row_h) - scroll.max(0.0);
    if top + f64::from(r.row_h) <= 0.0 || top >= f64::from(r.list.h) {
        return None;
    }
    // ONLY ever `list.y + top` with `top > -row_h`, and `list.y` is a whole title band
    // below the panel's own top, so this cannot go negative on any window this ships to.
    let y = (f64::from(r.list.y) + top).max(0.0) as u32;
    // ⛔ The scroll indicator's column is left free on EVERY row, scrollable or not. The
    // bar appears and disappears with the list's length, and a row that changed width
    // with it would make the whole list twitch the moment one stack is spent.
    Some(Rect::new(r.list.x, y, r.list.w.saturating_sub(r.bar.w), r.row_h))
}

/// The item-icon square at a row's left edge. Inside the row, so it is drawn and never
/// hit-tested on its own — a tap anywhere on the row is that row.
pub fn orb_icon_rect(row: &Rect) -> Rect {
    let side = (row.h * 68 / 100).max(1);
    let gap = (row.h.saturating_sub(side)) / 2;
    Rect::new(row.x + gap, row.y + gap, side, side)
}

/// What a tap at `(px, py)` asks the dialog for. `len` is the whole list (the viewport
/// scrolls by pixels, so what is on screen is a function of `scroll`, not of a count).
pub fn orb_hit(win_w: u32, win_h: u32, len: usize, scroll: f64, px: f64, py: f64) -> Option<OrbHit> {
    let r = orb_rects(win_w, win_h);
    if r.ok.contains(px, py) {
        return Some(OrbHit::Ok);
    }
    if r.cancel.contains(px, py) {
        return Some(OrbHit::Cancel);
    }
    if r.list.contains(px, py) {
        let dy = py - f64::from(r.list.y);
        return Some(crate::orbcombine::row_at(scroll, dy, len, r.row_h).map_or(OrbHit::List, OrbHit::Row));
    }
    r.panel.contains(px, py).then_some(OrbHit::Swallow)
}

// ── the DROP BOX — the verbs, on the row they belong to ───────────────────
//
// ★★ User, 2026-09-14: *"소비가방은 선택하면 따로 화면전환으로 사용하기, 퀵슬롯등록이
// 뜨는게아니라 소비가방 창에서 아이템 포커스 -> OK로 선택하면 그 위치에 드랍박스로(기존처럼)
// 사용하기, 퀵슬롯 등록을 넣으면 돼"*. "기존처럼" is the GAME'S own action popup, so the
// box is drawn in the game's own popup palette — measured at 79차-C, `menutouch`'s
// `POPUP_BODY_INK` and friends — rather than in this window's parchment chrome.
//
// ⛔ It is anchored to the ROW, which means it moves when the list scrolls. It cannot:
// the list does not scroll while it is up (every direction key belongs to the box).

/// Verb rows the drop box can show — four. The widest list is the online screens'
/// 상세보기 / 버리기 / 올리기 at three, and the 소비가방's 사용하기 / 퀵슬롯등록 / 버리기
/// at three (it was four until 꺼내기 was removed, 2026-09-16). The geometry is written for
/// any small number; this is the cap that keeps the box inside the list's viewport.
pub const ORB_DROP_MAX: usize = 4;

/// A verb row's height as a percentage of the list's row height.
///
/// ★ 68 → 75 (2026-09-16, user: *"드랍박스 각 선택지의 높이를 아주 조금만 키워주고"*). On
/// the two 1080-high phones the list row is 91 px, so a verb row goes 61 → 68 px; the
/// label budget (`62 %` of the row, below) stays inside game-font scale 3 (33 px) either
/// way — only the row grows, not the text, which is what "아주 조금만" asks for.
pub const ORB_DROP_ROW_PCT: u32 = 75;

/// The drop box's outer rect and its rows, for a box anchored on `anchor` at `scroll`.
/// `None` when that row is not on screen — which cannot happen while the box is up, but
/// is the honest answer rather than a box floating over the title band.
pub fn orb_drop_rects(win_w: u32, win_h: u32, scroll: f64, anchor: usize, verbs: usize) -> Option<(Rect, Vec<Rect>)> {
    let r = orb_rects(win_w, win_h);
    let row = orb_row_rect(&r, scroll, anchor)?;
    let verbs = verbs.min(ORB_DROP_MAX).max(1);
    let rh = (r.row_h * ORB_DROP_ROW_PCT / 100).max(1);
    let pad = (rh / 6).max(1);
    let h = rh * verbs as u32 + pad * 2;
    let w = (r.list.w * 54 / 100).max(1);
    // Left edge: past the icon column, so the box does not bury the item it belongs to.
    let x = orb_icon_rect(&row).x + row.h;
    let x = x.min(r.list.x + r.list.w.saturating_sub(w));
    // Below the row when it fits inside the panel, above it otherwise — the game's own
    // popup does the same (`0x427be` / `0x4456a` shift y when the rows do not fit).
    let below = row.y + row.h;
    let y = if below + h <= r.list.y + r.list.h {
        below
    } else {
        row.y.saturating_sub(h)
    };
    let rows = (0..verbs)
        .map(|i| Rect::new(x + pad, y + pad + i as u32 * rh, w.saturating_sub(pad * 2), rh))
        .collect();
    Some((Rect::new(x, y, w, h), rows))
}

/// Which verb row a tap lands on. `None` = outside the box entirely (the caller closes
/// it — the game's own popup dismisses the same way).
pub fn orb_drop_hit(win_w: u32, win_h: u32, scroll: f64, anchor: usize, verbs: usize, px: f64, py: f64) -> Option<usize> {
    let (_, rows) = orb_drop_rects(win_w, win_h, scroll, anchor, verbs)?;
    rows.iter().position(|r| r.contains(px, py))
}

/// Fill a rectangle whose four corners are cut at 45° — **the game's own window shape**.
///
/// ★★ 71차-G, and it is measured, not invented. `res/comm/ui_bm` (the game's HUD chrome,
/// decoded with `tools_re/gen_h3_icons.py`'s `_bm` reader) draws every one of its boxes
/// like this — frames 3 and 4, top-left corner, are byte-identical in shape:
///
/// ```text
///   . . D D D D      D = #181818  the outer edge
///   . D L L L L      L = #c5c2c5  the highlight bevel  (#949194 on the shadow side)
///   D L L K K K      K = #000000  the interior
///   D L K K K K
/// ```
///
/// i.e. a two-pixel diagonal, then straight edges. The player's note was *"창도 너무
/// 끝도 뾰족하고"* — square corners are the one thing the game never draws.
///
/// `cut` is the diagonal's length. Nesting two of these with different `cut` values is
/// what makes the bevel follow the corner instead of cutting across it.
fn fill_chamfered(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, cut: u32, color: u32) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let cut = cut.min(r.w / 2).min(r.h / 2);
    for y in 0..r.h {
        // Distance to the nearer of the top and bottom edges decides how far this row is
        // pulled in; past `cut` rows the edge is straight.
        let near = y.min(r.h - 1 - y);
        let inset = cut.saturating_sub(near);
        let w = r.w.saturating_sub(inset * 2);
        if w > 0 {
            fill(buf, win_w, win_h, r.x + inset, r.y + y, w, 1, color);
        }
    }
}

/// Shrink a rect by `n` on every side, keeping it non-empty.
fn deflate(r: Rect, n: u32) -> Rect {
    Rect::new(r.x + n, r.y + n, r.w.saturating_sub(n * 2).max(1), r.h.saturating_sub(n * 2).max(1))
}

/// Draw the picker. `rows` is the visible slice, `sel_visible` the highlighted row inside
/// it (`None` when the selection has scrolled off), and `more` says whether there is more
/// list above / below so the arrows can be dimmed.
pub fn draw_orb_dialog(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    title: &str,
    subtitle: &str,
    rows: &[crate::OrbRow],
    // The selected row, as an ABSOLUTE index. `Some(0)` on open since 2026-09-14 — see
    // `App::orb_open`.
    sel: Option<usize>,
    // Pixels the list is scrolled by (`orbcombine::clamp_scroll` keeps it honest).
    scroll: f64,
    // The bottom button's word — `결합` / `선택` / `확인`.
    ok_label: &str,
) {
    let r = orb_rects(win_w, win_h);
    // ⛔ Clamped HERE too, not only where the drag writes it: a rotation or a HEAT SAVER
    // divisor change resizes the viewport under a scroll position that was legal for the
    // old one, and an unclamped position draws an empty window with no way back.
    let scroll = crate::orbcombine::clamp_scroll(scroll, rows.len(), r.row_h, r.list.h);
    let p = r.panel;
    let edge = (p.h / 40).clamp(2, 5);
    let fh = crate::gamefont::KOR_H as u32;

    // ── the GAME'S OWN WINDOW, not the message box (71차-G) ─────────────────
    //
    // Player: *"오브 결합창의 디자인을 조금더 영웅서기3 다른 창 디자인과 비슷하게 …
    // 창도 너무 끝도 뿬족하고 테두리나 전체 분위기를 인게임 메뉴창처럼"*.
    //
    // * **The body is the game's LIST window**, parchment with the checker that makes it
    //   read as cloth — not `G_MODAL`, the dusty rose of 구매하시겠습니까?.
    // * **The corners are chamfered.** The game never draws a square one — see
    //   [`fill_chamfered`] for the two-pixel diagonal read out of `res/comm/ui_bm`.
    //
    // ⛔ Do not "simplify" the three nested chamfers back into `fill` calls: the corner
    // is the whole point, and a square corner behind a chamfered one shows through.
    let cut = (p.w.min(p.h) / 55).clamp(3, 16);
    fill_chamfered(buf, win_w, win_h, p, cut, G_FRAME);
    fill_chamfered(buf, win_w, win_h, deflate(p, edge), cut.saturating_sub(1).max(1), G_EDGE);
    let body = deflate(p, edge * 2);
    fill_chamfered(buf, win_w, win_h, body, cut.saturating_sub(2).max(1), G_PANEL);
    checker(buf, win_w, win_h, body, G_PANEL_ALT);

    // ── the rows ───────────────────────────────────────────
    //
    // ★★ CLIP BY REPAINT (2026-09-14). The viewport scrolls by PIXELS, so the first and
    // last rows are usually cut. Rather than teaching `fill_chamfered`, the icon blitter
    // and the text renderer about a clip rectangle, the rows are drawn whole and the two
    // opaque bands they can spill into — the title band above, the button band below —
    // are painted AFTERWARDS, over the overflow.
    //
    // ⛔ That is only sound while a row cannot spill past those bands, i.e. while
    // `row_h <= title_h` and `row_h <= btn_h`. With `ORB_ROWS = 7` the row is 1/7 of 70%
    // of the inner box and each band is 15% of it, so the margin is comfortable — but a
    // change to either number has to be checked against this.
    let at = atlas::atlas();
    let (first, _) = crate::orbcombine::first_visible(scroll, r.row_h);
    for i in first..rows.len() {
        let Some(b) = orb_row_rect(&r, scroll, i) else { break };
        let row = &rows[i];
        let inset = (b.h / 12).clamp(1, 3);
        let selected = sel == Some(i);
        let face = match (selected, row.enabled) {
            (true, true) => G_ROW_SEL,
            (_, false) => G_ROW_OFF,
            (false, true) => G_ROW,
        };
        // ★ 71차-G: the row is a chamfered plate with the game's near-black outline —
        // the shape its own list rows have. On the parchment body an unoutlined brown
        // block reads as a sticker; the outline is what seats it in the window.
        let plate = Rect::new(b.x, b.y + inset, b.w, b.h.saturating_sub(inset * 2));
        let rcut = (plate.h / 6).clamp(2, 10);
        fill_chamfered(buf, win_w, win_h, plate, rcut, G_FRAME);
        fill_chamfered(buf, win_w, win_h, deflate(plate, 1), rcut.saturating_sub(1).max(1), face);
        // ★★ 2026-08-27 — THE FOCUSED ROW, SAID THREE WAYS (player: *"현재 포커스된
        // 아이템에 대한 구분이 조금 어려운데 … 좀더 명확하게 알수 있도록"*):
        //   (a) [`G_ROW_SEL`] — the face itself is lit, visible before anything else;
        //   (b) a ring that SCALES with the row instead of a fixed 2 px;
        //   (c) a cream ▶ caret in the margin the icon column already leaves free.
        // ⛔ The caret's column is reserved on EVERY row, so turning it on cannot shift
        // the icon or the text.
        let ic = orb_icon_rect(&b);
        if selected {
            let ring = (plate.h / 16).clamp(2, 5);
            fill_chamfered(buf, win_w, win_h, plate, rcut, G_TITLE);
            fill_chamfered(buf, win_w, win_h, deflate(plate, ring), rcut.saturating_sub(1).max(1), face);
            let left = b.x + ring;
            let span = ic.x.saturating_sub(left);
            if span >= 8 {
                let cs = (span * 2 / 5).clamp(3, 9);
                // Cream even on a DISABLED row: the caret answers "which one am I on",
                // and that question is at its most urgent on a row that will refuse.
                draw_arrow(buf, win_w, win_h, left + span / 2, plate.y + plate.h / 2, cs, Dir::Right, G_TITLE);
            }
        }
        // ★ 71차-D: the game's OWN item icon, left of the text — `item_bm` frame `cat`
        // for equipment, so the row reads 투구 / 갑옷 / 검 / 방패 at a glance.
        if let Some(art) = at.item(row.icon) {
            blit_icon_rect(buf, win_w, win_h, ic, art);
        }
        let ink = if row.enabled { G_TITLE } else { G_MUTED };
        let pad = (b.w / 40).clamp(2, 8);
        // ⛔ Saturating, not plain arithmetic: a portrait or hand-shrunk window makes the
        // row short and wide-ish in ways the numbers above do not obviously bound.
        let tx = ic.x + ic.w + pad;
        let tw = (b.x + b.w).saturating_sub(tx + pad);
        let ns = game_fit_scale(&row.name, tw, b.h * ROW_NAME_PCT / 100);
        draw_game_text(buf, win_w, win_h, tx, b.y + inset + b.h / 14, ns, &row.name, ink);
        if !row.detail.is_empty() {
            let ds = game_fit_scale(&row.detail, tw, b.h * ROW_DETAIL_PCT / 100);
            let dy = b.y + b.h.saturating_sub(inset + fh * ds + b.h / 14);
            draw_game_text(buf, win_w, win_h, tx, dy, ds, &row.detail, if row.enabled { G_ACCENT } else { G_MUTED });
        }
    }

    // The bands that clip the list, repainted over whatever the rows spilled into.
    // ⛔ `r.band`, not the buttons: since 2026-09-15 the buttons are shorter than the band
    // they sit in, and clipping against them would leave a row's tail showing above.
    for band in [r.title, r.band] {
        fill(buf, win_w, win_h, band.x, band.y, band.w, band.h, G_PANEL);
        checker(buf, win_w, win_h, band, G_PANEL_ALT);
    }

    // ── the scroll indicator ───────────────────────────────────
    //
    // ★ The ▲/▼ buttons are gone (user, 2026-09-14) and with them the only thing that
    // said "there is more below". A thumb has nothing to aim at here — the whole list
    // drags — so this is a hairline TRACK + THUMB, drawn only when there is something to
    // scroll, exactly like the game's own 도움말 window (80차-L, dx 166..172).
    let span = crate::orbcombine::scroll_span(rows.len(), r.row_h, r.list.h);
    if span > 0.0 {
        let track = r.bar;
        fill(buf, win_w, win_h, track.x, track.y, track.w, track.h, G_PANEL_ALT);
        let frac = f64::from(r.list.h) / (rows.len() as f64 * f64::from(r.row_h));
        let th = ((f64::from(track.h) * frac) as u32).clamp(track.w * 2, track.h);
        let ty = track.y + ((f64::from(track.h - th) * (scroll / span)) as u32).min(track.h - th);
        fill(buf, win_w, win_h, track.x, ty, track.w, th, G_ROW);
    }

    // ── the title band ──────────────────────────────────────
    //
    // The game's dark-brown scroll bar with cream text in it, and the subtitle under it on
    // the parchment. Drawn AFTER the rows — it is half of the clip (see above).
    let t = r.title;
    let band_h = (t.h * 58 / 100).max(1);
    let band = Rect::new(t.x, t.y, t.w, band_h);
    fill_chamfered(buf, win_w, win_h, band, cut.saturating_sub(2).max(1), G_ROW);
    stroke(buf, win_w, win_h, (band.x, band.y, band.w, band.h), G_FRAME);
    let ts = game_fit_scale(title, t.w.saturating_sub(cut * 4), band_h * 66 / 100);
    draw_game_text_centered(buf, win_w, win_h, t.x, t.y + band_h.saturating_sub(fh * ts) / 2, t.w, ts, title, G_TITLE);
    if !subtitle.is_empty() {
        let ss = game_fit_scale(subtitle, t.w, (t.h - band_h) * 62 / 100);
        draw_game_text_centered(
            buf,
            win_w,
            win_h,
            t.x,
            t.y + band_h + (t.h - band_h).saturating_sub(fh * ss) / 2,
            t.w,
            ss,
            subtitle,
            G_ROW,
        );
    }

    // ── the buttons ───────────────────────────────────────
    //
    // ⛔ NEITHER IS FOCUSED ANY MORE (user decision, 2026-09-14). 71차-H put the ←/→ keys
    // on these two buttons, but the left-hand direction keys now belong to the LIST — a
    // gamepad scrolls with them. 취소 is still reachable two ways: the CLR key, and a tap
    // on the button itself.
    for (b, label, lit) in [(&r.ok, ok_label, true), (&r.cancel, "취소", false)] {
        // The game's buttons are the same plate: near-black outline, chamfered corners.
        // ★ The outline is TWO pixels, not one: the button face is parchment and so is
        // the window body behind it, so a hairline leaves them touching.
        let bcut = (b.h / 5).clamp(2, 12);
        let bord = (b.h / 24).clamp(2, 4);
        fill_chamfered(buf, win_w, win_h, *b, bcut, G_FRAME);
        fill_chamfered(
            buf,
            win_w,
            win_h,
            deflate(*b, bord),
            bcut.saturating_sub(bord).max(1),
            if lit { G_PANEL } else { G_PANEL_ALT },
        );
        let s = game_fit_scale(label, b.w.saturating_sub(b.h / 2), b.h * 46 / 100);
        draw_game_text_centered(buf, win_w, win_h, b.x, b.y + b.h.saturating_sub(fh * s) / 2, b.w, s, label, G_FRAME);
    }
}

/// The drop box over the list: the game's own action popup, on the row it belongs to.
///
/// Palette measured at 79차-C from the game's own 동작 팝업 (사용하기 / 상세보기 /
/// 버리기) — body `0xbdb2ad`, row `0x7b5952`, selected row `0x4a3121`. Those are the
/// numbers `menutouch` matches frames against, so a host box drawn in them is the same
/// object to the player's eye.
pub fn draw_orb_dropbox(buf: &mut [u32], win_w: u32, win_h: u32, scroll: f64, anchor: usize, verbs: &[String], sel: usize) {
    let Some((box_r, rows)) = orb_drop_rects(win_w, win_h, scroll, anchor, verbs.len()) else {
        return;
    };
    let fh = crate::gamefont::KOR_H as u32;
    let cut = (box_r.h / 14).clamp(2, 6);
    fill_chamfered(buf, win_w, win_h, box_r, cut, G_FRAME);
    fill_chamfered(buf, win_w, win_h, deflate(box_r, 1), cut.saturating_sub(1).max(1), POPUP_BODY);
    for (i, b) in rows.iter().enumerate() {
        let Some(label) = verbs.get(i) else { break };
        let face = if i == sel { POPUP_SEL } else { POPUP_ROW };
        fill(buf, win_w, win_h, b.x, b.y, b.w, b.h, face);
        let s = game_fit_scale(label, b.w.saturating_sub(b.h / 2), b.h * 62 / 100);
        draw_game_text_centered(buf, win_w, win_h, b.x, b.y + b.h.saturating_sub(fh * s) / 2, b.w, s, label, G_TITLE);
    }
}

/// How a settings row is drawn: its face, and whether the state line is lit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState {
    /// The default value — plain brown, muted state line.
    Normal,
    /// The player has chosen something other than the default.
    Active,
    /// Not available in this configuration; the row swallows the tap and says why.
    Disabled,
}

// The game's own ACTION POPUP palette (79차-C, measured): the drop box is the game's
// 사용하기 / 상세보기 / 버리기 box, so it is drawn in that box's colours and not in the
// parchment of the window it sits on.
const POPUP_BODY: u32 = 0xffbd_b2ad;
const POPUP_ROW: u32 = 0xff7b_5952;
const POPUP_SEL: u32 = 0xff4a_3121;

// The game's own window palette, sampled from its item / slot / notice windows.
const G_FRAME: u32 = 0xff1e_1610;
const G_EDGE: u32 = 0xffd8_ccb0;
const G_PANEL: u32 = 0xffc9_c2b4;
const G_PANEL_ALT: u32 = 0xffb6_af9f;
const G_ROW: u32 = 0xff6b_4535;
const G_ROW_ON: u32 = 0xff8d_5a2c;
/// The FOCUSED row's face. ★ 2026-08-27 (player: *"현재 포커스된 아이템에 대한 구분이 조금
/// 어려운데 … 좀더 명확하게 알수 있도록"*). `G_ROW_ON` sat only one step above `G_ROW`, and
/// one step of brown next to a 2-px ring is not a focus indicator on a phone held at arm's
/// length. This is the same hue carried far enough up that the row reads as lit on its own,
/// before the ring or the caret are counted.
const G_ROW_SEL: u32 = 0xffc8_7f36;
const G_ROW_OFF: u32 = 0xff4a_403a;
const G_TITLE: u32 = 0xfff2_e4c4;
const G_ACCENT: u32 = 0xffe8_a63c;
const G_MUTED: u32 = 0xffa9_9c8c;
// …and its MESSAGE-BOX style, which is a different window from the parchment panels
// above: the dusty rose of 구매하시겠습니까? / 개수를 설정하세요, with a near-black plate
// behind the number. Sampled from the device (2026-08-25 screenshots).
// ⛔ Kept, not deleted: it is a colour READ OFF THE GAME, and the message-box windows
// (구매하시겠습니까? / 개수를 설정하세요) are drawn with it by `draw_qty_dialog`'s own
// palette. 71차-G took the orb picker off it — a LIST window is parchment in this game,
// a two-line question is rose — which is what left this without a caller here.
#[allow(dead_code)]
const G_MODAL: u32 = 0xff8f_6f6b;
const G_MODAL_DARK: u32 = 0xff2a_211e;

/// One row of the settings panel, in the game's window style.
fn draw_row(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, title: &str, sub: &str, state: RowState) {
    let inset = (r.h / 14).clamp(2, 4);
    let face = match state {
        RowState::Normal => G_ROW,
        RowState::Active => G_ROW_ON,
        RowState::Disabled => G_ROW_OFF,
    };
    fill(buf, win_w, win_h, r.x, r.y, r.w, r.h, G_FRAME);
    fill(
        buf,
        win_w,
        win_h,
        r.x + inset,
        r.y + inset,
        r.w.saturating_sub(inset * 2),
        r.h.saturating_sub(inset * 2),
        face,
    );

    let ink = if state == RowState::Disabled { G_MUTED } else { G_TITLE };
    let sub_ink = match state {
        RowState::Active => G_ACCENT,
        RowState::Disabled => G_MUTED,
        RowState::Normal => G_EDGE,
    };
    let h = crate::gamefont::KOR_H as u32;
    let pad = inset * 3;
    let inner_w = r.w.saturating_sub(pad * 2);

    // ★ ONE LINE, name left and state right (2026-08-20). It was two centred lines,
    // and the player's verdict was "폰트크기 조금만 키워줘 너무 작아" — which two lines
    // cannot fix: the font scales in whole pixels, so a 55 px row can hold two 22 px
    // lines (scale 2) or one 33 px line (scale 3), and never two of those. Splitting
    // the row across instead of down buys the whole height for one line, and it is
    // also the shape the game's own option rows have.
    if sub.is_empty() {
        let s = game_fit_scale(title, inner_w, r.h * 70 / 100);
        draw_game_text_centered(buf, win_w, win_h, r.x, r.y + r.h.saturating_sub(h * s) / 2, r.w, s, title, ink);
        return;
    }
    // Both halves share one scale — a bigger name over a smaller state reads as two
    // different controls. The width budget is split in the ratio they need.
    let need = (crate::gamefont::width(title) + crate::gamefont::width(sub) + 2).max(1) as u32;
    let s = ((inner_w / need).min(r.h * 70 / 100 / h)).max(1);
    let y = r.y + r.h.saturating_sub(h * s) / 2;
    draw_game_text(buf, win_w, win_h, r.x + pad, y, s, title, ink);
    let sw = game_text_w(sub, s);
    draw_game_text(buf, win_w, win_h, r.x + r.w.saturating_sub(pad + sw), y, s, sub, sub_ink);
}

/// Largest glyph scale that keeps a `chars`-long ASCII label inside `r`.
fn fit_scale(r: Rect, chars: u32) -> u32 {
    let by_h = (r.h / 12).clamp(2, 8);
    let by_w = (r.w * 86 / 100 / (chars.max(1) * 6)).max(1);
    by_h.min(by_w).max(1)
}

/// Scale and the two baselines for a STACKED two-line button caption, sized so both
/// lines fit inside `r` with a gap.
///
/// ★ Device-caught 2026-07-31: the rows used to place the lines at fixed `h/5` and
/// `3h/5` with a scale derived only from `h/12`. That works at the old three-row
/// height (172 px) and collides at the four-row height (111 px) — glyphs are `7*s`
/// tall, so at `s = 8` line 1 ran 22..78 and line 2 started at 66 and spilled 11 px
/// out of the button. FULL/ON, ZOOM/220 and LAYOUT/EDIT were all printed on top of
/// themselves. Derive the positions from the glyph height instead of guessing.
fn two_line(r: Rect, chars: u32) -> (u32, u32, u32) {
    // 16 = two 7-tall lines + a ~2 gap, against 88 % of the button height.
    let by_h = ((r.h * 88 / 100) / 16).max(1);
    let by_w = (r.w * 86 / 100 / (chars.max(1) * 6)).max(1);
    let s = by_h.min(by_w).clamp(1, 8);
    let gap = (s * 3 / 7).max(1);
    let total = 14 * s + gap;
    let y1 = r.y + r.h.saturating_sub(total) / 2;
    (s, y1, y1 + 7 * s + gap)
}

impl Rect {
    fn rect_stroke(&self) -> (u32, u32, u32, u32) {
        (self.x, self.y, self.w, self.h)
    }
}

/// Alpha-blend `color` over the framebuffer pixel at `i` (softbuffer is opaque
/// XRGB, so translucency has to be composited by hand). `a` = 0..=255.
fn blend_px(buf: &mut [u32], i: usize, color: u32, a: u32) {
    let dst = buf[i];
    let mix = |sh: u32| {
        let s = (color >> sh) & 0xff;
        let d = (dst >> sh) & 0xff;
        ((s * a + d * (255 - a)) / 255) & 0xff
    };
    buf[i] = 0xff00_0000 | (mix(16) << 16) | (mix(8) << 8) | mix(0);
}

/// Alpha-blended disc (softbuffer has no alpha channel - see [`blend_px`]).
/// Hard-edged; [`blend_disc_soft`] is the anti-aliased one used by every key.
#[allow(dead_code)]
fn blend_disc(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, color: u32, a: u32) {
    if r <= 0.0 {
        return;
    }
    let x0 = (cx - r).max(0.0) as u32;
    let x1 = ((cx + r).max(0.0) as u32 + 1).min(win_w);
    let y0 = (cy - r).max(0.0) as u32;
    let y1 = ((cy + r).max(0.0) as u32 + 1).min(win_h);
    let r2 = r * r;
    for y in y0..y1 {
        let dy = f64::from(y) + 0.5 - cy;
        let row = (y * win_w) as usize;
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - cx;
            if dx * dx + dy * dy <= r2 {
                blend_px(buf, row + x as usize, color, a);
            }
        }
    }
}

/// Alpha-blended disc with an ANTI-ALIASED, optionally FEATHERED edge.
///
/// `feather` = width in px of the fade band ending at `r`. `feather <= 1` gives a
/// crisp (but still anti-aliased) edge; a larger value makes the disc dissolve into
/// whatever is behind it. Coverage is smoothstepped, so there is no visible
/// "shading starts here" line — the same reason the title-edge ramp uses a curve.
///
/// This is what makes a round key read as round on a busy game background: a
/// hard-clipped circle over pixel art shows staircase edges at this scale.
fn blend_disc_soft(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, color: u32, a: u32, feather: f64) {
    if r <= 0.0 || a == 0 {
        return;
    }
    let f = feather.max(0.9);
    let x0 = (cx - r - 1.0).max(0.0) as u32;
    let x1 = ((cx + r + 1.0).max(0.0) as u32 + 1).min(win_w);
    let y0 = (cy - r - 1.0).max(0.0) as u32;
    let y1 = ((cy + r + 1.0).max(0.0) as u32 + 1).min(win_h);
    for y in y0..y1 {
        let dy = f64::from(y) + 0.5 - cy;
        let row = (y * win_w) as usize;
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt();
            let cov = ((r - d) / f).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let cov = cov * cov * (3.0 - 2.0 * cov); // smoothstep
            let aa = (f64::from(a) * cov) as u32;
            if aa > 0 {
                blend_px(buf, row + x as usize, color, aa);
            }
        }
    }
}

/// Anti-aliased ring of thickness `thick` centred on radius `r`, alpha `a`.
fn blend_ring_soft(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, thick: f64, color: u32, a: u32) {
    if r <= 1.0 || a == 0 {
        return;
    }
    let half = (thick / 2.0).max(0.6);
    let pad = half + 1.5;
    let x0 = (cx - r - pad).max(0.0) as u32;
    let x1 = ((cx + r + pad).max(0.0) as u32 + 1).min(win_w);
    let y0 = (cy - r - pad).max(0.0) as u32;
    let y1 = ((cy + r + pad).max(0.0) as u32 + 1).min(win_h);
    for y in y0..y1 {
        let dy = f64::from(y) + 0.5 - cy;
        let row = (y * win_w) as usize;
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt();
            // Coverage falls off over ~1px either side of the band.
            let cov = (1.0 - ((d - r).abs() - half).max(0.0)).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let aa = (f64::from(a) * cov) as u32;
            if aa > 0 {
                blend_px(buf, row + x as usize, color, aa);
            }
        }
    }
}

/// The ONE round-key face used by every on-screen key (player request 2026-07-30:
/// "the 1/3/7/9/0/#/CANCEL keys and the menu 1/3/7/9 keys are square — make them
/// ALL round"). Inscribed circle of `r`, translucent fill + two anti-aliased rims
/// (outer bright, inner dim) so the key reads on any background.
/// Translucent anti-aliased TRIANGLE with a rim, pointing `dir` — the D-pad arrows.
///
/// The same visual language as [`draw_round_face`] (soft fill, soft rim, no hard
/// rasterised edge), just a different outline: the arrow IS the button, so there is no
/// glyph inside it.
///
/// Rendered from three half-plane distances rather than a scanline fill, because that
/// is what makes the edge anti-alias for free and the rim follow the slanted sides at
/// a constant width. The touch target stays the full rect — a D-pad wants a forgiving
/// hit area, and the four arms occupy disjoint cells, so there is nothing to be
/// ambiguous with.
/// The D-pad arrow's HEXAGON face (91차): the six corners from [`hexagon`], rounded
/// and anti-aliased by [`draw_convex_face`].
///
/// ★ ROUNDED CORNERS, as the triangles had ("뾰족한 부분을 살짝만 둥글게 처리해줘").
/// A hexagon's corners are 120°, so a tenth of the short side is plenty — the
/// triangles needed a fifth to soften their 50° tips.
#[allow(clippy::too_many_arguments)]
fn draw_hex_face(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, dir: Dir, fill_color: u32, rim_color: u32, fill_a: u32, rim_a: u32) {
    if r.w < 4 || r.h < 4 {
        return;
    }
    let round_r = (f64::from(r.w.min(r.h)) * 0.10).clamp(1.5, 16.0);
    // Rim thickness scaled off the button so a big D-pad does not get a hairline
    // outline and a small one does not get a fat one.
    let t = (f64::from(r.w.min(r.h)) / 13.0).clamp(1.4, 4.0);
    // The mask is the same for UP and DOWN (and for LEFT and RIGHT): the hexagon is
    // symmetric on both axes, so the key carries the AXIS, not the direction.
    let axis = matches!(dir, Dir::Up | Dir::Down) as u8;
    let verts = hexagon(Rect::new(0, 0, r.w, r.h), dir);
    draw_convex_face(
        buf,
        win_w,
        win_h,
        r,
        &verts,
        (FACE_HEX, axis),
        round_r,
        t,
        fill_color,
        rim_color,
        fill_a,
        rim_a,
    );
}

/// The arrow glyph inside a hexagon arm: a squat rounded triangle pointing `dir`,
/// about two fifths of the arm's length. The hexagon's own six corners are identical, so
/// without this a D-pad reads as four blank tiles.
fn draw_hex_glyph(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, dir: Dir, color: u32, a: u32) {
    // Half-height of the glyph: relative to the arm's LENGTH (the axis it points
    // along), so the four glyphs are the same size on a square seat.
    let along = match dir {
        Dir::Up | Dir::Down => r.h,
        Dir::Left | Dir::Right => r.w,
    };
    // ★ 91차-B: 0.19 of the length (was 0.16) — the hexagon got fat, the mark keeps up.
    let g = f64::from(along) * 0.19;
    if g < 3.0 {
        return;
    }
    // Squat: base 2.3g wide, 1.65g tall — the 2026-08-09 "양쪽끝이 조금더 넓었으면"
    // proportion, so the glyph is the old arrow in miniature. Built in a box of its
    // own, centred on the arm's centre.
    let (hb, tip, base) = (g * 1.15, g, g * 0.65);
    let side = (hb * 2.0 + 4.0).ceil() as u32;
    let (cx, cy) = (f64::from(side) / 2.0, f64::from(side) / 2.0);
    let v = match dir {
        Dir::Up => [(cx, cy - tip), (cx + hb, cy + base), (cx - hb, cy + base)],
        Dir::Down => [(cx, cy + tip), (cx - hb, cy - base), (cx + hb, cy - base)],
        Dir::Left => [(cx - tip, cy), (cx + base, cy - hb), (cx + base, cy + hb)],
        Dir::Right => [(cx + tip, cy), (cx - base, cy + hb), (cx - base, cy - hb)],
    };
    let bbox = Rect::new(
        (f64::from(r.x) + f64::from(r.w) / 2.0 - cx).max(0.0) as u32,
        (f64::from(r.y) + f64::from(r.h) / 2.0 - cy).max(0.0) as u32,
        side,
        side,
    );
    // No rim: the glyph is a flat mark, the rim belongs to the button around it.
    draw_convex_face(
        buf,
        win_w,
        win_h,
        bbox,
        &v,
        (FACE_GLYPH, dir as u8),
        (g * 0.22).max(1.0),
        0.0,
        color,
        color,
        a,
        0,
    );
}

/// Mask-cache key kinds — see [`convex_mask`].
const FACE_HEX: u8 = 1;
const FACE_GLYPH: u8 = 2;

/// `(kind, variant, w, h, round_r * 10, rim_t * 10)`: everything a mask's pixels
/// depend on. Colours and alphas are applied at blit time, so the live HUD and the
/// layout editor share one mask per shape.
type FaceKey = (u8, u8, u32, u32, u32, u32);

/// Rounded-convex-polygon masks, keyed by shape (see [`convex_mask`]). Each pixel is
/// `cover << 8 | edge * cover`, both 0..=255. A handful of entries: two hexagon axes
/// and four glyph directions per seat size.
static FACE_MASKS: std::sync::Mutex<Option<std::collections::HashMap<FaceKey, std::sync::Arc<Vec<u16>>>>> = std::sync::Mutex::new(None);
const FACE_MASKS_MAX: usize = 48;

/// Anti-aliased, ROUNDED convex polygon with a rim band just inside its outline,
/// drawn into `bbox` from vertices given RELATIVE to the box's origin.
///
/// The shape is the polygon ERODED by `round_r` and dilated back: its signed distance
/// is `sd(eroded) - round_r`, which rounds every corner with radius `round_r` exactly
/// (offsetting the edge lines and calling that rounding does not work — opening a
/// convex set with a disc returns the set unchanged; the corners have to be built).
/// The erosion is exact for any convex polygon: each edge line is pushed inward by
/// `round_r` along its normal and adjacent lines are intersected, which is what
/// [`inset_convex`] does. `round_r` is capped just under the inradius so the eroded
/// polygon cannot turn inside out.
///
/// ★ Generalised from the 2026-08-09 triangle painter (which eroded by a similarity
/// about the incentre — exact for a triangle, wrong for anything else) when the arrows
/// became hexagons (91차). The same routine draws the arrow GLYPH inside them.
///
/// ★ The per-pixel distance field is MEMOISED ([`convex_mask`]): four ~180 px
/// hexagons cost ~1.7x what the four triangles did if rasterised every repaint, and
/// the S9 is the low-end reference. A mask depends only on the shape, so the per-frame
/// work is one blend per pixel — the `scaled_icon` pattern from 72차.
#[allow(clippy::too_many_arguments)]
fn draw_convex_face(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    bbox: Rect,
    verts: &[(f64, f64)],
    kind: (u8, u8),
    round_r: f64,
    rim_t: f64,
    fill_color: u32,
    rim_color: u32,
    fill_a: u32,
    rim_a: u32,
) {
    if verts.len() < 3 || bbox.w == 0 || bbox.h == 0 || win_w == 0 || win_h == 0 {
        return;
    }
    let key: FaceKey = (kind.0, kind.1, bbox.w, bbox.h, (round_r * 10.0) as u32, (rim_t * 10.0) as u32);
    let mask = convex_mask(key, bbox.w, bbox.h, verts, round_r, rim_t);
    let x_end = (bbox.x + bbox.w).min(win_w);
    let y_end = (bbox.y + bbox.h).min(win_h);
    for py in bbox.y..y_end {
        let row = (py * win_w) as usize;
        let mrow = ((py - bbox.y) * bbox.w) as usize;
        for px in bbox.x..x_end {
            let m = mask[mrow + (px - bbox.x) as usize];
            let cover = u32::from(m >> 8);
            if cover == 0 {
                continue;
            }
            blend_px(buf, row + px as usize, fill_color, fill_a * cover / 255);
            let edge_cover = u32::from(m & 0xff);
            if rim_a > 0 && edge_cover > 0 {
                blend_px(buf, row + px as usize, rim_color, rim_a * edge_cover / 255);
            }
        }
    }
}

/// The distance-field mask of one rounded convex polygon in a `w x h` box — computed
/// once per `key` and kept (see [`draw_convex_face`]).
fn convex_mask(key: FaceKey, w: u32, h: u32, verts: &[(f64, f64)], round_r: f64, rim_t: f64) -> std::sync::Arc<Vec<u16>> {
    if let Ok(g) = FACE_MASKS.lock()
        && let Some(hit) = g.as_ref().and_then(|m| m.get(&key).cloned())
    {
        return hit;
    }
    let n = verts.len();
    let mut out = vec![0u16; (w * h) as usize];
    // Winding: +1 for counter-clockwise in screen coordinates (y down), -1 otherwise.
    let area2: f64 = (0..n)
        .map(|i| {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % n];
            ax * by - bx * ay
        })
        .sum();
    if area2.abs() < 1e-6 {
        return std::sync::Arc::new(out);
    }
    let wind = area2.signum();
    let (cx, cy) = verts.iter().fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (cx, cy) = (cx / n as f64, cy / n as f64);
    // Inradius bound: the nearest edge to the centroid.
    let r_in = (0..n)
        .map(|i| {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % n];
            let (ex, ey) = (bx - ax, by - ay);
            let len = (ex * ex + ey * ey).sqrt().max(1e-9);
            ((ex * (cy - ay) - ey * (cx - ax)) / len).abs()
        })
        .fold(f64::INFINITY, f64::min);
    let round_r = round_r.min(r_in * 0.9).max(0.0);
    let v = inset_convex(verts, round_r, wind);

    let aa = 1.2f64;
    for py in 0..h {
        let fy = f64::from(py) + 0.5;
        for px in 0..w {
            let fx = f64::from(px) + 0.5;
            // Exact distance to the ERODED polygon (nearest point on each SEGMENT, not
            // on its infinite line — that difference is what makes the corners round).
            let mut best = f64::INFINITY;
            let mut inside = true;
            for i in 0..n {
                let (ax, ay) = v[i];
                let (ex, ey) = (v[(i + 1) % n].0 - ax, v[(i + 1) % n].1 - ay);
                let (wx, wy) = (fx - ax, fy - ay);
                let hh = ((wx * ex + wy * ey) / (ex * ex + ey * ey).max(1e-9)).clamp(0.0, 1.0);
                let (qx, qy) = (wx - ex * hh, wy - ey * hh);
                best = best.min(qx * qx + qy * qy);
                if (ex * wy - ey * wx) * wind < 0.0 {
                    inside = false;
                }
            }
            // `d` > 0 inside the ROUNDED shape, in pixels.
            let d = round_r - best.sqrt() * if inside { -1.0 } else { 1.0 };
            if d < -aa {
                continue;
            }
            let cover = ((d + aa) / (2.0 * aa)).clamp(0.0, 1.0);
            // Rim: a band just inside the outline.
            let edge = if rim_t > 0.0 && d < rim_t {
                1.0 - (d / rim_t).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let c = (cover * 255.0).round() as u16;
            let e = (edge * cover * 255.0).round() as u16;
            out[(py * w + px) as usize] = (c << 8) | e;
        }
    }
    let out = std::sync::Arc::new(out);
    if let Ok(mut g) = FACE_MASKS.lock() {
        let m = g.get_or_insert_with(std::collections::HashMap::new);
        if m.len() >= FACE_MASKS_MAX {
            m.clear();
        }
        m.insert(key, out.clone());
    }
    out
}

/// `verts` with every edge line pushed INWARD by `e`, adjacent lines re-intersected.
/// `wind` is the polygon's winding sign (see [`convex_mask`]). An edge pair that will
/// not intersect (parallel — impossible for adjacent edges of a convex polygon, kept
/// for safety) keeps its original vertex.
fn inset_convex(verts: &[(f64, f64)], e: f64, wind: f64) -> Vec<(f64, f64)> {
    let n = verts.len();
    // Each edge as a line `nx * x + ny * y = c`, with the unit normal pointing INWARD.
    let lines: Vec<(f64, f64, f64)> = (0..n)
        .map(|i| {
            let (ax, ay) = verts[i];
            let (bx, by) = verts[(i + 1) % n];
            let (ex, ey) = (bx - ax, by - ay);
            let len = (ex * ex + ey * ey).sqrt().max(1e-9);
            // The left normal of the edge direction is inward for a positive winding
            // (`wind` says which side that is in y-down screen space).
            let (nx, ny) = (-ey / len * wind, ex / len * wind);
            (nx, ny, nx * ax + ny * ay + e)
        })
        .collect();
    (0..n)
        .map(|i| {
            let (n1x, n1y, c1) = lines[(i + n - 1) % n];
            let (n2x, n2y, c2) = lines[i];
            let det = n1x * n2y - n2x * n1y;
            if det.abs() < 1e-9 {
                return verts[i];
            }
            ((c1 * n2y - c2 * n1y) / det, (n1x * c2 - n2x * c1) / det)
        })
        .collect()
}

fn draw_round_face(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, fill_color: u32, rim_color: u32, fill_a: u32, rim_a: u32) -> (f64, f64, f64) {
    let rad = f64::from(r.w.min(r.h)) / 2.0;
    let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
    let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
    if rad < 1.0 {
        return (cx, cy, rad);
    }
    // Fade the fill out over the last ~9% of the radius so the disc melts into the
    // game instead of ending on a hard rasterised arc.
    blend_disc_soft(buf, win_w, win_h, cx, cy, rad, fill_color, fill_a, (rad * 0.09).max(1.2));
    let t = (rad / 13.0).clamp(1.4, 4.0);
    blend_ring_soft(buf, win_w, win_h, cx, cy, rad - t / 2.0, t, rim_color, rim_a);
    (cx, cy, rad)
}

/// Alpha-blended 2px ring (the opaque [`ring`] is too heavy over the game).
#[allow(dead_code)]
fn blend_ring(buf: &mut [u32], win_w: u32, win_h: u32, cx: f64, cy: f64, r: f64, color: u32, a: u32) {
    if r <= 1.0 {
        return;
    }
    let x0 = (cx - r - 1.0).max(0.0) as u32;
    let x1 = ((cx + r + 1.0).max(0.0) as u32 + 1).min(win_w);
    let y0 = (cy - r - 1.0).max(0.0) as u32;
    let y1 = ((cy + r + 1.0).max(0.0) as u32 + 1).min(win_h);
    for y in y0..y1 {
        let dy = f64::from(y) + 0.5 - cy;
        let row = (y * win_w) as usize;
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt();
            if (d - r).abs() <= 1.4 {
                blend_px(buf, row + x as usize, color, a);
            }
        }
    }
}

fn blend_rect(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, w: u32, h: u32, color: u32, a: u32) {
    let x1 = (x + w).min(win_w);
    let y1 = (y + h).min(win_h);
    for yy in y..y1 {
        let row = (yy * win_w) as usize;
        for xx in x..x1 {
            blend_px(buf, row + xx as usize, color, a);
        }
    }
}

/// Immersive-mode RECTANGULAR button face. Superseded by [`draw_round_face`]
/// (2026-07-30: every key is round); kept for the settings modal's rows.
#[allow(dead_code)]
fn draw_button_face_translucent(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, color: u32) {
    const FILL_A: u32 = 92; // ~36% — user asked for more see-through
    const RIM_A: u32 = 165;
    blend_rect(buf, win_w, win_h, r.x, r.y, r.w, r.h, color, FILL_A);
    let t = 2;
    blend_rect(buf, win_w, win_h, r.x, r.y, r.w, t, BORDER, RIM_A);
    blend_rect(buf, win_w, win_h, r.x, r.y + r.h.saturating_sub(t), r.w, t, BORDER, RIM_A);
    blend_rect(buf, win_w, win_h, r.x, r.y, t, r.h, BORDER, RIM_A);
    blend_rect(buf, win_w, win_h, r.x + r.w.saturating_sub(t), r.y, t, r.h, BORDER, RIM_A);
}

#[allow(dead_code)]
fn draw_button_face(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, color: u32) {
    fill(buf, win_w, win_h, r.x, r.y, r.w, r.h, color);
    stroke(buf, win_w, win_h, (r.x, r.y, r.w, r.h), BORDER);
}

/// The largest axis-aligned square that fits INSIDE the circle inscribed in `r`
/// (side = diameter / sqrt(2), 0.707..). Text sized against this can never be
/// clipped by the disc, whatever the string length.
fn label_box(r: Rect) -> Rect {
    let d = r.w.min(r.h);
    let side = (u64::from(d) * 707 / 1000) as u32;
    Rect::new(
        r.x + (r.w.saturating_sub(side)) / 2,
        r.y + (r.h.saturating_sub(side)) / 2,
        side.max(1),
        side.max(1),
    )
}

fn draw_centered_text(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, t: &str, color: u32) {
    let scale = (r.h / 14).min(r.w / (t.len() as u32 * 6 + 1)).max(2);
    draw_centered_text_in(buf, win_w, win_h, r.x, r.y + (r.h.saturating_sub(7 * scale)) / 2, r.w, t, color, scale);
}

fn draw_centered_text_in(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, w: u32, t: &str, color: u32, scale: u32) {
    let tw = t.len() as u32 * 6 * scale;
    let tx = x + (w.saturating_sub(tw)) / 2;
    draw_text(buf, win_w, win_h, tx, y, scale, t, color);
}

fn draw_gear(buf: &mut [u32], win_w: u32, win_h: u32, g: Rect) {
    if g.w == 0 || g.h == 0 {
        return;
    }
    // Round like every other key (2026-07-30). The cog glyph below is already
    // circular, so it sits inside the disc without clipping.
    draw_round_face(buf, win_w, win_h, g, BTN, BORDER, 92, 165);
    let cx = g.x + g.w / 2;
    let cy = g.y + g.h / 2;
    let r = (g.w.min(g.h) / 2).saturating_sub(g.w / 7).max(6);
    let dirs: [(i64, i64); 8] = [(0, -10), (7, -7), (10, 0), (7, 7), (0, 10), (-7, 7), (-10, 0), (-7, -7)];
    let tooth = (r / 3).max(3);
    for (dx, dy) in dirs {
        let tx = cx as i64 + dx * r as i64 / 10 - tooth as i64 / 2;
        let ty = cy as i64 + dy * r as i64 / 10 - tooth as i64 / 2;
        fill(buf, win_w, win_h, tx.max(0) as u32, ty.max(0) as u32, tooth, tooth, LABEL);
    }
    fill_circle(buf, win_w, win_h, cx, cy, r, LABEL);
    fill_circle(buf, win_w, win_h, cx, cy, r * 11 / 20, BTN);
    fill_circle(buf, win_w, win_h, cx, cy, r / 4, LABEL);
}

// --------------------------------------------------------------------------
// Low-level raster helpers
// --------------------------------------------------------------------------

/// The parchment checker the game's own window bodies carry, drawn row by row.
///
/// ★ 72차 — same pixels, far fewer calls. This was a per-pixel `fill(.., 1, 1, ..)`:
/// for the 오브 picker's body alone that is 1,078,800 loop iterations and 539,400 function
/// calls **per frame**, and `draw_settings` carried a byte-for-byte copy of it. The clip is
/// now computed once per rect instead of once per pixel.
///
/// `(x/2 + y/2) % 2 == 0` is the same predicate as `(x/2) & 1 == (y/2) & 1`, so each row
/// has a fixed phase and the inner test is one comparison.
fn checker(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, color: u32) {
    let x1 = (r.x + r.w).min(win_w);
    let y1 = (r.y + r.h).min(win_h);
    for y in r.y..y1 {
        let phase = (y / 2) & 1;
        let row = (y * win_w) as usize;
        for x in r.x..x1 {
            if (x / 2) & 1 == phase {
                buf[row + x as usize] = color;
            }
        }
    }
}

fn fill(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, w: u32, h: u32, color: u32) {
    let x1 = (x + w).min(win_w);
    let y1 = (y + h).min(win_h);
    for yy in y..y1 {
        let row = (yy * win_w) as usize;
        for xx in x..x1 {
            buf[row + xx as usize] = color;
        }
    }
}

fn stroke(buf: &mut [u32], win_w: u32, win_h: u32, r: (u32, u32, u32, u32), color: u32) {
    let (x, y, w, h) = r;
    let t = 2;
    fill(buf, win_w, win_h, x, y, w, t, color);
    fill(buf, win_w, win_h, x, y + h.saturating_sub(t), w, t, color);
    fill(buf, win_w, win_h, x, y, t, h, color);
    fill(buf, win_w, win_h, x + w.saturating_sub(t), y, t, h, color);
}

/// 하늘성 입장 연출: the whole window, one colour.
///
/// HOST-DRAWN on purpose (user decision 2026-09-10). The alternative the player floated
/// was alternating with the game's OWN game-over screen, and that screen lives on the
/// character-death path: putting the guest into that state to borrow the pixels has no
/// guaranteed way back, and the save is what would pay for it.
pub fn tower_flash(buf: &mut [u32], win_w: u32, win_h: u32, dark: bool) {
    fill(buf, win_w, win_h, 0, 0, win_w, win_h, if dark { 0xff05_0505 } else { 0xffff_ffff });
}

fn fill_circle(buf: &mut [u32], win_w: u32, win_h: u32, cx: u32, cy: u32, r: u32, color: u32) {
    let r2 = (r * r) as i64;
    let ri = r as i64;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if dx * dx + dy * dy <= r2 {
                let px = cx as i64 + dx;
                let py = cy as i64 + dy;
                if px >= 0 && py >= 0 && (px as u32) < win_w && (py as u32) < win_h {
                    buf[(py as u32 * win_w + px as u32) as usize] = color;
                }
            }
        }
    }
}

fn draw_text(buf: &mut [u32], win_w: u32, win_h: u32, mut x: u32, y: u32, scale: u32, text: &str, color: u32) {
    for ch in text.chars() {
        draw_glyph(buf, win_w, win_h, x, y, scale, ch, color);
        x += 6 * scale;
    }
}

// ── the GAME'S own font (60차) ───────────────────────────────────────────────
//
// `draw_text` above is a 5x7 ASCII bitmap, which is why every panel in this file
// used to be labelled in English. `crate::gamefont` draws with the typeface the game
// itself uses — Korean included, all 2350 KS X 1001 syllables — so anything the
// player reads can now be written in Korean and look like it belongs to the game.
//
// Cells are 11 px tall; `scale` multiplies both axes, so scale 2 is a 22 px line.

/// Width of `text` in the game's font at `scale`, in screen pixels.
fn game_text_w(text: &str, scale: u32) -> u32 {
    crate::gamefont::width(text) as u32 * scale
}

/// Largest scale that fits `text` inside `w` x `h`, at least 1.
fn game_fit_scale(text: &str, w: u32, h: u32) -> u32 {
    let by_h = (h / crate::gamefont::KOR_H as u32).max(1);
    let need = crate::gamefont::width(text).max(1) as u32;
    (w / need).min(by_h).max(1)
}

/// Draw `text` at `(x, y)` in the game's font. Returns the pen position after it.
fn draw_game_text(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, scale: u32, text: &str, color: u32) -> u32 {
    let mut pen = x;
    for cell in crate::gamefont::layout(text) {
        for (r, bits) in cell.rows.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for c in 0..crate::gamefont::KOR_W as u32 {
                if bits & (1 << (15 - c)) != 0 {
                    fill(buf, win_w, win_h, pen + c * scale, y + r as u32 * scale, scale, scale, color);
                }
            }
        }
        pen += cell.advance as u32 * scale;
    }
    pen
}

/// [`draw_game_text`] blended at `a` (0..=255; 255 draws exactly what it does) — for text
/// that fades (the side notes).
#[allow(clippy::too_many_arguments)]
fn draw_game_text_blend(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, scale: u32, text: &str, color: u32, a: u32) -> u32 {
    let a = a.min(255);
    let mut pen = x;
    for cell in crate::gamefont::layout(text) {
        for (r, bits) in cell.rows.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for c in 0..crate::gamefont::KOR_W as u32 {
                if bits & (1 << (15 - c)) != 0 {
                    blend_rect(buf, win_w, win_h, pen + c * scale, y + r as u32 * scale, scale, scale, color, a);
                }
            }
        }
        pen += cell.advance as u32 * scale;
    }
    pen
}

/// Draw `text` centred in `[x, x + w)` at `y`.
fn draw_game_text_centered(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, w: u32, scale: u32, text: &str, color: u32) {
    let tw = game_text_w(text, scale);
    let ox = x + w.saturating_sub(tw) / 2;
    draw_game_text(buf, win_w, win_h, ox, y, scale, text, color);
}

/// Width of `text` at a FRACTIONAL scale, in window px.
fn game_text_w_frac(text: &str, s: f64) -> f64 {
    crate::gamefont::width(text) as f64 * s
}

/// [`draw_game_text`] at a FRACTIONAL scale, for the native window replicas.
///
/// The game itself reaches the screen through a fractional nearest-neighbour
/// upscale (496x240 → 2340x1080 is x4.5), so one glyph pixel lands as `floor(s)`
/// or `ceil(s)` device pixels depending on position. Rounding every edge independently
/// reproduces exactly that, which is what makes the replica's text sit at the same
/// size as the game's own instead of one integer step smaller.
fn draw_game_text_frac(buf: &mut [u32], win_w: u32, win_h: u32, x: f64, y: f64, s: f64, text: &str, color: u32) -> f64 {
    let mut pen = x;
    for cell in crate::gamefont::layout(text) {
        for (r, bits) in cell.rows.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            let y0 = (y + r as f64 * s).round().max(0.0) as u32;
            let y1 = (y + (r + 1) as f64 * s).round().max(0.0) as u32;
            for c in 0..crate::gamefont::KOR_W {
                if bits & (1 << (15 - c)) != 0 {
                    let x0 = (pen + c as f64 * s).round().max(0.0) as u32;
                    let x1 = (pen + (c + 1) as f64 * s).round().max(0.0) as u32;
                    fill(buf, win_w, win_h, x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0), color);
                }
            }
        }
        pen += cell.advance as f64 * s;
    }
    pen
}

fn draw_glyph(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, scale: u32, ch: char, color: u32) {
    for (r, bits) in glyph(ch).iter().enumerate() {
        for c in 0..5u32 {
            if bits & (1 << (4 - c)) != 0 {
                fill(buf, win_w, win_h, x + c * scale, y + r as u32 * scale, scale, scale, color);
            }
        }
    }
}

fn draw_arrow(buf: &mut [u32], win_w: u32, win_h: u32, cx: u32, cy: u32, s: u32, dir: Dir, color: u32) {
    let (cx, cy, s) = (cx as i64, cy as i64, s as i64);
    for dy in -s..=s {
        for dx in -s..=s {
            let inside = match dir {
                Dir::Up => dx.abs() <= (dy + s) / 2,
                Dir::Down => dx.abs() <= (s - dy) / 2,
                Dir::Left => dy.abs() <= (dx + s) / 2,
                Dir::Right => dy.abs() <= (s - dx) / 2,
            };
            if inside {
                let px = cx + dx;
                let py = cy + dy;
                if px >= 0 && py >= 0 && (px as u32) < win_w && (py as u32) < win_h {
                    buf[(py as u32 * win_w + px as u32) as usize] = color;
                }
            }
        }
    }
}

/// Minimal 5x7 bitmap font (low 5 bits per row, MSB = leftmost column).
fn glyph(ch: char) -> [u8; 7] {
    // Uppercase-only table; fold so lowercase labels render too. Anything outside
    // it (Hangul, punctuation not listed) is a BLANK cell — keep overlay labels
    // inside this set. Device-confirmed 2026-07-28: a missing glyph is silent.
    match ch.to_ascii_uppercase() {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        '*' => [0b00000, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00000],
        '#' => [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b01010, 0b00001],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100],
        '%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        _ => [0; 7],
    }
}

// --------------------------------------------------------------------------
// 게임패드 HUD — position badges (59차)
// --------------------------------------------------------------------------
//
// Two visual languages, picked per control family (reworked 60차 — the player could
// not read the original all-abstract glyphs: "작대기 4개로 중에 노란색으로 들어온
// 버튼이 뭘 의미하는건지 모르겠어"):
//
// * FACE buttons: a diamond of four dots with the bound one lit. Positional on
//   purpose — the letter printed on the bottom face button is `A` on an Xbox pad,
//   `B` on a Switch pad and nothing at all on a PlayStation pad (see `padhud.rs`),
//   so a letter would lie on two of the three families while the diamond matches
//   the buttons under the thumb on all of them.
// * SHOULDERS and STICK PRESSES: the NAME — `L1 L2 R1 R2 L3 R3`. Unlike the face
//   letters these names conflict with nothing any pad prints (Xbox says LB/LT,
//   Switch says L/ZL — different words, same meaning, no collision), and the
//   abstract "2x2 bars, one lit" they replace had no physical anchor to read
//   against: shoulders are not visible while playing.
// * MIDDLE buttons: two pills side by side with the bound one lit, as they sit
//   either side of the pad's centre.

/// Badge fill and rim. Near-opaque so the glyph reads over bright skill art, and the
/// same slate/steel family as the rest of the HUD.
const BADGE_FILL: u32 = 0xff08_0b14;
const BADGE_RIM: u32 = 0xff9f_b6d4;
/// The lit member of a set (warm, matching the action disc's gold ring) and the
/// unlit ones (the HUD's dim label grey).
const BADGE_LIT: u32 = 0xffff_e0a0;
const BADGE_UNLIT: u32 = 0xff56_607a;

/// Paint the position badge for `pos` on the control at `r`.
pub fn draw_pad_badge(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, pos: crate::padhud::PadPos) {
    let Some(b) = crate::padhud::badge_at(r.x, r.y, r.w, r.h, win_w, win_h) else {
        return;
    };
    blend_disc(buf, win_w, win_h, b.cx, b.cy, b.r, BADGE_FILL, 225);
    blend_ring(buf, win_w, win_h, b.cx, b.cy, b.r, BADGE_RIM, 190);
    draw_pad_glyph(buf, win_w, win_h, b, pos);
}

/// Badge a control by what it DOES rather than by a guest key — the host-owned
/// controls (PAGE, SAVE) have no `KeyCode` to look up.
pub fn draw_pad_badge_action(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, action: crate::gamepad::PadAction) {
    if let Some(pos) = crate::gamepad::pos_for_action(action) {
        draw_pad_badge(buf, win_w, win_h, r, pos);
    }
}

/// Badge every control in the layout and every skill plate, in ONE pass after the
/// whole HUD is drawn.
///
/// A separate pass and not a step inside each control's own draw call: several
/// controls paint their face twice (the secondary weapon gets a round face from
/// `draw_controls` and its icon from `draw_secondary`), so a badge drawn with the
/// face would end up underneath. Badges are the layer that answers "which button is
/// this?" — they go on top of everything.
pub fn draw_pad_badges(buf: &mut [u32], win_w: u32, win_h: u32, layout: &Layout, quick: &[Rect], slots: &[crate::QuickSlot]) {
    for b in &layout.buttons {
        // The walk keys are not badged. In pad mode the arrows are not drawn at all
        // (see `layout_immersive`), and in the opaque portrait layouts — which have a
        // permanent D-pad — four copies of the same hat glyph is noise, not help.
        if b.dpad || b.face == Face::None {
            continue;
        }
        if let Some(pos) = crate::gamepad::pos_for_key(b.key) {
            draw_pad_badge(buf, win_w, win_h, b.rect, pos);
        }
    }
    // The plates. A pad button NAMES a plate since 79차-B (`PadAction::Quick`), so the
    // badge is a straight reverse lookup of that — no residency in it at all, which is
    // what makes the badge stable while the window shuffles underneath. A
    // `wie_gamepad.txt` remap moves the badge with the binding.
    //
    // ★ A plate nothing is bound to gets NO badge, which is the truth: a standard pad
    // has no seat left for plates 6 and 7 by default (`gamepad::default_button_action`),
    // and the player adds one with a `Q6` line if they want it.
    for (i, r) in quick.iter().enumerate() {
        let plate = match slots.get(i) {
            Some(s) => s.slot,
            None => i as u8,
        };
        if let Some(pos) = crate::gamepad::pos_for_action(crate::gamepad::PadAction::Quick(plate)) {
            draw_pad_badge(buf, win_w, win_h, *r, pos);
        }
    }
}

/// A short ASCII name centred in the badge, in the lit gold — for the controls whose
/// name IS the clearest picture (L1/L2/R1/R2/L3/R3). Spans ~1.5x the badge radius so
/// two characters sit inside the disc with the rim clear at every badge size.
fn badge_label(buf: &mut [u32], win_w: u32, win_h: u32, b: crate::padhud::Badge, label: &str) {
    // Visible width in font columns: 5 per glyph + 1 gap between glyphs.
    let cols = (label.chars().count() as u32 * 6).saturating_sub(1).max(1);
    let scale = ((b.r * 1.5 / f64::from(cols)).floor() as u32).max(1);
    let x = (b.cx - f64::from(cols * scale) / 2.0).max(0.0) as u32;
    let y = (b.cy - f64::from(7 * scale) / 2.0).max(0.0) as u32;
    draw_text(buf, win_w, win_h, x, y, scale, label, BADGE_LIT);
}

fn draw_pad_glyph(buf: &mut [u32], win_w: u32, win_h: u32, b: crate::padhud::Badge, pos: crate::padhud::PadPos) {
    use crate::padhud::{FacePos, PadPos};
    // A dot/bar for a member of the set: `lit` members are bigger as well as
    // brighter, so the badge still reads on a screen the player is looking past.
    let dot = |buf: &mut [u32], dx: f64, dy: f64, lit: bool| {
        let rr = b.r * if lit { 0.30 } else { 0.16 };
        let col = if lit { BADGE_LIT } else { BADGE_UNLIT };
        fill_disc(buf, win_w, win_h, b.cx + dx, b.cy + dy, rr, col);
    };
    // Horizontal bar (shoulders, middle buttons), centred on `(dx, dy)`.
    let bar = |buf: &mut [u32], dx: f64, dy: f64, half_w: f64, half_h: f64, lit: bool| {
        let col = if lit { BADGE_LIT } else { BADGE_UNLIT };
        let x = (b.cx + dx - half_w).max(0.0) as u32;
        let y = (b.cy + dy - half_h).max(0.0) as u32;
        let w = (half_w * 2.0).max(1.0) as u32;
        let h = (half_h * 2.0).max(1.0) as u32;
        fill(buf, win_w, win_h, x, y, w, h, col);
    };
    match pos {
        // Diamond of four: up, down, left, right — exactly how the buttons sit.
        PadPos::Face(f) => {
            let d = b.r * 0.46;
            dot(buf, 0.0, -d, f == FacePos::Up);
            dot(buf, 0.0, d, f == FacePos::Down);
            dot(buf, -d, 0.0, f == FacePos::Left);
            dot(buf, d, 0.0, f == FacePos::Right);
        }
        // The name, spelled out (60차): the original "2x2 bars, one lit" glyph was
        // unreadable in play. `1` = the upper row (bumper), `2` = the trigger.
        PadPos::Shoulder { right, lower } => {
            let label = match (right, lower) {
                (false, false) => "L1",
                (false, true) => "L2",
                (true, false) => "R1",
                (true, true) => "R2",
            };
            badge_label(buf, win_w, win_h, b, label);
        }
        // Two pills side by side, as they sit either side of the pad's centre.
        PadPos::Middle { right } => {
            let dx = b.r * 0.42;
            let hw = b.r * 0.30;
            bar(buf, -dx, 0.0, hw, b.r * 0.14, !right);
            bar(buf, dx, 0.0, hw, b.r * 0.14, right);
        }
        // Stick press-ins get their name too (60차): `L3`/`R3` is the same
        // vocabulary as the shoulders, and two abstract rings read no better than
        // the four bars did.
        PadPos::StickPress { right } => {
            badge_label(buf, win_w, win_h, b, if right { "R3" } else { "L3" });
        }
        // The hat as a whole: a cross, all four arms lit.
        PadPos::Dpad => {
            let arm = b.r * 0.62;
            let t = (b.r * 0.16).max(1.0);
            bar(buf, 0.0, 0.0, arm, t, true);
            bar(buf, 0.0, 0.0, t, arm, true);
        }
    }
}

// ── 패드 버튼 설정 — the in-game HUD, lit (2026-10-09, second form) ─────────────────
//
// User, 2026-10-09 — the same day the first screen (a list, in the orb list's widget) went
// out: *"패드 설정버튼 화면을 줄을 누르고 패드버튼을 누르는게 아니라 그냥 실제 인게임 화면상에
// 버튼들을 하이라이트 처리하고 각 버튼을 눌르고 패드 버튼을 누르는 방식으로 변경해줘. hero2도
// 마찬가지야"*.
//
// So there is no list. The screen is the FIELD HUD as it is played — the same controls, in
// the seats the resolver gives them, painted by the same painters (`draw_controls`,
// `draw_quickbar`, `draw_secondary`, `draw_top_label`, the PAGE mask) — over the live frame
// dimmed, and every control a pad button can be given is ringed in gold and stamped with the
// badge of the button that fires it IN THE DRAFT (the table being edited, not the one in
// force). Tap a control, press the pad button. A function with no control on screen (a plate
// the player has hidden) is a CELL in the screen's own small window, which also carries the
// two status lines and 취소 · 기본값 · 저장.
//
// ★ The FIELD set, as the layout editor shows it: the gear only exists OFF the field, so the
//   HUD this is opened from is never the one being configured.
// ⛔ Not drawn: the walk control and the minimap panel. A direction is not remappable
//   (`padmap::capturable`), and `*` rides the MAP tab — where its badge sits in play.

/// The action disc's gold (the 퀵바등록 rings'): "a control you can give a button".
const PM_OFFER: u32 = 0xffe0_9a3c;
/// The badges' lit gold: "this one is waiting for a button" / "this is the one you changed".
const PM_SEL: u32 = BADGE_LIT;
/// "What the pad button you just pressed does" — a bare press, nothing picked.
const PM_PROBE: u32 = 0xff6c_d0ff;
/// The second status line: the game windows' cream title ink, on the window's body.
const PM_HINT: u32 = G_TITLE;
/// The window's width in the game's logical px (the game's own boxes are 164..176): its
/// first line has to hold a function, `=` and a button's whole name.
const PM_W: f64 = 236.0;
/// The title strip, the body's margin, a status line's pitch (the message box's 13).
const PM_TITLE_H: f64 = 15.0;
const PM_PAD: f64 = 4.0;
const PM_LINE_H: f64 = 13.0;
/// A chip (a function with no control on screen), and the air between two.
const PM_CHIP_H: f64 = 15.0;
const PM_CHIP_GAP: f64 = 4.0;
/// The band of three buttons — the native boxes' OK band ([`NP_OK_H`]), split.
const PM_BAND_H: f64 = NP_OK_H;

/// How a target is drawn and lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadTargetShape {
    /// A disc control: OK, CLR, the 7 key, a plate, PAGE.
    Round,
    /// A flat one: SAVE, MAP, a key of the classic keypad layouts.
    Square,
    /// No control on screen: a cell in the window ([`PadMapPanel`]).
    Chip,
}

/// One thing the pad screen offers a button for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadTarget {
    /// Index into `gamepad::REMAP_ROWS`.
    pub row: usize,
    pub rect: Rect,
    pub shape: PadTargetShape,
}

/// The pad screen's own window, in window px ([`padmap_panel`]).
#[derive(Clone, Debug)]
pub struct PadMapPanel {
    pub panel: Rect,
    title: Rect,
    body: Rect,
    /// The cells of the functions with no control on screen and their captions, in the
    /// order their targets follow the on-screen ones in [`PadMapScene::targets`].
    chips: Vec<(Rect, String)>,
    cancel: Rect,
    reset: Rect,
    save: Rect,
    /// Window px per logical px.
    s: f64,
}

/// Everything the pad screen draws and answers to, for one window size: the FIELD HUD's own
/// pieces, the targets on them, and the window. Rebuilt per frame and per touch from the
/// resolver, so where a control is lit is always where it is played.
pub struct PadMapScene {
    /// The FIELD layout's buttons that are pad functions — OK, CLR, the 7 key (and on the
    /// classic keypad layouts `*`); no gear, no arrows, no stick.
    layout: Layout,
    /// The 7 key's seat, for `draw_secondary`'s weapon art (immersive only, as in play).
    secondary: Option<Rect>,
    /// The plates by plate index — zero-sized = hidden (`quick_buttons`' contract).
    quick: Vec<Rect>,
    page: Option<Rect>,
    save: Option<Rect>,
    map: Option<Rect>,
    /// The controls on screen first, then the chips.
    pub targets: Vec<PadTarget>,
    pub panel: PadMapPanel,
}

/// What the pad screen lights — `App`'s state, resolved before the surface is borrowed.
pub struct PadMapLook<'a> {
    /// One per target, in [`PadMapScene::targets`] order: the position of the button that
    /// fires it in the DRAFT (`None` = no button, or one with no known position).
    pub badges: &'a [Option<crate::padhud::PadPos>],
    /// The function picked last (pale gold), and the one a bare press just named (blue).
    pub sel: Option<usize>,
    pub probe: Option<usize>,
    /// The picked control's breath while it waits for a button, 0..=1; `None` = not waiting.
    pub pulse: Option<f64>,
    pub title: &'a str,
    pub lines: [&'a str; 2],
    /// Does the draft differ from the built-in layout — is there anything for 기본값 to do?
    pub can_reset: bool,
    /// Does it differ from the table in force — has 저장 anything to keep? (Lit gold.)
    pub dirty: bool,
}

/// What a touch on the pad screen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMapHit {
    Cancel,
    Reset,
    Save,
    /// A control on screen, or a chip: the `gamepad::REMAP_ROWS` index it offers.
    Row(usize),
    /// The window, but nothing on it.
    Panel,
}

/// A chip's caption: the function's name — and why it has no control on screen when that
/// was the player's own choice (a hidden plate).
fn padmap_chip_label(row: usize) -> String {
    let (action, label) = crate::gamepad::REMAP_ROWS[row];
    match action {
        crate::gamepad::PadAction::Quick(n) if crate::quickpage::plate_hidden(crate::quick_hidden(), n) => format!("{label} (숨김)"),
        _ => label.to_string(),
    }
}

/// The pad screen for this window: the FIELD controls a pad button can be given, in their
/// seats, and a chip in the window for every function left without one.
pub fn padmap_scene(win_w: u32, win_h: u32, joystick: bool) -> PadMapScene {
    use crate::gamepad::{PadAction, remap_row};
    use crate::placement::CtrlId;
    let mode = Mode::Field;
    let mut layout = layout(win_w, win_h, joystick, mode, false);
    // The plates BEFORE the gear goes: the classic layouts seat them below it.
    let quick = quick_buttons(layout.game, layout.gear, crate::quickpage::PLATES, mode);
    layout.buttons.retain(|b| !b.dpad && b.face != Face::None && remap_row(PadAction::Key(b.key)).is_some());
    layout.joystick = None;
    layout.gear = Rect::new(0, 0, 0, 0);
    let immersive = crate::immersive_active() && win_w > win_h;

    let mut targets: Vec<PadTarget> = Vec::new();
    // First come, first served — one control per function (a classic layout's `*` key and a
    // MAP tab never both stand, but the rule should not depend on that).
    let mut offer = |action: PadAction, rect: Rect, shape: PadTargetShape| {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        if let Some(row) = remap_row(action)
            && !targets.iter().any(|t| t.row == row)
        {
            targets.push(PadTarget { row, rect, shape });
        }
    };
    let mut secondary = None;
    for b in &layout.buttons {
        if immersive && b.key == KeyCode::NUM7 {
            secondary = Some(b.rect);
        }
        let shape = match b.face {
            Face::Action | Face::Round | Face::Solid => PadTargetShape::Round,
            _ => PadTargetShape::Square,
        };
        offer(PadAction::Key(b.key), b.rect, shape);
    }
    for (i, r) in quick.iter().enumerate() {
        offer(PadAction::Quick(i as u8), *r, PadTargetShape::Round);
    }
    // The three host-owned controls of the immersive HUD, where the resolver put them.
    let (mut page, mut save, mut map) = (None, None, None);
    if immersive {
        let placed = crate::placement::resolve(mode, win_w, win_h);
        let at = |id: CtrlId| placed.iter().find(|(c, _)| *c == id).map(|(_, r)| *r).filter(|r| r.w > 0 && r.h > 0);
        page = at(CtrlId::QuickPage);
        save = at(CtrlId::QuickSave);
        map = at(CtrlId::MinimapToggle);
        if let Some(r) = page {
            offer(PadAction::QuickPage, r, PadTargetShape::Round);
        }
        if let Some(r) = save {
            offer(PadAction::QuickSave, r, PadTargetShape::Square);
        }
        // `*` — the key the MAP tab's badge names in play (the panel itself is not drawn here).
        if let Some(r) = map {
            offer(PadAction::Key(KeyCode::STAR), r, PadTargetShape::Square);
        }
    }

    let avoid: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
    let missing: Vec<usize> = (0..crate::gamepad::REMAP_ROWS.len()).filter(|row| !targets.iter().any(|t| t.row == *row)).collect();
    let labels: Vec<String> = missing.iter().map(|row| padmap_chip_label(*row)).collect();
    let panel = padmap_panel(win_w, win_h, &avoid, &labels);
    for (row, (rect, _)) in missing.iter().zip(&panel.chips) {
        targets.push(PadTarget {
            row: *row,
            rect: *rect,
            shape: PadTargetShape::Chip,
        });
    }
    PadMapScene {
        layout,
        secondary,
        quick,
        page,
        save,
        map,
        targets,
        panel,
    }
}

/// The window: centred across the screen, as high as it can sit without covering a control —
/// the first free spot from the top (under the top-row tabs, as a rule) — or the middle of
/// the screen when every spot covers one.
fn padmap_panel(win_w: u32, win_h: u32, avoid: &[Rect], chips: &[String]) -> PadMapPanel {
    let (_, lh) = crate::logical();
    // The game's own scale, unless the window is too narrow for the panel at it (portrait):
    // then the panel takes the width and everything in it shrinks with it.
    let s = (f64::from(win_h) / f64::from(lh.max(1))).min(f64::from(win_w) * 0.94 / PM_W).max(0.25);
    let frame = NP_OUTLINE + NP_RING + 1.0; // outline, ring, shine
    let inner = PM_W - frame * 2.0 - PM_PAD * 2.0;
    // The chips, wrapped into rows: caption + a badge-sized square + margins.
    let chip_w = |t: &str| (crate::gamefont::width(t) as f64 + PM_CHIP_H + 7.0).min(inner);
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut used = f64::INFINITY;
    for t in chips {
        let w = chip_w(t);
        if used + PM_CHIP_GAP + w > inner {
            rows.push(Vec::new());
            used = -PM_CHIP_GAP;
        }
        used += PM_CHIP_GAP + w;
        if let Some(row) = rows.last_mut() {
            row.push(w);
        }
    }
    let chip_block = rows.len() as f64 * (PM_CHIP_H + PM_CHIP_GAP);
    let body_l = PM_PAD + PM_LINE_H * 2.0 + chip_block + PM_PAD;
    let total_l = frame + PM_TITLE_H + body_l + 1.0 + PM_BAND_H + NP_OUTLINE;
    let w = ((PM_W * s).round() as u32).min(win_w);
    let h = ((total_l * s).round() as u32).min(win_h);
    let x0 = win_w.saturating_sub(w) / 2;
    let gap = (3.0 * s).round().max(1.0) as u32;
    let step = (2.0 * s).round().max(1.0) as u32;
    let free = |y: u32| {
        let r = Rect::new(x0, y, w, h).grown(gap);
        !avoid.iter().any(|a| a.intersects(&r))
    };
    let y0 = (0u32..)
        .map(|k| gap + k * step)
        .take_while(|y| y + h + gap <= win_h)
        .find(|y| free(*y))
        .unwrap_or(win_h.saturating_sub(h) / 2);
    let y_at = |l: f64| y0 + (l * s).round() as u32;
    let fx = (frame * s).round() as u32;
    let o = (NP_OUTLINE * s).round().max(1.0) as u32;
    let title = Rect::new(x0 + fx, y_at(frame), w.saturating_sub(fx * 2), y_at(frame + PM_TITLE_H).saturating_sub(y_at(frame)));
    let body_top = frame + PM_TITLE_H;
    let body = Rect::new(title.x, y_at(body_top), title.w, y_at(body_top + body_l).saturating_sub(y_at(body_top)));
    let band_top = body_top + body_l + 1.0;
    let band = Rect::new(x0 + o, y_at(band_top), w.saturating_sub(o * 2), y_at(band_top + PM_BAND_H).saturating_sub(y_at(band_top)));
    // Three cells with one logical px of outline between them.
    let div = s.round().max(1.0) as u32;
    let cw = band.w.saturating_sub(div * 2) / 3;
    let cell = |i: u32| {
        let x = band.x + i * (cw + div);
        let w = if i == 2 { (band.x + band.w).saturating_sub(x) } else { cw };
        Rect::new(x, band.y, w, band.h)
    };
    let mut placed_chips = Vec::with_capacity(chips.len());
    let mut labels = chips.iter();
    for (k, row) in rows.iter().enumerate() {
        let row_w: f64 = row.iter().sum::<f64>() + PM_CHIP_GAP * (row.len() as f64 - 1.0);
        let mut x_l = frame + PM_PAD + (inner - row_w).max(0.0) / 2.0;
        let top_l = body_top + PM_PAD + PM_LINE_H * 2.0 + PM_CHIP_GAP / 2.0 + k as f64 * (PM_CHIP_H + PM_CHIP_GAP);
        for cw_l in row {
            let r = Rect::new(x0 + (x_l * s).round() as u32, y_at(top_l), (cw_l * s).round() as u32, (PM_CHIP_H * s).round() as u32);
            placed_chips.push((r, labels.next().cloned().unwrap_or_default()));
            x_l += cw_l + PM_CHIP_GAP;
        }
    }
    PadMapPanel {
        panel: Rect::new(x0, y0, w, h),
        title,
        body,
        chips: placed_chips,
        cancel: cell(0),
        reset: cell(1),
        save: cell(2),
        s,
    }
}

/// What a touch at `(x, y)` is on the pad screen. `None` = the game picture.
pub fn padmap_hit(sc: &PadMapScene, x: f64, y: f64) -> Option<PadMapHit> {
    let p = &sc.panel;
    // The window is drawn over whatever it covers, so it answers first.
    if p.panel.contains(x, y) {
        if p.cancel.contains(x, y) {
            return Some(PadMapHit::Cancel);
        }
        if p.reset.contains(x, y) {
            return Some(PadMapHit::Reset);
        }
        if p.save.contains(x, y) {
            return Some(PadMapHit::Save);
        }
        let chip = sc.targets.iter().find(|t| t.shape == PadTargetShape::Chip && t.rect.contains(x, y));
        return Some(chip.map_or(PadMapHit::Panel, |t| PadMapHit::Row(t.row)));
    }
    // Controls may overlap each other (2026-08-07); the smallest one under the finger is the
    // one a thumb can only have meant on purpose.
    sc.targets
        .iter()
        .filter(|t| t.shape != PadTargetShape::Chip && t.rect.contains(x, y))
        .min_by_key(|t| u64::from(t.rect.w) * u64::from(t.rect.h))
        .map(|t| PadMapHit::Row(t.row))
}

/// How one target is lit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PmLit {
    Offer,
    Sel,
    Probe,
}

fn padmap_lit(row: usize, look: &PadMapLook) -> PmLit {
    if look.sel == Some(row) {
        PmLit::Sel
    } else if look.probe == Some(row) {
        PmLit::Probe
    } else {
        PmLit::Offer
    }
}

/// A rectangle's outline `t` px thick, blended.
fn frame_blend(buf: &mut [u32], win_w: u32, win_h: u32, r: Rect, t: u32, color: u32, a: u32) {
    let t = t.min(r.w / 2).min(r.h / 2).max(1);
    blend_rect(buf, win_w, win_h, r.x, r.y, r.w, t, color, a);
    blend_rect(buf, win_w, win_h, r.x, (r.y + r.h).saturating_sub(t), r.w, t, color, a);
    blend_rect(buf, win_w, win_h, r.x, r.y + t, t, r.h.saturating_sub(t * 2), color, a);
    blend_rect(buf, win_w, win_h, (r.x + r.w).saturating_sub(t), r.y + t, t, r.h.saturating_sub(t * 2), color, a);
}

/// Ring one control: gold = it can be given a button; pale gold and washed = the one picked
/// (breathing while it waits for the button); blue = what the last bare press fires.
fn padmap_light(buf: &mut [u32], win_w: u32, win_h: u32, t: &PadTarget, lit: PmLit, pulse: Option<f64>) {
    let r = t.rect;
    let colour = match lit {
        PmLit::Offer => PM_OFFER,
        PmLit::Sel => PM_SEL,
        PmLit::Probe => PM_PROBE,
    };
    let breath = pulse.unwrap_or(0.0);
    if t.shape == PadTargetShape::Round {
        let rad = f64::from(r.w.min(r.h)) / 2.0;
        let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
        let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
        let th = (rad / 7.0).max(2.5);
        if lit == PmLit::Sel {
            blend_disc_soft(buf, win_w, win_h, cx, cy, rad, PM_SEL, 60 + (70.0 * breath) as u32, (rad * 0.08).max(1.0));
            if pulse.is_some() {
                blend_ring_soft(buf, win_w, win_h, cx, cy, rad + th * (1.4 + 1.6 * breath), th * 0.8, PM_SEL, (220.0 * (1.0 - 0.7 * breath)) as u32);
            }
        }
        let (thick, a) = if lit == PmLit::Offer { (th, 215) } else { (th * 1.5, 255) };
        blend_ring_soft(buf, win_w, win_h, cx, cy, rad + thick * 0.15, thick, colour, a);
    } else {
        let th = (r.w.min(r.h) / 10).max(3);
        if lit == PmLit::Sel {
            blend_rect(buf, win_w, win_h, r.x, r.y, r.w, r.h, PM_SEL, 50 + (70.0 * breath) as u32);
            if pulse.is_some() {
                let g = th * 2 + (f64::from(th) * 1.5 * breath) as u32;
                frame_blend(buf, win_w, win_h, r.grown(g), (th * 2 / 3).max(2), PM_SEL, (220.0 * (1.0 - 0.7 * breath)) as u32);
            }
        }
        let (t2, a) = if lit == PmLit::Offer { (th, 215) } else { (th * 3 / 2, 255) };
        frame_blend(buf, win_w, win_h, r.grown(t2 / 2), t2, colour, a);
    }
}

/// The largest fractional scale, at most `s`, at which `text` fits `avail` window px.
fn fit_frac(text: &str, avail: f64, s: f64) -> f64 {
    let w1 = crate::gamefont::width(text).max(1) as f64;
    s.min(avail / w1).max(0.5)
}

/// Draw the pad screen: the HUD as it is played, every target lit and badged, the window.
pub fn draw_padmap(buf: &mut [u32], win_w: u32, win_h: u32, sc: &PadMapScene, slots: &[QuickSlot], secondary: Option<IconRef>, look: &PadMapLook) {
    // The live frame dimmed — lighter than the layout editor's 45 %: this is the game with
    // its buttons lit, not a blueprint of it.
    for px in buf.iter_mut() {
        let v = *px;
        let r = ((v >> 16) & 0xff) * 55 / 100;
        let g = ((v >> 8) & 0xff) * 55 / 100;
        let b = (v & 0xff) * 55 / 100;
        *px = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }
    // The HUD as it is played, by its own painters.
    draw_controls(buf, win_w, win_h, &sc.layout, &HashSet::new(), None);
    if let Some(r) = sc.secondary {
        draw_secondary(buf, win_w, win_h, r, secondary);
    }
    draw_quickbar(buf, win_w, win_h, &sc.quick, slots, None, &[]);
    if let Some(r) = sc.page {
        draw_mask_button_spanned(buf, win_w, win_h, r, ui_icons::Icon::QuickPage, false, QUICKPAGE_ICON_SPAN);
    }
    if let Some(r) = sc.save {
        draw_top_label(buf, win_w, win_h, r, "SAVE", false);
    }
    if let Some(r) = sc.map {
        draw_top_label(buf, win_w, win_h, r, "MAP", false);
    }
    for t in sc.targets.iter().filter(|t| t.shape != PadTargetShape::Chip) {
        padmap_light(buf, win_w, win_h, t, padmap_lit(t.row, look), look.pulse);
    }
    // The badges over everything — the in-play badge pass's rule.
    for (t, b) in sc.targets.iter().zip(look.badges) {
        if t.shape != PadTargetShape::Chip
            && let Some(pos) = b
        {
            draw_pad_badge(buf, win_w, win_h, t.rect, *pos);
        }
    }
    draw_padmap_panel(buf, win_w, win_h, sc, look);
}

/// The window: the native boxes' frame, a dark title strip, the two lines and the chips on
/// the body, and the OK band split into 취소 · 기본값 · 저장.
fn draw_padmap_panel(buf: &mut [u32], win_w: u32, win_h: u32, sc: &PadMapScene, look: &PadMapLook) {
    let p = &sc.panel;
    let s = p.s;
    let pr = p.panel;
    let o = (NP_OUTLINE * s).round().max(1.0) as u32;
    fill(buf, win_w, win_h, pr.x, pr.y, pr.w, pr.h, N_OUTLINE);
    fill(buf, win_w, win_h, pr.x + o, pr.y + o, pr.w.saturating_sub(o * 2), pr.h.saturating_sub(o * 2), N_RING);
    // The shine: the 1-px halo around the title + body column, the native boxes' seam.
    let sh = s.round().max(1.0) as u32;
    let top = p.title.y.saturating_sub(sh);
    let bot = p.body.y + p.body.h + sh;
    fill(buf, win_w, win_h, p.title.x.saturating_sub(sh), top, p.title.w + sh * 2, bot.saturating_sub(top), N_SHINE);
    fill(buf, win_w, win_h, p.title.x, p.title.y, p.title.w, p.title.h, N_DARK);
    fill(buf, win_w, win_h, p.body.x, p.body.y, p.body.w, p.body.h, N_BODY);
    let kor = crate::gamefont::KOR_H as f64;
    let ty = f64::from(p.title.y) + (f64::from(p.title.h) - kor * s) / 2.0;
    draw_native_text(buf, win_w, win_h, f64::from(p.title.x) + PM_PAD * s, ty, fit_frac(look.title, f64::from(p.title.w) - PM_PAD * 2.0 * s, s), look.title, N_TEXT);
    // What happened / what to do, then the detail — each shrinks to fit rather than wrap.
    let avail = f64::from(p.body.w) - PM_PAD * 2.0 * s;
    for (k, line) in look.lines.iter().enumerate() {
        let ts = fit_frac(line, avail, s);
        let top = f64::from(p.body.y) + (PM_PAD + k as f64 * PM_LINE_H) * s;
        let y = top + (PM_LINE_H * s - kor * ts) / 2.0;
        draw_native_text(buf, win_w, win_h, f64::from(p.body.x) + PM_PAD * s, y, ts, line, if k == 0 { N_TEXT } else { PM_HINT });
    }
    // The chips: a dark cell (the game's selected-answer face), the caption, and the badge
    // at the right end, lit like the controls are.
    let mut chips = p.chips.iter();
    for (t, b) in sc.targets.iter().zip(look.badges) {
        if t.shape != PadTargetShape::Chip {
            continue;
        }
        let Some((r, label)) = chips.next() else { break };
        let lit = padmap_lit(t.row, look);
        fill(buf, win_w, win_h, r.x, r.y, r.w, r.h, N_DARK);
        if lit == PmLit::Sel {
            blend_rect(buf, win_w, win_h, r.x, r.y, r.w, r.h, PM_SEL, 60 + (70.0 * look.pulse.unwrap_or(0.0)) as u32);
        }
        let (colour, t2, a) = match lit {
            PmLit::Offer => (PM_OFFER, sh, 200),
            PmLit::Sel => (PM_SEL, sh * 2, 255),
            PmLit::Probe => (PM_PROBE, sh * 2, 255),
        };
        frame_blend(buf, win_w, win_h, *r, t2, colour, a);
        let br = f64::from(r.h) * 0.34;
        let badge = crate::padhud::Badge {
            cx: f64::from(r.x + r.w) - f64::from(r.h) / 2.0,
            cy: f64::from(r.y) + f64::from(r.h) / 2.0,
            r: br,
        };
        blend_disc(buf, win_w, win_h, badge.cx, badge.cy, br, BADGE_FILL, 225);
        blend_ring(buf, win_w, win_h, badge.cx, badge.cy, br, BADGE_RIM, 190);
        if let Some(pos) = b {
            draw_pad_glyph(buf, win_w, win_h, badge, *pos);
        }
        let cap = f64::from(r.w) - f64::from(r.h) - 4.0 * s;
        let ts = fit_frac(label, cap, s);
        let y = f64::from(r.y) + (f64::from(r.h) - kor * ts) / 2.0;
        draw_native_text(buf, win_w, win_h, f64::from(r.x) + 3.0 * s, y, ts, label, N_TEXT);
    }
    // The band: three dark cells on its light face — the game's own answer cell (the
    // selected face of `draw_native_confirm`), each word white on it. ⛔ Not the OK band's
    // outlined lettering: a one-logical-px outline closes up a Hangul syllable's strokes at
    // this size (device-caught — the three words read as smudges). 기본값 dims with nothing
    // to undo; 저장 is lit gold while there is something to keep.
    for (c, word, on) in [(p.cancel, "취소", true), (p.reset, "기본값", look.can_reset), (p.save, "저장", true)] {
        let cell = Rect::new(c.x + sh, c.y + sh, c.w.saturating_sub(sh * 2), c.h.saturating_sub(sh * 2));
        fill(buf, win_w, win_h, cell.x, cell.y, cell.w, cell.h, N_DARK);
        let tw = game_text_w_frac(word, s);
        let tx = f64::from(cell.x) + (f64::from(cell.w) - tw) / 2.0;
        let ty = f64::from(cell.y) + (f64::from(cell.h) - kor * s) / 2.0;
        let ink = if c == p.save && look.dirty {
            PM_SEL
        } else if on {
            N_TEXT
        } else {
            NC_DIM
        };
        draw_game_text_frac(buf, win_w, win_h, tx, ty, s, word, ink);
    }
}

#[cfg(test)]
mod native_replica_preview {
    //! Eyeball harness for the native-window replicas (70차): renders the quantity
    //! dialog and the message box into PPMs under the temp dir, to hold next to the
    //! player's device screenshots. `#[ignore]`d — it writes files, proves nothing by
    //! itself, and exists to be run by hand:
    //! `cargo test -p wie_android native_replica -- --ignored`

    fn write_ppm(path: &str, buf: &[u32], w: u32, h: u32) {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for px in buf {
            out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    #[ignore]
    fn render_native_dialogs() {
        let (w, h) = (2340u32, 1080u32);
        let _ = crate::init_logical(w, h, true, true, 240);
        let dir = std::env::temp_dir();

        let mut buf = vec![0xff20_4030u32; (w * h) as usize];
        super::draw_qty_dialog(&mut buf, w, h, "개수를 설정하세요", 3, 99, Some("최대 99"));
        write_ppm(dir.join("native_qty.ppm").to_str().unwrap(), &buf, w, h);

        let mut buf = vec![0xff20_4030u32; (w * h) as usize];
        super::draw_native_notice(&mut buf, w, h, "장착을 해제해야 합니다.");
        write_ppm(dir.join("native_notice.ppm").to_str().unwrap(), &buf, w, h);

        // ★ 71차-D: the combine picker, at the size and with the icon column the player
        // asked for. One row per equipment shape so the left column can be checked
        // against the game's own art (frame = cat for equipment).
        let mut buf = vec![0xff20_4030u32; (w * h) as usize];
        let row = |name: &str, detail: &str, enabled: bool, icon: u16| crate::OrbRow {
            name: name.into(),
            detail: detail.into(),
            enabled,
            notice: (!enabled).then(|| "장착을 해제해야 합니다.".to_string()),
            icon,
        };
        let rows = [
            row("카이져스톰의 투구", "홈 1/3  +5", true, 0x00),
            row("드래곤의 갑옷", "홈 0/2", true, 0x01),
            row("불멸의 스워드", "홈 2/2  빈 홈이 없습니다.", false, 0x05),
            row("수호의 방패", "홈 1/4  [장착]  장착을 해제해야 합니다.", false, 0x0b),
            row("백발백중의혼", "홈 0/1", true, 0x0c),
        ];
        // ⛔ 93차 replaced the row-index scroll (`scroll_top` + a `more` pair) with a
        // PIXEL scroll, and this preview harness kept the old arguments — which broke
        // `cargo test -p wie_android` outright (a `#[cfg(test)]` module has to compile
        // for ANY test in the crate to run), so the whole Linux test path had been dead
        // since. `0.0` = the top of the list, which is what this preview wants.
        super::draw_orb_dialog(&mut buf, w, h, "소켓확장", "소켓확장  x3", &rows, Some(1), 0.0, "결합");
        write_ppm(dir.join("native_orb.ppm").to_str().unwrap(), &buf, w, h);
    }
}
