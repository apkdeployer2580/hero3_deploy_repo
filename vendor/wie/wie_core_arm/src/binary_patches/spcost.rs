//! Skill SP-cost fixes for the raised level cap (2026-09-22).
//!
//! ── WHY A SKILL'S SP COST WENT NEGATIVE AT Lv99 ──────────────────────────────
//! The game computes a skill's SP cost as a PERCENTAGE that shrinks with level:
//!
//! ```text
//! cost = baseSP * (100 - level - spCostReduction) / 100
//! ```
//!
//! `level` is `CHAR+0xe2` read as `i16`, `spCostReduction` is the derived stat written
//! by `addStat(0x0e)` — the 현자의 orb / "SP소모 감소" gear (`CHAR+0x11e`). The `100 -
//! level` term is ORIGINAL game code (skills get cheaper as you level); the original
//! single-play cap was 70, so `100 - 70 = 30` was the floor and you needed +30% reduction
//! to zero it. 111차 raised the cap to 99 (`binary_patches.toml` LEVEL CAP), so `100 - 99
//! = 1` and any SP-cost-reduction gear drives the multiplier below zero — the reported
//! "스킬 SP 소모가 마이너스".
//!
//! Two guest sites compute this, both reading `level` with an `ldrsh`:
//!   * the SPEND path `0x4b864` (single caller `0x5004c`) — it already floors the amount
//!     it deducts to 5 (`0x4b8d0 cmp r0,#4 -> movs #5`), so SP never actually rises;
//!   * the skill window's "SP소모:%d" DISPLAY `0x43dxx` (`0x43e50` computes the number,
//!     format string `0x5deb0`) — which has NO floor, so it prints the raw negative.
//!
//! ── WHAT THESE HOOKS DO (player decision 2026-09-22) ─────────────────────────
//!   1. [`handle_level_spend_lgt`] / [`handle_level_disp_lgt`] clamp the `level` fed into
//!      the SP formula to [`LEVEL_FOR_SP`] (70, the original cap) at BOTH sites. The real
//!      level (`CHAR+0xe2`) is untouched — HUD, save, damage and EXP still see 99. So the
//!      SP curve is the one the game was tuned for, and `100 - 70 = 30` is the floor again.
//!   2. [`handle_disp_floor_lgt`] floors the DISPLAYED number at what the spend path
//!      actually deducts, IN THE DISPLAY'S OWN UNITS (the user's "실제차감치랑 동일하게")
//!      — which is the game's own "0" for any cost below one displayed point.
//!
//! ── THE TWO PATHS DO NOT SHARE A UNIT (★ 129차, 128차 code review ④) ────────────
//! The spend path subtracts its number from the SP word itself (`CHAR+0xf8`, 0..1000 —
//! `0x4b8da ldr r3,[r6,#0x44]` is `base+0x8c+0x44 = CHAR+0xf8`), and floors THAT at 5
//! (`0x4b8d0 cmp r0,#4 / bgt / movs r3,#5`). The display path takes the same
//! `baseSP*pct/100` and divides it by 10 ONCE MORE (`0x43e54 movs r1,#0xa` → `__divsi3`
//! at `0x43e56`) before `sprintf("SP소모:%d")` — and the game's own SP bar is drawn as
//! `(CHAR+0xf8)/10` over 100. So the number the player sees is the SP word in TENTHS:
//! "SP소모:6" is 60 of the word, 6 points of the 100-point bar. The spend floor of 5 is
//! therefore HALF A DISPLAYED POINT, and the first revision of this file, which floored
//! the display at 5, showed ten times the real deduction in the underflow case — and
//! worse, turned a legitimate "SP소모:4" (40 of the word, no underflow at all) into "5".
//!
//! The displayed floor is therefore [`DISP_FLOOR`] = `5 / 10 = 0`: exactly what the game
//! already shows for every sub-point cost (integer division — the starting 단도 reads
//! "SP소모:0" in the original and costs 5 of the word), and what the SP bar does. The
//! spend floor is the game's own and is left alone.
//!
//! ⛔ SETTLED BY THE USER (2026-09-22), do not reopen: SP is shown as 100 in the stats,
//! so the cost is shown on that scale; the 5-of-1000 minimum is "0", the way the original
//! shows every skill that "does not consume" (*"기존처럼 아예 소모안하던 스킬들은 똑같이
//! 0으로 표기하면돼"*). A "0.5" tenths display was built and tried on the probe (it read
//! 단도 as "SP소모:0.5", the truth to the tenth) and rejected for changing what those
//! skills always showed; raising the real floor to 50 so that a "5" would be true was
//! rejected as a balance change to the original.
//!
//! ── WHY REGISTERS ARE HARD-CODED PER SITE ────────────────────────────────────
//! The install writes an SVC over the two displaced bytes, so the handler can no longer
//! read the original `ldrsh` to decode its registers. The install-time head check (which
//! runs BEFORE the SVC is written) pins the exact instruction instead, and each site's
//! (base, offset, destination) registers are fixed here:
//!   * spend  `0x4b8b0`: `ldrsh r3,[r6,r0]`  (r6 = getBase()+0x8c, r0 = 0x2e -> &level)
//!   * display `0x43e36`: `ldrsh r4,[r0,r5]`  (r0 = getBase()+0xba -> &level, r5 = 0)
//!   * display `0x43e5c`: `adds r2,r0,#0` — r0 = the displayed integer, r2 = sprintf's arg.
//! Ships on BOTH flavours: for level <= 70 the clamp is a no-op, and the display floor is
//! a strict improvement wherever the same formula can underflow.

