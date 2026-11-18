//! Bag boxes — the WORKER half of 오브가방 · 재료가방 · 소비가방.
//!
//! The rules are in [`crate::bagbox`] (pure, selftested on the host). This owns the state
//! a running game needs: the file the boxes persist to, the drain of what the two guest
//! entry hooks took, the SWEEP that moves box-category cells out of the bag, the grant of
//! the three bag-item cells, the 열기 popup on one of them, the box windows, and — for
//! consumables — the MATERIALISE job that puts one unit back into a bag cell so the
//! game's own 사용하기 / hotkey can consume it.
//!
//! ── WHAT IS ON ───────────────────────────────────────────────────────────────
//! [`ENABLED`] = all three (3단계, 2026-09-15). The 재료가방 needed two things the other
//! two did not, because the game reads materials with code the host cannot virtualise:
//!   * **the crafting windows** (제련 win 13 · 대장간/일반조합 win 17) each walk the bag
//!     array with their own loop to draw their grid — 62 such walks exist
//!     (`BAG_TYPES_FEASIBILITY.md §7.2`), so there is no one function to virtualise. The
//!     window's OPENING edge pushes the box back into the bag
//!     ([`BagBoxHost::materialise`]) and the ordinary sweep takes it again when the
//!     player is next in the field. What fits in the bag is what the window shows;
//!   * **quest hand-ins**, which go through the script VM's own item helper — covered by
//!     the guest hooks `bagbox_take_lgt` / `bagbox_count_lgt` reading the MIRROR this
//!     module publishes ([`BagBoxHost::publish_mirror`]).
//!
//! ── HOW A BOXED CONSUMABLE IS USED ([`UseJob`]) ───────────────────────────────
//! The effect of a potion, a 귀환서, a cash scroll is the GUEST'S `useItem` (27 arms) —
//! nothing the host should re-implement. So "use one" is: take one unit out of the box
//! (persisted first), put it in a bag cell, then drive the game's own use of that cell:
//!   * from the box WINDOW — point the inventory cursor at the cell, synthesise OK
//!     (the popup opens on it), row 0 is 사용하기, synthesise OK again. Both OKs go
//!     through `apply_input` like the player's own, so `cash_use_pending` and the buff
//!     ledger see them exactly as before;
//!   * from a QUICKSLOT PLATE ([`crate::quickpage::Entry::Boxed`]) — loan a guest slot
//!     for one press (`activate_boxed`: Clear + BindItem + the slot's hotkey).
//!
//! ★★★ 127차 (2026-09-22) — WHICH CELL. It used to be a FREE cell, through the game's own
//! `giveItem` with `BAGBOX_BYPASS` armed for that one unit; a full bag has none, and
//! *"가방에 빈 칸이 하나 필요합니다"* was the answer (user: *"가방이 꽉차있어도 쓸 수
//! 있어야해 … 3종가방 내에서 바로사용되는 로직으로"*). Now it is the box's OWN cell: the
//! bag-item record pinned at bag 1, cell 1–3 is overwritten with the unit's record
//! ([`BagBoxHost::stage_unit`] — the bytes `giveItem`'s new-cell block writes, name
//! included, nothing else), and the `bagbox_restage_lgt` guest hook holds the record it
//! replaced: the moment the game's own removal primitive `0x5340c` reaches the cell — the
//! use tail, the hotkey tail, a 귀환 window's 예, the GAME OVER revive — it writes the
//! bag-item record back INSTEAD of clearing the cell, before the compactor and before the
//! tail's own save (오브원석). The 소비가방 cell reads as the potion for the few frames the
//! use takes, under the box window, and is itself again inside the same guest call that
//! consumed the unit. Whatever the game does NOT consume (a refused use, a declined
//! 귀환서) is put back by the host at the job's end ([`BagBoxHost::end_job`]: the record
//! over the cell, the unit into the box); a unit the job loses track of is a plain
//! consumable cell the sweep reclaims, with the knob still up for a late removal
//! ([`BagBoxHost::restage_watch`] disarms it once the cell is something else). The job
//! never runs the sweep while it is alive, so the staged cell is not taken back under
//! the game. The 110차 rule stands in front of all of it: a unit of the item ALREADY in
//! the bag is used first, without staging or debiting.
//!
//! ── WHERE THE FILE IS ────────────────────────────────────────────────────────
//! `<app root>/bagbox/slot<N>.txt`, keyed by the SAVE PIN (`vault::live_slot`, the same
//! authority the game's own record writes follow — 71차-M). Not inside the record store:
//! everything in `db/GameSave_11_<N>/` is vaulted as a save generation on every rewrite
//! and classified through the save cipher, and a text file there would be a permanent
//! `Md5Bad` in those logs. 초기화 (`Database::delete`) removes the slot's file explicitly
//! ([`BagBoxHost::forget_slot`]) so a new character never inherits the old one's boxes.
//!
//! ⛔ BOX WRITE FIRST, then the bag cell. Every path here persists the box before it
//! touches the guest; a crash between the two can only duplicate, never lose. The one
//! path that goes the other way — a materialised unit — is refunded when the give does
//! not land.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use wie_backend::{Emulator, Event, KeyCode};
use winit::event_loop::EventLoopProxy;

use crate::{
    InputEvent, UserEvent,
    bagbox::{self, Boxes, Kind, Screen, Stack, Verb},
    items,
    offline::{self, QtyAsk},
    quickpage,
};

/// The boxes that take items. See the module header.
pub const ENABLED: [Kind; 3] = [Kind::Orb, Kind::Material, Kind::Consumable];

/// Bag cells moved into a box per sweep poll — one poll is 30 frames, and each move is
/// seven guest writes plus a file write.
// ⛔ `SWEEP_MAX_PER_POLL` (4 cells per 30-frame poll) WAS HERE and is gone (100차,
// 2026-09-16). The sweep now takes the WHOLE plan in one pass — one box-file write, then
// every cell — because four-per-half-second is exactly what let a player open bag 2 and
// find it untouched while bag 1 had already moved (user: *"다음 가방으로 페이지를 넘기니까
// 그대로 전부 다 있어"*).

/// Frames a load-edge migration may wait for a quiet screen before the ordinary 30-frame
/// poll is left to catch it. World init and the online login both settle in well under a
/// second; this is a guard, not a schedule.
const MIGRATE_WAIT_FRAMES: u64 = 600;
/// A `giveItem` call is asynchronous; wait this long before deciding it did not land.
const GRANT_INFLIGHT_FRAMES: u64 = 90;
/// How long a materialised unit may take to appear in the bag, and the popup to open.
const USE_WAIT_MS: u64 = 1500;

/// How long one materialise batch may take to show up in the bag.
const MAT_WAIT_MS: u64 = 1500;
/// 118차-B: after a revive supply the bag would not take (no free cell), how many frames
/// before the next try — nothing frees a cell during a death, so this only keeps the log
/// from repeating every frame.
/// ⛔ 135차-B: NOT for a staging a live use job holds — that one DOES free during a death
/// (see `revive_supply`), so it is retried on the very next frame instead.
const REVIVE_RETRY_FRAMES: u64 = 600;
/// 125차-B: a stranded stack the bag would not take (no free cell) is tried again after this.
const EVICT_RETRY_FRAMES: u64 = 600;

/// What the orb-combine flow is spending: a bag cell (the 69차 path, untouched) or a
/// stack of the 오브가방.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbSource {
    Bag { tab: u8, slot: u8, key: u64 },
    Box { kind: Kind, at: usize, cat: u16, idx: u16 },
}

/// The rows a box window is showing, and where they came from.
///
/// ★★ 96차: two SETS of boxes. On [`Screen::Inventory`] and [`Screen::OnlineBag`] the rows
/// are the bag's own box ([`Boxes`]); on [`Screen::OnlineWarehouse`] they are the OFFLINE
/// STORE's rows of that box's categories — a view, not a store of its own
/// (`BAG_TYPES_FEASIBILITY.md §9.1`) — and each row carries the 8-byte key the store
/// knows it by, which is how a verb names it later.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listing {
    screen: Screen,
    stacks: Vec<Stack>,
    /// One key per stack for the warehouse view; empty for the bag's own boxes.
    keys: Vec<[u8; 8]>,
}

/// Which host dialog the shared `OrbPick` answer channel is answering for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dialog {
    /// The box window: rows are the stacks listed.
    Window { kind: Kind, list: Listing },
    /// The verb DROP BOX for one stack — [`bagbox::verbs`] of that box's kind and screen.
    ///
    /// ★ It carries the window's rows so dismissing it can put the dialog back WITHOUT
    /// republishing the list. The list is still on screen under it and still correct
    /// (nothing has been spent), and republishing would reset the player's scroll position
    /// to the top — on a 48-stack box that is a real loss.
    Verbs { kind: Kind, at: usize, list: Listing },
    /// 버리기 asked; the box now reads 예 / 아니오 on the same row.
    ///
    /// ⛔ The only verb that destroys something is the only one with a second box. The
    /// game asks before it throws an item away and so does this.
    Confirm { kind: Kind, at: usize, list: Listing },
    /// A host 올리기 / 내리기 (96차) waiting for its count in the warehouse's own picker,
    /// drawn over the list. Answered through `network::host_qty_reply`, not this channel.
    Qty { kind: Kind, at: usize, verb: Verb, list: Listing },
}

/// Where a materialised unit is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseTarget {
    /// The box window's 사용하기: drive the inventory popup.
    Menu,
    /// A quickslot plate: loan a guest slot and press its hotkey.
    Field { plate: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsePhase {
    /// Waiting for the unit to show in a bag cell — immediately, for a staged unit
    /// (127차: it is written there before the job exists); a leftover is found the same way.
    WaitCell,
    /// The cursor is on the cell and OK was sent; waiting for the popup.
    WaitPopup { tab: u8, slot: u8 },
    /// 사용하기 was confirmed; waiting for the GAME to run the use (its key handling
    /// lands ~100 ms later on the device) so the result can be shown and its box closed
    /// — 110차-B. `before` = the bag's units of the item when the OK went out.
    WaitResult { before: u32 },
    /// 127차: the quickslot hotkey went out on a STAGED cell; waiting for the game to run
    /// the use, so the job stays alive — and the poll's sweep stays away from the cell —
    /// until the unit is consumed or the guard runs out. No key is ever sent from here:
    /// the field's boxes are the game's (a 귀환서's window, say), not ours to answer.
    /// `before` = the bag's units of the item when the hotkey went out.
    ///
    /// ★★★ 129차 (128차 code review ①): `asked` = the game has put a box up since the
    /// hotkey — a 귀환서's 「이동하겠습니까?」, its refusal message — and the PLAYER owns the
    /// clock: the job stays (guard [`USE_ANSWER_MAX_MS`], like [`Self::WaitGameAnswer`])
    /// until that box is gone, then `settled` frames of nothing consumed and the scene
    /// (`scene`) unchanged mean declined, and the unit goes back into the box. ⛔ Without
    /// this the 1 s guard let the job go with the question still up, the next quiet poll
    /// swept the staged unit into the box (+1) and cleared the cell, the grant put the
    /// 소비가방 cell back there — and 예 then removed THAT cell and warped: a free 귀환 and
    /// a deleted bag-item cell, every time the player took more than a second to answer.
    WaitField { before: u32, asked: bool, scene: Option<u16>, settled: u8 },
    /// 116차/117차-C: 사용하기 confirmed on an item whose `useItem` RAISES THE GAME'S OWN
    /// DIALOG instead of consuming (귀환서 / 그리폰의피리 — `bagbox::use_opens_window`).
    /// Waiting for that dialog — the shared choice widget (`MenuState::choice`), NOT a
    /// window: the device read `win=3` with `rows=2 axis=0` right after 사용하기, which is why
    /// 116차's `window_id == 15` wait never fired. `before` = bag units of the item when the
    /// 사용하기 OK went out.
    WaitGameChoice { tab: u8, slot: u8, before: Option<u32> },
    /// The game's dialog is up and the PLAYER is answering it — through the box window
    /// (귀환서, `UserEvent::OrbReveal`: the game's own message box is re-blitted over the
    /// window and the keys are the game's — 117차-D) or with the window hidden (그리폰의피리's
    /// town list). Waiting for it to be gone and for the outcome. `before` = bag units of
    /// the item when the dialog came up (`None` while the bag could not be read — filled in
    /// while the dialog is still up, and a reclaim is REFUSED if it never was: not knowing
    /// whether 예 consumed is no licence to take the unit back — 117차), `scene` = the scene
    /// it opened in, `settled` = consecutive polls the dialog has been gone with nothing
    /// consumed (the reclaim waits [`USE_ANSWER_SETTLE_POLLS`] of them).
    WaitGameAnswer { before: Option<u32>, scene: Option<u16>, settled: u8 },
}

/// How long [`UsePhase::WaitResult`] waits for the game to consume the unit. Measured
/// ~100 ms (S25: OK pushed .740, `writeSaveRecord` .838); this is a guard.
const USE_RESULT_MS: u64 = 1000;

/// How long [`UsePhase::WaitGameAnswer`] waits for the player to answer the game's dialog
/// before the job lets go (the unit is then the sweep's, as ever). A player may sit on
/// 예/아니오 as long as they like; this only keeps a forgotten job from living forever.
const USE_ANSWER_MAX_MS: u64 = 600_000;
/// Polls the game's window must have been GONE, with nothing consumed and the scene
/// unchanged, before the unit is taken back into the box — 예 consumes inside the key
/// handler (`0x46854`), but the warp it posts lands a tick later, so a couple of frames
/// of patience keeps "answered 예" from ever reading as "declined". ~100 ms.
const USE_ANSWER_SETTLE_POLLS: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UseJob {
    kind: Kind,
    cat: u16,
    idx: u16,
    target: UseTarget,
    phase: UsePhase,
    deadline: u64,
    /// OKs spent closing a game box that was up when the job wanted the cell's popup
    /// (110차 — 오브원석's result widget). Capped by [`USE_DISMISS_MAX`].
    dismissed: u8,
    /// Whether `start_use` took a unit OUT OF THE BOX for this job (and wrote the file).
    /// `false` on the 110차 leftover path — a unit already in the bag is used and the box
    /// keeps its count — so a refund on THAT job would mint a unit the box never gave
    /// (117차 code review): `refund` only adds when this is set.
    debited: bool,
    /// 127차: the unit was written OVER the box's own bag-item cell (no free cell needed),
    /// and this is that cell with the record that goes back. `None` on the leftover path.
    staged: Option<items::BagStage>,
}

/// Where the job's unit is: `(tab, slot, count)`. A STAGED job knows its cell — nine
/// reads at the record the stage was taken in ([`items::bag_stage_cell`]) — and answers
/// `None` once that record is anything but the staged item; the 110차 leftover path has
/// no cell of its own and walks the bag for the first stack of the item (129차, 128차
/// code review ⑥: the field phases used to walk all 80 cells every frame either way).
fn job_cell(read: &impl Fn(u32, usize) -> Vec<u8>, job: &UseJob) -> Option<(u8, u8, u16)> {
    match job.staged {
        Some(st) => match items::bag_stage_cell(read, &st)? {
            Some((c, i, n)) if (c, i) == (job.cat, job.idx) => Some((st.tab, st.slot, n)),
            _ => None,
        },
        None => items::bag_cell_of(read, job.cat, job.idx),
    }
}

/// Units of the job's item the bag holds — what "consumed" is measured against. For a
/// staged job that is the staged cell's count while it still reads as the item and 0
/// once the restage hook has put the bag-item record back (the one way the game takes
/// a staged unit); a leftover job sums the bag. `None` when the bag cannot be read.
fn job_units(read: &impl Fn(u32, usize) -> Vec<u8>, job: &UseJob) -> Option<u32> {
    match job.staged {
        Some(st) => items::bag_stage_cell(read, &st).map(|cell| match cell {
            Some((c, i, n)) if (c, i) == (job.cat, job.idx) => u32::from(n),
            _ => 0,
        }),
        None => items::bag_units_of(read, job.cat, job.idx),
    }
}

/// How a use job ended, for [`BagBoxHost::end_job`] to settle a staged cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobEnd {
    /// The game consumed the unit (the bag's count dropped, or the scene moved on 예).
    Consumed,
    /// The game did not touch it: refused, declined, or the use was never reached. The
    /// unit goes back into the box and the bag-item record over the cell — now.
    Unused,
    /// Cannot tell (the world was left, the bag stopped reading, a guard ran out with
    /// the game's dialog still up). The cell is left as it is: the restage hook still
    /// answers a late consumption, the sweep still reclaims a leftover, and
    /// [`BagBoxHost::restage_watch`] disarms the knob once the cell is something else.
    Unknown,
}

/// How many OKs a use job may spend closing the game's own box before it sends the
/// cursor OK regardless. ⛔ ONE, not more: the keys are queued and the guest may close
/// the box a tick later than the flag is re-read, and a second "dismiss" OK arriving at
/// a box that is already gone opens the popup on whatever cell the cursor sits on —
/// the pinned 오브가방 cell, on a freshly opened window. One dismiss followed by the
/// cursor write + OK is safe in either order of the guest's timing: the bytes are
/// written before either key is processed, so the first OK closes the box and the
/// second opens the popup on OUR cell. Past this the job falls back to the pre-110차
/// behaviour (and its deadline).
const USE_DISMISS_MAX: u8 = 1;

/// One stack being pushed back into the bag: `giveItem` was called `sent` times with the
/// bypass armed, and `before` is what the bag held of that item when it was.
///
/// ★ The box is NOT debited until the units are SEEN in the bag. A crash in between
/// duplicates the stack, which the sweep then folds back in — the other order loses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatBatch {
    /// The box the stack came out of — per stack since 121차, when the craft windows
    /// started pushing back a recipe's boxed BASE (포도주 from the 소비가방) as well.
    kind: Kind,
    cat: u16,
    idx: u16,
    sent: u16,
    before: u32,
    deadline: u64,
}

/// 118차-B: the ONE 부활의서 lent from the 소비가방 to the bag while the player is dying, so
/// the game's own revive check finds it (`bagbox::REVIVE_SCROLL`). Same shape and the same
/// order as [`MatBatch`]: the box is debited AFTER the unit is seen in the bag, and the
/// debit is LOANED until a GameSave captures the bag (`bagbox::Loan`) — a game over that
/// goes to the title saves nothing, so the next load folds the loan back into the box
/// (108차) and the unit is the box's again. Never a loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReviveBatch {
    kind: Kind,
    cat: u16,
    idx: u16,
    sent: u16,
    before: u32,
    deadline: u64,
    /// 127차: the scroll was written over the 소비가방 cell (see `UseJob::staged`).
    staged: Option<items::BagStage>,
}

/// EVICT a stranded stack (a 조합서 an old file still keeps in the 재료가방) for good.
///
/// ⛔ The smith no longer materialises materials (125차-C: countOf/spend). Eviction is
/// the only remaining asker. An eviction has no window and stops when the field is left
/// (it is a guest call, and those are field-only).
#[derive(Debug, Clone)]
struct MatJob {
    /// Stacks still to push with the box each comes out of, in plan order (popped from
    /// the back). Carried, not derived: a STRANDED stack has no box by `bagbox::kind_of`
    /// any more, and `mat_frame` must still debit the box it actually sits in.
    queue: Vec<(Kind, Stack)>,
    batch: Option<MatBatch>,
    /// The window that asked, or `None` for an eviction.
    window: Option<u8>,
}

pub struct BagBoxHost {
    boxes: Boxes,
    /// The save slot the boxes belong to — `vault::live_slot()` as of the last frame.
    slot: Option<u8>,
    path: Option<PathBuf>,
    dirty: bool,
    /// Which bag-item cells the bag held the last time it was looked at (bit per
    /// `Kind::index`), for the vanish diagnostic in `frame`. `None` = not looked yet, or
    /// the bag was unreadable.
    cells_seen: Option<u8>,
    /// The frame the last bag-item give burst was called on. While set, `frame` watches
    /// for the cells to land and pins them at once; `poll` will not give again inside
    /// `GRANT_INFLIGHT_FRAMES`.
    grant_wait: Option<u64>,
    /// A window to put up: `(screen, kind, keep the scroll and the focused row)`. `true`
    /// is a REFRESH — the same list with new counts, after a use or a 버리기 — and
    /// throwing the player back to the top of a 48-stack box is exactly what 93차 fixed.
    open_request: Option<(Screen, Kind, bool)>,
    /// The screen the last window was opened from — what a `reopen` goes back to.
    last_screen: Screen,
    /// Keys minted for rows the host files in the warehouse (`offline::new_stack`).
    move_seq: u32,
    /// The host dialog that is up, if any.
    dialog: Option<Dialog>,
    /// A character record was just LOADED (`vault::take_load_edge`): run the migration sweep
    /// on the first quiet frame, not on the next 30-frame poll. `Some(frame seen)`.
    ///
    /// ★★★ 100차 — WHY AT THE EDGE. Two entrances show the bag: the field (게임시작) and the
    /// online 싱글가방 (네트워크창고). On the online path the guest loads the record, sends
    /// `04 3d`, and on the NEXT tick consumes the in-app server's ack — whose handler
    /// `saveGame`s the character. A sweep that runs between those two ticks lands BEFORE
    /// that save, so the record on disk is the cleaned bag from the first second; a sweep
    /// that waited for the poll would have let the ack persist the dirty bag first.
    migrate_pending: Option<u64>,
    /// ★★★ 100차-B: the absorbed ledger ([`bagbox::Boxes::absorbed`]) is consulted by the
    /// FIRST sweep after a record load (the migration, or the ordinary poll if the load
    /// edge found no quiet frame) and by no later one — inside a session nothing comes
    /// back from disk, so a later cell at a ledger position is a new stack (the 재료가방's
    /// own materialised 제련석, for one). `true` from the load until a sweep has read the
    /// bag. A GameSave commit that lands while this is still `true` keeps the ledger: the
    /// bag it wrote is the unswept one, so the disk still holds those cells.
    ledger_live: bool,
    /// Whether the hooks are being told to take anything this frame.
    armed: bool,
    /// Items the drain could not fit (should not happen — the hook decrements room).
    overflow: u32,
    /// The slot's file exists but cannot be read: stay off rather than start over it.
    unreadable: bool,
    /// The materialise-and-use in flight.
    job: Option<UseJob>,
    /// What a box absorbed while the use job was in [`UsePhase::WaitResult`] — the
    /// orb 오브원석 rolled — so the window can say it (110차-B).
    absorbed_since_use: Option<String>,
    /// 퀵슬롯등록 from the box window: the item the next plate tap binds.
    pending_bind: Option<(u16, u16)>,
    /// The materialise in flight (125차-C: stranded-recipe eviction only).
    mat: Option<MatJob>,
    /// 118차-B: the death-time 부활의서 supply in flight, and the frame before which no new
    /// one is started after the bag refused (see `REVIVE_RETRY_FRAMES`).
    revive: Option<ReviveBatch>,
    revive_retry_at: u64,
    /// 135차-B: the "a use job holds the staging cell" line has been said for this death.
    revive_held_said: bool,
    /// 125차-B: the eviction of stranded stacks retries no earlier than this frame — set
    /// after the bag had no free cell, or took fewer units than were sent.
    evict_retry_at: u64,
    /// Raised by `mat_frame` when an eviction batch fell short; `evict_stranded` turns it
    /// into the retry frame (it is the one with a frame counter).
    evict_backoff: bool,
    /// The mirror the guest hooks read is behind the boxes and must be republished.
    mirror_dirty: bool,
    /// Keys the job wants delivered THROUGH `apply_input` (so the cash-item ledger sees
    /// them), drained by the worker loop after the UI's own channel.
    self_input: VecDeque<InputEvent>,
    /// 재료가방 crash-durability: units the guest has taken from a box (use job, stranded
    /// eviction, TAKE hook, 125차-C spend) that a GameSave has not confirmed yet. The box
    /// FILE is written as `boxes + loan` so a crash leaves them recoverable in the box
    /// (see `bagbox::Loan`).
    loan: bagbox::Loan,
    /// The last `crate::gamesave_commit().0` this host acted on.
    last_gamesave_seq: u64,
    /// A `flush_fast` (104차 ③, no fsync) has published bytes that are not durable yet.
    /// The same bytes go down again WITH the fsyncs as soon as the use job that wanted the
    /// speed is over (`frame`), or before the file is left behind (`sync_slot`). See
    /// `flush_fast` for why a non-durable publish is a transient and not a state.
    durable_pending: bool,
}

