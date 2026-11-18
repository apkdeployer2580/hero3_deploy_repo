//! winit UI-thread handler. Blits frames the emulator worker pushes, draws the
//! orientation-aware control overlay + settings gear, and forwards touch input.
//! The softbuffer surface is recreated across suspend/resume (Android destroys
//! the native window when backgrounded).

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::Instant,
};

use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    event::{Touch, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

use wie_backend::KeyCode;

use crate::{CONTENT_H, CONTENT_W, InputEvent, QuickSlot, SlotKind, Stats, UserEvent, config, dialogs, menutouch, orientation, overlay, placement};
use std::sync::mpsc::Receiver;

pub struct App {
    input_tx: Sender<InputEvent>,
    data_dir: PathBuf,

    /// Shared background/lock gate (see lib.rs). Set while the app is not in the
    /// foreground so the emulator stops ticking and the synth emits silence.
    paused: Arc<AtomicBool>,

    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,

    /// RENDER size — the surface buffer the frame is drawn at, and the coordinate
    /// space of every layout rect and (scaled) touch point. Equal to the physical
    /// window unless the HEAT SAVER halves it (see [`Self::render_div`]).
    win_w: u32,
    win_h: u32,
    /// HEAT SAVER render divisor (57차): 1 = native, 2 = draw at half resolution.
    /// `Surface::resize` maps to `ANativeWindow_setBuffersGeometry`, whose contract
    /// is exactly this: a buffer smaller than the window is scaled to the screen by
    /// the compositor, in hardware, for free — so every pixel-proportional cost on
    /// this thread (clear, game blit, overlay draws, the present swizzle) drops 4x.
    /// Fixed at process start (`config::read_heat_saver`); physical touch
    /// coordinates are divided by this on entry so everything downstream — layout,
    /// hit-tests, the editor, the stored (fractional) placements — stays in render
    /// space and needs no other change.
    render_div: u32,

    layout: overlay::Layout,
    settings_items: Vec<overlay::SettingsItem>,
    show_settings: bool,
    /// The layout editor, while it is open (player request 2026-07-31: place and
    /// size every control by hand). `Some` takes over ALL touch input.
    editor: Option<Editor>,
    /// 설정 → 패드 버튼 설정, while it is open (2026-10-09). Modal like the editor: every
    /// touch and every pad / keyboard press goes to it.
    padmap: Option<Box<PadMapUi>>,

    /// Last content frame from the worker (CONTENT_W x CONTENT_H, 0xAARRGGBB).
    last_frame: Vec<u32>,
    /// Nearest-neighbour COLUMN map for the game blit: entry `dx` is the source column
    /// that destination column `dx` samples. See the free function [`sx_lut`].
    sx_lut: Vec<u32>,
    /// The `(game.w, logical_w)` the cached [`Self::sx_lut`] was built for.
    ///
    /// ★ Those two values are the COMPLETE input set of `sx_lut(dst_w, src_w)` — it is a
    /// pure function of nothing else — so this key cannot silently go stale. That is the
    /// whole risk of caching here: a map kept across a rotation, a layout edit or a window
    /// resize would sample the wrong columns, and a sheared picture has nothing in the code
    /// to point at. `logical_w` is fixed for the life of the process after `init_logical`,
    /// but it is IN the key rather than assumed, because assuming it is exactly the kind of
    /// invariant that quietly stops holding.
    sx_lut_key: (u32, u32),

    /// Active touch id -> the key it currently maps to (game controls).
    touches: HashMap<u64, KeyCode>,
    /// Touch ids consumed by UI (gear/settings) — excluded from game input.
    ui_touches: HashSet<u64>,
    /// Touch ids that STARTED on a control. Only these may change which key they
    /// hold by sliding (`TouchPhase::Moved`).
    ///
    /// ★ 2026-07-30: without this, ANY touch that had not been adopted by the
    /// joystick — e.g. a second finger while the first one is steering — acquired
    /// whatever button it happened to slide over. With the minimap panel sitting in
    /// the top-right corner as a plain `KeyCode::STAR` button, that is precisely the
    /// player-reported "the minimap keeps getting clicked by itself".
    slide_touches: HashSet<u64>,
    /// ★ 150차 review — a `slide_touches` finger whose key a host picker TOOK (`qty_key` in
    /// `update_touch`), and which key. That finger holds nothing in `touches` any more, so
    /// without this every `Moved` of a finger RESTING on the arrow pressed it again — the
    /// 상세보기 note ran to its end, a count ran away. Same key under the same finger = not a
    /// press; sliding off and back on is one.
    qty_slid: HashMap<u64, KeyCode>,
    /// In-flight deliberate tap on the minimap panel: `(id, down_x, down_y)`.
    /// The panel is no longer a Button, so it cannot fire on touch-down or by being
    /// slid over; it fires on RELEASE, and only for a real tap.
    mm_tap: Option<(u64, f64, f64)>,
    /// In-flight deliberate tap on the Lv/HP gauge block, which opens the game's
    /// main menu (`CLEAR`). Same tap discipline as `mm_tap`, for the same reason —
    /// see `overlay::stats_tap_box`.
    stats_tap: Option<(u64, f64, f64)>,
    /// 메뉴 터치화 (66차): an in-flight deliberate tap inside the game's own menu
    /// panel — a badge the GAME drew, or a row/cell the worker will resolve. Same tap
    /// discipline as `mm_tap`, and for a sharper reason: a drag across the panel must
    /// never pick an item. Carries WHAT was hit at press time so a finger that slides
    /// onto a different target does nothing at all.
    menu_tap: Option<(u64, f64, f64, MenuHit)>,
    /// Gamepad button overrides from `wie_gamepad.txt` (58차; empty = built-in
    /// layout). Read once — remapping is an app restart, like every other file knob.

    /// Guest keys currently held DOWN by a gamepad button or the keyboard, with
    /// whether that down was diverted to `InputEvent::PopupConfirm` (same pairing
    /// rule as `popup_confirm_touches`: the UP must match what the DOWN actually
    /// DID, not the popup state at release time). Keyed by GUEST key so a down/up
    /// pair stays matched even if the player remaps mid-hold cannot happen (map is
    /// fixed per run) — and so focus loss can release exactly what is held.
    pad_held: HashMap<KeyCode, bool>,
    /// The one latched walk direction the gamepad stick/hat currently holds —
    /// `overlay::joy_dir` state, exactly like the touch joystick's `Joy::dir`.
    pad_stick: Option<KeyCode>,
    /// ★ 150차 review — `pad_stick`'s latch was TAKEN by a host modal / picker (`qty_key`)
    /// instead of reaching the game: no keydown went out, so no keyup and no walk may follow.
    pad_stick_diverted: bool,
    /// Analog-trigger digital states `(L2, R2)` — see `gamepad::trigger_held`.
    pad_triggers: (bool, bool),
    /// Keys currently held by any touch or the joystick (for highlight).
    pressed: HashSet<KeyCode>,
    /// Left control style: floating joystick (true) vs classic D-pad.
    joystick: bool,
    /// The active joystick drag, if any.
    joy: Option<Joy>,
    /// Field HUD vs game-window (menu) controls — driven by the guest state the
    /// worker reports. See `overlay::Mode`.
    mode: overlay::Mode,
    /// The script window is showing a CHOICE LIST the player has to answer
    /// (`items::script_choice_open`). The ONE thing that separates "a cutscene is
    /// playing" from "a dialogue is asking me something" — window id 4 is both.
    choice: bool,
    /// The worker's HELD cutscene flag (`items::cutscene_hold`), kept separately from
    /// [`Self::mode`] because **`mode` cannot answer "is a scene playing?"**: the mode
    /// match makes `field` OUTRANK `cutscene` (see the `UserEvent::Hud` arm), so the
    /// one poll of a dialogue PAGE TURN — where the script window's id drops and comes
    /// straight back — reads `Mode::Field`. Any gate that wants "not during a scene,
    /// blips included" MUST read this and not `mode`; the control SET still comes from
    /// `mode`, where field outranking cutscene is the wanted behaviour (a real exit
    /// shows the field HUD at once).
    ///
    /// ★ 66차: the gate that needed it (61차's world-tap walk) is gone, so nothing
    /// reads this today — it is kept because the next such gate must not re-learn the
    /// lesson the hard way, as 63차 did. `#[allow(dead_code)]` is the honest marker
    /// for that, rather than a fake reader that pretends to use it.
    #[allow(dead_code)]
    cutscene: bool,
    /// The game's own map window is open -> paint a big copy of the map over it.
    map_open: bool,
    /// Out-of-world title/slot screen -> letterbox its 240-px art.
    shell: bool,
    /// Which shell screen (`UserEvent::Hud::shell_screen`); meaningful only while `shell`.
    shell_screen: Option<u32>,
    /// Published to the emulator worker: how far down the game's own top HUD band
    /// must go so it sits below our gauges (guest px). See `overlay::hud_y_shift`.
    hud_y: crate::HudYShift,

    /// The host's field quickbar PLATE row (`quickpage::PLATES` of them) for the
    /// action HUD, from the worker. Index == plate index == `QuickSlot::slot`.
    quickbar: Vec<QuickSlot>,
    /// Live player gauges for the top-left HUD.
    stats: Stats,
    /// Live cash-buff status lines (ASCII, from the worker — it owns the clock).
    /// Drawn under the gauge block while a game window is open.
    buff_lines: Vec<String>,
    /// The one line under the bars drawn on the FIELD as well as in menus, unlike
    /// `buff_lines` (today the set bonus). See `UserEvent::Hud::field_line`.
    field_line: Option<String>,
    /// ★ 145차: the 하늘성 banner (floor + monsters left), drawn at the top centre in the
    /// game's damage digits. See `UserEvent::Hud::tower_hud`.
    tower_hud: Option<crate::guardian::TowerHud>,
    /// 하늘성 입장 연출. `tower_flash_gen` is the worker's counter as last seen;
    /// `tower_flash_left` is how many frames of white/dark are still owed.
    /// ⛔ Driven off the UI's OWN frames, not a wall clock: the emulator is what
    /// paces this screen, and a timer would flash through a stall the player is
    /// staring at.
    tower_flash_gen: u32,
    tower_flash_left: u32,
    /// ★ 143차: the EXIT tab is offered (the worker's `UserEvent::Hud::tower_exit`).
    tower_exit: bool,
    /// ★ 143차: 「하늘성에서 나가시겠습니까?」 is up — which cell is selected (0 = 예) and
    /// when it opened (the arm guard: the tap that opened it must not answer it).
    exit_ask: Option<(u8, Instant)>,
    /// ★ 143차: THE WAKE's veil — `(from, to, since, ms)`: fading from `from` to `to`
    /// (0.0 clear … 1.0 white) over `ms` from `since`, then holding at `to` until the next
    /// `UserEvent::WakeVeil`. `None` = no veil. While `Some`, every input is swallowed.
    veil: Option<(f32, f32, Instant, u32)>,
    /// Host-side skill cooldowns, keyed by SKILL IDENTITY (atlas sheet, strip cell)
    /// rather than by seat, so a page flip or re-bind cannot transplant a running
    /// cooldown onto a different skill. Value = (when the cast was CONFIRMED, that
    /// skill's cooldown length in ms). Entries are only ever inserted by
    /// [`UserEvent::SkillCast`] — the worker's verdict that the cast actually
    /// started — never by the tap itself (2026-08-07: "스킬이 실행 됐을때만 쿨타임이
    /// 돌아야해"). A tap on a plate whose cooldown is still running is swallowed,
    /// and the plate draws a bottom-up recovery fill.
    skill_cd: HashMap<(u8, u8), (Instant, u64)>,
    /// Per-skill cooldown lengths from [`config::read_skill_cooldowns`]: builtin
    /// table plus `wie_cooldowns.txt`. Missing or `0` = no overlay. Host-owned.
    /// Worn-gear percent (`addStat(0x11)`) is applied at insert, not stored here.
    skill_cds_cfg: HashMap<(u8, u8), u64>,
    /// Host notices queued for the game-style message box (140차 — every former toast:
    /// the quick-save result, settings toggles, box refusals, tower lines …). The FRONT is
    /// the one on screen; OK / CLR / a tap on its band pops it and the next one follows.
    /// Kept on the UI side because it is pure presentation — the worker just says what
    /// happened. ⛔ Separate from `orb_notice`, which belongs to the list dialog and goes
    /// down WITH it (`orb_close`): a queued 「저장완료」 must survive a box window closing.
    notices: std::collections::VecDeque<std::borrow::Cow<'static, str>>,
    /// The NON-MODAL notes (`UserEvent::SideNote`, 2026-09-30): the SAVE button's answers,
    /// 「하늘성 입장」 and each floor's clear line, with when each was raised. Drawn at the left
    /// edge (`overlay::draw_side_notes`), faded and dropped on their own clock
    /// (`overlay::side_note_alpha`); they take no input, so play goes on under them.
    side_notes: std::collections::VecDeque<(std::borrow::Cow<'static, str>, Instant)>,
    /// Wall clock for time-based overlay animation. Only the unspent-point badges
    /// use it (~2 Hz blink); a frame counter would blink at a device-dependent rate,
    /// and the guest's own blink flag is frozen by the HUD-skip patch — see
    /// [`overlay::BADGE_BLINK_MS`].
    started: Instant,
    /// Equipped secondary weapon's icon, drawn on the key-'7' button (`None` =
    /// nothing equipped -> the button is dimmed, matching the game's own refusal).
    secondary: Option<crate::IconRef>,
    /// The game's minimap (top-right panel); `None` until it has been painted.
    minimap: Option<crate::Minimap>,
    /// 141차: the marks on it — the player and the live monsters, in tiles.
    minimap_marker: Option<(f32, f32)>,
    minimap_mobs: Vec<(f32, f32)>,
    /// 141차: the player's head for the status ring, as last published.
    portrait: Option<crate::Portrait>,
    /// Cached button rects for `quickbar`, recomputed on relayout / change.
    quick_rects: Vec<overlay::Rect>,
    /// The touch id + index currently pressing a quickbar button (for highlight).
    quick_press: Option<(u64, usize)>,
    /// The game's rewritten context popup ("퀵바등록"), while one is on screen. Its
    /// only job on this side is to tell OK apart: with a popup up, a confirm belongs
    /// to the popup and the WORKER decides what it means (see `InputEvent::PopupConfirm`).
    popup: Option<crate::items::PopupView>,
    /// Touch ids whose DOWN was diverted to [`InputEvent::PopupConfirm`] instead of
    /// being sent as a real key.
    ///
    /// ★ 2026-08-06. Without this the UP edge re-decided the question by reading
    /// `popup` a second time — and `popup` CHANGES between the two edges, because the
    /// press is what opens or closes the popup. Both orders are wrong:
    ///
    /// * a press that OPENS the popup sends keydown (popup was `None`), then finds
    ///   `Some` at release and SUPPRESSES the keyup — the guest is left holding OK, and
    ///   the worker's `held` map auto-repeats it every 100 ms;
    /// * a press that CONFIRMS on the popup sends no keydown (popup was `Some`), then
    ///   finds `None` at release and sends a BARE KEYUP — exactly the "key the guest
    ///   never saw pressed" the code below warns about.
    ///
    /// They alternate, so every popup cycle inverts the guest's OK state and the next
    /// press is spent putting it back: "선택을 한번하면 아무동작없고 두번째 한번 더
    /// 선택하면 그때 뜬다" (player, 2026-08-06). Remembering the DOWN decision is what
    /// makes the pair symmetric.
    popup_confirm_touches: HashSet<u64>,
    /// The live action popup's row count + highlight (`items::popup_list`), from the
    /// worker. `None` = no popup. Needed on THIS thread because finding the popup on
    /// screen is a pixel scan of `last_frame`, and the scan has to be told how many
    /// rows to look for (79차-C).
    popup_list: Option<crate::items::PopupList>,
    /// The worker asked us to highlight the quickbar plates and wait for a tap.
    picking_slot: bool,
    /// The cash-shop purchase confirm is up and the row under it takes a stack, so
    /// the quantity picker is on screen. See `overlay::draw_qty_dialog`.
    cash_qty: bool,
    /// The in-app warehouse is holding a 올리기/내리기 reply until we say how many:
    /// `(max, cat, idx)`. The same picker, a different source of truth for the value —
    /// the cash one lives in a global the worker writes, this one is ours until the
    /// player confirms it. See `network::wh_qty_pending`.
    wh_qty: Option<(u8, u8, u8, u8)>,
    /// What the player has dialled in the warehouse picker.
    wh_qty_value: u8,
    /// The in-game 오브결합 / 소켓확장 target picker, as the worker published it. The
    /// worker owns the candidate cells; this is only what to draw and what index to
    /// answer with. See `UserEvent::OrbPick`.
    orb_pick: Option<Box<crate::OrbPick>>,
    /// The live 제련 odds for the item under the smith's cursor, as the worker published
    /// them, or `None` when the stone spinner is not on screen. Numbers only — see
    /// `UserEvent::RefineOdds`. Read-only display: nothing here can reach guest memory.
    refine: Option<crate::refine::Refine>,
    /// ★ 150차 — the 상세보기 note as the worker last published it (`UserEvent::ItemNote`), and
    /// the identity of the item it is for (a different item resets the scroll).
    item_note: Option<Box<crate::itemnote::ItemNote>>,
    note_item: u64,
    /// How far the 상세보기 body is scrolled, LOGICAL px (`itemnote::clamp_scroll` bounds it on
    /// every input and on every draw).
    note_scroll: i32,
    /// A finger on the body: `(touch id, window y it went down at, the scroll then, whether
    /// it has dragged, the panel dy it went down at)`. Same discipline as `orb_drag`.
    note_drag: Option<(u64, f64, i32, bool, i32)>,
    /// The overlay was on the last presented frame — what a key or a touch arriving before the
    /// next draw answers to. Set by `redraw` only.
    note_up: bool,
    /// The body cut out of the game frame, and the composed patch (reused every frame).
    note_game: Vec<u32>,
    note_patch: Vec<u32>,
    /// ★ 167차 — the AVATAR page as the worker last published it (`UserEvent::AvatarPage`), the
    /// game panel cut out under it, the composed patch, and whether the last presented frame
    /// showed it (what a touch answers to — set by `redraw` only, like `note_up`).
    avatar_view: Option<Box<crate::avatar::View>>,
    avatar_game: Vec<u32>,
    avatar_patch: Vec<u32>,
    avatar_up: bool,
    /// ★ 168차 — frames the page has left to stay up after the worker LEFT it with a key the
    /// game acts on next (`UserEvent::AvatarLeave`): it is dropped on the first frame that no
    /// longer shows EQUIPMENT's worn grid, or when this runs out. 0 = not leaving.
    avatar_leaving: u8,
    /// ★ 168차 — the MENU list's AVATAR row as the worker proposes it (`UserEvent::MenuAvatar`),
    /// the game panel cut out under it, the composed seven rows, and whether the last presented
    /// frame showed them (what a touch answers to — set by `redraw` only).
    menu_sel: crate::avatar::MenuSel,
    menu_game: Vec<u32>,
    menu_patch: Vec<u32>,
    menu_up: bool,
    // ⛔ `save_lock: bool` (71차-K's 「저장하고 있습니다」 freeze) WAS HERE — gone, 136차:
    // the disconnect save it waited for does not exist (`UserEvent` doc).
    /// ★ 136차: the screen the 「정렬」 button is offered on, as last PUBLISHED BY THE
    /// WORKER (`items::sort_screen`), or `None`. Draws the label and arms its tap arm;
    /// the worker re-checks every gate when the tap arrives.
    sort_screen: Option<crate::bagsort::SortScreen>,
    /// Which candidate is highlighted.
    orb_sel: usize,
    /// How far the list is scrolled, IN PIXELS (2026-09-14: the ▲/▼ column is gone and
    /// the list is dragged). `orbcombine::clamp_scroll` is the only thing that writes it.
    orb_scroll: f64,
    /// A finger on the list: `(touch id, where it went down, the scroll it went down at,
    /// whether it has moved far enough to stop being a tap)`.
    ///
    /// ★ A drag and a tap start identically, so the ROW is not acted on until the finger
    /// comes up — and then only if it never smeared. That is the discipline `mm_tap` and
    /// `menu_tap` already follow for the same reason.
    orb_drag: Option<(u64, f64, f64, bool)>,
    /// The drop box over a row (사용하기 / 퀵슬롯등록) and which of its rows is lit.
    /// While this is up it owns every key and every tap inside it: it is the game's own
    /// action popup, which is modal.
    orb_drop: Option<Box<crate::OrbDrop>>,
    orb_drop_sel: usize,
    /// 117차-D: the GAME's own message box (귀환서's 「최근 귀환장소로 이동하겠습니까?」) is
    /// up under the box window. While set, `draw` re-blits the guest frame inside the
    /// box's measured frame (`menutouch::game_dialog_frame`) OVER the dialog, `qty_key`
    /// steps aside so every key is the game's, and a touch is answered by the game's
    /// 예/아니오 (`confirm_tap_hit`) or swallowed. The worker raises and lowers it
    /// (`UserEvent::OrbReveal`); `OrbPick(None)` lowers it too.
    orb_reveal: bool,
    /// ★★ 2026-09-14, user: *"그리고 들어가면 첫번째 아이템 기본 default 포커스 처리하면
    /// 돼"*. So the dialog now opens WITH row 0 focused, which reverses 70차's
    /// "opens unfocused" rule — that rule existed to stop the OK that summoned the picker
    /// from confirming row 0 instantly ("사용시 바로 결합완료" report).
    ///
    /// ⛔ That hazard is real and is handled a different way now: [`Self::orb_armed_at`]
    /// ignores a confirm for a few tens of milliseconds after the dialog appears, which no
    /// deliberate press can fall inside and no key bleeding through from the popup that
    /// opened it can fall outside. Do NOT drop the guard when touching this.
    orb_focused: bool,
    /// When the dialog went up. A confirm before `ORB_ARM_MS` have passed is refused —
    /// see [`Self::orb_focused`].
    orb_armed_at: std::time::Instant,
    /// A game-style message box over everything (e.g. confirming a WORN row in the
    /// combine picker). OK or CLR dismisses it; every other input is swallowed.
    orb_notice: Option<String>,
    /// A finger holding ▲/▼ on a quantity picker: `(touch id, step, next fire)`.
    /// The game's own window steps by auto-repeating a held arrow; this is that.
    qty_hold: Option<(u64, i32, std::time::Instant)>,
    /// Which host-owned quickbar page is live, as last published by the WORKER (never
    /// derived from our own taps — a refused flip would leave the label lying).
    quick_page: u8,
    /// The save-slot WINDOW `(first, total)` as last published by the WORKER (130차):
    /// the slot lists' ▲/▼ arrows and readout are drawn from this and only this.
    slot_window: Option<(u8, u8)>,
    /// Whether the minimap panel is shown. Player request 2026-08-04: the panel is a
    /// toggle you can travel with on or off, and tapping it hides it — the round
    /// [`placement::CtrlId::MinimapToggle`] button in its place brings it back. Runtime
    /// only: a hidden minimap is a per-session choice, not a setting worth a file.
    minimap_on: bool,
    /// The game's own save-inhibitor byte is clear (from the worker's HUD sample).
    /// False marks a no-save AREA — in this game the boss rooms — where the SAVE
    /// button and the whole minimap UI (panel, toggle, tap zone) are hidden
    /// (player request 2026-08-07).
    can_save: bool,

    /// Emulator boot is DEFERRED until the first `resumed`, because the guest
    /// framebuffer size is derived from the real window size (landscape
    /// immersive mode) and the WIPI screen framebuffer can only be allocated
    /// once. `None` after the worker has been spawned.
    pending_boot: Option<PendingBoot>,
}

/// Everything the emulator worker needs, held until the window size is known.
pub struct PendingBoot {
    pub proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    pub should_redraw: Arc<AtomicBool>,
    pub input_rx: Receiver<InputEvent>,
    pub game_zip: Vec<u8>,
    pub hud_y: crate::HudYShift,
}

/// Was `(x, y)` close enough to the touch-down point to count as a TAP rather than
/// a drag? Deliberately generous (fingers wobble) but far below the size of the
/// panel it guards, so a swipe that merely crosses the panel never counts.
const TAP_SLOP: f64 = 28.0;
fn is_tap(dx: f64, dy: f64, x: f64, y: f64) -> bool {
    (x - dx).abs() <= TAP_SLOP && (y - dy).abs() <= TAP_SLOP
}

/// The quantity picker's held-arrow auto-repeat (70차): how long a press must last
/// before it starts walking, and how fast it walks — the pacing of the game's own
/// key repeat, which is what the ▲/▼ of its window do on hardware keys.
const QTY_HOLD_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
const QTY_HOLD_REPEAT: std::time::Duration = std::time::Duration::from_millis(90);

/// ★ 168차: guest frames an AVATAR page the worker LEFT (`UserEvent::AvatarLeave`) may stay up
/// while the game still paints EQUIPMENT — the next page normally arrives on the very next frame;
/// this only bounds a key the game ignored (~0.5 s at the game's 20 ms frame timer).
const AVATAR_LEAVE_FRAMES: u8 = 25;

/// One of the top-row tabs (SAVE · MAP · EXIT) — what its box or its 146차 halo answers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TopTab {
    Map,
    Exit,
    Save,
}

/// What a tap inside the game's own menu panel resolved to (66차). Two kinds, because
/// two mechanisms: a BADGE is a key the game itself printed on screen, and everything
/// else is a point only the emulator worker can interpret (it needs guest memory to
/// know which screen is showing).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuHit {
    /// One of the game's own «1» «3» «7» «9» badges -> send exactly that key.
    Badge(KeyCode),
    /// The save-slot window's ▲ (`-1`) / ▼ (`+1`) on a slot list (130차) — a host verb,
    /// answered by the worker (`InputEvent::SlotScroll`).
    SlotScroll(i8),
    /// The 「정렬」 button on a bag screen's money strip (136차) — a host verb, answered by
    /// the worker (`InputEvent::SortItems`), which re-checks the screen and every gate.
    SortItems,
    /// The same idea, `n` times: walk a cursor the game itself draws to the row the
    /// player tapped. Used by the shell SLOT list (80차-C), where the row cursor lives
    /// in a widget byte we cannot read — no character is loaded on that screen — so the
    /// distance is measured in PIXELS and paid in the game's own UP/DOWN.
    ///
    /// ⛔ `n` is bounded by the detector (at most `SLOT_ROWS - 1`), and the presses are
    /// queued in one go rather than one per poll. That is safe here and was not for the
    /// equipment grid (79차-D): there the cursor SKIPS empty cells, so the arrival row
    /// could not be predicted and each press had to be re-checked. A slot list has no
    /// skipping — three rows, all reachable — so the delta computed from the pixels is
    /// the answer, and re-reading between presses would only reintroduce 79차-D's
    /// "read in the same tick" trap.
    Steps(KeyCode, u8),
    /// Somewhere else inside the panel, in GUEST-SCREEN px -> `InputEvent::MenuTap`,
    /// together with the item grid the game is drawing (derived from its own pixels) and
    /// the cell its cursor sits on. That pair is what tells the worker an item grid from
    /// a row list, and what shape the grid is.
    ///
    /// The trailing `bool` is `menutouch::cell_box`: a 17x17 CELL cursor is on screen.
    /// It separates the two very different reasons the grid can be `None` — see
    /// `InputEvent::MenuTap::cell_cursor`.
    Panel(i32, i32, Option<(menutouch::Grid, u8)>, Option<u8>, Option<menutouch::EquipMark>, bool),
    /// ★ 167차 — an action on the AVATAR page (`avatar_page::hit`), answered by the worker
    /// (`InputEvent::Avatar`), which owns the page's state and the guest.
    Avatar(crate::avatar::Act),
    /// ★ 168차 — a row of the seven-row MENU list (`avatar_page::menu_hit`) -> `InputEvent::MenuRow`.
    MenuRow(u8),
    /// The touch was CLAIMED and means nothing — a miss inside a box that owns the
    /// screen (80차-G). It exists so a modal's own dead space cannot fall through to the
    /// joystick or to whatever the game drew behind it, which is half of the
    /// 2026-09-05 report (*"팝업이 떠있는 상태로 터치가 된다"*).
    Swallow,
}

impl MenuHit {
    /// Does `other` mean the same thing to the player as `self`?
    ///
    /// ★ This is NOT `==`, and the difference is the whole reason it exists. A finger
    /// wobbles: `is_tap` allows ~28 window px, which is ~6 LOGICAL px, so the release
    /// point of a perfectly ordinary tap is rarely the exact pixel the press landed on.
    /// Comparing `Panel(gx, gy)` by value would therefore drop most real taps — and it
    /// would do it invisibly, on a device, in a way `adb shell input tap` (which lifts
    /// at the same pixel) can never reproduce.
    ///
    /// So: a badge must still be THE SAME badge (sliding from «7» to «9» does nothing),
    /// and a panel point need only still be a panel point — the press coordinates are
    /// what gets sent, because the press is what the player aimed at.
    fn same_target(self, other: Self) -> bool {
        match (self, other) {
            (MenuHit::Badge(a), MenuHit::Badge(b)) => a == b,
            (MenuHit::SlotScroll(a), MenuHit::SlotScroll(b)) => a == b,
            (MenuHit::SortItems, MenuHit::SortItems) => true,
            // Same target = same DIRECTION. The step count is allowed to differ between
            // the press and the release: the frame can repaint under a resting finger,
            // and what gets sent is the press's count anyway (see the `Ended` arm).
            (MenuHit::Steps(a, _), MenuHit::Steps(b, _)) => a == b,
            (MenuHit::Panel(..), MenuHit::Panel(..)) => true,
            // The same row, the same slot, the same badge — a finger that slid onto the next
            // row must not wear what it slid onto.
            (MenuHit::Avatar(a), MenuHit::Avatar(b)) => a == b,
            (MenuHit::MenuRow(a), MenuHit::MenuRow(b)) => a == b,
            (MenuHit::Swallow, MenuHit::Swallow) => true,
            _ => false,
        }
    }
}

/// The key the game printed on a badge. ★ These are the same four keys the on-screen
/// `MENU_KEYS` block has been sending since 2026-08-03 — which is exactly why stage 1
/// carries no new risk: the badge target is a second, better-placed way to press a
/// button the player already has.
fn badge_key(b: menutouch::Badge) -> KeyCode {
    match b {
        menutouch::Badge::Tab1 => KeyCode::NUM1,
        menutouch::Badge::Tab3 => KeyCode::NUM3,
        menutouch::Badge::Page7 => KeyCode::NUM7,
        menutouch::Badge::Page9 => KeyCode::NUM9,
    }
}

/// Nearest-neighbour column map for the game blit: entry `dx` is `(dx * src_w) / dst_w`
/// — exactly what the old inner loop recomputed with an integer DIVIDE for every single
/// destination pixel. At 1080x2340 that was ~2.5 M divides per presented frame, on top of
/// the 2.46 M guest instructions the same frame already costs (measured with `wie_probe`,
/// 2026-08-06); the map is `dst_w` entries and changes only when the geometry does.
///
/// A FREE function taking only its two inputs, deliberately. `cargo test` cannot run in
/// this crate (the linked winit/android deps kill the test binary on this host) and
/// `app.rs` is not one of the files the standalone root selftests `include!`, so the only
/// review this logic can get is a human reading it — which needs it to be independent of
/// `App`.
///
/// Bound the blit relies on: every entry is `< src_w`. For `dx <= dst_w - 1`,
/// `dx * src_w <= (dst_w - 1) * src_w < dst_w * src_w`, so the floor division by `dst_w`
/// yields at most `src_w - 1`. Done in u64 so the product cannot wrap at any window size
/// (u32 would be fine for real devices, but "fine for real devices" is not a proof).
fn sx_lut(dst_w: u32, src_w: u32) -> Vec<u32> {
    let d = u64::from(dst_w.max(1));
    (0..dst_w).map(|dx| (u64::from(dx) * u64::from(src_w) / d) as u32).collect()
}

/// The one mode the layout editor ever works in — the FIELD.
///
/// ★ NEVER `App::mode`. The worker rewrites that from the guest state and calls
/// `relayout()` on this same thread, so binding the editor to it would swap the control
/// set out from under a finger. And it has to be a simulation for a more basic reason:
/// the settings gear does not exist in the field (`overlay::layout` gives it a zero rect
/// there), so the FIELD controls can only ever be arranged from a menu.
///
/// ★★ A `mode` FIELD on [`Editor`], flipped by the "게임진행 / 메뉴오픈" chrome buttons,
/// until 2026-09-10. Both buttons are gone with the second seat table
/// (`placement::Table`): the field set is the whole vocabulary now, and what it resolves
/// to is also what a game window gets — so there is nothing left for a second page to
/// show. Still a named constant rather than an inlined literal, because every use below
/// is the same decision and it should be visible as one.
const EDIT_MODE: overlay::Mode = overlay::Mode::Field;

/// 설정 → 패드 버튼 설정 (2026-10-09). The rules — swap, refusal, the stored diff — are
/// `padmap.rs`; what is on screen is `overlay::padmap_scene` / `overlay::draw_padmap`.
///
/// User decisions 2026-10-09: a conflict SWAPS; the screen is driven by TOUCH (pick a
/// control) and the PAD (press the button) — no pad navigation, because the button being
/// remapped may be the very OK that would confirm it. Second form, the same day: the
/// control is picked ON THE IN-GAME HUD, lit where it is played (*"실제 인게임 화면상에
/// 버튼들을 하이라이트 처리하고 각 버튼을 눌르고 패드 버튼을 누르는 방식"*) — not from a list.
struct PadMapUi {
    /// The table being edited — a copy of the one in force. Nothing reaches
    /// `gamepad::set_overrides` or the file until 저장; 취소 just drops this.
    draft: HashMap<i32, Option<crate::gamepad::PadAction>>,
    /// The function picked last (a `gamepad::REMAP_ROWS` index): lit while it waits for a
    /// button, and after — so the player sees which one they just changed.
    sel: Option<usize>,
    /// Waiting for a button for `sel` since this instant (`padmap::CAPTURE_MS`).
    capture: Option<Instant>,
    /// The function a BARE press (nothing picked) fires — lit in blue beside the answer.
    probe: Option<usize>,
    /// The window's two lines: what happened, and what to do. The second gives way to the
    /// countdown while a button is awaited.
    lines: [String; 2],
}

