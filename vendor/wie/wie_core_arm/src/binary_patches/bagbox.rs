//! Bag boxes — container-category items handed to the HOST instead of the guest's bag.
//!
//! ── WHY TWO ENTRY HOOKS ──────────────────────────────────────────────────────
//! The player keeps orbs, materials and consumables in three host-owned boxes that open
//! from one bag cell each (`BAG_TYPES_FEASIBILITY.md`). "Every acquisition route lands in
//! the box, even when the bag is full" needs the item to never touch a bag cell, and the
//! game has exactly two functions through which anything is placed:
//!
//!   * `giveItem 0x52b14(cat, idx, announce, r3)` — the drop the player walks over
//!     (`0x54a68`, inside the loot-collision loop that starts at `0x549d8`), NPC shops
//!     (`0x1e754`), script gives (`0x3d592`, `0x469e4`), character creation (`0x493fe`),
//!     `giveItemN 0x5593c`, 오브원석 (`0x551da`), the host's own grants. One UNIT per call;
//!     returns 1 on success, 0 when the bag is full (`0x532a6` / `0x52b98`).
//!     ⛔ Its third argument is the ANNOUNCE flag — see [`give_announces`]. Earlier
//!     revisions of this comment had `0x3d592` down as the drop and `0x54a68` as crafting;
//!     both are wrong, and it was the drop's own call site that settled the flag.
//!   * `addItemToInventory 0x131d0(rec*)` — the two wire deliveries, warehouse `05 04`
//!     (`0x13cec`) and cash `05 40` (`0x13e4c`). `rec` is the 20-byte wire record with
//!     `cat` at `+8`, `idx` at `+9` and a SIGNED count at `+10` (72차). Returns the units
//!     LEFT OVER (0 = all placed; the handler calls it a second time with a non-zero
//!     return, `0x13d24`/`0x13eb6`), `0xff` when nothing could be placed.
//!
//! Both begin with `push {r4, r5, r6, r7, lr}` (`f0 b5`), the same shape as `blit565`, so
//! declining is [`super::hook::resume_past_push`] and the guest never notices.
//!
//! ── THE CONTRACT ─────────────────────────────────────────────────────────────
//! Every decision is [`wie_util::stats::bagbox_offer`]: the host publishes which
//! categories it takes, how many free stacks each box has and how many units each item's
//! stacks can still take, and the hook pushes what it took into a ring the host drains
//! every frame. A refused offer (feature off, box full, ring full, a bag-item cell itself)
//! falls through to the game's own placement — the box is only ever a place an item goes
//! INSTEAD of the bag, never a place it is lost.
//!
//! ★★★ 165차: everything a single guest call does is settled by those knobs alone, because
//! nothing drains inside one. So a stack is spent only when a unit cannot merge, units of
//! the item the newest ring entry already names FOLD into it (the shop gives a 99-unit
//! purchase as 99 calls of this hook in one key handler — one entry and one stack per unit
//! spilled everything past min(64, free stacks) into the bag), and an accepted unit is in
//! the box MIRROR at once (a scene script that gives a box item and takes it back a few
//! opcodes later — 리파이너의 유적 7's boss door, 그리폰의피리 — found it missing and said
//! 「이벤트 실행에 필요한 아이템이 부족합니다」).
//!
//! ⛔ Ships on BOTH flavours: the bag boxes are a feature of the remake, not of the item
//! tables, and with `BAGBOX_ACCEPT == 0` the hooks are byte-for-byte the game.
//!
//! ── WHAT THE HOOK OWES THE GAME BACK (★★★ 103차) ─────────────────────────────
//! Returning early skips more than the placement. Both functions queue the floating
//! "you got it" item name AFTER placing (`giveItem` at `0x52e04` for equipment and
//! `0x53114` for everything else, `addItemToInventory` at `0x134c4`), so an absorbed
//! drop lost a notice the same drop used to show — while equipment, which the boxes
//! never take, kept it. The hook cannot draw, but it is the only thing that can see
//! whether the game meant to speak, so it carries that one bit into the ring
//! (`BAGBOX_ANNOUNCE`) and the host appends the name to the game's own queue.
//!
//! ── THE QUEST VM'S ITEM HELPER (3단계, 2026-09-15) ────────────────────────
//! Scripts hand items in through ONE helper, `0x3bcf0(cat, idx, u16* out, mode)`:
//!   * `mode != 0` — COUNT: `*out = 0`, then every bag cell whose `(cat, idx)` match
//!     adds its count (`cat > 0x0c`) or 1. Callers: the `fe <cat><idx><n>` condition
//!     (`0x3bf5a`) and the take op `0x12` before it takes (`0x3d4aa`).
//!   * `mode == 0` — TAKE: `removeItem(tab, slot, (s8)*out)` on the first matching cell
//!     of each tab (`0x3bda4`). Caller: op `0x12` (`0x3d4e8`).
//! The corpus has 19 scripts taking cat-0x0e materials and two taking cat-0x12
//! consumables, and those items live in the host's boxes now — so the helper needs them:
//!   * [`handle_count_lgt`] sits on the helper's ONE exit (`0x3bdd2 add sp,#0x18`) and,
//!     in count mode, ADDS the box mirror (`wie_util::stats::bagbox_count`) to `*out`.
//!     The guest's own bag walk stays the guest's; the hook performs the displaced
//!     `add sp` and resumes at `exit_pc`.
//!   * [`handle_take_lgt`] sits on the entry (the same `push {r4-r7,lr}`): in take mode,
//!     when the box holds all `n`, it spends them from the box (`bagbox_take` → a ring
//!     entry with `BAGBOX_SRC_TAKE`) and returns to the caller WITHOUT the bag walk.
//!     Fewer than `n` in the box, count mode, a non-box category, feature off: the
//!     game's own code runs (`resume_past_push`).
//! ⛔ A bag+box split that only SUMS to `n` is taken from the bag by the game (the hook
//! declined). The sweep keeps the bag empty of box categories outside a window, so that
//! state is transient — `BAG_TYPES_FEASIBILITY.md §8`.

use alloc::format;

use wie_util::{Result, WieError, read_generic, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `push {r4, r5, r6, r7, lr}` — the first instruction of both functions.
pub const EXPECTED_HEAD: [u8; 2] = [0xf0, 0xb5];

/// `giveItem` returns this on success (`0x532a6 movs r0,#1`).
pub const GIVE_OK: u32 = 1;
/// `addItemToInventory` returns the units left over; 0 = everything was placed.
pub const ADD_ALL_PLACED: u32 = 0;

/// `add sp, #0x18` at `0x3bdd2` — the one exit of the quest VM's item helper.
pub const EXPECTED_VM_TAIL: [u8; 2] = [0x06, 0xb0];
/// The helper's frame: `sub sp, #0x18` at `0x3bcf4`, with `mode` (r3, low byte) spilled
/// to `[sp+0xc]`, `idx` (r1) to `[sp+0x10]`, `cat` (r0) to `[sp+0x14]`; `r5` keeps `out`.
pub const VM_FRAME: u32 = 0x18;
pub const VM_SP_MODE: u32 = 0xc;
pub const VM_SP_IDX: u32 = 0x10;
pub const VM_SP_CAT: u32 = 0x14;
/// `*out` is read back by the callers with `ldrsh` — never let the sum go negative.
pub const VM_COUNT_MAX: u32 = 0x7fff;

/// Offsets inside the 20-byte wire record `addItemToInventory` takes.
pub const WIRE_CAT: u32 = 8;
pub const WIRE_IDX: u32 = 9;
pub const WIRE_COUNT: u32 = 10;

/// The value of the announce argument that means "place it, but say nothing".
pub const SILENT: u32 = 0xff;

/// Would the game have announced this acquisition? (★★★ 103차)
///
/// `giveItem`'s third argument is nothing but an announce flag. It is masked to a byte on
/// entry (`0x52b16 lsls #24 / 0x52b1c lsrs #24`) and both placement branches then test it
/// the same way — the equipment one sign-extended (`0x52dfc asrs #24 / adds #1 / bne`), the
/// other against `0xff` (`0x5310e cmp #0xff`) — and on a match SKIP the block that appends
/// the item's name to the floating notice queue. The callers agree: the drop the player
/// walks over passes the loot's index (`0x54a5a`) and `giveItemN` the unit's ordinal
/// (`0x5594e`), while the shop (`0x1e74c`), both script gives (`0x469da`, `0x3d58e`) and
/// 오브원석 (`0x551d2`) pass `-1` because the game speaks for those itself.
///
/// ⛔ Do not simplify this to "always announce". Absorbing an item must not ADD a notice
/// the bag would not have shown either — a scripted give would then narrate itself twice.
pub fn give_announces(arg: u32) -> bool {
    arg & 0xff != SILENT
}

/// The same question for the wire route. `addItemToInventory` reads the record's SIGNED
/// count byte and skips its own notice block only at `-1` (`0x134ba ldrsb / adds #1 / bne`)
/// — so every count [`wire_triple`] accepts (it requires a positive one) is announced.
pub fn wire_announces(count: u8) -> bool {
    count as i8 != -1
}

/// The ring's announce flag, or nothing.
fn announce_bit(yes: bool) -> u8 {
    if yes { wie_util::stats::BAGBOX_ANNOUNCE } else { 0 }
}

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "bagbox hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (push {{r4-r7,lr}}), found {head:02x?}"
    )))
}

