//! Tower species — the host's species triple written over a scene header's, as it loads.
//!
//! ── WHY A HOOK ───────────────────────────────────────────────────────────────
//! `loadScene(scene)` (LGT `0x15f28`) reads `res/event/e%02d%02d_scn`, stores the scene
//! id (`strh` to `base+0x9e08` at `0x15f9c`), calls `parseScnHeader` (`0x14284`), loads
//! the map and spawns — one guest call, no host tick anywhere inside it. The spawn loop
//! (`0x1587e`..`0x158ee`) reads the species for a spawn point's `type` from the three
//! words `parseScnHeader` stored at `base+0x9e3c` (`0x142ae`..`0x142b6`, a `stm` of
//! `header[3..5]`). So "which monsters a floor spawns" is a property of the SCENE FILE,
//! and a tower that rotates seven scenes over 100 floors repeats its line-ups every few
//! floors — the report that started 88차 ("1,2,3 층 몬스터가 4,5,6 에도 똑같이").
//!
//! The words can only be replaced BETWEEN the header parse and the spawn loop, and that
//! window has no host boundary — hence a hook. It sits on the LAST instruction of
//! `parseScnHeader` before its epilogue, `0x142da ldr r0,[r6]` (`r6` = `base+0x9e1c`,
//! the map-id word the function returns), after every store the function makes. The
//! hook performs that load itself and resumes at `0x142dc`.
//!
//! ── THE CONTRACT ─────────────────────────────────────────────────────────────
//! * The host arms `wie_util::stats::TOWER_SPECIES` with `(scene, [s0, s1, s2])` before
//!   it posts the warp; `0xff` in a slot means "spawn nothing for that type", exactly
//!   as the header byte does, which is how the floor table also chooses the COUNT.
//! * The hook fires for EVERY scene load. It compares the armed scene against the id
//!   `loadScene` just stored and declines on a mismatch, so the origin town a `Leave`
//!   warps back to is never touched.
//! * Nothing else is written. The map id, the region, the intro byte all stay the
//!   header's; the species are the only three words this hook owns.
//!
//! ── FAIL SOFT ────────────────────────────────────────────────────────────────
//! Unset knob, wrong scene, unreadable memory: the scene loads with its own header,
//! which the generator fills with a sane fallback triple. The hook never refuses a load.

use alloc::format;

use wie_util::{Result, WieError, read_generic, write_generic};

use crate::{ArmCore, engine::ArmRegister, function::JumpTo};

/// `ldr r0, [r6]` at `0x142da` — the instruction the hook displaces and performs.
pub const EXPECTED_HEAD: [u8; 2] = [0x30, 0x68];

/// LGT: CHAR is a static at `.bss 0x1503a88`, and `getBase()` returns `CHAR + 0x28`.
const GUEST_BASE: u32 = 0x0150_3a88 + 0x28;
/// `(u16)[base+0x9e08]` — the scene id `loadScene` stores before parsing the header.
const SCENE_ID_OFF: u32 = 0x9e08;
/// The three species words, base-relative (`CHAR+0x9e64`).
const SPECIES_OFF: u32 = 0x9e3c;
/// What `r6` holds at the hook site: `base+0x9e1c`, the map-id word.
const MAP_ID_OFF: u32 = 0x9e1c;

pub fn check_head(head: [u8; 2], pc: u32) -> Result<()> {
    if head == EXPECTED_HEAD {
        return Ok(());
    }
    Err(WieError::FatalError(format!(
        "tower_species hook at {pc:#x}: expected {EXPECTED_HEAD:02x?} (ldr r0,[r6]), found {head:02x?}"
    )))
}

/// Should the armed triple be applied to this load? Pure: the knob, the scene the game
/// is loading, and the map-id pointer the function is about to return through.
///
/// The `r6` check is the frame check: if a future build moves the site or reallocates
/// registers, `r6` will not be the map-id word and the hook declines rather than write
/// three words next to whatever it does point at.
pub fn applies(armed: Option<(u8, [u8; 3])>, loading: u16, r6: u32) -> Option<[u8; 3]> {
    let (scene, triple) = armed?;
    if r6 != GUEST_BASE + MAP_ID_OFF || u16::from(scene) != loading {
        return None;
    }
    Some(triple)
}

/// Hook body: maybe replace the species words, then perform the displaced load.
pub async fn handle_lgt(core: &mut ArmCore, _hook_pc: u32, exit_pc: u32) -> Result<JumpTo> {
    let r6 = core.inner.lock().engine.reg_read(ArmRegister::R6);
    let loading: u16 = read_generic(core, GUEST_BASE + SCENE_ID_OFF)?;
    if let Some(triple) = applies(wie_util::stats::tower_species(), loading, r6) {
        let mut was = [0u32; 3];
        for (i, s) in triple.iter().enumerate() {
            let addr = GUEST_BASE + SPECIES_OFF + (i as u32) * 4;
            was[i] = read_generic(core, addr)?;
            write_generic(core, addr, u32::from(*s))?;
        }
        // warn, not info: the Android build filters the wie crates to warn, and "did
        // the floor get its own monsters" is the first question a repeat line-up raises.
        tracing::warn!("tower species: scene {loading} header {:?} -> {:?}", was, triple.map(u32::from));
    }
    // The displaced `ldr r0, [r6]`.
    let map_id: u32 = read_generic(core, r6)?;
    core.inner.lock().engine.reg_write(ArmRegister::R0, map_id);
    Ok(JumpTo(exit_pc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_is_ldr_r0_r6() {
        assert!(check_head([0x30, 0x68], 0x142db).is_ok());
        assert!(check_head([0x68, 0x30], 0x142db).is_err());
    }

    #[test]
    fn applies_only_to_the_armed_scene_with_the_right_frame() {
        let r6 = GUEST_BASE + MAP_ID_OFF;
        assert_eq!(applies(None, 243, r6), None);
        assert_eq!(applies(Some((243, [1, 2, 0xff])), 243, r6), Some([1, 2, 0xff]));
        // The origin town a `Leave` warps back to: same knob, different scene.
        assert_eq!(applies(Some((243, [1, 2, 0xff])), 111, r6), None);
        // A moved site: `r6` is not the map-id word.
        assert_eq!(applies(Some((243, [1, 2, 0xff])), 243, r6 + 4), None);
    }
}
