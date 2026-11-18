//! Dropped-item lifetime — 60 s instead of 6, and the 30-slot array stays 30 (162차, 2026-10-03).
//!
//! User: «아이템 없어지는데 시간을 늘려줘 60초정도로 늘렸으면 좋겠어».
//!
//! ── THE LOOT RECORDS ─────────────────────────────────────────────────────────
//! `grid = *(getBase()+0x9e00)` (= `*(CHAR+0x9e28)`, the inventory grid) also owns the
//! field's dropped items: 30 records of 20 B at `grid+0x60 + i*0x14`, count (i8) at
//! `grid+0x2b8` (the grid init `0x53760` memsets exactly 0x258 = 30 × 20 there; the scene
//! clear `0x54bb0` walks 0x1d+1 = 30 slots and fills them with 0xff).
//!
//! ```text
//! +0 x (i8 tile) · +1 y · +2 idx · +3 cat · +4 drop-group size · +8 u64 spawn time
//! +0x10 u16 lifetime counter · +0x12 active (1)
//! ```
//!
//! Spawn `0x54758(x, y, n, cats*, idxs*)` (the kill-loot routine `0x38e70`'s tail) appends
//! `n` records at `count..` and bumps the count — with NO bounds check. The loot tick
//! `0x54914` (world tick `0x3b6xx`, gated on «no menu open», once per 40 ms — the field
//! logic runs every other 20 ms frame; probe: one removal per two frames) bumps every
//! record's counter and removes the FIRST one past its lifetime (`0x54848`: zero the
//! record, compact the rest down in order, then `count -= 1`):
//!
//! ```text
//! 0x5493e  ldrh r3,[r2] ; adds r3,#1 ; strh r3,[r2]         counter += 1
//! 0x54944  lsls r3,r3,#16 ; asrs r3,r3,#16                   (i16) — a scene-cleared
//!                                                            slot starts at 0xffff = -1
//! 0x54948  cmp  r3,#0x96 ; bgt remove                        > 150 ticks = 6 s     <- PATCH
//! ```
//!
//! and the painter `0x54c00` blinks a record whose counter is past its own threshold:
//!
//! ```text
//! 0x54c7e  ldrh r3,[r5,#0x10] ; lsls r2,r3,#16 ; asrs r3,r2,#16
//! 0x54c84  cmp  r3,#0x77 ; bgt blink                         > 119 = the last 1.2 s  <- PATCH
//! 0x54c94  lsls r3,r2,#14 ; bne skip                         bit 1 of the counter
//! ```
//!
//! ── THE TWO BYTE PATCHES (both flavours, `binary_patches.toml` «LOOT LIFETIME») ───
//! 1500 ticks do not fit a `cmp rN,#imm8`, so each compare keeps its sign-extending
//! `lsls #16` and turns the `asrs #16` into `asrs #19`: the compare then sees
//! `counter / 8` — ARITHMETIC, so a -1 start stays -1 and survives exactly as before —
//! and `#0xbb` (187) removes from counter 1504 = 60.2 s, `#0xab` (171) blinks from
//! 1376 = 55.0 s (the last five seconds). Only the two halfwords after each `lsls`
//! change; the branches and the blink bit are the game's own. [`removed_at`] /
//! [`blinks_at`] are those compares in Rust and the tests pin the toml bytes to them.
//!
//! ── THE HOOK (both flavours) ─────────────────────────────────────────────────
//! Ten times the lifetime makes the unchecked append reachable: at 6 s thirty items on
//! the ground was out of reach, at 60 s a dense hell map gets there (2..5 rolls per kill
//! on the hard table plus the host's own drops), and the 31st record would be written
//! over the count byte and the acquisition-notice queue right behind it (`grid+0x2b9..`,
//! 103차). The hook sits on the spawn loop's count read (`0x54790 movs r2,#0`,
//! displaced): when the array is full it evicts the OLDEST record first (index 0 — spawn
//! appends, every removal compacts in order), shifts the rest down and zeroes the freed
//! slot the way the game's own remove does, decrements the count, then lets the append
//! run. Nothing is ever written past slot 29.
//!
//! FAIL SOFT: an unreadable count, or a count below the cap, is the displaced instruction
//! and nothing else.

