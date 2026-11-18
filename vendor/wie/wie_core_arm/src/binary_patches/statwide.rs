//! 방어% 상한 500 and attack / defence past the i16 (160차, 2026-10-03).
//!
//! `TOWER_100_CLEAR_AUDIT.md` T1 · T2, the user's answers: *"T1 - HP나 공격력처럼 MAX치
//! 상항자체를 해제할꺼야 32767 넘을수있어야해. T2 - 방어 % 상한을 둘꺼야."* (→ 500).
//!
//! ── the four getters ──────────────────────────────────────────────────────────
//! The wear recompute (`0x4f014`) asks four leaf functions for the character's derived
//! stats and stores each as a halfword:
//!
//! ```text
//! 0x4cdb4 / 0x4ce64  ATT1 / ATT2 = item power × (100 + 힘|정신 + 물공%|특공%) / 100
//! 0x4cf64 / 0x4cfa4  P.DEF / M.DEF = (armour + 10) × (100 + 물방%|특방%) / 100 + 체력
//!
//! 0x4cf72  ldrsh r3,[r0,r2]     ; the 물방% sum (stat+0x54; 0x4cfb2 = 특방%, stat+0x58)   <- CAP
//!   …
//! 0x4cf92  lsls r0,r0,#0x10     ; ┐ the result truncated to i16 — past 32767 it goes   <- WIDE
//! 0x4cf94  asrs r0,r0,#0x10     ; ┘ NEGATIVE (0x4ce4e / 0x4cefe / 0x4cfd2 are the same pair)
//! ```
//!
//! **CAP** re-does the `ldrsh` with the sum capped at `DEF_PCT_CAP`: a full 금강/정령 orb
//! is +225 % on EVERY worn item, so three of them took 1 damage from every monster in the
//! tower and a fourth wrapped the halfword (전설 방어구 + 정령 ×4 = 37,391 → −28,145, every
//! hit then 32,767).
//!
//! **WIDE** replaces the truncation with a saturation — the game's halfword (and so every
//! screen that prints it) tops out at 32,767 — and, when the caller is the recompute's
//! FINAL pass (not the equip preview, which calls the same getters for the candidate item),
//! publishes the true value to `wie_util::stats::PLAYER_WIDE`. The saved `lr` on the
//! getter's own frame says which caller it is.
//!
//! ── the two combat readers ────────────────────────────────────────────────────
//! ```text
//! 0x4bdac / 0x4be12  ldrsh r2,[r3,r0]   ; player → monster: ATT1/ATT2 (stat+0x4a / +0x4c)
//! 0x38556            ldrsh r6,[r3,r2]   ; monster → player: P.DEF / M.DEF (stat+0x52 / +0x56)
//! ```
//!
//! Each is re-done through `stats::player_wide`: the published value when the halfword is
//! what that value saturates to, the halfword itself otherwise — so a field rewritten
//! behind the hooks (a record loaded before its first recompute) is believed over a stale
//! publication, and an in-range stat is byte-identical to the original game. The damage
//! arithmetic after both loads is already 32-bit (102차).
//!
//! ★ 163차: MAIN ONLY (`flavor = "main"` in the TOML) — the 500 cap bit on vanilla too (금강 /
//! 정령 are original orbs), and the player keeps every 개정판 balance patch off vanilla.

use alloc::format;
use core::sync::atomic::{AtomicI32, Ordering};

use wie_util::{
    Result, WieError, read_generic,
    stats::{self, WIDE_ATK1, WIDE_ATK2, WIDE_MDEF, WIDE_PDEF},
};

use super::playerhp::STAT_BLOCK;
use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldrsh r3,[r0,r2]` — the defence % load in both defence getters.
pub const PCT_HEAD: [u8; 2] = [0x83, 0x5e];
/// `lsls r0,r0,#0x10` + `asrs r0,r0,#0x10` — the i16 truncation at each getter's tail.
pub const TAIL_HEAD: [u8; 4] = [0x00, 0x04, 0x00, 0x14];
/// `ldrsh r6,[r3,r2]` — the monster hit's defence load.
pub const DEF_READ_HEAD: [u8; 2] = [0x9e, 0x5e];
/// `ldrsh r2,[r3,r0]` — the player hit's attack load (both sites).
pub const ATK_READ_HEAD: [u8; 2] = [0x1a, 0x5e];

/// The four halfwords, in the stat block (`0x4f40e..0x4f460` builds exactly these pointers).
pub const ATK1_ADDR: u32 = STAT_BLOCK + 0x4a;
pub const ATK2_ADDR: u32 = STAT_BLOCK + 0x4c;
pub const PDEF_ADDR: u32 = STAT_BLOCK + 0x52;
pub const MDEF_ADDR: u32 = STAT_BLOCK + 0x56;

/// The 물방% getter's cap site (the other one is the 특방% getter's) — for the log only.
const PDEF_PCT_SITE: u32 = 0x4cf72;

