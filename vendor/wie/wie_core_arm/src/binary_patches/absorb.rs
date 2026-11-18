//! 흡수 (HP 흡혈) on a hit past 32767 — 163차 (2026-10-05).
//!
//! The player's report: *"소울마스터 암흑하고 업화 스킬이 흡수 적용이 안되고 패시브스킬은 흡혼도
//! 적용이 안되고있어, 흡수오브도 적용안돼."* Three sources, one funnel: the 흡혼 passive, the
//! 흡혈의 오브 (addStat id 0x10) and every new item's 흡수 option all add into the stat block's
//! `+0x6e` halfword, and exactly one function turns that percent into HP:
//!
//! ```text
//! computeAbsorb 0x4b978(block, i16 dmg):
//!     a = (i16) (((i16)[block+0x6e]) * dmg / 200)
//!     a < 0              -> 0
//!     a > [block+0x38]/5 -> [block+0x38]/5          ; at most 20 % of max HP
//! callers (the only two `bl`s in the image):
//!     0x4e20e  ldr r0,[sp,#0x38]; lsls r1,r0,#16; asrs r1,#16; bl 0x4b978; …; bl addHP   (0x4de90, every
//!              frame-event hit: basic attacks AND skills — 암흑 · 업화)
//!     0x4e936  lsls r1,r7,#16; asrs r1,#16; bl 0x4b978; …; bl addHP                       (the projectile twin)
//! ```
//!
//! ── WHY IT STOPPED ───────────────────────────────────────────────────────────
//! 102차 made the player's damage 32-bit end to end (cap 9,999,999) and took the folds out of
//! both callers' `[sp+0x38]` / `r7` — but not the two folds in front of THIS call. A hit of
//! 32768..65535 reached `computeAbsorb` NEGATIVE (absorb 0), 65536+ as the low half (a few HP
//! at random). Headless (scene 243, absorb % 200, HP 1000/30000, OK held 800 frames): a small
//! attack healed 1000 → 1584; the same run with the attack past 32767 killed monsters every
//! swing and healed **0** (998 → 998).
//!
//! ── THE FIX ──────────────────────────────────────────────────────────────────
//! Two byte patches hand the full 32-bit damage to the call (`binary_patches.toml` «ABSORB»),
//! and this hook replaces the function: the same formula in 64 bits, the cap at 20 % of the
//! TRUE max HP (142차 keeps the gauge at `block+0x38`; above 32767 the true max lives in
//! `wie_util::stats`, and `addHP` applies a heal in true units — see `playerhp`), and a result
//! the callers' `lsls/asrs #16` before `addHP` carries unchanged (<= 32767). With a hit and a
//! max inside the i16 every answer is the original function's. Both flavours (vanilla's hits
//! stop at 9,999 and its max HP stays far under 32767, so there it is the original function).
//!
//! FAIL SOFT: an unreadable block absorbs nothing — the hit still lands.

use alloc::format;

use wie_util::{Result, WieError, read_generic, stats};

use super::playerhp::{BLOCK_HP_MAX, STAT_BLOCK};
use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `push {r4, r5, r6, r7, lr}` at `0x4b978` — computeAbsorb's first instruction.
pub const HEAD: [u8; 2] = [0xf0, 0xb5];
/// `block + 0x6e` — the summed 흡수 percent (addStat id 0x10), i16.
pub const BLOCK_ABSORB_PCT: u32 = 0x6e;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!("absorb hook at {pc:#x}: expected {HEAD:02x?}, found {head:02x?}")))
}

/// The HP a hit gives back, pure: `pct × dmg / 200` (truncated toward zero like the guest's
/// signed divide), nothing when that is negative, at most a fifth of `max_hp`, and never past
/// the i16 the callers hand to `addHP`.
pub fn absorb_amount(pct: i16, dmg: i32, max_hp: i64) -> i16 {
    let raw = i64::from(pct) * i64::from(dmg) / 200;
    if raw < 0 {
        return 0;
    }
    raw.min(max_hp / 5).clamp(0, i64::from(i16::MAX)) as i16
}

