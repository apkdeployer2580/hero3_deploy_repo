//! 32-bit damage — the two hooks that let a hit carry more than an `i16` end to end.
//!
//! ── WHERE THE 16 BITS WERE (2026-09-18) ──────────────────────────────────────
//! Every damage number in the shipped game is a signed halfword somewhere on its way:
//! the player's hit resolver `0x4bca0` sums up to three hits into a `u16` and folds the
//! sum to `i16` before its `<= 0 → 1` / cap compare (`0x4be76`), both callers fold the
//! return again (`0x4e128`, `0x4e860`), `hitMonster 0x3863c` takes the value as `u16`,
//! stores the DISPLAY copy with `strh` into `rec+0xaa`, negates it as `i16` for the HP
//! writer `0x38620`, and the floating-number drawer `0x1ff8` reads that copy with
//! `ldrsh`. A raw hit of 32768 or more therefore read as NEGATIVE at the fold and took
//! the "minimum damage" branch — the player saw 「1」 (headless: monsters at atk 32767
//! hit a def-0 player for 1; `--poke` of the fix turned the same frames into 32199).
//!
//! The byte patches in `binary_patches.toml` (`DAMAGE 32-BIT`) remove every fold on the
//! player→monster path, raise the cap literal to 9,999,999, move the display copy to a
//! 32-bit slot at `rec+0xfc` and make `0x1ff8` load it with `ldr`. The digit renderer
//! `0x1e6c` was 32-bit all along (`% 10` / `/= 10` into a 16-byte buffer; headless it
//! drew 「1234567」 and a real 「65534」). Two things a byte patch has no room for are
//! these hooks:
//!
//! * [`handle_apply_lgt`] at `0x386cc` (`ldr r2,[sp,#4]`, past the MISS branch and
//!   before `0x38620`): the record's HP is still an `i16`, so the value the game
//!   subtracts has to be clamped — and this is also where a monster's VIRTUAL HP lives
//!   now. `rec+0x100` holds the host-seeded REMAINING total and `rec+0x104` the total it
//!   started from (0 = no pool); the hook takes the raw hit off the remainder, keeps
//!   `hp_cur = 32767 * remaining / total` (never below 1) while anything remains and
//!   hands the game a zero delta, and lets the game kill the record normally once the
//!   pool is spent. PROPORTIONAL, not `min(remaining, 32767)`: the target box draws its
//!   bar as `43 * hp_cur / hp_max` with `hp_max` forked to 32767, so a proportional
//!   `hp_cur` makes a 200k-HP boss's bar drain exactly like any other monster's — the
//!   user's requirement (2026-09-18: "20만에서 딜 들어가는거에 따라서 HP바가 줄어든 비율대로",
//!   "박스 디자인이나 UI등은 아무런 변경없고"). All three slots were proven spare with
//!   sentinels (`+0xf8..+0x107` survived combat, kills and respawn re-init on 80/80
//!   records, frames pixel-identical).
//! * [`handle_disp_lgt`] at `0x1ff8` (function entry): the same drawer draws the
//!   PLAYER's number from `base+0x210`, a halfword whose neighbour `+0x212` is the
//!   timer, so the `ldr` patch would read `dmg | timer << 16` there. For the player
//!   (`r2 == 0`, the flag the three callers pass) the hook sign-extends the halfword into
//!   a scratch word in the `.bss` page tail and points `r3` at that instead. The drawer's
//!   expiry `str 0` lands in the scratch; the hook mirrors that back to the halfword when
//!   it sees the post-expiry state (`timer == kind == -1`), so the guest's own slot ends
//!   up exactly as the unpatched game left it.
//!
//! ── FAIL SOFT (made true 2026-09-18, 104차-B code review) ────────────────────
//! Every read of the RECORD (`hp_cur`, the two pool words) and of the player's slot and
//! the drawer's frame is allowed to fail: the apply hook then hands the game
//! `min(raw, 32767)` with no pool and touches nothing, the display hook leaves `r3` on the
//! game's own halfword, and each says so ONCE per process (`tracing::warn!` reaches the
//! release logcat from a vendor crate — 78차-C's "fail closed, but SAY SO"). The one read
//! that stays fatal is the displaced load's own operand, `[sp+4]`: the instruction the
//! hook replaces would have faulted on it just the same, so that is not a failure mode
//! the hook adds. (Until this pass every read was `?`, so an unreadable record was a
//! `tick error` and the worker returned — the header said soft and the code was hard.)
//! The hooks refuse to install on any head other than the instruction they displace.
//!
//! ── WHY "0 = NO POOL" HOLDS ACROSS SCENE LOADS (104차-B) ──────────────────────
//! Nothing clears `rec+0x100` on purpose, and nothing has to: the creature array is
//! `MC_knlCalloc`'d — the map loader's allocation at `0x373c2` goes through the `.data`
//! import stub `0x1400264` = `(0x1fb, 0x76)`, and `0x76` is `Calloc` (`wie_lgt`
//! `svc_ids.rs`; `wie_wipi_c::api::kernel::calloc` zero-fills). Every scene load starts
//! every record at 0, so a floor left mid-fight cannot hand its half-spent pool to the
//! next scene's monsters and the next floor's boss never inherits one. The respawn
//! RE-INIT is the one path that keeps the word (sentinel-measured), and a respawn only
//! ever reuses a KILLED record, whose spending hit set the word to 0.
//! Measured headlessly (2026-09-18, `wie_probe`): sentinels `efbeadde` / `0dd0feca`
//! written into `+0x100` / `+0x104` of all 20 records on scene 243, then a warp to 244 —
//! the array came back at the SAME address (`0x40146070`, exactly the "the next scene's
//! array sits where the old one was" case) and every record read `00000000` in both
//! words. The two array-pointer words `CHAR+0x9cb8` / `+0x9cbc` were equal throughout.