/// One getter's tail: where the hook sits, which stat it is, where that function's own
/// prologue left `lr` relative to `sp` at the tail, and the `lr` of the recompute's final
/// pass (the only caller whose result becomes the character's stat).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Getter {
    pub pc: u32,
    pub slot: usize,
    pub lr_off: u32,
    pub final_lr: u32,
}

/// `0x4cdb4` / `0x4ce64` open with `push {r4-r7,lr}` + `sub sp,#4` (lr at sp+0x14);
/// `0x4cf64` / `0x4cfa4` with `push {r4,r5,lr}` (lr at sp+8). The final pass calls them at
/// `0x4f736` · `0x4f750` · `0x4f76e` · `0x4f78c` (the preview's calls are `0x4f4d0` ·
/// `0x4f4ee` · `0x4f59e` · `0x4f5e6` · `0x4f626`).
pub const GETTERS: [Getter; 4] = [
    Getter { pc: 0x4ce4e, slot: WIDE_ATK1, lr_off: 0x14, final_lr: 0x4f73b },
    Getter { pc: 0x4cefe, slot: WIDE_ATK2, lr_off: 0x14, final_lr: 0x4f755 },
    Getter { pc: 0x4cf92, slot: WIDE_PDEF, lr_off: 0x08, final_lr: 0x4f773 },
    Getter { pc: 0x4cfd2, slot: WIDE_MDEF, lr_off: 0x08, final_lr: 0x4f791 },
];

pub fn getter_at(hook_pc: u32) -> Option<Getter> {
    GETTERS.iter().copied().find(|g| g.pc == hook_pc & !1)
}

/// Which wide slot a combat reader's address is, if it is one of the player's four.
pub fn slot_of(addr: u32) -> Option<usize> {
    match addr {
        ATK1_ADDR => Some(WIDE_ATK1),
        ATK2_ADDR => Some(WIDE_ATK2),
        PDEF_ADDR => Some(WIDE_PDEF),
        MDEF_ADDR => Some(WIDE_MDEF),
        _ => None,
    }
}

fn check<const N: usize>(head: [u8; N], want: [u8; N], what: &str, pc: u32) -> Result<()> {
    if head == want {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "{what} hook at {pc:#x}: expected {want:02x?}, found {head:02x?}"
    )))
}

pub fn check_pct_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, PCT_HEAD, "def_pct_cap", pc)
}

pub fn check_tail_head(head: [u8; 4], pc: u32) -> Result<()> {
    check(head, TAIL_HEAD, "stat_wide", pc)?;
    if getter_at(pc).is_none() {
        return Err(WieError::FatalError(format!(
            "stat_wide hook at {pc:#x}: not one of the four getter tails"
        )));
    }
    Ok(())
}

pub fn check_def_read_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, DEF_READ_HEAD, "def_read_wide", pc)
}

pub fn check_atk_read_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, ATK_READ_HEAD, "atk_read_wide", pc)
}

/// The last sum each defence getter capped (for a log line on a change, not per call —
/// the equip preview runs the getters on every cursor move).
static CAP_LOGGED: [AtomicI32; 2] = [AtomicI32::new(0), AtomicI32::new(0)];

/// `0x4cf72` / `0x4cfb2`: re-do `ldrsh r3,[r0,r2]` with the defence % sum capped.
pub async fn handle_def_pct_cap_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let addr = {
        let inner = core.inner.lock();
        inner.engine.reg_read(ArmRegister::R0).wrapping_add(inner.engine.reg_read(ArmRegister::R2))
    };
    let sum: i16 = read_generic(core, addr)?;
    let capped = stats::capped_def_pct(sum);
    if capped != i32::from(sum) {
        let which = usize::from(hook_pc & !1 != PDEF_PCT_SITE);
        if CAP_LOGGED[which].swap(i32::from(sum), Ordering::Relaxed) != i32::from(sum) {
            tracing::warn!("def pct cap: {} {sum} % -> {capped} %", if which == 0 { "물방" } else { "특방" });
        }
    }
    core.inner.lock().engine.reg_write(ArmRegister::R3, capped as u32);
    Ok(JumpTo(exit_pc))
}

/// A getter's tail: saturate instead of truncating, and publish the true value when the
/// caller is the recompute's final pass.
pub async fn handle_stat_wide_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (wide, sp) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::R0) as i32, inner.engine.reg_read(ArmRegister::SP))
    };
    let sat = stats::sat_i16(wide);
    // FAIL SOFT: an unreadable frame or an unknown site only loses the publication — the
    // halfword is saturated either way, which is never worse than the game's own wrap.
    if let Some(g) = getter_at(hook_pc) {
        if read_generic::<u32, _>(core, sp.wrapping_add(g.lr_off)).ok() == Some(g.final_lr) {
            let before = stats::set_player_wide(g.slot, wide);
            if before != wide && (wide != sat || stats::sat_i16(before) != before) {
                tracing::warn!(
                    "stat wide: {} {wide} (the game's halfword holds {sat}; combat reads the true value)",
                    ["공격력(주)", "공격력(보조)", "물리방어", "특수방어"][g.slot]
                );
            }
        }
    }
    core.inner.lock().engine.reg_write(ArmRegister::R0, sat as u32);
    Ok(JumpTo(exit_pc))
}

