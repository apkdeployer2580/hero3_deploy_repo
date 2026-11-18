//! 아바타 옷장 (167차, 2026-10-06) — the costume a character wears replaces the sheet of six
//! paper-doll layers, on both paths that build a layer.
//!
//! ── HOW THE GAME DRESSES THE PLAYER ──────────────────────────────────────────
//! The figure (field, STATUS, the EQUIPMENT page's centre box) is a paper doll: one sprite sheet
//! per layer, `res/hero/h<digit><cat:02><sprite:02>_bm`, plus an optional recolour
//! `res/dat/phNNN_pa`. Exactly TWO functions load a layer, and each funnels its six equipment
//! layers through one instruction:
//!
//! * **The layer setter** `0x517bc(slot, cat, idx, remove)` — called by 장착 (`0x478e8`,
//!   `remove = 0`) and 해제 (`0x484b8`, `remove = 1`) and nothing else (literal scan; no `bl`).
//!   Two jump tables map the slot to a KIND (`[sp+0x20]`: 투구 6 · 갑옷 1 · 장갑 3→2 · 신발 7→3 ·
//!   무기 5 · 방패 7 · 보조무기 9); the item row's `+0x0c` (sprite) / `+0x0e` (palette) or the
//!   job's bare body (해제 of 갑옷/장갑/신발; 해제 of 투구/방패/무기 empties or skips the layer) land
//!   in `[sp+0x1c]` / `[sp+0x18]`, and every path joins at **`0x5191a add r5,sp,#0x38`** before
//!   the name builder `0x19dc8` is called.
//! * **The world-init loader** `0x4957c(class)` — reached from world init after
//!   `deserialiseSave` (`0x3ac60` → `0x49e64` → `0x4a01c`), loads layers 0..=7 from the EQUIPPED
//!   array. Layers 1/2/3/5/6/7 (the same six numbers as the setter's kinds) pick sprite
//!   (`[sp+0x20]`, also the builder's 5th argument at `[sp]`) and palette (`[sp+0x1c]`) and join
//!   at **`0x498c0 ldr r4,[pc,#0x2ec]`** (the builder's address, for the `bl` veneer that
//!   follows); the head (0) and the weapon effect (4) call the builder on their own and never get
//!   here. The 보조무기 layer (9) is not loaded at world init at all.
//!
//! ── THE CONTRACT ─────────────────────────────────────────────────────────────
//! At either join the hook asks [`wie_util::avatar::costume_sprite`]: the set the slot shows —
//! the host's try-on ([`wie_util::stats::avatar_preview`]) or else the save's own flag byte
//! (`base+0x9e98 + FLAG_WEAR + slot`) — and whether the jar carries that set's sheet for this
//! slot's group ([`wie_util::stats::avatar_sheet`]; the weapon's group is the category the
//! builder is about to use). On a yes it writes the costume sprite over the game's and palette
//! `0xff` (the sheet's own colours), then performs the displaced instruction and resumes.
//!
//! Nothing else changes: the item rows, the EQUIPPED array, the stats, the save bytes. Because
//! the world-init loader runs AFTER the save's flags are restored, a loaded character wears its
//! costume from its first frame, with no host timing anywhere.
//!
//! ── WHAT THE GAME STILL DECIDES ──────────────────────────────────────────────
//! The figure painter (`0x1ba60..`, the cif-record walk) skips layer 6 (투구) and layer 7 (방패)
//! when that EQUIPPED cell's item index is `-1` — `ldrsh r3,[getBase()+0x3c4]` then
//! `adds r3,#1; bne` at **`0x1bb68`** and again at **`0x1bbf0`** (the two branches of the
//! record walk), so in the shipped game a 투구 is never drawn without a worn 투구.
//!
//! ★ 170차: a HELMET costume is a hairstyle, a hat or a mask, so it is drawn WITHOUT a worn
//! 투구 — the third hook, [`handle_helmet_paint_lgt`] at both `ldrsh` sites, answers "a 투구 is
//! there" (`0` for the `-1`) when the slot shows a costume whose sheet the jar carries, the same
//! decision the two joins make, only for the player's own layer table, and only once a join has
//! put a costume sheet in the layer (`HELMET_LAYER_COSTUME`). The layer itself is
//! loaded by the two joins: the world-init loader already loads layer 6 for an empty cell
//! (sprite 0, which the join dresses), and the host's re-dress calls the setter with a stand-in
//! 투구 row for an empty cell (`avatar_host::redress`). 무기 / 방패 keep the game's rule — a
//! weapon costume is drawn per equipped weapon KIND, and only spear users wear a shield — so
//! those two show only over a worn piece (hero2's rule for its weapon and off-hand too).
//! 갑옷 / 장갑 / 신발 are always drawn (the job's bare body stands in), so those costumes always
//! show.
//!
//! ★ 171차: a hair costume drawn OVER the hero's head can never be smaller than that head (it
//! must hide every hair pixel), and 169차's drawings read as big-headed. A set whose costume.txt
//! says `head = own` therefore draws the WHOLE head in its 투구 sheet — the hero's own face
//! composited in, the hair pulled in around it (`tools_re/costume_atelier/helmfit.py`) — and the
//! fourth hook, [`handle_head_paint_lgt`] at the painter's per-piece `cmp r4,#6; bne`
//! (**`0x1bb56`** / **`0x1bbde`**), skips the hero's own head piece (layer 0) instead. Only when
//! the figure is the player's (`[sp+0x60]`), the 투구 layer holds such a costume
//! (`HELMET_LAYER_COSTUME` + a sheet pointer, the set's [`wie_util::stats::avatar_own_head`] bit)
//! and a 투구 piece FOLLOWS in the same cif record ([`helmet_follows`]): the 32 shipped records
//! that draw the head with no 투구 piece keep the hero's head (as they always did), and a head is
//! never skipped without the costume that replaces it being drawn after it.
//!
//! ── FAIL SOFT ────────────────────────────────────────────────────────────────
//! A frame that is not the expected function's (`[sp+0x24]` / `[sp+0x28]` must be `base+0x8c`,
//! the layer table both functions keep there), an unreadable byte, no published sheets, an
//! out-of-range set: the game's own sheet loads. The hooks never refuse a load and never name a
//! sheet the jar lacks.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use alloc::format;

