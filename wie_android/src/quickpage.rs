// Host-owned quickbar PLATES and PAGES.
//
// ── WHAT THE GUEST HAS, AND WHY IT IS EXACTLY FIVE ───────────────────────────
//
// The game has exactly five quickbar slots and no notion of a page: four parallel
// byte arrays inside CHAR, and — this is the part that decides everything in this
// file — the first three are BACK TO BACK with no padding (LGT `binary.mod`,
// md5 `6a352944…`, 2026-09-04):
//
//   type    CHAR+0x252 .. 0x256      itemref CHAR+0x257 .. 0x25b
//   skillref CHAR+0x25c .. 0x260     slotsel CHAR+0x261     skillidx CHAR+0x26c ..
//
// Three independent proofs that the width is five and not "five so far":
//
//   * `serialiseSave` at `0x1af3c` reads `[r1] / [r1+5] / [r1+0xa] / [r1+0x1a]` with
//     `r1 += 1` per iteration and `movs r4,#4 … subs r4,#1 / bpl` — five iterations,
//     twenty bytes, and the strides ARE the array widths.
//   * the reset at `0x49038` does `memset(CHAR+0x252, 0, 5)` — a literal 5 — and
//     clears `skillref` with one `str` plus `strb [r2,#4]`.
//   * the field HUD painter's loop ends on `cmp r3,#4 / bgt` (`0x2c64`) and its strip
//     is a hard-coded `0xaa` = 170 px = five cells.
//
// So `type[5]` IS `itemref[0]`. A sixth guest slot cannot be made without relocating
// four arrays across a dozen functions AND changing the save blob's layout, which
// breaks every existing save. See `QUICKSLOT_EXPAND_FEASIBILITY.md`.
//
// ── WHAT THIS FILE DOES INSTEAD ──────────────────────────────────────────────
//
// The player gets [`PLATES`] plates. The guest's five slots are a WINDOW onto them:
// a plate is "resident" when it currently occupies a guest slot, and pressing a plate
// presses that guest slot's own hotkey (`crate::quickslot_hotkey`). A plate that is
// not resident is admitted first, evicting the least-recently-used one — using the
// SAME four-byte writes a page flip has always used, nothing new.
//
// Why that is safe where a "swap in, press, swap back" would not be: there is no
// window during which the guest holds a binding we intend to undo. Every state the
// guest is ever left in is a state the game itself could have produced, so a save
// landing at any moment (and the warehouse saves constantly — CLAUDE.md) records
// something legitimate.
//
// The game's own HUD never draws any of this: `guest_anchors::hud_bottom_patch`
// replaces `0x2864` with `b 0x304a`, which skips the whole bottom-HUD block. The
// plates on screen are `overlay::draw_quickbar`'s, so plate COUNT is a host-side
// number and nothing in the guest constrains it.
//
// ── WHY THIS STORES IDENTITIES AND NOT BYTES ─────────────────────────────────
//
// The obvious design is "copy the guest's 5 bytes×4 arrays out, copy the other
// page's bytes in". That is wrong, and the RE says exactly why:
//
//  1. **`cat` is a PARTY SLOT, not a character.** `skill_bind` (guest 0x145c48 on
//     KTF) resolves the class from `CHAR+0xd1+cat`. When the party changes,
//     `(cat=0, cell=3)` names a different character's skill. The game knows this:
//     the party-transition routine at guest 0x154416 runs
//     `for cat in 0..=2 { for cell in 0..=5 { skill_bind(cat, cell, 5) } }`
//     (the `movs r2,#5` is the *slot* argument = the unregister sentinel, not a
//     loop count) which drops **every** skill binding. Replaying stored bytes after
//     that resurrects bindings the game deliberately invalidated, pointing at
//     whoever now occupies that party slot.
//  2. **Item refs are bag COORDINATES.** `item-ref = (tab<<5)|slot`. Consuming or
//     selling an item shifts the bag, so a coordinate stored on a dormant plate can
//     come back meaning a different item.
//  3. **Raw writes bypass the game's own bookkeeping** — `skill_bind` calls
//     `unbind_ref` (0x145e98) first so a skill can never sit in two slots, and it
//     writes all four arrays consistently including the icon cell at `CHAR+0x26c`.
//
// So a plate stores **what it was**, stamped with the identity that has to still
// hold: a skill carries the class id that was in its party slot, an item carries the
// (cat, idx) that was in its bag cell. On restore we re-read those and **drop the
// entry if the stamp no longer matches** (user decision 2026-08-04: "검증 실패한 칸만
// 비움"). An empty plate is a plate the player can see is empty; a plate holding the
// wrong skill is one they find out about mid-fight.
//
// (3) is the one the window has to reproduce by hand, and [`Pages::activate`] does:
// before a raw skill bind it clears any OTHER guest slot holding the same skill.
//
// ── WHY RESTORE CALLS THE GAME'S OWN BIND ROUTINE WHERE IT CAN ───────────────
//
// Skills are restored with `call_guest_function(0x145c48, [cat, cell, slot])` where
// that address is known, not by writing bytes. `skill_bind` is self-contained (it
// takes its arguments in registers and never reads the caller's frame — verified at
// 0x145c48..0x145cf8), so it is reachable, and letting it run means the learned
// check, the dedup, the ref packing (which goes through per-class jump tables we
// would otherwise have to reimplement) and the fourth array are all the game's
// problem, not ours. LGT has no located `skill_bind`; see [`Action::BindSkill`] for
// why replaying the captured four bytes is equivalent there.
//
// Items get raw writes because their bind body (guest 0x13ce34) is a jump-table
// target *inside* `popup_choose` whose first instructions read the caller's stack
// frame — it cannot be called. Writing `(tab<<5)|slot` is the whole of what it
// does to the quickbar arrays anyway, and there is no packing table involved.
//
// ── DEPENDENCY-FREE ON PURPOSE ───────────────────────────────────────────────
//
// `quickpage_selftest.rs` at the repo root `include!`s **this file** (not a copy),
// because `cargo test -p wie_android` cannot run on the Windows host — the linked
// winit/android deps abort at startup. Do not add `use crate::...` here.

/// The GUEST's quickbar width. Not configurable: it is the width of four byte arrays
/// in CHAR (see the header) and of the `slot < 5` domain `skill_bind` accepts.
pub const GUEST_SLOTS: usize = 5;

/// How many plates the PLAYER gets, per page. A host-side number — the guest never
/// sees it (the window above translates), and the game's own five-cell HUD strip is
/// patched off, so nothing downstream is sized by it.
///
/// Seven on player request (2026-09-04). Raising it further costs one ring seat and
/// one `wie_layout.txt` key each; the guest side does not change at all.
pub const PLATES: usize = 7;

/// Page 0 is where a fresh install starts. Pages are host shadows of the whole plate
/// row; the store is a `Vec` so raising this is data-only.
pub const PAGES: usize = 2;

/// How many OTHER characters' plate rows the store keeps alongside the active one.
///
/// The game has three save slots and `CHAR+0xb8` names a class, so there are six
/// reachable [`Owner`] keys plus one for "pin unknown". Eight covers that with room to
/// spare, and each parked row is a few hundred bytes of text.
pub const ARCHIVE_MAX: usize = 8;

/// The unregister sentinel `skill_bind` tests at guest 0x145ca6 (`cmp r7,#5`). Only
/// interesting here as the value we must never pass as a real guest slot.
pub const SKILL_BIND_UNREGISTER: u8 = 5;

/// Party slots the skill tables are indexed by (`cat` 0..=2, per guest 0x154416's
/// outer loop bound `cmp #2 / ble`).
pub const PARTY_SLOTS: u8 = 3;

/// Skill strip cells per class sheet (`s{N}_bm` = 9 cells of 15px).
pub const SKILL_CELLS: u8 = 9;

/// How many captures an UNCONFIRMED seat is left alone before the window gives up on
/// it. A seat is unconfirmed from the moment a plate is put in it until a capture
/// actually reads a binding out of that guest slot.
///
/// ── WHY IT EXISTS, TWICE OVER ────────────────────────────────────────────────
///  * Registration hands the bind to the GAME (the host writes the popup's row byte
///    and synthesises `OK` — `items::popup_view`), so the guest bytes appear some
///    frames later. Without this budget the very next capture would read the
///    still-empty slot, call the plate empty and release its residency, and the
///    game's write would land in a slot no plate owns.
///  * A bind of OURS can fail to write anything at all — CHAR unresolvable, or a
///    stored skill with no `sref` on a build with no located `skill_bind` (see
///    [`Action::BindSkill`]). Giving up on such a seat must NOT clear the plate: the
///    binding is still what the player put there, it simply is not in the guest right
///    now. [`Pages::capture`] only empties a plate whose seat was CONFIRMED.
///
/// Eight is the same budget the cash-quantity rollback uses.
pub const PENDING_POLLS: u8 = 8;

// The guest's unregister sentinel IS the guest slot count: `skill_bind` treats slot 5
// as "unbind" precisely because slots 0..4 are the whole array. Both numbers appear in
// this file, so assert they still agree — if the array ever grew, every bound check
// here would be wrong in the direction that corrupts CHAR past the array.
const _: () = assert!(GUEST_SLOTS == SKILL_BIND_UNREGISTER as usize);
// A window smaller than the guest's bar would leave guest slots nothing can reach; a
// plate row smaller than the window would make `admit` unable to fill it.
const _: () = assert!(PLATES >= GUEST_SLOTS);
// The hidden set is a BITMASK in a `u8`, one bit per plate (see below).
const _: () = assert!(PLATES <= 8);

// ── the HIDDEN set (player request 2026-09-10) ───────────────────────────────
//
// "혹시 상단 톱니바퀴에서 스킬 퀵슬롯을 선택 후 비활성화 처리가 가능해? 숨김처리를
// 하는거야 … 퀵슬롯이 너무많아서 불편하다는 유저도있어서". So a plate can be switched
// OFF: it is not drawn, not tappable, not warmed into a guest slot, and not reachable
// from a pad button — and switching it back on restores the binding that was on it,
// which is why hiding NEVER clears the entry (`Pages` keeps the row untouched).
//
// ★ A `u8` bitmask, bit `plate`, and it lives here rather than in `overlay`/`config`
// for one reason: every rule below has to be identical on the drawing side, the touch
// side, the pad side and the worker side, and this file is the one the standalone
// `quickpage_selftest.rs` `include!`s — i.e. the only place the rules can be tested on
// the Windows host at all.
//
// ⛔ The mask is a DEVICE preference, not part of a character's row: it is not
// serialised into `wie_quickpages.txt` and `adopt_owner` must not park or swap it.
// A per-row mask would follow the character, and hiding is about the screen.

/// Is `plate` switched off?
///
/// Out-of-range plates read as VISIBLE, deliberately: this is asked on the drawing and
/// firing paths, and "a plate number I do not understand" must not silently disable a
/// control. The range checks that matter are the ones already in `activate`/`reserve`.
pub fn plate_hidden(mask: u8, plate: u8) -> bool {
    usize::from(plate) < PLATES && mask & (1 << plate) != 0
}

/// The plates that are switched on, ascending. This is also the RING ORDER: seat `n`
/// of the live ring holds `visible_plates(mask)[n]`, which is what makes the remaining
/// plates close the gap and grow when some are hidden.
pub fn visible_plates(mask: u8) -> Vec<u8> {
    (0..PLATES as u8).filter(|p| !plate_hidden(mask, *p)).collect()
}

/// The plates that are switched off, ascending.
pub fn hidden_plates(mask: u8) -> Vec<u8> {
    (0..PLATES as u8).filter(|p| plate_hidden(mask, *p)).collect()
}

/// How many plates are switched on. Never zero — see [`sanitise_hidden`].
pub fn visible_count(mask: u8) -> usize {
    PLATES - hidden_plates(mask).len()
}

/// Where `plate` comes in the visible order, or `None` when it is hidden. The ring
/// seat the geometry uses is `1 + this` (seat 0 belongs to CLR) — see
/// `overlay::plate_seat`.
pub fn visible_index(mask: u8, plate: u8) -> Option<usize> {
    (!plate_hidden(mask, plate) && usize::from(plate) < PLATES)
        .then(|| visible_plates(mask).iter().position(|p| *p == plate))
        .flatten()
}

/// The mask with `plate` switched off, or `None` when that is refused.
///
/// ⛔ The LAST visible plate cannot be hidden. Not an arbitrary floor: with no plate on
/// screen the game's own 퀵슬롯등록 popup has no touch target to register into
/// (`app.rs` answers it through `quick_hit`, and a zero-sized rect never hits), the
/// page-flip and SAVE buttons are sized from a plate that would not exist, and the one
/// screen that could switch a plate back on is the layout editor — which is reached
/// past a modal the player has to know is there. Refusing one plate keeps every one of
/// those alive for the price of a control the player can still ignore.
pub fn hide_plate(mask: u8, plate: u8) -> Option<u8> {
    if usize::from(plate) >= PLATES || plate_hidden(mask, plate) || visible_count(mask) <= 1 {
        return None;
    }
    Some(mask | (1 << plate))
}

/// Everything visible again — what the editor's SHOW button does, and the ONLY way
/// back.
///
/// ★ There is deliberately no per-plate `show`: a hidden plate has no ring seat, so
/// there is nothing on screen to select it by (`placement::shown` keeps it out of the
/// editor too, or the editor would be drawing it at a phantom seat). Restoring the row
/// and switching off the ones you still do not want is two taps more and needs no ghost
/// furniture on the ring.
pub const SHOW_ALL: u8 = 0;

/// Force a mask read from disk (or from a build with a different [`PLATES`]) into the
/// domain this build can honour: bits past the plate row are dropped, and a mask that
/// would leave NOTHING on screen degrades to all-visible.
///
/// ★ Fail-safe at the parse boundary, so no later reader has to repeat it: a file that
/// says "hide everything" must not produce a field with no controls on it — that is
/// the state `ring_slots` has its own ⛔ note about ("Never return an EMPTY ring …
/// that leaves a field with no controls at all").
pub fn sanitise_hidden(mask: u8) -> u8 {
    let m = mask & ((1u16 << PLATES) - 1) as u8;
    if PLATES - hidden_plates(m).len() == 0 { SHOW_ALL } else { m }
}

/// Parse `wie_quick_hidden.txt`: one `hide <n> <n> …` line of 1-BASED plate numbers.
///
/// 1-based because the numbers are the ones printed on the plates and used by
/// `wie_gamepad.txt`'s `Q1..Q7` tokens — a file a player may well edit by hand should
/// not be the one place the plates are counted from zero. Unknown tokens and
/// out-of-range numbers are skipped rather than failing the read: a damaged file has to
/// degrade to "nothing hidden", never to "no quickbar".
pub fn parse_hidden(text: &str) -> u8 {
    let mut mask = 0u8;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        if it.next() != Some("hide") {
            continue;
        }
        // ⛔ No let-chain here: this file is compiled by `quickpage_selftest.rs` under
        // `--edition 2021`, where `if let … && …` does not parse.
        for tok in it {
            match tok.parse::<u8>() {
                Ok(n) if (1..=PLATES as u8).contains(&n) => mask |= 1 << (n - 1),
                _ => {}
            }
        }
    }
    sanitise_hidden(mask)
}

/// The text [`parse_hidden`] reads back. Always writes the keyword, so a file with
/// nothing hidden is a file that says so rather than an empty one.
pub fn serialize_hidden(mask: u8) -> String {
    let mut out = String::from("hide");
    for p in hidden_plates(sanitise_hidden(mask)) {
        out.push_str(&format!(" {}", p + 1));
    }
    out.push('\n');
    out
}

/// What one plate remembers. Identity plus the stamp that must still hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Entry {
    #[default]
    Empty,
    /// `class` = the value at `CHAR+0xd1+cat` when this was captured. The guard.
    ///
    /// `sref` = the skill-ref BYTE the game itself had written for this binding, kept
    /// verbatim. It is not part of the identity — `(cat, cell)` + `class` is — it is what
    /// lets a build with no located `skill_bind` restore the binding anyway, by writing
    /// back the four bytes the game wrote. See [`Action::BindSkill`]. `None` = read from a
    /// store file written before this field existed.
    Skill { cat: u8, cell: u8, class: u8, sref: Option<u8> },
    /// `(cat, idx)` = the item record that was in bag cell `(tab, slot)`. The guard.
    Item { tab: u8, slot: u8, cat: u8, idx: u8 },
    /// ★ 2026-09-14 (소비가방): a consumable that lives in the HOST'S BOX, not in a bag
    /// cell — so there is no `(tab, slot)` and no stamp to hold. The plate is never
    /// RESIDENT: pressing it materialises one unit into a bag cell, borrows a guest slot
    /// for that one press (`activate_boxed`), and the seat is released when the game
    /// has used the cell up. `capture` never overwrites such a plate from a LOAN's guest
    /// read (a 퀵슬롯등록 onto it does replace it — `Window::registering`, 157차), and
    /// `warm_plan` never spends a guest slot on it. See `bagbox_host::UseJob`.
    Boxed { cat: u8, idx: u8 },
}

impl Entry {
    pub fn is_empty(self) -> bool {
        matches!(self, Entry::Empty)
    }

    pub fn is_boxed(self) -> bool {
        matches!(self, Entry::Boxed { .. })
    }

    /// Do these two name the same SKILL? Used for the dedup a raw bind has to do by
    /// hand (`skill_bind` would have done it — header note 3). Items are excluded on
    /// purpose: the game's item bind writes the ref with no dedup at all, so two
    /// plates pointing at one bag cell is a state the game itself produces.
    fn same_skill(self, other: Entry) -> bool {
        match (self, other) {
            (Entry::Skill { cat: a, cell: b, .. }, Entry::Skill { cat: c, cell: d, .. }) => a == c && b == d,
            _ => false,
        }
    }
}

/// One page: [`PLATES`] plates, positionally.
pub type Page = [Entry; PLATES];

/// The live guest reading of one quickbar slot, as the caller resolves it. This is
/// deliberately *not* `crate::QuickSlot` — that type carries icons and counts for
/// drawing, and none of that is identity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Live {
    Empty,
    /// `sref` is the raw skill-ref byte as the game left it (`cat` is decoded from its top
    /// two bits). Carried so a capture can stamp it — see [`Entry::Skill`].
    Skill { cat: u8, cell: u8, sref: u8 },
    Item { tab: u8, slot: u8 },
}

/// One step the worker has to perform against the guest, in order. `guest` is always
/// a GUEST slot (0..[`GUEST_SLOTS`]), never a plate.
///
/// `Clear` comes first for every slot a plan touches so the outcome is deterministic
/// regardless of what was there: `skill_bind`'s dedup can clear *other* slots as a
/// side effect, and starting from a known-empty slot means the result does not depend
/// on the order the binds happen to land in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Zero all four arrays for this guest slot (type=0, item-ref=0, skill-ref=0, cell=-1).
    Clear { guest: u8 },
    /// `call_guest_function(skill_bind, [cat, cell, guest])` where that address is known
    /// for the running build; otherwise the raw four-array write of `sref` / `cell`.
    ///
    /// ── WHY A RAW WRITE IS EQUIVALENT HERE (2026-08-11) ──────────────────────────
    /// The module header's rule — store identities, never bytes — is about *identity*:
    /// `(cat, cell)` names a different skill after a party change, so replaying it blind
    /// is wrong. That is what `class` guards, and it is checked before either mechanism
    /// runs. What `skill_bind` adds on top is (a) the learned check, (b) the `unbind_ref`
    /// dedup and (c) the per-class packing of `cell` into a ref byte:
    ///   (a) the skill was bound when we captured it, and the game has no unlearn;
    ///   (b) the plan clears the target slot AND every other slot holding the same skill
    ///       first (`Pages::activate` / `warm_plan`), which is what the dedup did;
    ///   (c) `sref` IS the packed byte, read back out of the game's own array.
    /// So the four bytes written are the four bytes the game wrote. This is what makes the
    /// feature work on a build whose `skill_bind` has not been located.
    BindSkill { cat: u8, cell: u8, guest: u8, sref: Option<u8> },
    /// Raw write: type=2, item-ref=`(tab<<5)|slot`, skill-ref=0, cell=-1.
    BindItem { item_ref: u8, guest: u8 },
}