pub fn check_vm_tail(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_VM_TAIL {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "bagbox count hook at {pc:#x}: expected {EXPECTED_VM_TAIL:02x?} (add sp,#0x18), found {head:02x?}"
    )))
}

/// `add sp, #0xc` at `0x555d0` — countOf's one exit (frame is NOT the quest helper's).
pub const EXPECTED_COUNTOF_TAIL: [u8; 2] = [0x03, 0xb0];
pub const COUNTOF_FRAME: u32 = 0xc;
pub const COUNTOF_SP_IDX: u32 = 4;
pub const COUNTOF_SP_CAT: u32 = 8;

pub fn check_countof_tail(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_COUNTOF_TAIL {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "bagbox countof hook at {pc:#x}: expected {EXPECTED_COUNTOF_TAIL:02x?} (add sp,#0xc), found {head:02x?}"
    )))
}

/// The count the exit hook writes back: what the guest counted in the bag plus what the
/// box holds, saturated at what an `ldrsh` reader can hold.
pub fn vm_count_sum(bag: u16, boxed: u32) -> u16 {
    (u32::from(bag) + boxed).min(VM_COUNT_MAX) as u16
}

/// Should the take hook spend from the box? Pure: mode must be TAKE (0), the operand a
/// positive signed byte (the guest reads it `ldrsb`), the category one the boxes hold.
pub fn vm_take_request(mode: u32, cat: u32, idx: u32, operand: u8) -> Option<(u32, u32, u8)> {
    if mode & 0xff != 0 || cat > 0x1f || idx > 0xff {
        return None;
    }
    let n = operand as i8;
    if n <= 0 {
        return None;
    }
    wie_util::stats::bagbox_box_of(cat)?;
    Some((cat, idx, n as u8))
}

/// The wire record's `(cat, idx, count)` as the guest reads them: `cat`/`idx` unsigned
/// bytes, `count` a SIGNED byte (`0x135da ldrsb`). `None` for a count the guest would
/// treat as a subtraction — that record takes the game's own path.
pub fn wire_triple(cat: u8, idx: u8, count: u8) -> Option<(u32, u32, u8)> {
    let n = count as i8;
    if n <= 0 {
        return None;
    }
    Some((u32::from(cat), u32::from(idx), n as u8))
}

/// `giveItem` entry: offer ONE unit of `(r0, r1)`; taken → return 1 to the caller,
/// else run the game's own function.
pub async fn handle_give_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    let (cat, idx, announce) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::R2),
        )
    };
    // The guest masks both to 16 bits at `0x52b66`..`0x52b74` (`lsls #16 / asrs #16`).
    let (cat, idx) = (cat & 0xffff, idx & 0xffff);
    let source = wie_util::stats::BAGBOX_SRC_GIVE | announce_bit(give_announces(announce));
    if wie_util::stats::bagbox_offer(cat, idx, 1, source) {
        core.inner.lock().engine.reg_write(ArmRegister::R0, GIVE_OK);
        return Ok(JumpTo(lr));
    }
    super::hook::resume_past_push(core, hook_pc)
}

/// `addItemToInventory` entry: offer the wire record's whole count; taken → report
/// "nothing left over", else run the game's own function.
pub async fn handle_add_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    let rec = core.inner.lock().engine.reg_read(ArmRegister::R0);
    let cat: u8 = read_generic(core, rec + WIRE_CAT)?;
    let idx: u8 = read_generic(core, rec + WIRE_IDX)?;
    let count: u8 = read_generic(core, rec + WIRE_COUNT)?;
    let source = wie_util::stats::BAGBOX_SRC_WIRE | announce_bit(wire_announces(count));
    if let Some((cat, idx, n)) = wire_triple(cat, idx, count)
        && wie_util::stats::bagbox_offer(cat, idx, n, source)
    {
        core.inner.lock().engine.reg_write(ArmRegister::R0, ADD_ALL_PLACED);
        return Ok(JumpTo(lr));
    }
    super::hook::resume_past_push(core, hook_pc)
}

/// ★★ 166차 — the game's "can the BAG take `n` of this?" check, `0x3c0e4(cat, idx, n)`.
///
/// Its only two callers ask it right before they place through `giveItem`: the NPC shop
/// before it sells (`0x1e70c`, then `giveItem(cat, idx, -1, 0)` × n at `0x1e754`) and script
/// op 0x14 before it gives (`0x3d54a`, then `giveItem(cat, idx, -1, 1)` × n at `0x3d592`). It
/// counts bag cells only (empty cells × 99 plus a matching stack's room — `0x3c172..0x3c214`),
/// so a full bag refused a purchase the 소비가방 had room for — 「가방이 가득…」, or
/// 「인벤토리가 부족하여 이벤트를 실행할 수 없습니다」 (InGame_txt 136) for a script.
///
/// The hook takes out of `n` what the box will take ([`wie_util::stats::bagbox_room_for`])
/// and lets the game judge the REST against the bag — exactly the split the gives that
/// follow will make: units into the box until it is full, the remainder down the game's
/// own path. A box that takes all of `n` leaves 0, which the game answers "fits"
/// (`0x3c216`: `0 >= 0`). The function has no side effects, so the register is all that
/// changes; anything the box cannot speak for is the game's own answer, byte for byte.
pub fn room_check_rest(cat: i16, idx: u32, n: i32) -> Option<u32> {
    if cat < 0 || n <= 0 {
        return None;
    }
    let boxed = wie_util::stats::bagbox_room_for(cat as u32, idx, n as u32);
    (boxed > 0).then(|| n as u32 - boxed)
}

/// `0x3c0e4` entry: shrink `n` by what the box takes, then run the game's own check.
pub async fn handle_room_check_lgt(core: &mut ArmCore, hook_pc: u32, _lr: u32) -> Result<JumpTo> {
    let (r0, r1, r2) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::R2),
        )
    };
    // The guest's own masking: cat `lsls/lsrs/lsls/asrs #16` (`0x3c0f6`..`0x3c10c`), idx
    // `lsls/lsrs #16` (`0x3c0e8`), and `n` compared SIGNED at the end (`0x3c21c blt`).
    let (cat, idx, n) = ((r0 & 0xffff) as u16 as i16, r1 & 0xffff, r2 as i32);
    if let Some(rest) = room_check_rest(cat, idx, n) {
        core.inner.lock().engine.reg_write(ArmRegister::R2, rest);
        // warn, not info (the Android filter): "why did the shop sell into a full bag" — or
        // why it did not — is answered by this line.
        tracing::warn!("bagbox: the bag-room check for cat {cat:#x} idx {idx:#x} x{n} — the box takes {}, the bag is asked for {rest}", n as u32 - rest);
    }
    super::hook::resume_past_push(core, hook_pc)
}