impl BagBoxHost {
    pub fn new() -> Self {
        Self {
            boxes: Boxes::default(),
            slot: None,
            path: None,
            dirty: false,
            grant_wait: None,
            cells_seen: None,
            open_request: None,
            migrate_pending: None,
            ledger_live: false,
            last_screen: Screen::Inventory,
            move_seq: 0,
            dialog: None,
            armed: false,
            overflow: 0,
            unreadable: false,
            job: None,
            absorbed_since_use: None,
            mat: None,
            revive: None,
            revive_retry_at: 0,
            revive_held_said: false,
            evict_retry_at: 0,
            evict_backoff: false,
            mirror_dirty: true,
            pending_bind: None,
            self_input: VecDeque::new(),
            loan: bagbox::Loan::default(),
            last_gamesave_seq: 0,
            durable_pending: false,
        }
    }

    pub fn boxes(&self) -> &Boxes {
        &self.boxes
    }

    /// Units of `(cat, idx)` across the box that holds that category — the plate badge.
    pub fn count_of(&self, cat: u16, idx: u16) -> u16 {
        let Some(kind) = bagbox::kind_of(cat) else { return 0 };
        self.boxes
            .stacks(kind)
            .iter()
            .filter(|s| s.cat == cat && s.idx == idx)
            .map(|s| u32::from(s.count))
            .sum::<u32>()
            .min(u32::from(u16::MAX)) as u16
    }

    /// ★ 138차 — the 하늘성 exit reward's 닉스코어의 파편 go STRAIGHT into the box (the
    /// 재료가방), the way the intake hook files a picked-up unit: file first, then one
    /// 103차 획득 notice. ⛔ Not the game's `giveItem` N times: the tower's giver waits 90
    /// frames on a repeated `(cat, idx)`, so thirty shards were a 45-second drip that the
    /// poll block closed on (`TOWER_REWARD_BALANCE_DRAFT.md §3.1 ⓑ`).
    ///
    /// Returns the units that did NOT go in — no box armed for this slot yet, a category
    /// no box takes, or a full box — and the caller hands those to the game's giver
    /// instead, so nothing is lost. Immediately durable, like the intake: a reward earned
    /// in the tower is not rolled back by reloading an older save (a picked-up drop's rule).
    pub fn grant_boxed(&mut self, emulator: &mut dyn Emulator, cat: u16, idx: u16, n: u32) -> u32 {
        let Some(kind) = bagbox::kind_of(cat) else { return n };
        if n == 0 || !self.armed || !ENABLED.contains(&kind) {
            return n;
        }
        // ★ 165차: file what the intake hook holds reserved first, so this add cannot spend a
        //   stack one of those units was promised — and republish after, so the hook does not
        //   spend the one this takes (`publish_knobs`).
        self.drain_intake(emulator);
        let left = self.boxes.add(kind, cat, idx, n);
        if left < n {
            self.dirty = true;
            self.mirror_dirty = true;
            self.flush();
            self.announce(emulator, cat, idx);
            log::warn!(
                "bagbox: {} unit(s) of cat {cat:#x} idx {idx:#x} went straight into the {} (tower reward){}",
                n - left,
                kind.title(),
                if left > 0 { format!(" — {left} did not fit") } else { String::new() }
            );
        }
        self.publish_knobs(emulator);
        left
    }

