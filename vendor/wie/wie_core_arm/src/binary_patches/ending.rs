//! The 2회차 transition — the host hears it run (135차-B, 2026-09-24).
//!
//! ── WHAT THE GAME DOES ───────────────────────────────────────────────────────
//! After the staff roll (scene 241) `0x3e840` opens ending window 18, and that window's
//! KEY handler `0x3b420` calls `0x3af2c` as its FIRST command, for any key:
//!
//! ```text
//! 0x3b420  bl 0x3af2c        ; the transition — the only call site in the image
//! 0x3af2c  push {r4-r7,lr}   ; <- HOOK (displaced, `resume_past_push`)
//!          …                  ; walks tab 5 x slot 16 and removes cat 0x11 · (0x12, 0x09)
//!                             ; 그리폰의피리 · (0x0f, 0x12) its recipe; difficulty = 1
//!                             ; (`0x3b0bc`); warp to scene 83 NEMESIS_PLATEAU_1
//! ```
//!
//! Two host features hang off that call: the boxes give up what the bag just gave up
//! (124차 — the bag walk cannot see them) and a HARDCORE clear opens 헬 (133차), which
//! needs the difficulty the playthrough had BEFORE the transition writes 1.
//!
//! ── WHY A HOOK ───────────────────────────────────────────────────────────────
//! Both used to watch window 18 rise and fall on a poll every five worker iterations. The
//! window lives only until the next key — probe-measured 5 frames under 8-frame mashing and
//! 3 under 3-frame mashing (135차 §2 ②) — and the worker also runs several ticks in one
//! iteration when the game is work-bound, so a mashed ending could skip both with no log
//! line at all: the 그리폰의피리 stayed in the box and a hardcore clear did not open 헬.
//! The call itself cannot be missed. This hook latches it into
//! `wie_util::stats::note_ending_transition` — a counter plus the byte it found — and the
//! worker handles every increment on whatever poll it likes, because a counter does not
//! expire.
//!
//! ── THE CONTRACT ─────────────────────────────────────────────────────────────
//! * It changes NOTHING in the guest: it reads one byte, stores two atomics, and runs the
//!   displaced push. The game's transition is byte-for-byte its own.
//! * FAIL SOFT: an unreadable difficulty byte still bumps the counter (the boxes must still
//!   give up the 피리) with `None` for the byte, which the host reads as "no promotion".
//! * Both flavours: the box half is flavour-free; the 헬 half is gated on the host side.

use alloc::format;

use wie_util::{ByteRead, Result, WieError};

use crate::{ArmCore, function::JumpTo};

/// `push {r4-r7,lr}` at `0x3af2c` — the instruction the hook displaces.
pub const EXPECTED_HEAD: [u8; 2] = [0xf0, 0xb5];

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "ending_transition hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (push {{r4-r7,lr}}), found {head:02x?}"
    )))
}

/// `0x3af2c` entry: note the transition and the difficulty it is about to overwrite.
pub async fn handle_lgt(core: &mut ArmCore, hook_pc: u32) -> Result<JumpTo> {
    let mut byte = [0u8; 1];
    let before = match core.read_bytes(super::reaverdrop::HARD_FLAG_ADDR, &mut byte) {
        Ok(_) => Some(byte[0]),
        Err(e) => {
            tracing::warn!(
                "ending transition: cannot read the difficulty byte at {:#x} ({e:?}) — the boxes still give up the 피리, 헬 is not promoted",
                super::reaverdrop::HARD_FLAG_ADDR
            );
            None
        }
    };
    wie_util::stats::note_ending_transition(before);
    tracing::warn!("ending transition: 0x3af2c is running — the difficulty byte was {before:?}");
    super::hook::resume_past_push(core, hook_pc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_only_installs_over_the_transitions_own_push() {
        assert!(check_head([0xf0, 0xb5], 0x3af2d).is_ok());
        // `resume_past_push` emulates exactly `push {r4-r7,lr}` — any other head would
        // resume with a wrong frame.
        assert!(check_head([0xf0, 0xb4], 0x3af2d).is_err(), "push {{r4-r7}} without lr");
        assert!(check_head([0x83, 0xb0], 0x3af2d).is_err(), "sub sp — the next instruction");
    }

    /// The counter moves once per call and carries the byte as the transition found it —
    /// ⛔ the one fact the old window poll had to catch on its rising edge.
    #[test]
    fn a_transition_is_latched_with_the_byte_it_found() {
        let (n0, _) = wie_util::stats::ending_transition();
        wie_util::stats::note_ending_transition(Some(1));
        let (n1, d1) = wie_util::stats::ending_transition();
        assert_eq!(n1, n0.wrapping_add(1));
        assert_eq!(d1, Some(1), "a hardcore clear");
        wie_util::stats::note_ending_transition(None);
        let (n2, d2) = wie_util::stats::ending_transition();
        assert_eq!(n2, n1.wrapping_add(1), "an unreadable byte still counts");
        assert_eq!(d2, None);
    }
}
