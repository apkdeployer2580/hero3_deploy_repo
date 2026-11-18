//! 150차 (2026-10-01) — the bag summary box's first line shows a reworked accessory's
//! THIRD option instead of 「레벨1 장신구/저항0」.
//!
//! ── WHAT THE GAME DRAWS ──────────────────────────────────────────────────────
//! The in-game 가방 page (overlay page 2) paints its 이름 / 효과 box with `0x41680`; the shop's
//! sell grid calls the same function (`0x1f78c` → `0x41680(7, x, y)`). For equipment
//! (`cat <= 0x0c`, jump table `0x5dbc4`) the 효과 box's first line is one `sprintf`:
//!
//! ```text
//! 0x41e76  add  r0,sp,#0x114          ; the 52-byte line buffer (SP+0x114 .. +0x147)
//! 0x41e78  ldr  r1,=0x5ceb4           ; 「레벨%d %s/%s%d」
//! 0x41e7a  bl   sprintf               ; 레벨 dbrow[+0x11] · 장신구 · 저항 · rec+0x3ce
//! 0x41e7e  ldr  r1,=0x2ad65 / bl      ; r0 = the graphics context
//! 0x41e84  ldr  r2,=0x5ae34           ; strlen — r2 is the call target, ⛔ never touched
//! 0x41e86  adds r5,r0,#0              ;                                <- HOOK (displaced)
//! 0x41e88  add  r0,sp,#0x114          ;                                <- exit_pc
//! 0x41e8a  bl   strlen / … 0x41eac drawString(gc, x+0x20, y+0x56, buf, len, font 1, 0)
//! ```
//!
//! `r4` is the bag record (`grid + tab*0x3c4 + slot*0x3c`, set at `0x41e62`..`0x41e66` and
//! read again for the orb line at `0x41eb0`), so `cat` = u16 `[r4+0x3b6]` and `idx` = u16
//! `[r4+0x3b8]`. For the 11 reworked accessories (`wie_util::acc_options`) the handler
//! writes the third option's summary (EUC-KR, ≤ 134 px in font 1, ≤ 51 bytes) + NUL over the
//! formatted line; strlen and the game's own drawer then paint it. Line 2 (the orb) and
//! every other item are the game's.
//!
//! The online screens draw the same box with copies of this code — the online single bag
//! (panel 0, `0x11aac`: buffer `r6` = SP+0x118, `r4` = the record, `r2` = strlen) and the
//! network warehouse (panel 1, `0x10958`: buffer SP+0x104, the row's cat/idx at
//! `s8 [r5-8]` / `[r5-7]` with `r5` = row+0x10, `r1` = strlen) — see [`handle_online_lgt`]
//! and [`handle_warehouse_lgt`]. ⚠ The warehouse site cannot meet one of the 11 today: the
//! 올리기 arm refuses equipment above tier 3 (`0x6f94`, 「아이템 등급이 영웅 이상인 아이템은 올릴
//! 수 없습니다」) and they are tier 5 / 4 — it is there so the three painters agree if that gate
//! is ever lifted (probe-verified with a poked warehouse row).
//!
//! FAIL SOFT: an unreadable record or a failed write leaves the game's own line. Both
//! flavours (the item table is flavour-free).

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use wie_util::{ByteRead, ByteWrite, Result, WieError, acc_options};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `adds r5,r0,#0` — displaced at the bag site (`0x41e86`) and the online-bag site (`0x124e6`).
pub const HEAD_R5: [u8; 2] = [0x05, 0x1c];
/// `adds r4,r0,#0` — displaced at the warehouse site (`0x113e6`).
pub const HEAD_R4: [u8; 2] = [0x04, 0x1c];
/// `rec + 0x3b6` — the bag record's `(u16 cat, u16 idx)`.
pub const REC_CAT: u32 = 0x3b6;
/// The bag site's line buffer (`add r0,sp,#0x114`).
pub const BAG_LINE_SP: u32 = 0x114;
/// The online-bag site's line buffer, which the code holds in `r6` (`0x1234a add r6,sp,#0x118`).
pub const ONLINE_LINE_SP: u32 = 0x118;
/// The warehouse site's line buffer (`add r0,sp,#0x104`).
pub const WAREHOUSE_LINE_SP: u32 = 0x104;
/// `r5` at the warehouse site is `row + 0x10`; the row's cat/idx are bytes at `+8` / `+9`.
pub const WAREHOUSE_ROW_BACK: u32 = 8;
/// All three buffers are 0x34 bytes (the next local sits right after): 51 + NUL.
pub const LINE_BYTES: usize = 0x34;

