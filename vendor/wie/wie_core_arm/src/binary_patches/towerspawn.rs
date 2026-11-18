//! Tower spawn cap — a 하늘성 floor spawns exactly its count, not the room's twenty.
//!
//! ── WHY A HOOK ───────────────────────────────────────────────────────────────
//! The tower's halls carry twenty spawn points (`gen_tower_rooms.HALL_SPAWNS`), and a
//! floor's `count` column asks for fewer (138차: the fewest that reach the floor's HP
//! target, 3..20). Until 145차 the host trimmed the difference AFTER the scene loaded:
//! it wrote 0 to the extras' HP (`guardian::park_indices`), and the game ran its own
//! death for them — the player watched monsters fall over on arrival without being
//! touched (user, 2026-09-30: *"다음층으로 넘어가면 갑자기 몬스터들이 입장하자마자 내가
//! 때리지도 않았는데 죽는 버그"*; S9 log: `floor 2 — parked 7/20 extra monster(s)` 0.24 s
//! after landing, then their respawn timers armed every poll). Nothing to do with the
//! map's terrain: every spawn point sits on a walkable tile (`gen_tower_rooms.verify`).
//!
//! The spawn LIST is built by one loop inside the map parser (`0x1587e`..`0x158ee`, part
//! of `loadScene`, no host tick inside): for each spawn point it reads `type, x, y`,
//! takes `species = words[type]` (`base+0x9e3c`), and at `0x158b0 cmp r0,#0xff; beq
//! 0x158e0` skips a point whose species slot is empty — the game's own "no monster
//! here". Otherwise it appends the point and bumps `MAP+0x34` (`[r7+0x34]`, mirrored in
//! `[sp+0xa4]`), the count everything after the loop — the record array, the AI, the
//! draw loop — is sized by (`0x158f0`).
//!
//! So the cap is that branch: once the list already holds `cap` monsters, a further
//! point takes the empty-slot path. The extras are never created — no record, no death,
//! no respawn timer — and the kept set is exactly the one the old parking kept (the
//! first `cap` non-empty points in file order, which is also the order the floor
//! generator summed their HP in).
//!
//! ── THE SITE ─────────────────────────────────────────────────────────────────
//! `0x158b0 cmp r0,#0xff` (`ff 28`), displaced. The hook performs the compare by CHOOSING
//! the resume point: `exit_pc` = `0x158b4` (the append, i.e. the not-equal fall-through
//! of the `beq` it also skips) or `skip_pc` = `0x158e0` (the `beq` target). Neither path
//! reads the flags before setting its own (`0x158c4 lsls` / `0x158e4 adds`), which is
//! what makes choosing the destination equivalent to the compare-and-branch — the
//! `reaver_drop_lgt` hook has shipped the same shape since 75차.
//!
//! ── THE CONTRACT ─────────────────────────────────────────────────────────────
//! * The host arms `wie_util::stats::TOWER_SPAWN_CAP` with `(scene, cap)` next to the
//!   species triple, before it posts the warp. Keyed on the SCENE the game is loading
//!   (`(u16)[base+0x9e08]`, stored by `loadScene` before the header and the map), so a
//!   `Leave` into the origin town is never touched.
//! * Frame check: `[sp+0xc0]` is the base the loop indexes the species words from, and
//!   `[sp+0xa4]` equals `[r7+0x34]` (the loop stores both on every append). A moved site
//!   fails that and the hook does exactly what the displaced compare did.
//! * An empty slot (`0xff`) is skipped whatever the knob says — the game's own rule.
//!
//! ── FAIL SOFT ────────────────────────────────────────────────────────────────
//! Unset knob, another scene, a failed frame check, unreadable memory: the compare is
//! performed as the game wrote it. The worst outcome is a floor with more monsters than
//! its table asked for — never a monster that dies on its own.

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use wie_util::{Result, WieError, read_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `cmp r0, #0xff` at `0x158b0` — the instruction the hook displaces.
pub const EXPECTED_HEAD: [u8; 2] = [0xff, 0x28];

/// LGT: CHAR is a static at `.bss 0x1503a88`, and `getBase()` returns `CHAR + 0x28`.
const GUEST_BASE: u32 = 0x0150_3a88 + 0x28;
/// `(u16)[base+0x9e08]` — the scene id `loadScene` stores before it parses anything.
const SCENE_ID_OFF: u32 = 0x9e08;
/// `[sp+0xc0]` — the base the loop adds `0x9e3c + type*4` to (`0x15882`).
const SP_BASE: u32 = 0xc0;
/// `[sp+0xa4]` — how many spawns the list holds so far (`0x158d4`..`0x158dc`).
const SP_KEPT: u32 = 0xa4;
/// `[sp+0xa8]` — the loop's spawn-point index (`0x158e0`..`0x158ea`).
const SP_POINT: u32 = 0xa8;
/// `[r7+0x34]` — `MAP+0x34`, the spawned count the loop mirrors `[sp+0xa4]` into.
const MAP_SPAWNED_OFF: u32 = 0x34;
/// The species byte of an empty slot.
const SPECIES_NONE: u32 = 0xff;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "tower_spawn_cap hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (cmp r0,#0xff), found {head:02x?}"
    )))
}

