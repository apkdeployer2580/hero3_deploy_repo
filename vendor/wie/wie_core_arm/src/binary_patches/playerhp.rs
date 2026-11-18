//! 플레이어 HP 상한 해제 + 85제 세트 효과의 피격 절반 (142차, 2026-09-30).
//!
//! ── WHERE THE 16 BITS ARE ────────────────────────────────────────────────────
//! The player's stat block is `CHAR+0xb4` ([`STAT_BLOCK`]). `ComputeMaxHP 0x49000` computes
//! `(14*Lv + 18*체력) * (100 + HP%) / 100` in 32 bits and stores it as an s32 at `+0x38`
//! (`CHAR+0xec`, `0x4902a str r0,[r4,#0x38]`) — no clamp. The CURRENT HP is an i16 at
//! `+0x3e` (`CHAR+0xf2`), and it is written three ways:
//!
//! * `addHP 0x4b9c0(block, i16 delta)` — `hp = clamp(hp + delta, 0, hp_max)` in 16 bits.
//!   Every hit (`hitPlayer 0x4d410` → `0x4d4a6`, the projectile twin → `0x26592`), every
//!   regen tick (`0x4ba1c`), lifesteal (`0x4e220`, `0x4e946`), potions (`useItem` →
//!   `0x55104`), skill heals (`0x4f9b2`, `0x4ba62`), the level-up heal (`0x4d6c8`) and the
//!   script's percent heal (`0x3d7fc` → `0x3d824`) go through it (literal scan + direct
//!   `bl`s, 2026-09-30).
//! * `hp = hp_max` restores (`0x4914`, `0x493a`, `0x3b536` revive, `0x491c2`, `0x4f3fc`,
//!   `0x5165e`, `0x5522c`, `0x5531c`) — `ldr hp_max; strh` = the LOW HALF of the max.
//! * the save loader (`0x1a610`, `0x1a9ae`).
//!
//! So a max above 32767 is a max the game can never fill: the first restore wraps the
//! current HP negative and the player dies.
//!
//! ── WHAT THE HOOKS DO ────────────────────────────────────────────────────────
//! Same shape as the monsters' virtual HP (102차, `dmgwide`): the game keeps living in a
//! world where nothing exceeds 32767, and the true numbers live beside it.
//!
//! * [`handle_max_lgt`] at `0x4902a`: stores `min(T, 32767)` (the GAUGE max) and publishes
//!   the true `T` (`wie_util::stats::PLAYER_HP_MAX`). With the gauge max the restores store a
//!   value that fits, every percent the game computes off `hp_max` (regen, the level-up and
//!   script heals) comes out in gauge units, and `0x4d6c4`'s `ldrsh` of the max reads right.
//! * [`handle_add_lgt`] replaces `addHP` for the player's block: the true current HP comes
//!   from `wie_util::stats` (re-derived from the gauge when the gauge was written behind the
//!   hooks' back), the delta is applied in TRUE units — except the three callers whose
//!   delta is a percent of the (gauge) max ([`RELATIVE_LR`]), which are scaled up — and the
//!   i16 becomes the proportional gauge (`stats::hp_gauge`, never 0 while anything is left,
//!   so the game's own `hp <= 0` death checks stay exact). Any other block runs the
//!   original arithmetic bit for bit.
//! * [`handle_disp_cur_lgt`] / [`handle_disp_max_lgt`] at `0x40ca4` / `0x40ce6`: the status
//!   screen's two numbers read the true values. (Its bar, `0x40d52`, divides the gauge by
//!   the gauge max — already the right ratio.)
//!
//! With `T <= 32767` every path is the original game: the gauge is the HP.
//!
//! ── 85제 세트 효과 (the damage half) ─────────────────────────────────────────
//! A NEGATIVE delta is damage. When the host has published a complete set
//! (`stats::SET_BONUS`): 레갈리스근위 「교단의 가호」 takes 20 % off, 템플러 「수호의 맹세」
//! turns a hit that would leave <= 10 HP into 1 HP + 5 s of invulnerability (100 s cooldown,
//! game time). The rule is `stats::player_hit` (pure). 크루세이더 「기습」 lives in `dmgwide`.
//!
//! ── 142차-C 붉은마녀 「마녀의 생명력」 (the max half) ──────────────────────────────
//! A kill (stamped by `dmgwide`) gives 20 s of max HP +20 %: [`handle_max_lgt`] raises the
//! computed `T` by `stats::max_hp_bonus_pct` before anything else, and the host re-runs
//! `ComputeMaxHP` (`0x49000(block)`, pure — it only computes and stores) when the bonus turns
//! on or off, with `stats::HP_MAX_CLAMP` raised so the end clamps an HP above the new max
//! ([`edge_clamp`]). Without the set the bonus is 0 and nothing here changes.
//!
//! FAIL SOFT: an unreadable block reads the displaced instruction's own operand the way
//! the game would have, so a failure here is a failure the game would have had anyway.

use alloc::format;

