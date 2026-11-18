//! The player's own control layout: where each touch control sits and how big it
//! is, per HUD mode.
//!
//! ── WHY THIS EXISTS ──────────────────────────────────────────────────────────
//! Every control rect used to be a pure function of the window size, and the
//! bottom-right cluster was ONE function: `ring_slots` derives its radius from the
//! action disc and the secondary button, then shrinks all six plates together until
//! they fit. That coupling is invisible until you try to resize anything — measured
//! on a 2340x1080 device, growing OK by 40 % shrank every skill plate from 98 px to
//! 52 px, and a "make the plates bigger" knob was actually NON-MONOTONIC (98 px at
//! 100 %, 94 px at 120 %). So the player's placement is deliberately NOT fed back
//! into the built-in formulas: the built-ins stay a pure function of the window,
//! and this module lays the player's choices on top of them.
//!
//! ── WHAT IS STORED ───────────────────────────────────────────────────────────
//! Position as a FRACTION of the window, size as a PERCENT of the built-in size.
//! Never pixels: the layout has to survive a rotation, a different device and the
//! ZOOM setting. Never guest px either — those are frozen at boot (`lib::logical`).
//!
//! ── THE INVARIANT ────────────────────────────────────────────────────────────
//! Controls may overlap EACH OTHER freely (2026-08-07, player: "버튼 UI 변동이
//! 겹칠 수 있게" — before that a fifth-of-a-side tolerance, before THAT full
//! disjointness). What a control may still never cover is an OBSTACLE: the HUD
//! widgets the GAME itself draws (buff band, target-monster box), our own menu
//! chrome, and the editor's buttons — all of them win a touch the control would
//! otherwise get, so a control on top of one silently does something else (see
//! `obstacles`). [`resolve`] enforces that on the EFFECTIVE rects at every
//! layout, not just when the player lets go: the effective size depends on the
//! window, so a placement that cleared an obstacle on the device it was authored
//! on can land on it on another one. A placement that does not survive the check
//! silently falls back to its built-in rect. The editor uses the same predicate,
//! so what the player sees while dragging is what they get. Which of two
//! overlapping CONTROLS gets a shared touch is the `ids_for` order (first match
//! wins), same as before.

use std::sync::{LazyLock, RwLock};

use crate::overlay::{self, Mode, Rect};

/// Size bounds, as a percent of the built-in size. The floor keeps every control
/// tappable (the smallest built-in plate is ~40 px, so 50 % is ~20 px); the ceiling
/// stops one control from owning the screen.
pub const PCT_MIN: u32 = 50;
pub const PCT_MAX: u32 = 250;
pub const PCT_DEFAULT: u32 = 100;

/// A control the player can move and resize. `Quick` carries the SEAT index,
/// not a key code: the field plates have no `KeyCode` at all (their hotkey is
/// resolved at press time from the quickbar slot) and the menu keys 1/3/7/9 also
/// exist on the pull-out keypad, so a key code is not an identity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtrlId {
    /// The big action disc (OK).
    Ok,
    /// CLR — ring seat 0.
    Cancel,
    /// Secondary-weapon attack (key 7), field only.
    Secondary,
    /// The minimap panel (tapping it presses `*`), field only.
    Minimap,
    /// THE walk control — the stick AND the four-arrow D-pad, as ONE seat.
    ///
    /// ★★★ 91차 (2026-09-13), player: "인게임 조이스틱이랑 메뉴열렸을때 화살표를 그냥
    /// stick이라는 하나의 옵션으로 관리하고싶어". The seat is a SQUARE
    /// (`overlay::stick_seat`), bottom-left, and every walk-or-cursor control is cut
    /// from it:
    ///
    ///  * TOUCH TYPE A, field: the stick — its base disc runs through the four hexagons'
    ///    CENTRES (`overlay::stick_base_radius`, 2026-09-20; it was the square's inscribed
    ///    circle from 91차-B until then). The walk-start distance is a separate, smaller
    ///    basis again (`overlay::joy_engage_radius`).
    ///  * TOUCH TYPE B, field: the four arrows (`overlay::dpad_arms` tiles the square).
    ///  * a game WINDOW, and a dialogue CHOICE: the same four arrows, in every touch type.
    ///
    /// Why one and not two: the layout editor shows the FIELD set filtered by the live
    /// touch type, so once 83차 removed the MENU page a TYPE A player saw only the stick
    /// disc and had no control at all that resized the menu arrows. The former
    /// `CtrlId::Dpad` is gone; its `dpad` layout lines are read as this control's
    /// ([`CtrlId::parse`]).
    ///
    /// It is here (placeable) because a left-handed layout has to be able to send the
    /// stick to the other side: left where it was born, the buttons the player just
    /// moved left would sit on top of it and steal its touches.
    ///
    /// ★ RESIZABLE since 2026-08-16, on the player's terms ("터치타입 B일때는 크기를
    /// 조정 할 수있어야하니까"). It was position-only until then for a real reason —
    /// the stick's radius is ALSO the input deflection distance (`overlay::joy_radius`),
    /// so a size knob retunes how far a thumb must travel to start walking. That
    /// coupling is kept deliberately rather than broken: the ring you see is the
    /// throw you get, the thumb pins at the ring edge at full tilt, and decoupling
    /// the two would make the thumb stop dead somewhere inside the ring with no
    /// drawn reason. Since 91차-B the ring is the whole square's inscribed circle and
    /// the thumb pins INSIDE it (base minus thumb — the cap in its well), while the
    /// walk-START distance keeps the pre-91차 basis scaled by the knob: one knob sizes
    /// the arrows, the base and the deadzone together, and at 100 % the character
    /// starts walking after the same 27 px it always did.
    ///
    /// ★ The player asked for the arrows to be sized and moved **together** (2026-08-09:
    /// "화살표는 한개씩 크기조절 및 위치 조정하는게 아니라 조이스틱처럼 한번에 4개의
    /// 화살표를 모두 조절하는 방식으로 할꺼야"), which is why this is the arrows'
    /// BOUNDING SQUARE and `overlay::dpad_arms` derives the four arms from it. Four
    /// separate ids would have given the editor four things to drag and the on-disk
    /// layout four lines that could disagree with each other.
    Stick,
    /// Field quickbar plates, seats `0..quickpage::PLATES`.
    ///
    /// ★ SEVEN since 2026-09-04, and the extra two are not a different kind of
    /// control: the guest's bar is only five wide (its arrays are back to back — see
    /// `quickpage.rs`), so a plate is bound to a guest slot on demand and pressing it
    /// sends THAT slot's hotkey. Which is why there is no fixed "hotkey 1/3/9/0/#"
    /// per seat any more — `QuickSlot::guest` says which one a given plate is using
    /// at a given moment.
    Quick(u8),
    // `MenuKey(0..=4)` — the always-on 1/3/7/9/# block — was REMOVED in 79차-B. The
    // game's own «1» «3» «7» «9» badges are tap targets (66차) and the pull-out keypad
    // still carries every digit, so five permanent keys were a second way to do what a
    // tap already does. Stored `k0`..`k4` lines simply stop parsing, which drops them.
    // `Dpad` — the four-arrow D-pad as a control of its own (2026-08-09 .. 91차) — was
    // FOLDED INTO `Stick` in 91차. See the note on `Stick`; a stored `dpad` line is read
    // as a `stick` line.
    /// Flips the quickbar to the next host-owned page (see `quickpage.rs`). Field, PLUS
    /// while the plates are lit for 퀵바등록: a page flip re-binds guest quickbar slots,
    /// which is not something to do with a game window open in general — but registering
    /// onto page 2 is exactly that, and the player asked for it (2026-08-04). Every other
    /// off-field state still hides it.
    ///
    /// ★ A control of its own rather than a fixed offset from "the lowest plate": the
    /// plates are individually placeable (9차), so after the player rearranges them
    /// "lowest" is a different one every frame. Being a `CtrlId` also gets it the editor,
    /// the overlap rule, the forbidden zones and per-mode seats for free.
    QuickPage,
    /// Shows/hides the minimap panel. Its own control for the same reason, and because
    /// the panel it toggles is itself placeable.
    MinimapToggle,
    /// SAVE NOW — arms the four-step save chain (see `save.rs`). Field only.
    ///
    /// ★ Added 2026-08-04 because the settings-panel row could never be pressed
    /// successfully: the gear exists only OFF the field and the game's own save gate
    /// passes only ON it, so the two conditions were disjoint and the answer was
    /// always `CANNOT SAVE HERE`. A control here gets the editor, the overlap rule and
    /// the forbidden zones for free, exactly like the page button.
    QuickSave,
}

