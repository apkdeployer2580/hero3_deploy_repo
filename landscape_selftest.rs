// Standalone self-test of the landscape-immersive geometry (the wie_android test
// binary segfaults at startup on this Windows host because of the linked
// winit/android deps — same workaround as items_selftest.rs).
//
//   rustc --edition 2021 --test landscape_selftest.rs -o t2.exe && ./t2.exe
//
// Mirrors `lib.rs::init_logical` and `overlay.rs::layout_immersive` /
// `quick_buttons` so the arithmetic that decides "does the game fill the screen"
// is regression-tested without a device.

pub const CONTENT_W: u32 = 240;
pub const CONTENT_H: u32 = 320;
pub const LOGICAL_W_MIN: u32 = 240;
pub const LOGICAL_W_MAX: u32 = 1024;
/// Mirrors `lib.rs::POPUP_SAFE_LOGICAL_W`: above this the game's own popup
/// renderer truncates its interior origin to 8 bits and the whole list wraps to
/// the left edge of the screen. See `popup_interior_x` below for the derivation.
pub const POPUP_SAFE_LOGICAL_W: u32 = 496;
/// Mirrors `config.rs::BASE_H_CHOICES`.
pub const BASE_H_CHOICES: [u32; 4] = [160, 190, 220, 240];

/// Mirrors `items.rs::TARGET_BOX_SHIFT_MAX`: the game's target-monster box is moved
/// left by a one-byte immediate at a `subs r1,#44` site, so 255 - 44 is as far as it
/// can go. (This mirror said 160 before 2026-07-31 — harmless while nothing came
/// near it, but the minimap size knob is capped by INVERTING this number, so the two
/// have to agree.)
pub const TARGET_BOX_SHIFT_MAX: u32 = 211;

/// `landscape_pref` = the PERSISTED orientation choice, not `win_w > win_h`:
/// device-confirmed race — the first `resumed` can report the pre-rotation
/// portrait size, which silently dropped immersive mode on alternating launches.
pub fn init_logical(win_w: u32, win_h: u32, immersive: bool, landscape_pref: bool, base_h: u32) -> (u32, u32) {
    let (long, short) = (win_w.max(win_h), win_w.min(win_h));
    if immersive && landscape_pref && short > 0 {
        let base = base_h.max(120);
        let w = ((u64::from(base) * u64::from(long) / u64::from(short)) as u32)
            .clamp(LOGICAL_W_MIN, LOGICAL_W_MAX.min(POPUP_SAFE_LOGICAL_W))
            & !1;
        let h = ((u64::from(w) * u64::from(short) / u64::from(long)) as u32).max(120);
        (w, h)
    } else {
        (CONTENT_W, CONTENT_H)
    }
}

/// The x the game's popup renderer will actually use for the popup's INTERIOR,
/// given a logical width. Mirrors the verified guest arithmetic:
///   `win_x = (screen_w - 176) >> 1`                            (guest 0x12d85e)
///   `cell_x = win_x + col*(42+5) + 34`                         (guest 0x134f28, skill grid)
///   `popup_x = cell_x + 43`; if `popup_x > screen_w/2` then `cell_x - 76`
///                            else `popup_x + 1`                (guest 0x1361a2)
///   `interior = (u8)(popup_x + 6)`                             (guest 0x12e47c, `strb`)
/// The truncation is the bug: above 255 the interior wraps to the left edge.
pub fn popup_interior_x(screen_w: u32, col: u32) -> u32 {
    let win_x = (screen_w - 176) >> 1;
    let cell_x = win_x + col * 47 + 34;
    let pre = cell_x + 43;
    let popup_x = if pre > screen_w / 2 { cell_x - 76 } else { pre + 1 };
    popup_x + 6
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    /// Half-open point test, mirroring `overlay::Rect::contains` (which truncates to
    /// integers first, so a zero-sized rect can never be hit).
    fn contains(&self, px: f64, py: f64) -> bool {
        let (xi, yi) = (px as i64, py as i64);
        xi >= self.x as i64 && xi < (self.x + self.w) as i64 && yi >= self.y as i64 && yi < (self.y + self.h) as i64
    }
    fn inside(&self, o: &Rect) -> bool {
        self.x >= o.x && self.y >= o.y && self.x + self.w <= o.x + o.w && self.y + self.h <= o.y + o.h
    }
}

pub fn pad_of(win_w: u32, win_h: u32) -> u32 {
    (win_w.min(win_h) / 110).max(3)
}

/// Mirrors `overlay.rs`: action disc, skill cascade, CLR key, minimap panel,
/// MENU/SEL keys and the joystick home.
/// Cluster centre — FIXED (nominal radius), so shrinking the disc does not drag
/// the ring into the corner.
pub fn action_centre(win_w: u32, win_h: u32, pad: u32) -> (u32, u32) {
    let nominal = (win_h / 6).clamp(56, 190);
    (win_w.saturating_sub(pad * 3 + nominal), win_h.saturating_sub(pad * 3 + nominal))
}

pub fn action_disc(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let (cx, cy) = action_centre(win_w, win_h, pad);
    let r = (win_h / 8).clamp(48, 140);
    Rect::new(cx.saturating_sub(r), cy.saturating_sub(r), r * 2, r * 2)
}

/// The stick's pre-91차-B radius — mirrors `overlay::joy_r_legacy`. No longer drawn;
/// it survives as the WALK-START basis ([`stick_engage_radius`]).
pub fn joy_r_legacy(win_h: u32) -> f64 {
    f64::from((win_h / 12).clamp(34, 96))
}

/// The deadzone basis for a resolved STICK seat — mirrors `overlay::stick_engage_radius`
/// (91차-B): the legacy radius scaled by the seat's size percent, so a 100 % seat starts
/// walking after the same 27 px it always did while the BASE grew to the whole square.
pub fn stick_engage_radius(seat: &Rect, win_w: u32, win_h: u32) -> f64 {
    let b = stick_seat(win_w, win_h);
    let base = f64::from(b.w.min(b.h).max(1));
    joy_r_legacy(win_h) * f64::from(seat.w.min(seat.h)) / base
}

/// The thumb disc's radius as a fraction of the base's — mirrors `overlay::JOY_THUMB`.
pub const JOY_THUMB: f64 = 0.44;

/// Mirrors `overlay::stick_base_radius` (2026-09-20): the stick's base disc is the circle
/// through the four arrow hexagons' CENTRES, not the seat's inscribed circle. Player:
/// "화살표 크기보다 조이스틱이 조금 더 작아야해. 화살 육각형들의 각 중심점을 원으로 연결한게
/// 조이스틱크기". Derived from `ARM_LEN_PCT` and `dpad_arms`' own `inset`, so the disc and
/// the arrows cannot drift apart.
pub fn stick_base_radius(seat: &Rect) -> f64 {
    let s = seat.w.min(seat.h);
    let inset = f64::from((s / 50).max(1));
    let half_len = f64::from((s * ARM_LEN_PCT / 100).max(3)) / 2.0;
    let v = f64::from(seat.h) / 2.0 - inset - half_len;
    let h = f64::from(seat.w) / 2.0 - inset - half_len;
    v.min(h).max(1.0)
}

/// BUILT-IN stick home: the built-in STICK seat's centre and the circle through its four
/// hexagons' centres ([`stick_base_radius`]). Everything that feeds the placement resolver
/// must use THIS, never [`joy_home`] — it is reachable from `obstacles`, so going through
/// the placed position would recurse.
pub fn joy_home_builtin(win_w: u32, win_h: u32) -> (f64, f64, f64) {
    let s = stick_seat(win_w, win_h);
    (
        f64::from(s.x) + f64::from(s.w) / 2.0,
        f64::from(s.y) + f64::from(s.h) / 2.0,
        stick_base_radius(&s),
    )
}

/// The stick home the app actually draws: the RESOLVED seat's centre and base circle, so
/// the base follows the player's size as well as their position. Mirrors
/// `overlay::joy_home`, which reads the live placement table; here that table is the
/// `LIVE` thread-local (see [`t_stick`]).
pub fn joy_home(win_w: u32, win_h: u32) -> (f64, f64, f64) {
    let seat = t_stick(win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h));
    (
        f64::from(seat.x) + f64::from(seat.w) / 2.0,
        f64::from(seat.y) + f64::from(seat.h) / 2.0,
        stick_base_radius(&seat),
    )
}

/// Mirrors `lib.rs::StickMode` — the 조작 방식, three-way since 2026-09-22 ("자유스틱,
/// 십자키, 고정스틱 3가지로 바꿀꺼야 … 고정스틱은 자유스틱이랑 똑같이 조이스틱인데
/// 십자키처럼 위치가 고정된 조이스틱인거야").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StickMode {
    /// 자유 스틱 — touch anywhere on the field; the base spawns under the finger.
    Float,
    /// 십자키 — four pinned arrows; no stick at all.
    Dpad,
    /// 고정 스틱 — a stick whose base never leaves its seat.
    Pinned,
}

impl StickMode {
    pub fn field_arrows(self) -> bool {
        self == Self::Dpad
    }
    pub fn stick_on_field(self) -> bool {
        self != Self::Dpad
    }
    pub fn stick_is_pinned(self) -> bool {
        self == Self::Pinned
    }
    /// The settings row's cycle, in the order the player named the three.
    pub fn next(self) -> Self {
        match self {
            Self::Float => Self::Dpad,
            Self::Dpad => Self::Pinned,
            Self::Pinned => Self::Float,
        }
    }
}

pub const STICK_MODES: [StickMode; 3] = [StickMode::Float, StickMode::Dpad, StickMode::Pinned];

/// The joystick zone `layout_immersive` publishes **on the FIELD** — per mode, mirrors
/// `overlay::stick_zone_on_field` — and NONE anywhere else.
///
/// * 자유 스틱: the whole window ("touch anywhere to walk").
/// * 십자키 has no stick at all — the 2026-08-16 second round replaced its pinned
///   stick with the field D-pad ("조이스틱은 터치타입 B에서 히든처리 되어야해").
/// * 고정 스틱 (2026-09-22): the RESOLVED stick seat — the square the base is drawn
///   in and never leaves; a touch anywhere else on the field is nothing, exactly as
///   under 십자키. The mirror resolves the seat through `t_stick` (the `LIVE` table),
///   as the app resolves it through `placement::rect_of`.
/// * Off-field (menu / cutscene) every mode publishes nothing: the zone carries the
///   idle HOME HINT with it, and 2026-08-17 the player asked for that placeholder to
///   go ("메뉴 열렸을때 좌측하단에 조이스틱 placeholder 없애줘"). Walking does not
///   exist there and the menu D-pad is what steps a cursor.
pub fn stick_zone(win_w: u32, win_h: u32, mode: StickMode, scene: Scene) -> Option<Rect> {
    if scene != Scene::Field {
        return None;
    }
    match mode {
        StickMode::Float => Some(Rect::new(0, 0, win_w, win_h)),
        StickMode::Dpad => None,
        StickMode::Pinned => Some(t_stick(win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h))),
    }
}

/// BUILT-IN bounding square of the ONE walk control — the stick's home (touch type A)
/// AND the four arrows (type B on the field; every type with a window open). Bottom-
/// left, inset `pad * 4` from both edges. Mirrors `overlay::stick_seat` (91차).
///
/// ★ `2 x` the old three-cell key column since 91차 (was 7/4 since 2026-08-17, 11/8
/// since 2026-08-11, 5/4 since 2026-08-09): "누르기가 좀 어렵다는 말이 있어서 버튼
/// 크기를 좀 늘리는게". The left inset went from `win_w / 26` to `pad * 4`, which is
/// what keeps the grown square clear of the game's own menu panel on an 18:9 window.
///
/// ★★ 80차-F: BOTTOM-ANCHORED. 79차-B removed the 1/3/7/9/# key block that used to sit
/// above it, and the square was then "centred on its own", which walked it into the
/// vertical middle of the screen. The player asked for the seat back (2026-09-05:
/// "좌측에 화살표 나오는것도 원래 좌측하단쪽에 위치했엇는데 … 갑자기 좌측 중앙정렬로
/// 바뀌었어"). ⛔ Do not give the menu arrows an anchor of their own again — two
/// anchors means the arrows jump when a window opens.
pub fn stick_seat(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let cell = (win_h / 11).clamp(40, 120);
    let span = (cell * 3 + pad * 2) * 2;
    let span = span.min(win_h.saturating_sub(pad * 4).max(1)).min(win_w.max(1));
    let x = (pad * 4).min(win_w.saturating_sub(span));
    let y = win_h.saturating_sub(span + pad * 4);
    Rect::new(x, y, span, span)
}

/// The three SCENES the touch policy distinguishes. ★ Separate from [`Mode`] on
/// purpose: `Mode` here is the GEOMETRY key (and a cutscene stores no seats of its
/// own — it aliases `Menu`), while the question below is exactly "is this a
/// cutscene", so it needs the third value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Field,
    Menu,
    Cutscene,
}

/// What a cutscene touch can produce. (`Key` further down is the joystick's four
/// directions — a different alphabet, hence a second enum rather than a merge.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapKey {
    Ok,
    Hash,
    /// One of the four D-pad arms — which one does not matter to the policy, only
    /// that the arm answered instead of the screen-wide OK.
    Arrow,
}

/// What a touch that hit NO control sends — mirrors `overlay::tap_anywhere_key`
/// (player 2026-08-17: "컷신일때는 #을 제외한 어디를 눌러도 대화가 넘어가게 …
/// 이 정책은 컷신일때만 작동해야 해").
pub fn tap_anywhere_key(scene: Scene) -> Option<TapKey> {
    (scene == Scene::Cutscene).then_some(TapKey::Ok)
}

/// One arrow: where the hexagon is inked (its bounding box), and where a touch
/// counts. Mirrors `overlay::Arm`.
#[derive(Clone, Copy, Debug)]
pub struct Arm {
    pub draw: Rect,
    pub hit: Wedge,
    /// Which axis the hexagon's tips lie on: `true` = vertical (up / down).
    pub vertical: bool,
}

/// A quarter of the D-pad square cut by both diagonals — one arrow's touch target.
/// Mirrors `overlay::Wedge` (91차-B): a point belongs to the quarter whose axis it is
/// nearest to, ties on a diagonal go to the vertical pair, and a dead disc at the
/// centre belongs to nobody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wedge {
    pub square: Rect,
    /// 0 up, 1 down, 2 left, 3 right — the push order.
    pub dir: u8,
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
            0 => -dy >= dx.abs(),
            1 => dy >= dx.abs(),
            2 => -dx > dy.abs(),
            _ => dx > dy.abs(),
        }
    }
}

/// Mirrors `overlay::{ARM_LEN_PCT, ARM_WIDTH_PCT, HEX_TIP_PCT, DEAD_PCT}` (91차-B): the
/// hexagon's length along its axis and width across it as percents of the side, each
/// tip's share of the length, and the dead disc's radius as a percent of the side.
pub const ARM_LEN_PCT: u32 = 34;
pub const ARM_WIDTH_PCT: u32 = 44;
pub const HEX_TIP_PCT: u32 = 30;
pub const DEAD_PCT: u32 = 8;

/// The four D-pad arms derived from the control's bounding square — mirrors
/// `overlay::dpad_arms` (91차-B geometry).
///
/// INK: each arm is a FAT hexagon, `ARM_WIDTH_PCT` of the side across its axis and
/// `ARM_LEN_PCT` along it, its outer tip `inset` from the rim, all four congruent by
/// rotation. HIT: the square cut by both diagonals into four [`Wedge`]s plus a dead
/// disc — every pixel belongs to exactly one arrow or to the disc, the quarters share
/// no pixel (the "겹치는건 안돼" requirement, by construction), and each hexagon lies
/// inside its own quarter.
pub fn dpad_arms(r: Rect) -> [Arm; 4] {
    let s = r.w.min(r.h);
    let inset = (s / 50).max(1);
    let len = (s * ARM_LEN_PCT / 100).max(3);
    let wid = (s * ARM_WIDTH_PCT / 100).max(3);
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let hit = |dir| Wedge { square: r, dir };
    [
        Arm { draw: Rect::new(cx.saturating_sub(wid / 2), r.y + inset, wid, len), hit: hit(0), vertical: true },
        Arm {
            draw: Rect::new(cx.saturating_sub(wid / 2), (r.y + r.h).saturating_sub(inset + len), wid, len),
            hit: hit(1),
            vertical: true,
        },
        Arm { draw: Rect::new(r.x + inset, cy.saturating_sub(wid / 2), len, wid), hit: hit(2), vertical: false },
        Arm {
            draw: Rect::new((r.x + r.w).saturating_sub(inset + len), cy.saturating_sub(wid / 2), len, wid),
            hit: hit(3),
            vertical: false,
        },
    ]
}

/// The six corners of an arm's hexagon, clockwise from the OUTWARD tip — mirrors
/// `overlay::hexagon`. Each tip is `HEX_TIP_PCT` of the box's length on the pointing
/// axis; the two flat sides are what is left between them.
pub fn hexagon(r: Rect, vertical: bool) -> [(f64, f64); 6] {
    let (x0, y0) = (f64::from(r.x), f64::from(r.y));
    let (x1, y1) = (x0 + f64::from(r.w), y0 + f64::from(r.h));
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let tip = f64::from(HEX_TIP_PCT) / 100.0;
    if vertical {
        let t = (y1 - y0) * tip;
        [(cx, y0), (x1, y0 + t), (x1, y1 - t), (cx, y1), (x0, y1 - t), (x0, y0 + t)]
    } else {
        let t = (x1 - x0) * tip;
        [(x1, cy), (x1 - t, y1), (x0 + t, y1), (x0, cy), (x0 + t, y0), (x1 - t, y0)]
    }
}

/// Whether the script window shows the D-pad — mirrors `overlay::layout_immersive`'s
/// arrow gate for `Mode::Cutscene`. ★ 2026-08-17, the player's rule: "컷신일때는
/// 안나오고 인게임 내에서 대화에 선택지가 있을때는 나오고". `choice` is
/// `items::script_choice_open` (`CHAR+0xa0dc`), the byte that splits the two things
/// window id 4 conflates.
pub fn cutscene_shows_dpad(choice: bool) -> bool {
    choice
}

/// The whole cutscene touch chain, mirroring `App`'s `TouchPhase::Started` arms for
/// `Mode::Cutscene`: the BUTTONS answer first — the D-pad arms (only while a CHOICE is
/// up) and the game's own `# SKIP` corner target — and only what no control claims
/// falls through to the screen-wide OK.
///
/// ★ 64차: the `#` skip disc that used to sit on the CANCEL seat is GONE (player:
/// "우측하단 '#skip'을 눌러서 skip하는게 가능해져서 따로 기존 skip버튼(기존 취소 or
/// #버튼 위치)는 필요없을것 같아"), so this mirror no longer needs the `Table` — the
/// CANCEL-seat lookup was its only use. The REAL chain still layout-resolves the
/// choice arrows (they sit on the MENU D-pad seat); here they arrive pre-resolved
/// in `dpad`, and the corner target is corner-anchored.
///
/// `dpad` is the resolved bounding square of the arrows. ★ It is a PARAMETER rather
/// than a `resolve` lookup because this mirror's [`CtrlId`] still has no `Dpad` (the
/// drift CLAUDE.md warns about); the ORDER is what these tests are for.
pub fn cutscene_tap(win_w: u32, win_h: u32, x: f64, y: f64, dpad: Rect, choice: bool) -> Option<TapKey> {
    // Immersive derives the logical framebuffer from the window's own aspect; the
    // device measures 496x228 at 2340x1080. One helper so every call agrees.
    let (lw, lh) = logical_for(win_w, win_h);
    if cutscene_shows_dpad(choice) && dpad_arms(dpad).iter().any(|a| a.hit.contains(x, y)) {
        return Some(TapKey::Arrow);
    }
    // ★ 63차: the GAME'S own `# SKIP` corner is a target, and it must beat the
    // screen-wide OK — before this, tapping the word "SKIP" advanced one page.
    if game_skip_hint_at(Rect::new(0, 0, win_w, win_h), lw, lh).contains(x, y) {
        return Some(TapKey::Hash);
    }
    tap_anywhere_key(Scene::Cutscene)
}

/// Mirror of `overlay::game_skip_hint_at` — the GAME'S OWN `# SKIP` lettering in its
/// bottom-right corner, as a touch target (63차).
///
/// ★ The three logical-px constants are a PROBE MEASUREMENT, not a derivation, which is
/// what makes mirroring worthwhile here: `wie_probe` in the opening cutscene at four
/// logical sizes put the ink at
///
/// ```text
///   496x240 -> x 462..493  y 229..237     560x240 -> x 526..557  y 229..237
///   496x228 -> x 462..493  y 217..225     440x220 -> x 406..437  y 209..217
/// ```
///
/// i.e. 32 x 9 with a 2 px right/bottom margin, at every size. If the shipped formula
/// drifts from those numbers the tests below stop matching REALITY, not a copy of the
/// code.
pub const SKIP_HINT_W: f64 = 32.0;
pub const SKIP_HINT_H: f64 = 9.0;
pub const SKIP_HINT_MARGIN: f64 = 2.0;

pub fn game_skip_hint_at(game: Rect, lw: u32, lh: u32) -> Rect {
    if game.w == 0 || game.h == 0 || lw == 0 || lh == 0 {
        return Rect::new(0, 0, 0, 0);
    }
    let ink = game_skip_ink_at(game, lw, lh);
    let want_w = (game.x + game.w).saturating_sub(ink.x);
    let want_h = (game.y + game.h).saturating_sub(ink.y);
    let floor = (game.w.min(game.h) / 9).max(24);
    let w = want_w.max(floor).min(game.w);
    let h = want_h.max(floor).min(game.h);
    Rect::new((game.x + game.w).saturating_sub(w), (game.y + game.h).saturating_sub(h), w, h)
}

/// Where the game's own ink actually sits, in WINDOW px — the thing the target has to
/// cover. Derived straight from the probe constants, deliberately NOT from
/// [`game_skip_hint_at`], so a bug in the target maths cannot hide itself.
pub fn game_skip_ink_at(game: Rect, lw: u32, lh: u32) -> Rect {
    let sx = f64::from(game.w) / f64::from(lw);
    let sy = f64::from(game.h) / f64::from(lh);
    let x = f64::from(game.x) + (f64::from(lw) - SKIP_HINT_MARGIN - SKIP_HINT_W) * sx;
    let y = f64::from(game.y) + (f64::from(lh) - SKIP_HINT_MARGIN - SKIP_HINT_H) * sy;
    Rect::new(x as u32, y as u32, (SKIP_HINT_W * sx).ceil() as u32, (SKIP_HINT_H * sy).ceil() as u32)
}

/// The logical framebuffer immersive gives this window, via the [`init_logical`]
/// mirror already at the top of this file. Landscape immersive is the shipped default
/// (CLAUDE.md: the app is finished as landscape), so both flags are true and the base
/// is the shipped 240.
///
/// Checked against the device: 2340x1080 -> 240*2340/1080 = 520, clamped to
/// `POPUP_SAFE_LOGICAL_W` 496, then 496*1080/2340 = **228** — exactly the
/// `guest framebuffer 496x228` the vc132 boot log prints.
pub fn logical_for(win_w: u32, win_h: u32) -> (u32, u32) {
    init_logical(win_w, win_h, true, true, 240)
}