use wie_util::{Result, WieError, read_generic, stats, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `CHAR` (`.bss 0x1503a88`, `guest_anchors::char_anchor`) + `0xb4` — the stat block
/// `ComputeMaxHP` and `addHP` take in `r0`.
pub const STAT_BLOCK: u32 = 0x0150_3a88 + 0xb4;
/// `block + 0x38` — the max HP, s32.
pub const BLOCK_HP_MAX: u32 = 0x38;
/// `block + 0x3e` — the current HP, i16.
pub const BLOCK_HP: u32 = 0x3e;

/// `str r0,[r4,#0x38]` at `0x4902a` — ComputeMaxHP's one store.
pub const MAX_HEAD: [u8; 2] = [0xa0, 0x63];
/// `push {lr}` at `0x4b9c0` — addHP's first instruction.
pub const ADD_HEAD: [u8; 2] = [0x00, 0xb5];
/// `ldrsh r2,[r3,r4]` at `0x40ca4` — the status screen's current-HP load.
pub const DISP_CUR_HEAD: [u8; 2] = [0x1a, 0x5f];
/// `ldr r2,[r3]` at `0x40ce6` — the status screen's max-HP load.
pub const DISP_MAX_HEAD: [u8; 2] = [0x1a, 0x68];

/// Return addresses (Thumb bit set) of the `addHP` calls whose delta is a PERCENT of the
/// gauge max rather than an HP amount: regen (`0x4ba1c`, `f6 * hp_max / 1000`), the
/// level-up heal (`0x4d6c8`, `hp_max` itself) and the script heal (`0x3d824`,
/// `n * hp_max / 100`). Everything else — hits, lifesteal, potions, skill heals — is HP.
pub const RELATIVE_LR: [u32; 3] = [0x4ba21, 0x4d6cd, 0x3d829];

/// ★ 150차 — return addresses (Thumb bit set) of the two `addHP` calls that are a MONSTER
/// HITTING THE PLAYER: `hitPlayer 0x4d410` (`bl` at `0x4d4a6`) and its projectile twin
/// `0x2650c` (`bl` at `0x26596`). The hit clock (굴베이그의탈리스만) is stamped only for these,
/// and only these callers store the damage number from `r7` right after the call
/// (`0x4d4c2` / `0x265a2 strh r7,[r5,#0x184]`; `r7` is not read again before the pop) — so the
/// hook writes the REDUCED damage there and the floating number tells the truth.
pub const HIT_LR: [u32; 2] = [0x4d4ab, 0x2659b];

/// Host heals refused because the player was already dead (log only).
static HOST_HEAL_REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// ★ 150차 review — does this `addHP` get refused outright? Only a HOST call (the return
/// address `ArmCore::run_function` plants) that HEALS a player who is already at 0: the
/// host's 5-second regen must never raise the dead (see `handle_add_lgt`). Every guest call
/// and every host damage keeps the game's rule.
pub fn refuses_host_heal(lr: u32, delta: i16, hp: i16) -> bool {
    lr == crate::core::RUN_FUNCTION_LR && delta > 0 && hp <= 0
}

fn check(head: [u8; 2], want: [u8; 2], what: &str, pc: u32) -> Result<()> {
    if head == want {
        return Ok(());
    }
    Err(WieError::FatalError(format!("{what} hook at {pc:#x}: expected {want:02x?}, found {head:02x?}")))
}

pub fn check_max_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, MAX_HEAD, "hp_max (str r0,[r4,#0x38])", pc)
}

pub fn check_add_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, ADD_HEAD, "hp_add (push {lr})", pc)
}

pub fn check_disp_cur_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, DISP_CUR_HEAD, "hp_disp_cur (ldrsh r2,[r3,r4])", pc)
}

pub fn check_disp_max_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, DISP_MAX_HEAD, "hp_disp_max (ldr r2,[r3])", pc)
}

/// The original `addHP` arithmetic, bit for bit: the sum is taken in 16 bits and stored
/// with `strh`, then compared signed against the s32 max (store its low half when above)
/// and against 0.
pub fn original_add(hp: i16, delta: i16, max: i32) -> i16 {
    let sum = (hp as u16).wrapping_add(delta as u16) as i16;
    if i32::from(sum) > max {
        max as i16
    } else if sum < 0 {
        0
    } else {
        sum
    }
}