/// ★ The minimap is resolved FIRST because the game's own target-monster box dodges
/// it (the host pokes the box's anchor), so where that obstacle ends up is a
/// function of where the panel ended up.
pub const FIELD_IDS: [CtrlId; 15] = [
    CtrlId::Minimap,
    // Resolved right after the panel: its default seat is derived from where the panel
    // ended up, so the panel has to be placed first (same reason the panel itself is
    // first — see the note above).
    CtrlId::MinimapToggle,
    // THE walk control: the stick under touch type A, the four arrows under type B —
    // one seat either way (91차).
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
    // After the plates: both sit next to them on the plate row, so they should yield to
    // the plates rather than the other way round when a layout is tight.
    CtrlId::QuickPage,
    CtrlId::QuickSave,
];

/// Which controls exist with a game window open. A STRICT SUBSET of [`FIELD_IDS`] —
/// that is what lets the two modes share one seat table (see [`Table`]), and the `const`
/// block under [`slot`] proves it at compile time.
///
/// ★ The quickbar plates are in this set since 2026-08-03: they stay on screen with a
/// game window open (player request). Off the field a plate tap does NOTHING (79차-B)
/// except while the plates are lit for 퀵슬롯등록, which is exactly when a game window
/// IS open — so they have to be here, and they have to be where the player put them.
///
/// ★ This list is also the RESOLVER's priority order for menu mode, which is why it is
/// still its own list rather than a filter over the field one.
pub const MENU_IDS: [CtrlId; 11] = [
    CtrlId::Ok,
    CtrlId::Cancel,
    // The four arrows that step a window's cursor are cut from the STICK seat (91차) —
    // the same square the field's stick or arrows use, so the arrows do not move when
    // a window opens. (Touch priority is a different order — `layout_immersive` pushes
    // the pull-out keypad into `buttons` first, so a key parked on an arm keeps the
    // touch.)
    CtrlId::Stick,
    CtrlId::Quick(0),
    CtrlId::Quick(1),
    CtrlId::Quick(2),
    CtrlId::Quick(3),
    CtrlId::Quick(4),
    CtrlId::Quick(5),
    CtrlId::Quick(6),
    // ★ In the MENU set since 2026-08-04, for one reason: the 퀵바등록 flow lights the
    // plates while one of the GAME's windows is open, and the player asked to be able to
    // flip pages right there ("페이지 전환해서 등록도 가능해야해"). It is only DRAWN and
    // hit-tested while the plates are lit (`app.rs`).
    //
    // ★★ It used to need a menu-mode SEAT of its own, because reusing the field seat
    // would have put it wherever the FIELD layout said while the plates it belongs to sat
    // at their MENU seats. Since 2026-09-10 there is one layout, so that is no longer a
    // reason for a second table — the button and its plates move together by construction.
    CtrlId::QuickPage,
];

