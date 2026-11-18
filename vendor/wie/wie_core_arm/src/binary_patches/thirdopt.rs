//! 150차 (2026-10-01) — the reworked accessories' THIRD option, where it is an ordinary
//! game stat: fed to the game's own `addStat` inside the wear recompute.
//!
//! ── WHY THE THIRD OPTION CANNOT LIVE IN THE ROW ──────────────────────────────
//! An accessory row carries exactly two `(statId, i8)` pairs (`+0x1a..+0x1d` of the
//! 0x20-byte runtime row). The item-DB parser `0x539a8` reads the record's u16 length and
//! THROWS IT AWAY, then consumes fixed fields — a 19th/20th byte on one record shifts every
//! later record (63차's boot death). And the world init `0x3ab68` re-allocates and re-parses
//! the whole DB on every load and runs the first recompute inside that same guest call, so a
//! host write into the row's spare bytes (`+0x1e/+0x1f`, calloc'd zero, read by nothing)
//! would always lose the race. Hence a hook — on the one routine that turns options into
//! stats.
//!
//! ── WHERE OPTIONS BECOME STATS ───────────────────────────────────────────────
//! `addStat 0x4ee34(id, value)` has six callers, all inside the wear recompute `0x4f014`
//! (`recompute(preview, slot, cand, _, keepHp)`), and so do all nine derived getters. That
//! one function zeroes the sixteen accumulators, then walks the nine worn slots:
//!
//! ```text
//! 0x4f120  lsls r3,r1,#16 / asrs r1 / adds r3,r1,#1 / ble skip   ; r1 = (s16)cat >= 0
//! 0x4f128  lsls r3,r2,#16 / asrs r2 / adds r4,r2,#1 / ble skip   ; r2 = (s16)idx >= 0
//! 0x4f130  ldr  r0,[sp,#0x48]          ; the per-category row tables   <- HOOK (displaced)
//! 0x4f132  lsls r3,r1,#2               ;                                <- exit_pc
//! 0x4f134  ldr  r3,[r3,r0] / lsls r4,r2,#5 / adds r4,r4,r3              ; r4 = the DB row
//! 0x4f13a  addStat(row[0x1a], row[0x1b]) ; 0x4f146 addStat(row[0x1c], row[0x1d]) ; then the orb
//! ```
//!
//! At `0x4f130` the slot's `(cat, idx)` are already resolved in `r1`/`r2` — including the
//! 장비장착 PREVIEW substitution (`preview != 0 && i == slot` reads the candidate list's
//! record instead of `EQUIPPED[i]`), `r5` is the slot `i` (0..8) and `r6` the stat block
//! (`getBase()+0x8c` = `CHAR+0xb4`). The recompute's 19 callers (wear 0x478xx, unequip
//! 0x484cc…, the 장비장착 preview 0x479ee, stat points 0x4f948, buffs, the load path 0x5166e,
//! useItem, the state machine…) all pass this loop, so one hook covers wear, preview,
//! unequip, load and every re-derivation, and the preview's blue numbers include the third.
//!
//! ── WHAT THE HOOK DOES ───────────────────────────────────────────────────────
//! When `wie_util::acc_options::third_of(cat, idx)` is `Third::Stat(id)` with an id in
//! `addStat`'s additive arms ([`guest_stat`](wie_util::acc_options::guest_stat)), it calls
//! the game's own `addStat(id, value)` (`ArmCore::run_function` saves and restores every
//! register, so `r1`/`r2`/`r4..r7` are what the loop left), then performs the displaced
//! load and resumes. `addStat` is a pure 16-bit add into an accumulator the same call
//! zeroed, so the result is order-free and a second recompute is idempotent — the same rule
//! as the row's two pairs (two copies of one accessory add it twice, like their options).
//!
//! The other third kinds (공격속도 · 피해감소 · 주는 피해 · the timed max-HP buffs) are NOT
//! stats — the host sums them (`stats::acc_effects_of`, `stats::host_third_pair`) and the
//! damage hooks read them. This hook ignores them.
//!
//! Off switch: `stats::THIRD_OPTS_OFF` (the probe's `--third-opts 0`; the app never sets it).
//! Guard / fail soft: `r6` must be the stat block and `r5 < 9`, else only the displaced load
//! runs. The load itself is the game's own operand — if it faults, the game would have.
//! ★ 163차: MAIN ONLY (`flavor = "main"`, with the three summary painters) — the item table
//! stays flavour-free (77차-B), but the third option is 개정판 and vanilla cannot obtain the 11.

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use wie_util::{
    ByteRead, Result, WieError,
    acc_options::{self, Third},
    read_generic, stats,
};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldr r0,[sp,#0x48]` at `0x4f130` — the displaced instruction.
pub const HEAD: [u8; 2] = [0x12, 0x98];
/// `addStat 0x4ee34(id, value)`, Thumb.
pub const ADD_STAT: u32 = 0x0004_ee34 | 1;
/// What `r6` holds in the loop: the stat block (`CHAR+0xb4`, the same block `addHP` takes).
pub const STAT_BLOCK: u32 = super::playerhp::STAT_BLOCK;
/// The worn slots the loop walks (`EQUIPPED[0..9]`).
pub const SLOTS: u32 = 9;
/// `[sp+0x48]` — `*(grid+0)`, the per-category row tables (the displaced load's operand).
pub const TABLES_SP: u32 = 0x48;
/// `[sp+0x64]` / `[sp+0x60]` — the recompute's `preview` and `slot` arguments (logs only).
pub const PREVIEW_SP: u32 = 0x64;
pub const PREVIEW_SLOT_SP: u32 = 0x60;

/// Applications logged (first N only — a recompute runs on every wear, preview and buff).
const REPORT_MAX: u32 = 12;
static REPORTED: AtomicU32 = AtomicU32::new(0);

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "acc_third_stat hook at {pc:#x}: expected {HEAD:02x?} (ldr r0,[sp,#0x48]), found {head:02x?}"
    )))
}