/// What one `addHP` does to the player, pure. `gauge`/`gmax` are the game's two numbers,
/// `published` the true max, `stored`/`seen` the tracked true HP and the gauge written with
/// it, `mask`/`now`/`until`/`ready` the set state, `taken_pct` the worn accessories' 피해감소
/// (150차 이안의맹세). Returns `(new gauge, new true HP, guard)`.
#[allow(clippy::too_many_arguments)]
pub fn player_add(
    gauge: i16,
    gmax: i32,
    delta: i16,
    relative: bool,
    published: u32,
    stored: u32,
    seen: u32,
    mask: u32,
    now: u64,
    until: u64,
    ready: u64,
    taken_pct: u32,
) -> (i16, u32, Option<(u64, u64)>) {
    let g = gauge.max(0) as u32;
    let gm = gmax.max(0) as u32;
    let max = stats::hp_true_max(gm, published);
    // ⛔ `taken_pct` too (150차): with no set but 이안의맹세 worn, the original arithmetic
    //    would drop the -10 % on the floor.
    if max <= stats::HP_GAUGE_CEILING && (delta >= 0 || (mask == 0 && taken_pct == 0)) {
        // The original game — except the one thing it got wrong: its sum is taken in 16
        // bits, so a heal that carries HP past 32767 wraps NEGATIVE and stores 0 (dead).
        // Unreachable while max HP stayed in the low thousands; reachable since the legacy
        // HP options (150차 review: a level-up heal of delta = T at T 20000 and HP 15000
        // killed the player). A heal is summed in 32 bits and clamped to [0, max]; every sum
        // that did not wrap is bit-identical to the original. A hit (delta < 0) cannot wrap
        // (hp >= 0, delta >= -32768) and keeps the original arithmetic.
        let v = if delta >= 0 {
            (i32::from(gauge) + i32::from(delta)).min(gmax.min(stats::HP_GAUGE_CEILING as i32)).max(0) as i16
        } else {
            original_add(gauge, delta, gmax)
        };
        return (v, v.max(0) as u32, None);
    }
    let cur = stats::hp_true_current(g, max, stored, seen);
    let (new, guard) = if delta < 0 {
        let (left, guard) = stats::player_hit(cur, (-i32::from(delta)) as u32, mask, taken_pct, now, until, ready);
        (left.min(max), guard)
    } else {
        let heal = if relative && gm > 0 && max > gm {
            (u64::from(delta as u16) * u64::from(max) / u64::from(gm)) as u32
        } else {
            delta as u32
        };
        (cur.saturating_add(heal).min(max), None)
    };
    let gauge = if max <= stats::HP_GAUGE_CEILING { new } else { stats::hp_gauge(new, max) };
    (gauge as i16, new, guard)
}

/// `0x4b9c0` for the player's block: the whole of `addHP`, virtual when the max is above
/// the ceiling, with the set effects on damage. Returns to `lr` (the hook replaces the
/// function at its first instruction, so nothing was pushed).
pub async fn handle_add_lgt(core: &mut ArmCore, _hook_pc: u32) -> Result<JumpTo> {
    let (block, r1, lr) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::LR),
        )
    };
    let delta = r1 as u16 as i16;
    let hp: i16 = read_generic(core, block + BLOCK_HP)?;
    let max: i32 = read_generic(core, block + BLOCK_HP_MAX)?;
    // ★ 150차 review — a HOST heal (the 5-second regen, `ArmCore::run_function`'s return
    // address) never raises the dead. The worker checks "alive, not dying" when it polls, but
    // the call runs a tick later and the guest's tasks run in no fixed order: a monster's
    // killing blow in between left HP 0, and the heal then either undid the death before the
    // state machine saw it, or landed after the death entry (state 16 never re-reads HP) and
    // let GAME OVER run with HP above 0 — which also hid the death from the tower's
    // `player_hp == Some(0)` test (101차's in-place revive). The game's own heals keep the
    // original rule (they only ever run on the living).
    if refuses_host_heal(lr, delta, hp) {
        if HOST_HEAL_REFUSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 3 {
            tracing::warn!("player hp: a host heal of {delta} reached a dead player (HP {hp}) — refused");
        }
        return Ok(JumpTo(lr));
    }
    if block != STAT_BLOCK {
        write_generic(core, block + BLOCK_HP, original_add(hp, delta, max))?;
        return Ok(JumpTo(lr));
    }
    let now = stats::play_clock_ms();
    let mask = stats::set_bonus();
    let acc = stats::acc_effects();
    let published = stats::PLAYER_HP_MAX.load(core::sync::atomic::Ordering::Relaxed);
    let stored = stats::PLAYER_HP.load(core::sync::atomic::Ordering::Relaxed);
    let seen = stats::PLAYER_HP_SEEN.load(core::sync::atomic::Ordering::Relaxed);
    // The true HP going in — the same read `player_add` makes (for the hit clock below).
    let before = stats::hp_true_current(hp.max(0) as u32, stats::hp_true_max(max.max(0) as u32, published), stored, seen);
    let (gauge, true_hp, guard) = player_add(
        hp,
        max,
        delta,
        RELATIVE_LR.contains(&lr),
        published,
        stored,
        seen,
        mask,
        now,
        stats::GUARD_UNTIL_MS.load(core::sync::atomic::Ordering::Relaxed),
        stats::GUARD_READY_MS.load(core::sync::atomic::Ordering::Relaxed),
        u32::from(acc.taken_pct),
    );
    write_generic(core, block + BLOCK_HP, gauge)?;
    stats::set_player_hp(true_hp, gauge.max(0) as u32);
    if delta < 0 && HIT_LR.contains(&lr) {
        // ★ 150차 굴베이그의탈리스만: a monster's hit that cost HP (re)starts the hit clock.
        if let Some(until) = stats::hit_stamp(acc, before, true_hp, now) {
            stats::HIT_UNTIL_MS.store(until, core::sync::atomic::Ordering::Relaxed);
            if stats::HIT_STAMPS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 5 {
                tracing::warn!(
                    "accessory: hit clock — a hit of {} took {before} -> {true_hp}; max HP +{} % until {until} ms",
                    -i32::from(delta),
                    acc.hit_max_hp_pct
                );
            }
        }
        // The floating number is the caller's `r7` (the raw hit) — make it the damage the
        // reductions (교단의 가호 · 피해감소) left, so what the player reads is what they lost.
        let raw = (-i32::from(delta)) as u32;
        let taken = stats::taken_after(raw, mask, u32::from(acc.taken_pct));
        if taken != raw && true_hp < before {
            core.inner.lock().engine.reg_write(ArmRegister::R7, taken & 0xffff);
        }
    }
    if let Some((until, ready)) = guard {
        stats::GUARD_UNTIL_MS.store(until, core::sync::atomic::Ordering::Relaxed);
        stats::GUARD_READY_MS.store(ready, core::sync::atomic::Ordering::Relaxed);
        stats::GUARD_TRIGGERS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        tracing::warn!("set bonus: 수호의 맹세 — a hit of {} left {true_hp} HP; invulnerable until {until} ms (ready again at {ready} ms)", -i32::from(delta));
    }
    Ok(JumpTo(lr))
}