use alloc::format;
use core::sync::atomic::{AtomicBool, Ordering};

use wie_util::{Result, WieError, read_generic, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldr r2, [sp, #4]` at `0x386cc` — the instruction the apply hook displaces.
pub const APPLY_HEAD: [u8; 2] = [0x01, 0x9a];
/// `push {r4-r7, lr}` at `0x1ff8` — the instruction the display hook displaces.
pub const DISP_HEAD: [u8; 2] = [0xf0, 0xb5];

/// `rec + 0x7e` — the record's current HP, `i16`.
pub const HP_CUR_OFF: u32 = 0x7e;
/// `rec + 0x100` — the VIRTUAL HP word: what is still unspent (host-seeded, 0 = no pool).
pub const VHP_OFF: u32 = 0x100;
/// `rec + 0x104` — the total the virtual word started from (the bar's denominator).
pub const VHP_MAX_OFF: u32 = 0x104;
/// The most an `i16` HP field can hold, and therefore the most one delta may take.
pub const RECORD_CEILING: u32 = 32767;

/// `rec + 0x7c` — the record's max HP, `i16` (the 기습 test's denominator).
pub const HP_MAX_OFF: u32 = 0x7c;
/// `rec + 0x6c` / `+0x6e` — the record's effective physical / special defence, `i16`: what
/// the resolver `0x4bca0` subtracts per hit component (`0x4bdd4` for weapon categories
/// 4..6, `0x4be4c` for 7+). The 156차 방패무시 bonus gives part of it back.
pub const DEF_PHYS_OFF: u32 = 0x6c;
pub const DEF_SPEC_OFF: u32 = 0x6e;
/// The resolver's own cap (`0x4beac`, 102차) — a tripled 기습 hit stays under it too.
pub const DAMAGE_CAP: u32 = 9_999_999;

/// The player's display word: `.bss` page tail, past everything the ELF declares
/// (the save chain's scratch is `0x1512f00`; see `guest_anchors::save_chain`).
pub const PLAYER_SCRATCH: u32 = 0x0151_2ff0;

/// Whether an unreadable RECORD has been reported. Once per process — see the header.
static RECORD_UNREADABLE: AtomicBool = AtomicBool::new(false);
/// Whether an unreadable PLAYER slot / drawer frame has been reported. Once per process.
static PLAYER_UNREADABLE: AtomicBool = AtomicBool::new(false);

/// Say it once. A hook fires on every hit and a read that fails once fails every time,
/// so the first line carries all the information there is — and a line per hit would
/// bury the logcat the player is asked to send.
fn say_once(flag: &AtomicBool, line: impl FnOnce() -> alloc::string::String) {
    if !flag.swap(true, Ordering::Relaxed) {
        tracing::warn!("{}", line());
    }
}

pub fn check_apply_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == APPLY_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "dmg_apply hook at {pc:#x}: expected {APPLY_HEAD:02x?} (ldr r2,[sp,#4]), found {head:02x?}"
    )))
}