    /// Is a host dialog (window or verb picker — not the orb target picker) the one the
    /// UI is showing?
    pub fn window_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// ★ 136차 — why the bag must NOT be reordered right now, or `None` when it may.
    ///
    /// Everything here remembers a bag cell by its `(tab, slot)`: a use job's staged cell
    /// and its snapshot, a box window's rows, and — the one that would silently duplicate
    /// items — the load-edge LEDGER (`Boxes::absorbed`, 100차), which the first sweep after
    /// a load matches cell by cell. Sorting under it would make every ledger entry miss and
    /// the sweep would absorb those cells as NEW stacks (106차's rule, the duplicate
    /// direction). So a sort waits until that first sweep has run.
    pub fn sort_blocker(&self) -> Option<&'static str> {
        if self.job_pending() {
            Some("a bag-box job is using a cell")
        } else if self.stage_held() || self.revive.is_some() {
            Some("a staged unit is in the bag")
        } else if self.window_open() {
            Some("a bag-box window is open")
        } else if self.ledger_live {
            Some("the load-edge sweep has not run yet")
        } else {
            None
        }
    }

    /// The next key the job wants delivered through `apply_input`.
    pub fn take_input(&mut self) -> Option<InputEvent> {
        self.self_input.pop_front()
    }

    fn file_in(app_root: &Path, slot: u8) -> PathBuf {
        app_root.join("bagbox").join(format!("slot{slot}.txt"))
    }

    fn file_for(data_dir: &Path, slot: u8) -> Option<PathBuf> {
        Some(Self::file_in(&crate::vault::app_root(data_dir)?, slot))
    }

    /// 초기화 on `db_name` (`GameSave_11_<n>`): drop that slot's boxes too. Called from
    /// `AndroidDatabaseRepository::delete` with the app root (`<data>/<app_id>`), which is
    /// the only guest-reachable path that removes a character.
    pub fn forget_slot(app_root: &Path, db_name: &str) {
        let Some(slot) = db_name.strip_prefix("GameSave_11_").and_then(|s| s.parse::<u8>().ok()) else {
            return;
        };
        let p = Self::file_in(app_root, slot);
        if p.exists() {
            match std::fs::remove_file(&p) {
                Ok(()) => log::warn!("bagbox: slot {slot} was reset (초기화) — its boxes are gone with it"),
                Err(e) => log::warn!("bagbox: could not remove {}: {e}", p.display()),
            }
        }
    }

    /// A slot just received a NEWLY CREATED character: its boxes belong to whoever used
    /// to live there and must not be inherited. Removes the FILE and, when that slot is
    /// the live one, the IN-MEMORY copy as well.
    ///
    /// ★★★ WHY THIS EXISTS AND `forget_slot` IS NOT ENOUGH (2026-09-16, player report:
    /// *"캐릭터 삭제 후 재생성하니까 … 원래 처음 시작하면 포션 5개인데 9개로 되어있어"* —
    /// 5 new + 4 the deleted character had left in the 소비가방). Two separate holes:
    ///
    ///   ⓐ [`forget_slot`] is a STATIC method called from the DB repository, so it can
    ///     delete the file but cannot touch this struct. The live `boxes` kept the dead
    ///     character's stacks, `sync_slot` does not reload while the slot number is
    ///     unchanged, and the very next flush wrote them straight back.
    ///   ⓑ The device log of that session has NO `DB delete_database` line at all, so the
    ///     delete hook cannot be the only thing this relies on.
    ///
    /// So the trigger is the CREATION edge instead (`savevault::creation_slot`: on a
    /// creation screen, entering the world, onto a slot that holds no save) — which is
    /// true exactly when the previous occupant is already gone, and needs no delete event.
    pub fn forget_created_slot(&mut self, data_dir: &Path, slot: u8) {
        if let Some(p) = Self::file_for(data_dir, slot)
            && p.exists()
            && let Err(e) = std::fs::remove_file(&p)
        {
            log::warn!("bagbox: could not remove {}: {e}", p.display());
        }
        if self.slot == Some(slot) {
            self.boxes = Boxes::default();
            self.loan.clear();
            // The file is gone and the boxes are empty: there is nothing to write back —
            // and nothing left to make durable.
            self.dirty = false;
            self.durable_pending = false;
            // ⛔ The quest mirror still holds the dead character's counts — republish it.
            self.mirror_dirty = true;
            self.mat = None;
            self.job = None;
            self.revive = None;
            // 127차: a staged cell of the dead character is nothing to restore.
            wie_util::stats::clear_bagbox_restage();
            self.dialog = None;
            self.open_request = None;
            self.pending_bind = None;
            self.grant_wait = None;
            self.cells_seen = None;
        }
        log::warn!("bagbox: slot {slot} got a NEW character — its boxes are gone (file and memory)");
    }

    /// NEW GAME PLUS: repeat the game's own transition removal on the boxes (2026-09-21).
    ///
    /// The ending's window 18 runs `0x3af2c`, which takes 그리폰의피리, its recipe and every
    /// quest item out of the BAG — see [`bagbox::new_game_plus_removes`] for the
    /// disassembly. It walks bag cells, so it cannot see ours, and the player reported the
    /// result: *"난이도를 넘어가게 되면 소비가방에 여전히 남아있어"*.
    ///
    /// ★★★ 124차-B (2026-09-21): THE DEBIT IS A LOAN, NOT A WRITE. 제보: *"하드코어 들어가기
    /// 전에 저장을 하고 하드코어 클리어 이후 저장안하고 다시 세이브슬롯을 불러오면 …
    /// 피리가 사라지고"*. The game's half of this transition — the bag walk, the difficulty
    /// byte, the warp — lives in UNSAVED memory until a GameSave; a reload of the
    /// pre-transition record rolls all of it back. The first cut wrote the box debit
    /// straight to the file (and `loan.sub`bed any loan beside it), so the box alone did
    /// NOT roll back and the 피리 was gone for good. That is the 97차/107차/108차 rule
    /// broken one more time: *every path that takes units out of a box owes a loan, and
    /// the loan is settled wherever the bag changes.*
    ///
    /// So: `spend_units` + **`loan.add(took)`** — exactly the 107차 TAKE arm. The FILE
    /// keeps showing the rows (`flush` writes `boxes + loan`), the box WINDOW and the
    /// quest mirror read `self.boxes` and show them gone, and then:
    /// * a GameSave for this slot commits → `loan.clear()` promotes the debit (the
    ///   2회차 bag is durable, so the 피리 is really gone);
    /// * the SAME slot is reloaded without one → the load edge folds the loan back
    ///   (108차) and the 피리 is in the box again, next to the pre-transition bag;
    /// * a DIFFERENT slot is loaded → `sync_slot` flushes `boxes + loan` to this slot's
    ///   file first, so it too keeps the 피리 for whenever this record is loaded again;
    /// * the app is killed before any of that → the file never lost the row (this closes
    ///   §124차 §4's "known limit" as well).
    ///
    /// ⛔ NO `loan.sub` HERE. The first cut's reasoning — "an outstanding loan would put
    /// the row back into the file" — described the correct behaviour and called it a bug:
    /// a 피리 that was materialised into the bag and is now removed there BY the game is
    /// exactly a unit the pre-transition save still owns. Subtracting its loan erased the
    /// only record that it was ever the box's.
    ///
    /// ⛔ Nothing here is a filter. The boxes may hold these rows again the moment a quest
    /// hands one over — this runs once, on the edge, and that is the whole of it.
    ///
    /// Returns what it took, for the caller's log; empty when there was nothing.
    pub fn new_game_plus(&mut self) -> Vec<(Kind, u16, u16, u32)> {
        let plan = bagbox::new_game_plus_plan(&self.boxes);
        if plan.is_empty() {
            return plan;
        }
        for &(kind, cat, idx, n) in &plan {
            let took = self.boxes.spend_units(kind, cat, idx, n);
            // Only what the box actually gave up is owed back (107차: `took`, not `n`).
            self.loan.add(kind, cat, idx, took);
            log::warn!(
                "bagbox: new game plus — the {} gives up {took} x cat {cat:#x} idx {idx:#x} (the game does the same to the bag; loaned until a GameSave)",
                kind.title()
            );
        }
        self.mark_dirty();
        self.flush();
        plan
    }

    /// The boxes changed: persist on the next flush, and republish the mirror the quest
    /// hooks read. ⛔ Never set `dirty` alone — a stale mirror makes the script VM count
    /// or spend units the box no longer has.
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.mirror_dirty = true;
    }

    /// Publish what each box holds, per `(cat, idx)`, for the two quest-VM hooks
    /// (`bagbox_take_lgt` / `bagbox_count_lgt`) — they run inside a guest call and cannot
    /// reach this struct. Only on a change: the clear touches 1,280 atomics.
    ///
    /// ★ No arming term. `wie_util::stats::bagbox_count` returns 0 for a category that
    /// `BAGBOX_ACCEPT` does not name, so a box that is off already reads as empty.
    fn publish_mirror(&mut self) {
        if !self.mirror_dirty {
            return;
        }
        wie_util::stats::bagbox_clear_counts();
        for k in Kind::ALL {
            for ((cat, idx), n) in self.boxes.totals(k) {
                wie_util::stats::set_bagbox_count(u32::from(cat), u32::from(idx), n);
                // ★ 165차: and how much more those stacks take — the intake fills this before
                //   it spends a free stack (`wie_util::stats::BAGBOX_MERGE`).
                wie_util::stats::set_bagbox_merge(u32::from(cat), u32::from(idx), self.boxes.merge_room(k, cat, idx));
            }
        }
        self.mirror_dirty = false;
    }

    /// The free stacks the intake hook may open per box (`BAGBOX_ROOM`); 0 for a box that is
    /// off or not armed.
    fn publish_room(&self) {
        for k in Kind::ALL {
            let room = if self.armed && ENABLED.contains(&k) { self.boxes.room(k) } else { 0 };
            wie_util::stats::set_bagbox_room(k.index(), room);
        }
    }

    /// ★★ 165차 — bring everything the intake hook reads up to date: what it took is filed
    /// (the drain), and the mirror and the room are the boxes' again.
    ///
    /// The hook RESERVES as it takes (merge room, a stack, the mirror's count) and the drain
    /// settles those reservations into the boxes, so the two only agree right after a drain.
    /// `frame` used to be the one place that republished, and a host change to a box between
    /// that and the next tick — the sweep in `poll`, a refund in the step loop, a tower grant
    /// — left the hook working from room that was no longer there (the drain's "could not
    /// fit" line: a unit the guest was told it had, gone). Every path that changes a box
    /// outside `frame`'s own drain-then-publish now ends here.
    /// ⛔ Drain FIRST: publishing over undrained entries would wipe their reservations.
    fn publish_knobs(&mut self, emulator: &mut dyn Emulator) {
        self.drain_intake(emulator);
        self.publish_mirror();
        self.publish_room();
    }

    fn flush(&mut self) {
        self.flush_with(true);
    }

    /// The same write WITHOUT waiting for the flash (104차 ③, user decision 2026-09-18).
    ///
    /// Only the 사용하기 path uses it, and only because that is the one write a player is
    /// WAITING on: `start_use` persists the debited box (158차 D1: `boxes + loan`, so the
    /// bytes still show the unit until a GameSave confirms the use) before it hands the unit
    /// to the game, and `database::write_atomic`'s two `sync_all` calls are tens of milliseconds
    /// of the ~0.1 s the player reported. The publish is still atomic (temp + `rename`),
    /// so no reader — this process after a restart included — can see a half-written box.
    ///
    /// ⛔ THE ORDER STAYS: the box file is written BEFORE the guest is given the unit. What
    /// this drops is the durability of that write against a power cut, not its position.
    ///
    /// ★★ AND IT IS A TRANSIENT, NOT A STATE (104차-B code review). "Losing it is not a
    /// loss — the unit is still in the box on the next load" covered one outcome only. A
    /// rename that reaches the journal without its data blocks reads back after a power
    /// cut as an EMPTY file: the whole box, not the one unit, and `flush_with` had already
    /// dropped `dirty`, so nothing would have written it again. Now a non-durable publish
    /// raises `durable_pending`, `frame` writes the same bytes again WITH the fsyncs the
    /// moment the job is over (`job_pending()` false — the wait the player felt is behind
    /// them by then), and `sync_slot` does it before the file is left. The exposure is the
    /// job's own ~1 s, and `Boxes::parse_file` turns its worst case into a loud,
    /// fail-closed slot instead of an empty box written back for good.
    fn flush_fast(&mut self) {
        self.flush_with(false);
    }

    fn flush_with(&mut self, durable: bool) {
        if !self.dirty {
            return;
        }
        let Some(p) = self.path.as_ref() else {
            return;
        };
        if let Some(dir) = p.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::error!("bagbox: cannot create {}: {e}", dir.display());
            return;
        }
        // ★ The file holds `boxes + loan`: materials materialised into an unsaved bag
        //   still show in the box on disk until a GameSave confirms them (bagbox::Loan).
        let text = if self.loan.is_empty() {
            self.boxes.to_text()
        } else {
            self.loan.overlay(&self.boxes).to_text()
        };
        match crate::database::write_atomic_with(p, text.as_bytes(), durable) {
            Ok(()) => {
                self.dirty = false;
                // A non-durable publish owes a durable one; a durable one settles it.
                self.durable_pending = !durable;
            }
            Err(e) => log::error!("bagbox: write to {} FAILED: {e} — will retry", p.display()),
        }
    }

    /// Follow the save pin: a different slot means a different file (and a flush of the
    /// old one first). No pin = no boxes, and the hooks are told to take nothing.
    fn sync_slot(&mut self, data_dir: &Path, frame: u64, proxy: &EventLoopProxy<UserEvent>) {
        let slot = crate::vault::live_slot();
        // `vault::app_root` is `None` until the game has opened its first record store
        // (it scans for `db/`); a pin that arrived before that is retried, not forgotten.
        let retry = slot.is_some() && self.path.is_none() && !self.unreadable && frame % 60 == 0;
        if slot == self.slot && !retry {
            return;
        }
        // ★ 104차-B: a non-durable publish still owed its fsyncs goes down durably before
        //   the file is left behind (`flush_fast`).
        if self.durable_pending {
            self.dirty = true;
        }
        self.flush();
        self.unreadable = false;
        self.slot = slot;
        self.dialog = None;
        crate::network::host_qty_withdraw();
        self.open_request = None;
        self.grant_wait = None;
        self.cells_seen = None;
        self.job = None;
        self.mat = None;
        self.mirror_dirty = true;
        self.pending_bind = None;
        // A different character's bag is a different save: any outstanding loan was that
        // character's, and its box file already holds the pre-debit state.
        self.loan.clear();
        // The file being (re)read below carries the ledger of the bag on this slot's
        // disk record — the next sweep is the one that gets to consult it.
        self.ledger_live = true;
        wie_util::stats::set_bagbox_bypass(0, 0, 0);
        // 127차: a staged cell belongs to the bag that was just replaced — the address and
        // the record it names are that world's. Whatever it held is on that slot's disk.
        self.revive = None;
        wie_util::stats::clear_bagbox_restage();
        self.path = slot.and_then(|s| Self::file_for(data_dir, s));
        self.boxes = match self.path.as_ref() {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(text) => match Boxes::parse_file(&text) {
                    Some(b) => b,
                    None => {
                        // ⛔ A file without its header is DAMAGE, not an empty box (104차-B):
                        // every file this host ever wrote begins with `FILE_HEADER` (92차),
                        // so a zero-length or torn file — a publish whose data never reached
                        // the flash — must not be read as "nothing here" and then written
                        // back durably as exactly that.
                        //
                        // ★ 117차: staying OFF was a dead end — this arm re-ran on every launch
                        // (`retry` is gated on `!unreadable`, a reset only on a slot change) and
                        // the player had no way out of it inside the app. So the damaged bytes
                        // are QUARANTINED beside the file — never overwritten, which is the one
                        // thing 104차-B was protecting — and the slot starts over with empty
                        // boxes, loudly. If even the rename fails, the old answer stands: OFF.
                        let quarantine = p.with_file_name(format!(
                            "{}.damaged-{}",
                            p.file_name().and_then(|n| n.to_str()).unwrap_or("slot"),
                            crate::emulator::now_ms()
                        ));
                        match std::fs::rename(p, &quarantine) {
                            Ok(()) => {
                                log::error!(
                                    "bagbox: {} has no `{}` header ({} byte(s)) — damaged; moved to {} and this slot starts with EMPTY boxes",
                                    p.display(),
                                    bagbox::FILE_HEADER,
                                    text.len(),
                                    quarantine.display()
                                );
                                let _ = proxy.send_event(UserEvent::Notice("가방 파일이 손상되어 빈 가방으로 시작합니다"));
                                Boxes::default()
                            }
                            Err(e) => {
                                log::error!(
                                    "bagbox: {} has no `{}` header ({} byte(s)) — damaged, and it could not be moved aside ({e}); boxes stay OFF for this slot",
                                    p.display(),
                                    bagbox::FILE_HEADER,
                                    text.len()
                                );
                                self.path = None;
                                self.unreadable = true;
                                Boxes::default()
                            }
                        }
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Boxes::default(),
                Err(e) => {
                    // ⛔ Unreadable is not empty: refuse to arm rather than start a fresh
                    // box over one that may exist (the same rule the vault follows).
                    log::error!("bagbox: cannot read {}: {e} — boxes stay OFF for this slot", p.display());
                    self.path = None;
                    self.unreadable = true;
                    Boxes::default()
                }
            },
            None => Boxes::default(),
        };
        self.dirty = false;
        if let Some(s) = slot
            && self.path.is_some()
        {
            let n: Vec<String> = Kind::ALL.iter().map(|k| format!("{} {}", k.tag(), self.boxes.stacks(*k).len())).collect();
            log::warn!(
                "bagbox: slot {s} boxes loaded — {} (file {})",
                n.join(" · "),
                self.path.as_ref().map_or("none".into(), |p| p.display().to_string())
            );
        }
    }

    /// Drain what the intake hooks took since the last call: box, file, notice queue.
    ///
    /// Called every frame from [`Self::frame`] and again from [`Self::job_frame`] while a
    /// use is in `WaitResult` (117차): the worker's step loop runs tick → `job_frame` with
    /// no `frame` in between, so an 오브원석 whose use and whose orb give land in the SAME
    /// tick would otherwise have `WaitResult` close the job before `absorbed_since_use` was
    /// ever set — and the 「입수」 toast was lost. An empty ring costs one atomic read.
    /// ★ 165차: and from [`Self::publish_knobs`] / the head of `poll` and `grant_boxed` —
    /// anything that is about to change a box or republish what the hook reads.
    fn drain_intake(&mut self, emulator: &mut dyn Emulator) {
        // Each entry was accepted against `BAGBOX_ROOM` and the item's `BAGBOX_MERGE`, so it
        // fits unless the file changed under us; a leftover is logged and counted, never
        // dropped silently. An entry may carry up to 255 units (the hook folds an offer into
        // the newest entry of the same item — 165차).
        let mut fresh = bagbox::FreshIntake::default();
        while let Some((cat, idx, n, src)) = wie_util::stats::bagbox_pop() {
            let Some(kind) = bagbox::kind_of(cat) else { continue };
            let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
            // ★ A quest script's hand-in: the hook already told the guest the items are
            // gone (it decremented the MIRROR and skipped the bag walk), so this is the
            // host applying a decision, not making one.
            if src & wie_util::stats::BAGBOX_SRC_MASK == wie_util::stats::BAGBOX_SRC_TAKE {
                let took = self.boxes.spend_units(kind, cat, idx, u32::from(n));
                // ★★★ 107차 — AND IT IS A LOAN, exactly like a materialise.
                //
                // 제보 2026-09-19: *"특수조합으로 A아이템을 재료로 써서 B아이템을 만들고
                // 저장안하고 나갔다오면 레시피는 사라져있고 A아이템이 남아있어"*. The
                // asymmetry was this branch: material A rides `mat_frame`, which pairs its
                // `spend_units` with a `loan.add` (97차) so the FILE keeps showing the
                // units until a GameSave confirms the guest's side — but the take path
                // debited the box durably on its own.
                //
                // What the hook consumed lives ONLY in unsaved guest memory (the crafted
                // item, the quest step). Leave without saving and the guest rolls back
                // while the box does not: the recipe is gone from both sides. A loan makes
                // the two agree — the file shows the units until the save that makes the
                // guest's half real, and `gamesave_commit` promotes the debit then.
                //
                // ⛔ `took`, not `n`: only what the box actually gave up is owed back. The
                // mismatch already has its own error line below.
                //
                // ★★ 165차 — EXCEPT what the same guest call gave a moment ago. A scene script
                // that gives a box item and takes it back (리파이너의 유적 7's boss door and its
                // 그리폰의피리) has both entries in this one drain; the give is filed for good
                // above and a loan here would hand the unit back on the next reload — once per
                // unsaved boss attempt. Those units cancel (`bagbox::FreshIntake`).
                let cancelled = fresh.cancel(cat, idx, took);
                self.loan.add(kind, cat, idx, took - cancelled);
                self.mark_dirty();
                log::warn!(
                    "bagbox: a quest took {name} x{n} out of the {} (spent {took}, loaned until the next save{})",
                    kind.title(),
                    if cancelled > 0 {
                        format!(" — {cancelled} of them were given in the same guest call, so that much cancels instead")
                    } else {
                        String::new()
                    }
                );
                if took < u32::from(n) {
                    log::error!("bagbox: the {} only had {took} of {name} — the mirror and the box disagree", kind.title());
                }
                continue;
            }
            let left = self.boxes.add(kind, cat, idx, u32::from(n));
            fresh.add(cat, idx, u32::from(n) - left);
            self.mark_dirty();
            log::warn!(
                "bagbox: {} took {name} x{n} (cat {cat:#x} idx {idx:#x}, source {src}) — {} stack(s)",
                kind.title(),
                self.boxes.stacks(kind).len()
            );
            if matches!(
                self.job,
                Some(UseJob {
                    phase: UsePhase::WaitResult { .. },
                    ..
                })
            ) {
                self.absorbed_since_use = Some(name.to_string());
            }
            if left > 0 {
                self.overflow += left;
                log::error!(
                    "bagbox: {} could not fit {left} of {name} — the hook's room and the box disagree",
                    kind.title()
                );
            }
            // ★★★ 103차: the notice the game would have shown for this acquisition. The
            //   hook decided (only it can see `giveItem`'s announce argument); the host
            //   appends the name to the game's own floating queue, so an absorbed drop
            //   reads exactly like the equipment drop beside it.
            // ★ 165차: one entry can now carry several announced units (the hook folds an
            //   offer into the newest entry of the same item) — the game queues one name per
            //   unit until its five-line queue is full, and so does this.
            if src & wie_util::stats::BAGBOX_ANNOUNCE != 0 {
                for _ in 0..n.min(bagbox::NOTICE_SLOTS as u8) {
                    if !self.announce(emulator, cat, idx) {
                        break;
                    }
                }
            }
        }
    }

    /// Every frame: follow the pin, drain what the hooks took, persist, republish the
    /// knobs, keep the 열기 popup relabelled, put a dialog up when asked, and step the
    /// materialise job.
    pub fn frame(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        frame: u64,
        pages: &mut quickpage::Pages,
        window: &mut quickpage::Window,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        self.sync_slot(data_dir, frame, proxy);

        // ★ 142차-C: `wie_bagbox_add.txt` — N units straight into a box, for the slot it names,
        //   once that slot's boxes are armed (`config::BOX_ADD_TRIGGER`). One stat a second
        //   while armed; applied through `grant_boxed` (durable at once), then the file goes.
        if frame % 60 == 17
            && self.armed
            && let Some(slot) = self.slot
            && let Some(req) = crate::config::read_box_add()
            && req.slot == slot
        {
            let mut put = 0u32;
            let mut refused = 0u32;
            for &(cat, idx, n) in &req.rows {
                let left = self.grant_boxed(emulator, cat, idx, n);
                put += n - left;
                refused += left;
            }
            crate::config::clear_box_add();
            log::warn!(
                "bagbox add: {put} unit(s) into slot {slot}'s boxes from {}{} — file removed",
                crate::config::BOX_ADD_TRIGGER,
                if refused > 0 { format!(" ({refused} did not fit or no box takes them)") } else { String::new() }
            );
            if put > 0 {
                let _ = proxy.send_event(UserEvent::NoticeText(format!("요청한 아이템 {put}개를 가방 상자에 넣었습니다")));
            }
        }

        // ★ A GameSave for this slot has committed: the materials materialised into the bag
        //   are durable now, so the loan may lift and the box file promote to the real
        //   debited state. Only for OUR slot — a redirect could land the write elsewhere.
        //   `last_gamesave_seq` is refreshed EVERY frame so a GameSave that happened while
        //   the loan was empty (before this crafting session) cannot later be read as "a
        //   save landed for the loan" — only a commit since the previous frame counts.
        //   ★★★ 100차-B: the same commit EMPTIES THE ABSORBED LEDGER — the bag on disk is
        //   now the swept one, so no cell on it can come back from a reload. ⛔ Unless no
        //   sweep has read the bag since the load (`ledger_live`): then the commit wrote
        //   the UNSWEPT bag (the online `04 3d` ack's saveGame beating the migration to
        //   a quiet frame is exactly that), the disk still holds the stale cells, and the
        //   ledger has to survive to the sweep that finally reads them.
        {
            let (seq, slot) = crate::gamesave_commit();
            if seq != self.last_gamesave_seq && Some(slot) == self.slot {
                let loan = !self.loan.is_empty();
                let ledger = !self.ledger_live && !self.boxes.absorbed().is_empty();
                if loan || ledger {
                    if loan {
                        self.loan.clear();
                        log::info!("bagbox: a GameSave for slot {slot} committed — the materialise loan is durable now");
                    }
                    if ledger {
                        let n = self.boxes.absorbed().len();
                        self.boxes.clear_absorbed();
                        log::info!("bagbox: a GameSave for slot {slot} committed the swept bag — the absorbed ledger ({n} cell(s)) is cleared");
                    }
                    self.mark_dirty();
                    self.flush();
                }
            }
            self.last_gamesave_seq = seq;
        }

        self.drain_intake(emulator);
        // ★ 129차 (128차 code review ⑥): every fifth frame, not every frame — the knob
        //   outlives a job on purpose and may sit armed through a whole GAME OVER prompt,
        //   and the hook checks the cell's identity itself; `stage_unit` runs the same
        //   check inline before it refuses, so a stale knob never costs a use.
        if frame % 5 == 1 {
            self.restage_watch(emulator);
        }
        // ★ 104차-B: a `flush_fast` is a transient — once the use job it served is over,
        //   the same bytes go down again WITH the fsyncs (see `flush_fast`). `job_pending`
        //   is two field reads, so this costs nothing while nothing is owed.
        //   ★ 117차: "over" includes WAITING ON THE PLAYER. The 116차 job lives while the
        //   game's 예/아니오 is on screen — up to `USE_ANSWER_MAX_MS` — and the wait the speed
        //   was bought for (the give landing) is long behind by then; ten minutes of a box
        //   file that can read back empty is not a transient.
        //   ★ 129차: and a hotkey use whose game box the player is still answering
        //   (`WaitField { asked }`) lives the same ten minutes.
        let waiting_on_player = matches!(
            self.job,
            Some(UseJob {
                phase: UsePhase::WaitGameAnswer { .. } | UsePhase::WaitField { asked: true, .. },
                ..
            })
        );
        if self.durable_pending && (!self.job_pending() || waiting_on_player) {
            self.dirty = true;
        }
        self.flush();
        self.publish_mirror();

        // Arm only with a pin, a writable file and a LOADED character. The warehouse
        // screens count (app state 3, 66차-C): a `05 04` withdraw is exactly the route
        // the player named, and `04 3d` re-pins the slot before any of them (71차-M).
        let loaded = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::resolve_char(&read).is_some()
        };
        let armed = self.slot.is_some() && self.path.is_some() && loaded;
        if armed != self.armed {
            log::warn!("bagbox: intake {} (slot {:?})", if armed { "ARMED" } else { "off" }, self.slot);
            self.armed = armed;
        }
        let mask = if armed { bagbox::accept_mask(&ENABLED) } else { 0 };
        wie_util::stats::set_bagbox_accept(mask, bagbox::keep_idx_mask());
        self.publish_room();

        // ★★★ 100차: a record was just loaded — migrate the bag on the first quiet frame.
        //   `sync_slot` above has already switched to the loaded slot's box file (the pin
        //   moved on the same load), so the sweep below reads and writes the right ledger.
        if crate::vault::take_load_edge() {
            // ★★★ 108차 — AND THE LOAN IS FOLDED BACK INTO THE BOX HERE.
            //
            // 제보 2026-09-19: *"특수조합중에 메인메뉴를 몇번갔다오니까 재료가방에 재료가
            // 없어졌어"*.
            //
            // A loan says *"these units left the box for a bag no GameSave has confirmed
            // yet"* (97차), and the FILE already shows them, because `flush` writes
            // `boxes + loan`. A record LOAD replaces that bag wholesale with the one on
            // disk — and a non-empty loan is itself the proof that no save captured the
            // units, so the bag that just arrived cannot hold them. From this frame on
            // they live in the BOX and nowhere else, and memory has to say so.
            //
            // ⛔ WITHOUT THIS the in-memory box stays DEBITED while the file says
            // otherwise. The line below already knew the shape of the problem — a reload
            // of the SAME slot does not pass `sync_slot`'s reload arm, because the pin
            // only moves for a DIFFERENT slot — but it only re-armed the ledger. So a
            // 메인메뉴 → 같은 슬롯 round trip left `self.boxes` empty of the materials:
            // the box WINDOW reads `self.boxes` and showed nothing, the next materialise
            // planned from an empty box, and the first GameSave to commit cleared the
            // loan and made the debit PERMANENT. It compounds with every round trip,
            // which is why the report says 몇번.
            //
            // ★ The file does not change: it was `boxes + loan` and becomes `boxes'` with
            // an empty loan, which is the same content — so this cannot duplicate.
            // ★ On a slot CHANGE nothing happens here: `sync_slot` ran first, cleared the
            // loan and re-read the file, so the loan is already empty.
            if !self.loan.is_empty() {
                self.boxes = self.loan.overlay(&self.boxes);
                self.loan.clear();
                self.mark_dirty();
                self.flush();
                log::warn!(
                    "bagbox: a record load replaced the bag — the outstanding loan is back in the boxes (orb {} · mat {} · use {})",
                    self.boxes.stacks(Kind::Orb).len(),
                    self.boxes.stacks(Kind::Material).len(),
                    self.boxes.stacks(Kind::Consumable).len()
                );
            }
            self.migrate_pending = Some(frame);
            // A reload of the SAME slot does not pass through `sync_slot`'s reload arm;
            // the bag is from disk again all the same.
            self.ledger_live = true;
        }
        if let Some(since) = self.migrate_pending {
            if frame.saturating_sub(since) > MIGRATE_WAIT_FRAMES {
                log::info!("bagbox: the load-edge migration found no quiet frame in {MIGRATE_WAIT_FRAMES} — the ordinary poll will take it");
                self.migrate_pending = None;
            } else if self.armed && self.dialog.is_none() && self.job.is_none() && self.mat.is_none() && self.quiet(emulator) {
                self.migrate_pending = None;
                log::warn!(
                    "bagbox: load edge — migrating the bag now (frame {frame}, {} frame(s) after the load)",
                    frame.saturating_sub(since)
                );
                self.sweep(emulator, data_dir, pages, window, proxy);
                // ⛔ `grant` is a GUEST CALL (`giveItem`) and stays a field-only act — a host
                //   guest call on the slot list wedged 게임시작 once (72차-D). `pin` is record
                //   swaps in memory and is fine anywhere the sweep is.
                if self.in_field(emulator) {
                    self.grant(emulator, frame);
                }
                self.pin(emulator);
            }
        }

        // The 열기 popup: same cadence as the 오브결합 row — the count byte is shared with
        // other windows, so the write is re-applied every frame and is empty once ours.
        let popup = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bagbox_popup(&read)
        };
        if let Some(p) = popup {
            let writes = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::bagbox_popup_writes(&read, &p)
            };
            let n = writes.len();
            for (a, bytes) in writes {
                if !emulator.write_memory(a, &bytes) {
                    log::warn!("bagbox: popup write failed at {a:#x}");
                }
            }
            if n > 0 {
                log::info!("bagbox: popup on the {} cell relabelled to 열기", p.kind.title());
            }
        }

        // A 열기 row was taken, or a verb changed the list: put the window up.
        if let Some((screen, kind, keep_view)) = self.open_request.take() {
            self.open_window(emulator, data_dir, screen, kind, keep_view, proxy);
        }

        // ★ 96차: a host 올리기 / 내리기 waiting for its count. The picker is the
        // warehouse's own (`network::host_qty_ask`), answered on the next frame here.
        if matches!(self.dialog, Some(Dialog::Qty { .. }))
            && let Some(reply) = crate::network::host_qty_reply()
            && let Some(Dialog::Qty { kind, at, verb, list }) = self.dialog.take()
        {
            match reply {
                crate::network::QtyReply::Take(n) => self.finish_move(emulator, data_dir, kind, at, verb, n, list, proxy),
                // CLR: nothing has moved, and the list is still on screen as it was.
                crate::network::QtyReply::Cancel => self.dialog = Some(Dialog::Window { kind, list }),
            }
        }

        self.mat_frame(emulator);
        // ★ 118차-B: every frame, not the poll — the poll is gated on a quiet field, and a
        //   dying player is exactly when it must run.
        self.revive_supply(emulator, frame);
        self.revive_frame(emulator, frame);

        self.job_frame(emulator, pages, window, proxy);
        self.pin_when_landed(emulator, frame);
        if frame % 5 == 3 {
            self.watch_cells(emulator, frame);
        }
        // ★ 165차: the load-edge migration, a 내리기, a refund above all changed a box after
        //   the drain-and-publish at the top; the next tick's hook must see the boxes as
        //   they are now. Cheap when nothing changed (`mirror_dirty`, an empty ring).
        self.publish_knobs(emulator);
    }

    /// 127차: the restage knob outlives the job that armed it on purpose — a unit the game
    /// has not consumed yet stays in the cell with the knob up, so a late removal (the
    /// hotkey's tick) is still answered by the hook and the sweep still finds a plain
    /// consumable. This DISARMS it once the cell is something else: the bag-item record
    /// (the hook or the host restored it), an empty cell (swept), another item, an
    /// unreadable bag, or the shell (the world is gone and the address belongs to nothing).
    /// Not while a job or a revive holds the stage — those settle their own cell.
    /// ★ 129차: no longer the only place — the sweep restores and disarms a leftover's cell
    /// itself ([`Self::sweep`]) and `stage_unit` reclaims or disarms a stale knob inline
    /// ([`Self::reclaim_stale_stage`]); a 귀환 window's 예 is now waited for by the job
    /// (`WaitField { asked }`), not left to a late hook answer.
    fn restage_watch(&mut self, emulator: &mut dyn Emulator) {
        if self.stage_held() {
            return;
        }
        self.disarm_stale_knob(emulator);
    }

    /// Is the armed restage knob a live job's or a live revive supply's? Those settle
    /// their own cell; nobody else may touch it.
    fn stage_held(&self) -> bool {
        self.job.is_some_and(|j| j.staged.is_some()) || self.revive.is_some_and(|r| r.staged.is_some())
    }

    /// The knob's cell as it reads now, for a knob nobody holds: `Some((count))` when it
    /// still holds the staged item (the game has not consumed it — a `JobEnd::Unknown`
    /// leftover), `None` when it is anything else — and then the knob is DISARMED here.
    /// The shell counts as "anything else": the world is gone and the address is nobody's.
    fn disarm_stale_knob(&mut self, emulator: &mut dyn Emulator) -> Option<u16> {
        let k = wie_util::stats::bagbox_restage()?;
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        let shell = items::shell_state(&read);
        let cell = (!shell).then(|| items::orb_cell(&read, k.tab, k.slot)).flatten();
        if let Some(c) = cell.as_ref().filter(|c| c.cat == u16::from(k.cat) && c.idx == u16::from(k.idx)) {
            return Some(c.count);
        }
        wie_util::stats::clear_bagbox_restage();
        log::warn!(
            "bagbox: restage knob disarmed — bag {},{} no longer holds the staged cat {:#x} idx {:#x} (shell {shell}, cell {})",
            k.tab + 1,
            k.slot + 1,
            k.cat,
            k.idx,
            cell.map_or("unreadable/empty".to_string(), |c| format!("cat {:#x} idx {:#x} x{}", c.cat, c.idx, c.count))
        );
        None
    }

    /// ★★★ 129차 (128차 code review ③): a staged unit nobody holds — a job that ended
    /// `Unknown` (a potion at full HP, a refused hotkey) left it in the cell with the knob
    /// armed for the sweep to reclaim — is reclaimed HERE when something needs the cell
    /// before the sweep gets to it: the bag-item record goes back over the cell, the knob
    /// is disarmed and the box gets the unit(s) back. Exactly what `end_job(Unused)` does
    /// for a live job, and what the sweep does for a leftover, only sooner.
    ///
    /// Why sooner matters: `revive_supply` runs while the player is DYING, when the poll
    /// (and its sweep) is deliberately off — so a leftover from the last second of the
    /// fight blocked `stage_unit` with "already holds a staged", and the game's revive
    /// check found no 부활의서: the player lost the revive. A plate pressed twice inside a
    /// second met the same refusal (a toast and a refund instead of a use).
    ///
    /// `true` when the cell is free to stage over afterwards (reclaimed, disarmed, or
    /// nothing was armed); `false` only when the box will not take the unit back (a full
    /// box — the cell is left exactly as it is, for the sweep, which fails the same way
    /// and leaves it for the next poll).
    fn reclaim_stale_stage(&mut self, emulator: &mut dyn Emulator) -> bool {
        let Some(k) = wie_util::stats::bagbox_restage() else { return true };
        let Some(count) = self.disarm_stale_knob(emulator) else { return true };
        let (cat, idx) = (u16::from(k.cat), u16::from(k.idx));
        let Some(kind) = bagbox::kind_of(cat) else {
            // Not a box category any more (a table change between sessions): the record
            // is the game's, and so is the unit. Nothing of ours to put back.
            wie_util::stats::clear_bagbox_restage();
            return true;
        };
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let st = items::BagStage {
            tab: k.tab,
            slot: k.slot,
            rec: k.rec,
            snapshot: k.bytes,
        };
        // The restore is planned FIRST: it re-checks that the record at the knob's own
        // address is this bag's (`bag_stage_cell` — a moved grid is `None`) and still the
        // staged item. ⛔ Never credit the box for a unit this cannot write back: the cell
        // would keep the unit and the sweep would credit it a second time.
        let writes = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_stage_restore_writes(&read, &st, cat, idx)
        };
        let Some(writes) = writes else {
            wie_util::stats::clear_bagbox_restage();
            log::warn!(
                "bagbox: the restage knob names bag {},{} for {name} but the record there is not this bag's — disarmed, the sweep owns the cell",
                k.tab + 1,
                k.slot + 1
            );
            return true;
        };
        // ⛔ Box first (its file is the durable side), cell second — the same order as
        //   `end_job(Unused)`. A box that will not take it leaves the cell alone.
        let left = self.boxes.add(kind, cat, idx, u32::from(count));
        if left > 0 {
            let _ = self.boxes.spend_units(kind, cat, idx, u32::from(count) - left);
            log::warn!(
                "bagbox: a stale staged {name} x{count} sits over bag {},{} but the {} has no room for it — leaving it for the sweep",
                k.tab + 1,
                k.slot + 1,
                kind.title()
            );
            return false;
        }
        // A unit the revive supply LOANED (118차-B: debited with `loan.add`) — or a use
        // job's (158차 D1: its debit is a loan too) — is the box's again outright: the same
        // `loan.sub` the sweep does for a reclaimed leftover.
        self.loan.sub(kind, cat, idx, u32::from(count));
        self.mark_dirty();
        self.flush();
        wie_util::stats::clear_bagbox_restage();
        let mut restored = true;
        for (a, bytes) in &writes {
            if !emulator.write_memory(*a, bytes) {
                log::error!("bagbox: restore write failed at {a:#x} — bag {},{} may be half-written", st.tab + 1, st.slot + 1);
                restored = false;
            }
        }
        log::warn!(
            "bagbox: a stale staged {name} x{count} over bag {},{} was reclaimed before the sweep — the unit is in the {} again, the {} cell {}",
            k.tab + 1,
            k.slot + 1,
            kind.title(),
            kind.title(),
            if restored { "is back in its place" } else { "could NOT be written back (the grant will give it again)" }
        );
        true
    }

    /// ★★ THE VANISH DIAGNOSTIC (TODO `CP` ⓑ, 2026-09-16). The only trace the 소비가방
    /// cell left when it disappeared was the grant giving it again — nothing said WHAT
    /// removed it. So the three cells are watched, and the frame one goes missing logs
    /// every term a removal could hinge on: the popup and its tag/rows/selected row, the
    /// inventory cursor and the cell under it, the window and app state, the use job and
    /// the grant in flight. One line, at the moment it happens — not a build later.
    ///
    /// ⛔ A cell that was never seen is not a vanish (a new character has none yet), and
    /// an unreadable bag resets the watch rather than reporting three vanishes.
    fn watch_cells(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        // ⛔ Not in the shell (app state 3): the creation screens reset the character's
        // memory under a still-resolvable CHAR, and that empty bag is not three vanishes.
        if !self.armed || items::shell_state(&read) {
            self.cells_seen = None;
            return;
        }
        // ⛔ 127차: while a unit is staged over a bag-item cell that cell reads as the
        // unit, on purpose — not a vanish. The watch simply waits for the knob to clear.
        if wie_util::stats::bagbox_restage().is_some() {
            return;
        }
        let Some(cells) = items::bag_cells(&read) else {
            self.cells_seen = None;
            return;
        };
        let now = cells
            .iter()
            .filter_map(|c| Kind::from_bag_item(c.cat, c.idx))
            .fold(0u8, |m, k| m | (1 << k.index()));
        if let Some(before) = self.cells_seen {
            for k in Kind::ALL {
                let bit = 1 << k.index();
                if before & bit != 0 && now & bit == 0 {
                    log::warn!(
                        "bagbox: the {} cell VANISHED from the bag (frame {frame}) — {} | job={:?} mat={} grant_wait={:?} dialog={}",
                        k.title(),
                        items::bagbox_vanish_report(&read),
                        self.job.map(|j| (j.target, j.phase)),
                        self.mat.is_some(),
                        self.grant_wait,
                        self.dialog.is_some()
                    );
                }
            }
        }
        self.cells_seen = Some(now);
    }

    // ── materialise: stranded 조합서 back to the bag for good (125차-B; smith no longer uses this) ──

    /// Queue the push-back. `window` = None for eviction (the only remaining asker).
    /// `false` when nothing was queued (another job is running, nothing to push).
    fn start_materialise(&mut self, mut queue: Vec<(Kind, Stack)>, window: Option<u8>) -> bool {
        if queue.is_empty() || self.mat.is_some() || self.job.is_some() || !self.armed {
            return false;
        }
        let boxes: Vec<&str> = Kind::ALL
            .into_iter()
            .filter(|k| queue.iter().any(|(kind, _)| kind == k))
            .map(Kind::title)
            .collect();
        // The queue is popped from the back, so the highest-priority stack goes last.
        queue.reverse();
        match window {
            Some(w) => log::warn!(
                "bagbox: materialising {} stack(s) of the {} into the bag (window {w})",
                queue.len(),
                boxes.join("·")
            ),
            None => log::warn!(
                "bagbox: moving {} stranded stack(s) out of the {} back to the bag for good — not box items any more (125차-B)",
                queue.len(),
                boxes.join("·")
            ),
        }
        self.mat = Some(MatJob { queue, batch: None, window });
        true
    }

    /// ★★★ 125차-B: stacks an old file keeps in a box their category left — a 조합서 in the
    /// 재료가방 — go back to the bag, one stack per frame, through the game's own `giveItem`
    /// (the intake hook no longer accepts the category, so the game places it itself). The
    /// same machinery and the same LOAN as a materialise: the box is debited when the units
    /// are seen in the bag, and the file keeps them until a GameSave captures the bag, so a
    /// quit without saving puts them back in the box on the next load (108차). Never a loss.
    ///
    /// Field polls only (a guest call), and only with a free bag cell; a bag that would not
    /// take a stack is tried again `EVICT_RETRY_FRAMES` later.
    fn evict_stranded(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        if self.evict_backoff {
            self.evict_backoff = false;
            self.evict_retry_at = frame + EVICT_RETRY_FRAMES;
            return;
        }
        if frame < self.evict_retry_at || self.mat.is_some() || self.job.is_some() {
            return;
        }
        let stranded = self.boxes.stranded();
        if stranded.is_empty() {
            return;
        }
        let room = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_has_room(&read)
        };
        if room != Some(true) {
            self.evict_retry_at = frame + EVICT_RETRY_FRAMES;
            log::warn!(
                "bagbox: {} stranded stack(s) wait for a free bag cell (room {room:?}) — retrying in {EVICT_RETRY_FRAMES} frames",
                stranded.len()
            );
            return;
        }
        self.start_materialise(stranded, None);
    }

    /// One batch per frame: send a stack through the game's own `giveItem` with the
    /// bypass armed, then debit the box by what actually landed.
    ///
    /// ⛔ The box is debited AFTER the units are seen in the bag, never before: a failure
    /// in between then duplicates (the sweep folds it back) instead of losing the stack.
    fn mat_frame(&mut self, emulator: &mut dyn Emulator) {
        let Some(mut job) = self.mat.take() else { return };
        let now = crate::emulator::now_ms();
        if let Some(b) = job.batch {
            let units = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::bag_units_of(&read, b.cat, b.idx)
            };
            let settled = match units {
                Some(after) if after > b.before => Some(after),
                _ if now >= b.deadline => units,
                _ => None,
            };
            let Some(after) = settled else {
                self.mat = Some(job);
                return;
            };
            wie_util::stats::set_bagbox_bypass(0, 0, 0);
            let landed = after.saturating_sub(b.before).min(u32::from(b.sent));
            let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, b.cat, b.idx);
            if landed > 0 {
                let took = self.boxes.spend_units(b.kind, b.cat, b.idx, landed);
                // ★ A materialise is not durable until a GameSave captures the bag, so the
                //   box FILE keeps these units (the flush overlays the loan) — see
                //   `bagbox::Loan`. Eviction is the only remaining materialise.
                self.loan.add(b.kind, b.cat, b.idx, landed);
                // ★★★ 106차 — AND THE ABSORBED LEDGER DIES HERE. Its entries say "the box
                //   already holds this cell", which this very line has just made FALSE for
                //   these units: they left the box for the bag. `findSlot 0x529a0` puts
                //   them in the first free cell — normally the one the last sweep emptied,
                //   with the whole stack — so the new cell matches a ledger entry byte for
                //   byte, and a later sweep would delete it and add nothing
                //   (`bagbox::sweep_plan`). That is the 2026-09-19 report: 일반조합 → 실패
                //   → 메인메뉴 → 재진입 → 재료가방이 통째로 비었다.
                //   ⛔ Do not narrow this to the (cat, idx) being moved. The ledger is a
                //   statement about the box as a whole at the moment of a sweep, and a
                //   materialise invalidates that moment; the cost of dropping it is at
                //   worst a duplicate on a stale-save reload, which is the direction 100차
                //   chose. `sweep_plan` verifies the claim too — belt and braces, because
                //   this is the site that must never be missed again.
                if !self.boxes.absorbed().is_empty() {
                    let n = self.boxes.absorbed().len();
                    self.boxes.clear_absorbed();
                    log::info!("bagbox: a materialise left the box — the absorbed ledger ({n} cell(s)) no longer describes it");
                }
                self.mark_dirty();
                self.flush();
                if job.window.is_some() {
                    log::warn!(
                        "bagbox: {name} x{landed} of {} is in the bag now ({} debited {took}, loaned)",
                        b.sent,
                        b.kind.title()
                    );
                } else {
                    log::warn!(
                        "bagbox: {name} x{landed} of {} is back in the bag for good — not a {} item any more ({} debited {took}, loaned until the next save)",
                        b.sent,
                        b.kind.title(),
                        b.kind.title()
                    );
                }
            }
            job.batch = None;
            if landed < u32::from(b.sent) {
                // The bag would not take the rest, and the next stack needs a cell too.
                log::info!(
                    "bagbox: the bag took {landed} of {} {name} — the rest stays in the {}",
                    b.sent,
                    b.kind.title()
                );
                job.queue.clear();
                if job.window.is_none() {
                    self.evict_backoff = true;
                }
            }
            self.mat = (!job.queue.is_empty()).then_some(job);
            return;
        }
        // Between batches: stop if the window that asked has gone — or, for an eviction
        // (no window), if the field has: it is a guest call, and those are field-only.
        {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            match job.window {
                Some(w) => {
                    if items::window_id(&read) != Some(w) {
                        log::info!("bagbox: window {w} closed — {} stack(s) stay in their boxes", job.queue.len());
                        return;
                    }
                }
                None => {
                    if !items::in_field_state(&read) {
                        log::info!(
                            "bagbox: the field was left — {} stranded stack(s) wait for the next field poll",
                            job.queue.len()
                        );
                        return;
                    }
                }
            }
        }
        let Some(f) = crate::guest_build::give_item_fn() else { return };
        let Some((kind, s)) = job.queue.pop() else { return };
        let before = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_units_of(&read, s.cat, s.idx)
        };
        let Some(before) = before else {
            // Unreadable this frame: put the stack back and try again on the next.
            job.queue.push((kind, s));
            self.mat = Some(job);
            return;
        };
        // The box must still hold what the plan named (a quest hand-in may have spent it).
        let n = u32::from(s.count)
            .min(self.boxes.units_of(kind, s.cat, s.idx))
            .min(u32::from(bagbox::STACK_MAX));
        if n == 0 {
            self.mat = (!job.queue.is_empty()).then_some(job);
            return;
        }
        wie_util::stats::set_bagbox_bypass(s.cat, s.idx, n as u8);
        for _ in 0..n {
            emulator.call_guest_function(f, &[u32::from(s.cat), u32::from(s.idx), u32::MAX, 0]);
        }
        job.batch = Some(MatBatch {
            kind,
            cat: s.cat,
            idx: s.idx,
            sent: n as u16,
            before,
            deadline: now + MAT_WAIT_MS,
        });
        self.mat = Some(job);
    }

    /// The OK key, before the guest sees it. `true` = it was 열기 on a bag-item cell: the
    /// popup is closed with the game's own CLR instead, and the window follows next frame.
    /// The caller must then NOT forward the OK.
    pub fn take_open(&mut self, emulator: &mut dyn Emulator) -> bool {
        let popup = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bagbox_popup(&read)
        };
        if let Some(p) = popup.filter(|p| p.ours) {
            log::info!("bagbox: 열기 taken on the {} cell", p.kind.title());
            emulator.handle_event(Event::Keydown(KeyCode::CLEAR));
            emulator.handle_event(Event::Keyup(KeyCode::CLEAR));
            // The 열기 popup only exists on the inventory page (`bagbox_popup`).
            self.open_request = Some((Screen::Inventory, p.kind, false));
            return true;
        }
        // ★★★ 96차-B — THE ONLINE SCREENS HAVE NO 열기 ROW TO TAKE, so the key itself is
        // taken (제보: *"패드 이용자들은 터치를 잘 이용 안 하기 때문에 OK 키로도 열기가
        // 가능해야 해"*). The reason it cannot be a relabel is measured, not assumed — see
        // [`items::bagbox_open_key`]: the online OK arm writes only the shared open and
        // count bytes and no labels at all. ⛔ Not while one of our own dialogs is up; the
        // UI swallows keys then, and a `PopupConfirm` that slipped through must not reopen.
        if self.dialog.is_none() {
            let online = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                // ★ 109차: the UI's pixel evidence rides along — the shop's OK needs it
                //   (판매 grid vs 구매 list), the online screens do not look at it.
                items::bagbox_open_key(&read, crate::cell_cursor_shown())
            };
            if let Some((screen, kind)) = online {
                log::info!(
                    "bagbox: OK on the {} cell ({screen:?}) -> opening the box, the key is swallowed",
                    kind.title()
                );
                self.open_request = Some((screen, kind, false));
                return true;
            }
        }
        false
    }

    /// The rows a window shows: the bag's own box, or — on the warehouse grid — the
    /// WAREHOUSE's box, which is the offline store's rows of that box's categories
    /// (96차). `None` = no warehouse session is live at all (since 99차 every session
    /// IS the in-app store, so this is a defensive answer, not a server case).
    fn listing(&self, data_dir: &Path, screen: Screen, kind: Kind) -> Option<Listing> {
        if !screen.is_warehouse() {
            return Some(Listing {
                screen,
                stacks: self.boxes.stacks(kind).to_vec(),
                keys: Vec::new(),
            });
        }
        if !crate::network::offline_session_live() {
            return None;
        }
        let cats: Vec<u8> = kind.cats().iter().map(|c| *c as u8).collect();
        let (rows, _) = crate::network::with_offline_store(data_dir, |st| (st.boxed_rows(&cats), false));
        let mut stacks = Vec::with_capacity(rows.len());
        let mut keys = Vec::with_capacity(rows.len());
        for r in rows {
            stacks.push(Stack {
                cat: u16::from(r[offline::ITEM_CAT_OFF]),
                idx: u16::from(r[offline::ITEM_IDX_OFF]),
                count: u16::from(offline::unit_count(&r)),
            });
            let mut k = [0u8; 8];
            k.copy_from_slice(&r[..8]);
            keys.push(k);
        }
        Some(Listing { screen, stacks, keys })
    }

    /// Is row `at` of `list` still what it was when the list was built? An intake since
    /// the window opened only APPENDS to a bag box, and only a spend removes, so `at`
    /// still names the same stack there; a warehouse row is found again by its key.
    fn row_holds(&self, data_dir: &Path, kind: Kind, list: &Listing, at: usize) -> bool {
        let Some(s) = list.stacks.get(at) else { return false };
        if !list.screen.is_warehouse() {
            return self
                .boxes
                .stacks(kind)
                .get(at)
                .is_some_and(|live| live.cat == s.cat && live.idx == s.idx && live.count > 0);
        }
        let Some(key) = list.keys.get(at).copied() else { return false };
        let (row, _) = crate::network::with_offline_store(data_dir, |st| (st.items.iter().find(|it| it[..8] == key).copied(), false));
        row.is_some_and(|r| u16::from(r[offline::ITEM_CAT_OFF]) == s.cat && u16::from(r[offline::ITEM_IDX_OFF]) == s.idx)
    }

    fn open_window(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        screen: Screen,
        kind: Kind,
        keep_view: bool,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        if !ENABLED.contains(&kind) {
            let _ = proxy.send_event(UserEvent::Notice(match kind {
                Kind::Material => "재료가방은 다음 단계에서 열립니다",
                Kind::Consumable => "소비가방은 다음 단계에서 열립니다",
                Kind::Orb => "오브가방",
            }));
            return;
        }
        if self.job.is_some() {
            let _ = proxy.send_event(UserEvent::Notice("아이템을 사용하는 중입니다"));
            return;
        }
        let Some(list) = self.listing(data_dir, screen, kind) else {
            // ★ 96차 kept this arm for a real server's warehouse (on the server, out of a
            // box window's reach). 99차 made every session the in-app store, so it can only
            // fire if the grid is up with no session at all — still say so rather than open
            // the BAG's box under the warehouse's name (the narrow reading the user refused).
            let _ = proxy.send_event(UserEvent::Notice("창고 세션이 없습니다"));
            log::warn!(
                "bagbox: {} refused on the warehouse grid — no in-app warehouse session is live",
                kind.title()
            );
            return;
        };
        let rows: Vec<crate::OrbRow> = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            list.stacks
                .iter()
                .map(|s| {
                    let (_, name, effect) = bagbox::name_of(crate::bagbox_names::NAMES, s.cat, s.idx);
                    let detail = if effect.is_empty() {
                        format!("x{}", s.count)
                    } else {
                        format!("x{}  {effect}", s.count)
                    };
                    crate::OrbRow {
                        name,
                        detail,
                        enabled: true,
                        notice: None,
                        icon: items::item_frame(&read, s.cat, s.idx),
                    }
                })
                .collect()
        };
        // ★ 2026-09-15, user: *"상단에 x종 x개 (3/48 칸) 이런식으로 되어있는거 3/48 만
        // 있으면돼"*. The two counts it used to carry are both on the rows already — one
        // per row, and each row prints its own `xN`.
        // ★ 96차: the WAREHOUSE's box is titled as such. ★★★ 120차: and its second number is
        // that box's OWN cap (`offline::WH_BOX_CAP`, 128) — the warehouse's boxes are
        // counted apart from its grid now, the same shape as the bag's `N/80`. (96차 had
        // printed the whole warehouse's fill here because the boxes shared the grid's
        // cells; they no longer do.)
        let (title, subtitle) = if screen.is_warehouse() {
            (format!("창고 {}", kind.title()), format!("{}/{}", list.stacks.len(), offline::WH_BOX_CAP))
        } else {
            (kind.title().to_string(), format!("{}/{}", list.stacks.len(), bagbox::CAP))
        };
        log::info!("bagbox: {title} window ({screen:?}) — {} stack(s)", list.stacks.len());
        self.last_screen = screen;
        // ★★★ 110차-D (제보: *"사용중에 한번씩 드랍박스가 이상한데 뜨는데?"*). A REFRESH
        //   may land while the player has already opened the next drop box on a row: the
        //   UI keeps that box up (an `OrbPick(Some)` does not withdraw it), so if the host
        //   flattened its own `Verbs`/`Confirm` to `Window` here, the tap on 사용하기 —
        //   drop-box row 0 — was read as WINDOW row 0 and opened a verb box on the first
        //   stack. Keep the verb state when the rows still line up; when they do not (a
        //   stack ran out and the indices shifted), take the box down with the refresh.
        let same_shape = |old: &Listing| old.screen == list.screen && old.stacks.len() == list.stacks.len();
        self.dialog = match self.dialog.take() {
            Some(Dialog::Verbs { kind: k, at, list: old }) if keep_view && k == kind && same_shape(&old) => Some(Dialog::Verbs {
                kind,
                at,
                list: list.clone(),
            }),
            Some(Dialog::Confirm { kind: k, at, list: old }) if keep_view && k == kind && same_shape(&old) => Some(Dialog::Confirm {
                kind,
                at,
                list: list.clone(),
            }),
            Some(Dialog::Qty {
                kind: k,
                at,
                verb,
                list: old,
            }) if keep_view && k == kind && same_shape(&old) => Some(Dialog::Qty {
                kind,
                at,
                verb,
                list: list.clone(),
            }),
            Some(Dialog::Verbs { .. } | Dialog::Confirm { .. }) => {
                let _ = proxy.send_event(UserEvent::OrbDrop(None));
                log::info!("bagbox: the rows changed under the drop box — taking it down with the refresh");
                Some(Dialog::Window { kind, list })
            }
            _ => Some(Dialog::Window { kind, list }),
        };
        // ★ `keep_open`: choosing a row does not take this window down. What follows is a
        // DROP BOX on that very row (a consumable) or the target picker (an orb), and the
        // list underneath is the thing the player is still reading.
        let pick = crate::OrbPick {
            title,
            subtitle,
            rows,
            ok_label: "선택",
            keep_open: true,
            keep_view,
        };
        let _ = proxy.send_event(UserEvent::OrbPick(Some(Box::new(pick))));
    }

    /// The verbs for one stack, as a DROP BOX on its row.
    ///
    /// ★★ 2026-09-14, user: *"소비가방은 선택하면 따로 화면전환으로 사용하기, 퀵슬롯등록이
    /// 뜨는게아니라 소비가방 창에서 아이템 포커스 -> OK로 선택하면 그 위치에 드래박스로(기존처럼)
    /// 사용하기, 퀵슬롯 등록을 넣으면 돼"*. This used to be a whole second `OrbPick`, which
    /// REPLACED the list — a screen change where the game's own inventory has a small box.
    /// ⛔ The box is anchored on `at`, so the list must not scroll while it is up. It
    /// cannot: the UI gives every key and every tap inside it to the box.
    fn open_verbs(&mut self, kind: Kind, at: usize, list: Listing, proxy: &EventLoopProxy<UserEvent>) {
        // ★ 115차: the GAME's own per-item rule — 부활의서 has no 사용하기, the cash tier and
        //   오브원석 no 퀵슬롯등록 (`bagbox::verbs_for`); the pick in `answer` maps through
        //   the same list, so the two can never disagree about which row is which.
        let Some(s) = list.stacks.get(at).copied() else { return };
        let rows: Vec<String> = bagbox::verbs_for(kind, list.screen, s.cat, s.idx)
            .iter()
            .map(|v| v.label().to_string())
            .collect();
        if rows.is_empty() {
            return;
        }
        self.dialog = Some(Dialog::Verbs { kind, at, list });
        let _ = proxy.send_event(UserEvent::OrbDrop(Some(Box::new(crate::OrbDrop { anchor: at, rows }))));
    }

    /// Ask before throwing a stack away, on the same row the box is anchored to.
    fn open_confirm(&mut self, kind: Kind, at: usize, list: Listing, proxy: &EventLoopProxy<UserEvent>) {
        self.dialog = Some(Dialog::Confirm { kind, at, list });
        let rows: Vec<String> = bagbox::CONFIRM_ROWS.iter().map(|s| (*s).to_string()).collect();
        let _ = proxy.send_event(UserEvent::OrbDrop(Some(Box::new(crate::OrbDrop { anchor: at, rows }))));
    }

    /// Throw the whole stack away (the player said 예).
    fn discard(&mut self, kind: Kind, at: usize, cat: u16, idx: u16, proxy: &EventLoopProxy<UserEvent>) {
        let Some(s) = self.boxes.stacks(kind).get(at).copied().filter(|s| s.cat == cat && s.idx == idx) else {
            log::warn!("bagbox: 버리기 refused — {} stack {at} is not {cat:#x}/{idx:#x} any more", kind.title());
            return;
        };
        let gone = self.boxes.spend_units(kind, cat, idx, u32::from(s.count));
        self.mark_dirty();
        self.flush();
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        log::warn!("bagbox: threw {name} x{gone} out of the {}", kind.title());
        let _ = proxy.send_event(UserEvent::Notice("버렸습니다"));
    }

    /// Throw a WAREHOUSE row away (the player said 예 in the warehouse's box) — the whole
    /// row, as the grid's own 버리기 does (`05 05`: "the player means the cell").
    fn discard_warehouse(&mut self, data_dir: &Path, kind: Kind, list: &Listing, at: usize, proxy: &EventLoopProxy<UserEvent>) {
        let (Some(s), Some(key)) = (list.stacks.get(at).copied(), list.keys.get(at).copied()) else {
            return;
        };
        let (gone, persisted) = crate::network::with_offline_store(data_dir, |st| {
            let taken = st
                .take_item(&key)
                .filter(|r| u16::from(r[offline::ITEM_CAT_OFF]) == s.cat && u16::from(r[offline::ITEM_IDX_OFF]) == s.idx);
            (taken.map(|r| offline::unit_count(&r)), taken.is_some())
        });
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, s.cat, s.idx);
        match gone {
            Some(n) => {
                log::warn!(
                    "bagbox: threw {name} x{n} out of the warehouse's {} (persisted = {persisted})",
                    kind.title()
                );
                let _ = proxy.send_event(UserEvent::Notice("버렸습니다"));
            }
            None => log::warn!("bagbox: warehouse 버리기 refused — row {at} of the {} is gone", kind.title()),
        }
    }

    /// The game-style message box for one row: its name and count, then the effect line
    /// wrapped to the box's width (the online screens' 상세보기).
    /// The 상세보기 box: the name, then the effect. ⛔ No count — the row already shows
    /// it, and the player asked for it gone (2026-09-16: *"상세보기에서 갯수 나올 필요없어"*).
    fn detail_text(cat: u16, idx: u16) -> String {
        let (_, name, effect) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let mut lines = vec![name.to_string()];
        lines.extend(crate::overlay::wrap_notice(effect).into_iter().take(crate::overlay::NOTICE_LINES - 1));
        lines.join("\n")
    }

    /// A 올리기 / 내리기 was chosen (96차): apply the game's own refusals, measure what
    /// fits, and either move one unit outright or ask how many in the warehouse's picker.
    #[allow(clippy::too_many_arguments)]
    fn begin_move(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        kind: Kind,
        at: usize,
        verb: Verb,
        list: Listing,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let Some(s) = list.stacks.get(at).copied() else {
            self.dialog = Some(Dialog::Window { kind, list });
            return;
        };
        let (cat, idx) = (s.cat, s.idx);
        let room: u32 = match verb {
            Verb::Deposit => {
                // ⛔ The client's own gates (`0x6f4c`..), in the client's own words.
                if let Some(why) = bagbox::deposit_refusal(crate::bagbox_names::NAMES, cat, idx) {
                    log::info!("bagbox: 올리기 refused by the game's rule for cat {cat:#x} idx {idx:#x}: {why}");
                    let _ = proxy.send_event(UserEvent::OrbNotice(why.to_string()));
                    self.dialog = Some(Dialog::Window { kind, list });
                    return;
                }
                let probe = offline::new_stack(cat as u8, idx as u8, 1, 0);
                let (room, _) = crate::network::with_offline_store(data_dir, |st| (st.room_for(&probe), false));
                if room == 0 {
                    // ★ 120차: the room is the warehouse's OWN box of this kind (128 stacks,
                    // apart from the grid — `offline::WH_BOX_CAP`), so name it: the grid
                    // may well look empty while this box is full.
                    let _ = proxy.send_event(UserEvent::OrbNotice(format!("창고 {}이 가득찼습니다.", kind.title())));
                    self.dialog = Some(Dialog::Window { kind, list });
                    return;
                }
                room
            }
            Verb::Withdraw => {
                let fits = self.boxes.fits(kind, cat, idx);
                if fits == 0 {
                    let _ = proxy.send_event(UserEvent::OrbNotice(format!("{}이 가득찼습니다.", kind.title())));
                    self.dialog = Some(Dialog::Window { kind, list });
                    return;
                }
                fits
            }
            // ★ 105차 판매. The only gate is the game's own: a row it prices at 0 is a row
            // it refuses to buy (`0x1e364`), and the host must not pay for it either.
            // There is no capacity question — gold is a `u32` and the box is the source —
            // so the ceiling is simply what the stack holds.
            Verb::Sell => {
                if let Some(why) = bagbox::sell_refusal(crate::bagbox_names::NAMES, cat, idx) {
                    log::info!("bagbox: 판매 refused for cat {cat:#x} idx {idx:#x}: {why}");
                    let _ = proxy.send_event(UserEvent::OrbNotice(why.to_string()));
                    self.dialog = Some(Dialog::Window { kind, list });
                    return;
                }
                u32::from(s.count)
            }
            _ => {
                self.dialog = Some(Dialog::Window { kind, list });
                return;
            }
        };
        let max = room.min(u32::from(s.count)).min(u32::from(bagbox::STACK_MAX)).max(1) as u8;
        if s.count <= 1 {
            self.finish_move(emulator, data_dir, kind, at, verb, 1, list, proxy);
            return;
        }
        // ★ The same picker the grid's own 올리기 / 내리기 use (68차), over the list. A
        // stack of one has nothing to choose; `held > 1` asks even when only one fits, so
        // a partial move is never silent (the grid's rule).
        // ⛔ `op` only labels the question in the log — nothing in the picker's drawing or
        // hit-testing reads it (grep: no `ask.op` outside this file and the engine). 판매
        // rides the 올리기 value because it moves in the same direction, OUT of the box.
        let op = if verb == Verb::Withdraw {
            offline::OP_WH_DOWNLOAD
        } else {
            offline::OP_WH_UPLOAD
        };
        crate::network::host_qty_ask(QtyAsk {
            op,
            max,
            min: 1,
            cat: cat as u8,
            idx: idx as u8,
        });
        log::info!("bagbox: asking how many for {verb:?} — up to {max} of cat {cat:#x} idx {idx:#x}");
        self.dialog = Some(Dialog::Qty { kind, at, verb, list });
    }

    /// Move `n` units (the count answered, or 1 for a stack of one) and republish the list.
    #[allow(clippy::too_many_arguments)]
    fn finish_move(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        kind: Kind,
        at: usize,
        verb: Verb,
        n: u8,
        list: Listing,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let screen = list.screen;
        if let Some(s) = list.stacks.get(at).copied() {
            match verb {
                Verb::Deposit => self.deposit_units(data_dir, kind, s.cat, s.idx, n, proxy),
                Verb::Withdraw => {
                    if let Some(key) = list.keys.get(at).copied() {
                        self.withdraw_units(data_dir, kind, key, s.cat, s.idx, n, proxy);
                    }
                }
                Verb::Sell => self.sell_units(emulator, kind, s.cat, s.idx, n, proxy),
                _ => {}
            }
        }
        // Whatever happened, the list comes back with the live counts, in place.
        self.open_request = Some((screen, kind, true));
    }

    /// 판매: `n` units out of the bag box, paid for in the player's own gold (105차).
    ///
    /// ★ THE PRICE AND THE PURSE ARE BOTH THE GAME'S. `bagbox::sell_price` is the shop's
    /// own `price >> 2`, and the credit lands on `CHAR+0xcc`, the field the shop's sell
    /// arm adds to and its buy arm subtracts from (`items::GOLD_OFF` carries the two
    /// disassembled sites). Nothing here invents a number.
    ///
    /// ⛔ READ THE PURSE BEFORE DEBITING THE BOX. If CHAR cannot be resolved the sale is
    /// refused with the box untouched — the alternative is spending the item and then
    /// discovering there is nowhere to put the money. And if the write itself fails after
    /// the debit, the units go BACK (`Boxes::add`): an item that vanished without paying
    /// is the one outcome this feature must never produce.
    fn sell_units(&mut self, emulator: &mut dyn Emulator, kind: Kind, cat: u16, idx: u16, n: u8, proxy: &EventLoopProxy<UserEvent>) {
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let unit = bagbox::sell_price(crate::bagbox_names::NAMES, cat, idx);
        if unit == 0 {
            log::warn!("bagbox: 판매 refused — the shop pays nothing for {name}");
            return;
        }
        let have = self.boxes.units_of(kind, cat, idx).min(u32::from(bagbox::STACK_MAX)) as u8;
        let n = n.min(have);
        if n == 0 {
            log::warn!("bagbox: 판매 refused — the {} no longer holds {name}", kind.title());
            return;
        }
        // The purse first, while nothing has been spent.
        let pay = unit.saturating_mul(u32::from(n));
        let paid = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::gold_add_writes(&read, pay)
        };
        let Some(((addr, bytes), balance)) = paid else {
            log::warn!(
                "bagbox: 판매 refused — CHAR is not readable, so {name} x{n} stays in the {}",
                kind.title()
            );
            let _ = proxy.send_event(UserEvent::Notice("판매실패"));
            return;
        };
        let before = self.boxes.clone();
        let gone = self.boxes.spend_units(kind, cat, idx, u32::from(n));
        if gone == 0 {
            log::warn!("bagbox: 판매 refused — {name} left the {} between the two reads", kind.title());
            return;
        }
        self.mark_dirty();
        self.flush();
        // ⛔ THE FILE IS THE DURABLE SIDE — no gold until it holds the debit (117차, the same
        //   check `reclaim_unit` makes). Paying on a failed write left memory debited and the
        //   file not: a SAVE and a crash later, the file still had the item and CHAR the gold
        //   — item AND gold. `dirty` still up after `flush` IS the write having failed; the
        //   box goes back and the flag stays up so the retry writes the restored state.
        if self.dirty {
            self.boxes = before;
            self.mirror_dirty = true;
            log::error!(
                "bagbox: 판매 refused — the box file could not be written; {name} x{gone} stays in the {}",
                kind.title()
            );
            let _ = proxy.send_event(UserEvent::Notice("판매실패"));
            return;
        }
        if !emulator.write_memory(addr, &bytes) {
            // Put it back. The box file is the durable side and it has already been
            // written, so the refund is written too — leaving it debited would be a
            // silent theft that survives the next launch.
            self.boxes.add(kind, cat, idx, gone);
            self.mark_dirty();
            self.flush();
            log::error!("bagbox: 판매 FAILED — the gold write at {addr:#x} was refused; {name} x{gone} was put back");
            let _ = proxy.send_event(UserEvent::Notice("판매실패"));
            return;
        }
        log::warn!(
            "bagbox: sold {name} x{gone} out of the {} for {pay} gold (balance {balance})",
            kind.title()
        );
        let _ = proxy.send_event(UserEvent::Notice("판매했습니다"));
    }

    /// 올리기: `n` units out of the bag box into the offline warehouse. HOST TO HOST — the
    /// guest never sees it (`BAG_TYPES_FEASIBILITY.md §9.3`).
    ///
    /// ⛔ DESTINATION FIRST: the store is written before the box is debited, so a failure
    /// in between DUPLICATES (the player has it twice) rather than loses it.
    fn deposit_units(&mut self, data_dir: &Path, kind: Kind, cat: u16, idx: u16, n: u8, proxy: &EventLoopProxy<UserEvent>) {
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let have = self.boxes.units_of(kind, cat, idx).min(u32::from(bagbox::STACK_MAX)) as u8;
        let n = n.min(have);
        if n == 0 {
            log::warn!("bagbox: 올리기 refused — the {} no longer holds {name}", kind.title());
            return;
        }
        self.move_seq = self.move_seq.wrapping_add(1);
        let rec = offline::new_stack(cat as u8, idx as u8, n, self.move_seq);
        let (stored, persisted) = crate::network::with_offline_store(data_dir, |st| {
            let ok = st.deposit(rec);
            (ok, ok)
        });
        if !stored {
            let _ = proxy.send_event(UserEvent::OrbNotice(crate::gamefont::from_euckr(offline::WH_FULL)));
            return;
        }
        if !persisted {
            // The store took it in memory and will write it with its next change; the
            // box keeps its units until then. A duplicate, never a loss.
            log::error!(
                "bagbox: 올리기 — the warehouse file did not take {name} x{n}; the {} keeps them",
                kind.title()
            );
            let _ = proxy.send_event(UserEvent::Notice("창고 파일을 쓰지 못했습니다"));
            return;
        }
        let took = self.boxes.spend_units(kind, cat, idx, u32::from(n));
        self.mark_dirty();
        self.flush();
        log::warn!(
            "bagbox: 올리기 — {name} x{took} from the {} into the warehouse (key seq {})",
            kind.title(),
            self.move_seq
        );
        let _ = proxy.send_event(UserEvent::Notice("창고로 올렸습니다"));
    }

    /// 내리기: `n` units of warehouse row `key` into the bag box. Same order — the box
    /// (destination) is written first, the store row debited second.
    #[allow(clippy::too_many_arguments)]
    fn withdraw_units(&mut self, data_dir: &Path, kind: Kind, key: [u8; 8], cat: u16, idx: u16, n: u8, proxy: &EventLoopProxy<UserEvent>) {
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let (row, _) = crate::network::with_offline_store(data_dir, |st| (st.items.iter().find(|it| it[..8] == key).copied(), false));
        let Some(row) = row.filter(|r| u16::from(r[offline::ITEM_CAT_OFF]) == cat && u16::from(r[offline::ITEM_IDX_OFF]) == idx) else {
            log::warn!("bagbox: 내리기 refused — the warehouse row for {name} is gone");
            return;
        };
        let n = n.min(offline::unit_count(&row)).max(1);
        if self.boxes.fits(kind, cat, idx) < u32::from(n) {
            let _ = proxy.send_event(UserEvent::OrbNotice(format!("{}이 가득찼습니다.", kind.title())));
            return;
        }
        let left = self.boxes.add(kind, cat, idx, u32::from(n));
        debug_assert_eq!(left, 0);
        self.mark_dirty();
        self.flush();
        if self.dirty {
            // The box file did not take it: undo, and nothing has left the warehouse.
            self.boxes.spend_units(kind, cat, idx, u32::from(n) - left);
            self.dirty = false;
            self.mirror_dirty = true;
            log::error!("bagbox: 내리기 — the box file did not take {name} x{n}; the warehouse keeps them");
            let _ = proxy.send_event(UserEvent::Notice("가방 파일을 쓰지 못했습니다"));
            return;
        }
        let (taken, persisted) = crate::network::with_offline_store(data_dir, |st| {
            let t = st.take_units(&key, n).is_some();
            (t, t)
        });
        if !taken {
            // Gone between the two reads: give the units back to the warehouse's side by
            // taking them out of the box again.
            self.boxes.spend_units(kind, cat, idx, u32::from(n));
            self.mark_dirty();
            self.flush();
            log::warn!("bagbox: 내리기 — the warehouse row for {name} vanished; the box was put back");
            return;
        }
        if !persisted {
            // The warehouse file still shows the units the box now also has — a
            // duplicate until the store's next write lands, never a loss.
            log::error!("bagbox: 내리기 — the warehouse file did not record {name} x{n} leaving");
        }
        log::warn!(
            "bagbox: 내리기 — {name} x{n} from the warehouse into the {} (persisted = {persisted})",
            kind.title()
        );
        let _ = proxy.send_event(UserEvent::Notice("가방으로 내렸습니다"));
    }

    /// The host dialog answered. `Some(source)` = the player chose an ORB to combine;
    /// the worker then opens the target picker with it. Everything else (a consumable's
    /// verbs, a close) is handled here.
    pub fn answer(
        &mut self,
        pick: Option<usize>,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        proxy: &EventLoopProxy<UserEvent>,
    ) -> Option<OrbSource> {
        match self.dialog.take()? {
            Dialog::Window { kind, list } => {
                let Some(at) = pick else {
                    // Cancelled: the UI has taken the window down, and there is nothing
                    // held to give back — the box never left the host.
                    return None;
                };
                let s = *list.stacks.get(at)?;
                if !self.row_holds(data_dir, kind, &list, at) {
                    // ⛔ REOPEN, do not just refuse. The window stays on screen until the
                    // worker replaces it (`OrbPick::keep_open`), so a silent `None` here
                    // would leave the player looking at a dialog that answers nothing.
                    log::warn!("bagbox: the {} changed under the window — refusing", kind.title());
                    self.open_request = Some((list.screen, kind, true));
                    return None;
                }
                if bagbox::verbs(kind, list.screen).is_empty() {
                    // The in-game 오브가방: the row IS the combine, and the target picker
                    // follows. (Only the inventory has an empty verb list.)
                    return Some(OrbSource::Box {
                        kind,
                        at,
                        cat: s.cat,
                        idx: s.idx,
                    });
                }
                self.open_verbs(kind, at, list, proxy);
                None
            }
            Dialog::Verbs { kind, at, list } => {
                // Dismissed, or a row that is not a verb: the box goes and the LIST STAYS
                // — it is already on screen, it is still correct, and republishing it
                // would throw the player back to the top of a 48-stack box.
                let s = list.stacks.get(at).copied();
                let verb = match (pick, s) {
                    (Some(i), Some(s)) => bagbox::verbs_for(kind, list.screen, s.cat, s.idx).get(i).copied(),
                    _ => None,
                };
                let (Some(verb), Some(s)) = (verb, s) else {
                    self.dialog = Some(Dialog::Window { kind, list });
                    return None;
                };
                let (cat, idx) = (s.cat, s.idx);
                match verb {
                    // ★★ 2026-09-15, user: *"소비템 사용한뒤에 가방창이 닫히고있는데 …
                    // 안닫혔으면 좋겠고"*. The window STAYS. Nothing about the use needs it
                    // gone: both synthetic OKs go through `apply_input` on the worker's own
                    // side, not through the UI, so the game's popup runs perfectly well
                    // underneath. When the job is done the list is republished with the
                    // new counts — keeping the scroll and the row (`OrbPick::keep_view`).
                    Verb::Use => {
                        self.dialog = Some(Dialog::Window { kind, list });
                        self.start_use(emulator, kind, at, cat, idx, UseTarget::Menu, proxy);
                    }
                    // ⛔ This one still takes the list down: the player has to TAP A PLATE
                    // next, and the window swallows every touch on the panel.
                    Verb::Bind => {
                        let _ = proxy.send_event(UserEvent::OrbPick(None));
                        self.pending_bind = Some((cat, idx));
                        let _ = proxy.send_event(UserEvent::PickingSlot(true));
                        log::info!("bagbox: 퀵슬롯등록 armed for cat {cat:#x} idx {idx:#x} — waiting for a plate");
                    }
                    Verb::Drop => self.open_confirm(kind, at, list, proxy),
                    // ★ 96차: the online screens' three.
                    Verb::Detail => {
                        self.dialog = Some(Dialog::Window { kind, list });
                        let _ = proxy.send_event(UserEvent::OrbNotice(Self::detail_text(cat, idx)));
                    }
                    // ★ 105차: 판매 joins them — same shape (a count, then a host-side
                    // move), the destination is the player's purse instead of a store.
                    Verb::Deposit | Verb::Withdraw | Verb::Sell => self.begin_move(emulator, data_dir, kind, at, verb, list, proxy),
                }
                None
            }
            Dialog::Confirm { kind, at, list } => {
                if pick == Some(bagbox::CONFIRM_YES) {
                    let screen = list.screen;
                    if screen.is_warehouse() {
                        self.discard_warehouse(data_dir, kind, &list, at, proxy);
                    } else if let Some(s) = list.stacks.get(at).copied() {
                        self.discard(kind, at, s.cat, s.idx, proxy);
                    }
                    // The list is a row shorter now, so it HAS to be rebuilt.
                    self.open_request = Some((screen, kind, true));
                } else {
                    self.dialog = Some(Dialog::Window { kind, list });
                }
                None
            }
            Dialog::Qty { kind, list, .. } => {
                // An answer on the LIST's channel while the count picker had the screen
                // cannot be a row pick the player meant. Withdraw the question; the list
                // is still on screen as it was.
                crate::network::host_qty_withdraw();
                self.dialog = Some(Dialog::Window { kind, list });
                None
            }
        }
    }

    /// Open a box's window on the next frame — the tap router's way in
    /// (`items::bagbox_cell_focused`), which works on the online screens too. `screen`
    /// decides which SET of boxes and which verbs (96차).
    pub fn request_open(&mut self, screen: Screen, kind: Kind) {
        self.open_request = Some((screen, kind, false));
    }

    /// Reopen the window (the target picker was cancelled and the player is back in the
    /// box). The player's place in the list is kept — nothing was spent.
    pub fn reopen(&mut self, kind: Kind) {
        self.open_request = Some((self.last_screen, kind, true));
    }

    /// Withdraw the dialog without an answer (the menu closed under it). A count the
    /// host was asking for goes with it.
    pub fn close_window(&mut self) {
        self.dialog = None;
        crate::network::host_qty_withdraw();
    }

    /// The item a box 퀵슬롯등록 is waiting to put on a plate, if one is armed.
    pub fn take_pending_bind(&mut self) -> Option<(u16, u16)> {
        self.pending_bind.take()
    }

    /// Spend one unit of the stack a combine used. Persists before returning.
    pub fn spend(&mut self, kind: Kind, at: usize, cat: u16, idx: u16) -> bool {
        let ok = self.boxes.stacks(kind).get(at).is_some_and(|s| s.cat == cat && s.idx == idx) && self.boxes.spend(kind, at, 1);
        if ok {
            self.mark_dirty();
            self.flush();
        } else {
            log::warn!("bagbox: spend refused — {} stack {at} is not {cat:#x}/{idx:#x} any more", kind.title());
        }
        ok
    }

    // ── the stage-and-use job ────────────────────────────────────────────────

    /// 127차: write ONE unit of `(cat, idx)` OVER the bag-item cell of `kind` — the box's
    /// own bag cell — and arm the restage hook with the record it replaced. This is how a
    /// boxed unit reaches a bag cell now (`start_use`, `revive_supply`): no free cell is
    /// needed, so a full bag is no obstacle (user: *"가방이 꽉차있어도 쓸 수 있어야해"*).
    /// `None` — with the reason logged — when another unit is already staged (the knob is
    /// one cell), the bag-item cell is not in the bag, the record cannot be described, or
    /// the write did not read back.
    ///
    /// ⛔ Touches no box. The callers debit BEFORE (box first, cell second — the crash
    /// direction stays "duplicate, never lose") and put the unit back if this refuses.
    fn stage_unit(&mut self, emulator: &mut dyn Emulator, kind: Kind, cat: u16, idx: u16) -> Option<items::BagStage> {
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        if let Some(k) = wie_util::stats::bagbox_restage() {
            if self.stage_held() {
                log::warn!(
                    "bagbox: cannot stage {name} — bag {},{} already holds a staged cat {:#x} idx {:#x} that a live job / revive owns",
                    k.tab + 1,
                    k.slot + 1,
                    k.cat,
                    k.idx
                );
                return None;
            }
            // ★ 129차: a knob nobody holds is a leftover (`JobEnd::Unknown`) or stale —
            //   reclaimed / disarmed here rather than refused, so a plate pressed twice
            //   inside a second, or a death a second after a refused hotkey, still works.
            if !self.reclaim_stale_stage(emulator) {
                log::warn!(
                    "bagbox: cannot stage {name} — bag {},{} holds a stale staged cat {:#x} idx {:#x} the box has no room to take back",
                    k.tab + 1,
                    k.slot + 1,
                    k.cat,
                    k.idx
                );
                return None;
            }
        }
        let plan = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_stage_writes(&read, kind, cat, idx)
        };
        let Some((st, writes)) = plan else {
            log::warn!(
                "bagbox: cannot stage {name} (cat {cat:#x} idx {idx:#x}) — the {} cell is not in the bag, or its record / the item's name could not be read",
                kind.title()
            );
            return None;
        };
        for (a, bytes) in &writes {
            if !emulator.write_memory(*a, bytes) {
                log::error!("bagbox: stage write failed at {a:#x} — bag {},{} may be half-written; putting the {} record back", st.tab + 1, st.slot + 1, kind.title());
                let _ = emulator.write_memory(st.rec, &st.snapshot);
                return None;
            }
        }
        let landed = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_stage_cell(&read, &st)
        };
        if landed != Some(Some((cat, idx, bagbox::STAGE_COUNT))) {
            log::error!(
                "bagbox: the staged record did not read back (bag {},{} reads {landed:?}) — putting the {} record back",
                st.tab + 1,
                st.slot + 1,
                kind.title()
            );
            let _ = emulator.write_memory(st.rec, &st.snapshot);
            return None;
        }
        wie_util::stats::set_bagbox_restage(&wie_util::stats::BagboxRestage {
            tab: st.tab,
            slot: st.slot,
            cat: cat as u8,
            idx: idx as u8,
            rec: st.rec,
            bytes: st.snapshot,
        });
        log::warn!(
            "bagbox: {name} (cat {cat:#x} idx {idx:#x}) x1 staged over the {} cell (bag {},{}) — the restage hook holds its record",
            kind.title(),
            st.tab + 1,
            st.slot + 1
        );
        Some(st)
    }

    /// The host's own restore of a staged cell whose unit the game did NOT consume: the
    /// bag-item record goes back over it and the knob is disarmed. `true` when the cell
    /// still held the staged unit and the write landed; `false` (nothing written) when
    /// the cell reads as anything else — the hook already answered, or the sweep, or
    /// the world moved on — so the caller must not credit the box for it.
    fn unstage(&mut self, emulator: &mut dyn Emulator, st: &items::BagStage, cat: u16, idx: u16) -> bool {
        let writes = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_stage_restore_writes(&read, st, cat, idx)
        };
        wie_util::stats::clear_bagbox_restage();
        let Some(writes) = writes else {
            return false;
        };
        let mut ok = true;
        for (a, bytes) in &writes {
            if !emulator.write_memory(*a, bytes) {
                log::error!("bagbox: restore write failed at {a:#x} — bag {},{} may be half-written", st.tab + 1, st.slot + 1);
                ok = false;
            }
        }
        ok
    }

    /// Take one unit of `(cat, idx)` out of the box, stage it over the box's own bag cell
    /// ([`Self::stage_unit`]), then follow the game's own use of it (`job_frame`).
    #[allow(clippy::too_many_arguments)]
    fn start_use(
        &mut self,
        emulator: &mut dyn Emulator,
        kind: Kind,
        at: usize,
        cat: u16,
        idx: u16,
        target: UseTarget,
        proxy: &EventLoopProxy<UserEvent>,
    ) -> bool {
        if let Some(live) = self.job {
            // ★ 164차: a quickslot plate is pressed mid-fight; a modal box there blocks the
            //   very keys the player is fighting with (140차 made every notice modal). The
            //   press is dropped quietly — the job in flight is the previous press, still
            //   being answered — and the box window keeps its notice (nothing is at stake
            //   there and it explains a dead 사용하기).
            if matches!(target, UseTarget::Field { .. }) {
                log::info!("bagbox: {target:?} pressed while a use is still in flight ({:?}, cat {:#x} idx {:#x}) — ignored", live.phase, live.cat, live.idx);
            } else {
                let _ = proxy.send_event(UserEvent::Notice("아이템을 사용하는 중입니다"));
            }
            return false;
        }
        if !self.armed {
            let _ = proxy.send_event(UserEvent::Notice("가방창이 준비되지 않았습니다"));
            return false;
        }
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        // ★★★ 110차: a unit of this item ALREADY in the bag is used first, and the box is
        //   not debited for it. It is there because an earlier use left it — the game
        //   refused (a full bag, a no-save area: 오브원석's arm checks both), or the popup
        //   never opened (the result-widget case below) — and that unit was already paid
        //   for out of the box. Materialising another on top is what piled 오브원석 into
        //   the plain bag one per use (S25 vc349: `stays in bag 0,9` → next use topped the
        //   stack up and consumed one → one left behind, again). Units the sweep has not
        //   collected yet (a fresh shop purchase) land here too, and that is the same
        //   accounting: the bag's unit goes first, the box's stays where it is.
        let leftover = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_units_of(&read, cat, idx).unwrap_or(0)
        };
        if leftover > 0 {
            log::warn!(
                "bagbox: {name} (cat {cat:#x} idx {idx:#x}) x{leftover} is already in the bag — using that unit for {target:?}, the box keeps its count"
            );
            self.job = Some(UseJob {
                kind,
                cat,
                idx,
                target,
                phase: UsePhase::WaitCell,
                deadline: crate::emulator::now_ms() + USE_WAIT_MS,
                dismissed: 0,
                debited: false,
                staged: None,
            });
            return true;
        }
        // ★★★ 127차: no free cell is asked for any more. The unit is written over the
        //   box's OWN bag cell — the 소비가방 item pinned at bag 1, cell 3 — and the
        //   restage hook puts that record back the moment the game removes the unit.
        //   ⛔ The old `bag_has_room` refusal (*"가방에 빈 칸이 하나 필요합니다"*) is gone
        //   on purpose: a full bag was the one state in which a boxed potion could not
        //   be used at all (user: *"가방이 꽉차있어도 쓸 수 있어야해"*).
        if !self.boxes.spend(kind, at, 1) {
            log::warn!("bagbox: use refused — {} stack {at} has nothing to spend", kind.title());
            return false;
        }
        // ★★★ 158차 D1 (user «전부 권장대로», 2026-10-02): the use debit is a LOAN until the
        //   game saves, like every other path that takes a unit out of a box (97차 · 107차 ·
        //   108차 · 118차-B · 124차-B). Until now `start_use` confirmed the box at once, so a
        //   potion used and then NOT saved was gone when the pre-use save was reloaded —
        //   the one case in which a boxed consumable behaved worse than a bag one (a bag
        //   unit comes back with the bag). The FILE keeps showing the unit (`boxes + loan`)
        //   until a GameSave for this slot clears the loan; a reload without one folds it
        //   back into the box. Every exit that returns the unit to the box (`refund`,
        //   `end_job(Unused)`, the stage failure below, the sweep's reclaim) subtracts the
        //   loan it carried, so the file never double-counts.
        self.loan.add(kind, cat, idx, 1);
        self.mark_dirty();
        // ⛔ Box first, guest second — but without the fsync (104차 ③; see `flush_fast`).
        self.flush_fast();
        let Some(staged) = self.stage_unit(emulator, kind, cat, idx) else {
            // Nothing reached the bag: the unit is the box's again, durably.
            let left = self.boxes.add(kind, cat, idx, 1);
            self.loan.sub(kind, cat, idx, 1);
            self.mark_dirty();
            self.flush();
            let _ = proxy.send_event(UserEvent::Notice("가방창 칸을 쓸 수 없어 사용을 취소했습니다"));
            log::warn!("bagbox: use of {name} cancelled — could not stage it over the {} cell (refunded, leftover {left})", kind.title());
            return false;
        };
        log::warn!("bagbox: using {name} (cat {cat:#x} idx {idx:#x}) through bag {},{} for {target:?}", staged.tab + 1, staged.slot + 1);
        self.job = Some(UseJob {
            kind,
            cat,
            idx,
            target,
            phase: UsePhase::WaitCell,
            deadline: crate::emulator::now_ms() + USE_WAIT_MS,
            dismissed: 0,
            debited: true,
            staged: Some(staged),
        });
        true
    }

    /// The window stayed up through the use (2026-09-15) — republish it so the count on
    /// the row the player just spent from is right, keeping their scroll and focus. Asked
    /// only once the job is over (`open_window` refuses while one is alive).
    fn republish_after_use(&mut self, kind: Kind) {
        if self.dialog.is_some() {
            self.open_request = Some((Screen::Inventory, kind, true));
        }
    }

    /// A quickslot plate holding a boxed consumable was pressed.
    pub fn press_plate(&mut self, emulator: &mut dyn Emulator, plate: u8, cat: u8, idx: u8, proxy: &EventLoopProxy<UserEvent>) {
        let (cat, idx) = (u16::from(cat), u16::from(idx));
        let Some(kind) = bagbox::kind_of(cat) else { return };
        let Some(at) = self.boxes.stacks(kind).iter().position(|s| s.cat == cat && s.idx == idx && s.count > 0) else {
            let _ = proxy.send_event(UserEvent::Notice("소비가방에 남은 것이 없습니다"));
            log::info!("bagbox: plate {plate} pressed — none of cat {cat:#x} idx {idx:#x} left");
            return;
        };
        self.start_use(emulator, kind, at, cat, idx, UseTarget::Field { plate }, proxy);
    }

    fn refund(&mut self, job: UseJob) {
        wie_util::stats::set_bagbox_bypass(0, 0, 0);
        // ⛔ Only a unit the box actually gave goes back (117차 code review). The 110차
        //   leftover path starts a job on a unit that was ALREADY in the bag without
        //   debiting the box; "refunding" that one would mint a unit out of nothing while
        //   the bag's own unit stayed where it was.
        if !job.debited {
            log::warn!(
                "bagbox: the bag's own unit never showed in a cell read — nothing to refund, the {} was not debited",
                job.kind.title()
            );
            return;
        }
        let left = self.boxes.add(job.kind, job.cat, job.idx, 1);
        // 158차 D1: the debit carried a loan — settle it with the refund.
        self.loan.sub(job.kind, job.cat, job.idx, 1);
        self.mark_dirty();
        self.flush();
        log::warn!(
            "bagbox: the unit never reached the bag — refunded to the {} (leftover {left})",
            job.kind.title()
        );
    }

    /// End the use job and settle its staged cell (127차). Every exit of `job_frame` comes
    /// through here so the one cell the box lent can never be left half-accounted:
    ///   * [`JobEnd::Consumed`] — the restage hook has already put the bag-item record
    ///     back inside the game's own removal; only the knob is checked.
    ///   * [`JobEnd::Unused`] — the game did not take the unit: the host restores the
    ///     record itself and the box gets its unit back, in that order (both are host
    ///     state — the file is written after the cell, so a crash between them leaves a
    ///     unit in the bag for the sweep rather than one nowhere).
    ///   * [`JobEnd::Unknown`] — left alone; see the variant.
    /// A job that was not staged (the 110차 leftover path) has nothing to settle.
    fn end_job(&mut self, emulator: &mut dyn Emulator, job: UseJob, end: JobEnd) {
        self.job = None;
        let Some(st) = job.staged else { return };
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, job.cat, job.idx);
        let now = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_stage_cell(&read, &st)
        };
        let armed = wie_util::stats::bagbox_restage().is_some();
        match end {
            JobEnd::Consumed => {
                match now {
                    Some(Some((c, i, _))) if (c, i) == job.kind.bag_item() => {
                        // The hook fired inside the game's removal: the 소비가방 cell was
                        // back before this frame even began. The knob is already clear.
                        if armed {
                            wie_util::stats::clear_bagbox_restage();
                        }
                        log::info!(
                            "bagbox: {name} used through bag {},{} — the {} cell is back in its place (restaged {} time(s) so far)",
                            st.tab + 1,
                            st.slot + 1,
                            job.kind.title(),
                            wie_util::stats::bagbox_restaged()
                        );
                    }
                    Some(Some((c, i, n))) if (c, i) == (job.cat, job.idx) => {
                        // Consumed, yet the cell still reads as the item: the bag's total
                        // dropped elsewhere. Not this stage's unit — leave it to the sweep,
                        // and keep the knob so a later removal of THIS cell still restores.
                        log::warn!(
                            "bagbox: {name} read as consumed but bag {},{} still holds x{n} — leaving it for the sweep (knob {})",
                            st.tab + 1,
                            st.slot + 1,
                            if armed { "armed" } else { "off" }
                        );
                    }
                    other => {
                        // Consumed and the cell is something else (empty, or another item):
                        // the hook did not answer, and the game's own removal ran. The grant
                        // gives the bag-item cell again on the next poll; nothing to refund.
                        wie_util::stats::clear_bagbox_restage();
                        log::warn!(
                            "bagbox: {name} consumed but the restage hook did not put the {} cell back (bag {},{} reads {other:?}) — the grant will give it again",
                            job.kind.title(),
                            st.tab + 1,
                            st.slot + 1
                        );
                    }
                }
            }
            JobEnd::Unused => {
                let restored = self.unstage(emulator, &st, job.cat, job.idx);
                // What goes back to the box: the unit, when the cell still held it (the
                // record is the bag item's again) — or when the bag cannot be read at all
                // (`now == None`: the world is gone, the grid is another's; a unit in a bag
                // no game will ever read is a unit nowhere, and a later reload has the bag
                // from disk without it). ⛔ Not when the cell reads as something ELSE: the
                // unit was consumed after all (and the hook restored the cell) or swept,
                // and either is already accounted for — crediting the box would mint one.
                let refund = job.debited && (restored || now.is_none());
                if refund {
                    let left = self.boxes.add(job.kind, job.cat, job.idx, 1);
                    // 158차 D1: the debit carried a loan — settle it with the refund.
                    self.loan.sub(job.kind, job.cat, job.idx, 1);
                    self.mark_dirty();
                    self.flush();
                    log::warn!(
                        "bagbox: {name} was not used — {}, and the unit is in the box again (leftover {left})",
                        if restored {
                            format!("the {} cell is back over bag {},{}", job.kind.title(), st.tab + 1, st.slot + 1)
                        } else {
                            format!("bag {},{} could not be read (reads {now:?})", st.tab + 1, st.slot + 1)
                        }
                    );
                } else if restored {
                    log::warn!("bagbox: {name} was not used — the {} cell is back over bag {},{}", job.kind.title(), st.tab + 1, st.slot + 1);
                } else {
                    log::warn!(
                        "bagbox: {name} was not used, but bag {},{} no longer holds it (reads {now:?}) — nothing refunded, knob disarmed",
                        st.tab + 1,
                        st.slot + 1
                    );
                }
            }

            JobEnd::Unknown => {
                let still_ours = matches!(now, Some(Some((c, i, _))) if (c, i) == (job.cat, job.idx));
                if !still_ours {
                    wie_util::stats::clear_bagbox_restage();
                }
                log::warn!(
                    "bagbox: {name}'s use ended without a verdict — bag {},{} reads {now:?}; {}",
                    st.tab + 1,
                    st.slot + 1,
                    if still_ours {
                        "the unit stays there: the restage hook answers a late use, the sweep reclaims a leftover"
                    } else {
                        "the cell is something else now, the knob is disarmed"
                    }
                );
            }
        }
    }

    /// Is the use job still holding something this worker iteration could carry?
    /// Either keys it has pushed and nobody has handed to `apply_input` yet, or a live
    /// phase waiting on the guest. Cheap enough to ask every frame — see the step loop in
    /// `emulator.rs` (104차 ①).
    pub fn job_pending(&self) -> bool {
        self.job.is_some() || !self.self_input.is_empty()
    }

    /// One step of the materialise-and-use job. Called once from [`Self::frame`], and then
    /// again by the worker's bounded step loop for as long as each step produced keys to
    /// deliver — that loop is what turns a 사용하기 from four worker iterations into one
    /// (104차 ①). ⛔ Nothing here may block: every phase either advances or returns and
    /// waits for its deadline.
    pub fn job_frame(
        &mut self,
        emulator: &mut dyn Emulator,
        pages: &mut quickpage::Pages,
        window: &mut quickpage::Window,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        self.job_step(emulator, pages, window, proxy);
        // ★ 165차: the step loop ticks again right after this, and a step can put a unit
        //   back into a box (a refund, a reclaim) — see `publish_knobs`. Also with no job
        //   left: the loop still ticks while delivered keys remain, and every tick's intake
        //   is drained before the next one (`bagbox::FreshIntake` pairs a give and a take
        //   per drain). Cheap when nothing moved.
        self.publish_knobs(emulator);
    }

    fn job_step(
        &mut self,
        emulator: &mut dyn Emulator,
        pages: &mut quickpage::Pages,
        window: &mut quickpage::Window,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        let Some(job) = self.job else { return };
        let now = crate::emulator::now_ms();
        // ★ 117차: the step loop reaches here straight from a tick — see `drain_intake`.
        if matches!(job.phase, UsePhase::WaitResult { .. }) {
            self.drain_intake(emulator);
        }
        match job.phase {
            UsePhase::WaitCell => {
                let cell = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    job_cell(&read, &job)
                };
                let Some((tab, slot, count)) = cell else {
                    if now >= job.deadline {
                        if job.staged.is_some() {
                            // 127차: a staged unit is in the cell from the moment it is
                            // staged, so not finding it means the bag stopped reading. The
                            // cell itself decides what to give back (`end_job`).
                            self.end_job(emulator, job, JobEnd::Unused);
                        } else {
                            self.job = None;
                            self.refund(job);
                        }
                        let _ = proxy.send_event(UserEvent::Notice("가방에 넣지 못해 사용을 취소했습니다"));
                    }
                    return;
                };
                // The unit landed; the bypass has done its work either way.
                wie_util::stats::set_bagbox_bypass(0, 0, 0);
                match job.target {
                    UseTarget::Field { plate } => {
                        // ★★★ 164차: during a skill cast the game drops the hotkey before
                        //   its handler runs (`items::field_key_gate`), and the job used to
                        //   wait out its guard for nothing. A potion's arm is safe to run
                        //   over a cast (`bagbox::use_runs_through_a_cast`), so call it
                        //   the way the hotkey's item arm does — `useItem(tab, slot)` — and
                        //   follow the result exactly as for the hotkey. No guest slot is
                        //   loaned: nothing presses a key.
                        let gate = {
                            let read = |a: u32, l: usize| emulator.read_memory(a, l);
                            items::field_key_gate(&read)
                        };
                        if gate == Some(items::FieldKeyGate::Casting)
                            && bagbox::use_runs_through_a_cast(job.cat, job.idx)
                            && let Some(f) = crate::guest_build::use_item_fn()
                        {
                            emulator.call_guest_function(f, &[u32::from(tab), u32::from(slot)]);
                            log::warn!("bagbox: plate {plate} pressed during a skill cast — useItem(bag {tab},{slot}) called directly (x{count})");
                            let (before, scene) = {
                                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                                (job_units(&read, &job).unwrap_or(u32::from(count)), items::scene_now(&read))
                            };
                            self.job = Some(UseJob {
                                phase: UsePhase::WaitField {
                                    before,
                                    asked: false,
                                    scene,
                                    settled: 0,
                                },
                                deadline: now + USE_RESULT_MS,
                                ..job
                            });
                            return;
                        }
                        let plan = pages.activate_boxed(window, plate, tab, slot);
                        let quickpage::Activation::Press { guest, actions, .. } = plan else {
                            log::warn!("bagbox: plate {plate} would not take the loan — the unit in bag {tab},{slot} was not used");
                            self.end_job(emulator, job, JobEnd::Unused);
                            return;
                        };
                        crate::emulator::apply_quickpage_actions(emulator, &actions);
                        let Some((code, _)) = crate::quickslot_hotkey(guest) else {
                            self.end_job(emulator, job, JobEnd::Unused);
                            return;
                        };
                        emulator.handle_event(Event::Keydown(code));
                        emulator.handle_event(Event::Keyup(code));
                        log::warn!("bagbox: plate {plate} pressed through guest slot {guest} — bag {tab},{slot} (x{count})");
                        // ★ 127차: the job used to end right here. Now it waits for the
                        //   game to run the use (`WaitField`): while it is alive the poll
                        //   does not run, so the sweep cannot take the staged unit back
                        //   between the queued hotkey and the tick that handles it.
                        let (before, scene) = {
                            let read = |a: u32, l: usize| emulator.read_memory(a, l);
                            (job_units(&read, &job).unwrap_or(u32::from(count)), items::scene_now(&read))
                        };
                        self.job = Some(UseJob {
                            phase: UsePhase::WaitField {
                                before,
                                asked: false,
                                scene,
                                settled: 0,
                            },
                            deadline: now + USE_RESULT_MS,
                            ..job
                        });
                    }
                    UseTarget::Menu => {
                        // ★★★ 110차: a box the GAME put up takes the next OK before the
                        //   inventory does — 오브원석's arm shows the orb it rolled in the
                        //   shared choice widget (`items::game_box_up`), invisible under
                        //   our window. Close it with the key the player would press, and
                        //   come back to this phase on the next step (the worker's step
                        //   loop delivers the OK, ticks, and calls here again in the same
                        //   iteration). Bounded: past `USE_DISMISS_MAX` the cursor OK goes
                        //   out regardless and the deadline owns the failure, as before.
                        let box_up = {
                            let read = |a: u32, l: usize| emulator.read_memory(a, l);
                            items::game_box_up(&read)
                        };
                        if box_up && job.dismissed < USE_DISMISS_MAX {
                            self.self_input.push_back(InputEvent::Key {
                                code: KeyCode::OK,
                                down: true,
                            });
                            self.self_input.push_back(InputEvent::Key {
                                code: KeyCode::OK,
                                down: false,
                            });
                            self.job = Some(UseJob {
                                dismissed: job.dismissed + 1,
                                ..job
                            });
                            log::info!(
                                "bagbox: the game has a box up over the inventory — closing it before the popup (OK {}/{USE_DISMISS_MAX})",
                                job.dismissed + 1
                            );
                            return;
                        }
                        let writes = {
                            let read = |a: u32, l: usize| emulator.read_memory(a, l);
                            items::inv_cursor_writes(&read, tab, slot)
                        };
                        let Some(writes) = writes else {
                            log::warn!("bagbox: the inventory is not up any more — the unit in bag {tab},{slot} was not used");
                            self.end_job(emulator, job, JobEnd::Unused);
                            return;
                        };
                        for (a, bytes) in writes {
                            if !emulator.write_memory(a, &bytes) {
                                log::warn!("bagbox: cursor write failed at {a:#x}");
                                self.end_job(emulator, job, JobEnd::Unused);
                                return;
                            }
                        }
                        self.self_input.push_back(InputEvent::Key {
                            code: KeyCode::OK,
                            down: true,
                        });
                        self.self_input.push_back(InputEvent::Key {
                            code: KeyCode::OK,
                            down: false,
                        });
                        self.job = Some(UseJob {
                            phase: UsePhase::WaitPopup { tab, slot },
                            deadline: now + USE_WAIT_MS,
                            ..job
                        });
                        log::info!("bagbox: cursor on bag {tab},{slot} — opening the game's popup");
                    }
                }
            }
            UsePhase::WaitPopup { tab, slot } => {
                let open = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::popup_open_on(&read, tab, slot)
                };
                if !open {
                    if now >= job.deadline {
                        log::warn!("bagbox: the popup never opened — the unit in bag {tab},{slot} was not used");
                        self.end_job(emulator, job, JobEnd::Unused);
                    }
                    return;
                }
                let sel = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::popup_select_write(&read, 0)
                };
                if let Some((a, row)) = sel {
                    let _ = emulator.write_memory(a, &[row]);
                }
                self.self_input.push_back(InputEvent::Key {
                    code: KeyCode::OK,
                    down: true,
                });
                self.self_input.push_back(InputEvent::Key {
                    code: KeyCode::OK,
                    down: false,
                });
                // ★★★ 115차/116차/117차-C: 귀환서 / 그리폰의피리 — the game's useItem arm RAISES
                //   ITS OWN DIALOG instead of consuming (`bagbox::use_opens_window`) and
                //   consumes on 예, with the warp. ⛔ Not through `WaitResult`: it read that
                //   dialog as "a box, nothing consumed", toasted a refusal, and its dismissing
                //   OK pressed 예 for the player (115차). The job follows the DIALOG instead —
                //   the shared choice widget, not a window (117차-C, device-measured; 116차
                //   waited for window 15 and never saw it).
                if bagbox::use_opens_window(job.cat, job.idx) {
                    let before = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::bag_units_of(&read, job.cat, job.idx)
                    };
                    self.absorbed_since_use = None;
                    self.job = Some(UseJob {
                        phase: UsePhase::WaitGameChoice { tab, slot, before },
                        deadline: now + USE_WAIT_MS,
                        ..job
                    });
                    log::warn!(
                        "bagbox: 사용하기 on bag {tab},{slot} raises the game's own dialog (cat {:#x} idx {:#x}) — waiting for its choice widget (bag holds {before:?})",
                        job.cat,
                        job.idx
                    );
                    return;
                }
                // ★★★ 110차-B: not done yet — follow the use to its result (below).
                let before = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::bag_units_of(&read, job.cat, job.idx).unwrap_or(0)
                };
                self.absorbed_since_use = None;
                self.job = Some(UseJob {
                    phase: UsePhase::WaitResult { before },
                    deadline: now + USE_RESULT_MS,
                    ..job
                });
                // ⛔ The window's republish moved to the END of `WaitResult` (110차-C):
                //   `open_window` refuses while a job is alive, and vc351 asked here, one
                //   phase too early — the count never moved until the window was reopened.
                log::warn!("bagbox: 사용하기 confirmed on bag {tab},{slot}");
            }
            UsePhase::WaitResult { before } => {
                // ★★★ 110차-B (제보: *"오브 사용할때마다 무슨 오브 나왔는지 안내가 나와야하는데
                //   쓰고 창을 나와야 안내가 떠있어"*). The game's own result box draws under
                //   our window, so the host says it instead — with the name the ring just
                //   handed over — and closes the game's box right here, so nothing is left
                //   waiting behind the window and the NEXT use never meets it (the 110차
                //   dismiss in `WaitCell` stays as the fallback). The job holds until the
                //   game has actually run the use: a tap in that ~100 ms gets 「사용하는
                //   중」, which is the truth, and it is what makes fast taps safe — the
                //   next cursor OK cannot be queued behind an unprocessed use any more.
                let (units, box_up) = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    (items::bag_units_of(&read, job.cat, job.idx).unwrap_or(before), items::game_box_up(&read))
                };
                if units < before {
                    if box_up {
                        self.self_input.push_back(InputEvent::Key {
                            code: KeyCode::OK,
                            down: true,
                        });
                        self.self_input.push_back(InputEvent::Key {
                            code: KeyCode::OK,
                            down: false,
                        });
                    }
                    self.end_job(emulator, job, JobEnd::Consumed);
                    self.republish_after_use(job.kind);
                    if let Some(name) = self.absorbed_since_use.take() {
                        let _ = proxy.send_event(UserEvent::NoticeText(format!("{name}{} 입수 하였습니다.", bagbox::object_particle(&name))));
                    }
                    log::info!(
                        "bagbox: the game consumed the unit (bag {before} -> {units}); its result box was {}",
                        if box_up { "closed" } else { "not up" }
                    );
                } else if box_up {
                    // A box, but nothing consumed: the game refused (오브원석: a no-save
                    // area — a full bag no longer refuses, 127차). Close it and say so.
                    // A STAGED unit goes straight back into the box and the 소비가방 record
                    // over its cell (`end_job`); a leftover stays in the bag and the next
                    // use takes it first (`start_use`).
                    self.self_input.push_back(InputEvent::Key {
                        code: KeyCode::OK,
                        down: true,
                    });
                    self.self_input.push_back(InputEvent::Key {
                        code: KeyCode::OK,
                        down: false,
                    });
                    self.end_job(emulator, job, JobEnd::Unused);
                    self.republish_after_use(job.kind);
                    let _ = proxy.send_event(UserEvent::Notice("게임이 사용을 거절했습니다"));
                    log::warn!("bagbox: the game refused the use (bag still {units}) — its box closed");
                } else if now >= job.deadline {
                    // Nothing consumed and no box within the guard: the game did not use
                    // it (a potion at full HP says nothing). A staged unit is taken back.
                    self.end_job(emulator, job, JobEnd::Unused);
                    self.republish_after_use(job.kind);
                    log::warn!("bagbox: no result seen within {USE_RESULT_MS} ms (bag {before} -> {units}) — giving up the watch");
                }
            }
            UsePhase::WaitField {
                before,
                asked,
                scene,
                settled,
            } => {
                let (units, box_up, scene_now, in_shell) = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    (
                        job_units(&read, &job),
                        items::game_box_up(&read),
                        items::scene_now(&read),
                        items::shell_state(&read),
                    )
                };
                if in_shell {
                    // The world is gone under the question (a game over): the bag this job
                    // followed no longer exists. Nothing to take back — 117차's rule.
                    self.end_job(emulator, job, JobEnd::Unknown);
                    log::warn!("bagbox: the game left the world while a hotkey use was pending — letting the job go (bag {before} -> {units:?})");
                    return;
                }
                // The game took it: the staged cell reads as the bag item again (the hook
                // put it back inside the removal), or — once a box has been asked — 예 on a
                // 귀환서 posted the warp and the scene moved. Either is proof, and the second
                // survives a bag that cannot be read mid-load. ⛔ The scene counts only
                // after a box: a potion the game refused while the player walked through
                // a map exit inside the 1 s guard is not a consumed potion.
                let consumed = units.is_some_and(|u| u < before);
                let moved = asked && scene.is_some() && scene_now.is_some() && scene_now != scene;
                if consumed || moved {
                    self.end_job(emulator, job, JobEnd::Consumed);
                    log::info!("bagbox: the hotkey use consumed the unit (bag {before} -> {units:?}, scene {scene:?} -> {scene_now:?})");
                    return;
                }
                if box_up {
                    // ★★★ 129차 (128차 code review ①): the game is asking the PLAYER — a
                    //   귀환서's 「이동하겠습니까?」, its refusal message — and the player
                    //   owns the clock. The job stays, and with it the poll's sweep stays
                    //   off the staged cell, for as long as the box is up (guard
                    //   `USE_ANSWER_MAX_MS`, as `WaitGameAnswer`). ⛔ Never `Unknown` here
                    //   while the box is up: letting go handed the unit to the sweep and
                    //   turned 예 into a free 귀환 (see the variant).
                    if !asked {
                        self.job = Some(UseJob {
                            phase: UsePhase::WaitField {
                                before,
                                asked: true,
                                scene,
                                settled: 0,
                            },
                            deadline: now + USE_ANSWER_MAX_MS,
                            ..job
                        });
                        log::warn!("bagbox: the hotkey use raised a game box — holding the staged unit in bag {}, {} until the player answers", job.staged.map_or(0, |s| s.tab + 1), job.staged.map_or(0, |s| s.slot + 1));
                    } else if now >= job.deadline {
                        self.end_job(emulator, job, JobEnd::Unknown);
                        log::warn!("bagbox: the game's box stayed up for {USE_ANSWER_MAX_MS} ms after a hotkey use — letting the job go, the unit is the sweep's");
                    }
                    return;
                }
                if asked {
                    // The box is gone and nothing was consumed (yet): 예 consumes inside the
                    // key handler but its warp lands a tick later — the same patience as
                    // `WaitGameAnswer`, then it was declined (아니오 / the refusal's OK) and
                    // the unit goes straight back into the box, the bag-item record over
                    // its cell. A bag that cannot be read waits out the guard instead.
                    if units.is_none() {
                        if now >= job.deadline {
                            self.end_job(emulator, job, JobEnd::Unknown);
                            log::warn!("bagbox: the bag never read again after the game's box closed on a hotkey use — letting the job go");
                        }
                        return;
                    }
                    if settled + 1 < USE_ANSWER_SETTLE_POLLS {
                        self.job = Some(UseJob {
                            phase: UsePhase::WaitField {
                                before,
                                asked,
                                scene,
                                settled: settled + 1,
                            },
                            ..job
                        });
                        return;
                    }
                    self.end_job(emulator, job, JobEnd::Unused);
                    log::warn!("bagbox: the game's box closed with nothing consumed (bag {before} -> {units:?}) — the hotkey use was declined, the unit is the box's again");
                    return;
                }
                if now >= job.deadline {
                    // Not consumed within the guard and no box was ever raised: the game
                    // refused silently (a potion at full HP). The unit stays where it is —
                    // the restage hook answers a late removal, the field's next quiet poll
                    // (or the next stage over the cell) reclaims a leftover. ⛔ Never
                    // `Unused` here: a refund now plus a late use would be a free unit.
                    self.end_job(emulator, job, JobEnd::Unknown);
                    log::warn!("bagbox: the hotkey use showed no result within {USE_RESULT_MS} ms (bag {before} -> {units:?}) — letting the job go");
                }
            }
            // ── 116차/117차-D: a use that raises the GAME'S OWN DIALOG (귀환서 / 그리폰의피리) ──
            UsePhase::WaitGameChoice { tab, slot, before } => {
                let (st, scene) = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    (items::menu_state(&read), items::scene_now(&read))
                };
                let Some(st) = st.filter(|s| s.choice) else {
                    if now >= job.deadline {
                        // The dialog never came — nothing was consumed (that arm only
                        // consumes on 예). A staged unit goes back; a leftover stays in bag
                        // {tab},{slot} for the sweep. The box window stays as it is.
                        self.end_job(emulator, job, JobEnd::Unused);
                        self.republish_after_use(job.kind);
                        log::warn!(
                            "bagbox: the game's dialog never came up within {USE_WAIT_MS} ms after 사용하기 on bag {tab},{slot} — nothing was consumed"
                        );

                    }
                    return;
                };
                // ★★★ 117차-D (user: *"기존 팝업이 그대로 소비가방 위에 뜨게"*): the game's own
                //   dialog is what the player answers. 귀환서's is a framed MESSAGE BOX inside
                //   the panel, so the UI shows it THROUGH the box window (`OrbReveal`: the
                //   frame's rectangle is re-blitted over the dialog, the keys are the game's).
                //   The bag page around it stays covered — the detour the user refused twice
                //   is never on screen. 그리폰의피리's town list has no measured frame: for it
                //   the window steps aside (116차's shape) and comes back on cancel.
                if bagbox::use_dialog_over_window(job.cat, job.idx) && self.dialog.is_some() {
                    let _ = proxy.send_event(UserEvent::OrbReveal(true));
                    log::warn!(
                        "bagbox: the game's dialog is up (rows {} axis {}) — showing it through the box window",
                        st.list_rows,
                        st.list_axis
                    );
                } else {
                    self.dialog = None;
                    let _ = proxy.send_event(UserEvent::OrbPick(None));
                    log::warn!(
                        "bagbox: the game's dialog is up (rows {} axis {}) — box window hidden while the player answers (bag holds {before:?})",
                        st.list_rows,
                        st.list_axis
                    );
                }
                self.job = Some(UseJob {
                    phase: UsePhase::WaitGameAnswer { before, scene, settled: 0 },
                    deadline: now + USE_ANSWER_MAX_MS,
                    ..job
                });
            }
            UsePhase::WaitGameAnswer { before, scene, settled } => {
                let (up, units, scene_now, in_shell) = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    (
                        items::game_box_up(&read),
                        items::bag_units_of(&read, job.cat, job.idx),
                        items::scene_now(&read),
                        items::shell_state(&read),
                    )
                };
                if up {
                    // Still up — the player is deciding. Only the guard deadline ends this.
                    if now >= job.deadline {
                        self.end_job(emulator, job, JobEnd::Unknown);
                        let _ = proxy.send_event(UserEvent::OrbReveal(false));
                        log::warn!("bagbox: the game's dialog stayed up for {USE_ANSWER_MAX_MS} ms — letting the job go, the unit is the sweep's");
                        return;
                    }
                    // ★ 117차: `before` could not be read when the dialog came up. 예 consumes
                    //   when the dialog CLOSES, so a read while it is still up is still
                    //   "before" — take the first one that comes.
                    if before.is_none() && units.is_some() {
                        self.job = Some(UseJob {
                            phase: UsePhase::WaitGameAnswer {
                                before: units,
                                scene,
                                settled,
                            },
                            ..job
                        });
                    }
                    return;
                }
                // The dialog is gone: the window is whole again from this frame on (117차-D).
                if settled == 0 {
                    let _ = proxy.send_event(UserEvent::OrbReveal(false));
                }
                // ⛔ 117차: the world is gone (a game over, 시스템 → 메인메뉴). Whatever enters
                //   the next field is a record loaded from disk, and the bag this job was
                //   following no longer exists — nothing to take back, nothing to reopen. Let
                //   go, or the job blocks the sweep, the pin, the grant and every box window
                //   for the guard's ten minutes.
                if in_shell {
                    self.end_job(emulator, job, JobEnd::Unknown);
                    log::warn!("bagbox: the game left the world while a 귀환 answer was pending — letting the job go (bag {before:?} -> {units:?})");
                    return;
                }
                // 예 consumes the unit inside the key handler and posts the warp; either is
                // proof the game took it — and then OUR window goes too: the menu it sat over
                // leaves with the scene.
                let consumed = before.zip(units).is_some_and(|(b, u)| u < b);
                let moved = scene.is_some() && scene_now.is_some() && scene_now != scene;
                if consumed || moved {
                    self.end_job(emulator, job, JobEnd::Consumed);
                    if self.dialog.is_some() {
                        self.dialog = None;
                        let _ = proxy.send_event(UserEvent::OrbPick(None));
                    }
                    log::warn!(
                        "bagbox: the game took the unit (bag {before:?} -> {units:?}, scene {scene:?} -> {scene_now:?}) — 귀환 accepted, the box window is closed"
                    );
                    return;
                }
                if units.is_none() {
                    // Unreadable mid-transition (a scene load): wait — but not forever. ⛔ This
                    //   arm had no deadline (117차 code review): a bag that never read again
                    //   kept the job, and everything gated on `job_pending`, alive indefinitely.
                    if now >= job.deadline {
                        self.end_job(emulator, job, JobEnd::Unknown);
                        log::warn!(
                            "bagbox: the bag never read again after the game's dialog closed ({USE_ANSWER_MAX_MS} ms) — letting the job go, the unit is the sweep's"
                        );
                    }
                    return;
                }
                if settled + 1 < USE_ANSWER_SETTLE_POLLS {
                    self.job = Some(UseJob {
                        phase: UsePhase::WaitGameAnswer {
                            before,
                            scene,
                            settled: settled + 1,
                        },
                        ..job
                    });
                    return;
                }
                // Declined (아니오 / CLR) or refused (the game's own message): the unit goes
                // straight back into the box — nothing was used (user: *"아니오 누르면
                // 사용되면 안되고"*). Under the box window that is invisible; with the window
                // hidden it comes back where it was.
                self.job = None;
                // ⛔ FAIL CLOSED on an unknown `before` (117차): with no baseline, "nothing
                //   consumed" cannot be told from "예 into the same scene" (a stack of two, one
                //   spent), and a reclaim then would take back a unit the game DID use — a free
                //   귀환. The unit stays in the bag for the sweep, which conserves it either way.
                //   ★ 127차: a STAGED unit needs no baseline — its cell still reading as the
                //   unit (count 1) IS the proof nothing was consumed, since a consumed unit's
                //   cell reads as the bag item the hook put back. `end_job` asks the cell.
                let reclaimed = if job.staged.is_some() {
                    self.end_job(emulator, job, JobEnd::Unused);
                    true
                } else if before.is_some() {
                    self.reclaim_unit(emulator, job)
                } else {
                    log::warn!(
                        "bagbox: the bag was never readable while the game's dialog was up — not reclaiming, the sweep owns what is left (bag now {units:?})"
                    );
                    false
                };
                if self.dialog.is_some() {
                    // 귀환서 (117차-D): our window never left; its count comes back with the
                    // republish.
                    self.republish_after_use(job.kind);
                    log::info!("bagbox: the box window stays; unit reclaimed = {reclaimed}");
                    return;
                }
                let inventory = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::inventory_page_up(&read)
                };
                if inventory {
                    // `keep_view` puts the list back at the player's row. The confirm guard is
                    // the UI's: it re-arms whenever a list comes up from NOTHING, `keep_view`
                    // or not (117차 — the hidden window IS nothing on the UI side).
                    self.open_request = Some((Screen::Inventory, job.kind, true));
                } else {
                    log::info!(
                        "bagbox: the inventory page is not on screen after the game's dialog closed — the box window is not reopened (reclaimed = {reclaimed})"
                    );
                }
            }
        }
    }

    /// 116차: take the ONE unit a use materialised back into its box — the player declined
    /// the game's window (or the game refused), so nothing was used. The same shape as a
    /// sweep of one cell: box (and file) first, then the cell is cleared to the game's own
    /// empty record. If the unit merged into a stack the player already had in the bag,
    /// the whole stack comes along — exactly what the sweep would do on the next quiet
    /// poll — and the ledger notes the cell like any swept cell.
    fn reclaim_unit(&mut self, emulator: &mut dyn Emulator, job: UseJob) -> bool {
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, job.cat, job.idx);
        let found = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_cell_of(&read, job.cat, job.idx).and_then(|(tab, slot, count)| {
                let cell = bagbox::BagCell {
                    tab,
                    slot,
                    cat: job.cat,
                    idx: job.idx,
                    count,
                };
                items::cell_clear_writes(&read, &cell).map(|w| (cell, w))
            })
        };
        let Some((cell, writes)) = found else {
            log::warn!("bagbox: {name} is not in the bag any more — nothing to take back (the sweep owns whatever is left)");
            return false;
        };
        let before = self.boxes.clone();
        let loan_before = self.loan.clone();
        let left = self.boxes.add(job.kind, job.cat, job.idx, u32::from(cell.count));
        if left > 0 {
            self.boxes = before;
            log::warn!(
                "bagbox: the {} has no room for {name} x{} — left in bag {},{} for the sweep",
                job.kind.title(),
                cell.count,
                cell.tab,
                cell.slot
            );
            return false;
        }
        self.loan.sub(job.kind, job.cat, job.idx, u32::from(cell.count));
        self.boxes.extend_absorbed([cell]);
        self.mark_dirty();
        self.flush();
        if self.dirty {
            self.boxes = before;
            self.loan = loan_before;
            self.dirty = false;
            log::error!(
                "bagbox: the box file refused the reclaim — {name} stays in bag {},{}",
                cell.tab,
                cell.slot
            );
            return false;
        }
        let mut ok = true;
        for (a, bytes) in &writes {
            if !emulator.write_memory(*a, bytes) {
                log::error!(
                    "bagbox: reclaim write failed at {a:#x} — bag {},{} may be half-cleared",
                    cell.tab,
                    cell.slot
                );
                ok = false;
                break;
            }
        }
        log::warn!(
            "bagbox: 귀환 declined or refused — {name} x{} taken back from bag {},{} into the {} (ok = {ok})",
            cell.count,
            cell.tab,
            cell.slot,
            job.kind.title()
        );
        ok
    }

    // ── the poll ─────────────────────────────────────────────────────────────

    /// Every 30 frames, in the field with nothing open: the sweep, then the grant.
    /// `dialog_open` = the target picker is up with a box stack behind it — the sweep
    /// must not append to a box whose rows a dialog is naming.
    #[allow(clippy::too_many_arguments)]
    pub fn poll(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        frame: u64,
        dialog_open: bool,
        pages: &mut quickpage::Pages,
        window: &mut quickpage::Window,
        proxy: &EventLoopProxy<UserEvent>,
    ) {
        if !self.armed || self.dialog.is_some() || dialog_open || self.job.is_some() || self.mat.is_some() {
            return;
        }
        if !self.quiet(emulator) {
            return;
        }
        // ★★ 118차-B: no sweep while the player is dying or on the game-over screen — the
        //   one 부활의서 `revive_supply` just lent the bag has to still be there when the
        //   game looks for it (`bagbox::REVIVE_SCROLL`). The death animation is a field
        //   state (no window), so `quiet` alone would let a poll take it straight back.
        if self.player_dying(emulator) {
            return;
        }
        // ★ 165차: the step loop may have ticked since `frame` drained — file what the hook
        //   took (and the room it reserved) before the sweep plans against the box.
        self.drain_intake(emulator);
        self.sweep(emulator, data_dir, pages, window, proxy);
        // ⛔ Field only for the grant: it is a guest call (see the load-edge block in `frame`).
        if self.in_field(emulator) {
            self.grant(emulator, frame);
            // 125차-B: and for the eviction of stranded stacks, for the same reason.
            self.evict_stranded(emulator, frame);
        }
        self.pin(emulator);
        self.drop_exhausted_plates(emulator, data_dir, pages);
        // …and the sweep's additions are the hook's to see before the next tick.
        self.publish_knobs(emulator);
    }

    /// Is the player in the death animation (state 16) or on the game-over screen (7)?
    /// The player STATE byte of the field state machine — `items::PLAYER_STATE_OFF`, the
    /// same byte 101차's revive writes.
    fn player_dying(&self, emulator: &mut dyn Emulator) -> bool {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        let Some(c) = items::resolve_char(&read) else { return false };
        matches!(
            read(c + items::PLAYER_STATE_OFF, 1).first().copied(),
            Some(items::PLAYER_STATE_DYING) | Some(items::PLAYER_STATE_GAME_OVER)
        )
    }

    /// 118차-B: while the player is DYING, lend the game ONE 부활의서 out of the 소비가방 —
    /// since 127차 written over the 소비가방 cell itself ([`Self::stage_unit`]; it used to be
    /// the game's own `giveItem` into a free cell, which a full bag has none of). The box is
    /// debited when it is seen to land (`revive_frame`). The game's GAME OVER tick then finds
    /// it in the bag (`0x3bdec`), asks 「부활하시겠습니까?」, and 예 takes that one unit
    /// (`0x5340c(tab, slot, 1)` — where the restage hook puts the 소비가방 record straight
    /// back) — the rest stays in the box, which is what the player asked for (*"소비가방에서
    /// 그냥 1개 사용되고 일반가방으로 안나오게"*).
    /// ⛔ ONE, and only when the bag has none (`bagbox::revive_supply_units`): the game takes
    /// exactly one, and a second would be left in the plain bag.
    fn revive_supply(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        if !self.armed || self.revive.is_some() || frame < self.revive_retry_at {
            return;
        }
        let (cat, idx) = bagbox::REVIVE_SCROLL;
        let Some(kind) = bagbox::kind_of(cat) else { return };
        let box_units = self.boxes.units_of(kind, cat, idx);
        if box_units == 0 {
            return;
        }
        // ★ 129차 (128차 code review ⑥): the state byte FIRST — this runs every frame for
        //   as long as the box holds a 부활의서, and the bag walk (~325 reads) is only worth
        //   doing in the one second the player is actually dying.
        let state = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::resolve_char(&read).and_then(|c| read(c + items::PLAYER_STATE_OFF, 1).first().copied())
        };
        if !matches!(state, Some(items::PLAYER_STATE_DYING) | Some(items::PLAYER_STATE_GAME_OVER)) {
            self.revive_held_said = false;
            return;
        }
        // ★★ 135차-B (135차 §2 ⑤, probe-measured): a live USE JOB can hold the one staging
        //   cell while the player dies. A field 귀환서's 「이동하겠습니까?」 keeps
        //   `WaitField { asked }` alive for as long as it is up — and monsters keep hitting
        //   through it (HP 167 → 0 headless, boss scene 231), and the game's death animation
        //   then WAITS at state 16 until the question is answered (1000 frames, choice flag
        //   still 1). 아니오 closes it, the job ends `Unused` a few frames later and frees
        //   the cell, and the GAME OVER tick (`0x3b7ac`) looks for a 부활의서 on EVERY tick
        //   while its window is up (a scroll given 260 frames into GAME OVER still opened
        //   window 17). So the right wait is the next frame: `stage_unit` refusing here used
        //   to arm `REVIVE_RETRY_FRAMES` — ten seconds in which the player pressed through
        //   GAME OVER to the title with the scroll never staged. Checked before the bag
        //   walk, so a long question costs one byte a frame.
        if self.stage_held() {
            if !self.revive_held_said {
                self.revive_held_said = true;
                log::warn!(
                    "bagbox: the player is dying with {box_units} 부활의서 in the 소비가방 while a use job holds the staging cell — staging the scroll the frame it is free"
                );
            }
            return;
        }
        let bag_units = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_units_of(&read, cat, idx)
        };
        let Some(bag_units) = bag_units else { return };
        let n = bagbox::revive_supply_units(box_units, bag_units);
        if n == 0 {
            return;
        }
        // A use job holding the cell was answered above (next frame). What is left for
        //   `stage_unit` to refuse is a cell it cannot use at all — that one backs off.
        let Some(staged) = self.stage_unit(emulator, kind, cat, idx) else {
            self.revive_retry_at = frame + REVIVE_RETRY_FRAMES;
            log::warn!(
                "bagbox: the player is dying with {box_units} 부활의서 in the 소비가방 but the 소비가방 cell could not be staged — the game's revive check will not see one"
            );
            return;
        };
        self.revive = Some(ReviveBatch {
            kind,
            cat,
            idx,
            sent: n as u16,
            before: bag_units,
            deadline: crate::emulator::now_ms() + MAT_WAIT_MS,
            staged: Some(staged),
        });
        log::warn!(
            "bagbox: the player is dying (state {state:?}) — one 부활의서 out of the 소비가방 ({box_units} there) is staged over bag {},{} for the game's revive check",
            staged.tab + 1,
            staged.slot + 1
        );
    }

    /// 118차-B: settle the supply in flight — debit the box by what actually landed.
    fn revive_frame(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        let Some(b) = self.revive else { return };
        let now = crate::emulator::now_ms();
        let units = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_units_of(&read, b.cat, b.idx)
        };
        let settled = match units {
            Some(after) if after > b.before => Some(after),
            _ if now >= b.deadline => units,
            _ => None,
        };
        let Some(after) = settled else { return };
        wie_util::stats::set_bagbox_bypass(0, 0, 0);
        self.revive = None;
        let landed = after.saturating_sub(b.before).min(u32::from(b.sent));
        let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, b.cat, b.idx);
        if landed > 0 {
            let took = self.boxes.spend_units(b.kind, b.cat, b.idx, landed);
            // ★ Loaned, like a materialise (97차): the bag reaches disk only on a GameSave,
            //   so the file keeps showing the unit until then; a game over that goes to the
            //   title saves nothing, the next load folds the loan back (108차), and the box
            //   has its unit again. A revive that is saved later clears the loan. Never a loss.
            self.loan.add(b.kind, b.cat, b.idx, landed);
            // ★ And the absorbed ledger dies, for `mat_frame`'s reason (106차): the unit
            //   lands in a cell a sweep may have emptied.
            if !self.boxes.absorbed().is_empty() {
                let n = self.boxes.absorbed().len();
                self.boxes.clear_absorbed();
                log::info!("bagbox: a revive supply left the box — the absorbed ledger ({n} cell(s)) no longer describes it");
            }
            self.mark_dirty();
            self.flush();
            log::warn!("bagbox: {name} x{landed} is in the bag for the game's revive check (box debited {took}, loaned until the next save)");
        }
        if landed < u32::from(b.sent) {
            self.revive_retry_at = frame + REVIVE_RETRY_FRAMES;
            log::info!(
                "bagbox: the bag took {landed} of {} {name} — the rest stays in the {} for now",
                b.sent,
                b.kind.title()
            );
        }
    }

    /// The field proper — where a host-issued guest call (the grant's `giveItem`) is safe.
    fn in_field(&self, emulator: &mut dyn Emulator) -> bool {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::in_field_state(&read)
    }

    /// Empty the BOXED quickslot plates whose consumable is gone — none in the box and
    /// none left in the bag either (the sweep has just run, so a bag stack still here is
    /// one the box could not take). The host-side twin of the game's own unbind of a
    /// used-up cell; see [`quickpage::Pages::drop_exhausted_boxed`].
    fn drop_exhausted_plates(&mut self, emulator: &mut dyn Emulator, data_dir: &Path, pages: &mut quickpage::Pages) {
        let cells = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_cells(&read)
        };
        // ⛔ An unreadable bag is not an empty one: nothing is dropped on a failed read.
        let Some(cells) = cells else { return };
        let gone = |cat: u8, idx: u8| {
            let (cat, idx) = (u16::from(cat), u16::from(idx));
            self.count_of(cat, idx) == 0 && !cells.iter().any(|c| c.cat == cat && c.idx == idx && c.count > 0)
        };
        let n = pages.drop_exhausted_boxed(&gone);
        if n > 0 {
            crate::config::write_quickpages(data_dir, pages);
            log::warn!("bagbox: {n} quickslot plate(s) emptied — their consumable is used up (none in the box, none in the bag)");
        }
    }

    /// Queue the game's own floating "you got it" name for an item the box just took.
    ///
    /// ★★★ 103차 — the player's report: "3종 가방으로 들어가는 재료들은 드랍되서 주워도
    /// 획득했다는 안내가 안 나가고 있어. 장비는 정상적으로 나가고 있어." Both entry hooks
    /// return to the caller the moment the box accepts, and the block that appends the name
    /// to the notice queue sits after the placement they replaced (`giveItem` `0x52e04` /
    /// `0x53114`, `addItemToInventory` `0x134c4`). Equipment kept its notice because the
    /// boxes never take equipment — which is precisely the asymmetry that was reported.
    ///
    /// The name is the one the box window and the bag cell show
    /// ([`bagbox::name_of`] → `display_name`), colour marker included so the renderer tints
    /// the line the way the game tints it.
    /// ⚠ For an orb that is one word MORE than the game would have queued: `giveItem`
    /// copies the raw DB row (`투신의`) while `formatItemName 0x6b90` is what appends the
    /// category word. Deliberate — the player never sees a bag cell for a boxed item any
    /// more, so the notice has to agree with the box window, and a bare `투신의` rising over
    /// the player would be a worse regression than the one being fixed.
    ///
    /// `true` = the name went into the queue (165차: a folded ring entry is several
    /// acquisitions, and the drain stops asking once the queue is full).
    fn announce(&self, emulator: &mut dyn Emulator, cat: u16, idx: u16) -> bool {
        let (marker, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
        let writes = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bagbox_notice_writes(&read, &format!("{marker}{name}"))
        };
        // No queue room, no grid this frame: the item is in the box either way. A notice is
        // not worth a retry — by the time one landed the player would have walked on.
        let Some(writes) = writes else { return false };
        for (a, bytes) in writes {
            if !emulator.write_memory(a, &bytes) {
                log::error!("bagbox: the 획득 notice for {name} could not be written at {a:#x}");
                return false;
            }
        }
        true
    }

    /// Put the three bag-item cells in bag 1, cells 1–3 (user, 2026-09-14: *"항상 인벤토리의
    /// 맨첫번째 가방 1,2,3번 칸에 있어야해. 고정이야 이건"*).
    ///
    /// ★★ THE WHOLE PLAN IN ONE POLL (2026-09-16, player: *"1,2,3번째칸에 가방3종이 고정으로
    /// 있어야하는데 가방3종이 이상한데 위치하고있는데?"*). It used to do ONE swap per poll, and a
    /// poll is 30 frames AND gated on the field being quiet (`poll`'s `quiet`: in a window,
    /// nothing moves at all). So a freshly created character whose three cells had just been
    /// granted needed three quiet field polls to seat them — and opening the bag before that
    /// showed them scattered wherever `findSlot` had dropped them, which is what the player
    /// saw. Three swaps is still only six record writes; there is no reason to spread them.
    ///
    /// ⛔ RE-READ between swaps. `cell_swap_writes` resolves addresses from live memory and
    /// each swap moves two records, so a plan computed once and applied blind would aim the
    /// second swap at where the first one used to be.
    /// ⛔ Bounded by the number of kinds: `pin_plan` is a permutation and shrinks by one
    /// seat each time, so it terminates — the cap is a guard against a swap that silently
    /// does not take, not part of the algorithm.
    ///
    /// ⛔ AFTER the grant, never before: a cell the bag does not hold yet has no seat to
    /// be put in, and the grant is what creates it.
    /// ⛔ The sweep runs first for the same reason it always did — a box-category cell
    /// still in the bag may be sitting in one of the three seats, and moving it out is
    /// cheaper than swapping it sideways.
    fn pin(&mut self, emulator: &mut dyn Emulator) {
        for _ in 0..Kind::ALL.len() {
            let cells = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::bag_cells(&read)
            };
            let Some(cells) = cells else { return };
            let Some(sw) = bagbox::pin_plan(&cells).first().copied() else { return };
            let Some(kind) = cells
                .iter()
                .find(|c| (c.tab, c.slot) == sw.from)
                .and_then(|c| Kind::from_bag_item(c.cat, c.idx))
            else {
                return;
            };
            let writes = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::cell_swap_writes(&read, sw.from, kind.bag_item(), sw.to)
            };
            let Some(writes) = writes else {
                log::info!("bagbox: the {} cell moved before it could be pinned", kind.title());
                return;
            };
            let mut ok = true;
            for (a, bytes) in &writes {
                if !emulator.write_memory(*a, bytes) {
                    log::error!("bagbox: pin write failed at {a:#x}");
                    ok = false;
                    break;
                }
            }
            log::warn!(
                "bagbox: pinned the {} cell to bag {},{} (was {},{} — {} write(s), ok = {ok})",
                kind.title(),
                sw.to.0 + 1,
                sw.to.1 + 1,
                sw.from.0 + 1,
                sw.from.1 + 1,
                writes.len()
            );
            if !ok {
                return;
            }
        }
    }

    /// May the sweep / grant / pin touch the bag right now? The field with no window and no
    /// popup (the original rule), OR — since 100차 — the online screens (앱 상태 3 with a
    /// loaded character: the 온라인 슬롯 목록 and the 싱글가방 / 창고 / 캐시샵 panels), again
    /// with no game popup up. The online panels paint the bag records live, so a cleared
    /// cell simply stops being drawn; the game's own deposit arm re-reads the record under
    /// the cursor on OK and answers a cleared one with nothing. Screens 2 and 3 only: the
    /// title and the 게임시작 list (1 / 9) hold a resident character too, but nothing
    /// there shows a bag, and the sweep is not needed where it cannot be seen.
    ///
    /// ★★★ 129차 (128차 code review ①): in the field, NOT while the game has a box up
    /// (`items::game_box_up` — the shared choice widget, a `showModal` box). Those leave
    /// the window id at `WIN_NONE` and the selector at 0, so `in_field_state` alone read
    /// a 귀환서's 「이동하겠습니까?」 as a quiet field and the sweep took the staged unit
    /// back into the box under the question — 예 then removed an empty (or re-granted
    /// bag-item) cell and warped. The use job now holds the cell through the question
    /// (`UsePhase::WaitField { asked }`); this is the second net, for the sweep, the grant
    /// and the pin alike: nothing of ours moves in the bag while the player is answering
    /// the game. The online screens keep their old rule — the load-edge migration must
    /// beat the `04 3d` ack's saveGame there, and that path shows a modal while it waits.
    fn quiet(&self, emulator: &mut dyn Emulator) -> bool {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        if items::popup_list(&read).is_some() {
            return false;
        }
        if items::in_field_state(&read) {
            return !items::game_box_up(&read);
        }
        items::online_screen(&read) && matches!(items::shell_screen(&read), Some(2) | Some(items::SHELL_SCREEN_ONLINE))
    }

    /// Move box-category cells out of the bag: what the hooks let through (a save from
    /// before the boxes, a full ring, a full box at the time) and whatever a materialised
    /// unit left behind. Quickslot plates that pointed at such a cell become BOXED plates
    /// first, so they keep their identity instead of going `ItemMoved`.
    ///
    /// ★★★ 100차 — THE WHOLE PLAN IN ONE PASS, and the absorbed LEDGER. Every plannable
    /// cell in every bag goes in one poll: the box file is written ONCE with all of them,
    /// then each cell is cleared (box first, cells second — the invariant is unchanged, it
    /// just holds for the batch). On the first sweep after a load (`ledger_live`) a cell
    /// the ledger says the box already holds ([`bagbox::Boxes::absorbed`] — a stale save
    /// reloaded) is cleared without being added; every sweep ADDS what it cleared to the
    /// ledger (a union, emptied by the next GameSave commit — see `frame`). No
    /// announcement any more: the migration runs at the load edge, before the bag can be
    /// opened, and the boxes are pinned in plain sight at bag 1, cells 1–3.
    fn sweep(
        &mut self,
        emulator: &mut dyn Emulator,
        data_dir: &Path,
        pages: &mut quickpage::Pages,
        window: &mut quickpage::Window,
        _proxy: &EventLoopProxy<UserEvent>,
    ) {
        let cells = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bag_cells(&read)
        };
        let Some(cells) = cells else { return };
        let use_ledger = self.ledger_live;
        let plan = bagbox::sweep_plan(&self.boxes, &cells, &ENABLED, use_ledger);
        if plan.is_empty() {
            // The bag was read and holds nothing to take: whatever the ledger names is not
            // in it, so no later sweep needs the ledger either.
            self.ledger_live = false;
            return;
        }
        let (actions, changed) = pages.migrate_to_boxed(window, &|cat| bagbox::kind_of(u16::from(cat)).is_some_and(|k| ENABLED.contains(&k)));
        if changed {
            crate::emulator::apply_quickpage_actions(emulator, &actions);
            crate::config::write_quickpages(data_dir, pages);
            log::warn!("bagbox: {} quickslot plate(s) now point at the box instead of a bag cell", actions.len());
        }
        // 1. The writes for every cell, read NOW — a cell that changed since the plan was
        //    made is dropped from this pass (the next poll sees the bag as it is then).
        //    ★ 129차 (128차 code review ⑤): the cell the restage knob names — a staged
        //    unit a job left behind (`JobEnd::Unknown`) — is not CLEARED but given its
        //    bag-item record back (the knob's own snapshot), and the knob is disarmed
        //    with it. The box is credited the same way; what changes is that the grant
        //    and the pin have nothing to do afterwards — no `giveItem` into whatever cell
        //    `findSlot` picks, no swap back into seat 1–3.
        let knob = wie_util::stats::bagbox_restage().filter(|_| !self.stage_held());
        let is_knob_cell = |c: &bagbox::BagCell| {
            knob.as_ref()
                .is_some_and(|k| (k.tab, k.slot) == (c.tab, c.slot) && (u16::from(k.cat), u16::from(k.idx)) == (c.cat, c.idx))
        };
        let mut batch: Vec<(bagbox::Sweep, Vec<(u32, Vec<u8>)>, bool)> = Vec::with_capacity(plan.len());
        for sw in &plan {
            let restore = is_knob_cell(&sw.cell);
            let writes = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                match (restore, knob.as_ref()) {
                    (true, Some(k)) => {
                        let st = items::BagStage {
                            tab: k.tab,
                            slot: k.slot,
                            rec: k.rec,
                            snapshot: k.bytes,
                        };
                        items::bag_stage_restore_writes(&read, &st, sw.cell.cat, sw.cell.idx)
                    }
                    _ => items::cell_clear_writes(&read, &sw.cell),
                }
            };
            match writes {
                Some(w) => batch.push((*sw, w, restore)),
                None => log::info!(
                    "bagbox: sweep skipped bag {},{} — it changed since it was read",
                    sw.cell.tab,
                    sw.cell.slot
                ),
            }
        }
        if batch.is_empty() {
            return;
        }
        // 2. The box, on disk, ONCE for the whole batch. Anything the file will not take
        //    puts the box (and loan) back and clears no cell at all.
        let before = self.boxes.clone();
        let loan_before = self.loan.clone();
        let mut absorbed: Vec<bagbox::BagCell> = Vec::with_capacity(batch.len());
        let mut added = 0usize;
        let mut deduped = 0usize;
        batch.retain(|(sw, _, _)| {
            let c = sw.cell;
            if sw.already_in_box {
                deduped += 1;
                absorbed.push(c);
                return true;
            }
            let left = self.boxes.add(sw.kind, c.cat, c.idx, u32::from(c.count));
            if left > 0 {
                // Planned against a simulation that has since diverged (the intake ring
                // landed something in between): leave this cell for the next poll.
                log::info!(
                    "bagbox: bag {},{} no longer fits the {} — left for the next poll",
                    c.tab,
                    c.slot,
                    sw.kind.title()
                );
                return false;
            }
            // ★ Reclaimed materials came BACK from the bag: drop them from the loan so the
            //   file total is unchanged by the move (loan -> real box). Extra beyond the
            //   loan (materials the player had from elsewhere) leaves the loan at 0.
            self.loan.sub(sw.kind, c.cat, c.idx, u32::from(c.count));
            added += 1;
            absorbed.push(c);
            true
        });
        if batch.is_empty() {
            self.boxes = before;
            self.loan = loan_before;
            return;
        }
        self.boxes.extend_absorbed(absorbed);
        self.mark_dirty();
        self.flush();
        if self.dirty {
            // The file did not take it: put everything back, leave every cell alone.
            self.boxes = before;
            self.loan = loan_before;
            self.dirty = false;
            log::error!("bagbox: the box file refused the sweep — no cell was cleared");
            return;
        }
        // 3. The cells. The bag has now been read and swept once since the load: from
        //    here on a cell at a ledger position is a new stack, never a stale one.
        self.ledger_live = false;
        let mut cleared = 0usize;
        for (sw, writes, restore) in &batch {
            let c = sw.cell;
            if *restore {
                // The knob is ours to disarm now: whatever the write does to the cell,
                // the hook must not answer a later removal there with THIS snapshot.
                wie_util::stats::clear_bagbox_restage();
            }
            let mut ok = true;
            for (a, bytes) in writes {
                if !emulator.write_memory(*a, bytes) {
                    log::error!("bagbox: sweep write failed at {a:#x} — the cell may now be half-cleared");
                    ok = false;
                    break;
                }
            }
            cleared += usize::from(ok);
            let (_, name, _) = bagbox::name_of(crate::bagbox_names::NAMES, c.cat, c.idx);
            log::info!(
                "bagbox: swept {name} x{} from bag {},{} into the {} ({}, ok = {ok}{})",
                c.count,
                c.tab + 1,
                c.slot + 1,
                sw.kind.title(),
                if sw.already_in_box {
                    "already in the box — stale save, not added again"
                } else {
                    "added"
                },
                if *restore { "; a staged leftover — its bag-item record is back over the cell, knob disarmed" } else { "" }
            );
        }
        log::warn!(
            "bagbox: sweep cleared {cleared} cell(s) across the bag — {added} added to the boxes, {deduped} already there (a reloaded save{}); ledger now {} cell(s) until the next GameSave",
            if use_ledger { "" } else { ", ledger not consulted" },
            self.boxes.absorbed().len()
        );
    }

    /// Give the bag-item cells the bag is missing with the game's own `giveItem` (the
    /// grant primitive `wie_grant.txt` uses). The intake hook leaves these three alone
    /// (`BAGBOX_KEEP_IDX`), so the give lands in the bag.
    ///
    /// ★ ALL the missing cells in one burst, as many as the bag has free cells for — one
    /// per poll left a new character without its third cell for three quiet polls (TODO
    /// `CP` ⓐ). The gives are asynchronous, so the pin cannot follow in this call:
    /// `grant_wait` makes `frame` do it the moment the cells are seen in the bag.
    fn grant(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        let Some(f) = crate::guest_build::give_item_fn() else { return };
        let missing = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::bagbox_missing_all(&read, &Kind::ALL)
        };
        let Some((missing, free)) = missing else { return };
        if missing.is_empty() {
            self.grant_wait = None;
            return;
        }
        if self.grant_wait.is_some_and(|fr| frame.saturating_sub(fr) < GRANT_INFLIGHT_FRAMES) {
            return;
        }
        if free == 0 {
            return;
        }
        for &(cat, idx) in missing.iter().take(free) {
            emulator.call_guest_function(f, &[u32::from(cat), u32::from(idx), u32::MAX, 0]);
            let kind = Kind::from_bag_item(cat, idx).map_or("?", Kind::title);
            log::warn!("bagbox: giving the {kind} cell (0x52b14 cat {cat:#x} idx {idx:#x})");
        }
        self.grant_wait = Some(frame);
    }

    /// The tail of a grant: once every bag-item cell is in the bag, pin them right away
    /// instead of waiting for the next 30-frame poll. Only in the quiet field, like the
    /// poll — a swap under an open inventory would move a cell the game's cursor or popup
    /// is pointing at. Gives up (and lets `poll` try again) when the burst never landed.
    fn pin_when_landed(&mut self, emulator: &mut dyn Emulator, frame: u64) {
        let Some(since) = self.grant_wait else { return };
        let (missing, quiet) = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            (
                items::bagbox_missing_all(&read, &Kind::ALL),
                items::in_field_state(&read) && items::popup_list(&read).is_none(),
            )
        };
        let expired = frame.saturating_sub(since) >= GRANT_INFLIGHT_FRAMES;
        match missing {
            // Landed: pin at the first quiet frame (the player may have the bag open
            // already — then the next quiet poll does it, as before).
            Some((m, _)) if m.is_empty() && quiet && self.job.is_none() && self.mat.is_none() && self.dialog.is_none() => {
                self.grant_wait = None;
                self.pin(emulator);
            }
            _ if expired => self.grant_wait = None,
            _ => {}
        }
    }
}