/// A cutscene borrows the MENU seats (its CHOICE arrows use the MENU D-pad seat), so
/// it is not a third stored layout — see [`overlay::Mode::Cutscene`]. It used to draw
/// OK (until 2026-08-16) and a `#` skip disc in the CANCEL seat (until 64차 — the
/// game's own `# SKIP` lettering is the skip control now).
pub fn ids_for(mode: Mode) -> &'static [CtrlId] {
    match mode {
        Mode::Field => &FIELD_IDS,
        Mode::Menu | Mode::Cutscene => &MENU_IDS,
    }
}

/// Does this control actually exist on screen in `mode` right now?
///
/// ★ A FILTER over [`ids_for`], not a second id list: a filtered-out control keeps its
/// slot in the table and on disk, and `resolve` keeps indexing arrays by the full list.
///
/// ★ Until 91차 this also chose between the stick (TOUCH TYPE A) and the D-pad (TYPE
/// B) on the field ("화살표만 나와야해 … 조이스틱은 터치타입 B에서 히든처리"). They are
/// one control now — what the touch type still decides is what that seat DRAWS on the
/// field (`overlay::layout_immersive`), not whether it exists.
pub fn shown(_mode: Mode, id: CtrlId) -> bool {
    // ★★ A quickbar plate the player has switched off (2026-09-10) is not shown in ANY
    // mode.
    //
    // What this buys is the LAYOUT EDITOR: `draw_layout_editor` and the editor's own
    // hit-test both filter through here, so a hidden plate is neither drawn at the
    // phantom seat `plate_seat` answers with nor selectable by a blind tap on it. The
    // LIVE hud hides it separately, by way of the zero-sized rect `quick_buttons`
    // builds — these are two different paths and both are needed.
    match id {
        CtrlId::Quick(i) => !crate::quickpage::plate_hidden(crate::quick_hidden(), i),
        _ => true,
    }
}

impl CtrlId {
    /// Stable on-disk key.
    pub fn key(self) -> String {
        match self {
            CtrlId::Ok => "ok".into(),
            CtrlId::Cancel => "clr".into(),
            CtrlId::Secondary => "sec".into(),
            CtrlId::Minimap => "map".into(),
            CtrlId::Stick => "stick".into(),
            CtrlId::Quick(i) => format!("q{i}"),
            CtrlId::QuickPage => "qpage".into(),
            CtrlId::MinimapToggle => "mapt".into(),
            CtrlId::QuickSave => "qsave".into(),
        }
    }

    fn parse(s: &str) -> Option<CtrlId> {
        Some(match s {
            "ok" => CtrlId::Ok,
            "clr" => CtrlId::Cancel,
            "sec" => CtrlId::Secondary,
            "map" => CtrlId::Minimap,
            "stick" => CtrlId::Stick,
            // ★ 91차: the D-pad's own line becomes the STICK's. The two controls were
            // merged, and the square the `dpad` line was authored for IS the merged
            // seat, so its centre and percent carry over unchanged. Files written by
            // vc307 and earlier list `stick` before `dpad`, and `config::read_layout`
            // applies lines in order — so where BOTH exist the `dpad` line wins, which
            // is the one that describes a square rather than a 180 px disc.
            LEGACY_DPAD_KEY => CtrlId::Stick,
            // Before the single-char split below: these keys are multi-character and
            // "qpage" would otherwise be read as `Quick`-with-a-bad-number and dropped.
            "qpage" => CtrlId::QuickPage,
            "mapt" => CtrlId::MinimapToggle,
            "qsave" => CtrlId::QuickSave,
            _ => {
                let (kind, n) = s.split_at(1);
                let i: u8 = n.parse().ok()?;
                match kind {
                    "q" if usize::from(i) < crate::quickpage::PLATES => CtrlId::Quick(i),
                    _ => return None,
                }
            }
        })
    }

    /// Is this control drawn as a DISC in play? Everything except the minimap, which
    /// is a square panel. The editor mirrors this so a button reads as the shape it
    /// will actually be (player request 2026-07-31: "실제 버튼처럼 원형으로").
    pub fn is_round(self) -> bool {
        // The STICK joins the minimap as a SQUARE: what the player places is the
        // arrows' bounding box (the stick disc sits at its centre), and drawing that as
        // a disc in the editor would promise a round control and then paint a cross of
        // four arrows. `draw_layout_editor` paints the arrows and the disc inside it.
        !matches!(self, CtrlId::Minimap | CtrlId::Stick)
    }

    /// Editor caption. **ASCII only** — `overlay::draw_glyph` is a 5x7 ASCII bitmap
    /// font, so a Hangul label would render as blank space.
    pub fn label(self) -> String {
        match self {
            CtrlId::Ok => "OK".into(),
            CtrlId::Cancel => "CLR".into(),
            CtrlId::Secondary => "7".into(),
            CtrlId::Minimap => "MAP".into(),
            CtrlId::Stick => "STICK".into(),
            // ★ The SEAT number, not the hotkey it sends (2026-08-03). These used to be
            // labelled 1 / 3 / 9 / 0 / # because that is what a press emits, but the
            // player no longer thinks of them as number keys — "이제 우측에 스킬 퀵바는
            // 숫자로써의 역할이 아니야. 퀵바로써의 역할인거야." A caption that reads `#`
            // for the slot holding a potion describes the wire, not the control.
            CtrlId::Quick(i) => format!("Q{}", i + 1),
            CtrlId::QuickPage => "PAGE".into(),
            CtrlId::MinimapToggle => "MAP".into(),
            CtrlId::QuickSave => "SAVE".into(),
        }
    }
}

