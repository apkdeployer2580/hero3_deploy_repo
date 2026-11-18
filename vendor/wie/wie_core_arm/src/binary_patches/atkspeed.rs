//! 공격속도 / 스킬사용속도 — the player's action animations, shortened (2026-09-23).
//!
//! ── WHAT AN ACTION COSTS, IN THE GAME'S OWN UNITS ────────────────────────────
//! The player's field update is ONE function, the state machine `0x5089c` (jump table
//! `0x5e224`, player state `base+0x9b8e`, 0..16). Its first act every tick is
//!
//! ```text
//! 0x508bc   *(u16*)(base + 0x9ba0) += 1        // the POSE FRAME COUNTER
//! ```
//!
//! and every action arm then runs a POSE SCRIPT: `CHAR+0x23f[8]` holds pose ids
//! (`-1` terminates), `CHAR+0x24f` the current step, `CHAR+0x250` the step the hit lands
//! on. A step ends when
//!
//! ```text
//! poseFrameCounter == 0x4aee0(weaponKind, facing - 1, actionMode)
//! ```
//!
//! and `setPose` (`0x4a0f8` -> `0x56094`) zeroes the counter for the next one.
//!
//! ── ★★★ THAT COMPARISON IS AN EXACT `==`, AND THAT IS THE WHOLE STORY ────────
//! Disassembled (2026-09-23), state 2's arm reads:
//!
//! ```text
//! 0x50a94  bl 0x4aee0            ; r0 = frames for THIS pose, THIS facing, THIS weapon
//! 0x50aa6  ldr r0,[pc,#0x17c]    ; literal 0x9b14, and r6 = base+0x8c  =>  base+0x9ba0
//! 0x50ab0  ldrsh r3,[r1,r5]      ; r3 = the pose counter
//! 0x50ab4  cmp   r2, r3          ; r2 = (i8)frames
//! 0x50ab6  beq   0x50aba         ; ONLY here does the step end — and only beyond it is
//! 0x50ab8  b     0x50c94         ;   `step == CHAR+0x250` (the hit) ever tested
//! ```
//!
//! A counter that lands ABOVE `frames` therefore never meets it again: the pose never
//! completes, the script never advances, and **the hit test never runs at all**. The
//! character stays in state 2 until something else (a stagger, a knockback) resets it —
//! which is exactly what a player sees as "I attack and no damage goes in".
//!
//! ── ⛔ WHY THE PREVIOUS REVISION DID THAT (131차-D, fixed here) ───────────────
//! It advanced the counter from a hook on `0x508bc`, the state machine's own `+= 1`, and
//! clamped against a length RECORDED BY THE PREVIOUS TICK. But `0x508bc` runs BEFORE the
//! arm calls `0x4aee0`, so on the first tick of a pose the recorded length belongs to the
//! pose that just ended — or, since `facing` is one of `0x4aee0`'s three arguments, to the
//! same pose in another DIRECTION. Whenever the new pose is shorter, the clamp let the
//! counter step straight over it. The headless A/B never saw this because it repeated one
//! attack, in one direction, with one weapon: there the old length and the new one are the
//! same number.
//!
//! ⇒ The acceleration now happens INSIDE `0x4aee0`'s own tail, where `frames` for the pose
//! that is running right now is in `r0`. Clamping there cannot overshoot by construction, so
//! there is no step cap to tune and nothing to record between ticks.
//!
//! ── ★★★ AND A POSE ALWAYS SPENDS AT LEAST ONE TICK BEING THAT POSE ──────────
//! The counter is dragged to `frames - 1` at most; the game's own `+= 1` supplies the last
//! one. Measured (43 attacks, one monster pinned beside the player): without that floor a
//! swing collapsed to a single tick at +200 % and above and the attack dealt NOTHING, which
//! is 131차 §7.3 again — the combat code reaches the creature through its OWN animation
//! clock and `0x1b780`, not through this counter, so a swing has to actually happen. With
//! the floor, total damage over those 43 attacks went 200 (off) -> 400 (+127) -> 350 (+200)
//! -> 400 (+400): more damage, never less, right across the range.
//!
//! ── ★★★ AND EVERY FRAME THE CLOCK PASSES STILL GETS ITS EVENT (132차-C) ─────
//! Hits do NOT come from the pose ending. Every tick the state machine hands the CURRENT
//! counter to one function, `0x4b92c(frame)` — called from exactly one place, `0x50e2c`,
//! on the not-yet-complete path — which dispatches that frame's events through a
//! hard-coded switch, `0x57c3c(table, kinds, animSet-1, pose, frame)`: per weapon kind,
//! per pose, `if frame == N -> 0x4de90` (apply the attack to the targets collected at the
//! attack start). Traced headless with a 3-step combo (2026-09-24), the monster pinned in
//! six different places — the frames do not move, so it is a timing table, not geometry:
//!
//! ```text
//!   pose 2 (attack 1, 4 frames)   hit on frame 2
//!   pose 3 (attack 2, 6 frames)   hits on frames 1 AND 4
//!   pose 4 (attack 3, 6 frames)   hit on frame 2
//! ```
//!
//! A tick whose counter was dragged from `c` to `c'` dispatched `c'` only, so frames
//! `c..c'-1` — and any hit among them — never happened. Attack 1 survived by coincidence
//! (its only hit frame IS `frames - 1`, exactly where the drag stops) and that is all the
//! 132차 harness ever swung; the combo's attacks 2 and 3 lost one of two hits and their
//! only hit. ⇒ [`handle_event_lgt`] sits on that `bl 0x4b92c` and, when this tick's drag
//! skipped frames of the pose being dispatched, runs the game's own `0x4b92c` for each of
//! them first, oldest first, then makes the call the game made. Every frame a pose passes
//! through gets its event exactly once, as unhooked; a faster attack lands the same hits,
//! sooner.
//!
//! ── WHICH ACTIONS ARE SPED UP ────────────────────────────────────────────────
//! `0x4aee0` has exactly six callers, all inside the player's own state machine:
//!
//! | site      | state | what it times          | scaled |
//! |-----------|-------|------------------------|--------|
//! | `0x50a94` | 2     | the action's pose script (basic attack AND skill casts) | yes |
//! | `0x510ba` | 4     | the follow-through / recovery | yes |
//! | `0x510fe` | 4     | ditto, the other branch | yes |
//! | `0x50e74` | 3     | being hit (stagger)    | no |
//! | `0x5101c` | 8     | knockback              | no |
//! | `0x50ec8` | 16    | the death animation    | no |
//!
//! The hook is on the function's exit, so it reads the live state byte to tell them
//! apart ([`ACTION_STATES`]). Shortening the stagger would be a defensive buff and
//! shortening death would race the GAME OVER window — neither is "attack speed".
//!
//! ── ⛔ AND THE ANSWER ITSELF IS NEVER CHANGED ────────────────────────────────
//! Two revisions tried to shorten an attack by ANSWERING SMALLER — first on this wrapper,
//! then on the leaf `0x1b780` every reader shares — and BOTH made the attack deal ZERO
//! damage (43 attacks, 0 kills, against 3 kills unhooked). The hit lands on the tick the
//! counter REACHES a particular value, so the value has to stay where it is and the counter
//! has to get there sooner. That is what this file does.
//!
//! ★ 163차: MAIN ONLY (`flavor = "main"` on all four hooks) — 공격속도 is a 개정판 option and
//! vanilla keeps the game's own frame clock. (The host's OK-hold repeat is not here and stays
//! on both flavours.)

