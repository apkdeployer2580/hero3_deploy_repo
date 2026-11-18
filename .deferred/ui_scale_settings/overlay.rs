//! On-screen touch controls + settings gear + settings modal, drawn directly
//! into the softbuffer framebuffer. Orientation-aware: portrait places the game
//! on top with controls below; landscape centres the game with the D-pad on the
//! left and the keypad on the right.

use std::collections::HashSet;

use wie_backend::KeyCode;

use crate::{CONTENT_H, CONTENT_W, IconRef, QuickSlot, SlotKind, Stats, atlas};

const BTN: u32 = 0xff2c_3040;
const BTN_DPAD: u32 = 0xff26_3548;
const BTN_PRESSED: u32 = 0xff4a_6cff;
const BORDER: u32 = 0xff70_80a0;
const LABEL: u32 = 0xffff_ffff;
const PANEL: u32 = 0xff1a_1c26;
const PANEL_ACTIVE: u32 = 0xff36_50a0;
/// Quickbar action buttons — skills blue, items green, empty dim; pressed = bright.
const QB_SKILL: u32 = 0xff2a_4a8a;
const QB_ITEM: u32 = 0xff1f_5a3a;
const QB_EMPTY: u32 = 0xff2a_2e3a;
const QB_PRESSED: u32 = 0xff4a_6cff;
/// Player-gauge capsule colours (game-gauge style: thin rounded pill).
const BAR_TRACK: u32 = 0xff1c_2130; // inner track (dark slate)
const BAR_RIM: u32 = 0xff07_0910; // thin outline
const BAR_HP: u32 = 0xffe0_3a3a; // red
const BAR_SP: u32 = 0xff3a_84e6; // blue
const BAR_EXP: u32 = 0xfff0_c828; // amber

#[derive(Clone, Copy)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    fn contains(&self, px: f64, py: f64) -> bool {
        let (xi, yi) = (px as i64, py as i64);
        xi >= self.x as i64 && xi < (self.x + self.w) as i64 && yi >= self.y as i64 && yi < (self.y + self.h) as i64
    }
}

#[derive(Clone, Copy)]
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
}

pub struct Button {
    rect: Rect,
    pub key: KeyCode,
    label: Label,
    dpad: bool,
    face: Face,
}

pub struct Layout {
    pub game: Rect,
    pub buttons: Vec<Button>,
    pub gear: Rect,
    /// The floating-joystick zone (left control area) when joystick mode is on;
    /// `None` in classic D-pad mode. A touch here (that misses the corner
    /// buttons) drives the joystick.
    pub joystick: Option<Rect>,
    /// The full-keypad toggle (immersive mode only; zero-sized elsewhere, where a
    /// real keypad is already on screen).
    pub keypad_toggle: Rect,
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
    /// A game window is open: menu-navigation keys instead of the field HUD.
    Menu,
}

/// The keys the game's own windows need and which the immersive layout otherwise
/// has NO seat for. Player-reported: `1`/`3` move between menus (tabs), `7`/`9`
/// page within one. Field mode gives those ring seats to the quickbar instead.
const MENU_KEYS: [(&str, KeyCode); 4] = [
    ("1", KeyCode::NUM1),
    ("3", KeyCode::NUM3),
    ("7", KeyCode::NUM7),
    ("9", KeyCode::NUM9),
];

/// The pull-out full keypad (immersive mode). Every remaining phone key the
/// game's windows can ask for, so no game state is ever unreachable: the 12-key
/// pad plus both soft keys. `None` = an empty cell (keeps the grid aligned).
#[allow(clippy::type_complexity)]
const PAD_KEYS: [[Option<(&str, KeyCode)>; 3]; 5] = [
    [Some(("1", KeyCode::NUM1)), Some(("2", KeyCode::NUM2)), Some(("3", KeyCode::NUM3))],
    [Some(("4", KeyCode::NUM4)), Some(("5", KeyCode::NUM5)), Some(("6", KeyCode::NUM6))],
    [Some(("7", KeyCode::NUM7)), Some(("8", KeyCode::NUM8)), Some(("9", KeyCode::NUM9))],
    [Some(("*", KeyCode::STAR)), Some(("0", KeyCode::NUM0)), Some(("#", KeyCode::HASH))],
    [Some(("LSK", KeyCode::LEFT_SOFT_KEY)), None, Some(("RSK", KeyCode::RIGHT_SOFT_KEY))],
];

/// A live view of the active joystick, for rendering (base at `center`, thumb at
/// `thumb`, both in window pixels).
#[derive(Clone, Copy)]
pub struct JoyView {
    pub center: (f64, f64),
    pub thumb: (f64, f64),
}

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
    /// Cycle the minimap panel size (player request 2026-07-30). Applies at once.
    CycleMinimapScale,
    /// Cycle the OK / action-disc size. Applies at once.
    CycleActionScale,
    Close,
}

#[derive(Clone, Copy)]
enum SKind {
    Portrait,
    Landscape,
    Dpad,
    Joystick,
    Immersive,
    BaseH,
    MinimapScale,
    ActionScale,
    Close,
}