/// One control's player-chosen geometry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Place {
    /// Centre x, as a fraction of the window width.
    pub cx: f32,
    /// Centre y, as a fraction of the window height.
    pub cy: f32,
    /// Size, as a percent of the control's built-in size.
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

    /// The rect this placement asks for, clamped into the window. Clamping the
    /// POSITION (rather than rejecting) is what makes a drag against the screen edge
    /// feel like sliding along it instead of stopping dead.
    pub fn to_rect(self, builtin: Rect, win_w: u32, win_h: u32) -> Rect {
        let w = crate::scaled(builtin.w.max(1), self.pct).min(win_w.max(1));
        let h = crate::scaled(builtin.h.max(1), self.pct).min(win_h.max(1));
        let cx = (self.cx.clamp(0.0, 1.0) * win_w as f32) as i64;
        let cy = (self.cy.clamp(0.0, 1.0) * win_h as f32) as i64;
        let x = (cx - i64::from(w) / 2).clamp(0, i64::from(win_w.saturating_sub(w)));
        let y = (cy - i64::from(h) / 2).clamp(0, i64::from(win_h.saturating_sub(h)));
        Rect::new(x as u32, y as u32, w, h)
    }
}

/// Offsets [`Table::mirror`] tries, in order, when a control's exact mirror will not
/// fit: straight up first (the blocker is nearly always the bottom band — the chrome
/// and the mirrored cluster want the same corner), then progressively sideways.
///
/// A hand-picked shortlist was not enough: on a 2340x1080 device the `#` plate needed
/// to travel almost half the screen height before anything opened up, because the
/// whole right-hand cluster (a 270 px disc, a 162 px secondary and six plates) has to
/// fit into the left with the chrome band excluded.
fn mirror_search() -> impl Iterator<Item = (f32, f32)> {
    (0..11).flat_map(|step| {
        let dy = -0.05 * step as f32;
        [0.0f32, 0.04, -0.04, 0.08, -0.08].into_iter().map(move |dx| (dx, dy))
    })
}

/// The placement table — ONE arrangement, honoured in every mode. `None` = "use the
/// built-in geometry".
///
/// ★★★ **ONE array, not one per mode**, since 2026-09-10 (player: "menu섹션 완전
/// 삭제할꺼야. 이제 game탭에서 핸들링한대로 메뉴 모드일떄도 전부 동일한 레이아웃으로
/// 나오면돼"). The second array had stopped meaning anything:
///
/// * every control in [`MENU_IDS`] is also in [`FIELD_IDS`] (the menu set is a strict
///   subset — the field set is the whole vocabulary), and
/// * their BUILT-IN seats had become identical in both modes. `Ok`/`Cancel`/`Quick`/
///   `QuickPage` never read `mode` at all, and 80차-F made the menu D-pad seat a
///   one-line delegation to the field one (`overlay::dpad_seat`). The last thing that
///   really differed — the `1 3 7 9 #` key block — went with `CtrlId::MenuKey` in 79차-B,
///   once the touch conversion made the game's own badges tappable.
///
/// So the two editor pages differed only in what the player had dragged onto each, and
/// that difference was the whole defect: moving a control on the GAME page left the same
/// control somewhere else the moment a window opened.
///
/// ★ The MODE still decides WHICH controls exist ([`ids_for`]) and what they may not be
/// placed on ([`obstacles`]) — a seat the field allows on the gear row or the pull-out
/// keypad still falls back to its built-in with a window open. Only the seat REQUEST is
/// shared, and [`Table::mirror`] now validates against both modes so a flip does not
/// quietly produce one of those.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Table {
    places: [Option<Place>; FIELD_IDS.len()],
}

/// A control's index in the one table. Keyed off [`FIELD_IDS`] because that list is the
/// whole vocabulary; a menu-mode lookup therefore finds the very slot the field wrote.
fn slot(id: CtrlId) -> Option<usize> {
    FIELD_IDS.iter().position(|c| *c == id)
}

impl CtrlId {
    /// `==` usable in a `const` block: the derived `PartialEq` is not.
    const fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Quick(a), Self::Quick(b)) => a == b,
            (Self::Ok, Self::Ok)
            | (Self::Cancel, Self::Cancel)
            | (Self::Secondary, Self::Secondary)
            | (Self::Minimap, Self::Minimap)
            | (Self::Stick, Self::Stick)
            | (Self::QuickPage, Self::QuickPage)
            | (Self::MinimapToggle, Self::MinimapToggle)
            | (Self::QuickSave, Self::QuickSave) => true,
            _ => false,
        }
    }
}

/// COMPILE-TIME proof of the one invariant the single table rests on: every control in
/// [`MENU_IDS`] has a slot in [`FIELD_IDS`].
///
/// ★★ Worth a build break rather than a test. If someone adds a control to `MENU_IDS`
/// and forgets the field list, [`slot`] answers `None` for it and [`Table::set`] returns
/// without writing — the control silently becomes unplaceable, and the editor (which
/// only ever shows the field set) would give the player nowhere to notice. This file has
/// been caught by exactly that shape of quiet loss before, and the standalone selftest
/// mirror cannot guard it: the mirror's own id lists have drifted (TODO `BQ`).
const _: () = {
    let mut i = 0;
    while i < MENU_IDS.len() {
        let mut found = false;
        let mut j = 0;
        while j < FIELD_IDS.len() {
            if MENU_IDS[i].same(FIELD_IDS[j]) {
                found = true;
            }
            j += 1;
        }
        assert!(found, "every MENU_IDS control needs a FIELD_IDS slot: the table is keyed on it");
        i += 1;
    }
};

/// Is this the field? Still needed by [`builtin`] and [`obstacles`], which remain
/// mode-dependent — it is only the stored SEATS that no longer are. `Cutscene` counts
/// as off-field (it borrows the MENU control set; see [`ids_for`]).
fn is_field(mode: Mode) -> bool {
    mode == Mode::Field
}

impl Table {
    pub fn get(&self, id: CtrlId) -> Option<Place> {
        self.places[slot(id)?]
    }