/// Distinct items logged per site (the painter runs every frame the box is up, so a line is
/// logged when the site shows a DIFFERENT item than it last logged, first N only).
const REPORT_MAX: u32 = 12;
static REPORTED: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
static LAST_LOGGED: [AtomicU32; 3] = [const { AtomicU32::new(u32::MAX) }; 3];

fn check(head: [u8; 2], want: [u8; 2], what: &str, pc: u32) -> Result<()> {
    if head == want {
        return Ok(());
    }
    Err(WieError::FatalError(format!("{what} hook at {pc:#x}: expected {want:02x?}, found {head:02x?}")))
}

pub fn check_bag_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, HEAD_R5, "acc_summary (adds r5,r0,#0)", pc)
}

pub fn check_warehouse_head(head: [u8; 2], pc: u32) -> Result<()> {
    check(head, HEAD_R4, "acc_summary_warehouse (adds r4,r0,#0)", pc)
}

/// The bytes written over the game's line for `(cat, idx)`, pure: the summary + NUL, or
/// `None` when the item keeps the game's line.
pub fn line_for(cat: u16, idx: u16) -> Option<([u8; LINE_BYTES], usize)> {
    let t = acc_options::third_of(cat, idx)?;
    let text = t.summary_euc;
    if text.is_empty() || text.len() >= LINE_BYTES || text.contains(&0) {
        return None; // the generator refuses these; never overrun the game's buffer
    }
    let mut out = [0u8; LINE_BYTES];
    out[..text.len()].copy_from_slice(text);
    Some((out, text.len() + 1))
}

/// Write the summary for `(cat, idx)` at `buf` (fail soft), log the first few per site.
fn write_line(core: &mut ArmCore, site: usize, what: &str, buf: u32, cat: u16, idx: u16) {
    let Some((line, len)) = line_for(cat, idx) else { return };
    match core.write_bytes(buf, &line[..len]) {
        Ok(()) => {
            let key = u32::from(cat) << 16 | u32::from(idx);
            if LAST_LOGGED[site].swap(key, Ordering::Relaxed) != key && REPORTED[site].fetch_add(1, Ordering::Relaxed) < REPORT_MAX {
                let t = acc_options::third_of(cat, idx);
                tracing::warn!(
                    "acc summary ({what}): ({cat:#x},{idx:#x}) {} -> 「{}」",
                    t.map_or("?", |t| t.name),
                    t.map_or("?", |t| t.summary)
                );
            }
        }
        Err(e) => tracing::warn!("acc summary ({what}): cannot write the line at {buf:#x} ({e:?}) — the game's line stays"),
    }
}

/// The bag record's `(cat, idx)` (u16 each, adjacent), or `None` when unreadable.
fn record_item(core: &ArmCore, rec: u32) -> Option<(u16, u16)> {
    let mut b = [0u8; 4];
    core.read_bytes(rec.checked_add(REC_CAT)?, &mut b).ok()?;
    Some((u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]])))
}

/// `0x41e86` (가방 page + shop sell grid): `adds r5,r0,#0`, then the line.
pub async fn handle_bag_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (rec, sp) = {
        let mut inner = core.inner.lock();
        let r0 = inner.engine.reg_read(ArmRegister::R0);
        inner.engine.reg_write(ArmRegister::R5, r0);
        (inner.engine.reg_read(ArmRegister::R4), inner.engine.reg_read(ArmRegister::SP))
    };
    if let Some((cat, idx)) = record_item(core, rec) {
        write_line(core, 0, "bag", sp + BAG_LINE_SP, cat, idx);
    }
    Ok(JumpTo(exit_pc))
}