pub const RING_A0: f64 = 66.0;
pub const RING_A1: f64 = 195.0;

/// Mirrors `overlay::RING_STEP_MAX`: the six-seat ring's pitch, the widest the ring
/// will use once plates can be hidden (2026-09-10).
pub const RING_STEP_MAX: f64 = (RING_A1 - RING_A0) / 5.0;

/// Lay the ring out at exactly `side`; `None` if any seat would leave the window.
/// Test-side twin of the closure inside [`ring_slots`] — it exists so a test can ask
/// "would one pixel more have fitted?".
pub fn ring_slots_at(win_w: u32, win_h: u32, n: usize, pad: u32, side: f64) -> Option<Vec<Rect>> {
    if n == 0 {
        return None;
    }
    let a = action_disc(win_w, win_h, pad);
    let ar = f64::from(a.w) / 2.0;
    let (acx, acy) = {
        let (cx, cy) = action_centre(win_w, win_h, pad);
        (f64::from(cx), f64::from(cy))
    };
    let clear = acx - f64::from(secondary_button(win_w, win_h).x);
    // Mirrors `overlay::ring_slots` (2026-09-10): the pitch is capped at the six-seat
    // ring's, so a short ring ends the arc earlier instead of re-spreading over all of
    // it and throwing its plates at the far end.
    let step = if n > 1 {
        ((RING_A1 - RING_A0) / (n - 1) as f64).min(RING_STEP_MAX)
    } else {
        0.0
    };
    let need = if step > 0.0 { side * 1.485 / (2.0 * (step.to_radians() / 2.0).sin()) } else { 0.0 };
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
}

/// Ring of `n` controls around the action disc (mirrors `overlay::ring_slots`):
/// the plate shrinks until every position is fully on-screen, and the radius keeps
/// ADJACENT squares apart (chord >= side*1.485 — a plain `chord >= side` is not
/// enough for axis-aligned squares on a diagonal offset).
pub fn ring_slots(win_w: u32, win_h: u32, n: usize, pad: u32) -> Vec<Rect> {
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
    let clear = acx - f64::from(secondary_button(win_w, win_h).x);
    // Mirrors `overlay::ring_slots` (2026-09-10): the pitch is capped at the six-seat
    // ring's, so a short ring ends the arc earlier instead of re-spreading over all of
    // it and throwing its plates at the far end.
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
    // ★ Mirrors the 2026-07-31 fix: never hand back an EMPTY ring (that used to
    // degrade CLR to a 1x1 rect at the origin and delete the whole quickbar).
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

/// How many plates the player gets — mirrors `quickpage::PLATES` (7 since 2026-09-04).
/// The guest's bar is still five wide; a plate is bound into one on demand.
pub const PLATES: usize = 7;

/// Seats on the action ring: CLR + one per quickbar plate.
pub const RING_SEATS: usize = 1 + PLATES;

pub fn ring_all(win_w: u32, win_h: u32, pad: u32) -> Vec<Rect> {
    ring_slots(win_w, win_h, RING_SEATS, pad)
}

/// Mirrors `overlay::ring_seats_live` (2026-09-10): the ring lays out CLR plus one seat
/// per VISIBLE plate, so switching a quickslot off in the layout editor gives its seat
/// back and the survivors spread over the same arc and grow.
///
/// Parameterised rather than reading a preference: this mirror has no globals, and what
/// the tests below have to cover is EVERY plate count the player can produce (1..=7),
/// not the one this device happens to be set to.
pub fn ring_all_visible(win_w: u32, win_h: u32, pad: u32, visible: usize) -> Vec<Rect> {
    ring_slots(win_w, win_h, 1 + visible, pad)
}

// The menu-navigation key block (1/3/7/9/#) and its seat function are GONE, with the
// original (79차-B). The game draws those digits itself and 66차 made them taps; the
// pull-out keypad still carries every key. Nothing here mirrors them any more.

/// The Lv/HP gauge block as a TAP TARGET (mirrors `overlay::stats_tap_box`): tapping
/// it sends `CLEAR`, which is the game's own open-main-menu key on the field
/// (device-probed, CUSTOM_HUD.md). Grown by a pad because the drawn bars are thin.
///
/// ★ EXACTLY one pad. The zone is hit-tested before the button list, and the only
/// thing keeping a control out from under it is the UNGROWN gauge block in
/// `obstacles`, which `accepts` clears at `pad`. One pad puts the zone edge on the
/// nearest legal control edge (half-open intersection); two reach inside it. See
/// `the_gauge_tap_zone_never_shadows_a_legal_control`.
pub fn stats_tap_box(win_w: u32, win_h: u32) -> Rect {
    let game = Rect::new(0, 0, win_w, win_h);
    grown(gauge_block(game), pad_of(win_w, win_h))
}

/// BUILT-IN seat of potion button `i` (0 = HP, 1 = MP): a two-high COLUMN just left
/// of the ring's lowest plate, same size (mirrors `overlay::potion_button`). Derived
/// from the ring so the sizes cannot drift on the odd aspect ratios where the ring
/// shrinks itself to fit.
///
/// A column and not a row: a row extending left along the bottom edge runs into the
/// stick's home on a narrow screen (selftest-caught at 1080x2340), and the built-ins
/// have to be disjoint at EVERY window size — they are the fallback a placement drops
/// back to, so an overlapping built-in is an overlap with no way out.
pub fn potion_button(win_w: u32, win_h: u32, i: usize) -> Rect {
    let pad = pad_of(win_w, win_h);
    let base = ring_seat(win_w, win_h, pad, RING_SEATS - 1);
    let x = base.x.saturating_sub(base.w + pad * 4);
    let y = base.y.saturating_sub((base.h + pad * 2) * i as u32);
    Rect::new(x, y, base.w, base.h)
}

/// PAGE: below-left of the LOWEST plate, plate-sized — mirrors
/// `overlay::below_left_of_lowest_plate` (player 2026-08-06: "페이지 아이콘이 너무 크고
/// 지금 위치보다 더 내려가야해. 더 아래 스킬버튼 좌측에 존재해야해").
///
/// The anchor is the lowest plate (`max y`) rather than the leftmost (`min x`) it used
/// to be. On the current arc those are the same seat; naming the one we mean is what
/// keeps this right if the arc is ever re-spaced.
///
/// ★ The STICK valve survives from the row layout this replaces, and for the same
/// measured reason (`potion_button`, caught at 1080x2340): a control under the walking
/// thumb steals steering. With no room to the left it goes straight DOWN instead of up,
/// which is the direction that was asked for anyway.
pub fn below_left_of_lowest_plate(win_w: u32, win_h: u32, pad: u32) -> Rect {
    let ring = ring_all(win_w, win_h, pad);
    let base = ring
        .get(1..)
        .unwrap_or_default()
        .iter()
        .copied()
        .max_by_key(|r| r.y)
        .unwrap_or_else(|| ring_seat(win_w, win_h, pad, RING_SEATS - 1));
    // Plate-sized: 2026-08-06 shrank the icon inside it, not the button.
    let side = base.w;
    // ★ 91차: the STICK seat is the arrows' square too (touch type B walks with them
    // on the field), so the whole square is what the page button must stay clear of.
    let seat = stick_seat(win_w, win_h);
    let stick_right = seat.x + seat.w + pad * 2;
    let y = (base.y + base.h / 2).min(win_h.saturating_sub(side));
    let x = base.x.saturating_sub(side + pad * 2);
    if x >= stick_right {
        Rect::new(x, y, side, side)
    } else {
        Rect::new(base.x, (base.y + base.h + pad).min(win_h.saturating_sub(side)), side, side)
    }
}

pub fn quickpage_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    below_left_of_lowest_plate(win_w, win_h, pad)
}

/// SAVE leaves the plate row entirely: immediately LEFT of the box the GAME draws for
/// the targeted monster, top-right (player 2026-08-06). Mirrors
/// `overlay::quicksave_seat`, including its use of the BUILT-IN minimap rect — a
/// built-in seat that consulted the player's resolved placement would make the
/// placement resolver depend on the fallback it is resolving.
pub fn quicksave_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    // From the RING's actual plate, not the `win_h / 11` the ring starts from: the ring
    // shrinks itself until the arc fits, so the formula overshoots on a narrow window.
    let side = ring_seat(win_w, win_h, pad, 1).w;
    let game = Rect::new(0, 0, win_w, win_h);
    let (lw, lh) = init_logical(win_w, win_h, true, true, 240);
    let map = minimap_panel(win_w, win_h);
    let target = game_target_box_at(game, lw, lh, map);
    // Left of whichever reaches further left. The box only dodges the panel in
    // landscape immersive, so in portrait the panel juts out past it.
    let x = target.x.min(map.x).saturating_sub(side + pad * 2).min(win_w.saturating_sub(side));
    // ★ 141차: the same x, but ON THE TOP EDGE as a flat word — a plate wide and 60 % of
    // a plate tall (`overlay::TOP_LABEL_H_PCT`), the way the reference lays its buttons.
    // ★ 2026-09-30: flush with the top edge (y = 0), a tab bordered on its other three sides.
    let h = (side * 60 / 100).max(1);
    Rect::new(x, 0, side, h.min(win_h))
}

pub fn skill_slots(win_w: u32, win_h: u32, n: usize, pad: u32) -> Vec<Rect> {
    ring_all(win_w, win_h, pad).into_iter().skip(1).take(n).collect()
}

pub fn cancel_key(win_w: u32, win_h: u32, pad: u32) -> Rect {
    ring_all(win_w, win_h, pad)[0]
}

/// Mirrors `overlay::cancel_seat` (80차-H): the CLR DISC, which is twice a plate wide and
/// therefore does not fit on ring seat 0 — the ring's rightmost — without being clamped
/// back left onto plate 1.
pub fn cancel_seat(win_w: u32, win_h: u32, pad: u32) -> Rect {
    cancel_seat_visible(win_w, win_h, pad, PLATES)
}

/// [`cancel_seat`] with the number of VISIBLE plates given (2026-09-10). The real one
/// reads the hidden set through `ring_seat`, so its clearance is measured against the
/// first plate that is actually on screen — which grows as plates are switched off.
pub fn cancel_seat_visible(win_w: u32, win_h: u32, pad: u32, visible: usize) -> Rect {
    let ring = ring_all_visible(win_w, win_h, pad, visible);
    let d = secondary_button(win_w, win_h)
        .w
        .max(ring[0].w)
        .min(win_w.max(1))
        .min(win_h.max(1));
    let r = f64::from(d) / 2.0;
    let q = ring[1];
    let (qcx, qcy) = (f64::from(q.x) + f64::from(q.w) / 2.0, f64::from(q.y) + f64::from(q.h) / 2.0);
    let need = r + f64::from(q.w.min(q.h)) / 2.0 + f64::from(pad * 2);
    let cx = (f64::from(win_w) - f64::from(pad * 2) - r).max(r);
    let dx = (cx - qcx).abs();
    let dy = (need * need - dx * dx).max(0.0).sqrt();
    let cy = (qcy + dy).clamp(r, f64::from(win_h) - r);
    Rect::new((cx - r).max(0.0) as u32, (cy - r).max(0.0) as u32, d, d)
}

// ── regions the GAME's own field HUD owns (guest px; RE wf_477f0b16 QA/QB) ───
// TOP-LEFT: active buff/status list, up to 3 slots of 20x25 at x=20*i+1, y=2
//           (absolute, NOT screen-relative) -> reserve (0, 0, 64, 28).
// TOP-RIGHT: target monster name/level/HP box, 85x26 at x = screen_w - 87, y = 2
//           (RIGHT-EDGE anchored) -> reserve (screen_w - 90, 0, 90, 30).
pub const GUEST_BUFFLIST_W: u32 = 80;
pub const GUEST_BUFFLIST_H: u32 = 28;
pub const GUEST_TARGETBOX_W: u32 = 90;
pub const GUEST_TARGETBOX_H: u32 = 30;

/// How far DOWN (guest px) the game's top band is pushed by writing its y-origin
/// global, so it clears our gauge block. Mirrors `overlay::hud_y_shift`.
/// 0 now: the band is moved sideways (band_x_shift), not down.
pub fn hud_y_shift(_game: Rect, _lh: u32) -> u32 {
    0
}

/// Guest x the game's buff band is patched to start at: just right of our bars.
pub fn band_x_shift(game: Rect, lw: u32) -> u32 {
    let bars = stat_bars(game);
    let right = bars.x + bars.w - game.x;
    ((right * lw / game.w) + 3).min(200).max(1)
}

pub fn game_buff_box(game: Rect, lw: u32, lh: u32) -> Rect {
    let dy = hud_y_shift(game, lh) * game.h / lh;
    let x0 = band_x_shift(game, lw) * game.w / lw;
    Rect::new(game.x + x0, game.y + dy, GUEST_BUFFLIST_W * game.w / lw, GUEST_BUFFLIST_H * game.h / lh)
}

/// Same y-origin global as the buff list, so the shift moves this too.
/// ALSO shifted LEFT by `target_box_shift` guest px (host poke of the guest's
/// right-anchor immediate) so it lands beside, not under, our minimap panel.
pub fn game_target_box(game: Rect, lw: u32, lh: u32) -> Rect {
    game_target_box_at(game, lw, lh, minimap_panel(game.w, game.h))
}

/// The box BEFORE the host pokes it — right-edge anchored, as the guest draws it.
pub fn game_target_box_base(game: Rect, lw: u32, lh: u32) -> Rect {
    let w = GUEST_TARGETBOX_W * game.w / lw;
    let dy = hud_y_shift(game, lh) * game.h / lh;
    Rect::new(game.x + game.w - w, game.y + dy, w, GUEST_TARGETBOX_H * game.h / lh)
}

/// Where the box actually lands once it has dodged a panel at `map`.
pub fn game_target_box_at(game: Rect, lw: u32, lh: u32, map: Rect) -> Rect {
    let base = game_target_box_base(game, lw, lh);
    let dx = target_box_shift_for(game, lw, lh, map) * game.w / lw;
    Rect::new(base.x.saturating_sub(dx), base.y, base.w, base.h)
}

/// Mirrors `overlay::target_box_shift`: how far LEFT (guest px) the game's own
/// target-monster box is poked so it clears our corner minimap panel.
pub fn target_box_shift(game: Rect, lw: u32) -> u32 {
    let (_, lh) = init_logical(game.w, game.h, true, true, 240);
    target_box_shift_for(game, lw, lh, minimap_panel(game.w, game.h))
}

/// ★ Mirrors the 2026-07-31 rewrite: an OVERLAP QUERY, not a right-anchor
/// inversion. The box only moves when the panel actually covers it — which is what
/// makes a minimap dragged anywhere else leave the game's own HUD alone.
pub fn target_box_shift_for(game: Rect, lw: u32, lh: u32, map: Rect) -> u32 {
    if game.w == 0 || game.w <= game.h {
        return 0;
    }
    let base = game_target_box_base(game, lw, lh);
    if !base.intersects(&map) {
        return 0;
    }
    let need = (base.x + base.w).saturating_sub(map.x);
    (need * lw / game.w + 2).min(TARGET_BOX_SHIFT_MAX)
}

/// False when the dodge the panel demands no longer encodes, i.e. the game's own
/// monster box would be drawn underneath it.
pub fn target_box_can_dodge(win_w: u32, win_h: u32, map: Rect) -> bool {
    let game = Rect::new(0, 0, win_w, win_h);
    let (lw, lh) = init_logical(win_w, win_h, true, true, 240);
    if win_w <= win_h {
        return true;
    }
    !game_target_box_at(game, lw, lh, map).intersects(&map)
}

/// Secondary-weapon button (key `7`) — mirrors `overlay::secondary_button`.
/// IMMEDIATELY left of the disc, level with it, one gap apart (must not call back
/// into the ring — the ring clears IT).
pub fn secondary_button(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let a = action_disc(win_w, win_h, pad);
    let d = (a.w * 3 / 5).max(56);
    Rect::new(a.x.saturating_sub(pad * 2 + d), (a.y + a.h / 2).saturating_sub(d / 2), d, d)
}

/// Top-right, but LEFT of the game's own (right-anchored) target-monster box.
/// `lw/lh` = the live logical size, so the guest-space reservation scales.
pub fn minimap_panel_in(win_w: u32, win_h: u32, _lw: u32, _lh: u32) -> Rect {
    minimap_panel(win_w, win_h)
}

/// EXTREME top-right corner. The game's own target box is moved LEFT (not down)
/// so it lands beside the panel - player request 2026-07-30.
pub fn minimap_panel_at(win_w: u32, win_h: u32, _lh: u32) -> Rect {
    minimap_panel(win_w, win_h)
}

pub fn minimap_panel(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let side = (win_h * 26 / 100).clamp(120, 280);
    Rect::new(win_w.saturating_sub(pad * 2 + side), pad * 2, side, side)
}

/// Immersive gear size — SMALL (player request 2026-07-30: the old
/// `min/11` buttons covered the game's own active-buff list).
pub fn gear_size(win_w: u32, win_h: u32) -> u32 {
    (win_w.min(win_h) / 22).clamp(30, 64)
}

/// Player-gauge block (mirrors `overlay::stats_block`): the round level plate and
/// the HP/SP capsule stack to its right.
/// Extreme top-left with a small margin (the game's own band is moved DOWN via the
/// y-origin global instead of us dodging it).
pub fn level_plate(game: Rect) -> Rect {
    let pad = pad_of(game.w, game.h);
    let pr = (game.h / 26).clamp(14, 40);
    Rect::new(game.x + pad * 2, game.y + pad * 2, pr * 2, pr * 2)
}

pub fn level_plate_in(game: Rect, _lw: u32, _lh: u32) -> Rect {
    level_plate(game)
}

pub fn stat_bars(game: Rect) -> Rect {
    let pad = pad_of(game.w, game.h);
    let plate = level_plate(game);
    let bar_h = (game.h / 50).clamp(6, 16);
    let hp_bar_h = (bar_h + 2).min(18);
    let gap = (bar_h / 2).max(2);
    let bw = (game.w / 6).clamp(110, 420);
    let stack = hp_bar_h * 2 + bar_h + gap * 2; // HP + SP + EXP
    let bx = plate.x + plate.w + pad * 2;
    let by = plate.y + plate.h / 2 - stack / 2;
    Rect::new(bx, by, bw, stack)
}

/// The settings gear. ★ NON-FIELD modes only — in the field it is gone and that
/// strip belongs to the game's own buff band (player request 2026-07-30).
/// ★ 2026-07-30 (2): TOP-CENTRE, nudged slightly RIGHT. The `123` keypad toggle that
/// shared this row was removed 2026-09-18 (player request); the gear keeps the seat it
/// had beside it, so the row's spacing terms stay in the formula (mirrors
/// `overlay::gear_rect`).
pub fn gear_rect(win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    let gs = gear_size(win_w, win_h);
    let ts = (gs * 6 / 5).min(win_h / 8);
    let gap = pad * 2;
    let total = gs + gap + ts;
    let x0 = (win_w / 2 + win_w / 14).min(win_w.saturating_sub(total + pad)) - total / 2;
    let row_y = pad * 2;
    Rect::new(x0, row_y + (ts.saturating_sub(gs)) / 2, gs, gs)
}

// ── joystick input model (mirrors overlay::joy_follow / joy_dir) ─────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
}

/// Mirrors `overlay::joy_radius` in immersive mode — the thumb's PIN radius: the base
/// radius less the thumb's own, so the thumb disc never leaves the base (91차-B).
/// ★ It takes the WINDOW size, not the zone: the real function used to be handed the
/// zone rect and read `zone.w / zone.h` as if they were the window's, which was only
/// correct while the zone WAS the whole window.
pub fn joy_radius_immersive(win_w: u32, win_h: u32) -> f64 {
    joy_home(win_w, win_h).2 * (1.0 - JOY_THUMB)
}

/// Mirrors `overlay::joy_engage_radius` in immersive mode — what `joy_dir` is given as
/// its deadzone basis. Split from the pin radius in 91차-B.
pub fn joy_engage_immersive(win_w: u32, win_h: u32) -> f64 {
    let seat = t_stick(win_w, win_h).unwrap_or_else(|| stick_seat(win_w, win_h));
    stick_engage_radius(&seat, win_w, win_h)
}

/// One joystick drag, as the app models it: the centre is where the finger went
/// DOWN and never moves until it lifts (2026-08-03 — the base used to TRAIL the
/// finger, which on device read as the stick running away under the thumb).
#[derive(Clone, Copy, Debug)]
pub struct Joy {
    pub cx: f64,
    pub cy: f64,
    pub dir: Option<Key>,
}

impl Joy {
    /// Touch DOWN: the touch point becomes the centre.
    pub fn down(x: f64, y: f64) -> Joy {
        Joy { cx: x, cy: y, dir: None }
    }
    /// Touch MOVE: only the direction can change — mirrors `App::joy_update`.
    pub fn moved(&mut self, x: f64, y: f64, radius: f64) {
        self.dir = joy_dir(x - self.cx, y - self.cy, radius, self.dir);
    }
    /// Where the thumb disc is DRAWN: the finger, clamped to the ring.
    pub fn thumb(&self, x: f64, y: f64, radius: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.cx, y - self.cy);
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > radius && dist > 0.0 {
            (self.cx + dx / dist * radius, self.cy + dy / dist * radius)
        } else {
            (x, y)
        }
    }
}

pub fn joy_dir(dx: f64, dy: f64, radius: f64, active: Option<Key>) -> Option<Key> {
    const DEAD_ENGAGE: f64 = 0.30;
    const DEAD_KEEP: f64 = 0.15;
    const TURN_RATIO: f64 = 1.2;

    let dist = (dx * dx + dy * dy).sqrt();
    let dead = radius * if active.is_some() { DEAD_KEEP } else { DEAD_ENGAGE };
    if dist <= dead || dist == 0.0 {
        return None;
    }
    let comp = |k: Key| match k {
        Key::Up => -dy,
        Key::Down => dy,
        Key::Left => -dx,
        Key::Right => dx,
    };
    let cand = if dx.abs() >= dy.abs() {
        if dx >= 0.0 { Key::Right } else { Key::Left }
    } else if dy >= 0.0 {
        Key::Down
    } else {
        Key::Up
    };
    if let Some(cur) = active {
        if cur != cand && comp(cur) > 0.0 && comp(cand) < comp(cur) * TURN_RATIO {
            return Some(cur);
        }
    }
    Some(cand)
}


/// The game's own field HUD box in window px: a fixed 176x24 LOGICAL px box,
/// centred and bottom-anchored (measured with wie_probe at logical widths
/// 240/300/426/514/640 — the run length never moved and its centre tracked the
/// screen centre to within 1 px).
pub fn game_hud_box(game: Rect, lw: u32, lh: u32) -> Rect {
    let bw = (game.w * 176 / lw.max(1)).min(game.w);
    let bh = (game.h * 24 / lh.max(1)).max(2);
    Rect::new(game.x + (game.w - bw) / 2, (game.y + game.h).saturating_sub(bh), bw, bh)
}