use wie_util::{ByteRead, Result, WieError, avatar, read_generic, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `add r5, sp, #0x38` at `0x5191a` — the setter's join.
pub const SET_HEAD: [u8; 2] = [0x0e, 0xad];
/// `ldr r4, [pc, #0x2ec]` at `0x498c0` — the loader's join (the builder's address).
pub const LOAD_HEAD: [u8; 2] = [0xbb, 0x4c];

/// LGT: CHAR is a static at `.bss 0x1503a88`, and `getBase()` returns `CHAR + 0x28`.
const GUEST_BASE: u32 = 0x0150_3a88 + 0x28;
/// The script VM's persistent flag array (`base+0x9e98`, 256 bytes, carried by the save).
const FLAGS_OFF: u32 = 0x9e98;
/// The layer-table pointer both functions keep in their frame: `getBase() + 0x8c`.
const LAYER_TABLE_OFF: u32 = 0x8c;

/// Setter frame: the kind, sprite, palette, category argument and the layer-table pointer.
const SET_KIND: u32 = 0x20;
const SET_SPRITE: u32 = 0x1c;
const SET_PALETTE: u32 = 0x18;
const SET_CAT: u32 = 0x30;
const SET_TABLE: u32 = 0x24;

/// Loader frame: the builder's sprite argument, the sprite / palette locals, the layer index and
/// the layer-table pointer. The builder's category argument is in `r3` at the join.
const LOAD_ARG_SPRITE: u32 = 0x00;
const LOAD_SPRITE: u32 = 0x20;
const LOAD_PALETTE: u32 = 0x1c;
const LOAD_LAYER: u32 = 0x30;
const LOAD_TABLE: u32 = 0x28;

/// The two `ldrsh r3, [r0, rM]` the figure painter reads the 투구 cell's item index with —
/// `(pc, head, rM)` — before `adds r3, #1; bne` skips the layer for `-1`.
pub const HELMET_PAINT_SITES: [(u32, [u8; 2], usize); 2] = [(0x1bb68, [0xc3, 0x5f], 7), (0x1bbf0, [0x83, 0x5e], 2)];
/// Where the painter keeps the layer-sheet pointer table of the figure it draws (`[sp+0x60]`,
/// loaded at `0x1bab4`, indexed at `0x1baba`); the player's is `getBase() + 0x8c + 0x9b50`
/// (the setter's own table).
const PAINT_TABLE: u32 = 0x60;
const PLAYER_SHEETS_OFF: u32 = LAYER_TABLE_OFF + 0x9b50;

/// ★ 171차 — the painter's per-piece layer test in its two record-walk branches: `cmp r4,#6`
/// (`06 2c`) then `bne +0x10` (`08 d1`). The hook sits on the `cmp`, so it re-does both.
pub const HEAD_PAINT_SITES: [u32; 2] = [0x1bb56, 0x1bbde];
pub const HEAD_PAINT_HEAD: [u8; 4] = [0x06, 0x2c, 0x08, 0xd1];
/// Where the `bne` goes (`r4 != 6`), from the `cmp`: the `bne` (+2), then its own + 4 + 2 * 8.
const HEAD_PAINT_BNE: u32 = 2 + 4 + 2 * 8;
/// The record walk's "next piece" (`ldr r7,[sp,#0x54]; subs r7,#1` …) — where the game itself
/// goes for a piece it does not draw (a 투구 / 방패 over an empty cell, a null sheet).
pub const PIECE_NEXT: u32 = 0x1bd9c;
/// The walk's frame: `[sp+0x54]` = pieces left INCLUDING the current one, `[sp+0x64]` = the
/// cif object whose `+0x24` is the record data the cursor `r5` indexes.
const PAINT_LEFT: u32 = 0x54;
const PAINT_CIF: u32 = 0x64;
const CIF_DATA: u32 = 0x24;
/// The head and the 투구 layers as a cif piece names them (`b0 & 0x1f`).
const LAYER_HEAD: u8 = 0;
const LAYER_HELMET: u8 = 6;
/// A record never holds more pieces than this (shipped maximum 19) — the scan's bound.
const PIECES_MAX: u32 = 40;

/// "Use the sheet's own colours" — the value the game itself stores for a bare body.
const NATIVE_PALETTE: u32 = 0xff;

/// How many dressings are logged per process (warn — the Android build filters the wie crates to
/// warn, and "did the costume reach the loader" is the first question a missing costume raises).
const LOG_FIRST: u32 = 24;

/// ★ 172차: the two PAINTER hooks fire once per figure piece per frame, so they used up the
/// dressing budget above within a second of a 투구 costume being on (171차 device log) and the
/// setter / loader lines — the ones that answer "did the costume reach the layer" — were gone
/// for the rest of the session. They count against their own budget now.
static PAINT_LOGGED: AtomicU32 = AtomicU32::new(0);

fn paint_log_slot() -> bool {
    PAINT_LOGGED.fetch_add(1, Ordering::Relaxed) < LOG_FIRST
}

pub fn check_set_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == SET_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "avatar_layer_set hook at {pc:#x}: expected {SET_HEAD:02x?} (add r5,sp,#0x38), found {head:02x?}"
    )))
}