/// What `redraw` draws for the pad screen (`App::padmap_view`).
struct PadMapView {
    scene: overlay::PadMapScene,
    /// The plates as the field has them — the worker's last list by plate, an empty plate
    /// past it (the title screen publishes none).
    slots: Vec<crate::QuickSlot>,
    /// One per target: the button that fires it in the draft.
    badges: Vec<Option<crate::padhud::PadPos>>,
    sel: Option<usize>,
    probe: Option<usize>,
    /// `Some` while a button is awaited — the lit control breathes, and frames are owed.
    pulse: Option<f64>,
    lines: [String; 2],
    can_reset: bool,
    /// Does the draft differ from the table in force — is there anything for 저장 to save?
    dirty: bool,
}

/// The layout editor's state.
struct Editor {
    sel: Option<placement::CtrlId>,
    /// The layout as it was when the editor opened, for `CANCEL`. Edits are written to
    /// disk continuously (the app can be killed from the background at any moment), so
    /// backing out has to restore a snapshot rather than "not save".
    entry_snapshot: placement::Table,
    /// The hidden-plate set as it was when the editor opened, for the same reason and
    /// with the same rule: HIDE / SHOW persist immediately, so CANCEL has to put this
    /// back too. Two snapshots rather than one because they live in two files.
    hidden_snapshot: u8,
    /// The finger moving the selection.
    drag: Option<Drag>,
    /// Two-finger resize of the selection.
    pinch: Option<Pinch>,
    /// Live position of every finger currently down.
    ///
    /// ★ Needed because winit's Android backend reports `Moved` for EVERY pointer in
    /// the MotionEvent, not just the one that moved (platform_impl/android/mod.rs:
    /// `TouchPhase::Moved | Cancelled => Box::new(motion_event.pointers())`). A pinch
    /// computed from per-event deltas would therefore double-apply; the size is
    /// derived from the anchor captured at pinch start instead.
    points: HashMap<u64, (f64, f64)>,
}

/// One in-progress move. Stored as "where the finger went down" plus "where the
/// control was then", so the control follows the finger's DELTA — a drag that starts
/// on empty screen (which is normal now that the selection is sticky) must not
/// teleport the control under the fingertip.
#[derive(Clone, Copy)]
struct Drag {
    id: u64,
    from: (f64, f64),
    centre0: (f64, f64),
}

struct Pinch {
    a: u64,
    b: u64,
    /// Finger distance when the pinch started.
    dist0: f64,
    /// The control's size percent when the pinch started.
    pct0: u32,
}

fn distance(p: (f64, f64), q: (f64, f64)) -> f64 {
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
}

/// One in-progress joystick drag: which touch owns it, the base centre, the current
/// thumb position, and the single direction key it holds.
///
/// `cx`/`cy` is fixed for the life of the touch — see `App::joy_update`. Under 자유
/// 스틱 it is where the finger went DOWN; under 고정 스틱 it is the seat's centre
/// (`App::joy_start`).
struct Joy {
    id: u64,
    cx: f64,
    cy: f64,
    tx: f64,
    ty: f64,
    dir: Option<KeyCode>,
}

/// The message box on screen — the list dialog's own notice outranks the host queue's
/// front. A free function over the two FIELDS so the draw pass can ask while `buf`
/// (borrowed from `self.surface`) is live.
fn notice_text_of<'a>(orb: &'a Option<String>, queue: &'a std::collections::VecDeque<std::borrow::Cow<'static, str>>) -> Option<&'a str> {
    orb.as_deref().or_else(|| queue.front().map(|c| c.as_ref()))
}

impl App {
    pub fn new(input_tx: Sender<InputEvent>, data_dir: PathBuf, paused: Arc<AtomicBool>, hud_y: crate::HudYShift, pending_boot: PendingBoot) -> Self {
        let joystick = config::read_joystick(&data_dir);
        // Per-skill cooldown overrides, read once: the file only changes by hand.
        let skill_cds_cfg = config::read_skill_cooldowns(&data_dir);
        // The custom layout lives in a process global (the overlay geometry has no
        // `data_dir` in hand), so it has to be seeded from disk BEFORE the first
        // layout is built — otherwise the player's arrangement would only appear
        // after the next relayout.
        placement::set_table(config::read_layout(&data_dir));
        // Same reasoning as the layout table above: the touch type is read by
        // `overlay::layout_immersive` and `draw_joystick`, neither of which has a
        // `data_dir`, so it lives in a process global that must be seeded before the
        // first layout is built.
        crate::set_stick_mode(config::read_stick_mode(&data_dir));
        crate::game_options::load(&data_dir);
        // Same again for the hidden quickslot set (2026-09-10): `quick_buttons`,
        // `plate_seat` and `placement::shown` all read it while building the FIRST
        // layout, so a plate the player switched off has to be off before that runs —
        // otherwise it flashes on screen for one frame and, worse, takes its old ring
        // seat until something else forces a relayout.
        crate::set_quick_hidden(config::read_quick_hidden(&data_dir));
        let render_div = if config::read_heat_saver(&data_dir) { 2 } else { 1 };
        // ★ The pad override table goes into a global (`gamepad::set_overrides`) rather
        // than a field: the HUD badges are painted by free functions in `overlay.rs`
        // that have no `App` in hand, and one store is what keeps "what the button
        // does" and "what its badge says" from ever disagreeing.
        crate::gamepad::set_overrides(config::read_gamepad_map(&data_dir));
        // 게임패드 HUD preference (59차) — applies at once, so it is loaded into the
        // global the layout reads instead of being re-read per frame.
        crate::set_pad_hud_pref(config::read_pad_hud(&data_dir));
        Self {
            input_tx,
            data_dir,
            paused,
            window: None,
            context: None,
            surface: None,
            win_w: CONTENT_W,
            win_h: CONTENT_H,
            render_div,
            layout: overlay::layout(CONTENT_W, CONTENT_H, joystick, overlay::Mode::Field, false),
            choice: false,
            settings_items: overlay::settings_items(CONTENT_W, CONTENT_H),
            show_settings: false,
            editor: None,
            padmap: None,
            last_frame: vec![0xff00_0000u32; (CONTENT_W * CONTENT_H) as usize],
            // Empty + a key that no live geometry can equal (`redraw` only blits with
            // `g.w > 0`), so the first draw always builds it.
            sx_lut: Vec::new(),
            sx_lut_key: (0, 0),
            touches: HashMap::new(),
            popup_confirm_touches: HashSet::new(),
            ui_touches: HashSet::new(),
            slide_touches: HashSet::new(),
            qty_slid: HashMap::new(),
            mm_tap: None,
            stats_tap: None,
            menu_tap: None,
            cutscene: false,
            pad_held: HashMap::new(),
            pad_stick: None,
            pad_stick_diverted: false,
            pad_triggers: (false, false),
            pressed: HashSet::new(),
            joystick,
            joy: None,
            // Start in MENU mode: at boot the game is on its title screen, not in
            // the field, so the field HUD must not be up. (Device-caught: with
            // `Field` as the initial value the gauges + minimap panel were drawn
            // over the title screen, because the worker's first sample matches its
            // own initial `last_field` and so used to publish nothing.)
            mode: overlay::Mode::Menu,
            map_open: false,
            shell: false,
            shell_screen: None,
            hud_y,
            quickbar: Vec::new(),
            popup_list: None,
            stats: Stats::default(),
            buff_lines: Vec::new(),
            field_line: None,
            tower_hud: None,
            tower_flash_gen: 0,
            tower_flash_left: 0,
            tower_exit: false,
            exit_ask: None,
            veil: None,
            skill_cd: HashMap::new(),
            skill_cds_cfg,
            notices: std::collections::VecDeque::new(),
            side_notes: std::collections::VecDeque::new(),
            started: Instant::now(),
            secondary: None,
            minimap: None,
            minimap_marker: None,
            minimap_mobs: Vec::new(),
            portrait: None,
            quick_rects: Vec::new(),
            quick_press: None,
            popup: None,
            picking_slot: false,
            cash_qty: false,
            wh_qty: None,
            wh_qty_value: 1,
            orb_pick: None,
            refine: None,
            item_note: None,
            note_item: 0,
            note_scroll: 0,
            note_drag: None,
            note_up: false,
            note_game: Vec::new(),
            note_patch: Vec::new(),
            avatar_view: None,
            avatar_game: Vec::new(),
            avatar_patch: Vec::new(),
            avatar_up: false,
            avatar_leaving: 0,
            menu_sel: crate::avatar::MenuSel::Off,
            menu_game: Vec::new(),
            menu_patch: Vec::new(),
            menu_up: false,
            sort_screen: None,
            orb_sel: 0,
            orb_scroll: 0.0,
            orb_drag: None,
            orb_drop: None,
            orb_drop_sel: 0,
            orb_reveal: false,
            orb_armed_at: std::time::Instant::now(),
            orb_focused: false,
            orb_notice: None,
            qty_hold: None,
            quick_page: 0,
            slot_window: None,
            // Shown by default: this is how the HUD looked before the toggle existed.
            minimap_on: true,
            can_save: true,
            pending_boot: Some(pending_boot),
        }
    }

    /// The minimap panel as a TAP TARGET, or `None` when it is not on screen.
    ///
    /// ★ `overlay::minimap_panel` returns geometry unconditionally, but the panel is
    /// only built, drawn and tappable in the LANDSCAPE IMMERSIVE field HUD. Without
    /// this gate the tap region would also swallow touches in the portrait /
    /// non-immersive layouts, where that rect lands on a real key.
    fn minimap_tap_zone(&self) -> Option<overlay::Rect> {
        (self.mode == overlay::Mode::Field && self.can_save && crate::immersive_active() && self.win_w > self.win_h)
            .then(|| overlay::minimap_panel(self.win_w, self.win_h))
    }

    /// The Lv/HP gauge block as a tap target, or `None` when it is not on screen.
    /// Gated exactly like the block is DRAWN (`redraw`: field + landscape immersive)
    /// — a zone that outlived its drawing would be an invisible region that opens the
    /// menu, which is indistinguishable from a bug.
    fn stats_tap_zone(&self) -> Option<overlay::Rect> {
        (self.mode == overlay::Mode::Field && crate::immersive_active() && self.win_w > self.win_h).then(|| overlay::stats_tap_box(self.layout.game))
    }

    /// May a tap inside the game's own panel mean something right now (66차)?
    ///
    /// MENU mode only — that is the whole point of the mode: one of the game's own
    /// windows owns the keypad, so a tap on the panel cannot be steering, cannot be
    /// an attack, and (unlike 61차's world tap, which this replaced) cannot move the
    /// character. Everything modal of OURS is excluded, because those draw over the
    /// panel and own their own taps.
    ///
    /// ★ A held CUTSCENE cannot leak in here the way it leaked into the walk gate:
    /// the mode match makes `field` outrank `cutscene`, so a dialogue page-turn blip
    /// reads `Mode::Field` — never `Mode::Menu`. (Any future gate that needs "is a
    /// scene playing?" must read the worker's HELD flag and not `mode`. That is the
    /// 63차 lesson, and it is why the walk gate had to.)
    fn menu_tap_ok(&self) -> bool {
        // ★★★ 79차-E fix: `Mode::Menu` OR a dialogue that is ASKING something. The
        // script window is `Mode::Cutscene` (the worker reports `cutscene` for window
        // id 4 and the mode match sends `(false, true)` there), so the 대화 선택지 arm
        // in `items::menu_tap_action` was structurally unreachable: this gate refused
        // it, and even before that `tap_anywhere_key` claimed the touch. The whole
        // screen being OK is right for NARRATION and wrong for a CHOICE — it confirms
        // whichever row the game has highlighted, which is an irreversible branch.
        // `self.choice` is `items::script_choice_open` held over page-turn blips
        // (`choice_hold`), i.e. the game's own byte for "an OK would pick a row".
        (self.mode == overlay::Mode::Menu || (self.mode == overlay::Mode::Cutscene && self.choice)) && self.panel_tap_ok()
    }

    /// Everything in [`Self::menu_tap_ok`] EXCEPT the mode test — i.e. "no overlay of
    /// OURS owns this touch".
    ///
    /// ★★★ Split out in 80차-G so the game's own `예 / 아니오` confirm can be answered on
    /// the FIELD too. Player: *"이건 보스방 입장시도 마찬가지야"*. A confirm the game raises
    /// while walking leaves `Mode::Field` (it opens no window — device: `win=3`), so
    /// `menu_tap_ok` refused it, and under TOUCH TYPE A the joystick zone is the whole
    /// window, so the touch never even reached that arm. See `App::confirm_tap_hit`.
    fn panel_tap_ok(&self) -> bool {
        true
            // ⛔ NO LIVE-CHARACTER TERM HERE. It was `!self.shell` until 66차-C and
            // `self.stats.level > 0` until 80차-B, and BOTH refused screens the player
            // needs. `self.shell` is app state 3, which the 창고/캐시샵 screens share with
            // the title (66차-C); `level` is 0 for the whole 타이틀 → 온라인 → 창고 path,
            // because no character has been loaded yet.
            //
            // 2026-09-05, on vc244: *"confirm류 팝업들에서 터치안먹고있고, 네트워크 창고
            // 접속시 싱글가방, 네트워크가방, 캐시샵 등에서도 터치 다 안먹어"*. The device log
            // said `REFUSED mode=Menu shell=true` with every printed term false — because
            // the term that actually refused was the one NOT printed. A screenshot showed
            // the game's own `예 / 아니요` box, i.e. a target made of PIXELS, refused for
            // want of a character record it never needed.
            //
            // ★★ The split is by EVIDENCE, and it is in `menu_tap_decide`: the detectors
            // that read only `last_frame` (the game's «1»«3»«7»«9» badges, its `예/아니요`
            // pair, its message-box `OK`, its `OK :` footer, the smith spinner) synthesise
            // a key the GAME printed and are correct with no character at all. Only the
            // ones that hand a point to the worker to resolve against CHAR need one.
            //
            // What the old gate was really for still stands and is unaffected: a tap on
            // the title notice ("아무키나 누르세요") stays UNCLAIMED, because no detector
            // matches it — `menu_tap_hit` returns `None` and the touch falls through to
            // `tap_anywhere_key`. That was always the real mechanism; the level test was
            // a second, blunter copy of it that also caught live windows.
            && !self.picking_slot
            // (The pull-out keypad's `!keypad_open` term lived here until 2026-09-18;
            // the keypad is gone, so there is no panel of ours over the game's any more.)
            && !self.map_open
            && !self.show_settings
            && !self.cash_qty
            && self.wh_qty.is_none()
            // The in-game combine picker is drawn over the inventory it was opened from,
            // so its taps must not also move the game's cell cursor underneath.
            && self.orb_pick.is_none()
            && self.editor.is_none()
    }

    /// A quantity picker is up: the game's own direction keys dial it.
    ///
    /// ★ Player request 2026-08-25 — "갯수 조절하는게 터치로밖에안되는데 화살표 밑이나 위
    /// 클릭했을때도 동작하도록", and "오른쪽은 10개씩 올라가는 … 왼쪽은 10개씩 내려가는".
    /// So UP/DOWN step by one, RIGHT/LEFT by ten, and the window draws its buttons in
    /// that same cross so the mapping is visible. Both pickers take it — the touch HUD
    /// arrows, the pull-out keypad and the gamepad all arrive here as the same key.
    ///
    /// `true` = consumed; the guest must not see it. ⛔ OK is consumed for the WAREHOUSE
    /// picker only: that one is holding a reply the client is blocked on, and answering
    /// IS the OK. The cash picker's OK has to reach the guest — it is the 예 underneath
    /// that commits the purchase, and the worker latches the count on that very key
    /// (`emulator::cash_qty_commit`).
    // ── the list dialog's touch: DRAG to scroll, TAP to choose ──────────────
    //
    // ★★ User, 2026-09-14: *"우측에 버튼으로 스크롤 내리는 형식에서 드래그로 내리는 형식으로
    // 수정"*. A drag and a tap begin with the same event, so the ROW is not acted on
    // until the finger lifts, and then only if it never smeared — the discipline `mm_tap`,
    // `stats_tap` and `menu_tap` already follow for the same reason.
    //
    // ⛔ The BUTTONS still fire on the way down. They are not part of the scrolling
    // surface, and making 취소 wait for a clean release would be a regression on a control
    // the player presses to get out of a dialog.

    /// A finger went down on the dialog. `true` = the dialog owns this touch.
    fn orb_started(&mut self, id: u64, x: f64, y: f64) -> bool {
        let Some(len) = self.orb_pick.as_deref().map(|p| p.rows.len()) else {
            return false;
        };
        // ★ The drop box is modal over the list: a tap on it picks a verb row, a tap
        // anywhere else dismisses it — exactly what the game's own action popup does.
        //
        // ★★ 1탭 포커스 / 2탭 실행 (2026-09-16, user: *"터치로 선택했을때 기존처럼 포커스 ->
        // 한번더 클릭하면 OK로 인식 하도록"*). Until now a tap on any verb row confirmed it
        // on the way down. Now it is the grammar the game's own popup already has (66차-B,
        // 79차-C): a tap on a row that is not the highlighted one only moves the highlight,
        // and the highlighted row — row 0 on open, like the game's — confirms. The
        // `ORB_ARM_MS` guard inside `orb_drop_confirm` still covers the opening tap.
        if let Some(drop) = self.orb_drop.as_deref() {
            let n = drop.rows.len();
            match overlay::orb_drop_hit(self.win_w, self.win_h, self.orb_scroll, drop.anchor, n, x, y) {
                Some(i) if i == self.orb_drop_sel => self.orb_drop_confirm(),
                Some(i) => {
                    self.orb_drop_sel = i;
                    self.request_redraw();
                }
                None => self.orb_drop_cancel(),
            }
            return true;
        }
        // ⛔ A second finger does not take the drag off the first. It is swallowed — the
        // dialog owns the panel, and letting it through would move the game's cell cursor
        // under the window (the reason this arm is above everything else).
        if self.orb_drag.is_some() {
            return true;
        }
        let Some(hit) = overlay::orb_hit(self.win_w, self.win_h, len, self.orb_scroll, x, y) else {
            return false;
        };
        match hit {
            // The list surface: remember where the finger went down and wait.
            overlay::OrbHit::Row(_) | overlay::OrbHit::List => {
                self.orb_drag = Some((id, y, self.orb_scroll, false));
            }
            overlay::OrbHit::Ok => self.orb_confirm(self.orb_sel),
            // ⛔ Cancel ANSWERS. The worker is holding the orb until it hears back.
            overlay::OrbHit::Cancel => {
                crate::orb_answer(None);
                self.orb_close();
            }
            overlay::OrbHit::Swallow => {}
        }
        true
    }

    /// The finger moved: scroll the list by however far it has travelled since it went
    /// down. ⛔ Measured from the ORIGIN, not from the last event — accumulating deltas
    /// drifts, and a clamp at either end would eat the travel back.
    fn orb_moved(&mut self, y: f64) {
        let Some((id, y0, scroll0, moved)) = self.orb_drag else { return };
        let Some(len) = self.orb_pick.as_deref().map(|p| p.rows.len()) else {
            return;
        };
        let r = overlay::orb_rects(self.win_w, self.win_h);
        let slop = f64::from(r.row_h * overlay::ORB_DRAG_SLOP_PCT / 100);
        let moved = moved || (y - y0).abs() >= slop;
        self.orb_drag = Some((id, y0, scroll0, moved));
        if moved {
            self.orb_scroll = crate::orbcombine::clamp_scroll(scroll0 - (y - y0), len, r.row_h, r.list.h);
            self.request_redraw();
        }
    }

    /// The finger lifted. A clean tap that never smeared chooses the row under it —
    /// 1탭 포커스 / 2탭 실행, the grammar 66차 gave the menus at the player's own request.
    fn orb_ended(&mut self, clean: bool, x: f64, y: f64) {
        let Some((_, _, _, moved)) = self.orb_drag.take() else { return };
        if !clean || moved {
            return;
        }
        let Some(len) = self.orb_pick.as_deref().map(|p| p.rows.len()) else {
            return;
        };
        let Some(overlay::OrbHit::Row(idx)) = overlay::orb_hit(self.win_w, self.win_h, len, self.orb_scroll, x, y) else {
            return;
        };
        if self.orb_focused && idx == self.orb_sel {
            self.orb_confirm(idx);
        } else {
            self.orb_sel = idx;
            self.orb_focused = true;
        }
    }

    /// A confirm that arrives within this of the dialog appearing is not the player's.
    ///
    /// ★★ This is what replaced 70차's "opens unfocused" rule when the user asked for a
    /// default focus (2026-09-14). The hazard 70차 was answering is real: the OK that
    /// chose a row in the GAME'S popup, or the second press of a habitual double-tap,
    /// arrives a frame or two after the dialog goes up — and with row 0 pre-focused that
    /// would combine or use instantly. A deliberate press cannot land inside 250 ms of the
    /// window appearing; a bleed-through always does.
    const ORB_ARM_MS: u128 = 250;

    /// Has the dialog been up long enough for a confirm to be the player's own?
    fn orb_armed(&self) -> bool {
        self.orb_armed_at.elapsed().as_millis() >= Self::ORB_ARM_MS
    }

    /// Move the selection and bring it into view — the list follows the cursor, which is
    /// how a gamepad scrolls now that the ▲/▼ column is gone.
    fn orb_move(&mut self, step: i32) {
        let Some(pick) = self.orb_pick.as_deref() else { return };
        let len = pick.rows.len();
        self.orb_sel = crate::orbcombine::step_sel(self.orb_sel, len, step);
        self.orb_focused = true;
        let r = overlay::orb_rects(self.win_w, self.win_h);
        self.orb_scroll = crate::orbcombine::scroll_to_show(self.orb_scroll, self.orb_sel, len, r.row_h, r.list.h);
    }

    /// Confirm row `idx`. A row the worker marked disabled answers with the game's own
    /// message box instead of silence (70차, at the player's request).
    fn orb_confirm(&mut self, idx: usize) {
        let Some(pick) = self.orb_pick.as_deref() else { return };
        if !self.orb_focused || !self.orb_armed() {
            return;
        }
        let Some(row) = pick.rows.get(idx) else { return };
        if !row.enabled {
            self.orb_notice = row.notice.clone();
            self.request_redraw();
            return;
        }
        let keep = pick.keep_open;
        crate::orb_answer(Some(idx));
        // ★ A BOX window stays up: what follows is a drop box on this very row, or the
        // target picker. Closing it here would flash the game's inventory between the two.
        if !keep {
            self.orb_close();
        }
    }

    /// Take the list dialog down, with everything that hangs off it.
    fn orb_close(&mut self) {
        self.orb_pick = None;
        self.orb_drop = None;
        self.orb_notice = None;
        self.orb_drag = None;
    }

    // ── Host notices = the game-style message box (140차) ────────────────────────
    //
    // User, 2026-09-29: *"토스트로 뜨는 안내들을 게임 내부의 팝업으로 뜨도록 전부 전수 전환
    // 하고싶어"*. The self-expiring status line (`overlay::draw_toast`, 1.6 s at the bottom
    // of the game rect) is gone; every line that used it — the worker's `UserEvent::Notice`
    // / `NoticeText` and the UI's own settings / editor answers — is queued here and drawn
    // as the box the game itself uses for 「저장하였습니다」 (`draw_native_notice`, the 70차
    // replica). A box wants an answer, so it is MODAL: `notice_up` is what every input
    // path asks first, and OK / CLR / the band is the only way through (`dismiss_notice`).
    //
    // ⛔ A QUEUE, not a slot: two lines a frame apart (「층 클리어」 then the EXP line, a
    // box's 「버렸습니다」 after its refusal) must both be read — a slot showed the last one
    // only, which was fine for a whisper and wrong for a stop.

    /// Queue `text` for the message box, wrapped to its width and cut to its lines.
    fn push_notice(&mut self, text: impl Into<std::borrow::Cow<'static, str>>) {
        let text = text.into();
        let wrapped = overlay::wrap_notice_text(&text);
        if wrapped.is_empty() {
            return;
        }
        // ⛔ The same line twice in a row is ONE box: a refusal the worker repeats on every
        // tap (「아이템을 사용하는 중입니다」) must not stack up behind the one on screen.
        if self.notices.back().is_some_and(|last| last.as_ref() == wrapped) {
            return;
        }
        self.notices.push_back(wrapped.into());
        self.request_redraw();
    }

    /// How many side notes can be on screen at once; the oldest goes first.
    const SIDE_NOTE_MAX: usize = 3;

    /// Raise a side note (`UserEvent::SideNote`). The same line again (SAVE tapped twice)
    /// restarts its clock instead of stacking a copy.
    fn push_side_note(&mut self, text: std::borrow::Cow<'static, str>) {
        if text.is_empty() {
            return;
        }
        self.side_notes.retain(|(t, _)| *t != text);
        self.side_notes.push_back((text, Instant::now()));
        while self.side_notes.len() > Self::SIDE_NOTE_MAX {
            self.side_notes.pop_front();
        }
        self.request_redraw();
    }

    /// Is a message box up — the list dialog's own, or the front of the host queue?
    fn notice_up(&self) -> bool {
        self.orb_notice.is_some() || !self.notices.is_empty()
    }

    /// ★ 143차: is ANY host modal up — a message box, 「하늘성에서 나가시겠습니까?」, or the
    /// wake's veil? The pad's host-only buttons (plates, PAGE, SAVE) ask this; keys go
    /// through `qty_key`, which answers each modal in turn.
    fn modal_up(&self) -> bool {
        self.notice_up() || self.exit_ask.is_some() || self.veil.is_some()
    }

    /// OK / CLR / the band: take down the box on screen — and only that one.
    fn dismiss_notice(&mut self) {
        if self.orb_notice.is_some() {
            self.orb_notice = None;
        } else {
            self.notices.pop_front();
        }
        self.request_redraw();
    }

    fn orb_drop_confirm(&mut self) {
        if !self.orb_armed() {
            return;
        }
        let Some(drop) = self.orb_drop.as_deref() else { return };
        if self.orb_drop_sel >= drop.rows.len() {
            return;
        }
        crate::orb_answer(Some(self.orb_drop_sel));
        // ⛔ Only the BOX goes; the worker decides what happens to the list behind it
        // (사용하기 closes it so the game's own popup is visible, 퀵슬롯등록 closes it so
        // the plate can be tapped).
        self.orb_drop = None;
        self.request_redraw();
    }

    fn orb_drop_cancel(&mut self) {
        crate::orb_answer(None);
        self.orb_drop = None;
        self.request_redraw();
    }

    fn qty_key(&mut self, key: KeyCode) -> bool {
        let step = match key {
            KeyCode::UP => 1,
            KeyCode::DOWN => -1,
            KeyCode::RIGHT => 10,
            KeyCode::LEFT => -10,
            _ => 0,
        };
        // ★ 70차: the game-style message box outranks whatever it is drawn over. OK or
        // CLR dismisses it; everything else is swallowed — it is a modal, like the
        // game's own.
        // ★★ 143차: the wake's veil owns EVERY key while any of it is on screen — the player
        // is being carried home under it, and a key reaching the game now would land in the
        // town it is about to show.
        if self.veil.is_some() {
            return true;
        }
        // ★ 71차-N: the NOTICE is tested before the save lock. It is the more specific
        // modal — only the combine picker raises it — and a lock that happened to be up
        // underneath must not make it undismissable.
        if self.notice_up() {
            if matches!(key, KeyCode::OK | KeyCode::CLEAR) {
                self.dismiss_notice();
            }
            self.request_redraw();
            return true;
        }
        // ★ 143차: 「하늘성에서 나가시겠습니까?」 — a modal like the message box, under it.
        if self.exit_ask.is_some() {
            self.exit_key(key);
            return true;
        }
        // ★ 96차: the count picker outranks the list and its drop box — a box window's
        // 올리기 / 내리기 asks its count OVER the list (`bagbox_host`), and every key
        // while it is up belongs to the number. The engine's own ask never has a list
        // up, so nothing changes for it.
        if let Some((max, min, _, _)) = self.wh_qty {
            if step != 0 {
                let now = i32::from(self.wh_qty_value) + step;
                // ★ The FLOOR is not always 1: what the player keeps has to fit back in the
                // bag, and the engine works out the smallest deposit that leaves a remainder
                // it can deliver. See `offline::QtyAsk::min`.
                self.wh_qty_value = now.clamp(i32::from(min.max(1)), i32::from(max.max(1))) as u8;
                self.request_redraw();
                return true;
            }
            if key == KeyCode::OK {
                crate::network::wh_qty_answer(self.wh_qty_value);
                self.wh_qty = None;
                self.request_redraw();
                return true;
            }
            // ★ CLR = CANCEL (player request 2026-08-26: "CLR버튼 누르면 구매나 올리기 등
            // 행동이 취소되고 창이 닫혀야돼"). Still an ANSWER — the engine is holding the
            // client's reply, so the way out is `05 <op> 00 …`, not a closed dialog.
            // Nothing has moved at this point: 올리기 only empties the bag cell on the
            // SUCCESS arm of its reply handler, and 내리기 has touched nothing.
            if key == KeyCode::CLEAR {
                crate::network::wh_qty_cancel();
                self.wh_qty = None;
                self.request_redraw();
                return true;
            }
            // Everything else is swallowed too: the client is blocked on a reply we are
            // holding, so a key reaching it now can only confuse the screen underneath.
            return true;
        }
        // ★ The DROP BOX is tested first: it is the game's own action popup, which is
        // modal over the list it hangs on (the same rule `items::menu_state` enforces for
        // the game's real one — a tap underneath an open popup does nothing).
        if let Some(drop) = self.orb_drop.as_deref() {
            let n = drop.rows.len();
            match key {
                KeyCode::UP => self.orb_drop_sel = self.orb_drop_sel.saturating_sub(1),
                KeyCode::DOWN => self.orb_drop_sel = (self.orb_drop_sel + 1).min(n.saturating_sub(1)),
                KeyCode::OK => self.orb_drop_confirm(),
                // ⛔ Closing the box must ANSWER: the worker is holding the stack until it
                // hears back, and its own cancel arm is what puts the list back up.
                KeyCode::CLEAR => self.orb_drop_cancel(),
                _ => {}
            }
            self.request_redraw();
            return true;
        }
        // ★ The list dialog rides the same choke point, so the touch arrows, the pull-out
        // keypad and the gamepad all drive it without three more call sites. It SWALLOWS
        // everything while it is up: it is drawn over the game's inventory, and the popup
        // it replaced has already closed, so any key reaching the guest now would move the
        // cell cursor under a dialog the player is still reading.
        if self.orb_pick.is_some() {
            // ★★★ 117차-D: while the GAME's own dialog shows THROUGH the window
            //   (`orb_reveal`), every key is the game's — `false` hands it to the caller's
            //   ordinary game-key path (the worker delivers it, the game's 예/아니오 answers).
            //   The list gets its keys back the frame the worker lowers the reveal.
            if self.orb_reveal {
                return false;
            }
            // ★★ 2026-09-14: THE LEFT-HAND DIRECTION KEYS BELONG TO THE LIST. 71차-H had
            // ←/→ moving between 결합 and 취소, but the ▲/▼ scroll column is gone and a
            // gamepad's only way to reach the bottom of a long list is these keys (user:
            // *"패드일때는 좌측키로 내리는 방식으로 하면돼"*). So ↑/↓ step a row, ←/→ step a
            // page, and 취소 keeps its two ways out: the CLR key and a tap on the button.
            match key {
                KeyCode::UP => self.orb_move(-1),
                KeyCode::DOWN => self.orb_move(1),
                KeyCode::LEFT => self.orb_move(-(overlay::ORB_ROWS as i32)),
                KeyCode::RIGHT => self.orb_move(overlay::ORB_ROWS as i32),
                KeyCode::OK => self.orb_confirm(self.orb_sel),
                // ⛔ Cancel must ANSWER, not just close: the worker is holding the orb
                // until it hears back, exactly like the warehouse picker's reply.
                KeyCode::CLEAR => {
                    crate::orb_answer(None);
                    self.orb_close();
                }
                _ => {}
            }
            self.request_redraw();
            return true;
        }
        if self.cash_qty {
            if step != 0 {
                let now = i32::from(crate::cash_qty()) + step;
                crate::set_cash_qty(now.clamp(1, i32::from(crate::CASH_QTY_MAX)) as u8);
                self.request_redraw();
                return true;
            }
            // ★ CLR = 아니오. The box underneath has a cancel of its own, but reaching it
            // means pressing a DIRECTION key first (`modalKey[13]` toggles 예/아니오 on
            // -4/-3) and the spinner above owns those — so the player was left with
            // "무조건 구매하는 방법밖에없고". The worker writes the game's choice byte and
            // synthesises the OK that runs its `closeModal(0)`.
            // ⛔ OK is NOT taken: it belongs to the 예 underneath, and the worker latches
            // the committed count on that very key (`emulator::cash_qty_commit`).
            if key == KeyCode::CLEAR {
                let _ = self.input_tx.send(InputEvent::CashBuyCancel);
                self.request_redraw();
                return true;
            }
        }
        // ★ 150차: the 상세보기 body scrolls on the direction keys — LAST, under every host
        // modal and picker above (the 140차 order: veil, message box, EXIT confirm first).
        // This is the one choke point the HUD's arrows, a pad's buttons, a keyboard and (150차
        // review) a pad's stick/HAT share — `gamepad_axes` offers each new latch here first.
        if self.note_key(key) {
            return true;
        }
        false
    }