    pub fn set(&mut self, id: CtrlId, place: Option<Place>) {
        let Some(i) = slot(id) else { return };
        // ★ There used to be a "position only" clamp here that forced `CtrlId::Stick`
        // back to `PCT_DEFAULT`. Gone 2026-08-16 — every control is resizable now (see
        // the note on `Stick`). Existing `wie_layout.txt` files are unaffected: they
        // already carry `100` on the stick's line, which is what the clamp wrote.
        self.places[i] = place;
    }

    pub fn clear(&mut self) {
        self.places = Default::default();
    }

    /// Mirror one mode LEFT<->RIGHT (player request 2026-07-31: the OK/skill cluster
    /// should be reachable by a left hand, not only on the right).
    ///
    /// Controls with no stored placement get one derived from their built-in, so a
    /// single tap flips the WHOLE layout rather than only the parts already moved.
    ///
    /// ★ A straight mirror is not enough. The bottom-right cluster mirrors into the
    /// bottom-LEFT, and the ring's lower seats sweep through the bottom-centre band the
    /// editor's own chrome sits in — device-checked, 3 of 9 controls were refused and
    /// fell back to their old right-hand seats, leaving a confusing half-flipped
    /// layout. So each control gets a short, ordered search for the nearest legal spot
    /// (straight up first, since the blocker is almost always the bottom band).
    pub fn mirror(&mut self, win_w: u32, win_h: u32) {
        // ── phase 1: mirror everything, unvalidated ──────────────────────────
        //
        // ★ Validating as we go does not work: at control *i* the controls after it are
        // still on the side they came from, so their old seats block the mirror like
        // ghosts. Selftest-caught — the STICK (index 1) was refused because the OK disc
        // had not been mirrored out of the way yet, so the one control a left-handed
        // player most needs to move stayed put.
        for id in FIELD_IDS.iter() {
            let cur = self.get(*id).unwrap_or_else(|| {
                let b = builtin(*id, Mode::Field, win_w, win_h);
                Place::from_rect(b, b, win_w, win_h)
            });
            self.set(*id, Some(Place { cx: 1.0 - cur.cx, ..cur }));
        }
        // ── phase 2: repair whatever the resolver will not honour ────────────
        //
        // ★ A repair may not COST a higher-priority control its place. Selftest-caught
        // and traced: at the first resolve pass the STICK and the OK disc block each
        // other through the seats they have not vacated yet (the stick's old bottom-left
        // is where OK is going, and vice versa). While both were stalled, the `#` plate —
        // last in priority — was repaired into the very spot OK needed, and OK then lost
        // it for good. So each candidate is checked for collateral damage, not just for
        // its own success.
        for i in 0..FIELD_IDS.len() {
            let id = FIELD_IDS[i];
            let hon = self.honoured(win_w, win_h);
            if hon[i] {
                continue;
            }
            let Some(want) = self.get(id) else { continue };
            for (dx, dy) in mirror_search() {
                let cand = Place {
                    cx: (want.cx + dx).clamp(0.0, 1.0),
                    cy: (want.cy + dy).clamp(0.0, 1.0),
                    ..want
                };
                let mut trial = self.clone();
                trial.set(id, Some(cand));
                let after = trial.honoured(win_w, win_h);
                let keeps_betters = hon[..i].iter().zip(&after[..i]).all(|(was, now)| !*was || *now);
                if after[i] && keeps_betters {
                    *self = trial;
                    break;
                }
            }
        }
    }