pub fn check_load_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == LOAD_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "avatar_layer_load hook at {pc:#x}: expected {LOAD_HEAD:02x?} (ldr r4,[pc,#0x2ec]), found {head:02x?}"
    )))
}

pub fn check_paint_head(head: [u8; 2], pc: u32) -> Result<()> {
    let at = pc & !1;
    match HELMET_PAINT_SITES.iter().find(|s| s.0 == at) {
        Some(&(_, want, _)) if head == want => Ok(()),
        Some(&(_, want, _)) => Err(WieError::FatalError(format!(
            "avatar_helmet_paint hook at {pc:#x}: expected {want:02x?} (ldrsh r3,[r0,rM]), found {head:02x?}"
        ))),
        None => Err(WieError::FatalError(format!(
            "avatar_helmet_paint hook at {pc:#x}: not one of the painter's two 투구 reads"
        ))),
    }
}

pub fn check_head_paint_head(head: [u8; 4], pc: u32, exit_pc: u32) -> Result<()> {
    let at = pc & !1;
    if !HEAD_PAINT_SITES.contains(&at) {
        return Err(WieError::FatalError(format!("avatar_head_paint hook at {pc:#x}: not one of the painter's two layer tests")));
    }
    if head != HEAD_PAINT_HEAD {
        return Err(WieError::FatalError(format!(
            "avatar_head_paint hook at {pc:#x}: expected {HEAD_PAINT_HEAD:02x?} (cmp r4,#6; bne), found {head:02x?}"
        )));
    }
    if exit_pc & !1 != at + 4 {
        return Err(WieError::FatalError(format!("avatar_head_paint hook at {pc:#x}: exit_pc {exit_pc:#x} is not the bne's fall-through")));
    }
    Ok(())
}

