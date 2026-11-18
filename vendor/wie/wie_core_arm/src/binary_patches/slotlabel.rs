//! The slot list's label box, widened for a two-digit slot number (130차, 2026-09-23).
//!
//! ── WHAT THE GAME DRAWS ─────────────────────────────────────────────────────
//! The shell's slot-list painter draws each row through `0x53c0(x, y, "SLOT n", selected)`:
//! the row plate, the text centred at `x` (`0x3d70`, align mode 1), and then a **12x12
//! outline box** at the RIGHT END of the centred text —
//!
//! ```text
//! 0x548c  adds r1, r7, r1        ; r1 = x + textWidth/2   (r7 = x, r1 = width/2)
//! 0x5490  ldr  r2, [sp,#0x18]    ; y + 2
//! 0x5492  str  r3, [sp,#0xc]     ; blue
//! 0x5494  subs r1, #6            ; box left  = text end - 6        <- HOOK (displaced)
//! 0x5496  movs r3, #0xc          ; box width = 12                  <- skipped (exit_pc)
//! 0x5498  ldr  r4, [pc,#0x44]    ; 0x3e15 = drawRect(gc, x, y, w, [sp]=h, r, g, b)
//! ```
//!
//! That box is the «1»-style key cap around the slot digit. It does not look at the
//! string: it is always 12 px wide and always ends 6 px past the text, which fits exactly
//! ONE 7-px glyph. With the save-slot window (`wie_android::slotwindow`) the labels reach
//! `SLOT 10` … `SLOT 12`, and the cap then sits on the `0` alone with the `1` outside it —
//! the user's "박스가 작아서 두자리수중 1이 박스를 벗어나고있어".
//!
//! ── WHAT THIS HOOK DOES ─────────────────────────────────────────────────────
//! It re-does the displaced `subs r1,#6` and the skipped `movs r3,#0xc` with the width
//! following the label: the string is still in `r5` (set at `0x53cc`, untouched since),
//! so the handler reads the label cell, counts its trailing ASCII digits and makes the
//! box [`box_width`] wide, keeping its RIGHT edge where the game put it ([`box_left`]).
//! One digit → the game's own 12; two → 20 (one more 7-px glyph plus its 1-px advance).
//! Nothing else in the painter changes: text, colours and the plate are the game's.
//!
//! Ships on BOTH flavours (the slot window is flavour-independent). FAIL SOFT: if the label
//! cannot be read the game's own numbers are used, so the worst case is the original look.

use alloc::format;

use wie_util::{ByteRead, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `subs r1,#6` at `0x5494` — the displaced instruction.
pub const HEAD: [u8; 2] = [0x06, 0x39];
/// The game's own box width (`movs r3,#0xc`).
pub const BOX_W_ONE: u32 = 12;
/// The game's own right overhang (`subs r1,#6`): the box ends 6 px past the text's end.
pub const BOX_RIGHT_PAD: u32 = 6;
/// One more digit costs one more glyph advance in this font (7 px ink + 1 px gap).
pub const DIGIT_ADVANCE: u32 = 8;
/// The label cell is 8 bytes (`"SLOT 12\0"` fills it), so a slot number is two digits at most.
pub const MAX_DIGITS: u32 = 2;
/// Bytes of the label cell the handler reads.
pub const LABEL_CELL: usize = 8;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "slot_label_box hook at {pc:#x}: expected {HEAD:02x?} (subs r1,#6), found {head:02x?}"
    )))
}

/// How many ASCII digits end the NUL-terminated label (a cell with no NUL is read to its
/// end). `SLOT 1` → 1, `SLOT 12` → 2, `SLOT` → 0.
pub fn trailing_digits(label: &[u8]) -> u32 {
    let end = label.iter().position(|b| *b == 0).unwrap_or(label.len());
    label[..end].iter().rev().take_while(|b| b.is_ascii_digit()).count() as u32
}

/// The box width for a label with `digits` trailing digits: the game's own 12 for one
/// (or none — the game's box is drawn regardless), 20 for two. Capped at [`MAX_DIGITS`].
pub fn box_width(digits: u32) -> u32 {
    BOX_W_ONE + DIGIT_ADVANCE * (digits.clamp(1, MAX_DIGITS) - 1)
}

/// The box's left edge for a text ending at `text_end` (the painter's `r1 = x + w/2`),
/// keeping the game's right edge (`text_end + 6`). For width 12 this is exactly the
/// displaced `subs r1,#6`.
pub fn box_left(text_end: u32, width: u32) -> u32 {
    text_end.wrapping_add(BOX_RIGHT_PAD).wrapping_sub(width)
}

/// `0x5494`: re-do `subs r1,#6` + `movs r3,#0xc` with the width read off the label in `r5`.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (text_end, label_ptr) = {
        let inner = core.inner.lock();
        (inner.engine.reg_read(ArmRegister::R1), inner.engine.reg_read(ArmRegister::R5))
    };
    let mut cell = [0u8; LABEL_CELL];
    // FAIL SOFT: an unreadable label gets the game's own box.
    let digits = match core.read_bytes(label_ptr, &mut cell) {
        Ok(_) => trailing_digits(&cell),
        Err(_) => 1,
    };
    let width = box_width(digits);
    let mut inner = core.inner.lock();
    inner.engine.reg_write(ArmRegister::R1, box_left(text_end, width));
    inner.engine.reg_write(ArmRegister::R3, width);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_follows_the_digit_count_and_keeps_its_right_edge() {
        assert_eq!(trailing_digits(b"SLOT 1\0\0"), 1);
        assert_eq!(trailing_digits(b"SLOT 9\0\0"), 1);
        assert_eq!(trailing_digits(b"SLOT 10\0"), 2);
        assert_eq!(trailing_digits(b"SLOT 12\0"), 2);
        assert_eq!(trailing_digits(b"SLOT\0\0\0\0"), 0);
        assert_eq!(trailing_digits(b"SLOT 123"), 3, "no NUL in the cell: read to its end");
        // One digit (or none) is the game's own box; two is one glyph advance wider; more
        // is capped — the cell cannot hold more than two anyway.
        assert_eq!(box_width(0), 12);
        assert_eq!(box_width(1), 12);
        assert_eq!(box_width(2), 20);
        assert_eq!(box_width(3), 20);
        // Width 12 reproduces the displaced `subs r1,#6` exactly; width 20 grows LEFT.
        assert_eq!(box_left(100, 12), 94);
        assert_eq!(box_left(100, 20), 86);
        assert_eq!(box_left(100, 12) + 12, box_left(100, 20) + 20, "same right edge");
        assert!(check_head([0x06, 0x39], 0x5495).is_ok(), "subs r1,#6");
        assert!(check_head([0x0c, 0x23], 0x5497).is_err(), "movs r3,#0xc is the skipped one, not the hook site");
    }
}