use alloc::{format, vec};

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `movs r2,#0` at `0x54790` — the spawn loop's count read, displaced.
pub const EXPECTED_HEAD: [u8; 2] = [0x00, 0x22];
/// The array: 30 records of 20 bytes; the count byte sits right behind the last one.
pub const LOOT_SLOTS: u32 = 30;
pub const LOOT_REC_LEN: u32 = 0x14;
/// `grid + 0x60` = the first record · `grid + 0x2b8` = the count (= first + 30 × 20).
pub const LOOT_FIRST_OFF: u32 = 0x60;
pub const LOOT_COUNT_OFF: u32 = 0x2b8;

/// The two byte patches, as numbers rather than bytes: what the toml's halfwords mean.
/// Only the tests need them (the game runs the patched bytes themselves), so the whole
/// specification is test-only and the shipping build carries no dead code.
#[cfg(test)]
pub(super) mod spec {
    /// The compares' new shift (`asrs #19` = the sign-extending 16 plus this) and immediates.
    pub const LIFETIME_SHIFT: u32 = 3;
    pub const REMOVE_CMP: u32 = 0xbb;
    pub const BLINK_CMP: u32 = 0xab;
    /// World ticks per second on the field (the loot tick runs every other 20 ms frame).
    pub const TICKS_PER_SECOND: u32 = 25;
    /// The shipped game's two immediates, for the record.
    pub const SHIPPED_REMOVE_CMP: u32 = 0x96;
    pub const SHIPPED_BLINK_CMP: u32 = 0x77;
    /// The LGT 01.00.07 `binary.mod` the three sites were derived on (md5 6a352944…).
    pub const LGT_MD5: [u8; 16] = [
        0x6a, 0x35, 0x29, 0x44, 0xbb, 0x43, 0xc0, 0x78, 0x9c, 0xa9, 0xf4, 0xb6, 0xf0, 0x70, 0x2f, 0x4b,
    ];

    /// The two byte patches as the toml carries them: `(pc, expect, bytes)`.
    /// Tick: `asrs r3,r3,#19 ; cmp r3,#0xbb` over `asrs r3,r3,#16 ; cmp r3,#0x96`.
    pub const TICK_PATCH: (u32, [u8; 4], [u8; 4]) = (0x54946, [0x1b, 0x14, 0x96, 0x2b], [0xdb, 0x14, 0xbb, 0x2b]);
    /// Painter: `asrs r3,r2,#19 ; cmp r3,#0xab` over `asrs r3,r2,#16 ; cmp r3,#0x77`.
    pub const PAINT_PATCH: (u32, [u8; 4], [u8; 4]) = (0x54c82, [0x13, 0x14, 0x77, 0x2b], [0xd3, 0x14, 0xab, 0x2b]);
    /// The spawn-cap hook's trap and resume points (`movs r2,#0` → `ldrsb r2,[r5,r2]`).
    pub const HOOK_PC: u32 = 0x54791;
    pub const HOOK_EXIT_PC: u32 = 0x54793;

    /// The first counter value a `cmp #imm` after `asrs #(16+SHIFT)` branches on.
    pub const fn first_past(cmp: u32) -> u32 {
        (cmp + 1) << LIFETIME_SHIFT
    }

    /// The patched removal compare, pure: `(i16)counter >> 3 > 0xbb`.
    pub fn removed_at(counter: u16) -> bool {
        (i32::from(counter as i16) >> LIFETIME_SHIFT) > REMOVE_CMP as i32
    }

    /// The patched blink compare, pure: `(i16)counter >> 3 > 0xab`.
    pub fn blinks_at(counter: u16) -> bool {
        (i32::from(counter as i16) >> LIFETIME_SHIFT) > BLINK_CMP as i32
    }

    /// Decode a Thumb-1 `asrs Rd,Rm,#imm5` halfword into `(imm5, rm, rd)`, or `None`.
    pub fn decode_asrs(hw: u16) -> Option<(u32, u32, u32)> {
        (hw & 0xf800 == 0x1000).then(|| (u32::from(hw >> 6) & 0x1f, u32::from(hw >> 3) & 7, u32::from(hw) & 7))
    }

    /// Decode a Thumb-1 `cmp Rn,#imm8` halfword into `(rn, imm8)`, or `None`.
    pub fn decode_cmp_imm(hw: u16) -> Option<(u32, u32)> {
        (hw & 0xf800 == 0x2800).then(|| (u32::from(hw >> 8) & 7, u32::from(hw) & 0xff))
    }
}

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "loot_spawn_cap hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (movs r2,#0), found {head:02x?}"
    )))
}