impl Action {
    /// The guest slot this step writes. One accessor so a bound check cannot be
    /// written for two of the three variants and forgotten for the third.
    pub fn guest(self) -> u8 {
        match self {
            Action::Clear { guest } | Action::BindSkill { guest, .. } | Action::BindItem { guest, .. } => guest,
        }
    }
}

/// Why a stored entry did not come back. Logged, and worth logging: a player who
/// sees a plate go empty deserves a reason in the bug report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dropped {
    /// The party slot now holds a different class than when this was captured.
    ClassChanged { plate: u8, was: u8, now: Option<u8> },
    /// The bag cell no longer holds the item that was bound.
    ItemMoved { plate: u8, was: (u8, u8), now: Option<(u8, u8)> },
}

/// Who the store belongs to: the pinned save slot plus the live character identity
/// (`items::live_char_identity`).
///
/// ★ WHY THIS EXISTS (2026-09-04). Plates 0..4 have always survived a save load for
/// free — the GUEST carries them in its own save blob, so `Pages::reset` on leaving
/// the shell could throw the host store away and page 0 would be re-captured from the
/// bar the save restored. Plates 5..6 have no guest storage at all: a blanket reset
/// would delete them on every single load, which would make them a different kind of
/// plate from the other five. So the reset is now conditional on the owner having
/// CHANGED. Same safety as before where it matters (a different character never
/// inherits stamps), and the per-entry stamps still gate every individual restore.
///
/// ★★★ WHY THE STORE IS NOW PER-OWNER (2026-09-04, player report). Making the reset
/// conditional was not enough, because the store still held exactly ONE row: loading
/// somebody else threw the active row away to make space for theirs. Reported as —
/// "슬롯2에서 6번,7번에 스킬지정하고 슬롯2만 계속 메인메뉴갔다 실행 하면 잘 저장되어
/// 있는데 다른 슬롯 1, 3에 들어갔다 오면 6,7이 해제되어있어". Exactly right: staying on
/// one character never trips the mismatch, and one visit to another one is
/// unrecoverable, because plates 5..6 have no guest storage to be re-captured from.
///
/// So the outgoing row is PARKED rather than dropped ([`Pages::adopt_owner`]), and
/// coming back finds it. Nothing about the safety changes: a different character still
/// never sees another's stamps — it just gets its own row instead of the wreckage of
/// somebody else's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    pub slot: u8,
    pub identity: u64,
}

/// Which plate is resident in each guest slot, and how recently each was used.
///
/// Session state, never serialised: it describes the GUEST's five bytes, which are
/// rebuilt from the save on every load. [`Pages`] is the durable half.
#[derive(Clone, Debug)]
pub struct Window {
    /// Indexed by guest slot: `(plate, last-used tick)`.
    slots: [Option<(u8, u64)>; GUEST_SLOTS],
    tick: u64,
    /// Indexed by guest slot: `Some(n)` = the binding has not been seen in this slot
    /// yet and `n` more captures may read it empty before the window gives up; `None`
    /// = confirmed (or nothing is seated). See [`PENDING_POLLS`].
    pending: [Option<u8>; GUEST_SLOTS],
    /// Indexed by guest slot: the binding this seat was reserved to REPLACE, still
    /// sitting in the guest because [`Pages::reserve`] deliberately does not clear it.
    ///
    /// ★★ [`PENDING_POLLS`] only ever defended the EMPTY read. A non-empty one was
    /// taken as proof the game had bound — but on the registration path the slot is
    /// non-empty the whole time, with the OUTGOING plate's binding, so a capture that
    /// ran between the synthetic `OK` and the game's own write copied that binding onto
    /// the incoming plate and persisted it. It normally self-heals on the next capture;
    /// when the bind never lands (the commit is refused, the popup is gone) the
    /// duplicate is permanent. Remembering what we expect to be REPLACED is what makes
    /// "this read is not the new one" expressible.
    stale: [Option<Entry>; GUEST_SLOTS],
    /// Indexed by guest slot: the seat was reserved for the GAME's 퀵슬롯등록
    /// ([`Pages::reserve`]) and its write has not been captured yet.
    ///
    /// ★ 157차 (player: *"소비아이템이 등록된 퀵슬롯에 스킬을 등록하려고하니까 스킬 등록이
    /// 안되고있어"*). A BOXED plate has two reasons to hold a seat and they mean opposite
    /// things to a capture: the one-press LOAN (`activate_boxed` — the guest slot says
    /// nothing about the plate) and a REGISTRATION onto it (the guest slot is about to
    /// say what the plate now is). With no way to tell them apart every seat on a boxed
    /// plate was read as a loan, so the skill the game bound was never filed: the plate
    /// stayed the consumable and the skill sat in a guest slot nothing drew.
    registering: [bool; GUEST_SLOTS],
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Window {
    pub fn new() -> Self {
        Self {
            slots: [None; GUEST_SLOTS],
            tick: 0,
            pending: [None; GUEST_SLOTS],
            stale: [None; GUEST_SLOTS],
            registering: [false; GUEST_SLOTS],
        }
    }

    /// Forget every residency. The guest's bar is NOT touched by this — the caller
    /// pairs it with a plan that clears the bar (page flip) or with a fresh capture
    /// (character load).
    pub fn clear(&mut self) {
        self.slots = [None; GUEST_SLOTS];
        self.pending = [None; GUEST_SLOTS];
        self.stale = [None; GUEST_SLOTS];
        self.registering = [false; GUEST_SLOTS];
    }

    pub fn guest_of(&self, plate: u8) -> Option<u8> {
        self.slots
            .iter()
            .position(|s| matches!(s, Some((p, _)) if *p == plate))
            .map(|g| g as u8)
    }

    pub fn plate_at(&self, guest: u8) -> Option<u8> {
        self.slots.get(usize::from(guest)).copied().flatten().map(|(p, _)| p)
    }

    /// Every `(guest, plate)` pair currently live, guest-slot order.
    pub fn residents(&self) -> Vec<(u8, u8)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(g, s)| s.map(|(p, _)| (g as u8, p)))
            .collect()
    }

    fn bump(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// Put `plate` in `guest`, evicting whatever was there, and mark the seat
    /// UNCONFIRMED — somebody is about to write that guest slot and the write may not
    /// have landed (or may not land at all). See [`PENDING_POLLS`].
    pub fn seat(&mut self, plate: u8, guest: u8) {
        self.place(plate, guest, Some(PENDING_POLLS));
    }

    /// Put `plate` in `guest` and treat it as already true. Only for adopting the bar
    /// the GAME just loaded: those five bindings are in the guest by definition, so
    /// making the window wait for them would keep every seat busy for no reason.
    pub fn seat_settled(&mut self, plate: u8, guest: u8) {
        self.place(plate, guest, None);
    }

    fn place(&mut self, plate: u8, guest: u8, pending: Option<u8>) {
        let t = self.bump();
        let g = usize::from(guest);
        if let Some(s) = self.slots.get_mut(g) {
            *s = Some((plate, t));
        }
        if let Some(p) = self.pending.get_mut(g) {
            *p = pending;
        }
        // A fresh seat expects nothing in particular; `expect_replaced` is what arms it.
        if let Some(e) = self.stale.get_mut(g) {
            *e = None;
        }
        self.set_registering(guest, false);
    }

    fn set_registering(&mut self, guest: u8, on: bool) {
        if let Some(r) = self.registering.get_mut(usize::from(guest)) {
            *r = on;
        }
    }

    /// Is `guest`'s seat waiting on the game's own 퀵슬롯등록 write? See the field.
    fn registering(&self, guest: u8) -> bool {
        self.registering.get(usize::from(guest)).copied().unwrap_or(false)
    }

    /// Arm the seat with the binding it is about to REPLACE. Only the registration path
    /// needs this: every other seating either clears the guest slot itself
    /// ([`Pages::press`] emits `Action::Clear`) or adopts a bar that is already true.
    pub fn expect_replaced(&mut self, guest: u8, outgoing: Option<Entry>) {
        if let Some(e) = self.stale.get_mut(usize::from(guest)) {
            *e = outgoing.filter(|o| !o.is_empty());
        }
    }

    /// Drop `guest`'s residency (the caller is clearing that guest slot).
    pub fn vacate(&mut self, guest: u8) {
        let g = usize::from(guest);
        if let Some(s) = self.slots.get_mut(g) {
            *s = None;
        }
        if let Some(p) = self.pending.get_mut(g) {
            *p = None;
        }
        if let Some(e) = self.stale.get_mut(g) {
            *e = None;
        }
        self.set_registering(guest, false);
    }

    /// Mark `guest` as just used, so the LRU picks somebody else next.
    pub fn note_used(&mut self, guest: u8) {
        let t = self.bump();
        if let Some(Some((_, used))) = self.slots.get_mut(usize::from(guest)) {
            *used = t;
        }
    }

    /// The guest slot `plate` should occupy: the one it already has, or a free one,
    /// or the least-recently-used. Returns `(guest, already_resident, evicted_plate)`.
    ///
    /// ★ A free slot is preferred over the LRU even when the LRU is older, because an
    /// empty guest slot costs nothing to fill and evicting a plate the player can see
    /// is the thing to avoid.
    pub fn admit(&mut self, plate: u8) -> (u8, bool, Option<u8>) {
        if let Some(g) = self.guest_of(plate) {
            self.note_used(g);
            return (g, true, None);
        }
        let free = self.slots.iter().position(|s| s.is_none());
        let g = free.unwrap_or_else(|| {
            // `slots` is non-empty (GUEST_SLOTS >= 1), so this always finds one.
            self.slots
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.map(|(_, t)| t).unwrap_or(0))
                .map(|(i, _)| i)
                .unwrap_or(0)
        });
        let evicted = self.slots[g].map(|(p, _)| p);
        self.place(plate, g as u8, Some(PENDING_POLLS));
        (g as u8, false, evicted)
    }

    /// Like [`admit`](Self::admit), and named separately because the caller is the
    /// registration path: the GAME writes the bytes there, not us.
    pub fn reserve(&mut self, plate: u8) -> (u8, Option<u8>) {
        let (g, _, evicted) = self.admit(plate);
        self.set_registering(g, true);
        (g, evicted)
    }

    /// What a capture should do with guest slot `guest`, which read empty.
    ///
    /// `Wait` = the binding has not appeared yet; leave the plate AND the seat alone.
    /// `GiveUp` = it never will; release the seat but leave the plate's entry, because
    /// an unconfirmed seat proves nothing about what the player put on that plate.
    /// `Clear` = the seat was confirmed once, so an empty read is the GAME having
    /// unbound it (an item used up, the party-transition sweep) and the plate follows.
    fn settle_empty(&mut self, guest: u8) -> Settle {
        let g = usize::from(guest);
        match self.pending.get(g).copied().flatten() {
            None => Settle::Clear,
            Some(0) | Some(1) => {
                self.vacate(guest);
                Settle::GiveUp
            }
            Some(n) => {
                self.pending[g] = Some(n - 1);
                Settle::Wait
            }
        }
    }

    /// A capture read `entry` out of `guest`. `true` = it is the binding this seat was
    /// reserved to REPLACE and the game's own write has not landed yet, so the caller
    /// must not file it under the incoming plate.
    ///
    /// Spends the same [`PENDING_POLLS`] budget the empty read does, and for the same
    /// reason it is a budget rather than a wait: the player may have re-registered the
    /// very thing that was there, and a seat that waits forever would leave its plate
    /// empty forever. Anything OTHER than the outgoing binding disarms it at once —
    /// that read can only be the new one.
    fn settle_stale(&mut self, guest: u8, entry: &Entry) -> bool {
        let g = usize::from(guest);
        if self.stale.get(g).copied().flatten().as_ref() != Some(entry) {
            if let Some(e) = self.stale.get_mut(g) {
                *e = None;
            }
            return false;
        }
        match self.pending.get(g).copied().flatten() {
            Some(n) if n > 1 => {
                self.pending[g] = Some(n - 1);
                true
            }
            _ => {
                if let Some(e) = self.stale.get_mut(g) {
                    *e = None;
                }
                false
            }
        }
    }

    /// A capture read a real binding out of `guest`: the seat is true from now on.
    fn confirm(&mut self, guest: u8) {
        if let Some(p) = self.pending.get_mut(usize::from(guest)) {
            *p = None;
        }
        self.set_registering(guest, false);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Settle {
    Wait,
    GiveUp,
    Clear,
}

/// One character's parked plate row. See [`Pages::adopt_owner`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stashed {
    pub owner: Owner,
    current: usize,
    pages: Vec<Page>,
}

/// The host's plate store. Page `current` is the one the window is drawn from.
#[derive(Clone, Debug)]
pub struct Pages {
    pages: Vec<Page>,
    current: usize,
    owner: Option<Owner>,
    /// Other characters' rows, most recently parked first. Bounded by [`ARCHIVE_MAX`].
    stash: Vec<Stashed>,
    /// Which plates the player has switched off (bit per plate, see [`plate_hidden`]).
    ///
    /// ★ Carried on the store rather than passed to every method: `activate`,
    /// `reserve`, `warm_plan` and `adopt_guest_bar` all need it, and the worker already
    /// owns this struct — so one setter per poll keeps the four rules from drifting
    /// apart. ⛔ Not serialised and not touched by `adopt_owner`: it is a device
    /// preference (`wie_quick_hidden.txt`), not part of a character's row.
    hidden: u8,
    /// Has the ACTIVE row been reconciled with the bar the game last loaded?
    ///
    /// ★★★ 148차 (2026-10-01, S25 vc420). Player: *"1번 세이브슬롯과 2번세이브슬롯을
    /// 왔다갔다 계속 빠르게 변경하여 로드하니까 갑자기 퀵슬롯이 날라갔어"*. The worker
    /// swaps rows (`adopt_owner`) and re-seats the window in ONE place, the sync; between a
    /// character load and that sync the active row still belongs to the character who was
    /// just unloaded, and the window describes a bar that no longer exists. Every writer of
    /// the row that reads LIVE state treated that interval as normal play — `capture` filed
    /// slot 1's freshly loaded bar onto slot 2's plates, and the next sync dropped them as
    /// `ClassChanged`. The device log has both halves (00:25:26.710 and 00:25:30.024).
    ///
    /// So `false` from [`desync`](Self::desync) — the worker calls it on every observed
    /// character LOAD and on the title/slot shell — until a sync re-establishes the
    /// window ([`warm_plan`](Self::warm_plan) or [`adopt_guest_bar`](Self::adopt_guest_bar)),
    /// and while it is false every row writer that reads live state or presses a plate is
    /// refused: `capture`, `activate`, `activate_boxed`, `bind_boxed`, `reserve`,
    /// `flip_to`, `migrate_to_boxed`, `drop_exhausted_boxed`, `remap_item_cells`.
    /// Refusing costs at most the few frames of one sync; the alternative is writing one
    /// character's bar, bag or boxes into another character's row.
    ///
    /// Invariant: `!synced` ⇒ the window is empty (`desync` clears it, and every method
    /// that could seat anything is refused until a sync seats it properly).
    /// ⛔ Not serialised — it is a fact about this process's guest, not about any row.
    synced: bool,
}

impl Default for Pages {
    fn default() -> Self {
        Self::new()
    }
}

/// What pressing a plate should do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Activation {
    /// Nothing is bound here. Press nothing — the guest slot the plate would land in
    /// belongs to somebody else, and sending its key would fire the wrong thing.
    ///
    /// `stale` = the plate DID hold something and its stamp no longer holds, so it was
    /// just emptied. The caller persists on that, and only that: a tap on a plate that
    /// was already empty must not rewrite the store file.
    Empty { stale: bool },
    /// Run these first (possibly none), then press `guest`'s own hotkey.
    Press {
        guest: u8,
        actions: Vec<Action>,
        dropped: Vec<Dropped>,
    },
    /// A boxed consumable: materialise one unit of `(cat, idx)` into a bag cell, then
    /// call `activate_boxed` with where it landed.
    Boxed { cat: u8, idx: u8 },
    /// ★ 148차: a character was loaded and the row has not been synced to it yet — the
    /// active row may still be the previous character's. Press nothing, change nothing.
    Unsynced,
}

impl Pages {
    /// ★ 148차: a fresh store counts as synced so the pure tests (and a store that never
    /// meets a guest) keep their meaning; the worker [`desync`](Self::desync)s it at start,
    /// before any bar exists.
    pub fn new() -> Self {
        Self {
            pages: vec![[Entry::Empty; PLATES]; PAGES],
            current: 0,
            owner: None,
            stash: Vec::new(),
            hidden: SHOW_ALL,
            synced: true,
        }
    }

    pub fn current(&self) -> usize {
        self.current
    }

    /// ★ 148차: the guest bar is about to be (or was just) rewritten from a save the row has
    /// not been reconciled with — a character LOAD, or the title/slot shell on the way to
    /// one. Forgets the window and refuses every live-state writer until the next sync
    /// (the `synced` field says why).
    pub fn desync(&mut self, window: &mut Window) {
        window.clear();
        self.synced = false;
    }

    pub fn synced(&self) -> bool {
        self.synced
    }

    /// Publish the player's hidden set into the store. Called by the worker every poll
    /// from the process-wide preference, so a change made in the layout editor is in
    /// force by the next scan without anything having to be plumbed through.
    pub fn set_hidden(&mut self, mask: u8) {
        self.hidden = sanitise_hidden(mask);
    }

    pub fn hidden(&self) -> u8 {
        self.hidden
    }