pub fn check_disp_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == DISP_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "dmg_disp hook at {pc:#x}: expected {DISP_HEAD:02x?} (push {{r4-r7,lr}}), found {head:02x?}"
    )))
}

/// What the apply hook does with one hit. Pure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Apply {
    /// The delta the game's `i16` HP writer gets (as a positive amount).
    pub applied: u32,
    /// `Some((hp_cur, vhp))` when the hook owns the record's HP this hit.
    pub pool: Option<(u16, u32)>,
}

/// The record HP that shows `remaining` out of `total` on a bar whose denominator is
/// [`RECORD_CEILING`]: proportional, never 0 while anything remains (0 is "dead" to the
/// game), never above the ceiling. A missing total (an old seed) falls back to
/// `min(remaining, ceiling)`.
pub fn bar_hp(remaining: u32, total: u32) -> u16 {
    if total == 0 || remaining >= total {
        return remaining.min(RECORD_CEILING) as u16;
    }
    let scaled = (u64::from(remaining) * u64::from(RECORD_CEILING) / u64::from(total)) as u32;
    scaled.clamp(1, RECORD_CEILING) as u16
}

/// The pool arithmetic, pure: `raw` is the resolver's 32-bit hit, `hp_cur` the record's
/// current HP, `vhp` the virtual total still unspent (INCLUDING what the record holds),
/// `total` what `vhp` was seeded with.
///
/// * No pool (`vhp == 0`): the game applies `min(raw, 32767)` — the record cannot hold
///   more, so a bigger hit is a kill either way.
/// * Pool with something left after this hit: the hook writes
///   `hp_cur = bar_hp(remaining, total)` itself and the game applies nothing, so the
///   target box's bar (`43 * hp_cur / 32767`) drains in proportion to the real HP.
/// * Pool spent by this hit: the pool word goes to 0 and the game applies
///   `min(raw, 32767)`, which is `>= hp_cur` (`hp_cur` is the remainder's share of the
///   ceiling and `raw >= remainder`), so the record dies through the game's own path
///   (AI state 7, loot).
/// * A record already at 0 is a corpse (a DoT tick can land on one): untouched.
pub fn apply(raw: u32, hp_cur: i16, vhp: u32, total: u32) -> Apply {
    let clamped = raw.min(RECORD_CEILING);
    if vhp == 0 || hp_cur <= 0 {
        return Apply { applied: clamped, pool: None };
    }
    let remaining = vhp.saturating_sub(raw);
    if remaining == 0 {
        return Apply { applied: clamped, pool: Some((hp_cur as u16, 0)) };
    }
    Apply { applied: 0, pool: Some((bar_hp(remaining, total), remaining)) }
}

/// What the apply hook does when the RECORD cannot be read: the game's own hit, clamped
/// for the `i16` writer, and no pool — the same answer a readable record without a pool
/// gets. Pure. (FAIL SOFT, header.)
pub fn apply_unreadable(raw: u32) -> Apply {
    Apply { applied: raw.min(RECORD_CEILING), pool: None }
}

/// The target's `(remaining, full)` HP, pure: the pool when the record has one
/// (`vhp / total`), the record's own halfwords otherwise. `(0, _)` = a corpse.
pub fn target_hp(hp_cur: i16, hp_max: i16, vhp: u32, total: u32) -> (u64, u64) {
    if hp_cur <= 0 {
        return (0, 1);
    }
    if vhp > 0 && total > 0 {
        return (u64::from(vhp.min(total)), u64::from(total));
    }
    let cur = hp_cur as u64;
    (cur, (hp_max.max(0) as u64).max(cur))
}