/// ★★ 166차 — the cash shop's own full-bag gate, inside its OK-on-a-row arm (`0x72b2`).
///
/// `0x72dc` asks `isBagFull 0x535e8` (empty cells only) and, when the bag is full, runs a
/// partial-stack test that refuses at `0x7398` unless a stack of the row's item already sits
/// in the bag (`0x72e8..0x736a`) — never true for a box item, which the sweep keeps out of
/// the bag. Every row it refuses that way is then GRANTED by `addItemToInventory` (the cash
/// `05 40` grant `0x13e4c`, the engine's `05 04` for 소켓확장), whose entry hook would have
/// taken it whole into the box. So the hook re-answers "full" with "not full" when the box
/// takes the row's whole count, and the game goes down its own not-full path (`0x739c`: the
/// 여행가방 cap test, then the confirm). Equipment, quest items, a full box: the game's answer.
///
/// Site = `0x72e0 lsls r0, r0, #0x18` (the result's byte to the top, flags dead by `0x72e4
/// cmp`), re-done by the hook. At that point r4 = the row index (`0x72ca ldrb`) and `[sp,
/// #0x18]` = the shop's row base (read again at `0x72ee`); the row's wire bytes sit at
/// `base + row*0x14 + 0x328` — cat, idx, count, each read `ldrsb` (`0x72fc`, `0x72fa`, `0x7314`).
pub const CASH_GATE_HEAD: [u8; 2] = [0x00, 0x06];
pub const CASH_SP_ROWS: u32 = 0x18;
pub const CASH_ROW_STRIDE: u32 = 0x14;
pub const CASH_ROW_CAT: u32 = 0x328;

pub fn check_cash_gate_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == CASH_GATE_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "bagbox cash gate hook at {pc:#x}: expected {CASH_GATE_HEAD:02x?} (lsls r0,r0,#0x18), found {head:02x?}"
    )))
}

/// Does the box take ALL of the cash row the cursor is on? The three bytes as the guest
/// reads them (`ldrsb`): a negative one is never ours to answer for.
pub fn cash_gate_opens(cat: u8, idx: u8, count: u8) -> bool {
    let (cat, idx, n) = (cat as i8, idx as i8, count as i8);
    if cat < 0 || idx < 0 || n <= 0 {
        return false;
    }
    let n = n as u32;
    wie_util::stats::bagbox_room_for(cat as u32, idx as u32, n) == n
}

/// The row's `(cat, idx, count)`, or `None` when any of it cannot be read — the game's own
/// answer then stands.
fn cash_row(core: &mut ArmCore, sp: u32, row: u32) -> Option<(u8, u8, u8)> {
    let rows: u32 = read_generic(core, sp.wrapping_add(CASH_SP_ROWS)).ok()?;
    let at = rows
        .wrapping_add(((row as u8 as i8) as i32 * CASH_ROW_STRIDE as i32) as u32)
        .wrapping_add(CASH_ROW_CAT);
    let cat: u8 = read_generic(core, at).ok()?;
    let idx: u8 = read_generic(core, at.wrapping_add(1)).ok()?;
    let count: u8 = read_generic(core, at.wrapping_add(2)).ok()?;
    Some((cat, idx, count))
}

pub async fn handle_cash_gate_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (full, row, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R4),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    let mut answer = full;
    if full & 0xff != 0
        && let Some((cat, idx, count)) = cash_row(core, sp, row)
        && cash_gate_opens(cat, idx, count)
    {
        answer = 0;
        // warn, not info (the Android filter): the only trace of "why did the cash shop sell
        // into a full bag".
        tracing::warn!("bagbox: the cash shop's full-bag gate — cat {cat:#x} idx {idx:#x} x{count} fits the box, so the purchase goes on");
    }
    core.inner.lock().engine.reg_write(ArmRegister::R0, answer << 24);
    Ok(JumpTo(exit_pc))
}

/// `0x3bcf0` entry: in TAKE mode, spend the script's `n` from the host's box when the
/// box holds them all and return to the caller; otherwise run the game's own bag walk.
pub async fn handle_take_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    let (cat, idx, out, mode) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::R2),
            inner.engine.reg_read(ArmRegister::R3),
        )
    };
    let operand: u8 = read_generic(core, out)?;
    if let Some((cat, idx, n)) = vm_take_request(mode, cat, idx, operand)
        && wie_util::stats::bagbox_take(cat, idx, n)
    {
        // warn, not info: the Android build filters the wie crates to warn, and "did the
        // quest take its hand-in from the box" is the first question a stuck quest raises.
        tracing::warn!("bagbox: the quest script took cat {cat:#x} idx {idx:#x} x{n} from the host's box");
        return Ok(JumpTo(lr));
    }
    super::hook::resume_past_push(core, hook_pc)
}

/// `0x3bcf0` exit: in COUNT mode, add the box mirror to what the bag walk counted, then
/// perform the displaced `add sp, #0x18` and resume.
pub async fn handle_count_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (sp, out) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(ArmRegister::R5))
    };
    let mode: u32 = read_generic(core, sp + VM_SP_MODE)?;
    let cat: u32 = read_generic(core, sp + VM_SP_CAT)?;
    let idx: u32 = read_generic(core, sp + VM_SP_IDX)?;
    if mode & 0xff != 0 && cat <= 0x1f && idx <= 0xff {
        let boxed = wie_util::stats::bagbox_count(cat, idx);
        if boxed > 0 {
            let bag: u16 = read_generic(core, out)?;
            let sum = vm_count_sum(bag, boxed);
            write_generic(core, out, sum)?;
            tracing::warn!("bagbox: the quest script counted cat {cat:#x} idx {idx:#x}: bag {bag} + box {boxed} -> {sum}");
        }
    }
    core.inner.lock().engine.reg_write(ArmRegister::SP, sp.wrapping_add(VM_FRAME));
    Ok(JumpTo(exit_pc))
}

fn bag_total(r0: u32) -> u16 {
    let n = r0 as i32;
    if n <= 0 { 0 } else { (n as u32).min(VM_COUNT_MAX) as u16 }
}

/// How many of `n` the box can cover. `None` = run the game's own spend (empty box,
/// unknown category, n==0). ⛔ Do not debit then resume the bag walk for the same `n`:
/// that is a free craft (countOf said yes, spend walks an empty bag).
pub fn spend_from_box(cat: u32, idx: u32, n: u32) -> Option<u32> {
    if n == 0 || cat > 0x1f || idx > 0xff {
        return None;
    }
    let boxed = wie_util::stats::bagbox_count(cat, idx);
    if boxed == 0 {
        return None;
    }
    Some(boxed.min(n))
}

fn take_units(cat: u32, idx: u32, n: u32) -> u32 {
    let mut left = n;
    let mut took = 0u32;
    while left > 0 {
        let chunk = left.min(255) as u8;
        if !wie_util::stats::bagbox_take(cat, idx, chunk) {
            break;
        }
        took += u32::from(chunk);
        left -= u32::from(chunk);
    }
    took
}

/// The last `countOf` answer the hook logged, packed by [`countof_log_key`]; `0` = none.
///
/// ★ 129차 (128차 code review ②): the smith's painters call `countOf` for every material
/// row on EVERY paint (`0x21ad0` 일반조합 · `0x221f8/0x223e0/0x225d0` 특수조합 · `0x25214`
/// 제련), and with the materials living in the box the `boxed > 0` arm is the steady
/// state — so an unconditional `warn!` was ~100 logcat lines a second for as long as a
/// craft window stayed open, which wraps the 256 KB ring in seconds and takes the box
/// diagnostics (`logcat -d | grep bagbox`, the first command of every device check) with
/// it. Now a line goes out only when `(cat, idx, bag, boxed)` is not among the last
/// [`COUNTOF_LOG_RING`] answers logged — a new material, or a change in what either side
/// holds. A RING, not a single "last" word: a painter asks about its two or three rows in
/// turn, so "differs from the previous call" would still be every call.
const COUNTOF_LOG_RING: usize = 8;
static COUNTOF_LOGGED: [core::sync::atomic::AtomicU64; COUNTOF_LOG_RING] = [const { core::sync::atomic::AtomicU64::new(0) }; COUNTOF_LOG_RING];
static COUNTOF_LOG_NEXT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// `(cat, idx, bag, boxed)` as one word, never 0 (bit 63 is always set) so that an empty
/// ring slot cannot collide with a real answer. Pure.
pub fn countof_log_key(cat: u32, idx: u32, bag: u16, boxed: u32) -> u64 {
    1u64 << 63 | u64::from(cat & 0xff) << 48 | u64::from(idx & 0xff) << 40 | u64::from(bag) << 24 | u64::from(boxed & 0xff_ffff)
}

