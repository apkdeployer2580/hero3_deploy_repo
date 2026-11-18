//! Two stat-sum caps the game lacks — 명중 255 and 쿨타임 감소 70 %.
//!
//! `ITEM_BALANCE_AUDIT_156.md §8` P1 · P2, taken as recommended by the user («전부 권장대로»).
//!
//! ── P1: the 명중 sum WRAPS at 256 ─────────────────────────────────────────────
//! The wear recompute (`0x4f014`) sums passives + worn 명중 options into a halfword and
//! then stores it as a BYTE:
//!
//! ```text
//! 0x4f7c6  ldrh r3,[r7]          ; the halfword sum
//! 0x4f7c8  ldr  r0,[sp,#0x10]    ; &stat+0x2c
//! 0x4f7ca  strb r3,[r0]          ; (u8) — 256 folds to 0            <- HOOK (displaced)
//! 0x4f7cc  ldr  r1,[sp,#8]
//! 0x4f7ce  lsls r3,r3,#0x18 ; lsrs r3,r3,#0x18   ; r3 = (u8) r3
//! 0x4f7d2  strh r3,[r1]          ; a halfword copy of the SAME byte
//! ```
//!
//! and the player-hit roll reads it back with `ldrb` (`0x4bf64`). So 코르버스 4 (150) +
//! 검은공장의걸작 (90) + 조준 (20) = 260 reads as 4: stacking 명중 past 255 made the player
//! MISS more (hell 65 → 49 %, the tower's 15 % floor). The hook re-does the `strb` with the
//! value clamped at [`HIT_SUM_CAP`] and leaves r3 clamped too, so the game's own
//! `lsls/lsrs` + `strh` right after it stores the same 255. A sum of 0..=255 is
//! byte-identical. A NEGATIVE halfword (no shipped row or passive can make one) keeps the
//! game's own truncation — this is a cap, not a redefinition.
//!
//! ── P2: the 쿨타임 감소 sum has NO ceiling ──────────────────────────────────────
//! The cast path (`0x4fdb8` → `0x4bb82`) sets a skill's cooldown to `desc × (100 − sum) /
//! 100`:
//!
//! ```text
//! 0x4bbd4  adds r0,r7,#0 ; adds r0,#0xfc   ; r0 = base+0xfc = CHAR+0x124 (addStat(0x11) word)
//! 0x4bbda  ldrb r1,[r3,#0x1c]              ; desc's base cooldown
//! 0x4bbde  movs r3,#0
//! 0x4bbe0  ldrsh r2,[r0,r3]                ; the sum, signed                 <- HOOK (displaced)
//! 0x4bbe2  movs r3,#0x64 ; subs r3,r3,r2   ; 100 − sum   (signed — 100+ gives <= 0)
//! 0x4bbe6  adds r0,r3,#0 ; muls r0,r1,r0 ; … / 100
//! ```
//!
//! and the countdown (`0x4a30c`) treats a non-positive length as one tick. The new items
//! carry 25..45 each (섬광의군화 45 + 오딘의손길 38 + 굴베이그의탈리스만 30 = 113 %), so two
//! or three of them were a skill with no cooldown at all. The hook re-does the displaced
//! `ldrsh` with the sum clamped at [`COOLDOWN_REDUCE_CAP_PCT`] (70 — 30 % of the base
//! cooldown). The host's cooldown ring reads the same word through the same cap
//! (`wie_android::items::read_cd_reduce_pct`), so the two agree.
//!
//! ★ 163차: MAIN ONLY (`flavor = "main"` in the TOML). The old line here — "vanilla has no row
//! that can reach either" — was wrong: the ORIGINAL 폭풍의 오브 (addStat 0x11, strength 6) is
//! +90 % 쿨타임 per fully socketed item and 사신의 오브 (0x09, strength 8) +120 % 명중, so
//! both caps changed vanilla play, and the player wants vanilla on the game's own rules.
//! The only memory each touches is the displaced instruction's own operand, so a fault lands
//! exactly where the game's would.

use alloc::format;

use wie_util::{
    ByteWrite, Result, WieError, read_generic,
    stats::{COOLDOWN_REDUCE_CAP_PCT, HIT_SUM_CAP},
};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `strb r3,[r0]` at `0x4f7ca` — the 명중 store's displaced instruction.
pub const HIT_HEAD: [u8; 2] = [0x03, 0x70];
/// `ldrsh r2,[r0,r3]` at `0x4bbe0` — the cooldown sum's displaced load.
pub const CD_HEAD: [u8; 2] = [0xc2, 0x5e];

fn check(head: [u8; 2], want: [u8; 2], what: &str, pc: u32) -> Result<()> {
    if head == want {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "{what} hook at {pc:#x}: expected {want:02x?}, found {head:02x?}"
    )))
}

pub fn check_hit_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, HIT_HEAD, "hit_sum_clamp", pc)
}

pub fn check_cd_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, CD_HEAD, "cooldown_cap", pc)
}