use alloc::format;

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// LGT: CHAR is a static at `.bss 0x1503a88`, and `getBase()` returns `CHAR + 0x28`.
const GUEST_BASE: u32 = 0x0150_3a88 + 0x28;
/// The player state byte, base-relative (`base+0x9b8e`, the state machine's own input).
const PLAYER_STATE_OFF: u32 = 0x9b8e;
/// Absolute address of that byte on this build: `0x150d63e`.
pub const PLAYER_STATE_ADDR: u32 = GUEST_BASE + PLAYER_STATE_OFF;

/// The pose frame counter (`base+0x9ba0`, u16) the dispatcher bumps at `0x508bc`.
///
/// ★ Pinned twice from the image: the dispatcher's own literal at `0x50bec` is `0x9ba0`,
/// and state 2's arm reaches the same word as `r6 + 0x9b14` with `r6 = base + 0x8c`.
const POSE_COUNTER_OFF: u32 = 0x9ba0;
/// Absolute address of that counter on this build: `0x150d650`.
pub const POSE_COUNTER_ADDR: u32 = GUEST_BASE + POSE_COUNTER_OFF;

/// The pose byte (`base+0x9b8d`, `0x4aee0`'s second argument, what `0x4b92c` dispatches on).
const POSE_OFF: u32 = 0x9b8d;
/// Absolute address of the pose byte on this build: `0x150d63d`.
pub const POSE_ADDR: u32 = GUEST_BASE + POSE_OFF;