/// `0x4b978` replaced: answers in `r0` and returns to `lr` (nothing was pushed).
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32) -> Result<JumpTo> {
    let (block, dmg, lr) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1) as i32,
            inner.engine.reg_read(ArmRegister::LR),
        )
    };
    let read = (
        read_generic::<i16, _>(core, block + BLOCK_ABSORB_PCT),
        read_generic::<i32, _>(core, block + BLOCK_HP_MAX),
    );
    let amount = match read {
        (Ok(pct), Ok(gmax)) => {
            let max = if block == STAT_BLOCK {
                i64::from(stats::hp_true_max(gmax.max(0) as u32, stats::PLAYER_HP_MAX.load(core::sync::atomic::Ordering::Relaxed)))
            } else {
                i64::from(gmax)
            };
            let a = absorb_amount(pct, dmg, max);
            if dmg > i32::from(i16::MAX) && a > 0 && LOGGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 3 {
                tracing::warn!("absorb: a hit of {dmg} at {pct} % gives back {a} HP (max {max}) — past the i16 the shipped call folded");
            }
            a
        }
        _ => 0,
    };
    core.inner.lock().engine.reg_write(ArmRegister::R0, amount as i32 as u32);
    Ok(JumpTo(lr))
}

/// Big-hit absorbs logged so far (the first few prove the fix on a device log).
static LOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped function, bit for bit: `muls` in 32 bits, the signed divide, the fold to
    /// i16, `< 0 → 0`, the signed compare against `hp_max / 5`.
    fn shipped(pct: i16, dmg: i16, hp_max: i32) -> i16 {
        let prod = i32::from(pct).wrapping_mul(i32::from(dmg));
        let a = (prod / 200) as i16;
        if a < 0 {
            return 0;
        }
        let cap = hp_max / 5;
        if i32::from(a) > cap { cap as i16 } else { a }
    }

    #[test]
    fn the_hook_only_installs_over_the_push() {
        assert!(check_head([0xf0, 0xb5], 0x4b979).is_ok());
        assert!(check_head([0x00, 0xb5], 0x4b979).is_err(), "addHP's push {{lr}} is not this function");
    }

    /// Inside the i16 the hook IS the shipped function — every percent the game can sum,
    /// every damage it could produce before 102차, every max the gauge can hold. (Where the
    /// shipped quotient itself wrapped — 270 % of 32767 — the shipped answer was 0 and the
    /// hook's the capped share: that wrap is the same bug one step later.)
    #[test]
    fn inside_the_i16_every_answer_is_the_shipped_one() {
        for pct in [-50i16, 0, 1, 10, 30, 90, 200, 270, 1000] {
            for dmg in [0i16, 1, 2, 7, 100, 999, 9_999, 20_000, 32_767] {
                if (i32::from(pct) * i32::from(dmg) / 200).abs() > i32::from(i16::MAX) {
                    continue;
                }
                for hp_max in [1i32, 167, 5_570, 20_000, 32_767] {
                    assert_eq!(absorb_amount(pct, i32::from(dmg), i64::from(hp_max)), shipped(pct, dmg, hp_max), "pct {pct} dmg {dmg} max {hp_max}");
                }
            }
        }
    }

    /// The report: a hit past 32767 gave nothing (or a random sliver) — now the percent of it,
    /// capped as the game caps it.
    #[test]
    fn a_hit_past_the_i16_absorbs_its_share() {
        // what the folded call computed: (i16)40_000 = -25_536 -> 0; (i16)100_000 = -31_072 -> 0
        assert_eq!(shipped(200, 40_000u32 as i16, 30_000), 0);
        assert_eq!(shipped(200, 100_000u32 as i16, 30_000), 0);
        assert_eq!(shipped(30, 70_000u32 as i16, 30_000), 669, "65536 past: the low half, a sliver");
        // the hook
        assert_eq!(absorb_amount(200, 40_000, 30_000), 6_000, "capped at a fifth of max");
        assert_eq!(absorb_amount(30, 70_000, 30_000), 6_000);
        assert_eq!(absorb_amount(10, 40_000, 30_000), 2_000, "10 % of 40000 / 2");
        assert_eq!(absorb_amount(2, 9_999_999, 30_000), 6_000, "the resolver's own cap");
    }

    #[test]
    fn no_product_overflows_and_no_result_leaves_the_i16() {
        assert_eq!(absorb_amount(i16::MAX, 9_999_999, i64::from(u32::MAX)), i16::MAX, "the addHP delta stays an i16");
        assert_eq!(absorb_amount(400, 9_999_999, 100_000), 20_000, "400 % x 9,999,999 overflows an i32");
        assert_eq!(absorb_amount(-30, 9_999_999, 100_000), 0, "a negative percent takes nothing");
        assert_eq!(absorb_amount(30, -5, 100_000), 0);
    }

    /// Above the gauge the cap is 20 % of the TRUE max (addHP heals in true units there).
    #[test]
    fn the_cap_is_a_fifth_of_the_true_max() {
        assert_eq!(absorb_amount(200, 100_000, 60_000), 12_000, "60k true max: 12k, not the gauge's 6,553");
        assert_eq!(absorb_amount(200, 100_000, 32_767), 6_553);
    }
}