use alloc::format;

use wie_util::{Result, WieError, read_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// The level the SP-cost formula is clamped to — the original single-play cap.
pub const LEVEL_FOR_SP: i32 = 70;
/// The floor the SPEND path applies (`0x4b8d0 cmp r0,#4 -> movs r3,#5`), in the units of
/// the SP word it is subtracted from (`CHAR+0xf8`, 0..1000).
pub const SPEND_FLOOR_SP: i32 = 5;
/// SP-word units per DISPLAYED point: the display path divides by 10 once more than the
/// spend path (`0x43e54..0x43e56`), and the SP bar is `(CHAR+0xf8)/10` over 100.
pub const SP_PER_DISPLAY_POINT: i32 = 10;
/// The displayed floor — the spend floor in display units (`5 / 10 = 0`), the same
/// truncation the game applies to every sub-point cost. ⛔ Not 5: that is 50 of the SP
/// word, ten times what is deducted (see the module doc).
pub const DISP_FLOOR: i32 = SPEND_FLOOR_SP / SP_PER_DISPLAY_POINT;

/// `ldrsh r3,[r6,r0]` at `0x4b8b0` — the spend site's displaced instruction.
pub const SPEND_HEAD: [u8; 2] = [0x33, 0x5e];
/// `ldrsh r4,[r0,r5]` at `0x43e36` — the display site's displaced instruction.
pub const DISP_HEAD: [u8; 2] = [0x44, 0x5f];
/// `adds r2,r0,#0` at `0x43e5c` — the display floor's displaced instruction.
pub const DISP_FLOOR_HEAD: [u8; 2] = [0x02, 0x1c];

fn check(head: [u8; 2], want: [u8; 2], what: &str, pc: u32) -> Result<()> {
    if head == want {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "{what} hook at {pc:#x}: expected {want:02x?}, found {head:02x?}"
    )))
}

pub fn check_spend_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, SPEND_HEAD, "sp_level_spend", pc)
}
pub fn check_disp_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, DISP_HEAD, "sp_level_disp", pc)
}
pub fn check_disp_floor_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, DISP_FLOOR_HEAD, "sp_disp_floor", pc)
}

/// The clamped level value, pure. `level` is 1..=`LEVEL_CAP`; below 70 it is unchanged,
/// at/above 70 it becomes 70. A garbage negative is pinned to the low end (larger cost,
/// never a free skill).
pub fn level_for_sp(level: i16) -> u32 {
    (level as i32).clamp(1, LEVEL_FOR_SP) as u32
}