pub struct SettingsItem {
    rect: Rect,
    pub action: SettingsAction,
    kind: SKind,
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

/// Height (GUEST px) we would like the game's buff band to have: our HP+SP+EXP
/// stack, so the two read as one row. `items::band_height_poke` clamps it to what
/// the band can actually take (the icon inside is a hardcoded 15x15).
pub fn band_h_shift(game: Rect) -> u32 {
    if game.h == 0 {
        return 25;
    }
    let b = stats_block(game);
    let (_, lh) = crate::logical();
    (b.bars.h * lh.max(1) / game.h).max(1)
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
#[allow(dead_code)]
pub fn game_buff_box(game: Rect) -> Rect {
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
#[allow(dead_code)]
pub fn game_target_box(game: Rect) -> Rect {
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
    (
        win_w.saturating_sub(pad * 3 + nominal),
        win_h.saturating_sub(pad * 3 + nominal),
    )
}

/// Bounding square of the ACTION disc (key OK). Smaller than the cluster slot it
/// sits in: the player asked for OK and the secondary-weapon button to be a
/// same-size pair side by side (2026-07-30), and the space for the second one comes
/// out of the disc.
fn action_disc(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let (cx, cy) = action_centre(win_w, win_h, pad);
    // Player-adjustable (settings). Only the DISC scales — `action_centre` stays on
    // its nominal radius, so resizing OK never drags the whole cluster around; and
    // the upper bound keeps it inside the cluster slot the ring is drawn around.
    let nominal = (win_h / 6).clamp(56, 190);
    let r = crate::scaled((win_h / 8).clamp(48, 140), crate::action_scale())
        .clamp(32, nominal.saturating_sub(pad).max(33))
        .min(cx.min(cy));
    Rect::new(cx.saturating_sub(r), cy.saturating_sub(r), r * 2, r * 2)
}

/// The persistent joystick home (centre + radius), bottom-left. Small and faint:
/// it is a hint, not furniture (user feedback — the old one was huge). This is also
/// the INPUT deflection radius in immersive mode (see [`joy_radius`]).
pub fn joy_home(win_w: u32, win_h: u32) -> (f64, f64, f64) {
    let r = (win_h / 12).clamp(34, 96);
    let pad = (win_w.min(win_h) / 110).max(3);
    (
        f64::from(pad * 5 + r),
        f64::from(win_h.saturating_sub(pad * 5 + r)),
        f64::from(r),
    )
}

/// Angular span of the control ring, in screen degrees (`cy = acy - r*sin`, so 90
/// is UP and 180 is LEFT). The action disc sits in the bottom-right corner, so the
/// reachable arc runs from just right-of-top, over the top, down to just below-left
/// — that is the widest wrap the corner allows.
const RING_A0: f64 = 66.0;
/// Was 204: the last seat ('#') ended up hard against the bottom edge on device,
/// so the arc now stops higher (player request 2026-07-30).
const RING_A1: f64 = 195.0;

/// Ring geometry around the action disc: `n` equal steps over [`RING_A0`,
/// [`RING_A1`]] at a radius that (a) clears the disc and (b) keeps ADJACENT square
/// plates apart. Squares need more than "chord >= side": a diagonal offset can
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
    let step = if n > 1 { (RING_A1 - RING_A0) / (n - 1) as f64 } else { 0.0 };
    let mut side = f64::from((win_h / 11).clamp(40, 120));
    for _ in 0..10 {
        let need = if step > 0.0 {
            side * 1.485 / (2.0 * (step.to_radians() / 2.0).sin())
        } else {
            0.0
        };
        let radius = need.max(ar + side * 0.75).max(clear + side / 2.0 + f64::from(pad * 2));
        let mut fits = true;
        out.clear();
        for i in 0..n {
            let ang = (RING_A0 + step * i as f64).to_radians();
            let cx = acx + radius * ang.cos();
            let cy = acy - radius * ang.sin();
            let (x0, y0) = (cx - side / 2.0, cy - side / 2.0);
            if x0 < 0.0 || y0 < 0.0 || x0 + side > f64::from(win_w) || y0 + side > f64::from(win_h) {
                fits = false;
                break;
            }
            out.push(Rect::new(x0 as u32, y0 as u32, side as u32, side as u32));
        }
        if fits {
            return out;
        }
        side *= 0.9;
    }
    out
}

/// Ring position 0 (nearest the disc's right side) is the CANCEL key; 1..=5 are the
/// quickbar slots. Keeping them on ONE ring is what gives the reference look of the
/// action button being encircled by its skills.
fn ring_all(win_w: u32, win_h: u32, pad: u32) -> Vec<Rect> {
    ring_slots(win_w, win_h, 6, pad)
}

fn skill_slots(win_w: u32, win_h: u32, n: usize, pad: u32) -> Vec<Rect> {
    let ring = ring_all(win_w, win_h, pad);
    ring.into_iter().skip(1).take(n).collect()
}

/// The CANCEL key: ring position 0, i.e. immediately clockwise of slot 1.
fn cancel_key(win_w: u32, win_h: u32, pad: u32) -> Rect {
    ring_all(win_w, win_h, pad)
        .first()
        .copied()
        .unwrap_or_else(|| Rect::new(0, 0, 1, 1))
}

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
    Rect::new(
        a.x.saturating_sub(pad * 2 + d),
        (a.y + a.h / 2).saturating_sub(d / 2),
        d,
        d,
    )
}

/// Top-right minimap panel. Tapping it presses `*`, which is what makes the game
/// (re)paint its offscreen minimap buffer that we mirror here.
/// Minimap panel — the EXTREME top-right corner (player request 2026-07-30: "the
/// minimap has to be at the very top-right").
///
/// It shares that corner with the game's own target-monster box, which is
/// right-edge anchored — so instead of dodging sideways (which swapped their
/// positions and read as wrong), the game's box is pushed DOWN below the panel via
/// the y-origin global. See [`hud_y_shift`].
pub fn minimap_panel(win_w: u32, win_h: u32) -> Rect {
    let pad = (win_w.min(win_h) / 110).max(3);
    // Grown twice on player request (win_h/5 -> win_h/4 -> this).
    // Player-adjustable (settings). The upper bound is not cosmetic: the game's own
    // target-monster box is poked LEFT by [`target_box_shift`], whose immediate can
    // only encode 211 guest px — a runaway panel would silently stop moving it.
    let side = crate::scaled((win_h * 26 / 100).clamp(120, 280), crate::minimap_scale())
        .clamp(96, (win_h * 55 / 100).min(win_w * 30 / 100));
    // ★ 2026-07-30: the EXTREME top-right corner, with the game's own target-monster
    // box to its LEFT (player: "the minimap has to be at the very top-right and the
    // monster name/HP/LV box to its left"). The box is right-edge anchored in the
    // guest, so it is the BOX that moves: the host pokes its anchor immediate left by
    // [`target_box_shift`] guest px. Stacking the panel UNDER the box (the previous
    // arrangement) read as the two having been swapped.
    Rect::new(win_w.saturating_sub(pad * 2 + side), pad * 2, side, side)
}

/// How far LEFT (in GUEST px) the game's own target-monster box has to move so it
/// ends up immediately left of our minimap panel instead of underneath it.
///
/// Derived from the panel geometry, so it follows any restyle - the same pattern as
/// [`band_x_shift`]. The guest anchor is a one-byte immediate (`screen_w - 87`), so
/// the result is clamped to what still encodes with the box fully on screen.
pub fn target_box_shift(game: Rect) -> u32 {
    if game.w == 0 || !crate::immersive_active() || game.w <= game.h {
        return 0;
    }
    let p = minimap_panel(game.w, game.h);
    // Window px from the right edge to the panel's left edge, + a small gap.
    let from_right = game.w.saturating_sub(p.x);
    let (lw, _) = crate::logical();
    let guest = from_right * lw.max(1) / game.w + 2;
    // Keep the box on screen and every immediate inside a byte (the binding site is
    // `subs r1,#44`, so 255 - 44).
    guest.min(crate::items::TARGET_BOX_SHIFT_MAX)
}

/// The pull-out keypad panel: left edge, vertically centred (clear of the gear
/// row above and the joystick home below). 3 columns x 5 rows of [`PAD_KEYS`].
pub fn pad_panel(win_w: u32, win_h: u32) -> Rect {
    let cell = (win_h / 7).clamp(40, 112);
    let pad = (win_w.min(win_h) / 110).max(3);
    let (w, h) = ((cell * 3).min(win_w), (cell * 5).min(win_h));
    Rect::new(pad * 2, (win_h.saturating_sub(h)) / 2, w, h)
}

fn build_pad(out: &mut Vec<Button>, panel: Rect, pad: u32) {
    let cw = panel.w / 3;
    let ch = panel.h / 5;
    for (r, row) in PAD_KEYS.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some((label, key)) = cell else { continue };
            out.push(Button {
                rect: Rect::new(
                    panel.x + c as u32 * cw + pad,
                    panel.y + r as u32 * ch + pad,
                    cw.saturating_sub(2 * pad),
                    ch.saturating_sub(2 * pad),
                ),
                key: *key,
                label: Label::Text(label),
                dpad: false,
                face: Face::Round,
            });
        }
    }
}