// ── placement: the player-authored control layout (mirrors placement.rs) ─────
//
// Position is stored as a FRACTION of the window and size as a PERCENT of the
// built-in size, so a layout survives rotation and a change of device. The
// no-overlap rule is enforced on the EFFECTIVE rects at every resolve, and a
// placement that does not survive falls back to the built-in — which is why the
// tests below can assert disjointness unconditionally.

/// ★ `Mode::Cutscene` (the game's SCRIPT window, id 4) exists in the real overlay but
/// NOT here: it stores no seats of its own — `placement::ids_for` and `Table` alias it
/// to `Menu` — so every geometry property below is the same property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Field,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CtrlId {
    Ok,
    Cancel,
    Secondary,
    Minimap,
    /// The floating stick's home — POSITION ONLY (its radius is the walk-start
    /// distance). Placeable so a left-handed layout can send it to the other side.
    Stick,
    Quick(u8),
    /// The two dedicated potion buttons (0 = HP, 1 = MP), field only.
    Potion(u8),
}

pub const FIELD_IDS: [CtrlId; 14] = [
    CtrlId::Minimap,
    CtrlId::Stick,
    CtrlId::Ok,
    CtrlId::Cancel,
    CtrlId::Secondary,
    CtrlId::Quick(0),
    CtrlId::Quick(1),
    CtrlId::Quick(2),
    CtrlId::Quick(3),
    CtrlId::Quick(4),
    CtrlId::Quick(5),
    CtrlId::Quick(6),
    CtrlId::Potion(0),
    CtrlId::Potion(1),
];
/// Which controls exist with a game window open. A STRICT SUBSET of [`FIELD_IDS`] —
/// that relation is what lets the two modes share one [`Table`].
///
/// ★ The quickbar plates are in this set since 2026-08-03: they stay on screen with a
/// game window open. They used to need menu-mode SEATS of their own too; since
/// 2026-09-10 there is one arrangement, so this list only says what is DRAWN there.
pub const MENU_IDS: [CtrlId; 10] = [
    CtrlId::Ok,
    CtrlId::Cancel,
    // ★ 91차: the four arrows that step a window's cursor are cut from the STICK seat.
    CtrlId::Stick,
    CtrlId::Quick(0),
    CtrlId::Quick(1),
    CtrlId::Quick(2),
    CtrlId::Quick(3),
    CtrlId::Quick(4),
    CtrlId::Quick(5),
    CtrlId::Quick(6),
];

pub fn ids_for(mode: Mode) -> &'static [CtrlId] {
    match mode {
        Mode::Field => &FIELD_IDS,
        Mode::Menu => &MENU_IDS,
    }
}

pub const PCT_MIN: u32 = 50;
pub const PCT_MAX: u32 = 250;
pub const PCT_DEFAULT: u32 = 100;

pub fn scaled(v: u32, pct: u32) -> u32 {
    ((u64::from(v) * u64::from(pct.clamp(PCT_MIN, PCT_MAX)) / 100) as u32).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub cx: f32,
    pub cy: f32,
    pub pct: u32,
}

impl Place {
    pub fn from_rect(r: Rect, builtin: Rect, win_w: u32, win_h: u32) -> Place {
        let pct = if builtin.w == 0 {
            PCT_DEFAULT
        } else {
            ((u64::from(r.w) * 100 / u64::from(builtin.w)) as u32).clamp(PCT_MIN, PCT_MAX)
        };
        Place {
            cx: (r.x as f32 + r.w as f32 / 2.0) / win_w.max(1) as f32,
            cy: (r.y as f32 + r.h as f32 / 2.0) / win_h.max(1) as f32,
            pct,
        }
    }

    pub fn to_rect(self, builtin: Rect, win_w: u32, win_h: u32) -> Rect {
        let w = scaled(builtin.w.max(1), self.pct).min(win_w.max(1));
        let h = scaled(builtin.h.max(1), self.pct).min(win_h.max(1));
        let cx = (self.cx.clamp(0.0, 1.0) * win_w as f32) as i64;
        let cy = (self.cy.clamp(0.0, 1.0) * win_h as f32) as i64;
        let x = (cx - i64::from(w) / 2).clamp(0, i64::from(win_w.saturating_sub(w)));
        let y = (cy - i64::from(h) / 2).clamp(0, i64::from(win_h.saturating_sub(h)));
        Rect::new(x as u32, y as u32, w, h)
    }
}

/// ONE arrangement, honoured in every mode and keyed on [`FIELD_IDS`] — mirrors
/// `placement::Table` since 2026-09-10 (player: "menu섹션 완전 삭제할꺼야. 이제
/// game탭에서 핸들링한대로 메뉴 모드일때도 전부 동일한 레이아웃으로 나오면돼"). It was keyed on
/// `(Mode, CtrlId)` until then, and the editor carried a GAME / MENU page switch to fill
/// the two halves.
///
/// ★ The MODE still decides which controls EXIST ([`ids_for`]) and what they may not
/// cover ([`obstacles`]). Only the seat REQUEST is shared.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Table {
    entries: Vec<(CtrlId, Place)>,
}

impl Table {
    pub fn get(&self, id: CtrlId) -> Option<Place> {
        self.entries.iter().find(|(c, _)| *c == id).map(|(_, p)| *p)
    }
    pub fn set(&mut self, id: CtrlId, place: Place) {
        // ★ Against FIELD_IDS, which is the whole vocabulary: MENU_IDS is a subset of it,
        // and that subset relation is what lets one table serve both modes (the app
        // asserts it at compile time).
        if !FIELD_IDS.contains(&id) {
            return;
        }
        // ★ There used to be a `pct = PCT_DEFAULT` clamp for `CtrlId::Stick` here,
        // mirroring the "position only" rule. Gone 2026-08-16: the stick is resizable
        // now, and its size is also its deflection radius (see `joy_home`).
        self.entries.retain(|(c, _)| *c != id);
        self.entries.push((id, place));
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (id, p) in &self.entries {
            out.push_str(&format!("field {} {:.4} {:.4} {}\n", id_key(*id), p.cx, p.cy, p.pct));
        }
        out
    }
}

pub fn id_key(id: CtrlId) -> String {
    match id {
        CtrlId::Ok => "ok".into(),
        CtrlId::Cancel => "clr".into(),
        CtrlId::Secondary => "sec".into(),
        CtrlId::Minimap => "map".into(),
        CtrlId::Stick => "stick".into(),
        CtrlId::Quick(i) => format!("q{i}"),
        CtrlId::Potion(i) => format!("p{i}"),
    }
}

pub fn parse_id(s: &str) -> Option<CtrlId> {
    Some(match s {
        "ok" => CtrlId::Ok,
        "clr" => CtrlId::Cancel,
        "sec" => CtrlId::Secondary,
        "map" => CtrlId::Minimap,
        "stick" => CtrlId::Stick,
        // ★ 91차: the D-pad's own line becomes the STICK's — the square it was
        // authored for IS the merged seat. Mirrors `CtrlId::parse`.
        "dpad" => CtrlId::Stick,
        _ => {
            let (kind, n) = s.split_at(1);
            let i: u8 = n.parse().ok()?;
            match kind {
                "q" if usize::from(i) < PLATES => CtrlId::Quick(i),
                "p" if i < 2 => CtrlId::Potion(i),
                _ => return None,
            }
        }
    })
}

pub fn parse_line(line: &str) -> Option<(CtrlId, Place)> {
    let mut it = line.split_whitespace();
    // ★ Only `field`. A `menu …` line belongs to the second table that no longer
    // exists, so an old file keeps its field lines and loses its menu ones — dropping
    // them IS the migration.
    if it.next()? != "field" {
        return None;
    }
    let id = parse_id(it.next()?)?;
    if !FIELD_IDS.contains(&id) {
        return None;
    }
    let cx: f32 = it.next()?.parse().ok()?;
    let cy: f32 = it.next()?.parse().ok()?;
    let pct: u32 = it.next()?.parse().ok()?;
    if !cx.is_finite() || !cy.is_finite() || !(0.0..=1.0).contains(&cx) || !(0.0..=1.0).contains(&cy) {
        return None;
    }
    Some((id, Place { cx, cy, pct: pct.clamp(PCT_MIN, PCT_MAX) }))
}

pub fn parse_table(text: &str) -> Table {
    let mut t = Table::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((id, p)) = parse_line(line) {
            t.set(id, p);
        }
    }
    t
}

/// Built-in rect of one control — a pure function of the window.
pub fn builtin(id: CtrlId, win_w: u32, win_h: u32) -> Rect {
    let pad = pad_of(win_w, win_h);
    match id {
        CtrlId::Ok => action_disc(win_w, win_h, pad),
        // ★ 2026-08-07: the DEFAULT CLR is secondary-attack-button sized.
        // ★★ 80차-H: and since then it sits BETWEEN plate 1 and the OK disc, hugging the
        // right edge — not on ring seat 0. Mirrors `overlay::cancel_seat`.
        CtrlId::Cancel => cancel_seat(win_w, win_h, pad),
        CtrlId::Secondary => secondary_button(win_w, win_h),
        CtrlId::Minimap => minimap_panel(win_w, win_h),
        // ★ 91차: the ONE walk seat — the arrows' bounding square; the stick's base is
        // its inscribed circle (`joy_home`).
        CtrlId::Stick => stick_seat(win_w, win_h),
        // The quickbar plates keep the ring seats in BOTH modes.
        CtrlId::Quick(i) => ring_seat(win_w, win_h, pad, usize::from(i) + 1),
        CtrlId::Potion(i) => potion_button(win_w, win_h, usize::from(i)),
    }
}

pub fn ring_seat(win_w: u32, win_h: u32, pad: u32, i: usize) -> Rect {
    let ring = ring_all(win_w, win_h, pad);
    ring.get(i).copied().unwrap_or_else(|| {
        let s = (win_h / 11).clamp(40, 120);
        Rect::new(win_w.saturating_sub(s + pad), win_h.saturating_sub(s + pad), s, s)
    })
}

/// Source cell size of the unspent-point badges (`ui_bm` frame 6 = 14x7 = 2 x 7x7).
/// Mirrors `overlay::BADGE_SRC`.
pub const BADGE_SRC: u32 = 7;
/// Mirrors `overlay::BADGE_BLINK_MS`: 250 ms on / 250 ms off = 2 Hz.
pub const BADGE_BLINK_MS: u128 = 250;

/// Mirrors `overlay::badge_blink_on`.
pub fn badge_blink_on(ms: u128) -> bool {
    (ms / BADGE_BLINK_MS) % 2 == 0
}

/// The two unspent-point badges (red + = stat, blue + = skill) — mirrors
/// `overlay::point_badges`. Side by side, BELOW the bar stack, left-aligned with it
/// (moved from the bars' right side 2026-08-05 on player request).
pub fn point_badges(game: Rect) -> (Rect, Rect) {
    let bars = stat_bars(game);
    let bar_h = (game.h / 50).clamp(6, 16);
    let gap = (bar_h / 2).max(2);
    let scale = ((bar_h * 2) / BADGE_SRC).max(1);
    let side = BADGE_SRC * scale;
    let bgap = (2 * scale).max(2);
    let x = bars.x;
    let y = bars.y + bars.h + (gap * 2).max(4);
    (Rect::new(x, y, side, side), Rect::new(x + side + bgap, y, side, side))
}

/// Our own gauge block (level plate + HP/SP/EXP + both point badges) as ONE rect —
/// mirrors `overlay::stats_block_box`. The badges are included UNCONDITIONALLY even
/// though they only draw while points are unspent: reserving space that is usually
/// empty beats a control that is legal at level 1 and covered after the first
/// level-up.
pub fn gauge_block(game: Rect) -> Rect {
    let plate = level_plate(game);
    let bars = stat_bars(game);
    let (bstat, bskill) = point_badges(game);
    let x0 = plate.x.min(bars.x);
    let y0 = plate.y.min(bars.y).min(bstat.y);
    let x1 = (plate.x + plate.w).max(bars.x + bars.w).max(bskill.x + bskill.w);
    let y1 = (plate.y + plate.h).max(bars.y + bars.h).max(bstat.y + bstat.h);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The editor's own chrome (mirrors `overlay::edit_items`): ONE compact row across
/// the TOP band. ★ 2026-08-03 — it was in the bottom-centre, which is where every
/// long drag passes, so the player kept hitting SAVE/CANCEL while arranging buttons.
/// The span runs from the right edge of the gauge block to the left edge of the
/// minimap's built-in corner, which is what keeps it off every control's seat.
pub fn edit_items(win_w: u32, win_h: u32) -> Vec<Rect> {
    let pad = pad_of(win_w, win_h);
    let gap = pad * 2;
    let gauges = gauge_block(Rect::new(0, 0, win_w, win_h));
    let left = (gauges.x + gauges.w) + gap;
    let right = minimap_panel(win_w, win_h).x.saturating_sub(gap).max(left + 6);
    let avail = right - left;
    let cell_w = |cols: u32| ((avail + gap) / cols).saturating_sub(gap);
    // ★ SIX: `FLIP HIDE SHOW RESET CANCEL SAVE`. It was eight for one day, 2026-09-10 —
    // HIDE / SHOW joined that morning and GAME / MENU left that afternoon with the second
    // seat table. 6/3 and 8/4 both mean "one row or two", so the ROW COUNT — and therefore
    // this block's height and its footprint in `obstacles` — is what it has been since
    // 2026-08-03 either way.
    let cols: u32 = if cell_w(6) >= 56 { 6 } else { 3 };
    let w = cell_w(cols).clamp(40, 170);
    let h = (win_h / 18).clamp(26, 64);
    let span = w * cols + gap * (cols - 1);
    let x0 = (left + avail.saturating_sub(span) / 2).min(win_w.saturating_sub(span));
    let y0 = pad * 2;
    (0..6u32).map(|i| Rect::new(x0 + (i % cols) * (w + gap), y0 + (i / cols) * (h + gap), w, h)).collect()
}

/// Mirrors `Table::mirror`: flip a mode's whole layout left<->right, with a short
/// ordered search for the nearest legal spot. ★ A straight mirror leaves a
/// half-flipped layout: the ring's lower seats mirror into the bottom-centre band the
/// editor chrome occupies and get refused.
/// Offsets tried in order when a control's exact mirror will not fit — straight up
/// first, then sideways. Mirrors `placement::mirror_search`.
pub fn mirror_search() -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for step in 0..11 {
        let dy = -0.05 * step as f32;
        for dx in [0.0f32, 0.04, -0.04, 0.08, -0.08] {
            out.push((dx, dy));
        }
    }
    out
}

/// Per-control: is the STORED request what the resolver actually produces — **in every
/// mode the control exists in**? Indexed like `ids_for(mode)`.
///
/// ★ BOTH modes, like the app's `Table::honoured`, since 91차. It stayed per-mode from
/// 83차 until then because this mirror's menu obstacle set still reserved the joystick
/// square (which the app dropped on 2026-08-09), so demanding both modes failed for the
/// mirror's reasons and not the app's. 91차 removed that reservation — the stick seat is
/// a MENU CONTROL now (the arrows) — and the two-mode form passes the whole matrix.
pub fn honoured(t: &Table, mode: Mode, win_w: u32, win_h: u32) -> Vec<bool> {
    let ids = ids_for(mode);
    let mut out = vec![true; ids.len()];
    for m in modes_all() {
        let resolved = resolve(m, win_w, win_h, t);
        for (j, id) in ids_for(m).iter().enumerate() {
            let Some(i) = ids.iter().position(|c| c == id) else { continue };
            let Some(p) = t.get(*id) else { continue };
            if resolved[j].1 != p.to_rect(builtin(*id, win_w, win_h), win_w, win_h) {
                out[i] = false;
            }
        }
    }
    out
}

fn modes_all() -> [Mode; 2] {
    [Mode::Field, Mode::Menu]
}

pub fn mirror_table(t: &Table, mode: Mode, win_w: u32, win_h: u32) -> Table {
    let mut out = t.clone();
    // Phase 1: mirror everything, unvalidated. ★ Validating as we go blocks each
    // control on the OLD seats of the ones after it.
    for id in ids_for(mode) {
        let cur = out.get(*id).unwrap_or_else(|| {
            let b = builtin(*id, win_w, win_h);
            Place::from_rect(b, b, win_w, win_h)
        });
        out.set(*id, Place { cx: 1.0 - cur.cx, ..cur });
    }
    // Phase 2: repair what the resolver refuses — WITHOUT costing a higher-priority
    // control its place. ★ Traced: the stick and OK block each other through the seats
    // they have not vacated yet, and while both stalled the `#` plate (last priority)
    // was repaired into the spot OK needed and OK lost it for good.
    let ids = ids_for(mode);
    for i in 0..ids.len() {
        let id = ids[i];
        let hon = honoured(&out, mode, win_w, win_h);
        if hon[i] {
            continue;
        }
        let Some(want) = out.get(id) else { continue };
        for (dx, dy) in mirror_search() {
            let cand = Place {
                cx: (want.cx + dx).clamp(0.0, 1.0),
                cy: (want.cy + dy).clamp(0.0, 1.0),
                ..want
            };
            let mut trial = out.clone();
            trial.set(id, cand);
            let after = honoured(&trial, mode, win_w, win_h);
            let keeps_betters = hon[..i].iter().zip(&after[..i]).all(|(was, now)| !*was || *now);
            if after[i] && keeps_betters {
                out = trial;
                break;
            }
        }
    }
    out
}

/// The FIELD stick rect as the resolver produces it — used as a MENU obstacle. The
/// mirror keeps the live table in a thread-local so the signature stays simple.
pub fn t_stick(win_w: u32, win_h: u32) -> Option<Rect> {
    LIVE.with(|t| {
        let t = t.borrow();
        resolve(Mode::Field, win_w, win_h, &t).into_iter().find(|(c, _)| *c == CtrlId::Stick).map(|(_, r)| r)
    })
}

thread_local! {
    /// The table `obstacles` should consult for the cross-mode stick rect. Tests set
    /// it when they need the coupling; it defaults to "no custom layout".
    pub static LIVE: std::cell::RefCell<Table> = std::cell::RefCell::new(Table::default());
}