/// Should this `countOf` answer be logged? `true` (and the key is remembered) unless the
/// same key was logged within the last [`COUNTOF_LOG_RING`] distinct answers.
fn countof_log_once(key: u64) -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if COUNTOF_LOGGED.iter().any(|w| w.load(Relaxed) == key) {
        return false;
    }
    let at = COUNTOF_LOG_NEXT.fetch_add(1, Relaxed) as usize % COUNTOF_LOG_RING;
    COUNTOF_LOGGED[at].store(key, Relaxed);
    true
}

/// countOf `0x55534` EXIT: r0 is the bag total (`asrs r0,r5,#16`). Cat/idx were spilled
/// to `[sp+8]`/`[sp+4]`. Add the box mirror when it holds any, then the displaced
/// `add sp,#0xc`. Stack read failure still restores SP (fail soft).
pub async fn handle_countof_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (sp, r0) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(ArmRegister::R0))
    };
    let cat: Result<u32> = read_generic(core, sp.wrapping_add(COUNTOF_SP_CAT));
    let idx: Result<u32> = read_generic(core, sp.wrapping_add(COUNTOF_SP_IDX));
    if let (Ok(cat), Ok(idx)) = (cat, idx)
        && cat <= 0x1f
        && idx <= 0xff
    {
        let boxed = wie_util::stats::bagbox_count(cat, idx);
        if boxed > 0 {
            let bag = bag_total(r0);
            let sum = vm_count_sum(bag, boxed);
            core.inner.lock().engine.reg_write(ArmRegister::R0, u32::from(sum));
            if countof_log_once(countof_log_key(cat, idx, bag, boxed)) {
                tracing::warn!("bagbox: countOf cat {cat:#x} idx {idx:#x}: bag {bag} + box {boxed} -> {sum}");
            }
        }
    }
    core.inner.lock().engine.reg_write(ArmRegister::SP, sp.wrapping_add(COUNTOF_FRAME));
    Ok(JumpTo(exit_pc))
}

/// `0x555e0(cat, idx, n)` ENTRY: debit the box first. JumpTo(lr) when it covers all n
/// (the bag walk would otherwise fail after a debit). Remainder in r2 + resume when
/// the box covers only some. Empty box / feature off: resume, byte-identical.
pub async fn handle_spend_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    let (cat, idx, n) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::R2) & 0xffff,
        )
    };
    let Some(want) = spend_from_box(cat, idx, n) else {
        return super::hook::resume_past_push(core, hook_pc);
    };
    let took = take_units(cat, idx, want);
    if took == 0 {
        return super::hook::resume_past_push(core, hook_pc);
    }
    tracing::warn!("bagbox: spend cat {cat:#x} idx {idx:#x} took {took} of {n} from the host's box");
    if took >= n {
        return Ok(JumpTo(lr));
    }
    core.inner.lock().engine.reg_write(ArmRegister::R2, n - took);
    super::hook::resume_past_push(core, hook_pc)
}

// ── RESTAGE: the box's own bag cell is where a boxed unit is used (127차, 2026-09-22) ──
//
// User: *"가방이 꽉차있어도 쓸 수 있어야해 … 3종가방 내에서 바로사용되는 로직으로 변경해야해"*.
// A boxed consumable used to be pushed into a FREE bag cell for the game to consume
// (`giveItem` with the bypass), and a full bag has no free cell. Now the host writes the
// unit's record over the box's OWN bag-item cell (오브가방/재료가방/소비가방 sit in bag 1,
// cells 1–3), drives the game's own use of that cell, and arms `wie_util::stats`'s restage
// knob with the bag-item record it overwrote.
//
// Every consumption ends in ONE primitive, `removeItem 0x5340c(tab, slot, n)`: the use tail
// (`0x55450` → `0x53648` → here), the hotkey / 귀환 dialog tail (`0x46784` → `0x53648`), the
// revive 예 (`0x3b58e`, n = 1), the quest VM's take. It spends `n`, and at zero clears the
// record to the empty form — after which every one of those tails runs the compactor
// `0x5380c`, which bubbles the hole to the end of the bag (and 오브원석's tail then SAVES).
// This hook sits on the primitive's entry (`push {r4-r7,lr}`, like the intake pair): for
// the ARMED cell, while the record there still reads as the staged item, it writes the
// bag-item record back over it and returns to the caller — the cell is never empty, the
// compactor finds no hole, the tail's save captures a whole bag, and the 소비가방 cell is
// back before the host even looks. Anything else (knob off, another cell, a record that is
// not the staged item any more) runs the game's own code, byte for byte.
//
// ⛔ The identity check is what makes a STALE knob harmless: the host may lose track of a
// staged unit (the world was left with it in the cell); the sweep or the game's own
// removal of some later occupant of that cell must not be answered with a 60-byte write.

/// The staged cell the primitive was asked about, or `None` when the knob is off or the
/// call names another cell. Pure — the register masking is the guest's own (`0x5340e`
/// `lsls #24` on the tab, `0x53410` `lsls #16` on the slot).
pub fn restage_target(armed: Option<wie_util::stats::BagboxRestage>, r0: u32, r1: u32) -> Option<wie_util::stats::BagboxRestage> {
    let a = armed?;
    if (r0 & 0xff) as u8 != a.tab || (r1 & 0xffff) as u16 != u16::from(a.slot) {
        return None;
    }
    Some(a)
}

/// Does the record at the armed address still read as the staged item? Occupied byte
/// (`+0x1f`), category (`+0x22`) and index (`+0x24`), the same three the host verified
/// when it staged.
pub fn restage_matches(occupied: u8, cat: u16, idx: u16, a: &wie_util::stats::BagboxRestage) -> bool {
    occupied != 0 && cat == u16::from(a.cat) && idx == u16::from(a.idx)
}

/// Would the game's own `removeItem(tab, slot, n)` EMPTY the cell? That is the only
/// removal this hook may answer with the bag-item record. Read off the primitive itself:
///
/// ```text
/// 0x5341e..  n = (s8)r2 ; if n <= 0 -> 0x534e4 (clear the cell)
/// 0x53462    count = (i16)[rec+0x3ba] ; if n >= count -> 0x534e4 (clear)
/// 0x5346a    else [rec+0x3ba] = count - n            (the stack stays, smaller)
/// ```
///
/// ★ 135차 §3 ⓒ: the staged cell is ONE unit when the host writes it, but it can grow — the
/// intake hook declines when the box is full or its ring is, `giveItem`'s `findSlot` then
/// merges the next unit of the same item into the staged stack, and a `removeItem(.., 1)`
/// (the revive 예 `0x3b58e`, say) on that count-2 cell used to have all 60 bytes written back
/// over it: one unit consumed, the merged one GONE. Now a removal that leaves units behind is
/// the game's own subtraction, the knob stays armed for the last one, and `end_job`'s
/// "consumed, yet the cell still reads as the item" arm leaves the rest to the sweep, which
/// restores the bag-item record and banks them in the box.
pub fn restage_clears(count: i16, n: i8) -> bool {
    n <= 0 || i16::from(n) >= count
}