/// `0x124e6` (online single bag, panel 0): `adds r5,r0,#0`, then the line into `r6`.
/// Frame check: `r6` must be this painter's buffer (SP+0x118), else the game's line stays.
pub async fn handle_online_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (rec, buf, sp) = {
        let mut inner = core.inner.lock();
        let r0 = inner.engine.reg_read(ArmRegister::R0);
        inner.engine.reg_write(ArmRegister::R5, r0);
        (
            inner.engine.reg_read(ArmRegister::R4),
            inner.engine.reg_read(ArmRegister::R6),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    if buf == sp.wrapping_add(ONLINE_LINE_SP)
        && let Some((cat, idx)) = record_item(core, rec)
    {
        write_line(core, 1, "online bag", buf, cat, idx);
    }
    Ok(JumpTo(exit_pc))
}

/// `0x113e6` (network warehouse, panel 1): `adds r4,r0,#0`, then the line. The row is the
/// 20-byte wire record the warehouse caches (`CHAR+0xa57c`), `r5` = row+0x10.
pub async fn handle_warehouse_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let (row10, sp) = {
        let mut inner = core.inner.lock();
        let r0 = inner.engine.reg_read(ArmRegister::R0);
        inner.engine.reg_write(ArmRegister::R4, r0);
        (inner.engine.reg_read(ArmRegister::R5), inner.engine.reg_read(ArmRegister::SP))
    };
    let mut b = [0u8; 2];
    if let Some(at) = row10.checked_sub(WAREHOUSE_ROW_BACK)
        && core.read_bytes(at, &mut b).is_ok()
    {
        let (cat, idx) = (b[0] as i8, b[1] as i8);
        if cat >= 0 && idx >= 0 {
            write_line(core, 2, "warehouse", sp + WAREHOUSE_LINE_SP, cat as u16, idx as u16);
        }
    }
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use wie_util::{acc_options::ACC_THIRDS, read_generic};

    use super::*;

    #[test]
    fn the_hooks_only_install_over_the_displaced_moves() {
        assert!(check_bag_head([0x05, 0x1c], 0x41e87).is_ok());
        assert!(check_bag_head([0x45, 0xa8], 0x41e89).is_err(), "the `add r0,sp,#0x114` after it is the exit");
        assert!(check_warehouse_head([0x04, 0x1c], 0x113e7).is_ok());
        assert!(check_warehouse_head([0x05, 0x1c], 0x113e7).is_err());
    }

    /// Every summary fits the painter's 52-byte buffer with its NUL, holds no NUL of its own,
    /// and only the 11 table rows are rewritten.
    #[test]
    fn every_summary_fits_the_game_buffer() {
        for t in &ACC_THIRDS {
            assert!(!t.summary_euc.is_empty() && t.summary_euc.len() <= 51, "{}", t.name);
            assert!(!t.summary_euc.contains(&0), "{}", t.name);
            let (line, len) = line_for(u16::from(t.cat), u16::from(t.idx)).expect(t.name);
            assert_eq!(&line[..len - 1], t.summary_euc);
            assert_eq!(line[len - 1], 0, "NUL-terminated");
            assert!(len <= LINE_BYTES);
        }
        assert!(line_for(0x0c, 0x27).is_none(), "a stock accessory keeps 「레벨N 장신구/저항M」");
        assert!(line_for(0x00, 0x30).is_none(), "템플러헬름 keeps 「레벨85 투구/방206」");
        assert!(line_for(0x0b, 0x2a).is_none(), "the category counts");
    }

    /// The three sites in the shipped `binary.mod` (when this checkout has it).
    #[test]
    fn the_binary_agrees() {
        let path = std::format!("{}/../../../tools_re/_binary.mod", env!("CARGO_MANIFEST_DIR"));
        let Ok(elf) = std::fs::read(&path) else {
            std::eprintln!("skipped: {path} is not in this checkout");
            return;
        };
        let fo = |va: u32| (va - 0x1000 + 0x34) as usize;
        let at = |va: u32| [elf[fo(va)], elf[fo(va) + 1]];
        assert_eq!(at(0x41e86), HEAD_R5, "bag site");
        assert_eq!(at(0x41e88), [0x45, 0xa8], "add r0,sp,#0x114 (the buffer)");
        assert_eq!(at(0x41e84), [0xb6, 0x4a], "ldr r2,=strlen right before");
        assert_eq!(at(0x124e6), HEAD_R5, "online-bag site");
        assert_eq!(at(0x124e8), [0x30, 0x1c], "adds r0,r6,#0 (the buffer is r6)");
        assert_eq!(at(0x1234a), [0x46, 0xae], "add r6,sp,#0x118");
        assert_eq!(at(0x113e6), HEAD_R4, "warehouse site");
        assert_eq!(at(0x113e8), [0x41, 0xa8], "add r0,sp,#0x104 (the buffer)");
        assert_eq!(at(0x113e4), [0xde, 0x49], "ldr r1,=strlen right before");
    }

    #[futures_test::test]
    async fn the_bag_line_is_rewritten_for_a_table_item_only() -> Result<()> {
        const SP: u32 = 0x10_f000;
        const REC: u32 = 0x10_2000;
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10_0000, 0x10000)?;
        let game_line = b"\xb7\xb9\xba\xa7\x31 \xc0\xe5\xbd\xc5\xb1\xb8/\xc0\xfa\xc7\xd70\0";
        for (cat, idx, rewritten) in [(0x0cu16, 0x31u16, true), (0x0c, 0x27, false), (0x00, 0x30, false)] {
            core.write_bytes(SP + BAG_LINE_SP, &[0xaa; LINE_BYTES])?;
            core.write_bytes(SP + BAG_LINE_SP, game_line)?;
            core.write_bytes(REC + REC_CAT, &[cat.to_le_bytes(), idx.to_le_bytes()].concat())?;
            {
                let mut inner = core.inner.lock();
                inner.engine.reg_write(ArmRegister::R0, 0x4015_0000);
                inner.engine.reg_write(ArmRegister::R2, 0x5ae34);
                inner.engine.reg_write(ArmRegister::R4, REC);
                inner.engine.reg_write(ArmRegister::R5, 0);
                inner.engine.reg_write(ArmRegister::SP, SP);
            }
            let JumpTo(pc) = handle_bag_lgt(&mut core, 0x41e87, 0x41e89).await?;
            assert_eq!(pc, 0x41e89);
            {
                let inner = core.inner.lock();
                assert_eq!(inner.engine.reg_read(ArmRegister::R5), 0x4015_0000, "the displaced adds r5,r0,#0");
                assert_eq!(inner.engine.reg_read(ArmRegister::R2), 0x5ae34, "strlen's address is untouched");
                assert_eq!(inner.engine.reg_read(ArmRegister::R4), REC, "the record (line 2 reads it)");
            }
            let mut got = [0u8; LINE_BYTES];
            core.read_bytes(SP + BAG_LINE_SP, &mut got)?;
            if rewritten {
                let t = acc_options::third_of(cat, idx).unwrap();
                assert_eq!(&got[..t.summary_euc.len()], t.summary_euc, "{}", t.name);
                assert_eq!(got[t.summary_euc.len()], 0);
            } else {
                assert_eq!(&got[..game_line.len()], game_line, "({cat:#x},{idx:#x}) keeps the game's line");
            }
            let past: u8 = read_generic(&core, SP + BAG_LINE_SP + LINE_BYTES as u32)?;
            assert_eq!(past, 0, "nothing past the buffer");
        }
        Ok(())
    }

    #[futures_test::test]
    async fn the_warehouse_line_reads_the_wire_row() -> Result<()> {
        const SP: u32 = 0x10_f000;
        const ROW: u32 = 0x10_3000;
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10_0000, 0x10000)?;
        core.write_bytes(ROW + 8, &[0x0c, 0x2e])?; // 레아의목걸이
        core.write_bytes(SP + WAREHOUSE_LINE_SP, &[0xaa; LINE_BYTES])?;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, 0x4015_0000);
            inner.engine.reg_write(ArmRegister::R1, 0x5ae34);
            inner.engine.reg_write(ArmRegister::R5, ROW + 0x10);
            inner.engine.reg_write(ArmRegister::SP, SP);
        }
        let JumpTo(pc) = handle_warehouse_lgt(&mut core, 0x113e7, 0x113e9).await?;
        assert_eq!(pc, 0x113e9);
        {
            let inner = core.inner.lock();
            assert_eq!(inner.engine.reg_read(ArmRegister::R4), 0x4015_0000, "the displaced adds r4,r0,#0");
            assert_eq!(inner.engine.reg_read(ArmRegister::R1), 0x5ae34, "strlen's address is untouched");
            assert_eq!(inner.engine.reg_read(ArmRegister::R5), ROW + 0x10, "the row (the orb line reads it)");
        }
        let t = acc_options::third_of(0x0c, 0x2e).unwrap();
        let mut got = [0u8; LINE_BYTES];
        core.read_bytes(SP + WAREHOUSE_LINE_SP, &mut got)?;
        assert_eq!(&got[..t.summary_euc.len()], t.summary_euc);
        assert_eq!(got[t.summary_euc.len()], 0);
        Ok(())
    }
}