/// Everything a control may NOT cover — mirrors `placement::obstacles`. Three
/// families: the game's own HUD (field), our menu chrome (gear / 123 / pull-out pad,
/// all hit-tested BEFORE the buttons), and the editor's own chrome (hit-tested first
/// while editing, so a plate buried under it could never be picked up again).
pub fn obstacles(mode: Mode, win_w: u32, win_h: u32, map: Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    if win_w == 0 || win_h == 0 {
        return out;
    }
    let game = Rect::new(0, 0, win_w, win_h);
    if mode == Mode::Field && win_w > win_h {
        let (lw, lh) = init_logical(win_w, win_h, true, true, 240);
        out.push(game_buff_box(game, lw, lh));
        out.push(game_target_box_at(game, lw, lh, map));
        // ★ OUR gauge block: not a control, but painted AFTER the controls, so
        // anything under it silently disappears behind the bars.
        let plate = level_plate(game);
        let bars = stat_bars(game);
        let x0 = plate.x.min(bars.x);
        let y0 = plate.y.min(bars.y);
        let x1 = (plate.x + plate.w).max(bars.x + bars.w);
        let y1 = (plate.y + plate.h).max(bars.y + bars.h);
        out.push(Rect::new(x0, y0, x1 - x0, y1 - y0));
    }
    if mode == Mode::Menu {
        out.push(gear_rect(win_w, win_h));
        // ★ The stick's seat is NOT an obstacle here (91차 — the app dropped it on
        // 2026-08-09, and since 91차 the seat is a MENU CONTROL: the arrows). The mirror
        // reserved the old 180 px disc until then; with the seat now the arrows' square
        // that reservation would have blocked the whole bottom-left of a window.
    }
    out.extend(edit_items(win_w, win_h));
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

fn grown(r: Rect, n: u32) -> Rect {
    Rect::new(r.x.saturating_sub(n), r.y.saturating_sub(n), r.w + n * 2, r.h + n * 2)
}

fn accepts(mode: Mode, win_w: u32, win_h: u32, i: usize, cand: Rect, others: &[(CtrlId, Rect)], map: Rect) -> bool {
    if cand.w == 0 || cand.h == 0 || cand.x + cand.w > win_w || cand.y + cand.h > win_h {
        return false;
    }
    let near = grown(cand, pad_of(win_w, win_h));
    // ★ Control vs control is NOT tested at all since 2026-08-07 (player: "겹칠 수
    // 있게 바꿔줘"). Obstacles below stay strict — those are hit-tested before the
    // button list, so sharing pixels with one means the control silently does
    // something else.
    if others[i].0 == CtrlId::Minimap && !target_box_can_dodge(win_w, win_h, cand) {
        return false;
    }
    obstacles(mode, win_w, win_h, map).iter().all(|o| !near.intersects(o))
}

/// ★ Runs to a FIXPOINT (mirrors `placement::resolve_with`). One pass would check
/// each control against the BUILT-IN rects of everything after it, so a control the
/// player had already moved away still blocked its old seat like a ghost.
pub fn resolve(mode: Mode, win_w: u32, win_h: u32, table: &Table) -> Vec<(CtrlId, Rect)> {
    let ids = ids_for(mode);
    // Computed ONCE: each built-in re-derives the whole six-seat ring.
    let base_rects: Vec<Rect> = ids.iter().map(|id| builtin(*id, win_w, win_h)).collect();
    let mut out: Vec<(CtrlId, Rect)> = ids.iter().copied().zip(base_rects.iter().copied()).collect();
    if win_w == 0 || win_h == 0 {
        return out;
    }
    for _pass in 0..ids.len() {
        let mut changed = false;
        for i in 0..out.len() {
            let id = out[i].0;
            let base = base_rects[i];
            let Some(p) = table.get(id) else { continue };
            let cand = p.to_rect(base, win_w, win_h);
            if out[i].1 == cand {
                continue;
            }
            let map = if id == CtrlId::Minimap { cand } else { out[0].1 };
            if accepts(mode, win_w, win_h, i, cand, &out, map) {
                out[i].1 = cand;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out
}

/// "If I stored this, would it be honoured?" — answered by running the REAL
/// resolver, so the editor and the layout can never disagree.
pub fn lands(mode: Mode, win_w: u32, win_h: u32, id: CtrlId, place: Place, table: &Table) -> bool {
    let want = place.to_rect(builtin(id, win_w, win_h), win_w, win_h);
    let mut probe = table.clone();
    probe.set(id, place);
    resolve(mode, win_w, win_h, &probe).iter().any(|(c, r)| *c == id && *r == want)
}

/// What the editor actually asks (mirrors `placement::would_accept_place`): the edit
/// must land AND no other control may be knocked off its own placement to make room.
/// ★ 2026-08-03 — the resolver is priority-ordered, so a control early in `ids_for`
/// that grew into a later one's stored rect won and the loser jumped back to its
/// built-in seat. On the device that read as "the other buttons get pushed away".
pub fn would_accept_place(mode: Mode, win_w: u32, win_h: u32, id: CtrlId, place: Place, table: &Table) -> bool {
    if !lands(mode, win_w, win_h, id, place, table) {
        return false;
    }
    let mut probe = table.clone();
    probe.set(id, place);
    let before = resolve(mode, win_w, win_h, table);
    let after = resolve(mode, win_w, win_h, &probe);
    ids_for(mode).iter().enumerate().all(|(j, c)| {
        if *c == id {
            return true;
        }
        let Some(p) = table.get(*c) else { return true };
        let asked = p.to_rect(builtin(*c, win_w, win_h), win_w, win_h);
        before[j].1 != asked || after[j].1 == asked
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real device / emulator window sizes to exercise.
    const WINDOWS: [(u32, u32); 5] = [
        (2340, 1080), // Galaxy S25 Ultra landscape (19.5:9)
        (2400, 1080), // 20:9 phone
        (1920, 1080), // 16:9
        (1600, 1000), // 16:10 tablet
        (1080, 2340), // portrait (immersive must NOT engage)
    ];

    // ── player-authored layout (2026-07-31) ──────────────────────────────────

    /// Deterministic pseudo-random placements — a fixed LCG, so a failure is
    /// reproducible and the suite never flakes.
    fn lcg(seed: &mut u64) -> f32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 33) as f32) / ((1u64 << 31) as f32)
    }

    fn modes() -> [Mode; 2] {
        [Mode::Field, Mode::Menu]
    }

    /// An empty table must reproduce the layout that shipped, control for control —
    /// so no player who never opens the editor sees any change at all.
    #[test]
    fn an_empty_layout_resolves_to_the_builtin_controls() {
        let empty = Table::default();
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let ring = ring_all(w, h, pad);
            for mode in modes() {
                for (id, r) in resolve(mode, w, h, &empty) {
                    let want = match id {
                        CtrlId::Ok => action_disc(w, h, pad),
                        CtrlId::Cancel => builtin(CtrlId::Cancel, w, h),
                        CtrlId::Secondary => secondary_button(w, h),
                        CtrlId::Minimap => minimap_panel(w, h),
                        CtrlId::Stick => builtin(CtrlId::Stick, w, h),
                        CtrlId::Quick(i) => ring[usize::from(i) + 1],
                        CtrlId::Potion(i) => potion_button(w, h, usize::from(i)),
                    };
                    assert_eq!(r, want, "{w}x{h} {mode:?} {id:?} moved without being asked");
                }
            }
        }
    }

    /// THE invariant (rewritten 2026-08-07): whatever is stored, the controls that
    /// come out are on screen and clear of every OBSTACLE. Control-vs-control
    /// overlap is now the player's own business ("겹칠 수 있게 바꿔줘"), so it is no
    /// longer asserted — only obstacles (the game's HUD, our chrome, the editor's
    /// buttons) keep the strict rule.
    #[test]
    fn every_resolved_layout_is_onscreen_and_clear_of_obstacles() {
        let mut seed = 0x5eed_1234u64;
        // ★ Counted, because "everything was rejected" would make this test pass
        // while proving nothing. Measured 61 % applied at the time of writing.
        let (mut applied, mut total) = (0u32, 0u32);
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let screen = Rect::new(0, 0, w, h);
            for mode in modes() {
                for _round in 0..40 {
                    // Ask for a deliberately hostile layout: everything piled into a
                    // random spot at a random size.
                    let mut t = Table::default();
                    for id in ids_for(mode) {
                        t.set(
                            *id,
                            Place {
                                cx: lcg(&mut seed),
                                cy: lcg(&mut seed),
                                pct: PCT_MIN + (lcg(&mut seed) * (PCT_MAX - PCT_MIN) as f32) as u32,
                            },
                        );
                    }
                    let got = resolve(mode, w, h, &t);
                    let map = got.iter().find(|(c, _)| *c == CtrlId::Minimap).map(|(_, r)| *r).unwrap_or(screen);
                    for (id, r) in &got {
                        total += 1;
                        if *r != builtin(*id, w, h) {
                            applied += 1;
                        }
                    }
                    for (na, a) in got.iter() {
                        assert!(a.inside(&screen), "{w}x{h} {mode:?} {na:?} {a:?} offscreen");
                        assert!(a.w > 0 && a.h > 0, "{w}x{h} {mode:?} {na:?} vanished");
                        // ★ Obstacles keep the STRICT rule. Two controls sharing pixels
                        // is the player's own cosmetic choice; a control sharing pixels
                        // with the gear / keypad / editor chrome / gauge tap zone is a
                        // control that silently does something else, because all of
                        // those are hit-tested BEFORE the button list.
                        for o in obstacles(mode, w, h, map) {
                            // A PLACED control may not cover an obstacle; a control on
                            // its BUILT-IN seat is the fallback and is not validated.
                            if *a != builtin(*na, w, h) {
                                assert!(!a.intersects(&o), "{w}x{h} {mode:?} {na:?} covers the game's own HUD {o:?}");
                            }
                        }
                    }
                }
            }
        }
        assert!(
            applied * 3 > total,
            "only {applied}/{total} placements were applied — this test would be proving nothing"
        );
    }

    /// ★ The 2026-08-07 rule: dropping one control ON another is now ACCEPTED (the
    /// player asked for free overlap), and the control underneath keeps its own rect
    /// — nothing is displaced to "make room". What still falls back is a placement on
    /// an OBSTACLE (see `menu_controls_cannot_be_parked_on_our_own_chrome`).
    #[test]
    fn a_control_may_be_stacked_on_another_control() {
        let (w, h) = (2340u32, 1080u32);
        let pad = pad_of(w, h);
        let clr = builtin(CtrlId::Cancel, w, h);
        let mut t = Table::default();
        // Drop OK exactly on top of CLR.
        let p = Place {
            cx: (clr.x + clr.w / 2) as f32 / w as f32,
            cy: (clr.y + clr.h / 2) as f32 / h as f32,
            pct: 100,
        };
        assert!(
            would_accept_place(Mode::Field, w, h, CtrlId::Ok, p, &t),
            "stacking OK on CLR must now be accepted"
        );
        t.set(CtrlId::Ok, p);
        let got = resolve(Mode::Field, w, h, &t);
        let ok = got.iter().find(|(c, _)| *c == CtrlId::Ok).unwrap().1;
        assert_eq!(ok, p.to_rect(action_disc(w, h, pad), w, h), "the stacked OK must be honoured");
        let clr_now = got.iter().find(|(c, _)| *c == CtrlId::Cancel).unwrap().1;
        assert_eq!(clr_now, clr, "CLR must keep its seat under the stacked OK");
    }

    /// ★ The regression that motivated the whole feature: resizing OK used to shrink
    /// every skill plate (measured 98 px -> 52 px), because the ring derived its
    /// radius from the disc. The plates must now be completely unaffected.
    #[test]
    fn resizing_ok_no_longer_shrinks_the_skill_plates() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let base: Vec<u32> = resolve(Mode::Field, w, h, &Table::default())
                .iter()
                .filter(|(c, _)| matches!(c, CtrlId::Quick(_)))
                .map(|(_, r)| r.w)
                .collect();
            for pct in [PCT_MIN, 150, PCT_MAX] {
                let mut t = Table::default();
                let d = builtin(CtrlId::Ok, w, h);
                t.set(
                    CtrlId::Ok,
                    Place { cx: (d.x + d.w / 2) as f32 / w as f32, cy: (d.y + d.h / 2) as f32 / h as f32, pct },
                );
                let now: Vec<u32> = resolve(Mode::Field, w, h, &t)
                    .iter()
                    .filter(|(c, _)| matches!(c, CtrlId::Quick(_)))
                    .map(|(_, r)| r.w)
                    .collect();
                assert_eq!(base, now, "{w}x{h}: OK at {pct}% must not resize the plates");
            }
        }
    }

    /// Sizing a control is MONOTONE — the thing the old global knob failed at.
    #[test]
    fn a_bigger_percent_is_a_bigger_control() {
        let (w, h) = (2340u32, 1080u32);
        for id in [CtrlId::Ok, CtrlId::Quick(2), CtrlId::Minimap] {
            let base = builtin(id, w, h);
            let mut last = 0u32;
            for pct in [PCT_MIN, 70, 100, 150, 200, PCT_MAX] {
                let r = Place { cx: 0.5, cy: 0.5, pct }.to_rect(base, w, h);
                assert!(r.w > last, "{id:?}: {pct}% gave {} after {last}", r.w);
                last = r.w;
            }
        }
    }

    /// ★ Dragging must never change the SIZE. The app carries the size through a
    /// drag as a PERCENT for this reason: recomputing it from the pixel rect floors
    /// twice, so a plate at 130 % reads back as 129 %, then 128 %, and a control the
    /// player merely slides around visibly withers.
    #[test]
    fn dragging_never_changes_the_size() {
        let (w, h) = (2340u32, 1080u32);
        for id in [CtrlId::Ok, CtrlId::Quick(1), CtrlId::Minimap, CtrlId::Secondary] {
            let base = builtin(id, w, h);
            for pct in [70u32, 115, 130, 175] {
                let mut place = Place { cx: 0.5, cy: 0.5, pct };
                let first = place.to_rect(base, w, h);
                for step in 0..40 {
                    place.cx = 0.30 + 0.01 * step as f32;
                    let r = place.to_rect(base, w, h);
                    assert_eq!((r.w, r.h), (first.w, first.h), "{id:?} at {pct}% resized while being moved");
                }
            }
        }
        // …and the round trip really is lossy, which is what the rule protects against.
        let base = builtin(CtrlId::Quick(1), w, h);
        let mut pct = 130u32;
        for _ in 0..4 {
            pct = Place::from_rect(Place { cx: 0.5, cy: 0.5, pct }.to_rect(base, w, h), base, w, h).pct;
        }
        assert!(pct < 130, "px -> percent -> px was expected to drift downwards, got {pct}");
    }

    /// A layout authored on one screen must still be legal on another — the reason
    /// it is stored as fractions and percents rather than pixels.
    #[test]
    fn a_layout_authored_on_one_screen_survives_another() {
        let (aw, ah) = (2340u32, 1080u32);
        let mut seed = 0xc0ffee_u64;
        for _round in 0..25 {
            let mut t = Table::default();
            for id in ids_for(Mode::Field) {
                t.set(
                    *id,
                    Place {
                        cx: lcg(&mut seed),
                        cy: lcg(&mut seed),
                        pct: PCT_MIN + (lcg(&mut seed) * 150.0) as u32,
                    },
                );
            }
            // Author it on one device, then replay it on every other one.
            let _ = resolve(Mode::Field, aw, ah, &t);
            for (w, h) in [(1920u32, 1080u32), (3120, 1440), (1600, 720), (1080, 2340)] {
                let screen = Rect::new(0, 0, w, h);
                let got = resolve(Mode::Field, w, h, &t);
                for (na, a) in got.iter() {
                    assert!(a.inside(&screen), "{w}x{h} {na:?} offscreen after a device change");
                }
            }
        }
    }

    /// ★ winit's Android backend delivers `Moved` for EVERY pointer of a MotionEvent,
    /// so the same gesture arrives twice. The pinch is computed from the anchor taken
    /// at pinch start, which makes a duplicate delivery a no-op — a delta-based
    /// formula would double-apply and the control would rocket away.
    #[test]
    fn pinch_is_idempotent_under_duplicate_moves() {
        let (w, h) = (2340u32, 1080u32);
        let base = builtin(CtrlId::Ok, w, h);
        let pct0 = 100u32;
        let dist0 = 200.0f64;
        let apply = |dist: f64| ((f64::from(pct0) * (dist / dist0)) as u32).clamp(PCT_MIN, PCT_MAX);
        // The same event, delivered twice, must land on the same size.
        assert_eq!(apply(300.0), apply(300.0));
        // And a sequence of positions depends only on the LAST one, not the path.
        assert_eq!(apply(300.0), 150);
        assert_eq!(apply(260.0), 130, "a shrink after a grow follows the fingers, not the history");
        let r = Place { cx: 0.5, cy: 0.5, pct: apply(300.0) }.to_rect(base, w, h);
        assert_eq!(r.w, scaled(base.w, 150));
    }

    /// The minimap drives a poke into the running game (its target-monster box is
    /// moved aside). Dragged away from that corner, the box must be left ALONE —
    /// the old right-anchor inversion asked for a 476 px shift and parked the game's
    /// box in the middle of the field.
    #[test]
    fn a_minimap_moved_away_stops_moving_the_games_box() {
        let (w, h) = (2340u32, 1080u32);
        let (lw, lh) = init_logical(w, h, true, true, 240);
        let game = Rect::new(0, 0, w, h);
        // Default: the panel owns the corner, so the box dodges and ends left of it.
        let def = minimap_panel(w, h);
        let shift = target_box_shift_for(game, lw, lh, def);
        assert!(shift > 0, "the corner panel must still push the box aside");
        let box_at = game_target_box_at(game, lw, lh, def);
        assert!(box_at.x + box_at.w <= def.x, "the box must end left of the panel");
        // Moved to the bottom-left: nothing to dodge.
        let away = Rect::new(40, h - 340, 300, 300);
        assert_eq!(target_box_shift_for(game, lw, lh, away), 0, "a panel elsewhere must not move the game's box");
    }

    /// ...and a panel the box CANNOT dodge is refused outright, rather than being
    /// accepted and silently drawn over the monster's name and HP bar.
    #[test]
    fn a_minimap_the_box_cannot_dodge_is_refused() {
        let (w, h) = (2340u32, 1080u32);
        let t = Table::default();
        // A panel spanning the whole top edge: the box has nowhere to go.
        let greedy = Rect::new(0, 0, w, 300);
        let place = Place::from_rect(greedy, builtin(CtrlId::Minimap, w, h), w, h);
        assert!(!would_accept_place(Mode::Field, w, h, CtrlId::Minimap, place, &t), "an undodgeable panel must be refused");
        // The built-in corner panel is of course fine.
        assert!(target_box_can_dodge(w, h, minimap_panel(w, h)));
    }

    /// ★ The editor and the layout must agree. `would_accept_place` used to be a
    /// second, similar-looking predicate that checked every neighbour at its FINAL
    /// position, while `resolve` checked the tail of the list against built-ins — so
    /// the editor accepted placements the layout then silently discarded (the control
    /// snapped back under the finger and a dead line went to disk). Now the question
    /// is answered by running the resolver itself, and this fuzz proves the two can
    /// never disagree.
    #[test]
    fn what_the_editor_accepts_is_what_the_layout_does() {
        let mut seed = 0xfeed_beefu64;
        let (mut yes, mut no) = (0u32, 0u32);
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            for mode in modes() {
                for _round in 0..30 {
                    // A partly-customised starting layout, then one more edit on top.
                    let mut t = Table::default();
                    for id in ids_for(mode) {
                        if lcg(&mut seed) < 0.5 {
                            t.set(*id, Place { cx: lcg(&mut seed), cy: lcg(&mut seed), pct: 60 + (lcg(&mut seed) * 120.0) as u32 });
                        }
                    }
                    let ids = ids_for(mode);
                    let id = ids[(lcg(&mut seed) * ids.len() as f32) as usize % ids.len()];
                    let p = Place { cx: lcg(&mut seed), cy: lcg(&mut seed), pct: 60 + (lcg(&mut seed) * 120.0) as u32 };
                    let want = p.to_rect(builtin(id, w, h), w, h);
                    let editor_says = would_accept_place(mode, w, h, id, p, &t);
                    // What the layout actually does with that same edit.
                    let mut after = t.clone();
                    after.set(id, p);
                    let got = resolve(mode, w, h, &after).into_iter().find(|(c, _)| *c == id).unwrap().1;
                    // Accepting means the edit lands EXACTLY where it was asked for,
                    // and every other control keeps the rect it already had. (The
                    // converse no longer holds: an edit that lands only by shoving a
                    // neighbour off its seat is refused — see the next test.)
                    if editor_says {
                        assert_eq!(got, want, "{w}x{h} {mode:?} {id:?}: editor accepted but the layout gave {got:?}");
                        let before = resolve(mode, w, h, &t);
                        let now = resolve(mode, w, h, &after);
                        for (j, (c, r0)) in before.iter().enumerate() {
                            if *c == id {
                                continue;
                            }
                            let Some(pl) = t.get(*c) else { continue };
                            let asked = pl.to_rect(builtin(*c, w, h), w, h);
                            if *r0 == asked {
                                assert_eq!(now[j].1, asked, "{w}x{h} {mode:?}: editing {id:?} displaced {c:?}");
                            }
                        }
                    }
                    if editor_says { yes += 1 } else { no += 1 }
                }
            }
        }
        assert!(yes > 0 && no > 0, "the fuzz must exercise BOTH answers (yes {yes}, no {no})");
    }

    /// The ghost the fixpoint removes: a control that has been moved away must stop
    /// blocking the seat it vacated.
    #[test]
    fn a_vacated_seat_can_be_taken_by_another_control() {
        let (w, h) = (2340u32, 1080u32);
        let ok_seat = builtin(CtrlId::Ok, w, h);
        let mut t = Table::default();
        // Move OK far away, into empty space on the left.
        t.set(CtrlId::Ok, Place { cx: 0.18, cy: 0.62, pct: 100 });
        let moved = resolve(Mode::Field, w, h, &t).into_iter().find(|(c, _)| *c == CtrlId::Ok).unwrap().1;
        assert_ne!(moved, ok_seat, "OK must actually have moved for this test to mean anything");
        // Now drop the minimap into the corner OK just left. Minimap is index 0, i.e.
        // the case a single-pass resolver got wrong: it saw OK's built-in ghost.
        let p = Place {
            cx: (ok_seat.x + ok_seat.w / 2) as f32 / w as f32,
            cy: (ok_seat.y + ok_seat.h / 2) as f32 / h as f32,
            pct: 100,
        };
        assert!(
            would_accept_place(Mode::Field, w, h, CtrlId::Minimap, p, &t),
            "the seat OK vacated must be available to the minimap"
        );
        t.set(CtrlId::Minimap, p);
        let map = resolve(Mode::Field, w, h, &t).into_iter().find(|(c, _)| *c == CtrlId::Minimap).unwrap().1;
        assert_eq!(map, p.to_rect(builtin(CtrlId::Minimap, w, h), w, h), "…and actually honoured");
    }

    /// A menu control may not be parked on the gear, the `123` toggle or the pull-out
    /// keypad: all three are hit-tested BEFORE the button list, so the key would
    /// silently do something else (the pad is drawn UNDER the control, so there is no
    /// visual cue at all).
    #[test]
    fn menu_controls_cannot_be_parked_on_our_own_chrome() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let t = Table::default();
            for target in [gear_rect(w, h)] {
                let p = Place {
                    cx: (target.x + target.w / 2) as f32 / w as f32,
                    cy: (target.y + target.h / 2) as f32 / h as f32,
                    pct: 100,
                };
                assert!(
                    !would_accept_place(Mode::Menu, w, h, CtrlId::Quick(1), p, &t),
                    "{w}x{h}: a menu-mode control must not be placeable on {target:?}"
                );
            }
        }
    }

    /// …and no control, in either mode, may be buried under the editor's own
    /// buttons: the editor hit-tests its chrome first (it must, or a control dropped
    /// on DONE would trap the player), so a small plate under RESET could never be
    /// picked up again — and the tap meant to grab it would wipe the whole mode.
    #[test]
    fn no_control_can_be_buried_under_the_editor_chrome() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let t = Table::default();
            for chrome in edit_items(w, h) {
                let p = Place {
                    cx: (chrome.x + chrome.w / 2) as f32 / w as f32,
                    cy: (chrome.y + chrome.h / 2) as f32 / h as f32,
                    pct: 50, // the smallest a control can be = the easiest to bury
                };
                for (mode, id) in [(Mode::Field, CtrlId::Quick(0)), (Mode::Menu, CtrlId::Quick(0))] {
                    assert!(
                        !would_accept_place(mode, w, h, id, p, &t),
                        "{w}x{h} {mode:?}: {id:?} must not be placeable under the editor chrome {chrome:?}"
                    );
                }
            }
        }
    }

    /// ★ …and the chrome lives in the TOP band (2026-08-03), clear of every control's
    /// built-in seat and of our own gauge block, so a drag across the middle and lower
    /// screen can never reach SAVE / CANCEL / RESET by accident.
    #[test]
    fn the_editor_chrome_sits_in_the_free_top_band() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let screen = Rect::new(0, 0, w, h);
            let items = edit_items(w, h);
            assert_eq!(items.len(), 6, "FLIP HIDE SHOW RESET CANCEL SAVE");
            let gauges = gauge_block(screen);
            let map = minimap_panel(w, h);
            for it in &items {
                assert!(it.inside(&screen), "{w}x{h}: chrome {it:?} offscreen");
                // Top band: entirely above the vertical midpoint, in fact well above.
                assert!(it.y + it.h < h / 4, "{w}x{h}: chrome {it:?} is not in the top band");
                assert!(!it.intersects(&gauges), "{w}x{h}: chrome {it:?} covers the gauges {gauges:?}");
                assert!(!it.intersects(&map), "{w}x{h}: chrome {it:?} covers the minimap panel {map:?}");
                // ★ The clearance that matters: the chrome is hit-tested FIRST, so a
                // control sitting under it could never be picked up again.
                for mode in modes() {
                    for id in ids_for(mode) {
                        let b = builtin(*id, w, h);
                        assert!(!it.intersects(&b), "{w}x{h} {mode:?}: chrome {it:?} covers {id:?}'s seat {b:?}");
                    }
                }
            }
            // The old bottom-centre band is free again — that is the whole point.
            let bottom = Rect::new(0, h * 3 / 4, w, h / 4);
            assert!(items.iter().all(|i| !i.intersects(&bottom)), "{w}x{h}: chrome still in the bottom band");
        }
    }

    /// ★ Player-reported 2026-08-03: "설정에서 버튼 크기를 바꾸면 기존 위치에 있던 버튼이
    /// 갑자기 자동으로 밀려난다". Nothing but the control under the finger may move —
    /// and since 2026-08-07 (free overlap) that resolves the friendly way: dragging
    /// one control onto another is simply ACCEPTED, and the one underneath stays
    /// exactly where it was.
    #[test]
    fn an_edit_never_displaces_another_control() {
        let (w, h) = (2340u32, 1080u32);
        // CLR parked in open space on the left, honoured.
        let clr = Place { cx: 0.30, cy: 0.55, pct: 100 };
        let mut t = Table::default();
        t.set(CtrlId::Cancel, clr);
        let clr_rect = clr.to_rect(builtin(CtrlId::Cancel, w, h), w, h);
        assert_eq!(
            resolve(Mode::Field, w, h, &t).into_iter().find(|(c, _)| *c == CtrlId::Cancel).unwrap().1,
            clr_rect,
            "setup: CLR must actually be honoured where it was put"
        );
        // Drag the MINIMAP (index 0 — it resolves first) right on top of it: accepted,
        // honoured, and CLR does not move an inch.
        let onto = Place { cx: clr.cx, cy: clr.cy, pct: 100 };
        assert!(
            would_accept_place(Mode::Field, w, h, CtrlId::Minimap, onto, &t),
            "overlapping CLR must now be accepted"
        );
        let mut probe = t.clone();
        probe.set(CtrlId::Minimap, onto);
        let after = resolve(Mode::Field, w, h, &probe);
        assert_eq!(
            after.iter().find(|(c, _)| *c == CtrlId::Cancel).unwrap().1,
            clr_rect,
            "CLR must keep its place under the minimap"
        );
        assert_eq!(
            after.iter().find(|(c, _)| *c == CtrlId::Minimap).unwrap().1,
            onto.to_rect(builtin(CtrlId::Minimap, w, h), w, h),
            "the minimap must be honoured where it was dropped"
        );
    }

    /// ★ The left-handed requirement (player 2026-07-31: "왼손잡이를 배려하는 스킬이나
    /// OK 등 버튼들이 좌측에도 나열될 수 있게"). Every field control must be placeable in
    /// the LEFT half — which was impossible while the editor's own chrome (a no-go
    /// area) ran down the left edge.
    #[test]
    fn every_control_can_be_placed_on_the_left() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            // One at a time: every control must be able to live in the left half.
            // (Asking for ALL of them stacked in one left column is not a fair test —
            // the OK disc alone is 270 px and the set would not fit anywhere.)
            for id in ids_for(Mode::Field) {
                let mut t = Table::default();
                let p = Place { cx: 0.16, cy: 0.55, pct: 100 };
                assert!(
                    would_accept_place(Mode::Field, w, h, *id, p, &t),
                    "{w}x{h}: {id:?} must be placeable in the left half"
                );
                t.set(*id, p);
                let got = resolve(Mode::Field, w, h, &t).into_iter().find(|(c, _)| c == id).unwrap().1;
                assert_eq!(got, p.to_rect(builtin(*id, w, h), w, h), "{w}x{h}: {id:?} was dropped");
                assert!(got.x + got.w / 2 < w / 2, "{w}x{h}: {id:?} did not end up on the left");
            }
        }
    }

    /// FLIP mirrors the whole mode in one tap, and the result is a legal layout that
    /// actually sits on the other side.
    #[test]
    fn flip_moves_the_cluster_to_the_other_side() {
        let (w, h) = (2340u32, 1080u32);
        let before = resolve(Mode::Field, w, h, &Table::default());
        let flipped = mirror_table(&Table::default(), Mode::Field, w, h);
        let after = resolve(Mode::Field, w, h, &flipped);
        // ★ EVERY control must SWAP SIDES — not just most of them. A straight mirror
        // left 3 of 9 behind (the lower ring seats land in the chrome band), which read
        // as a broken FLIP on the device. The stick starts on the LEFT, so a left-handed
        // flip has to send it RIGHT, out from under the buttons that just moved left.
        for (id, b) in &before {
            if *id == CtrlId::Minimap {
                continue; // the game's own buff band owns the top-left corner
            }
            let a = after.iter().find(|(c, _)| c == id).unwrap().1;
            let (was_right, is_right) = (b.x + b.w / 2 > w / 2, a.x + a.w / 2 > w / 2);
            assert_ne!(was_right, is_right, "{id:?} did not change sides on FLIP ({b:?} -> {a:?})");
        }
        // Still on screen. (Pairwise disjointness is no longer a rule — controls may
        // overlap since 2026-08-07 — so it is no longer asserted here.)
        let screen = Rect::new(0, 0, w, h);
        for (na, ra) in after.iter() {
            assert!(ra.inside(&screen), "{na:?} offscreen after flip");
        }
    }

    /// ★★★ THE TWO MODES SHARE ONE ARRANGEMENT (2026-09-10). The exact inverse of the
    /// test that stood here — `the_two_modes_are_independent`, "that is the whole point
    /// of the 게임진행 / 메뉴오픈 switch" — because the switch and the second table are
    /// gone: the player asked for what they arrange on the one page to be what a game
    /// window gets ("메뉴 모드일때도 전부 동일한 레이아웃으로 나오면돼").
    ///
    /// The request has to be one the MENU obstacle set also accepts, or the comparison
    /// would only prove that the resolver refuses things; a plate nudged UP from its own
    /// ring seat stays on the right-hand edge, clear of the gear row and the pull-out
    /// keypad, so both modes can honour it.
    #[test]
    fn the_two_modes_share_one_arrangement() {
        let (w, h) = (2340u32, 1080u32);
        let seat = builtin(CtrlId::Quick(0), w, h);
        let base = Place::from_rect(seat, seat, w, h);
        let want = Place { cy: base.cy - 0.05, ..base };
        let mut t = Table::default();
        t.set(CtrlId::Quick(0), want);
        let landed = want.to_rect(seat, w, h);
        assert_ne!(landed, seat, "the edit has to move the plate, or this proves nothing");
        for mode in modes() {
            let got = resolve(mode, w, h, &t).into_iter().find(|(c, _)| *c == CtrlId::Quick(0)).unwrap().1;
            assert_eq!(got, landed, "{mode:?} must honour the ONE stored seat");
        }
        // …and the walk seat IS in the menu set since 91차: a window's cursor arrows are
        // cut from it, so sharing the table is exactly what puts them where the player
        // put the stick.
        assert!(ids_for(Mode::Menu).contains(&CtrlId::Stick));
    }

    /// A `menu …` line from a layout file written before 2026-09-10 is DROPPED, and the
    /// field lines beside it are not. That drop is the migration: the two sections' seats
    /// were already identical by then, so the menu controls simply follow the arrangement
    /// on the page that is left.
    #[test]
    fn a_legacy_menu_line_is_dropped_and_takes_nothing_with_it() {
        let text = "field ok 0.5000 0.2500 120\nmenu q0 0.3500 0.5000 200\nfield q3 0.1000 0.9000 70\n";
        let t = parse_table(text);
        assert_eq!(t.get(CtrlId::Ok).unwrap().pct, 120, "a field line before the menu line survives");
        assert_eq!(t.get(CtrlId::Quick(3)).unwrap().pct, 70, "and one after it survives too");
        // The plate the dead line named keeps its built-in seat: nothing adopted that
        // request, in either mode.
        assert!(t.get(CtrlId::Quick(0)).is_none(), "the menu line must not be adopted as a field seat");
        // And what we write back never mentions the old section again.
        assert!(!t.to_text().contains("menu "), "the rewritten file has one section");
    }

    /// The layout file round-trips, and one bad line does not take the rest with it.
    #[test]
    fn the_layout_file_round_trips_and_tolerates_damage() {
        let mut t = Table::default();
        t.set(CtrlId::Ok, Place { cx: 0.5, cy: 0.25, pct: 120 });
        t.set(CtrlId::Quick(3), Place { cx: 0.1, cy: 0.9, pct: 70 });
        let back = parse_table(&t.to_text());
        assert_eq!(back.get(CtrlId::Ok).unwrap().pct, 120);
        assert_eq!(back.get(CtrlId::Quick(3)).unwrap().pct, 70);
        assert!((back.get(CtrlId::Ok).unwrap().cy - 0.25).abs() < 0.001);

        // `menu p0` = a potion button in menu mode. Two reasons to drop it now, either
        // of which is enough: potions are FIELD-only, and since 2026-09-10 EVERY `menu`
        // line is superseded (see `a_legacy_menu_line_is_dropped_and_takes_nothing_with_it`).
        // `menu q0` was the example here before the plates gained menu seats in 2026-08-03.
        let damaged = format!("# comment\n{}garbage\nfield ok NOTANUMBER 0.5 100\nmenu p0 0.5 0.5 100\nfield map 9.9 0.5 100\n", t.to_text());
        let survived = parse_table(&damaged);
        assert_eq!(survived.get(CtrlId::Ok).unwrap().pct, 120, "good lines must survive bad ones");
        assert!(survived.get(CtrlId::Potion(0)).is_none(), "a control that does not exist in that mode is dropped");
        assert!(survived.get(CtrlId::Minimap).is_none(), "an out-of-range fraction is dropped");
    }

    /// The ring never comes back empty — the failure mode that used to delete CLR
    /// and every quickbar plate at once.
    #[test]
    fn the_ring_always_has_a_seat_per_plate_plus_clr() {
        for (w, h) in WINDOWS.iter().copied().chain([(640u32, 360u32), (960, 400), (480, 320)]) {
            if w <= h {
                continue;
            }
            let seats = ring_all(w, h, pad_of(w, h));
            assert_eq!(seats.len(), RING_SEATS, "{w}x{h} ring lost seats");
            let screen = Rect::new(0, 0, w, h);
            for (i, s) in seats.iter().enumerate() {
                assert!(s.w > 0 && s.h > 0, "{w}x{h} seat {i} is empty");
                assert!(s.inside(&screen), "{w}x{h} seat {i} {s:?} offscreen");
            }
        }
    }

    #[test]
    fn logical_matches_window_aspect_within_a_pixel() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let (lw, lh) = init_logical(w, h, true, true, 240);
            // The whole point: filling the window must not distort — the ASPECT is
            // what has to match, and it must keep matching after the popup-safe
            // width clamp (which is why the height is derived from the width).
            let want = f64::from(w) / f64::from(h);
            let got = f64::from(lw) / f64::from(lh);
            assert!((got - want).abs() <= 0.02, "{w}x{h}: logical {lw}x{lh} aspect {got} vs {want}");
            assert!(lh <= 240, "{w}x{h}: height never exceeds the base ({lh})");
            assert!(lw <= POPUP_SAFE_LOGICAL_W, "{w}x{h}: width {lw} must stay popup-safe");
        }
    }

    /// The player-reported "register list appears at the far left" bug: the game's
    /// popup renderer truncates its interior origin to 8 bits, so the logical width
    /// must stay inside the safe envelope. 518 is the last safe width for the skill
    /// grid; the player's phone was producing 520.
    #[test]
    fn popup_interior_never_wraps_at_the_capped_width() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            for base in BASE_H_CHOICES {
                let (lw, _) = init_logical(w, h, true, true, base);
                for col in 0..3 {
                    let x = popup_interior_x(lw, col);
                    assert!(x <= 255, "{w}x{h} base {base}: logical {lw} col {col} interior {x} wraps");
                }
            }
        }
        // The bug is real and this is where it starts: 520 (what a 19.5:9 phone
        // produced before the cap) wraps, 518 does not.
        assert_eq!(popup_interior_x(518, 0), 255, "518 is the last safe width");
        assert!(popup_interior_x(520, 0) > 255, "520 wraps — the reported bug");
        // …and the original phone resolution could never hit it.
        for col in 0..3 {
            assert!(popup_interior_x(240, col) <= 255, "240 is unreachable for the bug");
        }
    }

    #[test]
    fn portrait_preference_keeps_original_resolution() {
        assert_eq!(init_logical(1080, 2340, true, false, 240), (CONTENT_W, CONTENT_H));
    }

    #[test]
    fn pre_rotation_portrait_window_still_picks_the_wide_size() {
        // The device race: first `resumed` reports 1080x2340 even though the
        // activity is about to settle into landscape. Both orders must agree.
        assert_eq!(
            init_logical(1080, 2340, true, true, 240),
            init_logical(2340, 1080, true, true, 240),
            "logical size must not depend on when the rotation lands"
        );
        // 19.5:9 wants 520x240, which wraps the game's popup; the cap trims it to
        // 496 and re-derives the height so the fill stays 1:1.
        assert_eq!(init_logical(1080, 2340, true, true, 240), (496, 228));
    }

    #[test]
    fn immersive_off_keeps_original_resolution() {
        assert_eq!(init_logical(2340, 1080, false, true, 240), (CONTENT_W, CONTENT_H));
    }

    #[test]
    fn logical_width_is_even_and_clamped() {
        for base in BASE_H_CHOICES {
            let (lw, _) = init_logical(2340, 1080, true, true, base);
            assert_eq!(lw % 2, 0, "even width for base {base}");
            assert!((LOGICAL_W_MIN..=POPUP_SAFE_LOGICAL_W).contains(&lw));
        }
        // Absurd aspect still clamps instead of overflowing.
        let (lw, _) = init_logical(100_000, 100, true, true, 320);
        assert_eq!(lw, POPUP_SAFE_LOGICAL_W);
    }

    #[test]
    fn base_height_choices_trade_fill_for_zoom() {
        // Smaller base height => narrower logical width => more maps fill the
        // screen (Hero3 map widths: 176..1120 px, median 512). Every choice must
        // still make a DIFFERENCE on the phone this is tuned for — the old top
        // choices all clamped to the same width once the popup-safe cap landed.
        let widths: Vec<u32> = BASE_H_CHOICES.iter().map(|b| init_logical(2340, 1080, true, true, *b).0).collect();
        assert!(widths.windows(2).all(|w| w[0] < w[1]), "monotonic + distinct: {widths:?}");
        assert!(widths[0] < 512, "smallest choice fills the median map: {widths:?}");
        assert!(*widths.last().unwrap() <= POPUP_SAFE_LOGICAL_W, "top choice stays popup-safe: {widths:?}");
    }

    #[test]
    fn game_rect_is_the_whole_window() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            assert_eq!(Rect::new(0, 0, w, h), Rect::new(0, 0, w, h), "{w}x{h}");
        }
    }

    /// Every immersive control, in one place, so overlaps are caught as a set.
    fn controls(w: u32, h: u32) -> (Rect, Vec<Rect>, Rect, Rect) {
        let pad = pad_of(w, h);
        (
            action_disc(w, h, pad),
            skill_slots(w, h, 5, pad),
            cancel_key(w, h, pad),
            minimap_panel(w, h),
        )
    }

    #[test]
    fn every_control_is_onscreen_and_disjoint() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let screen = Rect::new(0, 0, w, h);
            let (disc, slots, _seat0, mm) = controls(w, h);
            // ★ The CLR BUTTON is deliberately bigger than its ring seat since
            // 2026-08-07 (secondary-button sized), so it may kiss the disc or a
            // plate — overlap between controls is allowed now. It only has to be on
            // screen; the OTHER built-ins still keep their historic disjointness.
            let clr = builtin(CtrlId::Cancel, w, h);
            assert!(clr.inside(&screen), "{w}x{h} clr {clr:?} offscreen");
            let mut all: Vec<(&str, Rect)> = vec![("action", disc), ("minimap", mm)];
            for (i, s) in slots.iter().enumerate() {
                all.push((["slot0", "slot1", "slot2", "slot3", "slot4"][i], *s));
            }
            for (na, a) in &all {
                assert!(a.inside(&screen), "{w}x{h} {na} {a:?} offscreen");
                for (nb, b) in &all {
                    if na != nb {
                        assert!(!a.intersects(b), "{w}x{h} {na} overlaps {nb}");
                    }
                }
            }
        }
    }

    #[test]
    fn ring_encircles_the_action_disc() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let (disc, _, _, _) = controls(w, h);
            let ring = ring_all(w, h, pad);
            assert_eq!(ring.len(), RING_SEATS, "{w}x{h} ring = CLR + one seat per plate");
            let ar = f64::from(disc.w) / 2.0;
            let (acx, acy) = (f64::from(disc.x) + ar, f64::from(disc.y) + ar);
            let radii: Vec<f64> = ring
                .iter()
                .map(|r| {
                    let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
                    let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
                    ((cx - acx).powi(2) + (cy - acy).powi(2)).sqrt()
                })
                .collect();
            // Equidistant from the disc centre => it reads as a ring, not a column.
            let (lo, hi) = (
                radii.iter().cloned().fold(f64::MAX, f64::min),
                radii.iter().cloned().fold(0.0, f64::max),
            );
            assert!(hi - lo < 2.0, "{w}x{h} ring radii not equal: {radii:?}");
            // Outside the disc, and the sweep really wraps (spans > 90 degrees).
            assert!(lo > ar, "{w}x{h} ring inside the disc");
            let ang = |r: &Rect| {
                let cx = f64::from(r.x) + f64::from(r.w) / 2.0;
                let cy = f64::from(r.y) + f64::from(r.h) / 2.0;
                (acy - cy).atan2(cx - acx).to_degrees()
            };
            let spread = ang(&ring[0]) - ang(&ring[5]);
            assert!(spread.abs() > 90.0, "{w}x{h} ring spread only {spread} deg");
        }
    }

    #[test]
    fn cancel_key_is_the_first_ring_seat() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let ring = ring_all(w, h, pad);
            assert_eq!(cancel_key(w, h, pad), ring[0], "{w}x{h} CLR not ring seat 0");
            // Seat 0 is the clockwise neighbour of slot 1 (lower / more to the right).
            assert!(ring[0].y >= ring[1].y, "{w}x{h} CLR should sit below slot 1");
            // ★ 2026-08-07: the CLR BUTTON is secondary-attack sized, so a small default
            // CLR can never come back by accident.
            let clr = builtin(CtrlId::Cancel, w, h);
            let sec = secondary_button(w, h);
            assert_eq!(clr.w, clr.h, "{w}x{h} CLR must stay a disc");
            assert_eq!(clr.w, sec.w.max(ring[0].w), "{w}x{h} CLR must be secondary-button sized");
            assert!(clr.x + clr.w <= w && clr.y + clr.h <= h, "{w}x{h} CLR {clr:?} runs off screen");
        }
    }

    /// ★★★ 80차-H — THE INVARIANT THIS FILE DID NOT HAVE, and the bug it let through.
    ///
    /// Player, 2026-09-05: *"늘어난 스킬 퀵슬롯 때문에 취소키랑 약간 겹치는데"*. CLR is
    /// SECONDARY-BUTTON sized (162 px at 2220x1080) while its old seat — ring seat 0 —
    /// is a PLATE (86 px) and is the ring's RIGHTMOST. So the disc ran off the right
    /// edge, the old `.min(win_w - d)` clamp pushed it back left, and it landed 30 px
    /// inside plate 1. Both are drawn as DISCS, so the test is circle-to-circle: a rect
    /// test called the un-clamped pair "clear" by one pixel while the strokes touched.
    ///
    /// ⛔ Assert it against the RESOLVED layout, not just the built-in: the resolver is
    /// allowed to move a control, and what the player sees is where it ends up.
    #[test]
    fn the_cancel_disc_never_touches_a_quickbar_plate() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let clr = builtin(CtrlId::Cancel, w, h);
            let ring = ring_all(w, h, pad);
            let cr = f64::from(clr.w.min(clr.h)) / 2.0;
            let (ccx, ccy) = (f64::from(clr.x) + f64::from(clr.w) / 2.0, f64::from(clr.y) + f64::from(clr.h) / 2.0);
            for (i, p) in ring.iter().enumerate().skip(1) {
                let pr = f64::from(p.w.min(p.h)) / 2.0;
                let (pcx, pcy) = (f64::from(p.x) + f64::from(p.w) / 2.0, f64::from(p.y) + f64::from(p.h) / 2.0);
                let dist = ((ccx - pcx).powi(2) + (ccy - pcy).powi(2)).sqrt();
                assert!(
                    dist >= cr + pr,
                    "{w}x{h} CLR {clr:?} overlaps plate {i} {p:?} — centres {dist} apart, radii {cr}+{pr}"
                );
            }
            // …and it is BETWEEN plate 1 and OK, which is what the player asked for:
            // below the plate (toward OK) and no further right than the window allows.
            let q1 = ring[1];
            assert!(ccy > f64::from(q1.y) + f64::from(q1.h) / 2.0, "{w}x{h} CLR is not below plate 1");
            let ok = action_disc(w, h, pad);
            assert!(
                ccy < f64::from(ok.y) + f64::from(ok.h) / 2.0,
                "{w}x{h} CLR went past the OK disc's centre — that is not 'between'"
            );
            // ⛔ And it must not swallow OK either.
            let (okcx, okcy) = (f64::from(ok.x) + f64::from(ok.w) / 2.0, f64::from(ok.y) + f64::from(ok.h) / 2.0);
            let d = ((ccx - okcx).powi(2) + (ccy - okcy).powi(2)).sqrt();
            assert!(d >= cr + f64::from(ok.w) / 2.0, "{w}x{h} CLR {clr:?} overlaps the OK disc {ok:?}");
        }
    }

    /// ...and it still does not, at EVERY plate count the player can produce by
    /// switching quickslots off (2026-09-10: "퀵슬롯이 너무많아서 불편하다는 유저도있어서").
    ///
    /// ⛔ This is the test the feature needed and this file did not have. Hiding a plate
    /// removes a ring SEAT, so `ring_slots` re-spaces the arc and takes a bigger side —
    /// which moves plate 1, the very thing CLR measures its clearance from. 80차-E's
    /// overlap was created by going from five plates to seven; going the other way is
    /// the same machine in reverse, and nothing was watching it.
    ///
    /// One plate visible is a real state (`quickpage::hide_plate` refuses only the LAST
    /// one), and it is the extreme: the ring is two seats over the whole arc.
    #[test]
    fn the_cancel_disc_never_touches_a_plate_at_any_visible_count() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            for visible in 1..=PLATES {
                let clr = cancel_seat_visible(w, h, pad, visible);
                let ring = ring_all_visible(w, h, pad, visible);
                assert_eq!(ring.len(), 1 + visible, "{w}x{h} v{visible}: seat count");
                let cr = f64::from(clr.w.min(clr.h)) / 2.0;
                let (ccx, ccy) = (f64::from(clr.x) + f64::from(clr.w) / 2.0, f64::from(clr.y) + f64::from(clr.h) / 2.0);
                for (i, p) in ring.iter().enumerate().skip(1) {
                    let pr = f64::from(p.w.min(p.h)) / 2.0;
                    let (pcx, pcy) = (f64::from(p.x) + f64::from(p.w) / 2.0, f64::from(p.y) + f64::from(p.h) / 2.0);
                    let dist = ((ccx - pcx).powi(2) + (ccy - pcy).powi(2)).sqrt();
                    assert!(
                        dist >= cr + pr,
                        "{w}x{h} v{visible}: CLR {clr:?} overlaps plate {i} {p:?} — centres {dist} apart, radii {cr}+{pr}"
                    );
                }
                // Still BETWEEN plate 1 and OK, and still not swallowing OK.
                let q1 = ring[1];
                assert!(
                    ccy > f64::from(q1.y) + f64::from(q1.h) / 2.0,
                    "{w}x{h} v{visible}: CLR is not below the first visible plate"
                );
                let ok = action_disc(w, h, pad);
                let (okcx, okcy) = (f64::from(ok.x) + f64::from(ok.w) / 2.0, f64::from(ok.y) + f64::from(ok.h) / 2.0);
                let d = ((ccx - okcx).powi(2) + (ccy - okcy).powi(2)).sqrt();
                assert!(
                    d >= cr + f64::from(ok.w) / 2.0,
                    "{w}x{h} v{visible}: CLR {clr:?} overlaps the OK disc {ok:?}"
                );
            }
        }
    }

    /// Fewer plates must mean BIGGER plates, never smaller — that is the whole visible
    /// half of the feature. And the ceiling still holds: `ring_slots` starts from
    /// `win_h / 11` and only ever shrinks from there, so a ring that already fitted is
    /// untouched (79차-E).
    #[test]
    fn switching_plates_off_never_makes_the_survivors_smaller() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let ceiling = (h / 11).clamp(40, 120);
            let mut prev = 0;
            for visible in (1..=PLATES).rev() {
                let ring = ring_all_visible(w, h, pad, visible);
                let side = ring[1].w;
                assert!(side >= prev, "{w}x{h}: {visible} plates are smaller than {} were", visible + 1);
                assert!(side <= ceiling, "{w}x{h} v{visible}: {side} is past the {ceiling} ceiling");
                assert!(side > 0);
                // Every seat is fully on screen at every count — the property
                // `ring_slots` shrinks to obtain.
                for (i, r) in ring.iter().enumerate() {
                    assert!(
                        r.x + r.w <= w && r.y + r.h <= h,
                        "{w}x{h} v{visible}: seat {i} {r:?} is off screen"
                    );
                }
                prev = side;
            }
        }
    }

    /// The measured numbers, so a future edit to the pitch cap cannot move them
    /// silently. 2220x1080 is both dev devices (S9 and S25 Ultra are the same window).
    ///
    /// ★ `k7 == 86` is the load-bearing one: nothing hidden has to be the layout that
    /// shipped, to the pixel. `k5 == 98` is the five-plate ring that shipped before
    /// 79차 — reaching it again by hiding two plates is what the cap is FOR.
    #[test]
    fn the_plate_grows_by_a_measured_amount_as_plates_are_switched_off() {
        let (w, h) = (2220u32, 1080u32);
        let pad = pad_of(w, h);
        let side = |visible: usize| ring_all_visible(w, h, pad, visible)[1].w;
        assert_eq!(side(7), 86, "nothing hidden must be the shipped ring");
        assert_eq!(side(7), ring_all(w, h, pad)[1].w, "…which is the full ring itself");
        assert_eq!(side(6), 97);
        assert_eq!(side(5), 98, "the six-seat ring that shipped before 79차");
        for visible in 1..=4 {
            assert_eq!(side(visible), 98, "at the ceiling from five plates down");
        }
        // The first plate stays next to the action disc instead of drifting up the arc:
        // its x MOVES LEFT only as far as the bigger plate needs.
        let x = |visible: usize| ring_all_visible(w, h, pad, visible)[1].x;
        assert_eq!((x(7), x(6), x(5), x(1)), (2008, 1981, 1952, 1952));
    }

    /// Top-right band, but tucked LEFT of the game's own target-monster box (which
    /// is right-edge anchored) instead of hugging the screen edge — it used to cover
    /// the monster's name and HP bar.
    #[test]
    fn minimap_panel_is_top_right_of_the_free_band() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let (lw, lh) = init_logical(w, h, true, true, 240);
            let mm = minimap_panel_in(w, h, lw, lh);
            let target = game_target_box(Rect::new(0, 0, w, h), lw, lh);
            // EXTREME top-right corner, with the game's own (poked) target box
            // immediately to its LEFT - not stacked above or below it.
            let pad = pad_of(w, h);
            assert_eq!(mm.x + mm.w + pad * 2, w, "{w}x{h} panel {mm:?} not flush right");
            assert_eq!(mm.y, pad * 2, "{w}x{h} panel {mm:?} not flush top");
            assert!(!mm.intersects(&target), "{w}x{h} panel {mm:?} overlaps {target:?}");
            assert!(
                target.x + target.w <= mm.x,
                "{w}x{h} target box {target:?} must end left of the panel {mm:?}"
            );
            // Same row: the box must not have been pushed below the panel.
            assert!(target.y < mm.y + mm.h, "{w}x{h} box dropped below the panel");
        }
    }

    /// The '#' plate (the lowest ring seat) was sitting hard against the bottom
    /// edge on device; it must keep a visible margin.
    #[test]
    fn lowest_ring_seat_clears_the_bottom_edge() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let ring = ring_all(w, h, pad_of(w, h));
            let lowest = ring.iter().map(|r| r.y + r.h).max().unwrap();
            assert!(h - lowest >= h / 40, "{w}x{h} lowest seat ends at {lowest} of {h}");
        }
    }

    /// ★★★ 91차-B — the stick IS the arrows' square: its base is centred on the seat and
    /// bottom-left with it. Player: "화살표 전체가 조이스틱 전체고 조이스틱 전체가 화살표
    /// 전체인 느낌". (The pre-91차 assertion here, `hr <= h / 10`, was the small 90 px hint.)
    ///
    /// ★★★ 2026-09-20 the RIM came in one notch: from the seat's inscribed circle (312 px
    /// at 1080) to the circle through the four hexagons' centres (194 px) — "화살표
    /// 크기보다 조이스틱이 조금 더 작아야해". Asserted against `dpad_arms` itself rather
    /// than against a second copy of the formula.
    #[test]
    fn the_stick_base_runs_through_the_arrow_centres() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let seat = stick_seat(w, h);
            let (hx, hy, hr) = joy_home_builtin(w, h);
            for arm in dpad_arms(seat) {
                let ax = f64::from(arm.draw.x) + f64::from(arm.draw.w) / 2.0;
                let ay = f64::from(arm.draw.y) + f64::from(arm.draw.h) / 2.0;
                let d = (ax - hx).hypot(ay - hy);
                // ⛔ Not exact: `dpad_arms` places each hexagon with INTEGER maths, so an
                // odd width leaves the arm's box a half pixel off the seat's axis and the
                // centre lands ~0.001 px off the rim. One pixel of slack, not `1e-9`.
                assert!((d - hr).abs() < 1.0, "{w}x{h}: arrow centre at {d} is not on the base rim {hr}");
            }
            assert!(hr < f64::from(seat.w.min(seat.h)) / 2.0, "{w}x{h}: the base must be SMALLER than the arrows");
            assert_eq!((hx, hy), (f64::from(seat.x) + f64::from(seat.w) / 2.0, f64::from(seat.y) + f64::from(seat.h) / 2.0));
            assert!(hx - hr >= 0.0 && hy + hr <= f64::from(h), "{w}x{h} stick off screen");
            assert!(hx < f64::from(w) / 4.0 && hy > f64::from(h) * 0.6, "{w}x{h} stick not bottom-left");
        }
    }

    // ── menu-mode controls + pull-out keypad (player feedback 2026-07-30) ────

    /// ★ 2026-08-03 the menu keys were made to sit BESIDE the quickbar rather than take
    /// its seats, and 79차-B removed them altogether. What survives from that test is the
    /// half that is still a rule: **a control the player is already reaching for must not
    /// move when a window opens** — the plates resolve to the same rects in both modes.
    #[test]
    fn the_plates_do_not_move_between_the_field_and_a_window() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let screen = Rect::new(0, 0, w, h);
            assert!(builtin(CtrlId::Cancel, w, h).inside(&screen), "{w}x{h} clr offscreen");
            let empty = Table::default();
            for i in 0..PLATES as u8 {
                let f = resolve(Mode::Field, w, h, &empty).into_iter().find(|(c, _)| *c == CtrlId::Quick(i)).unwrap().1;
                let m = resolve(Mode::Menu, w, h, &empty).into_iter().find(|(c, _)| *c == CtrlId::Quick(i)).unwrap().1;
                assert_eq!(f, m, "{w}x{h} plate {i} moved between modes");
                assert!(f.inside(&screen), "{w}x{h} plate {i} {f:?} offscreen");
            }
        }
    }

    /// The Lv/HP block became a tap target that opens the main menu (2026-08-03 r2).
    /// It is not a placeable control, so the invariant that matters is that it does
    /// not silently swallow touches meant for something else: it must stay a thin
    /// TOP-LEFT strip, clear of every field control, and clear of the editor chrome
    /// (which is hit-tested first and sits in the same top band).
    #[test]
    fn the_gauge_tap_target_does_not_swallow_other_controls() {
        let empty = Table::default();
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let zone = stats_tap_box(w, h);
            assert!(zone.inside(&Rect::new(0, 0, w, h)), "{w}x{h} tap zone offscreen: {zone:?}");
            // Top-left strip: well inside the upper band and the left half.
            assert!(zone.y + zone.h < h / 3, "{w}x{h} tap zone reaches too far down: {zone:?}");
            assert!(zone.x + zone.w < w * 2 / 3, "{w}x{h} tap zone reaches too far right: {zone:?}");
            for (id, r) in resolve(Mode::Field, w, h, &empty) {
                assert!(!zone.intersects(&r), "{w}x{h} gauge tap zone covers {id:?}");
            }
            for r in edit_items(w, h) {
                assert!(!zone.intersects(&r), "{w}x{h} gauge tap zone covers the editor chrome");
            }
        }
    }

    /// ★ 2026-08-07 ("겹칠 수 있게 바꿔줘"): control-vs-control has NO rule left —
    /// even fully stacking one plate on another is accepted, and both keep their
    /// rects. What still refuses is an OBSTACLE (chrome, keypad, editor buttons):
    /// those are hit-tested before the button list, so a control there silently
    /// does something else. Both halves pinned so neither can regress by accident.
    #[test]
    fn controls_overlap_freely_but_obstacles_still_refuse() {
        let (w, h) = (2340u32, 1080u32);
        let t = Table::default();
        // Fully stack plate q1 on plate q0's stored spot, in open space.
        let spot = Place { cx: 0.30, cy: 0.55, pct: 100 };
        let mut both = t.clone();
        both.set(CtrlId::Quick(0), spot);
        assert!(
            would_accept_place(Mode::Field, w, h, CtrlId::Quick(1), spot, &both),
            "fully stacking two plates must now be accepted"
        );
        both.set(CtrlId::Quick(1), spot);
        let got = resolve(Mode::Field, w, h, &both);
        let q0 = got.iter().find(|(c, _)| *c == CtrlId::Quick(0)).unwrap().1;
        let q1 = got.iter().find(|(c, _)| *c == CtrlId::Quick(1)).unwrap().1;
        assert_eq!(q0, spot.to_rect(builtin(CtrlId::Quick(0), w, h), w, h), "q0 must be honoured");
        assert_eq!(q1, spot.to_rect(builtin(CtrlId::Quick(1), w, h), w, h), "q1 must be honoured on top of q0");
        // …but the chrome band still refuses (the strict obstacle rule is intact).
        let chrome = edit_items(w, h)[0];
        let on_chrome = Place {
            cx: (chrome.x + chrome.w / 2) as f32 / w as f32,
            cy: (chrome.y + chrome.h / 2) as f32 / h as f32,
            pct: 50,
        };
        assert!(
            !would_accept_place(Mode::Field, w, h, CtrlId::Quick(1), on_chrome, &t),
            "an obstacle must still refuse a control"
        );
    }

    /// ★ THE invariant behind the gauge tap zone's size, and the one an adversarial
    /// review of the original `grown(pad*2)` caught (2026-08-03): the zone is
    /// hit-tested BEFORE `layout.buttons`, so every pixel it owns is a pixel the
    /// control underneath never gets — but the only thing keeping a control out from
    /// under it is the UNGROWN gauge block in `obstacles`, which `accepts` clears at
    /// `pad`. So the rule is an implication, not a distance:
    ///
    ///     a rect the resolver would ACCEPT must never intersect the tap zone.
    ///
    /// Brute-forced rather than reasoned about, because the original bug WAS the
    /// reasoning: `grown(pad*2)` left a pad-wide annulus of accepted-but-swallowed
    /// placements, and tapping the outer sliver of an OK disc parked there opened the
    /// main menu instead of attacking.
    #[test]
    fn the_gauge_tap_zone_never_shadows_a_legal_control() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let block = gauge_block(Rect::new(0, 0, w, h));
            let zone = stats_tap_box(w, h);
            let mut checked = 0u32;
            // Real control sizes: a skill plate, the secondary button, the OK disc.
            for side in [98u32, 162, 270] {
                let mut cy = 0;
                while cy + side <= h.min(block.y + block.h + side + pad * 4) {
                    let mut cx = 0;
                    while cx + side <= w.min(block.x + block.w + side + pad * 4) {
                        let cand = Rect::new(cx, cy, side, side);
                        // `accepts`'s own clearance rule against this one obstacle.
                        if !grown(cand, pad).intersects(&block) {
                            checked += 1;
                            assert!(
                                !zone.intersects(&cand),
                                "{w}x{h} the gauge tap zone {zone:?} swallows part of a LEGAL control at {cand:?} \
                                 (block {block:?}, pad {pad}) — that tap would send CLEAR instead of the button's key"
                            );
                        }
                        cx += 3;
                    }
                    cy += 3;
                }
            }
            assert!(checked > 1000, "{w}x{h} sweep proved nothing: only {checked} legal candidates");
        }
    }

    /// The two unspent-point badges (moved BELOW the HP/SP/EXP stack 2026-08-05):
    /// side by side like the original's own 14x7 pair, never overlapping each other,
    /// never off-screen, never overlapping the bars themselves, and never touching
    /// the minimap panel that owns the top-right.
    #[test]
    fn point_badges_sit_below_the_bars_and_clear_the_minimap() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let game = Rect::new(0, 0, w, h);
            let bars = stat_bars(game);
            let (stat, skill) = point_badges(game);
            let mm = minimap_panel(w, h);
            for (name, b) in [("stat", stat), ("skill", skill)] {
                assert!(b.w == b.h && b.w % BADGE_SRC == 0, "{w}x{h} {name} badge {b:?} is not an integer 7x7 scale");
                assert!(b.y >= bars.y + bars.h, "{w}x{h} {name} badge {b:?} overlaps the bars {bars:?}");
                assert!(b.x + b.w <= w && b.y + b.h <= h, "{w}x{h} {name} badge {b:?} is off-screen");
                assert!(
                    !b.intersects(&mm),
                    "{w}x{h} {name} badge {b:?} collides with the minimap panel {mm:?}"
                );
            }
            assert!(stat.x + stat.w < skill.x, "{w}x{h} the two badges overlap: {stat:?} / {skill:?}");
            // Left-aligned with the bar stack, so the column reads as one block.
            assert_eq!(stat.x, bars.x, "{w}x{h} badge {stat:?} is not aligned with the bars {bars:?}");
        }
    }

    /// The no-go rect reserved for the gauge block must swallow BOTH badges, or a
    /// control is legal at level 1 and then hidden behind a badge after the first
    /// level-up. (This is what makes the existing tap-zone sweep cover them too.)
    #[test]
    fn the_gauge_no_go_rect_contains_both_point_badges() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let game = Rect::new(0, 0, w, h);
            let block = gauge_block(game);
            let (stat, skill) = point_badges(game);
            for (name, b) in [("stat", stat), ("skill", skill)] {
                assert!(
                    b.x >= block.x
                        && b.y >= block.y
                        && b.x + b.w <= block.x + block.w
                        && b.y + b.h <= block.y + block.h,
                    "{w}x{h} {name} badge {b:?} sticks out of the reserved block {block:?}"
                );
            }
        }
    }

    /// ~2 Hz, from HOST time. Pinned because the guest's own blink flag is frozen by
    /// the HUD-skip patch, so "read the game's flag" is a wrong answer that looks
    /// right; and because a frame counter would blink at a device-dependent rate.
    #[test]
    fn badge_blink_is_two_hz_from_host_time() {
        assert!(badge_blink_on(0));
        assert!(badge_blink_on(249));
        assert!(!badge_blink_on(250));
        assert!(!badge_blink_on(499));
        assert!(badge_blink_on(500));
        // One full cycle = 500 ms => 2 Hz. Stated as the period identity rather than
        // an edge count, which is what "2 Hz" actually means.
        for ms in 0..3000u128 {
            assert_eq!(badge_blink_on(ms), badge_blink_on(ms + 2 * BADGE_BLINK_MS), "period is not 500 ms at {ms}");
            assert_ne!(badge_blink_on(ms), badge_blink_on(ms + BADGE_BLINK_MS), "half-cycle is not 250 ms at {ms}");
        }
    }

    /// The potion pair (player request 2026-08-03) is a column beside the ring's
    /// lowest plate: on screen, clear of every other control, and the same size as a
    /// plate so the bottom band reads as one row of controls.
    #[test]
    fn potion_buttons_sit_beside_the_ring_and_clear_everything() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let screen = Rect::new(0, 0, w, h);
            let plate = ring_seat(w, h, pad, RING_SEATS - 1);
            let empty = Table::default();
            let field = resolve(Mode::Field, w, h, &empty);
            for i in 0..2u8 {
                let r = field.iter().find(|(c, _)| *c == CtrlId::Potion(i)).unwrap().1;
                assert_eq!((r.w, r.h), (plate.w, plate.h), "{w}x{h} potion {i} is plate-sized");
                assert!(r.inside(&screen), "{w}x{h} potion {i} offscreen");
                assert!(r.x + r.w <= plate.x, "{w}x{h} potion {i} must sit left of the ring");
            }
            // …and they are FIELD-only: a menu never offers a control that cannot work.
            assert!(!ids_for(Mode::Menu).contains(&CtrlId::Potion(0)));
        }
    }

    /// PAGE is small and sits BELOW-LEFT of the lowest plate; SAVE has left the plate row
    /// entirely for the top-right, beside the game's own monster box (player 2026-08-06).
    ///
    /// The property this still pins down is the one that produced the original report:
    /// `ring_seat(.., SLOTS + 1)` asked a six-seat ring for seat 6 and got the bottom-right
    /// CORNER fallback instead of an error, so the page button was jammed in the corner. An
    /// out-of-range seat is not a compile error and the fallback rect is perfectly valid —
    /// only geometry catches it.
    ///
    /// Asserted as properties, not pixels: smaller than a plate, on screen, disjoint from
    /// the plates and from each other, and clear of the stick's home (a control under the
    /// walking thumb steals steering, which is the same class of failure as one off the
    /// edge).
    #[test]
    fn page_is_below_left_of_the_plates_and_save_sits_by_the_monster_box() {
        for (w, h) in WINDOWS {
            // ★ Landscape only (91차). These seats exist only in the landscape-immersive
            // HUD (`enter_editor` refuses everywhere else and the portrait layouts build
            // a fixed D-pad + keypad of their own), and the walk seat that the page
            // button now yields to — the arrows' square, 756 px tall on a 1080-wide
            // portrait window — pushes the fallback onto a ring seat there. A rect no
            // layout ever draws is not a defect worth constraining the real ones for.
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let screen = Rect::new(0, 0, w, h);
            let plates: Vec<Rect> = skill_slots(w, h, RING_SEATS - 1, pad);
            let lowest = plates.iter().copied().max_by_key(|r| r.y).unwrap();
            let page = quickpage_seat(w, h, pad);
            let save = quicksave_seat(w, h, pad);
            for (n, r) in [("page", page), ("save", save)] {
                assert!(r.w <= lowest.w && r.h <= lowest.h, "{w}x{h} {n} must not be bigger than a plate");
                assert!(r.inside(&screen), "{w}x{h} {n} offscreen: {r:?}");
                for (i, p) in plates.iter().enumerate() {
                    assert!(!r.intersects(p), "{w}x{h} {n} overlaps plate {i}");
                }
                let (jx, jy, jr) = joy_home_builtin(w, h);
                let stick = Rect::new(
                    (jx - jr).max(0.0) as u32,
                    (jy - jr).max(0.0) as u32,
                    (jr * 2.0) as u32,
                    (jr * 2.0) as u32,
                );
                assert!(!r.intersects(&stick), "{w}x{h} {n} sits on the stick's home");
            }
            assert!(!page.intersects(&save), "{w}x{h} page and save overlap");
            // ★ The corner fallback would fail here: it lands to the RIGHT of every plate.
            assert!(page.x <= lowest.x, "{w}x{h} page must not be right of the plates");
            // "더 아래" — its top is at or below the lowest plate's middle.
            assert!(page.y >= lowest.y + lowest.h / 2, "{w}x{h} page must sit below the plate");

            // SAVE's whole point is the monster box, so assert against that box and not
            // against the plates: left of it, and never on top of it or of the minimap.
            let game = Rect::new(0, 0, w, h);
            let (lw, lh) = init_logical(w, h, true, true, 240);
            let map = minimap_panel(w, h);
            let target = game_target_box_at(game, lw, lh, map);
            assert!(save.x + save.w <= target.x, "{w}x{h} save must be left of the monster box");
            assert!(!save.intersects(&target), "{w}x{h} save covers the monster box");
            assert!(!save.intersects(&map), "{w}x{h} save covers the minimap panel");
        }
    }

    /// The game owns BOTH top corners: the buff/status list (top-left, absolute
    /// guest coords) and the target-monster box (top-right, right-anchored). Nothing
    /// of ours may be drawn in either — player-reported: the minimap covered the
    /// monster box and the gauges + gear + toggle covered the buff list.
    #[test]
    fn overlay_keeps_out_of_the_games_own_hud_corners() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let (lw, lh) = init_logical(w, h, true, true, 240);
            let game = Rect::new(0, 0, w, h); // immersive: game == window
            let buff = game_buff_box(game, lw, lh);
            let target = game_target_box(game, lw, lh);
            let pad = pad_of(w, h);
            // Gear + toggle are NON-FIELD only and the game's buff band / target box
            // are FIELD-only HUD, so those never coexist and are not compared here.
            let ours: [(&str, Rect); 4] = [
                ("plate", level_plate_in(game, lw, lh)),
                ("bars", stat_bars(game)),
                ("minimap", minimap_panel_in(w, h, lw, lh)),
                ("secondary", secondary_button(w, h)),
            ];
            for (n, r) in &ours {
                assert!(!r.intersects(&buff), "{w}x{h} {n} {r:?} covers the buff list {buff:?}");
                assert!(!r.intersects(&target), "{w}x{h} {n} {r:?} covers the target box {target:?}");
            }
            // The ring + action disc live in the bottom-right; assert they too stay
            // out of the top band (they would if the ring ever grew).
            let mut cluster = ring_all(w, h, pad);
            cluster.push(action_disc(w, h, pad));
            for (i, r) in cluster.iter().enumerate() {
                assert!(!r.intersects(&buff), "{w}x{h} cluster {i} covers the buff list");
                assert!(!r.intersects(&target), "{w}x{h} cluster {i} covers the target box");
            }
            // The band is moved RIGHT of our gauges, on the same row.
            let bars = stat_bars(game);
            assert!(buff.x >= bars.x + bars.w, "{w}x{h} band {buff:?} must start right of the bars {bars:?}");
            assert!(buff.y < bars.y + bars.h, "{w}x{h} band should share the gauge row");
            // Sanity on the reservations themselves at the shipped logical size.
            if (lw, lh) == (496, 228) {
                assert_eq!((buff.w, buff.h), (GUEST_BUFFLIST_W * w / 496, GUEST_BUFFLIST_H * h / 228));
                let dx = target_box_shift(game, lw) * w / 496;
                assert_eq!(target.x, w - GUEST_TARGETBOX_W * w / 496 - dx);
                assert_eq!(target.y, 0, "the box keeps the top row");
                assert!(dx > 0, "the box must actually be shifted off the corner");
            }
        }
    }

    /// The gear sits at the TOP-CENTRE, nudged slightly right (player request
    /// 2026-07-30), hugging the top edge. ★ 2026-09-18: the `123` toggle that shared the
    /// row is gone, and the gear must NOT have moved with it — the seat is the one the
    /// player has been tapping for months.
    #[test] fn gear_sits_top_centre_slightly_right(){
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let g = gear_rect(w, h);
            let pad = pad_of(w, h);
            let gs = gear_size(w, h);
            assert_eq!(g.w, gs);
            assert_eq!(g.h, gs);
            assert!(g.y >= pad * 2 && g.y <= pad * 2 + gs / 5, "{w}x{h} the gear hugs the top edge (y {})", g.y);
            // The old gear+toggle row's centre was right of the screen middle; the gear
            // was its left half, so it sits at or a little left of that centre.
            let mid = g.x + g.w / 2;
            assert!(mid > w * 45 / 100, "{w}x{h} the gear must sit near the middle-right (mid {mid})");
            assert!(mid < w * 70 / 100, "{w}x{h} 'slightly' right, not far right (mid {mid})");
            assert!(g.x + g.w + pad <= w, "{w}x{h} the gear runs off the right edge");
        }
    }

    /// Every top-edge control must be reachable: the gear is useless if the
    /// player-gauge block is painted over it (which is what the first device build
    /// did to the old keypad toggle), and the level plate must not sit on it.
    #[test]
    fn top_edge_controls_and_gauges_do_not_overlap() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let screen = Rect::new(0, 0, w, h);
            let game = Rect::new(0, 0, w, h); // immersive: game == window
            let all: [(&str, Rect); 4] = [
                ("gear", gear_rect(w, h)),
                ("minimap", minimap_panel(w, h)),
                ("plate", level_plate(game)),
                ("bars", stat_bars(game)),
            ];
            for (na, a) in &all {
                assert!(a.inside(&screen), "{w}x{h} {na} {a:?} offscreen");
                for (nb, b) in &all {
                    if na != nb {
                        assert!(!a.intersects(b), "{w}x{h} {na} {a:?} overlaps {nb} {b:?}");
                    }
                }
            }
            // …and none of them may reach the bottom-right control cluster.
            let disc = action_disc(w, h, pad_of(w, h));
            for (na, a) in &all {
                assert!(!a.intersects(&disc), "{w}x{h} {na} overlaps action disc");
            }
        }
    }

    /// The new secondary-weapon button: bigger than a skill plate, smaller than the
    /// action disc, on screen, and clear of every other control (the space directly
    /// left of the disc is the ring's, so it lands just outside the ring).
    #[test]
    fn secondary_weapon_button_fits_between_the_ring_and_the_edge() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let pad = pad_of(w, h);
            let s = secondary_button(w, h);
            let disc = action_disc(w, h, pad);
            let plate = skill_slots(w, h, 5, pad)[0];
            assert!(s.inside(&Rect::new(0, 0, w, h)), "{w}x{h} secondary {s:?} offscreen");
            assert!(s.w > plate.w, "{w}x{h} secondary {} must exceed a skill plate {}", s.w, plate.w);
            assert!(s.w < disc.w, "{w}x{h} secondary {} must stay under the disc {}", s.w, disc.w);
            assert!(!s.intersects(&disc), "{w}x{h} secondary overlaps the action disc");
            for (i, seat) in ring_all(w, h, pad).iter().enumerate() {
                assert!(!s.intersects(seat), "{w}x{h} secondary overlaps ring seat {i}");
            }
            // Level with the action disc, and to its left (what the player asked for).
            assert!(s.x + s.w < disc.x, "{w}x{h} secondary must be left of the disc");
            let (sc, dc) = (s.y + s.h / 2, disc.y + disc.h / 2);
            assert!(sc.abs_diff(dc) <= 2, "{w}x{h} secondary centre {sc} vs disc {dc}");
        }
    }

    // ── joystick: one latched direction + trailing centre ────────────────────

    #[test]
    fn deadzone_and_single_direction() {
        let r = 90.0;
        assert_eq!(joy_dir(0.0, 0.0, r, None), None, "centre = no direction");
        assert_eq!(joy_dir(10.0, 5.0, r, None), None, "inside engage deadzone");
        assert_eq!(joy_dir(0.0, -50.0, r, None), Some(Key::Up));
        assert_eq!(joy_dir(0.0, 50.0, r, None), Some(Key::Down));
        assert_eq!(joy_dir(-50.0, 0.0, r, None), Some(Key::Left));
        assert_eq!(joy_dir(50.0, 0.0, r, None), Some(Key::Right));
        // A held direction survives a smaller deflection than it takes to engage.
        assert_eq!(joy_dir(0.0, -20.0, r, Some(Key::Up)), Some(Key::Up));
        assert_eq!(joy_dir(0.0, -20.0, r, None), None);
    }

    /// THE reported bug: walking up, slide the thumb left without lifting.
    #[test]
    fn sliding_from_up_to_left_turns() {
        let r = 90.0;
        assert_eq!(joy_dir(-80.0, -5.0, r, Some(Key::Up)), Some(Key::Left), "clear left wins");
        // …but a thumb parked on the diagonal must NOT flutter between the two.
        assert_eq!(joy_dir(-70.0, -68.0, r, Some(Key::Up)), Some(Key::Up), "diagonal keeps up");
        assert_eq!(joy_dir(-70.0, -68.0, r, Some(Key::Left)), Some(Key::Left), "and keeps left");
        // Crossing the centre flips immediately (the held axis went negative).
        assert_eq!(joy_dir(5.0, 60.0, r, Some(Key::Up)), Some(Key::Down));
    }

    // ── stick size (2026-08-16) ──────────────────────────────────────────────

    /// The stick used to be POSITION ONLY: `Table::set` forced its `pct` back to
    /// `PCT_DEFAULT`, so a size written by hand or by a pinch vanished on the way
    /// in. It must survive now, or the editor's pinch has nothing to store.
    #[test]
    fn a_stored_stick_size_is_not_clobbered() {
        let (w, h) = (2340u32, 1080u32);
        let b = builtin(CtrlId::Stick, w, h);
        let mut t = Table::default();
        let want = Place {
            cx: (b.x + b.w / 2) as f32 / w as f32,
            cy: (b.y + b.h / 2) as f32 / h as f32,
            pct: 160,
        };
        t.set(CtrlId::Stick, want);
        assert_eq!(t.get(CtrlId::Stick).map(|p| p.pct), Some(160));
        // …and the size actually reaches the rect: 160 % of a 2r square.
        let got = want.to_rect(b, w, h);
        assert_eq!(got.w, scaled(b.w, 160), "stored size did not reach the rect");
        assert_eq!(got.h, scaled(b.h, 160));
    }

    /// One number drives the drawn ring AND the walk-start distance — deliberately
    /// coupled (see `CtrlId::Stick`). Reading the BUILT-IN radius here is what made
    /// the old size knob a silent no-op. TOUCH TYPE A's stick; TYPE B has no stick
    /// at all since the 2026-08-16 second round.
    ///
    /// ★ 91차-B: the seat is the arrows' square, the ring runs through the four arrow
    /// centres (`stick_base_radius`), the thumb pins INSIDE the ring (base less thumb),
    /// and the walk-start basis is the
    /// legacy radius scaled by the same percent — all four honoured or all four refused
    /// together, because they come from ONE resolved rect.
    #[test]
    fn the_ring_the_player_sees_is_the_throw_they_get() {
        let (w, h) = (2340u32, 1080u32);
        let b = builtin(CtrlId::Stick, w, h);
        // ★ 91차: the seat is the arrows' square (624 px at 1080 tall), so the top of the
        // editor's range no longer fits — 250 % is 1560 px on a 1080 px screen, and the
        // resolver refuses it exactly as it refuses any control that runs off the
        // window (the pinch simply stops growing). The honoured range is what a phone
        // can hold; the refusal is asserted below so it is a stated fact, not a surprise.
        for pct in [PCT_MIN, 100, 125] {
            let place = Place {
                cx: (b.x + b.w / 2) as f32 / w as f32,
                cy: (b.y + b.h / 2) as f32 / h as f32,
                pct,
            };
            let mut t = Table::default();
            t.set(CtrlId::Stick, place);
            LIVE.with(|live| *live.borrow_mut() = t.clone());
            let resolved = t_stick(w, h).expect("the stick always resolves to something");
            // The size is HONOURED, not quietly refused back to the built-in — at its
            // home seat this range clears every obstacle. Without this the rest of the
            // test would still pass with the feature doing nothing at all.
            assert_eq!(resolved.w, scaled(b.w, pct), "{pct}% stick fell back to its built-in size");
            let (_cx, _cy, r) = joy_home(w, h);
            assert_eq!(r, stick_base_radius(&resolved), "{pct}% ring != resolved seat");
            assert_eq!(joy_radius_immersive(w, h), r * (1.0 - JOY_THUMB), "{pct}% pin != base less thumb");
            assert_eq!(joy_engage_immersive(w, h), stick_engage_radius(&resolved, w, h), "{pct}% deadzone basis != seat's");
        }
        // A seat the window cannot hold falls back to the built-in, ring and all.
        let mut t = Table::default();
        t.set(CtrlId::Stick, Place { cx: 0.15, cy: 0.68, pct: PCT_MAX });
        LIVE.with(|live| *live.borrow_mut() = t.clone());
        assert_eq!(t_stick(w, h), Some(b), "an impossible size must fall back to the built-in seat");
        assert_eq!(joy_home(w, h).2, stick_base_radius(&b), "…and the ring with it");
        assert_eq!(joy_engage_immersive(w, h), joy_r_legacy(h), "…and the deadzone basis with it");
        LIVE.with(|live| *live.borrow_mut() = Table::default());
    }

    /// 자유 스틱 must be untouched ON THE FIELD: its zone is still the whole window,
    /// which is what makes "touch anywhere to walk" work. 십자키 publishes NO zone —
    /// the stick is hidden outright and the field D-pad (an ordinary button set) walks
    /// instead (2026-08-16 second round).
    ///
    /// ★ OFF-FIELD, no mode publishes one (2026-08-17): the zone drags the idle home
    /// hint onto a menu that has its own D-pad, which is the "placeholder" the player
    /// asked to remove. Dropping the ZONE and not just the drawing is the point — a
    /// hit-test with nothing drawn is an invisible control.
    #[test]
    fn touch_type_a_owns_the_whole_window_on_the_field_only() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            assert_eq!(stick_zone(w, h, StickMode::Float, Scene::Field), Some(Rect::new(0, 0, w, h)), "{w}x{h} 자유 스틱 zone shrank");
            assert_eq!(stick_zone(w, h, StickMode::Dpad, Scene::Field), None, "{w}x{h} 십자키 still has a stick zone");
            for scene in [Scene::Menu, Scene::Cutscene] {
                for mode in STICK_MODES {
                    assert_eq!(
                        stick_zone(w, h, mode, scene),
                        None,
                        "{w}x{h} {scene:?} still publishes a stick zone ({mode:?})"
                    );
                }
            }
        }
    }

    /// 고정 스틱 (2026-09-22): the zone is the SEAT — the square the idle base is drawn
    /// in — and nothing else. Three things follow and each is pinned here:
    ///
    ///  * the drawn base (`joy_home`'s centre and ring) sits INSIDE the zone, so no
    ///    touch on the disc can miss the control;
    ///  * a touch on open ground is nothing (the 십자키 rule — a pinned stick is a
    ///    pinned control, and the point of pinning is that the rest of the field is
    ///    not a stick);
    ///  * the zone FOLLOWS the player's placement, as the base does: move or resize the
    ///    seat in the editor and the hit region moves with the drawing. A zone that
    ///    stayed on the built-in seat would be an invisible control there and a dead
    ///    disc where the player put it.
    #[test]
    fn the_pinned_stick_zone_is_the_seat_the_base_is_drawn_in() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            LIVE.with(|live| *live.borrow_mut() = Table::default());
            let zone = stick_zone(w, h, StickMode::Pinned, Scene::Field).expect("고정 스틱 publishes a zone on the field");
            assert_eq!(zone, stick_seat(w, h), "{w}x{h} the pinned zone is not the built-in seat");
            let (cx, cy, r) = joy_home(w, h);
            assert!(zone.contains(cx, cy), "{w}x{h} the base centre is outside its own zone");
            assert!(cx - r >= f64::from(zone.x) && cx + r <= f64::from(zone.x + zone.w), "{w}x{h} the ring runs out of the zone sideways");
            assert!(cy - r >= f64::from(zone.y) && cy + r <= f64::from(zone.y + zone.h), "{w}x{h} the ring runs out of the zone vertically");
            // Open ground — the window's centre, which is a stick under 자유 스틱 — is
            // not one here.
            assert!(!zone.contains(f64::from(w) / 2.0, f64::from(h) / 2.0), "{w}x{h} the pinned zone reaches the middle of the screen");
            // Smaller than the whole window: the one thing that separates it from 자유 스틱.
            assert!(zone.w < w && zone.h < h, "{w}x{h} the pinned zone is the whole window");
        }
        // A custom placement moves the zone with the base.
        let (w, h) = (2220u32, 1080u32);
        let mut t = Table::default();
        t.set(CtrlId::Stick, Place { cx: 0.30, cy: 0.55, pct: 80 });
        LIVE.with(|live| *live.borrow_mut() = t.clone());
        let placed = t_stick(w, h).expect("the stick always resolves");
        assert_ne!(placed, stick_seat(w, h), "the test placement must actually move the seat");
        let zone = stick_zone(w, h, StickMode::Pinned, Scene::Field).unwrap();
        assert_eq!(zone, placed, "the pinned zone did not follow the placed seat");
        let (cx, cy, _) = joy_home(w, h);
        assert!(zone.contains(cx, cy), "the placed base is outside the placed zone");
        assert!(!zone.contains(f64::from(stick_seat(w, h).x + 2), f64::from(stick_seat(w, h).y + stick_seat(w, h).h - 2)), "the built-in corner is still a stick after the seat moved");
        LIVE.with(|live| *live.borrow_mut() = Table::default());
    }

    /// The settings row cycles through all three, in the order the player named them,
    /// and lands back where it started — a cycle that skipped one would make that
    /// mode unreachable from the panel, and the two predicates the code asks instead
    /// of comparing variants must agree with the modes' meaning.
    #[test]
    fn the_three_stick_modes_cycle_and_answer_their_two_questions() {
        assert_eq!(StickMode::Float.next(), StickMode::Dpad);
        assert_eq!(StickMode::Dpad.next(), StickMode::Pinned);
        assert_eq!(StickMode::Pinned.next(), StickMode::Float);
        let mut seen = Vec::new();
        let mut m = StickMode::Float;
        for _ in 0..3 {
            seen.push(m);
            m = m.next();
        }
        assert_eq!(seen, STICK_MODES.to_vec());
        // 십자키 is the only mode with arrows on the field and the only one with no stick.
        for mode in STICK_MODES {
            assert_eq!(mode.field_arrows(), mode == StickMode::Dpad, "{mode:?} field_arrows");
            assert_eq!(mode.stick_on_field(), mode != StickMode::Dpad, "{mode:?} stick_on_field");
            assert_eq!(mode.stick_is_pinned(), mode == StickMode::Pinned, "{mode:?} stick_is_pinned");
            assert_ne!(mode.field_arrows(), mode.stick_on_field(), "{mode:?} draws both arrows and a stick on the field, or neither");
        }
    }

    /// The ONE walk seat (91차): fully on screen, square, parked in the bottom-left
    /// corner on every window — where TYPE A's stick home sits and TYPE B's arrows walk,
    /// and where a window's cursor arrows appear in every touch type.
    ///
    /// ★★★ 80차-F's rule survives inside it: the arrows are BOTTOM-ANCHORED. 79차-B
    /// removed the `1 3 7 9 #` block above them and let the square "centre on its own",
    /// which walked it into the vertical middle of the screen (player 2026-09-05:
    /// "좌측에 화살표 나오는것도 원래 좌측하단쪽에 위치했엇는데 … 갑자기 좌측 중앙정렬로
    /// 바뀌었어"). And since 91차 there is no second seat that could drift from this one:
    /// the field and a window resolve the SAME control to the SAME rect.
    #[test]
    fn the_stick_seat_sits_bottom_left_on_every_window() {
        let empty = Table::default();
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let d = stick_seat(w, h);
            assert!(d.w > 0 && d.h > 0, "{w}x{h} stick seat is empty");
            assert_eq!(d.w, d.h, "{w}x{h} stick seat is not square");
            assert!(d.x + d.w <= w && d.y + d.h <= h, "{w}x{h} stick seat {d:?} runs off screen");
            // Bottom-left: left of the window's midline, bottom edge in the lower third.
            assert!(d.x + d.w / 2 < w / 2, "{w}x{h} stick seat {d:?} is not on the left");
            assert!(d.y + d.h > h * 2 / 3, "{w}x{h} stick seat {d:?} is not at the bottom");
            // ⛔ The 79차-B failure mode, named: a square whose CENTRE sits on the
            // window's own midline is the "centred on its own" seat this replaced.
            assert!((d.y + d.h / 2) > h / 2 + h / 20, "{w}x{h} stick seat {d:?} is centred again, not bottom-anchored");
            // Same rect with a window open as on the field — the arrows do not jump.
            for mode in modes() {
                let got = resolve(mode, w, h, &empty).into_iter().find(|(c, _)| *c == CtrlId::Stick).unwrap().1;
                assert_eq!(got, d, "{w}x{h} {mode:?} resolves the stick somewhere else");
            }
        }
    }

    /// ★★★ 91차 — the square grew to make the arrows easier to press ("누르기가 좀
    /// 어렵다는 말이 있어서"), and the ONE thing that bounds how far it may grow on the
    /// right is the game's own menu panel: a hit band under the panel's left edge would
    /// steal taps meant for the panel (our buttons are hit-tested first). The panel is
    /// 176 logical px wide and centred, so its left edge is `(lw - 176) / 2` logical px
    /// in from the left — measured on the two devices in use (S9 2220x1080 = 713 px,
    /// S25 2340x1080 = 755 px) and on the 18:9 aspect the old `win_w / 26` inset would
    /// have failed.
    #[test]
    fn the_built_in_stick_seat_clears_the_games_menu_panel() {
        for (w, h) in [(2220u32, 1080u32), (2340, 1080), (2160, 1080), (2400, 1080), (3120, 1440)] {
            let d = stick_seat(w, h);
            let (lw, _lh) = logical_for(w, h);
            let scale = f64::from(w) / f64::from(lw);
            let panel_left = (f64::from((lw - 176) / 2) * scale) as u32;
            assert!(
                d.x + d.w < panel_left,
                "{w}x{h}: the stick seat's right edge {} reaches the menu panel at {panel_left}",
                d.x + d.w
            );
        }
    }

    /// ★★★ 91차-B — one knob, everything. The seat's size percent scales the arrows,
    /// the stick's BASE (`stick_base_radius`), the thumb's PIN (inside the base) and the
    /// walk-START basis together — and that last one is the pre-91차 radius (`win_h /
    /// 12`, clamped), so at 100 % the character moves after the same 27 px it always
    /// did. Tying the deadzone to the base instead would have needed 94 px (7 mm).
    #[test]
    fn one_knob_scales_the_arrows_the_base_the_pin_and_the_deadzone_together() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let seat = stick_seat(w, h);
            let legacy = f64::from((h / 12).clamp(34, 96));
            for pct in [PCT_MIN, 100, 125] {
                let side = scaled(seat.w, pct);
                let r = Rect::new(seat.x, seat.y, side, side);
                let base = stick_base_radius(&r);
                let engage = stick_engage_radius(&r, w, h);
                let want = legacy * f64::from(side) / f64::from(seat.w);
                assert!((engage - want).abs() < 1e-9, "{w}x{h}@{pct}%: engage basis {engage} != {want}");
                if pct == 100 {
                    assert_eq!(engage, legacy, "{w}x{h}: the 100 % walk-start basis moved");
                }
                // The deadzone is well inside the pin, the pin well inside the base.
                let pin = base * (1.0 - JOY_THUMB);
                assert!(engage * 0.30 < pin * 0.5, "{w}x{h}@{pct}%: engage {} is not well inside the pin {pin}", engage * 0.30);
                assert!(pin + base * JOY_THUMB <= base + 1e-9, "{w}x{h}@{pct}%: a pinned thumb leaves the base");
                // …and the arrows STRADDLE the rim (2026-09-20): every hexagon keeps its
                // outer tip outside the disc and its inner tip inside it, which is what
                // makes the two shapes read as two shapes rather than one blob.
                // ⛔ The 91차-B rule here was the opposite — "every corner inside the
                // base" — and it is what this round deliberately gave up.
                let (cx, cy) = (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
                for arm in dpad_arms(r) {
                    let ds: Vec<f64> = hexagon(arm.draw, arm.vertical).iter().map(|(vx, vy)| (vx - cx).hypot(vy - cy)).collect();
                    let far = ds.iter().cloned().fold(f64::MIN, f64::max);
                    let near = ds.iter().cloned().fold(f64::MAX, f64::min);
                    assert!(far > base, "{w}x{h}@{pct}%: the arrow does not reach past the base {base}");
                    assert!(near < base, "{w}x{h}@{pct}%: the arrow does not reach inside the base {base}");
                }
            }
        }
    }

    /// A `field dpad …` line written before 91차 is read as the STICK's seat, and where
    /// a file carries both (every FLIP wrote both), the LATER line — `dpad`, the one
    /// authored for the square — wins. Nothing is dropped and nothing is doubled.
    #[test]
    fn a_legacy_dpad_line_becomes_the_stick_seat() {
        let t = parse_table("field stick 0.0600 0.8750 130
field dpad 0.1600 0.7100 90
");
        let p = t.get(CtrlId::Stick).expect("the dpad line must land on the stick");
        assert_eq!(p.pct, 90, "the dpad line (written later) must win over the stick line");
        assert!((p.cx - 0.16).abs() < 1e-6 && (p.cy - 0.71).abs() < 1e-6);
        let only_stick = parse_table("field stick 0.0600 0.8750 130
");
        assert_eq!(only_stick.get(CtrlId::Stick).map(|p| p.pct), Some(130), "a lone stick line still loads");
        // …and it is written back under the ONE name.
        assert!(t.to_text().contains("field stick "), "written back as a stick line");
        assert!(!t.to_text().contains("dpad"), "no dpad line survives a round trip");
    }

    /// THE 2026-08-17 request: with OK gone from the cutscene there was no way to
    /// advance the script, so the whole screen became OK — "#을 제외한 어디를 눌러도
    /// 대화가 넘어가게 … 화면전체가 OK버튼인 느낌이야".
    #[test]
    fn a_cutscene_tap_that_hits_nothing_is_ok() {
        assert_eq!(tap_anywhere_key(Scene::Cutscene), Some(TapKey::Ok));
    }

    /// "이 정책은 컷신일때만 작동해야 해." On the field the same fallback would fire
    /// the attack key on every tap of open ground; in a menu it would confirm the
    /// cursor's row. Both must stay silent.
    #[test]
    fn tapping_nothing_outside_a_cutscene_sends_nothing() {
        assert_eq!(tap_anywhere_key(Scene::Field), None);
        assert_eq!(tap_anywhere_key(Scene::Menu), None);
    }

    /// 63차 — the GAME'S OWN `# SKIP` corner is tappable, and it beats the screen-wide
    /// OK. Player: "우측하단 skip 이라는 문구 클릭했을때 skip 되도록". ★ Since 64차 it
    /// is the ONLY skip control: the translucent `#`/SKIP disc on the CANCEL seat went
    /// with the player's "따로 기존 skip버튼 … 필요없을것 같아. 제거해줘."
    ///
    /// The FIRST assert is the one that matters: the probe-measured logical rect must
    /// reproduce at the sizes the probe actually ran, so this test fails against
    /// reality rather than against a copy of the formula.
    #[test]
    fn the_probe_measured_skip_ink_reproduces() {
        // (lw, lh, x0, x1, y0, y1) straight off the wie_probe dumps (63차).
        for (lw, lh, x0, x1, y0, y1) in [
            (496u32, 240u32, 462u32, 493u32, 229u32, 237u32),
            (496, 228, 462, 493, 217, 225),
            (560, 240, 526, 557, 229, 237),
            (440, 220, 406, 437, 209, 217),
        ] {
            // At 1:1 (window == logical) the ink rect IS the measured bbox.
            let game = Rect::new(0, 0, lw, lh);
            let ink = game_skip_ink_at(game, lw, lh);
            assert_eq!((ink.x, ink.x + ink.w - 1), (x0, x1), "{lw}x{lh}: ink x drifted from the probe");
            assert_eq!((ink.y, ink.y + ink.h - 1), (y0, y1), "{lw}x{lh}: ink y drifted from the probe");
        }
    }

    #[test]
    fn the_skip_target_covers_the_game_s_own_lettering() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let (lw, lh) = logical_for(w, h);
            let game = Rect::new(0, 0, w, h);
            let hit = game_skip_hint_at(game, lw, lh);
            let ink = game_skip_ink_at(game, lw, lh);
            assert!(ink.inside(&hit), "{w}x{h}: target {hit:?} does not cover the ink {ink:?}");
            assert!(hit.inside(&game), "{w}x{h}: target {hit:?} escaped the game rect");
            // Thumb-sized, and never so greedy that it owns a whole corner quadrant.
            let floor = (w.min(h) / 9).max(24);
            assert!(hit.w >= floor && hit.h >= floor, "{w}x{h}: target {hit:?} below the thumb floor {floor}");
            assert!(hit.w * 4 < w && hit.h * 3 < h, "{w}x{h}: target {hit:?} is too greedy");
        }
    }

    #[test]
    fn tapping_the_skip_lettering_skips_and_the_rest_of_the_screen_still_advances() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let dpad = stick_seat(w, h);
            let (lw, lh) = logical_for(w, h);
            let ink = game_skip_ink_at(Rect::new(0, 0, w, h), lw, lh);
            let (cx, cy) = (f64::from(ink.x + ink.w / 2), f64::from(ink.y + ink.h / 2));
            for choice in [false, true] {
                assert_eq!(
                    cutscene_tap(w, h, cx, cy, dpad, choice),
                    Some(TapKey::Hash),
                    "{w}x{h}: the game's own SKIP lettering did not skip (choice={choice})"
                );
            }
            // The opposite corner is still "advance" — the target did not swallow the
            // screen-wide OK.
            assert_eq!(
                cutscene_tap(w, h, 2.0, 2.0, dpad, false),
                Some(TapKey::Ok),
                "{w}x{h}: the top-left corner stopped advancing the scene"
            );
        }
    }

    /// The corner target must not sit on any OTHER cutscene control, or it would steal
    /// that control's taps. Since 64차 (the `#` disc is gone) the only other cutscene
    /// controls are the four arrows, and those only while a CHOICE is up.
    #[test]
    fn the_skip_target_does_not_overlap_the_other_cutscene_controls() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let (lw, lh) = logical_for(w, h);
            let hit = game_skip_hint_at(Rect::new(0, 0, w, h), lw, lh);
            for (i, arm) in dpad_arms(stick_seat(w, h)).iter().enumerate() {
                assert!(!hit.intersects(&arm.hit.square), "{w}x{h}: SKIP target {hit:?} overlaps arrow {i}");
            }
        }
    }

    /// ★ 64차: the skip DISC is gone — player: "우측하단 '#skip'을 눌러서 skip하는게
    /// 가능해져서 따로 기존 skip버튼(기존 취소 or #버튼 위치)는 필요없을것 같아.
    /// 제거해줘." Its old seat (CANCEL, right edge at mid height) is ordinary scene
    /// again, so a tap there ADVANCES in both choice states; skipping is the game's
    /// own `# SKIP` corner alone, which the tests above already pin down.
    #[test]
    fn the_old_skip_disc_seat_advances_the_scene() {
        let empty = Table::default();
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let dpad = stick_seat(w, h);
            let (lw, lh) = logical_for(w, h);
            let seat = resolve(Mode::Menu, w, h, &empty)
                .into_iter()
                .find(|(id, _)| *id == CtrlId::Cancel)
                .map(|(_, r)| r)
                .expect("the CANCEL seat still resolves (menus draw CLR there)");
            let (cx, cy) = (f64::from(seat.x + seat.w / 2), f64::from(seat.y + seat.h / 2));
            // Guard the premise: the seat's centre must sit on NO live control, or the
            // asserts below would be exercising the wrong pixels.
            assert!(
                !game_skip_hint_at(Rect::new(0, 0, w, h), lw, lh).contains(cx, cy),
                "{w}x{h}: the CANCEL seat drifted into the corner target"
            );
            assert!(
                !dpad_arms(dpad).iter().any(|a| a.hit.contains(cx, cy)),
                "{w}x{h}: the CANCEL seat drifted into the arrows"
            );
            for choice in [false, true] {
                assert_eq!(
                    cutscene_tap(w, h, cx, cy, dpad, choice),
                    Some(TapKey::Ok),
                    "{w}x{h}: the removed disc's seat still swallowed a tap (choice={choice})"
                );
            }
        }
    }

    /// THE 2026-08-17 rule, in two halves. Window id 4 is BOTH story narration and NPC
    /// dialogue (device-confirmed: the quest guide reads `win=4` exactly like the
    /// prologue), so the arrows are gated on the CHOICE byte instead — "컷신일때는
    /// 안나오고 인게임 내에서 대화에 선택지가 있을때는 나오고".
    ///
    /// With a choice up, each arm must also beat the screen-wide OK on its own pixels,
    /// or the tap would confirm the highlighted row instead of moving the cursor.
    #[test]
    fn the_arrows_appear_for_a_choice_and_only_for_a_choice() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            let dpad = stick_seat(w, h);
            for (i, arm) in dpad_arms(dpad).iter().enumerate() {
                assert!(arm.draw.w > 0 && arm.draw.h > 0, "{w}x{h}: arm {i} is empty");
                // The hexagon's centre — the middle of the ink, which is inside the quarter.
                let (cx, cy) = (f64::from(arm.draw.x + arm.draw.w / 2), f64::from(arm.draw.y + arm.draw.h / 2));
                assert_eq!(
                    cutscene_tap(w, h, cx, cy, dpad, true),
                    Some(TapKey::Arrow),
                    "{w}x{h}: arm {i} lost its centre to the screen-wide OK"
                );
                // A plain cutscene has no arrows at all: the same pixels advance the
                // scene, which is what "컷신일때는 안나오고" means for a tap.
                assert_eq!(
                    cutscene_tap(w, h, cx, cy, dpad, false),
                    Some(TapKey::Ok),
                    "{w}x{h}: arm {i} still swallowed a plain cutscene tap"
                );
            }
            // The square's CORNER now belongs to an arrow (2026-08-17 banding), so it
            // must NOT fall through to the screen-wide OK during a choice.
            let corner = (f64::from(dpad.x + 1), f64::from(dpad.y + 1));
            assert_eq!(
                cutscene_tap(w, h, corner.0, corner.1, dpad, true),
                Some(TapKey::Arrow),
                "{w}x{h}: the dpad corner is no longer part of an arrow"
            );
        }
        assert!(!cutscene_shows_dpad(false), "a plain cutscene must not draw arrows");
        assert!(cutscene_shows_dpad(true), "a choice must draw arrows");
    }

    /// THE 2026-08-17 arrow requirements, kept through the 91차-B fat hexagons: "원래
    /// 화면에 표시되는 영역보다 조금 더 큰범위를 눌러도 동작하게 해줘. 물론 각 화살표별
    /// 클릭되는 범위가 겹치는건 안돼." — on the diagonal-quarter partition, plus the 91차-B
    /// shape rules ("화살표 크기 더 늘려줘야해 … 뚱뚱한 육각형").
    #[test]
    fn the_arrow_quarters_tile_the_square_and_always_contain_their_ink() {
        for (w, h) in WINDOWS {
            if w <= h {
                continue;
            }
            // Every plausible D-pad square: the built-in seat and the extremes of the
            // editor's size range, which is where a partition is most likely to degenerate.
            let seat = stick_seat(w, h);
            for pct in [PCT_MIN, 100, PCT_MAX] {
                let side = scaled(seat.w, pct);
                let r = Rect::new(seat.x, seat.y, side, side);
                let arms = dpad_arms(r);
                let (cx, cy) = (f64::from(r.x) + f64::from(r.w) / 2.0, f64::from(r.y) + f64::from(r.h) / 2.0);
                let dead = f64::from(side) * f64::from(DEAD_PCT) / 100.0;
                for (i, a) in arms.iter().enumerate() {
                    assert!(a.draw.w > 0 && a.draw.h > 0, "{w}x{h}@{pct}%: arm {i} degenerated");
                    // 1) the ink stays inside the control's own footprint, so
                    //    `placement`'s collision checks still describe the truth.
                    assert!(a.draw.inside(&r), "{w}x{h}@{pct}%: arm {i} box {:?} leaves the square {r:?}", a.draw);
                    assert_eq!(a.hit.square, r, "{w}x{h}@{pct}%: arm {i} targets a different square");
                    // 2) every corner of the hexagon is inside ITS OWN quarter (a convex
                    //    shape whose corners are all inside a convex region is inside it),
                    //    clear of the diagonals by a visible margin and clear of the dead
                    //    disc — so the touch target is a superset of the ink, and two
                    //    hexagons can never touch (their quarters are disjoint).
                    let hx = hexagon(a.draw, a.vertical);
                    for (k, (vx, vy)) in hx.iter().enumerate() {
                        assert!(a.hit.contains(*vx, *vy), "{w}x{h}@{pct}%: arm {i} corner {k} ({vx},{vy}) is outside its quarter");
                        let (dx, dy) = ((vx - cx).abs(), (vy - cy).abs());
                        let to_diagonal = (dx - dy).abs() / 2f64.sqrt();
                        assert!(to_diagonal >= 3.0, "{w}x{h}@{pct}%: arm {i} corner {k} is only {to_diagonal} px off a diagonal");
                        assert!(dx.hypot(dy) > dead + 2.0, "{w}x{h}@{pct}%: arm {i} corner {k} is in the dead disc");
                    }
                    // 3) FAT and BIG: wider across than long along, at least 0.40 of the
                    //    side across and 0.33 along — and the tips are on the axis.
                    let (along, cross) = if a.vertical { (a.draw.h, a.draw.w) } else { (a.draw.w, a.draw.h) };
                    assert!(cross > along, "{w}x{h}@{pct}%: arm {i} box {cross}x{along} is not fat");
                    assert!(cross * 100 >= side * 40 && along * 100 >= side * 33, "{w}x{h}@{pct}%: arm {i} {cross}x{along} is small for a {side} square");
                    let (tip_a, tip_b) = (hx[0], hx[3]);
                    if a.vertical {
                        assert!((tip_a.0 - tip_b.0).abs() < 1e-9);
                    } else {
                        assert!((tip_a.1 - tip_b.1).abs() < 1e-9);
                    }
                }
                // 4) THE partition: the quarters TILE the square — every point (on a 3 px
                //    grid, plus the diagonals) is owned by exactly one arrow, except the
                //    dead disc which is owned by none. This is "겹치는건 안돼" and "corners
                //    answer" in one property.
                let step = 3u32;
                let mut y = r.y;
                while y < r.y + r.h {
                    let mut x = r.x;
                    while x < r.x + r.w {
                        let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                        let owners = arms.iter().filter(|a| a.hit.contains(px, py)).count();
                        let in_dead = (px - cx).hypot(py - cy) < dead;
                        assert_eq!(owners, usize::from(!in_dead), "{w}x{h}@{pct}%: ({px},{py}) has {owners} owner(s), dead={in_dead}");
                        x += step;
                    }
                    y += step;
                }
                for k in 0..side {
                    let (px, py) = (f64::from(r.x + k) + 0.5, f64::from(r.y + k) + 0.5);
                    if (px - cx).hypot(py - cy) >= dead {
                        assert_eq!(arms.iter().filter(|a| a.hit.contains(px, py)).count(), 1, "{w}x{h}@{pct}%: diagonal point {k} is not owned exactly once");
                    }
                }
                // 5) …and outside the square nothing answers.
                for (px, py) in [(f64::from(r.x) - 1.0, cy), (f64::from(r.x + r.w) + 1.0, cy), (cx, f64::from(r.y) - 1.0), (cx, f64::from(r.y + r.h) + 1.0)] {
                    assert!(!arms.iter().any(|a| a.hit.contains(px, py)), "{w}x{h}@{pct}%: ({px},{py}) outside the square answered");
                }
                // 6) THE 91차-B point — the ink is a lot bigger than the 2026-08-17
                //    triangle's on the same square: that one covered `0.42 * 0.255 / 2`
                //    of the side squared; a hexagon of box `cross x along` with 30 % tips
                //    covers `cross * along * 0.7`.
                let up = &arms[0];
                let hex_area = f64::from(up.draw.w) * f64::from(up.draw.h) * 0.7;
                let old_area = f64::from(side) * 0.42 * f64::from(side) * 0.255 / 2.0;
                assert!(hex_area > old_area * 1.8, "{w}x{h}@{pct}%: hexagon ink {hex_area} is not clearly more than the triangle's {old_area}");
            }
        }
    }

    /// The cost of the coupling, stated as a test rather than left to be discovered
    /// on a phone: a bigger stick needs a longer thumb travel before it walks
    /// (`joy_dir`'s engage deadzone is a fraction of the radius).
    #[test]
    fn a_bigger_stick_starts_walking_later() {
        let small = 60.0;
        let big = 150.0;
        // 25 px of travel engages the small stick and not the big one.
        assert_eq!(joy_dir(0.0, -25.0, small, None), Some(Key::Up));
        assert_eq!(joy_dir(0.0, -25.0, big, None), None);
        // The big one still engages, just further out.
        assert_eq!(joy_dir(0.0, -60.0, big, None), Some(Key::Up));
    }

    /// THE 2026-08-03 request: the first touch IS the centre, and it stays there for
    /// the life of the touch no matter how far the finger travels. Lift and touch
    /// again and the new point is the new centre.
    #[test]
    fn centre_is_the_touch_point_and_never_moves() {
        let r = 90.0;
        let mut j = Joy::down(100.0, 100.0);
        for (x, y) in [(140.0, 100.0), (100.0, -400.0), (900.0, 700.0)] {
            j.moved(x, y, r);
            assert_eq!((j.cx, j.cy), (100.0, 100.0), "centre moved to ({}, {})", j.cx, j.cy);
        }
        // Lift (the App drops the Joy) and touch elsewhere: a fresh centre.
        let j2 = Joy::down(640.0, 360.0);
        assert_eq!((j2.cx, j2.cy), (640.0, 360.0));
    }

    /// The drawn thumb is what tells the player they are at full tilt, so it must pin
    /// to the ring however far the finger goes — that is what stops the drag from
    /// wandering far enough for the fixed centre to make turning stiff.
    #[test]
    fn thumb_pins_to_the_ring() {
        let r = 90.0;
        let j = Joy::down(100.0, 100.0);
        let (tx, ty) = j.thumb(100.0, -400.0, r);
        assert!((tx - 100.0).abs() < 1e-9, "no sideways drift: {tx}");
        assert!((ty - (100.0 - r)).abs() < 1e-9, "pinned at the ring: {ty}");
        // Inside the ring it tracks the finger exactly.
        assert_eq!(j.thumb(130.0, 100.0, r), (130.0, 100.0));
    }

    /// Replay a finger path exactly as `App::joy_update` does (fixed centre),
    /// returning every direction transition it emits.
    fn replay(path: &[(f64, f64)], r: f64) -> Vec<(Option<Key>, Option<Key>)> {
        let mut j = Joy::down(path[0].0, path[0].1);
        let mut out = Vec::new();
        for (tx, ty) in path {
            let before = j.dir;
            j.moved(*tx, *ty, r);
            if j.dir != before {
                out.push((before, j.dir));
            }
        }
        out
    }

    /// THE gesture that made 8-way steering unusable, end to end: thumb deflects up
    /// and then sweeps left without lifting. Old behaviour: UP and LEFT both held,
    /// guest keeps walking up, nothing changes until the finger lifts. Required: one
    /// engage + exactly one turn — and it holds with the fixed centre because the
    /// sweep is an ARC at constant radius, which is what a thumb pivoting on its
    /// knuckle actually does.
    #[test]
    fn sweep_up_then_left_without_lifting_turns_once() {
        let r = 90.0;
        let mut path: Vec<(f64, f64)> = vec![(500.0, 500.0)];
        // Up to 70 px of deflection…
        path.extend((1..=7).map(|i| (500.0, 500.0 - 10.0 * f64::from(i))));
        // …then sweep round to the left at that same 70 px radius.
        path.extend((1..=18).map(|i| {
            let a = f64::from(i) * 5.0f64.to_radians();
            (500.0 - 70.0 * a.sin(), 500.0 - 70.0 * a.cos())
        }));
        let t = replay(&path, r);
        assert_eq!(
            t,
            vec![(None, Some(Key::Up)), (Some(Key::Up), Some(Key::Left))],
            "one engage + one turn, no flutter: {t:?}"
        );
    }

    /// The COST of the fixed centre, pinned down rather than left as a guess: after a
    /// long straight drag the turn needs sideways travel proportional to how far the
    /// finger went, because the direction is the angle of the whole vector from the
    /// spawn point. The trailing base used to turn this in ~120 px.
    ///
    /// This is a characterisation test — it documents what the player traded for a
    /// stick that stays put, and it fails loudly if anyone re-introduces a moving
    /// centre without saying so.
    #[test]
    fn fixed_centre_needs_proportional_sideways_travel_to_turn() {
        let r = 90.0;
        let mut path: Vec<(f64, f64)> = vec![(500.0, 500.0)];
        path.extend((1..=30).map(|i| (500.0, 500.0 - 10.0 * f64::from(i)))); // 300 px up
        let up_end = path.len();
        path.extend((1..=45).map(|i| (500.0 - 10.0 * f64::from(i), 200.0))); // push left
        let t = replay(&path, r);
        assert_eq!(t.first().copied(), Some((None, Some(Key::Up))), "engages up: {t:?}");
        assert_eq!(t.len(), 2, "exactly one turn, no flutter: {t:?}");
        assert_eq!(t[1], (Some(Key::Up), Some(Key::Left)), "turns left: {t:?}");
        // TURN_RATIO = 1.2 against a 300 px lever = 360 px of sideways travel.
        assert_eq!(replay(&path[..up_end + 35], r).len(), 1, "not yet turned at 350 px");
        assert_eq!(replay(&path[..up_end + 37], r).len(), 2, "turned by 370 px");
    }

    /// …and the reason that cost is acceptable: a thumb that STAYS on the ring (which
    /// is what the pinned thumb disc invites) turns in a few degrees of arc, whatever
    /// the deflection.
    #[test]
    fn turning_on_the_ring_is_immediate() {
        let r = 90.0;
        let mut path: Vec<(f64, f64)> = vec![(500.0, 500.0)];
        path.extend((1..=9).map(|i| (500.0, 500.0 - 10.0 * f64::from(i)))); // out to the ring
        let up_end = path.len();
        path.extend((1..=18).map(|i| {
            let a = f64::from(i) * 5.0f64.to_radians();
            (500.0 - 90.0 * a.sin(), 500.0 - 90.0 * a.cos())
        }));
        assert_eq!(replay(&path, r).len(), 2, "engage + one turn");
        // ~60 degrees of arc is enough — about a centimetre of thumb travel. (The
        // exact figure is where TURN_RATIO = 1.2 puts `tan a > 1.2`, i.e. 50.2°, so
        // the first sample past it is step 11.)
        assert_eq!(replay(&path[..up_end + 12], r).len(), 2, "turned within 60 degrees");
        assert_eq!(replay(&path[..up_end + 9], r).len(), 1, "and not before 45 degrees");
    }

    #[test]
    fn hud_box_is_the_measured_176x24_logical_block() {
        // 240x320 baseline: 176/240 of the width, 24/320 of the height.
        let g = Rect::new(0, 0, 480, 640); // 2x the baseline
        let b = game_hud_box(g, 240, 320);
        assert_eq!((b.w, b.h), (352, 48));
        assert_eq!(b.x, (480 - 352) / 2);
        assert_eq!(b.y, 640 - 48);
    }
}

