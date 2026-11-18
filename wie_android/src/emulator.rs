//! Emulator worker thread. Owns the `Emulator` + `Platform` and drives the
//! tick / redraw / key-repeat loop off the UI/JNI thread on a large stack (the
//! core-compat audit's R2 requirement). Combines wie_cli's key-repeat loop with
//! wie_web's adaptive multi-tick budget.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    thread,
    time::{Duration, Instant},
};

use winit::event_loop::EventLoopProxy;

use wie_backend::{Emulator, Event, KeyCode, Options, Platform, extract_zip};
use wie_j2me::J2MEEmulator;
use wie_ktf::KtfEmulator;
use wie_lgt::LgtEmulator;
use wie_skt::SktEmulator;

use crate::{
    InputEvent, QuickSlot, Stats, UserEvent, config, items, save,
    platform::{AndroidPlatform, AndroidScreen},
    quickpage,
};

/// Emulator worker stack. `EmulatedMemory.pages` alone is a 512 KB by-value
/// temporary during construction, plus interpreter recursion depth.
///
/// ★ 8 MB -> 64 MB (2026-08-06). This is MITIGATION, not a fix, and it is worth being
/// precise about which: five device tombstones (2026-08-05, 01:58 / 02:32 / 03:01 /
/// 03:24 / 03:57) are byte-identical in shape — `SIGSEGV / SEGV_ACCERR`, `Cause: stack
/// pointer is not in a rw map; likely due to stack overflow`, thread `wie-emu`, and a
/// backtrace of TWO addresses alternating for all 511 frames the tombstone will unwind.
/// Something in the future chain grows with elapsed play time and nothing else; three
/// in-process gauges have now RULED OUT the obvious layers rather than found it
/// (`ArmCoreThreadWrapper` nesting is 1 by construction, `ArmCore::run_function`
/// nesting tops out at 21, the executor's task count stays under 8).
///
/// A thread stack is reserved address space, not memory: on 64-bit Android the pages
/// commit only as they are touched, so the cost of the extra 56 MB is zero until the
/// bug that needs it happens. It buys roughly 8x the time to the crash — the observed
/// interval was 21-35 minutes — while [`stack_rss_kb`] measures the growth on the
/// device so the next session can name it instead of ruling things out.
const EMU_STACK: usize = 64 * 1024 * 1024;

/// Resident (touched) bytes of the mapping this thread's stack lives in, in KB.
///
/// ★ The one gauge that cannot measure the wrong layer. Every counter we can put
/// *inside* the code assumes it knows which function repeats; this asks the kernel how
/// much of the stack has ever been written, which is exactly the quantity that hits the
/// guard page. Linux never decommits stack pages, so it is a high-water mark.
///
/// `/proc/self/smaps` is not cheap to parse (tens of mappings), hence the ~30 s cadence
/// at the single call site — never inside the frame budget's hot path.
#[cfg(target_os = "android")]
fn stack_rss_kb() -> Option<u64> {
    let probe = 0u8;
    let sp = &probe as *const u8 as usize;
    let maps = std::fs::read_to_string("/proc/self/smaps").ok()?;
    let mut in_ours = false;
    for line in maps.lines() {
        if let Some((range, _)) = line.split_once(' ')
            && let Some((lo, hi)) = range.split_once('-')
            && let (Ok(lo), Ok(hi)) = (usize::from_str_radix(lo, 16), usize::from_str_radix(hi, 16))
        {
            // A header line: it either starts our mapping or ends it.
            in_ours = (lo..hi).contains(&sp);
            continue;
        }
        if in_ours && let Some(rest) = line.strip_prefix("Rss:") {
            return rest.trim().trim_end_matches(" kB").trim().parse().ok();
        }
    }
    None
}

#[cfg(not(target_os = "android"))]
fn stack_rss_kb() -> Option<u64> {
    None
}

/// Adaptive per-frame emulation budget (mirrors wie_web/src/rust/lib.rs).
const FRAME_BUDGET_MS: u128 = 24;
const TICK_FULL_MS: u128 = 6;
/// Target UI cadence; light frames sleep the remainder.
const FRAME_PACE: Duration = Duration::from_millis(16);
/// Guest key auto-repeat interval (matches wie_cli / wie_web).
const KEYREPEAT_MS: u128 = 100;

/// ★★★ 2026-09-23 — a HELD attack key is handed to the guest again whenever the previous
/// action has ended: the same missing edge the walk order supplies, for the same reason.
/// `handleCletEvent` throws KEY REPEATED away, so a thumb held on OK produces exactly ONE
/// swing no matter how long it stays down. Once 공격속도 made an attack two or three ticks
/// long, re-pressing by hand became the real limit on how fast a player could fight
/// (player: "공격속도가 너무 빠르면 ok키를 눌러 공격하기가 힘들어지는 경우가 존재해서 …
/// 꾹누르면 공격을 연속적으로 계속하도록").
///
/// ★★★ 132차-B (2026-09-24): WHEN is `atkspeed::AttackHold`, and it is stricter than the
/// first rule — OK also talks, and an OK three frames after a dialogue closes opens it
/// again, so a hold that has seen any UI never re-presses. See that type.
///
/// The key a held thumb means "attack again" by: OK, or the 보조무기 key `7`.
///
/// ⛔ Deterministic and OK-first — a `HashMap` has no order, and a player holding both must
/// not alternate between two different attacks frame to frame.
fn attack_key_held(held: &HashMap<KeyCode, Instant>) -> Option<KeyCode> {
    [KeyCode::OK, KeyCode::NUM7].into_iter().find(|c| held.contains_key(c))
}

/// The touch stick's standing walk order: the direction the thumb is holding, plus when
/// a re-press is allowed again.
///
/// ★★★ 2026-09-20 — see `InputEvent::Walk` and `items::walk_idle_on_field`. The guest's
/// walk runs from a keydown EDGE to the matching keyup and polls nothing in between, and
/// `handleCletEvent` discards KEY REPEATED outright, so `held`'s auto-repeat above cannot
/// restart a walk that an attack, a skill or a wall ended. This does — by supplying the
/// one thing that is missing, another keydown, and only while the character is standing
/// still on the field.
///
/// ⛔ This does NOT enter `held`: the UI owns the real press/release pair, and putting a
/// synthetic keydown in there would make the final keyup depend on how many times we
/// re-pressed.
#[derive(Default)]
struct WalkOrder {
    dir: Option<KeyCode>,
    /// `None` = may re-press now. Rate-limited at [`KEYREPEAT_MS`] so a thumb held
    /// against a wall (state stays 0 there — measured) costs the same as the auto-repeat
    /// it sits beside, not one key per frame.
    next: Option<Instant>,
}

/// How often the minimap panel is re-rendered from the guest's map arrays
/// (`items::render_minimap`), in frames — ~4x/s at the 16 ms pace. The render is a
/// couple of small guest reads plus a few thousand pixels, so the cadence is about
/// not being wasteful rather than about cost.
///
/// ★ 2026-08-03: this REPLACED a ~250-line state machine that drove the guest's own
/// map window open and shut behind a frozen screen, because that was the only way to
/// make the guest repaint the buffer we used to mirror. The host now draws the same
/// picture itself, so there is no window to hide, nothing to freeze, no keys sent on
/// the player's behalf, and the panel follows the player through a door with no
/// refresh step at all. See the block comment on `items::render_minimap` for the
/// decoded guest routine and the pixel-exact verification.
const MM_RENDER_EVERY: u64 = 15;

/// ── ★ DIAGNOSTIC GATE (2026-08-04, user decision) ───────────────────────────────
///
/// 17차's "퀵바등록" feature rewrites the game's own popup label buffers in guest RAM
/// and calls a guest function to commit — see PLAY_FEEDBACK_FIXES.md §17.
///
/// ★ ON since 2026-08-04 (20차). Both reasons it was off have expired:
///
///  1. It was held back so a brand-new set of host->guest writes would not land in the
///     same build as the stack-overflow diagnostics (18차) or the 19차 save work — a
///     changed crash signature would have been unattributable. 19차 is now
///     device-verified, so there is nothing left to confuse it with.
///  2. Its ONE hard prerequisite is now met. §17 claimed the commit path was "already
///     proven by useItemEffect"; that was wrong, and 19차 proved it wrong: the very
///     first host-originated `call_guest_function` on a device PANICKED, because the
///     JVM identifies threads by executor task id and host-originated calls were never
///     `attach_thread`ed. This feature's commit IS a `call_guest_function`, so before
///     `db2c704` turning it on could only ever have crashed. That fix is in, and a
///     save round-trip through the same path is device-verified.
///
/// Gating the publish is sufficient to gate the whole feature: with no `UserEvent::Popup`
/// ever sent, the UI's `popup` stays `None`, so OK is never diverted to
/// `InputEvent::PopupConfirm` and none of the bind path can be reached.
const POPUP_QUICKBIND: bool = true;

/// Wall-clock UNIX milliseconds. Cash-shop buffs are stored as absolute stamps so a
/// "1 week" buff keeps running down while the app is closed; a monotonic clock would
/// restart at every launch and make the week unbounded.
///
/// A clock set backwards can only ever make a buff look longer, never shorter, and a
/// clock before the epoch yields 0 (= expired) rather than panicking.
/// Leaf guest PC -> sample count, filled by the profiler callback (see `Options` in
/// [`run`]) and drained by the 30 s gauge. `std::sync::Mutex` rather than a lock-free
/// structure because it is touched once per FLUSH BATCH, not per sample.
static PROFILE_HIST: std::sync::Mutex<std::collections::BTreeMap<u32, u64>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// FNV of the last `items::audio_opts_audit` line, so it is logged only when it CHANGES.
/// See the call site — that line is how the game's 배경음/효과음 bytes get identified.
/// The 공격속도 percent last published to the guest hook, so the log line only speaks when
/// the number actually moves (gear on, gear off). `u32::MAX` = nothing published yet, so
/// the first poll always says what the run starts at.
static ATTACK_SPEED_LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
static AUDIO_OPTS_SIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// ★ 143차: the player answered 예 on 「하늘성에서 나가시겠습니까?」 (the EXIT tab,
/// `InputEvent::TowerExit`). `apply_input` has no `Run` to hand, so it leaves this note and
/// the tower poll takes it — see the EXIT arm there for what it does with and without a run.
static TOWER_EXIT_REQUEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// ★ 143차: the player is IN the tower — a run, a floor scene (a pre-143차 save made inside
/// one), or the wake — so the host quick save refuses. Written by the HUD poll with the
/// same value that hides the SAVE tab; read by `apply_input`, which the pad's quick-save
/// button reaches with no tab to hide.
static TOWER_NO_SAVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// ★ 143차: the way BACK after a run, as the landing block tracks it until it lands.
#[derive(Clone, Copy, Debug)]
struct ReturnLeg {
    /// The scene the leave warp was posted to.
    scene: u8,
    /// The tile to stand the player on once `scene` has loaded (the run's own start, or
    /// `guardian::fallback_home`'s). `None` = wherever the game put them.
    tile: Option<(u8, u8)>,
    /// The frame the request went out (or was last re-posted), and how many re-posts.
    posted: u64,
    reposts: u8,
}

/// ★ 143차: frames a posted way home may sit CONSUMED (the request byte back at 0) with the
/// scene word still on the floor before it is posted again, and how many times. The game
/// clears the byte before it dispatches, so "consumed, nothing loaded" is a request it ate;
/// a request still at 1 is only waiting (a window is open) and is left alone.
const RETURN_REPOST_FRAMES: u64 = 120;
const RETURN_REPOSTS: u8 = 3;

/// The `n` hottest guest addresses since the last call, as `addr:count` text, and
/// clears the histogram. Empty when the profiler is off.
fn profile_top(n: usize) -> String {
    let mut hist = match PROFILE_HIST.lock() {
        Ok(h) => h,
        Err(e) => e.into_inner(),
    };
    if hist.is_empty() {
        return String::new();
    }
    let total: u64 = hist.values().sum();
    let mut v: Vec<(u32, u64)> = hist.iter().map(|(a, c)| (*a, *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    let out = v
        .iter()
        .take(n)
        .map(|(a, c)| format!("{a:#x}={:.1}%", *c as f64 * 100.0 / total as f64))
        .collect::<Vec<_>>()
        .join(" ");
    hist.clear();
    format!(" | guest hot ({total} samples): {out}")
}

/// Take the profile histogram, leaving it empty, so the next tick's samples stand alone.
/// Paired with [`profile_merge_back`] — every take MUST be restored or the 30 s
/// aggregate silently loses everything before the take.
fn profile_take() -> std::collections::BTreeMap<u32, u64> {
    std::mem::take(&mut *PROFILE_HIST.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Summarise the samples taken since [`profile_take`] (i.e. exactly one tick's worth),
/// then fold them into `carry` and put the running total back.
///
/// `top == 0` skips the formatting, which is what every ordinary tick does — the swap
/// itself is two lock/move pairs per tick and costs nothing per SAMPLE, which is the
/// point: profiling one hitch must not change what the hitch costs.
fn profile_merge_back(carry: std::collections::BTreeMap<u32, u64>, top: usize) -> String {
    let mut hist = PROFILE_HIST.lock().unwrap_or_else(|e| e.into_inner());
    let out = if top == 0 || hist.is_empty() {
        String::new()
    } else {
        let total: u64 = hist.values().sum();
        let mut v: Vec<(u32, u64)> = hist.iter().map(|(a, c)| (*a, *c)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let hot = v
            .iter()
            .take(top)
            .map(|(a, c)| format!("{a:#x}={:.0}%", *c as f64 * 100.0 / total as f64))
            .collect::<Vec<_>>()
            .join(" ");
        format!(", hot pc ({total} samples): {hot}")
    };
    let mut merged = carry;
    for (pc, c) in hist.iter() {
        *merged.entry(*pc).or_insert(0) += *c;
    }
    *hist = merged;
    out
}

/// How long a save that is ALREADY in flight may take to finish before the app closes
/// regardless (the guest asked to quit while a SAVE / cash-item save was mid-chain).
///
/// Four queued guest calls at one per `System::tick`; 1.5 s is ~60 frames of slack
/// against a guest that is running normally, and a guest that is NOT running
/// normally is exactly what the player is trying to escape from.
///
/// ⛔ Nothing STARTS a save at quit any more (2026-09-16, 99차): the quit-time
/// `Reason::Auto` arm went with the 5-minute timer. In practice it never fired anyway —
/// 게임종료 lives on the title (app state 3), where the save gate always refused.
const EXIT_SAVE_GRACE_MS: u64 = 1_500;

/// How many consecutive HUD polls must agree that the script window is gone before the
/// cutscene control set is given up.
///
/// ★ 2026-08-21. This replaced a wall-clock deadline, and the reason is the shape of
/// the thing it is filtering. A page turn inside a dialogue drops the script window's
/// id for a moment and then puts it straight back; a scene that really ended never
/// puts it back. **Consecutive samples tell those two apart; elapsed time cannot** —
/// a deadline long enough to cover a slow blip is also long enough to hold the wrong
/// controls after a real exit, and a short one lets the blip through.
///
/// The cost of getting it wrong in the safe direction is ~2 extra polls (roughly
/// 100–150 ms) of cutscene controls after a scene ends — and usually not even that,
/// because leaving a scene lands on the FIELD and `field` outranks `cutscene` in the
/// mode match (`app.rs`), so the field set appears immediately anyway.
const CUTSCENE_EXIT_POLLS: u32 = 3;

/// Same idea for the script window's CHOICE flag, which raises the arrows on its own.
///
/// One sample is not enough to act on: the flag is written around the window's own
/// open/close (`items::script_choice_open`'s doc records `0xa0dc 1->0` happening in the
/// same transition as `0xa0c0 3->4`), so a single poll landing inside a page turn can
/// read a 1 that no choice box is behind. Requiring two agreeing polls costs ~50–70 ms
/// before a real choice's arrows appear, which is imperceptible, and it cannot hide a
/// real choice: that flag stays 1 until the player answers.
const CHOICE_CONFIRM_POLLS: u32 = 2;

/// How many `frame % 3` polls the commit-time count may stay in the client's row
/// before it is put back regardless of what the client did.
///
/// The normal path never waits that long — modal 7 (the 획득 box the grant itself
/// opens) restores it as soon as the row has been copied. This is the floor under
/// every OTHER ending: 아니오, a refused purchase, the player backing out, the app
/// being paused mid-confirm. A missed restore would leave the chosen count sitting in
/// the cache and the next purchase on that page would grant it unasked.
const CASH_QTY_GRACE_POLLS: u32 = 8;

/// The commit-time cash-count poke and how it is put back.
///
/// Lives across the worker loop because the two halves happen in different places by
/// necessity: the write must be issued from `apply_input`, before the key reaches the
/// guest (that is the only ordering the code guarantees — see `items::cash_qty_writes`),
/// and the restore must wait until the client has copied the row.
#[derive(Default)]
struct CashQtyPoke {
    /// `(addr, the byte the client itself put there)`.
    restore: Vec<(u32, u8)>,
    /// Polls remaining before the restore happens anyway.
    grace: u32,
}

/// Write the player's chosen count into the row the grant will read, and remember what
/// was there. Called on the commit key BEFORE it is handed to the guest.
fn cash_qty_commit(emulator: &mut dyn Emulator, cash: &mut CashQtyPoke) {
    let mut qty = crate::cash_qty();
    let writes = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        if !items::cash_buy_confirm_open(&read) {
            return;
        }
        // ★★★ BOUND THE COUNT BY WHAT THE BAG CAN HOLD (2026-08-25). For the five
        //   CLIENT-granted rows the grant is `0x13e4c` -> `0x131d0` + ONE retry whose
        //   return is discarded (`0x13ebe`) — the same two-placement machine the
        //   withdraw uses, and it destroys whatever it cannot place. Measured: a bag
        //   holding two 98-stacks with five free cells takes 2 of a 99 purchase and
        //   LOSES 97. The guest's own gate (`0x72dc`) is skipped entirely unless the bag
        //   is already full, so it saves nothing here.
        //
        //   ⛔ NOT for 오브상자 / 소켓확장: their real item is a cat-0x10 orb the ENGINE
        //   delivers, and `offline::bag_fits` is their check (it refuses the purchase
        //   outright rather than trimming it). `cash_qty_row_write` is exactly the
        //   predicate that splits the two families.
        //
        //   ★★ 166차: …unless the bag-box intake takes it first. `0x131d0`'s ENTRY hook offers
        //   the whole record to the host's box (소비가방 for every row this clamps) before the
        //   bag is looked at — all of it or nothing — so the count is safe up to whichever
        //   of the two takes MORE, not the bag's alone. A full bag clamped a box purchase to
        //   1 here once the guest's own gate stopped refusing it (`bagbox_cash_gate_lgt`).
        if let Some((_, cat, idx)) = items::cash_cursor_item(&read)
            && items::cash_qty_row_write(cat, idx)
        {
            let cap = u32::from(crate::CASH_QTY_MAX);
            let room = crate::offline::bag_room(&items::bag_view(&read), cat, idx);
            let boxed = wie_util::stats::bagbox_room_for(u32::from(cat), u32::from(idx), cap);
            qty = qty.min(room.max(boxed).max(1).min(cap) as u8);
        }
        // ★ LATCH FIRST, and even for a plain 1. The two purchases the ENGINE delivers
        //   (오브상자 / 소켓확장) have no client row to write, so their count is read
        //   when the `05 42` frame arrives — later than this, and after the picker may
        //   have gone. Latching here is the only moment that is certainly before the
        //   guest can send anything. Latching 1 as well is what stops a previous
        //   confirm's number leaking into a purchase the player did not set one for.
        crate::commit_cash_qty(qty);
        if qty <= 1 {
            Vec::new()
        } else {
            items::cash_qty_writes(&read, qty)
        }
    };
    if writes.is_empty() {
        return;
    }
    // Anything still outstanding goes back FIRST. Two commits without a restore in
    // between would otherwise strand the older original and the row would keep a count
    // the client never wrote.
    for (addr, old) in cash.restore.drain(..) {
        emulator.write_memory(addr, &[old]);
    }
    for (addr, new, old) in writes {
        if emulator.write_memory(addr, &[new]) {
            cash.restore.push((addr, old));
        } else {
            log::warn!("cash qty: write to {addr:#x} failed — this purchase grants the row's own count");
        }
    }
    cash.grace = CASH_QTY_GRACE_POLLS;
    log::info!("cash qty: {qty} written to {} row(s) on commit", cash.restore.len());
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A save in flight, plus the slot the current guest call's return value lands in.
///
/// One at a time by construction: a second serialise while the first buffer is still
/// unwritten would leak that buffer and could interleave two records into one file.
struct SaveState {
    job: Option<save::Job>,
    result: Arc<wie_backend::GuestCallResult>,
    // ⛔ `last_ms` (the autosave clock) WAS HERE and is gone with the 5-minute timer
    // (2026-09-16, 99차). No field of this struct measures time between saves any more.
    /// A save that could not start yet, kept trying until it lands or expires:
    /// `(reason, expires_at_unix_ms)`.
    ///
    /// ★ 2026-08-07, and it is the fix for the "가끔 SAVE를 누르면 CANNOT SAVE HERE"
    /// report. A refusal has three causes, and only ONE of them deserves that toast:
    ///  1. the AREA forbids saving (`canSave` byte — boss rooms): a real "no";
    ///  2. a save is already in flight (a cash-item save, or a second tap): try again
    ///     in a moment;
    ///  3. the moment is wrong but the place is fine (a window still closing, the
    ///     selector flag, the UI's field flag up to ~80 ms stale): also try again.
    /// The old code toasted for all three, so a save racing the button — or a tap
    /// straight after closing the menu — read as "this place cannot save", which is
    /// exactly what the player reported. Now cause 1 answers immediately, causes 2/3
    /// arm this retry and the toast only fires if the retry window runs dry.
    ///
    /// Also carries the [`save::Reason::Item`] retries (cash-item use), whose window is
    /// minutes: the use happens with the inventory open, and the save fires once the
    /// player is back on the field.
    retry: Option<(save::Reason, u64)>,
    /// `vault::refusal_count()` when the current step was issued (72차, F-05). A change
    /// across a step means the host refused a character-record write while that step was
    /// running, and the chain must not go on to write the other half of the pair.
    refusals_at_issue: u64,
    /// When the disk was last asked whether a slot row lost its summary (72차-D). 0 =
    /// never, so the first check happens on the first eligible poll after launch.
    summary_checked_ms: u64,
    /// Summary rebuilds attempted this process — see [`SUMMARY_REPAIR_MAX`].
    summary_repairs: u8,
}

impl SaveState {
    fn new() -> Self {
        Self {
            job: None,
            result: Arc::new(wie_backend::GuestCallResult::new()),
            retry: None,
            refusals_at_issue: 0,
            summary_checked_ms: 0,
            summary_repairs: 0,
        }
    }
}

/// How long a refused MANUAL save keeps retrying before the player is told no.
/// Covers a save already in flight (four guest calls, well under a second) and every
/// transient window/flag state with slack to spare, while still answering fast.
const MANUAL_RETRY_MS: u64 = 3000;
/// How long a cash-item save keeps trying. Minutes, not seconds: the player has to
/// close the inventory first, and using one mid-boss-room (부활서) legitimately
/// cannot save until the room is cleared — if they leave within this window the
/// consumption still gets committed. ★ Since 2026-09-16 there is no autosave behind
/// it: if the window runs dry the use is committed by the player's next SAVE, and a
/// crash before that rolls BOTH halves back (`config::read_cash_baseline` at boot),
/// which is consistent rather than a duplicate.
const ITEM_RETRY_MS: u64 = 10 * 60 * 1000;

/// How long after a combine the bag-wide derived-cache rebuild runs (71차-F).
///
/// It has to be AFTER the game's own popup close, because that is what runs the
/// inventory compactor (`0x55048`) — rebuilding before it would just be the same race
/// the single targeted call already loses. A quarter second is far longer than the
/// close takes and far shorter than the player can reach the 장비 screen.
const ORB_REFRESH_DELAY_MS: u64 = 250;
/// How many sweeps. Two, spaced: the first catches the normal case, the second covers a
/// compaction that happened late (the player closing another window on the way out).
/// ⛔ Not a poll loop — each sweep is one pass over the bag and then it stops.
const ORB_REFRESH_PASSES: u8 = 2;
/// Upper bound on refresh calls in one sweep. 5 tabs x 16 cells is the whole bag; the
/// cap exists so a corrupt bag-count byte cannot turn this into a flood of guest calls.
const ORB_REFRESH_MAX_CELLS: usize = 80;
/// How many times a sweep that found NO equipment may try again before giving up.
///
/// ★★★ 71차-L. A sweep used to consume a pass whether or not it saw anything, and the
/// one path that matters can legitimately see nothing: combining REQUIRES the item to be
/// unequipped, unequipping drops `hp_max` below `hp`, and `resolve_char` used to call
/// that "no character" (TODO `AP`). Both passes then evaporated over an empty bag and the
/// numbers stayed stale for ever. `resolve_char` no longer has that gate, but a sweep that
/// comes back empty is still evidence that the read failed rather than that the player
/// owns no equipment — so it retries instead of spending the pass. 8 x 250 ms = 2 s.
const ORB_REFRESH_EMPTY_RETRIES: u8 = 8;

/// Consecutive menu-less polls before the in-game combine picker gives up and hands the
/// orb back. Three at the `frame % 5` cadence is about a quarter of a second — long enough
/// to ride out the popup still being open on the poll after the OK key (and any transient
/// unresolvable read), short enough that leaving the inventory takes the dialog with it.
const ORB_EXIT_POLLS: u32 = 3;

/// ★★★ 104차 ①: how many extra (deliver the job's keys → tick → step the job) passes one
/// worker iteration may spend finishing the bag-box 사용하기 job. Three is the whole job
/// from the box window — OK opens the game's own popup, row 0 is 사용하기, the second OK
/// takes it — and a fourth pass has nothing left to carry.
///
/// ⛔ A BOUND, not a schedule: each pass stops the moment a step produced no keys, so a
/// phase still waiting on the guest costs nothing and falls back to the ordinary
/// once-per-iteration cadence with its own deadline (`bagbox_host::USE_WAIT_MS`).
const BAGBOX_JOB_STEPS: usize = 3;

/// A skill-plate tap being verified against guest state (2026-08-07). The plate's
/// hotkey has already been sent; the question is whether the game's own gates (idle,
/// SP cost, range) let the cast START — only then does the UI get a
/// [`UserEvent::SkillCast`] and only then does a cooldown begin. Two independent
/// signals, either one confirms: SP dropped since the tap (every real skill spends
/// some — read with `items::read_sp`, not `read_stats`), or the combo state machine
/// is visibly running a skill action (`items::skill_cast_active`).
struct SkillWatch {
    /// The PLATE the player pressed, not the guest slot it borrowed — the UI draws the
    /// cooldown on a plate (`quickpage.rs`).
    slot: u8,
    /// SP at the moment of the tap (`items::read_sp`). `None` = CHAR/SP unread;
    /// the state-machine signal alone decides.
    sp0: Option<u16>,
    /// Whether a skill action was ALREADY running at the tap. The state signal only
    /// confirms on a RISING edge: tapping plate B while plate A's animation is still
    /// playing must not read A's ongoing action as "B cast" — for that tap only an
    /// SP drop can confirm.
    active0: bool,
    deadline: Instant,
}

/// How long after the tap the guest gets to actually start the cast. The dispatcher
/// arms the cast on the very tick that consumes the key, so at the ~16 ms frame pace
/// this is dozens of polls of slack — while still short enough that an SP drop from
/// something unrelated (a monster draining it seconds later) cannot be mistaken for
/// the cast.
const SKILL_WATCH_WINDOW: Duration = Duration::from_millis(700);

/// A tick this long is a hitch the player can feel. The game re-arms its frame timer at
/// ~20 ms on the field, so a single tick past this has already eaten a whole frame.
///
/// ── WHY A PER-TICK DETECTOR AND NOT THE 30 s GAUGE (2026-08-15, 49차) ────────────
/// Player: "다른건 다 문제없는데 크리티컬이 터질때만 살짝 느려져." The throughput gauge
/// averages over 1800 frames, which is 30 SECONDS — a critical flash lasts a few hundred
/// milliseconds, so it is diluted roughly 100:1 and every window looks the same. The
/// first hypothesis off that gauge (the `blitlut565` transform 4~8 fallback, which
/// MIGRATION_LGT.md §2.20 left open) was refuted by it too: during combat the host draws
/// 98.8 % of blits and the transform refusals run at ~5/s against ~10,600 calls/s.
///
/// So measure the thing the player is actually reporting: catch the slow TICK and print
/// what moved inside it. The one number that splits the field in half is the guest
/// instruction delta — a 40 ms tick with millions of instructions is the interpreter
/// (i.e. a pixel loop we have not hosted), the same tick with a few thousand is host-side
/// work on this thread (sound parse, allocator, a blit refusal path).
const SLOW_TICK_MS: u128 = 24;

/// Minimum spacing between hitch reports. A hitch that repeats every frame would put ~80
/// lines/s into logcat, and logcat writes come out of THIS thread's frame budget — an
/// instrument that makes the stall it is measuring is worse than none.
const HITCH_REPORT_GAP: Duration = Duration::from_millis(400);

/// Counters sampled either side of one guest tick, so a hitch can be attributed instead
/// of guessed at. All are relaxed atomic loads (~20 of them), which is nothing against a
/// tick measured in milliseconds.
#[derive(Clone, Copy, Default)]
struct TickCounters {
    instr: u64,
    svc: u64,
    frames: u64,
    calls: u64,
    hle: u64,
    px: u64,
    px_hle: u64,
    /// Blits asking for a transform the hosted loops do not implement. Counted here (and
    /// not just in `refused`) because the two blitters SHARE the refusal array, so
    /// `refused[1]` alone cannot say it was the LUT blitter that gave up.
    transformed: u64,
    refused: [u64; 5],
    plays: u64,
    parses: u64,
    allocs: u64,
}

impl TickCounters {
    fn now() -> Self {
        let (instr, svc, _) = wie_util::stats::snapshot();
        let ([calls, hle, px, px_hle], _, transform, refused) = wie_util::stats::blit_snapshot();
        let audio = wie_util::stats::audio_snapshot();
        let (allocs, ..) = wie_util::stats::heap_snapshot();
        Self {
            instr,
            svc,
            frames: wie_util::stats::frames(),
            calls,
            hle,
            px,
            px_hle,
            // [0] is the identity, which every hosted blit uses; 1..=9 are the ones that
            // still run in the guest.
            transformed: transform[1..].iter().sum(),
            refused,
            plays: audio[1],
            parses: audio[2],
            allocs,
        }
    }
}

/// One line naming what a slow tick actually did. Read it like this: `inst` in the
/// millions = the guest interpreter (find the loop and host it); `inst` small with a big
/// `parses` = the sound path; `blit` short of `calls` = the host blitter refused and the
/// guest drew it the slow way, and `refused`/`transform!=0` say why.
fn report_hitch(took: Duration, a: &TickCounters, b: &TickCounters, hot: &str) {
    let d = |x: u64, y: u64| y.saturating_sub(x);
    let refused: Vec<u64> = (0..5).map(|i| d(a.refused[i], b.refused[i])).collect();
    log::warn!(
        "emu hitch: tick {:.1} ms, {} guest frame(s) — inst {}, svc {}, blit {}/{} calls ({} px, {} host px), \
         transform!=0 {}, refused [fmt {} tf {} geom {} mem {} null {}], sound plays {} parses {}, allocs {}{hot}",
        took.as_secs_f64() * 1000.0,
        d(a.frames, b.frames),
        d(a.instr, b.instr),
        d(a.svc, b.svc),
        d(a.hle, b.hle),
        d(a.calls, b.calls),
        d(a.px, b.px),
        d(a.px_hle, b.px_hle),
        d(a.transformed, b.transformed),
        refused[0],
        refused[1],
        refused[2],
        refused[3],
        refused[4],
        d(a.plays, b.plays),
        d(a.parses, b.parses),
        d(a.allocs, b.allocs),
    );
}

/// Issue one step of the chain, giving it a fresh result slot.
fn issue(emulator: &mut dyn Emulator, st: &mut SaveState, addr: u32, args: &[u32]) {
    st.result = Arc::new(wie_backend::GuestCallResult::new());
    emulator.call_guest_function_out(addr, args, st.result.clone());
}

/// Begin a save, if the guest is in a state where the game itself would allow one.
/// Returns false when it was refused (already saving, or the gate said no).
fn save_begin(emulator: &mut dyn Emulator, st: &mut SaveState, reason: save::Reason) -> bool {
    if st.job.is_some() {
        return false;
    }
    let names = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::save_gate(&read)
    };
    save_begin_named(emulator, st, reason, names)
}

/// [`save_begin`] with the gate already asked — `names` is `save_gate`'s answer, or (136차)
/// `items::sort_save_gate`'s for a 정렬 on the 네트워크 싱글가방 screen. `None` = refused.
fn save_begin_named(emulator: &mut dyn Emulator, st: &mut SaveState, reason: save::Reason, names: Option<(u32, u32)>) -> bool {
    if st.job.is_some() {
        return false;
    }
    let Some((name_game, name_slot)) = names else {
        return false;
    };
    // `save_gate` returning Some implies the chain exists, but resolve it explicitly —
    // the job carries the addresses it will call.
    let Some(chain) = crate::guest_build::save_chain_fns() else {
        return false;
    };
    let Some(job) = save::Job::new(reason, chain, name_game, name_slot) else {
        return false;
    };
    // Clear the out-parameter word before the serialise step writes it, so a stale
    // length from a previous save can never be mistaken for this one's.
    if !emulator.write_memory(chain.scratch, &0u32.to_le_bytes()) {
        log::warn!("save: could not clear the scratch word at {:#x}", chain.scratch);
        return false;
    }
    let save::Next::Call { addr, args, argc, .. } = job.first_call() else {
        return false;
    };
    st.job = Some(job);
    issue(emulator, st, addr, &args[..argc]);
    log::info!("save: started ({reason:?})");
    true
}

/// How often the host asks the DISK whether a slot row lost its summary. One
/// `read_dir` plus two `metadata` calls per character record — nothing next to the
/// `fsync` every save already does, but there is no reason to do it every frame either,
/// and the state it looks for is created by a save, not by playing.
const SUMMARY_CHECK_MS: u64 = 20_000;

/// How many summary rebuilds one process may attempt.
///
/// The healthy outcome is self-limiting (a written summary makes the next check answer
/// "no"), so this bounds the UNHEALTHY one: if the write is refused, or lands somewhere
/// the check does not look, this stops it becoming a guest call every twenty seconds for
/// the rest of the session. Four is enough to ride out a transient refusal and few enough
/// to be obvious in a log.
const SUMMARY_REPAIR_MAX: u8 = 4;

/// Carry out one [`crate::guardian::Action`] against the guest. Returns `true` when a
/// warp request was actually posted — the caller arms its landing check on that and
/// on nothing else (a `Leave` with no world to return to posts none).
///
/// Split out of the loop because it is called from three places (a control-file
/// entry, the floor-advance and the poll) and all three must apply an action the same
/// way — a warp posted with the bytes in the wrong order, or a fork written from a
/// cached array pointer, is the kind of difference that only shows up on a device.
/// What a `Leave` will need AFTER it has been applied: the scene to send the player to and
/// the tile to stand them on, read now because `apply_guardian_action` finishes the run
/// (`Run::finish` clears both). `None` for anything but a `Leave`, and for a `Leave` with
/// nowhere to go.
///
/// ★ 143차: a run with no origin of its own that is left from INSIDE the tower (a floor
/// scene) goes to the hero's entrance town (`guardian::fallback_home`) instead of staying
/// in a room with no door — the old answer, "leaving the player where they are", was the
/// soft-lock behind *"하늘성안에서 세이브버튼이나오고 그걸 누르면나올수가없어"*. Outside the
/// tower an originless leave still posts nothing (the player is somewhere legitimate).
fn leave_home(
    tower: &crate::guardian::Run,
    act: crate::guardian::Action,
    obs: &crate::guardian::Obs,
    hero: Option<crate::guardian::Hero>,
) -> Option<(u8, Option<(u8, u8)>)> {
    if !matches!(act, crate::guardian::Action::Leave { .. }) {
        return None;
    }
    match tower.origin() {
        Some(scene) => Some((scene, tower.origin_tile())),
        None if obs.scene.is_some_and(|s| tower.is_floor_scene(s)) => {
            let (scene, tile) = crate::guardian::fallback_home(hero);
            Some((scene, Some(tile)))
        }
        None => None,
    }
}

/// The destination of a floor warp, for the arrival placement to wait on. `None` for
/// everything else, including a `Leave` — that one is `leave_home`'s business, and its
/// destination tile is the one the run remembered rather than the arena's own.
fn warp_scene(act: crate::guardian::Action) -> Option<u8> {
    match act {
        crate::guardian::Action::Warp { scene } => Some(scene),
        _ => None,
    }
}

/// ★ 138차: load the pinned save slot's tower progress when the pin has moved — one atomic
/// read when it has not. `tower.best` follows it (the log lines read it).
fn sync_tower_progress(
    data_dir: &Path,
    prog: &mut crate::guardian::Progress,
    prog_slot: &mut Option<u8>,
    tower: &mut crate::guardian::Run,
) {
    let slot = crate::vault::live_slot();
    if slot == *prog_slot {
        return;
    }
    *prog = slot.map(|s| config::read_tower_progress(data_dir, s)).unwrap_or_default();
    *prog_slot = slot;
    tower.best = prog.best;
    if let Some(s) = slot {
        log::warn!(
            "guardian: slot {s} progress — best boss {} (best floor {}, finished {}x)",
            prog.best_boss,
            prog.best,
            prog.clears
        );
    }
}

/// ★ 138차: the quick-start picker for a slot whose best boss is `best_boss`, or `None`
/// when there is nothing to pick (no boss cleared — the caller enters floor 1 as before).
/// Rows are DESCENDING, so the list's default focus (row 0, 93차) is the deepest start —
/// the one a returning player wants — and 「1층 처음부터」 is the last row.
/// ⛔ Not the dialogue script's own choice list: its row count is fixed in the asset and
/// the widget's height is unmeasured; a host window costs no guest byte (§2.3).
fn tower_pick_rows(best_boss: u8) -> Option<(Vec<u8>, crate::OrbPick)> {
    let floors = crate::guardian::entry_floors(best_boss);
    if floors.len() < 2 {
        return None;
    }
    let offered: Vec<u8> = floors.into_iter().rev().collect();
    let rows = offered
        .iter()
        .map(|&f| crate::OrbRow {
            name: format!("{f}층"),
            detail: if f == 1 { "처음부터".to_string() } else { format!("{}층 보스 클리어", f - 1) },
            enabled: true,
            notice: None,
            // No item icon: `atlas::item` answers `None` past the sheet, and the row keeps
            // its icon column so it lines up with every other list the widget draws.
            icon: u16::MAX,
        })
        .collect();
    let pick = crate::OrbPick {
        title: "하늘성 입장".to_string(),
        subtitle: format!("최고 {best_boss}층 보스 클리어"),
        rows,
        ok_label: "입장",
        keep_view: false,
        keep_open: false,
    };
    Some((offered, pick))
}

/// ★ 138차 — a run is about to END: queue its exit reward (`TOWER_REWARD_BALANCE_DRAFT.md
/// §3`). ⛔ MUST be called with the action BEFORE `apply_guardian_action` applies it: a
/// `Leave` finishes the run there, and `Run::finish` forgets `run_best_boss`.
///
/// Shards go to `shards_owed` (paid straight into the 재료가방, `BagBoxHost::grant_boxed`),
/// the rest to `owed` for the game's own giver; both are paid on a FIELD poll, i.e. after
/// the way home has landed — *"다시 마을로 워프되면서 … 보상이 들어오는"*. Nothing is
/// saved (the player saves in town — the 85차 rule).
///
/// ★ 143차: `held` = the wake's list of boxes to show AFTER 「의식이 몽롱하다...」 — the
/// finished tower's reward box would otherwise be up under the white and sit on top of the
/// game's own line. `None` = say it now, as before.
fn queue_exit_reward(
    tower: &crate::guardian::Run,
    act: crate::guardian::Action,
    frame: u64,
    owed: &mut std::collections::VecDeque<(u16, u16)>,
    shards_owed: &mut u32,
    proxy: &EventLoopProxy<UserEvent>,
    held: Option<&mut Vec<String>>,
) {
    let crate::guardian::Action::Leave { why } = act else { return };
    let best = tower.run_best_boss;
    if best == 0 {
        return;
    }
    if !crate::guardian::pays_exit_reward(why) {
        log::warn!("guardian: run over ({why:?}) with boss {best} cleared — no exit reward (결정 ⑥: 메인메뉴)");
        return;
    }
    let seed = crate::guardian::mix(now_ms() ^ (frame << 16) ^ u64::from(best));
    let recipes = crate::guardian_rewards::RECIPES;
    let legacy = crate::guardian_rewards::LEGACY;
    let Some(p) = crate::guardian::exit_payout(
        best,
        seed,
        recipes.len(),
        |i| recipes[i].req_lv,
        legacy.len(),
        |i| legacy[i].req_lv,
    ) else {
        return;
    };
    *shards_owed = shards_owed.saturating_add(u32::from(p.shards));
    let mut rows = vec![];
    match p.recipe {
        Some(i) => rows.push(recipes[i]),
        None => log::warn!(
            "guardian: exit reward for boss {} — no 조합서 in its band ({} recipe row(s) shipped: an asset without the 138차 rows?)",
            p.boss,
            recipes.len()
        ),
    }
    rows.extend(p.legacy.map(|i| legacy[i]));
    rows.extend(p.endgame.map(|i| legacy[i]));
    for r in &rows {
        owed.push_back((u16::from(r.cat), u16::from(r.idx)));
    }
    log::warn!(
        "guardian: run over ({why:?}) — exit reward for boss {}: {} shard(s) + {:?} (recipe {:?} legacy {:?} endgame {:?}, seed {seed:#x})",
        p.boss,
        p.shards,
        rows.iter().map(|r| (r.cat, r.idx, r.req_lv)).collect::<Vec<_>>(),
        p.recipe,
        p.legacy,
        p.endgame
    );
    let text = format!("하늘성 {}층 보스 보상", p.boss);
    match held {
        Some(list) => list.push(text),
        None => {
            let _ = proxy.send_event(UserEvent::NoticeText(text));
        }
    }
}

/// `home` = the scene a `Leave` is posted to ([`leave_home`] — the run's origin, or the
/// hero's entrance town for an originless leave from inside the tower); ignored otherwise.
fn apply_guardian_action(
    emulator: &mut dyn Emulator,
    tower: &mut crate::guardian::Run,
    act: crate::guardian::Action,
    obs: &crate::guardian::Obs,
    creatures: &[crate::guardian::Creature],
    can_save_was: &mut Option<u8>,
    home: Option<u8>,
) -> bool {
    use crate::guardian::Action;
    match act {
        Action::None => false,

        Action::Warp { scene } => {
            // ★★★ ARM THE FLOOR'S MONSTERS FIRST (88차). The `tower_species_lgt` hook
            // writes this triple over the scene header's as THIS scene's header is
            // parsed, which happens inside the warp the request below triggers — so it
            // has to be armed before the request, and keyed on the scene so the origin
            // town a `Leave` loads is never touched. A legacy row (no species columns)
            // disarms it and the scene spawns what its header says.
            let want = tower.floor(tower.current_floor()).and_then(|f| f.species_override());
            wie_util::stats::set_tower_species(want);
            match want {
                Some((s, sp)) => log::warn!("guardian: species for scene {s} armed: {sp:?} (255 = empty slot)"),
                None => log::warn!("guardian: no species override for scene {scene}; the scene's own header decides"),
            }
            // ★★★ 145차: and the floor's COUNT, the same way. The `tower_spawn_cap_lgt`
            // hook stops the game's spawn loop at it while THIS scene's map is parsed, so
            // the surplus spawn points are never turned into monsters. ⛔ They used to be
            // spawned and then zeroed (`park_indices`, deleted) — the game ran its own
            // death for each, which the player saw as monsters dying on arrival.
            let cap = tower.floor(tower.current_floor()).and_then(|f| f.spawn_cap());
            wie_util::stats::set_tower_spawn_cap(cap);
            if let Some((s, n)) = cap {
                log::warn!("guardian: spawn cap for scene {s} armed: {n} monster(s)");
            }
            // ★★★ RAISE THE SEAL FIRST. Every floor scene's A rows are gated on
            // `GUARDIAN_FLAG_TOWER` being zero (`tools_re/patch_tower_gate.py`), so this
            // byte is what stops the room running its own scripts — its door, its
            // dialogue, its quest flags. It goes up BEFORE the destination is posted so
            // that the scene is already sealed the first time anything in it can fire.
            let seal = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::guardian_tower_flag(&read, 1)
            };
            if let Some((addr, want)) = seal {
                let ok = emulator.write_memory(addr, &[want]);
                log::warn!("guardian: tower seal at {addr:#x} -> {want} (write ok = {ok})");
            }
            let plan = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::guardian_warp_writes(&read, scene)
            };
            match plan {
                None => {
                    log::warn!("guardian: cannot post a warp on this build (no scene table)");
                    false
                }
                Some(writes) => {
                    // ⛔ IN ORDER. Entry point and destination first, the REQUEST last:
                    // the game's handler polls every tick and consumes the request byte
                    // before dispatching, so a request written first can be serviced
                    // with a stale destination.
                    let mut ok = true;
                    for (addr, bytes) in &writes {
                        ok &= emulator.write_memory(*addr, bytes);
                    }
                    log::warn!("guardian: warp to scene {scene} posted (all writes ok = {ok})");
                    true
                }
            }
        }

        // ★★ A CLEARED FLOOR IS HELD EXACTLY LIKE A LIVE ONE (code review 2026-09-11).
        // `Reward` is what the machine answers for the whole of the clear-hold and for
        // as long as an open window keeps `next_floor` waiting — and the hold (20 polls
        // = 600 frames) is the game's own respawn threshold
        // (`crate::guardian::RESPAWN_DISARMED`). With the disarm only on `Fork`, the
        // monsters came back under the 「클리어」 line. Read-compare-write throughout,
        // so a settled floor costs no writes.
        Action::Fork { floor } | Action::Reward { floor, .. } => {
            hold_floor(emulator, tower, floor, creatures, can_save_was);
            false
        }

        Action::Leave { why } => {
            // ★★★ A death is UNDONE before the way back is posted (2026-09-17). The
            // game's death sequence lives in the player STATE byte, not in the object
            // the scene reload recreates, so a warp + a later heal still ended on the
            // GAME OVER window and the title (S9 vc339, measured). This is the game's
            // own revive (window 17's 「예」, `0x3b510`) in host form: HP/SP first, then
            // state 0 / pose 0 — see `items::guardian_revive_writes`. Only for a death:
            // a Quit or a table error has nothing to undo.
            if matches!(why, crate::guardian::Abort::PlayerDown) {
                let (heal, revive) = {
                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                    (items::full_restore_writes(&read), items::guardian_revive_writes(&read))
                };
                match (heal, revive) {
                    (Some(h), Some((r, state))) => {
                        let mut ok = true;
                        for (addr, bytes) in h.iter().chain(r.iter()) {
                            ok &= emulator.write_memory(*addr, bytes);
                        }
                        log::warn!(
                            "guardian: the player died on the floor — revived in place (state {state} -> 0, {} write(s), ok = {ok}) before the way back",
                            h.len() + r.len()
                        );
                    }
                    (h, r) => log::warn!(
                        "guardian: the player died but the revive could not be planned (heal {} / state {}) — the game's own GAME OVER will run",
                        h.is_some(),
                        r.is_some()
                    ),
                }
            }
            // Put the player back where the run started, if that is expressible — and
            // if there is still a world to do it in.
            // ⛔ `!in_world` means the player left for the title / slot list. The warp
            // request byte lives in CHAR and is only serviced from the field, so a
            // request posted now would sit there until the NEXT character's first
            // field tick and teleport that character to this run's origin.
            // ★ 143차: the destination is the caller's `home` (`leave_home`) — the run's
            // origin, or for an originless leave from inside the tower the hero's
            // entrance town, where the old answer left the player in a doorless room.
            let posted = match home {
                Some(scene) if obs.in_world => {
                    let plan = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        items::guardian_warp_writes(&read, scene)
                    };
                    let posted = plan.is_some();
                    if let Some(writes) = plan {
                        for (addr, bytes) in &writes {
                            emulator.write_memory(*addr, bytes);
                        }
                    }
                    let whose = if tower.origin() == Some(scene) { "where the run began" } else { "the hero's entrance town — the run had no origin" };
                    log::warn!("guardian: run over ({why:?}) — sending the player back to scene {scene} ({whose})");
                    posted
                }
                Some(scene) => {
                    log::warn!("guardian: run over ({why:?}) — no world to return to (the player left for the shell); scene {scene} not posted");
                    false
                }
                None => {
                    log::warn!("guardian: run over ({why:?}) — no origin scene to return to and not inside the tower, leaving the player where they are");
                    false
                }
            };
            // The species override is scene-keyed, so the origin town was never at
            // risk — but a stale word must not outlive the run either. Same for the
            // spawn cap (145차).
            wie_util::stats::set_tower_species(None);
            wie_util::stats::set_tower_spawn_cap(None);
            // ⛔ And only now is the machine idle again. `Failed` keeps offering
            // `Leave` until this call, on purpose (see `Run::finish`).
            tower.finish();
            posted
        }
    }
}

/// Everything a live floor needs re-asserted on every poll — the stat fork, the
/// respawn disarm, the tower seal and the no-save byte. Shared by `Action::Fork` and
/// `Action::Reward` (see [`apply_guardian_action`] for why the cleared floor needs it).
fn hold_floor(
    emulator: &mut dyn Emulator,
    tower: &mut crate::guardian::Run,
    floor: u8,
    creatures: &[crate::guardian::Creature],
    can_save_was: &mut Option<u8>,
) {
    if tower.floor(floor).is_none() {
        return;
    }
    // ── The per-floor monster count (user 2026-09-16) ────────────────────────────
    //
    // ★★★ 145차: the count is applied at SPAWN — `Action::Warp` arms the
    // `tower_spawn_cap_lgt` hook, and the game's own spawn loop never creates the surplus.
    // ⛔ Nothing here removes a monster any more. Until 145차 this block "parked" the
    // surplus by writing 0 HP, and the game ran its own death for each one — the player
    // saw monsters drop dead on arrival without a hit (user 2026-09-30, *"어쨌든 자동으로
    // 몬스터가 죽는걸 원하지 않아"*). Every record the floor holds is forked and counted.
    //
    // Through the run's floor memory: the first sighting of a record on this floor is
    // its base (the boss records have zero in their base fields), and the virtual HP
    // pool lives there too. ⛔ `creatures` must have been read before any write of this
    // poll, which the caller guarantees by reading everything first.
    // ★ 158차 P8: the roll fields (명중 · 회피 · 기절 저항) are set against the PLAYER's
    // level, like hell's — read every poll, so a level-up re-targets them.
    let plv = {
        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
        items::player_level(&read)
    };
    let forks = tower.forks_for(floor, creatures, plv);
    // ⛔ CANCEL PENDING RESPAWNS. Without this the floor is not clearable: the
    // game re-spawns a dead monster into its own record after ~300 ticks, so the
    // live count climbs back and the clear test never fires (device vc286: live
    // 19 -> 13 -> 14 -> 15 -> 16 on floor 1). User requirement 2026-09-11.
    // ★ Re-asserted every poll on purpose — ONE write does not hold, the game
    // re-arms within ~50 frames. `crate::guardian::RESPAWN_DISARMED` has the
    // headless measurement.
    let disarms = crate::guardian::respawn_disarms(creatures);
    // ★ A dead BOSS keeps being drawn until something moves it (the "보스 잔상" —
    // `crate::guardian::corpse_hides` has the measurement). Park its map object.
    let hides = crate::guardian::corpse_hides(creatures);
    let plan = {
        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
        let stats = (!forks.is_empty())
            .then(|| items::guardian_fork_writes(&read, &forks))
            .flatten()
            .unwrap_or_default();
        let no_respawn = items::guardian_respawn_writes(&read, &disarms)
            .unwrap_or_default();
        let hide = items::guardian_corpse_hide_writes(&read, &hides).unwrap_or_default();
        // Forbid saving while a floor is live.
        //
        // ⛔ THIS DOES *NOT* HIDE THE SAVE BUTTON AND THE MINIMAP "FOR FREE".
        // §D.4 claimed it did, on the strength of 27차 making both follow this
        // byte — but `HIDE_UI_ON_NO_SAVE` has been false since 2026-08-09 (a
        // deliberate stand-down, see the constant), so 27차's hiding is switched
        // off and the controls stay on screen. Device-measured on S9 with vc279.
        let no_save = items::guardian_no_save_write(&read);
        // The value to give back when the run ends, captured BEFORE the first
        // zero-write. ⛔ Required: the byte does not heal on a scene load
        // (`crate::guardian::restore_can_save`).
        let was = items::guardian_can_save_byte(&read).map(|(_, v)| v);
        (stats, no_respawn, hide, no_save, was)
    };
    let (stats, no_respawn, hide, no_save, was) = plan;
    if !hide.is_empty() {
        let ok = hide.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
        log::warn!("guardian: floor {floor} — parked {} dead boss sprite(s) off the map ({ok}/{} writes)", hide.len() / 2, hide.len());
    }
    // Re-assert the seal: read-compare-write, so a settled floor writes nothing.
    // ⛔ Required as well as the write on `Action::Warp` — the flag array is
    // carried by the save and `deserialiseSave` is one of its writers, so a load
    // inside a run could put it back.
    {
        let seal = {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            items::guardian_tower_flag(&read, 1)
        };
        if let Some((addr, want)) = seal {
            let ok = emulator.write_memory(addr, &[want]);
            log::warn!("guardian: floor {floor} — tower seal re-asserted at {addr:#x} (write ok = {ok})");
        }
    }
    if can_save_was.is_none() {
        // Only a non-zero reading is worth remembering: zero is either our own
        // earlier write or a genuine no-save area, and in both cases there is
        // nothing to restore.
        *can_save_was = was.filter(|v| *v != 0);
    }
    if !stats.is_empty() {
        // Only ever non-empty when something actually drifted — the plan is
        // read-compare-write — so this line is a real event, not a heartbeat.
        log::info!("guardian: floor {floor} — re-asserting {} write(s) across {} monster(s)", stats.len(), forks.len());
        // ★ 145차: the fork's first pass is the floor's first sighting — say whether the
        // spawn cap held. More records than the row's count means the hook did not apply
        // (they are fought, never removed — see `crate::guardian::spawned_over_count`).
        let count = tower.floor(floor).and_then(|f| f.count);
        match crate::guardian::spawned_over_count(count, creatures) {
            Some(n) => log::warn!(
                "guardian: floor {floor} — {n} monster(s) spawned but the row asks for {} — the spawn cap did not apply; they stay (no monster is removed)",
                count.unwrap_or(0)
            ),
            None => {
                if let Some(c) = count {
                    log::warn!("guardian: floor {floor} — {} monster(s) spawned (row count {c})", creatures.len());
                }
            }
        }
        // ★ 112차: one record's growth in numbers, so a device log can show "Lv 120 = x6.8
        // HP" without a probe dump — the first fork against its spawn snapshot.
        if let Some(f0) = forks.first()
            && let Some(b) = tower.memory.bases.iter().find(|b| b.index == f0.index)
        {
            log::warn!(
                "guardian: floor {floor} — #{} spawn Lv{} hp {} atk {} def {}/{} -> Lv{} hp {}{} atk {} def {}/{} · rolls vs player {plv:?}: hit {:?} dodge {:?} res {:?}",
                f0.index,
                b.lv0.map_or(-1, i32::from),
                b.hp0, b.atk0, b.dp0, b.ds0,
                f0.level.map_or(-1, i32::from),
                f0.hp_max,
                f0.vhp.or(b.vhp_total).map(|t| format!(" (total {t})")).unwrap_or_default(),
                f0.atk_eff, f0.def_phys_eff, f0.def_spec_eff,
                f0.hit,
                f0.dodge.map(|d| d as i16),
                f0.res.map(|r| r as i16),
            );
        }
    }
    if !no_respawn.is_empty() {
        let ok = no_respawn
            .iter()
            .filter(|(addr, bytes)| emulator.write_memory(*addr, bytes))
            .count();
        log::info!(
            "guardian: floor {floor} — cancelled {ok}/{} pending respawn(s)",
            no_respawn.len()
        );
    }
    for (addr, bytes) in &stats {
        emulator.write_memory(*addr, bytes);
    }
    if let Some((addr, v)) = no_save {
        let ok = emulator.write_memory(addr, &[v]);
        log::info!("guardian: floor {floor} — saving disabled at {addr:#x} (write ok = {ok})");
    }
}

/// ★★★ 72차-D (TODO `BB` ⓑ) — REBUILD A SLOT SUMMARY THAT WENT MISSING.
///
/// The 「빈슬롯」 half of the 제련 report. A slot ROW is decided by `SlotSave_11_<slot>`
/// and nothing else (guest scan `0x5c06`), and the four highest-frequency saves —
/// 창고 올리기 `0x6ea0`, 내리기 `0x13a90`/`0x13d36`, 캐시 지급 `0x13dd8`/`0x13ec8` — go
/// through `saveGame 0x4748`, which writes the payload and never calls `slotSummary`. So
/// a perfectly intact character can stop appearing on the slot list, and the only way out
/// used to be 설정 → DIAG → 세이브 복원.
///
/// ⓐ ([`crate::vault::keep_summary_partner`]) made that restore possible. This makes it
/// unnecessary: when the payload is plausible and the summary is absent or not
/// [`crate::savevault::SUMMARY_LEN`] bytes, the host runs the last two steps of the
/// game's own chain and puts the row back.
///
/// Narrow on purpose — it does NOT serialise, so it commits nothing and cannot turn into
/// an unasked-for save. See [`items::summary_repair_gate`] for why the gate is not
/// [`items::save_gate`], and §72차-C §2 for the one hard constraint (⛔ no latch: the
/// write goes through `vault::write_check` on its own merits like every other).
fn summary_repair_poll(emulator: &mut dyn Emulator, st: &mut SaveState, data_dir: &Path, now: u64) {
    if st.job.is_some() || st.summary_repairs >= SUMMARY_REPAIR_MAX || now < st.summary_checked_ms.saturating_add(SUMMARY_CHECK_MS) {
        return;
    }
    st.summary_checked_ms = now;
    let gate = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::summary_repair_gate(&read)
    };
    let Some((name_slot, slot)) = gate else {
        return;
    };
    if !crate::vault::summary_needs_repair(data_dir, slot) {
        return;
    }
    let Some(chain) = crate::guest_build::save_chain_fns() else {
        return;
    };
    let Some(job) = save::Job::summary_only(chain, name_slot) else {
        return;
    };
    let save::Next::Call { addr, args, argc, .. } = job.first_call() else {
        return;
    };
    st.job = Some(job);
    st.summary_repairs += 1;
    issue(emulator, st, addr, &args[..argc]);
    log::warn!("save: rebuilding the slot summary for slot {slot} (attempt {} of {SUMMARY_REPAIR_MAX})", st.summary_repairs);
}

/// Advance a save in flight by one poll. Called once per frame.
// ⛔ `BackupState` (the host-driven recovery backup: `loadSaveRecord` → 219 B → the
// private server, or the offline store) WAS HERE and is gone (99차, 2026-09-16, 100 %
// offline). It was KTF-only in practice (`items::backup_target` required
// `guest_code_addresses_valid`, false on the shipped LGT build), and its destination —
// `hero3_link.rs` — no longer exists.

fn save_poll(emulator: &mut dyn Emulator, st: &mut SaveState, data_dir: &Path, proxy: &EventLoopProxy<UserEvent>) {
    let Some(mut job) = st.job else {
        return;
    };
    let result = st.result.poll();
    let scratch = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::save_scratch(&read).unwrap_or(0)
    };
    // ★ 72차 (F-05): tell the chain whether the record store actually took the write it
    // is about to step past. `writeSaveRecord` cannot — `0x1a1d4` returns the last
    // `free`'s r0, not the write's — so the only witness is the host's own refusal
    // counter. Sampling it around the step is what stops a summary from landing over a
    // payload that was refused, which is a slot whose row and whose bytes disagree.
    let refusals_now = crate::vault::refusal_count();
    if result.is_some() {
        job.last_write_ok = refusals_now == st.refusals_at_issue;
    }
    let next = job.advance(result, scratch);
    st.job = Some(job);
    st.refusals_at_issue = refusals_now;
    match next {
        save::Next::Wait => {}
        save::Next::Call { addr, args, argc, .. } => issue(emulator, st, addr, &args[..argc]),
        save::Next::Done => {
            st.job = None;
            log::info!("save: done ({:?}, payload {} B, slot record {} B)", job.reason, job.len, save::SLOT_RECORD_LEN);
            // ★★★ 72차-D — A SUMMARY REBUILD IS NOT A SAVE, and everything below this
            // point assumes one. It serialised nothing, so the cash-item retry it would
            // cancel is still owed, the rollback note it would clear is still the only
            // record of an uncommitted use, and the recovery backup it would take would
            // upload exactly the record that is already on the server.
            if job.reason == save::Reason::Summary {
                log::warn!("save: the slot summary is back (the row will no longer read 빈슬롯)");
                return;
            }
            if job.reason == save::Reason::Manual {
                // ★ The side line, not the box (user 2026-09-30): a box on the field wants
                // OK while the monsters keep moving. Every SAVE-button answer goes there.
                let _ = proxy.send_event(UserEvent::SideNote("저장완료".into()));
            }
            if job.reason == save::Reason::Sort {
                // The 정렬 already toasted; this is the line a device log needs.
                log::warn!("save: the sorted bag is on disk (네트워크 싱글가방 정렬, 136차)");
            }
            // ANY completed save serialised the current state, so a queued cash-item
            // save is now redundant. A queued MANUAL retry deliberately stays: its
            // serialise may predate the tap, and the player was promised a toast.
            if matches!(st.retry, Some((save::Reason::Item, _))) {
                st.retry = None;
            }
            // …and by the same argument the rollback note is spent: the record just
            // written to disk carries the guest's stack decrement, so host and guest
            // now agree and there is nothing left to undo. ANY reason counts — a manual
            // SAVE commits a cash-item use just as well as the armed retry does.
            config::clear_cash_baseline(data_dir);
            // ⛔ The recovery-backup upload that used to follow a save (`bk.begin`) is
            // gone with the private server (99차). The save vault (`vault.rs`) is the
            // on-device backup now, and it captured the previous record inside the write
            // that just completed.
        }
        save::Next::Failed(why) => {
            st.job = None;
            log::warn!("save: aborted at {:?} — {why}", job.step);
            // A 정렬's save failing is worth the same word: the order the player just saw
            // is not on disk (136차).
            // The SAVE button's failure is the side line like its success; a 정렬's stays the
            // box — it is answered on the bag screen, not in a fight.
            match job.reason {
                save::Reason::Manual => {
                    let _ = proxy.send_event(UserEvent::SideNote("저장실패".into()));
                }
                save::Reason::Sort => {
                    let _ = proxy.send_event(UserEvent::Notice("저장실패"));
                }
                _ => {}
            }
        }
    }
}

/// The 통화료 dialog, guest `0x5c054`: `네트워크 접속시;통화료가 부과됩니다.;접속하시겠습니까?`
/// (`;` is this game's line break), 54 bytes of EUC-KR plus its NUL.
///
/// ── WHY IT IS REWRITTEN (player, 2026-08-20) ────────────────────────────────
/// "네트워크창고 눌렀을떄 통화료가 부과됩니다 같은 안내 필요없고 네트워크 창고에
/// 접속하시겠습니까? 와 같이 띄워줘야하고". In offline mode nothing dials anything, so
/// quoting call charges is not just noise — it is false. The replacement is shorter
/// than the original, so it fits inside the same literal and the trailing bytes stay
/// NUL (the widget stops at the first one).
///
/// ★ Unconditional since 99차 (2026-09-16): the game is 100 % offline, so the rewrite is
/// always the honest wording. (It used to be gated on `NetMode::Offline`.)
///
/// Address: the string is at file offset `0x5b088` in `binary.mod`, and this build's
/// mapping is `vaddr = fo + 0xfcc` — cross-checked against `GameSave_11`
/// (fo `0x5abe8` -> `0x5bbb4`, the address `save.rs` already uses).
const DIALOG_TEXT_ADDR: u32 = 0x0005_c054;
/// Bytes of the original literal, so a rewrite can never run past it.
const DIALOG_TEXT_MAX: usize = 54;

/// `네트워크 창고에;접속하시겠습니까?` in EUC-KR, NUL-terminated (34 bytes).
const DIALOG_TEXT_OFFLINE: &[u8] = &[
    0xb3, 0xd7, 0xc6, 0xae, 0xbf, 0xf6, 0xc5, 0xa9, 0x20, 0xc3, 0xa2, 0xb0, 0xed, 0xbf, 0xa1, 0x3b, 0xc1, 0xa2, 0xbc,
    0xd3, 0xc7, 0xcf, 0xbd, 0xc3, 0xb0, 0xda, 0xbd, 0xc0, 0xb4, 0xcf, 0xb1, 0xee, 0x3f, 0x00,
];

// ── the guest's own 5-minute idle disconnect ─────────────────────────────────
//
// Player, 2026-08-20: "무작동 5분 네트워크 끊김도 없애야돼 오프라인이라 상관없잖아".
// Right — nothing can time out when there is no connection. The timer is the GAME's,
// not ours, and the disassembly is unambiguous (LGT `binary.mod` md5 6a352944…):
//
//   arm    0x182d8:  now64()                     ; bl 0x5a6f4
//                    *(u64*)0x150ea78 = now + 300000    ; literal at 0x182f8
//   check  0x18a0c:  if (!*(u8*)(0x150ea80+0x14)) return ;  timer enabled?
//                    if (deadline > now64()) return ;
//                    netTeardown(0)              ; bl 0x18608
//                    showModal(text, 5, 1, 1)    ; bl 0x5a6f8 — the disconnect modal
//                                                ;   (the same type-5 box 48차 drives)
//
// `300000` is the ONLY word in the whole image with that value, and it is a
// PC-relative literal read by that one arming function, so raising it cannot touch
// anything else. The deadline itself is pushed to the maximum too, so the session
// already running is safe before the next re-arm (the game re-arms on input, which
// is exactly what "무작동" means it will not get).
// ── the cash-purchase confirm box ────────────────────────────────────────────
//
// Player, 2026-08-20: "캐시템 살때 실제 현금 -1원의 추가 정보 이용료가 부과됩니다.
// 구입하시겠습니까? 는 이제 뜰필요없어. 그냥 구매하시겠습니까? 만 뜨면돼."
//
// The box is assembled, not stored (guest 0x737c-0x7390, and a second identical site
// at 0x7402-0x7416):
//
//   sprintf(dst, "%s %d%s", "실제 현금", price, "원의 추가 정보이용료(통화료별도)가 …")
//          fmt @0x5bec8      @0x5bed0     ctx+0x32c   @0x5be84
//
// So no single string can be edited into the wanted text. Instead the FORMAT POINTER
// is repointed: the tail literal (65 bytes of room) is overwritten with the message
// and the pool word both call sites load their format from is aimed at it. With no
// conversions left in the format the two remaining varargs are simply ignored, and
// the box reads exactly "구매하시겠습니까?".
//
// Both sites load the format from the SAME pool word (0x7410 -> 0x74dc, 0x738a ->
// 0x74dc), so one write covers both.
const BUY_FMT_PTR: u32 = 0x0000_74dc;
const BUY_TAIL_ADDR: u32 = 0x0005_be84;
/// Room in the tail literal, up to the next string (`실제 현금` at 0x5bed0).
const BUY_TAIL_MAX: usize = 65;
/// `구매하시겠습니까?` in EUC-KR, NUL-terminated (18 bytes).
const BUY_TEXT: &[u8] = &[
    0xb1, 0xb8, 0xb8, 0xc5, 0xc7, 0xcf, 0xbd, 0xc3, 0xb0, 0xda, 0xbd, 0xc0, 0xb4, 0xcf, 0xb1, 0xee, 0x3f, 0x00,
];

const IDLE_LIMIT_LITERAL: u32 = 0x0001_82f8;
const IDLE_DEADLINE: u32 = 0x0150_ea78;
/// The largest interval that stays positive in the guest's signed 64-bit compare.
const IDLE_NEVER: u32 = 0x7fff_ffff;

/// Idempotent guest patches for offline mode: the connect dialog's wording and the
/// idle disconnect. Both re-check before writing, so the periodic pass normally
/// costs two small reads.
/// Make the game's give-item routine write a count on a cell it just claimed.
///
/// See [`crate::guest_anchors::GuestKind::give_item_count_patch`] for the whole
/// derivation. In one line: the non-equipment arm at guest `0x52f48` increments
/// `rec+0x3ba` only when the bag scanner MERGED into an existing stack, so the first orb
/// of a kind — which is every orb 오브원석 hands out — is born with a count of 0, prints
/// as «0», and is refused by `items::orb_apply`'s `count == 0` guard, i.e. it cannot be
/// combined either. Since 71차-O delisted 오브상자, 오브원석 is the only source of orbs,
/// so this one byte disables the orb system for a new character.
///
/// ⛔ Do NOT "fix" this in the painter (`0x1214a` prints `+0x3ba` with no `> 1` guard):
/// hiding the zero would leave an orb that looks fine and still cannot be used.
fn give_item_count_patch(emulator: &mut dyn Emulator) {
    let Some((site, orig, patched)) = crate::guest_build::give_item_count_patch() else {
        return;
    };
    debug_assert_eq!(orig.len(), patched.len());
    let cur = emulator.read_memory(site, orig.len());
    if cur.len() != orig.len() {
        return;
    }
    if cur == patched {
        return; // already applied
    }
    if cur != orig {
        // ⚠ Someone else's bytes. Say so once and leave them alone — a blind write here
        // is a corrupted instruction stream in the middle of the inventory code.
        log::error!("orb count: {site:#x} does not hold the expected instructions — NOT patching");
        return;
    }
    let ok = emulator.write_memory(site, patched);
    log::warn!("orb count: give-item now writes count = merged ? old + 1 : 1 ({site:#x}, {} bytes, ok = {ok})", patched.len());
}

fn offline_guest_patches(emulator: &mut dyn Emulator) {
    const _: () = assert!(DIALOG_TEXT_OFFLINE.len() <= DIALOG_TEXT_MAX);
    // ★ Compare the WHOLE replacement, not a prefix. Device-caught 2026-08-20: both
    // strings begin "네트워크", so a 4-byte "is it already patched?" test matched the
    // ORIGINAL and the write never happened.
    let cur = emulator.read_memory(DIALOG_TEXT_ADDR, DIALOG_TEXT_OFFLINE.len());
    if cur.len() == DIALOG_TEXT_OFFLINE.len() && cur != DIALOG_TEXT_OFFLINE {
        let ok = emulator.write_memory(DIALOG_TEXT_ADDR, DIALOG_TEXT_OFFLINE);
        log::warn!("offline: connect dialog text rewritten at {DIALOG_TEXT_ADDR:#x} (ok = {ok})");
    }

    // The cash-purchase confirm box: message into the tail literal, then aim both
    // call sites' format pointer at it.
    const _: () = assert!(BUY_TEXT.len() <= BUY_TAIL_MAX);
    let cur = emulator.read_memory(BUY_TAIL_ADDR, BUY_TEXT.len());
    if cur.len() == BUY_TEXT.len() && cur != BUY_TEXT {
        let a = emulator.write_memory(BUY_TAIL_ADDR, BUY_TEXT);
        let b = emulator.write_memory(BUY_FMT_PTR, &BUY_TAIL_ADDR.to_le_bytes());
        log::warn!("offline: cash-purchase confirm box simplified (text {a}, format pointer {b})");
    }

    let lit = emulator.read_memory(IDLE_LIMIT_LITERAL, 4);
    if lit.len() == 4 && u32::from_le_bytes([lit[0], lit[1], lit[2], lit[3]]) != IDLE_NEVER {
        let ok = emulator.write_memory(IDLE_LIMIT_LITERAL, &IDLE_NEVER.to_le_bytes());
        log::warn!("offline: idle-disconnect interval raised at {IDLE_LIMIT_LITERAL:#x} (ok = {ok})");
    }
    // …and push the live deadline out, for a session that armed before we patched.
    let dl = emulator.read_memory(IDLE_DEADLINE, 8);
    let far = [0xffu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
    if dl.len() == 8 && dl != far {
        emulator.write_memory(IDLE_DEADLINE, &far);
    }
}

// ⛔ `show_disconnect_modal` (48차: raise the game's own modal 5 after Android killed a
// backgrounded socket) WAS HERE and is gone (99차). There is no socket for Android to
// kill: the in-app server lives in this process and survives any background.

// ── 메뉴 터치 라우터 (66차) ──────────────────────────────────────────────────
//
// The UI reports a tap in guest-screen px; everything else happens HERE, because
// the answer depends on guest memory: WHICH of the game's own overlay screens is
// showing, and which page a bag grid is on. The host then does what a finger on the
// handset would have done, minus the arrow presses — it writes the game's own cursor
// byte and synthesises OK, so every rule that follows (what that row opens, whether
// the cell holds an item, the action popup) is the game's own code on its own stack.
//
// ★ This replaced 61차's 탭-투-무브 executor, which is GONE (66차, player: the
// quickslot plates are small, so a tap that missed one walked the character and
// "조작이 불편해졌습니다"). The world tap has no meaning any more; the panel tap does.

/// The player tapped inside the game's own menu panel at guest-screen `(x, y)`.
///
/// Silently does nothing when the screen has no measured geometry — a tap inside a
/// game window means nothing else, so claiming it costs nothing and a toast for every
/// stray tap would be noise. Everything is re-read here: the game clears its own
/// cursors on its transitions (`TOUCH_MENU_FEASIBILITY.md §7-2`), so a value the UI
/// captured a frame ago is a wrong answer waiting to happen.
fn menu_tap(
    emulator: &mut dyn Emulator,
    bagbox: &mut crate::bagbox_host::BagBoxHost,
    x: i32,
    y: i32,
    item: Option<(crate::menutouch::Grid, u8)>,
    status: Option<u8>,
    equip: Option<crate::menutouch::EquipMark>,
    cell_cursor: bool,
) {
    let (lw, lh) = crate::logical();
    let Some((dx, dy)) = crate::menutouch::panel_point(lw as i32, lh as i32, x, y) else { return };
    let action = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::menu_tap_action(&read, dx, dy, item, status, equip, cell_cursor)
    };
    let Some(action) = action else {
        // Not a row, not a cell, or a screen whose geometry we have not measured. Said
        // out loud (once per tap, human rate) because the alternative is a player report
        // of "the tap does nothing" with no way to tell WHERE it stopped.
        log::info!("menu tap: panel ({dx},{dy}) is not a row or a cell on this screen");
        return;
    };
    match action {
        // FIRST tap: move the game's own focus and send NOTHING. The highlight follows
        // on the next painted frame all by itself (probe-proven with zero keys), so the
        // player gets to look at what they picked before it does anything.
        items::MenuTapAction::Focus { addr, byte } => {
            if !emulator.write_memory(addr, &[byte]) {
                log::warn!("menu tap: could not write the cursor byte at {addr:#x}");
                return;
            }
            log::info!("menu tap: panel ({dx},{dy}) -> focus {byte} (wrote {addr:#x}, no key)");
        }
        // SECOND tap on the focused thing: the same two events a finger on the handset
        // produces, and the same commit the 퀵바등록 flow has been shipping since 20차
        // (`popup_commit`'s SyntheticOk arm).
        //
        // ★ Like that path, this pair goes straight to the guest and does NOT touch the
        // `held` auto-repeat map: that map is for keys a FINGER is holding, and a
        // synthetic press is not held by anything. The one shared consequence is that a
        // second finger physically holding the on-screen OK keeps its own repeats
        // flowing after this Keyup — which is what a held OK does anyway.
        items::MenuTapAction::Activate => {
            // ★★ 2026-09-15 — A BAG-ITEM CELL OPENS ITS BOX HERE, not through the game's
            // popup. `bagbox_popup` can only speak for the inventory page; the online
            // screens (싱글가방 · 네트워크 창고가방 · 캐시샵, all app state 3 with
            // `MENU_PAGE_OFF == 0` — 66차-C) raise a popup whose tag this host has never
            // measured, which is why the boxes did nothing there (user: *"네트워크
            // 창고에서도 3종 가방 동작해야하는데"*). Asking the BAG RECORD under the cursor
            // works on every screen, and it is one tap shorter everywhere.
            //
            // ⛔ The OK is NOT sent. Row 0 of that popup is the DELISTED cash row's own
            // 사용하기 (배신의문장 / 명예의문장 / 오브상자) — the thing 92차 went out of its
            // way to keep away from the guest.
            let (boxed, why) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                (items::bagbox_cell_focused(&read), items::bagbox_focus_report(&read))
            };
            // ★★★ 117차-B — ONLY A TAP THAT LANDED ON AN ITEM-GRID CELL MAY OPEN A BOX.
            //
            // 제보 2026-09-20: *"아이템 구매창에서 맨처음 첫번째 두번째 세번째칸을 선택하면
            // 3종 상자가 열려"*. `Activate` is ONE variant for every second tap —
            // `menu_tap_action` returns it for a focused ROW of the list widget, the menu
            // list and the confirm pair just as it does for a grid cell — and
            // `bagbox_cell_focused` answers from MEMORY: on the shop it sees window 12 and
            // the shared inventory cursor bytes, which on the BUY list still hold their last
            // value (default: cells 0..2 of bag 1 = the three pinned bag-item cells). So a
            // second tap on buy-list rows 1..3 read as "our cell is focused" and opened a
            // box instead of buying — 105차's own warning about this window, arriving by the
            // tap door it had declared safe. The evidence the tap carries (66차-B / 109차) is
            // `item`: the grid the UI derived from the game's own empty-cell frames, `None`
            // on every row list. A box needs it; a row gets its OK as before.
            if let Some((screen, kind)) = boxed {
                if item.is_some() {
                    log::info!("menu tap: panel ({dx},{dy}) is the {} cell -> opening the box ({screen:?}) [{why}]", kind.title());
                    bagbox.request_open(screen, kind);
                    return;
                }
                log::info!(
                    "menu tap: panel ({dx},{dy}) is a focused ROW, not a grid cell — the {} cell under the shared cursor is ignored and OK goes to the game ({screen:?}) [{why}]",
                    kind.title()
                );
            }
            // ⛔ The report goes on the OK line too: "열기 가 안 나온다" is answered by
            // which term of the gate said no, and nothing else can tell us.
            log::info!("menu tap: panel ({dx},{dy}) is already focused -> OK [{why}]");
            emulator.handle_event(Event::Keydown(KeyCode::OK));
            emulator.handle_event(Event::Keyup(KeyCode::OK));
        }
        // ★★★ 장비 (80차-I): ONE WRITE, three bytes, no keys.
        //
        // 79차-D pressed the game's own RIGHT once per worker poll because the repo's
        // "the cursor skips empty slots" warning could not be disproved with a character
        // wearing all nine pieces. Player, 2026-09-05: *"장비창에서 신발을 터치하면
        // 포커스가 갑자기 다른칸들을 쭈르르르륵 순식간에 돈다음에 신발로 포커스이동"*.
        // The warning is now disproved on a character with four empty slots (see
        // `menutouch::EQUIP`), so the cursor moves the way every other screen's does.
        //
        // ⛔ Column and row FIRST, index last. The painter reads all three, and the host
        // writes between guest ticks — but ordering it so the index (the byte the tap
        // router cross-checks) lands last means a torn read can only ever show the OLD
        // cell, never a mixed one.
        items::MenuTapAction::FocusEquip { addr, cell } => {
            if !write_equip_cursor(emulator, addr, cell, true) {
                return;
            }
            log::info!("menu tap: panel ({dx},{dy}) -> 장비 slot focus {cell} (no key)");
        }
        // ★★ The candidate pane needs ONE key to finish the move (80차-I, device vc259):
        // the write takes — the next RIGHT continued from the written column/row — but
        // the yellow border does not repaint until a key arrives, because this pane
        // caches its drawn position where the left grid reads the bytes live.
        //
        // So park the cursor one cell BEFORE the target and let the game's own RIGHT
        // step onto it. One press, not a lap: the player sees a single jump, and the
        // arrival cell is the GAME's arithmetic rather than ours. RIGHT wraps across
        // rows (measured `(2,0) -> (0,1)`), so cell 0 is reached from cell 8.
        //
        // ⛔ The parked cell is never drawn — that is the whole reason this works — so
        // do not "simplify" it into a write of the target plus a key that would then
        // step PAST it.
        // ★★ 80차-J — back to the worn slots. The game's own CLR does it; see
        // `items::MenuTapAction::LeaveEquipCandidates` for why it is a key and not a write.
        items::MenuTapAction::LeaveEquipCandidates => {
            log::info!("menu tap: panel ({dx},{dy}) -> 장비 back to the worn slots (CLR)");
            emulator.handle_event(Event::Keydown(KeyCode::CLEAR));
            emulator.handle_event(Event::Keyup(KeyCode::CLEAR));
        }
        items::MenuTapAction::FocusEquipCandidate { addr, cell } => {
            let n = crate::menutouch::EQUIP_CAND.cells() as u8;
            let park = (cell + n - 1) % n;
            if !write_equip_cursor(emulator, addr, park, false) {
                return;
            }
            log::info!("menu tap: panel ({dx},{dy}) -> 장비 candidate focus {cell} (parked at {park} + RIGHT)");
            emulator.handle_event(Event::Keydown(KeyCode::RIGHT));
            emulator.handle_event(Event::Keyup(KeyCode::RIGHT));
        }
    }
}

/// Write 장비's three-byte cursor (index / column / row at `addr` / `+1` / `+2`).
///
/// ⛔ Column and row FIRST, index last. The painter reads all three and the host writes
/// between guest ticks, but ordering it so the INDEX — the byte the tap router
/// cross-checks — lands last means a torn read can only ever show the old cell.
/// `index` is false on the candidate pane, where the game leaves that byte at the slot
/// number being filled (device-measured; see `menutouch::EQUIP_CAND`).
fn write_equip_cursor(emulator: &mut dyn Emulator, addr: u32, cell: u8, index: bool) -> bool {
    let cols = crate::menutouch::EQUIP.cols as u8;
    for (a, v) in [(addr + 1, cell % cols), (addr + 2, cell / cols)] {
        if !emulator.write_memory(a, &[v]) {
            log::warn!("menu tap: could not write the 장비 cursor byte at {a:#x}");
            return false;
        }
    }
    if index && !emulator.write_memory(addr, &[cell]) {
        log::warn!("menu tap: could not write the 장비 cursor byte at {addr:#x}");
        return false;
    }
    true
}

/// Apply one UI-thread input event to the guest, maintaining the auto-repeat map.
///
/// `picking` = the UI is currently highlighting the quickbar plates, waiting for the
/// player to choose one. It lives here rather than on the UI thread because every
/// decision that reads it also has to RE-READ guest RAM in the same breath.
///
/// ★ The hidden-plate set (2026-09-10) is pushed into `pages` on the way in, here and
/// in the poll: `activate` / `reserve` / `warm_plan` / `adopt_guest_bar` all consult it,
/// and one setter is what keeps the four from drifting apart. The UI has its own gates
/// on the same rule — these are the ones nothing can go around.
// ── 세이브 슬롯 창 (130차) ────────────────────────────────────────────────────
//
// Twelve real slots (`slotwindow::TOTAL`) behind the game's three-row list: `slotwindow.rs` has the rules and the
// addresses, `database.rs` has the record-name map. What lives HERE is the part that
// touches the guest: the host owns the slot-list cursor outright while a list is up, and
// when the window slides it refills the guest's own slot arrays and label cells — the
// stores the guest's `setAppState(3)` scan would have made, which does not run again.

/// What the window's key arm needs to know about the guest right now.
struct SlotWindowCtx {
    char_base: u32,
    label_base: u32,
    /// The guest's slot-list ROW (0..=2).
    cursor: u8,
}

/// The gate: is a slot list up, with no modal over it, on a build that has the anchors?
///
/// ⛔ A modal owns the keys while it is up — 삭제 확인 (11), 빈슬롯입니다 (7), 복구 (10),
/// the WAIT box (3). Sliding the window under the delete confirm would make its 예 delete
/// a different real slot than the one the player was looking at, so nothing moves while
/// `modal_id != 0`, and an UNREADABLE modal word is treated the same way.
fn slot_window_ctx(emulator: &dyn Emulator) -> Option<SlotWindowCtx> {
    let label_base = crate::guest_build::slot_label_base()?;
    let read = |a: u32, l: usize| emulator.read_memory(a, l);
    if !items::shell_state(&read) || !crate::savevault::on_slot_list(items::shell_screen(&read)) {
        return None;
    }
    if items::modal_id(&read) != Some(0) {
        return None;
    }
    let (char_base, cursor) = items::slot_window_base(&read)?;
    Some(SlotWindowCtx { char_base, label_base, cursor })
}

/// Host-owned UP/DOWN on the slot lists. `true` = the key was consumed and must NOT be
/// forwarded — the guest's own arm would move (and wrap) the cursor a tick later than
/// this decision, which is the race that would turn two quick presses into a wrap.
fn slot_list_key(emulator: &mut dyn Emulator, data_dir: &Path, proxy: &EventLoopProxy<UserEvent>, up: bool) -> bool {
    let Some(ctx) = slot_window_ctx(&*emulator) else {
        return false;
    };
    let first = crate::slot_first();
    let Some(mv) = crate::slotwindow::on_key(first, ctx.cursor, up) else {
        return false;
    };
    match mv {
        crate::slotwindow::KeyMove::Move { cursor } => {
            let (addr, bytes) = crate::slotwindow::cursor_write(ctx.char_base, cursor);
            let ok = emulator.write_memory(addr, &bytes);
            log::info!(
                "save slots: row {} -> {} (real slot {:?}, write ok = {ok})",
                ctx.cursor,
                cursor,
                crate::slotwindow::real_slot(first, cursor)
            );
        }
        crate::slotwindow::KeyMove::Slide { first: f } => slot_window_apply(emulator, data_dir, proxy, &ctx, f, None),
        crate::slotwindow::KeyMove::Wrap { first: f, cursor } => slot_window_apply(emulator, data_dir, proxy, &ctx, f, Some(cursor)),
    }
    true
}

/// Move the window to `first`: the global map first (so the very next guest read of a
/// row already resolves to the slot about to be painted), then the guest's five arrays
/// for the three rows, the three label cells, and — on a wrap — the cursor word. All of
/// it lands between two ticks, so the guest sees either the old window or the new one.
fn slot_window_apply(
    emulator: &mut dyn Emulator,
    data_dir: &Path,
    proxy: &EventLoopProxy<UserEvent>,
    ctx: &SlotWindowCtx,
    first: u8,
    cursor: Option<u8>,
) {
    let first = crate::slotwindow::clamp_first(first);
    crate::set_slot_first(first);
    let mut writes: Vec<crate::slotwindow::GuestWrite> = Vec::new();
    let mut occupied = [false; crate::slotwindow::VISIBLE as usize];
    for g in 0..crate::slotwindow::VISIBLE {
        let real = first + g;
        // The game's own verdict (MD5 over the descrambled body) decides the row, so a
        // summary the guest would call 빈슬롯 is 빈슬롯 here too.
        let summary = crate::vault::summary_body(data_dir, real).and_then(|b| crate::slotwindow::parse_summary(&b));
        occupied[g as usize] = summary.is_some();
        writes.extend(crate::slotwindow::row_writes(ctx.char_base, g, summary.as_ref()));
    }
    writes.extend(crate::slotwindow::label_writes(ctx.label_base, first));
    if let Some(c) = cursor {
        writes.push(crate::slotwindow::cursor_write(ctx.char_base, c));
    }
    let mut failed = 0usize;
    for (addr, bytes) in &writes {
        if !emulator.write_memory(*addr, bytes) {
            failed += 1;
            log::error!("save slots: guest write failed at {addr:#x} ({} byte(s))", bytes.len());
        }
    }
    crate::config::write_slot_first(data_dir, first);
    let _ = proxy.send_event(UserEvent::SlotWindow {
        first,
        total: crate::slotwindow::TOTAL,
    });
    // Where the PINNED character's slot now sits in the window (`None` = scrolled out of
    // view) — the one fact a save-slot report from this screen will need.
    let pinned = crate::vault::live_slot();
    log::warn!(
        "save slots: window -> SLOT {}..{} of {} (occupied {occupied:?}, cursor {}, pinned slot {pinned:?} at row {:?}, {} write(s), {failed} failed)",
        first + 1,
        first + crate::slotwindow::VISIBLE,
        crate::slotwindow::TOTAL,
        match cursor {
            Some(c) => format!("wrapped to row {c}"),
            None => format!("row {}", ctx.cursor),
        },
        pinned.and_then(|s| crate::slotwindow::guest_slot(first, s)),
        writes.len()
    );
}

/// Keep the three `"SLOT n"` label cells in the guest image saying the window's real
/// slots. Runs with the other periodic guest patches: the image is loaded fresh with
/// `SLOT 1/2/3` every boot, and a window that starts above 0 has to relabel before the
/// first list is painted. Byte-compared first, so a matching image costs one read.
fn slot_window_labels(emulator: &mut dyn Emulator) {
    let Some(label_base) = crate::guest_build::slot_label_base() else {
        return;
    };
    let first = crate::slot_first();
    let want = crate::slotwindow::expected_labels(first);
    let cur = emulator.read_memory(label_base, want.len());
    if cur.len() == want.len() && cur != want {
        let mut ok = true;
        for (addr, bytes) in crate::slotwindow::label_writes(label_base, first) {
            ok &= emulator.write_memory(addr, &bytes);
        }
        log::warn!("save slots: row labels rewritten for window {first} (SLOT {}..{}, ok = {ok})", first + 1, first + crate::slotwindow::VISIBLE);
    }
}

/// 161차: open the slot list on `target`, the real slot the player loaded last. The window
/// slides only when that slot is out of view (`slotwindow::focus_target`); in view, the
/// cursor word alone moves — the same two kinds of write the player's own UP / DOWN make.
fn slot_list_focus(emulator: &mut dyn Emulator, data_dir: &Path, proxy: &EventLoopProxy<UserEvent>, ctx: &SlotWindowCtx, target: u8) {
    let first = crate::slot_first();
    let (want, row) = crate::slotwindow::focus_target(first, target);
    if want != first {
        slot_window_apply(emulator, data_dir, proxy, ctx, want, Some(row));
        log::warn!(
            "save slots: the list opened on SLOT {} (the last loaded slot) — window slid {first} -> {want}, row {row}",
            target + 1
        );
    } else if row != ctx.cursor {
        let (addr, bytes) = crate::slotwindow::cursor_write(ctx.char_base, row);
        let ok = emulator.write_memory(addr, &bytes);
        log::warn!(
            "save slots: the list opened on SLOT {} (the last loaded slot) — row {} -> {row} in window {first} (write ok = {ok})",
            target + 1,
            ctx.cursor
        );
    } else {
        log::info!("save slots: the list opened on SLOT {} (the last loaded slot) — already row {row} of window {first}", target + 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_input(
    emulator: &mut dyn Emulator,
    held: &mut HashMap<KeyCode, Instant>,
    picking: &mut bool,
    pages: &mut quickpage::Pages,
    window: &mut quickpage::Window,
    buffs: &mut crate::buffs::Buffs,
    saving: &mut SaveState,
    skill_watch: &mut Option<SkillWatch>,
    levelups: &mut u32,
    cash: &mut CashQtyPoke,
    // `orb_take` = where to leave the bag cell whose 오브결합 row the player just took, as
    // (tab, slot, instance key). It has to be latched HERE, on the key, because the guest's
    // own OK handler frees the popup. The KEY goes with it because the bag compacts.
    orb_take: &mut Option<(u8, u8, u64)>,
    bagbox: &mut crate::bagbox_host::BagBoxHost,
    avatar: &mut crate::avatar_host::AvatarHost,
    walk: &mut WalkOrder,
    data_dir: &std::path::Path,
    proxy: &EventLoopProxy<UserEvent>,
    ev: InputEvent,
) {
    pages.set_hidden(crate::quick_hidden());
    match ev {
        // ★ 167차: the AVATAR page owns the panel while it is up — a panel point would be
        //   read as EQUIPMENT's grid under it. The UI routes the page's own taps through
        //   `InputEvent::Avatar`; anything that still arrives here is stale.
        InputEvent::MenuTap { .. } if avatar.is_open() => {
            log::info!("menu tap: dropped — the AVATAR page owns the panel");
        }
        InputEvent::MenuTap {
            x,
            y,
            item,
            status,
            equip,
            cell_cursor,
        } => menu_tap(emulator, bagbox, x, y, item, status, equip, cell_cursor),
        InputEvent::Avatar(act) => avatar.tap(emulator, act, proxy),
        // ★ 168차: a row of the seven-row MENU list (AVATAR between EQUIPMENT and SKILL).
        InputEvent::MenuRow(row) => avatar.menu_row(emulator, row, proxy),
        InputEvent::Key { code, down: true } => {
            // ★ `#` IS NOT SWALLOWED — not even on the slot screen. 복구 is retired
            //   (2026-08-20: "우리 이제 서버 안쓸꺼니까 복구도 끊을꺼야") but the SAME key
            //   is 초기화 on a slot that holds a character, and that half stays
            //   ("이미 존재하는 세이브 파일을 #으로 삭제는 가능해야해"). Blocking the key
            //   here would need the host to know which of the two the screen is
            //   offering, and it does not: the occupancy array read through the online
            //   module's singleton answered `[1,1,1]` for a screen showing one
            //   character, and the cursor had no verified anchor at all.
            //
            //   So the refusal lives at the far end instead. A 복구 IS a `04 3e`
            //   request; a delete sends nothing. `offline::Engine::recovery` answers
            //   that request with 이용할 수 없는 기능입니다. and never with a character,
            //   which cannot mistake one for the other.
            if !held.contains_key(&code) {
                // ★ 167차 아바타 옷장: a «3» on EQUIPMENT's worn grid opens the AVATAR page
                //   instead of SKILL, and while the page is up it owns every key (CLR and «3»
                //   leave it and still reach the game). Consumed keys never enter `held`.
                if avatar.key(emulator, code, proxy) {
                    return;
                }
                // ★ 130차: on the two slot lists the HOST owns UP/DOWN — the save-slot
                //   window (`slot_list_key`). Consumed keys never enter `held`, so the
                //   matching release below is a no-op and nothing reaches the guest.
                if matches!(code, KeyCode::UP | KeyCode::DOWN) && slot_list_key(emulator, data_dir, proxy, code == KeyCode::UP) {
                    return;
                }
                // ★ 136차: on the 창고 panel `*` opened the gold-deposit box (「금액을
                //   설정하세요」). The touch HUD has had no `*` since 102차-C, its `✱` hint is
                //   painted over now, and the user retired the feature — the one road left
                //   is a pad's SELECT, which stops here. Elsewhere `*` is still the map.
                if code == KeyCode::STAR {
                    let wh = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::warehouse_panel_up(&read)
                    };
                    if wh {
                        log::info!("net: `*` on the 창고 panel swallowed — the gold deposit box was retired with its hint (136차)");
                        return;
                    }
                }
                // ★ An OK on a cash item's 사용하기 row starts the host-side effect. The
                //   guest state is read HERE, one statement before the key is forwarded,
                //   so it cannot be stale — the same reason 17차's PopupConfirm re-reads.
                //   We do not change the key: the GAME still runs `useItem`, whose shared
                //   tail (guest 0x154542) decrements the stack and removes the record at
                //   zero. The host only watches and supplies the effect the original
                //   client never had.
                if code == KeyCode::OK {
                    // ⛔ 71차-L: this used to record the slot cursor as a "pending
                    // execute" for the save-slot pin. It is gone — an OK on a slot
                    // screen means "run/back up THIS row", which is not evidence about
                    // where the LIVE character lives. See `savevault::execute_pin_slot`.
                    let pending = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::cash_use_pending(&read)
                    };
                    if let Some((cat, idx)) = pending {
                        let now = now_ms();
                        // ★ 2026-08-14 — BEFORE any host-side effect touches disk, write
                        //   down the state the last save already committed. The guest's
                        //   stack decrement will not reach disk until the retry armed at
                        //   the bottom of this block fires, and an app death in between
                        //   used to leave the effect without the cost (the player's
                        //   "저장을 안 하면 다시 사용할 수 있다"). See `buffs::CashBaseline`.
                        //
                        //   Ordering is the whole trick: a crash between this line and the
                        //   effect leaves a note EQUAL to the live state, so restoring it
                        //   is a no-op. The other order would commit the effect for free.
                        if cat == crate::buffs::CASH_CAT {
                            config::arm_cash_baseline(data_dir, buffs, *levelups);
                        }
                        if let Some(g) = crate::buffs::grant_for(cat, idx) {
                            buffs.apply(g, now);
                            config::write_buffs(data_dir, buffs);
                            log::info!(
                                "cash buff used: {g:?} -> exp x{}, relief {} levels, {} min left",
                                buffs.exp_multiplier(now),
                                buffs.relief_levels(now),
                                buffs.minutes_left(now)
                            );
                        } else if let Some(r) = crate::buffs::box_reward(cat, idx, now) {
                            // ★ Random boxes (비밀의상자 / 오브상자). `now` doubles as the
                            //   roll — one box open per millisecond is plenty of spread,
                            //   and it keeps this file free of an RNG dependency.
                            match r {
                                crate::buffs::BoxReward::Exp { mult, ms } => {
                                    buffs.apply_timed(crate::buffs::Grant::Exp(mult), now, ms);
                                    config::write_buffs(data_dir, buffs);
                                }
                                crate::buffs::BoxReward::Relief { ms } => {
                                    buffs.apply_timed(crate::buffs::Grant::LevelRelief(crate::buffs::RELIEF_LEVELS), now, ms);
                                    config::write_buffs(data_dir, buffs);
                                }
                                crate::buffs::BoxReward::FullRestore => {
                                    let writes = {
                                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                                        items::full_restore_writes(&read)
                                    };
                                    match writes {
                                        Some(ws) => {
                                            for (a, bytes) in ws {
                                                let ok = emulator.write_memory(a, &bytes);
                                                log::info!("box full restore: {a:#x} <- {bytes:02x?} (write ok = {ok})");
                                            }
                                        }
                                        None => log::warn!("box full restore skipped — no live character"),
                                    }
                                }
                            }
                            let _ = proxy.send_event(UserEvent::Notice(crate::buffs::box_reward_notice(r)));
                            log::info!("cash box used: (cat {cat:#x}, idx {idx:#x}) -> {r:?}");
                        } else if crate::buffs::is_orb_box(cat, idx) {
                            // ★ 오브상자 (2026-08-22) — the HOST pays nothing for this any
                            //   more. Its 2026-08-05 buff pool was a stand-in for a
                            //   capability we lacked (dropping a real orb); 44차 built it,
                            //   and the engine now puts one real cat-0x10 orb per unit in
                            //   the network warehouse. The player asked for the buff to go:
                            //   "오브상자는 그냥 오브를 얻는 용도인건데?" — correct.
                            //
                            //   Same shape as the 소켓확장 arm below: this only fires for a
                            //   copy left in someone's bag by an older build, and it says
                            //   where the real one is instead of vanishing silently.
                            log::warn!("orb box used from the bag — the host pays nothing since 65차; the warehouse orb is the real one");
                            let _ = proxy.send_event(UserEvent::Notice(crate::buffs::ORB_BOX_BAG_NOTICE));
                        } else if crate::buffs::is_socket_expand(cat, idx) {
                            // ★ 소켓확장 — the HOST no longer grants this (44차). 38차 wrote
                            //   the socket byte onto "the first equipped item with room",
                            //   which answered neither half of what the player asked for:
                            //   they want to CHOOSE the target, and to choose it out of the
                            //   network warehouse. It now runs through the game's own
                            //   오브결합 picker instead, entirely server-side — see
                            //   `buffs::IDX_SOCKET_EXPAND` and the server's
                            //   `ORB_IDX_SOCKET_EXPAND`.
                            //
                            //   This arm only exists for a copy left in someone's bag by the
                            //   old shop row: the guest's consume tail eats it regardless of
                            //   what we do, so it says where the working one lives rather
                            //   than vanishing silently.
                            log::warn!("socket expand used from the bag — inert since 44차; the warehouse copy is the real one");
                            let _ = proxy.send_event(UserEvent::Notice(crate::buffs::SOCKET_EXPAND_BAG_NOTICE));
                        } else if crate::buffs::is_level_elixir(cat, idx) {
                            // ★ 천상의 영약 (2026-08-13): the grant is a guest call into
                            //   the game's own checkLevelUp, which must run in the same
                            //   context the kill path runs it in — the FIELD — and the
                            //   inventory window is open right now. So the use only ARMS
                            //   a pending grant; the worker loop fires it on the next
                            //   field poll (see the levelup block by the autoloot poll).
                            //   Persisted so an app death between the two cannot eat a
                            //   consumed item.
                            if crate::guest_build::level_up_fn().is_some() {
                                *levelups = levelups.saturating_add(1);
                                config::write_pending_levelups(data_dir, *levelups);
                                log::info!("level elixir used: {} level-up(s) pending (fires on field return)", *levelups);
                            } else {
                                // The item should not be purchasable on such a build (the
                                // server lists it for everyone), so say WHY nothing happened.
                                log::warn!("level elixir used but checkLevelUp is not located on this build — no effect");
                            }
                        }
                        // ★ 2026-08-07: a CASH item use arms a save (경험의 보주, 상자,
                        // 부활서, …). The game consumes the item but the player may not
                        // save for a long while — a crash then rolls the count back,
                        // and for the host-granted buffs that is a free duplicate. The
                        // save cannot run NOW (the inventory window is open, the gate
                        // refuses), so it is armed as a long retry that fires once the
                        // player is back on a saveable field. A 부활서 popped inside a
                        // boss room simply keeps waiting — the room forbids saving,
                        // which is the accepted behaviour.
                        if cat == crate::buffs::CASH_CAT {
                            // Never DOWNGRADE a pending Manual retry to a silent one.
                            if !matches!(saving.retry, Some((save::Reason::Manual, _))) {
                                saving.retry = Some((save::Reason::Item, now + ITEM_RETRY_MS));
                                log::info!("save: armed after cash-item use (cat {cat:#x}, idx {idx:#x})");
                            }
                        }
                    }
                }
                // ★ The cash-shop count goes in HERE and nowhere else. `handle_event`
                //   only queues the key, so the guest cannot run the confirm handler
                //   (`0xc3a4`), send `05 42`, or reach the grant (`0x13e4c`) until a
                //   later tick — which makes this the one moment the write is
                //   guaranteed to be in place before it is read. Writing it while the
                //   box was merely OPEN is what made the whole page draw the count.
                // ★ 열기 on a bag-item cell (2026-09-14): a pure host verb like the
                //   오브결합 row — the popup is closed with the game's own CLR and the OK
                //   never reaches the guest (its row 0 would be 사용하기 on the cell).
                if code == KeyCode::OK && bagbox.take_open(emulator) {
                    return;
                }
                if code == KeyCode::OK {
                    cash_qty_commit(&mut *emulator, cash);
                    // ★ …and the in-game 오브결합 row, latched HERE for the same reason the
                    //   cash count is: `handle_event` only QUEUES the key, so this is the
                    //   last moment the popup still exists. The guest's own OK handler
                    //   closes and FREES it (`0x47b3a`-`0x47b54`) and runs nothing for our
                    //   row — `popup_choose` dispatches tag 0x64 for rows 0 and 1 only
                    //   (`0x47590`-`0x4759e`). That fall-through is what makes a host-owned
                    //   row leak-free, and this is where we learn it was taken.
                    let taken = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::orb_popup(&read).filter(|p| p.ours && items::orb_row_selected(&read))
                    };
                    if let Some(p) = taken {
                        log::info!("orb: 오브결합 chosen for bag {},{} (idx {:#04x})", p.tab, p.slot, p.orb_idx);
                        *orb_take = Some((p.tab, p.slot, p.key));
                    }
                }
                emulator.handle_event(Event::Keydown(code));
                held.insert(code, Instant::now());
            }
        }
        InputEvent::Key { code, down: false } => {
            if held.remove(&code).is_some() {
                emulator.handle_event(Event::Keyup(code));
            }
        }
        // The stick's standing order. It arrives right after the `Key` pair that turned
        // the direction, so the re-press gate starts closed: the character is about to
        // leave idle on that very keydown.
        InputEvent::Walk { dir } => {
            walk.dir = dir;
            walk.next = dir.map(|_| Instant::now() + Duration::from_millis(KEYREPEAT_MS as u64));
        }
        // ── the slot list's ▲/▼ arrows (130차) ───────────────────────────────────────
        InputEvent::SlotScroll { delta } => {
            let ctx = slot_window_ctx(&*emulator);
            match ctx.as_ref().and_then(|_| crate::slotwindow::on_arrow(crate::slot_first(), delta)) {
                Some(first) => slot_window_apply(emulator, data_dir, proxy, ctx.as_ref().expect("checked"), first, None),
                None => log::info!(
                    "save slots: arrow {delta:+} ignored ({})",
                    if ctx.is_some() { "the window is at that end" } else { "no slot list up, or a modal owns it" }
                ),
            }
        }

        // ── 136차: 「정렬」 ────────────────────────────────────────────────────────
        //
        // ★ Every gate is re-asked HERE, on the tap: the UI's `sort_screen` is a published
        //   copy, and what matters is the screen at the moment the records are rewritten.
        //   Three screens, two data sets: the INVENTORY page and the 네트워크 싱글가방 are
        //   the same guest bag records; the 네트워크 창고 is the host's `offline::Store`.
        InputEvent::SortItems => {
            use crate::bagsort::SortScreen;
            let refuse = |why: &str, proxy: &EventLoopProxy<UserEvent>| {
                log::warn!("sort: refused — {why}");
                let _ = proxy.send_event(UserEvent::NoticeText("지금은 정렬할 수 없습니다".into()));
            };
            let screen = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::sort_screen(&read)
            };
            let Some(screen) = screen else {
                return refuse("no sortable screen is up (a popup, a modal, or another screen)", proxy);
            };
            if screen == SortScreen::OnlineWarehouse {
                let level = crate::item_levels::item_level;
                match crate::network::sort_warehouse(data_dir, |cat, idx, n, pos| crate::bagsort::wire_key(cat, idx, n, pos, &level)) {
                    Some(moved) => {
                        log::warn!("sort: the warehouse is in order (rows moved = {moved}) — a page-1 refresh is queued for the client");
                        let _ = proxy.send_event(UserEvent::NoticeText("정렬을 완료했습니다".into()));
                    }
                    None => refuse("no warehouse session is live", proxy),
                }
                return;
            }
            // ── the bag (INVENTORY page or 네트워크 싱글가방) ──
            if let Some(why) = bagbox.sort_blocker() {
                return refuse(why, proxy);
            }
            if *picking {
                return refuse("a quickslot registration is in progress", proxy);
            }
            if orb_take.is_some() {
                return refuse("an 오브결합 pick is being applied", proxy);
            }
            if saving.job.is_some() {
                return refuse("a save is being written — it is serialising this bag", proxy);
            }
            let plan = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::bag_sort_writes(&read)
            };
            let Some(plan) = plan else {
                return refuse("the bag cannot be read", proxy);
            };
            if plan.writes.is_empty() {
                log::info!("sort: the bag ({} cell(s)) is already in order", plan.cells);
                let _ = proxy.send_event(UserEvent::NoticeText("정렬을 완료했습니다".into()));
                return;
            }
            let mut ok = 0usize;
            for (a, bytes) in &plan.writes {
                if !emulator.write_memory(*a, bytes) {
                    log::error!("sort: write failed at {a:#x} after {ok} of {} — the bag may be half-sorted; the game's own compaction keeps it consistent", plan.writes.len());
                    break;
                }
                ok += 1;
            }
            // Our own plates name cells too; they follow the same moves (the guest's item
            // refs went with the writes above).
            let moves = plan.moves.clone();
            if pages.remap_item_cells(&|t, s| moves.iter().find(|(from, _)| *from == (t, s)).map(|(_, to)| *to)) {
                crate::config::write_quickpages(data_dir, pages);
            }
            log::warn!(
                "sort: {} cell(s), {} moved, {ok}/{} write(s) ok, cursor -> {:?} ({screen:?})",
                plan.cells,
                plan.moved,
                plan.writes.len(),
                plan.cursor_to
            );
            match screen {
                // The 장비 screen's candidate list and the smith's recipe list are
                // `(tab, slot)` indexes the game rebuilds in `0x4d858` — the same call the
                // orb refresh makes with the inventory open (71차-L). World init calls it too
                // (`0x3ad50`), which is why the online screen below needs no guest call.
                SortScreen::Inventory => {
                    if let Some(r) = crate::guest_build::item_refresh()
                        && r.fn_equip_snapshot != 0
                    {
                        emulator.call_guest_function(r.fn_equip_snapshot, &[]);
                    }
                }
                // The game saves this bag only in its deposit / withdraw tails, never on the
                // way out — so the sort is written now, as those tails would (user decision
                // 2026-09-25). `Reason::Sort`: full chain, no toast of its own.
                SortScreen::OnlineBag => {
                    let gate = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::sort_save_gate(&read)
                    };
                    if !save_begin_named(emulator, saving, save::Reason::Sort, gate) {
                        log::warn!("sort: the follow-up save did not start — the order lives until the next deposit or withdraw saves the bag");
                    }
                }
                SortScreen::OnlineWarehouse => {}
            }
            let _ = proxy.send_event(UserEvent::NoticeText("정렬을 완료했습니다".into()));
        }

        // ── the "퀵바등록" flow ────────────────────────────────────────────────
        //
        // ★ Every arm re-reads the popup state FIRST. The UI's copy is up to ~80 ms
        //   old, and the row it names decides which guest function runs — the skill
        //   dispatcher has no bounds check of its own (see items::popup_view), so a
        //   stale row is not a cosmetic error.
        InputEvent::PopupSelectRow { row } => {
            // Move the game's own highlight, exactly the way the 퀵슬롯등록 commit moves
            // it — one byte, no key. The painter re-reads that byte every frame (LGT
            // `0x427ca`: `ldrsb r2,[base+0x9c5e]` feeding `0x40570`'s third argument),
            // so the highlight follows with nothing synthesised.
            //
            // ⛔ The refusal lives in `items::popup_select_write`, which re-reads the
            // live row count: this value goes into a byte whose only range check is in
            // the game's key arm.
            let write = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::popup_select_write(&read, row)
            };
            match write {
                Some((addr, row)) => {
                    if emulator.write_memory(addr, &[row]) {
                        log::info!("popup: row {row} focused by tap");
                    } else {
                        log::warn!("popup: row {row} focus write failed at {addr:#x}");
                    }
                }
                None => log::info!("popup: row {row} tap dropped — the popup has no such row any more"),
            }
        }
        InputEvent::PopupConfirm => {
            // The 열기 popup is a one-row list `popup_view` does not match (it wants the
            // native 8 or our 4), so it has to be asked first or the confirm would be
            // handed to the guest as a plain OK.
            if bagbox.take_open(emulator) {
                *picking = false;
                let _ = proxy.send_event(UserEvent::PickingSlot(false));
                return;
            }
            let view = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::popup_view(&read)
            };
            let Some(view) = view else {
                // The popup went away between the tap and here: hand the confirm to
                // the game unchanged, which is what the player's finger asked for.
                *picking = false;
                let _ = proxy.send_event(UserEvent::PickingSlot(false));
                emulator.handle_event(Event::Keydown(KeyCode::OK));
                emulator.handle_event(Event::Keyup(KeyCode::OK));
                return;
            };
            match view.sel {
                // 사용하기 / 포인트투자 — the game's own row 0. Pass it through.
                0 => {
                    emulator.handle_event(Event::Keydown(KeyCode::OK));
                    emulator.handle_event(Event::Keyup(KeyCode::OK));
                }
                // 퀵바등록. Do NOT commit yet — the row means "ask which plate".
                1 if view.can_bind => {
                    *picking = true;
                    let _ = proxy.send_event(UserEvent::PickingSlot(true));
                    log::info!("popup: 퀵바등록 armed (kind={}, item={})", view.kind, view.item);
                }
                // An unlearned skill: the game's bind guard would silently UNBIND
                // instead of registering, so refuse rather than offer it.
                1 => log::info!("popup: 퀵바등록 refused — skill not learned"),
                // 상세보기 / 등록취소 / 버리기: remap the display row to the guest row
                // the dispatcher expects and let the game run it.
                d => {
                    if let Some(row) = items::popup_guest_row(&view, d, 0) {
                        popup_commit(emulator, &view, row);
                    }
                }
            }
        }
        InputEvent::PopupBindSlot { slot } => {
            *picking = false;
            let _ = proxy.send_event(UserEvent::PickingSlot(false));
            // ★ 소비가방 (2026-09-14): a 퀵슬롯등록 armed from the box window binds the
            //   plate to the BOX (`Entry::Boxed`) — no guest write beyond freeing the seat
            //   the plate held, because a boxed plate owns no seat between presses.
            if let Some((cat, idx)) = bagbox.take_pending_bind() {
                let (Ok(c), Ok(i)) = (u8::try_from(cat), u8::try_from(idx)) else { return };
                match pages.bind_boxed(window, slot, c, i) {
                    Some(actions) => {
                        apply_quickpage_actions(emulator, &actions);
                        crate::config::write_quickpages(data_dir, pages);
                        log::warn!("bagbox: plate {slot} now holds boxed cat {cat:#x} idx {idx:#x} ({} seat write(s))", actions.len());
                        let _ = proxy.send_event(UserEvent::Notice("퀵슬롯에 등록했습니다"));
                    }
                    None if !pages.synced() => log::info!("bagbox: bind to plate {slot} dropped — the plate row is not synced to this character yet"),
                    None => log::info!("bagbox: bind to plate {slot} dropped — not a plate, or switched off"),
                }
                return;
            }
            let view = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::popup_view(&read)
            };
            // ★ `slot` is a PLATE. The game can only bind into one of its own five
            //   slots, so the plate is given a seat (`reserve`) and the popup is
            //   pointed at THAT row. No write of ours: the game is about to overwrite
            //   that slot itself, and clearing it first would only widen the window in
            //   which the bar is missing a binding. The seat is UNCONFIRMED for
            //   `quickpage::PENDING_POLLS` captures so the poll that runs before the
            //   game's own write does not conclude the plate is empty.
            //
            // ⛔ The whole gate FIRST (the popup may have closed, the cursor may have
            //   moved to another cell, the character may have changed) and the seat
            //   only after it passes. Reserving first would evict a live plate on the
            //   way to doing nothing — and leave its binding in a guest slot no plate
            //   owns, which a PAD button (which addresses guest slots by digit) could
            //   then still fire.
            let Some(v) = view.as_ref().filter(|v| items::popup_guest_row(v, 1, 0).is_some()) else {
                log::info!("popup: bind to plate {slot} dropped — popup no longer valid");
                return;
            };
            // ★ 157차: a plate holding a boxed consumable gives up its loan seat first,
            //   so the registration's seat reads empty until the game binds
            //   (`Pages::release_boxed_seat`).
            let freed = pages.release_boxed_seat(window, slot);
            if !freed.is_empty() {
                apply_quickpage_actions(emulator, &freed);
                log::info!("popup: plate {slot} held a boxed consumable's loan seat — cleared before the bind");
            }
            let Some(guest) = pages.reserve(window, slot) else {
                // `reserve` refuses two things and they are very different reports:
                // an out-of-range plate is a bug, a hidden one is the player's own
                // setting (2026-09-10). Say which.
                let why = if !pages.synced() {
                    "the plate row is not synced to this character yet (148차)"
                } else if crate::quickpage::plate_hidden(pages.hidden(), slot) {
                    "the player switched that plate off"
                } else {
                    "not a plate"
                };
                log::info!("popup: bind to plate {slot} dropped — {why}");
                return;
            };
            let Some(row) = items::popup_guest_row(v, 1, guest) else {
                window.vacate(guest);
                log::warn!("popup: bind to plate {slot} dropped — guest slot {guest} has no popup row");
                return;
            };
            log::info!("popup: binding plate {slot} through guest slot {guest} (guest row {row}, kind {})", v.kind);
            popup_commit(emulator, v, row);
        }
        InputEvent::PopupCancelPick => {
            *picking = false;
            let _ = proxy.send_event(UserEvent::PickingSlot(false));
            if bagbox.take_pending_bind().is_some() {
                log::info!("bagbox: 퀵슬롯등록 cancelled");
            }
        }

        // ── CLR on the cash shop's quantity spinner = 아니오 ──────────────────
        //
        // The game's own cancel, driven from here because both halves are guest state.
        // ⛔ Gated on the confirm box actually being up: the choice byte belongs to that
        // modal, and a synthetic OK with no box on screen would land on whatever screen
        // IS up. See `items::cash_confirm_choice_addr` for the handler this drives.
        InputEvent::CashBuyCancel => {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            match (items::cash_buy_confirm_open(&read), items::cash_confirm_choice_addr()) {
                (true, Some(at)) => {
                    emulator.write_memory(at, &[0u8]);
                    emulator.handle_event(Event::Keydown(KeyCode::OK));
                    emulator.handle_event(Event::Keyup(KeyCode::OK));
                    crate::set_cash_qty(1);
                    log::info!("cash: CLR — answered the game's confirm box with 아니오 (choice byte {at:#x} = 0)");
                }
                (open, at) => log::info!("cash: CLR dropped — confirm box open = {open}, choice byte = {at:x?}"),
            }
        }

        // ── quickbar page flip ────────────────────────────────────────────────
        //
        // Everything here needs guest memory, so the whole operation lives on this
        // thread and the UI only reported a tap. Order matters and is enforced by
        // `Pages::flip_to`: capture the outgoing page from a FRESH read, switch, then
        // plan the incoming one. Doing it in two steps from the UI side would lose a
        // page if the second half were dropped.
        // The player tapped SAVE. The gate and the whole four-call chain live on this
        // side because every one of their inputs is guest memory; the UI only reports
        // the tap and waits for a toast.
        InputEvent::TowerExit => {
            // ★ 143차: the EXIT tab's 「예」. The tower poll owns the run, so this only
            //   leaves the note; see the EXIT arm there.
            log::warn!("guardian: EXIT confirmed by the player");
            TOWER_EXIT_REQUEST.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        InputEvent::QuickSave => {
            // ★★ 143차: never inside the tower. A save made in a floor arena loads the player
            //   back into a room with no door (the pre-143차 soft-lock: the finished run went
            //   idle on the top floor, SAVE came back, and the save trapped the player). The
            //   tab is hidden there; this is the pad's quick-save button, which has no tab.
            if TOWER_NO_SAVE.load(std::sync::atomic::Ordering::Relaxed) {
                log::info!("save: refused — the player is in the tower");
                let _ = proxy.send_event(UserEvent::SideNote("하늘성에서는 저장할 수 없습니다".into()));
                return;
            }
            // ★ The control is NOT hidden and the tap is NOT short-circuited on a build
            //   whose save chain is unported (2026-08-11): removing shipped behaviour to
            //   paper over an unfinished port is the wrong trade. What the log does say
            //   is WHICH of the two situations produced a refusal, so a device report is
            //   readable — see the `save_chain_supported` note.
            if !crate::guest_build::save_chain_supported() {
                log::warn!(
                    "save: {} build — the host save chain's guest entry points are not ported yet (MIGRATION_LGT.md §4-3), so this tap cannot complete; the game's own menu -> SYSTEM save works",
                    crate::guest_build::name()
                );
            }
            let now = now_ms();
            if save_begin(emulator, saving, save::Reason::Manual) {
                saving.retry = None; // whatever was queued, this save covers it
            } else {
                let area_ok = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::can_save_here(&read)
                };
                if saving.job.is_none() && !area_ok {
                    // The AREA forbids it (boss room) — the game's own answer; say so.
                    log::info!("save: refused — the game's canSave byte says no here");
                    let _ = proxy.send_event(UserEvent::SideNote("저장불가".into()));
                } else {
                    // A save already in flight, or a transient state (window closing,
                    // stale field flag): retry briefly instead of lying about the place.
                    log::info!("save: busy/transient refusal — retrying for {MANUAL_RETRY_MS} ms");
                    saving.retry = Some((save::Reason::Manual, now + MANUAL_RETRY_MS));
                }
            }
        }
        InputEvent::QuickActivate { plate, cast } => {
            // ── admit, then press ────────────────────────────────────────────────
            //
            // The plate may not be in the guest's five-slot bar at all. `activate`
            // answers with the guest slot it belongs in plus the writes that put it
            // there — the SAME writes a page flip has always used — and both halves
            // run here, on the worker, holding the emulator. That is what makes the
            // pair atomic with respect to the guest: there is no poll in between for
            // the game to observe a bar we are half-way through arranging.
            let plan = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                pages.activate(window, plate, &|cat| items::class_of(&read, cat), &|tab, s| items::item_at(&read, tab, s))
            };
            // ★ 소비가방 (2026-09-14): a boxed consumable is materialised first and pressed
            //   by the job once the unit is in a bag cell.
            if let quickpage::Activation::Boxed { cat, idx } = plan {
                bagbox.press_plate(emulator, plate, cat, idx, proxy);
                return;
            }
            let quickpage::Activation::Press { guest, actions, dropped } = plan else {
                // Empty plate, or a stamp that no longer holds. Pressing the guest
                // slot's key anyway would fire whatever ELSE is sitting there.
                if plan == (quickpage::Activation::Empty { stale: true }) {
                    // The plate was just emptied, so the store moved: persist, or the
                    // ghost comes back on the next launch.
                    crate::config::write_quickpages(data_dir, pages);
                    log::info!("quickbar: plate {plate} was stale — emptied");
                } else if plan == quickpage::Activation::Unsynced {
                    // ★ 148차: the few frames between a load and the plate row's sync.
                    log::info!("quickbar: plate {plate} not pressed — a character was just loaded and the plate row is not synced to it yet");
                } else if crate::quickpage::plate_hidden(pages.hidden(), plate) {
                    // Not reachable from a finger (the rect is zero-sized) and gated on
                    // the pad too, so this is the belt-and-braces line — and the only
                    // one that would explain a "패드 버튼이 안 먹는다" report.
                    log::info!("quickbar: plate {} is hidden — nothing pressed", plate + 1);
                } else {
                    log::info!("quickbar: plate {plate} is empty — nothing pressed");
                }
                return;
            };
            for d in &dropped {
                log::info!("quickbar: dropped an entry admitting plate {plate}: {d:?}");
            }
            if !actions.is_empty() {
                log::info!("quickbar: plate {plate} admitted into guest slot {guest} ({} action(s))", actions.len());
                apply_quickpage_actions(emulator, &actions);
                // The bar changed, so the store's own copy of it is a poll behind.
                // Persisting here (rather than at the next flip) is what keeps plates
                // 5..6 across a crash, and the file only moves when a plate does.
                crate::config::write_quickpages(data_dir, pages);
            }
            let Some((code, _)) = crate::quickslot_hotkey(guest) else {
                log::warn!("quickbar: guest slot {guest} has no hotkey — plate {plate} not pressed");
                return;
            };
            // One-shot hotkey, exactly what the plate used to send from the UI thread —
            // then arm the cast watch (see `SkillWatch`): only a CONFIRMED cast answers
            // with `UserEvent::SkillCast`, which is what starts the UI cooldown.
            emulator.handle_event(Event::Keydown(code));
            emulator.handle_event(Event::Keyup(code));
            if cast {
                let (sp0, active0) = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    (items::read_sp(&read), items::skill_cast_active(&read))
                };
                // The watch reports back in PLATE terms: the UI's cooldown is drawn on
                // the plate the player pressed, not on the guest slot it borrowed.
                *skill_watch = Some(SkillWatch {
                    slot: plate,
                    sp0,
                    active0,
                    deadline: Instant::now() + SKILL_WATCH_WINDOW,
                });
            }
        }
        InputEvent::QuickPageNext => {
            // ★ 148차: `flip_to` refuses too; saying so here keeps the store file and the UI's
            //   page number from being touched by a flip that did not happen.
            if !pages.synced() {
                log::info!("quickpage: flip refused — a character was just loaded and the plate row is not synced to it yet");
                return;
            }
            let to = pages.next_page();
            let (live, class_seen) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                (items::quickbar_identities(&read), items::class_of(&read, 0).is_some())
            };
            // No CHAR yet (boot / teardown): a flip now would capture five empty slots
            // over a real page. Refuse and leave the store untouched.
            if live.len() != quickpage::GUEST_SLOTS || !class_seen {
                log::info!("quickpage: flip to {to} refused — no live character");
                return;
            }
            let (actions, dropped) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                pages.flip_to(
                    to,
                    window,
                    &live,
                    &|cat| items::class_of(&read, cat),
                    &|tab, slot| items::item_at(&read, tab, slot),
                )
            };
            for d in &dropped {
                // Worth a line each: this is why a plate the player filled came back
                // empty, and it is the first thing they will ask about.
                log::info!("quickpage: dropped an entry restoring page {to}: {d:?}");
            }
            apply_quickpage_actions(emulator, &actions);
            crate::config::write_quickpages(data_dir, pages);
            let _ = proxy.send_event(UserEvent::QuickPage(pages.current() as u8));
            log::info!("quickpage: now on page {} ({} action(s))", pages.current(), actions.len());
        }
    }
}

/// How many quickbar scans the plate-row reconciliation may wait for the live
/// character's identity to become readable before deciding without it.
///
/// The scan runs every 15 frames, so this is a couple of seconds — long enough for the
/// world-init write at guest `0x3ab68` to have happened, short enough that a session
/// where it never does (title -> online -> warehouse, where `CHAR+0xb8` stays 0 —
/// 71차-L) still ends up with a plate row rather than none.
const QB_SYNC_WAIT: u8 = 12;

/// ★★★ 148차 — how long after an observed character LOAD every "was the shell reached?"
/// question in the worker answers yes.
///
/// Those questions are samples of the app-state word at a cadence (every 5th frame for the
/// tower's debts, heal and quick-start picker; every 30th for the run itself), and a player
/// mashing OK through 메인메뉴 → 게임시작 → a slot takes the game from one world to the next
/// in TWO ticks (device log 2026-10-01 00:25:25.9–26.7) — the shell existed for one worker
/// iteration and none of them saw it. A load is the event those questions stand for ("the
/// world was replaced by a record from disk"), so it answers them for one full cycle of the
/// slowest of them: 32 consecutive frames contain every residue mod 30.
const WORLD_SWAP_FRAMES: u64 = 32;

/// Run a page-restore plan against the guest.
///
/// Skills go through the game's own `skill_bind`; items and clears are raw writes into
/// the four quickbar arrays. See `guest_build::skill_bind_fn` / `items::slot_writes` for
/// why the two halves differ.
///
/// ★ 2026-08-11: this used to bail out entirely unless `guest_code_addresses_valid()`, so on
/// LGT a page tap flipped the STORE and changed nothing in the game — the player's "퀵슬롯
/// 페이지 버튼이 동작을 안한다". Only `BindSkill`'s guest CALL ever needed a code address, and
/// only where the build has one; clears and item binds are data writes at offsets §2.16
/// verified on LGT. Split accordingly (MIGRATION_LGT.md §3.6: per item, never the flag).
pub(crate) fn apply_quickpage_actions(emulator: &mut dyn Emulator, actions: &[quickpage::Action]) {
    for a in actions {
        // ⛔ The one hard bound, checked at the write boundary rather than trusted from
        // the planner: past the guest's five slots `CHAR+0x252+slot` IS `itemref[0]`
        // (the three arrays are back to back — `quickpage.rs` header), and the guest's
        // own register path has no upper bound of its own. A plan that ever produced
        // one of these would corrupt the character record silently.
        if usize::from(a.guest()) >= quickpage::GUEST_SLOTS {
            log::warn!("quickpage: refusing {a:?} — guest slot past the bar");
            continue;
        }
        match *a {
            quickpage::Action::Clear { guest } | quickpage::Action::BindItem { guest, .. } => {
                let item_ref = match *a {
                    quickpage::Action::BindItem { item_ref, .. } => Some(item_ref),
                    _ => None,
                };
                let writes = {
                    let read = |ad: u32, l: usize| emulator.read_memory(ad, l);
                    items::slot_writes(&read, guest, item_ref)
                };
                let Some(writes) = writes else {
                    log::warn!("quickpage: guest slot {guest} write refused (CHAR gone or slot out of range)");
                    continue;
                };
                for (addr, byte) in writes {
                    if !emulator.write_memory(addr, &[byte]) {
                        log::warn!("quickpage: write failed at {addr:#x}");
                    }
                }
            }
            quickpage::Action::BindSkill { cat, cell, guest, sref } => match crate::guest_build::skill_bind_fn() {
                Some(f) => {
                    // The game does the learned check, the dedup and the ref packing.
                    // `guest` is < 5 by construction (quickpage refuses anything else) —
                    // the guest dispatcher has no bound of its own.
                    emulator.call_guest_function(f, &[u32::from(cat), u32::from(cell), u32::from(guest)]);
                }
                // No located `skill_bind` on this build: write back the four bytes the
                // game itself wrote when the player made this binding.
                None => match sref {
                    Some(sref) => {
                        let writes = {
                            let read = |ad: u32, l: usize| emulator.read_memory(ad, l);
                            items::skill_slot_writes(&read, guest, sref, cell)
                        };
                        match writes {
                            Some(writes) => {
                                for (addr, byte) in writes {
                                    if !emulator.write_memory(addr, &[byte]) {
                                        log::warn!("quickpage: skill write failed at {addr:#x}");
                                    }
                                }
                            }
                            None => log::warn!(
                                "quickpage: guest slot {guest} skill write refused (CHAR gone, or this build's quickbar offsets are unverified)"
                            ),
                        }
                    }
                    // A store file written before the ref column existed, on a build with
                    // no `skill_bind`. Nothing correct to write, so the plate stays empty —
                    // and says why, because the player will ask.
                    None => log::info!(
                        "quickpage: guest slot {guest} left empty — the stored skill predates the ref column and {} has no located skill_bind",
                        crate::guest_build::name()
                    ),
                },
            },
        }
    }
}

/// Make the GAME take popup row `row`: write its highlighted-row byte, then call its
/// own popup key handler with the select key.
///
/// This is the only host->guest write in the feature, and it is one byte. Everything
/// that follows — the bind, the unbind sentinel, the "is the skill learned" guard,
/// the item's existing-binding cleanup, the popup teardown — is the game's own code
/// running on its own stack.
fn popup_commit(emulator: &mut dyn Emulator, view: &items::PopupView, row: u8) {
    let Some(how) = crate::guest_build::popup_commit() else {
        log::warn!("popup: no commit mechanism for the {} build — row {row} dropped", crate::guest_build::name());
        return;
    };
    // ★ Restore the NATIVE row count first, and only on the synthetic-OK path. See
    //   `items::popup_count_restore` for the tick this closes.
    if how == crate::guest_build::PopupCommit::SyntheticOk {
        let restore = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::popup_count_restore(&read, view)
        };
        if let Some((a, native)) = restore
            && !emulator.write_memory(a, &[native])
        {
            log::warn!("popup: could not restore the native row count at {a:#x}");
            return;
        }
    }
    let addr = {
        let read = |a: u32, l: usize| emulator.read_memory(a, l);
        items::popup_sel_addr(&read)
    };
    let Some(addr) = addr else {
        log::warn!("popup: CHAR vanished before commit");
        return;
    };
    if !emulator.write_memory(addr, &[row]) {
        log::warn!("popup: could not write the highlighted row at {addr:#x}");
        return;
    }
    match how {
        crate::guest_build::PopupCommit::GuestCall { fn_addr, select_key } => {
            emulator.call_guest_function(fn_addr, &[u32::from(view.kind), select_key]);
        }
        // The same two events the row-0 pass-through above sends, which is what a finger
        // on the handset produces. The game's own popup dispatcher then reads the row
        // byte we just wrote.
        crate::guest_build::PopupCommit::SyntheticOk => {
            emulator.handle_event(Event::Keydown(KeyCode::OK));
            emulator.handle_event(Event::Keyup(KeyCode::OK));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    proxy: EventLoopProxy<UserEvent>,
    should_redraw: Arc<AtomicBool>,
    input_rx: Receiver<InputEvent>,
    data_dir: PathBuf,
    game_zip: Vec<u8>,
    paused: Arc<AtomicBool>,
    hud_y: crate::HudYShift,
) {
    thread::Builder::new()
        .name("wie-emu".into())
        .stack_size(EMU_STACK)
        .spawn(move || run(proxy, should_redraw, input_rx, data_dir, game_zip, paused, hud_y))
        .expect("failed to spawn emulator thread");
}

/// The ring's remembered pictures (153차), in the app-private data dir.
const PORTRAIT_MEMORY_FILE: &str = "wie_portraits.bin";

/// Save what the ring remembers. A cache — a lost write costs one front view, so it is not
/// fsynced; the replace is still atomic, so a torn file cannot be read back.
fn save_portrait_memory(path: &std::path::Path, fp: u64, keeper: &items::PortraitKeeper<crate::Portrait>) {
    let entries: Vec<(u64, bool, u32, u32, &[u32])> =
        keeper.memory().iter().map(|(look, settled, p)| (*look, *settled, u32::from(p.w), u32::from(p.h), p.px.as_slice())).collect();
    let bytes = items::encode_memory(fp, &entries);
    if let Err(e) = crate::database::write_atomic_with(path, &bytes, false) {
        log::warn!("portrait: could not save the remembered looks to {path:?}: {e}");
    }
}

/// One line for what the keeper did with the look on the field, when it did something.
fn portrait_observe_log(look: u64, what: items::Observed) {
    match what {
        items::Observed::Recalled { settled } => log::warn!(
            "portrait: look {look:#018x} was seen before — its picture is up at once ({})",
            if settled { "final" } else { "to be confirmed at another spot" }
        ),
        items::Observed::Dropped => log::warn!(
            "portrait: a different look is on the field ({look:#018x}), never seen — the last character's picture is dropped until this one faces the viewer"
        ),
        items::Observed::Same | items::Observed::Waiting => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    proxy: EventLoopProxy<UserEvent>,
    should_redraw: Arc<AtomicBool>,
    input_rx: Receiver<InputEvent>,
    data_dir: PathBuf,
    game_zip: Vec<u8>,
    paused: Arc<AtomicBool>,
    hud_y: crate::HudYShift,
) {
    // The screen consumes `proxy` for Paint; keep a clone for the item-HUD channel.
    let item_proxy = proxy.clone();
    let (logical_w, logical_h) = crate::logical();
    log::info!("emulator: guest framebuffer {logical_w}x{logical_h}");
    let screen = AndroidScreen::new(proxy, should_redraw.clone(), logical_w, logical_h);
    // The synth reads the same `paused` flag to emit silence in lock-step.
    let platform: Box<dyn Platform> = Box::new(AndroidPlatform::new(screen, data_dir.clone(), paused.clone()));

    // ★ Guest PC profiler, off unless `wie_profile.txt` says otherwise — see
    // `config::read_guest_profile`. The callback runs ON the emulator thread inside the
    // interpreter, so it does the cheapest possible thing: fold the batch into a
    // leaf-address histogram and let the 30 s gauge print the top of it. Formatting or
    // logging per batch here would be measuring the profiler.
    // Hoisted out of the `Options` literal: the tick loop needs to know whether samples
    // exist, so it can give each tick its own set for the hitch report.
    let profiling = config::read_guest_profile(&data_dir);
    let options = Options {
        enable_gdbserver: false,
        profile: if profiling {
            Some(Box::new(move |batch: Vec<wie_backend::ProfileSample>| {
                let mut hist = PROFILE_HIST.lock().unwrap_or_else(|e| e.into_inner());
                for s in batch {
                    if let Some(pc) = s.stack.first() {
                        // Thumb bit off: this is an ADDRESS, and it has to match what
                        // `tools_re/h3dis.py` prints.
                        *hist.entry(*pc & !1).or_insert(0) += s.count;
                    }
                }
            }))
        } else {
            None
        },
    };

    let files = match extract_zip(&game_zip) {
        Ok(f) => f,
        Err(e) => {
            log::error!("failed to extract game zip: {e:?}");
            return;
        }
    };
    // ★ 153차: the ring's remembered pictures belong to THIS game asset (`portrait.rs`
    // `asset_fingerprint` — an update that recolours an item keeps its look key).
    let portrait_fp = items::asset_fingerprint(&game_zip);
    // ★ 167차 아바타 옷장: which costume sheets the jar carries — read off its own central
    // directory, ONCE, before the guest exists. The two paper-doll hooks never hand the game a
    // sheet that is not here (all-zero masks = the hooks change nothing).
    match files.iter().find(|(name, _)| name.ends_with(".jar")) {
        Some((_, jar)) => {
            let masks = crate::avatar_rules::sheets_in_zip(jar);
            let n: u32 = masks.iter().map(|m| m.count_ones()).sum();
            wie_util::stats::set_avatar_sheets(masks);
            log::warn!("avatar: the jar carries {n} costume sheet(s)");
            // ★ 171차: the sets whose 투구 sheet draws the whole head (costume.txt `head = own`).
            let own_head = crate::avatar_catalog::SETS
                .iter()
                .filter(|s| s.own_head && (1..=wie_util::avatar::SET_MAX).contains(&s.id))
                .fold(0u64, |m, s| m | 1 << (s.id - 1));
            wie_util::stats::set_avatar_own_head(own_head);
            log::warn!("avatar: {} set(s) draw the whole head in their 투구 sheet", own_head.count_ones());
        }
        None => log::warn!("avatar: no jar in the archive — no costume sheets"),
    }

    let mut emulator: Box<dyn Emulator> = if KtfEmulator::loadable_archive(&files) {
        crate::guest_build::set_ktf();
        match KtfEmulator::from_archive(platform, files, options) {
            Ok(e) => Box::new(e),
            Err(e) => {
                log::error!("KTF load failed: {e:?}");
                return;
            }
        }
    } else if LgtEmulator::loadable_archive(&files) {
        crate::guest_build::set_lgt();
        // Not a downgrade of the byte-validation guards — see guest_build.rs for why
        // this has to be airtight now that LgtEmulator can run host-initiated calls.
        // The split is between DATA (ported: the CHAR record is a `.bss` static and
        // every container hop is one deref shorter) and guest CODE addresses (not
        // ported: none of the KTF routine addresses exist in this compile).
        // ★ The per-item state is READ OUT OF THE TABLES, not spelled out in the string
        // (2026-08-11). The hand-written list had gone stale in exactly the way §3.6's
        // per-item discipline invites: it still said "HUD skip, EXP … inactive" three
        // rounds after both had shipped their measured LGT addresses, so the one line a
        // device session trusts to say what is live was lying about two features.
        log::warn!(
            "{} build: data-side features active (CHAR gauges, quickbar scan, window/state gating, minimap, full-restore); containers {}; per-item guest-code patches: hud_bottom={} exp_divisor={} equip_relief={} band_x={} target_box={}; quickbar writes {} (skill_bind {}); cash-buff trigger (popup labels) {}; save chain (host quick-save + cash-item commit) {}; auto-loot {}; still gated (guest_code_addresses_valid=false): hud_y origin, band height; blit565_lgt + blend565_lgt + blit_lut565_lgt install via binary_patches",
            crate::guest_build::name(),
            match crate::guest_build::container_data_off() {
                Some(off) => format!("= *(handle) + {off:#x}"),
                None => "= direct pointers".to_string(),
            },
            crate::guest_build::hud_bottom_patch().map_or("-".to_string(), |(s, _, _)| format!("{s:#x}")),
            crate::guest_build::exp_divisor_site().map_or("-".to_string(), |s| format!("{s:#x}")),
            match crate::guest_build::equip_relief_site() {
                Some(crate::guest_build::EquipReliefSite::MoveHalfword { site }) => format!("{site:#x} (halfword)"),
                Some(crate::guest_build::EquipReliefSite::LevelBias { site }) => format!("{site:#x} (level-bias pair)"),
                None => "-".to_string(),
            },
            // The two HUD widgets the player asked to be moved out from under our own
            // chrome (2026-08-11): the game's buff band goes right of our gauges and its
            // target-monster box left of our minimap. Both are lists of one-byte
            // immediates, so "how many did this build supply" is the whole status.
            match crate::guest_build::band_x_sites().len() {
                0 => "-".to_string(),
                n => format!("{n} sites"),
            },
            match crate::guest_build::target_box_x_sites().len() {
                0 => "-".to_string(),
                n => format!("{n} sites"),
            },
            if crate::guest_build::quickbar_writes_ok() { "open" } else { "gated" },
            crate::guest_build::skill_bind_fn().map_or("unlocated -> raw ref writes".to_string(), |f| format!("{f:#x}")),
            // Without this the EXP and relief patches can never be ASKED for, however
            // right their sites are: `cash_use_pending` is the only thing that grants a
            // buff, and it reads the highlighted popup row's label through this array.
            match crate::guest_build::popup_handles() {
                Some(crate::guest_build::PopupHandles::Pointer { global }) => format!("via slot {global:#x}"),
                Some(crate::guest_build::PopupHandles::Static(base)) => format!("via .bss array {base:#x}"),
                None => "NOT LOCATED — no cash buff can start".to_string(),
            },
            // The control stays on screen either way (see `app::quicksave_zone`); this
            // is here so a device report says WHICH situation a refusal came from.
            if crate::guest_build::save_chain_supported() { "ACTIVE" } else { "guest entry points unported — SAVE taps cannot complete (MIGRATION_LGT.md §4-3)" },
            // ★ Read from the table for the same reason as everything above it. This line
            // said "still gated: autoloot" for six weeks while the feature was dead on the
            // shipped build, and the server had delisted the cash item that does the same
            // job on the strength of that claim. Never hand-write a capability again.
            match crate::guest_build::autoloot_flag() {
                // 50차: the flag is an ARRAY and the save slot picks the element, so the
                // line has to say that — a fixed address here is precisely the bug that
                // made this feature dead for every character not in slot 1.
                Some(f) => match f.slot_off {
                    Some(slot) => format!("forced ON (CHAR+{:#x} + saveSlot@CHAR+{slot:#x} <- {:#04x})", f.base_off, f.value),
                    None => format!("forced ON (CHAR+{:#x} <- {:#04x})", f.base_off, f.value),
                },
                None => "unlocated — the 오토루팅 cash item is the only way".to_string(),
            },
        );
        match LgtEmulator::from_archive(platform, files, options) {
            Ok(e) => Box::new(e),
            Err(e) => {
                log::error!("LGT load failed: {e:?}");
                return;
            }
        }
    } else if SktEmulator::loadable_archive(&files) {
        crate::guest_build::set_skt();
        log::warn!("SKT build: KTF guest-address features inactive");
        match SktEmulator::from_archive(platform, files) {
            Ok(e) => Box::new(e),
            Err(e) => {
                log::error!("SKT load failed: {e:?}");
                return;
            }
        }
    } else {
        // J2ME archives are jad/jar based, not zip; treat unknown zip as fatal.
        log::error!("unknown archive format (not KTF/LGT/SKT)");
        let _ = J2MEEmulator::from_jar; // keep the import meaningful for future jar support
        return;
    };

    log::info!("emulator loaded; entering tick loop");

    let mut held: HashMap<KeyCode, Instant> = HashMap::new();
    let mut walk = WalkOrder::default();
    // The held attack key, its rule (`atkspeed::AttackHold`) and how many re-presses it has
    // sent — the count is only for the log lines that say what a hold did.
    let mut attack_hold: Option<(KeyCode, crate::atkspeed::AttackHold, u32)> = None;
    let hold_epoch = Instant::now();
    let mut frame: u64 = 0;
    // Whether the one-shot auto-loot capability line has been logged for this run.
    let mut autoloot_notice_done = false;
    // Highest `stack_rss_kb` seen, so the log is a staircase and not a stream.
    let mut stack_rss_peak: u64 = 0;
    // Last HUD state pushed; only re-send on change.
    let mut last_slots: Vec<QuickSlot> = Vec::new();
    // The minimap as last published. Rendered by the host from the guest's own map
    // arrays, so it is simply whatever the current map looks like — there is no
    // acquisition state to track any more.
    let mut last_minimap: Option<crate::Minimap> = None;
    // ★ 141차: the minimap's moving marks (player + monsters) and the status ring's
    // portrait, each as last published — both change far more often than the map.
    let mut last_marks: Option<(Option<(f32, f32)>, Vec<(f32, f32)>)> = None;
    let mut last_portrait: Option<crate::Portrait> = None;
    // ★ 153차: the ring's picture, kept per WORN LOOK (`portrait.rs` `PortraitKeeper`) —
    // fed after every tick that drew the player on the idle loop's first frame, so it is
    // the same picture for as long as the equipment is the same. `portrait_seq_seen` is
    // the capture's run count at the last look (`wie_util::stats::PORTRAIT_SEQ`).
    let mut portrait_keeper: items::PortraitKeeper<crate::Portrait> = items::PortraitKeeper::new();
    let mut portrait_seq_seen: u32 = 0;
    // …and what it remembers (`<data>/wie_portraits.bin`): every look seen before, so a save
    // loaded facing away from the viewer has its picture at once (player, 2026-10-02:
    // *"세이브를 로드 후에 움직이기 전까지 외형표시가 안나오고있는데?"*).
    let portrait_memory_path = data_dir.join(PORTRAIT_MEMORY_FILE);
    {
        let entries = std::fs::read(&portrait_memory_path).map(|b| items::decode_memory(&b, portrait_fp)).unwrap_or_default();
        log::info!("portrait: {} remembered look(s) loaded from {portrait_memory_path:?}", entries.len());
        portrait_keeper.set_memory(
            entries.into_iter().map(|(look, settled, w, h, px)| (look, settled, crate::Portrait { w: w as u16, h: h as u16, px })).collect(),
        );
    }
    // Last line the focus probe printed, so an unchanging screen stays silent (72차).
    let mut last_focus_probe: Option<String> = None;
    let mut last_stats: Stats = Stats::default();
    let mut last_field = false;
    let mut last_cutscene = false;
    let mut last_choice = false;
    let mut last_map_open = false;
    let mut last_shell = false;
    let mut last_shell_screen: Option<u32> = None;
    let mut last_can_save = true;
    /// ★★ OFF since 2026-08-09, and this is a deliberate feature stand-down, not a tidy-up.
    ///
    /// 27차 hid the SAVE button and the minimap wherever it believed saving was refused,
    /// reading `(s8)*(CHAR+0xa280)`. That byte is the shared list widget's CURSOR ROW (see
    /// `crate::save::AREA_CAN_SAVE_OFF` for the disassembly), so the feature fired on an
    /// ordinary field and stayed fired — device-measured 2026-08-09, thirteen minutes with
    /// both controls gone, and the player found the mechanism before the RE did: opening any
    /// menu and closing it brought them back, because opening a menu is the only thing that
    /// writes that cursor to 0.
    ///
    /// The gate is now the game's REAL one (`items::area_can_save`). But its value comes out
    /// of each map's own data file, so `client.bin` cannot say which maps — if any — actually
    /// ship a refusal. Hiding UI on an unmeasured signal is exactly the mistake that produced
    /// this bug, so the signal is READ and LOGGED this round and hides nothing. Turn this on
    /// once a device log shows `area gate: can_save=Some(false)` in a boss room and
    /// `Some(true)` on the field; if `Some(false)` never appears anywhere, delete the feature.
    ///
    /// Nothing else changes: `save_begin` still asks the game, so a genuine refusal still
    /// answers with the 저장불가 toast instead of a silent no-op.
    const HIDE_UI_ON_NO_SAVE: bool = false;
    /// Consecutive "no-save" samples before the UI believes it. Was 24 (~2 s) to paper over
    /// the wrong byte's flicker; the real per-area byte only changes when the area does, so
    /// two samples are enough to ride out a map transition.
    const NO_SAVE_DEBOUNCE: u32 = 2;
    let mut no_save_run: u32 = 0;
    let mut last_secondary: Option<crate::IconRef> = None;
    // Cash-buff status lines as last published (menu-mode HUD, ~minute granularity).
    let mut last_buff_lines: Vec<String> = Vec::new();
    let mut last_field_line: Option<String> = None;
    // ★ 145차: the 하늘성 banner as last published.
    let mut last_tower_hud: Option<crate::guardian::TowerHud> = None;
    // ★ 143차: the EXIT tab as last published.
    let mut last_tower_exit = false;
    // 하늘성 입장 연출. Bumped by the entrance dialogue, published on the next HUD edge.
    let mut tower_flash: u32 = 0;
    // Whether the UI has been told the HUD state at least once (see below).
    let mut hud_published = false;
    // The game's rewritten context popup, as last published.
    let mut last_popup: Option<items::PopupView> = None;
    let mut last_popup_list: Option<items::PopupList> = None;
    // The 제련 odds readout, as last published. `Copy`, so this is a plain compare.
    let mut last_refine: Option<crate::refine::Refine> = None;
    // ★ 150차: the 상세보기 note — what it was built from (the target and the worn set, so a
    // frame where neither moved costs no rebuild), and what was last sent (identity + note).
    let mut note_basis: Option<(items::NoteTarget, Vec<(u16, u16)>)> = None;
    let mut last_note: Option<(u64, crate::itemnote::ItemNote)> = None;
    // ★ 71차-J: the cursor/identity FROM THE PREVIOUS POLL, so `load_pin_slot` can
    // pin the slot `loadSaveRecord` actually read instead of the drifted list cursor.
    let mut pin_prev_cursor: Option<u8> = None;
    let mut pin_prev_id: u64 = 0;
    // ★★★ 148차: the character-LOAD counter as last seen (`vault::load_generation`), and
    //   what a load owes the rest of this loop: the pin latch's next poll (its previous
    //   cursor described the OUTGOING world) and the tower's per-world bookkeeping, which
    //   reads "a load happened" as "the shell was reached" until `world_swap_until`. See
    //   the block right after the tick.
    let mut seen_load_gen = crate::vault::load_generation();
    let mut loaded_since_poll = false;
    let mut world_swap_until: u64 = 0;
    // ★ 161차: the slot lists open on the last loaded slot. `slot_list_was_up` is the
    //   previous frame's "a slot list is up" (its rising edge is the OPENING — a modal
    //   closing over a list that is already up is not one), `slot_focus_pending` is an
    //   opening whose cursor is not placed yet (a modal may own the list for a frame), and
    //   `slot_focus_written` is what `wie_save_last.txt` holds.
    let mut slot_list_was_up = false;
    let mut slot_focus_pending = false;
    let mut slot_focus_written: Option<u8> = crate::slot_focus();
    // ★ 71차-K: warehouse session edge + slot-list lock after 네트워크 → 뒤로.
    let mut was_warehouse = false;
    // ★ 78차: the previous poll's `items::shell_state`, so the creation pin can fire on the
    // shell → world EDGE instead of on every poll. Starts `true`: boot is the shell.
    let mut was_shell = true;
    // The UI is highlighting the plates, waiting for one to be tapped.
    let mut picking = false;
    // Host-owned quickbar pages. Page 0 IS the guest's bar, so a fresh/empty store
    // behaves exactly as before the feature existed.
    let mut pages = config::read_quickpages(&data_dir);
    // Which plate is in which of the guest's five slots right now. Session state — the
    // guest's bar is rebuilt from the save on every load, so there is nothing to persist
    // (see `quickpage::Window`).
    let mut qb_window = quickpage::Window::new();
    // ★ 148차: no bar has been reconciled with the store yet — nothing may write the row
    //   from live state until the first sync (the `synced` field of `quickpage::Pages`).
    pages.desync(&mut qb_window);
    // "A character was just loaded; reconcile the plate row with it at the next scan."
    let mut qb_sync = false;
    // ...and how many more scans that reconciliation may wait for a readable identity.
    let mut qb_sync_wait = QB_SYNC_WAIT;
    // The plate row's own writes cannot run inside the read-only poll block (it holds an
    // immutable borrow of the emulator), so they are carried out of it.
    let mut qb_restore: Option<Vec<quickpage::Action>> = None;
    // A slot that just received a newly created character this poll — its bag boxes go
    // with the character that used to live there (`BagBoxHost::forget_created_slot`).
    // Carried out of the read block because that borrows the emulator.
    let mut created_slot: Option<u8> = None;
    let mut qb_store_dirty = false;
    let _ = item_proxy.send_event(UserEvent::QuickPage(pages.current() as u8));
    let _ = item_proxy.send_event(UserEvent::SlotWindow {
        first: crate::slot_first(),
        total: crate::slotwindow::TOTAL,
    });
    // Host-owned cash-shop buffs (EXP x2/x4, equip level relief). Persisted as
    // absolute UNIX-ms expiries, so they run down while the app is closed.
    //
    // ★ With no buff live this costs the guest NOTHING: both patch helpers ask for the
    // ORIGINAL halfword, find it already there, and return `None`. So a build carrying
    // this feature is byte-identical in the guest until a player actually uses one of
    // the three items — which matters while we are still bisecting a crash.
    // Host-driven save (quick-save button + cash-item commit retry). See `save.rs`.
    let mut saving = SaveState::new();
    // `wie_grant.txt`: one guest give per poll into ONE save slot's bag, then a save.
    // `grant_wait` is the row in flight (`call_guest_function` is async), `grant_done`
    // counts gives this session, `grant_saved` arms the save exactly once.
    let mut grant = crate::config::read_grant();
    let mut grant_wait: Option<(u16, u16, u64)> = None;
    let mut grant_done: u32 = 0;
    let mut grant_saved = false;
    if let Some(g) = &grant {
        log::warn!("grant: {} row(s) queued for save slot {} (waiting for that slot to be live in the field)", g.items.len(), g.slot);
    }
    // 하늘성 / THE TOWER (2026-09-08). The floor state machine is `crate::guardian`
    // and is pure; this loop is the half that reads and writes the guest. The wiring
    // plan is `GUARDIAN_CASTLE_FEASIBILITY.md §C.4`.
    //
    // ★ WITH NO FLOOR TABLE THIS COSTS NOTHING. `Run::new(vec![])` starts `Idle`,
    // `Idle` emits `Action::None` for every poll, and `forking()` is false — so the
    // guest is byte for byte the shipped game until a player actually starts a run.
    // That is the same invariant the buff table keeps (§4-4).
    // ★ 114차: the VANILLA flavour has no tower at all (`flavor::TOWER` is false there) —
    //   the floor table is never read, so a pushed `wie_guardian_floors.txt` is inert and
    //   this run is `Idle` for the life of the process. Same zero-cost invariant as above.
    let mut tower = crate::guardian::Run::new(if crate::flavor::TOWER {
        crate::config::read_guardian_floors()
    } else {
        vec![]
    });
    // ★ 138차: progress is per SAVE SLOT (`guardian::Progress`) — loaded when the pin names
    // a slot (`sync_tower_progress`), written the moment a floor is cleared. The old
    // device-wide file is ignored (결정 ⑤) and says so once.
    crate::config::note_legacy_guardian_progress(&data_dir);
    let mut tower_prog = crate::guardian::Progress::default();
    let mut tower_prog_slot: Option<u8> = None;
    // ★ 138차: the quick-start picker's floors while it is on screen. It answers through
    // the shared `orb_answer` channel, which is safe because it only ever opens at the
    // entrance NPC — no box window or target picker can be up there.
    let mut tower_pick: Option<Vec<u8>> = None;
    // ★ 138차: the exit reward's shards, paid straight into the 재료가방 on a field poll,
    // and how many polls that has waited for a box to be armed before the game's giver
    // takes them instead.
    let mut tower_shards_owed: u32 = 0;
    let mut tower_shard_waits: u32 = 0;
    // A control-file request waiting for a poll that can read the guest.
    let mut tower_cmd: Option<crate::guardian::Command> = None;
    // The floor whose clear has already been counted (its EXP, the slot's progress), so
    // `Action::Reward` — which is offered on every poll, not once — counts once.
    let mut tower_paid: Option<u8> = None;
    // ★ 138차: the EXIT reward still owed (`queue_exit_reward`), as `(cat, idx)` rows to
    // hand to the game's own giver — one per poll, exactly like `wie_grant.txt` does, and
    // ANNOUNCED (the giver's third argument, 103차), so the player sees what came. The
    // shards do not come this way (`tower_shards_owed`).
    //
    // ⛔ Do NOT arm a save after paying: saving is FORKED OFF for the whole run
    // (`guardian: floor N — saving disabled`), so a save here would be refused by the
    // game's own canSave byte. The reward persists when the player leaves and saves,
    // which is why "save before entering" is the standing advice (§3.3).
    let mut tower_owed: std::collections::VecDeque<(u16, u16)> = std::collections::VecDeque::new();
    // The row in flight, because `call_guest_function` is asynchronous. Same shape and
    // same 90-frame patience as the maintenance grant.
    let mut tower_give_wait: Option<(u16, u16, u64)> = None;
    // The clear EXP still owed (112차, `Floor::exp`), summed across floors until a field
    // poll can hand it to the game's own `checkLevelUp`. Same field-only rule as the
    // item rows above, and for the same reason.
    let mut tower_exp_owed: u32 = 0;
    // Polls to linger on a cleared floor before moving up, so the player can loot and
    // read the toast. A PRESENTATION delay, not a rule of the tower, which is why it
    // lives here and not in `guardian.rs`; the "next floor / leave" choice this stands
    // in for needs the UI that is still to come (§C.4).
    const TOWER_CLEAR_HOLD: u32 = 20;
    // ★ 143차: the TOP floor's hold before the wake takes over — shorter, because nothing
    // comes after it but the way out, and the wake itself waits for a quiet screen
    // (`guardian::WakePhase::Quiet`) before it whitens — which since 2026-09-30 includes
    // the clear's own side line fading out (`crate::notice_up`), no longer a box.
    const TOWER_TOP_HOLD: u32 = 6;
    let mut tower_hold: u32 = 0;
    // ★ 143차 (testing only): the `clear` lever, armed until this frame. It is read from the
    // control file every 300 frames, and a weak test character dies about a second after
    // landing on the top floor — a lever that only acted on a floor already live when it
    // was read could never win that race. So it arms, and the first live floor inside the
    // window takes it (the block right after the control-file command below).
    const TOWER_AUTOCLEAR_FRAMES: u64 = 6_000;
    let mut tower_autoclear_until: u64 = 0;
    /// Polls an `Enter` request may wait for the field before it is given up on. The
    /// entrance NPC's dialogue window takes a frame or two to close, so some wait is
    /// mandatory; it is bounded so a request made in a menu the player never leaves
    /// reports itself instead of firing minutes later.
    const TOWER_CMD_WAITS: u32 = 30;
    let mut tower_cmd_waits: u32 = 0;
    // The AREA canSave byte as it read before the tower first zeroed it, kept until it
    // has been given back. ⛔ NOT OPTIONAL — see `crate::guardian::restore_can_save`
    // for the device measurement: without this, one tower run refuses every save for
    // the rest of the session, in the town the player came from.
    let mut tower_can_save_was: Option<u8> = None;
    // Polls left in which to check that the player is standing somewhere legal.
    //
    // A host-posted warp carries NO destination coordinates (`guardian::landing_tile`),
    // so the check has to run AFTER the game has serviced the request and loaded the
    // new map — which is a few polls later, and is also true of the leave warp back to
    // the town. Armed on every warp, and it keeps the poll block alive on its own so
    // the leave case is covered after the run has already ended.
    // ★★★ 2026-09-21 — A FRAME BUDGET, NOT A POLL BUDGET, AND NO KEY IS EVER PRESSED.
    //
    // Zero arrival facing caused both invisibility and the skill freeze, even with
    // correct coordinates and no injected keys. `guardian::WARP_ENTRY` now supplies
    // down (2) before loading; this block only chooses the destination coordinates.
    //
    // So the arrival is now a PLACEMENT and nothing else: frames left in which one may
    // land, and the scene it is waiting for. Spent a frame at a time rather than once
    // every 30, because until it lands the player is standing — and being drawn — on
    // the previous map's tile.
    let mut tower_land_frames: u32 = 0;
    let mut tower_arrive: Option<u8> = None;
    // ★★★ NEW GAME PLUS (2026-09-21) — how many 2회차 transitions have been handled.
    //
    // The game's 2회차 transition takes 그리폰의피리, its recipe and the quest items out of
    // the BAG, and it walks bag cells — so it cannot see the boxes, and the player
    // reported the result: *"난이도를 넘어가게 되면 소비가방에 여전히 남아있어"*. And a
    // hardcore clear opens 헬 (133차), which needs the difficulty the playthrough had
    // BEFORE the transition writes 1 over it.
    //
    // ★★ 135차-B: the `ending_transition_lgt` hook on the transition's own entry
    // (`0x3af2c`, `binary_patches/ending.rs`) counts every run and latches that byte. This
    // used to be ending window 18's rise-and-fall on a 5-iteration poll, and the window
    // lives only until the next key — a mashed ending (3 frames, probe-measured, 135차
    // §2 ②) skipped both halves with no log line. The counter does not expire.
    //
    // ⛔ NOT the difficulty byte. `0 -> 1` looks like the same event but world
    // initialisation zeroes it and then restores it from the save (`0x3abb0` →
    // `0x3ac60`), so LOADING a hard character produces that edge too — and firing there
    // would delete a 그리폰의피리 the player legitimately owns. It also cannot see a
    // hard→hard clear, which writes 1 over 1.
    let mut ending_transitions_seen = wie_util::stats::ending_transition().0;
    // 헬 몬스터 포크: first-sighting snapshots (same `FloorMemory` as the tower, its
    // own instance) and the scene they belong to — the creature array is reallocated
    // on every scene load, so a scene change drops them. `hell_said` = the scene the
    // one-line diagnostic already spoke for.
    let mut hell_mem = crate::guardian::FloorMemory::default();
    let mut hell_scene_was: Option<u16> = None;
    let mut hell_said: Option<u16> = None;
    // ★ The way BACK (2026-09-17, report: "죽어서 마을로 복귀하면 … 엉뚱한곳에서 나오고
    // … 한번 이동버튼 누르면 그제서야"). The leave warp lands the player on the LAST
    // FLOOR's tile. `Some((scene, tile))` = the origin the run remembered; the landing block
    // below stands the player on that tile — draw corner and all — once `scene` has
    // arrived. ★ It cannot share `tower_arrive`: that one lands on the arena's own
    // spawn tile, and the way back wants the tile the run started from.
    // ★ 143차: a leg without a tile is tracked too (the re-post and the wake both wait on
    // the SCENE), which is why this is no longer `(scene, tile)`.
    let mut tower_return: Option<ReturnLeg> = None;
    let mut tower_restore_polls: u32 = 0;
    // ★★★ 143차 — THE WAKE: the finished tower's way out (`guardian::WakePhase`). `Some`
    // from the top floor's hold to the line 「의식이 몽롱하다...」; the scene it is carrying
    // the player to (for its "home" check); whether the wake flag was raised there; and the
    // boxes it holds back until the line has been read (the exit reward's).
    let mut wake: Option<crate::guardian::WakePhase> = None;
    let mut wake_home: Option<u8> = None;
    let mut wake_flag_armed = false;
    let mut wake_notices: Vec<String> = Vec::new();
    // 하늘성 인게임 입장구: the last (HARD, HEARD) pair we reported, so the diagnostic
    // line below fires on a CHANGE rather than six times a second.
    let mut tower_npc_flags: (Option<u8>, Option<u8>) = (None, None);
    // 하늘성 입장구: was the confirm choice open last poll, so the cursor is only reset
    // to 예 on the RISING edge (the player can still press DOWN to reach 아니오).
    let mut entrance_choice_was: bool = false;
    // 하늘성: heal the player to full when a run ends in death, so returning to town
    // dead becomes returning alive (user 2026-09-16: "죽은채로오면안돼 HP풀피가 되어야").
    // Applied in the post-run block once the return warp has landed in a field.
    let mut tower_heal_pending: bool = false;
    // The frame by which the heal must land, so a death that ends on the title (rather
    // than back in town) does not leave the flag armed for a later, unrelated character.
    let mut tower_heal_deadline: u64 = 0;
    let mut last_tower_flash: u32 = 0;
    let mut last_sort_screen: Option<crate::bagsort::SortScreen> = None;
    // Live monsters as of the last tower poll, for the overlay's status line. Cached
    // because the HUD publishes six times more often than the tower polls, and reading
    // the creature array on the HUD's cadence would be a real cost for one number.
    let mut tower_alive: i32 = 0;
    // A skill-plate tap being verified against guest state — see `SkillWatch`.
    let mut skill_watch: Option<SkillWatch> = None;
    // When the quit-time save must be given up on (`None` = no quit in progress).
    // See the exit block in the loop below and `crate::request_exit`.
    let mut exit_deadline: Option<u64> = None;
    // Consecutive polls that saw NO script window, and consecutive polls that saw the
    // choice flag set. Both are hysteresis counters — see `CUTSCENE_EXIT_POLLS`.
    let mut cutscene_gone: u32 = CUTSCENE_EXIT_POLLS;
    let mut choice_seen: u32 = 0;
    // Is the cash purchase confirm (and therefore the quantity picker) on screen?
    let mut cash_qty_open = false;
    let mut cash_confirm_open = false;
    let mut cash_qty = CashQtyPoke::default();
    let mut wh_qty_ask: Option<(u8, u8, u8, u8)> = None;
    // In-game 오브결합 / 소켓확장 (2026-08-25). Three locals, and they are separate on
    // purpose: `orb_pending` is the cell whose 오브결합 row was just taken (latched on the
    // OK key, before the guest saw it — the popup closes on that key); `orb_source` is
    // that same cell held for as long as the picker is up; `orb_targets` are the
    // candidates behind the list the UI is showing, so the index it answers with can be
    // turned back into a cell. An orb without its list, or a list without its orb, is not
    // a state this feature has — they are cleared together.
    let mut orb_pending: Option<(u8, u8, u64)> = None;
    // 장비 커서 걷기 (79차-D): one RIGHT per worker poll until it arrives.
    // ★ 2026-09-14: an enum — the orb can also come from the 오브가방 (`bagbox_host`).
    let mut orb_source: Option<crate::bagbox_host::OrbSource> = None;
    // The bag boxes (오브가방 · 재료가방 · 소비가방): intake drain, sweep, grant, window.
    let mut bagbox = crate::bagbox_host::BagBoxHost::new();
    // ★ 167차 아바타 옷장: the AVATAR page's worker half (keys, try-on, worn byte, rewards).
    let mut avatar = crate::avatar_host::AvatarHost::new();
    let mut orb_targets: Vec<crate::orbcombine::Cell> = Vec::new();
    // ★★★ 71차-F: a deferred, bag-wide rebuild of the derived item cache after a
    // combine — `(due_ms, passes_left, target_key)`. See `items::equipment_cells` for
    // why one targeted call at commit time is not enough.
    let mut orb_refresh: Option<(u64, u8, u64, u8)> = None;
    // Overlay screen at the previous poll, so entering 장비 can be seen as an EDGE.
    let mut prev_overlay: Option<u8> = None;
    // ⛔ Consecutive polls with no menu, NOT a single reading. `items::menu_state` returns
    // `None` while the game's own action popup is open — and the popup that started this
    // is still open on the poll right after the OK key, because `handle_event` only QUEUES
    // the key. A one-shot test therefore withdrew the picker on the very frame it opened.
    // Same hysteresis, same reason, as `CUTSCENE_EXIT_POLLS` / `CHOICE_CONFIRM_POLLS`.
    let mut orb_gone: u32 = 0;
    let mut last_modal: u32 = 0;
    let mut buffs = config::read_buffs(&data_dir);
    // 천상의 영약 level-ups consumed but not yet delivered (they fire on the field).
    let mut levelups = config::read_pending_levelups(&data_dir);
    if levelups > 0 {
        log::info!("level elixir: {levelups} pending level-up(s) restored from disk");
    }
    // ★ A rollback note that survived means the app died between a cash item's effect
    //   and the save that would have paid for it — so the guest save about to be loaded
    //   is the PRE-USE one, item and all. Put the host back where that save left it:
    //   both sides then roll back together instead of the player keeping the effect
    //   AND the item. See `buffs::CashBaseline`.
    if let Some(base) = config::read_cash_baseline(&data_dir) {
        if base.buffs != buffs || base.levelups != levelups {
            log::warn!(
                "cash rollback: a save never landed after a cash-item use — reverting host state \
                 (exp_x2 {} -> {}, exp_x4 {} -> {}, relief {} -> {}, levelups {levelups} -> {})",
                buffs.exp_x2_until,
                base.buffs.exp_x2_until,
                buffs.exp_x4_until,
                base.buffs.exp_x4_until,
                buffs.relief_until,
                base.buffs.relief_until,
                base.levelups
            );
            buffs = base.buffs;
            levelups = base.levelups;
            config::write_buffs(&data_dir, &buffs);
            config::write_pending_levelups(&data_dir, levelups);
        }
        config::clear_cash_baseline(&data_dir);
    }
    {
        let now = now_ms();
        if buffs.any_live(now) {
            log::info!(
                "cash buffs live: exp x{}, relief {} levels, {} min left",
                buffs.exp_multiplier(now),
                buffs.relief_levels(now),
                buffs.minutes_left(now)
            );
        } else {
            log::info!("cash buffs: none live (the guest is left exactly as shipped)");
        }
    }

    // Wall clock for the throughput gauge below. Counted from here, not from process
    // start, so boot and asset extraction do not drag the average down.
    // Measurement switch for the host sprite blitter — see `config::read_blit_hle`.
    let blit_hle = config::read_blit_hle(&data_dir);
    wie_util::stats::set_blit_hle(blit_hle);

    // ★★ 78차-B - the reaver-drop TEST knobs (`config::read_reaver_test`). The drop is
    //    gated on eight HARD-mode bosses at Lv 65-67; a tester mid normal mode cannot
    //    reach one, and a non-boss kill logs nothing, so «the hook works» and «the hook
    //    is dead again» are indistinguishable from outside - which is how the wrong-
    //    register bug survived vc211..vc219. This lets one ordinary kill prove the whole
    //    chain. Read AFTER `install_binary_patches` on purpose: the values are consulted
    //    per kill, not at install, and the file deletes itself as it is read.
    //    ⛔ It cannot ship enabled: no file means the shipped constants are untouched.
    if let Some(t) = config::read_reaver_test() {
        wie_util::stats::set_reaver_test(t.rate_pct, t.as_species, t.force_hard);
    }
    // ★★★ 2026-09-23 (DT) — the 공격속도 / 스킬사용속도 override, same one-run lifetime.
    //    Nothing in the shipped asset carries the option yet, so this is the only way to
    //    exercise the `atk_speed_lgt` hook on a device. `None` = use the worn gear's sum.
    let atk_speed_override = config::read_attack_speed_test();
    log::warn!("host sprite blitter (HLE): {}", if blit_hle { "ON" } else { "OFF (wie_blit_hle.txt)" });

    // ★ Guest frame-timer ceiling — see `config::read_frame_timer_cap`. This is the one
    //   term §2.24 could not remove from the modal screens (the game asks for 244 ms
    //   there and 50 on the field), and it is a behaviour change, so it ships as a
    //   switch that can be flipped without reinstalling.
    let timer_cap = config::read_frame_timer_cap(&data_dir);
    wie_util::stats::set_timer_clamp_ms(timer_cap);
    log::warn!(
        "guest frame-timer cap: {}",
        if timer_cap == 0 {
            "OFF (honour the guest exactly)".to_string()
        } else {
            format!("{timer_cap} ms (wie_frame_timer_cap.txt)")
        }
    );


    let (mut gauge_at, mut gauge_instr, mut gauge_frames) = (Instant::now(), 0u64, 0u64);
    // ★ WHERE DOES THE WORKER'S WALL CLOCK GO (2026-08-09). On the low-spec target the
    // guest ran at 12.3 fps while `wie-emu` sat at 58 % of a core — half the target
    // framerate with 40 % of the thread idle, which no amount of interpreter tuning can
    // explain. These say whether the loop is WORK-bound (ticks long, budget capped) or
    // PACED (ticks short, sleeping out the rest of every 16 ms).
    let (mut gauge_tick_us, mut gauge_ticks, mut gauge_short_ticks) = (0u64, 0u64, 0u64);
    let (mut gauge_capped, mut gauge_sleep_us, mut gauge_loops) = (0u64, 0u64, 0u64);
    // Last hitch report, so a stall that repeats every frame cannot flood logcat.
    let mut hitch_at: Option<Instant> = None;
    // ★ 142차: the GAME-TIME clock the set effects run on (수호의 맹세's 5 s / 100 s). It
    // only advances while the guest ticks — backgrounding the app freezes it — and a
    // single pass contributes at most 250 ms, so a stall cannot eat a cooldown.
    let mut play_ms: u64 = 0;
    let mut play_last: Option<Instant> = None;
    // The last set mask published (u32::MAX = none yet), for the activation box.
    let mut set_mask_last: u32 = u32::MAX;
    let mut guard_triggers_last: u32 = 0;
    let mut set_hp_bonus_last: u32 = 0;
    // ★ 150차: the accessory effects as last published (for the change log), the 5-second
    // regen's next due time on the play clock (0 = disarmed), and how many heals it has done
    // (the first `REGEN_LOG_HEALS` are logged).
    let mut acc_last = wie_util::stats::AccEffects::NONE;
    // ★ 156차 방패무시: `(pct, special weapon)` as last published (for the change log).
    let mut pierce_last: (u32, bool) = (0, false);
    let mut regen_due: u64 = 0;
    let mut regen_heals: u32 = 0;
    const REGEN_LOG_HEALS: u32 = 6;

    loop {
        // 0) Backgrounded / screen locked: do not tick (so the guest generates no
        //    new audio and burns no battery) — the synth emits silence in parallel.
        //    Still drain+discard input so the channel can't back up and we notice
        //    teardown; clear held keys so nothing auto-repeats on resume.
        if paused.load(Ordering::Relaxed) {
            loop {
                match input_rx.try_recv() {
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        log::info!("input channel disconnected while paused; stopping emulator");
                        return;
                    }
                }
            }
            held.clear();
            // …and the stick's standing order with them: the input the UI sent to end it
            // was discarded by the drain above, so leaving it armed would re-press a
            // direction on resume whose keyup is never coming.
            walk = WalkOrder::default();
            play_last = None;
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        let frame_start = Instant::now();
        if let Some(t) = play_last {
            play_ms += (frame_start - t).as_millis().min(250) as u64;
        }
        play_last = Some(frame_start);
        wie_util::stats::set_play_clock_ms(play_ms);
        let in_tower = {
            let read = |a, n| emulator.read_memory(a, n);
            items::scene_now(&read).is_some_and(|scene| tower.is_floor_scene(scene))
        };
        wie_util::gameplay::set_respawn_rate(if in_tower { 1 } else { crate::game_options::respawn_rate() });
        wie_util::gameplay::set_exp_rate(crate::game_options::exp_rate().max(buffs.exp_multiplier(now_ms())));


        // 1) Redraw if the guest asked for one.
        if should_redraw.swap(false, Ordering::SeqCst) {
            emulator.handle_event(Event::Redraw);
        }

        // 1b/1c) ⛔ The dropped-connection notice (48차) and the failed-connect notice
        //     (2026-08-21) WERE HERE and are gone (99차): with the in-app server there is
        //     no connection to drop and no connect that can fail.

        // 2) Drain input from the UI thread straight to the guest. (It used to be
        //    stashed while the hidden minimap refresh held the guest's map window
        //    open; there is no such window any more.)
        loop {
            match input_rx.try_recv() {
                Ok(ev) => {
                    // ★ 161차: ONE OK press per iteration. The guest's event loop drains
                    //   everything queued inside a single tick (104차), so two OK pairs
                    //   queued behind a long tick (world → shell is 300+ ms — 148차) would
                    //   open the slot list AND load its row 0 in the SAME tick, before the
                    //   post-tick pass (4a2) could put the cursor on the last loaded slot.
                    //   The rest of the queue waits for the next iteration: that press
                    //   arrives one frame (≤ 21 ms) later and nothing else changes — a
                    //   thumb cannot press twice inside one frame, so only presses that
                    //   piled up behind a long tick are ever spread out.
                    if let InputEvent::Key { code, down } = &ev {
                        if let Some(keys) = crate::game_options::key(*code, *down, &data_dir) {
                            for key in keys {
                                emulator.handle_event(Event::Keydown(key));
                                emulator.handle_event(Event::Keyup(key));
                            }
                            let _ = item_proxy.send_event(UserEvent::GameOptionsChanged);
                            continue;
                        }
                    }
                    let ok_release = matches!(ev, InputEvent::Key { code: KeyCode::OK, down: false });
                    apply_input(&mut *emulator, &mut held, &mut picking, &mut pages, &mut qb_window, &mut buffs, &mut saving, &mut skill_watch, &mut levelups, &mut cash_qty, &mut orb_pending, &mut bagbox, &mut avatar, &mut walk, &data_dir, &item_proxy, ev);
                    if ok_release {
                        break;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::info!("input channel disconnected; stopping emulator");
                    return;
                }
            }
        }

        // 2b) Keys the bag-box use job wants delivered THROUGH the same path as the
        //     player's (the cash-item ledger lives in `apply_input`'s OK arm).
        while let Some(ev) = bagbox.take_input() {
            apply_input(&mut *emulator, &mut held, &mut picking, &mut pages, &mut qb_window, &mut buffs, &mut saving, &mut skill_watch, &mut levelups, &mut cash_qty, &mut orb_pending, &mut bagbox, &mut avatar, &mut walk, &data_dir, &item_proxy, ev);
        }

        // 2c) ★★ 104차 ②: THE BOX WINDOW'S ANSWER IS READ HERE, BEFORE THE TICK.
        //
        //     It used to be read after it (next to the orb target picker's, ~2000 lines
        //     below), and that cost a whole worker iteration on its own: a 사용하기 queues
        //     the game's `giveItem` through `call_guest_function`, which is ASYNCHRONOUS —
        //     it runs inside the next `emulator.tick()`. Answering after the tick meant
        //     this frame's tick had already gone, so the unit could not land in a bag cell
        //     until the frame after. Read here, the give runs in THIS frame's tick and the
        //     step loop below (104차 ①) carries the rest of the job to the end.
        //
        //     ⛔ The picker it may want next is NOT built here: that needs
        //     `build_target_picker`, which reads the item DB and lives with the rest of the
        //     orb code below. The chosen stack is parked in `box_pick` and picked up there
        //     — one frame later than before for the 오브가방, which nobody is waiting on
        //     (no guest call is in flight), and one frame EARLIER for 사용하기, which is
        //     what the player asked for.
        // ★ 138차: the quick-start picker's answer. Read before the box window's, which
        //   shares the channel — only one list dialog is ever up (see `tower_pick`).
        if tower_pick.is_some()
            && let Some(answer) = crate::take_orb_answer()
        {
            let offered = tower_pick.take().unwrap_or_default();
            match answer.and_then(|i| offered.get(i).copied()) {
                Some(floor) => {
                    log::warn!("guardian: quick start — floor {floor} chosen");
                    tower_cmd = Some(crate::guardian::Command::Enter { floor, force: false });
                    tower_cmd_waits = 0;
                }
                None => log::warn!("guardian: quick start cancelled — nothing happens (the NPC can be asked again)"),
            }
        }
        let mut box_pick: Option<crate::bagbox_host::OrbSource> = None;
        if bagbox.window_open()
            && let Some(answer) = crate::take_orb_answer()
        {
            box_pick = bagbox.answer(answer, &mut *emulator, &data_dir, &item_proxy);
            if box_pick.is_none() {
                log::info!("bagbox: dialog answered (closed, or a consumable verb taken)");
            }
        }

        // 3) Key auto-repeat.
        let now = Instant::now();
        for (code, last) in held.iter_mut() {
            if now.duration_since(*last).as_millis() > KEYREPEAT_MS {
                emulator.handle_event(Event::Keyrepeat(*code));
                *last = now;
            }
        }

        // 3b) …and a HELD attack key, which the repeat above cannot serve either: the
        //     guest discards KEY REPEATED, so one press is one swing. WHEN to hand it
        //     another is `atkspeed::AttackHold`: back at idle on the field (the SAME read
        //     as the walk order below), never in the iteration of the physical press, and
        //     NEVER once this hold has seen a window, a dialogue, a choice or a box — OK
        //     also talks, and an OK just after a dialogue closes opens it again (132차-B).
        //     ⛔ Read on every iteration the key is held, not only when a press is due: a
        //     UI that opens and closes in between must still spoil the hold.
        let mut attacked = false;
        match attack_key_held(&held) {
            None => attack_hold = None,
            Some(code) => {
                let now_ms = now.duration_since(hold_epoch).as_millis() as u64;
                if !matches!(attack_hold, Some((c, _, _)) if c == code) {
                    attack_hold = Some((code, crate::atkspeed::AttackHold::new(now_ms, frame), 0));
                }
                let view = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::attack_hold_view(&read)
                };
                if let Some((_, hold, sent)) = attack_hold.as_mut() {
                    let was_spoiled = hold.spoiled();
                    if hold.step(now_ms, frame, view) {
                        emulator.handle_event(Event::Keydown(code));
                        attacked = true;
                        *sent += 1;
                        if *sent == 1 {
                            log::info!("attack hold: {code:?} is still down with the field idle — re-pressing until it is released");
                        }
                    } else if !was_spoiled && hold.spoiled() && *sent > 0 {
                        log::info!("attack hold: {code:?} re-pressed {sent} time(s), then a window/box came up — silent until it is released");
                    }
                }
            }
        }

        // 3c) …and the touch stick's standing walk order, which the repeat above cannot
        //     serve: the guest throws KEY REPEATED away and walks from keydown edges only
        //     (see `WalkOrder`). One read of the guest's own player-state byte says
        //     whether the walk it was given has ended; if it has and the thumb is still
        //     holding a direction, hand it the edge again.
        //
        //     ⛔ NOT in the same frame as an attack re-press. Both gates are "standing idle
        //     on the field", so both can come true on the same frame, and a direction
        //     keydown landing on top of a fresh attack cancels the attack. The thumb on the
        //     attack key wins; the walk resumes the moment it lifts.
        if let Some(dir) = walk.dir
            && !attacked
            && walk.next.is_none_or(|t| now >= t)
        {
            let idle = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::walk_idle_on_field(&read)
            };
            // ⛔ The gate is re-read every time, never latched: the answer changes with
            // the window, the dialogue box and the character's own state machine.
            if idle {
                emulator.handle_event(Event::Keydown(dir));
            }
            walk.next = Some(now + Duration::from_millis(KEYREPEAT_MS as u64));
        }

        // 4) Adaptive multi-tick: keep running while the game stays work-bound
        //    this frame, up to a hard wall-clock cap.
        loop {
            // Sample before the tick so a hitch can be attributed to what happened INSIDE
            // it — see `TickCounters` for why the 30 s gauge cannot answer this. The
            // profile swap gives the tick its own PC samples without costing anything per
            // sample; with the profiler off it does not happen at all.
            let carry = profiling.then(profile_take);
            let before = TickCounters::now();
            let tick_start = Instant::now();
            // ★ 141차: arm the portrait capture window on the player's draw corner for
            // this tick, with the anchor where the player's own shadow blit must land
            // (`portrait.rs`). A few CHAR reads a frame; off the field it disarms and
            // the blitter's extra test is one atomic load per blit.
            {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                wie_util::stats::set_portrait_window(items::portrait_arm_window(&read, logical_w, logical_h));
            }
            if let Err(e) = emulator.tick() {
                log::error!("emulator tick error: {e:?}");
                return;
            }
            // ★★ 153차: the ring's picture is decided HERE, after every tick — never from
            // the HUD poll's sample, which landed on any of the idle loop's four breathing
            // frames and put a new picture in the ring four times a cycle (player:
            // *"인게임 플레이중에 계속 뭔가 바뀌고있어 … 장비를 바꿔서 외형이 바뀔때말고는
            // 바뀌면안돼"*). Only a tick that drew the player (the capture opened a run)
            // AND was the canonical frame offers a picture; the keeper takes one per look
            // and holds it. Off the canonical frame this is one atomic load and three
            // byte reads; on it (one tick in four while standing) the worn look too.
            {
                let (seq, _, _) = wie_util::stats::portrait_run_state();
                if seq != portrait_seq_seen {
                    portrait_seq_seen = seq;
                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                    if items::portrait_canonical(&read)
                        && let (Some(look), Some(spot)) = (items::portrait_look(&read), items::portrait_spot(&read))
                    {
                        portrait_observe_log(look, portrait_keeper.observe(look));
                        if portrait_keeper.wants(look)
                            && let Some(picture) = items::portrait_take(logical_w, logical_h)
                            && portrait_keeper.offer(look, spot, picture)
                        {
                            log::warn!(
                                "portrait: the ring took look {look:#018x} at {spot:?} — it stays until the worn look changes (final after {} spots agree)",
                                items::FINAL_SPOTS
                            );
                        }
                        if portrait_keeper.take_memory_dirty() {
                            save_portrait_memory(&portrait_memory_path, portrait_fp, &portrait_keeper);
                        }
                    }
                }
            }
            let took = tick_start.elapsed();
            gauge_tick_us += took.as_micros() as u64;
            gauge_ticks += 1;
            let hitch = took.as_millis() >= SLOW_TICK_MS && hitch_at.is_none_or(|t| t.elapsed() >= HITCH_REPORT_GAP);
            // Always restore, hitch or not — a skipped merge would quietly empty the 30 s
            // aggregate the throughput gauge prints.
            let hot = carry.map_or(String::new(), |c| profile_merge_back(c, if hitch { 8 } else { 0 }));
            if hitch {
                hitch_at = Some(Instant::now());
                report_hitch(took, &before, &TickCounters::now(), &hot);
            }
            if took.as_millis() < TICK_FULL_MS {
                gauge_short_ticks += 1;
                break; // game went idle -> done this frame
            }
            if frame_start.elapsed().as_millis() >= FRAME_BUDGET_MS {
                gauge_capped += 1;
                break; // hard cap
            }
        }

        // 4b) DEBUG (Phase 1 of the skill-HUD track): honour a guest-RAM dump
        //     request every ~0.5 s. Drop a `dump_trigger` file (lines of
        //     "addr_hex len_hex") into the app data dir to snapshot live memory —
        //     used to live-diff and locate the equipped-skill fields.
        frame = frame.wrapping_add(1);

        // 4a) ★★★ 148차 — A CHARACTER WAS LOADED IN THAT TICK (S25 vc420, 2026-10-01).
        //
        //     Player: *"세이브 로드를 슬롯 변경하면서 빠르게 진행하면 기존에 등록되어있는
        //     퀵슬롯이 날라가는 이슈"*. OK mashed through 시스템 → 메인메뉴 → 게임시작 →
        //     SLOT 1 took the game from SLOT 2's world to SLOT 1's in two ticks, and every
        //     "the shell was reached" in this loop is a SAMPLE — the 5-frame poll never saw
        //     app state 3. So the plate row never re-synced (its capture filed SLOT 1's bar
        //     onto SLOT 2's plates), the pin latch took the outgoing world's cursor for the
        //     new character (`live character is slot 1 (was 0)` — a save there would have
        //     been redirected onto SLOT 2), and the bag boxes followed that pin.
        //
        //     The record store sees every load synchronously (`vault::note_slot_loaded`:
        //     the `GameSave` read and the online `04 3d`), so the load is taken HERE, before
        //     any poll below can act on the old world's picture of things:
        //       * the plate row stops accepting live-state writes until its sync
        //         (`Pages::desync`), and the sync is forced by the scan (`!synced`);
        //       * the pin latch's next poll takes the loaded slot as its evidence
        //         (`savevault::pin_evidence_cursor`);
        //       * the tower's per-world bookkeeping hears "the shell was reached" for
        //         `WORLD_SWAP_FRAMES`, and a landing still in flight is abandoned now — it
        //         would place the NEW character on the old run's tile.
        let load_gen = crate::vault::load_generation();
        if load_gen != seen_load_gen {
            seen_load_gen = load_gen;
            loaded_since_poll = true;
            world_swap_until = frame + WORLD_SWAP_FRAMES;
            pages.desync(&mut qb_window);
            if tower_return.is_some() || tower_arrive.is_some() || tower_land_frames > 0 {
                log::warn!(
                    "guardian: a save was loaded — the landing in flight is abandoned (return {tower_return:?}, arrive {tower_arrive:?})"
                );
                tower_return = None;
                tower_arrive = None;
                tower_land_frames = 0;
            }
            // An ENTER still waiting for the field (a window was up when it was asked) was
            // asked by the character who just left — the next one to reach a field would be
            // carried into the tower in their place.
            if let Some(cmd @ crate::guardian::Command::Enter { .. }) = tower_cmd {
                log::warn!("guardian: a save was loaded — the pending {cmd:?} belonged to the unloaded character and is dropped");
                tower_cmd = None;
                tower_cmd_waits = 0;
            }
            // ★ 150차: the TIMED max-HP buffs and the hooks' tracked HP belonged to the character
            // who just left. They are forgotten in `vault::note_slot_loaded` itself
            // (`stats::forget_character`) — that runs BEFORE the new world's init, this edge a
            // tick AFTER it. What is left for here: one clamped `ComputeMaxHP` re-run for
            // whoever is in the world now (`set_hp_bonus_last` = "unknown" — the old
            // character's last value would let a bonus the init computed with stand), the set
            // mask re-read as news-free (no completion box for a character loaded already
            // wearing one), and the 5-second regen re-armed from the first field poll.
            set_hp_bonus_last = u32::MAX;
            set_mask_last = u32::MAX;
            regen_due = 0;
            // ★ 153차: the ring's picture may be the character who just left — hidden until
            // the look on the field is known (kept if it is the same look).
            portrait_keeper.suspect();
            log::info!("world: a character record was loaded (load #{load_gen}, pin {:?}) — the plate row waits for its sync", crate::vault::live_slot());
        }

        // 4a2) ★ 161차 (2026-10-03) — THE SLOT LISTS OPEN ON THE LAST LOADED SLOT.
        //
        //      Player: *"5번째 세이브슬롯에서 게임을 하다가 저장하고 메인메뉴 나가고 바로
        //      게임시작하는 등 OK 를 연타해서 들어가도 5번째 세이브슬롯에서 시작하기를 … 지금은
        //      무조건 첫번째 세이브 슬롯부터 시작해"*. The game's own 게임시작 / 온라인 picks
        //      zero the list's row as they set the screen word (`0xa876` / `0xa88a`, one store
        //      sequence), and the host's window (130차) remembers WHERE the list was scrolled,
        //      not which row. So on the frame a slot list first shows (the screen word's
        //      rising edge — a modal closing over a list already up is not an opening) the
        //      cursor goes onto the slot loaded last (`crate::slot_focus`, fed by every pin
        //      source), sliding the window when that slot is out of view
        //      (`slotwindow::focus_target`). Every frame, not the 5-frame poll: the very next
        //      OK may be the one that loads, and the input drain above holds OK to one per
        //      iteration so this pass always runs in between.
        //      ⛔ This writes the cursor WORD the player could have set with DOWN — nothing
        //      in real-slot space moves, the pin is untouched, and the gate is the window's
        //      own (`slot_window_ctx`: a slot list, no modal, the build has the anchors).
        {
            let on_list = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::shell_state(&read) && crate::savevault::on_slot_list(items::shell_screen(&read))
            };
            if on_list && !slot_list_was_up {
                slot_focus_pending = true;
            } else if !on_list {
                slot_focus_pending = false;
            }
            slot_list_was_up = on_list;
            if slot_focus_pending {
                match crate::slot_focus() {
                    None => slot_focus_pending = false,
                    Some(target) => {
                        if let Some(ctx) = slot_window_ctx(&*emulator) {
                            slot_focus_pending = false;
                            slot_list_focus(&mut *emulator, &data_dir, &item_proxy, &ctx, target);
                        }
                    }
                }
            }
            // …and the slot is remembered across restarts the moment it changes.
            if let Some(f) = crate::slot_focus()
                && slot_focus_written != Some(f)
            {
                slot_focus_written = Some(f);
                crate::config::write_slot_last(&data_dir, f);
            }
        }
        let world_swapped = frame < world_swap_until;

        if frame % 30 == 0 {
            debug_ram_dump(&*emulator, &data_dir);
        }

        // 4b2) STACK HIGH-WATER, ~every 30 s. See `stack_rss_kb`: this is the gauge that
        //      turns the intermittent 21-35 minute crash into a number that can be read
        //      off a single play session. A healthy run plateaus within the first minute;
        //      a rising staircase is the leak, and its slope says how long a player has.
        if frame % 1800 == 0
            && let Some(kb) = stack_rss_kb()
            && kb > stack_rss_peak
        {
            stack_rss_peak = kb;
            log::warn!("emu stack high-water {kb} KB of {} KB", EMU_STACK / 1024);
        }

        // 4b3) THROUGHPUT, ~every 30 s. The one number that says whether the emulator can
        //      keep up on THIS device: the game needs about 123 M guest instructions per
        //      second of real time to run at its intended 25 fps, so `M inst/s` below 123
        //      is exactly how much the player is missing. `blit` is the share of sprite
        //      blits the host draws instead of interpreting (2026-08-06: 99.99% headless,
        //      and 36% of all guest instructions); a collapse there means the binary
        //      patch did not install and the device is back to interpreting everything.
        if frame % 1800 == 900 {
            let (instr, svc, _) = wie_util::stats::snapshot();
            let frames = wie_util::stats::frames();
            let ([calls, hle, _px, _px_hle], _, _, refused) = wie_util::stats::blit_snapshot();
            // ★ INTERVAL, not cumulative. A running average is unreadable here: it mixes the
            // title screen (where the loop sleeps out most of every 16 ms because the guest
            // has nothing to do) with real field play, so a low number could mean either
            // "too slow" or "nothing to draw". The delta since the last report is the rate
            // the player is actually getting right now.
            let now = Instant::now();
            let secs = now.duration_since(gauge_at).as_secs_f64().max(1e-9);
            // `tint` is the critical-hit colour transform (49차). Separate from `blit`
            // because it draws no sprite — see `wie_util::stats::TINT_CALLS`.
            let (tint_calls, tint_hle, tint_px) = wie_util::stats::tint_snapshot();
            log::warn!(
                "emu throughput {:.1} M inst/s (need ~123 for 25 fps), guest {:.1} fps, svc {svc}, blit {hle}/{calls}, refused {refused:?}, tint {tint_hle}/{tint_calls} ({tint_px} px), guest mem {} MB",
                (instr - gauge_instr) as f64 / secs / 1e6,
                (frames - gauge_frames) as f64 / secs,
                // Committed guest pages. Pinned at 256 MB+ before the lazy-map change, which is
                // what made this process the low-memory killer's first choice; if it ever climbs
                // back toward 256 the saving has been undone somewhere.
                wie_util::stats::guest_page_bytes() / (1024 * 1024),
            );
            // ★ The guest SOUND path, which runs on THIS thread. Every `MC_mdaPlay` used to
            // re-parse the whole SMAF file (chunk walk + event Vec + sort) inside the frame
            // budget, and every `MC_mdaClipPutData` minted a handle nothing ever removed.
            // Reading this line: `parses` should settle at the number of DISTINCT sound
            // files the session has touched (Hero3 ships 33) while `plays` climbs freely.
            // **`parses` tracking `plays` means the cache is not being hit** and the crit
            // hitch is back. `handles` climbing without bound means the clip lifecycle
            // fix stopped working. `catchup` counts songs re-based after the emulator was
            // paused (backgrounded) — a few per resume is expected, a stream is not.
            let pct = |us: u64| (us as f64 / (secs.max(1e-9) * 1e4)).min(999.9);
            log::warn!(
                "emu loop: {} frames/s, tick {:.2} ms avg ({} ticks, {} short, {} budget-capped), in-tick {:.0}% of wall, sleeping {:.0}%",
                (gauge_loops as f64 / secs) as u64,
                gauge_tick_us as f64 / gauge_ticks.max(1) as f64 / 1000.0,
                gauge_ticks,
                gauge_short_ticks,
                gauge_capped,
                pct(gauge_tick_us),
                pct(gauge_sleep_us),
            );
            (gauge_tick_us, gauge_ticks, gauge_short_ticks) = (0, 0, 0);
            (gauge_capped, gauge_sleep_us, gauge_loops) = (0, 0, 0);
            // ★ 57차: how many guest paints actually reached the UI thread vs were
            // dropped as bit-identical (platform.rs frame hash). On a static screen
            // (title, open menu) `skipped` should carry nearly the whole guest
            // framerate — `skipped 0` there means the gate has stopped working and
            // the UI is back to one full blit+present per identical frame.
            let (paint_sent, paint_skipped) = crate::platform::paint_stats_take();
            log::warn!(
                "emu paint: sent {}/s, skipped {}/s identical",
                (paint_sent as f64 / secs) as u64,
                (paint_skipped as f64 / secs) as u64,
            );
            // ★ WHICH SVCs, and what the recv throttle absorbed (§2.24). `svc` above is a
            // count with no attribution: on LGT the C stdlib is an SVC too, so a stall can
            // be `memcpy` (a Rust helper, ~nothing) or `MC_netSocketRecv` (a socket
            // syscall). These two lines are how the warehouse frame drop was pinned down,
            // and they are what says whether it has come back: `recv polls` in the tens of
            // thousands per second with `absorbed` near 100 % is healthy; `absorbed 0 %` at
            // that rate means the throttle is gone and the game is back to 5 fps.
            let (cats, hot) = wie_util::stats::svc_mix_take(8);
            let hot = hot
                .iter()
                .map(|(id, n)| format!("{id:#x}={}", (*n as f64 / secs) as u64))
                .collect::<Vec<_>>()
                .join(" ");
            log::warn!(
                "emu svc/s by category [init {} wipic {} stdlib {} hook {}] hot ids/s: {hot}",
                (cats[1] as f64 / secs) as u64,
                (cats[3] as f64 / secs) as u64,
                (cats[5] as f64 / secs) as u64,
                (cats[0] as f64 / secs) as u64,
            );
            let (sets, tmin, tmax, tmean, naps, nap_ms) = wie_util::stats::frame_timing_take();
            let (clamped, cap) = wie_util::stats::timer_clamp_take();
            let (unsets, stale_drops) = wie_util::stats::timer_cancel_take();
            if sets > 0 || naps > 0 {
                // `min/mean/max` are what the GUEST asked for; `capped` is how much of
                // that we overrode. In a modal screen with the cap on, expect max 244 and
                // capped ≈ 100 % — that pair is what says the clamp is the thing moving
                // the frame rate, and not some other change.
                // `unset`/`stale` are the timer-cancellation health pair (§41 item 7):
                // across a network round trip `guest set N/s` must come back to its
                // field value, and `stale` > 0 is the direct proof that a replaced or
                // cancelled shot was actually discarded instead of firing.
                log::warn!(
                    "emu frame timing: guest set {}/s timers (min {tmin} ms, mean {tmean} ms, max {tmax} ms); \
                     capped {}% to {cap} ms; unset {}/s, stale dropped {}/s; pump napped {}/s for {:.1} ms avg",
                    (sets as f64 / secs) as u64,
                    if sets == 0 { 0 } else { clamped * 100 / sets },
                    (unsets as f64 / secs) as u64,
                    (stale_drops as f64 / secs) as u64,
                    (naps as f64 / secs) as u64,
                    nap_ms as f64 / naps.max(1) as f64,
                );
            }
            let (polls, absorbed) = crate::network::recv_stats_take();
            if polls > 0 {
                log::warn!(
                    "emu net: recv polls {}/s, absorbed {:.0}% by the 2 ms gate (in-app server)",
                    (polls as f64 / secs) as u64,
                    absorbed as f64 * 100.0 / polls as f64,
                );
            }
            let hot = profile_top(12);
            if !hot.is_empty() {
                log::warn!("emu profile{hot}");
            }
            let a = wie_util::stats::audio_snapshot();
            log::warn!(
                "emu sound loads {} plays {} parses {} ({} KB) cache {} handles {} catchup {}",
                a[0],
                a[1],
                a[2],
                a[3] / 1024,
                a[4],
                a[5],
                a[6],
            );
            (gauge_at, gauge_instr, gauge_frames) = (now, instr, frames);
        }

        // 4c) HUD channel. The field/menu flag decides the ENTIRE overlay control
        //     set (field: skill plates + gauges + minimap; game window: menu
        //     navigation keys), so it is polled ~12x/s — the cheap read — while the
        //     pointer-chasing quickbar scan stays at ~4x/s. Gauges keep their LAST
        //     GOOD value on an implausible read instead of collapsing to zero
        //     (which is what drew HP/MP bars as empty whenever a window was open).
        // 4c0) AUTO-LOOT forced ON (player request 2026-07-30: "auto-loot should be
        //      on from character creation"). The game only ever enables it through
        //      the 오토루팅 cash item and clears it in CHAR_init / the new-game reset,
        //      and it is not serialised — so the host re-asserts it. One byte read,
        //      and a write only when it is actually off, so this is free and needs no
        //      "is this character creation?" logic. See items::autoloot_to_force.
        //      ★ 2026-08-13: the offset is per BUILD now, and on LGT it is unknown, so
        //      this does nothing there. It used to do nothing there SILENTLY — gated
        //      behind the KTF-only guest_code_addresses_valid() — which is how the
        //      shipped app came to have neither the forcing nor the cash item that does
        //      the same job. Say so once at boot so the next regression is visible.
        if frame % 30 == 0 {
            if !autoloot_notice_done {
                autoloot_notice_done = true;
                if items::autoloot_forcible() {
                    log::info!("auto-loot: host setting is {}", crate::game_options::auto_loot());
                } else {
                    log::warn!(
                        "auto-loot: NOT forcible on this build (offset unlocated) — the \
                         player must buy the game's own 오토루팅 item; see \
                         guest_anchors::autoloot_off"
                    );
                }
            }
            let want = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::autoloot_to_force(&read)
            };
            if let Some((a, v)) = want {
                // The VALUE comes from the table too: KTF's arm writes 1, LGT's writes
                // 0x81. Writing the wrong constant is half of why the first LGT attempt
                // did nothing even once it had an address.
                let ok = emulator.write_memory(a, &[v]);
                log::info!("auto-loot: applying {} at {a:#x} <- {v:#04x} (write ok = {ok})", crate::game_options::auto_loot());
            }

            // ★ 2026-08-13: let 영웅-이상 (gold/white CASH) equipment into the network
            //   warehouse. The shipped client refuses deposit for grade > 3 with
            //   "아이템 등급이 영웅 이상인 아이템은 올릴 수 없습니다"; this one-halfword
            //   patch (cmp #3 -> cmp #7) routes every grade to the same 05 41 upload.
            //   Written once — the site then reads the patched value, so this is a
            //   no-op on later polls. See items::warehouse_grade_patch.
            let wh_patch = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::warehouse_grade_patch(&read)
            };
            if let Some((a, bytes)) = wh_patch {
                let ok = emulator.write_memory(a, &bytes);
                log::info!("warehouse deposit: un-gating 영웅+ equipment at {a:#x} (write ok = {ok})");
            }

            // ★ 천상의 영약: deliver pending level-ups, one per poll, only on the
            //   field — the exact context the game's own kill path runs checkLevelUp
            //   in. The mechanism is the game's own: exp = expToNext + 1, then its
            //   checkLevelUp (LGT 0x4d715, guest_anchors::level_up_fn), which grants
            //   points, refills HP/SP, honours the 70 cap and plays the level-up
            //   effect itself. At the cap the pending grant is burned with a toast
            //   rather than armed forever.
            if levelups > 0
                && let Some(f) = crate::guest_build::level_up_fn()
            {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                if items::in_field_state(&read) {
                    match items::level_elixir_writes(&read) {
                        Some((addr, bytes, level)) => {
                            let ok = emulator.write_memory(addr, &bytes);
                            emulator.call_guest_function(f, &[]);
                            levelups -= 1;
                            config::write_pending_levelups(&data_dir, levelups);
                            let _ = item_proxy.send_event(UserEvent::Notice("천상의 영약: 레벨 UP!"));
                            log::info!(
                                "level elixir fired: level {level} -> checkLevelUp({f:#x}), exp write ok = {ok}, {levelups} pending left"
                            );
                        }
                        // resolve_char refuses a non-live record, and at the level cap
                        // (`LEVEL_CAP`, 99 since 111차) the writes helper declines; tell
                        // the two apart for the player.
                        None => {
                            if items::read_stats(&read).is_some_and(|s| s.level >= items::LEVEL_CAP) {
                                levelups = 0;
                                config::write_pending_levelups(&data_dir, 0);
                                let _ = item_proxy.send_event(UserEvent::Notice("이미 최대 레벨입니다"));
                                log::info!("level elixir: at the level cap — pending grant(s) cleared");
                            }
                        }
                    }
                }
            }

            // Push the game's own TOP HUD band (buff/status list + target-monster
            // box) below our gauges, by writing the y-origin global it adds to every
            // y up there. Same deal as auto-loot: read first, write only on a
            // mismatch, re-assert forever (the value has no other writers, so this
            // cannot fight the game). The UI thread publishes the amount.
            let packed = hud_y.load(Ordering::Relaxed);
            let want = packed & 0xff;
            let band_x = ((packed >> 8) & 0xff) as u8;
            let band_h = ((packed >> 16) & 0xff) as u16;
            let box_dx = (packed >> 24) & 0xff;
            let h_poke = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::band_height_poke(&read, band_h)
            };
            if let Some((a, bytes)) = h_poke {
                let ok = emulator.write_memory(a, &bytes);
                log::info!("buff band height: {a:#x} <- {} (write ok = {ok})", u16::from_le_bytes(bytes));
            }
            let band_patches = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::band_x_patches(&read, band_x)
            };
            for (a, b) in band_patches {
                let ok = emulator.write_memory(a, &[b]);
                log::info!("buff band x: {a:#x} <- {b} (write ok = {ok})");
            }
            // Slide the game's own target-monster box LEFT, out of the top-right
            // corner our minimap now owns (player request 2026-07-30: minimap at the
            // very top-right, the monster box beside it on the left). Six one-byte
            // immediates, all or nothing — see items::target_box_x_patches.
            let box_patches = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::target_box_x_patches(&read, box_dx)
            };
            for (a, bytes) in box_patches {
                let ok = emulator.write_memory(a, &bytes);
                log::info!(
                    "target box x: {a:#x} <- {:#06x} (shift {box_dx}, write ok = {ok})",
                    u16::from_le_bytes(bytes)
                );
            }
            let y_addr = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::hud_y_origin_to_set(&read, want)
            };
            if let Some(a) = y_addr {
                let ok = emulator.write_memory(a, &want.to_le_bytes());
                log::info!("hud y-origin: {a:#x} <- {want} (write ok = {ok})");
            }

            // Stop the game drawing its own BOTTOM field HUD (panel + HP/SP/EXP bars
            // + quickbar cells), which our overlay replaces. One Thumb branch, and
            // the world that was already drawn underneath shows through — masking it
            // black was the wrong answer. See items::hud_bottom_patch.
            let patch = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::hud_bottom_patch(&read)
            };
            if let Some((a, bytes)) = patch {
                let ok = emulator.write_memory(a, &bytes);
                log::info!("game bottom HUD: skipping its draw at {a:#x} (write ok = {ok})");
            }

            // Host-owned cash-shop buffs. Re-asserted the same way as auto-loot —
            // read, compare, write only on a mismatch — which makes it idempotent
            // across New Game / Continue / death / map change with no ordering logic,
            // AND makes expiry self-cleaning: once the week is up the helpers ask for
            // the ORIGINAL halfword and put the shipped instruction back.
            //
            // ★ While no buff is live both helpers find what they want already in
            // place and return `None`, so this block performs ZERO guest writes for a
            // player who has never used one of the three items.
            let now = now_ms();
            let relief = buffs.relief_levels(now);
            let mult = buffs.exp_multiplier(now);
            let relief_patch = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::equip_relief_patch(&read, relief)
            };
            if let Some((a, bytes)) = relief_patch {
                let ok = emulator.write_memory(a, &bytes);
                log::info!("equip level relief: {a:#x} <- -{relief} levels (write ok = {ok})");
            }
            let exp_patch = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::exp_multiplier_patch(&read, if crate::guest_build::exp_divisor_site() == Some(0x4d3cc) { 1 } else { mult })
            };
            if let Some((a, bytes)) = exp_patch {
                let ok = emulator.write_memory(a, &bytes);
                log::info!("exp multiplier: {a:#x} <- x{mult} (write ok = {ok})");
            }
        }

        // 4d) Host-driven save. Polled EVERY frame while a job is in flight: each step
        //     is a queued guest call that lands on the next `System::tick`, and holding
        //     a serialised heap buffer for longer than necessary is the one thing that
        //     could go wrong here. Jobs are started by the SAVE tap (`InputEvent::
        //     QuickSave`) and the cash-item retry below — the gate inside `save_begin`
        //     is what decides whether one may actually run, and it is the game's own
        //     "may I save here" byte.
        //
        // ⛔⛔ THE 5-MINUTE AUTOSAVE ARM WAS HERE AND IS GONE (2026-09-16, 99차, user
        //     decision after the save-slot audit). It was the only writer of a character
        //     record that fired without the player doing anything — at arbitrary moments
        //     (the first poll after a long background, the first poll after entering the
        //     world when the timer was overdue) and, being silent, with no way for the
        //     player to notice when its writes were being refused for a whole session.
        //     Do not add a timer-driven save back; `save::Reason::Auto` no longer exists.
        save_poll(&mut *emulator, &mut saving, &data_dir, &item_proxy);
        if frame % 30 == 0 && saving.job.is_none() {
            // The 72차-D slot-summary repair. It has its own 20 s clock inside; this
            // block only keeps it off the per-frame path. It rebuilds the 32-byte row
            // record and never serialises — see `summary_repair_poll`.
            summary_repair_poll(&mut *emulator, &mut saving, &data_dir, now_ms());
        }
        // 4c85) ★★★ 72차 — the «오브 0개» fix. UNCONDITIONAL, unlike the block below:
        //       the bug is in the game's give-item routine and has nothing to do with
        //       the network mode. Idempotent (it compares before writing) and on the
        //       same lazy cadence as the offline patches, so it also heals a guest that
        //       re-loaded its image.
        if frame % 120 == 0 {
            give_item_count_patch(&mut *emulator);
            // …and repair the zeros an earlier build already wrote into the save. Bounded
            // by the bag (5 x 16 cells) and almost always empty, so it costs one bag walk
            // every two seconds.
            let repairs = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::zero_count_repairs(&read)
            };
            if !repairs.is_empty() {
                let n = repairs.len();
                for addr in repairs {
                    emulator.write_memory(addr, &1u16.to_le_bytes());
                }
                log::warn!("orb count: repaired {n} bag cell(s) that were occupied with a stack count of 0");
            }
        }

        // ★ Reaver drops are a GUEST HOOK now (`wie_core_arm::binary_patches::reaverdrop`,
        // `binary_patches.toml` kind `reaver_drop_lgt`). The vc204 table write that lived
        // here never reached a live drop record — see that module's header.

        // 4c86) Maintenance grant (`config::GRANT_TRIGGER`): the game's own `0x52b14`,
        //       one call per poll, only while the requested slot IS the live character
        //       and the player is in the field. When every row is present the host
        //       saves once and deletes the file. `vault::live_slot` is the save pin, so a
        //       grant for slot 1 can never land in slot 0's bag.
        if let Some(g) = &mut grant
            && crate::vault::live_slot() == Some(g.slot)
            && let Some(f) = crate::guest_build::give_item_fn()
        {
            // ★ 141차: a row that has landed is CONSUMED — struck from the file and from
            // this list — so the file is always "what is still owed". Without this a
            // grant larger than the bag would hand a row out again the moment the
            // player sold or stored it to make room (the step below only asks "is it
            // in the bag?").
            if let Some((c, i, _)) = grant_wait {
                let landed = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::bag_has(&read, c, i)
                };
                if landed {
                    g.items.retain(|&r| r != (c, i));
                    crate::config::drop_grant_row(c, i);
                    grant_wait = None;
                    log::warn!("grant: cat {c:#x} idx {i:#x} landed in slot {} — {} row(s) still owed in {}", g.slot, g.items.len(), crate::config::GRANT_TRIGGER);
                }
            }
            let step = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::in_field_state(&read).then(|| items::grant_step(&read, &g.items))
            };
            match step {
                Some(items::GrantStep::Give(cat, idx)) => {
                    let inflight = grant_wait.is_some_and(|(c, i, fr)| c == cat && i == idx && frame.saturating_sub(fr) < 90);
                    if !inflight {
                        emulator.call_guest_function(f, &[u32::from(cat), u32::from(idx), u32::MAX, 0]);
                        grant_wait = Some((cat, idx, frame));
                        grant_done += 1;
                        log::warn!("grant: 0x52b14(cat {cat:#x}, idx {idx:#x}) into slot {} ({grant_done} this session)", g.slot);
                    }
                }
                // ★ 141차: the bag is full with rows still missing. The file STAYS (it is
                // the only record of what is owed) and the rest comes in as the player
                // makes room — said once per 30 s so the log tells the two apart.
                Some(items::GrantStep::Wait) if frame % 1800 == 0 => {
                    log::warn!("grant: the bag has no free cell — {} row(s) queued for slot {} wait for room ({grant_done} given this session)", g.items.len(), g.slot);
                }
                Some(items::GrantStep::Done) if grant_done > 0 && !grant_saved => {
                    if save_begin(&mut *emulator, &mut saving, save::Reason::Item) {
                        grant_saved = true;
                        crate::config::clear_grant();
                        let n = g.items.len();
                        let _ = item_proxy.send_event(UserEvent::Notice("요청한 아이템을 가방에 넣었습니다"));
                        log::warn!("grant: all {n} row(s) present in slot {} — save armed, {} deleted", g.slot, crate::config::GRANT_TRIGGER);
                        grant = None;
                    }
                }
                _ => {}
            }
        }

        // 4c9) Offline: dismiss the login notice for the player, and say "네트워크
        //      창고에 접속하시겠습니까?" instead of quoting call charges.
        // ★ 99차: unconditional — the game is 100 % offline, there is no other mode.
        if crate::network::offline_notice_due() {
            log::info!("offline: dismissing the login notice");
            emulator.handle_event(Event::Keydown(KeyCode::OK));
            emulator.handle_event(Event::Keyup(KeyCode::OK));
        }
        if frame % 120 == 0 {
            offline_guest_patches(&mut *emulator);
            slot_window_labels(&mut *emulator);
        }

        // 4c95) Cash-shop purchase quantity (60차, reworked 2026-08-21). This block no
        //       longer WRITES anything: the count goes in on the commit key
        //       (`cash_qty_commit`, called from `apply_input`) because the drawn corner
        //       number and the granted count are the same byte — writing it while the
        //       box was open made the whole page read 99, which is what the player
        //       reported. All that is left here is (a) telling the UI when to offer the
        //       picker and (b) putting the row back once the client has consumed it.
        if frame % 3 == 0 {
            let (open, confirm, modal) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                // ★ Log every modal id we see, once per change. The cash confirm is
                //   `showModal(text, 0xd, …)` in the disassembly; this is how that is
                //   confirmed on a device (and how the next unknown box is identified)
                //   without a debug build.
                let id = items::modal_id(&read).unwrap_or(0);
                if id != last_modal {
                    last_modal = id;
                    if id != 0 {
                        // ★ The cursor row is logged, not just the verdict. The whole
                        //   quantity feature now hinges on resolving WHICH row the
                        //   player is buying (`items::cash_cursor_item`), and that
                        //   expression is static-only — a device that answers `none`
                        //   here is telling us the anchor is wrong, which would
                        //   otherwise show up as a picker that never appears.
                        let cur = match items::cash_cursor_item(&read) {
                            Some((row, cat, idx)) => format!("row {row:#x} cat {cat:#04x} idx {idx:#04x}"),
                            None => "none".to_string(),
                        };
                        log::info!("modal {id} opened (cash cursor = {cur}, qty offerable = {})", items::cash_qty_offerable(&read));
                    }
                }
                let confirm = items::cash_buy_confirm_open(&read);
                (confirm && items::cash_qty_offerable(&read), confirm, id)
            };
            // ★ Put the count back. Two triggers, and the pair is deliberate:
            //   * modal 7 = the 획득/안내 box the grant itself opens (`0x13eea`), so by
            //     then `0x13e9e` has already copied the row — the fast, normal path.
            //   * a frame budget, for every other ending: 아니오, a refused purchase
            //     (status 0 opens an error box, not modal 7), the player backing out,
            //     the app being paused mid-confirm. Without it a missed restore would
            //     leave 99 in the cache and the NEXT purchase on that page would grant
            //     99 unasked — far worse than the cosmetic bug this whole change is about.
            // Never while the confirm box is still up: the client has not read the row yet.
            if !cash_qty.restore.is_empty() && !open {
                cash_qty.grace = cash_qty.grace.saturating_sub(1);
                if modal == items::MODAL_GRANT_NOTICE || cash_qty.grace == 0 {
                    for (addr, old) in cash_qty.restore.drain(..) {
                        emulator.write_memory(addr, &[old]);
                    }
                    log::info!("cash qty: row count restored (modal {modal})");
                }
            }
            if open != cash_qty_open {
                cash_qty_open = open;
                let _ = item_proxy.send_event(UserEvent::CashQty(open));
            }
            // ★ The player's bag, for the offline engine's capacity checks. Only this
            //   thread may read guest memory, and `network.rs` has no emulator handle —
            //   see `network::set_bag_view` and `offline::Env::bag`.
            {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                crate::network::set_bag_view(items::bag_view(&read));
            }
            // ★ The in-app warehouse's own picker (2026-08-25). It is raised inside
            // `network.rs`, on the emulator thread, which has no proxy — so the worker
            // carries it across, the same way it does for the cash one.
            let wh_ask = crate::network::wh_qty_pending().map(|a| (a.max, a.min, a.cat, a.idx));
            if wh_ask != wh_qty_ask {
                wh_qty_ask = wh_ask;
                let _ = item_proxy.send_event(UserEvent::WhQty(wh_ask));
            }
            // ★ Reset on the CONFIRM BOX closing, not on the picker being withdrawn.
            //   Those are different events since the picker became per-item: the
            //   offer test reads the grid cursor, so it can answer "no" while the box
            //   is still up, and resetting on that put the live count back to 1 before
            //   the engine had read it — which is exactly the reported
            //   "소켓확장은 수량을 여러개 사도 1개만 들어와". Every purchase still
            //   starts at one: a count that survived the box would spend the player's
            //   next buy without them choosing it.
            if confirm != cash_confirm_open {
                cash_confirm_open = confirm;
                if !confirm {
                    crate::set_cash_qty(1);
                }
            }
        }

        // 4d1) The guest asked to quit (`MC_knlExit` -> `crate::request_exit`).
        //
        // ⛔ No save is STARTED here any more (2026-09-16, 99차 — it was the second
        // `Reason::Auto` arm). It could not have fired anyway: 게임종료 is a title-menu
        // item (app state 3) and `save_gate` requires the field. What remains is the
        // courtesy of letting a save that is ALREADY in flight (a SAVE tap or a cash-item
        // save mid-chain) finish before the process goes away.
        //
        // The wait is BOUNDED. A save is four queued guest calls; if any of them never
        // lands (a wedged guest is exactly the state a player quits from) the app must
        // still close, or the fix for "종료가 안 된다" would have reintroduced it.
        if exit_deadline.is_none() && crate::take_exit_request() {
            let now = now_ms();
            log::warn!("exit: quitting (a save {})", if saving.job.is_some() { "is in flight — waiting for it" } else { "is not in flight" });
            exit_deadline = Some(now + EXIT_SAVE_GRACE_MS);
        }
        if let Some(deadline) = exit_deadline {
            let now = now_ms();
            if saving.job.is_none() || now >= deadline {
                if saving.job.is_some() {
                    log::warn!("exit: final save did not finish within {EXIT_SAVE_GRACE_MS} ms; quitting anyway");
                }
                let _ = item_proxy.send_event(UserEvent::ExitApp);
                return;
            }
        }

        // 4d2) Queued save retry — a manual tap that hit a busy/transient refusal, or a
        //      cash-item use waiting for the inventory to close. See `SaveState::retry`.
        if frame % 10 == 0
            && saving.job.is_none()
            && let Some((reason, until)) = saving.retry
        {
            let now = now_ms();
            if save_begin(&mut *emulator, &mut saving, reason) {
                saving.retry = None;
            } else if now > until {
                saving.retry = None;
                log::info!("save: queued {reason:?} retry expired unfulfilled");
                if reason == save::Reason::Manual {
                    let _ = item_proxy.send_event(UserEvent::SideNote("저장불가".into()));
                }
            }
        }

        // 4d3) Skill-cast watch: dozens of cheap reads only while a tap is pending —
        //      see `SkillWatch` for what confirms and why only a confirmed cast may
        //      start a cooldown.
        if let Some(w) = skill_watch.as_ref() {
            let (sp_now, active, reduce_pct) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                (
                    items::read_sp(&read),
                    items::skill_cast_active(&read),
                    items::read_cd_reduce_pct(&read).unwrap_or(0),
                )
            };
            let spent = matches!((w.sp0, sp_now), (Some(before), Some(after)) if after < before);
            let active = active && !w.active0; // rising edge only — see `SkillWatch::active0`
            if spent || active {
                let _ = item_proxy.send_event(UserEvent::SkillCast { slot: w.slot, reduce_pct });
                log::info!(
                    "skill watch: slot {} cast confirmed (sp spent={spent}, state={active}, cd −{reduce_pct}%)",
                    w.slot
                );
                skill_watch = None;
            } else if Instant::now() >= w.deadline {
                // The game's own gates swallowed it (not idle / no SP / out of range):
                // no cast, so no cooldown — exactly the reported bug's fix.
                log::info!("skill watch: slot {} tap never became a cast — no cooldown", w.slot);
                skill_watch = None;
            }
        }

        // ── 하늘성: the control file ─────────────────────────────────────────
        // Polled slowly on purpose: it is an `adb` lever, so five seconds of latency
        // costs nothing and a hot path would spend a failed `open` four times a second.
        // ⛔ 114차: not on vanilla — besides `floor N`, that file carries the `hard 1`
        //    testing knob, which would flip the HARD mirror and change the entrance NPCs'
        //    dialogue on a flavour that must keep the shipped one (`flavor.rs`).
        if crate::flavor::TOWER && frame % 300 == 45 {
            if let Some(cmd) = crate::config::read_guardian_command() {
                tower_cmd = Some(cmd);
            }
        }

        // ── 하늘성: the in-game entrance (the passer-by's own dialogue) ──────
        //
        // Two bytes of the guest's PERSISTENT FLAG ARRAY do all of it
        // (GUARDIAN_CASTLE_FEASIBILITY.md §J):
        //   * HARD  — the host mirrors the difficulty byte into it, because the script
        //             VM has NO difficulty condition (§J.3 enumerates all five arms).
        //             The asset's condition container then reads it like any other flag.
        //   * ENTER — our 「예」 branch runs `op 0x23 <ENTER> 1`. We consume it and write
        //             it back to 0.
        //
        // ⛔ NOT a string match on the choice rows any more. `[예, 아니오]` is the exact
        //    row pair of **43 other choice sites** in this game (measured), and a wrong
        //    match drops the player into level-60 monsters. Nothing but our own script
        //    ever writes ENTER, so the byte is a signature the rows can never be.
        // ⛔ Both writes are read-verify-then-write and only happen when the byte really
        //    differs — that array is carried by the save (`serialiseSave 0x1affc`).
        // ★ Costs two byte reads per poll while idle, and nothing once a run is going.
        if frame % 5 == 2 && tower.current_floor() == 0 && !tower.forking() {
            // ★ 138차: a quick-start picker whose world is gone is withdrawn — its answer
            //   would start a run for whatever character enters the field next.
            if tower_pick.is_some() {
                // ★ 148차: `|| world_swapped` — a load is the shell reached (see `WORLD_SWAP_FRAMES`).
                let in_shell = world_swapped || {
                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                    items::shell_state(&read)
                };
                if in_shell {
                    tower_pick = None;
                    let _ = item_proxy.send_event(UserEvent::OrbPick(None));
                    log::warn!("guardian: quick-start picker withdrawn — the shell was reached");
                }
            }
            let (mirror, entry, seal) = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                // ⛔ Unreadable difficulty is NOT hard — fail closed, same as
                // `guardian::entry_allowed`. The testing knob is the only override.
                // ★ 114차: on VANILLA `hard` is always false — the mirror then writes 0 over
                //   a HARD flag a main save may have carried in, and both entrance NPCs fall
                //   through to the ORIGINAL block of their condition container on normal and
                //   hard alike (`flavor.rs`). Still read-verify-then-write: a settled vanilla
                //   session writes nothing.
                let hard = crate::flavor::TOWER
                    && crate::guardian::hard_override()
                        .unwrap_or_else(|| items::difficulty_byte(&read).is_some_and(|b| b != 0));
                (
                    items::guardian_hard_mirror(&read, u8::from(hard)),
                    items::guardian_entry_flag(&read),
                    // ⛔⛔ LOWER THE SEAL. This block only runs when no run is going, so
                    // "idle" is exactly when every scene must be allowed to run its own
                    // scripts again. Leaving it raised would make the whole game
                    // script-dead: no doors, no NPCs, no quests, and no way to tell why.
                    // Read-verify-then-write, so the normal case costs one byte read.
                    items::guardian_tower_flag(&read, 0),
                )
            };
            if let Some((addr, want)) = mirror {
                let ok = emulator.write_memory(addr, &[want]);
                log::warn!("guardian: hard-mode mirror at {addr:#x} -> {want} (write ok = {ok})");
            }
            if let Some((addr, want)) = seal {
                let ok = emulator.write_memory(addr, &[want]);
                log::warn!("guardian: tower seal at {addr:#x} -> {want} (write ok = {ok}) — the scenes run their own scripts again");
            }
            // ★ One line, only when it changes: "why is the passer-by still saying
            //   무섭다" and "why is there no shortcut yet" are the two questions this
            //   feature generates, and both are answered by these two bytes. Same rule
            //   as 80차-E's `char stats` line — fail closed, BUT SAY SO.
            {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                let now = (
                    items::guardian_flag(&read, items::GUARDIAN_FLAG_HARD),
                    items::guardian_flag(&read, items::GUARDIAN_FLAG_HEARD),
                );
                if now != tower_npc_flags && now.0.is_some() {
                    tower_npc_flags = now;
                    log::warn!(
                        "guardian: entrance flags — hard={:?} heard={:?} (hard 0 = the passer-by keeps the shipped dialogue)",
                        now.0, now.1
                    );
                }
            }
            if let Some(addr) = entry {
                // ⛔ Clear FIRST. If the entry is refused (no floor table, not in the
                // field yet, the ending gate) the flag must not fire again on the next
                // poll — the player answered once.
                let cleared = emulator.write_memory(addr, &[0]);
                log::warn!("guardian: the entrance dialogue asked to enter (flag {addr:#x}, cleared = {cleared})");
                // ★ No flash here any more: it is raised where the warp actually goes
                //   out (the `Enter` arm of the tower poll), so a refused entry — no
                //   floor table on this device, a request that never reached the field —
                //   does not flash and then go nowhere (code review 2026-09-11).
                // Straight through the SAME gate the control file uses, so the
                // difficulty rule, the field requirement and the floor table are all
                // checked in one place (`guardian::HARD_ONLY`).
                if crate::flavor::TOWER {
                    // ★ 138차: a slot that has cleared a boss picks its start floor (1 · 11 · …
                    //   · best boss + 1) in a host list window; one that has not enters floor 1
                    //   as before. Either way the `Enter` goes through the same gates below.
                    sync_tower_progress(&data_dir, &mut tower_prog, &mut tower_prog_slot, &mut tower);
                    match tower_pick_rows(tower_prog.best_boss) {
                        Some((offered, pick)) => {
                            log::warn!(
                                "guardian: quick start offered — floors {offered:?} (slot {tower_prog_slot:?}, best boss {})",
                                tower_prog.best_boss
                            );
                            tower_pick = Some(offered);
                            let _ = item_proxy.send_event(UserEvent::OrbPick(Some(Box::new(pick))));
                        }
                        None => tower_cmd = Some(crate::guardian::Command::Enter { floor: 1, force: false }),
                    }
                } else {
                    // Only our own hard-only script ever sets ENTER and vanilla never lets
                    // that script run, so this is a main save's leftover: cleared above,
                    // and nothing starts (`Run::new(vec![])` would refuse anyway).
                    log::warn!("guardian: vanilla flavour — the ENTER flag was set (a save from main?) and was cleared; there is no tower on this flavour");
                }
            }

            // ── The confirm cursor sticks on 아니오 ─────────────────────────────
            //
            // The game only resets the shared list cursor when it equals 2 (`0x3c65c
            // cmp #2`), so for our two-row 예/아니오 confirm the cursor PERSISTS: after
            // the player picks 아니오 (cursor -> 1), re-opening the dialogue keeps it at
            // 1 and every OK lands on 아니오 again — "ok를 계속 눌러도 안 들어가진다"
            // (user 2026-09-16). The guest sets ENTER correctly (asset B[18]/B[19] end
            // in `23 c2 01`); the only failure is that the player cannot reach 예.
            //
            // So on the RISING edge of the confirm opening — in the entrance town, in
            // hard mode (our dialogue is hard-only) — write the shared list cursor
            // (`CHOICE_CURSOR_OFF`, the byte the choice VM reads via 0x574c0) to 0, so OK
            // defaults to 예. Written ONCE per opening, so DOWN still reaches 아니오.
            let (choice_open, on_entrance, hard_set, cursor_addr) = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                (
                    items::guardian_choice_open(&read).unwrap_or(false),
                    items::scene_now(&read).is_some_and(|s| items::GUARDIAN_ENTRANCE_SCENES.contains(&s)),
                    items::guardian_flag(&read, items::GUARDIAN_FLAG_HARD) == Some(1),
                    items::guardian_choice_cursor_addr(&read),
                )
            };
            if crate::flavor::TOWER && choice_open && !entrance_choice_was && on_entrance && hard_set {
                if let Some(addr) = cursor_addr {
                    let ok = emulator.write_memory(addr, &[0]);
                    log::warn!("guardian: entrance confirm opened — cursor reset to 예 at {addr:#x} (write ok = {ok})");
                }
            }
            entrance_choice_was = choice_open;
        }

        // ── 하늘성: apply the revive once the player is back in a field ───────────
        // Armed by a death that ended a run (see the tower poll). We wait for a field
        // with a resolvable character — the return warp has to land and the town scene
        // load — then write full HP+SP, exactly the game's own revive/full-restore.
        // ⛔ `full_restore_writes` is `None` on a half-built or 0-HP-unresolvable CHAR, so
        // this simply retries next poll until the character reads live again. Dropped at
        // the deadline so it cannot heal a later, unrelated character.
        if tower_heal_pending && frame % 5 == 4 {
            let (in_field, in_shell, plan) = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                // ★ 148차: a load is the shell reached — the heal was for the unloaded world.
                (items::in_field_state(&read), world_swapped || items::shell_state(&read), items::full_restore_writes(&read))
            };
            if frame >= tower_heal_deadline {
                tower_heal_pending = false;
                log::warn!("guardian: gave up on the post-death heal — never got back to a field in time");
            } else if in_shell {
                // The heal was for the character carried back to town IN MEMORY at 0 HP.
                // Once the shell is reached (a game-over the run's warp did not pre-empt,
                // 메인메뉴), whatever enters the next field is a record loaded from disk
                // with its own saved HP — nothing of ours to undo, and possibly another
                // slot's character.
                tower_heal_pending = false;
                log::warn!("guardian: the post-death heal is dropped — the shell was reached, the next character comes from a saved record");
            } else if in_field
                && let Some(writes) = plan
            {
                for (addr, bytes) in &writes {
                    emulator.write_memory(*addr, bytes);
                }
                tower_heal_pending = false;
                log::warn!("guardian: revived the player to full HP after leaving the tower");
            }
        }

        // ── 하늘성: a debt is owed to the character who earned it (117차) ─────────
        // The clear EXP (112차) and the boss-floor rows (85차) are OWED and paid on a
        // later field poll — and nothing ever forgave them. A clear with a window open,
        // then 메인메뉴 or a game over, then another slot's 게임시작: the first field
        // poll paid THAT character (`exp +=` → `checkLevelUp`, `giveItem` rows). The
        // same rule as the heal above: once the shell is reached, whatever enters the
        // next field is a record loaded from disk, and the debt dies with the world it
        // was earned in. (A run's own unsaved progress dies the same way — saving is
        // blocked inside the tower, so nothing of it survives the shell either.)
        if (tower_exp_owed > 0 || !tower_owed.is_empty() || tower_shards_owed > 0) && frame % 5 == 4 {
            // ★ 148차: a load is the shell reached — the debt belongs to the unloaded world,
            //   and a fast 메인메뉴 → 게임시작 would otherwise pay it to the next character.
            let in_shell = world_swapped || {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::shell_state(&read)
            };
            if in_shell {
                log::warn!(
                    "guardian: the shell was reached with {tower_exp_owed} EXP, {} reward row(s) and {tower_shards_owed} shard(s) still owed — dropped: the next character comes from a saved record",
                    tower_owed.len()
                );
                tower_exp_owed = 0;
                tower_owed.clear();
                tower_give_wait = None;
                tower_shards_owed = 0;
                tower_shard_waits = 0;
            }
        }

        // ── 하늘성: one poll of the tower ────────────────────────────────────
        // Skipped entirely while idle and unasked, which is what makes the feature
        // free in a build that nobody has started a run in.
        // ★ `tower_can_save_was` keeps this block alive for a few polls AFTER the run
        //   ends, which is the whole point: the byte has to be given back and the leave
        //   warp has not landed yet at the moment `Action::Leave` is applied. It is
        //   cleared the instant the restore is confirmed, so an idle session is still
        //   an idle session.
        if frame % 30 == 11
            && (tower_cmd.is_some()
                || tower.forking()
                || tower.current_floor() != 0
                || tower_can_save_was.is_some()
                || tower_land_frames > 0
                // ★ 138차 §3.1 ⓐ: the exit reward is owed AFTER the run, and the block used to
                //   close ~20 polls later with rows still queued (dropped at the shell).
                || !tower_owed.is_empty()
                || tower_shards_owed > 0
                || tower_exp_owed > 0
                // ★ 143차: the EXIT tab's answer (no run needed — a floor scene is enough)
                //   and the wake, which outlives the run it ends.
                || TOWER_EXIT_REQUEST.load(std::sync::atomic::Ordering::Relaxed)
                || wake.is_some())
        {
            // Every READ first, then the writes: the read closure borrows the emulator
            // and `write_memory` needs it mutably. Same shape as the auto-loot fork.
            // ★ 143차: + the hero (the fallback way home), the script window and the wake
            //   flag (the wake's "has the game said it" view).
            let (obs, difficulty, creatures, hero, script_open, wake_flag) = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                let obs = crate::guardian::Obs {
                    scene: items::scene_now(&read),
                    // Filled in below from the snapshot, which is bounded by the
                    // MAP object's own count — reading past it would invent monsters.
                    spawned: 0,
                    alive: 0,
                    // STRICT: may a warp be posted? Only the field services the
                    // request byte.
                    in_field: items::in_field_state(&read),
                    // LOOSE: is the run still valid? ⛔ NOT `in_field_state` — that is
                    // false whenever any window is up, so using it here would throw the
                    // player out of the tower for opening their bag. App state 3 is the
                    // title / slot / online screens (66차-C), and leaving the world is
                    // the only thing that really ends a floor.
                    // ★★ 148차: `&& !world_swapped`. A save loaded from inside the tower in
                    // under 30 frames never showed this poll the shell; the run then saw a
                    // foreign scene WITH a world, answered `LeftTheField`, and posted the way
                    // home onto the character that had just been loaded.
                    in_world: !world_swapped && !items::shell_state(&read),
                    // ⛔ `None` (an unreadable stat) is NOT death — see `Obs::player_hp`.
                    player_hp: items::read_stats(&read).map(|s| s.hp.min(u32::from(u16::MAX)) as u16),
                    player_tile: items::player_position(&read).map(|(_, _, _, tx, ty, _, _)| (tx, ty)),
                };
                let creatures = items::guardian_creatures(&read).unwrap_or_default();
                (
                    obs,
                    items::difficulty_byte(&read),
                    creatures,
                    items::char_hero(&read),
                    items::cutscene_open(&read),
                    items::guardian_flag(&read, items::GUARDIAN_FLAG_WAKE),
                )
            };
            let obs = crate::guardian::Obs {
                spawned: creatures.len() as i32,
                alive: creatures.iter().filter(|c| c.alive()).count() as i32,
                ..obs
            };
            tower_alive = obs.alive;

            // ★ 143차: the EXIT tab's 「예」 — and the control file's `leave`, which is the
            // same request made with `adb`.
            let mut exit_now = TOWER_EXIT_REQUEST.swap(false, std::sync::atomic::Ordering::SeqCst);
            // A request from the control file, judged now that the guest is readable.
            if let Some(cmd) = tower_cmd.take() {
                match cmd {
                    crate::guardian::Command::Leave => {
                        log::warn!("guardian: leaving on request (floor {})", tower.current_floor());
                        exit_now = true;
                    }
                    // ★ 143차 (testing only): every live monster on the floor to 0 HP — the
                    //   next poll sees the floor clear. What drives a top-floor wake on a
                    //   device, where a fight cannot be scripted with `adb`. Arms the lever;
                    //   the block right after this match applies it (now, if the floor is
                    //   live; else to the first floor that goes live inside the window).
                    crate::guardian::Command::Clear => {
                        tower_autoclear_until = frame + TOWER_AUTOCLEAR_FRAMES;
                        if !tower.forking() {
                            log::warn!(
                                "guardian: test lever `clear` armed — the next floor to go live within {TOWER_AUTOCLEAR_FRAMES} frames goes to 0 HP (state {:?})",
                                tower.state
                            );
                        }
                    }
                    crate::guardian::Command::Enter { floor, force } => {
                        // ⛔ FAIL CLOSED, AND SAY SO. An unreadable difficulty byte
                        // refuses, and the refusal is logged with the byte in it — 78차
                        // survived eight releases because its gate declined in silence.
                        // ★★★ 122차 (2026-09-21): EVERY refusal below is also a TOAST. The
                        //   entrance NPC's 「예」 used to end in a log line and nothing on
                        //   screen — a device with no floor table (every hand-out APK, and
                        //   every reinstall) looked exactly like "예를 눌러도 아무일도
                        //   안일어난다". Fail closed, BUT SAY SO (78차-C's rule). The table
                        //   itself is built into the binary now (`config::GUARDIAN_FLOORS_BUILTIN`),
                        //   so the table arm is a guard against a floor number the table lacks.
                        if !force && !crate::guardian::entry_allowed(difficulty) {
                            // ★ WHICH RULE REFUSED IS IN THE LINE. `hard_only=false` means
                            // the only thing that can refuse here is an unreadable byte,
                            // and that is a very different report from "wrong difficulty".
                            log::warn!(
                                "guardian: refusing floor {floor} — difficulty byte at \
                                 CHAR+{:#x} reads {difficulty:?} (hard_only={}), so this \
                                 character may not enter (a control file may say `force 1` to test)",
                                items::DIFFICULTY_OFF,
                                crate::guardian::HARD_ONLY
                            );
                            let _ = item_proxy.send_event(UserEvent::Notice("하늘성 입장 실패: 난이도 판독 불가"));
                        } else if !obs.in_field {
                            // ⛔ WAIT, DO NOT REFUSE. The warp request byte is only
                            // serviced from the field, but the most ordinary way to ask
                            // for a floor is the entrance NPC — and when that dialogue
                            // closes the script window is still up for a frame or two,
                            // so refusing here would make the in-game entrance fail
                            // silently every time. Same reasoning as `next_floor`
                            // (§D.4): a request nothing consumes is worse than a wait.
                            if tower_cmd_waits < TOWER_CMD_WAITS {
                                tower_cmd_waits += 1;
                                tower_cmd = Some(crate::guardian::Command::Enter { floor, force });
                            } else {
                                tower_cmd_waits = 0;
                                log::warn!(
                                    "guardian: giving up on floor {floor} — never reached the field in {TOWER_CMD_WAITS} poll(s) (a window stayed open, or no world is loaded)"
                                );
                                let _ = item_proxy.send_event(UserEvent::Notice("하늘성 입장 실패: 필드가 아님"));
                            }
                        } else if tower.floor(floor).is_none() {
                            // Since 122차 the table is compiled in, so this is either a floor
                            // number the table does not have or an OVERRIDE file that
                            // replaced it with fewer rows (`config::read_guardian_floors`).
                            log::warn!(
                                "guardian: refusing floor {floor} — the floor table has {} row(s) \
                                 (built in, unless {} overrides it)",
                                tower.len(),
                                crate::config::GUARDIAN_FLOORS_FILE
                            );
                            let _ = item_proxy.send_event(UserEvent::Notice("하늘성 입장 실패: 층 표 없음"));
                        } else {
                            log::warn!(
                                "guardian: entering floor {floor} from scene {:?} (force = {force}, \
                                 best so far {})",
                                obs.scene,
                                tower.best
                            );
                            tower_paid = None;
                            tower_hold = 0;
                            tower_cmd_waits = 0;
                            let act = tower.enter(floor, &obs);
                            // ⛔ ONLY on a warp that is actually going to happen.
                            // `enter` also answers `Leave` (a floor the table does not
                            // have), and "하늘성 입장" on a refusal is worse than silence.
                            // ⛔ And only here: `next_floor` enters too, and a toast per
                            // floor would fight the status line for the same corner.
                            if matches!(act, crate::guardian::Action::Warp { .. }) {
                                // ★ The side line (user 2026-09-30), not the box: the first
                                //   floor's monsters are already moving, and a box would
                                //   also have swallowed the EXIT tab until it was answered.
                                let _ = item_proxy.send_event(UserEvent::SideNote("하늘성 입장".into()));
                                // ★ The entrance flash obeys the same rule. It used to be
                                // raised where the ENTER flag was consumed, i.e. before any
                                // of the gates above had been asked — so a device without a
                                // floor table, or a request that never reached the field,
                                // flashed and then went nowhere (code review 2026-09-11).
                                tower_flash = tower_flash.wrapping_add(1);
                            }
                            let home = leave_home(&tower, act, &obs, hero);
                            let dest = warp_scene(act);
                            queue_exit_reward(&tower, act, frame, &mut tower_owed, &mut tower_shards_owed, &item_proxy, None);
                            if apply_guardian_action(&mut *emulator, &mut tower, act, &obs, &creatures, &mut tower_can_save_was, home.map(|h| h.0)) {
                                tower_land_frames = crate::guardian::LANDING_FRAMES;
                                tower_return = home.map(|(scene, tile)| ReturnLeg { scene, tile, posted: frame, reposts: 0 });
                                tower_arrive = dest;
                            }
                        }
                    }
                }
            }

            // ── ★ 143차: the EXIT tab (and the control file's `leave`) ────────────
            //
            // User 2026-09-30: *"하늘성안으로 들어갔을떄는 map버튼 왼쪽에 exit 버튼을 둬서 그걸
            // 누르면 하늘성에서 나가시겠습니까? 창이뜨고 예누르면 나와졌으면 좋겠어"*. A run
            // leaves by its own road (`Run::leave` → `Abort::Quit`: the exit reward, the way
            // back, `finish`) — the poll right below answers the `Leave`. With NO run and the
            // player standing in a floor scene (a save made inside the tower before 143차),
            // there is nothing to leave, so the way out is posted here directly to the hero's
            // entrance town (`guardian::fallback_home`).
            // ⛔ Not during the wake (it is already on its way out), and not on a death — the
            //    run's own `PlayerDown` arm revives before it warps, and a Quit would not.
            if exit_now {
                let in_floor_scene = obs.scene.is_some_and(|s| tower.is_floor_scene(s));
                if wake.is_some() {
                    log::warn!("guardian: EXIT ignored — the finished tower is already taking the player home");
                } else if obs.player_hp == Some(0) {
                    log::warn!("guardian: EXIT ignored — the player is down; the run's own death arm revives and sends them back");
                } else if tower.current_floor() != 0 {
                    log::warn!("guardian: EXIT — leaving the run on floor {}", tower.current_floor());
                    let _ = tower.leave();
                } else if in_floor_scene && obs.in_world {
                    let (scene, tile) = crate::guardian::fallback_home(hero);
                    let plan = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        items::guardian_warp_writes(&read, scene)
                    };
                    match plan {
                        Some(writes) => {
                            let ok = writes.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
                            log::warn!(
                                "guardian: EXIT with no run from floor scene {:?} (a save made inside the tower) — sending the {hero:?} home to scene {scene} tile {tile:?} ({ok}/{} write(s))",
                                obs.scene,
                                writes.len()
                            );
                            tower_land_frames = crate::guardian::LANDING_FRAMES;
                            tower_return = Some(ReturnLeg { scene, tile: Some(tile), posted: frame, reposts: 0 });
                            tower_arrive = None;
                        }
                        None => log::warn!("guardian: EXIT — cannot post a warp on this build (no scene table)"),
                    }
                } else {
                    log::warn!("guardian: EXIT ignored — no run and not in a floor scene ({:?})", obs.scene);
                }
            }

            // ★ 143차: a wake whose world is gone (메인메뉴, a game over) has nothing left to
            // carry home: take the veil down now and forget what it held back — the next
            // character comes from a saved record (the 117차 rule the owed rows follow).
            if wake.is_some() && !obs.in_world {
                log::warn!("guardian: the wake is dropped — the shell was reached ({} held box(es) dropped)", wake_notices.len());
                let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: false, ms: 0 });
                wake = None;
                wake_home = None;
                wake_flag_armed = false;
                wake_notices.clear();
            }

            // The linger on a cleared floor, then up.
            if matches!(tower.state, crate::guardian::State::Cleared { .. }) {
                tower_hold = tower_hold.saturating_add(1);
                if tower.at_the_top() {
                    // ★★★ 143차: the TOP floor does not leave on this clock any more. It used
                    // to (138차: `next_floor` → `Abort::Completed` → the plain way home), and
                    // the user asked for a way out that reads as waking up: the wake whitens
                    // the screen, carries the player home under it, clears it, and the game
                    // says 「의식이 몽롱하다...」 (`guardian::WakePhase`, driven just below).
                    if tower_hold >= TOWER_TOP_HOLD && wake.is_none() {
                        log::warn!("guardian: floor {} finished — the wake begins (waiting for a quiet screen)", crate::guardian::FLOORS);
                        wake = Some(crate::guardian::WakePhase::start(now_ms()));
                        wake_home = None;
                        wake_flag_armed = false;
                        wake_notices.clear();
                    }
                } else if tower_hold >= TOWER_CLEAR_HOLD {
                    tower_hold = 0;
                    let act = tower.next_floor(&obs);
                    let home = leave_home(&tower, act, &obs, hero);
                    let dest = warp_scene(act);
                    queue_exit_reward(&tower, act, frame, &mut tower_owed, &mut tower_shards_owed, &item_proxy, None);
                    if apply_guardian_action(&mut *emulator, &mut tower, act, &obs, &creatures, &mut tower_can_save_was, home.map(|h| h.0)) {
                        tower_land_frames = crate::guardian::LANDING_FRAMES;
                        tower_return = home.map(|(scene, tile)| ReturnLeg { scene, tile, posted: frame, reposts: 0 });
                        tower_arrive = dest;
                    }
                }
            }

            // ★ 143차: a wake that has not gone home yet belongs to a top floor that is still
            // WAITING. If the run ended some other way meanwhile (a death, a warp the tower
            // did not post), there is nothing to wake from: drop it — and if its veil was
            // already rising, bring it down — rather than whiten over a run that is gone
            // and then say 「의식이 몽롱하다...」 about nothing.
            if let Some(p) = wake
                && !p.gone_home()
                && !tower.at_the_top()
            {
                log::warn!("guardian: the wake is cancelled — the top floor stopped waiting (state {:?})", tower.state);
                if p.veiled() {
                    let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: false, ms: crate::guardian::WAKE_CLEAR_MS });
                }
                wake = None;
                wake_home = None;
                wake_flag_armed = false;
                wake_notices.clear();
            }

            // ── ★★★ 143차: the wake, one step per poll ───────────────────────────
            if let Some(phase) = wake.as_mut() {
                let now = now_ms();
                let view = crate::guardian::WakeView {
                    now,
                    quiet: obs.in_field && !crate::notice_up(),
                    home: wake_home.is_some_and(|h| obs.scene == Some(u16::from(h))) && tower_return.is_none(),
                    script_open,
                    flag_armed: wake_flag_armed,
                    flag: wake_flag,
                };
                let step = crate::guardian::wake_step(phase, &view);
                let mut over = false;
                match step {
                    crate::guardian::WakeStep::None => {}
                    crate::guardian::WakeStep::Whiten { ms } => {
                        log::warn!("guardian: wake — the screen whitens over {ms} ms");
                        let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: true, ms });
                    }
                    crate::guardian::WakeStep::GoHome => {
                        // The screen is white. ⛔ The flag FIRST, the warp second: the entry
                        // row that says the line runs once, at the load the warp starts.
                        let act = tower.next_floor(&obs);
                        let home = leave_home(&tower, act, &obs, hero);
                        if !matches!(act, crate::guardian::Action::Leave { why: crate::guardian::Abort::Completed }) {
                            // Something else ended the run first (a death, the shell) — its
                            // own arm has already carried it. The veil comes down.
                            log::warn!("guardian: wake — the top floor was no longer waiting ({act:?}); taking the veil down");
                            let s = phase.home_failed(now);
                            if let crate::guardian::WakeStep::Clear { ms } = s {
                                let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: false, ms });
                            }
                        } else {
                            wake_flag_armed = false;
                            if let Some((scene, _)) = home
                                && items::GUARDIAN_ENTRANCE_SCENES.contains(&u16::from(scene))
                            {
                                let plan = {
                                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                                    items::guardian_wake_flag(&read, 1)
                                };
                                wake_flag_armed = match plan {
                                    Some((addr, want)) => {
                                        let ok = emulator.write_memory(addr, &[want]);
                                        log::warn!("guardian: wake — the line is armed for scene {scene} (flag at {addr:#x} -> 1, write ok = {ok})");
                                        ok
                                    }
                                    // Already up (a line that never ran) — still armed.
                                    None => wake_flag == Some(1),
                                };
                            }
                            queue_exit_reward(&tower, act, frame, &mut tower_owed, &mut tower_shards_owed, &item_proxy, Some(&mut wake_notices));
                            if apply_guardian_action(&mut *emulator, &mut tower, act, &obs, &creatures, &mut tower_can_save_was, home.map(|h| h.0)) {
                                tower_land_frames = crate::guardian::LANDING_FRAMES;
                                tower_return = home.map(|(scene, tile)| ReturnLeg { scene, tile, posted: frame, reposts: 0 });
                                tower_arrive = None;
                                wake_home = home.map(|h| h.0);
                                log::warn!("guardian: wake — floor {} finished; the way home to {wake_home:?} is posted under the white", crate::guardian::FLOORS);
                            } else {
                                // Nothing went out (no world, no scene table): the flag must
                                // not wait for a load that is not coming.
                                if wake_flag_armed {
                                    let plan = {
                                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                                        items::guardian_wake_flag(&read, 0)
                                    };
                                    if let Some((addr, want)) = plan {
                                        emulator.write_memory(addr, &[want]);
                                    }
                                    wake_flag_armed = false;
                                }
                                log::warn!("guardian: wake — no way home could be posted; the veil comes down where the player stands");
                                if let crate::guardian::WakeStep::Clear { ms } = phase.home_failed(now) {
                                    let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: false, ms });
                                }
                            }
                        }
                    }
                    crate::guardian::WakeStep::Clear { ms } => {
                        log::warn!(
                            "guardian: wake — {} the veil lifts over {ms} ms",
                            if view.home { "home;" } else { "the way home never showed its scene;" }
                        );
                        let _ = item_proxy.send_event(UserEvent::WakeVeil { rise: false, ms });
                    }
                    crate::guardian::WakeStep::SayOnHost => {
                        // ⛔ A flag still up would speak at the NEXT visit to the town.
                        if wake_flag == Some(1) {
                            let plan = {
                                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                                items::guardian_wake_flag(&read, 0)
                            };
                            if let Some((addr, want)) = plan {
                                emulator.write_memory(addr, &[want]);
                            }
                        }
                        log::warn!("guardian: wake — the game did not say the line (armed = {wake_flag_armed}, flag {wake_flag:?}); the host says it");
                        let _ = item_proxy.send_event(UserEvent::Notice(crate::guardian::WAKE_LINE));
                        over = true;
                    }
                    crate::guardian::WakeStep::Done => {
                        log::warn!("guardian: wake — over (the line was said and closed)");
                        over = true;
                    }
                }
                if over {
                    for text in wake_notices.drain(..) {
                        let _ = item_proxy.send_event(UserEvent::NoticeText(text));
                    }
                    wake = None;
                    wake_home = None;
                    wake_flag_armed = false;
                }
            }

            let act = tower.poll(&obs);
            // ── Revive on a death that ends the run (user 2026-09-16) ─────────────
            // The passer-by promises "실제로 죽는건 아니니까" — so a run that ends because
            // the player died must not leave them in town at 0 HP. Arm a full heal to be
            // applied once they are back in a field with a resolvable character. ONLY a
            // death heals: `PlayerDown` is the run's own leave-warp carrying a 0-HP
            // character back to town in memory, which is the case the player named.
            // ⛔ Not `LeftTheField` (code review 2026-09-17): that arm covers `!in_world`
            // — 시스템 → 메인메뉴, or the game's own game-over to the title — after which
            // the next 게임시작 loads a record (any slot) at its SAVED HP, and a heal armed
            // here wrote HP=max · SP=1000 into that unrelated, freshly loaded character.
            if let crate::guardian::Action::Leave { why } = act {
                if matches!(why, crate::guardian::Abort::PlayerDown) {
                    tower_heal_pending = true;
                    tower_heal_deadline = frame.saturating_add(1800);
                }
            }
            // Pay a cleared floor exactly once. `Action::Reward` is offered on every
            // poll (see `Run::finish`), so the guard is the floor number and not an edge.
            if let crate::guardian::Action::Reward { floor, boss } = act {
                if tower_paid != Some(floor) {
                    tower_paid = Some(floor);
                    // ★ 138차: the SLOT's progress — the quick start reads its best boss —
                    //   written now, whatever the player saves afterwards (결정 ⑤). The item
                    //   reward is no longer paid floor by floor: it waits for the run to END
                    //   (`queue_exit_reward`), and `tower.run_best_boss` now carries a boss.
                    sync_tower_progress(&data_dir, &mut tower_prog, &mut tower_prog_slot, &mut tower);
                    if tower_prog.note_clear(floor, boss) {
                        match tower_prog_slot {
                            Some(slot) => {
                                if config::write_tower_progress(&data_dir, slot, &tower_prog) && boss {
                                    log::warn!(
                                        "guardian: slot {slot} cleared boss floor {floor} — best boss now {} (the quick start offers {:?})",
                                        tower_prog.best_boss,
                                        crate::guardian::entry_floors(tower_prog.best_boss)
                                    );
                                }
                            }
                            None => log::warn!("guardian: floor {floor} cleared with no save slot pinned — the progress is not recorded"),
                        }
                    }
                    log::warn!(
                        "guardian: floor {floor} cleared (boss = {boss}); run best boss {}, slot best floor {}",
                        tower.run_best_boss,
                        tower_prog.best
                    );
                    // ── The clear EXP (112차, "새 컨텐츠가 exp 직접 주게") ─────────────
                    // Owed, not written here: the grant needs a field poll (below), and
                    // this arm can fire with a window open.
                    let exp = tower.floor(floor).map_or(0, |f| f.exp);
                    if exp > 0 {
                        tower_exp_owed = tower_exp_owed.saturating_add(exp);
                        log::warn!("guardian: floor {floor} pays {exp} EXP on clear ({tower_exp_owed} owed)");
                        // ★ The side line, not the box (user 2026-09-30: *"하늘성 각 층 클리어
                        //   보상도 팝업형태가아니라 좌측안내문구로만"*) — the next floor comes on
                        //   its own, and a box would stand in the way of it. The top floor's
                        //   wake still waits for this line to fade (`crate::notice_up`).
                        let _ = item_proxy.send_event(UserEvent::SideNote(format!("하늘성 {floor}층 클리어 EXP +{exp}").into()));
                    } else {
                        let _ = item_proxy.send_event(UserEvent::SideNote("층 클리어".into()));
                    }
                }
            }
            let home = leave_home(&tower, act, &obs, hero);
            let dest = warp_scene(act);
            queue_exit_reward(&tower, act, frame, &mut tower_owed, &mut tower_shards_owed, &item_proxy, None);
            if apply_guardian_action(&mut *emulator, &mut tower, act, &obs, &creatures, &mut tower_can_save_was, home.map(|h| h.0)) {
                tower_land_frames = crate::guardian::LANDING_FRAMES;
                tower_return = home.map(|(scene, tile)| ReturnLeg { scene, tile, posted: frame, reposts: 0 });
                tower_arrive = dest;
            }

            // ★ 143차 (testing only): the armed `clear` lever meets a live floor — AFTER the
            // poll's fork has been written, and as early as the poll that entered the floor.
            // ⛔ Not before the fork: its writes compare against memory at write time, so a
            // zero written first is read as a wound to top up and the fork puts every HP back
            // (device, vc417: 9 zeroed, 9 re-asserted in the same poll, the player dead 0.4 s
            // later). Written after it, the next poll reads the records dead, the fork skips
            // corpses, and `Run::poll` sees the floor cleared.
            if frame < tower_autoclear_until && tower.forking() {
                tower_autoclear_until = 0;
                let live: Vec<i32> = creatures.iter().filter(|c| c.alive()).map(|c| c.index).collect();
                let plan = {
                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                    items::guardian_zero_hp_writes(&read, &live)
                };
                let ok = plan
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|(addr, bytes)| emulator.write_memory(*addr, bytes))
                    .count();
                log::warn!("guardian: test lever `clear` on floor {} — {} live monster(s) to 0 HP ({ok} write(s))", tower.current_floor(), live.len());
            }

            // ── Hand over the clear EXP (112차) ──────────────────────────────────
            //
            // Through the game's own `checkLevelUp`, exactly like 천상의 영약: add to
            // `CHAR+0xe4`, then let the game level up (points, HP/SP refills, the
            // level-up effect, the 99 cap — and a multi-level climb, its loop runs up
            // to 15). ⛔ Field only, like the item rows below: the shell screens reach
            // this loop with a merely `resolve`-able record (80차-E).
            if tower_exp_owed > 0
                && let Some(f) = crate::guest_build::level_up_fn()
            {
                let plan = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::in_field_state(&read)
                        .then(|| items::tower_exp_writes(&read, tower_exp_owed))
                        .flatten()
                };
                if let Some((addr, bytes, before, after, level)) = plan {
                    if emulator.write_memory(addr, &bytes) {
                        emulator.call_guest_function(f, &[]);
                        log::warn!("guardian: gave {tower_exp_owed} EXP (exp {before} -> {after} at Lv{level}) -> checkLevelUp({f:#x})");
                        tower_exp_owed = 0;
                    } else {
                        log::warn!("guardian: the EXP write at {addr:#x} was refused — {tower_exp_owed} EXP still owed");
                    }
                }
            }

            // ── The exit reward's shards (138차) ─────────────────────────────────
            //
            // Straight into the 재료가방 (`BagBoxHost::grant_boxed` — file first, one 획득
            // notice), not thirty trips through the giver below. What the box does not take
            // (not armed for this slot within ten polls, or full) goes to the giver, so
            // nothing is lost. ⛔ Field only, like every payment here.
            // ★ 143차: and not during the wake — the finished tower's 획득 lines would scroll
            //   away under the white, and after it they would crowd the line the game says.
            //   The wake hands them the screen once it is over (`wake` back to `None`).
            if tower_shards_owed > 0 && wake.is_none() {
                let in_field = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::in_field_state(&read)
                };
                if in_field {
                    let m = crate::guardian_rewards::MATERIAL;
                    let left = bagbox.grant_boxed(&mut *emulator, u16::from(m.cat), u16::from(m.idx), tower_shards_owed);
                    if left == tower_shards_owed && tower_shard_waits < 10 {
                        tower_shard_waits += 1;
                    } else {
                        if left > 0 {
                            for _ in 0..left {
                                tower_owed.push_back((u16::from(m.cat), u16::from(m.idx)));
                            }
                            log::warn!("guardian: {left} shard(s) handed to the game's giver (the box was not armed, or is full)");
                        }
                        tower_shards_owed = 0;
                        tower_shard_waits = 0;
                    }
                }
            }

            // ── Hand over what the ended run owes ────────────────────────────────
            //
            // One row per poll through the game's own `0x52b14`, the same primitive
            // (and the same async patience) the `wie_grant.txt` maintenance grant
            // uses. The game does the bag walk, the 99-cap and the stack merge, so
            // the host never touches a bag cell.
            //
            // ⛔ Field only. The giver walks the bag grid off CHAR, and the shell
            // screens reach this loop with a character that is merely `resolve`-able
            // (80차-E) — paying there would write into whatever record is loaded.
            // ★ 143차: after the wake, like the shards above.
            if !tower_owed.is_empty()
                && wake.is_none()
                && let Some(f) = crate::guest_build::give_item_fn()
            {
                let in_field = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::in_field_state(&read)
                };
                if in_field && let Some(&(cat, idx)) = tower_owed.front() {
                    let inflight = tower_give_wait
                        .is_some_and(|(c, i, fr)| c == cat && i == idx && frame.saturating_sub(fr) < 90);
                    if !inflight {
                        // ★ 138차: announced — `giveItem`'s third argument is the notice flag
                        //   (103차: 0xff = silent), so the game's own 획득 line names the item.
                        emulator.call_guest_function(f, &[u32::from(cat), u32::from(idx), 0, 0]);
                        tower_give_wait = Some((cat, idx, frame));
                        tower_owed.pop_front();
                        log::warn!("guardian: gave cat {cat:#x} idx {idx:#x} ({} row(s) still owed)", tower_owed.len());
                    }
                }
            }

            // ── The wedge net, for the rest of the floor ─────────────────────────
            //
            // The arrival placement above puts the player somewhere chosen, once. This
            // is the other half of *"이동후에 못움직일때가 있어"*: whatever else ends
            // up moving them — a fork write, a knockback, a cause not yet found — a
            // player who can no longer walk ANYWHERE gets put back in the room.
            //
            // ⛔ Only while a floor is live, only once the arrival has settled (so the
            // two never fight), and only from the field. `guardian_landing_writes`
            // answers `None` unless the player is genuinely wedged
            // (`crate::guardian::LANDING_MIN_ROOM` — one flood fill capped at eight
            // tiles), so the steady state of a floor is one capped fill per poll and no
            // writes at all.
            if tower_land_frames == 0 && tower.current_floor() != 0 && obs.in_field {
                let plan = {
                    let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                    items::guardian_landing_writes(&read)
                };
                if let Some((writes, from, to)) = plan {
                    let ok = writes.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
                    log::warn!(
                        "guardian: the player could not walk anywhere from {from:?} — moved to {to:?} ({ok}/{} write(s) ok)",
                        writes.len()
                    );
                }
            }

            // ── Give the canSave byte back once the run is over ──────────────────
            //
            // Re-asserted rather than written once, because the AREA chain reads `None`
            // for a few frames either side of the leave warp and the object may be a
            // different one afterwards. Bounded by `RESTORE_POLLS` so a restore that can
            // never land says so instead of spinning for the session.
            //
            // ⛔ Do not move this into `Action::Leave`: that runs BEFORE the warp back
            // has been serviced, so it would restore the tower floor's AREA and the
            // player would arrive in a town that still refuses to save — which is the
            // exact bug this fixes.
            if tower.current_floor() == 0 && !tower.forking() {
                if let Some(was) = tower_can_save_was {
                    let live = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        items::guardian_can_save_byte(&read)
                    };
                    tower_restore_polls = tower_restore_polls.saturating_add(1);
                    match crate::guardian::restore_can_save(was, live.map(|(_, v)| v)) {
                        crate::guardian::Restore::Done => {
                            log::warn!(
                                "guardian: canSave given back — the byte at {:?} reads {:?} again",
                                live.map(|(a, _)| format!("{a:#x}")),
                                live.map(|(_, v)| v)
                            );
                            tower_can_save_was = None;
                            tower_restore_polls = 0;
                        }
                        crate::guardian::Restore::Write(v) => {
                            if let Some((addr, _)) = live {
                                let ok = emulator.write_memory(addr, &[v]);
                                log::info!("guardian: restoring canSave={v} at {addr:#x} (write ok = {ok})");
                            }
                        }
                        crate::guardian::Restore::Wait => {}
                    }
                    // ⛔ FAIL LOUD. A player whose saving stays broken must be able to
                    // find out why from one line, which is the rule 78차-C wrote down.
                    if tower_restore_polls >= crate::guardian::RESTORE_POLLS && tower_can_save_was.is_some() {
                        log::warn!(
                            "guardian: GIVING UP on restoring the canSave byte after {} poll(s) — saving may stay refused until the app is restarted (byte at {:?} reads {:?}, wanted {was})",
                            tower_restore_polls,
                            live.map(|(a, _)| format!("{a:#x}")),
                            live.map(|(_, v)| v)
                        );
                        tower_can_save_was = None;
                        tower_restore_polls = 0;
                    }
                }
            }
        }
        // ★★★ 하늘성: no loot inside the tower (user decision 2026-09-11 — the tower's
        // rewards are per-floor, §3.4). Set AFTER the poll and OUTSIDE its gate, on the
        // same cheap cadence. After, so that the poll that posts the entry warp already
        // refuses drops and the poll that ends the run already allows them again — read
        // before it, both lagged by one 30-frame poll (code review 2026-09-11). Outside
        // the gate, because that gate stops being true the moment a run ends and the flag
        // has to be FALSE again exactly then.
        // ⛔ Do not latch it: the reaver hook reads it on every kill, so a stale `true`
        // would silently delete the player's normal drops for the rest of the session.
        // ── Stand the player where the warp was meant to put them, per frame ────
        //
        // ⛔ NOT OPTIONAL, and not a nicety. The warp primitive is two bytes and carries
        // no destination coordinates, so the game lands the player on the tile they were
        // standing on in the map they came from, and paints them at that map's draw
        // corner (`crate::guardian::arrival_tile` and `draw_offsets` have the
        // disassembly). Every floor scene is an arena of its own, so without this the
        // player arrives somewhere nobody chose — often off the map entirely: not drawn,
        // unable to move, camera stuck.
        //
        // ★ PER FRAME, and it stops the moment it lands. The scene id is one pointer hop
        // and the window is only the second or so after a warp; checking once every 30
        // frames left up to half a second in which the player was being drawn on the last
        // floor and a direction key walked an off-map character around.
        if tower_land_frames > 0 {
            tower_land_frames -= 1;
            let scene = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::scene_now(&read)
            };
            // The way back first (2026-09-17): only once the ORIGIN scene has actually
            // arrived, and only into that map — `guardian_place_writes` refuses a tile
            // outside the map it finds, which is the second lock on that.
            if let Some(leg) = tower_return {
                let want = leg.scene;
                if scene == Some(u16::from(want)) {
                    match leg.tile {
                        // ★ 143차: a leg with no tile has landed the moment its scene shows.
                        None => {
                            log::warn!("guardian: back in scene {want} — no tile to stand the player on, the game's own spot stands");
                            tower_return = None;
                            tower_arrive = None;
                            tower_land_frames = 0;
                        }
                        Some(tile) => {
                            let plan = {
                                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                                items::guardian_place_writes(&read, tile)
                            };
                            if let Some((writes, from)) = plan {
                                let ok = writes.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
                                log::warn!(
                                    "guardian: back in scene {want} — the player stood at {from:?}, put back where the run started {tile:?} ({ok}/{} write(s))",
                                    writes.len()
                                );
                                tower_return = None;
                                tower_arrive = None;
                                tower_land_frames = 0;
                            }
                        }
                    }
                } else if frame.saturating_sub(leg.posted) >= RETURN_REPOST_FRAMES && leg.reposts < RETURN_REPOSTS {
                    // ★★ 143차 — THE WAY HOME IS RE-POSTED IF THE GAME ATE IT. The handler clears
                    // the request byte BEFORE it dispatches (`0x3f1d4`), so a byte back at 0 with
                    // the scene word still on the floor is a request that went nowhere — and
                    // nothing used to notice: the landing gave up after ten seconds and the
                    // player was left in a room with no door. A byte still at 1 is only
                    // waiting (a window is open): the clock restarts, nothing is written.
                    let (pending, in_floor_scene, plan) = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        (
                            items::warp_request_pending(&read),
                            scene.is_some_and(|s| tower.is_floor_scene(s)),
                            items::guardian_warp_writes(&read, want),
                        )
                    };
                    match (pending, in_floor_scene, plan) {
                        (Some(false), true, Some(writes)) => {
                            let ok = writes.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
                            log::warn!(
                                "guardian: the way home to scene {want} was consumed but the floor ({scene:?}) is still loaded — re-posted ({}/{RETURN_REPOSTS}, {ok}/{} write(s))",
                                leg.reposts + 1,
                                writes.len()
                            );
                            tower_return = Some(ReturnLeg { posted: frame, reposts: leg.reposts + 1, ..leg });
                            // The re-post gets a full landing window of its own.
                            tower_land_frames = tower_land_frames.max(crate::guardian::LANDING_FRAMES / 2);
                        }
                        (Some(true), _, _) => {
                            tower_return = Some(ReturnLeg { posted: frame, ..leg });
                        }
                        _ => {}
                    }
                }
            } else if let Some(want) = tower_arrive {
                if scene == Some(u16::from(want)) {
                    let plan = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        items::guardian_arrival_writes(&read)
                    };
                    if let Some((writes, from, to)) = plan {
                        let ok = writes.iter().filter(|(addr, bytes)| emulator.write_memory(*addr, bytes)).count();
                        // ⛔ LOUD. "My character disappeared" has to be answerable from one
                        // line of logcat; that is what 2026-09-11 cost when it was not.
                        log::warn!(
                            "guardian: scene {want} arrived — the warp left the player at {from:?}, placed at {to:?} ({ok}/{} write(s) ok)",
                            writes.len()
                        );
                        tower_arrive = None;
                        tower_land_frames = 0;
                    }
                }
            } else {
                tower_land_frames = 0;
            }
            // ⛔ FAIL LOUD. A placement that never found its scene means the player is
            // standing on the last map's tile, which is exactly the report this answers.
            if tower_land_frames == 0 && (tower_return.is_some() || tower_arrive.is_some()) {
                log::warn!(
                    "guardian: the warp never showed its scene within {} frame(s) (return {tower_return:?}, arrive {tower_arrive:?}) — the player stays where the game put them",
                    crate::guardian::LANDING_FRAMES
                );
                tower_return = None;
                tower_arrive = None;
            }
        }
        // ── NEW GAME PLUS: the boxes lose what the bag loses ────────────────────
        //
        // Fired once per run of the game's own transition (`ending_transition_lgt`'s
        // counter), on the first poll after it. ⛔ Not a rule about what a box may hold:
        // a 그리폰의피리 handed over by a quest afterwards sweeps back in and works exactly
        // as before (user: *"사라진 뒤 게임 중간에 퀘스트로 얻을땐 또 문제없이 잘 작동해야해"*).
        if frame % 5 == 3 {
            let (runs, difficulty_before) = wie_util::stats::ending_transition();
            if runs != ending_transitions_seen {
                ending_transitions_seen = runs;
                let taken = bagbox.new_game_plus();
                // ★ 167차 아바타 옷장: the clear's costume reward (owned bits in the save's flag
                //   array — memory only until the player saves, like the bag loan below).
                avatar.on_ending(&mut *emulator, &item_proxy, difficulty_before);
                log::warn!(
                    "bagbox: new game plus — the 2회차 transition ran (difficulty was {difficulty_before:?}), so the boxes gave up {} row(s)",
                    taken.len()
                );
                // ★★★ 헬 승격 (133차, user: "하드코어 클리어 이후 헬모드가 열리도록").
                // The game's transition has just written difficulty = 1; if the byte was
                // ALREADY non-zero when it started, this clear was hardcore's (or hell's
                // own — hell stays hell) and the next playthrough is hell. The write is
                // memory-only until the player saves, so reloading the pre-clear save
                // rolls it back — the same rule as the bag loan above.
                if crate::flavor::HELL
                    && crate::guardian::hell_promotion(difficulty_before).is_some()
                {
                    let plan = {
                        let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                        items::hell_difficulty_write(&read)
                    };
                    if let Some((addr, want)) = plan {
                        let ok = emulator.write_memory(addr, &[want]);
                        log::warn!(
                            "hellmode: a hardcore clear — difficulty {difficulty_before:?} -> {want} at {addr:#x} (write ok = {ok}); hell is open"
                        );
                        // ⛔ No notice here (140차): the ending window's own line already
                        // said 「헬모드로 재시작합니다」 — the `ending_label_lgt` hook picks
                        // that row when the pre-clear byte is non-zero. A second, host-drawn
                        // announcement after it was the "뭔가 안맞아" the user reported.
                    }
                }
            }
        }
        if frame % 30 == 11 {
            wie_util::stats::set_tower_no_drops(tower.current_floor() != 0);
            // Same rule for the species override and the spawn cap: only a floor being
            // entered may hold one. Both are armed on `Action::Warp` and this is what takes
            // them down again.
            if tower.current_floor() == 0 {
                wie_util::stats::set_tower_species(None);
                wie_util::stats::set_tower_spawn_cap(None);
            }
        }
        // ── 헬모드 몬스터 포크 (133차) ────────────────────────────────────────────
        //
        // In hell (difficulty byte == 2) every field scene's monsters are grown from
        // their hard-table spawn level (38..67) to `guardian::hell_level` (68..100) by the
        // game's own per-level ratios — the 112차 tower fork widened to the whole
        // world, per-creature instead of per-floor. Read-compare-write: a settled
        // scene costs one creature read and writes nothing.
        //
        // ⛔ NOT inside the tower: a run's own fork owns the records there (its ladder
        // already goes past 100), so this block stands down while one is going — and
        // its own frame slot (26) keeps it off the tower poll's tick (11).
        if frame % 30 == 26 && crate::flavor::HELL && tower.current_floor() == 0 && !tower.forking() {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            if items::shell_state(&read) || !crate::guardian::is_hell(items::difficulty_byte(&read)) {
                // Not hell (or not in the world): drop any stale snapshots so the next
                // hell scene starts from its own spawns.
                if hell_scene_was.is_some() {
                    hell_mem.clear();
                    hell_scene_was = None;
                    hell_said = None;
                }
            } else {
                let scene = items::scene_now(&read);
                if scene != hell_scene_was {
                    hell_mem.clear();
                    hell_scene_was = scene;
                }
                if let Some(creatures) = items::guardian_creatures(&read) {
                    // ★ 156차: the roll fields (명중 · 회피 · 기절 저항) are set against the
                    // PLAYER's level — read every poll, so a level-up re-targets them.
                    let plv = items::player_level(&read);
                    let forks = crate::guardian::hell_forks(&creatures, &mut hell_mem, plv);
                    if let Some(stats) = items::guardian_fork_writes(&read, &forks) {
                        if !stats.is_empty() && hell_said != scene {
                            hell_said = scene;
                            let top = forks.iter().max_by_key(|f| f.level);
                            let lv = top.and_then(|f| f.level);
                            log::warn!(
                                "hellmode: scene {scene:?} — {} write(s) across {} monster(s), top level {lv:?} vs player {plv:?}: hit {:?} dodge {:?} res {:?} (difficulty 2)",
                                stats.len(),
                                forks.len(),
                                top.and_then(|f| f.hit),
                                top.and_then(|f| f.dodge),
                                top.and_then(|f| f.res.map(|r| r as i16)),
                            );
                        }
                        for (addr, bytes) in &stats {
                            emulator.write_memory(*addr, bytes);
                        }
                    }
                }
            }
        }

        if frame % 5 == 0 {
            let (field, cutscene, choice, map_open, shell, shell_screen, can_save_raw, slots, stats, secondary) = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                let field = items::in_field_state(&read);
                // The game's own save-inhibitor byte: false marks a no-save AREA (boss
                // room), where the UI hides the SAVE button and the minimap.
                let can_save = items::can_save_here(&read);
                // ★ SCRIPT window (id 4) vs any other window: this is what makes
                //   "cutscene or menu?" answerable, and it is why a dialogue gets the
                //   OK + `#` pair instead of the menu-navigation set.
                let cutscene_now = items::cutscene_open(&read);
                // ★ …and the one bit that window id CANNOT give: is this script window
                //   showing a CHOICE the player has to answer? (2026-08-17 — the arrows
                //   come back for that and only that.) See `items::script_choice_open`.
                let choice = items::script_choice_open(&read);
                // ★ STICKY (2026-08-20). Two player reports, one cause: "컷신에서
                // 대화중간중간에 화살표와 OK버튼이 한번씩 깜빡깜빡 뜬다" and "컷신에
                // 들어가면 1,3,#,7,9 버튼이 뜨고 우측에 CLR, OK버튼이 뜬다".
                //
                // The mode is chosen from `(field, cutscene)` and falls through to
                // MENU when both are false — and the script window's id (`CHAR+0xa0c0`)
                // does not stay 4 for the whole of a scene: it drops to 3 between text
                // pages. This poll runs every fifth frame, so each of those blips
                // becomes a full control-set swap on screen: arrows, OK, and the
                // 1/3/7/9/# menu block, for a few frames.
                //
                // Holding the flag for a moment after it was last seen absorbs the
                // blips without hiding a real exit.
                //
                // ★ 2026-08-21 — the FIRST version of this hold did not work, and the
                // reason is worth keeping. It was a wall-clock deadline with a fast
                // cancel: `else if field { cutscene_until = 0 }`. But `in_field_state`
                // is only true with NO window open, so the moment a page turn passes
                // through "no window" the cancel fired and **threw the hold away** —
                // and the very next poll, with the window already reopening (id 3), was
                // neither field nor script, which is the definition of MENU. That is
                // exactly the control set the player saw flash:
                // "좌측 화살표(메뉴 오픈했을 때 나오는 화살표)가 … 깜빡깜빡".
                // Counting consecutive polls instead removes the cancel entirely: a
                // blip cannot expire the hold because it is not long, it is short.
                // ★ 66차-B: the hold is cancelled the moment ANOTHER window is up. See
                // `items::cutscene_hold_gated` — without it every window opened from a
                // dialogue (상점 · 제련 · 창고) began with ~0.5 s of "a tap anywhere is
                // OK", which is what the player reported.
                let cutscene = items::cutscene_hold_gated(
                    &mut cutscene_gone,
                    cutscene_now,
                    items::other_window_open(&read),
                    CUTSCENE_EXIT_POLLS,
                );
                // ★ …and the same filter on the choice flag. Player, 2026-08-21:
                // "대화1 -> 대화2로 넘어갈 때 그 사이 찰나의 순간에 떠서 깜빡깜빡 뜨는
                // 느낌이야". Two things can raise those arrows — falling through to
                // MENU (which the counter above now prevents) and this flag — so both
                // are filtered, because a single poll cannot tell a page turn from a
                // state the player is meant to act on.
                let choice = items::choice_hold(&mut choice_seen, choice, CHOICE_CONFIRM_POLLS);
                let map_open = items::map_window_open(&read);
                let shell = items::shell_state(&read);
                // ★★ Pin the save slot while the game is IN PLAY (2026-08-25). The
                // `_<slot>` suffix every `writeSaveRecord` appends is the slot-list
                // CURSOR (`CHAR+0xa410`), and the only code that moves it lives in the
                // title/network shell screens — so in play it IS the loaded slot, and
                // latching it here gives `savevault::write_verdict` the one fact it needs
                // when the network shell later re-aims it.
                //
                // ★ 69차-C: do NOT blindly adopt the cursor on every in-play poll.
                // Walking the network slot screen leaves the aim there (CLR → 0,
                // ↑↓ wrap 0..2) and the character stays loaded. Coming back to the
                // field would then re-pin the drifted value and the next save would
                // be allowed onto that slot. `note_live_character` keeps the pin
                // when the name at CHAR+0xb8 is unchanged, and still adopts when
                // a different character was loaded from the title screen.
                let cursor = items::live_save_slot(&read);
                let identity = items::live_char_identity(&read).unwrap_or(0);
                let warehouse = crate::network::warehouse_open();
                // ⛔ 71차-L: the warehouse OPEN edge no longer pins anything. It used to
                // commit the slot the player had executed on the network slot screen —
                // which the guest disassembly shows is the row being BACKED UP (`04 3d`),
                // not the live character's home. See the tombstone on
                // `savevault::execute_pin_slot`.
                // ⛔ 136차: and the CLOSE edge no longer locks the slot list. 71차-K froze it
                // "until the disconnect save finishes" — there is no such save (probe,
                // both CLRs out of the warehouse: zero record writes), so the lock only
                // ever fell to its timeout while 「저장하고 있습니다」 announced nothing.
                if !warehouse && was_warehouse && shell {
                    log::info!("save slot: back from the warehouse — nothing to wait for (the game does not save on the way out)");
                }
                was_warehouse = warehouse;
                // ★★★ 71차-J — pin the LOAD, not the live cursor. vc177 on S25:
                // the shell never printed `pinning slot` (`resolve_char` was blind),
                // the first in-play poll adopted cursor 1, then REDIRECT wrote
                // GameSave_11_0's bytes onto GameSave_11_1.
                // `load_pin_slot` is Some only when the identity just appeared or
                // changed, and the slot it names is the cursor FROM THE PREVIOUS
                // poll — the word `loadSaveRecord` actually read.
                // ★ 71차-K: a name appearing on an EXISTING pin is not a load —
                // that is vc178 (`already_pinned && prev_id == 0`). Every pin that
                // can exist here now carries an identity (71차-L), so the guard only
                // absorbs a poll where CHAR was briefly unreadable.
                // ★★★ 76차 — THE THIRD LOAD-EQUIVALENT: a character CREATED in the slot the
                // cursor names. Neither load observer fires for it (the record read FAILS —
                // that is what opens the creation screens), and with a same-class character
                // loaded earlier in the process `pin_holds` kept the OLD pin, so the first
                // save was REDIRECTED onto the old slot (2026-09-03: 슬롯 3 생성 → 슬롯 1
                // 덮임). The shell's screen word stays 10/11/12 once the world is entered
                // (nothing in state 4/5 writes it — probe: 12 for 2,400 frames), so
                // "in the world + creation screen" means "created in slot `cursor`".
                // `note_slot_loaded` is idempotent, so this may run every poll.
                // ⛔ Not while `shell`: ON the creation screens the player can still back
                // out with the old character resident — see `savevault::creation_slot`.
                // ★★★ 78차 — TWO GUARDS THIS ARM DID NOT HAVE, both from the 2026-09-03
                // report «1슬롯으로 반복 플레이했는데 2슬롯이 1슬롯의 세이브로 덮였다»:
                //   ⓐ only on the shell → world EDGE. 76차 ran this every poll, which turns a
                //     screen word that is never reset (see `savevault::creation_slot`) into a
                //     pin source that tracks the raw slot cursor for the whole session — the
                //     one thing 67차..71차 forbid. Entering the world is the moment the fix is
                //     actually about, and a later 창고 → 뒤로 return is another edge whose
                //     screen word is the warehouse's (3), not a creation screen.
                //   ⓑ only onto a slot that holds NO character. A created character's slot is
                //     empty by construction; an occupied one is somebody else's save.
                let screen = items::shell_screen(&read);
                let entered_world = !shell && was_shell;
                if entered_world
                    && crate::savevault::on_creation_screen(screen)
                    && let Some(c) = cursor
                    && let Some(c) = crate::savevault::creation_slot(screen, Some(c), crate::vault::slot_has_save(&data_dir, c))
                {
                    crate::vault::note_slot_loaded(c, "entered the world from the character-creation screens");
                    // ★★★ 2026-09-16 — A CREATION RECYCLES THE SLOT, so anything the host
                    //   keeps keyed to that slot belonged to the previous occupant and must
                    //   NOT be inherited. Player report: a character deleted and remade in
                    //   the same slot came up with the dead one's quickslot still bound and
                    //   its 소비가방 potions still in the box (5 new + 4 old = 9).
                    //   ⛔ The delete hook (`crate::note_slot_reset`) is NOT enough on its
                    //   own — that session's log has no `DB delete_database` line at all.
                    //   This edge needs no delete event: `creation_slot` already proved the
                    //   slot held NO save, so there is nothing legitimate to lose here.
                    //   ⛔ Must run BEFORE `adopt_owner` below, which is what would hand the
                    //   dead character's row to the new one — it is in this same block, and
                    //   this arm comes first.
                    if pages.forget_slot(c) {
                        qb_store_dirty = true;
                        log::warn!("quickbar: slot {c} got a NEW character — the stored plate row of the slot's previous occupant is dropped");
                    }
                    created_slot = Some(c);
                }
                was_shell = shell;
                // ★★★ 148차: after an observed LOAD the previous poll described the OUTGOING
                //   world, and its cursor is not evidence about who is loaded now — the load's
                //   own slot (the pin `note_slot_loaded` just set) is. vc420 took slot 2's
                //   world cursor for slot 1's character, and `pin_holds` then defended it.
                let evidence = crate::savevault::pin_evidence_cursor(
                    pin_prev_cursor,
                    std::mem::take(&mut loaded_since_poll).then(crate::vault::live_slot).flatten(),
                );
                if let Some(slot) = crate::savevault::load_pin_slot(cursor, identity, evidence, pin_prev_id, crate::vault::live_slot().is_some())
                {
                    if shell {
                        crate::vault::note_shell_character(slot, identity);
                    } else {
                        crate::vault::note_live_character(slot, identity);
                    }
                } else if !shell && crate::vault::live_slot().is_some() {
                    // Same character, in play: let `pin_holds` keep the pin if the
                    // cursor has drifted. ⛔ Do not call this when there is no pin
                    // — that is exactly how vc177 adopted the drifted cursor.
                    // ★ 71차-L: an unreadable identity now HOLDS the pin here instead
                    // of re-adopting the cursor (`pin_holds`), because an unproven pin
                    // no longer authorises anything.
                    if let Some(c) = cursor {
                        crate::vault::note_live_character(c, identity);
                    }
                }
                pin_prev_cursor = cursor;
                pin_prev_id = identity;
                // ★ 70차: the OBSERVATION is published on every poll, shell included —
                // that is not a violation of the 67차 "no latch in the shell" rule but
                // its complement: the redirect needs to see the DRIFTED cursor next to
                // the unchanged identity, which is exactly the pair the latch above
                // refuses to adopt. `vault::write_check` uses it to land a
                // drifted-cursor save on the pinned slot instead of refusing it, which
                // is what makes the warehouse's own saveGame calls save again.
                crate::vault::note_cursor_observation(cursor, identity);
                crate::vault::note_shell_screen(screen);
                // The shell -> world edge: the game just loaded a save or started a
                // new one. Handled on the next SCAN poll rather than here, because the
                // sync needs a readable character and the first frame out of the shell
                // does not guarantee one.
                if last_shell && !shell {
                    qb_sync = true;
                    qb_sync_wait = QB_SYNC_WAIT;
                }
                // ★★ …and the OTHER half of that edge: on the TITLE / SLOT shell the guest's
                // five-slot bar is about to be rewritten from a save, so the window that says
                // which plate sits in which of those slots is stale. Nothing used to clear it —
                // `Window::new()` ran once per process — so after the first load `residents()`
                // was never empty again and the `mine` arm below always took the "back from an
                // online screen" shortcut. A second 게임시작 into the same character then had
                // its freshly deserialised bar CAPTURED under the old mapping, filing (say)
                // guest slot 0's binding onto plate 5.
                //
                // ⛔ The discriminator is the SHELL SCREEN word, not `items::online_screen`:
                // state 3 is the online screens AND the title/slot shell (66차-C), and on the
                // 게임시작 slot list the previous character is still resolvable — which is the
                // very case this has to catch. Screen 3 IS 창고/캐시샵, and only that keeps the
                // residency (the bar is untouched there). A build with no anchor for the word
                // reads `None` and clears, which is the old unconditional behaviour.
                // ★ 148차: `desync`, not just a clear — until the sync nothing may write the
                //   row from live state (the load itself desyncs too, right after the tick;
                //   this is the half for the slow road through the title).
                if shell && screen != Some(items::SHELL_SCREEN_ONLINE) {
                    pages.desync(&mut qb_window);
                }
                // ★ The hidden-plate set, into the store, before anything below reads it
                // (2026-09-10). `warm_plan` skips hidden plates so they stop consuming
                // one of the five guest slots, and `adopt_guest_bar` hands those slots to
                // the VISIBLE plates first — both of them run in this block. Cheap: a
                // relaxed atomic load and a byte store.
                pages.set_hidden(crate::quick_hidden());
                // ── the host's PLATE row (see `quickpage.rs`) ─────────────────────
                //
                // ★ Scanned for the whole of app state 4, not just field free-roam:
                //   the plates stay on screen with a game window open (2026-08-03), so
                //   an empty list there would blank them the moment a menu opened.
                //   Boot and the title shell are still excluded — CHAR is not a live
                //   character there.
                //
                // ★★ 2026-09-04, and the ORDER is the point. There are more plates than
                // the guest has slots, so the guest's bar is a WINDOW onto the store and
                // the store is what the row is drawn from:
                //
                //   1. SYNC, once per character load. Leaving the shell means the game
                //      rewrote its five-slot bar from a save we cannot audit, and after
                //      an app restart we do not even know which plate each of those five
                //      belongs to. If the store belongs to this character (`Owner`), the
                //      STORE wins: clear the bar and warm it from plates 0.. — that both
                //      re-establishes the mapping and is the only thing that can keep
                //      plates past the fifth, which have no guest storage at all. If it
                //      belongs to someone else (or to nobody — a file written before the
                //      owner line existed), the GUEST wins: reset the store and adopt the
                //      bar as plates 0..4, which is exactly what every build before this
                //      one did unconditionally.
                //   2. CAPTURE, every scan. Refreshes the resident plates from the live
                //      bar, so the store never lags the game — an item used up, or the
                //      party-transition sweep, empties its plate here.
                //   3. RENDER, from the store, so the plates the guest is not currently
                //      holding still draw.
                let scan = frame % 15 == 0;
                let slots = if !items::in_game_state(&read) {
                    Vec::new()
                } else if scan {
                    let class_of = |cat: u8| items::class_of(&read, cat);
                    let item_at = |tab: u8, s: u8| items::item_at(&read, tab, s);
                    // ⛔ Both halves of the identity have to be READABLE before the
                    // ownership question can be asked. `identity` is 0 when the class
                    // name at `CHAR+0xb8` has not been written yet (71차-L), and
                    // comparing 0 against a stored owner reads as "different character"
                    // — which would reset a store that in fact belongs to the player
                    // sitting in front of it. Wait instead; `QB_SYNC_WAIT` bounds the
                    // wait so an identity that never arrives still ends in a decision
                    // (the guest's own bar, which is always safe).
                    // ★★ 148차: an unsynced row in the world IS a sync owed — whatever missed
                    //   the edge (a load the 5-frame poll never saw the shell for), the scan
                    //   does not leave the plates refusing every press for the session.
                    if !pages.synced() && !qb_sync {
                        qb_sync = true;
                        qb_sync_wait = QB_SYNC_WAIT;
                        log::info!("quickbar: the plate row is not synced to this character — syncing now");
                    }
                    let ready = class_of(0).is_some() && (identity != 0 || qb_sync_wait == 0);
                    let mut warmed = false;
                    if qb_sync && !ready {
                        qb_sync_wait = qb_sync_wait.saturating_sub(1);
                    } else if qb_sync {
                        // ★★ The PIN, not the cursor. `items::live_save_slot` reads
                        // `CHAR+0xa410` raw, and 67차 wrote down what that word is: the slot
                        // list's CURSOR, which the online menu sets to 0 on entry
                        // (`0xa876`/`0xa88a`). Building the owner key from it meant a slot-2
                        // character came back from the 창고 keyed `(0, 리츠)`, failed the
                        // `mine` test against its own stored `(2, 리츠)`, and took the
                        // "different character" arm below — which `reset()`s the store. Plates
                        // 5..6 live NOWHERE ELSE, so that is a silent, permanent loss, and the
                        // wrong key was then written back so the next run mismatched again.
                        // `vault::live_slot()` is the stabilised answer the save guard already
                        // maintains (67차~71차-M): it moves only on an observed character LOAD.
                        let pin = crate::vault::live_slot();
                        let owner = quickpage::Owner {
                            slot: pin.unwrap_or(u8::MAX),
                            identity,
                        };
                        // ★★★ 2026-09-04 (second pass) — ADOPT, never RESET. Making the
                        // reset conditional on the owner was half the fix: the store still
                        // held ONE row, so loading somebody else threw this character's row
                        // away to make room. Plates 5..6 live nowhere else, so that is a
                        // permanent loss, and the player found it immediately — "다른 슬롯
                        // 1, 3에 들어갔다 오면 6,7이 해제되어있어". `adopt_owner` parks the
                        // outgoing row instead and hands back the incoming one if it has
                        // seen it before. ⛔ Do not put `pages.reset()` back here.
                        //
                        // An unreadable identity is passed as `None`, NOT as an owner with
                        // identity 0: a key we cannot verify must not claim a row, and two
                        // different unnamed characters must not collide on one.
                        let mine = pages.adopt_owner((identity != 0).then_some(owner));
                        if mine && !qb_window.residents().is_empty() {
                            // ★ Not every shell -> world edge is a character load. App
                            //   state 3 is the ONLINE screens too (창고 · 캐시샵 —
                            //   66차-C), and coming back from those changes nothing:
                            //   the window still describes the bar correctly. Re-warming
                            //   would rewrite five guest slots for no reason and shuffle
                            //   the arrangement back to plate order under the player.
                            log::info!("quickbar: back from an online screen — the plate row is already in sync");
                        } else if mine {
                            // Same character: the STORE wins. Clearing the bar and
                            // warming it from the plate row is what re-establishes which
                            // plate each guest slot belongs to — after an app restart
                            // the guest's five bytes are the right bindings in an
                            // unknown arrangement — and it is the only thing that can
                            // bring back the plates past the fifth.
                            let (mut actions, dropped) = pages.warm_plan(&mut qb_window, &class_of, &item_at);
                            for d in &dropped {
                                log::info!("quickbar: dropped an entry restoring the plate row: {d:?}");
                            }
                            // ★★ 148차: the store wins, but it may not FORGET — a skill the
                            //   freshly loaded bar holds that the row holds nowhere goes onto an
                            //   empty plate instead of being cleared (`salvage_guest_skills`).
                            //   Read now, before any of these writes lands. It is also how a row
                            //   the vc420 wipe emptied gets its plates back from the save.
                            let salvaged = pages.salvage_guest_skills(&items::quickbar_identities(&read), &class_of);
                            if salvaged > 0 {
                                actions = pages.warm_plan(&mut qb_window, &class_of, &item_at).0;
                                log::warn!("quickbar: {salvaged} skill(s) the save's bar holds were missing from the plate row — put back on empty plates");
                            }
                            log::info!(
                                "quickbar: same character (slot {pin:?}) — restoring the stored plate row ({} action(s)); {} other row(s) parked",
                                actions.len(),
                                pages.parked()
                            );
                            qb_restore = Some(actions);
                            // ⛔ And NO capture this scan: `live` was read before those
                            // writes land, so capturing now would file the OUTGOING
                            // bindings under the incoming plates.
                            warmed = true;
                        } else {
                            // A character this store has never seen — or none we can
                            // name. Its stamps would belong to somebody else's playthrough
                            // (agreed policy 2026-08-04), so the GUEST wins: this row
                            // starts empty and adopts the bar the save just restored as
                            // plates 0..4. `adopt_owner` has already parked whoever was
                            // active, so nothing was destroyed to get here.
                            pages.adopt_guest_bar(&mut qb_window);
                            let _ = item_proxy.send_event(UserEvent::QuickPage(0));
                            log::info!(
                                "quickbar: a new character (slot {pin:?}, id {identity:#x}) — plate row adopted from the guest's own bar; {} other row(s) parked",
                                pages.parked()
                            );
                        }
                        qb_sync = false;
                        qb_store_dirty = true;
                    }
                    if !warmed {
                        let live = items::quickbar_identities(&read);
                        // A plate whose binding changed (the player registered one, used
                        // up a stack, or the party sweep cleared it) is a store that has
                        // to reach disk: plates past the guest's five exist nowhere else.
                        qb_store_dirty |= pages.capture(&mut qb_window, &live, &class_of, &item_at);
                    }
                    items::plate_views(&read, pages.plates(), &qb_window, &|cat, idx| bagbox.count_of(u16::from(cat), u16::from(idx)))
                } else {
                    last_slots.clone()
                };
                // Equipment only changes in a menu, so this is cheap enough here.
                let secondary = if field { items::secondary_weapon_icon(&read) } else { None };
                (
                    field,
                    cutscene,
                    choice,
                    map_open,
                    shell,
                    // ★ 80차-P — WHICH shell screen, for the UI's slot-list arm: `shell`
                    // alone cannot tell the two slot lists from the 창고/캐시샵 grids.
                    screen,
                    can_save,
                    slots,
                    items::read_stats(&read).unwrap_or(last_stats),
                    secondary,
                )
            };
            // The plate row's guest writes, out here where the emulator can be borrowed
            // mutably again. `warm_plan` produced them from the STORE, so this is the
            // same five clears + up to five binds a page flip has always done.
            if let Some(actions) = qb_restore.take() {
                apply_quickpage_actions(&mut *emulator, &actions);
            }
            // ★ 초기화 removed a character (2026-09-15 report — a same-class successor in
            //   that slot came up with the deleted one's skill quickslots): its plate row
            //   goes with it, active or parked. The note is left by the DB repository
            //   inside the guest's tick; taking it here keeps the store on one thread.
            // The bag boxes of a slot that just got a new character (the plate row was
            // already dropped inside the read block, before `adopt_owner` could hand it over).
            if let Some(c) = created_slot.take() {
                bagbox.forget_created_slot(&data_dir, c);
                // ★ 138차: …and its 하늘성 progress: the new occupant starts at floor 1.
                if let Some(root) = crate::vault::app_root(&data_dir) {
                    config::forget_tower_progress(&root, c);
                }
                if tower_prog_slot == Some(c) {
                    tower_prog = crate::guardian::Progress::default();
                    tower_prog_slot = None;
                }
            }
            for slot in crate::take_slot_resets() {
                // ★ 138차: the repository already removed the progress FILE; the copy in
                //   memory is dropped here so the next read sees the empty slot.
                if tower_prog_slot == Some(slot) {
                    tower_prog = crate::guardian::Progress::default();
                    tower_prog_slot = None;
                }
                if pages.forget_slot(slot) {
                    qb_store_dirty = true;
                    log::warn!("quickbar: slot {slot} was reset (초기화) — its plate row is gone with it");
                } else {
                    log::info!("quickbar: slot {slot} was reset (초기화) — no plate row was keyed to it");
                }
            }
            if std::mem::take(&mut qb_store_dirty) {
                config::write_quickpages(&data_dir, &pages);
            }
            // ★ DEBOUNCED (2026-08-08, player: "갑자기 미니맵이랑 save 버튼이 없어졌어 ... 잠시동안
            // ... 보스방에서만 없어져야 하는데"). 27차 hides the SAVE button AND the minimap the
            // instant the game's save-inhibitor byte reads non-zero, and that byte is reached
            // through a resolved CHAR pointer — a single sample taken mid-transition is enough
            // to blank both controls on an ordinary field.
            //
            // ASYMMETRIC on purpose: hiding waits for [`NO_SAVE_DEBOUNCE`] consecutive samples,
            // showing again is immediate. A boss room lasts minutes, so a ~2 s delay before the
            // controls go is invisible; the flicker it removes is what the player actually saw.
            // This does NOT decide whether the byte is the right signal — the `save gate:` log
            // below is what will answer that, with a place and a time.
            if can_save_raw {
                no_save_run = 0;
            } else {
                no_save_run = no_save_run.saturating_add(1);
            }
            // `HIDE_UI_ON_NO_SAVE` is false this round, so the AREA byte hides nothing and
            // the SAVE button and the minimap stay on screen everywhere. See the constant.
            //
            // ★★ EXCEPT INSIDE THE TOWER (user decision 2026-09-08: "탑에서만 숨긴다").
            // §D.4 assumed 27차's hiding came for free with the no-save fork; it does not,
            // because that switch is off — device-measured on S9 with vc279, where the
            // fork moved `area gate: can_save` to `Some(false)` and both controls stayed
            // put. The gate is therefore the RUN, not the byte:
            //   * a live run is the one no-save state this build has actually measured, so
            //     it is the one place the hiding cannot fire on an ordinary field, and
            //   * outside a run this expression is byte for byte what it was, which keeps
            //     the 2026-08-09 stand-down (and the 2026-08-08 report behind it) intact.
            // ⛔ Do not "simplify" this into `HIDE_UI_ON_NO_SAVE = true` — that trusts the
            // byte on every shipped map, which is the untested step the stand-down exists
            // to avoid.
            // ★ 143차: a FLOOR SCENE with no run counts too — a save made inside the tower
            // before 143차 loads the player into an arena with nothing going, and SAVE there
            // is the soft-lock itself (*"하늘성안에서 세이브버튼이나오고 그걸 누르면
            // 나올수가없어"*). And the wake: its veil is up and the player is being carried.
            let floor_scene = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::scene_now(&read).is_some_and(|s| tower.is_floor_scene(s))
            };
            let in_tower = tower.forking() || tower.current_floor() != 0 || floor_scene || wake.is_some();
            TOWER_NO_SAVE.store(in_tower, std::sync::atomic::Ordering::Relaxed);
            let can_save = !in_tower && (!HIDE_UI_ON_NO_SAVE || can_save_raw || no_save_run < NO_SAVE_DEBOUNCE);
            // ★ 143차: the EXIT tab — wherever the player is in the tower and could walk
            // out: not while the wake is carrying them, and not while a way home is
            // already in flight (the tab would ask a question already answered).
            let tower_exit = crate::flavor::TOWER
                && (tower.current_floor() != 0 || floor_scene)
                && wake.is_none()
                && tower_return.is_none()
                && !matches!(tower.state, crate::guardian::State::Failed { .. });

            // The live buff readout the menu HUD shows. Minute-granular, so this
            // only triggers a publish when a buff starts, ends, or ticks a minute.
            let mut buff_lines = buffs.status_lines(now_ms());
            // ★ 142차: the worn 85제 sets (and 수호의 맹세's cooldown) join the menu's
            // buff lines; the invulnerability itself joins the FIELD line below.
            let set_mask = wie_util::stats::set_bonus();
            // ★ 150차: the two TIMED accessory buffs (레아의목걸이 · 굴베이그의탈리스만) join
            // the same lines — the percents off the published effects, the clocks the hooks
            // stamp (the kill clock is SHARED with 마녀의 생명력: one trigger, one 20 s).
            let acc = wie_util::stats::acc_effects();
            let clocks = crate::setbonus::Clocks {
                now_ms: play_ms,
                guard_until_ms: wie_util::stats::GUARD_UNTIL_MS.load(std::sync::atomic::Ordering::Relaxed),
                guard_ready_ms: wie_util::stats::GUARD_READY_MS.load(std::sync::atomic::Ordering::Relaxed),
                kill_until_ms: wie_util::stats::WITCH_UNTIL_MS.load(std::sync::atomic::Ordering::Relaxed),
                hit_until_ms: wie_util::stats::HIT_UNTIL_MS.load(std::sync::atomic::Ordering::Relaxed),
            };
            buff_lines.extend(crate::setbonus::menu_lines(set_mask, acc.kill_max_hp_pct, acc.hit_max_hp_pct, clocks));
            // ★ Its OWN field, not a buff line: buff lines are MENU-only on purpose
            // ("on the field those pixels belong to the game world"), and the one place
            // this line has to be visible is the field. `None` when no set is live.
            let field_line = crate::setbonus::field_line(set_mask, acc.kill_max_hp_pct, acc.hit_max_hp_pct, clocks);
            // ★ 145차: the 하늘성 floor and count are no longer a line under the bars —
            // the UI draws them at the top centre in the damage digits (user 2026-09-30,
            // *"중앙 상단 … 데미지 폰트에 색상만 다르게"*). `None` outside a run.
            let tower_hud = tower.hud(tower_alive);
            // ★ 141차 · 153차: the ring's picture — whatever the keeper holds for the worn
            // look (it is fed after every tick, above). The title is a suspicion (the next
            // character may be someone else); the field with a known look settles it.
            let portrait = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                if shell {
                    portrait_keeper.suspect();
                } else if field && let Some(look) = items::portrait_look(&read) {
                    portrait_observe_log(look, portrait_keeper.observe(look));
                    if portrait_keeper.take_memory_dirty() {
                        save_portrait_memory(&portrait_memory_path, portrait_fp, &portrait_keeper);
                    }
                }
                portrait_keeper.shown().cloned()
            };
            // ★ 136차: where the 「정렬」 button goes, or nowhere. Read fresh every poll —
            // it is a screen fact, and the worker re-decides on the tap anyway.
            let sort_screen = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::sort_screen(&read)
            };
            // Always publish the FIRST sample, even when it equals our own initial
            // values: the UI cannot distinguish "nothing sent yet" from "nothing
            // changed", and both sides starting at a guessed default is how the
            // field HUD ended up drawn over the title screen.
            if !hud_published
                || map_open != last_map_open
                || shell != last_shell
                || shell_screen != last_shell_screen
                || field != last_field
                || cutscene != last_cutscene
                || choice != last_choice
                || can_save != last_can_save
                || slots != last_slots
                || stats != last_stats
                || secondary != last_secondary
                || buff_lines != last_buff_lines
                || field_line != last_field_line
                || tower_hud != last_tower_hud
                || tower_flash != last_tower_flash
                || tower_exit != last_tower_exit
                || sort_screen != last_sort_screen
                || portrait != last_portrait
            {
                hud_published = true;
                // ★ Leaving the out-of-world SHELL (title / slot select) means the game
                // just loaded a save or started a new one, i.e. it rewrote the quickbar
                // from a source we cannot audit. Any shadow page still held is now stamped
                // with someone else's playthrough, so drop the lot and go back to page 0 —
                // which IS the guest's own bar, so the player loses only their page-2
                // arrangement. The alternative is silently re-binding skills from a
                // different save (agreed policy 2026-08-04).
                // ★ 2026-08-08, player: "갑자기 미니맵이랑 save 버튼이 없어졌어 ... 보스방에서만
                // 없어져야 하는데 왜 필드에서?". 27차 uses the game's own save-inhibitor byte as
                // a stand-in for "boss room", and this says when that byte actually moves, so
                // the next report comes with a place and a time instead of a guess.
                if can_save != last_can_save {
                    log::warn!(
                        "save gate: canSave {last_can_save} -> {can_save} (raw={can_save_raw}, run={no_save_run}, field={field}, map_open={map_open}, cutscene={cutscene}, tower={in_tower})"
                    );
                }
                last_field = field;
                last_cutscene = cutscene;
                last_choice = choice;
                last_map_open = map_open;
                last_shell = shell;
                last_shell_screen = shell_screen;
                last_can_save = can_save;
                last_slots = slots.clone();
                last_stats = stats;
                last_secondary = secondary;
                last_buff_lines = buff_lines.clone();
                last_field_line = field_line.clone();
                last_tower_hud = tower_hud;
                last_tower_flash = tower_flash;
                last_tower_exit = tower_exit;
                last_sort_screen = sort_screen;
                last_portrait = portrait.clone();
                let _ = item_proxy.send_event(UserEvent::Hud {
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
                });
            }
        }

        // 4c2) The game's own context popup -> "퀵바등록".
        //
        //  ★ EVERY FRAME, not the HUD's ~12/s (player, 2026-08-06: "상세보기/1번등록/
        //  3번등록 박스가 한번 깜빡했다가 퀵슬롯등록 박스로 바뀐다"). The guest repaints the
        //  popup on every frame it is up, so the player sees the ORIGINAL list for as
        //  long as we take to rewrite it — at 5-frame polling that is 80-120 ms, which
        //  is exactly the flash being reported. One frame is the floor: the popup is
        //  composed inside the same tick that opens it, so the first repaint after our
        //  write is the earliest one that can be ours.
        //
        //  It is affordable because `popup_view` bails after ~3 reads when no menu
        //  overlay is up (`CHAR+0x9c6c == 0`), which is every frame of normal play.
        //
        //  The REWRITE is re-applied every sample rather than once on the open edge:
        //  the count byte `CHAR+0x9c85` is shared with other windows, so something else
        //  can put it back. `items::popup_relabel` returns nothing when the list is
        //  already ours, which is what makes that cheap.
        //
        //  ★ This writes into the guest's own label buffers, not over its pixels. The
        //    popup's screen anchor is NOT readable (draw_popup keeps x/y in its stack
        //    frame), so drawing over it is not an option — see items.rs.
        if POPUP_QUICKBIND {
            let view = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::popup_view(&read)
            };
            if let Some(v) = view.as_ref() {
                let writes = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::popup_relabel(&read, v)
                };
                for (a, bytes) in &writes {
                    if !emulator.write_memory(*a, bytes) {
                        log::warn!("popup: relabel write failed at {a:#x}");
                    }
                }
                // ★ Only on the OPEN EDGE. This block now runs every frame, and 18차
                // measured what an unconditional log costs here: an `__android_log_write`
                // inside the frame budget, ~100/s, is a performance bug of its own.
                if !writes.is_empty() && last_popup.is_none() {
                    log::info!("popup: relabelled to 퀵슬롯등록 ({} write(s), kind {})", writes.len(), v.kind);
                }
            } else if picking {
                // The popup closed under us while the plates were highlighted.
                picking = false;
                let _ = item_proxy.send_event(UserEvent::PickingSlot(false));
            }
            if view != last_popup {
                last_popup = view;
                let _ = item_proxy.send_event(UserEvent::Popup(last_popup));
            }
            // ★ 79차-C: and the ROW COUNT + highlight for popups of every flavour, which
            // is what lets the UI find the box in the frame and tell a focus tap from a
            // confirm tap. Same cadence, same change-only publish; three byte reads.
            let list = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::popup_list(&read)
            };
            if list != last_popup_list {
                last_popup_list = list;
                let _ = item_proxy.send_event(UserEvent::PopupList(list));
            }
        }

        // 4b-1) 제련 확률 (71차-I, player: "제련시 제련확률을 표시해주고싶어 … 제련석을
        //     올릴때마다 확률도 마찬가지로 계산되어서 최신화 되어야해").
        //
        //     EVERY FRAME, and for the same reason the popup rewrite is: the thing being
        //     watched is a byte the player is changing with a held key. The HUD's ~12/s
        //     would lag a spinner that auto-repeats at 10/s by up to a whole step, and
        //     the number is the entire feature.
        //
        //     It is affordable for the same reason too — `items::refine_odds` bails after
        //     TWO reads (`resolve_char` is a constant on LGT, then the window id) on every
        //     frame the smith's window is not open, which is every frame of normal play.
        //
        //     ★ READ-ONLY. Unlike the two blocks around it this writes no guest byte and
        //     synthesises no key; the whole feature is a number drawn over the game.
        let refine = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            items::refine_odds(&read)
        };
        if refine != last_refine {
            // The OPEN EDGE only, for the reason 18차 gives above: an
            // `__android_log_write` per frame is its own performance bug, and this
            // changes on every arrow press.
            if last_refine.is_none() {
                log::info!("refine: odds readout up — {refine:?}");
            }
            last_refine = refine;
            let _ = item_proxy.send_event(UserEvent::RefineOdds(refine));
        }

        // 4b-1b) ★ 150차 — the 상세보기 (item detail) NOTE (`itemnote.rs`).
        //
        //     EVERY FRAME, for the refine readout's reason plus one: the window's first frame
        //     goes out INSIDE the tick that opens it, so a note built only after page 7 is
        //     seen would always land a frame late and the game's own body — 「기절 저항 +0」
        //     — would flash first. So the note follows the CURSOR already (the bag cell on
        //     page 2, the candidate on the 장비 page) and is waiting when page 7 opens;
        //     the UI draws it only when its frame shows that window for that item.
        //
        //     Affordable: `item_note_target` bails after two reads (the menu flag) on every
        //     frame of normal play; with a menu page up it re-reads the target and the nine
        //     worn slots, and rebuilds only when one of them moved.
        //
        //     ★ READ-ONLY: no guest byte is written and no key is synthesised.
        {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            let basis = items::item_note_target(&read).map(|t| (t, items::worn_items(&read)));
            if basis != note_basis {
                let built = basis.as_ref().and_then(|(t, worn)| items::item_note(&read, t, worn).map(|n| (t.identity(), n)));
                if built != last_note {
                    let item = built.as_ref().map_or(0, |(id, _)| *id);
                    let note = built.as_ref().map(|(_, n)| Box::new(n.clone()));
                    let _ = item_proxy.send_event(UserEvent::ItemNote { item, note });
                    last_note = built;
                }
                // ★ ONE line per window OPENED (18차's rule for the per-frame blocks): what the
                // overlay was asked to draw there, or that this item gets nothing.
                let was_open = note_basis.as_ref().is_some_and(|(t, _)| t.open);
                if let Some((t, _)) = basis.as_ref()
                    && t.open
                    && !was_open
                {
                    match last_note.as_ref() {
                        Some((_, n)) => log::info!(
                            "item note: 상세보기 of ({:#04x},{:#04x}) at bag {},{} — {} host line(s), drops {:?}, keeps rows {:?}, name prefix {:?}",
                            t.cat,
                            t.idx,
                            t.tab,
                            t.slot,
                            n.lines.len(),
                            n.dropped,
                            n.keep,
                            n.name_prefix.as_ref().map(|(p, _)| p.as_str())
                        ),
                        None => log::info!(
                            "item note: 상세보기 of ({:#04x},{:#04x}) at bag {},{} — nothing of the host's (an ordinary item, or a name that did not read)",
                            t.cat,
                            t.idx,
                            t.tab,
                            t.slot
                        ),
                    }
                }
                note_basis = basis;
            }
        }

        // 4b-2) IN-GAME 오브결합 / 소켓확장 (2026-08-25, player request: "네트워크 창고
        //     말고 인게임에서 인벤토리 들어가서 오브결합 누르면 바로 아이템에 결합이
        //     가능한거지. 장착중인 아이템에도 오브결합이나 소켓확장이 가능한거고").
        //
        //     Until now this needed a network-warehouse round trip: the game offers its
        //     오브결합 row only when the highlighted WAREHOUSE row's category byte is 0x10
        //     (sole gate 0x7674), its target picker walks the same wire-row array, and the
        //     mutation is done by the server. None of that can be re-pointed at the bag.
        //
        //     What makes the in-game version possible is that the BAG record carries the
        //     whole socket state and the save serialises it — see `orbcombine.rs`. So the
        //     host supplies the verb (one extra row on the game's own popup) and the
        //     object (its own target list), and writes three bytes.
        //
        //     Same cadence and the same reasoning as the 퀵슬롯등록 rewrite above: the row
        //     count byte `CHAR+0x9c85` is shared with other windows, so the write is
        //     re-applied every sample, and `items::orb_popup_writes` returns nothing once
        //     the row is already ours.
        {
            let popup = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::orb_popup(&read)
            };
            if let Some(p) = popup {
                let writes = {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    items::orb_popup_writes(&read, &p)
                };
                let n = writes.len();
                for (a, bytes) in writes {
                    if !emulator.write_memory(a, &bytes) {
                        log::warn!("orb: popup write failed at {a:#x}");
                    }
                }
                if n > 0 {
                    log::info!("orb: popup grown to 오브결합 (bag {},{} idx {:#04x} x{})", p.tab, p.slot, p.orb_idx, p.count);
                }
            }
        }
        // ★ The bag boxes: every frame — drain what the entry hooks took, persist, republish
        //   the knobs, keep the 열기 popup relabelled, put the box window up when asked.
        bagbox.frame(&mut *emulator, &data_dir, frame, &mut pages, &mut qb_window, &item_proxy);
        // ★ 167차: the AVATAR page — closes under any screen that is not EQUIPMENT's worn grid,
        //   opens the one a «1» on SKILL asked for, refreshes what the UI draws.
        avatar.frame(&mut *emulator, &item_proxy, frame);
        // ★ 176차 TEST GRANT: the bits are in memory — the game's own save makes them the slot's
        //   (asked every 30 iterations until the gate says yes; 「저장완료」 on the side line).
        if avatar.save_wanted() && frame % 30 == 9 && save_begin(&mut *emulator, &mut saving, save::Reason::Manual) {
            avatar.save_started();
        }
        // ★★★ 104차 ①: FINISH THAT JOB INSIDE THIS ITERATION.
        //
        // `job_frame` (the last thing `frame` above does) advances the 사용하기 job by one
        // phase and ends each phase by PUSHING KEYS that only `apply_input` may deliver —
        // step 2b, at the top of the next iteration. So the job used to spend one whole
        // worker iteration per phase, and an iteration is 16~21 ms (`FRAME_PACE` is a cap;
        // the guest re-arms its frame timer at ~21 ms). Four of them is the ~0.1 s the
        // player reported: *"소모품 사용하기 이후에 0.1초정도 딜레이가 있는데"*
        // (`§103차 §7` has the per-iteration table).
        //
        // So the keys go out here instead: deliver what the step just pushed, tick once so
        // the guest acts on it, step the job again. With the answer now read before the
        // tick (②) the give, the cursor, both OKs and the game's own use all land in this
        // one iteration.
        //
        // ⛔ Not a `while`. A phase that is waiting on the guest (the popup has not opened
        // yet) pushes nothing, `sent` stays false and the loop leaves — no spin, and the
        // deadline in `bagbox_host` still owns the failure case. Costs nothing when no job
        // is live: `job_pending` is two field reads.
        for _ in 0..BAGBOX_JOB_STEPS {
            if !bagbox.job_pending() {
                break;
            }
            let mut sent = false;
            while let Some(ev) = bagbox.take_input() {
                apply_input(&mut *emulator, &mut held, &mut picking, &mut pages, &mut qb_window, &mut buffs, &mut saving, &mut skill_watch, &mut levelups, &mut cash_qty, &mut orb_pending, &mut bagbox, &mut avatar, &mut walk, &data_dir, &item_proxy, ev);
                sent = true;
            }
            if !sent {
                break;
            }
            // Same accounting as the adaptive multi-tick above — these ticks are real work
            // and must not go missing from the 30 s throughput gauge.
            let step_start = Instant::now();
            if let Err(e) = emulator.tick() {
                log::error!("emulator tick error: {e:?}");
                return;
            }
            gauge_tick_us += step_start.elapsed().as_micros() as u64;
            gauge_ticks += 1;
            bagbox.job_frame(&mut *emulator, &mut pages, &mut qb_window, &item_proxy);
        }
        if frame % 30 == 23 {
            bagbox.poll(&mut *emulator, &data_dir, frame, orb_source.is_some(), &mut pages, &mut qb_window, &item_proxy);
        }
        // The player took our row: build the list of what it could go into.
        //
        // One builder for both sources (a bag cell, 69차; a 오브가방 stack, 2026-09-14):
        // `(orb DB index, subtitle)` in, the candidate list out, and the picker is on screen.
        let build_target_picker = |emulator: &mut dyn Emulator, orb_idx: u16, subtitle: String, from: &str| -> Vec<crate::orbcombine::Cell> {
            let targets = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::orb_targets(&read)
            };
            if targets.is_empty() {
                // Nothing to combine into. Say so and drop it: silence here is
                // indistinguishable from a broken feature. (Worn gear is listed since
                // 70차, so an empty list means NO equipment at all.)
                log::info!("orb: nothing to combine into ({from})");
                let _ = item_proxy.send_event(UserEvent::Notice(crate::orbcombine::NO_TARGET));
                return Vec::new();
            }
            let kind = crate::orbcombine::kind_of(orb_idx);
            // ★ 70차: worn gear is back IN `targets` (disabled rows whose
            // confirmation pops the game-style notice) — the player asked for a
            // confirm popup on worn gear instead of the 69차 hiding.
            let rows: Vec<crate::OrbRow> = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                targets
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let r = crate::orbcombine::row(i, c, crate::gamefont::from_euckr(&c.name), kind, orb_idx);
                        // ★ 71차-D: the game's own item icon for the left column.
                        // Resolved HERE, in the worker, because it can need a live
                        // item-DB read (`items::item_frame`) — the UI thread has no
                        // guest memory.
                        let icon = items::item_frame(&read, c.cat, c.idx);
                        crate::OrbRow { name: r.name, detail: r.detail, enabled: r.enabled, notice: r.notice, icon }
                    })
                    .collect()
            };
            log::info!("orb: picker for {from} idx {orb_idx:#04x} — {} candidate(s)", rows.len());
            let pick = crate::OrbPick { title: crate::orbcombine::title(kind).to_string(), subtitle, rows, ok_label: "결합", keep_open: false, keep_view: false };
            let _ = item_proxy.send_event(UserEvent::OrbPick(Some(Box::new(pick))));
            targets
        };
        if let Some((tab, slot, key)) = orb_pending.take() {
            let orb = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::orb_cell(&read, tab, slot)
            };
            match orb {
                Some(orb) if orb.same_item(key) && orb.is_orb() => {
                    let name = crate::gamefont::from_euckr(&orb.name);
                    let subtitle = if orb.count > 1 { format!("{name}  x{}", orb.count) } else { name };
                    let targets = build_target_picker(&mut *emulator, orb.idx, subtitle, &format!("bag {tab},{slot}"));
                    if !targets.is_empty() {
                        orb_targets = targets;
                        orb_source = Some(crate::bagbox_host::OrbSource::Bag { tab, slot, key });
                    }
                }
                // The cell moved under us.
                _ => {
                    log::info!("orb: the cell moved (bag {tab},{slot})");
                    let _ = item_proxy.send_event(UserEvent::Notice(crate::orbcombine::NO_TARGET));
                }
            }
        }
        // …and the answer. `Some(None)` = cancelled, and the orb is simply kept.
        //
        // ★ Two dialogs share the one answer channel, and the worker knows which it put
        //   up: while the 오브가방 WINDOW is up the index names a stack, and choosing one
        //   opens the target picker with that stack as the source. That half is answered
        //   BEFORE the tick now (step 2c, 104차 ②) and hands the chosen stack over here in
        //   `box_pick`; what is left on the channel at this point belongs to the picker.
        //   ⛔ Keep the `!bagbox.window_open()` guard: with the window still up (a verb that
        //   keeps it, 95차) a second answer is the window's, not the picker's.
        if let Some(src @ crate::bagbox_host::OrbSource::Box { kind, at, cat, idx }) = box_pick {
            let (_, name, _) = crate::bagbox::name_of(crate::bagbox_names::NAMES, cat, idx);
            let count = bagbox.boxes().stacks(kind).get(at).map_or(0, |s| s.count);
            let subtitle = if count > 1 { format!("{name}  x{count}") } else { name };
            let targets = build_target_picker(&mut *emulator, idx, subtitle, &format!("{} #{at}", kind.title()));
            if !targets.is_empty() {
                orb_targets = targets;
                orb_source = Some(src);
            } else {
                // Back to the box: the player has nowhere to put it, not nothing to see.
                bagbox.reopen(kind);
            }
        } else if !bagbox.window_open()
            // ★★★ 158차-B (2026-10-03, user: *"하늘성 입장이 안되는데? 층선택 팝업창에서 포커스된
            //   층을 다시누르거나 입장을누르거나 ok를 누르거나 모두 입장이 안되고있어"*): the
            //   quick-start picker's answer is read in step 2c, BEFORE the tick — and this arm
            //   sits AFTER it, in the same iteration. A tap that lands during the tick (most of
            //   the iteration's wall time) was taken HERE first, matched against an EMPTY
            //   `orb_targets`, and dropped in silence — so 「입장」 did nothing nearly every time.
            //   138차 wrote "only one list dialog is ever up" and never tried the picker on a
            //   device (TODO `EG`). While the tower picker is up its answer is nobody else's:
            //   leave it in the channel for 2c.
            && tower_pick.is_none()
            && let Some(answer) = crate::take_orb_answer()
        {
            let source = orb_source.take();
            let picked = answer.and_then(|i| orb_targets.get(i).map(|c| (c.tab, c.slot, c.key)));
            orb_targets.clear();
            // A cancelled target picker whose orb came from the box goes back to the box.
            if picked.is_none()
                && let Some(crate::bagbox_host::OrbSource::Box { kind, .. }) = source
            {
                bagbox.reopen(kind);
            }
            match (source, picked) {
                (Some(source), Some(target_at)) => {
                    // ★ RE-READ AND RE-VALIDATE at the moment of the write, never trust the
                    //   list: the same discipline `items::item_grid_action` follows. Whatever
                    //   moved since the dialog opened means we do nothing and keep the orb.
                    let plan = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        match source {
                            crate::bagbox_host::OrbSource::Bag { tab, slot, key } => items::orb_apply(&read, (tab, slot, key), target_at),
                            crate::bagbox_host::OrbSource::Box { idx, .. } => items::orb_apply_from_box(&read, idx, target_at),
                        }
                    };
                    match plan {
                        Some((writes, (t_tab, t_slot, cat, idx), kind)) => {
                            // ⛔ ABORT on the first refusal, do not push on. The plan is
                            //   ordered so that the target's orb id comes before its filled
                            //   count and the orb is spent LAST — so stopping early can
                            //   only ever leave the player with MORE than they should have,
                            //   never with a spent orb and nothing to show for it.
                            let mut ok = true;
                            for (a, bytes) in &writes {
                                if !emulator.write_memory(*a, bytes) {
                                    log::warn!("orb: write failed at {a:#x} — aborting the rest");
                                    ok = false;
                                    break;
                                }
                            }
                            // ★ A box orb is spent AFTER the target writes landed (the bag
                            //   path spends last for the same reason — `orb_apply`'s order).
                            //   The spend persists the box before returning.
                            if ok && let crate::bagbox_host::OrbSource::Box { kind, at, cat, idx } = source {
                                ok = bagbox.spend(kind, at, cat, idx);
                            }
                            // ★ Then the game's OWN two refresh helpers, in the order its
                            //   save loader uses them (0x1a3f4 then 0x1a40a) and with the
                            //   same argument tuple. Without the first the socket byte is
                            //   INERT: only four sites in the image ever compute the derived
                            //   halfwords `rec+0x3cc` / `rec+0x3ce`, and none of them runs
                            //   by itself. Combat reads `rec+0x3ce` live through the equipped
                            //   array's (tab, slot) back-references, which is why this works
                            //   ⛔ That alone is NOT enough for a WORN item — those two
                            //   getters have five callers and every one is inside `0x4f014`,
                            //   which caches into `CHAR+0x190`. Which is exactly why
                            //   `orbcombine::eligible` refuses a worn target outright.
                            // ⛔ ONLY the recalc. The name rebuild is deliberately NOT called:
                            //   `0x5283c`'s tail is `sprintf(rec+0x394, "%s", name)` — ONE
                            //   argument, no orb prefix and no refine suffix (that formatting
                            //   lives in the WIRE-row renderer `0x6b90`, which no bag record
                            //   passes through). A combine changes neither cat nor idx, so the
                            //   name it would write is the name already there.
                            //
                            // ⚠ The call is ASYNCHRONOUS — `wie_lgt`'s `call_guest_function`
                            //   SPAWNS it, so it runs on a later tick with the tuple captured
                            //   here. The inventory compactor (`0x5380c` via `0x55048`) swaps
                            //   60-byte records, so in principle it could move this cell in
                            //   between and the recalc would rebuild a different item's derived
                            //   halfwords. The window is one tick, the player has just answered
                            //   a dialog, and the damage is confined to a CACHE the save loader
                            //   rebuilds from scratch (`0x1a3f4`) — but it is not zero, and it
                            //   is the reason nothing here writes anything the game cannot
                            //   re-derive.
                            if ok && let Some(r) = crate::guest_build::item_refresh() {
                                let args = [u32::from(t_tab), u32::from(t_slot), u32::from(cat), u32::from(idx)];
                                emulator.call_guest_function(r.fn_recalc, &args);
                            }
                            // ★★★ 71차-F — AND AGAIN, FOR THE WHOLE BAG, A MOMENT LATER.
                            // Player report 2026-08-26: *"장비->장비장착->우측 장착할 장비
                            // 목록에는 결합한 오브가 최신화가 반영 안되고있어. 근데 장비를
                            // 장착하는 순간 반영이되서 적용이돼."* That is the exact
                            // signature of a DERIVED CACHE that did not get rebuilt: the
                            // raw six bytes are right (equipping recomputes from them and
                            // works), while `rec+0x3cc`/`+0x3ce` — which the item windows
                            // read live (150차: `0x4572c`/`0x45982` are in the 상세보기 painter
                            // `0x4503c`, page 7 — 71차-F's "screen 17" was that, not 장비) —
                            // still hold the old value.
                            //
                            // The single call above can miss for two reasons, and this
                            // covers both: the bag COMPACTS on the popup close and moves
                            // the cell out from under an asynchronous call that captured
                            // `(tab, slot)` before it, and the spawned call can fail with
                            // nothing but a `tracing::warn` to show for it.
                            // ⛔ Do not "optimise" this back to the one cell. The rebuild
                            // is a pure function of each record's own bytes — the save
                            // loader runs it over every record on every load — so doing
                            // all of them is idempotent, and it is the only version that
                            // cannot be defeated by something having moved.
                            if ok {
                                // `target_at.2` is the item's own u64 instance key, which is
                                // what survives a compaction — the diagnostic below finds
                                // the record by it rather than by a (tab, slot).
                                orb_refresh = Some((now_ms() + ORB_REFRESH_DELAY_MS, ORB_REFRESH_PASSES, target_at.2, ORB_REFRESH_EMPTY_RETRIES));
                            }
                            // ⛔ NO AUTOSAVE HERE (2026-09-15, player decision). This used
                            //   to arm a `Reason::Item` retry so the combine reached disk on
                            //   the next saveable field. The player asked for 제련/조합/오브결합
                            //   to STOP auto-saving — the chronic save-corruption bug tracks the
                            //   write frequency — so the result now persists on the player's
                            //   next manual SAVE (the 5-minute autosave and the on-quit save are
                            //   gone too since 2026-09-16, 99차), exactly
                            //   like the 제련/조합 guest saves NOP'd in binary_patches.toml. The
                            //   combined item stays live in the bag record either way (the
                            //   recalc above keeps its derived cache right); only the disk
                            //   write is deferred. ⛔ Do not re-arm this without also reverting
                            //   those two guest patches — the three were removed together.
                            let msg = if ok { crate::orbcombine::done_text(kind) } else { crate::orbcombine::FAILED };
                            log::info!("orb: {kind:?} -> bag {t_tab},{t_slot} cat {cat:#x} idx {idx:#x} ok={ok}");
                            let _ = item_proxy.send_event(UserEvent::Notice(msg));
                        }
                        None => {
                            log::warn!("orb: refused at commit — the cell moved under the dialog");
                            let _ = item_proxy.send_event(UserEvent::Notice(crate::orbcombine::FAILED));
                        }
                    }
                }
                _ => log::info!("orb: cancelled"),
            }
        }
        // The dialog cannot outlive the screen it belongs to. Leaving the inventory — or
        // the game taking the screen back — withdraws it and gives the orb back untouched.
        if orb_source.is_some() || bagbox.window_open() {
            let live = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::menu_state(&read).is_some() || items::orb_popup(&read).is_some() || items::bagbox_popup(&read).is_some()
            };
            orb_gone = if live { 0 } else { orb_gone + 1 };
            if orb_gone >= ORB_EXIT_POLLS {
                orb_source = None;
                orb_targets.clear();
                bagbox.close_window();
                orb_gone = 0;
                let _ = item_proxy.send_event(UserEvent::OrbPick(None));
                log::info!("orb: picker withdrawn — the menu closed under it");
            }
        } else {
            orb_gone = 0;
        }

        // ★★★ 71차-F — THE DEFERRED, BAG-WIDE DERIVED-CACHE REBUILD.
        //
        // `rec+0x3cc` (socket bonus %) and `rec+0x3ce` (effective stat) are a CACHE. The
        // equip screen's candidate list reads them live (and so does the 상세보기 window, page
        // 7 — `0x4572c` / `0x45982` are in ITS painter `0x4503c`; 71차-F's "overlay screen 17"
        // was a flat table index, 150차), so a stale cache is exactly the reported symptom: the list shows the
        // old numbers until the item is equipped, at which point `0x4f014` recomputes the
        // character's own stats from the raw bytes and everything "works".
        //
        // The commit already calls the game's rebuild for the cell it wrote. That call can
        // miss — the popup close compacts the bag (`0x55048`) under an ASYNCHRONOUS call
        // that captured `(tab, slot)` beforehand — so this second pass rebuilds EVERY
        // ★★★ 71차-L — RE-SWEEP WHEN THE 장비 SCREEN OPENS.
        //
        // 71차-F's sweep only ever ran on a TIMER after a combine (250 ms, then 500 ms),
        // so the whole opportunity closed 750 ms after the popup and never came back
        // until the next save/load. If anything went wrong in that window — a compaction,
        // a failed read, the player taking a moment — the 장비 screen showed the old
        // numbers for the rest of the session, which is exactly the standing report
        // ("오브결합 후에 장비장착할때 … 최신화가 안되", TODO `AQ`).
        //
        // Entering the screen is the natural trigger: it is the moment the stale numbers
        // are about to be READ, the sweep is idempotent (it is what the save loader runs
        // over every record on every load), and an EDGE means it costs one pass per visit
        // rather than one per frame.
        //
        // ★ The pips themselves (`rec+0x3c6` / `rec+0x3c9`, drawn at `0x4593a`/`0x4594a`)
        // are read LIVE and were never the cache's business — `0x528f0` writes only
        // `rec+0x3cc` / `rec+0x3ce`. So if a pip is still wrong after this, the combine's
        // own bytes never landed, and the `orb:` lines are the place to look.
        {
            let overlay = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::overlay_screen(&read)
            };
            if overlay == Some(items::EQUIP_OVERLAY) && prev_overlay != Some(items::EQUIP_OVERLAY) && orb_refresh.is_none() {
                // key 0 = "no particular item": the probe line below just reports the
                // sweep size, because there is nothing to find by instance key here.
                // ★ 150차 review: TWO passes, like the combine's. The ~80 recalcs and the
                // snapshot rebuild are separate guest tasks and run in no fixed order, so one
                // pass can bake the copies before some recalcs land — the second, 500 ms on,
                // bakes them from settled records.
                orb_refresh = Some((now_ms(), ORB_REFRESH_PASSES, 0, ORB_REFRESH_EMPTY_RETRIES));
            }
            prev_overlay = overlay;
        }

        // equipment record instead of chasing one. Idempotent by construction: the game's
        // own save loader does exactly this over every record on every load.
        if let Some((due, passes, key, empty_left)) = orb_refresh
            && now_ms() >= due
        {
            let (cells, probe) = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                (items::equipment_cells(&read), items::derived_probe(&read, key))
            };
            if let Some(r) = crate::guest_build::item_refresh() {
                for (tab, slot, cat, idx) in cells.iter().copied().take(ORB_REFRESH_MAX_CELLS) {
                    emulator.call_guest_function(r.fn_recalc, &[u32::from(tab), u32::from(slot), u32::from(cat), u32::from(idx)]);
                }
                // ★★★ 71차-L — AND RE-BAKE THE COPIES. `fn_recalc` writes two halfwords
                // per record and nothing else; the 장비 screen's candidate list and its
                // EQUIPPED array are whole-record BYTE COPIES (`base+0x580`, `base+0x364`)
                // that carry the socket bytes, and only this rebuild refreshes them. Without
                // it a combined item kept showing its old holes until the player equipped
                // something — the report this sweep exists to answer.
                // ⛔ No arguments (proven at `0x55048`), and ⛔ not `0x55048` itself: that
                // compacts the bag first and would move records under the u64-key re-check.
                if r.fn_equip_snapshot != 0 {
                    emulator.call_guest_function(r.fn_equip_snapshot, &[]);
                }
                // ★ ONE diagnostic line, and it is the whole answer to "did the cache
                //   actually refresh": where the item ended up (it may have MOVED — the
                //   probe finds it by its u64 key, never by a stale (tab, slot)), its
                //   sockets, and the two derived halfwords.
                //   ⚠ Read BEFORE this pass's calls, which are asynchronous anyway — so
                //   `pass 1` shows the state the single commit-time call left behind and
                //   `pass 2` shows what pass 1 achieved. That is the pair worth comparing.
                //   If pass 2 already shows the new numbers and the screen still does not,
                //   the screen is reading something else and this is not the cause.
                let pass = ORB_REFRESH_PASSES - passes + 1;
                match if key == 0 { None } else { probe } {
                    Some((tab, slot, sockets, filled, bonus, stat)) => log::info!(
                        "orb refresh (pass {pass}): {} cell(s); the combined item is at bag {tab},{slot} — holes {filled}/{sockets}, bonus {bonus}%, stat {stat}",
                        cells.len()
                    ),
                    None if key == 0 => log::info!("orb refresh (장비 screen): {} equipment cell(s) rebuilt", cells.len()),
                    None => log::warn!("orb refresh (pass {pass}): {} cell(s); the combined item was NOT found by its key", cells.len()),
                }
            }
            orb_refresh = if cells.is_empty() && empty_left > 0 {
                // ★ 71차-L: an empty sweep is a FAILED read, not "no equipment". Try
                // again rather than spending the pass — see `ORB_REFRESH_EMPTY_RETRIES`.
                Some((now_ms() + ORB_REFRESH_DELAY_MS, passes, key, empty_left - 1))
            } else {
                match passes.saturating_sub(1) {
                    0 => None,
                    left => Some((now_ms() + ORB_REFRESH_DELAY_MS * 2, left, key, empty_left)),
                }
            };
        }

        // 4c) MINIMAP. Rendered by the host straight from the guest's live map
        //     arrays — see the block comment on `items::render_minimap` for the
        //     decoded guest routine and the pixel-exact verification.
        //
        //     ★ 2026-08-03: this is the whole of it now. What used to be here was a
        //     six-phase state machine that pressed `*` for the player to make the
        //     guest repaint its own minimap buffer, froze the screen so the window it
        //     opened would not be seen, stashed and replayed the player's input while
        //     it ran, debounced a map-change signature to decide when to do it again,
        //     and gave up after three tries. Every player-reported minimap bug came
        //     out of that machinery. None of it was ever needed: the arrays the map
        //     window paints from are the field's own collision and warp data, live at
        //     all times, so the picture can simply be drawn.
        if frame % MM_RENDER_EVERY == 7 {
            let mm = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::render_minimap(&read)
            };
            if mm != last_minimap {
                last_minimap = mm.clone();
                let _ = item_proxy.send_event(UserEvent::MinimapFrame(mm));
            }
        }
        // ★ 141차: the marks on it, every 5 frames — the player's dot and the monsters'
        // squares move while the map does not, and a 15-frame dot lags a walking player.
        if frame % 5 == 2 && last_minimap.is_some() {
            let marks = {
                let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
                items::minimap_marks(&read)
            };
            if marks != last_marks {
                last_marks = marks.clone();
                if let Some((player, mobs)) = marks {
                    let _ = item_proxy.send_event(UserEvent::MinimapMarks { player, mobs });
                }
            }
        }

        // 4d) Quickbar read probe (~every 5s) — device-validation aid: confirms
        //     CHAR resolves + the per-slot type bytes. Remove once field-confirmed.
        if frame % 300 == 0 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            // ★ The one line that settles whether the game HAS no-save areas at all
            // (2026-08-09). `Some(true)` on an ordinary field and `Some(false)` in a boss
            // room is what earns `HIDE_UI_ON_NO_SAVE`. `None` everywhere means the AREA
            // chain does not resolve and the RE is still not finished. `Some(true)`
            // everywhere — including boss rooms — means the feature has nothing to detect
            // and should be deleted rather than re-aimed.
            // ★ WITH THE ADDRESS (2026-09-08). The tower's no-save fork writes through
            // this same chain, and when a device report says "saving is refused where it
            // used to work" the only thing that separates "the AREA object moved" from
            // "it is the same byte we wrote" is the address.
            log::warn!(
                "area gate: can_save={:?} byte={:?} at {:?}",
                items::area_can_save(&read),
                items::guardian_can_save_byte(&read).map(|(_, v)| v),
                items::guardian_can_save_byte(&read).map(|(a, _)| format!("{a:#x}"))
            );
            let (c, field, win, types, state, sel) = items::debug_probe(&read);
            // `state` / `window` / `selector` are the three bytes the mode decision is made
            // of, and `in_game` is the derived gate that decides whether the slot list is
            // published at all — see `items::debug_probe`. Raw bytes AND the derived answer
            // on one line, so it says both "what did the guest look like" and "what did we
            // conclude", which is the pair a disagreement needs.
            log::info!(
                "quickbar probe: char={c:x?} field={field} window={win:?} state={state:?} selector={sel:?} in_game={} types={types:?} minimap={:?}",
                items::in_game_state(&read),
                // Tile dims of the last render (pixels / 2). `None` while the map
                // chain does not resolve — the single line that says whether the
                // minimap port is live on this build without opening the panel.
                last_minimap.as_ref().map(|m| (m.w / 2, m.h / 2))
            );
            // ⛔ …AND THE THREE NUMBERS `resolve_char` JUDGES, always, not only when it
            // refuses. A silent refusal takes the whole host blind — HUD, quick plates,
            // every touch target that resolves against CHAR — and `char=None` alone
            // cannot tell "no character has been created" from "a live record failed one
            // plausibility test". 71차-L paid for that once (`hp <= hp_max` refused every
            // unequipped character) and 80차-E paid again in the network warehouse, where
            // the game was drawing the bag out of a record this said did not exist.
            // Same lesson as the difficulty byte (78차-C): fail closed, but SAY SO.
            if let Some((base, level, hp_max, sp)) = items::char_raw_stats(&read) {
                log::info!(
                    "char stats at {base:#x}: level={level:?} (want 1..={}) hp_max={hp_max:?} (want <={}) sp={sp:?} (want <={}) -> {}",
                    items::LEVEL_CAP,
                    items::HP_CAP,
                    items::SP_MAX,
                    if c.is_some() { "resolved" } else { "REFUSED" },
                );
            }
            // ⛔ AND WHERE THE PLAYER IS. `char stats` says the record is readable; it
            // says nothing about whether the character is standing anywhere a person
            // can see. "I entered the tower and I vanished" (2026-09-11) cost a session
            // of screenshots and frame arithmetic to answer, and this line answers it.
            if let Some((obj, px, py, tx, ty, mw, mh)) = items::player_position(&read) {
                log::info!("player at {obj:#x}: px=({px},{py}) tile=({tx},{ty}) map={mw}x{mh}");
            }
        }

        // 4d') Offset audit (~every 5 s, offset from the probe above so the two do not
        //      land on the same frame). MIGRATION_LGT.md §2.11: five features are ON
        //      on LGT with CHAR offsets that were only ever verified on KTF, so a
        //      moved offset shows a WRONG value rather than none. `wie_probe
        //      --verify-offsets` prints the same numbers but cannot reach the states
        //      that make them informative (a level-up, a bound quick slot, an
        //      equipped secondary, an open item popup) — a human playing does, in
        //      minutes. Read-only; drop this block once the matrix has no 🔶 rows.
        // 4g-2) 4단계 실측 (66차-B): which byte holds the FOCUS on whatever screen is
        //       open. Three times a second while a window or the menu overlay is up, so
        //       a human can walk the game to a shop / smith / warehouse and the log says
        //       which candidate followed the highlight. ⛔ Read-only; delete with
        //       `items::focus_probe` once §2.1's screens are measured.
        //
        // ★★★ 2026-08-29 (72차) — CHANGE-GATED, because it was a frame-drop of its own.
        // The `None` arm only fires on a BARE FIELD (`overlay == 0 && window == WIN_NONE`),
        // and an NPC conversation is the SCRIPT window — so this ran continuously for the
        // whole of every dialogue: 7.5 times a second, 14 `read_memory` calls each (a `Vec`
        // per 1..4 bytes), a ~250-char `format!`, and an `__android_log_write` that the
        // shipped `warn,main=info` filter really does emit. 18차 already recorded the rule
        // this broke — *"an `__android_log_write` inside the frame budget, ~100/s, is a
        // performance bug of its own"* — and logd can block for milliseconds when the
        // buffer is contended, which is exactly the "가끔 살짝씩" the player described.
        // ⛔ Do NOT delete it outright: 66차-C used this line to fix the warehouse touch
        // bug in a single build, and it is still the only instrument that says which byte
        // follows the highlight on a screen nobody has mapped. Printing it only when it
        // CHANGES keeps all of that and makes a static screen silent — the same treatment
        // the audio-option probe below already has.
        if frame % 8 == 0 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            if let Some(line) = items::focus_probe(&read) {
                if last_focus_probe.as_deref() != Some(line.as_str()) {
                    log::warn!("{line}");
                    last_focus_probe = Some(line);
                }
            } else {
                last_focus_probe = None;
            }
        }
        // ★★★ 2026-08-28 — the audio option bytes, printed ONLY when they change.
        // The in-game 환경설정 sound rows reach no media API at all (vc189 device log:
        // 배경음 2 → 1 → OFF with zero `MC_mdaClipSetVolume`), so the level has to be read
        // out of the game's own state. This line is how the byte gets identified: move ONE
        // slider and see which of them moves. Change-gated, so an unchanged setting is
        // silent and the log stays readable.
        if frame % 30 == 7 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            // ★★★ 2026-08-28 — THE 배경음 SLIDER, READ FROM THE GAME'S OWN OPTION BYTE.
            // Re-asserted every poll (~1 s) rather than latched, so it also corrects a
            // stale level that `SmafPlayer` pushed from the clip cell. See
            // `audio_sink::set_music_level` for why the clip cell cannot be trusted.
            if let Some((bgm, _sfx)) = items::audio_option_levels(&read) {
                crate::audio_sink::set_music_level(bgm);
            }
            if let Some(line) = items::audio_opts_audit(&read) {
                let sig = crate::savevault::identity_hash(line.as_bytes());
                if AUDIO_OPTS_SIG.swap(sig, std::sync::atomic::Ordering::Relaxed) != sig {
                    log::warn!("{line}");
                }
            }
        }
        // ★★★ 2026-09-23 (DT) — publish 공격속도 / 스킬사용속도 to the guest hook.
        //
        // `atk_speed_lgt` reads ONE word (`wie_util::stats::ATTACK_SPEED_PCT`) every time
        // the player's state machine asks how long a pose lasts, so the host's only job is
        // to keep that word current. Re-asserted rather than latched: the sum changes the
        // moment a piece of gear is worn or removed, and a stale value would be a stat
        // that does not match the equipment screen.
        //
        // ⛔ 0 when there is no character, no grid or nothing worn — and 0 is the original
        //    timing, so every screen outside the field is automatically untouched.
        if frame % 30 == 3 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            let pct = crate::atkspeed::effective_pct(items::attack_speed_equipment_pct(&read), atk_speed_override);
            if ATTACK_SPEED_LAST.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                log::warn!(
                    "attack speed: {pct} % ({}) — an attack now runs at about {}.{:02}x (the pose clock advances faster; no pose is shortened)",
                    if atk_speed_override.is_some() { "wie_attack_speed.txt" } else { "worn equipment" },
                    crate::atkspeed::speed_x100_at(pct) / 100,
                    crate::atkspeed::speed_x100_at(pct) % 100,
                );
            }
            wie_util::stats::set_attack_speed_pct(pct);

            // ★ 142차 — the 85제 set effects (`setbonus.rs`): which sets have ANY 4 of their 6
            // pieces worn (150차 — it was 5), published for the hooks (`playerhp` · `dmgwide`)
            // the same way — re-asserted, never latched. A set that becomes complete ON THE
            // EQUIPMENT SCREEN gets the game-style box (loading a character already wearing
            // one is not news); every change is logged.
            // ★ 150차: that screen is page 3 with the menu open (`items::EQUIP_OVERLAY` was 17
            // until now — never a page — so the box had never once been shown).
            let worn = items::worn_items(&read);
            let mask = crate::setbonus::active_mask(&worn, crate::set_items::SET_ITEMS);
            if mask != set_mask_last {
                let on_equip_screen = items::overlay_screen(&read) == Some(items::EQUIP_OVERLAY);
                log::warn!("set bonus: {:#x} -> {mask:#x}{}", if set_mask_last == u32::MAX { 0 } else { set_mask_last }, if on_equip_screen { " (equipment screen)" } else { "" });
                if on_equip_screen && set_mask_last != u32::MAX {
                    for s in &crate::setbonus::SETS {
                        if mask & s.bit != 0 && set_mask_last & s.bit == 0
                            && let Some(text) = crate::setbonus::activation_notice(s.bit)
                        {
                            let _ = item_proxy.send_event(UserEvent::NoticeText(text));
                        }
                    }
                }
                set_mask_last = mask;
            }
            wie_util::stats::set_set_bonus(mask);
            // ★ 150차 — the reworked accessories' third options that are HOST / HOOK effects
            // (피해감소 · 주는 피해 · 처치/피격 시 최대 HP), summed off the same nine slots —
            // every copy counts, like the game's own option pairs — and published as ONE word
            // the hooks read on every hit (`dmgwide`, `playerhp`). Re-asserted, never latched;
            // a change is logged. (The `Stat` thirds are the guest hook's, at the wear
            // recompute; 공격속도 rides `worn_option_pairs` above.)
            let acc = wie_util::stats::acc_effects_of(worn.iter().copied());
            if acc != acc_last {
                log::warn!("accessory effects: {acc_last:?} -> {acc:?}");
                acc_last = acc;
            }
            wie_util::stats::set_acc_effects(acc);
            // ★ 156차 방패무시: the game's own summed option (stat block `+0x72`, never read by
            // the game) and the main weapon's defence kind, published for the `dmg_apply_lgt`
            // hook the same way — re-asserted, never latched; a change is logged.
            // ★ 163차: 개정판 only — vanilla keeps the game's rule (summed, never read), and 직격의
            // 오브 is an original orb, so on vanilla this would have been a live change.
            let pierce = if crate::flavor::REBALANCE {
                (items::shield_pierce_pct(&read), items::worn_weapon_special(&read))
            } else {
                (0, false)
            };
            if pierce != pierce_last {
                log::warn!(
                    "방패무시: {} % -> {} % (the main weapon resolves against the {} defence)",
                    pierce_last.0,
                    pierce.0,
                    if pierce.1 { "special" } else { "physical" }
                );
                pierce_last = pierce;
            }
            wie_util::stats::set_pierce(pierce.0, pierce.1);
            // ★ The TIMED max-HP bonus — 142차-C 붉은마녀 「마녀의 생명력」 and 150차 레아의목걸이
            // (a kill) / 굴베이그의탈리스만 (a hit taken): +N % for 20 s, the running sources
            // added. The `ComputeMaxHP` hook (`playerhp`) scales every recompute; the worker
            // re-runs that function (`0x49000(block)`, pure — it only computes and stores)
            // when the bonus changes, and on the way down raises the clamp flag so an HP
            // above the lowered max is brought to it at once.
            // ⛔ Only in the plain FIELD (150차 review): the 장비 screen's preview
            // (`0x4f014(preview = 1)`) leaves the CANDIDATE's stats in the block, and a re-run
            // there — with the clamp flag, on the way down — kept the candidate's lower max as
            // the player's HP for good. The edge simply waits: `set_hp_bonus_last` only moves
            // when the re-run is actually issued.
            let hp_bonus = wie_util::stats::max_hp_bonus_pct(
                mask,
                acc,
                wie_util::stats::play_clock_ms(),
                wie_util::stats::WITCH_UNTIL_MS.load(std::sync::atomic::Ordering::Relaxed),
                wie_util::stats::HIT_UNTIL_MS.load(std::sync::atomic::Ordering::Relaxed),
            );
            if hp_bonus != set_hp_bonus_last
                && items::in_field_state(&read)
                && let Some(f) = crate::guest_build::compute_max_hp_fn()
                && let Some(block) = items::player_stat_block(&read)
            {
                if hp_bonus < set_hp_bonus_last {
                    wie_util::stats::HP_MAX_CLAMP.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                emulator.call_guest_function(f, &[block]);
                log::warn!(
                    "timed max HP: bonus {} -> {hp_bonus} % (ComputeMaxHP re-run; set {mask:#x}, kill +{} %, hit +{} %)",
                    if set_hp_bonus_last == u32::MAX { "(a load)".to_string() } else { format!("{set_hp_bonus_last} %") },
                    acc.kill_max_hp_pct,
                    acc.hit_max_hp_pct
                );
                set_hp_bonus_last = hp_bonus;
            }
            // ★ 150차 레아의목걸이 「5초당 HP회복 +N%」 (option id 0x78 — the game ignores it):
            // every 5 s of PLAY clock in the field, heal N % of the true max through the
            // game's own `addHP` (`(STAT_BLOCK, n)` — a host call's delta is true HP, see
            // `guest_build::add_hp_fn`). The due time only moves while healing is allowed, so
            // a menu or a scene change delays the next heal rather than eating it; a due time
            // that has passed pays ONE heal and is re-armed — never a burst (`regen_tick`).
            // (A fresh reader: the re-run above borrowed the emulator mutably.)
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            let regen = items::regen_pct(&read);
            if regen == 0 {
                regen_due = 0;
            } else if items::regen_may_heal(&read) {
                let (fire, next) = wie_util::stats::regen_tick(play_ms, regen_due, wie_util::acc_options::REGEN_PERIOD_MS);
                regen_due = next;
                if fire
                    && let Some(f) = crate::guest_build::add_hp_fn()
                    && let Some(block) = items::player_stat_block(&read)
                    && let Some(s) = items::read_stats(&read)
                {
                    let n = wie_util::stats::regen_amount(s.hp, s.hp_max, regen);
                    if n > 0 {
                        emulator.call_guest_function(f, &[block, n]);
                        regen_heals = regen_heals.saturating_add(1);
                        // The first few say it works; after that it is a heartbeat nobody reads.
                        if regen_heals <= REGEN_LOG_HEALS {
                            log::warn!(
                                "regen: +{n} HP ({regen} % of {} every {} ms; was {}) — heal #{regen_heals}",
                                s.hp_max,
                                wie_util::acc_options::REGEN_PERIOD_MS,
                                s.hp
                            );
                        }
                    }
                }
            }
            let triggers = wie_util::stats::GUARD_TRIGGERS.load(std::sync::atomic::Ordering::Relaxed);
            if triggers != guard_triggers_last {
                log::warn!("set bonus: 수호의 맹세 fired ({triggers} time(s) this session)");
                guard_triggers_last = triggers;
            }
        }
        if frame % 300 == 150 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            if let Some(line) = items::offset_audit(&read) {
                log::warn!("{line}");
            }
            if let Some(line) = items::autoloot_candidates(&read) {
                log::warn!("{line}");
            }
        }

        // 4h) FLAG WINDOW watcher — how a 🔶 offset gets closed on a device instead of by
        //     more disassembly. It reports any byte in the character record's flag block
        //     that changed since the last look, so using the 오토루팅 item names the byte
        //     it sets. Polled faster than the audit (≈2x/s) because the interesting
        //     transitions are single-frame, and it is silent unless something moved.
        if frame % 30 == 15 {
            let read = |addr: u32, len: usize| emulator.read_memory(addr, len);
            if let Some(line) = items::flag_window_changes(&read) {
                log::warn!("{line}");
            }
        }

        // 5) Pace to the target cadence — but never sleep PAST the guest's own next
        //    wake-up (2026-08-13, `MIGRATION_LGT.md` §2.24).
        //
        //    `FRAME_PACE` is a ceiling on how often this loop runs, and treating it as
        //    the nap length quantises every guest timer up to the next 16 ms boundary.
        //    Hero3 re-arms its frame timer at ~21 ms, so a frame that finished at 5 ms
        //    was not looked at again until 16 — then 32 — and the panel-heavy screens
        //    ran at 7.5 fps with this thread 79 % idle. Asking the emulator when it next
        //    has work turns that back into "wake when the game asked to be woken".
        gauge_loops += 1;
        let elapsed = frame_start.elapsed();
        if elapsed < FRAME_PACE {
            let mut nap = FRAME_PACE - elapsed;
            if let Some(ms) = emulator.next_wakeup_ms() {
                // 1 ms floor: `Some(0)` means a task is runnable, and this loop must not
                // become a spin (the multi-tick above is what serves a work-bound guest).
                nap = nap.min(Duration::from_millis(ms.max(1)));
            }
            gauge_sleep_us += nap.as_micros() as u64;
            thread::sleep(nap);
        }
    }
}

/// If `<data_dir>/dump_trigger` exists, read the regions it names and write a
/// snapshot to `<data_dir>/ram_dump.bin` (each region prefixed by u32 LE addr +
/// u32 LE length), logging the head of each. The trigger is consumed (deleted).
fn debug_ram_dump(emulator: &dyn Emulator, data_dir: &std::path::Path) {
    let trigger = data_dir.join("dump_trigger");
    let Ok(spec) = std::fs::read_to_string(&trigger) else {
        return;
    };
    let _ = std::fs::remove_file(&trigger);
    let mut out: Vec<u8> = Vec::new();
    for line in spec.lines() {
        let mut it = line.split_whitespace();
        let (Some(a), Some(l)) = (it.next(), it.next()) else {
            continue;
        };
        let hex = |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16);
        let (Ok(addr), Ok(len)) = (hex(a), hex(l)) else {
            continue;
        };
        let (addr, len) = (addr as u32, (len as usize).min(0x20000));
        let bytes = emulator.read_memory(addr, len);
        let head: Vec<String> = bytes.iter().take(24).map(|b| alloc_hex(*b)).collect();
        log::info!("ramdump {addr:#010x} len={len} got={} head={}", bytes.len(), head.join(""));
        out.extend_from_slice(&addr.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&bytes);
    }
    if !out.is_empty() {
        let _ = std::fs::write(data_dir.join("ram_dump.bin"), &out);
        log::info!("ramdump -> ram_dump.bin ({} bytes)", out.len());
    }
}

fn alloc_hex(b: u8) -> String {
    format!("{b:02x}")
}