/// The damage-dealt set effects on a hit (142차 크루세이더 「기습」 · 142차-C 코르버스 「처형」 ·
/// 빙해 「빙해의 일격」 · 150차 the accessories' 주는 피해 +`dealt_pct` % — 검은 눈물), pure:
/// [`wie_util::stats::set_hit`] on the record's HP, under the resolver's cap. `None` =
/// unchanged.
pub fn set_multiplied(raw: u32, hp_cur: i16, hp_max: i16, vhp: u32, total: u32, mask: u32, dealt_pct: u32) -> Option<u32> {
    let (cur, max) = target_hp(hp_cur, hp_max, vhp, total);
    wie_util::stats::set_hit(raw, cur, max, mask, dealt_pct).map(|hit| hit.min(DAMAGE_CAP).max(raw))
}

/// Does this hit kill the record, pure: it was alive and the game is about to subtract at
/// least what it has left (`apply` gives the pooled record's last hit as `applied`, and
/// nothing — `applied == 0` — to a hit the pool absorbs).
pub fn kills(hp_cur: i16, applied: u32) -> bool {
    hp_cur > 0 && applied >= hp_cur as u32
}

/// The record's current HP and its two pool words, or the first read that failed.
fn read_pool(core: &mut ArmCore, rec: u32) -> Result<(i16, u32, u32)> {
    let hp_cur: i16 = read_generic(core, rec + HP_CUR_OFF)?;
    let vhp: u32 = read_generic(core, rec + VHP_OFF)?;
    let total: u32 = read_generic(core, rec + VHP_MAX_OFF)?;
    Ok((hp_cur, vhp, total))
}

/// `hitMonster`: clamp the 32-bit hit for the `i16` writer and run the pool.
///
/// Frame at `0x386cc`: `r5` = the record (`adds r5,r0,#0` at `0x3864a`), `[sp+4]` = the
/// raw hit (32-bit once `0x3863e`/`0x38642` are nops). Resumes at `exit_pc` with `r2`
/// holding what the displaced load would have loaded, minus the clamp.
pub async fn handle_apply_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (sp, rec) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(ArmRegister::R5))
    };
    // The displaced `ldr r2,[sp,#4]`'s own operand: fatal exactly where the game's own
    // instruction would have faulted, and nowhere else (FAIL SOFT, header).
    let mut raw: u32 = read_generic(core, sp + 4)?;
    // ★ 156차 방패무시 first — it restores defence the resolver took off BEFORE any multiplier,
    // so the set multipliers below (and the pool, the clamp, the floating number — `0x386fc`
    // stores `[sp+4]` into `rec+0xfc`) all see the pierced hit. FAIL SOFT: an unreadable
    // defence halfword leaves the hit alone.
    let (pierce_pct, special) = wie_util::stats::pierce();
    if pierce_pct > 0
        && raw > 1
        && let Ok(def) = read_generic::<i16, _>(core, rec + if special { DEF_SPEC_OFF } else { DEF_PHYS_OFF })
        && let Some(hit) = wie_util::stats::pierce_bonus(raw, def, pierce_pct)
    {
        write_generic(core, sp + 4, hit)?;
        if wie_util::stats::PIERCE_HITS.fetch_add(1, Ordering::Relaxed) < 12 {
            tracing::warn!(
                "방패무시: hit on rec {rec:#x} — {} defence {def} × {pierce_pct} % given back: {raw} -> {hit}",
                if special { "special" } else { "physical" }
            );
        }
        raw = hit;
    }
    // The set multipliers next: the multiplied hit is the hit — the pool, the clamp and
    // the floating number all see it.
    let mask = wie_util::stats::set_bonus();
    // 150차: the worn accessories' 주는 피해 +N % rides the same product (검은 눈물).
    let acc = wie_util::stats::acc_effects();
    const HIT_SETS: u32 = wie_util::stats::SET_AMBUSH | wie_util::stats::SET_EXECUTE | wie_util::stats::SET_ICE;
    if (mask & HIT_SETS != 0 || acc.dealt_pct > 0)
        && let Ok((hp_cur, vhp, total)) = read_pool(core, rec)
        && let Ok(hp_max) = read_generic::<i16, _>(core, rec + HP_MAX_OFF)
        && let Some(hit) = set_multiplied(raw, hp_cur, hp_max, vhp, total, mask, u32::from(acc.dealt_pct))
    {
        write_generic(core, sp + 4, hit)?;
        let n = wie_util::stats::SET_HITS.fetch_add(1, Ordering::Relaxed);
        if n < 20 {
            let (cur, max) = target_hp(hp_cur, hp_max, vhp, total);
            tracing::warn!(
                "set bonus: hit on rec {rec:#x} (hp {cur}/{max}, sets {mask:#x}, accessory dealt +{} %) {raw} -> {hit}",
                acc.dealt_pct
            );
        }
        raw = hit;
    }
    let a = match read_pool(core, rec) {
        Ok((hp_cur, vhp, total)) => {
            let a = apply(raw, hp_cur, vhp, total);
            if let Some((cur, left)) = a.pool {
                // The words were read a moment ago at these very addresses; a write that
                // fails here is not a readability question any more.
                if cur != hp_cur as u16 {
                    write_generic(core, rec + HP_CUR_OFF, cur)?;
                }
                write_generic(core, rec + VHP_OFF, left)?;
                tracing::trace!("dmg_apply: rec {rec:#x} raw {raw} -> pool {vhp} -> {left}, hp_cur {hp_cur} -> {cur}, applied {}", a.applied);
            }
            // 142차-C 마녀의 생명력 / 150차 레아의목걸이: a kill (re)starts the kill clock.
            if kills(hp_cur, a.applied)
                && let Some(until) = wie_util::stats::on_kill(mask, acc, wie_util::stats::play_clock_ms())
                && wie_util::stats::WITCH_KILLS.load(Ordering::Relaxed) <= 5
            {
                tracing::warn!(
                    "set bonus: kill clock — rec {rec:#x} killed; max HP +{} % until {until} ms (set {} % + accessory {} %)",
                    wie_util::stats::max_hp_bonus_pct(mask, acc, wie_util::stats::play_clock_ms(), until, 0),
                    if mask & wie_util::stats::SET_WITCH != 0 { wie_util::stats::WITCH_MAX_HP_PCT } else { 0 },
                    acc.kill_max_hp_pct
                );
            }
            a
        }
        Err(e) => {
            say_once(&RECORD_UNREADABLE, || {
                format!("dmg_apply: record {rec:#x} cannot be read ({e:?}); the hit goes through clamped with no virtual HP pool — this time and every later time")
            });
            apply_unreadable(raw)
        }
    };
    core.inner.lock().engine.reg_write(ArmRegister::R2, a.applied);
    Ok(JumpTo(exit_pc))
}