/// The per-frame event dispatcher, `0x4b92c(frame)` (Thumb).
pub const EVENT_FN: u32 = 0x0004_b92d;

/// `bl 0x4b92c` at `0x50e2c` — both halfwords, since the event hook displaces the whole
/// 32-bit call and resumes past it.
pub const EVENT_HEAD: [u8; 4] = [0xfa, 0xf7, 0x7e, 0xfd];

pub fn check_event_head(head: [u8; 4], pc: u32) -> Result<()> {
    if head == EVENT_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "atk_event hook at {pc:#x}: expected {EVENT_HEAD:02x?}, found {head:02x?}"
    )))
}

/// "This tick's drag skipped nothing" — see [`skip_record`].
pub const SKIP_NONE: u32 = u32::MAX;

/// Pack "the drag moved `pose`'s clock from `from` to `to` this tick". [`SKIP_NONE`] when
/// nothing was skipped or the numbers do not fit the game's own `i8` frames.
pub fn skip_record(pose: i8, from: i32, to: i32) -> u32 {
    if !(0..to).contains(&from) || to > i8::MAX as i32 {
        return SKIP_NONE;
    }
    (pose as u8 as u32) << 16 | (from as u32) << 8 | to as u32
}

/// The frames to replay before the game dispatches `frame` of `pose_now`: `from..frame`,
/// but only when the record is THIS tick's — the same pose, and the frame the drag
/// stopped on. Anything else replays nothing (fail closed: a stale record must never fire
/// an attack the player did not make).
pub fn replay_range(record: u32, pose_now: i8, frame: i32) -> Option<core::ops::Range<i32>> {
    if record == SKIP_NONE {
        return None;
    }
    let pose = (record >> 16) as u8 as i8;
    let from = ((record >> 8) & 0xff) as i32;
    let to = (record & 0xff) as i32;
    (pose == pose_now && to == frame && from < to).then_some(from..to)
}

/// The states whose pose timing is the player's ATTACK: the pose script (`2`) and its
/// follow-through (`4`). ⛔ Not 3 (staggered), 8 (knocked back) or 16 (dying).
pub const ACTION_STATES: [u8; 2] = [2, 4];

/// Highest bonus the hook will honour. Above this a pose is a single tick and more percent
/// buys nothing, so the cap keeps the arithmetic in a sane range rather than imposing a
/// balance decision.
pub const MAX_PCT: u32 = 400;

/// `lsls r0, r0, #0x18` at `0x4af12` — the instruction this hook displaces. It is the
/// first half of `0x4aee0`'s `(i8)` narrowing of `0x1b780`'s return; `exit_pc` is the
/// `asrs r0, r0, #0x18` that completes it, so the handler leaves the value pre-shifted.
pub const HEAD: [u8; 2] = [0x00, 0x06];

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "atk_speed hook at {pc:#x}: expected {HEAD:02x?}, found {head:02x?}"
    )))
}

