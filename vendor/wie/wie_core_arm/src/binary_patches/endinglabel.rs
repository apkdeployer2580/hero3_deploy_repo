//! The ending window's line — 「헬모드로 재시작합니다」 after a hardcore clear (140차, 2026-09-29).
//!
//! ── WHAT THE GAME DOES ───────────────────────────────────────────────────────
//! After the staff roll `0x3e840` opens ending window 18. Its ENTER (`0x3f5e2`) calls the
//! common-widget installer `0x46b7c(id)` with id `0x13`, and that function's switch picks
//! the `InGame_txt` row the widget will paint (`0x575bc`) from the id:
//!
//! ```text
//! 0x46b94  cmp  r7, #0x13
//! 0x46b98  b    0x46cec
//! 0x46cec  movs r3, #0xaa      ; row 170 = 「하드코어 모드로;재시작합니다」   <- HOOK (displaced)
//! 0x46cee  b    0x46cc4        ; str r3,[sp,#0x2c] -> the text getter          <- exit_pc
//! ```
//!
//! (window 17, id `0xf`, takes the same road to `0x46dc2 movs r2,#0xa9` = 「부활하시겠습니까?」.)
//! The row is a constant, so the line says 하드코어 whatever difficulty the clear was on —
//! and since 133차 a hardcore clear restarts in 헬, not 하드코어 (the user: *"하드코어모드로
//! 재시작합니다. 라고 팝업이 뜨고 재시작하면 헬모드가 열립니다. 라고 토스트가 떠서 뭔가
//! 안맞아"*).
//!
//! ── WHAT THIS HOOK DOES ─────────────────────────────────────────────────────
//! It re-does the displaced `movs r3,#0xaa` with the row chosen by the difficulty byte the
//! playthrough is on ([`label_for`]): 0 (normal, whose 2회차 IS 하드코어) keeps the game's own
//! row; anything else (하드코어, or 헬 itself — hell stays hell) takes [`HELL_RESTART_LABEL`],
//! the row `tools_re/add_hell_restart_label.py` appends to `InGame_txt`. The byte is still
//! the pre-clear value here: the transition (`0x3af2c`, `ending_transition_lgt`) that
//! rewrites it runs from the window's KEY handler, i.e. AFTER this line is up.
//!
//! ⛔ MAIN ONLY (`hook.rs` skips it under `hero3_vanilla`): vanilla has no 헬 — a hardcore
//! clear there restarts in 하드코어 again, which is what the game's own row says. The row
//! itself ships on both (item tables and text are flavour-free; an unused row is inert).
//! FAIL SOFT: an unreadable byte gets the game's own row.

use alloc::format;

use wie_util::{ByteRead, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `movs r3,#0xaa` at `0x46cec` — the displaced instruction.
pub const HEAD: [u8; 2] = [0xaa, 0x23];
/// The game's own row: 170 = 「하드코어 모드로;재시작합니다」 (`tools_re/ingame_ids.txt`).
pub const HARDCORE_RESTART_LABEL: u32 = 0xaa;
/// Our row: 「헬모드로 재시작합니다」 (one line — the widget centres each line), appended AFTER the 공격속도 label (row 198), so 199.
/// ⛔ Mirrored by hand in `tools_re/add_hell_restart_label.py::EXPECTED_INDEX`;
/// `patch_asset.py --check` reads this constant back out of this file and refuses a zip
/// whose row sits anywhere else.
pub const HELL_RESTART_LABEL: u32 = 199;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "ending_label hook at {pc:#x}: expected {HEAD:02x?} (movs r3,#0xaa), found {head:02x?}"
    )))
}

/// Which `InGame_txt` row the ending window shows for a playthrough on `difficulty`
/// (the byte at `reaverdrop::HARD_FLAG_ADDR`; `None` = unreadable). Normal keeps the game's
/// 하드코어 line — its next playthrough IS 하드코어; 하드코어 and 헬 get the 헬 line.
pub fn label_for(difficulty: Option<u8>) -> u32 {
    match difficulty {
        Some(b) if b != 0 => HELL_RESTART_LABEL,
        _ => HARDCORE_RESTART_LABEL,
    }
}

/// `0x46cec`: re-do `movs r3,#<row>` with the row [`label_for`] picks, then continue at the
/// `b 0x46cc4` that stores it.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let mut byte = [0u8; 1];
    let difficulty = match core.read_bytes(super::reaverdrop::HARD_FLAG_ADDR, &mut byte) {
        Ok(_) => Some(byte[0]),
        Err(e) => {
            tracing::warn!(
                "ending label: cannot read the difficulty byte at {:#x} ({e:?}) — the game's own 하드코어 line stays",
                super::reaverdrop::HARD_FLAG_ADDR
            );
            None
        }
    };
    let row = label_for(difficulty);
    tracing::warn!("ending label: difficulty {difficulty:?} -> InGame_txt row {row} ({})", if row == HELL_RESTART_LABEL { "헬모드로 재시작합니다" } else { "the game's own 하드코어 line" });
    core.inner.lock().engine.reg_write(ArmRegister::R3, row);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_only_installs_over_the_displaced_movs() {
        assert!(check_head([0xaa, 0x23], 0x46ced).is_ok(), "movs r3,#0xaa");
        assert!(check_head([0xe9, 0xe7], 0x46cef).is_err(), "the `b 0x46cc4` after it is the exit, not the site");
        assert!(check_head([0xa9, 0x22], 0x46dc3).is_err(), "window 17's `movs r2,#0xa9` is another site");
    }

    /// Normal's next playthrough IS 하드코어, so its line stays the game's; 하드코어 and
    /// 헬 (2, the host's own value — hell stays hell) get ours. Unreadable = the game's.
    #[test]
    fn the_row_follows_the_difficulty_and_fails_soft() {
        assert_eq!(label_for(Some(0)), HARDCORE_RESTART_LABEL);
        assert_eq!(label_for(Some(1)), HELL_RESTART_LABEL, "하드코어 clear");
        assert_eq!(label_for(Some(2)), HELL_RESTART_LABEL, "헬 clear stays 헬");
        assert_eq!(label_for(Some(0x81)), HELL_RESTART_LABEL, "the game's readers are all `cmp #0`");
        assert_eq!(label_for(None), HARDCORE_RESTART_LABEL, "unreadable: the game's own line");
    }

    /// The row index is a hand-mirrored contract with the asset tool: the stock table has
    /// 198 rows, 공격속도 is appended at 198, and this row right after it.
    #[test]
    fn the_hell_row_sits_right_after_the_attack_speed_label() {
        assert_eq!(HELL_RESTART_LABEL, 198 + 1);
        assert_eq!(HARDCORE_RESTART_LABEL, 170, "tools_re/ingame_ids.txt: 170 0xaa 하드코어 모드로;재시작합니다");
        assert!(HELL_RESTART_LABEL <= 0xff, "the switch loads it with an 8-bit movs");
    }
}