    // Test-only accessors. The running app never needs them — `next_page` uses the
    // length directly, and `plates()` is the only way anything outside this module
    // should read a page — so they are gated rather than left as dead public API.
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.pages.len()
    }

    /// Move the open page with no window to warm - `flip_to` needs a live character and
    /// these tests are about the STORE, not the guest bar.
    #[cfg(test)]
    pub fn flip_to_for_test(&mut self, to: usize) {
        self.current = to;
    }

    #[cfg(test)]
    pub fn page(&self, i: usize) -> Option<&Page> {
        self.pages.get(i)
    }

    /// The live page's plates, for drawing. Always [`PLATES`] long.
    pub fn plates(&self) -> &Page {
        // `current` is kept in range by every writer (`flip_to`, `parse`), and `pages`
        // is never empty, so the fallback is unreachable — it exists so this cannot
        // panic on a future edit.
        self.pages.get(self.current).unwrap_or(&self.pages[0])
    }

    pub fn plate(&self, plate: u8) -> Entry {
        self.plates().get(usize::from(plate)).copied().unwrap_or(Entry::Empty)
    }

    /// Who the active row belongs to. Test-only since `adopt_owner` became the single
    /// entry point: the app asks it to make an owner active and reads its answer, and
    /// nothing outside needs the raw field any more.
    #[cfg(test)]
    pub fn owner(&self) -> Option<Owner> {
        self.owner
    }

    /// Stamp the active row's owner without adopting. Test-only for the same reason;
    /// the app must go through [`adopt_owner`](Self::adopt_owner), which also parks the
    /// row it is replacing.
    #[cfg(test)]
    pub fn set_owner(&mut self, owner: Option<Owner>) {
        self.owner = owner;
    }

    /// The page a `next` press moves to. Wraps, because the button is one button.
    pub fn next_page(&self) -> usize {
        if self.pages.is_empty() { 0 } else { (self.current + 1) % self.pages.len() }
    }

    // ── capture ──────────────────────────────────────────────────────────────────

    /// Refresh every RESIDENT plate from the guest's live bar, stamping each entry
    /// with the identity that has to hold for it to be restorable later.
    ///
    /// ★ Called every poll, not only at a page flip (2026-09-04). It has to be: a
    /// plate that is not resident is drawn from this store, so a plate registered and
    /// then evicted before any capture would draw empty. It is also what makes the
    /// GAME the authority over resident plates — an item the player used up, or the
    /// party-transition sweep at guest 0x154416, empties the guest slot and this turns
    /// that into an empty plate, exactly as before plates existed.
    ///
    /// `class_of(cat)` = `CHAR+0xd1+cat`; `item_at(tab, slot)` = the (cat, idx) of the
    /// item record in that bag cell. Both return `None` when the read failed, and a
    /// failed read stores `Empty` rather than a half-identity — an entry we cannot
    /// stamp is an entry we could never validate.
    ///
    /// A guest slot that reads empty also loses its residency, which puts it back in
    /// the free pool. The one exception is a slot [`Window::reserve`] is still waiting
    /// on; see [`PENDING_POLLS`].
    ///
    /// Returns whether any plate's ENTRY changed — the caller's cue to persist. It has
    /// to be a return value rather than the caller diffing: this runs several times a
    /// second and the answer is almost always "no", so the store file must not be
    /// rewritten (or even re-serialised) on the strength of a poll happening.
    pub fn capture(
        &mut self,
        window: &mut Window,
        live: &[Live],
        class_of: &impl Fn(u8) -> Option<u8>,
        item_at: &impl Fn(u8, u8) -> Option<(u8, u8)>,
    ) -> bool {
        // ★ 148차: THE reported write. Unsynced, the active row may be the previous
        // character's and `live` the next one's bar.
        if !self.synced || live.len() != GUEST_SLOTS {
            return false;
        }
        let mut changed = false;
        let Some(page) = self.pages.get_mut(self.current) else {
            return false;
        };
        for g in 0..GUEST_SLOTS {
            let Some(plate) = window.plate_at(g as u8) else { continue };
            let entry = match live[g] {
                Live::Skill { cat, cell, sref } => match class_of(cat) {
                    Some(class) => Entry::Skill {
                        cat,
                        cell,
                        class,
                        sref: Some(sref),
                    },
                    None => Entry::Empty,
                },
                Live::Item { tab, slot } => match item_at(tab, slot) {
                    Some((cat, idx)) => Entry::Item { tab, slot, cat, idx },
                    None => Entry::Empty,
                },
                Live::Empty => Entry::Empty,
            };
            // ★ A BOXED plate is never described by the guest's bytes: the seat it holds is
            // the one-press loan `activate_boxed` made, and whatever the guest slot reads
            // now (the loaned item, or empty once the game used it up) says nothing about
            // the plate. Release the seat when it reads empty; never touch the entry.
            //
            // ★ 157차: unless the seat is a REGISTRATION (`Window::registering`) — then
            // the game is binding something else onto this plate and the ordinary rules
            // below apply: an empty read waits (the plate stays boxed if the bind never
            // lands), and the binding the game writes replaces the consumable.
            if page.get(usize::from(plate)).is_some_and(|e| e.is_boxed()) && !window.registering(g as u8) {
                if entry.is_empty() {
                    window.vacate(g as u8);
                }
                continue;
            }
            if entry.is_empty() {
                match window.settle_empty(g as u8) {
                    // Not there yet, or never going to be: either way the plate keeps
                    // what the player put on it. See `PENDING_POLLS`.
                    Settle::Wait | Settle::GiveUp => continue,
                    Settle::Clear => {}
                }
            } else if window.settle_stale(g as u8, &entry) {
                // The OUTGOING binding, read back before the game bound the new one.
                // Leave the plate and the seat exactly as they are.
                continue;
            } else {
                window.confirm(g as u8);
            }
            if let Some(cell) = page.get_mut(usize::from(plate)) {
                changed |= *cell != entry;
                *cell = entry;
            }
            if entry.is_empty() {
                window.vacate(g as u8);
            }
        }
        changed
    }

    /// Seed the window with the guest's own bar, plate `i` <-> guest slot `i`.
    ///
    /// ★ This is the continuity story for every install that predates plates
    /// (2026-09-04): the five bindings the save just restored land on the same five
    /// plates they were on before, and the two new ones start empty. Without it the
    /// window would be empty after a load and the first tap on any plate would evict
    /// a live binding to make room for nothing.
    /// ★★ With plates HIDDEN this is no longer `i <-> i`: the guest slots are handed
    /// out to the VISIBLE plates first, in plate order, and only then to the hidden
    /// ones. Both halves matter.
    ///
    ///  * Visible first, so the five bindings the save just restored land on plates the
    ///    player can actually see and press. Seating them positionally would file the
    ///    game's own hotkeys onto switched-off plates.
    ///  * Hidden ones still get the leftover slots (there are always [`PLATES`] >=
    ///    [`GUEST_SLOTS`] of them), so no guest slot is left owned by nothing. An
    ///    unowned slot is never captured and never cleared, and its binding stays
    ///    firable from the pull-out keypad's own digits — the orphan `reserve`'s ⛔ note
    ///    above is about. It costs nothing: the next `warm_plan` drops those seats.
    pub fn adopt_guest_bar(&mut self, window: &mut Window) {
        window.clear();
        let order: Vec<u8> = visible_plates(self.hidden)
            .into_iter()
            .chain(hidden_plates(self.hidden))
            .collect();
        // ⛔ `seat_settled(plate, guest)` — plate FIRST. The old `seat_settled(g, g)`
        // could not tell the two apart; this one can, and getting it backwards seats
        // plate `g` in guest slot `plate`.
        for (guest, plate) in (0..GUEST_SLOTS as u8).zip(order) {
            window.seat_settled(plate, guest);
        }
        // ★ 148차: one of the two ways a sync ends (the other is `warm_plan`).
        self.synced = true;
    }

    /// On a SAME-character sync: every skill the freshly loaded bar holds that this row
    /// holds nowhere is put on the row's first empty plate, instead of being cleared by
    /// the warm-up that follows. Returns how many plates it filled.
    ///
    /// ★★★ 148차 — THE STORE WINS, BUT IT MAY NOT FORGET. The sync's "same character"
    /// arm clears the bar and warms it from the row, which is right for the arrangement
    /// and for plates 5..6 (they live nowhere else) — and it made the row the ONLY record
    /// of plates 0..4 as well, although the save carries those five bindings. So when the
    /// row lost them (148차: they were overwritten with another character's skills, then
    /// dropped as `ClassChanged`), every later load of that character wiped the save's
    /// copy too. The warm-up's own doc says it: "the guest's five bytes are the right
    /// bindings" — this keeps them wherever the row has nothing to say.
    ///
    /// * Only EMPTY plates are filled, visible ones first (the order `adopt_guest_bar`
    ///   uses) — nothing the row holds is ever replaced, so plates 5..6 are untouched.
    /// * Only skills. An item binding names a bag CELL, which sorting and the box
    ///   sweep move; a stale one would bind whatever sits there now.
    /// * "Holds nowhere" = no plate on ANY page of the row is the same skill.
    /// * Call it AFTER `warm_plan` has dropped the entries whose stamp no longer holds,
    ///   so a plate emptied as `ClassChanged` is free to take the save's binding back —
    ///   then warm again.
    /// ⚠ The one visible consequence: a skill the player took off the bar and then did
    /// not save comes back on an empty plate when that save is reloaded — which is what
    /// reloading a save means for the other five bytes of it anyway.
    pub fn salvage_guest_skills(&mut self, live: &[Live], class_of: &impl Fn(u8) -> Option<u8>) -> usize {
        let order: Vec<u8> = visible_plates(self.hidden)
            .into_iter()
            .chain(hidden_plates(self.hidden))
            .collect();
        let mut filled = 0;
        for l in live {
            let Live::Skill { cat, cell, sref } = *l else { continue };
            // Same door as `parse`: these coordinates reach a guest dispatcher unchecked.
            if cat >= PARTY_SLOTS || cell >= SKILL_CELLS {
                continue;
            }
            let Some(class) = class_of(cat) else { continue };
            let entry = Entry::Skill {
                cat,
                cell,
                class,
                sref: Some(sref),
            };
            if self.pages.iter().any(|page| page.iter().any(|e| e.same_skill(entry))) {
                continue;
            }
            let Some(&plate) = order.iter().find(|&&p| self.plate(p).is_empty()) else {
                break;
            };
            if let Some(slot) = self.pages.get_mut(self.current).and_then(|page| page.get_mut(usize::from(plate))) {
                *slot = entry;
                filled += 1;
            }
        }
        filled
    }

    // ── plans ────────────────────────────────────────────────────────────────────

    /// The steps that put plate `plate` into guest slot `guest`, or `None` when the
    /// stamp no longer holds (the caller is told why through `dropped`).
    fn bind_steps(
        &self,
        plate: u8,
        guest: u8,
        class_of: &impl Fn(u8) -> Option<u8>,
        item_at: &impl Fn(u8, u8) -> Option<(u8, u8)>,
        dropped: &mut Vec<Dropped>,
    ) -> Option<Action> {
        match self.plate(plate) {
            Entry::Empty => None,
            Entry::Skill { cat, cell, class, sref } => {
                // Out-of-domain values can only come from a corrupted store file;
                // refuse them here rather than hand them to a guest dispatcher
                // that has no range check of its own.
                if cat >= PARTY_SLOTS || cell >= SKILL_CELLS {
                    return None;
                }
                match class_of(cat) {
                    Some(now) if now == class => Some(Action::BindSkill { cat, cell, guest, sref }),
                    now => {
                        dropped.push(Dropped::ClassChanged { plate, was: class, now });
                        None
                    }
                }
            }
            // No permanent binding exists for a boxed item; the press path is
            // `activate_boxed`, and `warm_plan` skips these before asking.
            Entry::Boxed { .. } => None,
            Entry::Item { tab, slot: bag, cat, idx } => {
                if tab > 7 || bag > 0x1f {
                    return None;
                }
                match item_at(tab, bag) {
                    Some(now) if now == (cat, idx) => Some(Action::BindItem {
                        item_ref: (tab << 5) | (bag & 0x1f),
                        guest,
                    }),
                    now => {
                        dropped.push(Dropped::ItemMoved {
                            plate,
                            was: (cat, idx),
                            now,
                        });
                        None
                    }
                }
            }
        }
    }

    /// What a tap on `plate` should do, and the writes it needs first.
    ///
    /// Mutates the window (residency is a consequence of using a plate) and the store
    /// (a stamp that no longer holds empties the plate, so the player sees the truth
    /// rather than a ghost that refuses to fire).
    pub fn activate(
        &mut self,
        window: &mut Window,
        plate: u8,
        class_of: &impl Fn(u8) -> Option<u8>,
        item_at: &impl Fn(u8, u8) -> Option<(u8, u8)>,
    ) -> Activation {
        // ★ A HIDDEN plate never fires, and the refusal is here rather than only at the
        // two entry points: a touch cannot reach one (its rect is zero-sized) but a pad
        // button mapped to it still can, and this is the single place both arrive at.
        // `Empty { stale: false }` on purpose — nothing is wrong with the binding, so
        // the store must not be rewritten and the plate must not be emptied.
        // ★ 148차: before the sync the plate row may be the unloaded character's — a press
        // would bind its skill into the new bar, or drop its plate as `ClassChanged`.
        if !self.synced {
            return Activation::Unsynced;
        }
        if usize::from(plate) >= PLATES || plate_hidden(self.hidden, plate) || self.plate(plate).is_empty() {
            return Activation::Empty { stale: false };
        }
        // A boxed consumable: the caller materialises a unit first and comes back
        // through `activate_boxed` with the cell it landed in.
        if let Entry::Boxed { cat, idx } = self.plate(plate) {
            return Activation::Boxed { cat, idx };
        }
        // Already live: press its key and touch the LRU. No guest write at all, which
        // is the common case once the player settles into an arrangement.
        if let Some(g) = window.guest_of(plate) {
            window.note_used(g);
            return Activation::Press {
                guest: g,
                actions: Vec::new(),
                dropped: Vec::new(),
            };
        }
        // Validate BEFORE evicting anything: a plate whose stamp is stale must not
        // cost a live plate its seat.
        let mut dropped = Vec::new();
        let probe = self.bind_steps(plate, 0, class_of, item_at, &mut dropped);
        if probe.is_none() {
            if let Some(cell) = self.pages[self.current].get_mut(usize::from(plate)) {
                *cell = Entry::Empty;
            }
            return Activation::Empty { stale: true };
        }
        let (guest, _, _) = window.admit(plate);
        let mut actions = vec![Action::Clear { guest }];
        // The dedup `skill_bind` would have done (header note 3): the same skill may
        // not sit in two guest slots. The other plate keeps its ENTRY — it is simply
        // no longer resident, so it draws from the store and re-binds when tapped.
        let me = self.plate(plate);
        for (g, other) in window.residents() {
            if g != guest && self.plate(other).same_skill(me) {
                actions.push(Action::Clear { guest: g });
                window.vacate(g);
            }
        }
        let mut sink = Vec::new();
        if let Some(bind) = self.bind_steps(plate, guest, class_of, item_at, &mut sink) {
            actions.push(bind);
        }
        Activation::Press {
            guest,
            actions,
            dropped,
        }
    }

    /// The one-press loan for a BOXED plate: the unit has just been materialised into
    /// bag cell `(tab, slot)`; seat the plate (evicting the LRU if the bar is full),
    /// point the guest slot at that cell, and press. The seat is released by the next
    /// capture that reads the slot empty — the game unbinds a used-up cell itself
    /// (`0x466b4`) — and the plate's entry is never touched.
    pub fn activate_boxed(&mut self, window: &mut Window, plate: u8, tab: u8, slot: u8) -> Activation {
        if !self.synced {
            return Activation::Unsynced;
        }
        if usize::from(plate) >= PLATES || plate_hidden(self.hidden, plate) || !self.plate(plate).is_boxed() || tab > 7 || slot > 0x1f {
            return Activation::Empty { stale: false };
        }
        let (guest, _, _) = window.admit(plate);
        // A loan from here on, whatever the seat was reserved for (157차).
        window.set_registering(guest, false);
        let actions = vec![Action::Clear { guest }, Action::BindItem { item_ref: (tab << 5) | (slot & 0x1f), guest }];
        Activation::Press {
            guest,
            actions,
            dropped: Vec::new(),
        }
    }

    /// Put a BOXED consumable on `plate` (the box window's 퀵슬롯등록). Returns the
    /// guest writes that free the seat the plate held before, if any — a boxed plate
    /// owns no seat between presses. `None` for a plate that cannot take it.
    pub fn bind_boxed(&mut self, window: &mut Window, plate: u8, cat: u8, idx: u8) -> Option<Vec<Action>> {
        if !self.synced || usize::from(plate) >= PLATES || plate_hidden(self.hidden, plate) {
            return None;
        }
        let mut actions = Vec::new();
        if let Some(g) = window.guest_of(plate) {
            actions.push(Action::Clear { guest: g });
            window.vacate(g);
        }
        let cell = self.pages.get_mut(self.current)?.get_mut(usize::from(plate))?;
        *cell = Entry::Boxed { cat, idx };
        Some(actions)
    }

    /// The game's 퀵슬롯등록 is about to bind onto `plate`: if that plate is BOXED and
    /// still holds the seat of a one-press loan, give the seat up. Returns the guest
    /// write that empties it. Call it right before [`reserve`](Self::reserve).
    ///
    /// ★ 157차. The loaned slot may still hold the item ref `activate_boxed` wrote (the
    /// unit was not used, or the staged cell was put back under it). `reserve` would hand
    /// the same seat back as the registration's, and the capture that runs before the
    /// game's write would file that ref on the plate as an `Item`. Cleared, the slot
    /// reads empty until the game binds — which is what a registration seat waits on.
    /// The same clear [`bind_boxed`](Self::bind_boxed) does, for the same reason.
    pub fn release_boxed_seat(&mut self, window: &mut Window, plate: u8) -> Vec<Action> {
        if !self.synced || plate_hidden(self.hidden, plate) || !self.plate(plate).is_boxed() {
            return Vec::new();
        }
        let Some(guest) = window.guest_of(plate) else {
            return Vec::new();
        };
        window.vacate(guest);
        vec![Action::Clear { guest }]
    }

    /// ★ 136차 (2026-09-25) — the 정렬 moved bag cells: point every `Item` plate of the
    /// ACTIVE row at its item's new cell. The guest's own refs are rewritten by the same
    /// sort (`items::bag_sort_writes`), so without this the next `capture`/`plan` would
    /// read the stored `(tab, slot)` against a different item and drop the plate as
    /// `ItemMoved`. `moved(tab, slot)` = the new cell of the item that was there, or
    /// `None` when that cell held nothing (the plate is left as it was). Returns whether
    /// anything changed — the store is dirty then.
    pub fn remap_item_cells(&mut self, moved: &impl Fn(u8, u8) -> Option<(u8, u8)>) -> bool {
        // (No let-chains: `quickpage_selftest.rs` compiles this file as edition 2021.)
        // ★ 148차: `moved` describes the LIVE bag; unsynced, the row may be someone else's.
        if !self.synced {
            return false;
        }
        let mut changed = false;
        for page in self.pages.iter_mut() {
            for e in page.iter_mut() {
                let Entry::Item { tab, slot, cat, idx } = *e else { continue };
                match moved(tab, slot) {
                    Some((t, s)) if (t, s) != (tab, slot) => {
                        *e = Entry::Item { tab: t, slot: s, cat, idx };
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        changed
    }

    /// Turn every `Item` plate whose category now lives in a box into a `Boxed` plate
    /// (the sweep moved its cell; the stamp would otherwise fail as `ItemMoved` and the
    /// plate would go empty). Returns the seats to clear — the guest slot still points
    /// at a cell that is about to be emptied — and whether anything changed.
    pub fn migrate_to_boxed(&mut self, window: &mut Window, boxed: &impl Fn(u8) -> bool) -> (Vec<Action>, bool) {
        let mut actions = Vec::new();
        let mut changed = false;
        // ★ 148차: the sweep is about the LIVE bag. Unsynced, the active row may belong to
        // the character that was just unloaded (the online slot list's load, or the few
        // frames before a sync) — rewriting its plates would describe someone else's bag.
        if !self.synced {
            return (actions, changed);
        }
        for plate in 0..PLATES as u8 {
            let Entry::Item { cat, idx, .. } = self.plate(plate) else { continue };
            if !boxed(cat) {
                continue;
            }
            if let Some(g) = window.guest_of(plate) {
                actions.push(Action::Clear { guest: g });
                window.vacate(g);
            }
            if let Some(cell) = self.pages.get_mut(self.current).and_then(|p| p.get_mut(usize::from(plate))) {
                *cell = Entry::Boxed { cat, idx };
                changed = true;
            }
        }
        (actions, changed)
    }

    /// Empty every BOXED plate whose consumable is gone — the box holds none of it and
    /// the bag holds none either. Returns how many plates were emptied.
    ///
    /// ★ 2026-09-16 (player: *"소비아이템을 다 써도 퀵슬롯 플레이트가 안 비고, 그 상태로
    /// 쓰면 「남은것이 없습니다」가 뜬다"*). For a plate on a BAG cell the game itself
    /// empties the seat when the stack is used up (`0x466b4(page, cell, 1)` clears the
    /// matching item refs) and the next capture empties the plate. A boxed plate has no
    /// bag cell and no seat, so nothing ever told it — it kept drawing with a `0` badge
    /// and answered a press with a toast. This is that missing unbind, host-side.
    ///
    /// ⛔ `gone` must answer for the BAG as well as the box: a stack the sweep could not
    /// take yet (box full at the time) is still the player's, and clearing its plate
    /// would make them register it again only to have the sweep box it a poll later.
    /// ⛔ Hidden plates are left alone — hiding never touches a binding (82차) — and a
    /// hidden plate that comes back with a `0` badge is emptied by the next poll anyway.
    /// ⛔ Every page, not just the open one: the count is the box's, not a page's.
    pub fn drop_exhausted_boxed(&mut self, gone: &impl Fn(u8, u8) -> bool) -> usize {
        // ★ 148차: `gone` answers for the LIVE character's boxes and bag. Unsynced, the
        // active row may be the previous character's, and a consumable THEY have plenty of
        // would read as used up and cost them the plate.
        if !self.synced {
            return 0;
        }
        let hidden = self.hidden;
        let mut n = 0;
        for page in &mut self.pages {
            for (i, cell) in page.iter_mut().enumerate() {
                let Entry::Boxed { cat, idx } = *cell else { continue };
                if plate_hidden(hidden, i as u8) || !gone(cat, idx) {
                    continue;
                }
                *cell = Entry::Empty;
                n += 1;
            }
        }
        n
    }

    /// Reserve a guest slot for `plate` so the GAME can bind into it (the 퀵슬롯등록
    /// flow: the host writes the popup's row byte, the game writes the quickbar).
    /// Returns the guest slot whose row the popup must be pointed at.
    ///
    /// No [`Action`]s: the game is about to overwrite that slot itself, so clearing it
    /// first would only widen the window in which the bar is missing a binding.
    pub fn reserve(&mut self, window: &mut Window, plate: u8) -> Option<u8> {
        // ⛔ HIDDEN plates are refused HERE, before `window.reserve` — the same rule the
        // popup-validity gate in `emulator.rs` is written to: reserving first would
        // evict a live plate on the way to doing nothing and leave its binding in a
        // guest slot no plate owns, which a pad digit could then still fire.
        // ★ 148차: and before a sync the row may be the unloaded character's.
        if !self.synced || usize::from(plate) >= PLATES || plate_hidden(self.hidden, plate) {
            return None;
        }
        let (guest, evicted) = window.reserve(plate);
        // ★★ What is still IN that guest slot is, by the window's own invariant, the
        // evicted plate's binding — this path writes nothing, so the game's bind is the
        // only thing that will change it. Telling the seat so is what keeps the next
        // capture from filing the outgoing binding under `plate`.
        let outgoing = evicted
            .and_then(|p| self.pages.get(self.current).and_then(|page| page.get(usize::from(p))))
            .copied();
        window.expect_replaced(guest, outgoing);
        Some(guest)
    }

    /// Clear the guest's bar and seat the first [`GUEST_SLOTS`] non-empty plates of the
    /// current page in it. Used after a page flip and after a character load.
    ///
    /// Plate order, not recency: right after a flip there is no recency to speak of,
    /// and "the leftmost plates are the ones that are live" is a rule the player can
    /// see. Every plate past the window is one tap away regardless.
    pub fn warm_plan(
        &mut self,
        window: &mut Window,
        class_of: &impl Fn(u8) -> Option<u8>,
        item_at: &impl Fn(u8, u8) -> Option<(u8, u8)>,
    ) -> (Vec<Action>, Vec<Dropped>) {
        window.clear();
        let mut actions: Vec<Action> = (0..GUEST_SLOTS as u8).map(|guest| Action::Clear { guest }).collect();
        let mut dropped = Vec::new();
        let mut guest = 0u8;
        for plate in 0..PLATES as u8 {
            if usize::from(guest) >= GUEST_SLOTS {
                break;
            }
            // ★ A hidden plate is skipped, so it stops consuming one of the five guest
            // slots — which is why hiding two plates makes the remaining five fit the
            // window exactly and the LRU stop churning. ⚠ From the NEXT warm, that is:
            // this plan runs on the shell → world edge (a character load, an app
            // restart), and a plate hidden mid-session keeps the seat it already holds
            // until `reserve` evicts it (code review 2026-09-11). Its ENTRY is untouched: the
            // store is the only record of what is on it, and for plates past the fifth
            // it is the ONLY record there has ever been (the 6·7 loss of 80차).
            if plate_hidden(self.hidden, plate) || self.plate(plate).is_empty() || self.plate(plate).is_boxed() {
                continue;
            }
            // A stamp that no longer holds empties the plate here too, for the same
            // reason as in `activate`: a ghost that can never fire is worse than a
            // visibly empty plate.
            match self.bind_steps(plate, guest, class_of, item_at, &mut dropped) {
                Some(bind) => {
                    actions.push(bind);
                    window.seat(plate, guest);
                    guest += 1;
                }
                None => {
                    if let Some(cell) = self.pages[self.current].get_mut(usize::from(plate)) {
                        *cell = Entry::Empty;
                    }
                }
            }
        }
        // ★ 148차: the window now says exactly what these writes put in the bar — one of
        // the two ways a sync ends (the other is `adopt_guest_bar`). `flip_to`, the other
        // caller, is refused unsynced, so a flip can never be what ends one.
        self.synced = true;
        (actions, dropped)
    }

    /// Move to `to`, capturing the live bar into the current page first. Returns the
    /// steps that reproduce `to` in the guest, plus whatever had to be dropped.
    ///
    /// Capture-then-switch is one operation on purpose: a flip that captured but
    /// failed to switch, or switched without capturing, loses a page.
    pub fn flip_to(
        &mut self,
        to: usize,
        window: &mut Window,
        live: &[Live],
        class_of: &impl Fn(u8) -> Option<u8>,
        item_at: &impl Fn(u8, u8) -> Option<(u8, u8)>,
    ) -> (Vec<Action>, Vec<Dropped>) {
        // ★ 148차: unsynced, the capture below would be refused anyway — and switching
        // then warming would write the unloaded character's other page into the new bar.
        if !self.synced || to >= self.pages.len() || to == self.current {
            return (Vec::new(), Vec::new());
        }
        self.capture(window, live, class_of, item_at);
        self.current = to;
        self.warm_plan(window, class_of, item_at)
    }

    /// Forget every page and go back to page 0.
    ///
    /// ⛔ NOT the character-changed path any more — that is
    /// [`adopt_owner`](Self::adopt_owner), which PARKS the outgoing row instead of
    /// destroying it. This was that path until 2026-09-04, and calling it on a character
    /// change is what silently deleted plates 5..6 the first time the player loaded
    /// another save slot. Kept as the "wipe everything" primitive the tests pin.
    #[cfg(test)]
    pub fn reset(&mut self) {
        for p in &mut self.pages {
            *p = [Entry::Empty; PLATES];
        }
        self.current = 0;
        self.owner = None;
    }

    /// Make `owner` the active row, PARKING the outgoing one instead of dropping it.
    ///
    /// Returns true when the store already holds a row for them - the caller then warms
    /// the guest bar from it. False means this key is new here and the guest's own bar
    /// is the truth (`adopt_guest_bar`).
    ///
    /// `None` is "we could not read who this is": the outgoing row is still parked, and
    /// the active row starts empty rather than claiming a key it cannot verify. That row
    /// is then found again by a later scan that CAN read the identity, which is the whole
    /// difference from the `reset()` this replaced.
    ///
    /// DO NOT short-circuit on `self.owner == owner` when both are `None`: two different
    /// unidentified characters must not end up sharing a row.
    pub fn adopt_owner(&mut self, owner: Option<Owner>) -> bool {
        if self.owner.is_some() && self.owner == owner {
            return true;
        }
        if let Some(prev) = self.owner {
            self.stash.retain(|s| s.owner != prev);
            self.stash.insert(
                0,
                Stashed {
                    owner: prev,
                    current: self.current,
                    pages: core::mem::take(&mut self.pages),
                },
            );
            self.stash.truncate(ARCHIVE_MAX);
        }
        self.pages = vec![[Entry::Empty; PLATES]; PAGES];
        self.current = 0;
        self.owner = owner;
        if let Some(o) = owner {
            if let Some(i) = self.stash.iter().position(|s| s.owner == o) {
                let st = self.stash.remove(i);
                self.pages = st.pages;
                self.current = st.current;
                return true;
            }
        }
        false
    }

    /// How many OTHER characters' rows are parked. Diagnostics and tests.
    pub fn parked(&self) -> usize {
        self.stash.len()
    }

    /// 초기화 on save slot `slot`: every row keyed to that slot is gone with the character.
    ///
    /// ★★★ WHY (2026-09-15, player report): *"새 캐릭터 만들어서 스킬배운뒤에 그 캐릭터
    /// 삭제하고 새로운 캐릭터를 해당슬롯에 만들면 새로운 캐릭터가 기존 삭제된 캐릭터의
    /// 스킬퀵슬롯이 남겨진채로 시작"*. The [`Owner`] key is `(slot, identity)` and the
    /// identity is the CLASS name at `CHAR+0xb8` (71차-L) — 리츠 or 케이, nothing more —
    /// so a new character of the same class in the same slot has the SAME key as the one
    /// the player just deleted. `adopt_owner` then finds the old row, `warm_plan` writes
    /// its skills into the fresh guest bar, and the new character starts with the dead
    /// one's quickslots. The guest side is clean (a new save has an empty bar); the store
    /// is what carries them across.
    ///
    /// The deletion is the one event that says "this key's row is over": on LGT the only
    /// guest-reachable path that removes a character is 초기화 (`deleteSaveRecord 0x1a2b8`
    /// → `AndroidDatabaseRepository::delete`), the same place the bag boxes are dropped
    /// (`bagbox_host::forget_slot`). So the repository notes the slot and the worker calls
    /// this on its next poll. Both the ACTIVE row (if it is that slot's) and any PARKED row
    /// for that slot go. The active row is cleared rather than parked — parking it would
    /// hand it straight back to the next same-class character. Returns whether anything
    /// changed, so the caller can persist.
    ///
    /// ⛔ Not keyed on identity: the identity cannot tell the two characters apart, which
    /// is the whole bug. ⛔ Not a reason to touch `hidden` — that is a device setting.
    pub fn forget_slot(&mut self, slot: u8) -> bool {
        let mut changed = false;
        if self.owner.is_some_and(|o| o.slot == slot) {
            self.pages = vec![[Entry::Empty; PLATES]; PAGES];
            self.current = 0;
            self.owner = None;
            changed = true;
        }
        let before = self.stash.len();
        self.stash.retain(|s| s.owner.slot != slot);
        changed |= self.stash.len() != before;
        changed
    }

    // ── persistence ─────────────────────────────────────────────────────────────
    //
    // A tiny line format, for the same reason the layout table uses one: it is
    // greppable in a bug report, and an unparsable line costs one slot instead of
    // the file. Unknown/short lines are skipped, never fatal.
    //
    //   cur <page>
    //   own <slot> <identity>
    //   s <page> <plate> <cat> <cell> <class> [<sref>]
    //   i <page> <plate> <tab> <bag> <cat> <idx>
    //   arc <slot> <identity>     <- begins a PARKED row; cur/s/i under it are its own
    //
    // `arc` is a separate keyword rather than a second `own` on purpose: `own` is
    // optional, so a store with no active owner but a parked row would otherwise have
    // its first `arc` read as the active `own` and swallow that row's entries.
    //
    // `sref` was appended on 2026-08-11 and is OPTIONAL on the way in, so a store file
    // written before it still loads (its skills then restore only where `skill_bind` is
    // callable — see `Action::BindSkill`). `own` was appended on 2026-09-04 and its
    // absence simply means "unknown owner", which reads as a mismatch and resets —
    // i.e. exactly the old behaviour for a file written by an older build. Appending
    // rather than renumbering is what keeps both true.

    pub fn serialize(&self) -> String {
        let mut out = format!("cur {}\n", self.current);
        if let Some(o) = self.owner {
            out.push_str(&format!("own {} {}\n", o.slot, o.identity));
        }
        Self::write_row(&mut out, &self.pages);
        for st in &self.stash {
            out.push_str(&format!("arc {} {}
", st.owner.slot, st.owner.identity));
            out.push_str(&format!("cur {}
", st.current));
            Self::write_row(&mut out, &st.pages);
        }
        out
    }

    fn write_row(out: &mut String, pages: &[Page]) {
        for (p, page) in pages.iter().enumerate() {
            for (s, e) in page.iter().enumerate() {
                match *e {
                    Entry::Empty => {}
                    Entry::Skill { cat, cell, class, sref } => {
                        out.push_str(&format!("s {p} {s} {cat} {cell} {class}"));
                        if let Some(r) = sref {
                            out.push_str(&format!(" {r}"));
                        }
                        out.push('\n');
                    }
                    Entry::Item { tab, slot, cat, idx } => {
                        out.push_str(&format!("i {p} {s} {tab} {slot} {cat} {idx}\n"));
                    }
                    Entry::Boxed { cat, idx } => {
                        out.push_str(&format!("b {p} {s} {cat} {idx}\n"));
                    }
                }
            }
        }
    }

    pub fn parse(text: &str) -> Self {
        // The ACTIVE row accumulates into `me`, exactly as it did before sections
        // existed. Once an `arc` header is seen every following `cur`/`s`/`i` belongs to
        // THAT parked row instead, until the next `arc` or end of file — so a store
        // written by an older build (cur, own, entries; no `arc` at all) parses into the
        // active row and nothing else, which is the whole compatibility story.
        let mut me = Self::new();
        let mut arc: Option<Stashed> = None;
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| -> Option<u32> { f.get(i)?.parse().ok() };
            let owner_of = || -> Option<Owner> {
                let (Some(slot), Some(id)) = (num(1), f.get(2).and_then(|s| s.parse::<u64>().ok())) else {
                    return None;
                };
                u8::try_from(slot).ok().map(|slot| Owner { slot, identity: id })
            };
            match f.first().copied() {
                Some("cur") => {
                    if let Some(c) = num(1) {
                        let c = c as usize;
                        if c < PAGES {
                            match arc.as_mut() {
                                Some(a) => a.current = c,
                                None => me.current = c,
                            }
                        }
                    }
                }
                // Only ever the ACTIVE row's owner. A stray `own` inside a parked
                // section is ignored rather than silently re-keying the active one.
                Some("own") if f.len() == 3 && arc.is_none() => {
                    if let Some(o) = owner_of() {
                        me.owner = Some(o);
                    }
                }
                Some("arc") if f.len() == 3 => {
                    if let Some(a) = arc.take() {
                        me.park_parsed(a);
                    }
                    arc = owner_of().map(|owner| Stashed {
                        owner,
                        current: 0,
                        pages: vec![[Entry::Empty; PLATES]; PAGES],
                    });
                }
                Some("s") if f.len() == 6 || f.len() == 7 => {
                    let (Some(p), Some(s), Some(cat), Some(cell), Some(class)) = (num(1), num(2), num(3), num(4), num(5)) else {
                        continue;
                    };
                    // Validate at the door: the restore path hands `cat`/`cell` to a
                    // guest function with no range check, so a hand-edited file must
                    // not be able to reach it.
                    if cat >= u32::from(PARTY_SLOTS) || cell >= u32::from(SKILL_CELLS) {
                        continue;
                    }
                    // A 7th field out of byte range is junk, not a binding: drop the whole
                    // line rather than restore a skill with a truncated ref.
                    let sref = match num(6) {
                        Some(r) if r <= 0xff => Some(r as u8),
                        Some(_) => continue,
                        None if f.len() == 7 => continue,
                        None => None,
                    };
                    let row = match arc.as_mut() {
                        Some(a) => &mut a.pages,
                        None => &mut me.pages,
                    };
                    if let Some(slot) = slot_in(row, p, s) {
                        *slot = Entry::Skill {
                            cat: cat as u8,
                            cell: cell as u8,
                            class: class as u8,
                            sref,
                        };
                    }
                }
                Some("b") if f.len() == 5 => {
                    let (Some(p), Some(s), Some(cat), Some(idx)) = (num(1), num(2), num(3), num(4)) else {
                        continue;
                    };
                    if cat > 0xff || idx > 0xff {
                        continue;
                    }
                    let row = match arc.as_mut() {
                        Some(a) => &mut a.pages,
                        None => &mut me.pages,
                    };
                    if let Some(slot) = slot_in(row, p, s) {
                        *slot = Entry::Boxed { cat: cat as u8, idx: idx as u8 };
                    }
                }
                Some("i") if f.len() == 7 => {
                    let (Some(p), Some(s), Some(tab), Some(bag), Some(cat), Some(idx)) = (num(1), num(2), num(3), num(4), num(5), num(6)) else {
                        continue;
                    };
                    if tab > 7 || bag > 0x1f {
                        continue;
                    }
                    let row = match arc.as_mut() {
                        Some(a) => &mut a.pages,
                        None => &mut me.pages,
                    };
                    if let Some(slot) = slot_in(row, p, s) {
                        *slot = Entry::Item {
                            tab: tab as u8,
                            slot: bag as u8,
                            cat: cat as u8,
                            idx: idx as u8,
                        };
                    }
                }
                _ => {}
            }
        }
        if let Some(a) = arc.take() {
            me.park_parsed(a);
        }
        me
    }

    /// File order is parked order, so this appends. A row whose key duplicates the
    /// active owner or one already parked is dropped: the store must not be able to
    /// hold two rows for one character, or which one `adopt_owner` finds would depend
    /// on line order.
    fn park_parsed(&mut self, row: Stashed) {
        if self.owner == Some(row.owner) || self.stash.iter().any(|s| s.owner == row.owner) {
            return;
        }
        if self.stash.len() < ARCHIVE_MAX {
            self.stash.push(row);
        }
    }

}

fn slot_in(pages: &mut [Page], page: u32, plate: u32) -> Option<&mut Entry> {
    pages.get_mut(page as usize)?.get_mut(plate as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Party slot 0 = class 7, slot 1 = class 4, slot 2 absent.
    fn classes(cat: u8) -> Option<u8> {
        match cat {
            0 => Some(7),
            1 => Some(4),
            _ => None,
        }
    }

    /// Bag cell (0,0) holds item (0x12, 0x01); (0,1) holds (0x12, 0x00).
    fn bag(tab: u8, slot: u8) -> Option<(u8, u8)> {
        match (tab, slot) {
            (0, 0) => Some((0x12, 0x01)),
            (0, 1) => Some((0x12, 0x00)),
            _ => None,
        }
    }

    fn live5(v: [Live; GUEST_SLOTS]) -> Vec<Live> {
        v.to_vec()
    }

    const SK0: Live = Live::Skill { cat: 0, cell: 3, sref: 0x03 };
    const SK1: Live = Live::Skill { cat: 1, cell: 8, sref: 0x48 };

    /// A store whose page 0 holds `entries`, seated identity-wise where they fit.
    fn seeded(entries: [Live; GUEST_SLOTS]) -> (Pages, Window) {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        p.capture(&mut w, &live5(entries), &classes, &bag);
        (p, w)
    }

    // ── the shape of the thing ───────────────────────────────────────────────

    #[test]
    fn a_fresh_store_is_empty_on_page_zero_with_seven_plates() {
        let p = Pages::new();
        assert_eq!(p.current(), 0);
        assert_eq!(p.count(), PAGES);
        assert_eq!(PLATES, 7);
        assert_eq!(p.page(0), Some(&[Entry::Empty; PLATES]));
    }

    /// The guest's own width must stay five whatever the plate count is: it is the
    /// width of four byte arrays in CHAR, and the sentinel `skill_bind` tests.
    #[test]
    fn the_guest_bar_is_still_five_wide() {
        assert_eq!(GUEST_SLOTS, 5);
        assert_eq!(GUEST_SLOTS, SKILL_BIND_UNREGISTER as usize);
    }

    #[test]
    fn capture_stamps_the_class_and_the_item_record() {
        let (p, _) = seeded([SK0, Live::Item { tab: 0, slot: 0 }, Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(
            p.plate(0),
            Entry::Skill {
                cat: 0,
                cell: 3,
                class: 7,
                sref: Some(0x03)
            }
        );
        assert_eq!(
            p.plate(1),
            Entry::Item {
                tab: 0,
                slot: 0,
                cat: 0x12,
                idx: 0x01
            }
        );
    }

    /// An identity we cannot stamp is not stored half-way — it would be unvalidatable
    /// forever, i.e. it would restore blind.
    #[test]
    fn an_unstampable_slot_is_captured_as_empty() {
        let (p, _) = seeded([
            Live::Skill { cat: 2, cell: 1, sref: 0x81 }, // classes(2) == None
            Live::Item { tab: 3, slot: 9 },              // bag(3,9) == None
            Live::Empty,
            Live::Empty,
            Live::Empty,
        ]);
        assert_eq!(p.plate(0), Entry::Empty);
        assert_eq!(p.plate(1), Entry::Empty);
    }

    // ── the window ───────────────────────────────────────────────────────────

    /// The continuity rule: after a load, plate `i` is guest slot `i`, so an install
    /// that predates plates sees its five bindings exactly where they were.
    #[test]
    fn adopting_the_guest_bar_seats_plate_i_in_guest_slot_i() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        for g in 0..GUEST_SLOTS as u8 {
            assert_eq!(w.plate_at(g), Some(g));
            assert_eq!(w.guest_of(g), Some(g));
        }
        // ...and the two new plates are not resident anywhere.
        assert_eq!(w.guest_of(5), None);
        assert_eq!(w.guest_of(6), None);
        // The first capture then hands the seats of the EMPTY guest slots back, which
        // is what leaves room for plates 5 and 6 without evicting anybody.
        p.capture(&mut w, &live5([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]), &classes, &bag);
        assert_eq!(w.residents(), vec![(0, 0), (1, 1)]);
    }

    /// A capture that reads an empty guest slot frees it, which is what gives plate 5
    /// somewhere to go without evicting anything the player can see.
    #[test]
    fn an_empty_guest_slot_is_released_and_reused_before_any_eviction() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(w.plate_at(2), None, "an empty live slot must not stay resident");
        // Plate 5 holds a skill (put there by hand, as registration would).
        p.pages[0][5] = Entry::Skill {
            cat: 0,
            cell: 5,
            class: 7,
            sref: Some(0x05),
        };
        let act = p.activate(&mut w, 5, &classes, &bag);
        let Activation::Press { guest, actions, .. } = act else {
            panic!("plate 5 must be pressable: {act:?}")
        };
        assert_eq!(guest, 2, "the free slot, not an eviction");
        assert!(actions.contains(&Action::Clear { guest: 2 }));
        assert!(actions.contains(&Action::BindSkill {
            cat: 0,
            cell: 5,
            guest: 2,
            sref: Some(0x05)
        }));
        // Plates 0 and 1 are untouched.
        assert_eq!(w.guest_of(0), Some(0));
        assert_eq!(w.guest_of(1), Some(1));
    }

    /// The whole point of the window: a plate already live costs NO guest write.
    #[test]
    fn pressing_a_resident_plate_writes_nothing() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        let act = p.activate(&mut w, 0, &classes, &bag);
        assert_eq!(
            act,
            Activation::Press {
                guest: 0,
                actions: Vec::new(),
                dropped: Vec::new()
            }
        );
    }

    /// With every guest slot busy, the LEAST RECENTLY USED one is the one that yields.
    #[test]
    fn a_full_window_evicts_the_least_recently_used_plate() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        p.capture(&mut w, &live5([SK0, SK0, SK0, SK0, SK0]), &classes, &bag);
        // Five plates resident. Use 1..4 so plate 0 is the oldest.
        for plate in 1..5u8 {
            assert!(matches!(p.activate(&mut w, plate, &classes, &bag), Activation::Press { .. }));
        }
        p.pages[0][6] = Entry::Item {
            tab: 0,
            slot: 1,
            cat: 0x12,
            idx: 0x00,
        };
        let act = p.activate(&mut w, 6, &classes, &bag);
        let Activation::Press { guest, .. } = act else { panic!("{act:?}") };
        assert_eq!(guest, 0, "plate 0's seat is the oldest");
        assert_eq!(w.guest_of(6), Some(0));
        assert_eq!(w.guest_of(0), None, "plate 0 is evicted...");
        assert!(!p.plate(0).is_empty(), "...but keeps its binding, so it still draws");
    }

    /// The dedup `skill_bind` does for us on a build where it is callable, done by
    /// hand: the same skill may never sit in two guest slots at once.
    #[test]
    fn binding_a_skill_that_is_already_resident_elsewhere_clears_the_other_seat() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        p.capture(&mut w, &live5([SK0, SK1, SK1, SK1, SK1]), &classes, &bag);
        // Plate 5 holds the SAME skill as plate 0, and plate 0 is resident in guest 0.
        p.pages[0][5] = p.plate(0);
        // Use 1..4 so plate 0 is the LRU and would be evicted anyway — force the
        // interesting case by using plate 0 last instead.
        for plate in [1u8, 2, 3, 4, 0] {
            assert!(matches!(p.activate(&mut w, plate, &classes, &bag), Activation::Press { .. }));
        }
        let act = p.activate(&mut w, 5, &classes, &bag);
        let Activation::Press { guest, actions, .. } = act else { panic!("{act:?}") };
        assert_ne!(guest, 0, "plate 0 was just used, so it is not the LRU");
        assert!(
            actions.contains(&Action::Clear { guest: 0 }),
            "the duplicate skill's seat must be cleared: {actions:?}"
        );
        assert_eq!(w.guest_of(0), None);
        assert!(!p.plate(0).is_empty(), "plate 0 keeps its entry");
    }

    /// An empty plate must never press a key: the guest slot it would land in belongs
    /// to somebody else, and its hotkey would fire THAT.
    #[test]
    fn an_empty_plate_presses_nothing() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(p.activate(&mut w, 6, &classes, &bag), Activation::Empty { stale: false });
        assert_eq!(p.activate(&mut w, PLATES as u8, &classes, &bag), Activation::Empty { stale: false });
    }

    /// A stale stamp must not cost a LIVE plate its seat on the way to failing.
    #[test]
    fn a_stale_plate_is_emptied_without_evicting_anything() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        p.capture(&mut w, &live5([SK0, SK0, SK0, SK0, SK0]), &classes, &bag);
        p.pages[0][5] = Entry::Item {
            tab: 0,
            slot: 0,
            cat: 0x12,
            idx: 0x01,
        };
        // The bag shifted: (0,0) now holds something else.
        let shifted = |tab: u8, slot: u8| -> Option<(u8, u8)> {
            if (tab, slot) == (0, 0) { Some((0x12, 0x00)) } else { bag(tab, slot) }
        };
        assert_eq!(
            p.activate(&mut w, 5, &classes, &shifted),
            Activation::Empty { stale: true },
            "the caller has to know the store changed"
        );
        assert_eq!(p.plate(5), Entry::Empty, "the ghost is cleared");
        for g in 0..GUEST_SLOTS as u8 {
            assert_eq!(w.plate_at(g), Some(g), "no seat may have been taken");
        }
    }

    // ── registration ─────────────────────────────────────────────────────────

    /// The 퀵슬롯등록 flow: reserve a guest slot, let the GAME write it, and do not
    /// let the capture in between conclude the plate is empty.
    #[test]
    fn a_reserved_slot_survives_the_captures_before_the_game_writes_it() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        let guest = p.reserve(&mut w, 5).expect("plate 5 is in range");
        assert_eq!(guest, 2, "the first free guest slot");
        // Several polls go by with the slot still empty — the game has not run yet.
        for _ in 0..3 {
            p.capture(&mut w, &live5([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]), &classes, &bag);
            assert_eq!(w.plate_at(2), Some(5), "the reservation must hold");
        }
        // Now the game's bind lands.
        p.capture(
            &mut w,
            &live5([SK0, SK1, Live::Item { tab: 0, slot: 0 }, Live::Empty, Live::Empty]),
            &classes,
            &bag,
        );
        assert_eq!(
            p.plate(5),
            Entry::Item {
                tab: 0,
                slot: 0,
                cat: 0x12,
                idx: 0x01
            }
        );
    }

    /// ⛔⛔ ...and the SEAT WAS FULL, which is the case both tests above miss: they seed
    /// an empty guest slot, so the capture in between reads EMPTY and `PENDING_POLLS`
    /// covers it. With all five seats taken, `reserve` evicts a plate and deliberately
    /// writes nothing (the game is about to), so the slot keeps the OUTGOING binding —
    /// and a capture that ran before the game's write used to take that non-empty read
    /// as proof and copy plate 0's binding onto plate 5. Normally the next capture undid
    /// it; when the bind never lands (a refused commit, a popup that closed) the
    /// duplicate was permanent, and a pad button addressing guest slots by digit would
    /// fire it.
    #[test]
    fn a_reservation_that_evicts_does_not_copy_the_outgoing_binding_onto_the_new_plate() {
        let full = [
            SK0,
            SK1,
            Live::Item { tab: 0, slot: 0 },
            Live::Item { tab: 0, slot: 1 },
            Live::Empty,
        ];
        let (mut p, mut w) = seeded(full);
        // Plate 4 is empty, so seat 4 is free: fill it too, then every seat is taken.
        p.pages[0][4] = Entry::Skill {
            cat: 1,
            cell: 2,
            class: 4,
            sref: Some(0x12),
        };
        w.seat_settled(4, 4);
        let guest = p.reserve(&mut w, 5).expect("plate 5 is in range");
        let evicted = 0u8;
        assert_eq!(guest, evicted, "the LRU seat, which plate 0 was holding");
        assert_eq!(w.plate_at(guest), Some(5));
        let outgoing = p.plate(0);
        assert!(!outgoing.is_empty(), "the test is only meaningful if there IS an outgoing binding");

        // The polls before the game's own write still read the OUTGOING binding.
        for _ in 0..3 {
            p.capture(&mut w, &live5(full), &classes, &bag);
            assert_eq!(p.plate(5), Entry::Empty, "plate 5 must not inherit plate 0's binding");
            assert_eq!(w.plate_at(guest), Some(5), "and the reservation still holds");
        }
        // Then it lands.
        let mut bound = full;
        bound[usize::from(guest)] = Live::Item { tab: 0, slot: 1 };
        p.capture(&mut w, &live5(bound), &classes, &bag);
        assert_eq!(
            p.plate(5),
            Entry::Item {
                tab: 0,
                slot: 1,
                cat: 0x12,
                idx: 0x00
            }
        );
    }

    /// ...and the guard is a BUDGET, not a wait: re-registering the very thing that was
    /// already in that seat reads back as the outgoing binding forever, and a seat that
    /// waited forever would leave its plate empty forever.
    #[test]
    fn re_registering_the_outgoing_binding_is_accepted_once_the_budget_runs_out() {
        let full = [
            SK0,
            SK1,
            Live::Item { tab: 0, slot: 0 },
            Live::Item { tab: 0, slot: 1 },
            Live::Empty,
        ];
        let (mut p, mut w) = seeded(full);
        p.pages[0][4] = Entry::Skill {
            cat: 1,
            cell: 2,
            class: 4,
            sref: Some(0x12),
        };
        w.seat_settled(4, 4);
        let guest = p.reserve(&mut w, 5).expect("plate 5 is in range");
        for _ in 0..=PENDING_POLLS {
            p.capture(&mut w, &live5(full), &classes, &bag);
        }
        assert_eq!(w.plate_at(guest), Some(5), "the seat is the new plate's");
        assert_eq!(p.plate(5), p.plate(0), "and it took the binding it was reading");
    }

    /// ...but the budget is finite: a registration the game never completed must not
    /// hold a guest slot hostage forever.
    #[test]
    fn a_reservation_the_game_never_honours_expires() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        p.reserve(&mut w, 5);
        for _ in 0..=PENDING_POLLS {
            p.capture(&mut w, &live5([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]), &classes, &bag);
        }
        assert_eq!(w.plate_at(2), None, "the seat is back in the free pool");
    }

    /// ⛔ A bind that wrote NOTHING must not cost the plate its binding. Two real ways
    /// to get here: CHAR unresolvable when the write is attempted, and a stored skill
    /// with no `sref` on a build with no located `skill_bind` (`Action::BindSkill`) —
    /// in both cases the guest slot stays empty forever, and an earlier version of
    /// `capture` read that as "the plate is empty" and erased the player's binding.
    #[test]
    fn a_bind_that_never_lands_releases_the_seat_but_keeps_the_plate() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.pages[0][5] = Entry::Skill {
            cat: 0,
            cell: 4,
            class: 7,
            sref: None,
        };
        let act = p.activate(&mut w, 5, &classes, &bag);
        assert!(matches!(act, Activation::Press { .. }));
        assert_eq!(w.guest_of(5), Some(0));
        // The write did nothing: every capture from here on reads an empty bar.
        for _ in 0..=PENDING_POLLS {
            p.capture(&mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        }
        assert_eq!(w.guest_of(5), None, "the seat goes back to the pool");
        assert!(!p.plate(5).is_empty(), "...but the player's binding is still there");
    }

    /// ...whereas a seat that WAS confirmed and then reads empty is the game having
    /// unbound it (a stack used up, the party-transition sweep), and the plate follows.
    #[test]
    fn a_confirmed_seat_that_goes_empty_empties_its_plate() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        assert!(!p.plate(0).is_empty());
        p.capture(&mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        assert_eq!(p.plate(0), Entry::Empty);
        assert_eq!(w.guest_of(0), None);
    }

    // ── pages ────────────────────────────────────────────────────────────────

    /// ★ The user's requirement, stated as a test: page 1 survives whatever happens
    /// on page 2, including the game's own unregister-all.
    #[test]
    fn page_one_survives_a_wipe_that_happens_on_page_two() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        // Flip to page 1 — the guest bar is what page 0 held at the moment of the flip.
        let (acts, dropped) = p.flip_to(
            1,
            &mut w,
            &live5([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]),
            &classes,
            &bag,
        );
        assert!(dropped.is_empty());
        // Page 1 is empty, so the plan is five clears and nothing else.
        assert_eq!(acts.len(), GUEST_SLOTS);
        assert!(acts.iter().all(|a| matches!(a, Action::Clear { .. })));

        // Now the game wipes every binding (guest 0x154416) while page 1 is live, and
        // the player flips back with an EMPTY bar.
        let (acts, dropped) = p.flip_to(0, &mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        assert!(dropped.is_empty(), "the class did not change, so nothing may be dropped");
        assert!(
            acts.contains(&Action::BindSkill {
                cat: 0,
                cell: 3,
                guest: 0,
                sref: Some(0x03)
            }),
            "page 0's skill must come back: {acts:?}"
        );
    }

    /// A page with more entries than the guest bar can hold warms its first five and
    /// leaves the rest to the window — none of them are lost.
    #[test]
    fn warming_a_full_page_seats_five_and_keeps_the_other_two() {
        let mut p = Pages::new();
        let mut w = Window::new();
        for plate in 0..PLATES as u8 {
            p.pages[0][usize::from(plate)] = Entry::Skill {
                cat: 0,
                cell: plate % SKILL_CELLS,
                class: 7,
                sref: Some(plate),
            };
        }
        let (acts, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::Clear { .. })).count(), GUEST_SLOTS);
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::BindSkill { .. })).count(), GUEST_SLOTS);
        for plate in 0..GUEST_SLOTS as u8 {
            assert_eq!(w.guest_of(plate), Some(plate));
        }
        assert_eq!(w.guest_of(5), None);
        assert!(!p.plate(5).is_empty(), "plate 5 still has its binding");
        assert!(!p.plate(6).is_empty());
    }

    /// ...and tapping one of those two then works, which is the whole feature.
    #[test]
    fn the_sixth_and_seventh_plates_are_pressable_after_a_warm_up() {
        let mut p = Pages::new();
        let mut w = Window::new();
        for plate in 0..PLATES as u8 {
            p.pages[0][usize::from(plate)] = Entry::Skill {
                cat: 0,
                cell: plate % SKILL_CELLS,
                class: 7,
                sref: Some(plate),
            };
        }
        p.warm_plan(&mut w, &classes, &bag);
        for plate in [5u8, 6] {
            let act = p.activate(&mut w, plate, &classes, &bag);
            let Activation::Press { guest, actions, .. } = act else { panic!("plate {plate}: {act:?}") };
            assert!(usize::from(guest) < GUEST_SLOTS);
            assert!(actions.iter().any(|a| matches!(a, Action::BindSkill { .. })));
            assert_eq!(w.guest_of(plate), Some(guest));
        }
    }

    /// ★ ...but only when it is still the same skill. `cat` is a party slot, so a
    /// party change makes the stored coordinate name somebody else's skill.
    #[test]
    fn a_party_change_drops_the_entry_instead_of_binding_the_wrong_skill() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        // Party slot 0 now holds a different class.
        let reshuffled = |cat: u8| -> Option<u8> { if cat == 0 { Some(9) } else { classes(cat) } };
        let (acts, dropped) = p.warm_plan(&mut w, &reshuffled, &bag);
        assert!(
            !acts.iter().any(|a| matches!(a, Action::BindSkill { .. })),
            "must not bind: {acts:?}"
        );
        assert_eq!(
            dropped,
            vec![Dropped::ClassChanged {
                plate: 0,
                was: 7,
                now: Some(9)
            }]
        );
    }

    #[test]
    fn a_moved_item_drops_the_entry_instead_of_binding_the_wrong_item() {
        let (mut p, mut w) = seeded([Live::Item { tab: 0, slot: 0 }, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        // The bag shifted: cell (0,0) now holds what used to be at (0,1).
        let shifted = |tab: u8, slot: u8| -> Option<(u8, u8)> {
            if (tab, slot) == (0, 0) { Some((0x12, 0x00)) } else { bag(tab, slot) }
        };
        let (acts, dropped) = p.warm_plan(&mut w, &classes, &shifted);
        assert!(!acts.iter().any(|a| matches!(a, Action::BindItem { .. })));
        assert_eq!(
            dropped,
            vec![Dropped::ItemMoved {
                plate: 0,
                was: (0x12, 0x01),
                now: Some((0x12, 0x00))
            }]
        );
    }

    /// A warm-up always clears all five first: a raw bind's dedup is ours to do, so
    /// the result must not depend on what the previous page left behind.
    #[test]
    fn every_warm_up_clears_all_five_before_binding_anything() {
        let (mut p, mut w) = seeded([SK0, Live::Item { tab: 0, slot: 0 }, SK1, Live::Empty, Live::Empty]);
        let (acts, _) = p.warm_plan(&mut w, &classes, &bag);
        let clears: Vec<u8> = acts
            .iter()
            .take_while(|a| matches!(a, Action::Clear { .. }))
            .map(|a| a.guest())
            .collect();
        assert_eq!(clears, vec![0, 1, 2, 3, 4]);
        assert_eq!(acts.len(), GUEST_SLOTS + 3);
    }

    /// The guest slot handed to `skill_bind` must never be the unregister sentinel
    /// (or past it): the register path at guest 0x145caa writes `CHAR+0x252+slot` with
    /// no upper bound of its own — and past the array is `itemref[0]`.
    #[test]
    fn no_plan_can_ever_emit_the_unregister_sentinel_as_a_guest_slot() {
        let mut p = Pages::new();
        let mut w = Window::new();
        for plate in 0..PLATES as u8 {
            p.pages[0][usize::from(plate)] = Entry::Skill {
                cat: plate % PARTY_SLOTS,
                cell: plate % SKILL_CELLS,
                class: if plate % PARTY_SLOTS == 0 { 7 } else { 4 },
                sref: Some(plate),
            };
        }
        let (mut acts, _) = p.warm_plan(&mut w, &classes, &bag);
        for plate in 0..PLATES as u8 {
            if let Activation::Press { actions, .. } = p.activate(&mut w, plate, &classes, &bag) {
                acts.extend(actions);
            }
        }
        assert!(!acts.is_empty());
        for a in &acts {
            assert!(
                a.guest() < SKILL_BIND_UNREGISTER,
                "guest slot {} reaches the sentinel: {a:?}",
                a.guest()
            );
        }
    }

    #[test]
    fn next_page_wraps() {
        let mut p = Pages::new();
        let mut w = Window::new();
        assert_eq!(p.next_page(), 1);
        p.flip_to(1, &mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        assert_eq!(p.current(), 1);
        assert_eq!(p.next_page(), 0);
    }

    #[test]
    fn flipping_to_the_current_page_is_a_no_op() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_guest_bar(&mut w);
        let (acts, dropped) = p.flip_to(0, &mut w, &live5([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]), &classes, &bag);
        assert!(acts.is_empty() && dropped.is_empty());
        // ...and it must not have captured either, or a stray tap would rewrite a page.
        assert_eq!(p.plate(0), Entry::Empty);
    }

    #[test]
    fn reset_clears_every_page_and_returns_to_page_zero() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.flip_to(1, &mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        p.set_owner(Some(Owner { slot: 1, identity: 9 }));
        p.reset();
        assert_eq!(p.current(), 0);
        assert_eq!(p.owner(), None);
        for i in 0..p.count() {
            assert_eq!(p.page(i), Some(&[Entry::Empty; PLATES]));
        }
    }

    // ── persistence ──────────────────────────────────────────────────────────

    #[test]
    fn serialize_round_trips() {
        let (mut p, mut w) = seeded([SK0, Live::Item { tab: 0, slot: 1 }, Live::Empty, Live::Empty, Live::Empty]);
        p.pages[0][6] = Entry::Skill {
            cat: 1,
            cell: 2,
            class: 4,
            sref: Some(0x42),
        };
        p.set_owner(Some(Owner {
            slot: 2,
            identity: 0x1122_3344_5566_7788,
        }));
        p.flip_to(1, &mut w, &live5([Live::Empty; GUEST_SLOTS]), &classes, &bag);
        p.pages[1][5] = Entry::Item {
            tab: 1,
            slot: 3,
            cat: 0x12,
            idx: 0x05,
        };
        let back = Pages::parse(&p.serialize());
        assert_eq!(back.current(), p.current());
        assert_eq!(back.owner(), p.owner());
        for i in 0..p.count() {
            assert_eq!(back.page(i), p.page(i), "page {i}");
        }
    }

    /// A store written before plates existed must load unchanged — the five plates it
    /// knows about keep their positions and the two new ones come up empty.
    #[test]
    fn a_five_plate_store_file_still_loads() {
        let p = Pages::parse("cur 0\ns 0 0 0 3 7 3\ni 0 4 0 1 18 0\n");
        assert_eq!(
            p.plate(0),
            Entry::Skill {
                cat: 0,
                cell: 3,
                class: 7,
                sref: Some(3)
            }
        );
        assert_eq!(
            p.plate(4),
            Entry::Item {
                tab: 0,
                slot: 1,
                cat: 0x12,
                idx: 0x00
            }
        );
        assert_eq!(p.plate(5), Entry::Empty);
        assert_eq!(p.plate(6), Entry::Empty);
        // No `own` line = unknown owner, which the worker reads as "not mine" and
        // resets — i.e. exactly what an older build did unconditionally.
        assert_eq!(p.owner(), None);
    }

    /// A hand-edited store must not be able to smuggle an out-of-domain coordinate
    /// into a guest call. Both the parser and the planner refuse it.
    #[test]
    fn parse_refuses_out_of_domain_coordinates() {
        let p = Pages::parse("s 0 0 3 3 7\ns 0 1 0 9 7\ni 0 2 8 0 18 1\ni 0 3 0 32 18 1\n");
        assert_eq!(p.page(0), Some(&[Entry::Empty; PLATES]), "nothing may survive");
    }

    /// A plate index past the row is dropped, not wrapped: wrapping would put one
    /// player's binding on another plate on the next build that changes `PLATES`.
    #[test]
    fn parse_refuses_a_plate_index_past_the_row() {
        let p = Pages::parse(&format!("s 0 {} 0 3 7\n", PLATES));
        assert_eq!(p.page(0), Some(&[Entry::Empty; PLATES]));
    }

    #[test]
    fn parse_ignores_junk_without_losing_good_lines() {
        let p = Pages::parse("cur 1\nnonsense\ns 0 0 0 3 7\ns bad line here\n\ni 1 6 0 1 18 0\nown nope\n");
        assert_eq!(p.current(), 1);
        // Six fields = a pre-2026-08-11 line: it still loads, with no `sref`.
        assert_eq!(
            p.page(0).unwrap()[0],
            Entry::Skill {
                cat: 0,
                cell: 3,
                class: 7,
                sref: None
            }
        );
        assert_eq!(
            p.page(1).unwrap()[6],
            Entry::Item {
                tab: 0,
                slot: 1,
                cat: 0x12,
                idx: 0x00
            }
        );
        assert_eq!(p.owner(), None, "a malformed owner line must not stamp the store");
    }

    /// The `sref` column is append-only: a file without it loads (as `None`) and is
    /// written back without it, and a file with it round-trips. That is what lets an
    /// existing install keep its pages across this change.
    #[test]
    fn the_skill_ref_column_is_append_only() {
        let legacy = Pages::parse("s 0 2 1 8 4\n");
        assert_eq!(
            legacy.page(0).unwrap()[2],
            Entry::Skill {
                cat: 1,
                cell: 8,
                class: 4,
                sref: None
            }
        );
        assert_eq!(legacy.serialize(), "cur 0\ns 0 2 1 8 4\n");

        let modern = Pages::parse("s 0 2 1 8 4 72\n");
        assert_eq!(
            modern.page(0).unwrap()[2],
            Entry::Skill {
                cat: 1,
                cell: 8,
                class: 4,
                sref: Some(72)
            }
        );
        assert_eq!(modern.serialize(), "cur 0\ns 0 2 1 8 4 72\n");

        // A ref that is not a byte is a corrupt line, not a binding with a truncated ref:
        // the raw-write path would otherwise poke a wrong value into CHAR.
        assert_eq!(Pages::parse("s 0 2 1 8 4 256\n").page(0), Some(&[Entry::Empty; PLATES]));
    }

    #[test]
    fn the_owner_line_round_trips_and_is_optional() {
        let p = Pages::parse("cur 0\nown 3 18446744073709551615\n");
        assert_eq!(
            p.owner(),
            Some(Owner {
                slot: 3,
                identity: u64::MAX
            })
        );
        assert_eq!(p.serialize(), "cur 0\nown 3 18446744073709551615\n");
    }

    #[test]
    fn parse_refuses_a_current_page_that_does_not_exist() {
        let p = Pages::parse("cur 99\n");
        assert_eq!(p.current(), 0);
    }

    // -- the store holds more than one character (2026-09-04) --------------------
    //
    // *** THE REGRESSION TEST FOR THE REPORTED BUG. Player, on vc237: plates 6 and 7
    // registered on save slot 2 survive any number of 메인메뉴 -> 게임시작 round trips on
    // that slot, and are GONE after one visit to slot 1 or 3. Plates 0..4 come back
    // because the guest's save blob carries them; 5..6 exist only here, so a store that
    // can hold one character loses them the first time another one is loaded.

    fn own_key(slot: u8, identity: u64) -> Owner {
        Owner { slot, identity }
    }

    /// A plate-5 skill, i.e. one of the two that has nowhere else to live.
    fn plate5() -> Entry {
        Entry::Skill {
            cat: 1,
            cell: 8,
            class: 4,
            sref: Some(72),
        }
    }

    #[test]
    fn visiting_another_save_slot_does_not_destroy_this_row() {
        let mut p = Pages::new();
        assert!(!p.adopt_owner(Some(own_key(2, 7))), "first sight of slot 2 is not known");
        p.pages[0][5] = plate5();
        p.pages[0][6] = plate5();

        // Slot 1. A different character: its own row starts empty...
        assert!(!p.adopt_owner(Some(own_key(1, 7))));
        assert_eq!(p.page(0).unwrap()[5], Entry::Empty, "slot 1 must not inherit slot 2's plate");
        assert_eq!(p.parked(), 1, "...and slot 2's row was PARKED, not dropped");

        // ...and coming back hands it straight back.
        assert!(p.adopt_owner(Some(own_key(2, 7))), "slot 2 is known now");
        assert_eq!(p.page(0).unwrap()[5], plate5(), "plate 6 survived the trip");
        assert_eq!(p.page(0).unwrap()[6], plate5(), "plate 7 survived the trip");
        assert_eq!(p.owner(), Some(own_key(2, 7)));
    }

    #[test]
    fn deleting_the_slot_forgets_the_row_so_a_same_class_successor_starts_empty() {
        // The report: make 리츠 in slot 2, learn skills, 초기화 slot 2, make another 리츠
        // there. Same (slot, class) key — without the forget, the old row comes back.
        let mut p = Pages::new();
        assert!(!p.adopt_owner(Some(own_key(2, 7))));
        p.pages[0][5] = plate5();
        p.pages[0][0] = plate5();

        assert!(p.forget_slot(2), "the active row was slot 2's — it changed");
        assert_eq!(p.owner(), None, "nobody owns the row any more");
        assert_eq!(p.parked(), 0, "and it was NOT parked — parking would hand it back");

        // The successor has the identical key and must be a stranger to the store.
        assert!(!p.adopt_owner(Some(own_key(2, 7))), "the successor is a new character");
        assert_eq!(p.page(0).unwrap()[0], Entry::Empty);
        assert_eq!(p.page(0).unwrap()[5], Entry::Empty);
    }

    #[test]
    fn deleting_a_slot_drops_its_parked_row_and_spares_the_others() {
        // Slot 2 is parked (the player loaded slot 1 before deleting slot 2 from the
        // title): the parked row must go, the active slot-1 row must not.
        let mut p = Pages::new();
        p.adopt_owner(Some(own_key(2, 7)));
        p.pages[0][5] = plate5();
        p.adopt_owner(Some(own_key(1, 7)));
        p.pages[0][6] = plate5();
        assert_eq!(p.parked(), 1);

        assert!(p.forget_slot(2));
        assert_eq!(p.parked(), 0, "slot 2's parked row is gone");
        assert_eq!(p.owner(), Some(own_key(1, 7)), "slot 1 stays active");
        assert_eq!(p.page(0).unwrap()[6], plate5(), "slot 1's plate 7 untouched");

        // Coming back to slot 2 with the same class finds nothing.
        assert!(!p.adopt_owner(Some(own_key(2, 7))));
        assert_eq!(p.page(0).unwrap()[5], Entry::Empty);
        // A slot nobody has a row for is a no-op, and says so.
        assert!(!p.forget_slot(0));
    }

    #[test]
    fn forgetting_a_slot_survives_the_file_round_trip() {
        let mut p = Pages::new();
        p.adopt_owner(Some(own_key(2, 7)));
        p.pages[0][5] = plate5();
        p.adopt_owner(Some(own_key(1, 9)));
        p.forget_slot(2);
        let q = Pages::parse(&p.serialize());
        assert_eq!(q.parked(), 0);
        assert_eq!(q.owner(), Some(own_key(1, 9)));
        assert!(!Pages::parse(&q.serialize()).adopt_owner(Some(own_key(2, 7))));
    }

    #[test]
    fn the_same_owner_twice_is_a_no_op_and_stays_known() {
        // The 창고/캐시샵 edge: state 3 is an online screen too (66차-C), so this runs
        // without a character load. It must not park, clear or renumber anything.
        let mut p = Pages::new();
        p.adopt_owner(Some(own_key(2, 7)));
        p.pages[0][5] = plate5();
        p.flip_to_for_test(1);
        assert!(p.adopt_owner(Some(own_key(2, 7))));
        assert_eq!(p.page(0).unwrap()[5], plate5());
        assert_eq!(p.current(), 1, "the open page is not reset by a re-adopt");
        assert_eq!(p.parked(), 0);
    }

    #[test]
    fn an_unreadable_identity_parks_rather_than_claiming_a_row() {
        // `identity` is 0 until the class name lands (71차-L). The caller passes None,
        // and the row we cannot name must neither claim the outgoing key nor destroy it.
        let mut p = Pages::new();
        p.adopt_owner(Some(own_key(2, 7)));
        p.pages[0][5] = plate5();
        assert!(!p.adopt_owner(None));
        assert_eq!(p.owner(), None, "an unverifiable key claims nothing");
        assert_eq!(p.parked(), 1);
        assert!(p.adopt_owner(Some(own_key(2, 7))), "the row is still there for a later scan");
        assert_eq!(p.page(0).unwrap()[5], plate5());
    }

    #[test]
    fn parked_rows_round_trip_through_the_file() {
        let mut p = Pages::new();
        p.adopt_owner(Some(own_key(2, 0x1122_3344_5566_7788)));
        p.pages[0][5] = plate5();
        p.adopt_owner(Some(own_key(1, 9)));
        p.pages[1][6] = Entry::Item {
            tab: 1,
            slot: 3,
            cat: 0x12,
            idx: 0x05,
        };
        p.flip_to_for_test(1);

        let mut back = Pages::parse(&p.serialize());
        assert_eq!(back.owner(), p.owner());
        assert_eq!(back.current(), 1);
        assert_eq!(back.parked(), 1);
        for i in 0..p.count() {
            assert_eq!(back.page(i), p.page(i), "active page {i}");
        }
        // And the parked row is reachable, contents intact.
        assert!(back.adopt_owner(Some(own_key(2, 0x1122_3344_5566_7788))));
        assert_eq!(back.page(0).unwrap()[5], plate5());
    }

    #[test]
    fn a_store_written_before_sections_still_loads_as_the_active_row() {
        // Exactly what an older build wrote: cur, own, entries, no `arc` anywhere.
        let p = Pages::parse("cur 0\nown 2 7\ns 0 5 1 8 4 72\n");
        assert_eq!(p.owner(), Some(own_key(2, 7)));
        assert_eq!(p.page(0).unwrap()[5], plate5());
        assert_eq!(p.parked(), 0);
    }

    #[test]
    fn a_stray_own_inside_a_parked_section_does_not_re_key_the_active_row() {
        // `own` is optional, which is why parked rows get their own keyword. A file
        // whose active row has no `own` must not adopt the first parked row's key - nor
        // swallow its entries.
        let p = Pages::parse("cur 0\narc 2 7\ns 0 5 1 8 4 72\nown 4 4\n");
        assert_eq!(p.owner(), None, "the active row is still unowned");
        assert_eq!(p.page(0).unwrap()[5], Entry::Empty, "the entry belongs to the parked row");
        assert_eq!(p.parked(), 1);
    }

    #[test]
    fn the_archive_is_bounded_and_never_holds_one_character_twice() {
        let mut p = Pages::new();
        for i in 0..(ARCHIVE_MAX as u64 + 4) {
            p.adopt_owner(Some(own_key(i as u8, i)));
        }
        assert_eq!(p.parked(), ARCHIVE_MAX, "bounded");

        // The same key twice keeps ONE row, and it is the newer one.
        let mut q = Pages::new();
        q.adopt_owner(Some(own_key(2, 7)));
        q.pages[0][5] = plate5();
        q.adopt_owner(Some(own_key(1, 9)));
        q.adopt_owner(Some(own_key(2, 7)));
        q.adopt_owner(Some(own_key(1, 9)));
        assert_eq!(q.parked(), 1, "two keys, one parked row");
        assert!(q.adopt_owner(Some(own_key(2, 7))));
        assert_eq!(q.page(0).unwrap()[5], plate5());

        // A duplicate key on the way IN from a file is dropped, not stored twice.
        let dup = Pages::parse("cur 0\nown 2 7\narc 1 9\narc 1 9\narc 2 7\n");
        assert_eq!(dup.parked(), 1, "the active key and the repeat are both refused");
    }

    // ── the HIDDEN set (player request 2026-09-10) ───────────────────────────
    //
    // "스킬 퀵슬롯을 선택 후 비활성화 … 퀵슬롯이 너무많아서 불편하다는 유저도있어서".
    // The rules live in this file because the drawing side, the touch side, the pad
    // side and the worker all have to agree on them, and this is the only one of those
    // that can be tested on the Windows host at all.

    /// Every plate is on until the player says otherwise — a fresh install and a
    /// missing file both mean this.
    #[test]
    fn nothing_is_hidden_by_default() {
        assert_eq!(SHOW_ALL, 0);
        assert_eq!(visible_count(SHOW_ALL), PLATES);
        assert_eq!(visible_plates(SHOW_ALL), (0..PLATES as u8).collect::<Vec<_>>());
        assert!(hidden_plates(SHOW_ALL).is_empty());
        for plate in 0..PLATES as u8 {
            assert!(!plate_hidden(SHOW_ALL, plate));
            assert_eq!(visible_index(SHOW_ALL, plate), Some(usize::from(plate)));
        }
        assert_eq!(Pages::new().hidden(), SHOW_ALL);
    }

    /// The RING closes the gap: a hidden plate gives its seat back and every plate
    /// after it moves up one. This is the whole reason the survivors grow — the
    /// geometry lays out `1 + visible_count` seats over the same fixed arc.
    #[test]
    fn hiding_a_plate_gives_its_ring_seat_back() {
        let mask = hide_plate(SHOW_ALL, 2).unwrap();
        assert_eq!(visible_count(mask), PLATES - 1);
        assert_eq!(visible_plates(mask), vec![0, 1, 3, 4, 5, 6]);
        assert_eq!(hidden_plates(mask), vec![2]);
        // Before the hole: unchanged. After it: one seat earlier. In it: no seat.
        assert_eq!(visible_index(mask, 0), Some(0));
        assert_eq!(visible_index(mask, 1), Some(1));
        assert_eq!(visible_index(mask, 2), None);
        assert_eq!(visible_index(mask, 3), Some(2));
        assert_eq!(visible_index(mask, 6), Some(5));
        // Two holes, and the seats stay dense.
        let two = hide_plate(mask, 0).unwrap();
        assert_eq!(visible_plates(two), vec![1, 3, 4, 5, 6]);
        assert_eq!(visible_index(two, 1), Some(0));
        assert_eq!(visible_index(two, 3), Some(1));
    }

    /// ⛔ The last visible plate may not be hidden. See `hide_plate`: with nothing on
    /// the ring the game's own 퀵슬롯등록 popup has no target, and the page / save
    /// buttons are sized from a plate that would not exist.
    #[test]
    fn the_last_visible_plate_cannot_be_hidden() {
        let mut mask = SHOW_ALL;
        for plate in 0..PLATES as u8 - 1 {
            mask = hide_plate(mask, plate).expect("all but the last may go");
        }
        assert_eq!(visible_count(mask), 1);
        assert_eq!(visible_plates(mask), vec![PLATES as u8 - 1]);
        assert_eq!(hide_plate(mask, PLATES as u8 - 1), None, "the last one is refused");
        // …and the refusal does not quietly change the mask on the way out.
        assert_eq!(visible_count(mask), 1);
    }

    #[test]
    fn hiding_twice_or_hiding_a_plate_that_does_not_exist_is_refused() {
        let mask = hide_plate(SHOW_ALL, 3).unwrap();
        assert_eq!(hide_plate(mask, 3), None, "already off");
        assert_eq!(hide_plate(SHOW_ALL, PLATES as u8), None, "out of range");
        assert_eq!(hide_plate(SHOW_ALL, 200), None);
        // An out-of-range plate reads as VISIBLE, so no drawing path can be disabled
        // by a plate number it does not understand.
        assert!(!plate_hidden(SHOW_ALL, PLATES as u8));
        assert!(!plate_hidden(0xff, PLATES as u8));
        assert_eq!(visible_index(mask, PLATES as u8), None);
    }

    /// SHOW is all-or-nothing, and it has to put the ring back EXACTLY as it was —
    /// including the plate order, since the seats are handed out in plate order.
    #[test]
    fn showing_puts_every_plate_back() {
        let mask = hide_plate(hide_plate(SHOW_ALL, 1).unwrap(), 5).unwrap();
        assert_eq!(hidden_plates(mask), vec![1, 5]);
        assert_eq!(visible_count(mask), PLATES - 2);
        assert_eq!(visible_count(SHOW_ALL), PLATES);
        assert_eq!(visible_plates(SHOW_ALL), (0..PLATES as u8).collect::<Vec<_>>());
        for plate in 0..PLATES as u8 {
            assert_eq!(visible_index(SHOW_ALL, plate), Some(usize::from(plate)));
        }
    }

    /// A mask from another build, or a half-written file, must never produce a field
    /// with no controls on it — `ring_slots` has its own note about that state.
    #[test]
    fn a_mask_that_would_empty_the_ring_degrades_to_all_visible() {
        assert_eq!(sanitise_hidden(0x7f), SHOW_ALL, "all seven off");
        assert_eq!(sanitise_hidden(0xff), SHOW_ALL, "…including the bits past the row");
        // Bits past PLATES are dropped, and what is left is honoured.
        assert_eq!(sanitise_hidden(0x80), SHOW_ALL);
        // Bit 6 IS plate 7, so only bit 7 is out of the row.
        assert_eq!(sanitise_hidden(0x80 | 0x01), 0x01);
        assert_eq!(sanitise_hidden(0xc0 | 0x01), 0x41);
        assert_eq!(visible_count(sanitise_hidden(0xff)), PLATES);
        // Sanitising is idempotent for every mask a `u8` can hold, and never leaves the
        // screen empty.
        for m in 0..=u8::MAX {
            let once = sanitise_hidden(m);
            assert_eq!(sanitise_hidden(once), once, "{m:#04x}");
            assert!(visible_count(once) >= 1, "{m:#04x} left nothing on screen");
        }
    }

    #[test]
    fn the_hidden_set_round_trips_through_its_file() {
        for m in 0..=u8::MAX {
            let m = sanitise_hidden(m);
            assert_eq!(parse_hidden(&serialize_hidden(m)), m, "{m:#04x}");
        }
        // The numbers on disk are the ones printed on the plates: 1-based.
        assert_eq!(serialize_hidden(hide_plate(SHOW_ALL, 0).unwrap()), "hide 1\n");
        assert_eq!(serialize_hidden(0b110_0000), "hide 6 7\n");
        assert_eq!(serialize_hidden(SHOW_ALL), "hide\n");
    }

    /// A damaged file hides NOTHING. It must never hide everything, and it must never
    /// fail the read — the app has to start.
    #[test]
    fn a_damaged_hidden_file_hides_nothing() {
        for text in ["", "\n\n", "garbage", "show 1 2", "hide", "hide x y", "hide 0", "hide 8 9 99", "hide -1"] {
            assert_eq!(parse_hidden(text), SHOW_ALL, "{text:?}");
        }
        // Good tokens survive next to bad ones, and repeats are harmless.
        assert_eq!(parse_hidden("hide 2 nonsense 2 0 8 4\n"), 0b000_1010);
        // Later lines add to earlier ones; unknown lines are skipped.
        assert_eq!(parse_hidden("# a comment\nhide 1\nhide 7\n"), 0b100_0001);
    }

    /// A hidden plate does not fire — and, just as important, the refusal leaves the
    /// store alone: the binding is what the player put there, and unhiding brings it
    /// back. `Empty { stale: true }` here would EMPTY the plate.
    #[test]
    fn a_hidden_plate_never_fires_and_keeps_its_binding() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        let before = p.plate(1);
        assert!(!before.is_empty());
        p.set_hidden(hide_plate(SHOW_ALL, 1).unwrap());
        assert_eq!(p.activate(&mut w, 1, &classes, &bag), Activation::Empty { stale: false });
        assert_eq!(p.plate(1), before, "the binding is untouched");
        // The visible neighbour still works.
        assert!(matches!(p.activate(&mut w, 0, &classes, &bag), Activation::Press { .. }));
        // Unhide and it fires again — nothing had to be re-registered.
        p.set_hidden(SHOW_ALL);
        assert!(matches!(p.activate(&mut w, 1, &classes, &bag), Activation::Press { .. }));
    }

    /// ⛔ The game may not register into a plate the player cannot see, and the refusal
    /// happens BEFORE any seat changes hands: `reserve` evicts on the way in, so a
    /// refusal after the fact would cost a live plate its seat for nothing.
    #[test]
    fn a_hidden_plate_cannot_be_registered_into() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        p.set_hidden(hide_plate(SHOW_ALL, 6).unwrap());
        let residents_before = w.residents();
        assert_eq!(p.reserve(&mut w, 6), None);
        assert_eq!(w.residents(), residents_before, "no seat moved");
        assert_eq!(p.reserve(&mut w, PLATES as u8), None, "still out of range");
        // A visible plate is still reservable.
        assert!(p.reserve(&mut w, 5).is_some());
    }

    /// Warming skips hidden plates, which is how they stop consuming one of the five
    /// guest slots — hide two of seven and the five that are left all fit at once.
    #[test]
    fn warming_skips_hidden_plates_so_the_visible_ones_all_fit() {
        let mut p = Pages::new();
        let mut w = Window::new();
        for plate in 0..PLATES as u8 {
            p.pages[0][usize::from(plate)] = Entry::Skill {
                cat: 0,
                cell: plate % SKILL_CELLS,
                class: 7,
                sref: Some(plate),
            };
        }
        // Hide the first two: the survivors are 2..=6, which is exactly five.
        p.set_hidden(hide_plate(hide_plate(SHOW_ALL, 0).unwrap(), 1).unwrap());
        let (acts, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        // ⛔ All five guest slots are still cleared unconditionally. Narrowing this to
        // "only the slots we refill" leaves an evicted binding live in the guest bar,
        // firable from the pull-out keypad by a plate that no longer owns it.
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::Clear { .. })).count(), GUEST_SLOTS);
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::BindSkill { .. })).count(), GUEST_SLOTS);
        for (guest, plate) in (0..GUEST_SLOTS as u8).zip(2..PLATES as u8) {
            assert_eq!(w.guest_of(plate), Some(guest), "plate {plate}");
        }
        // The hidden plates hold no seat…
        assert_eq!(w.guest_of(0), None);
        assert_eq!(w.guest_of(1), None);
        // …and have lost nothing.
        assert!(!p.plate(0).is_empty(), "a hidden plate keeps its binding");
        assert!(!p.plate(1).is_empty());
        // Switch them back on and the store still has everything: the first five plates
        // take the bar again, exactly as before the hide.
        p.set_hidden(SHOW_ALL);
        let (acts, _) = p.warm_plan(&mut w, &classes, &bag);
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::BindSkill { .. })).count(), GUEST_SLOTS);
        for plate in 0..GUEST_SLOTS as u8 {
            assert_eq!(w.guest_of(plate), Some(plate));
        }
    }

    /// The save's own five bindings go to the plates the player can SEE, and the
    /// leftovers still get a plate so no guest slot is left owned by nobody.
    #[test]
    fn adopting_the_guest_bar_gives_the_visible_plates_the_first_slots() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.set_hidden(hide_plate(hide_plate(SHOW_ALL, 0).unwrap(), 2).unwrap());
        p.adopt_guest_bar(&mut w);
        // Visible first, in plate order: 1, 3, 4, 5, 6.
        assert_eq!(w.plate_at(0), Some(1));
        assert_eq!(w.plate_at(1), Some(3));
        assert_eq!(w.plate_at(2), Some(4));
        assert_eq!(w.plate_at(3), Some(5));
        assert_eq!(w.plate_at(4), Some(6));
        // Every guest slot is owned by exactly one plate — nothing orphaned.
        let owned: Vec<u8> = (0..GUEST_SLOTS as u8).filter_map(|g| w.plate_at(g)).collect();
        assert_eq!(owned.len(), GUEST_SLOTS);
        // With more plates hidden than there is room to spare, the hidden ones take the
        // slots the visible ones do not need — still no orphan.
        let mut q = Pages::new();
        let mut v = Window::new();
        let mut mask = SHOW_ALL;
        for plate in [0u8, 1, 2, 3] {
            mask = hide_plate(mask, plate).unwrap();
        }
        q.set_hidden(mask);
        q.adopt_guest_bar(&mut v);
        assert_eq!(v.plate_at(0), Some(4), "the first visible plate");
        assert_eq!(v.plate_at(1), Some(5));
        assert_eq!(v.plate_at(2), Some(6));
        assert_eq!(v.plate_at(3), Some(0), "then the hidden ones, so the slot has an owner");
        assert_eq!(v.plate_at(4), Some(1));
        assert_eq!((0..GUEST_SLOTS as u8).filter_map(|g| v.plate_at(g)).count(), GUEST_SLOTS);
    }

    /// The mask is a DEVICE preference, not part of a character's row: parking a row
    /// and adopting another one must not carry it, and it has no business in the store
    /// file. ⛔ A per-row mask would follow the character — `adopt_owner` replaces
    /// `pages`/`current` wholesale — and hiding is about the screen.
    #[test]
    fn the_hidden_set_is_not_part_of_a_characters_row() {
        let mut p = Pages::new();
        let mask = hide_plate(SHOW_ALL, 4).unwrap();
        p.set_hidden(mask);
        p.pages[0][5] = plate5();
        p.adopt_owner(Some(own_key(1, 7)));
        assert_eq!(p.hidden(), mask, "adopting another character keeps the preference");
        p.adopt_owner(Some(own_key(2, 9)));
        assert_eq!(p.hidden(), mask);
        assert!(!p.serialize().contains("hide"), "the store file has no business with it");
        // …and a store read back from disk starts from the neutral value, which the
        // worker then overwrites from the real preference on every poll.
        assert_eq!(Pages::parse(&p.serialize()).hidden(), SHOW_ALL);
    }

    /// A mask set through `set_hidden` is sanitised, so no caller can put the store in
    /// the "nothing on screen" state even by writing the preference atomic directly.
    #[test]
    fn set_hidden_sanitises() {
        let mut p = Pages::new();
        p.set_hidden(0xff);
        assert_eq!(p.hidden(), SHOW_ALL);
        p.set_hidden(0b1000_0000);
        assert_eq!(p.hidden(), SHOW_ALL, "bits past the plate row are dropped");
        p.set_hidden(0b0100_0000);
        assert_eq!(p.hidden(), 0b0100_0000);
    }

    // ── boxed consumables (소비가방, 2026-09-14) ─────────────────────────────

    #[test]
    fn a_boxed_plate_is_never_overwritten_by_a_capture_and_only_loses_its_seat() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(p.bind_boxed(&mut w, 5, 0x12, 0x01), Some(Vec::new()));
        assert_eq!(p.plate(5), Entry::Boxed { cat: 0x12, idx: 0x01 });
        // The press path: materialised into (0,0), loaned a seat, pressed.
        let act = p.activate(&mut w, 5, &classes, &bag);
        assert_eq!(act, Activation::Boxed { cat: 0x12, idx: 0x01 });
        let Activation::Press { guest, actions, .. } = p.activate_boxed(&mut w, 5, 0, 0) else { panic!("press") };
        assert_eq!(actions, vec![Action::Clear { guest }, Action::BindItem { item_ref: 0, guest }]);
        assert_eq!(w.guest_of(5), Some(guest));
        // The guest slot reads the loaned item: the plate is untouched.
        let mut live = [Live::Empty; GUEST_SLOTS];
        live[0] = SK0;
        live[usize::from(guest)] = Live::Item { tab: 0, slot: 0 };
        assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(5), Entry::Boxed { cat: 0x12, idx: 0x01 });
        // The game used it up: the seat is released, the plate still stands.
        live[usize::from(guest)] = Live::Empty;
        assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(5), Entry::Boxed { cat: 0x12, idx: 0x01 });
        assert_eq!(w.guest_of(5), None);
    }

    #[test]
    fn a_boxed_plate_takes_no_guest_slot_when_the_page_is_warmed() {
        let (mut p, mut w) = seeded([SK0, SK1, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 2, 0x12, 0x00);
        let (actions, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        // Five clears, then the two skills into guest slots 0 and 1 — the boxed plate
        // is skipped, not emptied, and consumes no seat.
        assert_eq!(actions.len(), GUEST_SLOTS + 2);
        assert_eq!(p.plate(2), Entry::Boxed { cat: 0x12, idx: 0x00 });
        assert_eq!(w.guest_of(2), None);
        assert_eq!(w.guest_of(0), Some(0));
        assert_eq!(w.guest_of(1), Some(1));
    }

    #[test]
    fn migration_turns_item_plates_of_a_boxed_category_into_boxed_ones() {
        let (mut p, mut w) = seeded([SK0, Live::Item { tab: 0, slot: 0 }, Live::Item { tab: 0, slot: 1 }, Live::Empty, Live::Empty]);
        assert_eq!(p.plate(1), Entry::Item { tab: 0, slot: 0, cat: 0x12, idx: 0x01 });
        let (actions, changed) = p.migrate_to_boxed(&mut w, &|cat| cat == 0x12);
        assert!(changed);
        // Both seats are cleared (the cells are about to be swept), both plates keep
        // their identity as boxed ones, the skill is untouched.
        assert_eq!(actions, vec![Action::Clear { guest: 1 }, Action::Clear { guest: 2 }]);
        assert_eq!(p.plate(1), Entry::Boxed { cat: 0x12, idx: 0x01 });
        assert_eq!(p.plate(2), Entry::Boxed { cat: 0x12, idx: 0x00 });
        assert_eq!(w.guest_of(1), None);
        assert_eq!(p.plate(0), Entry::Skill { cat: 0, cell: 3, class: 7, sref: Some(0x03) });
        // Nothing to do the second time.
        assert_eq!(p.migrate_to_boxed(&mut w, &|cat| cat == 0x12), (Vec::new(), false));
    }

    #[test]
    fn an_exhausted_boxed_plate_is_emptied_and_a_stocked_one_is_kept() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 1, 0x12, 0x00);
        p.bind_boxed(&mut w, 2, 0x12, 0x01);
        p.bind_boxed(&mut w, 6, 0x0d, 0x02);
        // 0x12/0x01 is gone, the other two are still in the box (or the bag).
        let gone = |cat: u8, idx: u8| (cat, idx) == (0x12, 0x01);
        assert_eq!(p.drop_exhausted_boxed(&gone), 1);
        assert_eq!(p.plate(1), Entry::Boxed { cat: 0x12, idx: 0x00 });
        assert_eq!(p.plate(2), Entry::Empty);
        assert_eq!(p.plate(6), Entry::Boxed { cat: 0x0d, idx: 0x02 });
        // A skill plate is never a candidate, and the second pass finds nothing.
        assert_eq!(p.plate(0), Entry::Skill { cat: 0, cell: 3, class: 7, sref: Some(0x03) });
        assert_eq!(p.drop_exhausted_boxed(&gone), 0);
        // An emptied plate presses nothing — and does not toast either, that is the point.
        assert_eq!(p.activate(&mut w, 2, &classes, &bag), Activation::Empty { stale: false });
    }

    #[test]
    fn a_hidden_exhausted_boxed_plate_keeps_its_binding() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 5, 0x12, 0x00);
        p.set_hidden(0b0010_0000);
        assert_eq!(p.drop_exhausted_boxed(&|_, _| true), 0);
        assert_eq!(p.plate(5), Entry::Boxed { cat: 0x12, idx: 0x00 });
        // Shown again, the next pass empties it.
        p.set_hidden(0);
        assert_eq!(p.drop_exhausted_boxed(&|_, _| true), 1);
        assert_eq!(p.plate(5), Entry::Empty);
    }

    #[test]
    fn an_exhausted_boxed_plate_on_the_other_page_is_emptied_too() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.bind_boxed(&mut w, 3, 0x12, 0x00);
        p.flip_to_for_test(1);
        p.bind_boxed(&mut w, 4, 0x12, 0x00);
        assert_eq!(p.drop_exhausted_boxed(&|_, _| true), 2);
        assert_eq!(p.plate(4), Entry::Empty);
        p.flip_to_for_test(0);
        assert_eq!(p.plate(3), Entry::Empty);
    }

    #[test]
    fn boxed_plates_round_trip_through_the_store_file() {
        let mut p = Pages::new();
        let mut w = Window::new();
        p.bind_boxed(&mut w, 6, 0x0d, 0x02);
        let text = p.serialize();
        assert!(text.contains("b 0 6 13 2
"), "{text}");
        let q = Pages::parse(&text);
        assert_eq!(q.plate(6), Entry::Boxed { cat: 0x0d, idx: 0x02 });
        // Out-of-byte values are junk, not a binding.
        assert_eq!(Pages::parse("b 0 6 300 2
").plate(6), Entry::Empty);
    }

    /// 136차: a sort moves cells; the plates that named them follow, the rest do not.
    #[test]
    fn a_sort_remaps_item_plates_to_their_items_new_cells() {
        let (mut p, _w) = seeded([SK0, Live::Item { tab: 0, slot: 0 }, Live::Item { tab: 0, slot: 1 }, Live::Empty, Live::Empty]);
        // (0,0) -> (1,4); (0,1) stays; anything else is not an item cell.
        let moved = |t: u8, s: u8| match (t, s) {
            (0, 0) => Some((1u8, 4u8)),
            (0, 1) => Some((0, 1)),
            _ => None,
        };
        assert!(p.remap_item_cells(&moved));
        assert!(matches!(p.plate(1), Entry::Item { tab: 1, slot: 4, cat: 0x12, idx: 0x01 }));
        assert!(matches!(p.plate(2), Entry::Item { tab: 0, slot: 1, .. }));
        assert!(matches!(p.plate(0), Entry::Skill { .. }), "a skill plate is not a cell");
        assert!(!p.remap_item_cells(&moved), "nothing left to move the second time");
    }

    #[test]
    fn a_hidden_plate_refuses_a_boxed_binding_and_a_boxed_press() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.set_hidden(1 << 6);
        assert_eq!(p.bind_boxed(&mut w, 6, 0x12, 0x00), None);
        p.set_hidden(0);
        p.bind_boxed(&mut w, 6, 0x12, 0x00);
        p.set_hidden(1 << 6);
        assert_eq!(p.activate(&mut w, 6, &classes, &bag), Activation::Empty { stale: false });
        assert_eq!(p.activate_boxed(&mut w, 6, 0, 0), Activation::Empty { stale: false });
    }

    // ── 157차: a 퀵슬롯등록 onto a plate that holds a boxed consumable ──────────────
    //
    // Player: *"퀵슬롯에 소비아이템을 등록해두고 그 소비아이템이 등록된 퀵슬롯에 스킬을
    // 등록하려고하니까 스킬 등록이 안되고있어"*. The game DID bind the skill — into the
    // guest slot `reserve` handed out — and the capture read that seat as a boxed plate's
    // loan, so the plate stayed the consumable.

    const POTION: Entry = Entry::Boxed { cat: 0x12, idx: 0x01 };

    #[test]
    fn a_skill_registered_onto_a_boxed_plate_replaces_the_consumable() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 1, 0x12, 0x01);
        assert!(p.release_boxed_seat(&mut w, 1).is_empty(), "no loan in flight — nothing to clear");
        let guest = p.reserve(&mut w, 1).expect("plate 1 is in range");
        assert_eq!(guest, 1, "the first free guest slot");
        // The polls before the game's own write: the seat holds, the plate is unchanged.
        let mut live = [SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty];
        for _ in 0..3 {
            assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
            assert_eq!(w.plate_at(guest), Some(1), "the reservation must hold");
            assert_eq!(p.plate(1), POTION);
        }
        // The game's bind lands.
        live[usize::from(guest)] = SK1;
        assert!(p.capture(&mut w, &live5(live), &classes, &bag), "the store changed — the caller persists");
        assert_eq!(p.plate(1), Entry::Skill { cat: 1, cell: 8, class: 4, sref: Some(0x48) });
        // An ordinary resident skill plate from here on.
        assert_eq!(
            p.activate(&mut w, 1, &classes, &bag),
            Activation::Press {
                guest,
                actions: Vec::new(),
                dropped: Vec::new()
            }
        );
    }

    #[test]
    fn a_registration_that_never_lands_leaves_the_boxed_plate_as_it_was() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 1, 0x12, 0x01);
        let guest = p.reserve(&mut w, 1).expect("plate 1 is in range");
        let mut live = [SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty];
        for _ in 0..=PENDING_POLLS {
            assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
        }
        assert_eq!(w.plate_at(guest), None, "the seat is back in the free pool");
        assert_eq!(p.plate(1), POTION);
        // …and a loan on that plate is a loan again: the item ref is not filed.
        let Activation::Press { guest: loan, .. } = p.activate_boxed(&mut w, 1, 0, 0) else { panic!("press") };
        live[usize::from(loan)] = Live::Item { tab: 0, slot: 0 };
        assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(1), POTION);
    }

    /// The loan seat may outlive its press (the unit was not used). `reserve` would hand
    /// the same seat back, item ref and all, so the registration gives it up first.
    #[test]
    fn a_registration_onto_a_boxed_plate_first_gives_up_its_loan_seat() {
        let (mut p, mut w) = seeded([SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty]);
        p.bind_boxed(&mut w, 1, 0x12, 0x01);
        let Activation::Press { guest: loan, .. } = p.activate_boxed(&mut w, 1, 0, 0) else { panic!("press") };
        let mut live = [SK0, Live::Empty, Live::Empty, Live::Empty, Live::Empty];
        live[usize::from(loan)] = Live::Item { tab: 0, slot: 0 };
        p.capture(&mut w, &live5(live), &classes, &bag);
        assert_eq!(w.guest_of(1), Some(loan), "the loan is still seated");
        // A hidden plate is not registered onto, so nothing of it is released either.
        p.set_hidden(1 << 1);
        assert!(p.release_boxed_seat(&mut w, 1).is_empty());
        p.set_hidden(0);
        assert_eq!(p.release_boxed_seat(&mut w, 1), vec![Action::Clear { guest: loan }]);
        assert_eq!(w.guest_of(1), None);
        live[usize::from(loan)] = Live::Empty;
        let guest = p.reserve(&mut w, 1).expect("plate 1 is in range");
        assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(1), POTION, "an empty read is a wait, not the loan's release");
        assert_eq!(w.plate_at(guest), Some(1));
        live[usize::from(guest)] = SK1;
        assert!(p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(1), Entry::Skill { cat: 1, cell: 8, class: 4, sref: Some(0x48) });
        // A skill plate has no loan to give up.
        assert!(p.release_boxed_seat(&mut w, 1).is_empty());
        assert_eq!(w.guest_of(1), Some(guest));
    }

    /// All five seats taken: `reserve` evicts the LRU and its binding is still in the
    /// guest slot. The boxed plate must not take THAT one either.
    #[test]
    fn a_registration_onto_a_boxed_plate_that_evicts_ignores_the_outgoing_binding() {
        let mut live = [
            SK0,
            SK1,
            Live::Item { tab: 0, slot: 0 },
            Live::Item { tab: 0, slot: 1 },
            Live::Skill { cat: 1, cell: 2, sref: 0x12 },
        ];
        let (mut p, mut w) = seeded(live);
        p.bind_boxed(&mut w, 5, 0x12, 0x01);
        let guest = p.reserve(&mut w, 5).expect("plate 5 is in range");
        assert_eq!(guest, 0, "the LRU seat, which plate 0 was holding");
        let outgoing = p.plate(0);
        for _ in 0..3 {
            assert!(!p.capture(&mut w, &live5(live), &classes, &bag));
            assert_eq!(p.plate(5), POTION, "plate 5 must not inherit plate 0's binding");
            assert_eq!(w.plate_at(guest), Some(5));
        }
        live[usize::from(guest)] = Live::Skill { cat: 0, cell: 5, sref: 0x05 };
        assert!(p.capture(&mut w, &live5(live), &classes, &bag));
        assert_eq!(p.plate(5), Entry::Skill { cat: 0, cell: 5, class: 7, sref: Some(0x05) });
        assert_eq!(p.plate(0), outgoing, "the evicted plate keeps its entry");
    }

    // ── 148차: a character load the sync has not seen yet ─────────────────────────
    //
    // S25 vc420, 2026-10-01: SLOT 2 (class 7/4 here) with a skill on every plate; OK mashed
    // through 메인메뉴 → 게임시작 → SLOT 1 (class 9/10 here) inside two ticks, so no poll
    // saw the shell and no sync ran. The next capture filed SLOT 1's bar onto SLOT 2's
    // plates (00:25:26.710), and the next load of SLOT 2 dropped them (00:25:30.024).

    /// The OTHER character: different classes in both party slots.
    fn other_classes(cat: u8) -> Option<u8> {
        match cat {
            0 => Some(9),
            1 => Some(10),
            _ => None,
        }
    }

    fn skill(cat: u8, cell: u8) -> Entry {
        Entry::Skill {
            cat,
            cell,
            class: classes(cat).unwrap(),
            sref: Some(cell),
        }
    }

    fn live_skill(cat: u8, cell: u8) -> Live {
        Live::Skill { cat, cell, sref: cell }
    }

    /// SLOT 2 as it was at 00:25:24: seven skills, plates 0..4 warmed into guest 0..4 and
    /// confirmed by a capture of that bar.
    fn slot2_in_play() -> (Pages, Window, [Entry; PLATES]) {
        let row = [skill(0, 0), skill(1, 1), skill(0, 2), skill(1, 3), skill(0, 4), skill(1, 5), skill(0, 6)];
        let mut p = Pages::new();
        let mut w = Window::new();
        assert!(!p.adopt_owner(Some(own_key(1, 0x7))));
        p.pages[0] = row;
        let (_, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        let bar = live5([live_skill(0, 0), live_skill(1, 1), live_skill(0, 2), live_skill(1, 3), live_skill(0, 4)]);
        assert!(!p.capture(&mut w, &bar, &classes, &bag), "the warmed bar is what the row says");
        (p, w, row)
    }

    /// SLOT 1's bar, as world init deserialised it: three skills, two empty slots.
    fn slot1_bar() -> Vec<Live> {
        live5([live_skill(0, 5), live_skill(1, 6), live_skill(0, 7), Live::Empty, Live::Empty])
    }

    #[test]
    fn a_load_the_sync_has_not_seen_cannot_rewrite_the_row() {
        // ── vc420, reproduced: no desync, so the capture is an ordinary one ──
        let (mut old, mut ow, _) = slot2_in_play();
        assert!(old.capture(&mut ow, &slot1_bar(), &other_classes, &bag));
        assert_eq!(
            old.plate(0),
            Entry::Skill {
                cat: 0,
                cell: 5,
                class: 9,
                sref: Some(5)
            },
            "SLOT 1's skill on SLOT 2's plate"
        );
        assert_eq!(old.plate(3), Entry::Empty, "a confirmed seat read empty empties its plate");
        assert_eq!(old.plate(5), skill(1, 5), "plates 6·7 had no seat and survived");
        // …and the next load of SLOT 2 (same owner — the pin had followed the wrong slot):
        assert!(old.adopt_owner(Some(own_key(1, 0x7))));
        let (_, dropped) = old.warm_plan(&mut ow, &classes, &bag);
        assert_eq!(dropped.len(), 3, "00:25:30.024: three ClassChanged");
        assert!((0..5).all(|i| old.plate(i).is_empty()), "plates 1..5 gone — the report");

        // ── 148차: the load desyncs, and nothing writes the row until the sync ──
        let (mut p, mut w, row) = slot2_in_play();
        p.desync(&mut w);
        assert!(!p.synced());
        assert!(w.residents().is_empty(), "the window describes a bar that no longer exists");
        assert!(!p.capture(&mut w, &slot1_bar(), &other_classes, &bag));
        assert_eq!(p.plates(), &row, "SLOT 2's row untouched");
        // The sync for SLOT 1: a character this store has not seen.
        assert!(!p.adopt_owner(Some(own_key(0, 0x9))));
        p.adopt_guest_bar(&mut w);
        assert!(p.synced());
        p.capture(&mut w, &slot1_bar(), &other_classes, &bag);
        // Back to SLOT 2: the row comes back whole and warms with nothing dropped.
        p.desync(&mut w);
        assert!(p.adopt_owner(Some(own_key(1, 0x7))));
        let (_, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        assert_eq!(p.plates(), &row);
        assert!(p.synced());
    }

    #[test]
    fn every_live_state_writer_is_refused_until_the_sync() {
        let (mut p, mut w, row) = slot2_in_play();
        p.pages[1][6] = Entry::Boxed { cat: 0x12, idx: 0x00 };
        p.pages[1][5] = Entry::Item {
            tab: 0,
            slot: 1,
            cat: 0x12,
            idx: 0x00,
        };
        let before = p.clone();
        p.desync(&mut w);

        assert_eq!(p.activate(&mut w, 0, &classes, &bag), Activation::Unsynced);
        assert_eq!(p.activate(&mut w, 6, &classes, &bag), Activation::Unsynced);
        assert_eq!(p.activate_boxed(&mut w, 0, 0, 0), Activation::Unsynced);
        assert_eq!(p.bind_boxed(&mut w, 2, 0x12, 0x01), None);
        assert_eq!(p.reserve(&mut w, 3), None);
        assert_eq!(p.flip_to(1, &mut w, &slot1_bar(), &other_classes, &bag), (Vec::new(), Vec::new()));
        assert_eq!(p.current(), 0, "no flip");
        assert_eq!(p.migrate_to_boxed(&mut w, &|_| true), (Vec::new(), false));
        assert_eq!(p.drop_exhausted_boxed(&|_, _| true), 0);
        assert!(!p.remap_item_cells(&|_, _| Some((3, 3))));
        assert!(!p.capture(&mut w, &slot1_bar(), &other_classes, &bag));

        assert_eq!(p.plates(), &row);
        assert_eq!(p.page(1), before.page(1), "the other page too");
        assert!(w.residents().is_empty(), "nothing got seated — the invariant `desync` relies on");

        // The sync ends it: pressing works again.
        p.warm_plan(&mut w, &classes, &bag);
        assert!(matches!(p.activate(&mut w, 0, &classes, &bag), Activation::Press { .. }));
    }

    #[test]
    fn a_fresh_store_is_synced_so_the_worker_has_to_desync_it_itself() {
        // `Pages::new()`/`parse` keep the pre-148 meaning for the pure tests; the worker
        // desyncs at start (no bar exists before the first load).
        assert!(Pages::new().synced());
        assert!(Pages::parse("cur 0\n").synced());
        let mut p = Pages::new();
        let mut w = Window::new();
        p.desync(&mut w);
        p.adopt_guest_bar(&mut w);
        assert!(p.synced(), "adopting the guest bar is a sync too");
    }

    // ── 148차: the store wins, but it may not forget ─────────────────────────────

    #[test]
    fn salvage_gives_the_rows_lost_plates_back_from_the_save() {
        // SLOT 2's row as the device now has it: 1..5 empty, 6·7 kept. The save on disk is
        // older than the wipe, so the bar it restores still has the five.
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_owner(Some(own_key(1, 0x7)));
        p.pages[0][5] = skill(1, 5);
        p.pages[0][6] = skill(0, 6);
        let bar = live5([live_skill(0, 0), live_skill(1, 1), live_skill(0, 2), live_skill(1, 3), live_skill(0, 4)]);
        let (_, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert!(dropped.is_empty());
        assert_eq!(p.salvage_guest_skills(&bar, &classes), 5);
        assert_eq!(p.plates(), &[skill(0, 0), skill(1, 1), skill(0, 2), skill(1, 3), skill(0, 4), skill(1, 5), skill(0, 6)]);
        // …and the warm that follows binds them in plate order.
        let (actions, _) = p.warm_plan(&mut w, &classes, &bag);
        let binds: Vec<(u8, u8)> = actions
            .iter()
            .filter_map(|a| match *a {
                Action::BindSkill { cat, cell, .. } => Some((cat, cell)),
                _ => None,
            })
            .collect();
        assert_eq!(binds, vec![(0, 0), (1, 1), (0, 2), (1, 3), (0, 4)]);
    }

    #[test]
    fn salvage_changes_nothing_when_the_row_already_holds_the_bar() {
        let (mut p, _w, row) = slot2_in_play();
        let bar = live5([live_skill(0, 0), live_skill(1, 1), live_skill(0, 2), live_skill(1, 3), live_skill(0, 4)]);
        assert_eq!(p.salvage_guest_skills(&bar, &classes), 0);
        assert_eq!(p.plates(), &row);
        // A skill the row keeps on ANOTHER page is held too.
        let mut p = Pages::new();
        p.pages[1][3] = skill(0, 8);
        assert_eq!(p.salvage_guest_skills(&live5([live_skill(0, 8), Live::Empty, Live::Empty, Live::Empty, Live::Empty]), &classes), 0);
        assert_eq!(p.plate(0), Entry::Empty);
    }

    #[test]
    fn salvage_never_replaces_and_only_takes_skills_it_can_stamp() {
        // A full row: nowhere to put anything, and nothing it holds is touched.
        let (mut p, _w, row) = slot2_in_play();
        let foreign = live5([live_skill(0, 7), live_skill(1, 8), Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(p.salvage_guest_skills(&foreign, &classes), 0);
        assert_eq!(p.plates(), &row);
        // Items are never salvaged (a cell is not an identity); an unknown class, or
        // coordinates a hand-edited save could hold, are not either.
        let mut p = Pages::new();
        let bar = live5([
            Live::Item { tab: 0, slot: 0 },
            live_skill(2, 1), // classes(2) == None
            Live::Skill { cat: 0, cell: SKILL_CELLS, sref: 1 },
            Live::Skill { cat: PARTY_SLOTS, cell: 0, sref: 1 },
            live_skill(1, 2),
        ]);
        assert_eq!(p.salvage_guest_skills(&bar, &classes), 1);
        assert_eq!(p.plate(0), skill(1, 2));
        assert!((1..PLATES as u8).all(|i| p.plate(i).is_empty()));
    }

    #[test]
    fn salvage_fills_plates_a_class_change_just_emptied() {
        // The contaminated row itself (what vc420 wrote at 00:25:26.710): plates 0 and 2 hold
        // the OTHER character's skills; plate 1's stamp happens to match this character's
        // class and stays. Warm first — it drops the two — then salvage.
        let mut p = Pages::new();
        let mut w = Window::new();
        p.adopt_owner(Some(own_key(1, 0x7)));
        for (i, cat, cell, class) in [(0usize, 0u8, 5u8, 9u8), (1, 1, 6, 4), (2, 0, 7, 9)] {
            p.pages[0][i] = Entry::Skill {
                cat,
                cell,
                class,
                sref: Some(cell),
            };
        }
        p.pages[0][5] = skill(1, 5);
        let (_, dropped) = p.warm_plan(&mut w, &classes, &bag);
        assert_eq!(dropped.len(), 2, "the two class-9 plates (party slot 0 is class 7 here)");
        let bar = live5([live_skill(0, 0), live_skill(1, 1), Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(p.salvage_guest_skills(&bar, &classes), 2);
        assert_eq!(p.plate(0), skill(0, 0));
        assert_eq!(p.plate(2), skill(1, 1), "the first EMPTY plate — plate 1 still holds a stamp that matched");
        assert_eq!(p.plate(5), skill(1, 5));
    }

    #[test]
    fn salvage_fills_visible_plates_before_hidden_ones() {
        let mut p = Pages::new();
        p.set_hidden((1 << 0) | (1 << 1));
        let bar = live5([live_skill(0, 0), live_skill(1, 1), Live::Empty, Live::Empty, Live::Empty]);
        assert_eq!(p.salvage_guest_skills(&bar, &classes), 2);
        assert_eq!(p.plate(2), skill(0, 0));
        assert_eq!(p.plate(3), skill(1, 1));
        assert!(p.plate(0).is_empty() && p.plate(1).is_empty());
    }
}