/// Where the loop goes for this spawn point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// Append it (the `beq` not taken).
    Append,
    /// The empty-slot path (the `beq` taken).
    Skip,
    /// The empty-slot path, because the floor's cap is reached — the hook's own doing.
    Capped,
}

/// The decision, pure. `armed` = the knob; `loading` = the scene being loaded;
/// `frame_ok` = the frame check passed; `kept` = how many the list already holds.
pub fn decide(species: u32, armed: Option<(u8, u8)>, loading: u16, frame_ok: bool, kept: u32) -> Resume {
    if species & 0xff == SPECIES_NONE {
        return Resume::Skip;
    }
    match armed {
        Some((scene, cap)) if frame_ok && u16::from(scene) == loading && kept >= u32::from(cap) => Resume::Capped,
        _ => Resume::Append,
    }
}

/// The last capped point this process logged, `scene << 16 | point`: one line per load
/// (the first capped point), not one per skipped point. A new load of any scene starts
/// its loop at a point index no greater than the last one logged.
static LAST_LOGGED: AtomicU32 = AtomicU32::new(u32::MAX);

/// Hook body: perform the displaced compare by choosing the resume point.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32, skip_pc: u32) -> Result<JumpTo> {
    let (species, r7, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R7),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    let armed = wie_util::stats::tower_spawn_cap();
    // Read the frame only when a cap is armed: an unarmed hook must cost the game
    // nothing but the compare, and must never fail a load on a read.
    let (loading, frame_ok, kept, point) = if armed.is_some() && species & 0xff != SPECIES_NONE {
        let loading: u16 = read_generic(core, GUEST_BASE + SCENE_ID_OFF).unwrap_or(u16::MAX);
        let base: u32 = read_generic(core, sp + SP_BASE).unwrap_or(0);
        let kept: u32 = read_generic(core, sp + SP_KEPT).unwrap_or(u32::MAX);
        let mirrored: u32 = read_generic(core, r7 + MAP_SPAWNED_OFF).unwrap_or(u32::MAX - 1);
        let point: u32 = read_generic(core, sp + SP_POINT).unwrap_or(0);
        let frame_ok = base == GUEST_BASE && kept == mirrored;
        if !frame_ok {
            tracing::warn!(
                "tower spawn cap: frame check failed (base {base:#x}, kept {kept} vs MAP+0x34 {mirrored}); the game's own compare only"
            );
        }
        (loading, frame_ok, kept, point)
    } else {
        (u16::MAX, false, 0, 0)
    };
    match decide(species, armed, loading, frame_ok, kept) {
        Resume::Append => Ok(JumpTo(exit_pc)),
        Resume::Skip => Ok(JumpTo(skip_pc)),
        Resume::Capped => {
            let key = (u32::from(loading) << 16) | (point & 0xffff);
            let last = LAST_LOGGED.swap(key, Ordering::Relaxed);
            // The first capped point of a load: a different scene, or a point index that
            // did not move forward (a new loop over the same scene).
            if last == u32::MAX || last >> 16 != key >> 16 || (last & 0xffff) >= (key & 0xffff) {
                // warn: the Android build filters the wie crates to warn, and "how many
                // did the floor spawn" is the first question a count report raises.
                tracing::warn!(
                    "tower spawn cap: scene {loading} holds its {kept} monster(s) — spawn point #{point} and any after it are skipped"
                );
            }
            Ok(JumpTo(skip_pc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_cmp_r0_0xff() {
        assert!(check_head([0xff, 0x28], 0x158b1).is_ok());
        assert!(check_head([0x28, 0xff], 0x158b1).is_err());
    }

    #[test]
    fn an_empty_slot_is_skipped_whatever_the_knob_says() {
        assert_eq!(decide(0xff, None, 243, true, 0), Resume::Skip);
        assert_eq!(decide(0xff, Some((243, 5)), 243, true, 0), Resume::Skip);
        // Only the low byte is the species (`ldrb`).
        assert_eq!(decide(0x1ff, None, 243, true, 0), Resume::Skip);
    }

    #[test]
    fn unarmed_is_the_game_itself() {
        for kept in [0, 5, 19, 40] {
            assert_eq!(decide(18, None, 243, true, kept), Resume::Append);
        }
    }

    #[test]
    fn the_cap_keeps_the_first_n_and_skips_the_rest() {
        let armed = Some((244, 13));
        for kept in 0..13 {
            assert_eq!(decide(42, armed, 244, true, kept), Resume::Append, "kept {kept}");
        }
        assert_eq!(decide(42, armed, 244, true, 13), Resume::Capped);
        assert_eq!(decide(42, armed, 244, true, 14), Resume::Capped);
    }

    #[test]
    fn another_scene_or_a_moved_frame_is_left_alone() {
        let armed = Some((244, 3));
        // The origin town a `Leave` loads.
        assert_eq!(decide(42, armed, 111, true, 10), Resume::Append);
        // A build where the site moved: the frame check failed.
        assert_eq!(decide(42, armed, 244, false, 10), Resume::Append);
    }

    #[test]
    fn a_cap_of_zero_would_spawn_nothing_but_the_host_never_arms_one() {
        // `Floor::count` is 1..=20 by construction (`Floor::valid`); the hook itself does
        // not second-guess the knob.
        assert_eq!(decide(42, Some((244, 0)), 244, true, 0), Resume::Capped);
    }
}