/// The displayed SP number, pure: the game's own display value (`baseSP*pct/100/10`,
/// possibly negative — the underflow the player reported) floored at [`DISP_FLOOR`].
/// Never below what the spend path deducts, expressed in the display's units; a value
/// already at or above the floor is shown exactly as the game computed it.
pub fn floored_display(value: i32) -> u32 {
    value.max(DISP_FLOOR) as u32
}

/// Re-do a displaced `ldrsh <dst>,[<base>,<off>]` with the loaded level clamped to
/// [`LEVEL_FOR_SP`]. The displaced load's own operand is the one read allowed to fault,
/// exactly where the game's `ldrsh` would have.
async fn clamp_level(core: &mut ArmCore, base: ArmRegister, off: ArmRegister, dst: ArmRegister, exit_pc: u32) -> Result<JumpTo> {
    let addr = {
        let inner = core.inner.lock();
        inner.engine.reg_read(base).wrapping_add(inner.engine.reg_read(off))
    };
    let level: i16 = read_generic(core, addr)?;
    core.inner.lock().engine.reg_write(dst, level_for_sp(level));
    Ok(JumpTo(exit_pc))
}

/// Spend `0x4b8b0`: `ldrsh r3,[r6,r0]` -> r3 = min(level, 70). The spend fn keeps its own
/// `<= 4 -> 5` floor after this.
pub async fn handle_level_spend_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    clamp_level(core, ArmRegister::R6, ArmRegister::R0, ArmRegister::R3, exit_pc).await
}

/// Display `0x43e36`: `ldrsh r4,[r0,r5]` -> r4 = min(level, 70).
pub async fn handle_level_disp_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    clamp_level(core, ArmRegister::R0, ArmRegister::R5, ArmRegister::R4, exit_pc).await
}

/// Display `0x43e5c`: the displaced `adds r2,r0,#0`, but with the value floored at
/// [`DISP_FLOOR`] so a build that still underflows shows the game's own "0" for a
/// sub-point cost instead of a negative number.
pub async fn handle_disp_floor_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let mut inner = core.inner.lock();
    let value = inner.engine.reg_read(ArmRegister::R0) as i32;
    inner.engine.reg_write(ArmRegister::R2, floored_display(value));
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_clamped_to_seventy_only_above_it() {
        assert_eq!(level_for_sp(1), 1);
        assert_eq!(level_for_sp(30), 30);
        assert_eq!(level_for_sp(70), 70);
        assert_eq!(level_for_sp(71), 70);
        assert_eq!(level_for_sp(99), 70);
        // garbage lower bound never becomes a free (huge-u32) skill
        assert_eq!(level_for_sp(0), 1);
        assert_eq!(level_for_sp(-5), 1);
    }

    #[test]
    fn the_display_floor_is_the_spend_floor_in_display_units() {
        // 5 of the SP word is half a displayed point: the display truncates it to 0, the
        // way it truncates every other sub-point cost (the original's 단도 reads "0").
        assert_eq!(DISP_FLOOR, 0);
        assert_eq!(SPEND_FLOOR_SP / SP_PER_DISPLAY_POINT, DISP_FLOOR);
        // An underflow (the negative the player reported) is clamped to that floor.
        assert_eq!(floored_display(-30), 0);
        assert_eq!(floored_display(-1), 0);
        assert_eq!(floored_display(0), 0);
        // ⛔ A legitimate small cost is shown as computed — the first revision turned a
        //   real "SP소모:4" (40 of the word) into "5".
        assert_eq!(floored_display(1), 1);
        assert_eq!(floored_display(4), 4);
        assert_eq!(floored_display(5), 5);
        assert_eq!(floored_display(6), 6);
        assert_eq!(floored_display(150), 150);
        assert!(check_disp_floor_head([0x02, 0x1c], 0x43e5d).is_ok(), "adds r2,r0,#0");
        assert!(check_disp_floor_head([0x0a, 0x21], 0x43e55).is_err(), "movs r1,#0xa is not this site");
    }
}