/// The `addStat(id, value)` the third option of `(cat, idx)` feeds the recompute, pure —
/// `None` for an item without a third, a host-effect third, or an id `addStat` would not
/// add (0x04 is its no-op arm, 0x13.. are mode switches; the generated table already
/// refuses those, this is the hook's own check).
pub fn stat_pair(cat: i16, idx: i16) -> Option<(u8, i8)> {
    if cat < 0 || idx < 0 {
        return None;
    }
    let t = acc_options::third_of(cat as u16, idx as u16)?;
    match t.third {
        Third::Stat(id) if acc_options::guest_stat(id) => Some((id, t.value)),
        _ => None,
    }
}

/// Should this loop iteration add a third? Pure: the frame check (`r6` = the stat block,
/// `r5` a slot), the A/B switch, then the table.
pub fn applies(block: u32, slot: u32, on: bool, cat: i16, idx: i16) -> Option<(u8, i8)> {
    if block != STAT_BLOCK || slot >= SLOTS || !on {
        return None;
    }
    stat_pair(cat, idx)
}

/// `0x4f130`: maybe `addStat` the slot's third, then the displaced `ldr r0,[sp,#0x48]`.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (cat, idx, slot, block, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R1) as i16,
            inner.engine.reg_read(ArmRegister::R2) as i16,
            inner.engine.reg_read(ArmRegister::R5),
            inner.engine.reg_read(ArmRegister::R6),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    if let Some((id, value)) = applies(block, slot, stats::third_opts_on(), cat, idx) {
        if REPORTED.fetch_add(1, Ordering::Relaxed) < REPORT_MAX {
            let mut b = [0u8; 1];
            let preview = core.read_bytes(sp + PREVIEW_SP, &mut b).ok().map(|_| b[0]);
            let pslot = core.read_bytes(sp + PREVIEW_SLOT_SP, &mut b).ok().map(|_| b[0]);
            let name = acc_options::third_of(cat as u16, idx as u16).map_or("?", |t| t.name);
            tracing::warn!(
                "third option: slot {slot} ({cat:#x},{idx:#x}) {name} preview={preview:?} (preview slot {pslot:?}) -> addStat({id:#x},{value})"
            );
        }
        // The game's own add, with the loop's registers restored around it.
        core.run_function::<()>(ADD_STAT, &[u32::from(id), value as i32 as u32]).await?;
    }
    let tables: u32 = read_generic(core, sp + TABLES_SP)?;
    core.inner.lock().engine.reg_write(ArmRegister::R0, tables);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use wie_util::{ByteWrite, acc_options::ACC_THIRDS};

    use super::*;

    #[test]
    fn the_hook_only_installs_over_the_displaced_load() {
        assert!(check_head([0x12, 0x98], 0x4f131).is_ok(), "ldr r0,[sp,#0x48]");
        assert!(check_head([0x8b, 0x00], 0x4f133).is_err(), "the `lsls r3,r1,#2` after it is the exit");
        assert!(check_head([0xff, 0xf7], 0x4f14f).is_err(), "the second addStat call is not the site");
    }

    /// Every table row resolves by its own `(cat, idx)`; a `Stat` row gives its pair, every
    /// other kind (host effects) gives nothing to `addStat`.
    #[test]
    fn every_stat_third_resolves_and_nothing_else_does() {
        let mut stats_seen = 0;
        for t in &ACC_THIRDS {
            assert!(acc_options::third_of(u16::from(t.cat), u16::from(t.idx)).is_some_and(|x| x.idx == t.idx), "{}", t.name);
            let got = stat_pair(i16::from(t.cat), i16::from(t.idx));
            match t.third {
                Third::Stat(id) => {
                    assert_eq!(got, Some((id, t.value)), "{}", t.name);
                    stats_seen += 1;
                }
                _ => assert_eq!(got, None, "{} is a host effect", t.name),
            }
        }
        assert_eq!(stats_seen, 6, "율리케 · 레갈리스 · 엘지스 · 일레느 · 클레르 · 닉스");
        assert_eq!(stat_pair(0x0c, 0x2a), Some((0x06, 30)), "율리케의목걸이: 특수공격 +30");
        assert_eq!(stat_pair(0x0c, 0x28), None, "타이탄의 파편's 공격속도 is the host's");
        assert_eq!(stat_pair(0x0c, 0x27), None, "a stock accessory");
        assert_eq!(stat_pair(0x00, 0x30), None, "템플러헬름");
        assert_eq!(stat_pair(-1, 0x2a), None, "an empty slot");
        assert_eq!(stat_pair(0x0c, -1), None);
    }

    #[test]
    fn the_frame_check_and_the_switch_gate_the_add() {
        assert_eq!(applies(STAT_BLOCK, 2, true, 0x0c, 0x2a), Some((0x06, 30)));
        assert_eq!(applies(STAT_BLOCK, 8, true, 0x0c, 0x32), Some((0x0a, 20)), "the second accessory slot");
        assert_eq!(applies(STAT_BLOCK, 9, true, 0x0c, 0x2a), None, "r5 is not a slot");
        assert_eq!(applies(STAT_BLOCK + 4, 2, true, 0x0c, 0x2a), None, "r6 is not the stat block");
        assert_eq!(applies(STAT_BLOCK, 2, false, 0x0c, 0x2a), None, "--third-opts 0");
        assert_eq!(STAT_BLOCK, 0x0150_3b3c);
    }

    /// The shipped `binary.mod` (md5 6a352944…, tracked at `tools_re/_binary.mod`), when this
    /// checkout has it: the hook site's bytes, and `addStat`'s jump table — `guest_stat(id)` must
    /// be exactly "the arm ADDS the value into an accumulator" (id 0x04 is the no-op tail, 0x13..
    /// switch modes, 0x17 CLEARS `+0xc0`).
    #[test]
    fn the_binary_agrees() {
        let path = std::format!("{}/../../../tools_re/_binary.mod", env!("CARGO_MANIFEST_DIR"));
        let Ok(elf) = std::fs::read(&path) else {
            std::eprintln!("skipped: {path} is not in this checkout");
            return;
        };
        let fo = |va: u32| (va - 0x1000 + 0x34) as usize;
        let h = |va: u32| u16::from_le_bytes([elf[fo(va)], elf[fo(va) + 1]]);
        let w = |va: u32| u32::from_le_bytes(elf[fo(va)..fo(va) + 4].try_into().unwrap());
        assert_eq!(elf[fo(0x4f130)..fo(0x4f130) + 2], HEAD, "the hook site");
        assert_eq!(h(0x4ee4c), 0x2b15, "addStat's bound: cmp r3,#0x15 on id-2");
        // The shared tail every additive arm reaches: ldrh r3,[r2] / adds r3,r5,r3 / b -> strh.
        const TAIL: u32 = 0x4ee6c;
        assert_eq!([h(TAIL), h(TAIL + 2)], [0x8813, 0x18eb]);
        let mut fields = std::vec::Vec::new();
        for id in 0x02u8..=0x17 {
            let t = w(0x5e1cc + u32::from(id - 2) * 4) & !1;
            let additive = if h(t) == 0x8f83 {
                // id 2 inline: ldrh r3,[r0,#0x3c] / adds r3,r5,r3 / strh r3,[r0,#0x3c]
                assert_eq!([h(t + 2), h(t + 4)], [0x18eb, 0x8783]);
                fields.push((id, 0x3c));
                true
            } else if h(t) == 0x1c02 && h(t + 2) >> 8 == 0x32 {
                // adds r2,r0,#0 / adds r2,#field, then either the tail itself or a branch to it
                let third = h(t + 4);
                let reaches_tail = if t + 4 == TAIL {
                    true
                } else if third >> 11 == 0b11100 {
                    let off = i32::from(((third & 0x7ff) as i16) << 5 >> 5) * 2;
                    (t + 4 + 4).wrapping_add(off as u32) == TAIL
                } else {
                    false
                };
                if reaches_tail {
                    fields.push((id, h(t + 2) & 0xff));
                }
                reaches_tail
            } else {
                false
            };
            assert_eq!(acc_options::guest_stat(id), additive, "id {id:#x} -> {t:#x}");
        }
        assert!(!acc_options::guest_stat(0x00) && !acc_options::guest_stat(0x01) && !acc_options::guest_stat(0x18));
        // The accumulator map the RE report and CLAUDE.md quote (report_stat-apply §1.4).
        let want: [(u8, u16); 16] = [
            (0x02, 0x3c),
            (0x03, 0x42),
            (0x05, 0x4e),
            (0x06, 0x50),
            (0x07, 0x54),
            (0x08, 0x58),
            (0x09, 0x5a),
            (0x0a, 0x5e),
            (0x0b, 0x60),
            (0x0c, 0x64),
            (0x0d, 0x68),
            (0x0e, 0x6a),
            (0x0f, 0x6c),
            (0x10, 0x6e),
            (0x11, 0x70),
            (0x12, 0x72),
        ];
        assert_eq!(fields, want);
    }

    /// The handler end to end on a live core: a stand-in `addStat` at `0x4ee34` records its
    /// two arguments, so the test sees exactly what the game's function would have been given
    /// — and that the loop's registers came back and `r0` holds the displaced load.
    #[futures_test::test]
    async fn the_handler_calls_add_stat_then_performs_the_load() -> Result<()> {
        const SCRATCH: u32 = 0x4f800;
        const SP: u32 = 0x10_f000;
        let mut core = ArmCore::new(false, None)?;
        core.map(0x4e000, 0x2000)?;
        core.map(0x10_0000, 0x10000)?;
        // ldr r2,[pc,#4] / str r0,[r2] / str r1,[r2,#4] / bx lr / .word SCRATCH
        let mut stub = std::vec![0x01, 0x4a, 0x10, 0x60, 0x51, 0x60, 0x70, 0x47];
        stub.extend_from_slice(&SCRATCH.to_le_bytes());
        core.write_bytes(ADD_STAT & !1, &stub)?;
        core.write_bytes(SP + TABLES_SP, &0x4014_0000u32.to_le_bytes())?;
        core.write_bytes(SP + PREVIEW_SP, &[1])?;
        core.write_bytes(SP + PREVIEW_SLOT_SP, &[2])?;

        let set = |core: &mut ArmCore, cat: u32, idx: u32, slot: u32, block: u32| {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R1, cat);
            inner.engine.reg_write(ArmRegister::R2, idx);
            inner.engine.reg_write(ArmRegister::R4, idx + 1);
            inner.engine.reg_write(ArmRegister::R5, slot);
            inner.engine.reg_write(ArmRegister::R6, block);
            inner.engine.reg_write(ArmRegister::R7, 0x4012_3450);
            inner.engine.reg_write(ArmRegister::SP, SP);
        };

        // 클레르의가보 in the second accessory slot: addStat(0x05, 30).
        core.write_bytes(SCRATCH, &[0xee; 8])?;
        set(&mut core, 0x0c, 0x30, 8, STAT_BLOCK);
        let JumpTo(pc) = handle_lgt(&mut core, 0x4f131, 0x4f133).await?;
        assert_eq!(pc, 0x4f133);
        let got: [u32; 2] = [read_generic(&core, SCRATCH)?, read_generic(&core, SCRATCH + 4)?];
        assert_eq!(got[0] & 0xffff, 0x05, "id");
        assert_eq!(got[1] & 0xffff, 30, "value");
        {
            let inner = core.inner.lock();
            assert_eq!(inner.engine.reg_read(ArmRegister::R0), 0x4014_0000, "the displaced load");
            assert_eq!(inner.engine.reg_read(ArmRegister::R1), 0x0c, "cat survives the call");
            assert_eq!(inner.engine.reg_read(ArmRegister::R2), 0x30, "idx survives the call");
            assert_eq!(inner.engine.reg_read(ArmRegister::R5), 8);
            assert_eq!(inner.engine.reg_read(ArmRegister::R6), STAT_BLOCK);
            assert_eq!(inner.engine.reg_read(ArmRegister::R7), 0x4012_3450);
            assert_eq!(inner.engine.reg_read(ArmRegister::SP), SP);
        }

        // Not a third / a host-effect third / a failed frame check: only the load.
        for (cat, idx, slot, block) in [(0x0c, 0x27, 2, STAT_BLOCK), (0x0c, 0x28, 2, STAT_BLOCK), (0x0c, 0x2a, 9, STAT_BLOCK), (0x0c, 0x2a, 2, 0x1234)] {
            core.write_bytes(SCRATCH, &[0xee; 8])?;
            set(&mut core, cat, idx, slot, block);
            core.inner.lock().engine.reg_write(ArmRegister::R0, 0);
            let JumpTo(pc) = handle_lgt(&mut core, 0x4f131, 0x4f133).await?;
            assert_eq!(pc, 0x4f133);
            let untouched: u32 = read_generic(&core, SCRATCH)?;
            assert_eq!(untouched, 0xeeee_eeee, "({cat:#x},{idx:#x}) slot {slot} block {block:#x}: no addStat");
            assert_eq!(core.inner.lock().engine.reg_read(ArmRegister::R0), 0x4014_0000);
        }
        Ok(())
    }
}