fn layout_immersive(win_w: u32, win_h: u32, pad: u32, mode: Mode, keypad_open: bool) -> (Rect, Vec<Button>, Option<Rect>) {
    let game = Rect::new(0, 0, win_w, win_h);
    let mut buttons = Vec::new();
    // The pull-out keypad takes touch PRIORITY over everything it covers, so it is
    // pushed first: `hit` returns the first rect containing the point.
    if keypad_open && mode == Mode::Menu {
        build_pad(&mut buttons, pad_panel(win_w, win_h), pad);
    }
    // Big ACTION disc, bottom-right.
    buttons.push(Button {
        rect: action_disc(win_w, win_h, pad),
        key: KeyCode::OK,
        label: Label::Text(""),
        dpad: false,
        face: Face::Action,
    });
    // CANCEL beside the first skill plate.
    buttons.push(Button {
        rect: cancel_key(win_w, win_h, pad),
        key: KeyCode::CLEAR,
        label: Label::Text("CLR"),
        dpad: false,
        face: Face::Round,
    });
    // Menu mode: the ring seats that hold the quickbar plates on the field become
    // the menu-navigation keys (the quickbar is empty off-field, so the two never
    // occupy the same seat at the same time).
    if mode == Mode::Menu {
        for (rect, (label, key)) in skill_slots(win_w, win_h, MENU_KEYS.len(), pad).into_iter().zip(MENU_KEYS) {
            buttons.push(Button {
                rect,
                key,
                label: Label::Text(label),
                dpad: false,
                face: Face::Round,
            });
        }
    }
    // Field-only buttons: the secondary-weapon attack (key `7`) and the minimap
    // panel, which doubles as the `*` key (that is what repaints it). Off-field
    // neither is drawn, and leaving them hit-testable would fire their key on a
    // blind tap (a source of the "map opened by itself" report).
    if mode == Mode::Field {
        buttons.push(Button {
            rect: secondary_button(win_w, win_h),
            key: KeyCode::NUM7,
            // Face is drawn by `draw_secondary` (weapon icon, or a dimmed "7").
            label: Label::Text(""),
            dpad: false,
            face: Face::Round,
        });
        // ★ The minimap panel is deliberately NOT a Button. As one it fired STAR on
        // touch-DOWN and — worse — any touch that had not been adopted by the joystick
        // acquired it just by SLIDING over the corner, which is the player-reported
        // "the minimap keeps getting clicked by itself". app.rs gives it real
        // tap semantics (down and up inside, little movement) instead.
    }
    // LSK/RSK and 2/4/5/6/8/0/* live on the pull-out keypad (toggle beside the
    // gear) rather than permanently on screen.
    (game, buttons, Some(Rect::new(0, 0, win_w, win_h)))
}