fn main() {}


// --------------------------------------------------------------------------
// the ring takes the LARGEST plate that fits — 79차-E
//
// Player, 2026-09-04: "퀵슬롯 버튼 크기를 전부 조금씩만 키워줘". Going from 5 plates to 7
// (six ring seats to eight) crowded the arc enough that the plate has to shrink, and
// the old `side *= 0.9` loop overshot: on 2340x1080 the largest side that fits is 87
// and it handed back 79.
// --------------------------------------------------------------------------

/// The one number the player actually sees.
fn plate_side(win_w: u32, win_h: u32, n: usize) -> u32 {
    let pad = pad_of(win_w, win_h);
    ring_slots(win_w, win_h, n, pad).first().map(|r| r.w).unwrap_or(0)
}

#[test]
fn the_ring_plate_is_the_largest_that_fits() {
    for (w, h) in [(2340u32, 1080u32), (1920, 1080), (2400, 1080), (1600, 720)] {
        let pad = pad_of(w, h);
        let n = RING_SEATS;
        let side = plate_side(w, h, n);
        assert!(side > 0, "{w}x{h}: empty ring");
        // Every seat is square, the same size, and fully on screen.
        for r in ring_slots(w, h, n, pad) {
            assert_eq!((r.w, r.h), (side, side));
            assert!(r.x + r.w <= w && r.y + r.h <= h, "{w}x{h}: seat off screen");
        }
        // …and one pixel more would not fit (or we are already at the ceiling).
        let ceiling = (h / 11).clamp(40, 120);
        if side < ceiling {
            let bigger = ring_slots_at(w, h, n, pad, f64::from(side + 2));
            assert!(bigger.is_none(), "{w}x{h}: {} would have fit, we returned {side}", side + 2);
        }
    }
}

/// The seven-plate ring is smaller than the five-plate one — but not by the 20 % the
/// coarse loop used to cost. Both numbers are this machine's arithmetic, not a wish.
#[test]
fn seven_plates_cost_about_a_ninth_of_the_plate() {
    let (w, h) = (2340u32, 1080u32);
    let six = plate_side(w, h, 6);
    let eight = plate_side(w, h, 8);
    assert_eq!(six, 98, "six seats still fit the base size");
    assert!(eight >= 86 && eight < six, "eight seats: {eight}");
}