    /// A finger went down while the warehouse count picker is up. `true` = it landed on
    /// the picker (a step, or its OK) and was handled; `false` = it did not touch it.
    fn wh_qty_tap(&mut self, id: u64, x: f64, y: f64) -> bool {
        let Some((max, min, _, _)) = self.wh_qty else { return false };
        let Some(hit) = overlay::qty_hit(self.win_w, self.win_h, x, y) else {
            return false;
        };
        match hit {
            overlay::QtyHit::Step(d) => {
                let now = i32::from(self.wh_qty_value) + d;
                self.wh_qty_value = now.clamp(i32::from(min.max(1)), i32::from(max.max(1))) as u8;
                // ★ 70차: the native ▲/▼ auto-repeat while held, like the game's own key
                // repeat (the ±10 buttons are gone from the face; LEFT/RIGHT still step 10).
                if d != 0 {
                    self.qty_hold = Some((id, d, std::time::Instant::now() + QTY_HOLD_DELAY));
                }
            }
            // Answering is the ONLY way the held reply gets out, so the dialog has no
            // cancel: `network.rs`'s deadline takes the whole stack if nobody ever presses
            // this.
            overlay::QtyHit::Ok => {
                crate::network::wh_qty_answer(self.wh_qty_value);
                self.wh_qty = None;
            }
        }
        self.ui_touches.insert(id);
        self.request_redraw();
        true
    }

    /// Step the picker a held ▲/▼ is pointing at, on the frame clock (called from
    /// `draw`, which runs every frame while a picker is up). The native window has
    /// no ±10 buttons — the game's own reaches 99 by auto-repeating a held arrow,
    /// and this is that repeat. Armed in the touch handler, disarmed on touch end
    /// or when the picker it belonged to is gone.
    fn pump_qty_hold(&mut self) {
        let Some((id, step, next)) = self.qty_hold else { return };
        if self.wh_qty.is_none() && !self.cash_qty {
            self.qty_hold = None;
            return;
        }
        let now = std::time::Instant::now();
        if now < next {
            return;
        }
        if let Some((max, min, _, _)) = self.wh_qty {
            let v = i32::from(self.wh_qty_value) + step;
            self.wh_qty_value = v.clamp(i32::from(min.max(1)), i32::from(max.max(1))) as u8;
        } else {
            let v = i32::from(crate::cash_qty()) + step;
            crate::set_cash_qty(v.clamp(1, i32::from(crate::CASH_QTY_MAX)) as u8);
        }
        self.qty_hold = Some((id, step, now + QTY_HOLD_REPEAT));
        self.request_redraw();
    }

    /// The game's own `예 / 아니오` confirm, answered from PIXELS alone — on ANY screen,
    /// the FIELD included.
    ///
    /// ★★★ 80차-G. Player, 2026-09-05: *"인게임 -> 시스템 -> 세이브 … 예 혹은 아니오 선택해도
    /// 팝업창에서 동작을 안하고 갑자기 뒤에있는 화면에 터치가돼 … 이건 보스방 입장시도
    /// 마찬가지야"*. TWO bugs, and they had different causes:
    ///
    /// 1. **The in-game confirm is not a modal.** It is the SHARED LIST WIDGET in its
    ///    horizontal mode (device `focus probe` with the box up: `axis=0 rows=2 cur=0
    ///    choice=1`), which `menutouch::confirm_pair`'s two modal recognisers cannot see —
    ///    they look for a 40x16 box with a `#9c7d73` outline beside it, and this one is
    ///    52x14 with no box at all on the unselected side. `confirm_pair_widget` reads it.
    /// 2. **A confirm on the FIELD was unreachable.** `menu_tap_ok` needs `Mode::Menu`,
    ///    and this box opens no window (`win=3`), so the mode stays `Field`; and under
    ///    TOUCH TYPE A the joystick zone is the whole window, so the touch was taken as a
    ///    walk before any panel arm ran. This arm therefore sits ABOVE the joystick.
    ///
    /// ⛔ PIXELS ONLY, and deliberately: the box is drawn before any of it is in a state
    /// we can name, and it appears with no character loaded (the title's own confirms) as
    /// well as with one. The recognisers are exact — a filled rect of one exact colour at
    /// a measured position and size — so a false positive on the map is not a real risk,
    /// and `panel_tap_ok` still keeps our own overlays' touches.
    fn confirm_tap_hit(&self, x: f64, y: f64) -> Option<MenuHit> {
        if !self.panel_tap_ok() {
            return None;
        }
        let (lw, lh) = crate::logical();
        let (gx, gy) = self.guest_px(x, y);
        let (dx, dy) = menutouch::panel_point(lw as i32, lh as i32, gx, gy)?;
        let (sel, other) = menutouch::confirm_pair(&self.last_frame, lw as i32, lh as i32)?;
        // ⛔ The box OWNS the panel while it is up: a miss is swallowed, not passed to
        // whatever is drawn behind it. That is the half of the report that says the
        // background reacted while the popup was open.
        let (hit, why) = if sel.contains(dx, dy) {
            (MenuHit::Badge(KeyCode::OK), "selected -> OK".to_string())
        } else if other.contains(dx, dy) {
            let k = if other.dx < sel.dx { KeyCode::LEFT } else { KeyCode::RIGHT };
            (MenuHit::Badge(k), format!("other -> {k:?}"))
        } else {
            (MenuHit::Swallow, format!("OWNS the panel (sel {sel:?} other {other:?}) — miss"))
        };
        log::warn!("confirm tap: panel ({dx},{dy}) -> {why}");
        Some(hit)
    }

    /// The release-time re-check for `self.menu_tap`, and the ONLY place the two press
    /// arms are put back together.
    ///
    /// The press chain tests `confirm_tap_hit` above the joystick zone and `menu_tap_hit`
    /// below it, so a release has to consult both — a confirm answered by the first would
    /// otherwise be re-checked with the second, which on the FIELD always says `None`, and
    /// the tap would never fire. Order matches the press chain.
    fn tap_target(&self, x: f64, y: f64) -> Option<MenuHit> {
        self.avatar_tap_hit(x, y)
            .or_else(|| self.menu7_tap_hit(x, y))
            .or_else(|| self.confirm_tap_hit(x, y))
            .or_else(|| self.menu_tap_hit(x, y))
    }

    /// What a tap at window `(x, y)` would mean inside the game's own menu panel.
    ///
    /// Two answers, and the order matters. A BADGE is what the game itself printed
    /// («1» «3» «7» «9»), found by scanning the live frame for its ink — so it needs
    /// no guest anchor at all, works on any screen that draws one (the warehouse and
    /// the cash shop included), and can never claim a spot where the game drew
    /// nothing. Anything else inside the panel is a ROW or a CELL, which only the
    /// worker can resolve, so the point is forwarded raw.
    fn menu_tap_hit(&self, x: f64, y: f64) -> Option<MenuHit> {
        let (lw, lh) = crate::logical();
        let (gx, gy) = self.guest_px(x, y);
        // ★ Outside the game's own panel this arm has nothing to say, and saying so in
        // the log would be every stray touch on the map. Silence.
        // ★★ 80차-F — 패널 밑단 **바깥**의 [`menutouch::FOOTER_OVERHANG`] 줄까지 받는다.
        // 그 밴드에서는 `menu_tap_decide` 가 밑단 프롬프트 둘만 답하게 한다.
        let (dx, dy) = menutouch::panel_point_footer(lw as i32, lh as i32, gx, gy)?;
        let (hit, why) = self.menu_tap_decide(dx, dy, gx, gy);
        // ★★ ONE LINE PER PANEL TOUCH, and it is load-bearing (66차-B). "왜 이 화면은
        // 터치가 안 되나" was answered three times by guesswork in a row — a stale widget
        // byte, a paint flag that is 0 on one screen and 12 on another, a grid whose shape
        // was not the bag's — and each round cost a build. The detectors live on the UI
        // side where the worker's `focus probe` cannot see them, so the UI prints its own
        // verdict: which detector claimed the point, or which gate refused it.
        log::warn!("menu tap: panel ({dx},{dy}) -> {why}");
        hit
    }

    /// 패널 밑단의 두 프롬프트 — `# : …` 와 `OK : …` — 를 판정하는 **유일한** 자리.
    ///
    /// 판정이 필요한 곳이 둘이라 함수로 뺐다: 패널 **안**의 밑단(다른 감지기들 사이의 제자리)과,
    /// 패널 **밖**의 [`menutouch::FOOTER_OVERHANG`] 밴드(거기서는 이 둘만 답한다).
    ///
    /// ★★★ 80차-D — the game's own `# : 초기화` footer prompt (player, 2026-09-05:
    /// *"초기화버튼도 터치로 되게"*). Same contract as the `OK : …` arm below it — the
    /// game printed the key on the label, so the label is the button — but this one
    /// finds the WORDS rather than the strip: `hash_prompt` verifies the `#` glyph's
    /// own bitmap and claims only that prompt's run. That is why it needs no
    /// live-character gate and why it goes FIRST: on the 게임시작 slot list both
    /// prompts share the strip, and `ok_prompt` would otherwise answer a tap on
    /// 「초기화」 with OK — which on that screen is 「이어하기」.
    ///
    /// ⛔ Single tap fires, deliberately, unlike the slot ROWS below (80차-C). The
    /// game answers `#` with its own 「이대로 진행합니까?」 confirm — whose 예/아니요 is
    /// already tappable (80차-B) — so a stray thumb is one CLR away from undone. The
    /// rows have no such second gate, which is why they are focus-only.
    ///
    /// ── the `OK : …` half ────────────────────────────────────────────────────────
    /// It says "press OK", so the words are the button (player, 2026-08-23). Tested after
    /// the badges so the page «7»/«9» keep their pixels — on 퀘스트/스킬 they sit in the
    /// same strip.
    ///
    /// ⛔⛔ AND THIS ONE KEEPS THE LIVE-CHARACTER GATE, unlike the four detectors above it
    /// in `menu_tap_decide`. `menutouch::ok_prompt` returns a box spanning the WHOLE PANEL
    /// WIDTH — it finds the strip, not the words — which is right on the screens it was
    /// measured on (제련 재료창 · 스피너: one prompt, nothing else in the footer). The SHELL
    /// screens this arm newly reaches have TWO: the online slot list draws
    /// `OK : 선택   CLR : 메인메뉴`. A full-width box there would answer a tap on
    /// 「메인메뉴」 with OK — which on that screen LOADS the slot and, per 71차-L, has the
    /// `04 3d` ack overwrite it. Silently doing the opposite of the label on a destructive
    /// control is worse than not being tappable, and the OK button is on screen anyway, so
    /// nothing is unreachable.
    /// ★ To open this up properly, `ok_prompt` has to return the WORDS (split the strip on
    /// its gap and key each run by the glyphs in it) rather than the strip.
    /// ★★ 80차-D got half of that: the `# : …` run is claimed by the arm above, and
    /// `ok_prompt` clips its box to exclude it. What still keeps this gate is `CLR`, and
    /// the measurement says why it is not a colour test: the shell draws `CLR` in the SAME
    /// cream sprite font as `OK` (online slot list, dx 90..122 vs dx 0..21), so telling
    /// them apart needs the token's glyph run — and the in-game `@OK:}제련하기` prompt uses
    /// the small WHITE ASCII `OK` instead, which nothing has measured yet. Until it is, a
    /// positive ID of `OK` cannot cover both fonts.
    ///
    /// ★★★ 80차-P — `words_only`: answer ONLY a positively identified WORD (`#`, an `OK`/`CLR`
    /// token). The caller runs that form BEFORE the window arms (pager · OPTION) for the
    /// footer band, because those windows draw their own `CLR : 메인메뉴` in the footer and
    /// their arms own the whole panel: on the device (vc271) a tap on the title OPTION's
    /// 「메인메뉴」 at dy 155 came back `option owns the panel — miss`, while the same word
    /// one row below the panel edge was a CLR. The footer prompt is the window's own way
    /// out, so it must win over the window's frame. What `words_only` leaves for the later
    /// call: the "split footer, point on no word -> Swallow" rule and the `ok_prompt` strip,
    /// neither of which may pre-empt a window's body (HELP's scroll half reaches dy 147).
    fn footer_prompt_hit(&self, dx: i32, dy: i32, words_only: bool) -> Option<(Option<MenuHit>, String)> {
        let (lw, lh) = crate::logical();
        if let Some(r) = menutouch::hash_prompt(&self.last_frame, lw as i32, lh as i32)
            && r.contains(dx, dy)
        {
            return Some((Some(MenuHit::Badge(KeyCode::HASH)), format!("hash prompt {r:?}")));
        }
        // ★★★ 80차-F — the `OK : …` run, POSITIVELY IDENTIFIED, and therefore with no
        // live-character gate. Player: *"게임시작 초기화 및 이어하기 터치영역도 너무 작아서
        // 어디를 터치해야 버튼이 동작하는지 모르겠어"* — 「이어하기」 had never been tappable
        // at all (cold boot on that screen is `level == 0`, and the strip detector below is
        // gated on `level > 0`). `ok_token_prompt` measures the `TOKEN :` prefix, which is
        // what tells `OK` (26 px) from `CLR` (37 px); it fails closed, so this only ADDS a
        // target.
        //
        // ★★★ 80차-P — SHELL ONLY, and the footer becomes WORDS once any token is identified.
        // Two review findings (2026-09-06) closed at once:
        //  ① the width rule (`OK :` 26 vs `CLR :` 37) was measured on the shell's cream
        //    sprites only, yet this arm ran in-game too, where the footer band also holds
        //    `LV1`, `PLAY TIME` and the STATUS stat table — a probe frame of STATUS yields
        //    two colon-shaped runs and survived only on the "two candidates -> refuse" rule.
        //    In-game has always been `ok_prompt`'s (orange action name, `level > 0`).
        //  ② `CLR : 메인메뉴` on the online slot list: with a character loaded earlier in the
        //    session (`level > 0`), `ok_prompt` below returned the WHOLE strip, so a tap on
        //    「메인메뉴」 sent OK — which on that screen LOADS the slot. Now the `CLR` token is
        //    identified too (37 px on device AND probe, `FOOTER_CLR_TOKEN_*`), a tap on it
        //    sends CLR, and a footer that has been split into words answers ONLY its words:
        //    a point on none of them is a miss, never `ok_prompt`'s strip.
        if self.shell {
            let toks = menutouch::footer_token_prompts(&self.last_frame, lw as i32, lh as i32);
            if let Some((t, r)) = toks.iter().find(|(_, r)| r.contains(dx, dy)) {
                // Fail closed on ambiguity: two prompts of the same token cannot both be it.
                if toks.iter().filter(|(u, _)| u == t).count() == 1 {
                    let key = match t {
                        menutouch::FooterToken::Ok => KeyCode::OK,
                        menutouch::FooterToken::Clr => KeyCode::CLEAR,
                    };
                    return Some((Some(MenuHit::Badge(key)), format!("{t:?} token prompt {r:?}")));
                }
                return Some((Some(MenuHit::Swallow), format!("footer has two {t:?} prompts {toks:?} — refused")));
            }
            if !words_only && !toks.is_empty() && dy >= menutouch::FOOTER_BOX_TOP {
                return Some((
                    Some(MenuHit::Swallow),
                    format!("footer split into words {toks:?} — this point is on none"),
                ));
            }
        }
        if words_only {
            return None;
        }
        if self.stats.level > 0
            && let Some(r) = menutouch::ok_prompt(&self.last_frame, lw as i32, lh as i32)
            && r.contains(dx, dy)
        {
            return Some((Some(MenuHit::Badge(KeyCode::OK)), format!("ok prompt {r:?}")));
        }
        None
    }