pub fn layout(win_w: u32, win_h: u32, joystick: bool, mode: Mode, keypad_open: bool) -> Layout {
    let landscape = win_w > win_h;
    let pad = (win_w.min(win_h) / 110).max(3);
    let mut buttons = Vec::new();
    let game;
    let mut joy_zone = None;

    // Landscape + immersive: the whole window is the game, controls float on top.
    if landscape && crate::immersive_active() {
        let (g, b, jz) = layout_immersive(win_w, win_h, pad, mode, keypad_open);
        // Immersive: the gear + keypad toggle are SMALL and sit immediately right of
        // the player-gauge block, centred on it. Player request (2026-07-30): they
        // were big and on the left, where they covered the game's own active-buff
        // list and were themselves painted over by the gauges.
        // ★ FIELD: no gear, no keypad toggle. The player asked for that strip back
        // (2026-07-30) — the game's own buff band is moved into it instead. Both
        // buttons stay available on every NON-field screen (title, menus), which is
        // where settings are actually used: orientation applies at once, and
        // immersive / ZOOM apply at the next start anyway.
        let (gear, toggle) = if mode == Mode::Field {
            (Rect::new(0, 0, 0, 0), Rect::new(0, 0, 0, 0))
        } else {
            // ★ 2026-07-30: TOP-CENTRE, nudged slightly RIGHT of the middle (player
            // request). The old spot was beside the gauge block on the left, which the
            // player kept meeting when they did not want it. `123` is a touch larger
            // than the gear ("아주 조금만 크게").
            let gs = (win_w.min(win_h) / 22).clamp(30, 64);
            let ts = (gs * 6 / 5).min(win_h / 8);
            let gap = pad * 2;
            let total = gs + gap + ts;
            let x0 = (win_w / 2 + win_w / 14).min(win_w.saturating_sub(total + pad)) - total / 2;
            let row_y = pad * 2;
            (
                Rect::new(x0, row_y + (ts.saturating_sub(gs)) / 2, gs, gs),
                Rect::new(x0 + gs + gap, row_y, ts, ts),
            )
        };
        return Layout {
            game: g,
            buttons: b,
            gear,
            joystick: jz,
            keypad_toggle: toggle,
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
    // Non-immersive layouts already put the whole keypad on screen, so no toggle.
    Layout { game, buttons, gear, joystick: joy_zone, keypad_toggle: Rect::new(0, 0, 0, 0) }
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
    out.push(Button { rect: corner(0, 0), key: KeyCode::LEFT_SOFT_KEY, label: Label::Text("LSK"), dpad: false, face: Face::Solid });
    out.push(Button { rect: corner(2, 0), key: KeyCode::RIGHT_SOFT_KEY, label: Label::Text("RSK"), dpad: false, face: Face::Solid });
    out.push(Button { rect: corner(0, 2), key: KeyCode::CLEAR, label: Label::Text("CLR"), dpad: false, face: Face::Solid });
    out.push(Button { rect: corner(2, 2), key: KeyCode::OK, label: Label::Text("OK"), dpad: false, face: Face::Solid });
    region
}

/// Max drag (px) for full joystick deflection, from a zone rect.
pub fn joy_radius(zone: &Rect) -> f64 {
    if crate::immersive_active() {
        // The immersive zone is the whole window, so deriving the deflection radius
        // from it gave ~450 px (drag half the screen for full tilt). Use the small
        // drawn stick radius instead — this is the INPUT radius too, so movement
        // now responds to a short thumb flick.
        return joy_home(zone.w, zone.h).2;
    }
    (zone.w.min(zone.h) as f64) * 0.42
}

/// Trailing joystick centre. Once the finger is more than `radius` from where the
/// stick spawned, the base FOLLOWS it, staying `radius` behind — so the deflection
/// measures the last `radius` px of travel rather than the whole drag from the
/// original touch point.
///
/// Player-reported bug this fixes: walking UP and then sliding the thumb LEFT
/// *without lifting* kept the character walking up, because a vector measured
/// from the spawn point is still mostly upward after a sideways slide. With a
/// trailing centre the turn registers as soon as the thumb actually travels
/// sideways.
pub fn joy_follow(cx: f64, cy: f64, tx: f64, ty: f64, radius: f64) -> (f64, f64) {
    let (dx, dy) = (tx - cx, ty - cy);
    let dist = (dx * dx + dy * dy).sqrt();
    if radius <= 0.0 || dist <= radius || dist == 0.0 {
        return (cx, cy);
    }
    (tx - dx / dist * radius, ty - dy / dist * radius)
}

/// The ONE direction key a joystick deflection `(dx, dy)` maps to: 4-way,
/// dominant axis, with hysteresis.
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
    let mut push = |c, r, key, label, dpad| out.push(Button { rect: cell(c, r), key, label, dpad, face: Face::Solid });
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
            });
        }
    }
}

pub fn hit(buttons: &[Button], x: f64, y: f64) -> Option<KeyCode> {
    buttons.iter().find(|b| b.rect.contains(x, y)).map(|b| b.key)
}