/// Does a 투구 piece follow in the record? `bytes` start at the piece AFTER the current one and
/// `left` pieces remain. A piece is 4 bytes, 9 with `b0 & 0x80`; `0x7f` is a 4-byte filler (the
/// walk's own lengths, `0x1ba76` / `0x1bae0`). Bytes running out = no (the head stays).
pub fn helmet_follows(bytes: &[u8], left: u32) -> bool {
    let mut p = 0usize;
    for _ in 0..left.min(PIECES_MAX) {
        let Some(&b0) = bytes.get(p) else { return false };
        if b0 != 0x7f && b0 & 0x1f == LAYER_HELMET {
            return true;
        }
        p += if b0 != 0x7f && b0 & 0x80 != 0 { 9 } else { 4 };
    }
    false
}

/// The set a slot shows right now: the host's try-on first, else the save's flag byte.
fn worn(core: &mut ArmCore, slot: usize) -> Option<u8> {
    if let Some(set) = wie_util::stats::avatar_preview(slot) {
        return Some(set);
    }
    read_generic::<u8, _>(core, GUEST_BASE + FLAGS_OFF + u32::from(avatar::FLAG_WEAR) + slot as u32).ok()
}

/// The costume sprite for `(layer, cat)`, when the frame checks out and a sheet is published.
fn decide(core: &mut ArmCore, layer: u32, cat: u32, table: u32) -> Option<(usize, u8, u8)> {
    if table != GUEST_BASE + LAYER_TABLE_OFF {
        return None;
    }
    let slot = avatar::slot_of_layer(layer)?;
    let set = worn(core, slot)?;
    let sprite = avatar::costume_sprite(slot, cat, set, wie_util::stats::avatar_sheet)?;
    Some((slot, set, sprite))
}

/// Whether the player's 투구 layer holds a COSTUME sheet right now. The two joins see every load
/// into the layer (the setter's non-remove path and the world-init loader), so they record it;
/// the painter draws an EMPTY cell's layer only then. Without it the game's stand-in sheet
/// (sprite 0, `h60000`, which the loader puts in layer 6 for an empty cell) would show for the
/// frame between a try-on and the host's re-dress. A 해제 frees the layer (pointer 0) without
/// passing a join — the painter's own null check (`0x1bb84`, `[sp+0x34]`) skips it then.
static HELMET_LAYER_COSTUME: AtomicBool = AtomicBool::new(false);

/// Record what a join just put in the player's 투구 layer (other layers and figures: no-op).
fn note_helmet_layer(layer: u32, table: u32, dressed: bool) -> bool {
    let player_helmet = table == GUEST_BASE + LAYER_TABLE_OFF && avatar::slot_of_layer(layer) == Some(avatar::SLOT_HELMET);
    if player_helmet {
        HELMET_LAYER_COSTUME.store(dressed, Ordering::Relaxed);
    }
    player_helmet
}

/// Does the 투구 slot show a costume the jar carries? (The joins' decision, without a frame.)
fn helmet_costume(core: &mut ArmCore) -> bool {
    worn(core, avatar::SLOT_HELMET)
        .and_then(|set| avatar::costume_sprite(avatar::SLOT_HELMET, 0, set, wie_util::stats::avatar_sheet))
        .is_some()
}

/// What the painter should see for the 투구 cell's item index: the game's own value, except
/// that an empty cell (`-1`) reads as a worn one (`0`) while the slot shows a costume.
pub fn painted_index(index: i16, costume: bool) -> i16 {
    if index == -1 && costume { 0 } else { index }
}

fn log_dressing(path: &str, layer: u32, slot: usize, set: u8, was: u32, sprite: u8) {
    let n = wie_util::stats::AVATAR_DRESSED.fetch_add(1, Ordering::Relaxed);
    if n < LOG_FIRST {
        tracing::warn!("avatar: {path} layer {layer} (slot {slot}) wears set {set} — sheet {was} -> {sprite}");
    }
}