/// Which word the drawer should read, pure: the player's halfword is copied into the
/// scratch (sign-extended); a monster's `r3` already points at its 32-bit slot.
pub fn player_word(flag: u32, halfword: i16) -> Option<i32> {
    (flag == 0).then_some(i32::from(halfword))
}

/// Is the drawer's slot in the post-expiry state the unpatched game leaves
/// (`timer == -1`, `kind == -1`, and the number slot cleared to 0)?
pub fn expired(timer: i16, kind: i16, scratch: i32) -> bool {
    timer == -1 && kind == -1 && scratch == 0
}

/// `0x1ff8`: for the player, redirect the number pointer to a 32-bit scratch word.
///
/// Frame at entry: `r2` = flag (monsters 1, the player 0), `r3` = `&number`,
/// `[sp]` = `&kind`, `[sp+4]` = `&timer` (the caller's stack arguments 5 and 6).
pub async fn handle_disp_lgt(core: &mut ArmCore, hook_pc: u32) -> Result<JumpTo> {
    let (flag, num_ptr, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R2),
            inner.engine.reg_read(ArmRegister::R3),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    if flag == 0
        && let Err(e) = redirect_player(core, num_ptr, sp)
    {
        // `r3` stays on the game's own halfword and the patched `ldr` reads it as a word:
        // a wrong NUMBER for the player at worst, never a stopped game (FAIL SOFT, header).
        say_once(&PLAYER_UNREADABLE, || {
            format!("dmg_disp: the player's number slot {num_ptr:#x} or the drawer's frame cannot be read ({e:?}); drawing the game's own word")
        });
    }
    super::hook::resume_past_push(core, hook_pc)
}