/// What `ComputeMaxHP`'s store does, pure: `(gauge max to store, Some((true HP, gauge)) when
/// the current HP has to be re-expressed)`. `t` is the freshly computed max, `old_gmax` the
/// value about to be overwritten, `gauge` the current i16, `clamp` the host's buff-edge flag
/// ([`stats::HP_MAX_CLAMP`]): that one recompute cuts an HP above the new max at once.
pub fn max_store(t: i32, old_gmax: i32, gauge: i16, published: u32, stored: u32, seen: u32, clamp: bool) -> (i32, Option<(u32, i16)>) {
    if t <= 0 {
        return (t, None);
    }
    let new_max = t as u32;
    let gstore = new_max.min(stats::HP_GAUGE_CEILING) as i32;
    let old_max = stats::hp_true_max(old_gmax.max(0) as u32, published);
    if old_max <= stats::HP_GAUGE_CEILING && new_max <= stats::HP_GAUGE_CEILING {
        return (gstore, None); // the original game: the HP is not touched by a recompute
    }
    let g = gauge.max(0) as u32;
    // Tracked in this session (the gauge is the one the hooks wrote): keep the HP itself.
    // Written behind the hooks' back (a load, a restore): the gauge is a share of THIS max.
    let cur = if seen == g { stored } else { stats::hp_from_gauge(g, new_max) };
    // ★ 150차 review: a LOWER max does not cut the true HP — the gauge shows it clamped, the
    // tracked value is kept (every reader clamps it: `hp_true_current`, the next `addHP`).
    // That is the original game's own rule (an HP above a lowered max stays until the next
    // `addHP`), and the only one a recompute that is UNDONE can live with: the 장비 screen's
    // preview (`0x4f014(preview = 1)` → this store with the CANDIDATE's max) followed by
    // CLR used to keep the candidate's lower max as the player's HP for good.
    // The host's buff edge is the exception: a 20-second bonus that ends must not leave its
    // HP behind (`HP_MAX_CLAMP`, 142차-D) — above the ceiling exactly as below it.
    let shown = cur.min(new_max);
    let kept = if clamp { shown } else { cur };
    let gauge = if new_max <= stats::HP_GAUGE_CEILING { shown } else { stats::hp_gauge(shown, new_max) };
    (gstore, Some((kept, gauge as i16)))
}

/// The host's buff-edge clamp, pure: `(gauge, true HP)` to write when the current HP sits
/// above the freshly stored max, `None` otherwise. Only for the gauge-is-the-HP case —
/// above the ceiling `max_store` already re-expresses the HP against the new max.
pub fn edge_clamp(gstore: i32, gauge: i16, rewrite: Option<(u32, i16)>) -> Option<(i16, u32)> {
    if rewrite.is_some() || gstore <= 0 || i32::from(gauge) <= gstore {
        return None;
    }
    Some((gstore as i16, gstore as u32))
}

/// `0x4902a str r0,[r4,#0x38]`: store the gauge max, publish the true one. With 142차-C
/// 붉은마녀 「마녀의 생명력」 running the computed max is raised first
/// (`stats::max_hp_bonus_pct`), so every recompute — the game's own (level-up, equip) and the
/// host's at the buff's two edges — agrees.
pub async fn handle_max_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (t, block) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::R0) as i32, inner.engine.reg_read(ArmRegister::R4))
    };
    if block != STAT_BLOCK {
        write_generic(core, block + BLOCK_HP_MAX, t)?;
        return Ok(JumpTo(exit_pc));
    }
    let bonus = stats::max_hp_bonus_pct(
        stats::set_bonus(),
        stats::acc_effects(),
        stats::play_clock_ms(),
        stats::WITCH_UNTIL_MS.load(core::sync::atomic::Ordering::Relaxed),
        stats::HIT_UNTIL_MS.load(core::sync::atomic::Ordering::Relaxed),
    );
    let t = stats::boosted_max(t, bonus);
    let old_gmax: i32 = read_generic(core, block + BLOCK_HP_MAX)?;
    let gauge: i16 = read_generic(core, block + BLOCK_HP)?;
    let clamp = stats::HP_MAX_CLAMP.swap(false, core::sync::atomic::Ordering::Relaxed);
    let (gstore, rewrite) = max_store(
        t,
        old_gmax,
        gauge,
        stats::PLAYER_HP_MAX.load(core::sync::atomic::Ordering::Relaxed),
        stats::PLAYER_HP.load(core::sync::atomic::Ordering::Relaxed),
        stats::PLAYER_HP_SEEN.load(core::sync::atomic::Ordering::Relaxed),
        clamp,
    );
    write_generic(core, block + BLOCK_HP_MAX, gstore)?;
    if clamp && let Some((g, hp)) = edge_clamp(gstore, gauge, rewrite) {
        write_generic(core, block + BLOCK_HP, g)?;
        stats::set_player_hp(hp, g.max(0) as u32);
    }
    if t > 0 {
        let before = stats::PLAYER_HP_MAX.swap(t as u32, core::sync::atomic::Ordering::Relaxed);
        if before != t as u32 && (t as u32 > stats::HP_GAUGE_CEILING || before > stats::HP_GAUGE_CEILING) {
            tracing::warn!("player hp: max {before} -> {t} (the game keeps a gauge of {gstore}; the true numbers are the hooks')");
        }
    }
    if let Some((hp, g)) = rewrite {
        write_generic(core, block + BLOCK_HP, g)?;
        stats::set_player_hp(hp, g.max(0) as u32);
    }
    Ok(JumpTo(exit_pc))
}