/// The setter's join (`0x5191a`): maybe dress the layer, then `add r5, sp, #0x38`.
pub async fn handle_set_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let sp = core.inner.lock().engine.reg_read(ArmRegister::SP);
    let frame = (
        read_generic::<u32, _>(core, sp + SET_KIND),
        read_generic::<u32, _>(core, sp + SET_CAT),
        read_generic::<u32, _>(core, sp + SET_TABLE),
        read_generic::<u32, _>(core, sp + SET_SPRITE),
    );
    if let (Ok(kind), Ok(cat), Ok(table), Ok(was)) = frame {
        let dressed = if let Some((slot, set, sprite)) = decide(core, kind, cat & 0xffff, table)
            && write_generic(core, sp + SET_SPRITE, u32::from(sprite)).is_ok()
            && write_generic(core, sp + SET_PALETTE, NATIVE_PALETTE).is_ok()
        {
            log_dressing("setter", kind, slot, set, was, sprite);
            true
        } else {
            false
        };
        note_helmet_layer(kind, table, dressed);
    }
    // The displaced `add r5, sp, #0x38`.
    core.inner.lock().engine.reg_write(ArmRegister::R5, sp + 0x38);
    Ok(JumpTo(exit_pc))
}

/// The loader's join (`0x498c0`): maybe dress the layer, then `ldr r4, [pc, #0x2ec]`.
pub async fn handle_load_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (sp, cat) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(ArmRegister::R3))
    };
    let frame = (
        read_generic::<u32, _>(core, sp + LOAD_LAYER),
        read_generic::<u32, _>(core, sp + LOAD_TABLE),
        read_generic::<u32, _>(core, sp + LOAD_SPRITE),
    );
    if let (Ok(layer), Ok(table), Ok(was)) = frame {
        let dressed = if let Some((slot, set, sprite)) = decide(core, layer, cat, table)
            && write_generic(core, sp + LOAD_ARG_SPRITE, u32::from(sprite)).is_ok()
            && write_generic(core, sp + LOAD_SPRITE, u32::from(sprite)).is_ok()
            && write_generic(core, sp + LOAD_PALETTE, NATIVE_PALETTE).is_ok()
        {
            log_dressing("loader", layer, slot, set, was, sprite);
            true
        } else {
            false
        };
        note_helmet_layer(layer, table, dressed);
    }
    // The displaced `ldr r4, [pc, #imm8*4]`: PC reads as the instruction + 4, word-aligned.
    let insn = hook_pc & !1;
    let literal = ((insn + 4) & !3) + u32::from(LOAD_HEAD[0]) * 4;
    let builder: u32 = read_generic(core, literal)?;
    core.inner.lock().engine.reg_write(ArmRegister::R4, builder);
    Ok(JumpTo(exit_pc))
}

/// The painter's 투구 read (`0x1bb68` / `0x1bbf0`): `ldrsh r3, [r0, rM]`, except that an
/// empty cell reads as worn while the player's 투구 slot shows a costume (170차 — module doc).
pub async fn handle_helmet_paint_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    const REGS: [ArmRegister; 8] = [
        ArmRegister::R0,
        ArmRegister::R1,
        ArmRegister::R2,
        ArmRegister::R3,
        ArmRegister::R4,
        ArmRegister::R5,
        ArmRegister::R6,
        ArmRegister::R7,
    ];
    let at = hook_pc & !1;
    let rm = HELMET_PAINT_SITES.iter().find(|s| s.0 == at).map_or(2, |s| s.2);
    let (sp, base, off) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(REGS[0]), inner.engine.reg_read(REGS[rm]))
    };
    // The displaced load itself — it faults exactly where the game's would.
    let index: i16 = read_generic(core, base.wrapping_add(off))?;
    let costume = index == -1
        && read_generic::<u32, _>(core, sp + PAINT_TABLE).ok() == Some(GUEST_BASE + PLAYER_SHEETS_OFF)
        && HELMET_LAYER_COSTUME.load(Ordering::Relaxed)
        && helmet_costume(core);
    if costume && paint_log_slot() {
        tracing::warn!("avatar: painter — no 투구 is worn, but the 투구 slot shows a costume: the layer is drawn");
    }
    core.inner.lock().engine.reg_write(ArmRegister::R3, painted_index(index, costume) as i32 as u32);
    Ok(JumpTo(exit_pc))
}