/// Quickbar action-button rects: a top-to-bottom column of round slots along the
/// RIGHT edge of the game rect (H5-style), starting below the top-right gear.
/// One rect per quickbar slot passed.
pub fn quick_buttons(game: Rect, gear: Rect, n: usize) -> Vec<Rect> {
    let mut out = Vec::new();
    if n == 0 || game.w == 0 || game.h == 0 {
        return out;
    }
    if crate::immersive_active() && game.w > game.h {
        // Immersive (reference-style HUD): the slots are SKILL BUTTONS on an arc
        // up-and-left of the big action disc, not a keypad column.
        let pad = (game.w.min(game.h) / 110).max(3);
        return skill_slots(game.w, game.h, n, pad);
    }
    let s = (game.w / 6).clamp(28, 84).min(game.h / 6);
    let gap = (s / 6).max(3);
    let x = game.x + game.w.saturating_sub(s + gap);
    let mut y = (gear.y + gear.h + gap).max(game.y + gap);
    for _ in 0..n {
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

/// Index of the quickbar button under `(x, y)`, if any.
pub fn quick_hit(rects: &[Rect], x: f64, y: f64) -> Option<usize> {
    rects.iter().position(|r| r.contains(x, y))
}

/// Draw the quickbar as ROUND action buttons (H5-style): filled disc + rim,
/// coloured by kind. Occupied slots show the DECODED skill/potion icon (from the
/// embedded atlas, number-free) + for items the live stack count; empty slots
/// show the hotkey char as a placeholder.
pub fn draw_quickbar(buf: &mut [u32], win_w: u32, win_h: u32, rects: &[Rect], slots: &[QuickSlot], pressed: Option<usize>) {
    if crate::immersive_active() {
        draw_skill_buttons(buf, win_w, win_h, rects, slots, pressed);
        return;
    }
    let at = atlas::atlas();
    for (i, r) in rects.iter().enumerate() {
        let Some(slot) = slots.get(i) else {
            break;
        };
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
        } else if let Some((_, ch)) = crate::quickslot_hotkey(slot.slot) {
            // No icon (empty / unresolved): hotkey char placeholder, centred.
            let mut lbl = String::new();
            lbl.push(ch);
            draw_centered_text(buf, win_w, win_h, *r, &lbl, LABEL);
        }
    }
}

/// Top-right minimap panel: the game's own offscreen minimap buffer, nearest-
/// neighbour enlarged to fill the panel. Until the player opens the map once the
/// game has not painted that buffer, so the panel shows a `MAP` hint instead
/// (tapping it sends `*`, which is what triggers the paint).
pub fn draw_minimap(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, mm: Option<&crate::Minimap>) {
    if !crate::immersive_active() || game.w == 0 {
        return;
    }
    draw_minimap_in(buf, win_w, win_h, minimap_panel(win_w, win_h), mm, true);
}

/// The panel the game's OWN map window gets covered with: a big centred version of
/// the same mirrored image (player request 2026-07-30 — the game draws its map at
/// 1:1, which is tiny on a wide framebuffer). Drawn only while the map window is
/// actually open.
pub fn map_overlay_panel(win_w: u32, win_h: u32) -> Rect {
    let side = (win_h * 82 / 100).min(win_w * 82 / 100);
    Rect::new((win_w - side) / 2, (win_h - side) / 2, side, side)
}

pub fn draw_map_overlay(buf: &mut [u32], win_w: u32, win_h: u32, mm: Option<&crate::Minimap>) {
    if !crate::immersive_active() {
        return;
    }
    draw_minimap_in(buf, win_w, win_h, map_overlay_panel(win_w, win_h), mm, false);
}

/// Shared painter: framed panel + the mirrored map image scaled to fit.
/// `hint` = draw a "MAP" placeholder when there is no image yet (the corner panel
/// wants it; the full-screen one should stay out of the way instead).
fn draw_minimap_in(buf: &mut [u32], win_w: u32, win_h: u32, p: Rect, mm: Option<&crate::Minimap>, hint: bool) {
    const PANEL: u32 = 0xff0c_1018;
    const FRAME: u32 = 0xffc8_ae72;
    if p.w == 0 || p.h == 0 {
        return;
    }
    blend_rect(buf, win_w, win_h, p.x, p.y, p.w, p.h, PANEL, 120);
    let t = (p.w / 40).max(2);
    blend_rect(buf, win_w, win_h, p.x, p.y, p.w, t, FRAME, 190);
    blend_rect(buf, win_w, win_h, p.x, p.y + p.h.saturating_sub(t), p.w, t, FRAME, 190);
    blend_rect(buf, win_w, win_h, p.x, p.y, t, p.h, FRAME, 190);
    blend_rect(buf, win_w, win_h, p.x + p.w.saturating_sub(t), p.y, t, p.h, FRAME, 190);

    let Some(mm) = mm.filter(|m| m.w > 0 && m.h > 0 && m.px.len() == usize::from(m.w) * usize::from(m.h)) else {
        if hint {
            draw_centered_text(buf, win_w, win_h, p, "MAP", LABEL);
        }
        return;
    };
    // Fit + centre, integer-free nearest sampling (pixel art: no smoothing).
    let inner = t * 2;
    let (aw, ah) = (p.w.saturating_sub(inner * 2), p.h.saturating_sub(inner * 2));
    let (mw, mh) = (u32::from(mm.w), u32::from(mm.h));
    if aw == 0 || ah == 0 {
        return;
    }
    let sc = (f64::from(aw) / f64::from(mw)).min(f64::from(ah) / f64::from(mh));
    let dw = ((f64::from(mw) * sc) as u32).max(1);
    let dh = ((f64::from(mh) * sc) as u32).max(1);
    let ox = p.x + inner + (aw.saturating_sub(dw)) / 2;
    let oy = p.y + inner + (ah.saturating_sub(dh)) / 2;
    for y in 0..dh {
        let sy = y * mh / dh;
        let row = ((oy + y) * win_w) as usize;
        for x in 0..dw {
            if ox + x >= win_w || oy + y >= win_h {
                continue;
            }
            let c = mm.px[(sy * mw + x * mw / dw) as usize];
            if c & 0x00ff_ffff != 0 {
                buf[row + (ox + x) as usize] = c;
            }
        }
    }
}

/// Reference-style skill buttons — ROUND (player request 2026-07-30: "make the
/// 1/3/7/9/0/# keys all round"). A translucent disc with a bright anti-aliased rim
/// (gold for a skill, steel for an item, dim when empty), the decoded icon feathered
/// into the disc, and the live stack count for items. Empty slots stay dim with just
/// the hotkey character so the layout never jumps.
fn draw_skill_buttons(buf: &mut [u32], win_w: u32, win_h: u32, rects: &[Rect], slots: &[QuickSlot], pressed: Option<usize>) {
    const PLATE: u32 = 0xff10_1420;
    const RIM_SKILL: u32 = 0xffd8_aa38;
    const RIM_ITEM: u32 = 0xff9d_b4d0;
    const RIM_EMPTY: u32 = 0xff4a_5568;
    let at = atlas::atlas();
    for (i, r) in rects.iter().enumerate() {
        let Some(slot) = slots.get(i) else {
            break;
        };
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
            None => {
                if let Some((_, ch)) = crate::quickslot_hotkey(slot.slot) {
                    let mut lbl = String::new();
                    lbl.push(ch);
                    draw_centered_text(buf, win_w, win_h, label_box(*r), &lbl, LABEL);
                }
            }
        }
    }
}