/// Is this state one whose pose timing belongs to an attack or a skill cast?
pub fn is_action_state(state: u8) -> bool {
    ACTION_STATES.contains(&state)
}

/// How many EXTRA ticks of pose clock this tick is worth, on top of the game's own `+= 1`.
///
/// `carry` holds the leftover hundredths between ticks, so a bonus under +100 % still does
/// something: +50 % adds one tick every other tick. Pure.
pub fn pose_extra(pct: u32, carry: &mut u32) -> u32 {
    if pct == 0 {
        *carry = 0;
        return 0;
    }
    let total = pct.min(MAX_PCT) + *carry;
    *carry = total % 100;
    total / 100
}

/// Where the pose counter should stand once this tick's bonus is applied.
///
/// ⛔ TWO CLAMPS, AND BOTH ARE LOAD-BEARING.
///
/// 1. `frames` here is what `0x4aee0` is returning for the pose running RIGHT NOW, so the
///    counter can never step over the value five sites compare with `==`. That is what
///    makes an unbounded `extra` safe and why there is no step cap.
/// 2. The counter is never taken to `frames` itself — the last tick is left to the guest's
///    own `+= 1`, so a pose always spends at least one tick BEING that pose. A swing that
///    completes on the tick it starts loses its hit, because the combat code reaches the
///    creature through its own animation clock rather than this counter.
///
/// ★ `counter > frames` is put BACK on `frames`. Nothing in this file can produce that any
/// more, but a character that somehow reached it is frozen mid-swing forever, and handing
/// the state machine the value it is waiting for is the cheapest possible way out.
pub fn advanced_counter(counter: i32, frames: i32, extra: u32) -> i32 {
    if frames <= 0 {
        return counter;
    }
    if counter > frames {
        return frames;
    }
    if extra == 0 {
        return counter;
    }
    // ⛔ NEVER the last tick. A pose that COMPLETES in a single tick loses its hit: the
    // combat code reaches the creature through its OWN animation clock and `0x1b780`, not
    // through this counter (131차 §7.3), so a swing has to actually spend a tick being a
    // swing. Measured with the pinned-monster harness (43 attacks, scene 7):
    //
    //     pct        0    50   127   200   400
    //     kills      3     2     3     1     0     <- +400 lost damage entirely
    //     swing      3     2     2     1     1     ticks (frames = 3)
    //
    // and WITH the floor, total damage over the same 43 attacks:
    //
    //     pct        0   127   200   400
    //     damage   200   400   350   400
    //     kills      4     6     5     6
    //
    // Everything that still landed a hit spent two or more ticks swinging, and the two
    // runs that collapsed to one tick are the two that lost damage. Leaving the final tick
    // to the guest's own `+= 1` puts a floor of two ticks under every pose that had more
    // than one to begin with, and leaves a genuinely 1-tick pose exactly as it was.
    let ceiling = frames - 1;
    if counter >= ceiling {
        return counter;
    }
    (counter + extra as i32).min(ceiling)
}