async fn wide_read(core: &mut ArmCore, base: ArmRegister, index: ArmRegister, dest: ArmRegister, exit_pc: u32) -> Result<JumpTo> {
    let addr = {
        let inner = core.inner.lock();
        inner.engine.reg_read(base).wrapping_add(inner.engine.reg_read(index))
    };
    let guest: i16 = read_generic(core, addr)?;
    let value = match slot_of(addr) {
        Some(slot) => stats::player_wide(slot, guest),
        None => i32::from(guest),
    };
    core.inner.lock().engine.reg_write(dest, value as u32);
    Ok(JumpTo(exit_pc))
}

/// `0x38556`: re-do `ldrsh r6,[r3,r2]` — the defence a monster's hit is reduced by.
pub async fn handle_def_read_wide_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    wide_read(core, ArmRegister::R3, ArmRegister::R2, ArmRegister::R6, exit_pc).await
}

/// `0x4bdac` / `0x4be12`: re-do `ldrsh r2,[r3,r0]` — the attack the player's hit starts from.
pub async fn handle_atk_read_wide_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    wide_read(core, ArmRegister::R3, ArmRegister::R0, ArmRegister::R2, exit_pc).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hooks_only_install_over_their_displaced_instructions() {
        assert!(check_pct_head([0x83, 0x5e], 0x4cf73).is_ok(), "ldrsh r3,[r0,r2]");
        assert!(check_pct_head([0x24, 0x14], 0x4cf75).is_err(), "the asrs after it is the exit");
        for g in GETTERS {
            assert!(check_tail_head([0x00, 0x04, 0x00, 0x14], g.pc | 1).is_ok());
        }
        assert!(check_tail_head([0x00, 0x04, 0x00, 0x14], 0x4cf85).is_err(), "the right bytes at the wrong site");
        assert!(check_tail_head([0x00, 0x04, 0x00, 0x0c], 0x4cf93).is_err(), "lsls/LSRS is another truncation");
        assert!(check_def_read_head([0x9e, 0x5e], 0x38557).is_ok(), "ldrsh r6,[r3,r2]");
        assert!(check_def_read_head([0x00, 0x22], 0x38555).is_err(), "the movs r2,#0 before it");
        assert!(check_atk_read_head([0x1a, 0x5e], 0x4bdad).is_ok(), "ldrsh r2,[r3,r0]");
        assert!(check_atk_read_head([0x4b, 0x5f], 0x4bdb1).is_err(), "the skill coefficient load after it");
    }

    /// The four tails, their frames and the final pass's return addresses — the things the
    /// publication depends on, pinned against the disassembly.
    #[test]
    fn the_four_getters_are_the_recomputes_final_calls() {
        assert_eq!(GETTERS.map(|g| g.slot), [WIDE_ATK1, WIDE_ATK2, WIDE_PDEF, WIDE_MDEF]);
        // bl at 0x4f736 / 0x4f750 / 0x4f76e / 0x4f78c, each 4 bytes, Thumb bit set in lr
        assert_eq!(GETTERS.map(|g| g.final_lr), [0x4f736 + 5, 0x4f750 + 5, 0x4f76e + 5, 0x4f78c + 5]);
        // push {r4-r7,lr} + sub sp,#4 for the attack pair; push {r4,r5,lr} for the defence pair
        assert_eq!(GETTERS.map(|g| g.lr_off), [4 * 4 + 4, 4 * 4 + 4, 2 * 4, 2 * 4]);
        assert_eq!(getter_at(0x4cf93).map(|g| g.slot), Some(WIDE_PDEF), "the hook pc carries the Thumb bit");
        assert_eq!(getter_at(0x4cf73), None, "the cap site is not a tail");
    }

    #[test]
    fn the_readers_only_widen_the_players_four_halfwords() {
        assert_eq!(STAT_BLOCK, 0x0150_3b3c);
        assert_eq!(slot_of(0x0150_3b86), Some(WIDE_ATK1));
        assert_eq!(slot_of(0x0150_3b88), Some(WIDE_ATK2));
        assert_eq!(slot_of(0x0150_3b8e), Some(WIDE_PDEF), "base+0xde, the monster hit's physical arm");
        assert_eq!(slot_of(0x0150_3b92), Some(WIDE_MDEF), "base+0xe2");
        assert_eq!(slot_of(0x0150_3b90), None, "stat+0x54 is the 물방% SUM, not a stat");
        assert_eq!(slot_of(0x4014_6070), None, "a monster record");
    }
}