/// Should the player's head piece be skipped — its costume draws the whole head? (171차)
fn head_replaced(core: &mut ArmCore, sp: u32, cursor: u32) -> bool {
    if !HELMET_LAYER_COSTUME.load(Ordering::Relaxed) {
        return false;
    }
    let Ok(table) = read_generic::<u32, _>(core, sp + PAINT_TABLE) else { return false };
    if table != GUEST_BASE + PLAYER_SHEETS_OFF {
        return false;
    }
    // The 투구 layer's sheet must be there, or the costume that replaces the head is not drawn.
    if !matches!(read_generic::<u32, _>(core, table + 4 * u32::from(LAYER_HELMET)), Ok(p) if p != 0) {
        return false;
    }
    let Some(set) = worn(core, avatar::SLOT_HELMET) else { return false };
    if !wie_util::stats::avatar_own_head(set) || avatar::costume_sprite(avatar::SLOT_HELMET, 0, set, wie_util::stats::avatar_sheet).is_none() {
        return false;
    }
    let (Ok(left), Ok(cif)) = (read_generic::<u32, _>(core, sp + PAINT_LEFT), read_generic::<u32, _>(core, sp + PAINT_CIF)) else {
        return false;
    };
    let Ok(data) = read_generic::<u32, _>(core, cif + CIF_DATA) else { return false };
    let left = left.saturating_sub(1).min(PIECES_MAX);
    let mut bytes = [0u8; 9 * PIECES_MAX as usize];
    let n = (9 * left) as usize;
    if core.read_bytes(data.wrapping_add(cursor), &mut bytes[..n]).is_err() {
        return false;
    }
    helmet_follows(&bytes[..n], left)
}