/// How many scaled decisions narrate themselves before the hook goes quiet.
const REPORT_MAX: u32 = 24;
static REPORTED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// The leftover hundredths of a tick between poses — see [`pose_extra`].
static POSE_CARRY: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// What this tick's drag stepped over, for [`handle_event_lgt`] — see [`skip_record`].
static SKIPPED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(SKIP_NONE);
/// How many replays narrate themselves before the event hook goes quiet.
static REPLAYS_REPORTED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// ── THE OTHER HALF OF AN ATTACK: THE RECOVERY COUNTDOWN ──────────────────────
//
// State 2 is only the swing. What follows it is state 4, and state 4 does NOT use the
// pose-length comparison at all — its arm dispatches on `[r6,#0x74]` and the arms that
// matter (`0x51062`, `0x51086`) just tick a byte down:
//
// ```text
//   r3 = *(u8*)(r6 + 0x19d)   // = CHAR+0x251
//   r3 -= 1 ; store
//   if (i8)r3 <= 0 -> 0x50774 (the idle reset: state 0, pose 0)
// ```
//
// Measured by diffing CHAR frame by frame across one attack: `0x251` goes 4, 3, 2, 1, 0
// and the action ends on the 0 — four of the eight ticks. Halving only the swing left the
// attack 13 % faster at +100 %, which is not a stat anybody would notice.
//
// ★ This site is the EASY one: the test is `<= 0`, not `==`, so subtracting more than one
// cannot be stepped over. The carry here spreads the extra across ticks so the countdown
// lasts `4 * 100 / (100 + pct)` ticks on average.
static RECOVER_CARRY: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// How much to take off the recovery counter this tick — at least 1, which is the game's
/// own step and therefore what `pct == 0` must produce.
///
/// Pure. `carry` holds the leftover hundredths between ticks.
pub fn recover_decrement(pct: u32, carry: &mut u32) -> u32 {
    if pct == 0 {
        return 1;
    }
    let total = pct.min(MAX_PCT) + *carry;
    *carry = total % 100;
    1 + total / 100
}

/// `0x51068` / `0x5108c`: the displaced `subs r3,#1` of state 4's recovery countdown, but
/// taking off as many ticks as the stat is worth. `r3` holds the byte the game just loaded.
///
/// ⛔ Saturating at 0: the game's own test is `(i8)r3 <= 0`, and 0 ends the action on this
/// very tick. Going negative would work too, but 0 keeps the byte readable for the key
/// handler, which uses the same counter to answer "is an action still running?".
pub async fn handle_recover_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    use core::sync::atomic::Ordering;
    let pct = wie_util::stats::attack_speed_pct();
    let mut inner = core.inner.lock();
    let left = inner.engine.reg_read(ArmRegister::R3);
    let step = if pct == 0 {
        1
    } else {
        let mut carry = RECOVER_CARRY.load(Ordering::Relaxed);
        let d = recover_decrement(pct, &mut carry);
        RECOVER_CARRY.store(carry, Ordering::Relaxed);
        d
    };
    inner.engine.reg_write(ArmRegister::R3, left.saturating_sub(step));
    Ok(JumpTo(exit_pc))
}

/// `subs r3,#1` — the instruction both recovery sites displace.
pub const RECOVER_HEAD: [u8; 2] = [0x01, 0x3b];

pub fn check_recover_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == RECOVER_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "atk_recover hook at {pc:#x}: expected {RECOVER_HEAD:02x?}, found {head:02x?}"
    )))
}

/// `0x4af12`: the displaced `lsls r0,r0,#0x18` — and, while the player is attacking, the
/// tick that drags the pose clock towards the length this very call is answering.
///
/// ⛔ THE ANSWER IS NEVER TOUCHED. `r0` leaves exactly as the guest computed it; the only
/// thing that moves is the counter, and only ever TOWARDS `frames`.
///
/// ⛔ FAIL SOFT in the game's favour. A state byte we cannot read, a state that is not an
/// action, a counter we cannot read or write, or a pose with no length all leave the tick
/// exactly as the guest would have run it. The hook never returns an error, because
/// everything it could fail at is optional.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    use core::sync::atomic::Ordering;
    let frames = core.inner.lock().engine.reg_read(ArmRegister::R0) as i8 as i32;
    let pct = wie_util::stats::attack_speed_pct();
    // What this call's drag steps over — re-decided on EVERY call, so a record can only
    // ever describe the tick it was made in (see [`handle_event_lgt`]).
    let mut skipped = SKIP_NONE;
    if pct != 0 && frames > 0 {
        let mut state = [0u8; 1];
        if core.read_bytes(PLAYER_STATE_ADDR, &mut state).is_ok() && is_action_state(state[0]) {
            let mut raw = [0u8; 2];
            if core.read_bytes(POSE_COUNTER_ADDR, &mut raw).is_ok() {
                let counter = i16::from_le_bytes(raw) as i32;
                let mut carry = POSE_CARRY.load(Ordering::Relaxed);
                let extra = pose_extra(pct, &mut carry);
                POSE_CARRY.store(carry, Ordering::Relaxed);
                let next = advanced_counter(counter, frames, extra);
                if next != counter {
                    let wrote = core.write_bytes(POSE_COUNTER_ADDR, &(next as u16).to_le_bytes()).is_ok();
                    // Only a clock that really moved forward skipped anything. The pose byte
                    // is what `0x4b92c` will dispatch on.
                    let mut pose = [0u8; 1];
                    if wrote && next > counter && core.read_bytes(POSE_ADDR, &mut pose).is_ok() {
                        skipped = skip_record(pose[0] as i8, counter, next);
                    }
                    let n = REPORTED.fetch_add(1, Ordering::Relaxed);
                    if n < REPORT_MAX {
                        tracing::warn!(
                            "atk speed: state {} pose tick {counter} -> {next} of {frames} ({pct} %)",
                            state[0]
                        );
                    }
                }
            }
        }
    }
    SKIPPED.store(skipped, Ordering::Relaxed);
    core.inner
        .lock()
        .engine
        .reg_write(ArmRegister::R0, (frames as u8 as u32) << 24);
    Ok(JumpTo(exit_pc))
}