/// Copy the player's halfword into the scratch word and point `r3` there. A read that
/// fails returns before anything is written or `r3` is moved, so the caller can simply
/// resume the game.
fn redirect_player(core: &mut ArmCore, num_ptr: u32, sp: u32) -> Result<()> {
    let halfword: i16 = read_generic(core, num_ptr)?;
    let Some(mut word) = player_word(0, halfword) else { return Ok(()) };
    // The drawer's expiry wrote 0 into the scratch last frame; give the guest's own
    // halfword the same 0 the unpatched game would have left there.
    let kind_ptr: u32 = read_generic(core, sp)?;
    let timer_ptr: u32 = read_generic(core, sp + 4)?;
    let scratch: i32 = read_generic(core, PLAYER_SCRATCH)?;
    let timer: i16 = read_generic(core, timer_ptr)?;
    let kind: i16 = read_generic(core, kind_ptr)?;
    if halfword > 0 && expired(timer, kind, scratch) {
        write_generic(core, num_ptr, 0i16)?;
        word = 0;
    }
    write_generic(core, PLAYER_SCRATCH, word)?;
    core.inner.lock().engine.reg_write(ArmRegister::R3, PLAYER_SCRATCH);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_pool_the_game_applies_the_hit_clamped_to_the_record() {
        assert_eq!(apply(100, 5000, 0, 0), Apply { applied: 100, pool: None });
        assert_eq!(apply(65534, 5000, 0, 0), Apply { applied: 32767, pool: None });
        assert_eq!(apply(9_999_999, 32767, 0, 0), Apply { applied: 32767, pool: None });
    }

    /// The bar is the real HP's share of the ceiling — a big boss drains like any
    /// other monster instead of sitting at full until the remainder fits.
    #[test]
    fn a_pooled_record_shows_its_remaining_share_on_the_bar() {
        let t = 112_000;
        // 40k off 112k leaves 72k = 64.3 % of the bar.
        assert_eq!(apply(40_000, 32767, t, t), Apply { applied: 0, pool: Some((21_064, 72_000)) });
        // Another 50k leaves 22k = 19.6 %.
        assert_eq!(apply(50_000, 21_064, 72_000, t), Apply { applied: 0, pool: Some((6_436, 22_000)) });
        // Never 0 while anything remains: 1 HP left on a 112k boss is still alive.
        assert_eq!(bar_hp(1, t), 1);
        assert_eq!(bar_hp(t, t), 32767);
        assert_eq!(bar_hp(t / 2, t), 16383);
        // 200k, the user's example: half dealt = half a bar.
        assert_eq!(bar_hp(100_000, 200_000), 16383);
        // An old seed without a total falls back to the raw remainder.
        assert_eq!(bar_hp(22_000, 0), 22_000);
        assert_eq!(bar_hp(100_000, 0), 32767);
    }

    #[test]
    fn the_hit_that_spends_the_pool_kills_through_the_game() {
        let a = apply(30_000, 6_436, 21_000, 112_000);
        assert_eq!(a.pool, Some((6_436, 0)), "pool word goes to 0, hp_cur untouched");
        assert!(a.applied >= 6_436, "the game's delta covers what the record holds");
        assert_eq!(a.applied, 30_000);
        // Exactly spent counts as spent.
        assert_eq!(apply(21_000, 6_436, 21_000, 112_000).pool, Some((6_436, 0)));
    }

    #[test]
    fn a_corpse_is_left_alone() {
        assert_eq!(apply(500, 0, 40_000, 40_000), Apply { applied: 500, pool: None });
        assert_eq!(apply(500, -3, 40_000, 40_000), Apply { applied: 500, pool: None });
    }

    /// FAIL SOFT (104차-B): an unreadable record is not a different game, only an unknown
    /// one — the hit goes through exactly as it would for a readable record without a pool.
    #[test]
    fn an_unreadable_record_gets_the_clamped_hit_and_no_pool() {
        assert_eq!(apply_unreadable(100), Apply { applied: 100, pool: None });
        assert_eq!(apply_unreadable(9_999_999), Apply { applied: 32767, pool: None });
        assert_eq!(apply_unreadable(65534), apply(65534, 5000, 0, 0));
    }

    #[test]
    fn every_point_of_a_pool_is_dealt_exactly_once() {
        let mut vhp = 100_000u32;
        let mut hp: i16 = 32767;
        let mut dealt = 0u32;
        loop {
            let a = apply(7_777, hp, vhp, 100_000);
            match a.pool {
                Some((cur, left)) if left > 0 => {
                    dealt += vhp - left;
                    vhp = left;
                    hp = cur as i16;
                }
                Some((_, 0)) => {
                    dealt += vhp;
                    break;
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(dealt, 100_000);
    }

    /// 기습: only with the set, only on a full record (pooled or not), capped, never less.
    /// ★ 158차 P4: ×2 (was ×3 — the 85제 set out-damaged every 90제 set in hell).
    #[test]
    fn ambush_doubles_the_first_hit_on_a_full_record() {
        assert_eq!(wie_util::stats::AMBUSH_MULT, 2, "158차 P4");
        let m = wie_util::stats::SET_AMBUSH;
        let f = |raw, cur, max, vhp, total, mask| set_multiplied(raw, cur, max, vhp, total, mask, 0);
        assert_eq!(f(1_000, 5_000, 5_000, 0, 0, m), Some(2_000));
        assert_eq!(f(1_000, 5_000, 5_000, 0, 0, 0), None, "no set");
        assert_eq!(f(1_000, 4_999, 5_000, 0, 0, m), None, "already hurt");
        assert_eq!(f(1_000, 32_767, 32_767, 200_000, 200_000, m), Some(2_000), "an untouched pool");
        assert_eq!(f(1_000, 32_767, 32_767, 199_000, 200_000, m), None, "a spent pool");
        assert_eq!(f(5_000_000, 5_000, 5_000, 0, 0, m), Some(DAMAGE_CAP));
        assert_eq!(f(1_000, 0, 5_000, 0, 0, m), None, "a corpse");
    }

    #[test]
    fn execute_doubles_hits_on_a_record_at_half_hp_or_less() {
        let m = wie_util::stats::SET_EXECUTE;
        let f = |raw, cur, max, vhp, total| set_multiplied(raw, cur, max, vhp, total, m, 0);
        assert_eq!(f(1_000, 2_500, 5_000, 0, 0), Some(2_000), "exactly 50 %");
        assert_eq!(f(1_000, 2_501, 5_000, 0, 0), None, "just above");
        assert_eq!(f(1_000, 1, 5_000, 0, 0), Some(2_000), "one HP left");
        assert_eq!(f(1_000, 0, 5_000, 0, 0), None, "a corpse");
        assert_eq!(f(1_000, 32_767, 32_767, 100_000, 200_000), Some(2_000), "the pool decides, not the bar");
        assert_eq!(f(1_000, 32_767, 32_767, 100_001, 200_000), None);
        assert_eq!(f(1_000, 5_000, 5_000, 0, 0), None, "a full record");
    }

    #[test]
    fn ice_multiplies_every_hit_by_one_and_a_half() {
        let m = wie_util::stats::SET_ICE;
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, m, 0), Some(1_500));
        assert_eq!(set_multiplied(1_000, 10, 5_000, 0, 0, m, 0), Some(1_500), "hurt or not");
        assert_eq!(set_multiplied(101, 5_000, 5_000, 0, 0, m, 0), Some(151), "rounded down");
        assert_eq!(set_multiplied(1, 5_000, 5_000, 0, 0, m, 0), None, "1 x 1.5 is still 1");
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, 0, 0), None, "no set");
        assert_eq!(set_multiplied(9_000_000, 5_000, 5_000, 0, 0, m, 0), Some(DAMAGE_CAP));
    }

    #[test]
    fn the_hit_sets_stack_under_the_cap() {
        let all = wie_util::stats::SET_AMBUSH | wie_util::stats::SET_EXECUTE | wie_util::stats::SET_ICE;
        // Full HP: ambush x2 and ice x1.5, execute cannot apply.
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, all, 0), Some(3_000));
        // Low HP: execute x2 and ice x1.5.
        assert_eq!(set_multiplied(1_000, 100, 5_000, 0, 0, all, 0), Some(3_000));
        assert_eq!(set_multiplied(9_000_000, 100, 5_000, 0, 0, all, 0), Some(DAMAGE_CAP));
    }

    /// 150차 검은 눈물 「주는 피해 +25%」: ×(100 + p)/100 on top of the sets, rounded down,
    /// never less than the raw hit, under the cap — and on its own with no set worn.
    #[test]
    fn the_accessory_dealt_bonus_multiplies_on_top_of_the_sets() {
        let ice = wie_util::stats::SET_ICE;
        let amb = wie_util::stats::SET_AMBUSH;
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, 0, 25), Some(1_250), "no set, the accessory alone");
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, ice, 25), Some(1_875), "빙해 x1.5 then x1.25");
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, amb | ice, 25), Some(3_750), "기습 x2, 빙해 x1.5, x1.25");
        assert_eq!(set_multiplied(3, 5_000, 5_000, 0, 0, 0, 25), None, "3 x 1.25 rounds back to 3");
        assert_eq!(set_multiplied(4, 5_000, 5_000, 0, 0, 0, 25), Some(5));
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, 0, 50), Some(1_500), "two copies");
        assert_eq!(set_multiplied(9_000_000, 5_000, 5_000, 0, 0, 0, 25), Some(DAMAGE_CAP));
        assert_eq!(set_multiplied(1_000, 0, 5_000, 0, 0, 0, 25), None, "a corpse");
        assert_eq!(set_multiplied(1_000, 5_000, 5_000, 0, 0, 0, 0), None, "nothing worn");
    }

    /// A kill is the hit the game subtracts the record's last HP with: a plain record's
    /// clamped hit reaching its HP, a pooled record's last hit (the pool's own arithmetic).
    #[test]
    fn a_kill_is_the_hit_that_empties_the_record() {
        assert!(kills(500, 500));
        assert!(kills(500, 32_767));
        assert!(!kills(500, 499));
        assert!(!kills(0, 100), "a corpse cannot die again");
        let hurt = apply(1_000, 32_767, 200_000, 200_000);
        assert!(!kills(32_767, hurt.applied), "the pool absorbs a hit that leaves something");
        let last = apply(5_000, 820, 5_000, 200_000);
        assert!(kills(820, last.applied), "the pool's last hit is the game's kill");
    }

    /// 156차 방패무시: `def × pct / 100` back onto a real hit, never onto a floored one, never
    /// without the option or a defence, under the resolver's cap.
    #[test]
    fn shield_pierce_gives_back_a_share_of_the_defence() {
        use wie_util::stats::pierce_bonus;
        assert_eq!(pierce_bonus(1_000, 400, 50), Some(1_200));
        assert_eq!(pierce_bonus(1_000, 400, 100), Some(1_400), "100 % = the whole defence");
        assert_eq!(pierce_bonus(1_000, 400, 130), Some(1_400), "more than the whole defence is the whole defence");
        assert_eq!(pierce_bonus(1_000, 400, 0), None, "nothing worn");
        assert_eq!(pierce_bonus(1_000, 0, 50), None, "nothing to pierce");
        assert_eq!(pierce_bonus(1_000, -5, 50), None, "a negative defence is not one");
        assert_eq!(pierce_bonus(1, 400, 50), None, "a floored hit: the attack did not exceed the defence");
        assert_eq!(pierce_bonus(2, 400, 50), Some(202));
        assert_eq!(pierce_bonus(1_000, 1, 30), None, "a share that rounds to 0 changes nothing");
        assert_eq!(pierce_bonus(9_999_990, 400, 50), Some(DAMAGE_CAP));
    }

    #[test]
    fn only_the_player_is_redirected_and_the_halfword_is_sign_extended() {
        assert_eq!(player_word(1, 26), None);
        assert_eq!(player_word(0, 26), Some(26));
        assert_eq!(player_word(0, -1), Some(-1));
    }

    #[test]
    fn the_expiry_mirror_needs_all_three_signs() {
        assert!(expired(-1, -1, 0));
        assert!(!expired(0, -1, 0), "a fresh hit (timer 0) is not expiry");
        assert!(!expired(-1, 0, 0));
        assert!(!expired(-1, -1, 26), "the scratch still holds a number");
    }
}