/// `removeItem 0x5340c(tab, slot, n)` ENTRY: put the bag-item record back over the
/// staged cell instead of clearing it. JumpTo(lr) when it did; the game's own removal
/// (`resume_past_push`) for every other call.
pub async fn handle_restage_lgt(core: &mut ArmCore, hook_pc: u32, lr: u32) -> Result<JumpTo> {
    let Some(armed) = wie_util::stats::bagbox_restage() else {
        return super::hook::resume_past_push(core, hook_pc);
    };
    let (r0, r1, r2) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R0),
            inner.engine.reg_read(ArmRegister::R1),
            inner.engine.reg_read(ArmRegister::R2),
        )
    };
    let Some(a) = restage_target(Some(armed), r0, r1) else {
        return super::hook::resume_past_push(core, hook_pc);
    };
    // Identity, read where the host said the record is. A failed read is "not ours".
    let occupied: Result<u8> = read_generic(core, a.rec.wrapping_add(0x1f));
    let cat: Result<u16> = read_generic(core, a.rec.wrapping_add(0x22));
    let idx: Result<u16> = read_generic(core, a.rec.wrapping_add(0x24));
    let ours = matches!((occupied, cat, idx), (Ok(o), Ok(c), Ok(i)) if restage_matches(o, c, i, &a));
    // `+0x26` = the grid's `+0x3ba` stack count, in the same record-relative frame as the
    // three identity fields above. A failed read keeps the old answer (write the record back).
    let count: Result<i16> = read_generic(core, a.rec.wrapping_add(0x26));
    let n = (r2 & 0xff) as u8 as i8;
    if ours && let Ok(count) = count && !restage_clears(count, n) {
        tracing::warn!(
            "bagbox: removeItem(bag {},{}, {n}) on the staged cell, which holds x{count} — the game subtracts, the knob stays for the last unit",
            a.tab + 1,
            a.slot + 1
        );
        return super::hook::resume_past_push(core, hook_pc);
    }
    if !ours {
        wie_util::stats::clear_bagbox_restage();
        tracing::warn!(
            "bagbox: removeItem(bag {},{}) — the restage knob named this cell but it no longer holds cat {:#x} idx {:#x}; disarmed, the game's own removal runs",
            a.tab + 1,
            a.slot + 1,
            a.cat,
            a.idx
        );
        return super::hook::resume_past_push(core, hook_pc);
    }
    for (i, chunk) in a.bytes.chunks(4).enumerate() {
        let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        write_generic(core, a.rec.wrapping_add(i as u32 * 4), w)?;
    }
    wie_util::stats::clear_bagbox_restage();
    wie_util::stats::BAGBOX_RESTAGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    tracing::warn!(
        "bagbox: the game removed the staged cat {:#x} idx {:#x} (n {}) from bag {},{} — the bag-item cell is back in its place",
        a.cat,
        a.idx,
        r2 & 0xff,
        a.tab + 1,
        a.slot + 1
    );
    Ok(JumpTo(lr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spin::{Mutex, MutexGuard, relax::Spin};
    use wie_util::stats;

    fn bagbox_lock() -> MutexGuard<'static, (), Spin> {
        static M: Mutex<()> = Mutex::new(());
        M.lock()
    }

    /// Every knob the intake reads, back to "feature off, nothing waiting".
    fn reset() {
        stats::set_bagbox_accept(0, 0);
        stats::bagbox_clear_counts();
        stats::set_bagbox_bypass(0, 0, 0);
        for b in 0..3 {
            stats::set_bagbox_room(b, 0);
        }
        while stats::bagbox_pop().is_some() {}
    }

    fn drain() -> alloc::vec::Vec<(u16, u16, u8, u8)> {
        core::iter::from_fn(stats::bagbox_pop).collect()
    }

    #[test]
    fn the_vm_tail_is_the_add_sp() {
        assert!(check_vm_tail([0x06, 0xb0], 0x3bdd3).is_ok());
        assert!(check_vm_tail([0xb0, 0x06], 0x3bdd3).is_err());
    }

    #[test]
    fn the_countof_tail_is_add_sp_c_not_the_vm_helper() {
        assert!(check_countof_tail([0x03, 0xb0], 0x555d1).is_ok());
        assert!(
            check_countof_tail([0x06, 0xb0], 0x555d1).is_err(),
            "quest helper tail is a different frame"
        );
        assert!(check_head([0xf0, 0xb5], 0x555e1).is_ok());
    }

    #[test]
    fn spend_from_box_covers_first_and_fails_closed() {
        let _g = bagbox_lock();
        stats::set_bagbox_accept(0, 0);
        stats::bagbox_clear_counts();
        stats::set_bagbox_count(0x0e, 3, 10);
        assert_eq!(spend_from_box(0x0e, 3, 5), None, "a box that is off reads as empty");
        stats::set_bagbox_accept(1 << 0x0e, 0);
        assert_eq!(spend_from_box(0x0e, 3, 5), Some(5));
        assert_eq!(spend_from_box(0x0e, 3, 20), Some(10), "has fewer than n: debit what it has");
        assert_eq!(spend_from_box(0x0e, 3, 0), None);
        assert_eq!(spend_from_box(0x11, 3, 5), None, "quest items stay in the bag");
        stats::set_bagbox_count(0x0e, 3, 0);
        assert_eq!(spend_from_box(0x0e, 3, 5), None);
        stats::bagbox_clear_counts();
        stats::set_bagbox_accept(0, 0);
    }

    #[test]
    fn take_units_chunks_past_a_u8() {
        let _g = bagbox_lock();
        stats::set_bagbox_accept(1 << 0x0e, 0);
        stats::bagbox_clear_counts();
        stats::set_bagbox_count(0x0e, 3, 300);
        assert_eq!(take_units(0x0e, 3, 300), 300);
        assert_eq!(stats::bagbox_count(0x0e, 3), 0);
        assert_eq!(stats::bagbox_pop(), Some((0x0e, 3, 255, stats::BAGBOX_SRC_TAKE)));
        assert_eq!(stats::bagbox_pop(), Some((0x0e, 3, 45, stats::BAGBOX_SRC_TAKE)));
        assert_eq!(stats::bagbox_pop(), None);
        stats::bagbox_clear_counts();
        stats::set_bagbox_accept(0, 0);
    }

    #[test]
    fn the_count_sum_saturates_at_what_ldrsh_can_hold() {
        assert_eq!(vm_count_sum(3, 0), 3);
        assert_eq!(vm_count_sum(3, 20), 23);
        assert_eq!(vm_count_sum(0x7ff0, 0x100), 0x7fff);
    }

    #[test]
    fn a_take_request_is_take_mode_positive_and_a_box_category() {
        assert_eq!(vm_take_request(0, 0x0e, 3, 10), Some((0x0e, 3, 10)));
        assert_eq!(vm_take_request(1, 0x0e, 3, 10), None, "count mode belongs to the exit hook");
        assert_eq!(vm_take_request(0, 0x0e, 3, 0), None);
        assert_eq!(vm_take_request(0, 0x0e, 3, 0x80), None, "the guest reads it signed");
        assert_eq!(vm_take_request(0, 0x11, 3, 10), None, "quest items stay in the bag");
        assert_eq!(vm_take_request(0, 0x05, 3, 10), None, "equipment too");
    }

    #[test]
    fn the_mirror_answers_counts_and_takes_only_while_accepted() {
        let _g = bagbox_lock();
        stats::set_bagbox_accept(0, 0);
        stats::bagbox_clear_counts();
        stats::set_bagbox_count(0x0e, 3, 30);
        assert_eq!(stats::bagbox_count(0x0e, 3), 0, "a box that is off reads as empty");
        assert!(!stats::bagbox_take(0x0e, 3, 10));
        stats::set_bagbox_accept(1 << 0x0e, 0);
        assert_eq!(stats::bagbox_count(0x0e, 3), 30);
        assert!(stats::bagbox_take(0x0e, 3, 10));
        assert_eq!(stats::bagbox_count(0x0e, 3), 20);
        assert!(!stats::bagbox_take(0x0e, 3, 21), "fewer than n: the game's own bag path");
        assert_eq!(stats::bagbox_count(0x0e, 3), 20, "a refused take spends nothing");
        assert_eq!(stats::bagbox_pop(), Some((0x0e, 3, 10, stats::BAGBOX_SRC_TAKE)));
        assert_eq!(stats::bagbox_pop(), None);
        stats::bagbox_clear_counts();
        stats::set_bagbox_accept(0, 0);
    }

    #[test]
    fn the_announce_argument_is_the_games_own_rule() {
        // The drop the player walks over passes the loot's index, `giveItemN` the unit's
        // ordinal — both announce. Everything the game narrates itself passes -1.
        assert!(give_announces(0), "the first loot object on the ground");
        assert!(give_announces(3));
        assert!(!give_announces(u32::MAX), "the host's own grants pass u32::MAX");
        assert!(!give_announces(0xff), "and -1 as the guest masks it");
        // Only the low byte is the flag: `giveItem` masks it at `0x52b16`/`0x52b1c`, so a
        // high word must not turn a silent give into a loud one.
        assert!(!give_announces(0x1234_56ff));
        assert!(give_announces(0xffff_ff00));
        // The wire route reads the record's SIGNED count and only -1 is silent — and
        // `wire_triple` never accepts that, so a delivery always speaks.
        assert!(wire_announces(1));
        assert!(wire_announces(99));
        assert!(!wire_announces(0xff));
        assert_eq!(wire_triple(0x10, 3, 0xff), None, "the one silent count is never taken anyway");
    }

    #[test]
    fn the_announce_bit_rides_beside_the_route_not_over_it() {
        // The host reads the route with `BAGBOX_SRC_MASK` and the notice with the flag, so
        // the two must not collide — a give that announces must still read as a give.
        let loud = stats::BAGBOX_SRC_GIVE | announce_bit(true);
        assert_eq!(loud & stats::BAGBOX_SRC_MASK, stats::BAGBOX_SRC_GIVE);
        assert_ne!(loud & stats::BAGBOX_ANNOUNCE, 0);
        let quiet = stats::BAGBOX_SRC_GIVE | announce_bit(false);
        assert_eq!(quiet, stats::BAGBOX_SRC_GIVE);
        assert_eq!(quiet & stats::BAGBOX_ANNOUNCE, 0);
        for src in [stats::BAGBOX_SRC_GIVE, stats::BAGBOX_SRC_WIRE, stats::BAGBOX_SRC_TAKE] {
            assert_eq!(src & stats::BAGBOX_SRC_MASK, src, "route {src} survives the mask");
            assert_eq!(src & stats::BAGBOX_ANNOUNCE, 0, "route {src} does not set the flag by itself");
        }
    }

    #[test]
    fn head_is_the_shared_push() {
        assert!(check_head([0xf0, 0xb5], 0x52b15).is_ok());
        assert!(check_head([0xb5, 0xf0], 0x52b15).is_err());
    }

    #[test]
    fn a_wire_count_the_guest_reads_negative_is_left_to_the_game() {
        assert_eq!(wire_triple(0x10, 3, 12), Some((0x10, 3, 12)));
        assert_eq!(wire_triple(0x10, 3, 0), None);
        // 128 and up subtract from the player's stack (72차) — not ours to take.
        assert_eq!(wire_triple(0x10, 3, 0x80), None);
        assert_eq!(wire_triple(0x10, 3, 0xff), None);
    }

    #[test]
    fn the_offer_respects_mask_room_and_the_kept_cells() {
        let _g = bagbox_lock();
        reset();
        // Feature off: nothing is taken, whatever the room says.
        stats::set_bagbox_room(0, 5);
        assert!(!stats::bagbox_offer(0x10, 3, 1, stats::BAGBOX_SRC_GIVE));
        // Orbs on, one free stack: the first offer opens it, the same orb MERGES into it
        // (★ 165차 — it used to cost a second stack and be refused), and a different orb,
        // which would need a stack of its own, is refused.
        stats::set_bagbox_accept(1 << 0x10, 0);
        stats::set_bagbox_room(0, 1);
        assert!(stats::bagbox_offer(0x10, 3, 1, stats::BAGBOX_SRC_GIVE));
        assert!(stats::bagbox_offer(0x10, 3, 1, stats::BAGBOX_SRC_GIVE), "the open stack takes it");
        assert!(!stats::bagbox_offer(0x10, 4, 1, stats::BAGBOX_SRC_GIVE), "no stack left for another orb");
        assert_eq!(drain(), [(0x10, 3, 2, stats::BAGBOX_SRC_GIVE)], "and the two units are one entry");
        // Equipment is never a box item.
        stats::set_bagbox_room(0, 9);
        assert!(!stats::bagbox_offer(0x05, 3, 1, stats::BAGBOX_SRC_GIVE));
        // The bag-item cells themselves stay in the bag even when 0x12 is accepted.
        stats::set_bagbox_accept(1 << 0x12, 1 << 0x16);
        stats::set_bagbox_room(2, 9);
        assert!(!stats::bagbox_offer(0x12, 0x16, 1, stats::BAGBOX_SRC_GIVE));
        assert!(stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE));
        assert_eq!(drain(), [(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE)]);
        // The materialise bypass lets exactly `n` units of one item reach the bag.
        stats::set_bagbox_bypass(0x12, 0x00, 1);
        assert!(!stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE), "bypassed to the game");
        assert_eq!(stats::bagbox_bypass_left(), 0);
        assert!(stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE), "taken again once spent");
        assert_eq!(drain(), [(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE)]);
        reset();
    }

    // ── 165차: one guest call, many units — and a give that is counted at once ──

    #[test]
    fn the_intake_cost_fills_the_open_stacks_before_it_opens_one() {
        assert_eq!(stats::bagbox_intake_cost(0, 1), (1, 98), "a new item opens one stack");
        assert_eq!(stats::bagbox_intake_cost(98, 98), (0, 0), "the rest of that stack");
        assert_eq!(stats::bagbox_intake_cost(0, 99), (1, 0), "a 99-unit purchase is ONE stack");
        assert_eq!(stats::bagbox_intake_cost(10, 99), (1, 10), "10 merge, 89 open a stack of 99");
        assert_eq!(stats::bagbox_intake_cost(0, 100), (2, 98));
        assert_eq!(stats::bagbox_intake_cost(5, 3), (0, 2));
        assert_eq!(stats::bagbox_intake_cost(0, 255), (3, 42), "a folded entry's most");
    }

    #[test]
    fn a_99_unit_purchase_in_one_guest_call_goes_into_the_box_whole() {
        let _g = bagbox_lock();
        reset();
        // The shop's key handler: 99 × `giveItem(포션)` with no drain in between. Two free
        // stacks in the 소비가방 — the old one-stack-per-unit rule took exactly two potions.
        stats::set_bagbox_accept(1 << 0x12, 0);
        stats::set_bagbox_room(2, 2);
        for _ in 0..99 {
            assert!(stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert_eq!(stats::BAGBOX_ROOM[2].load(core::sync::atomic::Ordering::Relaxed), 1, "one stack spent");
        assert_eq!(stats::bagbox_count(0x12, 0x00), 99, "all 99 counted before any drain");
        assert_eq!(drain(), [(0x12, 0x00, 99, stats::BAGBOX_SRC_GIVE)], "one ring entry, not 99");
        // The next purchase opens the last stack; the one after that has nowhere to go.
        for _ in 0..99 {
            assert!(stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert!(!stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE), "the box is full: the game's own path");
        assert_eq!(drain(), [(0x12, 0x00, 99, stats::BAGBOX_SRC_GIVE)]);
        reset();
    }

    #[test]
    fn a_unit_given_inside_a_script_is_counted_and_taken_before_the_drain() {
        let _g = bagbox_lock();
        reset();
        // 리파이너의 유적 7, scene 188 `B[12]`: op 0x14 gives 그리폰의피리 (0x12, 0x09), op
        // 0x12 counts it (`fe`-style, mode 1) and takes it back — all in one guest call.
        stats::set_bagbox_accept(1 << 0x12, 0);
        stats::set_bagbox_room(2, 5);
        assert_eq!(stats::bagbox_count(0x12, 0x09), 0, "none before");
        assert!(stats::bagbox_offer(0x12, 0x09, 1, stats::BAGBOX_SRC_GIVE));
        assert_eq!(stats::bagbox_count(0x12, 0x09), 1, "the give is countable at once");
        assert!(stats::bagbox_take(0x12, 0x09, 1), "and takeable");
        assert_eq!(stats::bagbox_count(0x12, 0x09), 0);
        // The host files them in order: the give, then the take — never folded together.
        assert_eq!(drain(), [(0x12, 0x09, 1, stats::BAGBOX_SRC_GIVE), (0x12, 0x09, 1, stats::BAGBOX_SRC_TAKE)]);
        reset();
    }

    #[test]
    fn only_the_newest_entry_of_the_same_item_and_source_folds() {
        let _g = bagbox_lock();
        reset();
        stats::set_bagbox_accept(1 << 0x12, 0);
        stats::set_bagbox_room(2, 40);
        let give = stats::BAGBOX_SRC_GIVE;
        let loud = stats::BAGBOX_SRC_GIVE | stats::BAGBOX_ANNOUNCE;
        assert!(stats::bagbox_offer(0x12, 0, 1, give));
        assert!(stats::bagbox_offer(0x12, 0, 1, give));
        assert!(stats::bagbox_offer(0x12, 4, 1, give));
        assert!(stats::bagbox_offer(0x12, 0, 1, give), "an older entry of the item is not the newest");
        assert!(stats::bagbox_offer(0x12, 0, 1, loud), "an announced unit keeps its own entry");
        assert!(stats::bagbox_offer(0x12, 0, 1, loud));
        assert_eq!(drain(), [(0x12, 0, 2, give), (0x12, 4, 1, give), (0x12, 0, 1, give), (0x12, 0, 2, loud)]);
        // The count byte never wraps: past 255 a new entry starts.
        stats::set_bagbox_count(0x12, 0, 0);
        stats::set_bagbox_merge(0x12, 0, 0);
        for _ in 0..300 {
            assert!(stats::bagbox_offer(0x12, 0, 1, give));
        }
        assert_eq!(drain(), [(0x12, 0, 255, give), (0x12, 0, 45, give)]);
        // An entry the host has already filed is never folded into after the fact.
        assert!(stats::bagbox_offer(0x12, 0, 1, give));
        assert_eq!(drain(), [(0x12, 0, 1, give)]);
        assert!(stats::bagbox_offer(0x12, 0, 1, give));
        assert_eq!(drain(), [(0x12, 0, 1, give)], "a fresh entry, not the filed one");
        reset();
    }

    // ── 166차: the game's bag-room check asks the box first ──

    #[test]
    fn the_box_units_and_the_ring_margin_are_plain_arithmetic() {
        assert_eq!(stats::bagbox_box_units(0, 0), 0);
        assert_eq!(stats::bagbox_box_units(40, 0), 40, "the open stacks only");
        assert_eq!(stats::bagbox_box_units(40, 2), 40 + 2 * 99);
        // 99 single offers of one item fold into one entry; one more is kept free.
        assert!(stats::bagbox_ring_takes(0, 99));
        assert!(stats::bagbox_ring_takes(stats::BAGBOX_RING_LEN - 2, 99));
        assert!(!stats::bagbox_ring_takes(stats::BAGBOX_RING_LEN - 1, 99), "no slot left beside the margin");
        assert!(stats::bagbox_ring_takes(stats::BAGBOX_RING_LEN - 3, 300), "300 units = two entries + the margin");
        assert!(!stats::bagbox_ring_takes(stats::BAGBOX_RING_LEN - 2, 300));
    }

    #[test]
    fn a_full_bag_is_asked_only_for_what_the_box_will_not_take() {
        let _g = bagbox_lock();
        reset();
        // Box off: the game's own check judges the whole purchase.
        stats::set_bagbox_room(2, 80);
        assert_eq!(room_check_rest(0x12, 0x00, 99), None);
        // The 소비가방 on, one free stack: 99 potions are all the box's — the bag is asked for 0.
        stats::set_bagbox_accept(1 << 0x12, 1 << 0x19);
        stats::set_bagbox_room(2, 1);
        assert_eq!(room_check_rest(0x12, 0x00, 99), Some(0));
        // …and exactly what `bagbox_offer` then does, one unit at a time.
        for _ in 0..99 {
            assert!(stats::bagbox_offer(0x12, 0x00, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert_eq!(drain(), [(0x12, 0x00, 99, stats::BAGBOX_SRC_GIVE)]);
        // A box that can take only part of it: the bag is asked for the rest.
        stats::set_bagbox_room(2, 0);
        stats::set_bagbox_merge(0x12, 0x00, 30);
        assert_eq!(room_check_rest(0x12, 0x00, 99), Some(69));
        assert_eq!(room_check_rest(0x12, 0x00, 20), Some(0));
        // Nothing the box can speak for: the game's own check, untouched.
        stats::set_bagbox_merge(0x12, 0x00, 0);
        assert_eq!(room_check_rest(0x12, 0x00, 99), None, "a full box");
        stats::set_bagbox_room(2, 5);
        assert_eq!(room_check_rest(0x12, 0x19, 1), None, "a bag-item cell stays in the bag");
        assert_eq!(room_check_rest(0x05, 0x00, 1), None, "equipment is never boxed");
        assert_eq!(room_check_rest(0x11, 0x0b, 1), None, "nor quest items");
        assert_eq!(room_check_rest(0x12, 0x00, 0), None, "nothing asked");
        assert_eq!(room_check_rest(0x12, 0x00, -3), None, "a count the guest reads negative");
        stats::set_bagbox_bypass(0x12, 0x00, 1);
        assert_eq!(room_check_rest(0x12, 0x00, 1), None, "a bypassed item goes to the bag");
        stats::set_bagbox_bypass(0, 0, 0);
        // A ring with no slot to spare promises nothing.
        for i in 0..(stats::BAGBOX_RING_LEN as u32 - 1) {
            assert!(stats::bagbox_offer(0x12, 1 + i % 2, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert_eq!(room_check_rest(0x12, 0x00, 5), None);
        reset();
    }

    #[test]
    fn the_cash_gate_hook_only_installs_over_the_shift_after_is_bag_full() {
        assert!(check_cash_gate_head([0x00, 0x06], 0x72e1).is_ok(), "lsls r0,r0,#0x18");
        assert!(check_cash_gate_head([0x25, 0x06], 0x72e3).is_err(), "`lsls r5,r4,#0x18` is the exit, not the site");
        assert!(check_cash_gate_head(EXPECTED_HEAD, 0x72e1).is_err(), "not a function entry");
    }

    /// The cash shop's full-bag gate opens only for a row the box takes WHOLE — the grant
    /// that follows is one `addItemToInventory` offer of the row's count, all or nothing.
    #[test]
    fn a_full_bag_lets_a_cash_row_through_only_when_the_box_takes_all_of_it() {
        let _g = bagbox_lock();
        reset();
        // Box off: the game's own refusal stands.
        stats::set_bagbox_room(2, 80);
        assert!(!cash_gate_opens(0x12, 0x07, 1));
        stats::set_bagbox_accept(1 << 0x12 | 1 << 0x0e | 1 << 0x0d, 1 << 0x19);
        assert!(cash_gate_opens(0x12, 0x07, 1), "오브원석 into the 소비가방");
        assert!(cash_gate_opens(0x0d, 0x1b, 1), "a battle scroll too");
        // The 제련석 row is a 99-stack: one free stack in the 재료가방 takes it, merge room
        // short of 99 with no stack does not.
        stats::set_bagbox_room(1, 1);
        assert!(cash_gate_opens(0x0e, 0x03, 99));
        stats::set_bagbox_room(1, 0);
        stats::set_bagbox_merge(0x0e, 0x03, 98);
        assert!(!cash_gate_opens(0x0e, 0x03, 99), "98 of 99 is not the whole row");
        stats::set_bagbox_merge(0x0e, 0x03, 99);
        assert!(cash_gate_opens(0x0e, 0x03, 99));
        // Never ours: equipment, a bag-item cell, bytes the guest reads negative, a full box.
        assert!(!cash_gate_opens(0x05, 0x10, 1), "equipment is never boxed");
        assert!(!cash_gate_opens(0x12, 0x19, 1), "the 소비가방 cell itself");
        assert!(!cash_gate_opens(0x12, 0x07, 0), "no count");
        assert!(!cash_gate_opens(0x12, 0x07, 0x80), "a count the guest reads negative");
        assert!(!cash_gate_opens(0x92, 0x07, 1), "a category the guest reads negative");
        stats::set_bagbox_room(2, 0);
        assert!(!cash_gate_opens(0x12, 0x07, 1), "the 소비가방 is full too");
        reset();
    }

    /// The engine's view of a box agrees with what `bagbox_offer` will then do.
    #[test]
    fn the_box_view_is_the_offers_own_arithmetic() {
        let _g = bagbox_lock();
        reset();
        assert_eq!(stats::bagbox_view(0x10, 16, 16), None, "box off");
        stats::set_bagbox_accept(1 << 0x10 | 1 << 0x12, 1 << 0x19);
        stats::set_bagbox_room(0, 2);
        stats::set_bagbox_merge(0x10, 0x0f, 40);
        let (free, merge) = stats::bagbox_view(0x10, 16, 16).expect("the 오브가방 is on");
        assert_eq!((free, merge.len(), merge[0x0f], merge[0]), (2, 16, Some(40), Some(0)));
        // A bag-item cell reads as turned away; so does a bypassed item.
        let (_, consumables) = stats::bagbox_view(0x12, 0x20, 1).unwrap();
        assert_eq!((consumables[0x19], consumables[0x07]), (None, Some(0)));
        stats::set_bagbox_bypass(0x10, 0x03, 1);
        assert_eq!(stats::bagbox_view(0x10, 16, 16).unwrap().1[3], None);
        stats::set_bagbox_bypass(0, 0, 0);
        // What the view promises, the offer keeps: 40 + 2 x 99 = 238 of orb 0x0f.
        assert_eq!(stats::bagbox_box_units(40, free), 238);
        assert!(stats::bagbox_offer(0x10, 0x0f, 99, stats::BAGBOX_SRC_WIRE));
        assert!(stats::bagbox_offer(0x10, 0x0f, 99, stats::BAGBOX_SRC_WIRE));
        assert!(!stats::bagbox_offer(0x10, 0x0f, 41, stats::BAGBOX_SRC_WIRE), "one past the view's 238");
        assert!(stats::bagbox_offer(0x10, 0x0f, 40, stats::BAGBOX_SRC_WIRE));
        // A ring that cannot take the deliveries plus one: no view at all.
        stats::set_bagbox_room(2, 1000);
        for i in 0..(stats::BAGBOX_RING_LEN as u32 - 16) {
            assert!(stats::bagbox_offer(0x12, i % 2, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert_eq!(stats::bagbox_view(0x10, 16, 16), None);
        reset();
    }

    #[test]
    fn merge_room_is_spent_before_a_free_stack_and_a_full_ring_gives_the_stack_back() {
        let _g = bagbox_lock();
        reset();
        stats::set_bagbox_accept(1 << 0x0e, 0);
        // No free stack, but the box's 제련석 stacks can take five more.
        stats::set_bagbox_room(1, 0);
        stats::set_bagbox_merge(0x0e, 3, 5);
        for _ in 0..5 {
            assert!(stats::bagbox_offer(0x0e, 3, 1, stats::BAGBOX_SRC_GIVE));
        }
        assert!(!stats::bagbox_offer(0x0e, 3, 1, stats::BAGBOX_SRC_GIVE), "the sixth needs a stack");
        assert_eq!(drain(), [(0x0e, 3, 5, stats::BAGBOX_SRC_GIVE)]);
        // A ring full of other entries: an offer that needs a NEW entry is refused and takes
        // no stack with it; one that folds into the newest entry still goes in.
        stats::set_bagbox_room(1, 1000);
        for i in 0..stats::BAGBOX_RING_LEN as u32 {
            assert!(stats::bagbox_offer(0x0e, 10 + i % 2, 1, stats::BAGBOX_SRC_GIVE));
        }
        let room = || stats::BAGBOX_ROOM[1].load(core::sync::atomic::Ordering::Relaxed);
        let before = room();
        assert!(!stats::bagbox_offer(0x0e, 20, 1, stats::BAGBOX_SRC_GIVE), "a 65th entry");
        assert_eq!(room(), before, "the refused offer reserved nothing");
        assert_eq!(stats::bagbox_count(0x0e, 20), 0, "nor counted anything");
        assert!(stats::bagbox_offer(0x0e, 11, 1, stats::BAGBOX_SRC_GIVE), "the newest entry's item folds in");
        assert_eq!(drain().len(), stats::BAGBOX_RING_LEN);
        reset();
    }

    // ── restage (127차) ──

    fn armed(tab: u8, slot: u8, cat: u8, idx: u8) -> stats::BagboxRestage {
        let mut bytes = [0u8; stats::BAGBOX_REC_LEN];
        bytes[0x1f] = 1;
        bytes[0x22..0x24].copy_from_slice(&0x12u16.to_le_bytes());
        bytes[0x24..0x26].copy_from_slice(&0x19u16.to_le_bytes());
        bytes[0x26..0x28].copy_from_slice(&1u16.to_le_bytes());
        stats::BagboxRestage {
            tab,
            slot,
            cat,
            idx,
            rec: 0x4014_0000,
            bytes,
        }
    }

    #[test]
    fn the_restage_head_is_the_shared_push_too() {
        // `removeItem 0x5340c` begins `f0 b5` like the intake pair, so the same check installs it.
        assert!(check_head([0xf0, 0xb5], 0x5340d).is_ok());
    }

    #[test]
    fn the_restage_answers_only_the_armed_cell_as_the_guest_masks_it() {
        let a = armed(0, 2, 0x12, 0x00);
        assert!(restage_target(None, 0, 2).is_none(), "knob off");
        assert!(restage_target(Some(a), 0, 2).is_some());
        assert!(restage_target(Some(a), 0, 3).is_none(), "another slot");
        assert!(restage_target(Some(a), 1, 2).is_none(), "another tab");
        // The guest masks r0 to a byte and r1 to a halfword before it uses them (`0x5340e`,
        // `0x53410`): high bits are noise, not a different cell.
        assert!(restage_target(Some(a), 0xffff_ff00, 0xffff_0002).is_some());
        assert!(restage_target(Some(a), 0x0000_0100, 2).is_some(), "0x100 is tab 0 to the guest's `lsls #24`");
        assert!(restage_target(Some(a), 0, 0x0001_0002).is_some(), "0x10002 is slot 2 to its `lsls #16`");
    }

    #[test]
    fn the_restage_identity_is_occupied_plus_cat_idx() {
        let a = armed(0, 2, 0x12, 0x05);
        assert!(restage_matches(1, 0x12, 0x05, &a));
        // ★ 135차 §3 ⓒ: only a removal that EMPTIES the cell is answered — the game's own
        // `removeItem` clears on `n <= 0 || n >= count` and subtracts otherwise.
        assert!(restage_clears(1, 1), "the staged unit, removed");
        assert!(restage_clears(0, 0), "the use tail's count-0 call");
        assert!(restage_clears(2, 2), "the whole merged stack");
        assert!(!restage_clears(2, 1), "a merged unit stays — the game subtracts, nothing is written back");
        assert!(restage_clears(5, -1), "a negative count clears, as the guest's `ble` does");
        assert!(!restage_matches(0, 0x12, 0x05, &a), "an empty cell is not the staged item");
        assert!(!restage_matches(1, 0x12, 0x19, &a), "the bag-item cell is already back");
        assert!(!restage_matches(1, 0x0e, 0x05, &a));
        assert!(!restage_matches(1, 0x12, 0x06, &a));
    }

    #[test]
    fn the_restage_knob_round_trips_and_clears() {
        let _g = bagbox_lock();
        stats::clear_bagbox_restage();
        assert_eq!(stats::bagbox_restage(), None);
        let a = armed(0, 2, 0x12, 0x07);
        stats::set_bagbox_restage(&a);
        assert_eq!(stats::bagbox_restage(), Some(a), "the cell, the item, the address and all 60 bytes come back");
        assert_eq!(stats::bagbox_restage_pack(0, 2, 0x12, 0x07), 0x0002_1207);
        stats::clear_bagbox_restage();
        assert_eq!(stats::bagbox_restage(), None);
    }

    #[test]
    fn the_countof_log_fires_once_per_distinct_answer_across_a_painters_rows() {
        let _g = bagbox_lock();
        // Two rows a smith window paints in turn, then a third: each answer logs once,
        // and re-asking them in any order does not log again — that alternation is what
        // a single "last logged" word would have logged on every call.
        let a = countof_log_key(0x0e, 3, 0, 27);
        let b = countof_log_key(0x0e, 5, 2, 4);
        let c = countof_log_key(0x12, 5, 0, 1);
        assert_ne!(a, 0, "bit 63 keeps a real answer apart from an empty ring slot");
        assert!(countof_log_once(a));
        assert!(countof_log_once(b));
        for _ in 0..50 {
            assert!(!countof_log_once(a));
            assert!(!countof_log_once(b));
        }
        assert!(countof_log_once(c));
        // The same material with a DIFFERENT count is a new answer (the box was spent).
        assert!(countof_log_once(countof_log_key(0x0e, 3, 0, 26)));
        assert!(!countof_log_once(a), "the old answer is still remembered while the ring has room");
        // The key carries every field: cat, idx, bag and box each change it.
        assert_ne!(countof_log_key(0x0e, 3, 0, 27), countof_log_key(0x0f, 3, 0, 27));
        assert_ne!(countof_log_key(0x0e, 3, 0, 27), countof_log_key(0x0e, 4, 0, 27));
        assert_ne!(countof_log_key(0x0e, 3, 0, 27), countof_log_key(0x0e, 3, 1, 27));
        assert_ne!(countof_log_key(0x0e, 3, 0, 27), countof_log_key(0x0e, 3, 0, 28));
    }
}