/// `0x50e2c`: the displaced `bl 0x4b92c` — the state machine's one per-tick frame-event
/// dispatch — preceded by the frames this tick's drag stepped over (132차-C).
///
/// `r0` is the frame the game is about to dispatch (`ldrsb` of the counter). When the
/// record from [`handle_lgt`] says the clock was dragged to exactly that frame of exactly
/// this pose, the skipped frames `from..frame` are run through the game's own `0x4b92c`
/// first, oldest first; then the original call is made with the original `r0`, returning
/// past the 4-byte `bl`. `run_function` saves and restores every register, so the call the
/// game made sees the state it would have seen.
///
/// ⛔ The record is consumed here whatever happens — a record that did not match THIS
/// dispatch is stale by definition and must never fire later. With the stat off there is
/// never a record, and the handler is exactly the `bl` it replaced.
pub async fn handle_event_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    use core::sync::atomic::Ordering;
    let record = SKIPPED.swap(SKIP_NONE, Ordering::Relaxed);
    if record != SKIP_NONE {
        let frame = core.inner.lock().engine.reg_read(ArmRegister::R0) as i8 as i32;
        let mut pose = [0u8; 1];
        if core.read_bytes(POSE_ADDR, &mut pose).is_ok()
            && let Some(range) = replay_range(record, pose[0] as i8, frame)
        {
            let n = REPLAYS_REPORTED.fetch_add(1, Ordering::Relaxed);
            if n < REPORT_MAX {
                tracing::warn!(
                    "atk speed: pose {} frame events {:?} replayed before frame {frame} (the drag stepped over them)",
                    pose[0] as i8,
                    range
                );
            }
            for f in range {
                core.run_function::<()>(EVENT_FN, &[f as u32]).await?;
            }
        }
    }
    core.inner.lock().engine.reg_write(ArmRegister::LR, exit_pc);
    Ok(JumpTo(EVENT_FN))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recovery_countdown_matches_the_percent() {
        let mut carry = 0;
        for _ in 0..8 {
            assert_eq!(recover_decrement(0, &mut carry), 1, "off is the game's own step");
        }
        assert_eq!(carry, 0);

        for pct in [10u32, 25, 50, 75, 100, 200, MAX_PCT] {
            let mut carry = 0;
            let mut ticks = 0u32;
            let runs = 200;
            for _ in 0..runs {
                // One recovery: the byte starts at 4 and the action ends when it hits 0.
                let mut left = 4i32;
                while left > 0 {
                    left -= recover_decrement(pct, &mut carry) as i32;
                    ticks += 1;
                    assert!(ticks < runs * 8, "the countdown never ran out at {pct} %");
                }
            }
            let want = 4.0 * runs as f64 * 100.0 / (100 + pct) as f64;
            let got = ticks as f64;
            assert!(got >= want - runs as f64, "pct {pct}: {got} ticks is FASTER than {want}");
            assert!(got <= want + runs as f64, "pct {pct}: {got} ticks, wanted about {want}");
            assert!(got <= 4.0 * runs as f64, "pct {pct} made the recovery LONGER");
            assert!(got >= runs as f64, "pct {pct}: a recovery still costs at least one tick");
        }
    }

    /// The step is never 0 — that would make the countdown immortal.
    #[test]
    fn the_recovery_step_is_never_zero() {
        let mut carry = 0;
        for pct in 0..=MAX_PCT {
            assert!(recover_decrement(pct, &mut carry) >= 1, "pct {pct}");
        }
    }

    /// ⛔ THE REGRESSION THIS FILE EXISTS FOR: the counter may never end up above the
    /// length the pose was just given, because `counter == frames` is an exact test and a
    /// counter past it never meets it again — no completed pose, no hit test, no damage.
    #[test]
    fn the_counter_can_never_step_over_the_live_length() {
        for frames in 1..=12i32 {
            for counter in 0..=12i32 {
                for pct in [0u32, 1, 25, 50, 99, 100, 127, 200, 300, MAX_PCT, MAX_PCT + 9000] {
                    let mut carry = 0;
                    let extra = pose_extra(pct, &mut carry);
                    let next = advanced_counter(counter, frames, extra);
                    assert!(next <= frames, "c{counter} f{frames} {pct}% -> {next} OVERSHOT");
                    if counter <= frames {
                        assert!(next >= counter, "c{counter} f{frames} {pct}% went BACKWARDS");
                    }
                }
            }
        }
    }

    /// A pose whose length shrank between ticks — the exact shape of the shipped bug (a
    /// different weapon, a different facing, the next step of a skill's script).
    #[test]
    fn a_pose_that_got_shorter_still_completes() {
        // The previous pose was 8 long, this one is 1. The shipped code jumped the counter
        // to 4 and the character froze in state 2 forever; here a 1-tick pose is left
        // exactly as the game wrote it.
        let mut carry = 0;
        let extra = pose_extra(MAX_PCT, &mut carry);
        assert_eq!(advanced_counter(0, 1, extra), 0, "a 1-tick pose is untouched");
        assert_eq!(advanced_counter(1, 1, extra), 1, "already there, left alone");
        // And a counter that is somehow already past gets put back on the value the
        // state machine is waiting for rather than being left to spin forever.
        assert_eq!(advanced_counter(9, 1, extra), 1, "unfreezes");
    }

    /// ⛔ THE SECOND REGRESSION: a pose that COMPLETES on the tick it starts loses the hit.
    /// However big the bonus, the counter stops one short and the guest's own `+= 1`
    /// finishes the job — so every pose that had more than one tick still has two.
    #[test]
    fn a_pose_never_completes_on_the_tick_it_starts() {
        for frames in 2..=12i32 {
            for pct in [100u32, 127, 200, 300, MAX_PCT, MAX_PCT + 9000] {
                let mut carry = 0;
                let extra = pose_extra(pct, &mut carry);
                let next = advanced_counter(0, frames, extra);
                assert!(next < frames, "f{frames} {pct}% finished the swing in one tick");
                // …and it really does move, otherwise the stat would buy nothing.
                assert!(next >= 1, "f{frames} {pct}% did not advance at all");
            }
        }
    }

    /// Off is byte-identical to the original: no extra, so no write.
    #[test]
    fn off_is_the_games_own_timing() {
        let mut carry = 0;
        for frames in 1..=8i32 {
            for counter in 0..=8i32 {
                assert_eq!(pose_extra(0, &mut carry), 0);
                assert_eq!(
                    advanced_counter(counter, frames, 0),
                    if counter > frames { frames } else { counter },
                    "c{counter} f{frames}"
                );
            }
        }
    }

    /// A bonus under +100 % still buys something, spread across ticks.
    #[test]
    fn a_small_bonus_is_spread_across_ticks() {
        let mut carry = 0;
        let got: u32 = (0..100).map(|_| pose_extra(50, &mut carry)).sum();
        assert_eq!(got, 50, "+50 % is one extra tick every other tick");
        let mut carry = 0;
        let got: u32 = (0..100).map(|_| pose_extra(127, &mut carry)).sum();
        assert_eq!(got, 127);
    }

    #[test]
    fn only_the_attack_states_are_scaled() {
        assert!(is_action_state(2), "the pose script");
        assert!(is_action_state(4), "the follow-through");
        for s in [0u8, 1, 3, 5, 6, 7, 8, 9, 10, 11, 16] {
            assert!(!is_action_state(s), "state {s} is not an attack");
        }
    }

    #[test]
    fn the_displaced_instruction_and_addresses_are_pinned() {
        assert!(check_head([0x00, 0x06], 0x4af13).is_ok(), "lsls r0,r0,#0x18");
        assert!(check_head([0x00, 0x16], 0x4af13).is_err(), "asrs is the NEXT instruction");
        assert_eq!(PLAYER_STATE_ADDR, 0x0150_d63e);
        assert_eq!(POSE_COUNTER_ADDR, 0x0150_d650);
    }

    /// ★★★ 132차-C: the frames a drag steps over are exactly the ones replayed, and only
    /// for the dispatch of the frame the drag stopped on, of the pose it was made for.
    #[test]
    fn a_dragged_tick_replays_exactly_the_frames_it_stepped_over() {
        // Pose 3 (6 frames, hits on 1 and 4): +400 % drags 0 -> 4 in the tick it is set.
        let r = skip_record(3, 0, 4);
        assert_eq!(replay_range(r, 3, 4), Some(0..4), "frames 0..3, then the game's own 4");
        // Any other dispatch replays nothing: another pose, another frame, a stale record.
        assert_eq!(replay_range(r, 4, 4), None);
        assert_eq!(replay_range(r, 3, 5), None);
        assert_eq!(replay_range(r, 3, 3), None);
        assert_eq!(replay_range(SKIP_NONE, 3, 4), None);
        // A clock that did not move forward skipped nothing.
        assert_eq!(skip_record(3, 4, 4), SKIP_NONE);
        assert_eq!(skip_record(3, 5, 4), SKIP_NONE);
        assert_eq!(skip_record(3, -1, 4), SKIP_NONE, "a counter below 0 is not a frame");
        // Negative pose ids survive the packing.
        assert_eq!(replay_range(skip_record(-2, 1, 3), -2, 3), Some(1..3));
    }

    /// The replay plus the game's own dispatch covers every frame the pose passes through,
    /// once — the invariant that keeps a faster combo's damage equal to a slow one's.
    #[test]
    fn replay_and_dispatch_visit_every_frame_once() {
        for frames in 1..=12i32 {
            for pct in [0u32, 25, 50, 100, 127, 200, 300, MAX_PCT] {
                let mut carry = 0;
                let mut seen = alloc::vec::Vec::new();
                // The setPose tick: the counter starts at 0, then each tick the guest adds 1.
                let mut counter = 0i32;
                let mut first = true;
                loop {
                    if !first {
                        counter += 1;
                    }
                    first = false;
                    if counter == frames {
                        break; // completes: the next pose begins
                    }
                    let extra = pose_extra(pct, &mut carry);
                    let next = advanced_counter(counter, frames, extra);
                    if let Some(r) = replay_range(skip_record(3, counter, next), 3, next) {
                        seen.extend(r);
                    }
                    seen.push(next); // the game's own dispatch
                    counter = next;
                }
                let want: alloc::vec::Vec<i32> = (0..frames).collect();
                assert_eq!(seen, want, "frames {frames} at {pct} %");
            }
        }
    }
}