/// Alpha-aware bilinear blit of an atlas icon into `dst` (aspect preserved,
/// centred). The disc-clipped variant is [`blit_icon`], which every key now uses
/// (2026-07-30: all round) - this stays for any future rectangular face.
#[allow(dead_code)]
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
    for py in 0..dh {
        for px in 0..dw {
            let (x, y) = (ox + px, oy + py);
            if x >= win_w || y >= win_h {
                continue;
            }
            let sx = (f64::from(px) + 0.5) / f64::from(dw) * iw - 0.5;
            let sy = (f64::from(py) + 0.5) / f64::from(dh) * ih - 0.5;
            if let Some(c) = sample_bilinear(ic, sx, sy) {
                buf[(y * win_w + x) as usize] = c;
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
            let cov = cov * cov * (3.0 - 2.0 * cov); // smoothstep
            // Dest pixel centre -> source coords (half-pixel corrected).
            let fx = (ox as f64 + 0.5) * iw / dw as f64 - 0.5;
            let fy = (oy as f64 + 0.5) * ih / dh as f64 - 0.5;
            if let Some(c) = sample_bilinear(ic, fx, fy) {
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
    fill(buf, win_w, win_h, game.x, strip_y, game.w, (game.y + game.h).saturating_sub(strip_y), mask);
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

/// Geometry of the immersive player-gauge block: the round level plate and the
/// HP/SP capsule stack beside it. ONE source of truth, so the controls that sit
/// next to it (settings gear, keypad toggle) cannot drift on top of it — which is
/// exactly what happened on the first device build.
///
/// Sized from player feedback (2026-07-30): thin bars, small plate — the previous
/// `game.h/26` bars (34 px on a 1080-tall screen) and `game.h/16` plate covered
/// the game's own active-buff list.
pub struct StatsBlock {
    pub plate: Rect,
    /// All THREE capsules (HP / SP / EXP) + the gaps between them.
    pub bars: Rect,
    pub bar_h: u32,
    pub gap: u32,
}

/// Player-gauge block: extreme TOP-LEFT with a small margin (player request
/// 2026-07-30 — an earlier version pushed it right, to dodge the game's own buff
/// band, and it read as "floating in the middle"; the band is moved DOWN instead,
/// see `crate::items::hud_y_origin`). HP, SP and EXP are one stack: the EXP bar
/// used to live in the game's bottom HUD box, which meant masking that box black.
pub fn stats_block(game: Rect) -> StatsBlock {
    let pad = (game.w.min(game.h) / 110).max(3);
    let pr = (game.h / 26).clamp(14, 40); // plate radius
    let plate = Rect::new(game.x + pad * 2, game.y + pad * 2, pr * 2, pr * 2);
    let bar_h = (game.h / 50).clamp(6, 16);
    let gap = (bar_h / 2).max(2);
    let bw = (game.w / 6).clamp(110, 420);
    let stack = bar_h * 3 + gap * 2;
    let bx = plate.x + plate.w + pad * 2;
    let by = plate.y + plate.h / 2 - stack / 2; // vertically centred on the plate
    StatsBlock { plate, bars: Rect::new(bx, by, bw, stack), bar_h, gap }
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
pub fn draw_stats(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, s: &Stats) {
    if game.w == 0 || game.h == 0 {
        return;
    }
    if crate::immersive_active() {
        draw_stats_immersive(buf, win_w, win_h, game, s);
        return;
    }
    let pad = (game.w / 48).max(3);
    let bw = (game.w * 40 / 100).max(52);
    let bh = (game.h / 60).clamp(5, 14); // thin
    let gap = (bh * 3 / 4).max(3);
    let x = game.x + pad;
    let mut y = game.y + pad;
    let bar = |buf: &mut [u32], y: u32, cur: u32, max: u32, fillc: u32| {
        let frac = if max == 0 { 0.0 } else { f64::from(cur.min(max)) / f64::from(max) };
        capsule_bar(buf, win_w, win_h, x, y, bw, bh, frac, BAR_TRACK, fillc, BAR_RIM);
    };
    bar(buf, y, u32::from(s.hp), u32::from(s.hp_max), BAR_HP);
    y += bh + gap;
    bar(buf, y, u32::from(s.sp), u32::from(s.sp_max.max(1)), BAR_SP);
    y += bh + gap;
    bar(buf, y, s.exp, s.exp_next.max(1), BAR_EXP);
    y += bh + gap;
    let lvl = format!("L{}", s.level.min(99));
    let ts = (bh / 6).max(2);
    draw_text(buf, win_w, win_h, x, y, ts, &lvl, LABEL);
}

/// Reference-style status block: a round portrait plate with the level inside,
/// HP and SP bars beside it carrying their CURRENT VALUE as text, and the EXP
/// bar moved to a thin strip at the bottom centre (inside the band the game's
/// own HUD already occupies, so it costs no extra field pixels).
fn draw_stats_immersive(buf: &mut [u32], win_w: u32, win_h: u32, game: Rect, s: &Stats) {
    const PLATE: u32 = 0xff10_1420;
    const FRAME: u32 = 0xffc8_ae72;
    let b = stats_block(game);
    // Level plate. Small (player request) and labelled "LV.n" rather than a bare
    // number — the 5x7 font is uppercase-only, so "Lv." renders as "LV.".
    let pr = b.plate.w / 2;
    let (pcx, pcy) = (b.plate.x + pr, b.plate.y + pr);
    blend_disc(buf, win_w, win_h, f64::from(pcx), f64::from(pcy), f64::from(pr), PLATE, 190);
    ring(buf, win_w, win_h, f64::from(pcx), f64::from(pcy), f64::from(pr), FRAME);
    let lvl = format!("LV.{}", s.level.min(99));
    // Fit the label across 80 % of the disc width (the glyphs are only 7*scale
    // tall, so the chord at that height is still nearly the full diameter). 70 %
    // rounded "LV.70" down to scale 1 = unreadable on a 1080p screen.
    let ls = ((b.plate.w * 8 / 10) / (lvl.len() as u32 * 6)).max(1);
    draw_centered_text_in(
        buf,
        win_w,
        win_h,
        b.plate.x,
        pcy.saturating_sub(7 * ls / 2),
        b.plate.w,
        &lvl,
        LABEL,
        ls,
    );

    // HP / SP capsules right of the plate. No value text (earlier player request):
    // the fill level IS the readout.
    let (bx, bw, bh, gap) = (b.bars.x, b.bars.w, b.bar_h, b.gap);
    let mut by = b.bars.y;
    let bar = |buf: &mut [u32], y: u32, cur: u32, max: u32, fillc: u32| {
        let frac = if max == 0 { 0.0 } else { f64::from(cur.min(max)) / f64::from(max) };
        capsule_bar(buf, win_w, win_h, bx, y, bw, bh, frac, BAR_TRACK, fillc, BAR_RIM);
    };
    bar(buf, by, u32::from(s.hp), u32::from(s.hp_max), BAR_HP);
    by += bh + gap;
    bar(buf, by, u32::from(s.sp), u32::from(s.sp_max.max(1)), BAR_SP);
    by += bh + gap;
    // EXP right under SP (player request 2026-07-30): it used to be centred in the
    // game's own bottom HUD box, which forced us to mask that box opaque black —
    // and that black band is what looked wrong on device.
    bar(buf, by, s.exp.min(s.exp_next), s.exp_next.max(1), BAR_EXP);
}

/// A horizontal capsule (pill) gauge: rounded track with a fill grown from the
/// left to `frac` (0..1), plus a thin rounded rim. Height `h` is the pill
/// diameter (corner radius = h/2). Renders with hard edges (softbuffer has no
/// alpha blend) but the rounded caps read as a smooth game gauge.
#[allow(clippy::too_many_arguments)]
fn capsule_bar(buf: &mut [u32], win_w: u32, win_h: u32, x: u32, y: u32, w: u32, h: u32, frac: f64, track: u32, fillc: u32, rim: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let r = h as f64 / 2.0;
    let cy = y as f64 + r;
    let spine0 = x as f64 + r; // capsule spine endpoints (cap centres)
    // Guard the degenerate w < 2r case: keep spine1 >= spine0 so the later
    // `clamp(spine0, spine1)` never panics (f64::clamp requires min <= max).
    let spine1 = ((x + w) as f64 - r).max(spine0);
    let fill_edge = x as f64 + frac.clamp(0.0, 1.0) * w as f64;
    let x1 = (x + w).min(win_w);
    let y1 = (y + h).min(win_h);
    for py in y..y1 {
        let dy = py as f64 + 0.5 - cy;
        for px in x..x1 {
            let cxp = px as f64 + 0.5;
            let qx = cxp.clamp(spine0, spine1); // nearest point on the spine
            let dx = cxp - qx;
            let d = (dx * dx + dy * dy).sqrt();
            if d > r {
                continue; // outside the capsule
            }
            // Thin rim along the whole outline (caps + top/bottom edges).
            let c = if r - d < 1.4 {
                rim
            } else if cxp < fill_edge {
                fillc
            } else {
                track
            };
            buf[(py * win_w + px) as usize] = c;
        }
    }
}

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

// --------------------------------------------------------------------------
// Settings modal
// --------------------------------------------------------------------------

fn settings_panel(win_w: u32, win_h: u32) -> Rect {
    // Lower bound must never exceed the upper bound (tiny placeholder dims in
    // App::new would otherwise make clamp panic with min > max).
    let pw = (win_w * 74 / 100).clamp(360.min(win_w), win_w.max(1));
    let ph = (win_h * 56 / 100).clamp(340.min(win_h), win_h.max(1));
    Rect::new((win_w - pw) / 2, (win_h - ph) / 2, pw, ph)
}

pub fn settings_items(win_w: u32, win_h: u32) -> Vec<SettingsItem> {
    let p = settings_panel(win_w, win_h);
    let close_s = (p.w / 8).clamp(56, 110);
    let close = Rect::new(p.x + p.w - close_s - p.w / 30, p.y + p.h / 30, close_s, close_s);

    let bw = p.w * 36 / 100;
    let bh = p.h * 30 / 100;
    let gap = p.w * 6 / 100;
    let total = bw * 2 + gap;
    let bx0 = p.x + (p.w - total) / 2;
    // Row 1: screen orientation. Row 2: control style. Row 3: immersive + zoom
    // (both need a restart, so they are labelled and sit apart from the rest).
    // Four rows now (2026-07-30: the player asked to size the minimap and the OK
    // button from here), so the buttons get shorter and the rows tighter.
    let bh = bh * 56 / 100;
    let row1 = p.y + p.h * 16 / 100;
    let row2 = p.y + p.h * 37 / 100;
    let row3 = p.y + p.h * 58 / 100;
    let row4 = p.y + p.h * 79 / 100;
    let portrait = Rect::new(bx0, row1, bw, bh);
    let landscape = Rect::new(bx0 + bw + gap, row1, bw, bh);
    let dpad = Rect::new(bx0, row2, bw, bh);
    let joystick = Rect::new(bx0 + bw + gap, row2, bw, bh);
    let immersive = Rect::new(bx0, row3, bw, bh);
    let baseh = Rect::new(bx0 + bw + gap, row3, bw, bh);
    let mmscale = Rect::new(bx0, row4, bw, bh);
    let okscale = Rect::new(bx0 + bw + gap, row4, bw, bh);

    vec![
        SettingsItem { rect: close, action: SettingsAction::Close, kind: SKind::Close },
        SettingsItem { rect: portrait, action: SettingsAction::SetPortrait, kind: SKind::Portrait },
        SettingsItem { rect: landscape, action: SettingsAction::SetLandscape, kind: SKind::Landscape },
        SettingsItem { rect: dpad, action: SettingsAction::SetDpad, kind: SKind::Dpad },
        SettingsItem { rect: joystick, action: SettingsAction::SetJoystick, kind: SKind::Joystick },
        SettingsItem { rect: immersive, action: SettingsAction::ToggleImmersive, kind: SKind::Immersive },
        SettingsItem { rect: baseh, action: SettingsAction::CycleBaseH, kind: SKind::BaseH },
        SettingsItem { rect: mmscale, action: SettingsAction::CycleMinimapScale, kind: SKind::MinimapScale },
        SettingsItem { rect: okscale, action: SettingsAction::CycleActionScale, kind: SKind::ActionScale },
    ]
}

/// `120` -> `"120%"`. The glyph font is a 5x7 ASCII bitmap, so this must stay ASCII.
fn pct(v: u32) -> String {
    format!("{v}%")
}

pub fn settings_hit(items: &[SettingsItem], win_w: u32, win_h: u32, x: f64, y: f64) -> Option<SettingsAction> {
    if let Some(item) = items.iter().find(|i| i.rect.contains(x, y)) {
        return Some(item.action);
    }
    // Tap outside the panel closes the settings.
    if !settings_panel(win_w, win_h).contains(x, y) {
        return Some(SettingsAction::Close);
    }
    None
}

// --------------------------------------------------------------------------
// Drawing
// --------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn draw_controls(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    layout: &Layout,
    pressed: &HashSet<KeyCode>,
    joy: Option<JoyView>,
    keypad_open: bool,
) {
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
            Face::None => {}
        }
        if b.face == Face::None {
            continue; // the widget paints its own face and label (minimap panel)
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
    // Keypad toggle (immersive only — zero-sized otherwise).
    let t = layout.keypad_toggle;
    if t.w > 0 && t.h > 0 {
        draw_round_face(buf, win_w, win_h, t, if keypad_open { BTN_PRESSED } else { BTN }, BORDER, 92, 165);
        draw_centered_text(buf, win_w, win_h, label_box(t), "123", LABEL);
    }
}

fn draw_joystick(buf: &mut [u32], win_w: u32, win_h: u32, zone: &Rect, joy: Option<JoyView>) {
    // Immersive (reference-style): the stick has a PERSISTENT home bottom-left so
    // the HUD reads like the reference screenshot; a drag still spawns the live
    // stick wherever the finger lands.
    if crate::immersive_active() {
        // Small + faint, and the LIVE stick uses the same small radius (the zone is
        // the whole window in immersive mode, so the old zone-derived radius was
        // ~450 px: a huge dead feel).
        //
        // The IDLE stick is only a hint about where to put your thumb, so it is
        // drawn far fainter than the live one (player request 2026-07-30 — the old
        // idle placeholder read as a real control). While a finger is down the
        // stick brightens to its previous strength so the deflection stays legible.
        let (hx, hy, hr) = joy_home(win_w, win_h);
        let live = joy.is_some();
        let (base_a, ring_a, thumb_a) = if live { (55, 90, 105) } else { (18, 34, 40) };
        let (cx, cy) = joy.map_or((hx, hy), |v| v.center);
        blend_disc(buf, win_w, win_h, cx, cy, hr, 0xff1a_2436, base_a);
        blend_ring(buf, win_w, win_h, cx, cy, hr, BORDER, ring_a);
        let (tx, ty) = joy.map_or((hx, hy), |v| v.thumb);
        blend_disc(buf, win_w, win_h, tx, ty, hr * 0.44, 0xff9f_b6d4, thumb_a);
        return;
    }
    // On-demand floating stick: nothing is drawn until a finger is down, then the
    // base renders centred at the touch point (so "first press = centre", with no
    // fixed idle base that reads as a stick pinned to one spot).
    let Some(view) = joy else {
        return;
    };
    let r = joy_radius(zone);
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
) {
    // Dim everything behind the modal (read + darken each pixel).
    for px in buf.iter_mut() {
        let v = *px;
        let r = ((v >> 16) & 0xff) * 4 / 10;
        let g = ((v >> 8) & 0xff) * 4 / 10;
        let b = (v & 0xff) * 4 / 10;
        *px = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }

    let p = settings_panel(win_w, win_h);
    fill(buf, win_w, win_h, p.x, p.y, p.w, p.h, PANEL);
    stroke(buf, win_w, win_h, p.rect_stroke(), BORDER);

    // Title "SET".
    let ts = (p.w / 22).clamp(4, 22);
    draw_text(buf, win_w, win_h, p.x + p.w / 16, p.y + p.h / 12, ts, "SET", LABEL);

    for item in items {
        let r = item.rect;
        match item.kind {
            SKind::Close => {
                draw_button_face(buf, win_w, win_h, r, BTN);
                let s = (r.w.min(r.h) / 12).max(2);
                draw_glyph(buf, win_w, win_h, r.x + (r.w - 5 * s) / 2, r.y + (r.h - 7 * s) / 2, s, 'X', LABEL);
            }
            SKind::Portrait => {
                let active = !landscape_active;
                draw_button_face(buf, win_w, win_h, r, if active { PANEL_ACTIVE } else { BTN });
                // Tall screen icon (3:4).
                let ih = r.h * 52 / 100;
                let iw = (ih * 3 / 4).min(r.w * 66 / 100);
                let ix = r.x + (r.w - iw) / 2;
                let iy = r.y + (r.h - ih) / 2;
                fill(buf, win_w, win_h, ix, iy, iw, ih, 0xff0a_0a12);
                stroke(buf, win_w, win_h, (ix, iy, iw, ih), LABEL);
            }
            SKind::Landscape => {
                let active = landscape_active;
                draw_button_face(buf, win_w, win_h, r, if active { PANEL_ACTIVE } else { BTN });
                // Wide screen icon (4:3).
                let iw = r.w * 64 / 100;
                let ih = (iw * 3 / 4).min(r.h * 66 / 100);
                let ix = r.x + (r.w - iw) / 2;
                let iy = r.y + (r.h - ih) / 2;
                fill(buf, win_w, win_h, ix, iy, iw, ih, 0xff0a_0a12);
                stroke(buf, win_w, win_h, (ix, iy, iw, ih), LABEL);
            }
            SKind::Dpad => {
                let active = !joystick_active;
                draw_button_face(buf, win_w, win_h, r, if active { PANEL_ACTIVE } else { BTN });
                // Plus/cross icon.
                let cx = r.x + r.w / 2;
                let cy = r.y + r.h / 2;
                let arm = (r.w.min(r.h) / 4).max(4);
                let t = (arm / 3).max(2);
                fill(buf, win_w, win_h, cx - t, cy - arm, 2 * t, 2 * arm, LABEL);
                fill(buf, win_w, win_h, cx - arm, cy - t, 2 * arm, 2 * t, LABEL);
            }
            SKind::Joystick => {
                let active = joystick_active;
                draw_button_face(buf, win_w, win_h, r, if active { PANEL_ACTIVE } else { BTN });
                // Ring + knob icon.
                let cx = r.x as f64 + r.w as f64 / 2.0;
                let cy = r.y as f64 + r.h as f64 / 2.0;
                let rr = (r.w.min(r.h) as f64) * 0.28;
                ring(buf, win_w, win_h, cx, cy, rr, LABEL);
                fill_disc(buf, win_w, win_h, cx, cy, rr * 0.42, LABEL);
            }
            SKind::Immersive => {
                // "FULL" = landscape immersive (game fills the screen, floating keys).
                draw_button_face(buf, win_w, win_h, r, if immersive_pref { PANEL_ACTIVE } else { BTN });
                let s = (r.h / 12).clamp(2, 8);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h / 5, r.w, "FULL", LABEL, s);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h * 3 / 5, r.w, if immersive_pref { "ON" } else { "OFF" }, LABEL, s);
            }
            SKind::BaseH => {
                // Logical height = zoom vs how much of the map fills the screen.
                draw_button_face(buf, win_w, win_h, r, BTN);
                let s = (r.h / 12).clamp(2, 8);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h / 5, r.w, "ZOOM", LABEL, s);
                let mut num = [0u8; 8];
                let txt = {
                    let v = base_h.min(9999);
                    let mut n = 0usize;
                    for d in [1000u32, 100, 10, 1] {
                        if v >= d || n > 0 || d == 1 {
                            num[n] = b'0' + ((v / d) % 10) as u8;
                            n += 1;
                        }
                    }
                    core::str::from_utf8(&num[..n]).unwrap_or("?")
                };
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h * 3 / 5, r.w, txt, LABEL, s);
            }
            // Live control-size knobs (player request 2026-07-30). Unlike ZOOM these
            // take effect on the spot, so the label shows the value being applied.
            SKind::MinimapScale => {
                draw_button_face(buf, win_w, win_h, r, BTN);
                let s = (r.h / 12).clamp(2, 8);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h / 5, r.w, "MAP", LABEL, s);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h * 3 / 5, r.w, &pct(crate::minimap_scale()), LABEL, s);
            }
            SKind::ActionScale => {
                draw_button_face(buf, win_w, win_h, r, BTN);
                let s = (r.h / 12).clamp(2, 8);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h / 5, r.w, "OK", LABEL, s);
                draw_centered_text_in(buf, win_w, win_h, r.x, r.y + r.h * 3 / 5, r.w, &pct(crate::action_scale()), LABEL, s);
            }
        }
    }
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
fn draw_round_face(
    buf: &mut [u32],
    win_w: u32,
    win_h: u32,
    r: Rect,
    fill_color: u32,
    rim_color: u32,
    fill_a: u32,
    rim_a: u32,
) -> (f64, f64, f64) {
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
    Rect::new(r.x + (r.w.saturating_sub(side)) / 2, r.y + (r.h.saturating_sub(side)) / 2, side.max(1), side.max(1))
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