    /// The decision behind [`Self::menu_tap_hit`], with a reason string for the log.
    fn menu_tap_decide(&self, dx: i32, dy: i32, gx: i32, gy: i32) -> (Option<MenuHit>, String) {
        if !self.menu_tap_ok() {
            return (
                None,
                // ⛔ EVERY TERM OF THE GATE, INCLUDING THE ONES THAT ARE NOT BOOLEANS.
                // 80차-B: this line printed eight flags, all of them `false`, while the
                // arm refused — because the term doing the refusing (`level`) was not on
                // it. That cost a screenshot and a round trip on a diagnostic whose entire
                // job is to make one line enough. If a term is added to `menu_tap_ok`, it
                // is added here in the same edit.
                format!(
                    "REFUSED mode={:?} level={} shell={} picking={} map={} settings={} cashqty={} whqty={} orb={} editor={}",
                    self.mode,
                    self.stats.level,
                    self.shell,
                    self.picking_slot,
                    self.map_open,
                    self.show_settings,
                    self.cash_qty,
                    self.wh_qty.is_some(),
                    self.orb_pick.is_some(),
                    self.editor.is_some(),
                ),
            );
        }
        // ★★★ 80차-F — 패널 밑단 **바깥**의 밴드. 게임이 밑단 프롬프트의 글자를 패널 밖까지
        // 그리므로(실측 `#` 글리프 dy 155..161, 패널은 160 에서 끝난다) 그 두 줄을 누른 엄지도
        // 받아야 하는데, 그 좌표는 패널 안의 어떤 것으로도 해석하면 안 된다 — 특히 마지막 arm 은
        // 좌표를 워커에 **그대로** 넘긴다. 그래서 여기서 갈라진다.
        if dy >= menutouch::PANEL_H {
            return self
                .footer_prompt_hit(dx, dy, false)
                .unwrap_or((None, "below the panel — only the footer prompts answer here".into()));
        }
        let (lw, lh) = crate::logical();
        let badges = menutouch::badges(&self.last_frame, lw as i32, lh as i32);
        if let Some(b) = menutouch::badge_at(&badges, dx, dy) {
            return (Some(MenuHit::Badge(badge_key(b))), format!("badge {b:?}"));
        }
        // ★ 136차 — the 「정렬」 button the HOST draws on the money strip, only while the
        // worker says a sortable screen is up (a popup, a modal, the cash shop: `None`,
        // and the strip is the game's again). The worker re-checks everything on arrival.
        if let Some(screen) = self.sort_screen
            && menutouch::sort_button_hit(dx, dy)
        {
            return (Some(MenuHit::SortItems), format!("정렬 button ({screen:?})"));
        }
        // ★★ A `예 / 아니요` confirm the game's MODULE drew itself (제련's "착용중인 장비
        // 입니다. 계속하시겠습니까?"). Its selection lives nowhere in the character record —
        // RIGHT moves it with zero bytes changing — so the pixels are the only source, and
        // that makes this the ONE thing on screen while it is up: tapping the selected
        // button is OK, tapping the other one is the arrow that walks over to it.
        if let Some((sel, other)) = menutouch::confirm_pair(&self.last_frame, lw as i32, lh as i32) {
            return if sel.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::OK)), "confirm pair: selected -> OK".into())
            } else if other.contains(dx, dy) {
                let k = if other.dx < sel.dx { KeyCode::LEFT } else { KeyCode::RIGHT };
                (Some(MenuHit::Badge(k)), format!("confirm pair: other -> {k:?}"))
            } else {
                (None, format!("confirm pair OWNS the panel (sel {sel:?} other {other:?}) — miss"))
            };
        }
        // A message box's centred `OK` button (제련 결과창 · 상점 대화창). Same idea as the
        // footer prompt: the game drew a button, so the button is the button.
        if let Some(r) = menutouch::ok_button(&self.last_frame, lw as i32, lh as i32)
            && r.contains(dx, dy)
        {
            return (Some(MenuHit::Badge(KeyCode::OK)), format!("ok button {r:?}"));
        }
        // ★★★ 80차-K — the TITLE CAROUSEL (`≪ 항목 ≫`), and it is also the 「게임을
        // 종료하시겠습니까?」 confirm: that box draws its 예/아니오 with the SAME widget
        // (headless probe, six screens byte-identical), which is why none of the three
        // confirm recognisers could see it. Player: *"메인화면에서도 터치가 안되고있어.
        // 다음 화살표 누르면 다음인 네트워크창고, 환경설정 이런식으로 … 게임종료 눌러서
        // 게임 종료하시겠습니까? 에서도 터치 안되고있어"*.
        //
        // The whole line is the target, split in three: everything left of `≪` steps
        // back, everything right of `≫` steps forward, the label between them is OK.
        // Nothing else is drawn on that line on any of the six screens.
        //
        // ★ 80차-P — ONE RULE FOR A MISS: every arm from here down that recognises a
        // window or list which OWNS the panel answers a miss with `MenuHit::Swallow`
        // (carousel · pager · OPTION · spinner · slot list alike). `None` would let the
        // touch fall through the chain to `tap_anywhere_key`, which is OK in a cutscene
        // with a choice up — the one mode besides Menu this function runs in. The footer
        // OVERHANG below the panel is the deliberate exception: there only the two footer
        // prompts answer, and a miss there is `None` because nothing owns the game's
        // background (review, 2026-09-06).
        // ★★★ 80차-P — a footer WORD the game printed beats any window that owns the panel:
        // the title OPTION/HELP windows put `CLR : 메인메뉴` in the footer band, and their
        // arms below swallow every miss inside the panel — including that word (device,
        // vc271: `panel (110,155) -> option owns the panel — miss`). Words only here; the
        // strip and the "on no word" rule stay in the later call.
        if dy >= menutouch::FOOTER_BOX_TOP
            && let Some(v) = self.footer_prompt_hit(dx, dy, true)
        {
            return v;
        }
        if let Some(c) = menutouch::title_carousel(&self.last_frame, lw as i32, lh as i32) {
            return if c.prev.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::LEFT)), format!("carousel prev {:?}", c.prev))
            } else if c.next.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::RIGHT)), format!("carousel next {:?}", c.next))
            } else if c.pick.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::OK)), format!("carousel pick {:?}", c.pick))
            } else {
                (
                    Some(MenuHit::Swallow),
                    format!("carousel line only (prev {:?} next {:?}) — miss", c.prev, c.next),
                )
            };
        }
        // ★★★ 80차-L, generalised in 80차-N — the PAGER HEADER `◀ 제목 ▶`. Three screens
        // share it: 게임방법(HELP), and the new-character 국가(FORCE) and 직업(CLASS).
        // The arrows page/choose (LEFT/RIGHT), the title strip commits (OK) and the body
        // scrolls (UP/DOWN) — all four probe-confirmed, none assumed.
        //
        // ★ OK on HELP is a no-op (probe: the frame does not change by a pixel), which is
        // what makes one rule safe for all three.
        if let Some(h) = menutouch::pager_header(&self.last_frame, lw as i32, lh as i32) {
            return if h.prev.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::LEFT)), format!("pager prev {:?}", h.prev))
            } else if h.next.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::RIGHT)), format!("pager next {:?}", h.next))
            } else if h.pick.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::OK)), format!("pager pick {:?}", h.pick))
            } else if h.up.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::UP)), format!("pager scroll up {:?}", h.up))
            } else if h.down.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::DOWN)), format!("pager scroll down {:?}", h.down))
            } else {
                // The window's own frame: it owns the panel, so a miss does nothing rather
                // than falling through to whatever is drawn behind it.
                (Some(MenuHit::Swallow), "pager owns the panel — miss".into())
            };
        }
        // ★★★ 80차-K — 환경설정 (OPTION). The TITLE's and the in-game 시스템 → 환경설정 are
        // the same window. Seven rows, the cursor being the one that is NOT bright — the
        // 80차-C slot-list rule — and the selected row's `< N >` is a LEFT/RIGHT pair.
        //
        // ⛔ BEFORE the slot-list arm below: the shell SLOT list is three rows of the same
        // colour at the same pitch, 2 px higher, so `slot_rows` answers "cursor 0" on this
        // screen. Requiring all SEVEN rows is what tells them apart, and testing the
        // stricter one first is what keeps the answer.
        if crate::game_options::cursor().is_some() {
            return match crate::game_options::tap_row(dx, dy) {
                Some((cur, row)) if row != cur => {
                    let (key, n) = if row > cur { (KeyCode::DOWN, row - cur) } else { (KeyCode::UP, cur - row) };
                    (Some(MenuHit::Steps(key, n as u8)), format!("option {cur} -> {row}"))
                }
                Some(_) if dx >= menutouch::OPTION_VALUE_X0 => {
                    (Some(MenuHit::Badge(if dx < menutouch::OPTION_VALUE_SPLIT { KeyCode::LEFT } else { KeyCode::RIGHT })), "option value".into())
                }
                _ => (Some(MenuHit::Swallow), "option panel".into()),
            };
        }
        // ★★ 80차-F — 밑단 프롬프트 둘은 한 자리에 있다: [`Self::footer_prompt_hit`].
        // 패널 밑단 **바깥**의 밴드에서도 같은 함수가 답해야 하므로(위의 조기 return)
        // 판정이 두 벌이 되면 안 된다.
        if let Some(v) = self.footer_prompt_hit(dx, dy, false) {
            return v;
        }
        // ★★ The smith's quantity spinner. Its two triangles are UP/DOWN keys, and while
        // it is on screen NOTHING else in the panel is ours: that window (13) keeps the
        // shared widget's STALE row count, so a tap resolved as "row 0 = the cursor"
        // would fire OK — which on that screen is 제련하기 (66차-B).
        if let Some((up, down)) = menutouch::spinner_arrows(&self.last_frame, lw as i32, lh as i32) {
            return if up.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::UP)), format!("spinner up {up:?}"))
            } else if down.contains(dx, dy) {
                (Some(MenuHit::Badge(KeyCode::DOWN)), format!("spinner down {down:?}"))
            } else {
                (Some(MenuHit::Swallow), format!("spinner OWNS the panel (up {up:?} down {down:?}) — miss"))
            };
        }
        // ★★★ 80차-C — the shell SLOT list (게임시작 · 온라인 both — device-measured as the
        // SAME widget). It is a PIXEL detector like the ones above, so it belongs above
        // the live-character gate. Player: *"네트워크창고와 게임시작에서 캐릭터 터치로
        // 포커스 이동하는거 되어야된다"*.
        //
        // ⛔⛔ 80차-M: it USED to sit inside `stats.level == 0`, and that made it dead on
        // the ONLINE list in every session where a character had already been loaded —
        // `level` stays non-zero after a playthrough, so 타이틀 → 온라인 fell through to
        // `is not a row or a cell on this screen` (device, vc264). The gate that is
        // actually right is `self.shell`: this list only exists in app state 3, and that
        // word needs no character (66차-C).
        // ⛔ It STILL has to be a gate. Without one the six-row in-game MENU list matches:
        // its rows are the same colour, and with row 0 highlighted the three bands this
        // scans read "exactly one dark" — which is `slot_rows`' whole test.
        // ⛔ And it stays BELOW the OPTION arm, which is the other list that collides.
        //
        // ★★ 80차-M — THE SECOND TAP EXECUTES, reversing 80차-C's one exception.
        // Player, 2026-09-05: *"게임시작 및 네트워크 창고 접속할떄 이미 포커스되어있는
        // 슬롯을 다시한번더 터치하면 ok처럼 작동하도록 하고싶어"*. Every other screen has
        // worked that way since 66차-B; being the only one that did not is itself a cost.
        //
        // ⛔ WHAT 80차-C WAS AFRAID OF, written down so it is not re-derived: OK here
        // LOADS the slot, and 71차-L measured what a load costs — the `04 3d` ack writes
        // that slot back. The cursor starts on SLOT 1, so for THAT row the rule is a
        // one-tap execute, not a two-tap.
        // ★ What makes it acceptable is 71차-M: `vault::note_slot_loaded` re-pins on every
        // outgoing `04 3d <slot>`, so the ack lands on the slot the character was just
        // read from — the write is the record it already held. The 2026-08-26 hazard was a
        // STALE pin, and that is the half that got fixed.
        //
        // ★★ 80차-P — …and NOT on the ONLINE screens (창고 · 캐시샵), which are app state 3
        // too (66차-C). Their 8x2 item grids put parchment and dark cell frames through the
        // same three bands, and a frame in which exactly one band reads dark would hand a
        // grid tap to this arm instead of the worker's cell cursor (review, 2026-09-06 —
        // the grid taps were device-verified on vc252, BEFORE this arm moved to `shell`).
        // The shell SCREEN word tells them apart, and it needs no character either.
        // `None` (unreadable) keeps the arm live: the two slot lists are the common case.
        // ★ 130차: the save-slot window's arrows sit in the list's side margin, ABOVE the
        //   row arm — the rows own the panel and would swallow the margin as a miss.
        //   Only while the worker has published a window, on the two slot lists.
        if self.shell
            && crate::savevault::on_slot_list(self.shell_screen)
            && self.slot_window.is_some()
            && let Some(delta) = overlay::slot_window_hit(dx, dy)
        {
            return (Some(MenuHit::SlotScroll(delta)), format!("slot window arrow {delta:+}"));
        }
        if self.shell
            && self.shell_screen != Some(crate::items::SHELL_SCREEN_ONLINE)
            && let Some((cursor, rows)) = menutouch::slot_rows(&self.last_frame, lw as i32, lh as i32)
        {
            return match rows.iter().position(|r| r.contains(dx, dy)) {
                Some(row) if row == cursor => (Some(MenuHit::Badge(KeyCode::OK)), format!("slot row {row} is already focused -> OK")),
                Some(row) => {
                    let (code, n) = if row > cursor {
                        (KeyCode::DOWN, (row - cursor) as u8)
                    } else {
                        (KeyCode::UP, (cursor - row) as u8)
                    };
                    (Some(MenuHit::Steps(code, n)), format!("slot row {cursor} -> {row}: {code:?} x{n}"))
                }
                // The list OWNS the panel while it is up: a miss is a miss, not a point
                // for the worker to reinterpret against a character that may not be there.
                None => (Some(MenuHit::Swallow), format!("slot list OWNS the panel (cursor {cursor}) — miss")),
            };
        }
        // ★★★ 80차-B — THE LIVE-CHARACTER GATE LIVES HERE, and only here.
        //
        // Everything above this line is a target the GAME drew, resolved from pixels, and
        // answered with a key the game itself printed on it. Below it the point is handed
        // to the worker to resolve against CHAR — a cursor byte to write, a cell index to
        // cross-check — so without a character there is nothing to resolve it against.
        // ⛔ Do not lift this back onto `menu_tap_ok`: that is what made the title's own
        // `예 / 아니요` box, the warehouse and the cash shop dead to touch (vc244 report).
        if self.stats.level == 0 {
            return (None, "no live character (level 0) — the game's own targets only".into());
        }
        // No measured cursor on this build -> rows and cells stay off, badges do not.
        // The cell-cursor border comes along for the ride: the worker cannot see it.
        match crate::guest_build::menu_cursors() {
            Some(_) => {
                let item = menutouch::item_grid(&self.last_frame, lw as i32, lh as i32);
                // 79차-C: and the 상태보기 stat marker, for the same reason and by the same
                // division of labour — the worker cannot see pixels, and this one gates an
                // irreversible action (spending a point), so it is a cross-check.
                let status = menutouch::status_marker(&self.last_frame, lw as i32, lh as i32);
                let equip = menutouch::equip_marker(&self.last_frame, lw as i32, lh as i32);
                // ★★ …and, separately, whether a CELL cursor is drawn at all. `item_grid`
                // needs the game's empty-slot frames to derive the grid, so on a FULL bag
                // it answers `None` for a reason that has nothing to do with "this is a
                // row list" — see `InputEvent::MenuTap::cell_cursor`.
                let cell_cursor = menutouch::cell_box(&self.last_frame, lw as i32, lh as i32).is_some();
                let note = match item {
                    Some((g, c)) => format!(
                        "item grid {}x{} pitch {}x{} at ({},{}) cursor {c}",
                        g.cols, g.rows, g.pitch_x, g.pitch_y, g.x0, g.y0
                    ),
                    // ⛔ `cell_box_report` names the term that refused (2026-09-15): without
                    // it "cell cursor false" is a dead end, and the player is the only one
                    // who can reach the screen that produced it.
                    None => format!(
                        "no item grid (cell cursor {cell_cursor}, stat {status:?} equip {equip:?}) [{}]",
                        menutouch::cell_box_report(&self.last_frame, lw as i32, lh as i32)
                    ),
                };
                (
                    Some(MenuHit::Panel(gx, gy, item, status, equip, cell_cursor)),
                    format!("panel -> worker ({note})"),
                )
            }
            None => (None, "no measured cursors on this build".into()),
        }
    }

    /// 117차-D: re-blit the guest frame INSIDE the game's message-box frame over whatever
    /// was drawn there — the box window — so the game's own dialog shows through.
    ///
    /// Same mapping as the main blit (`sy = dy * lh / g.h`, `sx = sx_lut[dx]`), restricted
    /// to the rows and columns that land inside `menutouch::game_dialog_frame`. Nothing is
    /// drawn when the frame is not on screen: a reveal armed a frame early or late shows
    /// the window as it is, never a hole onto the bag page.
    ///
    /// An associated fn, not a method: `buf` is the surface's buffer and pins `self`
    /// mutably while the draw runs (see the main blit), so the pieces come in by field.
    fn reveal_game_dialog(buf: &mut [u32], win_w: u32, win_h: u32, g: overlay::Rect, lut: &[u32], last: &[u32]) {
        let (lw, lh) = crate::logical();
        if lw == 0 || lh == 0 || last.len() != (lw * lh) as usize {
            return;
        }
        let Some(b) = menutouch::game_dialog_frame(last, lw as i32, lh as i32) else {
            return;
        };
        let Some((ox, oy)) = menutouch::panel_origin(lw as i32, lh as i32) else {
            return;
        };
        // ★ 150차: the copy itself is `overlay::blit_logical_rect` now — the 상세보기 body
        // patch goes through the very same mapping.
        overlay::blit_logical_rect(
            buf,
            win_w,
            win_h,
            g,
            lut,
            (lw, lh),
            last,
            lw,
            (0, 0),
            ((ox + b.dx) as u32, (oy + b.dy) as u32, b.w as u32, b.h as u32),
        );
    }

    /// ★ 150차 — compose the 상세보기 body patch for THIS frame, or `None` when nothing of
    /// ours belongs on it. Fills `self.note_patch` (`itemnote::BODY_W` x `BODY_H` logical
    /// pixels) and returns the panel origin the patch is placed against.
    ///
    /// Every gate is on the GAME's frame (`last_frame`), never on anything composed: MENU
    /// mode, no editor, a note from the worker, the page-7 window really on screen
    /// (`menutouch::detail_page` — all its outline / border / clear rows, pixel-exact), and
    /// that window really showing the note's item (`itemnote::shows` — the name box reads
    /// back the note's name in its inks, and a dropped line reads exactly what it drops). A
    /// stale note therefore cannot land on an item of another NAME: it simply is not drawn.
    /// (Two same-named items are told apart by the worker, which builds the note from the
    /// cell the painter reads — at worst one frame late; see the `itemnote.rs` header.)
    ///
    /// The scroll is clamped HERE as well as on input (an unclamped value composes an empty
    /// body — `itemnote::clamp_scroll`).
    fn compose_note(&mut self) -> Option<(i32, i32)> {
        if self.mode != overlay::Mode::Menu || self.editor.is_some() {
            return None;
        }
        let note = self.item_note.as_deref()?;
        let (lw, lh) = crate::logical();
        let (lw, lh) = (lw as i32, lh as i32);
        if !menutouch::detail_page(&self.last_frame, lw, lh) {
            return None;
        }
        let origin = menutouch::panel_origin(lw, lh)?;
        let f = crate::itemnote::Frame { px: &self.last_frame, w: lw as usize, h: lh as usize };
        if !crate::itemnote::shows(f, origin, note) {
            return None;
        }
        let n = crate::itemnote::BODY_W * crate::itemnote::BODY_H;
        self.note_game.resize(n, 0);
        self.note_patch.resize(n, 0);
        if !crate::itemnote::body_patch(f, origin, &mut self.note_game) {
            return None;
        }
        // A window that has just come up starts at the top of its body (`note_up` still
        // says what the PREVIOUS frame showed) — reopening an item never lands mid-scroll.
        if !self.note_up {
            self.note_scroll = 0;
            self.note_drag = None;
        }
        self.note_scroll = crate::itemnote::clamp_scroll(note, self.note_scroll);
        crate::itemnote::compose(&self.note_game, note, self.note_scroll, &mut self.note_patch).then_some(origin)
    }

    /// ★ 167차 — compose the AVATAR page for THIS frame, or `None` when it does not belong on it.
    /// Fills `self.avatar_patch` (`avatar_page::W` x `H` logical px) and returns the panel
    /// origin. Every gate is on the GAME's frame, like `compose_note`: MENU mode, no editor, a
    /// view from the worker, and EQUIPMENT's worn grid really on screen — its yellow cell frame
    /// on the left grid (`menutouch::equip_marker`) and the header reading EQUIPMENT
    /// (`avatar_page::equipment_on_screen`), both of which the game keeps drawing under the page
    /// because it never left EQUIPMENT. A stale view over any other screen is not drawn.
    fn compose_avatar(&mut self) -> Option<(i32, i32)> {
        if self.mode != overlay::Mode::Menu || self.editor.is_some() {
            return None;
        }
        self.avatar_view.as_ref()?;
        let (ox, oy) = self.equipment_grid_shown()?;
        let view = self.avatar_view.as_deref()?;
        let atlas = crate::atlas::atlas();
        let icons: [Option<crate::avatar_page::Icon>; 6] = std::array::from_fn(|s| {
            atlas.item(view.icon[s]).map(|ic| crate::avatar_page::Icon { w: usize::from(ic.w), h: usize::from(ic.h), px: &ic.px })
        });
        crate::avatar_page::compose(&self.avatar_game, view, &icons, &mut self.avatar_patch).then_some((ox, oy))
    }

    /// The game's EQUIPMENT page in its worn-grid mode on THIS frame — the header reading
    /// EQUIPMENT (`avatar_page::equipment_on_screen`) and the yellow cell frame on the left grid
    /// (`menutouch::equip_marker`). Leaves the panel in `avatar_game` and returns its origin.
    fn equipment_grid_shown(&mut self) -> Option<(i32, i32)> {
        let (lw, lh) = crate::logical();
        let (lw, lh) = (lw as i32, lh as i32);
        if self.last_frame.len() != (lw * lh) as usize {
            return None;
        }
        if menutouch::equip_marker(&self.last_frame, lw, lh).is_none_or(|m| m.candidates) {
            return None;
        }
        let (ox, oy) = menutouch::panel_origin(lw, lh)?;
        let (w, h) = (crate::avatar_page::W, crate::avatar_page::H);
        self.avatar_game.resize(w * h, 0);
        self.avatar_patch.resize(w * h, 0);
        for y in 0..h {
            let src = (oy as usize + y) * lw as usize + ox as usize;
            self.avatar_game[y * w..(y + 1) * w].copy_from_slice(&self.last_frame[src..src + w]);
        }
        crate::avatar_page::equipment_on_screen(&self.avatar_game).then_some((ox, oy))
    }

    /// ★ 168차 — compose the seven-row MENU list for THIS frame, or `None` when it does not belong
    /// on it: MENU mode, no editor, the worker offering the AVATAR row, and the game's own
    /// six-row list really on screen (`avatar_page::menu_on_screen`). A row the worker forced
    /// (`MenuSel::Row`) is dropped back to the game's own selection once the frame agrees with
    /// it — the game has painted the move — so it can never outstay a later move of the game's.
    fn compose_menu7(&mut self) -> Option<(i32, i32)> {
        if self.mode != overlay::Mode::Menu || self.editor.is_some() || self.menu_sel == crate::avatar::MenuSel::Off {
            return None;
        }
        let (lw, lh) = crate::logical();
        let (lw, lh) = (lw as i32, lh as i32);
        if self.last_frame.len() != (lw * lh) as usize {
            return None;
        }
        let (ox, oy) = menutouch::panel_origin(lw, lh)?;
        let (w, h) = (crate::avatar_page::W, crate::avatar_page::H);
        self.menu_game.resize(w * h, 0);
        self.menu_patch.resize(w * h, 0);
        for y in 0..h {
            let src = (oy as usize + y) * lw as usize + ox as usize;
            self.menu_game[y * w..(y + 1) * w].copy_from_slice(&self.last_frame[src..src + w]);
        }
        let game_row = crate::avatar_page::menu_on_screen(&self.menu_game)?;
        if let crate::avatar::MenuSel::Row(r) = self.menu_sel
            && r == crate::avatar::menu_row_of_game(game_row)
        {
            self.menu_sel = crate::avatar::MenuSel::Game;
        }
        let row = self.menu_sel.row(game_row)?;
        crate::avatar_page::compose_menu(&self.menu_game, row, &mut self.menu_patch).then_some((ox, oy))
    }

    /// ★ 168차 — what a tap at window `(x, y)` means while the seven-row MENU list is on screen:
    /// a row, or — anywhere else inside the panel — nothing (the list owns the panel; the game's
    /// six rows under it must not answer). `None` = the list is not up, or the point is outside
    /// the panel.
    fn menu7_tap_hit(&self, x: f64, y: f64) -> Option<MenuHit> {
        if !self.menu_up {
            return None;
        }
        let (lw, lh) = crate::logical();
        let (gx, gy) = self.guest_px(x, y);
        let (dx, dy) = menutouch::panel_point(lw as i32, lh as i32, gx, gy)?;
        let hit = match crate::avatar_page::menu_hit(dx, dy)? {
            Some(row) => MenuHit::MenuRow(row),
            None => MenuHit::Swallow,
        };
        log::info!("menu tap: panel ({dx},{dy}) -> seven-row list {hit:?}");
        Some(hit)
    }

    /// ★ 167차 — what a tap at window `(x, y)` means while the AVATAR page is on screen: the
    /// page owns the whole panel (a miss inside it is swallowed, never passed to EQUIPMENT's
    /// grid underneath). `None` = the page is not up, or the point is outside the panel.
    fn avatar_tap_hit(&self, x: f64, y: f64) -> Option<MenuHit> {
        if !self.avatar_up {
            return None;
        }
        let (lw, lh) = crate::logical();
        let (gx, gy) = self.guest_px(x, y);
        let (dx, dy) = menutouch::panel_point(lw as i32, lh as i32, gx, gy)?;
        let hit = match crate::avatar_page::hit(dx, dy)? {
            crate::avatar_page::Hit::Act(a) => MenuHit::Avatar(a),
            crate::avatar_page::Hit::Swallow => MenuHit::Swallow,
        };
        log::info!("avatar tap: panel ({dx},{dy}) -> {hit:?}");
        Some(hit)
    }

    /// ★ 150차: a key for the 상세보기 body — UP/DOWN a line, LEFT/RIGHT a page — while the
    /// overlay is on screen AND its content overflows. `true` = consumed (the caller keeps
    /// the key from the game). Anything else, and every key while there is nothing to
    /// scroll, goes to the game: on page 7 only CLR does anything there (`0x46340`), and CLR
    /// and OK are never taken here.
    fn note_key(&mut self, key: KeyCode) -> bool {
        if !self.note_up {
            return false;
        }
        let Some(note) = self.item_note.as_deref() else { return false };
        if crate::itemnote::max_scroll(note) == 0 {
            return false;
        }
        let step = match key {
            KeyCode::UP => -crate::itemnote::SCROLL_LINE,
            KeyCode::DOWN => crate::itemnote::SCROLL_LINE,
            KeyCode::LEFT => -crate::itemnote::SCROLL_PAGE,
            KeyCode::RIGHT => crate::itemnote::SCROLL_PAGE,
            _ => return false,
        };
        self.note_scroll = crate::itemnote::clamp_scroll(note, self.note_scroll + step);
        self.request_redraw();
        true
    }

    /// The panel-relative point under window `(x, y)` when it lies in the 상세보기 BODY and
    /// the overlay is on screen — the only place a touch is ours on that window.
    fn note_body_point(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        if !self.note_up || self.item_note.is_none() {
            return None;
        }
        let (lw, lh) = crate::logical();
        let (gx, gy) = self.guest_px(x, y);
        let (dx, dy) = menutouch::panel_point(lw as i32, lh as i32, gx, gy)?;
        menutouch::DETAIL_BODY.contains(dx, dy).then_some((dx, dy))
    }

    /// A finger came down on the body: remember where, and wait — a drag scrolls, a clean
    /// tap steps one line (HELP-style: upper half up, lower half down). `true` = claimed.
    fn note_started(&mut self, id: u64, x: f64, y: f64) -> bool {
        let Some((_, dy)) = self.note_body_point(x, y) else { return false };
        // A second finger does not take the drag off the first; it is swallowed with it.
        if self.note_drag.is_none() {
            self.note_drag = Some((id, y, self.note_scroll, false, dy));
        }
        self.ui_touches.insert(id);
        true
    }

    /// A drag of the body, measured from where the finger went down (the orb list's rule:
    /// accumulating deltas drifts). Logical px, so it means the same on every screen.
    fn note_moved(&mut self, y: f64) {
        let Some((id, y0, scroll0, moved, dy)) = self.note_drag else { return };
        let Some(note) = self.item_note.as_deref() else { return };
        let (_, lh) = crate::logical();
        let to_logical = f64::from(lh) / f64::from(self.layout.game.h.max(1));
        let travel = (y - y0) * to_logical;
        let moved = moved || travel.abs() >= Self::NOTE_DRAG_SLOP;
        self.note_drag = Some((id, y0, scroll0, moved, dy));
        if moved {
            self.note_scroll = crate::itemnote::clamp_scroll(note, scroll0 - travel.round() as i32);
            self.request_redraw();
        }
    }

    /// The finger lifted: a clean tap that never dragged steps a line.
    fn note_ended(&mut self, clean: bool) {
        let Some((_, _, _, moved, dy)) = self.note_drag.take() else { return };
        if !clean || moved {
            return;
        }
        if let Some(note) = self.item_note.as_deref() {
            self.note_scroll = crate::itemnote::clamp_scroll(note, self.note_scroll + crate::itemnote::tap_step(dy));
            self.request_redraw();
        }
    }

    /// How far (LOGICAL px) a finger on the body has to travel before it is a drag, not a
    /// tap — a quarter of the game's line, the orb list's `ORB_DRAG_SLOP_PCT` of a row.
    const NOTE_DRAG_SLOP: f64 = 4.0;

    /// Window px -> guest logical-framebuffer px. The game fills `layout.game` 1:1
    /// (immersive) or letterboxed (classic); either way the rect IS the guest frame.
    fn guest_px(&self, x: f64, y: f64) -> (i32, i32) {
        let g = &self.layout.game;
        let (lw, lh) = crate::logical();
        let gx = (x - f64::from(g.x)) * f64::from(lw) / f64::from(g.w.max(1));
        let gy = (y - f64::from(g.y)) * f64::from(lh) / f64::from(g.h.max(1));
        (gx as i32, gy as i32)
    }

    /// SAVE NOW as a TAP TARGET, or `None` when it is not on screen.
    ///
    /// One predicate for both the draw and the hit-test, for the reason `stats_tap_zone`
    /// gives: a zone that outlives its drawing is an invisible button, and here it would
    /// be an invisible button that saves. Landscape-immersive FIELD only — the field
    /// because the guest's own gate passes nowhere else, landscape-immersive because that
    /// is the only HUD with placeable controls (every other layout builds a fixed keypad
    /// from the screen edges, and a floating disc would land on top of a real key).
    /// `can_save` too (2026-08-07): in a no-save AREA (boss room) the button is gone
    /// entirely rather than present-but-refusing.
    /// ★ NOT gated on whether the save chain is ported (2026-08-11, reverted the same
    /// day): a feature that already shipped does not get hidden because a carrier port
    /// is unfinished. Hiding it was a temporary paper-over; the fix is to finish
    /// MIGRATION_LGT.md §4-3, and until then the control stays where the player put it.
    fn quicksave_zone(&self) -> Option<overlay::Rect> {
        (self.mode == overlay::Mode::Field && self.can_save && crate::immersive_active() && self.win_w > self.win_h)
            .then(|| placement::rect_of(placement::CtrlId::QuickSave, overlay::Mode::Field, self.win_w, self.win_h))
            .flatten()
    }

    /// ★ 143차: the EXIT tab as a TAP TARGET, or `None` when it is not on screen — one
    /// predicate for the draw and the hit-test, `quicksave_zone`'s rule. The worker decides
    /// whether the tower offers it (`tower_exit`); the same FIELD / immersive-landscape gate
    /// as SAVE and MAP, whose row it stands in; never under the wake's veil.
    fn exit_zone(&self) -> Option<overlay::Rect> {
        (self.mode == overlay::Mode::Field
            && self.tower_exit
            && self.veil.is_none()
            && crate::immersive_active()
            && self.win_w > self.win_h)
            .then(|| placement::rect_of(placement::CtrlId::MinimapToggle, overlay::Mode::Field, self.win_w, self.win_h))
            .flatten()
            .map(|map| overlay::exit_tab_rect(self.win_w, map))
    }

    /// The MAP tab as a tap target (141차 toggle): FIELD, a save-able AREA (the tower and the
    /// boss rooms hide the whole minimap UI), wherever the resolver put it.
    fn minimap_toggle_zone(&self) -> Option<overlay::Rect> {
        (self.mode == overlay::Mode::Field && self.can_save)
            .then(|| placement::rect_of(placement::CtrlId::MinimapToggle, overlay::Mode::Field, self.win_w, self.win_h))
            .flatten()
    }

    /// ★ 146차: the top-row tab whose HALO (`overlay::top_tab_halo`, the enlarged tap area)
    /// is under `(x, y)`. Neighbouring halos meet in the middle of their gap, so at most one
    /// answers; MAP and SAVE never share the row with EXIT (the tower hides them).
    fn top_tab_halo_hit(&self, x: f64, y: f64) -> Option<TopTab> {
        let hit = |r: overlay::Rect| overlay::rect_hit(&overlay::top_tab_halo(r, self.win_w, self.win_h), x, y);
        if self.minimap_toggle_zone().is_some_and(hit) {
            Some(TopTab::Map)
        } else if self.exit_zone().is_some_and(hit) {
            Some(TopTab::Exit)
        } else if self.quicksave_zone().is_some_and(hit) {
            Some(TopTab::Save)
        } else {
            None
        }
    }

    /// What a top-row tab does when pressed — one body for the box and for its halo.
    fn press_top_tab(&mut self, id: u64, tab: TopTab) {
        match tab {
            // ★ 141차: a TOGGLE, drawn on the top row whether the view is up or not.
            TopTab::Map => self.minimap_on = !self.minimap_on,
            // ★ 143차: the EXIT tab asks first — leaving ends the run (and a thumb resting
            // near the top row must not throw anyone out). Default 예, like the game's own
            // boxes: the tab was the choice.
            TopTab::Exit => self.exit_ask = Some((0, Instant::now())),
            // Same one-way report as the page button: the worker arms the save chain,
            // checks the guest's own gate and answers with a notice.
            TopTab::Save => {
                let _ = self.input_tx.send(InputEvent::QuickSave);
            }
        }
        self.ui_touches.insert(id);
        self.request_redraw();
    }

    /// The tap that opened 「하늘성에서 나가시겠습니까?」 must not also answer it (the 70차 /
    /// 93차 hazard, the list dialog's `ORB_ARM_MS`).
    const EXIT_ARM_MS: u128 = 250;

    /// Answer the exit box: `yes` sends the request, either way the box goes.
    fn exit_answer(&mut self, yes: bool) {
        if yes {
            let _ = self.input_tx.send(InputEvent::TowerExit);
        }
        self.exit_ask = None;
        self.request_redraw();
    }

    /// The exit box's keys (`qty_key`'s arm): ←/→/↑/↓ move between 예 and 아니오, OK takes
    /// the selected one, CLR is 아니오. Everything else is swallowed — it is a modal.
    fn exit_key(&mut self, key: KeyCode) {
        let Some((sel, at)) = self.exit_ask else { return };
        match key {
            KeyCode::LEFT | KeyCode::RIGHT | KeyCode::UP | KeyCode::DOWN => {
                self.exit_ask = Some((1 - sel.min(1), at));
                self.request_redraw();
            }
            KeyCode::OK | KeyCode::NUM5 => {
                if at.elapsed().as_millis() >= Self::EXIT_ARM_MS {
                    self.exit_answer(sel == 0);
                }
            }
            KeyCode::CLEAR => self.exit_answer(false),
            _ => {}
        }
    }

    /// The wake's veil right now: its alpha and whether it is still moving (the caller
    /// keeps redrawing while it is). `None` = no veil.
    fn veil_now(&self) -> Option<(u8, bool)> {
        let (from, to, since, ms) = self.veil?;
        let el = since.elapsed().as_millis() as u64;
        Some((overlay::wake_veil_alpha(from, to, el, ms), el < u64::from(ms)))
    }

    /// Every key and finger up, and every half-made gesture dropped — the wake's veil takes
    /// the screen away mid-press, and a held direction would otherwise walk the player on
    /// into the town it lands them in. The same list `Focused(false)` releases.
    fn release_all_input(&mut self) {
        self.release_gamepad();
        for key in self.pressed.drain() {
            let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
        }
        let _ = self.input_tx.send(InputEvent::Walk { dir: None });
        self.touches.clear();
        self.popup_confirm_touches.clear();
        self.ui_touches.clear();
        self.slide_touches.clear();
        self.qty_slid.clear();
        self.mm_tap = None;
        self.stats_tap = None;
        self.menu_tap = None;
        self.note_drag = None;
        self.joy = None;
        self.quick_press = None;
    }

    fn relayout(&mut self) {
        self.layout = overlay::layout(self.win_w, self.win_h, self.joystick, self.mode, self.choice);
        self.settings_items = overlay::settings_items(self.win_w, self.win_h);
        self.relayout_quick();
        // Tell the worker where the game's own top band has to go, now that we know
        // how tall our gauge block is at this window size.
        // Packed for the worker: [31:24] target-box left shift, [23:16] band height,
        // [15:8] band x, [7:0] y origin.
        let packed = if crate::immersive_active() && self.win_w > self.win_h {
            let g = self.layout.game;
            (overlay::target_box_shift(g).min(255) << 24)
                | (overlay::band_h_shift(g).min(255) << 16)
                | (overlay::band_x_shift(g).min(255) << 8)
                | overlay::hud_y_shift(g).min(255)
        } else {
            (25 << 16) | (1 << 8) // the game's own defaults
        };
        self.hud_y.store(packed, Ordering::Relaxed);
    }

    /// Recompute quickbar-button rects for the current game rect + slot count.
    fn relayout_quick(&mut self) {
        self.quick_rects = overlay::quick_buttons(self.layout.game, self.layout.gear, self.quickbar.len(), self.mode);
        // ★ The whole of the "메뉴 열면 우측 퀵바가 안 보인다" diagnosis, in one line, on the
        // UI side — where the worker's `quickbar probe` cannot see. Static reading has
        // ruled out everything it can (`MENU_IDS` holds `Quick(0..4)`; `resolve_with`
        // returns a built-in even when a stored placement is rejected, so it never drops
        // a seat; `layout_immersive` gives `game` the whole window in EVERY mode; nothing
        // in `draw_quickbar` looks at the mode; the guest frame is blitted BEFORE the
        // overlay, so its window cannot cover the plates). What is left needs values:
        //
        //   mode=Cutscene  -> a menu was misclassified as the SCRIPT window and
        //                     `quick_buttons` returns empty by design.
        //   slots=0        -> the worker emptied the list (`in_game_state` false).
        //   rects=0 w/ slots>0 and mode=Menu -> `quick_buttons`/the resolver.
        //   rects=5, kinds all Empty -> they ARE drawn, but an empty plate is "nothing
        //                     but its dim rim" at alpha 70, which is indistinguishable
        //                     from absent on a dark menu.
        //
        // Cheap: one line per relayout, and a relayout only happens on a mode change, a
        // resize or a slot-list change.
        //
        //   hidden=0x60 -> the player switched plates 6 and 7 off in the layout editor,
        //                  and `off` counts the zero-sized rects that answers with. A
        //                  report of "퀵슬롯이 안 보인다" is that line or it is a bug.
        let hidden = crate::quick_hidden();
        log::info!(
            "quickbar ui: mode={:?} immersive={} landscape={} game={}x{} slots={} rects={} hidden={:#04x} off={} kinds={:?}",
            self.mode,
            crate::immersive_active(),
            self.win_w > self.win_h,
            self.layout.game.w,
            self.layout.game.h,
            self.quickbar.len(),
            self.quick_rects.len(),
            hidden,
            self.quick_rects.iter().filter(|r| r.w == 0 || r.h == 0).count(),
            self.quickbar.iter().map(|s| s.kind).collect::<Vec<_>>(),
        );
    }

    /// Did this WINDOW-px touch land on the game's action popup, and on which row?
    ///
    /// `Some((row, sel))` = inside the box; `row` is `None` for the box's own chrome,
    /// which the caller still swallows because the popup is modal. `None` = no popup,
    /// or the touch is somewhere else entirely.
    ///
    /// ⛔ Not while the plates are lit: mid-registration the popup is still on screen
    /// but the question being asked is "which plate", and the plates own the tap.
    fn popup_row_hit(&self, x: f64, y: f64) -> Option<(Option<u8>, i8)> {
        // ⛔ …and NOT with the pull-out keypad open, for the reason `menu_tap_ok`
        // spells out and this arm was missing (79차-C review): the keypad is drawn
        // CENTRED on the window, which in immersive mode is where the game centres its
        // own 176x160 panel — i.e. right on top of the action popup — and this scan
        // reads the GUEST framebuffer (`last_frame`), which the keypad does not cover.
        // `overlay::hit` is an exact rect test, so a thumb in a key gutter or on the
        // one empty cell fell through to here and became `PopupSelectRow` — or, on the
        // row the cursor already sat on, `PopupConfirm`, whose row 0 is `사용하기`.
        // This arm sits 110 lines ABOVE the keypad's own hit-test, so it wins.
        let list = self.popup_list.filter(|_| !self.picking_slot)?;
        let rows = i32::from(list.rows);
        // The scan and the geometry are in LOGICAL guest px — the coordinate system the
        // game re-anchors its own chrome to. `guest_px` is the same conversion
        // `menu_tap_hit` uses.
        let (lw, lh) = crate::logical();
        let (lw, lh) = (lw as i32, lh as i32);
        let (lx, ly) = self.guest_px(x, y);
        let (bx, by) = menutouch::popup_box(&self.last_frame, lw, lh, rows)?;
        menutouch::popup_contains(bx, by, rows, lx, ly).then(|| (menutouch::popup_row_at(bx, by, rows, lx, ly), list.sel))
    }

    /// Fraction of the skill cooldown ELAPSED for quickbar slot `slot` (0..1), or
    /// `None` when the slot is ready. Resolved through the slot's skill IDENTITY
    /// (sheet, cell) — see `skill_cd`. Expired entries are cleared lazily here, so
    /// nothing has to tick a timer.
    fn cooldown_frac(&mut self, slot: u8) -> Option<f64> {
        let icon = self.quickbar.iter().find(|s| s.slot == slot).and_then(|s| s.icon)?;
        let crate::IconRef::Skill(sheet, cell) = icon else { return None };
        let (t0, dur) = *self.skill_cd.get(&(sheet, cell))?;
        let ms = t0.elapsed().as_millis();
        if dur == 0 || ms >= u128::from(dur) {
            self.skill_cd.remove(&(sheet, cell));
            return None;
        }
        Some(ms as f64 / dur as f64)
    }

    /// Is the gauge block (LV plate + HP/SP/EXP + badges) shown right now? The field
    /// always shows it; since 2026-08-05 a game WINDOW shows it too (player request:
    /// "메뉴 버튼 열렸을때 좌측상단 UI는 다 보이게") — but only in the immersive
    /// landscape HUD, only once a live character has been seen (`level > 0` filters
    /// the boot default), and never over the title/slot shell.
    fn stats_hud_on(&self) -> bool {
        match self.mode {
            overlay::Mode::Field => true,
            overlay::Mode::Menu => !self.shell && self.stats.level > 0 && crate::immersive_active() && self.win_w > self.win_h,
            overlay::Mode::Cutscene => false,
        }
    }

    /// Rebuild `pressed` (highlight set) from the held touch keys + joystick dir.
    fn refresh_pressed(&mut self) {
        let mut p: HashSet<KeyCode> = self.touches.values().copied().collect();
        if let Some(k) = self.joy.as_ref().and_then(|j| j.dir) {
            p.insert(k);
        }
        self.pressed = p;
    }

    /// Update the joystick from a new thumb position: resolve the single walk
    /// direction against the FIXED centre this drag spawned at, and emit keyup(old)
    /// before keydown(new) so the guest re-latches on every turn. `active` = finger
    /// down.
    ///
    /// ★ `joy.cx/cy` is written ONCE, at touch-down, and never again for the life of
    /// the touch (player request 2026-08-03 — see the note on `overlay::joy_dir` for
    /// what the trailing base used to do and why it went).
    fn joy_update(&mut self, x: f64, y: f64, active: bool) {
        // ★ 91차-B: the ENGAGE radius, not the pin radius. The base disc is the whole
        // arrows' square now and the thumb pins inside it (`joy_radius`, used by the
        // render below), but the distance a thumb travels before walking starts is
        // still the pre-91차 one — see `overlay::joy_engage_radius`.
        let radius = self
            .layout
            .joystick
            .as_ref()
            .map(|z| overlay::joy_engage_radius(z, self.win_w, self.win_h))
            .unwrap_or(1.0);
        let Some(joy) = self.joy.as_mut() else { return };
        joy.tx = x;
        joy.ty = y;
        let new_dir = if active {
            overlay::joy_dir(x - joy.cx, y - joy.cy, radius, joy.dir)
        } else {
            None
        };
        if new_dir != joy.dir {
            // Release first, then press: the guest latches a walk direction on
            // keydown, so the new direction must arrive AFTER the old one is gone.
            if let Some(old) = joy.dir {
                let _ = self.input_tx.send(InputEvent::Key { code: old, down: false });
            }
            if let Some(new) = new_dir {
                let _ = self.input_tx.send(InputEvent::Key { code: new, down: true });
            }
            joy.dir = new_dir;
            // ★★★ 2026-09-20: and tell the worker what the THUMB is holding, so it can
            // re-supply the walk's keydown edge after anything that interrupted the walk
            // (an attack, a skill, a wall). See `InputEvent::Walk`. Sent AFTER the pair
            // above — the worker's re-press is gated on the character being idle, and
            // the fresh keydown is what takes it out of idle.
            let _ = self.input_tx.send(InputEvent::Walk { dir: new_dir });
        }
        self.refresh_pressed();
    }

    /// Adopt touch `id` as the walk stick and resolve its first direction. The two
    /// STICK modes only — under 십자키 the layout publishes no joystick zone (the
    /// field walks with the D-pad arrows), so nothing can ever call this.
    ///
    /// Where the centre comes from is the whole difference between the two sticks:
    ///
    ///  * 자유 스틱 — the finger's own landing point, so the first press is always
    ///    neutral and the base appears wherever the thumb is.
    ///  * 고정 스틱 (2026-09-22) — the SEAT's centre (`overlay::joy_home`), the same
    ///    point the idle base is drawn at, so the base never moves and a touch that
    ///    lands off-centre already leans the stick that way — which is what a pinned
    ///    stick is for: the thumb aims at the drawn disc, not at empty ground. The
    ///    zone is the seat's square (`layout_immersive`), so a touch that can reach
    ///    this arm is never further from the centre than the arrows' corner.
    ///
    /// ⛔ IMMERSIVE only, like the setting itself (its row is gated on
    /// `layout_editor_available`): the classic layouts build their own left region
    /// and never read the mode, and `joy_home` is the immersive seat — pinning a
    /// classic drag to it would centre the stick somewhere outside its own zone.
    fn joy_start(&mut self, id: u64, x: f64, y: f64) {
        let (cx, cy) = if crate::immersive_active() && crate::stick_mode().stick_is_pinned() {
            let (hx, hy, _) = overlay::joy_home(self.win_w, self.win_h);
            (hx, hy)
        } else {
            (x, y)
        };
        self.joy = Some(Joy {
            id,
            cx,
            cy,
            tx: x,
            ty: y,
            dir: None,
        });
        // 자유 스틱: harmless (dx = dy = 0, so `joy_dir` answers `None`). 고정 스틱:
        // the first direction, if the touch landed past the deadzone.
        self.joy_update(x, y, true);
        self.request_redraw();
    }

    /// Release the joystick entirely (held direction up).
    fn joy_end(&mut self) {
        if let Some(joy) = self.joy.take()
            && let Some(k) = joy.dir
        {
            let _ = self.input_tx.send(InputEvent::Key { code: k, down: false });
            // ⛔ Unconditionally, and only when a direction was actually held: the worker
            // must forget the walk order in the SAME order the guest sees it end, or its
            // re-press would put back a key whose keyup has already gone (see
            // `InputEvent::Walk`). A stick with no direction never armed one.
            let _ = self.input_tx.send(InputEvent::Walk { dir: None });
        }
        self.refresh_pressed();
    }

    fn configure_surface(&mut self) {
        let (Some(context), Some(window)) = (self.context.as_ref(), self.window.as_ref()) else {
            return;
        };
        if self.surface.is_none() {
            match Surface::new(context, window.clone()) {
                Ok(s) => self.surface = Some(s),
                Err(e) => {
                    log::error!("Surface::new failed: {e}");
                    return;
                }
            }
        }
        // RENDER size, not the physical window (57차): with the HEAT SAVER divisor
        // this asks `ANativeWindow_setBuffersGeometry` for a half-size buffer and
        // the compositor upscales it to the window in hardware.
        let (Some(w), Some(h)) = (NonZeroU32::new(self.win_w), NonZeroU32::new(self.win_h)) else {
            return;
        };
        if let Some(surface) = self.surface.as_mut()
            && let Err(e) = surface.resize(w, h)
        {
            log::error!("surface resize failed: {e}");
        }
    }

    /// Frames of 하늘성 entrance flash: six alternations at two frames each. The guest
    /// runs at ~14-24 fps here, so this is roughly half a second either way.
    const TOWER_FLASH_FRAMES: u32 = 12;

    /// Request a coalesced redraw instead of drawing inline. winit merges any
    /// number of requests between frames into a single `RedrawRequested`, so the
    /// joystick's high-rate touch-move stream (which on a 120/240 Hz-sampling
    /// panel would otherwise trigger one full-screen blit + `present()` per
    /// event — the cause of the drag lag) collapses to at most one draw per frame.
    fn request_redraw(&self) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let (win_w, win_h) = (self.win_w, self.win_h);
        let landscape = win_w > win_h;
        // ★ Everything below is resolved BEFORE the surface is borrowed: `buffer_mut`
        // holds a mutable borrow of `self` for the whole of the draw, so neither a
        // `&self` method (`quicksave_zone`, `stats_hud_on`) nor a `&mut self` one
        // (`cooldown_frac`, which expires entries lazily) borrow-checks inside it.
        // Per-plate cooldown progress; 1.0 = ready.
        let cooldowns: Vec<f64> = (0..self.quick_rects.len())
            .map(|i| {
                let slot = self.quickbar.get(i).map(|s| s.slot);
                match slot {
                    Some(sl) => self.cooldown_frac(sl).unwrap_or(1.0),
                    None => 1.0,
                }
            })
            .collect();
        let stats_on = self.stats_hud_on();
        let quicksave = self.quicksave_zone();
        // 설정 → 패드 버튼 설정 (2026-10-09) — the rows, and the prompt's expiry, resolved here
        // for the same `buffer_mut` reason.
        let padmap = self.padmap_view();
        let (option_w, option_h) = crate::logical();
        let options_frame = crate::game_options::render(&self.last_frame, option_w as i32, option_h as i32);
        // ★ 143차: the EXIT tab, the exit box and the wake's veil — resolved here for the
        // same reason (`buffer_mut` pins `self` for the whole draw) — and the message-box
        // flag the worker's wake waits on (`crate::notice_up`).
        let exit_tab = self.exit_zone();
        let exit_ask = self.exit_ask.map(|(sel, _)| sel);
        let veil = self.veil_now();
        // The side notes still on screen, at their strength; the faded ones are dropped
        // HERE (the draw below cannot touch `self`).
        self.side_notes.retain(|(_, at)| overlay::side_note_alpha(at.elapsed().as_millis()).is_some());
        let side_notes: Vec<(std::borrow::Cow<'static, str>, u8)> = self
            .side_notes
            .iter()
            .filter_map(|(text, at)| overlay::side_note_alpha(at.elapsed().as_millis()).map(|a| (text.clone(), a)))
            .collect();
        // ★ A side note counts as "up" for the wake too: the top floor's clear line has to
        // be read before the screen whitens, exactly as its box had to be closed.
        crate::set_notice_up(self.notice_up() || !side_notes.is_empty());

        // ── game blit setup, while `self` is still freely usable ────────────────────
        let g = self.layout.game;
        let (lw, lh) = crate::logical();
        // Everything the blit's index arithmetic needs, established HERE rather than
        // inherited from `overlay::layout`:
        //
        //   * `last_frame.len() == lw * lh` and `sy <= lh - 1`  ->  `sy * lw + lw <= len`;
        //   * `g` lies wholly inside the window and `dy <= g.h - 1`
        //       ->  `(g.y + dy) * win_w + g.x + g.w <= (win_h - 1) * win_w + win_w`
        //         = `win_w * win_h` = `buf.len()` (checked just below).
        //
        // The two containment tests and the `lw > 0` test are NEW. They only ADD to the
        // old guard (`len` match + non-empty rect) — nothing that used to blit stops
        // blitting, because every `overlay::layout` branch already satisfies them: the
        // immersive branch hands `game` the whole window, and the other two `contain()` a
        // sub-rect of an on-screen area. They are here so the no-panic argument is local
        // to this function instead of living in another module that is free to change.
        let blit = self.last_frame.len() == (lw * lh) as usize
            && lw > 0
            && lh > 0
            && g.w > 0
            && g.h > 0
            && g.x.saturating_add(g.w) <= win_w
            && g.y.saturating_add(g.h) <= win_h;
        // ★ Refreshed before the surface is borrowed, for the reason the note above gives:
        // `buffer_mut` pins `self`, so this `&mut self` cache update would not borrow-check
        // once the draw has started. The length is part of the test as belt-and-braces —
        // with it, `zip` in the gather loop cannot even in principle read past the map.
        if blit && (self.sx_lut_key != (g.w, lw) || self.sx_lut.len() != g.w as usize) {
            self.sx_lut = sx_lut(g.w, lw);
            self.sx_lut_key = (g.w, lw);
        }

        // ★ 70차: drive the held ▲/▼ auto-repeat from the frame clock — the native
        // quantity window has no ±10 buttons, so a held arrow must walk, exactly like
        // the game's own key repeat. Before the surface borrow pins `self`.
        self.pump_qty_hold();
        // ★ 150차: the 상세보기 body patch for THIS frame — composed here for the same reason
        // (`compose_note` is `&mut self`), drawn below with the game blit's own mapping. What
        // was drawn is what the next key or touch answers to (`note_up`).
        let note_at = if blit { self.compose_note() } else { None };
        self.note_up = note_at.is_some();
        // ★ 167차: the AVATAR page patch, for the same reason and with the same mapping.
        let avatar_at = if blit { self.compose_avatar() } else { None };
        self.avatar_up = avatar_at.is_some();
        // ★ 168차: the seven-row MENU list, for the same reason and with the same mapping.
        let menu_at = if blit { self.compose_menu7() } else { None };
        self.menu_up = menu_at.is_some();

        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let mut buf = match surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("buffer_mut failed: {e}");
                return;
            }
        };
        if buf.len() != (win_w * win_h) as usize {
            return;
        }

        // ★ Clear only the pixels the game blit will NOT overwrite (letterbox + control
        // margins). The full-window `buf.fill(0xff00_0000)` that stood here wrote every
        // pixel of the window — 10 MB of stores per frame at 1080x2340 — and the blit then
        // immediately overwrote the whole `layout.game` rect. In the immersive HUD that
        // rect IS the window, so the fill was pure waste on the layout the game is
        // actually played in.
        //
        // The fallback is exact, not approximate: when `blit` is false NOTHING covers the
        // game rect this frame, so the old full-window fill has to stay or the rect would
        // show whatever the previous frame left in the recycled surface buffer.
        if blit {
            let (w, gx, gy, gw, gh) = (win_w as usize, g.x as usize, g.y as usize, g.w as usize, g.h as usize);
            // `gy * w` and `(gy + gh) * w` are both `<= win_h * w == buf.len()` because
            // `g.y + g.h <= win_h`, so neither slice can be out of range.
            buf[..gy * w].fill(0xff00_0000);
            // Skipped when the game spans the full width (the immersive case), where this
            // would be `win_h` iterations of two empty fills for nothing.
            if gx > 0 || gx + gw < w {
                for row in gy..gy + gh {
                    let base = row * w;
                    buf[base..base + gx].fill(0xff00_0000);
                    buf[base + gx + gw..base + w].fill(0xff00_0000);
                }
            }
            buf[(gy + gh) * w..].fill(0xff00_0000);
        } else {
            buf.fill(0xff00_0000);
        }

        // Blit the game into its layout rect (nearest-neighbour, aspect-correct).
        if blit {
            let (lut, last) = (&self.sx_lut, options_frame.as_ref().unwrap_or(&self.last_frame));
            let gw = g.w as usize;
            // ★ Vertical runs are COPIED, not re-gathered. The guest framebuffer is ~240
            // logical rows and the device window is 2340 device rows, so about 90 % of
            // destination rows resolve to the same source row as the row above them — the
            // old loop ran the full per-pixel gather ~9 times over each source row to
            // produce nine identical destination rows.
            let mut prev: Option<(u32, usize)> = None;
            for dy in 0..g.h {
                // `sy < lh`: `dy <= g.h - 1`, so `dy * lh < g.h * lh`.
                let sy = (dy * lh) / g.h;
                let dst_row = ((g.y + dy) * win_w + g.x) as usize;
                match prev {
                    // Same source row as the last row we gathered: memcpy that finished
                    // destination row instead of re-reading the source. `from` is a row
                    // STRICTLY ABOVE `dst_row` (both start at `g.x` and `dy` only grows),
                    // and `from + gw <= dst_row` because `g.x + g.w <= win_w` — so the two
                    // ranges are disjoint and this is a plain memcpy, not a memmove.
                    Some((psy, from)) if psy == sy => buf.copy_within(from..from + gw, dst_row),
                    _ => {
                        let src_row = (sy * lw) as usize;
                        let src = &last[src_row..src_row + lw as usize];
                        // `zip` (not `for dx in 0..gw`) is what makes this unable to panic
                        // on a stale map: a wrong-length LUT would draw fewer columns
                        // rather than index out of range. `src[sx]` is in range because
                        // every LUT entry is `< src_w`, and `src_w` is `lw` by the key.
                        for (d, &sx) in buf[dst_row..dst_row + gw].iter_mut().zip(lut.iter()) {
                            *d = src[sx as usize];
                        }
                        prev = Some((sy, dst_row));
                    }
                }
            }
        }

        // Joystick view (thumb clamped to the base radius for rendering).
        let joy_view = self.joy.as_ref().and_then(|j| {
            let zone = self.layout.joystick.as_ref()?;
            let r = overlay::joy_radius(zone, win_w, win_h);
            let (dx, dy) = (j.tx - j.cx, j.ty - j.cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let thumb = if dist > r && dist > 0.0 {
                (j.cx + dx / dist * r, j.cy + dy / dist * r)
            } else {
                (j.tx, j.ty)
            };
            Some(overlay::JoyView { center: (j.cx, j.cy), thumb })
        });
        // FIELD only: cover the game's own field HUD (bottom strip + top-left skill
        // box) so our custom HUD replaces it, then draw our HUD on top. When one of
        // the game's own windows is open, all of that must stand down — the mask
        // would black out part of the window, and the gauges/minimap panel would
        // cover it (that, plus a zeroed Stats, is the reported "HP/MP read 0 in the
        // menu" bug). The buttons alone stay, now as menu-navigation keys.
        let field = self.mode == overlay::Mode::Field;
        if field {
            overlay::mask_game_hud(&mut buf, win_w, win_h, self.layout.game);
        }
        // Title / slot screens are 240-px art that the game mirrors outwards to fill
        // a wide framebuffer; letterbox it so it reads as centred, not cropped.
        if self.shell {
            overlay::letterbox_shell(&mut buf, win_w, win_h, self.layout.game);
            // ★ 130차: the save-slot window's ▲/▼ (the `n~m / 12` readout was removed at the
            //   user's request, 130차 §1-B), on the two slot lists only, from what the
            //   WORKER published (never from our own taps).
            if crate::savevault::on_slot_list(self.shell_screen)
                && let Some((first, total)) = self.slot_window
            {
                overlay::draw_slot_window(&mut buf, win_w, win_h, self.layout.game, first, total);
            }
        }
        // ★ 136차: 「정렬」 on the money strip of the inventory / 싱글가방 / 창고 panels,
        //   from what the WORKER published. Our own dialogs draw later and cover it.
        if let Some(screen) = self.sort_screen
            && self.editor.is_none()
        {
            overlay::draw_sort_button(&mut buf, win_w, win_h, self.layout.game, screen);
        }
        // ★ The centred full-screen map copy is GONE (player request 2026-08-04:
        // "가운데 확대해서 표시하는 기능은 제거하고"). The minimap is now a panel you
        // travel with, shown or hidden by tapping it — blowing it up over the middle of
        // the screen was the old answer to "the guest draws its map window tiny", and
        // the toggle replaced the need for it.
        //
        // `map_open` is still tracked — it is part of what selects the overlay control
        // set — but nothing draws over the guest's own map window any more.
        // ★ 설정 → 패드 버튼 설정 (2026-10-09): modal over the game picture, like the editor —
        // the FIELD HUD as it is played, every pad function lit on it (`overlay::draw_padmap`),
        // and nothing of ours over it but a message box.
        if let Some(v) = padmap.as_ref() {
            let look = overlay::PadMapLook {
                badges: &v.badges,
                sel: v.sel,
                probe: v.probe,
                pulse: v.pulse,
                title: Self::PADMAP_TITLE,
                lines: [&v.lines[0], &v.lines[1]],
                can_reset: v.can_reset,
                dirty: v.dirty,
            };
            overlay::draw_padmap(&mut buf, win_w, win_h, &v.scene, &v.slots, self.secondary, &look);
            if let Some(text) = notice_text_of(&self.orb_notice, &self.notices) {
                overlay::draw_native_notice(&mut buf, win_w, win_h, text);
            }
            if let Err(e) = buf.present() {
                log::warn!("present failed: {e}");
            }
            // The countdown and the breath are drawn from the clock, so a prompt is owed
            // frames until it is answered or runs out — the game's own frames may be paused
            // under us.
            if v.pulse.is_some() {
                self.request_redraw();
            }
            return;
        }
        // ★ The editor replaces the whole control layer: it shows the set for the
        // mode BEING EDITED, which is usually not the mode the game is in.
        if let Some(ed) = self.editor.as_ref() {
            let items = overlay::edit_items(win_w, win_h);
            overlay::draw_layout_editor(&mut buf, win_w, win_h, EDIT_MODE, ed.sel, &items);
            // ★★ The notice, HERE too (2026-09-10, 140차). This arm RETURNS, so the shared
            // notice pass at the end of this function is unreachable from the editor —
            // and HIDE / SHOW raise notices that are the whole answer to a tap
            // ("퀵슬롯 하나는 남겨야 합니다"). Without this, refusing looks exactly like
            // a broken button, which is the thing the panel's own rule forbids.
            // (Field borrows, not `notice_text()`: `buf` holds `self.surface` mutably.)
            if let Some(text) = notice_text_of(&self.orb_notice, &self.notices) {
                overlay::draw_native_notice(&mut buf, win_w, win_h, text);
            }
            if let Err(e) = buf.present() {
                log::warn!("present failed: {e}");
            }
            return;
        }
        // ★ 150차 — the 상세보기 BODY, recomposed (`compose_note`): the game's own rows it keeps,
        // the host lines after them (the third option, the set section), a HELP-style bar
        // when it overflows — placed over the game frame through the SAME logical→window
        // mapping as the frame itself, so it reads as the game's own window. It is part of
        // the game picture: under every control and dialog of ours.
        if let Some((ox, oy)) = note_at {
            let (bx, by) = ((ox + crate::itemnote::BODY_DX) as u32, (oy + crate::itemnote::BODY_DY) as u32);
            let (bw, bh) = (crate::itemnote::BODY_W as u32, crate::itemnote::BODY_H as u32);
            overlay::blit_logical_rect(&mut buf, win_w, win_h, g, &self.sx_lut, (lw, lh), &self.note_patch, bw, (bx, by), (bx, by, bw, bh));
        }
        // ★ 167차 — the AVATAR page over the game's EQUIPMENT panel (`compose_avatar`): the whole
        // panel, through the same logical→window mapping as the frame, so it reads as one of the
        // game's own pages. Part of the game picture: under every control and dialog of ours.
        if let Some((ox, oy)) = avatar_at {
            let (w, h) = (crate::avatar_page::W as u32, crate::avatar_page::H as u32);
            let (ox, oy) = (ox as u32, oy as u32);
            overlay::blit_logical_rect(&mut buf, win_w, win_h, g, &self.sx_lut, (lw, lh), &self.avatar_patch, w, (ox, oy), (ox, oy, w, h));
        }
        // ★ 168차 — the MENU list's seven rows (`compose_menu7`), the same way.
        if let Some((ox, oy)) = menu_at {
            let (w, h) = (crate::avatar_page::W as u32, crate::avatar_page::H as u32);
            let (ox, oy) = (ox as u32, oy as u32);
            overlay::blit_logical_rect(&mut buf, win_w, win_h, g, &self.sx_lut, (lw, lh), &self.menu_patch, w, (ox, oy), (ox, oy, w, h));
        }
        // ★ 145차: the 하늘성 banner — the floor and the monsters left, top centre, in the
        // game's damage digits with only the colour changed (user 2026-09-30; 147차: three
        // inks, 0.8x, 7 % down). UNDER the controls (it is a readout of the world, like the
        // damage numbers it is drawn in). Not with a game window open: the game's 176x160
        // menu panel begins 40 logical px down and the banner hangs from ~17 to ~45 (145차:
        // 24 to 59) — still across its tab row. `None` outside a run.
        if self.mode != overlay::Mode::Menu
            && !self.shell
            && let Some(hud) = self.tower_hud.as_ref()
        {
            overlay::draw_tower_hud(&mut buf, win_w, win_h, self.layout.game, lh, hud);
        }
        // ★ 141차: the EXP bar sits at the bottom centre (hero5's place for it) and goes
        // UNDER the controls — it is not an obstacle to them, so a control the player
        // parks over the bottom edge covers it rather than the other way round.
        if stats_on {
            overlay::draw_exp_bar(&mut buf, win_w, win_h, self.layout.game, &self.stats);
        }
        overlay::draw_controls(&mut buf, win_w, win_h, &self.layout, &self.pressed, joy_view);
        // Gauges first, skill plates ON TOP: where the arc meets the bottom
        // HUD strip the EXP bar would otherwise clip the last slot.
        //
        // ★ The gauges are drawn in MENU mode too since 2026-08-05 (player request:
        // "메뉴 버튼 열렸을때 좌측상단 UI는 다 보이게" — LV, HP, MP and EXP all stay
        // visible with a game window open). `stats_hud_on` gates out the shell and
        // the boot default. The live cash-buff readout rides below them, MENU only:
        // on the field those pixels belong to the game world.
        if stats_on {
            let blink = overlay::badge_blink_on(self.started.elapsed().as_millis());
            overlay::draw_stats(&mut buf, win_w, win_h, self.layout.game, &self.stats, blink, self.portrait.as_ref());
            // ★ The field line (the set bonus, 142차) comes FIRST and is drawn in EVERY
            // mode; the buff clocks stay MENU-only (2026-08-05's rule, unchanged). With
            // no set live it is `None`, so on an ordinary field this is the same nothing
            // it always was. (Until 145차 the 하늘성 floor readout rode here too — it is
            // the top-centre banner now, `draw_tower_hud` below.)
            let mut lines: Vec<String> = self.field_line.iter().cloned().collect();
            if self.mode == overlay::Mode::Menu {
                lines.extend(self.buff_lines.iter().cloned());
            }
            if !lines.is_empty() {
                overlay::draw_buff_lines(&mut buf, win_w, win_h, self.layout.game, &lines);
            }
        }
        // ★ The plates are drawn in MENU mode too since 2026-08-03 (the minimap is
        // not — it belongs to the field HUD the game replaces with its window).
        // `quick_rects` is empty during a cutscene and off-field in the
        // non-immersive layouts, so this is one call for every mode that has them.
        overlay::draw_quickbar(
            &mut buf,
            win_w,
            win_h,
            &self.quick_rects,
            &self.quickbar,
            self.quick_press.map(|(_, i)| i),
            &cooldowns,
        );
        // The game is asking which plate to register into: ring them all.
        if self.picking_slot {
            overlay::draw_pick_highlight(&mut buf, win_w, win_h, &self.quick_rects);
        }
        // ★ The PAGE button is FIELD or PICKING. Off-field it is normally hidden (a page
        // flip re-binds guest quickbar slots, which is not a thing to do with a game
        // window open) — but the 퀵바등록 flow is the one exception the player asked for
        // (2026-08-04): "등록할때도 페이지 전환 버튼 있어야해. 페이지 전환해서 등록도
        // 가능해야해". While the plates are lit the flip is the point, so it is allowed
        // and taken from the CURRENT mode's seats, the same ones the lit plates came from.
        //
        // No page DIGIT any more (player request 2026-08-04: "퀵바 페이지 번호 제거").
        if field || self.picking_slot {
            if let Some(r) = placement::rect_of(placement::CtrlId::QuickPage, self.mode, win_w, win_h) {
                overlay::draw_mask_button_spanned(
                    &mut buf,
                    win_w,
                    win_h,
                    r,
                    crate::ui_icons::Icon::QuickPage,
                    false,
                    overlay::QUICKPAGE_ICON_SPAN,
                );
            }
        }
        // SAVE NOW — see `quicksave_zone` for why this is the only gate it gets.
        // ★ 141차: a flat word on the top row (`overlay::draw_top_label`), the way the
        // reference screenshot lays its buttons along the top edge.
        if let Some(r) = quicksave {
            overlay::draw_top_label(&mut buf, win_w, win_h, r, "SAVE", false);
        }
        if field {
            // The minimap toggle stays FIELD only: hiding the minimap is meaningless with
            // one of the game's own windows covering it. In a no-save AREA (boss room)
            // the whole minimap UI disappears — panel AND toggle (2026-08-07: "보스방일
            // 때는 미니맵도 꺼지고").
            if !self.can_save {
                // nothing: no panel, no toggle
            } else {
                if self.minimap_on {
                    overlay::draw_minimap(&mut buf, win_w, win_h, self.layout.game, self.minimap.as_ref(), self.minimap_marker, &self.minimap_mobs);
                }
                // ★ 141차: the MAP word is on the top row beside SAVE whether the view is
                // up or not — it TOGGLES now (it used to appear only while the view was
                // hidden). The view itself still hides on a tap, as before.
                if let Some(r) = placement::rect_of(placement::CtrlId::MinimapToggle, overlay::Mode::Field, win_w, win_h) {
                    overlay::draw_top_label(&mut buf, win_w, win_h, r, "MAP", !self.minimap_on);
                }
            }
            // ★ 143차: the EXIT tab — the tower's way out (user 2026-09-30: *"map버튼 왼쪽에
            // exit 버튼을 둬서"*). SAVE and MAP are hidden in the tower (2026-09-08), so it
            // stands alone in their row, one gap left of MAP's seat (`overlay::exit_tab_rect`).
            if let Some(r) = exit_tab {
                overlay::draw_top_label(&mut buf, win_w, win_h, r, "EXIT", false);
            }
            if crate::immersive_active() && landscape {
                // Key-'7' secondary-weapon button (immersive only — the other
                // layouts already have a real keypad). ★ Take the rect from the
                // RESOLVER, not from `overlay::secondary_button`: the hit-test comes
                // from `layout.buttons`, which is resolved, so drawing from the
                // built-in would paint the face somewhere the touch is not.
                if let Some(r) = placement::rect_of(placement::CtrlId::Secondary, overlay::Mode::Field, win_w, win_h) {
                    overlay::draw_secondary(&mut buf, win_w, win_h, r, self.secondary);
                }
            }
        }
        // ★ 59차: 게임패드 HUD — one pass over every control that a pad button fires,
        // stamping the POSITION of that button on it (`overlay::draw_pad_badges`). Last,
        // so a badge is never painted under a face that is drawn twice (the secondary
        // weapon), and before the settings modal, whose dim pass should cover it like it
        // covers everything else.
        //
        // The three host-owned controls have no guest key to look up, so they are badged
        // by ACTION — and each is badged only where it is actually drawn above, or the
        // badge would be a promise about a control that is not on screen.
        if crate::pad_hud_active() && self.editor.is_none() {
            overlay::draw_pad_badges(&mut buf, win_w, win_h, &self.layout, &self.quick_rects, &self.quickbar);
            if field || self.picking_slot {
                if let Some(r) = placement::rect_of(placement::CtrlId::QuickPage, self.mode, win_w, win_h) {
                    overlay::draw_pad_badge_action(&mut buf, win_w, win_h, r, crate::gamepad::PadAction::QuickPage);
                }
            }
            if let Some(r) = quicksave {
                overlay::draw_pad_badge_action(&mut buf, win_w, win_h, r, crate::gamepad::PadAction::QuickSave);
            }
            // The minimap TOGGLE is a host control, but the key that repaints the map is
            // the guest's `*` — so the badge is the one the pad sends for `*`, which is
            // also what the panel itself responds to. Only while the toggle is the thing
            // on screen: with the panel up the map is already open.
            if field && self.can_save {
                if let Some(r) = placement::rect_of(placement::CtrlId::MinimapToggle, overlay::Mode::Field, win_w, win_h) {
                    overlay::draw_pad_badge_action(&mut buf, win_w, win_h, r, crate::gamepad::PadAction::Key(KeyCode::STAR));
                }
            }
        }
        // ★ 71차-I: the live 제련 odds, in the game's own chrome, under the smith's own
        // window. Above the HUD so the game's art cannot swallow it; below every host
        // dialog — and NOT DRAWN AT ALL while one is up. None of those can co-occur with
        // the smith's stone spinner (the pickers belong to the inventory, the warehouse
        // and the cash shop), so a bar peeking out from behind one could only ever mean
        // the worker's gate had gone wrong, and half a percentage is worse than none.
        if let Some(r) = self.refine.as_ref()
            && self.editor.is_none()
            && !self.cash_qty
            && self.orb_pick.is_none()
            && self.wh_qty.is_none()
            && self.orb_notice.is_none()
            && self.notices.is_empty()
        {
            overlay::draw_refine_odds(&mut buf, win_w, win_h, self.layout.game, r);
        }
        // The cash quantity picker sits above the settings modal in the stack but
        // below its dim pass — it belongs to the game screen, not to our menus.
        if self.cash_qty {
            // ★ The game's own wording AND the game's own window, pixel-measured from
            // the player's screenshot (2026-08-25 "임의디자인이 아닌 기존의 클라이언트
            // 디자인 그대로"). The dark strip carries the ceiling — the cash rows are
            // free, so the price the 상점 version prints there would read `0`.
            let info = format!("최대 {}", crate::CASH_QTY_MAX);
            overlay::draw_qty_dialog(
                &mut buf,
                win_w,
                win_h,
                "개수를 설정하세요",
                crate::cash_qty(),
                crate::CASH_QTY_MAX,
                Some(&info),
            );
        } else {
            if let Some(pick) = self.orb_pick.as_deref() {
                // Drawn in the same slot as the quantity pickers — over the game screen, under
                // the settings modal's dim pass. The WHOLE list goes in: the viewport scrolls
                // by pixels now, so the dialog itself decides what is on screen.
                overlay::draw_orb_dialog(
                    &mut buf,
                    win_w,
                    win_h,
                    &pick.title,
                    &pick.subtitle,
                    &pick.rows,
                    self.orb_focused.then_some(self.orb_sel),
                    self.orb_scroll,
                    pick.ok_label,
                );
                // ★ The drop box is part of the same dialog — drawn after it, over the row it
                // is anchored to.
                if let Some(drop) = self.orb_drop.as_deref() {
                    overlay::draw_orb_dropbox(&mut buf, win_w, win_h, self.orb_scroll, drop.anchor, &drop.rows, self.orb_drop_sel);
                }
                // ★★ …and the ARROWS over everything (user, 2026-09-15: *"가방창이 좌측
                // 화살표위로 올라와있는데 화살표가 더 위여야돼"*). They are the dialog's own
                // scroll control now, so they cannot be the one thing the dialog covers.
                overlay::draw_direction_controls(&mut buf, win_w, win_h, &self.layout, &self.pressed);
                // ★★★ 117차-D: the GAME's own message box, through the window (user:
                //   *"기존 팝업이 그대로 소비가방 위에 뜨게"*). Drawn LAST in this block so
                //   nothing of ours covers it; only its measured frame is re-blitted, so the
                //   bag page around it stays under the window.
                if self.orb_reveal {
                    Self::reveal_game_dialog(&mut buf, win_w, win_h, self.layout.game, &self.sx_lut, &self.last_frame);
                }
            }
            if let Some((max, min, _, _)) = self.wh_qty {
                // The warehouse's own 올리기/내리기 count. The strip says what the bounds
                // are, because unlike the cash shop the ceiling is not a fixed 99 — it is
                // whatever the stack holds — and 69차-B gave partial deposits a FLOOR.
                // ★ 96차: drawn AFTER the list, not instead of it — a box window's 올리기 /
                // 내리기 asks its count over the list it came from (`bagbox_host`), and the
                // engine's own ask never has a list up, so the order costs nothing there.
                let info = if min > 1 {
                    format!("최소 {min} 최대 {max}")
                } else {
                    format!("최대 {max}")
                };
                overlay::draw_qty_dialog(&mut buf, win_w, win_h, "개수를 설정하세요", self.wh_qty_value, max, Some(&info));
            }
        }
        if self.show_settings {
            overlay::draw_settings(
                &mut buf,
                win_w,
                win_h,
                &self.settings_items,
                landscape,
                self.joystick,
                config::read_immersive(&self.data_dir),
                config::read_base_h(&self.data_dir),
                config::read_heat_saver(&self.data_dir),
            );
        }
        // ★★★ The game-style message box — a list dialog's own notice (`orb_notice`) or the
        // front of the host queue (`notices`: the quick-save result, settings toggles, box
        // refusals …) — drawn LAST, after the settings modal too, so nothing of ours paints
        // over it: it is the game answering the player's last press, and it owns every
        // input until OK / CLR / its band. ★ 57차 learned this the hard way with the old
        // toast: raised BY a settings row, it sat under that modal's dim pass and was
        // silently eaten (HEAT SAVER's "재시작 후 적용" was the first one anybody could miss).
        // ★ The side notes (2026-09-30) — over the HUD and the settings modal, under every
        // box: they are the answers that do NOT stop the game.
        if !side_notes.is_empty() {
            let notes: Vec<(&str, u8)> = side_notes.iter().map(|(t, a)| (t.as_ref(), *a)).collect();
            overlay::draw_side_notes(&mut buf, win_w, win_h, self.layout.game, &notes);
        }
        // ★ 143차: 「하늘성에서 나가시겠습니까?」 — the game's own yes/no box, replicated.
        // Under a message box (which answers first), over everything else of ours.
        if let Some(sel) = exit_ask {
            overlay::draw_native_confirm(&mut buf, win_w, win_h, "하늘성에서 나가시겠습니까?", sel);
        }
        if let Some(text) = notice_text_of(&self.orb_notice, &self.notices) {
            overlay::draw_native_notice(&mut buf, win_w, win_h, text);
        }

        // ── 하늘성 입장 연출 ────────────────────────────────────────────────
        // Last of all, so it covers the game, the HUD and any toast — this is the
        // screen going white, not a widget.
        if self.tower_flash_left > 0 {
            self.tower_flash_left -= 1;
            overlay::tower_flash(&mut buf, win_w, win_h, (self.tower_flash_left / 2) % 2 == 1);
        }
        // ★★ 143차: THE WAKE's veil — after the flash, after everything: it is the screen
        // itself going white (and coming back), the way 「의식이 돌아오는듯한」 reads.
        if let Some((a, _)) = veil {
            overlay::draw_wake_veil(&mut buf, a);
        }

        if let Err(e) = buf.present() {
            log::warn!("present failed: {e}");
        }
        // ⛔ AFTER present, and only while frames are owed: the emulator may be between
        // frames of its own, and without this the flash would stop wherever the last
        // unrelated redraw happened to leave it. The veil's fades are owed frames too.
        // A side note fades on the UI's own clock, so it is owed frames too — and one more
        // after the last, which drops it and clears the wake's flag.
        if self.tower_flash_left > 0 || veil.is_some_and(|(_, moving)| moving) || !side_notes.is_empty() {
            self.request_redraw();
        }
        // A veil that has finished fading OUT is gone — inputs come back.
        if let Some((a, moving)) = veil
            && !moving
            && a == 0
        {
            self.veil = None;
        }
    }

    fn update_touch(&mut self, id: u64, new_key: Option<KeyCode>) {
        let old_key = self.touches.get(&id).copied();
        // ★ 150차 review: still resting on the key a host picker took below — not a press
        // (see `qty_slid`). Anything else under the finger ends that record.
        match self.qty_slid.get(&id).copied() {
            Some(k) if new_key == Some(k) => return,
            Some(_) => {
                self.qty_slid.remove(&id);
            }
            None => {}
        }
        // ⛔⛔ SAME KEY AS LAST TIME IS NOT A PRESS, and this test has to come FIRST
        // (2026-08-26). Android delivers `TouchPhase::Moved` continuously for a finger
        // that is merely resting, and `Moved` routes a `slide_touches` finger straight
        // back in here with the key it is already holding. The quantity test used to sit
        // ABOVE this line, so the very OK that opened a picker answered it on the next
        // jitter — at the ceiling, since a fresh ask starts there:
        //   "올리기를 선택하면 잠깐 반짝 수량창이 떠있다가 내가 ok를 한번 더 누르지도
        //    않았는데 바로 최대 수량으로 올라가버려".
        // A picker is opened BY a key, so it must never be answerable by that same key
        // still being down.
        if old_key == new_key {
            return;
        }
        // ★ A quantity picker owns the direction keys (2026-08-25). This is the choke
        // point every HELD control goes through — the D-pad, a finger sliding between
        // its arms, and the joystick's latched direction — so catching it here covers
        // them all with one test. A fresh press dials the number; a slide or a release
        // is simply swallowed, because a picker is not something you hold down.
        // ⛔ Only a PRESS is intercepted. A release must fall through to the normal
        // path, which sends a keyup only for a key it actually pressed — swallowing
        // releases here would strand a key that was already down when the picker opened.
        if let Some(k) = new_key
            && self.qty_key(k)
        {
            self.touches.remove(&id);
            self.qty_slid.insert(id, k);
            // The guest saw `old_key` go DOWN; it has to see it come up, or the finger
            // that slid from a control onto the picker leaves a key stuck under the
            // dialog. Same release rule as the normal path below.
            let was_diverted = self.popup_confirm_touches.remove(&id);
            if let Some(ok) = old_key
                && !self.touches.values().any(|&v| v == ok)
                && !was_diverted
            {
                let _ = self.input_tx.send(InputEvent::Key { code: ok, down: false });
            }
            self.refresh_pressed();
            self.request_redraw();
            return;
        }
        match new_key {
            Some(k) => {
                self.touches.insert(id, k);
            }
            None => {
                self.touches.remove(&id);
            }
        }
        // ★ The UP is paired with what the DOWN actually DID, not with the popup state
        // now — see `popup_confirm_touches`. Cleared unconditionally, so a touch that
        // slides from OK onto another key does not leave a stale record behind.
        let was_diverted = self.popup_confirm_touches.remove(&id);
        if let Some(ok) = old_key
            && !self.touches.values().any(|&v| v == ok)
            // A diverted OK was never pressed as far as the guest is concerned, so it
            // must not be released.
            && !was_diverted
        {
            let _ = self.input_tx.send(InputEvent::Key { code: ok, down: false });
        }
        if let Some(nk) = new_key
            && self.touches.values().filter(|&&v| v == nk).count() == 1
        {
            // ★ With the game's rewritten context popup on screen, a CONFIRM belongs to
            // the popup, and only the WORKER can say what the highlighted row means:
            // our copy of it is up to ~80 ms old, and the row decides which guest
            // function runs. So OK becomes `PopupConfirm` and the worker re-reads the
            // row before doing anything — including passing a real OK through when the
            // row is the game's own 사용하기 / 포인트투자.
            //
            // Only the DOWN edge is diverted. The matching UP is dropped rather than
            // sent, because the worker synthesises the whole down+up pair itself for
            // the pass-through case; sending a bare keyup for a key the guest never saw
            // pressed is how you get a stuck key.
            if nk == KeyCode::OK && self.popup.is_some() {
                self.popup_confirm_touches.insert(id);
                let _ = self.input_tx.send(InputEvent::PopupConfirm);
            } else {
                // CLR while the plates are lit means "never mind": drop out of picking
                // AND let the key through, which closes the game's popup too. Leaving
                // the highlight on after the popup is gone would be a HUD that lies.
                if nk == KeyCode::CLEAR && self.picking_slot {
                    let _ = self.input_tx.send(InputEvent::PopupCancelPick);
                }
                let _ = self.input_tx.send(InputEvent::Key { code: nk, down: true });
            }
        }
        self.refresh_pressed();
        self.request_redraw();
    }

    /// Which input source was used last (59차). Under the default AUTO preference this
    /// is what decides whether the HUD shows on-screen walk controls or pad badges, so
    /// it relayouts the moment the answer changes — and also the first time a pad is
    /// seen at all, because that is when the PAD HUD settings row appears.
    ///
    /// Called from the INPUT path only. Calling it from the drawing path would make the
    /// HUD depend on what was last drawn, which is a loop.
    /// Returns true if the HUD changed as a result — the caller may then want to
    /// SWALLOW the input that caused the change (see the touch handler).
    fn note_input(&mut self, pad: bool) -> bool {
        let (was_active, was_seen) = (crate::pad_hud_active(), crate::pad_seen());
        crate::note_input_source(pad);
        let flipped = crate::pad_hud_active() != was_active;
        if flipped || crate::pad_seen() != was_seen {
            self.relayout();
            self.request_redraw();
        }
        flipped
    }

    /// One gamepad/keyboard control edge (58차). The DOWN mirrors `update_touch`'s
    /// split exactly — popup diversion for OK, and while the plates are lit (60차)
    /// a quickbar button binds its own plate, CLR cancels, everything else is
    /// swallowed — because as far as the guest is concerned a pad button IS a
    /// phone key, and two entry points with different popup rules would be two
    /// OK behaviours to debug.
    fn gamepad_action(&mut self, action: crate::gamepad::PadAction, down: bool, repeat: bool) {
        use crate::gamepad::PadAction;
        match action {
            PadAction::Quick(plate) => {
                // ★★ 79차-B — a pad button names a PLATE now, not the digit that used to
                // stand in for one. Everything below mirrors the touch plate exactly:
                //
                //  * while the plates are LIT, it means "register into MY plate" (60차's
                //    rule, now expressed directly instead of being recovered from a digit);
                //  * on the field it goes to the worker, which is the only side that can
                //    say which guest slot this plate is in — or put it in one;
                //  * off the field it does NOTHING. A window's digits live on the
                //    pull-out keypad since the permanent block was removed (79차-B).
                //
                // No `pad_held` bookkeeping: this never becomes a guest keydown, so there
                // is no keyup to match and nothing can stick.
                //
                // ★ 81차 — EXCEPT off the field. A pad has no pull-out keypad to reach
                // for, so off the field the first five plates are the digits they sent
                // before 79차-B (`gamepad::menu_digit`: 1·3·9·0·#, a fixed table). "Off
                // the field" is `Menu` AND `Cutscene` on purpose: in a scene `#` is the
                // game's own SKIP, which L2 was before 79차-B too (what `#` does on a
                // dialogue CHOICE is still unmeasured — §55차-D — same as it was then).
                // The digit goes through the `Key` arm so it gets the same held/release
                // bookkeeping as any other key. The release is matched by what is HELD,
                // not by the mode at release time: a digit pressed in a window whose
                // close lands the player on the field must still be released, or the
                // key sticks (`pad_held` rule). Touch plates are untouched — 79차-B's
                // "a plate does nothing off the field" still holds for a finger.
                //
                // ⛔ Only the PRESS is gated on `picking_slot` (81차-B code review): the
                // release must go out whatever state the plates are in. A digit held
                // while the 퀵바등록 popup lights the plates would otherwise skip this
                // arm on its keyup, stay in `pad_held`, and the worker's 100 ms
                // `Keyrepeat` would keep pressing it — the same rule the `Key` arm's
                // own release path follows (it is not gated on `picking_slot` either).
                if let Some(key) = crate::gamepad::menu_digit(plate) {
                    let pressed_off_field = down && !self.picking_slot && self.mode != overlay::Mode::Field;
                    let releasing_held = !down && self.pad_held.contains_key(&key);
                    if pressed_off_field || releasing_held {
                        self.gamepad_action(PadAction::Key(key), down, repeat);
                        return;
                    }
                }
                if !down || repeat {
                    return;
                }
                // ★ 140차: a message box is up — a plate press under a modal would cast
                // a skill the player cannot see. (Its OK / CLR come through `Key` below.)
                // ★ 143차: …or the exit box, or the wake's veil (`modal_up`).
                if self.modal_up() {
                    return;
                }
                // ★★ A plate the player has SWITCHED OFF (2026-09-10) is not reachable
                // from a pad button either — and the pad is the one path that has to say
                // so in code, because it never consults `quick_rects`. Hiding a plate
                // makes its rect zero-sized, which is what stops a finger; a pad button
                // mapped to it would otherwise cast an invisible skill, and mid-
                // registration would BIND into a plate nobody can see.
                //
                // ⛔ BELOW the `menu_digit` bypass above, deliberately. That bypass sends
                // the game's own «1»«3»«9»`0``#` menu keys while a window is open, and
                // those are menu keys, not quickslots — 81차 restored them and hiding a
                // plate is not a request to take them away again.
                //
                // ⛔ ABOVE the `picking_slot` arm, equally deliberately: registration is
                // exactly the moment a hidden plate must not be chosen.
                //
                // The mapping stays IDENTITY (user decision 2026-09-10): `Quick(n)` is
                // always plate n, so `wie_gamepad.txt`'s `Q1..Q7` never change meaning
                // and a button cannot silently start casting a different skill. Hiding
                // plate 2 therefore leaves Y dead on the field (its badge is gone too,
                // since `padhud::badge_at` refuses a zero-sized rect) — remap it with one
                // `wie_gamepad.txt` line if you want that seat back.
                if crate::quickpage::plate_hidden(crate::quick_hidden(), plate) {
                    // "fail closed, but SAY SO": the three silent returns around this one
                    // are all states the player can see (no binding, cooling down, not on
                    // the field). "I hid it" is not, and this is the only line that could
                    // tell a report from a dead button.
                    log::info!("pad: plate {} is hidden — ignoring", plate + 1);
                    return;
                }
                if self.picking_slot {
                    let _ = self.input_tx.send(InputEvent::PopupBindSlot { slot: plate });
                    return;
                }
                if self.mode != overlay::Mode::Field {
                    return;
                }
                let Some(slot) = self.quickbar.iter().find(|s| s.slot == plate).copied() else {
                    return;
                };
                let is_skill = matches!(slot.kind, SlotKind::Skill);
                // Same cooldown swallow as a tap: a skill still recovering does not recast.
                if is_skill && self.cooldown_frac(plate).is_some() {
                    return;
                }
                let _ = self.input_tx.send(InputEvent::QuickActivate { plate, cast: is_skill });
            }
            PadAction::QuickPage => {
                // Same gate as the on-screen PAGE button: field, or while the game
                // is asking which plate to register into. A flip re-binds guest
                // quickbar slots, which is not a thing to do with a window open.
                if down && !repeat && !self.modal_up() && (self.mode == overlay::Mode::Field || self.picking_slot) {
                    let _ = self.input_tx.send(InputEvent::QuickPageNext);
                }
            }
            PadAction::QuickSave => {
                // Same shape as the SAVE row: the UI decides nothing. Whether a save is
                // possible depends on the guest's own "may I save here" byte, which only
                // the worker can read without a race — it answers with a `Notice` either
                // way, which is also the feedback that the button did anything at all.
                // Not while the plates are lit: mid-registration the pad only registers
                // or cancels (60차 — see the `picking_slot` block below). Nor under a
                // message box (140차): its answer would queue behind the one on screen.
                if down && !repeat && !self.picking_slot && !self.modal_up() {
                    let _ = self.input_tx.send(InputEvent::QuickSave);
                }
            }
            PadAction::Key(key) => {
                if down {
                    // The system's auto-repeat and any double-down are dropped: the
                    // WORKER replays every held key each 100 ms (`Keyrepeat`), and a
                    // second keydown would make the guest re-latch mid-hold.
                    if repeat || self.pad_held.contains_key(&key) {
                        return;
                    }
                    // ★ 60차: while the plates are lit (the 퀵바등록 flow is asking WHICH
                    // plate), the pad follows the same rules as a finger. CLR cancels
                    // (out of picking AND the key goes through to close the game's popup,
                    // same as the touch keypad). EVERY other KEY is swallowed whole: a
                    // key leaking into the guest mid-registration navigates or closes the
                    // popup under the highlight (player request 2026-08-18: "다른 버튼을
                    // 누르면 아예 동작을 안하면 돼").
                    //
                    // ★★ 79차-B: "register into my plate" moved OUT of here and into
                    // `PadAction::Quick`, which says the plate outright. It used to be
                    // recovered from the digit the button sent — a translation that only
                    // worked while a plate was a guest slot, and that could not name the
                    // plates past the fifth at all. A raw digit mapped by hand is now just
                    // a digit, and is swallowed here like any other key.
                    if self.picking_slot {
                        if key == KeyCode::CLEAR {
                            self.pad_held.insert(key, false);
                            let _ = self.input_tx.send(InputEvent::PopupCancelPick);
                            let _ = self.input_tx.send(InputEvent::Key { code: key, down: true });
                        }
                        return;
                    }
                    if self.qty_key(key) {
                        // Diverted: the guest never saw the press, so it must not see
                        // the release either (stuck-key rule).
                        self.pad_held.insert(key, true);
                        return;
                    }
                    let diverted = key == KeyCode::OK && self.popup.is_some();
                    self.pad_held.insert(key, diverted);
                    if diverted {
                        let _ = self.input_tx.send(InputEvent::PopupConfirm);
                    // ★★ 79차-B: two arms used to sit here, and both were the same
                    // translation — "this button sent a quickslot DIGIT, so work out
                    // which plate that means". One did it for the 퀵바등록 pick
                    // (2026-08-20), one for the skill-cast watch on the field. Both are
                    // gone: `PadAction::Quick` names the plate outright, which is exact,
                    // reaches all seven, and cannot be confused by what is seated in a
                    // guest slot at the time. A raw digit is now only ever a digit.
                    } else {
                        let _ = self.input_tx.send(InputEvent::Key { code: key, down: true });
                    }
                } else if let Some(diverted) = self.pad_held.remove(&key) {
                    // A diverted OK was never pressed as far as the guest is
                    // concerned, so it must not be released (stuck-key rule —
                    // see `popup_confirm_touches`).
                    if !diverted {
                        let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
                    }
                }
            }
        }
    }

    /// One stick/hat/trigger sample. Stick and hat are SUMMED before quantising:
    /// the hat is digital ±1 so it dominates whenever pressed, and nobody deflects
    /// both at once on purpose. `joy_dir` then applies the same engage/keep/turn
    /// hysteresis as the touch stick, and the transition is
    /// release-old-THEN-press-new because the guest latches its walk direction on
    /// keydown. Triggers become synthetic `BUTTON_L2/R2` edges through the SAME
    /// mapping table, so a `wie_gamepad.txt` remap of 104/105 applies to the analog
    /// triggers too.
    fn gamepad_axes(&mut self, x: f32, y: f32, hx: f32, hy: f32, lt: f32, rt: f32) {
        // ★ 패드 버튼 설정 (2026-10-09): the stick and the hat do nothing under the screen
        // (directions are not remappable — `padmap::capturable`), but an analog trigger's edge
        // is a button press there like any other, so the triggers still run.
        if self.padmap.is_some() {
            self.pad_trigger(true, lt);
            self.pad_trigger(false, rt);
            return;
        }
        let (dx, dy) = ((x + hx) as f64, (y + hy) as f64);
        let new = overlay::joy_dir(dx, dy, 1.0, self.pad_stick);
        if new != self.pad_stick {
            // A sample that changes the latched direction is real pad input — an idle
            // stick's noise is not (see the `GamepadAxes` arm).
            let _ = self.note_input(true);
            if let Some(old) = self.pad_stick
                && !self.pad_stick_diverted
            {
                let _ = self.input_tx.send(InputEvent::Key { code: old, down: false });
            }
            // ★★ 150차 review: the HAT is most pads' d-pad (`MainActivity`), so a new latch
            // goes to the host's modals and pickers FIRST, exactly like the pad's DPAD
            // buttons and the touch arrows (`qty_key`: the veil, the message box, the EXIT
            // confirm, the count pickers, the box window list, the 상세보기 note). It used to
            // go straight to the game — under a modal, and past a note that only the stick
            // could not scroll. A key taken there is never sent: no keydown, no keyup, no walk.
            self.pad_stick_diverted = false;
            if let Some(nk) = new {
                if self.qty_key(nk) {
                    self.pad_stick_diverted = true;
                } else {
                    let _ = self.input_tx.send(InputEvent::Key { code: nk, down: true });
                }
            }
            self.pad_stick = new;
            // ★ Same standing order as the touch stick (2026-09-20). A physical stick or
            // hat is a LEVEL too, and the guest's walk is the same keydown edge, so it
            // loses its walk to an attack or a skill in exactly the same way.
            let walk = if self.pad_stick_diverted { None } else { new };
            let _ = self.input_tx.send(InputEvent::Walk { dir: walk });
        }
        self.pad_trigger(true, lt);
        self.pad_trigger(false, rt);
    }

    /// One analog trigger's edge detection + dispatch (`left` = L2, else R2).
    fn pad_trigger(&mut self, left: bool, value: f32) {
        let held = if left { self.pad_triggers.0 } else { self.pad_triggers.1 };
        let now = crate::gamepad::trigger_held(held, value);
        if now == held {
            return;
        }
        if left {
            self.pad_triggers.0 = now;
        } else {
            self.pad_triggers.1 = now;
        }
        let code = if left {
            crate::gamepad::KEYCODE_BUTTON_L2
        } else {
            crate::gamepad::KEYCODE_BUTTON_R2
        };
        // ⛔ The edge is tracked above whether or not the pad screen is open, so a trigger
        // held across 저장 releases cleanly instead of re-pressing on the next sample.
        if self.padmap.is_some() {
            self.padmap_button(code, now, false);
            return;
        }
        if let Some(action) = crate::gamepad::button_action(code) {
            self.gamepad_action(action, now, false);
        }
    }

    /// Release everything the gamepad holds. Called from the same teardown sites
    /// as the touch cleanup (suspend, focus loss): once the activity loses focus
    /// Java stops dispatching, so a still-held key would auto-repeat at the guest
    /// forever — the exact stuck-key failure the touch paths already guard against.
    fn release_gamepad(&mut self) {
        for (key, diverted) in self.pad_held.drain() {
            if !diverted {
                let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
            }
        }
        if let Some(old) = self.pad_stick.take() {
            if !self.pad_stick_diverted {
                let _ = self.input_tx.send(InputEvent::Key { code: old, down: false });
            }
            let _ = self.input_tx.send(InputEvent::Walk { dir: None });
        }
        self.pad_stick_diverted = false;
        // Their keyups (if any) just went out through `pad_held` above.
        self.pad_triggers = (false, false);
    }

    /// Toggle the shared background/lock gate. Idempotent; logs only on change.
    /// While set, the emulator worker stops ticking and the MIDI synth emits
    /// silence, so no audio continues when the app is backgrounded or locked.
    fn set_paused(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::SeqCst) != paused {
            log::info!(
                "{}",
                if paused {
                    "app -> PAUSED (background/lock): audio+emu halted"
                } else {
                    "app -> RESUMED (foreground): audio+emu running"
                }
            );
            // ⛔ `network::note_backgrounded()` (2026-08-15: Android resets sockets in the
            // background, re-probe on resume) WAS HERE and is gone (99차): the in-app
            // server has no socket for Android to reset.
        }
    }

    fn handle_settings(&mut self, action: overlay::SettingsAction) {
        match action {
            overlay::SettingsAction::SetPortrait => {
                orientation::set(false);
                config::write_landscape(&self.data_dir, false);
                self.show_settings = false;
            }
            overlay::SettingsAction::SetLandscape => {
                orientation::set(true);
                config::write_landscape(&self.data_dir, true);
                self.show_settings = false;
            }
            overlay::SettingsAction::SetDpad => {
                self.set_control_style(false);
            }
            overlay::SettingsAction::SetJoystick => {
                self.set_control_style(true);
            }
            overlay::SettingsAction::ToggleImmersive => {
                let now = !config::read_immersive(&self.data_dir);
                config::write_immersive(&self.data_dir, now);
                log::info!("immersive pref -> {now} (applies at next start)");
            }
            overlay::SettingsAction::CycleBaseH => {
                let cur = config::read_base_h(&self.data_dir);
                let i = config::BASE_H_CHOICES.iter().position(|h| *h == cur).unwrap_or(1);
                let next = config::BASE_H_CHOICES[(i + 1) % config::BASE_H_CHOICES.len()];
                config::write_base_h(&self.data_dir, next);
                log::info!("immersive base height -> {next} (applies at next start)");
            }
            overlay::SettingsAction::EditLayout => self.enter_editor(),
            overlay::SettingsAction::ToggleTouchType => {
                // Applies at once — the layout is rebuilt below, which is what swaps
                // the whole-window stick zone (자유 스틱) for the field D-pad buttons
                // (십자키) or the seat-sized zone (고정 스틱). Cycles in the order the
                // player named the three (`StickMode::next`).
                let now = crate::stick_mode().next();
                // Anything the old type was holding is released first, exactly like
                // `set_control_style`: the zone changes under the finger, so a held
                // direction would have no touch left that could ever end it.
                self.joy_end();
                crate::set_stick_mode(now);
                config::write_stick_mode(&self.data_dir, now);
                self.relayout();
                self.request_redraw();
                log::info!("touch type -> {now:?} ({})", now.label());
            }
            overlay::SettingsAction::ToggleHeatSaver => {
                // Applies at next start: the render scale fixes the guest
                // framebuffer/window coordinate mapping and the audio synth cost at
                // boot, so flipping either live would remap every rect mid-frame.
                let now = !config::read_heat_saver(&self.data_dir);
                config::write_heat_saver(&self.data_dir, now);
                self.push_notice(if now {
                    "발열 절감: 켬 (재시작 후 적용)"
                } else {
                    "발열 절감: 끔 (재시작 후 적용)"
                });
                log::info!("heat saver -> {now} (applies at next start)");
            }
            overlay::SettingsAction::TogglePadHud => {
                // Applies AT ONCE, unlike HEAT SAVER: the layout is rebuilt from the
                // preference, so `relayout` is the whole "apply" step. The row stays
                // reachable for the next tap (the modal is not closed) because this is a
                // three-state cycle a player will step through to see what each does.
                let now = crate::pad_hud_pref().next();
                crate::set_pad_hud_pref(now);
                config::write_pad_hud(&self.data_dir, now);
                self.relayout();
                self.push_notice(match now {
                    crate::padhud::PadHud::Auto => "게임패드 HUD: 자동",
                    crate::padhud::PadHud::On => "게임패드 HUD: 항상 켬",
                    crate::padhud::PadHud::Off => "게임패드 HUD: 끔",
                });
                log::info!("pad hud -> {now:?}");
            }
            // ⛔ `ToggleNetMode` / `EditServer` / `EditAccount` WERE HERE and are gone
            // (99차, 2026-09-16 — 100 % offline). See `overlay::settings_items`.
            overlay::SettingsAction::CycleSfxLevel => {
                // Takes effect on the NEXT sound: the audio thread reads the level per
                // clip, so there is nothing to restart and nothing to apply here beyond
                // storing it. See `audio_mix::SFX_LEVELS` for why this row exists.
                let now = crate::audio_mix::sfx_level_next(crate::sfx_level());
                crate::set_sfx_level(now);
                config::write_sfx_level(&self.data_dir, now);
                self.push_notice("효과음 크기");
                self.request_redraw();
                log::info!("sfx level -> {now} ({})", crate::audio_mix::sfx_trim(now));
            }
            overlay::SettingsAction::QuickSave => {
                // The UI does not decide anything: whether a save is possible depends on
                // the guest's own "may I save here" byte, which only the worker can read
                // without a race. It answers with a `Notice` either way.
                let _ = self.input_tx.send(InputEvent::QuickSave);
                self.show_settings = false;
            }
            overlay::SettingsAction::Diagnostics => {
                // The Rust half of the text; Java prepends app/OS/device versions and
                // offers 복사. See `audio_sink::audio_status` for why this exists.
                let (lw, lh) = crate::logical();
                let info = format!(
                    "audio: {}\nrender {}x{} (heat saver div {}), logical {}x{}, immersive {}\nmode {:?}, landscape pref {}, blit hle {}, heat saver {}\npad hud {:?} (now {}), pad seen {}\n창고 오프라인 (앱 내장)\n\n{}",
                    crate::audio_sink::audio_status(),
                    self.win_w,
                    self.win_h,
                    self.render_div,
                    lw,
                    lh,
                    crate::immersive_active(),
                    self.mode,
                    config::read_landscape(&self.data_dir),
                    config::read_blit_hle(&self.data_dir),
                    config::read_heat_saver(&self.data_dir),
                    crate::pad_hud_pref(),
                    if crate::pad_hud_active() { "PAD" } else { "TOUCH" },
                    crate::pad_seen(),
                    // ★ 99차: the warehouse line is a constant now — there is no source to
                    // choose and no endpoint to print. (2026-08-27 already removed the
                    // endpoint from this text on the player's instruction; 99차 removed the
                    // endpoint itself.) ★ Unrelated to 세이브 복원: that is the diagnostics
                    // dialog's NEUTRAL BUTTON (`MainActivity.showDiagnostics` →
                    // `showSaveRestore`), not part of this text.
                    // ★ 59차: the pad map in Korean, by POSITION. This dialog is Java, so
                    // it can hold Hangul the overlay's 5x7 font cannot — which makes it
                    // the one place a player can read "면버튼 왼쪽 = 퀵슬롯 1" instead of
                    // guessing what the letter printed on their pad means.
                    crate::gamepad::legend(),
                );
                dialogs::show_diagnostics(&info);
                self.show_settings = false;
            }
            overlay::SettingsAction::PadMap => self.padmap_open(),
            overlay::SettingsAction::Close => {
                self.show_settings = false;
            }
        }
        self.request_redraw();
    }

    // ── 설정 → 패드 버튼 설정 (2026-10-09) ────────────────────────────────────────
    //
    // User: *"상단 톱니바퀴 설정으로 패드 키를 맵핑하는 기능을 넣을꺼야"*, then — the same day —
    // *"줄을 누르고 패드버튼을 누르는게 아니라 그냥 실제 인게임 화면상에 버튼들을 하이라이트
    // 처리하고 각 버튼을 눌르고 패드 버튼을 누르는 방식으로 변경해줘"*. So the screen is the
    // FIELD HUD as it is played, every control a pad button can be given lit where it sits
    // (`overlay::padmap_scene` / `draw_padmap`): a tap on one starts a five-second prompt, and
    // the next pad button pressed is that control's button. A conflict swaps
    // (`padmap::Pad::assign`). A bare press — nothing picked — says what that button does and
    // lights its control. 저장 installs the table and writes `wie_gamepad.txt`; 취소 drops
    // the copy.

    const PADMAP_TITLE: &'static str = "패드 버튼 설정";

    /// What the window says when nothing is going on.
    fn padmap_hint() -> [String; 2] {
        ["화면의 버튼을 누르고 패드 버튼을 누르세요".to_string(), "패드 버튼만 누르면 하는 일을 보여줍니다".to_string()]
    }

    /// Open the screen. Everything held is let go first — the fingers like the editor, and
    /// the PAD too: while it is open every button is an answer, not a game key.
    fn padmap_open(&mut self) {
        self.show_settings = false;
        self.release_touch_input();
        self.release_gamepad();
        self.padmap = Some(Box::new(PadMapUi {
            draft: crate::gamepad::overrides_snapshot(),
            sel: None,
            capture: None,
            probe: None,
            lines: Self::padmap_hint(),
        }));
        log::info!("pad map: open");
        self.request_redraw();
    }

    /// A function's name — its own label, or its file token for an action this screen does
    /// not offer (an adb-written `5`, say).
    fn padmap_label(action: Option<crate::gamepad::PadAction>) -> String {
        match action {
            None => "없음".to_string(),
            Some(a) => crate::gamepad::REMAP_ROWS
                .iter()
                .find(|(x, _)| *x == a)
                .map_or_else(|| crate::gamepad::token(Some(a)), |(_, l)| (*l).to_string()),
        }
    }

    /// What `redraw` draws, resolved before the surface is borrowed — and where an unanswered
    /// prompt runs out.
    fn padmap_view(&mut self) -> Option<PadMapView> {
        self.padmap.as_ref()?;
        let scene = overlay::padmap_scene(self.win_w, self.win_h, self.joystick);
        let slots: Vec<crate::QuickSlot> = (0..crate::quickpage::PLATES as u8)
            .map(|i| {
                self.quickbar.get(usize::from(i)).copied().unwrap_or(crate::QuickSlot {
                    slot: i,
                    guest: None,
                    kind: crate::SlotKind::Empty,
                    count: 0,
                    icon: None,
                })
            })
            .collect();
        let ui = self.padmap.as_deref_mut()?;
        let left = ui.capture.and_then(|at| crate::padmap::seconds_left(at.elapsed().as_millis()));
        if ui.capture.is_some() && left.is_none() {
            ui.capture = None;
            ui.sel = None;
            ui.lines = ["시간이 지났습니다".to_string(), "다시 화면의 버튼을 누르세요".to_string()];
            log::info!("pad map: the prompt ran out");
        }
        let badges = scene
            .targets
            .iter()
            .map(|t| crate::gamepad::pos_for_action_in(&ui.draft, crate::gamepad::REMAP_ROWS[t.row].0))
            .collect();
        // One breath every 1.1 s, from the moment the control was picked.
        let pulse = ui
            .capture
            .filter(|_| left.is_some())
            .map(|at| 0.5 - 0.5 * (at.elapsed().as_secs_f64() * std::f64::consts::TAU / 1.1).cos());
        let lines = [
            ui.lines[0].clone(),
            match left {
                Some(s) => format!("바꿀 패드 버튼을 누르세요 ({s}초)"),
                None => ui.lines[1].clone(),
            },
        ];
        let pad = crate::gamepad::pad();
        let mut normal = ui.draft.clone();
        pad.normalize(&mut normal);
        let mut in_force = crate::gamepad::overrides_snapshot();
        pad.normalize(&mut in_force);
        Some(PadMapView {
            scene,
            slots,
            badges,
            sel: ui.sel,
            probe: ui.probe,
            pulse,
            lines,
            dirty: normal != in_force,
            can_reset: !normal.is_empty(),
        })
    }

    /// A finger went down: on a lit control (or a chip in the window) it picks that function
    /// and starts the prompt; on 취소 · 기본값 · 저장 it does that; on the game picture it lets
    /// go of whatever was picked. Answered on the way DOWN — nothing here scrolls, so there is
    /// no gesture to tell a tap from.
    fn padmap_started(&mut self, x: f64, y: f64) {
        if self.padmap.is_none() {
            return;
        }
        let scene = overlay::padmap_scene(self.win_w, self.win_h, self.joystick);
        let hit = overlay::padmap_hit(&scene, x, y);
        match hit {
            Some(overlay::PadMapHit::Cancel) => return self.padmap_cancel(),
            Some(overlay::PadMapHit::Save) => return self.padmap_save(),
            _ => {}
        }
        let Some(ui) = self.padmap.as_deref_mut() else { return };
        match hit {
            Some(overlay::PadMapHit::Reset) => {
                ui.draft.clear();
                ui.sel = None;
                ui.capture = None;
                ui.probe = None;
                ui.lines = ["기본값으로 되돌렸습니다".to_string(), "저장을 누르면 적용됩니다".to_string()];
                log::info!("pad map: draft reset to the built-in layout");
            }
            Some(overlay::PadMapHit::Row(row)) => {
                let (action, label) = crate::gamepad::REMAP_ROWS[row];
                let now = crate::padmap::describe(&crate::gamepad::pad().codes_for(&ui.draft, action), crate::gamepad::button_name);
                ui.sel = Some(row);
                ui.capture = Some(Instant::now());
                ui.probe = None;
                ui.lines = [format!("{label}: 지금 {now}"), String::new()];
                log::info!("pad map: waiting for a button for {action:?}");
            }
            Some(_) => {}
            // The game picture: "never mind".
            None => {
                if ui.sel.is_some() || ui.probe.is_some() {
                    ui.sel = None;
                    ui.capture = None;
                    ui.probe = None;
                    ui.lines = Self::padmap_hint();
                }
            }
        }
        self.request_redraw();
    }

    /// A pad button (a real key, or a trigger's synthetic L2 / R2 edge) while the screen is
    /// open. Only a PRESS means anything: with a control picked and the prompt up it becomes
    /// that control's button; with nothing picked the window says what the button does and
    /// lights its control. Every edge is swallowed — the game must not see a button the
    /// player is pressing at this screen.
    ///
    /// ⛔ A release is NOT matched to anything on the way out: an up for a button held across
    /// 저장 / 취소 reaches `gamepad_action` with nothing in `pad_held` (opening cleared it), and
    /// every arm there ignores an unmatched release.
    fn padmap_button(&mut self, code: i32, down: bool, repeat: bool) {
        let Some(ui) = self.padmap.as_deref_mut() else { return };
        if !down || repeat {
            return;
        }
        let pad = crate::gamepad::pad();
        let name = crate::gamepad::button_name(code);
        let waiting = ui.capture.is_some_and(|at| crate::padmap::seconds_left(at.elapsed().as_millis()).is_some());
        let Some(row) = ui.sel.filter(|_| waiting) else {
            if ui.capture.take().is_some() {
                // The redraw that expires it may not have run yet; the press is still late.
                ui.sel = None;
                ui.lines = ["시간이 지났습니다".to_string(), "다시 화면의 버튼을 누르세요".to_string()];
            } else if !crate::padmap::capturable(code) {
                ui.probe = None;
                ui.lines = ["방향키는 바꿀 수 없습니다".to_string(), "걷기와 메뉴 이동에 쓰입니다".to_string()];
            } else {
                let action = pad.effective(&ui.draft, code);
                ui.probe = action.and_then(crate::gamepad::remap_row);
                ui.sel = None;
                ui.lines = [format!("{name} = {}", Self::padmap_label(action)), "바꾸려면 화면의 버튼을 먼저 누르세요".to_string()];
                log::info!("pad map: code {code} ({name}) fires {action:?}");
            }
            self.request_redraw();
            return;
        };
        let (action, label) = crate::gamepad::REMAP_ROWS[row];
        let result = pad.assign(&mut ui.draft, action, code);
        ui.lines = match result {
            // The prompt stays up: a direction is not an answer, it is a wrong key.
            crate::padmap::Assign::NotCapturable => ["방향키는 바꿀 수 없습니다".to_string(), String::new()],
            crate::padmap::Assign::Same => [format!("{label}: 이미 {name} 입니다"), Self::padmap_hint()[0].clone()],
            crate::padmap::Assign::Refused { lost } => [
                format!("{} 버튼이 없어지므로 바꿀 수 없습니다", Self::padmap_label(Some(lost))),
                "다른 패드 버튼으로 다시 해 보세요".to_string(),
            ],
            // The second line says where the swap sent the other function, in the first
            // line's own form — `기능 = 버튼` both, so no particle has to agree with a digit.
            crate::padmap::Assign::Done { displaced: Some(d), to: Some(from) } => [
                format!("{label} = {name}"),
                format!("{} = {}", Self::padmap_label(Some(d)), crate::gamepad::button_name(from)),
            ],
            crate::padmap::Assign::Done { displaced: Some(d), to: None } => {
                [format!("{label} = {name}"), format!("{} = 없음", Self::padmap_label(Some(d)))]
            }
            crate::padmap::Assign::Done { displaced: None, .. } => [format!("{label} = {name}"), "저장을 누르면 적용됩니다".to_string()],
        };
        if result != crate::padmap::Assign::NotCapturable {
            ui.capture = None;
        }
        log::info!("pad map: {action:?} <- code {code} ({name}): {result:?}");
        self.request_redraw();
    }

    /// 저장: install the copy, write it, take the screen down.
    fn padmap_save(&mut self) {
        let Some(ui) = self.padmap.take() else { return };
        let pad = crate::gamepad::pad();
        let mut table = ui.draft;
        pad.normalize(&mut table);
        let text = pad.serialize(&table, crate::gamepad::MAP_FILE_HEADER, crate::gamepad::token);
        let written = config::write_gamepad_map(&self.data_dir, &text);
        log::info!("pad map: saved {} override(s), written = {written}", table.len());
        crate::gamepad::set_overrides(table);
        let _ = self.note_input(false);
        // The badges are a reverse lookup of the table — rebuild what they hang off.
        self.relayout();
        if written {
            self.push_side_note("패드 버튼 설정을 저장했습니다".into());
        } else {
            // Applied for this run either way; the box says it will not survive a restart.
            self.push_notice("패드 버튼 설정을 파일에 쓰지 못했습니다 (이번 실행에만 적용)");
        }
        self.request_redraw();
    }

    /// 취소: drop the copy — the table in force was never touched.
    fn padmap_cancel(&mut self) {
        self.padmap = None;
        log::info!("pad map: cancelled");
        let _ = self.note_input(false);
        self.request_redraw();
    }

    // ── layout editor ───────────────────────────────────────────────────────

    /// Let go of everything the FINGERS hold, before a full-screen host mode (the layout
    /// editor, the pad screen) takes the input over. The worker replays held keys every
    /// 100 ms, so a key still down when input routing changes would be auto-repeated at
    /// the guest forever.
    fn release_touch_input(&mut self) {
        // ★ Order matters: `joy_end` ends with `refresh_pressed`, which rebuilds
        // `pressed` FROM `touches`. Clearing the touches afterwards would leave every
        // still-held key re-inserted into `pressed` after its key-up had already gone
        // to the guest — a button drawn lit for a key nobody is holding.
        self.touches.clear();
        self.popup_confirm_touches.clear();
        for key in self.pressed.drain() {
            let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
        }
        self.joy_end();
        self.pressed.clear();
        self.ui_touches.clear();
        self.slide_touches.clear();
        self.qty_slid.clear();
        self.mm_tap = None;
        self.stats_tap = None;
        self.menu_tap = None;
        self.quick_press = None;
    }

    /// Enter the editor. Everything the player is holding is released first: the
    /// worker replays held keys every 100 ms, so a key still down when input routing
    /// changes would be auto-repeated at the guest forever. Same body as the
    /// suspend / focus-loss teardown, for the same reason.
    fn enter_editor(&mut self) {
        // ★ Only the landscape-immersive HUD has placeable controls. Every other
        // layout family (portrait, and non-immersive landscape) builds a fixed D-pad
        // + keypad from the screen edges and would ignore every stored placement, so
        // opening the editor there would let the player arrange something that never
        // appears. Refuse and say why rather than writing entries for a layout that
        // cannot express them.
        if !overlay::layout_editor_available(self.win_w, self.win_h) {
            // The row is drawn dimmed with "FULL ONLY" in this case and its tap is
            // swallowed by `settings_hit`, so this is belt-and-braces.
            log::info!("layout editor: only available in landscape full-screen (FULL) mode");
            return;
        }
        self.show_settings = false;
        self.release_touch_input();
        // Open on the FIELD set: that is the HUD the player spends the game in, and
        // it is the one they cannot reach the gear from.
        self.editor = Some(Editor {
            sel: None,
            entry_snapshot: placement::table(),
            hidden_snapshot: crate::quick_hidden(),
            drag: None,
            pinch: None,
            points: HashMap::new(),
        });
        self.relayout();
        self.request_redraw();
        log::info!("layout editor: open");
    }

    /// Switch the SELECTED quickbar plate off (editor `HIDE`).
    ///
    /// ★ Player request 2026-09-10: "상단 톱니바퀴에서 스킬 퀵슬롯을 선택 후 비활성화
    /// 처리가 가능해? 숨김처리를 하는거야 … 퀵슬롯이 너무많아서 불편하다는 유저도있어서".
    ///
    /// Applies AT ONCE, like TOUCH TYPE and PAD HUD and unlike HEAT SAVER: the ring is
    /// laid out from the preference, so `relayout` IS the apply step and the player
    /// watches the survivors close the gap. The selection is cleared because the thing
    /// it named has just left the screen.
    ///
    /// ⛔ The refusal cases both say something. A silent no-op here is the "full-width
    /// button that does nothing" the settings panel has a rule against, and the reason
    /// the last plate cannot go is spelled out in `quickpage::hide_plate`.
    fn editor_hide_plate(&mut self) {
        let Some(placement::CtrlId::Quick(plate)) = self.editor.as_ref().and_then(|ed| ed.sel) else {
            self.push_notice("퀵슬롯을 먼저 선택하세요");
            log::info!("layout editor: HIDE needs a quickslot plate selected");
            return;
        };
        let Some(mask) = crate::quickpage::hide_plate(crate::quick_hidden(), plate) else {
            self.push_notice("퀵슬롯 하나는 남겨야 합니다");
            log::info!("layout editor: refusing to hide plate {} — it is the last visible one", plate + 1);
            return;
        };
        crate::set_quick_hidden(mask);
        config::write_quick_hidden(&self.data_dir, mask);
        if let Some(ed) = self.editor.as_mut() {
            ed.sel = None;
            ed.drag = None;
            ed.pinch = None;
        }
        // ⛔ A `&'static str`, not the plate number: the number is in the log line below
        // anyway — the player has just watched that plate leave the ring, which says which
        // one better than a digit would.
        self.push_notice("퀵슬롯 숨김");
        log::info!(
            "layout editor: plate {} hidden — mask {:#04x}, {} visible",
            plate + 1,
            mask,
            crate::quickpage::visible_count(mask)
        );
        self.relayout();
        self.request_redraw();
    }

    /// Switch every hidden plate back on (editor `SHOW`).
    fn editor_show_plates(&mut self) {
        let was = crate::quick_hidden();
        if was == crate::quickpage::SHOW_ALL {
            self.push_notice("숨긴 퀵슬롯이 없습니다");
            return;
        }
        crate::set_quick_hidden(crate::quickpage::SHOW_ALL);
        config::write_quick_hidden(&self.data_dir, crate::quickpage::SHOW_ALL);
        self.push_notice("퀵슬롯 전체 복원");
        log::info!("layout editor: plates restored — was mask {was:#04x}");
        self.relayout();
        self.request_redraw();
    }

    /// Drop any half-finished drag / pinch without closing the editor. Called from
    /// the same places that release held keys: a gesture whose `Ended` will never
    /// arrive (backgrounded, focus stolen) must not stay armed, and a resize
    /// invalidates the anchors outright.
    fn clear_editor_gesture(&mut self) {
        if let Some(ed) = self.editor.as_mut() {
            ed.drag = None;
            ed.pinch = None;
            ed.points.clear();
        }
    }

    fn exit_editor(&mut self) {
        self.editor = None;
        config::write_layout(&self.data_dir, &placement::table());
        self.relayout();
        self.request_redraw();
        log::info!("layout editor: closed, layout saved");
    }

    /// Commit a new centre and size percent for `id`, but only if the result keeps
    /// the whole set legal.
    ///
    /// ★ The size is passed in as a PERCENT, never re-derived from a pixel rect.
    /// Round-tripping px -> percent -> px floors twice, so a drag that carried its
    /// own width through would shave ~1 % off the control on every touch event: a
    /// 130 %-sized plate (98 px built-in -> 127 px) reads back as 129 %, then 128 %,
    /// and a control the player merely dragged around would visibly wither.
    ///
    /// What IS round-tripped is the rect: it is rebuilt from the stored fractions
    /// before being validated, so the editor accepts exactly what `resolve` will
    /// reconstruct later.
    fn editor_commit(&mut self, id: placement::CtrlId, cx: f64, cy: f64, pct: u32) -> bool {
        // The editor has to be OPEN — the guard is the whole point of the binding, which
        // is why it stays even though the mode no longer comes out of it.
        if self.editor.is_none() {
            return false;
        }
        let (mode, w, h) = (EDIT_MODE, self.win_w, self.win_h);
        let place = placement::Place {
            cx: (cx / f64::from(w.max(1))) as f32,
            cy: (cy / f64::from(h.max(1))) as f32,
            pct: pct.clamp(placement::PCT_MIN, placement::PCT_MAX),
        };
        if !placement::would_accept_place(mode, w, h, id, place) {
            return false;
        }
        placement::set(id, Some(place));
        self.relayout();
        self.request_redraw();
        true
    }

    /// The size percent `id` is currently stored at (100 = built-in).
    fn editor_pct(&self, id: placement::CtrlId) -> u32 {
        self.editor
            .as_ref()
            .and_then(|_| placement::get(id))
            .map(|p| p.pct)
            .unwrap_or(placement::PCT_DEFAULT)
    }

    /// The centre `id` is anchored at, in window px — the STORED fraction where there
    /// is one, so a pinch against a screen edge cannot walk the control sideways.
    fn editor_centre(&self, id: placement::CtrlId) -> (f64, f64) {
        let (w, h) = (self.win_w, self.win_h);
        if self.editor.is_none() {
            return (0.0, 0.0);
        }
        match placement::get(id) {
            Some(p) => (f64::from(p.cx) * f64::from(w), f64::from(p.cy) * f64::from(h)),
            None => {
                let b = placement::builtin(id, EDIT_MODE, w, h);
                (f64::from(b.x) + f64::from(b.w) / 2.0, f64::from(b.y) + f64::from(b.h) / 2.0)
            }
        }
    }

    fn editor_started(&mut self, id: u64, x: f64, y: f64) {
        let (w, h) = (self.win_w, self.win_h);
        let items = overlay::edit_items(w, h);
        if let Some(action) = overlay::edit_hit(&items, x, y) {
            match action {
                overlay::EditAction::Reset => {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.sel = None;
                        ed.drag = None;
                        ed.pinch = None;
                        placement::clear();
                    }
                    config::write_layout(&self.data_dir, &placement::table());
                    self.relayout();
                    self.request_redraw();
                }
                overlay::EditAction::Flip => {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.drag = None;
                        ed.pinch = None;
                        let mut t = placement::table();
                        t.mirror(w, h);
                        placement::set_table(t);
                        config::write_layout(&self.data_dir, &placement::table());
                        log::info!("layout editor: layout mirrored left<->right");
                    }
                    self.relayout();
                    self.request_redraw();
                }
                overlay::EditAction::HidePlate => self.editor_hide_plate(),
                overlay::EditAction::ShowPlates => self.editor_show_plates(),
                overlay::EditAction::Cancel => {
                    // Put back exactly what was on disk when the editor opened.
                    if let Some(ed) = self.editor.as_ref() {
                        placement::set_table(ed.entry_snapshot.clone());
                        let mask = ed.hidden_snapshot;
                        if mask != crate::quick_hidden() {
                            crate::set_quick_hidden(mask);
                            config::write_quick_hidden(&self.data_dir, mask);
                        }
                    }
                    config::write_layout(&self.data_dir, &placement::table());
                    self.editor = None;
                    self.relayout();
                    self.request_redraw();
                    log::info!("layout editor: cancelled, layout restored");
                }
                overlay::EditAction::Save => self.exit_editor(),
            }
            if let Some(ed) = self.editor.as_mut() {
                ed.points.insert(id, (x, y));
            }
            return;
        }

        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.insert(id, (x, y));
        // ★ THE SELECTION IS STICKY. A touch that misses every control does NOT clear
        // it (player-reported 2026-07-31: "두 손가락으로 확대하려면 선택이 풀려버려서
        // 동작이 안 돼"). Two fingers never land in the same event — the second one
        // arrives as its own `Started`, almost always on empty screen — so treating a
        // miss as "deselect" made the pinch unreachable by construction. While
        // something is selected, every touch either MOVES or RESIZES it; the selection
        // changes only by tapping a different control.
        let mode = EDIT_MODE;
        // `shown`: a quickbar plate the player switched off is not drawn by the
        // editor, so it must not be selectable by a blind tap either. (Until 91차 the
        // same filter hid whichever of stick / D-pad the live touch type did not use;
        // they are one STICK control now, shown under both types.)
        let hit = placement::resolve(mode, w, h)
            .into_iter()
            .find(|(id, r)| placement::shown(mode, *id) && overlay::rect_hit(r, x, y));
        if let Some((cid, _)) = hit
            && Some(cid) != ed.sel
        {
            // Tapping a DIFFERENT control is the one thing that moves the selection.
            ed.sel = Some(cid);
            ed.drag = None;
            ed.pinch = None;
        }

        let Some(sel) = ed.sel else {
            // Nothing selected and nothing under the finger: nothing to do.
            self.request_redraw();
            return;
        };

        // Second finger while something is selected = resize it, from wherever the
        // fingers are. Pinching ON the control is not workable: a skill plate is ~50 px
        // and will not hold two fingertips.
        if ed.pinch.is_none() && ed.points.len() >= 2 {
            // ★ Pair the finger that just landed with the one actually driving the
            // gesture — the dragging finger, else the nearest other contact. Taking the
            // two LOWEST pointer ids (Android hands out the lowest free id) would pick
            // the OLDEST contacts, i.e. a thumb resting on the screen since before the
            // gesture began, and the resize would be measured against a finger that
            // never moves.
            let other = ed.drag.map(|d| d.id).filter(|d| *d != id && ed.points.contains_key(d)).or_else(|| {
                ed.points
                    .iter()
                    .filter(|(k, _)| **k != id)
                    .min_by(|a, b| distance(*a.1, (x, y)).total_cmp(&distance(*b.1, (x, y))))
                    .map(|(k, _)| *k)
            });
            if let Some(a) = other {
                let dist0 = distance(ed.points[&a], (x, y));
                if dist0 > 1.0 {
                    let pct0 = placement::get(sel).map(|p| p.pct).unwrap_or(placement::PCT_DEFAULT);
                    ed.pinch = Some(Pinch { a, b: id, dist0, pct0 });
                    ed.drag = None;
                    self.request_redraw();
                    return;
                }
            }
        }

        // Single finger, anywhere: MOVE the selection. Tracked as a delta from where
        // the finger went down (not as an offset inside the rect), so a drag that
        // starts off the control does not teleport it under the fingertip.
        if ed.pinch.is_none() {
            let centre = self.editor_centre(sel);
            if let Some(ed) = self.editor.as_mut() {
                ed.drag = Some(Drag {
                    id,
                    from: (x, y),
                    centre0: centre,
                });
            }
        }
        self.request_redraw();
    }

    fn editor_moved(&mut self, id: u64, x: f64, y: f64) {
        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.insert(id, (x, y));
        let sel = ed.sel;
        let Some(sel) = sel else { return };

        if let Some(p) = ed.pinch.as_ref() {
            let (Some(&pa), Some(&pb)) = (ed.points.get(&p.a), ed.points.get(&p.b)) else {
                return;
            };
            // ABSOLUTE, from the anchor captured at pinch start — never a running
            // product of per-event deltas, which winit's per-pointer Moved would
            // apply twice per MotionEvent.
            let ratio = distance(pa, pb) / p.dist0.max(1.0);
            let pct = ((f64::from(p.pct0) * ratio) as u32).clamp(placement::PCT_MIN, placement::PCT_MAX);
            // Resize about the control's own anchor, so it grows in place.
            let (cx, cy) = self.editor_centre(sel);
            self.editor_commit(sel, cx, cy, pct);
            return;
        }

        if let Some(d) = ed.drag
            && d.id == id
        {
            // Carry the size through as a PERCENT (see `editor_commit`) and move by the
            // finger's DELTA from where it went down.
            let pct = self.editor_pct(sel);
            let cx = d.centre0.0 + (x - d.from.0);
            let cy = d.centre0.1 + (y - d.from.1);
            // A rejected position is simply not committed — the control stays where
            // it last legally was, which reads as sliding along the obstacle.
            self.editor_commit(sel, cx, cy, pct);
        }
    }

    fn editor_ended(&mut self, id: u64) {
        let Some(ed) = self.editor.as_mut() else { return };
        ed.points.remove(&id);
        if ed.drag.map(|d| d.id) == Some(id) {
            ed.drag = None;
        }
        if ed.pinch.as_ref().is_some_and(|p| p.a == id || p.b == id) {
            ed.pinch = None;
            // One finger of the pinch is still down: hand it back to MOVE, so lifting
            // one of two fingers slides the control instead of freezing it.
            let still: Vec<u64> = ed.points.keys().copied().collect();
            if let (Some(sel), [other]) = (ed.sel, still.as_slice()) {
                let at = ed.points[other];
                let centre = self.editor_centre(sel);
                if let Some(ed) = self.editor.as_mut() {
                    ed.drag = Some(Drag {
                        id: *other,
                        from: at,
                        centre0: centre,
                    });
                }
            }
        }
        // Persist on every finger-up rather than only on DONE: the app can be killed
        // from the background at any moment, and a lost layout is worse than a write.
        config::write_layout(&self.data_dir, &placement::table());
        self.request_redraw();
    }

    fn set_control_style(&mut self, joystick: bool) {
        if self.joystick != joystick {
            self.joy_end(); // release any held joystick dirs before switching
            self.joystick = joystick;
            config::write_joystick(&self.data_dir, joystick);
            self.relayout();
        }
        self.show_settings = false;
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // Foregrounded: resume emulation + audio (BGM continues from where it froze).
        self.set_paused(false);

        // ⛔ THE "세이브 복원 가능" TOAST IS GONE (2026-08-27, player: *"게임 처음
        // 실행했을떄 세이브복원가능 이라는 문구도 제거해줘 굳이 필요없는거같아"*).
        //
        // 60차 raised it when a character record was MISSING while the vault still held a
        // usable copy. The verdict itself is worth keeping and `vault::check_at_boot` still
        // computes it — it just logs now (`vault: … is MISSING or unreadable — a N-byte
        // copy from T can be restored`) instead of interrupting every launch. Restoring was
        // never something the toast did: 설정 → 진단 정보 → [세이브 복원] is the only way in,
        // and it is always there whether or not anything is missing.

        let window = match event_loop.create_window(Window::default_attributes().with_title("WIE")) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create_window failed: {e}");
                return;
            }
        };
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Context::new failed: {e}");
                return;
            }
        };

        let size = window.inner_size();
        // HEAT SAVER (57차): everything from here on happens in RENDER space —
        // possibly half the physical window on each axis, upscaled by the
        // compositor. Touch input is divided by the same factor on entry.
        self.win_w = (size.width.max(1) / self.render_div).max(1);
        self.win_h = (size.height.max(1) / self.render_div).max(1);
        log::info!(
            "resumed: window size {}x{} -> render {}x{} (heat saver div {})",
            size.width,
            size.height,
            self.win_w,
            self.win_h,
            self.render_div
        );

        self.window = Some(window);
        self.context = Some(context);
        self.surface = None;

        // First resume: now that the real window size is known, fix the guest
        // framebuffer size and start the emulator. Landscape immersive mode gives
        // the guest an aspect-matched wide framebuffer so the game frame fills the
        // screen with no letterbox and no distortion (see `crate::init_logical`).
        if let Some(boot) = self.pending_boot.take() {
            // ★ 90차: a slot the vault put back at boot is announced on the first frame.
            if let Some(msg) = crate::vault::take_boot_notice() {
                self.push_notice(msg);
            }
            let immersive = config::read_immersive(&self.data_dir);
            let landscape_pref = config::read_landscape(&self.data_dir);
            let base_h = config::read_base_h(&self.data_dir);
            let (lw, lh) = crate::init_logical(self.win_w, self.win_h, immersive, landscape_pref, base_h);
            log::info!(
                "boot: window {}x{} landscape_pref={landscape_pref} -> guest framebuffer {lw}x{lh} (immersive={immersive}, base_h={base_h})",
                self.win_w,
                self.win_h
            );
            self.last_frame = vec![0xff00_0000u32; (lw * lh) as usize];
            crate::emulator::spawn(
                boot.proxy,
                boot.should_redraw,
                boot.input_rx,
                self.data_dir.clone(),
                boot.game_zip,
                self.paused.clone(),
                boot.hud_y,
            );
        }

        self.relayout();
        self.configure_surface();
        self.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("suspended: dropping surface/context/window");
        // Backgrounded: halt emulation + audio so nothing keeps playing.
        self.set_paused(true);
        self.surface = None;
        self.context = None;
        self.window = None;
        self.release_gamepad();
        for key in self.pressed.drain() {
            let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
        }
        self.touches.clear();
        self.popup_confirm_touches.clear();
        self.ui_touches.clear();
        self.slide_touches.clear();
        self.qty_slid.clear();
        self.mm_tap = None;
        self.stats_tap = None;
        self.menu_tap = None;
        self.joy = None;
        self.quick_press = None;
        self.clear_editor_gesture();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            // The guest's own 종료, after the worker's final save. Ending the winit
            // loop alone is not enough on Android — the Activity would stay in the
            // task list and resume into a stopped emulator — so the Java side
            // finishes the task and ends the process (`MainActivity.quitApp`).
            UserEvent::ExitApp => {
                log::warn!("exit: tearing down");
                dialogs::quit_app();
                event_loop.exit();
            }
            UserEvent::GamepadButton { code, down, repeat } => {
                let _ = self.note_input(true);
                // ★ 패드 버튼 설정 (2026-10-09): while it is open a button is an ANSWER, looked
                // at by its raw keycode — before the table, which is the thing being changed.
                if self.padmap.is_some() {
                    self.padmap_button(code, down, repeat);
                    return;
                }
                if let Some(action) = crate::gamepad::button_action(code) {
                    self.gamepad_action(action, down, repeat);
                }
            }
            UserEvent::GamepadAxes { x, y, hx, hy, lt, rt } => {
                // A stick at rest still streams samples on some pads, so this is the one
                // pad entry point that must NOT claim the input source on its own — only
                // a sample that actually moves something does (`gamepad_axes`).
                self.gamepad_axes(x, y, hx, hy, lt, rt)
            }
            UserEvent::GameOptionsChanged => {
                self.joy_end();
                self.relayout();
                self.request_redraw();
            }
            UserEvent::Paint(data) => {
                // ★ The frame we are DISPLACING goes back to the worker's pool instead of
                // being freed (2026-08-06). `AndroidScreen::paint` allocated a fresh
                // ~476 KB `Vec<u32>` for every guest frame and this line dropped the
                // previous one — an allocate+free pair per frame for a buffer whose size
                // never changes. See `platform::recycle_frame`.
                let retired = std::mem::replace(&mut self.last_frame, data);
                crate::platform::recycle_frame(retired);
                // ★ 168차: a page the worker LEFT stays up only while the game still paints
                // EQUIPMENT's worn grid — the first other frame (SKILL, the MENU list) takes it
                // down, so the hand-over is one frame to the next with no EQUIPMENT between.
                if self.avatar_leaving > 0 {
                    self.avatar_leaving -= 1;
                    if self.avatar_leaving == 0 || self.equipment_grid_shown().is_none() {
                        self.avatar_leaving = 0;
                        self.avatar_view = None;
                    }
                }
                // ★ 150차 review: what the 상세보기 note answers to is the frame ON SCREEN. A
                // frame that no longer shows page 7 (CLR closed it) ends the note's claim on
                // keys and taps NOW, not at the next redraw — or the DOWN right after CLR
                // scrolled a closed note instead of moving the inventory cursor, and a quick
                // tap on the bag's «9» badge (inside `DETAIL_BODY`) was taken as a drag.
                if self.note_up {
                    let (lw, lh) = crate::logical();
                    if !menutouch::detail_page(&self.last_frame, lw as i32, lh as i32) {
                        self.note_up = false;
                        self.note_drag = None;
                    }
                }
                // ★★★ 109차 — publish "a bag CELL cursor is drawn" for the OK-key path.
                // The worker cannot see pixels, and on the 상점 that one bit is the whole
                // difference between the 판매 grid (swallow OK, open the box) and the 구매
                // list (leave OK alone, or the player cannot buy) — the two are the same
                // window id with the same shared cursor. See `crate::cell_cursor_shown`.
                // ⛔ MENU only: off in the field this is both meaningless and a scan the
                // per-frame path should not pay.
                crate::set_cell_cursor_shown(if self.mode == overlay::Mode::Menu {
                    let (lw, lh) = crate::logical();
                    menutouch::cell_box(&self.last_frame, lw as i32, lh as i32).is_some()
                } else {
                    false
                });
                self.request_redraw();
            }
            UserEvent::MinimapFrame(mm) => {
                // ★ `None` CLEARS the panel (2026-08-03). It used to be dropped, which
                // was a hedge against flicker back when the worker could produce a
                // spurious `None`; it no longer can — the only sender is a confirmed
                // map change — and the hedge meant that walking through a door left the
                // PREVIOUS room's map on screen, stated as confidently as a correct one.
                // An empty panel says "tap me", a wrong one says nothing at all.
                if self.minimap != mm {
                    self.minimap = mm;
                    self.request_redraw();
                }
            }
            UserEvent::MinimapMarks { player, mobs } => {
                // ★ 141차: only worth a repaint while the view is actually on screen.
                if self.minimap_marker != player || self.minimap_mobs != mobs {
                    self.minimap_marker = player;
                    self.minimap_mobs = mobs;
                    if self.mode == overlay::Mode::Field && self.minimap_on && self.can_save {
                        self.request_redraw();
                    }
                }
            }
            UserEvent::Popup(view) => {
                if self.popup != view {
                    self.popup = view;
                    self.request_redraw();
                }
            }
            UserEvent::PopupList(list) => self.popup_list = list,
            UserEvent::PickingSlot(on) => {
                if self.picking_slot != on {
                    self.picking_slot = on;
                    self.request_redraw();
                }
            }
            UserEvent::Notice(text) => self.push_notice(text),
            UserEvent::NoticeText(text) => self.push_notice(text),
            UserEvent::SideNote(text) => self.push_side_note(text),
            // ★ 143차: the wake's veil. Rising takes every input away at once (a held
            // direction would walk the player on into the town they are about to see) and
            // closes the exit box if it was up; falling starts from wherever the veil is.
            UserEvent::WakeVeil { rise, ms } => {
                let from = self.veil_now().map_or(0.0, |(a, _)| f32::from(a) / 255.0);
                if rise {
                    self.release_all_input();
                    self.exit_ask = None;
                }
                self.veil = Some((from, if rise { 1.0 } else { 0.0 }, Instant::now(), ms));
                self.request_redraw();
            }
            UserEvent::QuickPage(page) => {
                if self.quick_page != page {
                    self.quick_page = page;
                    self.request_redraw();
                }
            }
            UserEvent::SlotWindow { first, total } => {
                if self.slot_window != Some((first, total)) {
                    self.slot_window = Some((first, total));
                    self.request_redraw();
                }
            }
            UserEvent::SkillCast { slot, reduce_pct } => {
                // The worker confirmed the cast actually started: NOW the cooldown
                // begins, for this skill's own length (builtin + wie_cooldowns.txt)
                // after worn-gear `addStat(0x11)` percent. Tap-block and fill share
                // this one `dur` — a 0 base still means no overlay.
                if let Some(crate::IconRef::Skill(sheet, cell)) = self.quickbar.iter().find(|s| s.slot == slot).and_then(|s| s.icon) {
                    let base = self.skill_cds_cfg.get(&(sheet, cell)).copied().unwrap_or(0);
                    let dur = config::apply_cd_reduce(base, reduce_pct);
                    if dur > 0 {
                        if reduce_pct > 0 {
                            log::info!("skill cd: s{} cell {cell} {base} ms → {dur} ms (−{reduce_pct}%)", sheet + 4);
                        }
                        self.skill_cd.insert((sheet, cell), (Instant::now(), dur));
                        self.request_redraw();
                    }
                }
            }
            UserEvent::CashQty(on) => {
                if self.cash_qty != on {
                    self.cash_qty = on;
                    self.request_redraw();
                }
            }
            UserEvent::OrbPick(pick) => {
                if let Some(p) = pick.as_deref() {
                    // ★ A REFRESH keeps the player's place: the same list came back with
                    // one row's count changed (a use) or one row gone (a 버리기), and
                    // snapping a 48-stack box back to the top is the very thing 93차 fixed.
                    if !p.keep_view {
                        self.orb_sel = 0;
                        self.orb_scroll = 0.0;
                    }
                    self.orb_sel = self.orb_sel.min(p.rows.len().saturating_sub(1));
                    self.orb_scroll = crate::orbcombine::clamp_scroll(
                        self.orb_scroll,
                        p.rows.len(),
                        overlay::orb_rects(self.win_w, self.win_h).row_h,
                        overlay::orb_rects(self.win_w, self.win_h).list.h,
                    );
                    self.orb_drag = None;
                    // ★★ 2026-09-14, user: *"들어가면 첫번째 아이템 기본 default 포커스
                    // 처리하면 돼"*. Row 0 is focused on open, which reverses 70차's
                    // "opens unfocused". What 70차 was guarding against is now guarded by
                    // `orb_armed_at` — see `Self::ORB_ARM_MS`.
                    self.orb_focused = true;
                    // ⛔ A refresh does not re-arm (110차-D): the list did not just open,
                    //   and a drop box the player already has up would otherwise swallow
                    //   its next tap for `ORB_ARM_MS` — a dead tap right after a use.
                    //   ⛔ Unless NO list was on screen (117차 code review): 116차 hides the box
                    //   window while the game's 귀환 dialog is up and brings it back with
                    //   `keep_view` so the player's row survives — but a list that comes up
                    //   from nothing IS a fresh open, and without the arm the very next OK
                    //   confirmed a row on it (the 70차/93차 hazard, back by another door).
                    if !p.keep_view || self.orb_pick.is_none() {
                        self.orb_armed_at = std::time::Instant::now();
                    }
                }
                // A withdrawn dialog takes its notice and its drop box with it.
                if pick.is_none() {
                    self.orb_notice = None;
                    self.orb_drop = None;
                    self.orb_reveal = false;
                }
                self.orb_pick = pick;
                self.request_redraw();
            }
            UserEvent::OrbDrop(drop) => {
                if drop.is_some() {
                    self.orb_drop_sel = 0;
                    self.orb_armed_at = std::time::Instant::now();
                    self.orb_drag = None;
                }
                self.orb_drop = drop;
                self.request_redraw();
            }
            // ★ 96차: a box row's 상세보기, or one of the game's own 올리기 refusals, in the
            // game-style message box — over the list, which stays where it is underneath.
            UserEvent::OrbReveal(on) => {
                if self.orb_reveal != on {
                    self.orb_reveal = on;
                    self.request_redraw();
                }
            }
            UserEvent::OrbNotice(text) => {
                self.orb_notice = Some(overlay::wrap_notice_text(&text));
                self.orb_drag = None;
                self.request_redraw();
            }
            UserEvent::RefineOdds(odds) => {
                // The worker only sends this when the number CHANGES, so an unconditional
                // redraw here is exactly one repaint per stone the player dials in.
                self.refine = odds;
                self.request_redraw();
            }
            // ★ 150차: the 상세보기 note. Another ITEM starts at the top of its body; the same
            // item with a refreshed note (a piece put on or taken off) keeps its place, clamped
            // again on the next draw. Nothing is drawn from this alone — `compose_note` checks
            // the game's own frame first.
            UserEvent::ItemNote { item, note } => {
                if item != self.note_item {
                    self.note_item = item;
                    self.note_scroll = 0;
                    self.note_drag = None;
                }
                self.item_note = note;
                self.request_redraw();
            }
            // ★ 167차: the AVATAR page. Nothing is drawn from this alone — `compose_avatar`
            // checks the game's own frame first.
            UserEvent::AvatarPage(view) => {
                self.avatar_view = view;
                self.avatar_leaving = 0;
                self.request_redraw();
            }
            // ★ 168차: left with a key the game acts on next — kept up until the frame moves on
            // (the `Paint` arm drops it), so EQUIPMENT never shows in between.
            UserEvent::AvatarLeave => {
                if self.avatar_view.is_some() {
                    self.avatar_leaving = AVATAR_LEAVE_FRAMES;
                }
            }
            UserEvent::MenuAvatar(sel) => {
                if self.menu_sel != sel {
                    self.menu_sel = sel;
                    self.request_redraw();
                }
            }
            UserEvent::WhQty(ask) => {
                if self.wh_qty != ask {
                    // A fresh ask starts at the whole stack — the answer that matches
                    // what 올리기/내리기 did before there was a picker, so a player who
                    // just presses OK sees no change in behaviour.
                    if let Some((max, _, _, _)) = ask {
                        // Start at the ceiling — the common answer is "all of it" — but never
                        // below the floor the engine set.
                        self.wh_qty_value = max.max(1);
                    }
                    self.wh_qty = ask;
                    self.request_redraw();
                }
            }
            UserEvent::Hud {
                field,
                cutscene,
                choice,
                map_open,
                shell,
                shell_screen,
                can_save,
                slots,
                stats,
                secondary,
                buff_lines,
                field_line,
                tower_hud,
                tower_flash,
                tower_exit,
                sort_screen,
                portrait,
            } => {
                let mode = match (field, cutscene) {
                    (true, _) => overlay::Mode::Field,
                    (_, true) => overlay::Mode::Cutscene,
                    _ => overlay::Mode::Menu,
                };
                let choice_changed = choice != self.choice;
                self.choice = choice;
                self.cutscene = cutscene;
                let mut changed = slots != self.quickbar
                    || stats != self.stats
                    || secondary != self.secondary
                    || map_open != self.map_open
                    || shell != self.shell
                    || can_save != self.can_save
                    || buff_lines != self.buff_lines
                    || field_line != self.field_line
                    || tower_hud != self.tower_hud
                    || tower_exit != self.tower_exit
                    || sort_screen != self.sort_screen
                    || portrait != self.portrait;
                self.secondary = secondary;
                self.portrait = portrait;
                self.buff_lines = buff_lines;
                self.field_line = field_line;
                self.tower_hud = tower_hud;
                self.tower_exit = tower_exit;
                // ★ 143차: the tab went away (the run ended, the player left the tower) —
                // a question about leaving has nothing left to ask about.
                if !tower_exit {
                    self.exit_ask = None;
                }
                self.sort_screen = sort_screen;
                // ⛔ Only a CHANGE arms it, and only after the first HUD event has set
                // the baseline — otherwise a fresh app would flash on boot.
                if tower_flash != self.tower_flash_gen {
                    self.tower_flash_gen = tower_flash;
                    self.tower_flash_left = Self::TOWER_FLASH_FRAMES;
                    self.request_redraw();
                }
                self.map_open = map_open;
                self.shell = shell;
                // Not in `changed`: nothing is DRAWN from it — only the slot-list tap arm reads it.
                self.shell_screen = shell_screen;
                self.can_save = can_save;
                // ★ The choice flag changes the BUTTON SET (the arrows appear for it),
                // so it needs the same relayout the mode does — a redraw alone would
                // paint the old layout. Folded into the mode branch below by `||`.
                if self.mode != mode || choice_changed {
                    // Field <-> game-window <-> cutscene: the whole control set
                    // changes, and so does which touch controls exist at all.
                    self.mode = mode;
                    self.relayout();
                    // ★ The stick zone is FIELD-only (2026-08-17), so leaving the field
                    // mid-drag would otherwise leave a held direction with nothing on
                    // screen to explain it — and `joy_end` is what releases that key.
                    if self.layout.joystick.is_none() {
                        self.joy_end();
                    }
                    changed = true;
                }
                self.stats = stats;
                if slots != self.quickbar {
                    self.quickbar = slots;
                    // A shrunk list can orphan a held press index; clear it.
                    if self.quick_press.is_some_and(|(_, i)| i >= self.quickbar.len()) {
                        self.quick_press = None;
                    }
                    self.relayout_quick();
                }
                if changed {
                    self.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                self.win_w = (new_size.width.max(1) / self.render_div).max(1);
                self.win_h = (new_size.height.max(1) / self.render_div).max(1);
                log::info!("resized: {}x{} -> render {}x{}", new_size.width, new_size.height, self.win_w, self.win_h);
                // Every in-flight gesture anchor is in the OLD coordinate space.
                self.clear_editor_gesture();
                self.relayout();
                self.configure_surface();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            // Hardware keyboards and D-pad-only devices (58차). Gamepad-source
            // events normally never arrive here — `MainActivity.dispatchKeyEvent`
            // consumes them into the JNI route — but winit's `Unidentified` Android
            // scancodes go through the same button table as belt-and-braces.
            WindowEvent::KeyboardInput { event, .. } => {
                // ★ 패드 버튼 설정: a pad button that came the winit way (belt-and-braces —
                // the Java hooks normally take them) is an answer too; a keyboard key is not
                // a pad button and is swallowed.
                if self.padmap.is_some() {
                    if let winit::keyboard::PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Android(sc)) = event.physical_key {
                        self.padmap_button(sc as i32, event.state.is_pressed(), event.repeat);
                    }
                    return;
                }
                if let Some(action) = crate::gamepad::winit_key_action(&event.physical_key) {
                    // A hardware keyboard / TV remote is not a pad, but it is not a
                    // TOUCH either: it wants the same "no on-screen walk controls" HUD,
                    // and leaving the source as touch would draw arrows nobody can reach.
                    let _ = self.note_input(true);
                    self.gamepad_action(action, event.state.is_pressed(), event.repeat);
                }
            }
            // Lost/gained foreground focus. This catches screen-lock (and other
            // focus-stealing overlays) even when the surface is NOT destroyed, so
            // it complements suspended()/resumed(): either signal halts audio.
            WindowEvent::Focused(focused) => {
                self.set_paused(!focused);
                if !focused {
                    // Also release any held game keys so nothing sticks on return.
                    self.release_gamepad();
                    for key in self.pressed.drain() {
                        let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
                    }
                    self.touches.clear();
                    self.popup_confirm_touches.clear();
                    self.ui_touches.clear();
                    self.slide_touches.clear();
                    self.qty_slid.clear();
                    self.mm_tap = None;
                    self.stats_tap = None;
                    self.menu_tap = None;
                    self.note_drag = None;
                    self.joy = None;
                    self.quick_press = None;
                    self.clear_editor_gesture();
                }
            }
            WindowEvent::Touch(Touch { phase, location, id, .. }) => {
                // Physical -> render space (HEAT SAVER divisor; 1 = identity).
                let s = self.render_div as f64;
                let (x, y) = (location.x / s, location.y / s);
                // ★ The editor is the FIRST branch of every phase. It has to sit above
                // `map_open` (which the worker can set on its own and which swallows
                // any tap into a CLEAR) and above `show_settings` (whose "tap outside
                // the panel closes it" rule would end the editor on the first drag).
                // ★★★ 140차: a host notice — the game-style message box — outranks EVERYTHING,
                // the editor included (its HIDE / SHOW answers come this way now) and the
                // pickers it is drawn over. Its band or the HUD's OK / CLR dismisses it;
                // every other press is swallowed, because it is a modal like the game's own.
                // ★ 71차-N (player, 2026-08-27): *"이 confirm팝업에서 ok버튼과 CLR버튼이 작동을
                // 안해. 둘다 confirm에서 OK역할을 해야해."* — the box's own OK band worked, but
                // the arm used to return before the HUD buttons were hit-tested, so the OK
                // and CLR the player's thumbs actually live on did nothing. `layout.buttons`
                // is that test. ⛔ Only a PRESS: a finger already down keeps its own life (its
                // release must reach whatever it pressed).
                // ★★ 143차: the wake's veil owns the screen — every finger is swallowed
                // (a press's own release is dropped with it, `ui_touches`).
                if self.veil.is_some() {
                    if phase == TouchPhase::Started {
                        self.ui_touches.insert(id);
                    }
                    return;
                }
                if phase == TouchPhase::Started && self.notice_up() {
                    let on_band = overlay::notice_hit(self.win_w, self.win_h, x, y) == Some(true);
                    let hud = overlay::hit(&self.layout.buttons, x, y);
                    if on_band || matches!(hud, Some(KeyCode::OK) | Some(KeyCode::CLEAR)) {
                        self.dismiss_notice();
                    }
                    self.ui_touches.insert(id);
                    self.request_redraw();
                    return;
                }
                // ★ 143차: 「하늘성에서 나가시겠습니까?」 — a modal like the message box. A
                // cell: the first tap selects it, a tap on the selected one answers (the
                // game's own 1-tap-focus / 2-tap-take rule, 80차-G); the OK band answers
                // with the selection; the HUD's OK / CLR are OK / 아니오 (71차-N's rule);
                // everything else is swallowed.
                if phase == TouchPhase::Started && self.exit_ask.is_some() {
                    let armed = self.exit_ask.is_some_and(|(_, at)| at.elapsed().as_millis() >= Self::EXIT_ARM_MS);
                    let sel = self.exit_ask.map_or(0, |(s, _)| s);
                    match overlay::confirm_hit(self.win_w, self.win_h, x, y) {
                        Some(overlay::ConfirmHit::Cell(i)) if i == sel => {
                            if armed {
                                self.exit_answer(i == 0);
                            }
                        }
                        Some(overlay::ConfirmHit::Cell(i)) => {
                            self.exit_ask = self.exit_ask.map(|(_, at)| (i, at));
                        }
                        Some(overlay::ConfirmHit::Ok) => {
                            if armed {
                                self.exit_answer(sel == 0);
                            }
                        }
                        Some(overlay::ConfirmHit::Panel) => {}
                        None => match overlay::hit(&self.layout.buttons, x, y) {
                            Some(KeyCode::OK) if armed => self.exit_answer(sel == 0),
                            Some(KeyCode::CLEAR) => self.exit_answer(false),
                            _ => {}
                        },
                    }
                    self.ui_touches.insert(id);
                    self.request_redraw();
                    return;
                }
                // ★ 패드 버튼 설정: modal — every touch is the screen's, and only a touch going
                // DOWN means anything (nothing on it moves or scrolls). ⛔ Above `note_input`:
                // picking (touch) and assigning (pad) alternate here, so the AUTO HUD must not
                // flip — and swallow a tap — under them. The tap that closes the screen is
                // recorded instead (`padmap_save` · `padmap_cancel`), or the next tap on the
                // game would be eaten (S9, 2026-10-09).
                if self.padmap.is_some() {
                    if phase == TouchPhase::Started {
                        self.padmap_started(x, y);
                    }
                    return;
                }
                if self.editor.is_some() {
                    match phase {
                        TouchPhase::Started => self.editor_started(id, x, y),
                        TouchPhase::Moved => self.editor_moved(id, x, y),
                        TouchPhase::Ended | TouchPhase::Cancelled => self.editor_ended(id),
                    }
                    return;
                }
                if phase == TouchPhase::Started && self.note_input(false) {
                    // ★ AUTO: the last input source wins, so a finger on the glass brings
                    // the touch HUD (arrows, stick) back — and THAT touch does nothing
                    // else. Swallowing it is the point: the controls it would be
                    // hit-tested against did not exist when the finger came down, so
                    // dispatching it could open the map or pick a menu row by
                    // "accident" from the player's side. `ui_touches` is how every other
                    // consumed-by-the-UI touch is marked, so its keyup is dropped too.
                    self.ui_touches.insert(id);
                    return;
                }
                match phase {
                    TouchPhase::Started => {
                        // ★ The cash quantity picker takes the tap before anything
                        // else on screen: it is drawn over the game's own confirm box,
                        // so a tap that lands on it must never also reach the guest.
                        // ★ The warehouse's own picker takes the tap first, for the
                        // same reason the cash one does: it is drawn over the game's
                        // panel, and the client is BLOCKED on the reply it is holding.
                        // (★ 70차's message-box arm that stood here is now the FIRST arm
                        // of the whole Touch event, above the editor — 140차.)
                        // ★★★ 117차-D: the GAME's own dialog shows THROUGH the box window.
                        //   Its 예/아니오 answer by pixels exactly as on any other screen
                        //   (80차-G); the HUD's discs are the game's keys (`qty_key` steps
                        //   aside while the reveal is on); anything else is the dialog's and
                        //   is swallowed — the list under it must not take a row.
                        if self.orb_pick.is_some() && self.orb_reveal {
                            if let Some(hit) = self.confirm_tap_hit(x, y) {
                                self.menu_tap = Some((id, x, y, hit));
                                self.ui_touches.insert(id);
                                return;
                            }
                            if let Some(key) = overlay::hit(&self.layout.buttons, x, y) {
                                self.slide_touches.insert(id);
                                self.update_touch(id, Some(key));
                                return;
                            }
                            self.ui_touches.insert(id);
                            return;
                        }
                        // ★ 96차: a box window's 올리기 / 내리기 count is asked OVER the
                        // list, and while it is up the NUMBER owns every touch — the
                        // picker's own ▲ ▼ OK, the HUD's arrows / OK / CLR through
                        // `qty_key`, and nothing else. A tap that fell through to the list
                        // would pick a row under a question that is still open.
                        if self.orb_pick.is_some() && self.wh_qty.is_some() {
                            if !self.wh_qty_tap(id, x, y)
                                && let Some(key) = overlay::hit(&self.layout.buttons, x, y)
                            {
                                self.qty_key(key);
                            }
                            self.ui_touches.insert(id);
                            self.request_redraw();
                            return;
                        }
                        // ★★ 2026-09-15 — THE TOUCH HUD DRIVES THE DIALOG (user: *"가방창
                        // 열렸을때 화살표 동작해야해"*, and *"드랍박스 떴을떄 화살표나 OK키로
                        // 동작이 안돼 … 드랍박스 자체가 닫히고있어"*). The dialog used to
                        // swallow every touch on the screen, so a tap on the arrows did
                        // nothing — and with a drop box up it was read as "a tap outside
                        // the box", which dismissed it. `handle_key` already knows what
                        // each key means to the list and to the box; this is the same fix
                        // 71차-N made for the message box's OK / CLR.
                        //
                        // ⛔ OUTSIDE THE PANEL ONLY. A plate or a disc that happens to sit
                        // under the window must not steal a tap on a ROW — the list is the
                        // thing the player is aiming at there.
                        if self.orb_pick.is_some()
                            && !overlay::rect_hit(&overlay::orb_rects(self.win_w, self.win_h).panel, x, y)
                            && let Some(key) = overlay::hit(&self.layout.buttons, x, y)
                            && self.qty_key(key)
                        {
                            self.ui_touches.insert(id);
                            return;
                        }
                        // ★ The host list dialog takes the touch before ANYTHING else,
                        // for the same reason the two quantity pickers do: it is drawn
                        // over the game's own inventory panel and a touch falling through
                        // would move the cell cursor underneath it.
                        if self.orb_started(id, x, y) {
                            self.ui_touches.insert(id);
                            self.request_redraw();
                            return;
                        }
                        if self.wh_qty_tap(id, x, y) {
                            return;
                        }
                        if self.cash_qty
                            && let Some(hit) = overlay::qty_hit(self.win_w, self.win_h, x, y)
                        {
                            match hit {
                                overlay::QtyHit::Step(d) => {
                                    let now = i32::from(crate::cash_qty()) + d;
                                    crate::set_cash_qty(now.clamp(1, i32::from(crate::CASH_QTY_MAX)) as u8);
                                    if d != 0 {
                                        self.qty_hold = Some((id, d, std::time::Instant::now() + QTY_HOLD_DELAY));
                                    }
                                }
                                // ★ The dialog's own OK, which the game's window has and
                                // the old bottom bar did not: it answers the 예 that is
                                // sitting underneath. Nothing is latched here — the
                                // WORKER latches on the OK key, before the guest sees it
                                // (`emulator::cash_qty_commit`), and that ordering is the
                                // whole reason the count arrives in time.
                                overlay::QtyHit::Ok => {
                                    let _ = self.input_tx.send(InputEvent::Key {
                                        code: KeyCode::OK,
                                        down: true,
                                    });
                                    let _ = self.input_tx.send(InputEvent::Key {
                                        code: KeyCode::OK,
                                        down: false,
                                    });
                                }
                            }
                            self.ui_touches.insert(id);
                            self.request_redraw();
                            return;
                        }
                        if self.map_open {
                            // Enlarged map is up: ANY tap closes it. The game's map
                            // window closes on any key (guest 0x124360), so one CLEAR
                            // tap dismisses both it and our overlay copy.
                            let _ = self.input_tx.send(InputEvent::Key {
                                code: KeyCode::CLEAR,
                                down: true,
                            });
                            let _ = self.input_tx.send(InputEvent::Key {
                                code: KeyCode::CLEAR,
                                down: false,
                            });
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.show_settings {
                            if let Some(action) = overlay::settings_hit(&self.settings_items, self.win_w, self.win_h, x, y) {
                                self.handle_settings(action);
                            }
                            self.ui_touches.insert(id);
                        } else if overlay::gear_hit(&self.layout.gear, x, y) {
                            self.show_settings = true;
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.minimap_toggle_zone().is_some_and(|r| overlay::rect_hit(&r, x, y)) {
                            // ★ 141차: the MAP toggle's BOX. Tested BEFORE the plates and the
                            // panel's own tap zone so a control the player dragged on top of
                            // it cannot make the minimap unrecoverable. (Its enlarged halo is
                            // tested much later — see the 146차 arm.)
                            self.press_top_tab(id, TopTab::Map);
                        } else if (self.mode == overlay::Mode::Field || self.picking_slot)
                            && placement::rect_of(placement::CtrlId::QuickPage, self.mode, self.win_w, self.win_h)
                                .is_some_and(|r| overlay::rect_hit(&r, x, y))
                        {
                            // The worker owns the flip end to end (it needs guest memory for
                            // the capture and the re-bind), so this only reports the tap.
                            //
                            // ★ Tested BEFORE the plates, and while PICKING too: flipping mid-
                            // registration is what lets the player put a skill on page 2 without
                            // closing the popup. Picking survives the flip on purpose — the
                            // popup is untouched, so the pending 퀵바등록 is still pending.
                            let _ = self.input_tx.send(InputEvent::QuickPageNext);
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.exit_zone().is_some_and(|r| overlay::rect_hit(&r, x, y)) {
                            self.press_top_tab(id, TopTab::Exit);
                        } else if self.quicksave_zone().is_some_and(|r| overlay::rect_hit(&r, x, y)) {
                            self.press_top_tab(id, TopTab::Save);
                        } else if let Some((row, sel)) = self.popup_row_hit(x, y) {
                            // ★★ 79차-C — the game's own action popup (드랍박스), the last
                            // inventory item 66차 left open. It could not be done then
                            // because the popup's screen anchor is not in memory: its
                            // painter takes x/y as arguments and they never leave its
                            // stack frame. They are in the FRAME, though, and the box is
                            // unmistakable there — see `menutouch::popup_box`.
                            //
                            // 66차-B's rule, unchanged: first tap FOCUSES (one byte, no
                            // key), a second tap on the row already focused TAKES it —
                            // and taking it is the confirm path that already exists and
                            // already understands the relabelled 퀵슬롯등록 rows.
                            //
                            // A tap on the popup's own chrome falls in here too (`row`
                            // is `None` there) and is swallowed: the popup is modal, and
                            // letting it through would move the inventory cell cursor
                            // underneath — which is exactly the hazard 66차 avoided by
                            // refusing every tap while a popup is up.
                            match row {
                                Some(r) if i32::from(r) == i32::from(sel) => {
                                    let _ = self.input_tx.send(InputEvent::PopupConfirm);
                                }
                                Some(r) => {
                                    let _ = self.input_tx.send(InputEvent::PopupSelectRow { row: r });
                                }
                                None => {}
                            }
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if let Some(i) = overlay::quick_hit(&self.quick_rects, x, y) {
                            // ★ While the plates are HIGHLIGHTED the tap means "register
                            // here", not "use this" — that is the whole point of the
                            // 퀵바등록 flow. The worker re-validates and lets the GAME do
                            // the binding; see `InputEvent::PopupBindSlot`.
                            if self.picking_slot {
                                let slot = self.quickbar.get(i).map_or(i as u8, |s| s.slot);
                                let _ = self.input_tx.send(InputEvent::PopupBindSlot { slot });
                            } else if let Some(slot) = self.quickbar.get(i).copied() {
                                // ★ Skill cooldown (2026-08-05, host-owned; reworked
                                // 2026-08-07): a skill plate whose cooldown is still
                                // running swallows the tap instead of casting again; the
                                // plate's bottom-up fill shows the recovery. Field only —
                                // with a game window open the same digits navigate the
                                // window, which must never be blocked. Items are exempt
                                // (potions have no cooldown).
                                let is_skill = matches!(slot.kind, SlotKind::Skill);
                                let on_field = self.mode == overlay::Mode::Field;
                                let blocked = is_skill && on_field && self.cooldown_frac(slot.slot).is_some();
                                if !blocked {
                                    if on_field {
                                        // ★ The WORKER sends the hotkey and then watches
                                        // guest state to see whether the cast actually
                                        // started (the game's own gates can swallow it:
                                        // not idle, no SP, out of range). Only a confirmed
                                        // cast comes back as `UserEvent::SkillCast`, and
                                        // THAT is what starts the cooldown — a tap that
                                        // cast nothing costs nothing (2026-08-07:
                                        // "스킬이 실행 됐을때만 쿨타임이 돌아야해").
                                        //
                                        // ★★ 2026-09-04: EVERY field activation goes here,
                                        // items included. A plate is not a guest slot — with
                                        // more plates than the guest has slots, only the
                                        // worker can say which slot this plate is in, or put
                                        // it in one (`quickpage::Pages::activate`). Sending a
                                        // key from here would press whatever else happened to
                                        // be in that slot.
                                        let _ = self.input_tx.send(InputEvent::QuickActivate {
                                            plate: slot.slot,
                                            cast: is_skill,
                                        });
                                    }
                                    // ★★ 79차-B — OFF THE FIELD A PLATE DOES NOTHING, and
                                    // that is the point of removing the digit block.
                                    //
                                    // Until now the plates doubled as a digit pad in a game
                                    // window: plate `i` sent `quickslot_hotkey(i)`, which the
                                    // WINDOW's key handler received and used to page. That was
                                    // the last thing making plate 0..4 different in kind from
                                    // plate 5..6 — the extra plates have no digit, so they
                                    // were dead buttons there and nowhere else.
                                    //
                                    // It could not be fixed by routing through residency
                                    // instead: which digit a plate sends would then depend on
                                    // what is currently seated in it, and menu navigation has
                                    // to be predictable. So the role goes away entirely. The
                                    // digits are still reachable — the game draws «1» «3» «7»
                                    // «9» itself and 66차 made those taps (`menu_tap_hit`),
                                    // and the pull-out `123` keypad carries all twelve.
                                    //
                                    // The touch is still SWALLOWED (below): a plate sitting
                                    // over the game's panel must not pass its tap through to
                                    // `menu_tap` / `tap_anywhere_key` and press something the
                                    // player cannot see.
                                }
                            }
                            self.quick_press = Some((id, i));
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.minimap_tap_zone().is_some_and(|p| overlay::rect_hit(&p, x, y)) {
                            // Minimap panel: a DELIBERATE TAP opens the map (that is
                            // also what makes the guest repaint the buffer we mirror).
                            // Nothing is sent yet — see TouchPhase::Ended.
                            self.mm_tap = Some((id, x, y));
                            self.ui_touches.insert(id);
                        } else if self.stats_tap_zone().is_some_and(|p| overlay::rect_hit(&p, x, y)) {
                            // Lv/HP block: a DELIBERATE TAP opens the main menu
                            // (`CLEAR`). Nothing is sent yet — see TouchPhase::Ended.
                            self.stats_tap = Some((id, x, y));
                            self.ui_touches.insert(id);
                        } else if let Some(key) = overlay::hit(&self.layout.buttons, x, y)
                            && self.qty_key(key)
                        {
                            // A quantity picker owns the direction keys while it is up.
                            self.ui_touches.insert(id);
                        } else if let Some(key) = overlay::hit(&self.layout.buttons, x, y) {
                            // ★ The TOUCH TYPE B "stick before the buttons" special
                            // case that used to sit above this arm is GONE (2026-08-16
                            // second round): B has no stick at all any more — the
                            // field D-pad it walks with is an ordinary button set, so
                            // the plain order below covers every type.
                            self.slide_touches.insert(id);
                            self.update_touch(id, Some(key));
                        } else if let Some(hit) = self.avatar_tap_hit(x, y).or_else(|| self.menu7_tap_hit(x, y)) {
                            // ★ 167차 — the AVATAR page owns the game's panel while it is up:
                            // its rows, its slot strip and the header's «1» «3» are the page's,
                            // and every other point inside the panel is swallowed. After our
                            // own controls (the HUD's OK / CLR / arrows are the page's keys,
                            // and the worker takes them as such), before anything that would
                            // read the panel as EQUIPMENT. Sent on a clean release, like a
                            // menu tap.
                            self.menu_tap = Some((id, x, y, hit));
                            self.ui_touches.insert(id);
                        } else if let Some(hit) = self.confirm_tap_hit(x, y) {
                            // ★★★ 80차-G — the game's own `예 / 아니오` confirm, and it goes
                            // ABOVE the joystick zone on purpose. Under TOUCH TYPE A that
                            // zone is the WHOLE WINDOW, so a confirm the game raises on the
                            // FIELD (boss room entry, 시스템 → 세이브) had its buttons taken
                            // as walk orders before any panel arm ran. `confirm_tap_hit` is
                            // pixels only and answers `None` unless the box is actually on
                            // screen, so the field costs one exact-colour rect test per
                            // touch-down and behaves exactly as before when there is none.
                            //
                            // It also answers `Swallow` for a miss INSIDE the panel while
                            // the box is up: a modal must not let the thumb through to the
                            // map underneath.
                            self.menu_tap = Some((id, x, y, hit));
                            self.ui_touches.insert(id);
                        } else if self.note_started(id, x, y) {
                            // ★ 150차 — the 상세보기 BODY while our overlay is on it: a drag
                            // scrolls it, a clean tap steps a line (see `note_ended`). BEFORE
                            // `menu_tap_hit`, which would hand the point to the worker; after
                            // every control of ours and the game's own confirm, so OK / CLR
                            // (the HUD discs) still reach the game — CLR is how the window
                            // closes. `note_started` claims nothing off the body.
                            self.request_redraw();
                        } else if let Some(tab) = self.top_tab_halo_hit(x, y) {
                            // ★★ 146차 — the top-row tabs' ENLARGED tap area (user 2026-10-01:
                            // *"실제 클릭영역을 대폭 증가시켜줘"*). Deliberately this LATE: every
                            // control above has already had the touch, so the halo only takes
                            // what would otherwise have gone to the joystick zone (the whole
                            // window under touch type A) — it can never steal a plate, the
                            // minimap panel, the stats block or the game's own confirm.
                            self.press_top_tab(id, tab);
                        } else if self.joy.is_none() && overlay::in_joystick_zone(&self.layout, x, y) {
                            self.joy_start(id, x, y);
                        } else if let Some(hit) = self.menu_tap_hit(x, y) {
                            // ★ 66차 메뉴 터치화: a MENU-mode touch that hit none of our
                            // controls and landed inside the game's own panel. Nothing is
                            // sent yet: see TouchPhase::Ended.
                            //
                            // ★★ ABOVE `tap_anywhere_key` since 79차-E's fix, and ONLY that
                            // ordering makes the 대화 선택지 arm reachable: the script window
                            // is `Mode::Cutscene`, so the "whole screen is OK" arm used to
                            // eat every touch during a choice and confirm the highlighted
                            // row on the first tap. It costs the cutscene nothing —
                            // `menu_tap_ok` still refuses plain narration (no `choice`
                            // flag), `menu_tap_hit` returns `None` outside the game's own
                            // 176x160 panel, and either way the touch falls through to the
                            // arm below exactly as before.
                            self.menu_tap = Some((id, x, y, hit));
                            self.ui_touches.insert(id);
                        } else if let Some(key) = overlay::tap_anywhere_key(self.mode) {
                            // ★ CUTSCENE: the whole screen is OK (2026-08-17). Every
                            // control above it, the `#` skip included, keeps its own
                            // meaning, and what is left is the scene itself.
                            //
                            // NOT added to `slide_touches`: that set exists so a finger can
                            // slide between D-pad arms, and re-running the button hit-test
                            // on every move would release this OK the moment the finger
                            // drifted. Held until the touch ends, like a button press.
                            self.update_touch(id, Some(key));
                        }
                    }
                    TouchPhase::Moved => {
                        // ★ A finger on the list dialog owns this touch until it lifts.
                        if self.orb_drag.is_some_and(|(did, ..)| did == id) {
                            self.orb_moved(y);
                            return;
                        }
                        // ★ 150차: …and so does a finger on the 상세보기 body.
                        if self.note_drag.is_some_and(|(did, ..)| did == id) {
                            self.note_moved(y);
                            return;
                        }
                        if self.joy.as_ref().is_some_and(|j| j.id == id) {
                            self.joy_update(x, y, true);
                            self.request_redraw();
                        } else if self.mm_tap.is_some_and(|(tid, dx, dy)| tid == id && !is_tap(dx, dy, x, y)) {
                            // Smeared into a drag: not a tap, so it will not open the map.
                            self.mm_tap = None;
                        } else if self.stats_tap.is_some_and(|(tid, dx, dy)| tid == id && !is_tap(dx, dy, x, y)) {
                            self.stats_tap = None;
                        } else if self.menu_tap.is_some_and(|(tid, dx, dy, _)| tid == id && !is_tap(dx, dy, x, y)) {
                            // Smeared into a drag: not a menu tap.
                            self.menu_tap = None;
                        } else if self.slide_touches.contains(&id) {
                            // Sliding between adjacent keys is legitimate (the D-pad
                            // depends on it) — but ONLY for a touch that began on a
                            // control. A stray finger can no longer pick up keys.
                            let key = overlay::hit(&self.layout.buttons, x, y);
                            self.update_touch(id, key);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if self.orb_drag.is_some_and(|(did, ..)| did == id) {
                            self.orb_ended(phase == TouchPhase::Ended, x, y);
                            self.ui_touches.remove(&id);
                            self.request_redraw();
                            return;
                        }
                        if self.note_drag.is_some_and(|(did, ..)| did == id) {
                            self.note_ended(phase == TouchPhase::Ended);
                            self.ui_touches.remove(&id);
                            return;
                        }
                        self.slide_touches.remove(&id);
                        self.qty_slid.remove(&id);
                        // ★ 70차: lifting the finger that held a quantity ▲/▼ stops its
                        // auto-repeat, whatever else this touch was doing.
                        if self.qty_hold.is_some_and(|(hid, _, _)| hid == id) {
                            self.qty_hold = None;
                        }
                        // A real tap that both started AND ended inside the panel HIDES the
                        // minimap (player request 2026-08-04: "미니맵을 클릭하면 미니맵이
                        // 사라지면서 미니맵 버튼으로 나타나게"). It used to send `*` to open
                        // the guest's own map window, which is the feature that got
                        // removed along with the centred blow-up.
                        //
                        // The tap discipline is kept exactly as it was, and it matters more
                        // now: the panel is large, so without "started and ended inside,
                        // and not a drag" a thumb sliding across the corner would make the
                        // minimap vanish mid-fight. `Cancelled` still never counts.
                        if let Some((tid, dx, dy)) = self.mm_tap
                            && tid == id
                        {
                            self.mm_tap = None;
                            if phase == TouchPhase::Ended
                                && is_tap(dx, dy, x, y)
                                && self.minimap_tap_zone().is_some_and(|p| overlay::rect_hit(&p, x, y))
                            {
                                self.minimap_on = false;
                                self.request_redraw();
                            }
                        }
                        // Lv/HP block -> main menu. `CLEAR` is the game's own
                        // open-menu key on the field (device-probed, CUSTOM_HUD.md),
                        // so this is the CLR button's key with a bigger target.
                        if let Some((tid, dx, dy)) = self.stats_tap
                            && tid == id
                        {
                            self.stats_tap = None;
                            if phase == TouchPhase::Ended
                                && is_tap(dx, dy, x, y)
                                && self.stats_tap_zone().is_some_and(|p| overlay::rect_hit(&p, x, y))
                            {
                                let _ = self.input_tx.send(InputEvent::Key {
                                    code: KeyCode::CLEAR,
                                    down: true,
                                });
                                let _ = self.input_tx.send(InputEvent::Key {
                                    code: KeyCode::CLEAR,
                                    down: false,
                                });
                            }
                        }
                        // 66차 메뉴 터치화: fire on a clean tap end, and only when the
                        // release still means the same thing the press did — a finger
                        // that slid onto a different badge, or off the panel, or onto a
                        // badge that has since been repainted away, must do nothing
                        // rather than something else. (`Cancelled` never counts, as
                        // everywhere.) ★ "The same thing", not the same PIXEL: see
                        // `MenuHit::same_target`. The coordinates sent are the PRESS
                        // ones, which is what the player aimed at.
                        if let Some((tid, dx, dy, hit)) = self.menu_tap
                            && tid == id
                        {
                            self.menu_tap = None;
                            if phase == TouchPhase::Ended && is_tap(dx, dy, x, y) && self.tap_target(x, y).is_some_and(|now| hit.same_target(now)) {
                                match hit {
                                    // The key the GAME itself printed on that badge (or on
                                    // its `OK : …` prompt, or on a spinner triangle).
                                    //
                                    // ★ An OK while the game's rewritten context popup is
                                    // up belongs to the popup, exactly as it does for the
                                    // OK BUTTON: only the worker can say what the
                                    // highlighted row means. Same diversion, same reason
                                    // (see `popup_confirm_touches`); no keyup follows,
                                    // because the worker synthesises its own pair.
                                    MenuHit::Badge(code) => {
                                        if code == KeyCode::OK && self.popup.is_some() {
                                            let _ = self.input_tx.send(InputEvent::PopupConfirm);
                                        } else {
                                            let _ = self.input_tx.send(InputEvent::Key { code, down: true });
                                            let _ = self.input_tx.send(InputEvent::Key { code, down: false });
                                        }
                                    }
                                    // Walk the game's own list cursor. Each step is a
                                    // complete press/release pair — the guest drains its
                                    // event queue in order, so N pairs move N rows.
                                    MenuHit::Steps(code, n) => {
                                        for _ in 0..n {
                                            let _ = self.input_tx.send(InputEvent::Key { code, down: true });
                                            let _ = self.input_tx.send(InputEvent::Key { code, down: false });
                                        }
                                    }
                                    // The worker re-reads which screen this is, maps the
                                    // point to a row or a cell, writes the game's own
                                    // cursor byte and synthesises OK.
                                    MenuHit::Panel(gx, gy, item, status, equip, cell_cursor) => {
                                        let _ = self.input_tx.send(InputEvent::MenuTap {
                                            x: gx,
                                            y: gy,
                                            item,
                                            status,
                                            equip,
                                            cell_cursor,
                                        });
                                    }
                                    // The slot list's ▲/▼ (130차): the worker slides the window.
                                    MenuHit::SlotScroll(delta) => {
                                        let _ = self.input_tx.send(InputEvent::SlotScroll { delta });
                                    }
                                    // 「정렬」 (136차): the worker reorders the bag or the store.
                                    MenuHit::SortItems => {
                                        let _ = self.input_tx.send(InputEvent::SortItems);
                                    }
                                    // ★ 167차: the AVATAR page's own action.
                                    MenuHit::Avatar(act) => {
                                        let _ = self.input_tx.send(InputEvent::Avatar(act));
                                    }
                                    // ★ 168차: a row of the seven-row MENU list.
                                    MenuHit::MenuRow(row) => {
                                        let _ = self.input_tx.send(InputEvent::MenuRow(row));
                                    }
                                    // Claimed and silent — see `MenuHit::Swallow`.
                                    MenuHit::Swallow => {}
                                }
                            }
                        }
                        let was_quick = self.quick_press.map(|(pid, _)| pid) == Some(id);
                        if was_quick {
                            self.quick_press = None;
                        }
                        if self.joy.as_ref().is_some_and(|j| j.id == id) {
                            // ★ 66차: the touch-type-A half of 61차's 탭-투-무브 lived
                            // here — a short joystick press that never latched a
                            // direction was treated as a tap on the world and walked
                            // there. It is GONE with the rest of the feature: on the
                            // FIELD the whole window is the stick zone, so that arm made
                            // every imprecise thumb-down a walk order, which is exactly
                            // what the player reported ("퀵슬롯 … 버튼 사이를 누르면 그
                            // 위치로 이동해서 컨트롤에 방해가 되는 경우"). Releasing the
                            // stick is all that is left.
                            self.joy_end();
                            self.request_redraw();
                        } else if !self.ui_touches.remove(&id) {
                            self.update_touch(id, None);
                        } else if was_quick {
                            self.request_redraw();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