/// `0x40ca4 ldrsh r2,[r3,r4]`: the status screen's current HP.
pub async fn handle_disp_cur_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let addr = {
        let inner = core.inner.lock();
        inner.engine.reg_read(ArmRegister::R3).wrapping_add(inner.engine.reg_read(ArmRegister::R4))
    };
    let raw: i16 = read_generic(core, addr)?;
    let value = if addr == STAT_BLOCK + BLOCK_HP {
        match read_generic::<i32, _>(core, STAT_BLOCK + BLOCK_HP_MAX) {
            Ok(gmax) => stats::player_hp_view(raw.max(0) as u32, gmax.max(0) as u32).0,
            Err(_) => raw as i32 as u32,
        }
    } else {
        raw as i32 as u32
    };
    core.inner.lock().engine.reg_write(ArmRegister::R2, value);
    Ok(JumpTo(exit_pc))
}

/// `0x40ce6 ldr r2,[r3]`: the status screen's max HP.
pub async fn handle_disp_max_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let addr = core.inner.lock().engine.reg_read(ArmRegister::R3);
    let raw: u32 = read_generic(core, addr)?;
    let value = if addr == STAT_BLOCK + BLOCK_HP_MAX {
        stats::hp_true_max(raw, stats::PLAYER_HP_MAX.load(core::sync::atomic::Ordering::Relaxed))
    } else {
        raw
    };
    core.inner.lock().engine.reg_write(ArmRegister::R2, value);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: u32 = u32::MAX;

    #[test]
    fn the_hooks_only_install_over_their_instructions() {
        assert!(check_max_head([0xa0, 0x63], 0x4902b).is_ok());
        assert!(check_add_head([0x00, 0xb5], 0x4b9c1).is_ok());
        assert!(check_disp_cur_head([0x1a, 0x5f], 0x40ca5).is_ok());
        assert!(check_disp_max_head([0x1a, 0x68], 0x40ce7).is_ok());
        assert!(check_add_head([0xf0, 0xb5], 0x4b9e5).is_err(), "the regen function next door");
    }

    /// The original arithmetic, including its 16-bit wrap and the low-half store.
    #[test]
    fn original_add_is_the_games() {
        assert_eq!(original_add(100, -30, 500), 70);
        assert_eq!(original_add(100, 900, 500), 500);
        assert_eq!(original_add(10, -30, 500), 0);
        assert_eq!(original_add(100, 900, 40_000), 1000);
        // A heal past 32767 wraps NEGATIVE and floors to 0 — the death the hooks exist for.
        assert_eq!(original_add(32_000, 1_000, 40_000), 0);
        assert_eq!(original_add(30_000, 5_000, 40_000), 0);
    }

    /// 마녀의 생명력's end: an HP above the lowered max is clamped, but only on the host's
    /// edge recompute and only in the gauge-is-the-HP case.
    #[test]
    fn the_buff_edge_clamps_an_hp_above_the_new_max() {
        assert_eq!(edge_clamp(1_000, 1_150, None), Some((1_000, 1_000)));
        assert_eq!(edge_clamp(1_000, 900, None), None, "under the max: untouched");
        assert_eq!(edge_clamp(1_000, 1_150, Some((900, 1_000))), None, "max_store already did it");
        assert_eq!(stats::boosted_max(1_000, 20), 1_200);
        assert_eq!(stats::boosted_max(1_000, 0), 1_000);
        assert_eq!(stats::boosted_max(30_000, 20), 36_000, "past the ceiling: the virtual HP takes it");
        assert_eq!(stats::boosted_max(i32::MAX, 20), i32::MAX);
        let none = stats::AccEffects::NONE;
        assert_eq!(stats::max_hp_bonus_pct(stats::SET_WITCH, none, 5_000, 20_000, 0), 20);
        assert_eq!(stats::max_hp_bonus_pct(stats::SET_WITCH, none, 20_000, 20_000, 0), 0, "expired");
        assert_eq!(stats::max_hp_bonus_pct(0, none, 5_000, 20_000, 0), 0, "the set came off");
    }

    /// Up to the ceiling, with no set, the player's HP is the original game bit for bit.
    #[test]
    fn under_the_ceiling_nothing_changes() {
        for (hp, d, m) in [(500i16, -120i16, 800i32), (500, 400, 800), (5, -40, 800), (32_000, 767, 32_767)] {
            let (g, t, guard) = player_add(hp, m, d, false, m as u32, 0, NONE, 0, 0, 0, 0, 0);
            assert_eq!(g, original_add(hp, d, m));
            assert_eq!(t, g as u32);
            assert!(guard.is_none());
        }
        // Relative callers too: the gauge max IS the max.
        let (g, _, _) = player_add(100, 800, 800, true, 800, 0, NONE, 0, 0, 0, 0, 0);
        assert_eq!(g, 800);
    }

    /// Above it the gauge is a share of the true HP, hits and heals are in true units, and
    /// the level-up heal (delta = the gauge max, a relative caller) fills the TRUE max.
    #[test]
    fn above_the_ceiling_the_true_hp_carries() {
        let t = 60_000u32;
        let gm = 32_767i32;
        // Full: the gauge is the ceiling, the true HP the max.
        let (g, h, _) = player_add(32_767, gm, -6_000, false, t, t, 32_767, 0, 0, 0, 0, 0);
        assert_eq!(h, 54_000);
        assert_eq!(g as u32, stats::hp_gauge(54_000, t));
        assert_eq!(g, 29_490);
        // A potion of 3000 is 3000 HP.
        let (g2, h2, _) = player_add(g, gm, 3_000, false, t, h, g as u32, 0, 0, 0, 0, 0);
        assert_eq!(h2, 57_000);
        assert_eq!(g2 as u32, stats::hp_gauge(57_000, t));
        // The level-up heal passes the gauge max: a relative caller fills the true max.
        let (g3, h3, _) = player_add(g2, gm, 32_767, true, t, h2, g2 as u32, 0, 0, 0, 0, 0);
        assert_eq!((g3, h3), (32_767, t));
        // A killing blow reaches 0 through the game's own check.
        let (g4, h4, _) = player_add(g3, gm, -30_000, false, t, 20_000, g3 as u32, 0, 0, 0, 0, 0);
        assert_eq!(h4, 0);
        assert_eq!(g4, 0);
        // One HP left still shows as alive.
        let (g5, h5, _) = player_add(100, gm, -1, false, t, 2, 100, 0, 0, 0, 0, 0);
        assert_eq!((g5, h5), (1, 1));
    }

    /// A gauge written behind the hooks' back (a restore stored the gauge max) reads as
    /// full; one the hooks wrote keeps its exact true value.
    #[test]
    fn a_restore_is_read_off_the_gauge() {
        let t = 45_000u32;
        let (_, h, _) = player_add(32_767, 32_767, -1_000, false, t, 12_345, 7_000, 0, 0, 0, 0, 0);
        assert_eq!(h, 44_000, "the gauge changed since the hooks wrote 7000: full, then the hit");
        let (_, h, _) = player_add(9_000, 32_767, -1_000, false, t, 12_345, 9_000, 0, 0, 0, 0, 0);
        assert_eq!(h, 11_345, "the gauge is the hooks' own: the tracked 12345, then the hit");
    }

    /// The set effects: the blessing takes 20 % off, the guard turns a would-be kill into
    /// 1 HP + 5 s invulnerability, and does nothing while cooling down.
    #[test]
    fn the_sets_shape_the_hit() {
        let t = 800u32;
        let (_, h, _) = player_add(500, 800, -100, false, t, 0, NONE, stats::SET_BLESSING, 0, 0, 0, 0);
        assert_eq!(h, 420);
        let (g, h, guard) = player_add(50, 800, -300, false, t, 0, NONE, stats::SET_GUARD, 10_000, 0, 0, 0);
        assert_eq!((g, h), (1, 1));
        assert_eq!(guard, Some((15_000, 110_000)));
        // Invulnerable: the next hit does nothing.
        let (g, h, guard) = player_add(1, 800, -300, false, t, 1, 1, stats::SET_GUARD, 12_000, 15_000, 110_000, 0);
        assert_eq!((g, h, guard), (1, 1, None));
        // After the window, before the cooldown: the hit kills.
        let (g, h, guard) = player_add(1, 800, -300, false, t, 1, 1, stats::SET_GUARD, 20_000, 15_000, 110_000, 0);
        assert_eq!((g, h, guard), (0, 0, None));
        // A hit that leaves more than 10 does not trigger it.
        let (_, h, guard) = player_add(500, 800, -300, false, t, 0, NONE, stats::SET_GUARD, 0, 0, 0, 0);
        assert_eq!((h, guard), (200, None));
        // Exactly 10 left does.
        let (_, h, guard) = player_add(310, 800, -300, false, t, 0, NONE, stats::SET_GUARD, 0, 0, 0, 0);
        assert_eq!(h, 10);
        assert!(guard.is_some());
    }

    /// ★ 150차 이안의맹세 「피해감소 +10%」: with no set worn the hit still leaves the
    /// original arithmetic (400 would be the bug), it multiplies with 교단의 가호, a 1 stays 1.
    #[test]
    fn the_accessory_reduction_shapes_the_hit() {
        let t = 800u32;
        let (g, h, _) = player_add(500, 800, -100, false, t, 0, NONE, 0, 0, 0, 0, 10);
        assert_eq!((g, h), (410, 410), "no set: the -10 % still applies");
        let (_, h, _) = player_add(500, 800, -100, false, t, 0, NONE, stats::SET_BLESSING, 0, 0, 0, 10);
        assert_eq!(h, 428, "x0.8 x0.9 = 72");
        let (_, h, _) = player_add(500, 800, -1, false, t, 0, NONE, stats::SET_BLESSING, 0, 0, 0, 10);
        assert_eq!(h, 499, "a hit still does at least 1");
        let (_, h, _) = player_add(500, 800, -100, false, t, 0, NONE, 0, 0, 0, 0, 20);
        assert_eq!(h, 420, "two copies");
        let (_, h, _) = player_add(500, 800, 100, false, t, 0, NONE, 0, 0, 0, 0, 10);
        assert_eq!(h, 600, "a heal is not a hit");
        assert_eq!(stats::taken_after(100, 0, 90), 50, "capped at 50 %");
        assert_eq!(stats::taken_after(0, stats::SET_BLESSING, 10), 0);
        assert_eq!(stats::taken_after(14, stats::SET_BLESSING, 0), 11, "the probe's 교단의 가호: floor(x0.8)");
        assert_eq!(stats::taken_after(14, 0, 0), 14);
    }

    /// ★ 150차 the timed max-HP sources: the kill clock carries the set and 레아, the hit
    /// clock 굴베이그; running sources add, capped.
    #[test]
    fn the_timed_max_hp_sources_add_up() {
        let lea = stats::AccEffects { kill_max_hp_pct: 20, ..stats::AccEffects::NONE };
        let gul = stats::AccEffects { hit_max_hp_pct: 20, ..stats::AccEffects::NONE };
        let both = stats::AccEffects { kill_max_hp_pct: 20, hit_max_hp_pct: 20, ..stats::AccEffects::NONE };
        assert_eq!(stats::max_hp_bonus_pct(0, lea, 1_000, 20_000, 0), 20, "레아 alone");
        assert_eq!(stats::max_hp_bonus_pct(stats::SET_WITCH, lea, 1_000, 20_000, 0), 40, "set + 레아");
        assert_eq!(stats::max_hp_bonus_pct(0, gul, 1_000, 0, 20_000), 20, "굴베이그 alone");
        assert_eq!(stats::max_hp_bonus_pct(0, gul, 1_000, 20_000, 0), 0, "a kill does not start the hit buff");
        assert_eq!(stats::max_hp_bonus_pct(stats::SET_WITCH, both, 1_000, 20_000, 20_000), 60, "all three");
        assert_eq!(stats::max_hp_bonus_pct(stats::SET_WITCH, both, 25_000, 20_000, 30_000), 20, "the kill clock ran out");
        let huge = stats::AccEffects { kill_max_hp_pct: 200, hit_max_hp_pct: 200, ..stats::AccEffects::NONE };
        assert_eq!(stats::max_hp_bonus_pct(0, huge, 1, 2, 2), stats::MAX_HP_BONUS_CAP_PCT);
        // The hit clock is stamped only for a hit that cost HP and left the player alive.
        assert_eq!(stats::hit_stamp(gul, 500, 400, 1_000), Some(21_000));
        assert_eq!(stats::hit_stamp(gul, 500, 500, 1_000), None, "an invulnerable hit");
        assert_eq!(stats::hit_stamp(gul, 500, 0, 1_000), None, "the killing blow");
        assert_eq!(stats::hit_stamp(lea, 500, 400, 1_000), None, "no 굴베이그 worn");
        // The kill clock: the set or 레아.
        assert!(stats::on_kill(0, lea, 1_000).is_some());
        assert!(stats::on_kill(stats::SET_WITCH, stats::AccEffects::NONE, 1_000).is_some());
        assert!(stats::on_kill(0, gul, 1_000).is_none());
    }

    #[test]
    fn a_host_heal_never_raises_the_dead() {
        let host = crate::core::RUN_FUNCTION_LR;
        assert!(refuses_host_heal(host, 600, 0), "the regen after a killing blow");
        assert!(!refuses_host_heal(host, 600, 1), "a living player is healed");
        assert!(!refuses_host_heal(host, -5, 0), "not a heal");
        assert!(!refuses_host_heal(0x4ba21, 600, 0), "the game's own call keeps its rule");
        assert!(!refuses_host_heal(HIT_LR[0], 600, 0));
    }

    #[test]
    fn the_hit_callers_are_the_two_monster_hits() {
        for lr in HIT_LR {
            assert!(!RELATIVE_LR.contains(&lr), "a hit is never a relative heal");
            assert_eq!(lr & 1, 1, "Thumb return addresses");
        }
    }

    /// ★ 150차 review — the 장비 screen's PREVIEW stores the candidate's (lower) max and its
    /// cancel restores the real one: above the ceiling the HP must come back with it, both
    /// for a candidate still above the ceiling and for one below it.
    #[test]
    fn a_previewed_lower_max_does_not_cost_hp() {
        let t = 50_000u32;
        let g = stats::hp_gauge(45_000, t);
        for preview in [40_000i32, 30_000] {
            let (s1, r1) = max_store(preview, 32_767, g as i16, t, 45_000, g, false);
            let (hp1, g1) = r1.expect("re-expressed");
            assert_eq!(s1, preview.min(32_767));
            assert_eq!(hp1, 45_000, "the true HP is kept");
            let shown = stats::hp_true_current(g1.max(0) as u32, preview as u32, hp1, g1.max(0) as u32);
            assert_eq!(shown, preview as u32, "and shown clamped to the candidate's max");
            // The cancel: the real max again, published = the candidate's.
            let (_, r2) = max_store(t as i32, s1, g1, preview as u32, hp1, g1.max(0) as u32, false);
            assert_eq!(r2, Some((45_000, g as i16)), "nothing was lost");
        }
        // The host's buff edge (the clamp flag) DOES cut it — above the ceiling too.
        let (_, r) = max_store(40_000, 32_767, g as i16, t, 45_000, g, true);
        assert_eq!(r, Some((40_000, 32_767)));
    }

    /// ★ 150차 review — a heal under the ceiling that carries the 16-bit sum past 32767 used
    /// to wrap to 0 (death): the level-up heal (delta = T) at T 20000 and HP 15000.
    #[test]
    fn a_heal_under_the_ceiling_never_wraps() {
        let (g, h, _) = player_add(15_000, 20_000, 20_000, true, 20_000, 0, NONE, 0, 0, 0, 0, 0);
        assert_eq!((g, h), (20_000, 20_000));
        let (g, _, _) = player_add(31_999, 32_000, 1_000, false, 32_000, 0, NONE, 0, 0, 0, 0, 0);
        assert_eq!(g, 32_000);
        let (g, _, _) = player_add(32_767, 32_767, 32_767, false, 32_767, 0, NONE, 0, 0, 0, 0, 0);
        assert_eq!(g, 32_767);
        // every sum that did not wrap is the original arithmetic
        for (hp, d, m) in [(100i16, 50i16, 800i32), (790, 50, 800), (0, 1, 800), (900, 10, 800)] {
            assert_eq!(player_add(hp, m, d, false, m as u32, 0, NONE, 0, 0, 0, 0, 0).0, original_add(hp, d, m));
        }
    }

    /// The recompute: under the ceiling it never touches the HP; crossing it re-expresses
    /// the HP as a gauge of the new max; a fresh load reads the saved gauge as a share.
    #[test]
    fn the_max_store_keeps_the_hp_honest() {
        assert_eq!(max_store(900, 800, 500, 800, 0, NONE, false), (900, None));
        // 30000 of 30000 -> equip to 50000: the tracked 30000 HP stays 30000.
        let (s, r) = max_store(50_000, 30_000, 30_000, 30_000, 30_000, 30_000, false);
        assert_eq!(s, 32_767);
        assert_eq!(r, Some((30_000, stats::hp_gauge(30_000, 50_000) as i16)));
        // A save loaded behind the hooks' back: its full gauge is a full 50000.
        let (_, r) = max_store(50_000, 0, 32_767, 0, 0, NONE, false);
        assert_eq!(r, Some((50_000, 32_767)));
        // Unequip 50000 -> 20000 with 45000 HP tracked: the GAUGE shows the new max, the true
        // HP is kept (150차 — readers clamp it; a preview's undo or a quick re-equip gets it
        // back, as in the original game, whose HP only an `addHP` clamps).
        let g = stats::hp_gauge(45_000, 50_000);
        let (s, r) = max_store(20_000, 32_767, g as i16, 50_000, 45_000, g, false);
        assert_eq!((s, r), (20_000, Some((45_000, 20_000))));
        // The buff edge's recompute cuts it to the new max instead.
        assert_eq!(max_store(20_000, 32_767, g as i16, 50_000, 45_000, g, true), (20_000, Some((20_000, 20_000))));
        assert_eq!(stats::hp_true_current(20_000, 20_000, 45_000, 20_000), 20_000, "below the ceiling the gauge IS the HP");
    }

    #[test]
    fn the_gauge_round_trip() {
        assert_eq!(stats::hp_gauge(0, 60_000), 0);
        assert_eq!(stats::hp_gauge(1, 60_000), 1, "alive stays alive");
        assert_eq!(stats::hp_gauge(60_000, 60_000), 32_767);
        assert_eq!(stats::hp_from_gauge(32_767, 60_000), 60_000);
        assert_eq!(stats::hp_from_gauge(0, 60_000), 0);
        assert_eq!(stats::hp_gauge(700, 800), 700, "identity under the ceiling");
        assert_eq!(stats::hp_true_max(32_767, 60_000), 60_000);
        assert_eq!(stats::hp_true_max(800, 60_000), 800, "another character's publication does not apply");
        assert_eq!(stats::hp_true_max(800, 0), 800);
    }
}