/// What the hook does at the count read, pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Room left (or an unreadable count): the displaced instruction only.
    Append,
    /// The array is full: evict the oldest record, then append. Carries the count the
    /// shift works on (clamped to the array — a count past 30 means something else
    /// already overflowed; the slots that exist are what gets compacted).
    Evict { count: u32 },
}

pub fn plan(count: Option<i8>) -> Plan {
    match count {
        Some(n) if n >= LOOT_SLOTS as i8 => Plan::Evict { count: (n as u32).min(LOOT_SLOTS) },
        _ => Plan::Append,
    }
}

/// Hook body: evict the oldest record when the array is full, then do `movs r2,#0`.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let r5 = core.inner.lock().engine.reg_read(ArmRegister::R5);
    let count = {
        let mut b = [0u8; 1];
        core.read_bytes(r5, &mut b).ok().map(|_| b[0] as i8)
    };
    if let Plan::Evict { count } = plan(count) {
        let first = r5.wrapping_sub(LOOT_COUNT_OFF).wrapping_add(LOOT_FIRST_OFF);
        let mut oldest = [0u8; LOOT_REC_LEN as usize];
        core.read_bytes(first, &mut oldest)?;
        // Records 1..count move down one slot; the freed last slot is zeroed like `0x54814`.
        let mut rest = vec![0u8; ((count - 1) * LOOT_REC_LEN) as usize];
        core.read_bytes(first + LOOT_REC_LEN, &mut rest)?;
        core.write_bytes(first, &rest)?;
        core.write_bytes(first + (count - 1) * LOOT_REC_LEN, &[0u8; LOOT_REC_LEN as usize])?;
        core.write_bytes(r5, &[(count - 1) as u8])?;
        // warn: the Android build filters the wie crates to warn.
        tracing::warn!(
            "loot: the field already holds its {LOOT_SLOTS} dropped items — the oldest (cat {:#x} idx {:#x} at tile ({},{}), {} tick(s) old) makes room for the new drop",
            oldest[3],
            oldest[2],
            oldest[0] as i8,
            oldest[1] as i8,
            u16::from_le_bytes([oldest[0x10], oldest[0x11]]) as i16
        );
    }
    core.inner.lock().engine.reg_write(ArmRegister::R2, 0);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::{spec::*, *};
    use crate::binary_patches::hook::HookKind;

    #[test]
    fn head_is_movs_r2_0() {
        assert!(check_head([0x00, 0x22], 0x54791).is_ok());
        assert!(check_head([0xaa, 0x56], 0x54793).is_err(), "the ldrsb after it is the exit");
    }

    #[test]
    fn the_lifetime_is_about_a_minute_and_the_blink_its_last_five_seconds() {
        assert_eq!(first_past(REMOVE_CMP), 1504, "removed once the counter reaches 1504");
        assert_eq!(first_past(BLINK_CMP), 1376, "blinks from 1376");
        // 60 s ± one tick-octet, at the field's 25 world ticks per second.
        assert!((1500..=1512).contains(&first_past(REMOVE_CMP)));
        assert_eq!(first_past(REMOVE_CMP) / TICKS_PER_SECOND, 60);
        assert_eq!((first_past(REMOVE_CMP) - first_past(BLINK_CMP)) / TICKS_PER_SECOND, 5);
        // the shipped game: 150 ticks = 6 s, blinking from 120.
        assert_eq!((SHIPPED_REMOVE_CMP + 1) / TICKS_PER_SECOND, 6);
        assert_eq!(SHIPPED_BLINK_CMP + 1, 120);
    }

    /// The compares in Rust: the shipped threshold no longer removes, the new one does,
    /// and the -1 a scene-cleared slot starts at survives (arithmetic shift).
    #[test]
    fn the_patched_compares_keep_their_sign() {
        assert!(!removed_at(0xffff), "a scene-cleared slot starts at -1 and must live");
        assert!(!blinks_at(0xffff));
        assert!(!removed_at(0));
        assert!(!removed_at(150), "the shipped threshold");
        assert!(!removed_at(151));
        assert!(!removed_at(1503));
        assert!(removed_at(1504));
        assert!(removed_at(1505));
        assert!(!blinks_at(119), "the shipped blink threshold");
        assert!(!blinks_at(1375));
        assert!(blinks_at(1376));
        assert!(blinks_at(1504), "still blinking when it goes");
        assert!(removed_at(0x7fff));
    }

    /// The toml's bytes ARE these compares: decode both halfwords of each patch.
    #[test]
    fn the_byte_patches_encode_exactly_the_compares_in_rust() {
        for (what, (_, expect, bytes), rm, cmp, shipped) in [
            ("tick", TICK_PATCH, 3, REMOVE_CMP, SHIPPED_REMOVE_CMP),
            ("paint", PAINT_PATCH, 2, BLINK_CMP, SHIPPED_BLINK_CMP),
        ] {
            let old_asr = decode_asrs(u16::from_le_bytes([expect[0], expect[1]])).expect(what);
            let new_asr = decode_asrs(u16::from_le_bytes([bytes[0], bytes[1]])).expect(what);
            assert_eq!(old_asr, (16, rm, 3), "{what}: the shipped asrs #16 into r3");
            assert_eq!(new_asr, (16 + LIFETIME_SHIFT, rm, 3), "{what}: same registers, #19");
            let old_cmp = decode_cmp_imm(u16::from_le_bytes([expect[2], expect[3]])).expect(what);
            let new_cmp = decode_cmp_imm(u16::from_le_bytes([bytes[2], bytes[3]])).expect(what);
            assert_eq!(old_cmp, (3, shipped), "{what}: the shipped cmp r3,#imm");
            assert_eq!(new_cmp, (3, cmp), "{what}: cmp r3 against the new immediate");
        }
        // and the two sites are 2 bytes past their `lsls #16` (0x54944 / 0x54c80)
        assert_eq!(TICK_PATCH.0, 0x54944 + 2);
        assert_eq!(PAINT_PATCH.0, 0x54c80 + 2);
    }

    /// The embedded `data/binary_patches.toml` carries exactly these three sites on the
    /// shipped LGT entry, on both flavours — the bytes above are not just a Rust copy.
    #[test]
    fn the_embedded_toml_carries_the_three_sites_on_the_shipped_binary() {
        let entries = crate::binary_patches::parser::binary_patches();
        let lgt = entries.iter().find(|e| e.hash == Some(LGT_MD5)).unwrap_or_else(|| {
            panic!("no entry with hash {LGT_MD5:02x?}; entries: {:?}", entries.iter().map(|e| (e.name.clone(), e.hash)).collect::<alloc::vec::Vec<_>>())
        });
        for (what, (pc, expect, bytes)) in [("tick", TICK_PATCH), ("paint", PAINT_PATCH)] {
            let p = lgt.patches.iter().find(|p| p.pc == pc).unwrap_or_else(|| panic!("{what}: a patch at {pc:#x}"));
            assert_eq!(p.bytes, bytes.to_vec(), "{what}: bytes");
            assert_eq!(p.expect.as_deref(), Some(&expect[..]), "{what}: expect pins the shipped halfwords");
            assert_eq!(p.flavor, None, "{what}: both flavours");
        }
        let caps: alloc::vec::Vec<_> = lgt.hooks.iter().filter(|h| matches!(h.kind, HookKind::LootSpawnCapLgt { .. })).collect();
        assert_eq!(caps.len(), 1, "exactly one spawn-cap hook");
        assert_eq!(caps[0].pc, HOOK_PC);
        assert!(matches!(caps[0].kind, HookKind::LootSpawnCapLgt { exit_pc } if exit_pc == HOOK_EXIT_PC));
        // and the hook does not sit on either byte patch
        assert!(lgt.patches.iter().all(|p| !(p.pc..p.pc + p.bytes.len() as u32).contains(&(HOOK_PC & !1))));
    }

    #[test]
    fn the_cap_evicts_only_when_the_array_is_full() {
        assert_eq!(plan(None), Plan::Append, "unreadable = the game's own");
        for n in [0i8, 1, 15, 29] {
            assert_eq!(plan(Some(n)), Plan::Append, "count {n}");
        }
        assert_eq!(plan(Some(30)), Plan::Evict { count: 30 });
        // a count past the array (already overflowed) compacts the slots that exist
        assert_eq!(plan(Some(31)), Plan::Evict { count: 30 });
        assert_eq!(plan(Some(-1)), Plan::Append, "a negative count is not full");
    }

    #[test]
    fn the_array_geometry_matches_the_games_init() {
        // `0x53760`: memset(grid+0x60, 0, 0x96<<2); the count byte follows the last record.
        assert_eq!(LOOT_SLOTS * LOOT_REC_LEN, 0x96 << 2);
        assert_eq!(LOOT_FIRST_OFF + LOOT_SLOTS * LOOT_REC_LEN, LOOT_COUNT_OFF);
    }
}