/// What the displaced `strb` stores and what r3 holds afterwards, pure: `r3` as the
/// `ldrh` left it (a halfword in the low 16 bits). A sum above [`HIT_SUM_CAP`] stores the
/// cap and hands the cap on to the `strh` copy; anything else is the game's own byte and
/// the game's own r3 (a negative halfword included — the cap does not redefine it).
pub fn clamped_hit_store(r3: u32) -> (u8, u32) {
    let sum = (r3 & 0xffff) as u16 as i16;
    if i32::from(sum) > HIT_SUM_CAP {
        (HIT_SUM_CAP as u8, HIT_SUM_CAP as u32)
    } else {
        ((r3 & 0xff) as u8, r3)
    }
}

/// The cooldown-reduction sum the cast formula sees, pure: the game's signed halfword,
/// capped at [`COOLDOWN_REDUCE_CAP_PCT`]. Below the cap (negatives included) it is unchanged.
pub fn capped_cooldown(sum: i16) -> i32 {
    i32::from(sum).min(COOLDOWN_REDUCE_CAP_PCT)
}

/// `0x4f7ca`: re-do `strb r3,[r0]` with the 명중 sum clamped, then resume at the `ldr`
/// that follows (the `lsls/lsrs/strh` copy sees the clamped r3).
pub async fn handle_hit_clamp_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (r3, r0) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::R3), inner.engine.reg_read(ArmRegister::R0))
    };
    let (byte, r3_after) = clamped_hit_store(r3);
    if byte as u32 != (r3 & 0xff) {
        tracing::warn!("hit sum clamp: 명중 {} -> {} (the byte at {r0:#x} would have wrapped)", (r3 & 0xffff) as u16 as i16, byte);
    }
    core.write_bytes(r0, &[byte])?;
    core.inner.lock().engine.reg_write(ArmRegister::R3, r3_after);
    Ok(JumpTo(exit_pc))
}

/// `0x4bbe0`: re-do `ldrsh r2,[r0,r3]` with the 쿨타임 감소 sum capped.
pub async fn handle_cooldown_cap_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let addr = {
        let inner = core.inner.lock();
        inner.engine.reg_read(ArmRegister::R0).wrapping_add(inner.engine.reg_read(ArmRegister::R3))
    };
    let sum: i16 = read_generic(core, addr)?;
    core.inner.lock().engine.reg_write(ArmRegister::R2, capped_cooldown(sum) as u32);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hooks_only_install_over_their_displaced_instructions() {
        assert!(check_hit_head([0x03, 0x70], 0x4f7cb).is_ok(), "strb r3,[r0]");
        assert!(check_hit_head([0x3b, 0x88], 0x4f7c7).is_err(), "the ldrh before it is not the site");
        assert!(check_cd_head([0xc2, 0x5e], 0x4bbe1).is_ok(), "ldrsh r2,[r0,r3]");
        assert!(check_cd_head([0x64, 0x23], 0x4bbe3).is_err(), "the movs r3,#0x64 after it is the exit");
    }

    /// 0..=255 is byte-identical to the game's own `strb`; 256+ stores 255 and hands 255 to
    /// the `strh` copy; a negative halfword keeps the game's truncation (not redefined).
    #[test]
    fn the_hit_sum_is_capped_at_the_byte_ceiling_and_nowhere_else() {
        assert_eq!(clamped_hit_store(0), (0, 0));
        assert_eq!(clamped_hit_store(40), (40, 40));
        assert_eq!(clamped_hit_store(255), (255, 255));
        assert_eq!(clamped_hit_store(256), (255, 255), "the wrap-to-0 case");
        assert_eq!(clamped_hit_store(260), (255, 255), "코르버스 4 + 걸작 + 조준 = 260 read as 4 before");
        assert_eq!(clamped_hit_store(0x7fff), (255, 255));
        // garbage above the halfword never reaches the byte or the compare
        assert_eq!(clamped_hit_store(0x0001_0104), (255, 255));
        // a negative sum: the game's own low byte, r3 untouched
        assert_eq!(clamped_hit_store(0xfff6), (0xf6, 0xfff6));
    }

    #[test]
    fn the_cooldown_sum_is_capped_at_seventy() {
        assert_eq!(COOLDOWN_REDUCE_CAP_PCT, 70, "user decision 2026-10-10");
        assert_eq!(capped_cooldown(0), 0);
        assert_eq!(capped_cooldown(45), 45, "섬광의군화 alone");
        assert_eq!(capped_cooldown(60), 60);
        assert_eq!(capped_cooldown(69), 69);
        assert_eq!(capped_cooldown(70), 70);
        assert_eq!(capped_cooldown(71), 70);
        assert_eq!(capped_cooldown(83), 70, "섬광 45 + 걸작 38");
        assert_eq!(capped_cooldown(113), 70, "섬광 + 오딘 + 굴베이그 — was a one-tick cooldown");
        assert_eq!(capped_cooldown(-20), -20, "a negative stays the game's own");
        // the formula the game applies to the capped value never reaches zero
        assert!(100 - capped_cooldown(i16::MAX) > 0);
    }
}