    /// Per-control: is this control's STORED request the rect the resolver actually
    /// produces — **in every mode the control exists in**? Indexed like [`FIELD_IDS`].
    /// A control with no request counts as honoured (its built-in is always used).
    ///
    /// ★★ BOTH modes since the table was unified (2026-09-10). FLIP used to be checked
    /// only in the mode it was pressed in, and the second table absorbed the difference.
    /// With one table, a mirror that only the field accepts is a control that jumps back
    /// to the right-hand side the moment a window opens — so a candidate has to clear the
    /// MENU obstacle set too (the settings gear). The menu
    /// controls are a subset of the field ones, so this only ever ADDS a constraint, and a
    /// control no candidate satisfies keeps its mirrored request and falls back exactly as
    /// it did before.
    fn honoured(&self, win_w: u32, win_h: u32) -> Vec<bool> {
        let mut out = vec![true; FIELD_IDS.len()];
        for mode in [Mode::Field, Mode::Menu] {
            let ids = ids_for(mode);
            let resolved = resolve_with(mode, win_w, win_h, self);
            for (i, id) in ids.iter().enumerate() {
                let Some(k) = slot(*id) else { continue };
                let Some(p) = self.get(*id) else { continue };
                if resolved[i].1 != p.to_rect(builtin(*id, mode, win_w, win_h), win_w, win_h) {
                    out[k] = false;
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.places.iter().all(Option::is_none)
    }

    /// ★ The `field` token is KEPT in the on-disk format even though only one section
    /// is left. It costs nothing, every existing file already carries it, and it leaves
    /// the format self-describing for anyone reading the file by hand.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for id in FIELD_IDS.iter() {
            if let Some(p) = self.get(*id) {
                out.push_str(&format!("field {} {:.4} {:.4} {}\n", id.key(), p.cx, p.cy, p.pct));
            }
        }
        out
    }
}

/// The prefix of a line written by vc284 and earlier for the second table, the one that
/// no longer exists. Recognised only so [`crate::config::read_layout`] can count those
/// lines as SUPERSEDED instead of as corrupt — dropping them IS the migration.
pub const LEGACY_MENU_PREFIX: &str = "menu ";

/// The on-disk key of the D-pad control that 91차 folded into [`CtrlId::Stick`]. A
/// `field dpad …` line is read as the stick's seat (see [`CtrlId::parse`]);
/// `config::read_layout` counts such lines so the log can say the old arrows' seat was
/// adopted rather than dropped.
pub const LEGACY_DPAD_KEY: &str = "dpad";

/// Is this a `field dpad …` line — the merged control under its pre-91차 name?
pub fn is_legacy_dpad_line(line: &str) -> bool {
    let mut it = line.split_whitespace();
    it.next() == Some("field") && it.next() == Some(LEGACY_DPAD_KEY)
}

/// One `field id cx cy pct` line. `None` = drop this line and keep the rest.
pub fn parse_line(line: &str) -> Option<(CtrlId, Place)> {
    let mut it = line.split_whitespace();
    // ★ Only `field`. A `menu` line is deliberately unparseable now: there is one
    // arrangement and the field section IS it, so an old file keeps its field lines and
    // loses its menu ones. The two sections' built-ins were already identical, so the
    // menu controls simply follow whatever the player arranged on the page that is left.
    if it.next()? != "field" {
        return None;
    }
    let id = CtrlId::parse(it.next()?)?;
    slot(id)?; // reject a control name that is not in the table
    let cx: f32 = it.next()?.parse().ok()?;
    let cy: f32 = it.next()?.parse().ok()?;
    let pct: u32 = it.next()?.parse().ok()?;
    if !cx.is_finite() || !cy.is_finite() || !(0.0..=1.0).contains(&cx) || !(0.0..=1.0).contains(&cy) {
        return None;
    }
    Some((id, Place { cx, cy, pct: pct.clamp(PCT_MIN, PCT_MAX) }))
}

// ── the live table ──────────────────────────────────────────────────────────
//
// Read on every layout and written by the editor, both on the UI thread; the lock
// is for soundness, not contention.

static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(|| RwLock::new(Table::default()));

pub fn table() -> Table {
    TABLE.read().map(|t| t.clone()).unwrap_or_default()
}

pub fn set_table(t: Table) {
    if let Ok(mut g) = TABLE.write() {
        *g = t;
    }
}

pub fn get(id: CtrlId) -> Option<Place> {
    TABLE.read().ok().and_then(|t| t.get(id))
}

pub fn set(id: CtrlId, place: Option<Place>) {
    if let Ok(mut g) = TABLE.write() {
        g.set(id, place);
    }
}

pub fn clear() {
    if let Ok(mut g) = TABLE.write() {
        g.clear();
    }
}

pub fn is_customised() -> bool {
    !TABLE.read().map(|t| t.is_empty()).unwrap_or(true)
}

// ── resolution ──────────────────────────────────────────────────────────────

/// Built-in rect of one control — a pure function of the window, with no player
/// input anywhere in it. This is the fallback whenever a placement will not fit.
///
/// ★ `mode` is no longer read by any arm (91차 removed the last one, the D-pad's). It
/// stays in the signature because every caller has it and the obstacle sets still
/// differ per mode — a built-in that ever needs to differ again has the parameter.
pub fn builtin(id: CtrlId, mode: Mode, win_w: u32, win_h: u32) -> Rect {
    let _ = mode;
    let pad = overlay::pad_of(win_w, win_h);
    match id {
        CtrlId::Ok => overlay::action_disc(win_w, win_h, pad),
        // ★ 2026-08-07 (player: "취소버튼의 크기가 너무 작아서"): the DEFAULT CLR is
        // sized like the SECONDARY-ATTACK button, not like a ring plate.
        // ★★ 80차-H: and it no longer sits ON ring seat 0. That seat is the ring's
        // rightmost, so a disc twice the seat's size ran off the right edge, got clamped
        // back left and landed on plate 1 — see `overlay::cancel_seat`, which puts it
        // between plate 1 and the OK disc instead.
        CtrlId::Cancel => overlay::cancel_seat(win_w, win_h, pad),
        CtrlId::Secondary => overlay::secondary_button(win_w, win_h),
        CtrlId::Minimap => overlay::minimap_panel_builtin(win_w, win_h),
        // The ONE walk seat (91차): the arrows' bounding square, bottom-left. The stick's
        // base is cut FROM it (`overlay::stick_base_radius`), not the other way round —
        // the same rect in every mode, so nothing jumps when a window opens.
        CtrlId::Stick => overlay::stick_seat(win_w, win_h),
        // The quickbar plates keep the ring seats in BOTH modes — the player asked
        // for them in the same place with a window open as without.
        //
        // ★ Through `plate_seat`, not `ring_seat`, since 2026-09-10: a plate switched
        // off in the layout editor gives its seat back and the survivors close the gap,
        // so a plate NUMBER is no longer a seat INDEX.
        CtrlId::Quick(i) => overlay::plate_seat(win_w, win_h, pad, i),
        // ★ One plate-pitch LEFT of the leftmost plate — NOT `ring_seat(SLOTS + 1)`,
        // which was off the end of a six-seat ring and therefore always answered with
        // `ring_seat`'s corner fallback. See `overlay::quickpage_seat`.
        CtrlId::QuickPage => overlay::quickpage_seat(win_w, win_h, pad),
        // The next step along that same row. Field only, like the page button: the
        // guest refuses to save anywhere else.
        CtrlId::QuickSave => overlay::quicksave_seat(win_w, win_h, pad),
        // ★ 141차: the MAP word on the top row, one gap left of SAVE — a toggle now,
        // drawn whether the view is up or not (it used to sit in the panel's corner and
        // appear only while the view was hidden).
        CtrlId::MinimapToggle => overlay::minimap_toggle_seat(win_w, win_h, pad),
    }
}

/// Everything a control may NOT be placed on. Three families, all of them things
/// that win a touch the control would otherwise get:
///
/// 1. **The game's own HUD** — its buff/status band (top-left) and its
///    target-monster box (top-right). Field only; off-field the game draws neither.
///    `map` is the minimap panel as already resolved, because the target box is
///    poked sideways to dodge it (see `overlay::target_box_shift`).
/// 2. **Our own menu chrome** — the settings gear. It exists only in MENU mode, and
///    `app.rs` hit-tests it BEFORE the button list, so a menu key parked on top of it
///    is a key that silently opens the settings instead. (The `123` toggle and the
///    pull-out keypad panel that used to be here were removed 2026-09-18.)
/// 3. **The layout editor's own buttons**, in both modes. The editor hit-tests its
///    chrome first (it must, or a control dropped on DONE would trap the player), so
///    a small plate dragged fully under RESET could never be picked up again — and
///    the tap meant to grab it would wipe that mode's whole layout instead.
pub fn obstacles(mode: Mode, win_w: u32, win_h: u32, map: Rect) -> Vec<Rect> {
    let mut out = obstacles_fixed(mode, win_w, win_h);
    out.extend(target_box_obstacle(mode, win_w, win_h, map));
    out
}

/// Everything in [`obstacles`] that does NOT depend on where the minimap panel ended
/// up — i.e. all of it except the game's target-monster box, which the host pokes
/// sideways to dodge that panel.
///
/// ★ Split out purely so [`resolve_with`] can build it ONCE per resolve instead of
/// once per candidate. It walks the whole six-seat ring (via `edit_items`) and, in
/// menu mode, resolves the entire FIELD layout to find the stick — and the fixpoint
/// was rebuilding all of that up to a hundred times per touch-move event.
fn obstacles_fixed(mode: Mode, win_w: u32, win_h: u32) -> Vec<Rect> {
    let mut out = Vec::new();
    if win_w == 0 || win_h == 0 {
        return out;
    }
    let game = Rect::new(0, 0, win_w, win_h);
    if mode == Mode::Field && crate::immersive_active() && win_w > win_h {
        out.push(overlay::game_buff_box(game));
    }
    // ★ OUR OWN gauge block (level plate + HP/SP/EXP), top-left. It is not a control
    // and cannot be moved, but it IS painted AFTER the controls (`draw_controls`
    // then `draw_stats`), so a control placed underneath is silently covered by the
    // bars. Measured: a FLIP sent the minimap straight into it on every window size
    // in the matrix. Since 2026-08-05 the block is drawn in MENU mode too (player
    // request: "메뉴 버튼 열렸을때 좌측상단 UI는 다 보이게"), so it is an obstacle in
    // every mode, not only the field.
    if crate::immersive_active() && win_w > win_h {
        out.push(overlay::stats_block_box(game));
    }
    // ★ `!is_field`, not `== Menu`: a cutscene resolves against the MENU table, so it
    // has to see the MENU obstacle set too — otherwise the same stored placement is
    // honoured in one mode and silently dropped in the other, and the OK disc moves
    // when a dialogue starts.
    if !is_field(mode) {
        out.push(overlay::gear_rect(win_w, win_h));
        // ★ The STICK is deliberately NOT an obstacle here since 2026-08-09 (player:
        // "메뉴 layout 조절할때는 조이스틱 빨간영역 필요없어. 조이스틱 영역 없애고").
        //
        // It used to be one because the stick's hint is painted in menu mode too, so a
        // key parked on it would look like it belonged to the stick. But it is only a
        // HINT there: `layout.buttons` is hit-tested before the joystick zone, so a
        // control sitting on the stick still gets its own touches — unlike the gear, the
        // keypad and the editor chrome above, which really do steal them. Reserving that
        // corner cost the menu layout the entire bottom-left of the screen and showed up
        // in the editor as a big red block the player could not use.
    }
    out.extend(overlay::edit_items(win_w, win_h).into_iter().map(|i| i.rect));
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

/// The game's own target-monster box where it will actually be drawn once it has
/// dodged a minimap panel at `map` — the one obstacle that moves with the layout.
fn target_box_obstacle(mode: Mode, win_w: u32, win_h: u32, map: Rect) -> Option<Rect> {
    if win_w == 0 || win_h == 0 || mode != Mode::Field || !crate::immersive_active() || win_w <= win_h {
        return None;
    }
    let r = overlay::game_target_box_at(Rect::new(0, 0, win_w, win_h), map);
    (r.w > 0 && r.h > 0).then_some(r)
}

/// Every control of `mode`, in draw / hit-test order, with the player's placement
/// applied wherever it survives the no-overlap invariant.
pub fn resolve(mode: Mode, win_w: u32, win_h: u32) -> Vec<(CtrlId, Rect)> {
    resolve_with(mode, win_w, win_h, &table())
}

/// [`resolve`] against an explicit table — the form the editor uses to ask "if I
/// stored this, would it actually be honoured?".
///
/// ★ Runs to a FIXPOINT rather than in one pass. In a single pass, control *i* is
/// checked against the BUILT-IN rects of every control after it, so a control that
/// has itself been moved away still blocks its old seat like a ghost: the player
/// moves OK out of the corner, drags MAP into the space it vacated, and MAP refuses
/// to go — blocked by an OK that is no longer there. Re-running until nothing more
/// is accepted fixes that, and it always terminates: a control only ever moves from
/// its built-in to its requested rect, once, and an accepted rect can never be
/// invalidated later (acceptance already requires disjointness from every current
/// rect), so the state is monotone with at most one change per control.
pub fn resolve_with(mode: Mode, win_w: u32, win_h: u32, table: &Table) -> Vec<(CtrlId, Rect)> {
    let ids = ids_for(mode);
    // Computed ONCE: each built-in re-derives the whole six-seat ring (up to ten
    // shrink passes of trigonometry), and this runs on every touch-move of a drag.
    let base_rects: Vec<Rect> = ids.iter().map(|id| builtin(*id, mode, win_w, win_h)).collect();
    let mut out: Vec<(CtrlId, Rect)> = ids.iter().copied().zip(base_rects.iter().copied()).collect();
    if win_w == 0 || win_h == 0 {
        return out;
    }
    // Also computed ONCE, for the same reason as `base_rects` (see `obstacles_fixed`).
    let fixed = obstacles_fixed(mode, win_w, win_h);
    for _pass in 0..ids.len() {
        let mut changed = false;
        for i in 0..out.len() {
            let (id, base) = (out[i].0, base_rects[i]);
            let Some(p) = table.get(id) else { continue };
            let cand = p.to_rect(base, win_w, win_h);
            if out[i].1 == cand {
                continue; // already applied
            }
            // The minimap is index 0 in FIELD_IDS, so every obstacle query sees the
            // panel the player actually gets.
            let map = if id == CtrlId::Minimap { cand } else { out[0].1 };
            if accepts(mode, win_w, win_h, i, cand, &out, map, &fixed) {
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

/// Would `cand` be a legal rect for entry `i` of `others`? Used by [`resolve`] and,
/// unchanged, by the editor while a finger is still down — so the layout the player
/// is looking at is exactly the one that will be resolved later.
#[allow(clippy::too_many_arguments)]
fn accepts(mode: Mode, win_w: u32, win_h: u32, i: usize, cand: Rect, others: &[(CtrlId, Rect)], map: Rect, fixed: &[Rect]) -> bool {
    if cand.w == 0 || cand.h == 0 || cand.x + cand.w > win_w || cand.y + cand.h > win_h {
        return false;
    }
    let gap = overlay::pad_of(win_w, win_h);
    let near = cand.grown(gap);
    // ★ CONTROL vs CONTROL is NOT tested at all since 2026-08-07 (player: "버튼 UI
    // 변동이 겹칠 수 없게 되어있는데 이거 겹칠 수 있게 바꿔줘"). It went "no overlap
    // plus a pad gap" (repelling from a long way off) -> a fifth-of-a-side tolerance
    // (2026-08-03, "살짝씩은 겹쳐도 상관없게") -> gone entirely. Two controls sharing
    // pixels is now purely the player's cosmetic choice; a shared touch goes to the
    // first match in `ids_for` order, exactly as it always did.
    //
    // Obstacles below KEEP the strict `grown(gap)` rule, and that asymmetry is the
    // whole point: a control sharing pixels with the settings gear, the pull-out
    // keypad, the editor chrome or the game's own HUD is a control that silently does
    // something else — all of those are hit-tested BEFORE the button list.
    //
    // The game's own HUD widgets are immovable obstacles, not controls: covering
    // them is what the whole target-box poke exists to avoid.
    if others[i].0 == CtrlId::Minimap && !overlay::target_box_can_dodge(win_w, win_h, cand) {
        return false;
    }
    if fixed.iter().any(|o| near.intersects(o)) {
        return false;
    }
    target_box_obstacle(mode, win_w, win_h, map).is_none_or(|o| !near.intersects(&o))
}

/// "If I stored this placement, would it be honoured — and would EVERY OTHER control
/// stay exactly where it is?"
///
/// ★ Answered by RUNNING THE REAL RESOLVER on a table with the candidate in it,
/// rather than by a second, similar-looking predicate. The two used to differ — the
/// editor validated against every neighbour's final position while `resolve`
/// validated the tail of the list against built-ins — so the editor accepted
/// placements the layout then silently dropped: the control snapped back under the
/// finger and a dead line went into `wie_layout.txt`. Asking the resolver itself
/// makes divergence impossible by construction.
///
/// ★ The second half of the question was added 2026-08-03, and it is the fix for the
/// player-reported "설정에서 버튼 크기를 바꾸면 기존 위치에 있던 버튼이 갑자기 자동으로
/// 밀려난다". Asking only "does the edit land?" is not enough, because the resolver is
/// PRIORITY-ORDERED: a control early in `ids_for` that grows or moves into a later
/// control's stored rect wins, and the loser silently drops back to its built-in seat
/// — which on screen is that control leaping across the display. Nothing the player
/// arranged should move except the thing under their finger, so a candidate that
/// costs another control its placement is simply refused; the edited control stops
/// growing (or slides along) instead, which is what "just don't overlap the buttons
/// that are already there" means.
///
/// A control that was ALREADY falling back may still change: that is a repair (it
/// gets the seat the player originally asked for), not a displacement.
pub fn would_accept_place(mode: Mode, win_w: u32, win_h: u32, id: CtrlId, place: Place) -> bool {
    accepts_in(&table(), mode, win_w, win_h, id, place)
}

/// [`would_accept_place`] against an explicit table — the form that can be asked about
/// a table other than the live one.
fn accepts_in(table: &Table, mode: Mode, win_w: u32, win_h: u32, id: CtrlId, place: Place) -> bool {
    let want = place.to_rect(builtin(id, mode, win_w, win_h), win_w, win_h);
    let mut probe = table.clone();
    probe.set(id, Some(place));
    let after = resolve_with(mode, win_w, win_h, &probe);
    if !after.iter().any(|(c, r)| *c == id && *r == want) {
        return false;
    }
    let before = resolve_with(mode, win_w, win_h, table);
    ids_for(mode).iter().enumerate().all(|(j, c)| {
        if *c == id {
            return true;
        }
        let Some(p) = table.get(*c) else {
            return true; // no placement of its own: it sits on its built-in, which never moves
        };
        let asked = p.to_rect(builtin(*c, mode, win_w, win_h), win_w, win_h);
        before[j].1 != asked || after[j].1 == asked
    })
}

/// The resolved rect of one control, or `None` when it does not exist in `mode`.
pub fn rect_of(id: CtrlId, mode: Mode, win_w: u32, win_h: u32) -> Option<Rect> {
    resolve(mode, win_w, win_h)
        .into_iter()
        .find(|(c, _)| *c == id)
        .map(|(_, r)| r)
}