/// The painter's per-piece `cmp r4,#6; bne` (`0x1bb56` / `0x1bbde`), re-done — except that the
/// player's head piece goes straight to the next piece while its 투구 costume draws the whole
/// head (171차 — module doc).
pub async fn handle_head_paint_lgt(core: &mut ArmCore, hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let at = hook_pc & !1;
    let (sp, layer, cursor) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::SP), inner.engine.reg_read(ArmRegister::R4), inner.engine.reg_read(ArmRegister::R5))
    };
    if layer == u32::from(LAYER_HELMET) {
        return Ok(JumpTo(exit_pc));
    }
    if layer == u32::from(LAYER_HEAD) && head_replaced(core, sp, cursor) {
        if paint_log_slot() {
            tracing::warn!("avatar: painter — the 투구 costume draws the whole head: the hero's own head piece is skipped");
        }
        return Ok(JumpTo(PIECE_NEXT | 1));
    }
    Ok(JumpTo((at + HEAD_PAINT_BNE) | 1))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn heads_are_pinned() {
        assert!(check_set_head([0x0e, 0xad], 0x5191b).is_ok());
        assert!(check_set_head([0xad, 0x0e], 0x5191b).is_err());
        assert!(check_load_head([0xbb, 0x4c], 0x498c1).is_ok());
        assert!(check_load_head([0xbb, 0x4b], 0x498c1).is_err(), "ldr r3 is a different register");
    }

    #[test]
    fn the_painter_sites_are_pinned() {
        assert!(check_paint_head([0xc3, 0x5f], 0x1bb69).is_ok());
        assert!(check_paint_head([0x83, 0x5e], 0x1bbf1).is_ok());
        assert!(check_paint_head([0x83, 0x5e], 0x1bb69).is_err(), "each site has its own register");
        assert!(check_paint_head([0xc3, 0x5f], 0x1bb6b).is_err(), "not a painter site");
        // ldrsh r3,[r0,rM] = 0101 111 mmm nnn ddd
        for (_, head, rm) in HELMET_PAINT_SITES {
            let h = u16::from_le_bytes(head);
            assert_eq!(h >> 9, 0b0101111);
            assert_eq!(usize::from((h >> 6) & 7), rm);
            assert_eq!((h >> 3) & 7, 0, "base r0");
            assert_eq!(h & 7, 3, "into r3");
        }
    }

    #[test]
    fn only_the_players_helmet_layer_is_recorded() {
        let player = GUEST_BASE + LAYER_TABLE_OFF;
        assert!(!note_helmet_layer(1, player, true), "갑옷 is not the 투구 layer");
        assert!(!note_helmet_layer(6, player + 0x1000, true), "another figure's table");
        assert!(note_helmet_layer(6, player, true));
        assert!(HELMET_LAYER_COSTUME.load(Ordering::Relaxed));
        assert!(note_helmet_layer(6, player, false), "a stand-in load clears it");
        assert!(!HELMET_LAYER_COSTUME.load(Ordering::Relaxed));
    }

    #[test]
    fn the_head_test_sites_are_pinned() {
        for at in HEAD_PAINT_SITES {
            assert!(check_head_paint_head(HEAD_PAINT_HEAD, at | 1, (at + 4) | 1).is_ok());
            assert!(check_head_paint_head(HEAD_PAINT_HEAD, at | 1, (at + 6) | 1).is_err(), "exit is the fall-through");
            assert!(check_head_paint_head([0x06, 0x2c, 0x09, 0xd1], at | 1, (at + 4) | 1).is_err(), "bne pinned too");
        }
        assert!(check_head_paint_head(HEAD_PAINT_HEAD, 0x1bb59, 0x1bb5d).is_err(), "not a painter site");
        // `bne` = 1101 0001 imm8: target = its own address + 4 + imm8 * 2
        assert_eq!(0x1bb56 + HEAD_PAINT_BNE, 0x1bb6c);
        assert_eq!(0x1bbde + HEAD_PAINT_BNE, 0x1bbf4);
    }

    #[test]
    fn a_head_is_replaced_only_when_a_helmet_piece_follows() {
        // [helmet] after the head: yes
        assert!(helmet_follows(&[0x06, 0, 0, 0xfc], 1));
        // an arm (4 bytes) then the helmet: yes
        assert!(helmet_follows(&[0x02, 1, 0, 0, 0x06, 0, 0, 0], 2));
        // a flagged (9-byte) piece then the helmet, flipped (0x20): yes
        assert!(helmet_follows(&[0x83, 0, 0, 0, 1, 2, 3, 4, 5, 0x26, 0, 0, 0], 2));
        // the helmet byte sitting INSIDE a 9-byte piece's tail is not a piece
        assert!(!helmet_follows(&[0x81, 0, 0, 0, 0x06, 0x06, 0x06, 0x06, 0x06], 1));
        // the 0x7f filler is 4 bytes and never a layer
        assert!(helmet_follows(&[0x7f, 0x06, 0x06, 0x06, 0x06, 0, 0, 0], 2));
        // nothing left, or only other layers, or the bytes run out: no
        assert!(!helmet_follows(&[0x06, 0, 0, 0], 0));
        assert!(!helmet_follows(&[0x05, 0, 0, 0, 0x03, 0, 0, 0], 2));
        assert!(!helmet_follows(&[0x05, 0, 0], 2));
        // layer is the low five bits: 0x16 is layer 22, not 6
        assert!(!helmet_follows(&[0x16, 0, 0, 0], 1));
    }

    #[test]
    fn only_an_empty_cell_under_a_costume_reads_as_worn() {
        assert_eq!(painted_index(-1, true), 0);
        assert_eq!(painted_index(-1, false), -1, "no costume: the game skips the layer as it always did");
        assert_eq!(painted_index(7, true), 7, "a worn 투구 keeps its own index");
        assert_eq!(painted_index(0, false), 0);
    }

    #[test]
    fn the_loader_literal_is_the_one_the_game_reads() {
        // `0x498c0 ldr r4,[pc,#0x2ec]`: (0x498c4 & !3) + 0x2ec.
        let insn = 0x498c1u32 & !1;
        assert_eq!(((insn + 4) & !3) + u32::from(LOAD_HEAD[0]) * 4, 0x49bb0);
    }

    /// The shipped binary (when this checkout has it): both heads at their sites, the setter's
    /// six kinds and the loader's six layers really reach the joins, and the flag ids are free of
    /// the frames' own words.
    #[test]
    fn the_shipped_binary_agrees() {
        let path = std::format!("{}/../../../tools_re/_binary.mod", env!("CARGO_MANIFEST_DIR"));
        let Ok(elf) = std::fs::read(&path) else {
            std::eprintln!("skipped: {path} is not in this checkout");
            return;
        };
        let at = |va: usize, n: usize| elf[va - 0x1000 + 0x34..va - 0x1000 + 0x34 + n].to_vec();
        assert_eq!(at(0x5191a, 2), SET_HEAD);
        assert_eq!(at(0x498c0, 2), LOAD_HEAD);
        // The painter's two 투구 reads: `movs r?,#0xf1; lsls #2` (getBase()+0x3c4 = EQUIPPED[1]'s
        // item index), the `ldrsh`, then `adds r3,#1; bne` — and the sheet table it draws from is
        // `[sp+0x60]` (`ldr r6,[sp,#0x60]` at 0x1bab4).
        for (pc, head, _) in HELMET_PAINT_SITES {
            assert_eq!(at(pc as usize, 2), head);
        }
        assert_eq!(at(0x1bb60, 4), [0xf1, 0x24, 0xa4, 0x00], "movs r4,#0xf1; lsls r4,r4,#2");
        assert_eq!(at(0x1bbe8, 4), [0xf1, 0x21, 0x89, 0x00], "movs r1,#0xf1; lsls r1,r1,#2");
        assert_eq!(at(0x1bb7e, 2), [0x01, 0x33], "adds r3,#1");
        assert_eq!(at(0x1bc06, 2), [0x01, 0x33], "adds r3,#1");
        assert_eq!(at(0x1bab4, 2), [0x18, 0x9e], "ldr r6,[sp,#0x60]");
        // 171차: the per-piece layer test, the walk's next-piece, and the frame the scan reads
        for site in HEAD_PAINT_SITES {
            assert_eq!(at(site as usize, 4), HEAD_PAINT_HEAD);
        }
        assert_eq!(at(0x1ba68, 2), [0x15, 0x93], "str r3,[sp,#0x54] — the record's piece count");
        assert_eq!(at(0x1ba6a, 4), [0x19, 0x9f, 0x7a, 0x6a], "ldr r7,[sp,#0x64]; ldr r2,[r7,#0x24]");
        assert_eq!(at(0x1ba76, 2), [0x7f, 0x2c], "cmp r4,#0x7f — the filler");
        assert_eq!(at(PIECE_NEXT as usize, 6), [0x15, 0x9f, 0x01, 0x3f, 0x15, 0x97], "ldr r7,[sp,#0x54]; subs r7,#1; str");
        let word = |va: usize| u32::from_le_bytes(at(va, 4).try_into().unwrap());
        // The loader's literal is the name builder, and the setter's two `bl` targets use the same one.
        assert_eq!(word(0x49bb0), 0x19dc9);
        assert_eq!(word(0x51a9c), 0x19dc9);
        // Both functions keep the layer table at `getBase() + 0x8c`: `adds r0, #0x8c` then the store.
        assert_eq!(at(0x517de, 4), [0x8c, 0x30, 0x09, 0x90], "setter: adds r0,#0x8c / str r0,[sp,#0x24]");
        assert_eq!(at(0x49590, 4), [0x8c, 0x30, 0x23, 0x16], "loader: adds r0,#0x8c (stored at [sp,#0x28] next)");
        assert_eq!(at(0x49594, 2), [0x0a, 0x90], "loader: str r0,[sp,#0x28]");
        // The setter's slot -> kind tables (remove == 0 path at 0x5e28c): every dressed slot's kind.
        let kind_of = |target: u32| -> Option<u32> {
            // movs rX,#k at the jump target (the store follows within two instructions).
            let h = u16::from_le_bytes(at(target as usize, 2).try_into().unwrap());
            (h & 0xf800 == 0x2000).then_some(u32::from(h & 0xff))
        };
        let equip_table = 0x5e28c;
        let kinds: std::vec::Vec<Option<u32>> = (0..9).map(|s| kind_of(word(equip_table + 4 * s) & !1)).collect();
        assert_eq!(kinds[0], Some(5), "slot 0 (무기) -> kind 5");
        assert_eq!(kinds[1], Some(6), "slot 1 (투구) -> kind 6");
        assert_eq!(kinds[3], Some(2), "slot 3 (장갑) -> kind 2");
        assert_eq!(kinds[4], Some(1), "slot 4 (갑옷) -> kind 1");
        assert_eq!(kinds[5], Some(7), "slot 5 (방패) -> kind 7");
        assert_eq!(kinds[6], Some(9), "slot 6 (보조무기) -> kind 9");
        assert_eq!(kinds[7], Some(3), "slot 7 (신발) -> kind 3");
        for s in 0..avatar::SLOTS {
            let eq = usize::from(avatar::EQUIPPED_SLOT[s]);
            assert_eq!(kinds[eq].and_then(avatar::slot_of_layer), Some(s), "avatar slot {s} <- EQUIPPED[{eq}]");
        }
    }
}
