//! Tiny persisted settings (currently just the orientation preference), stored
//! as a one-line file in the app-private data dir so it survives restarts.

use std::path::{Path, PathBuf};

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_config.txt")
}

/// Returns the saved orientation preference.
///
/// ★ Default changed to LANDSCAPE 2026-07-31, when the orientation row was removed
/// from the settings modal (player request). Defaulting to portrait with no way to
/// change it would strand a fresh install: the layout editor is landscape-only, so
/// the one screen that could fix anything would be unreachable. Only an explicit
/// "portrait" in the file selects portrait now.
pub fn read_landscape(data_dir: &Path) -> bool {
    match std::fs::read_to_string(config_path(data_dir)) {
        Ok(s) => s.trim() != "portrait",
        Err(_) => true,
    }
}

pub fn write_landscape(data_dir: &Path, landscape: bool) {
    let value = if landscape { "landscape" } else { "portrait" };
    if let Err(e) = std::fs::write(config_path(data_dir), value) {
        log::warn!("failed to persist orientation pref: {e}");
    }
}

fn quickpage_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_quickpages.txt")
}

/// Load the host-owned quickbar pages. A missing or unreadable file gives an empty
/// store on page 0, which is the guest's own bar — i.e. exactly the pre-feature
/// behaviour, so a lost file costs the player their page-2 arrangement and nothing else.
pub fn read_quickpages(data_dir: &Path) -> crate::quickpage::Pages {
    match std::fs::read_to_string(quickpage_path(data_dir)) {
        Ok(s) => crate::quickpage::Pages::parse(&s),
        Err(_) => crate::quickpage::Pages::new(),
    }
}

/// Persist the pages. Called on every flip: the store is the only record of what is on
/// an inactive page, and the app can be killed at any moment from the background.
pub fn write_quickpages(data_dir: &Path, pages: &crate::quickpage::Pages) {
    if let Err(e) = std::fs::write(quickpage_path(data_dir), pages.serialize()) {
        log::warn!("failed to persist quickbar pages: {e}");
    }
}

fn control_style_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_control_style.txt")
}

/// Whether the left control is a floating JOYSTICK (default) vs the classic
/// D-pad. Persisted so the choice survives restarts.
pub fn read_joystick(data_dir: &Path) -> bool {
    match std::fs::read_to_string(control_style_path(data_dir)) {
        Ok(s) => s.trim() != "dpad", // default (missing/other) = joystick
        Err(_) => true,
    }
}

pub fn write_joystick(data_dir: &Path, joystick: bool) {
    let value = if joystick { "joystick" } else { "dpad" };
    if let Err(e) = std::fs::write(control_style_path(data_dir), value) {
        log::warn!("failed to persist control style: {e}");
    }
}

fn touch_type_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_touch_type.txt")
}

/// 조작 방식 (TOUCH TYPE, player request 2026-08-16; a third choice 2026-09-22). One
/// token: `float` = 자유 스틱 (the shipped default — touch anywhere, the base spawns
/// under the finger), `dpad` = 십자키 (the four arrows pinned bottom-left, no stick),
/// `pinned` = 고정 스틱 (the stick with its base nailed to that same seat). See
/// [`crate::StickMode`].
///
/// ⛔ `fixed` is the LEGACY spelling of 십자키 — every device that chose the arrows
/// before 2026-09-22 has that word on disk, and reading it as anything else would
/// silently swap those players' control. It is still accepted, never written.
/// (The name was the first "B" talking: it began as a pinned stick.)
///
/// Its own file rather than a second value in `wie_control_style.txt`: that one
/// selects D-pad vs joystick for the NON-immersive layouts, which is a different
/// question, and folding two settings into one line would make a partial write lose
/// both.
pub fn read_stick_mode(data_dir: &Path) -> crate::StickMode {
    match std::fs::read_to_string(touch_type_path(data_dir)) {
        Ok(s) => parse_stick_mode(&s),
        Err(_) => crate::StickMode::Float,
    }
}

/// The token → mode rule on its own, so the legacy spelling is pinned by a test.
/// Missing / unknown = 자유 스틱.
pub fn parse_stick_mode(s: &str) -> crate::StickMode {
    match s.trim() {
        "dpad" | "fixed" => crate::StickMode::Dpad,
        "pinned" => crate::StickMode::Pinned,
        _ => crate::StickMode::Float,
    }
}

pub fn write_stick_mode(data_dir: &Path, mode: crate::StickMode) {
    let value = match mode {
        crate::StickMode::Float => "float",
        crate::StickMode::Dpad => "dpad",
        crate::StickMode::Pinned => "pinned",
    };
    if let Err(e) = std::fs::write(touch_type_path(data_dir), value) {
        log::warn!("failed to persist touch type: {e}");
    }
}

fn buffs_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_buffs.txt")
}

/// Host-owned cash-shop buff expiries, one absolute UNIX-ms stamp per line:
/// `exp_x2 <ms>` / `exp_x4 <ms>` / `relief <ms>`.
///
/// ★ These are OURS, not the game's — the three items they come from have no
/// client-side effect at all (see [`crate::buffs`]), and the game's save file is
/// encrypted, so there is nowhere in it to put them. Losing this file loses the
/// remaining time; that is the accepted trade.
///
/// A damaged or partly written file degrades to "no buff" per line rather than
/// failing the read: an unparsable stamp must not hand out a free week, and it must
/// not stop the app either.
pub fn read_buffs(data_dir: &Path) -> crate::buffs::Buffs {
    let mut b = crate::buffs::Buffs::default();
    let Ok(text) = std::fs::read_to_string(buffs_path(data_dir)) else {
        return b;
    };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        let Ok(ms) = val.parse::<u64>() else {
            continue;
        };
        match key {
            "exp_x2" => b.exp_x2_until = ms,
            "exp_x4" => b.exp_x4_until = ms,
            "relief" => b.relief_until = ms,
            _ => {}
        }
    }
    b
}

pub fn write_buffs(data_dir: &Path, b: &crate::buffs::Buffs) {
    let text = format!("exp_x2 {}\nexp_x4 {}\nrelief {}\n", b.exp_x2_until, b.exp_x4_until, b.relief_until);
    if let Err(e) = std::fs::write(buffs_path(data_dir), text) {
        log::warn!("failed to persist cash buffs: {e}");
    }
}

fn levelups_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_levelups.txt")
}

/// Pending 천상의 영약 level-ups, a bare decimal count. Persisted for the same
/// reason the buffs are: the game consumes the item on OK, but the grant fires
/// only once the player is back on the field — an app death in between must not
/// eat a purchase. A damaged file degrades to 0 (no free level-ups), same policy
/// as [`read_buffs`].
pub fn read_pending_levelups(data_dir: &Path) -> u32 {
    std::fs::read_to_string(levelups_path(data_dir))
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

pub fn write_pending_levelups(data_dir: &Path, n: u32) {
    if let Err(e) = std::fs::write(levelups_path(data_dir), format!("{n}\n")) {
        log::warn!("failed to persist pending level-ups: {e}");
    }
}

fn cash_baseline_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_cash_uncommitted.txt")
}

/// Note down the host state that the last completed save already accounts for, so
/// an app death before the NEXT one can be undone. See [`crate::buffs::CashBaseline`]
/// for why this exists at all.
///
/// A note that is already there is left alone: the first uncommitted use is the one
/// that defines the baseline, and overwriting it on the second use would silently
/// commit the first.
pub fn arm_cash_baseline(data_dir: &Path, buffs: &crate::buffs::Buffs, levelups: u32) {
    let path = cash_baseline_path(data_dir);
    if path.exists() {
        return;
    }
    let text = crate::buffs::serialise_baseline(&crate::buffs::CashBaseline { buffs: *buffs, levelups });
    if let Err(e) = std::fs::write(&path, text) {
        // Not fatal: the effect and the retry still happen, we just lose the ability
        // to undo them. Worth a warning because it re-opens the duplication window.
        log::warn!("failed to arm the cash rollback note: {e}");
    }
}

/// Read the rollback note without consuming it. `None` = nothing pending, or a note
/// too damaged to trust (see [`crate::buffs::parse_baseline`] for that policy).
pub fn read_cash_baseline(data_dir: &Path) -> Option<crate::buffs::CashBaseline> {
    let text = std::fs::read_to_string(cash_baseline_path(data_dir)).ok()?;
    let parsed = crate::buffs::parse_baseline(&text);
    if parsed.is_none() {
        log::warn!("cash rollback note is unreadable — leaving host state as it is");
    }
    parsed
}

/// Drop the note: the guest save on disk now includes the consumption, so there is
/// nothing left to roll back to.
pub fn clear_cash_baseline(data_dir: &Path) {
    match std::fs::remove_file(cash_baseline_path(data_dir)) {
        Ok(()) => log::info!("cash rollback note cleared — the save committed the consumption"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("failed to clear the cash rollback note: {e}"),
    }
}

fn cooldowns_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_cooldowns.txt")
}

/// Skill-detail "쿨타임:" is the u8 at in-memory rec+0x1c (loader `0x57924`,
/// painter `0x44246`). File stats byte 4 of `res/skill/s{4..10}_dat`. Combat
/// still does not enforce it (SP + cast delay only). `0` = no overlay.
///
/// The guest prints that byte as-is. Host fill is the byte in **100 ms** ticks
/// so 80 → 8 s (the 27차 file-format example was `8000`). `wie_cooldowns.txt`
/// still overlays, including `0` to turn one off.
const SKILL_CD_TICK_MS: u64 = 100;

/// `(atlas sheet 0..6, strip cell 0..8, ms)` — same key as `items::skill_icon`.
/// HUD cells: rec 0 = cell 0 (basic), recs 7..14 = cells 1..8. Combo-stage
/// rows (창술2..7) are not cells. Only positive lengths are listed.
const BUILTIN_SKILL_COOLDOWNS: &[(u8, u8, u64)] = &[
    // s4 창술
    (0, 2, 80 * SKILL_CD_TICK_MS),  // 자격
    (0, 3, 120 * SKILL_CD_TICK_MS), // 압도
    (0, 4, 120 * SKILL_CD_TICK_MS), // 유도
    (0, 5, 10 * SKILL_CD_TICK_MS),  // 장벽
    // s5 검술
    (1, 1, 120 * SKILL_CD_TICK_MS), // 선풍
    (1, 2, 80 * SKILL_CD_TICK_MS),  // 양단
    (1, 3, 40 * SKILL_CD_TICK_MS),  // 질풍
    (1, 4, 10 * SKILL_CD_TICK_MS),  // 결의
    (1, 5, 10 * SKILL_CD_TICK_MS),  // 혼신
    // s6 단도
    (2, 1, 80 * SKILL_CD_TICK_MS),  // 참혼
    (2, 2, 40 * SKILL_CD_TICK_MS),  // 암영
    (2, 3, 120 * SKILL_CD_TICK_MS), // 난무
    (2, 4, 10 * SKILL_CD_TICK_MS),  // 육감
    (2, 5, 10 * SKILL_CD_TICK_MS),  // 은신
    // s7 사격
    (3, 1, 40 * SKILL_CD_TICK_MS),  // 연사
    (3, 2, 80 * SKILL_CD_TICK_MS),  // 난사
    (3, 3, 40 * SKILL_CD_TICK_MS),  // 곡예
    (3, 4, 120 * SKILL_CD_TICK_MS), // 저격
    (3, 5, 10 * SKILL_CD_TICK_MS),  // 장탄
    // s8 격발
    (4, 1, 120 * SKILL_CD_TICK_MS), // 직격
    (4, 2, 120 * SKILL_CD_TICK_MS), // 연쇄
    (4, 3, 32 * SKILL_CD_TICK_MS),  // 위협
    (4, 4, 144 * SKILL_CD_TICK_MS), // 지축
    (4, 5, 10 * SKILL_CD_TICK_MS),  // 일심
    // s9 영탄
    (5, 1, 40 * SKILL_CD_TICK_MS),  // 암흑
    (5, 2, 80 * SKILL_CD_TICK_MS),  // 업화
    (5, 3, 104 * SKILL_CD_TICK_MS), // 나락
    (5, 4, 120 * SKILL_CD_TICK_MS), // 망각
    (5, 5, 120 * SKILL_CD_TICK_MS), // 전율
    // s10 광아
    (6, 1, 80 * SKILL_CD_TICK_MS),  // 파동
    (6, 2, 120 * SKILL_CD_TICK_MS), // 격광
    (6, 3, 200 * SKILL_CD_TICK_MS), // 회복
    (6, 4, 40 * SKILL_CD_TICK_MS),  // 초월
    (6, 5, 40 * SKILL_CD_TICK_MS),  // 광휘
];

fn builtin_skill_cooldowns() -> std::collections::HashMap<(u8, u8), u64> {
    BUILTIN_SKILL_COOLDOWNS
        .iter()
        .filter(|&&(_, _, ms)| ms > 0)
        .map(|&(sheet, cell, ms)| ((sheet, cell), ms))
        .collect()
}

/// Apply `wie_cooldowns.txt` lines onto `out`. Returns how many lines stuck.
///
/// ```text
/// # sheet cell ms        ("#" starts a comment)
/// s4 0 0                 # sheet as the game's own id (s4..s10), cell 0..8
/// 2  3 8000              # or sheet as the atlas index (0..6 = s4..s10)
/// ```
///
/// `ms` = that skill's cooldown; 0 = none (and is stored, so a builtin can be
/// turned off). A damaged line is skipped, never fatal.
fn apply_cooldown_lines(out: &mut std::collections::HashMap<(u8, u8), u64>, text: &str) -> usize {
    let mut n = 0;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let (Some(sheet), Some(cell), Some(ms)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        // `s4`..`s10` (the game's sheet id) or a bare atlas index 0..6.
        let sheet: u8 = match sheet.strip_prefix(['s', 'S']) {
            Some(id) => match id.parse::<u8>() {
                Ok(id @ 4..=10) => id - 4,
                _ => continue,
            },
            None => match sheet.parse::<u8>() {
                Ok(a @ 0..=6) => a,
                _ => continue,
            },
        };
        let (Ok(cell), Ok(ms)) = (cell.parse::<u8>(), ms.parse::<u64>()) else {
            continue;
        };
        if cell > 8 {
            continue;
        }
        out.insert((sheet, cell), ms);
        n += 1;
    }
    n
}

/// Per-skill cooldown lengths. Builtin table first, then `wie_cooldowns.txt`.
/// A skill not in either, or listed as `0`, has no overlay.
pub fn read_skill_cooldowns(data_dir: &Path) -> std::collections::HashMap<(u8, u8), u64> {
    let mut out = builtin_skill_cooldowns();
    match std::fs::read_to_string(cooldowns_path(data_dir)) {
        Ok(text) => {
            let n = apply_cooldown_lines(&mut out, &text);
            out.retain(|_, ms| *ms > 0);
            let live = out.len();
            log::info!("skill cd: {n} line(s) from wie_cooldowns.txt, {live} skill(s) have a cooldown");
        }
        Err(_) => {
            let live = out.len();
            log::info!("skill cd: wie_cooldowns.txt missing — {live} builtin skill(s) have a cooldown");
        }
    }
    out
}

/// Worn-gear cooldown reduction is a **percent** (`addStat(0x11)` total).
/// Combat never used it; the host overlay is the first consumer.
///
/// 100 % and above would erase a skill that still has a base length, so the
/// cut is capped at 99 % and a positive base never becomes 0.
pub fn apply_cd_reduce(base_ms: u64, pct: u16) -> u64 {
    if base_ms == 0 {
        return 0;
    }
    let keep = 100u64.saturating_sub(u64::from(pct.min(99)));
    (base_ms * keep / 100).max(1)
}

fn immersive_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_immersive.txt")
}

/// Landscape **immersive** mode (default ON): give the guest an aspect-matched
/// wide framebuffer and let the game frame fill the whole screen, with the
/// controls as translucent overlays instead of side columns.
///
/// Off = the classic layout (portrait-shaped game centred, control columns at
/// the sides). Only takes effect at app start — the WIPI screen framebuffer is
/// allocated once and its pointer is cached by the guest.
pub fn read_immersive(data_dir: &Path) -> bool {
    match std::fs::read_to_string(immersive_path(data_dir)) {
        Ok(s) => s.trim() != "off", // default (missing/other) = on
        Err(_) => true,
    }
}

pub fn write_immersive(data_dir: &Path, on: bool) {
    let value = if on { "on" } else { "off" };
    if let Err(e) = std::fs::write(immersive_path(data_dir), value) {
        log::warn!("failed to persist immersive pref: {e}");
    }
}

/// Cheap-synth switch (default OFF = today's sound, unchanged).
///
/// ★ 2026-08-09. The MIDI synth's fixed per-sample cost is paid on the AAudio callback
/// thread whether or not a note is playing: `oxisynth`'s `Core::one_block` runs reverb
/// and chorus with **no voice-count guard**, so a silent menu still renders 16 comb +
/// 8 allpass filters and three 5-tap chorus blocks at 44,100 Hz. Measured on the
/// developer's own S21 at the title screen: the `AudioTrack` thread sits at 13.3 % of a
/// core. On a slower device that is a callback that can miss its deadline, and a missed
/// deadline is the crackle the player hears.
///
/// This is NOT on by default, and that is the point: turning reverb and chorus off
/// CHANGES HOW THE GAME SOUNDS, and the standing rule for this round is that nothing
/// regresses on the phone that already works. So the default build is bit-identical to
/// today, and this file is the one-command escape hatch for a device that cannot keep
/// up — flip it, restart, and compare `AudioTrack` in `top -H`.
///
/// Read ONCE, when the synth is built (`audio_sink::build_synth`), so a change needs an
/// app restart.
/// Effect-bus level, as an index into [`crate::audio_mix::SFX_LEVELS`].
///
/// ★ 2026-08-27 (Fold 6: 효과음이 너무 작다). See `audio_mix::SFX_LEVELS` for why this
/// is a setting rather than a fourth guess at a global constant. Missing / unreadable /
/// out of range all mean the shipped default, so a fresh install and a corrupt file
/// sound the same as every build before this one.
pub fn read_sfx_level(data_dir: &Path) -> u8 {
    match std::fs::read_to_string(data_dir.join("wie_sfx_level.txt")) {
        Ok(s) => s
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|v| usize::from(*v) < crate::audio_mix::SFX_LEVELS.len()),
        Err(_) => None,
    }
    .unwrap_or(crate::audio_mix::SFX_LEVEL_DEFAULT)
}

pub fn write_sfx_level(data_dir: &Path, level: u8) {
    let path = data_dir.join("wie_sfx_level.txt");
    if let Err(e) = std::fs::write(&path, level.to_string()) {
        log::warn!("config: cannot write {path:?}: {e}");
    }
}

fn quick_hidden_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_quick_hidden.txt")
}

/// Which quickbar PLATES are switched off (player request 2026-09-10: "스킬 퀵슬롯을
/// 선택 후 비활성화 … 퀵슬롯이 너무많아서 불편하다는 유저도있어서").
///
/// One `hide <n> <n> …` line of 1-based plate numbers, parsed by
/// [`crate::quickpage::parse_hidden`] — which also sanitises, so a file from another
/// build or a half-written one degrades to "nothing hidden" and never to a field with
/// no quickbar on it. Missing file = nothing hidden = every build before this one.
pub fn read_quick_hidden(data_dir: &Path) -> u8 {
    match std::fs::read_to_string(quick_hidden_path(data_dir)) {
        Ok(s) => crate::quickpage::parse_hidden(&s),
        Err(_) => crate::quickpage::SHOW_ALL,
    }
}

pub fn write_quick_hidden(data_dir: &Path, mask: u8) {
    let path = quick_hidden_path(data_dir);
    if let Err(e) = std::fs::write(&path, crate::quickpage::serialize_hidden(mask)) {
        log::warn!("config: cannot write {path:?}: {e}");
    }
}

fn slot_first_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_save_first.txt")
}

/// Where the save-slot WINDOW was left (130차): `first <n>`, parsed and clamped by
/// [`crate::slotwindow::parse_first`]. Missing or unreadable = window 0 = the three
/// original slots, which is every build before this one.
pub fn read_slot_first(data_dir: &Path) -> u8 {
    match std::fs::read_to_string(slot_first_path(data_dir)) {
        Ok(s) => crate::slotwindow::parse_first(&s),
        Err(_) => 0,
    }
}

pub fn write_slot_first(data_dir: &Path, first: u8) {
    let path = slot_first_path(data_dir);
    if let Err(e) = std::fs::write(&path, crate::slotwindow::serialize_first(first)) {
        log::warn!("config: cannot write {path:?}: {e}");
    }
}

fn slot_last_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_save_last.txt")
}

/// The real slot the player LAST LOADED (161차): `last <n>`, parsed by
/// [`crate::slotwindow::parse_last`]. Missing or unreadable = `None` = the slot lists open
/// where the game itself puts them (row 0), which is every build before this one.
pub fn read_slot_last(data_dir: &Path) -> Option<u8> {
    std::fs::read_to_string(slot_last_path(data_dir))
        .ok()
        .and_then(|s| crate::slotwindow::parse_last(&s))
}

pub fn write_slot_last(data_dir: &Path, slot: u8) {
    let path = slot_last_path(data_dir);
    if let Err(e) = std::fs::write(&path, crate::slotwindow::serialize_last(slot)) {
        log::warn!("config: cannot write {path:?}: {e}");
    }
}

pub fn read_audio_lite(data_dir: &Path) -> bool {
    // Same two-location search as `read_blit_hle`, and for the same reason: a RELEASE
    // build's internal data dir is unreachable over adb, and a debug build would be
    // measuring a different binary.
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_audio_lite.txt"),
        data_dir.join("wie_audio_lite.txt"),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let on = s.trim() == "on";
            log::warn!("audio lite switch read from {path:?}: {}", if on { "on" } else { "off" });
            return on;
        }
    }
    false
}

/// Guest PC profiler (default OFF).
///
/// ★ 2026-08-09. The one lever that actually moves the framerate on a slow device is
/// removing guest INSTRUCTIONS, and 26차 proved the method: find the hottest guest
/// function, reimplement it on the host, gate it, verify pixel-identical output. That
/// single patch (`blit565`, guest 0x110fe4) took 36 % of all guest instructions away.
///
/// This turns the sampler on so the NEXT such function can be named instead of guessed.
/// It costs a stack walk per sample, so it is not something to ship enabled.
pub fn read_guest_profile(data_dir: &Path) -> bool {
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_profile.txt"),
        data_dir.join("wie_profile.txt"),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let on = s.trim() == "on";
            log::warn!("guest profiler switch read from {path:?}: {}", if on { "on" } else { "off" });
            return on;
        }
    }
    false
}

fn heat_saver_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_heat_saver.txt")
}

fn pad_hud_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_pad_hud.txt")
}

/// 게임패드 전용 HUD (59차, default AUTO). `auto` = the last input source decides,
/// `on` = always, `off` = never. Same two-location search as the switches above so a
/// release build can be flipped over adb.
///
/// Unlike HEAT SAVER this applies AT ONCE: the HUD is rebuilt from
/// `crate::pad_hud_active()` every relayout, so there is nothing to restart.
pub fn read_pad_hud(data_dir: &Path) -> crate::padhud::PadHud {
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_pad_hud.txt"),
        pad_hud_path(data_dir),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            return crate::padhud::PadHud::parse(&s);
        }
    }
    crate::padhud::PadHud::default()
}

pub fn write_pad_hud(data_dir: &Path, p: crate::padhud::PadHud) {
    if let Err(e) = std::fs::write(pad_hud_path(data_dir), p.as_token()) {
        log::warn!("failed to persist pad hud pref: {e}");
    }
}

/// 발열 절감 모드 (57차, default OFF). One switch, three effects, all fixed at
/// process start:
///
///  * the UI renders at HALF resolution and `ANativeWindow_setBuffersGeometry`
///    has the compositor upscale in hardware — every pixel-proportional cost on
///    the UI thread (clear, game blit, overlay draws, the present copy) drops 4x;
///  * the MIDI synth runs in its lite preset (see [`read_audio_lite`] — reverb/
///    chorus off), which is the documented escape hatch for exactly this device
///    class and worth ~13 % of a core even in silence;
///  * (nothing changes in the emulator worker: game speed is timer-driven and a
///    slower render never slows the game).
///
/// Default OFF because the half-resolution upscale visibly softens the overlay a
/// little and the standing rule is that nothing regresses on the phone that
/// already works — the player flips it per device from the settings modal
/// (HEAT SAVER row). Same two-location search as [`read_blit_hle`] so a release
/// build can be flipped over adb too.
pub fn read_heat_saver(data_dir: &Path) -> bool {
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_heat_saver.txt"),
        heat_saver_path(data_dir),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            return s.trim() == "on";
        }
    }
    false
}

pub fn write_heat_saver(data_dir: &Path, on: bool) {
    let value = if on { "on" } else { "off" };
    if let Err(e) = std::fs::write(heat_saver_path(data_dir), value) {
        log::warn!("failed to persist heat saver pref: {e}");
    }
}

/// Gamepad button overrides (58차, `wie_gamepad.txt`; default = empty, i.e. the
/// built-in layout in `gamepad::default_button_action`). One line per button:
/// `<android keycode>=<TOKEN>` where TOKEN is `UP DOWN LEFT RIGHT OK CLR 0`..`9 #
/// * PAGE NONE` — e.g. `96=OK`, `107=PAGE`, `97=NONE`. Read once at App start;
/// garbage lines are skipped (a typo must never take the pad down with it). Same
/// two-location search as [`read_blit_hle`] so a release build can be remapped
/// over adb without a rebuild.
pub fn read_gamepad_map(data_dir: &Path) -> std::collections::HashMap<i32, Option<crate::gamepad::PadAction>> {
    let mut map = std::collections::HashMap::new();
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_gamepad.txt"),
        data_dir.join("wie_gamepad.txt"),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            for line in s.lines() {
                if let Some((code, action)) = crate::gamepad::parse_map_line(line) {
                    map.insert(code, action);
                }
            }
            log::info!("gamepad map read from {path:?}: {} override(s)", map.len());
            break;
        }
    }
    map
}

/// Persist the pad screen's table (2026-10-09) — the first WRITER `wie_gamepad.txt` has.
///
/// ★ It writes the file [`read_gamepad_map`] will READ: the external copy when one exists
/// (an adb-pushed remap), the internal one otherwise. Writing the internal file while an
/// external one exists would be shadowed by it at the next start, and the player's change
/// would silently come undone.
///
/// Atomic (temp + rename): a crash mid-write leaves the old file, never half a table —
/// half a table is a pad with buttons that quietly went back to their defaults.
pub fn write_gamepad_map(data_dir: &Path, text: &str) -> bool {
    let external = PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_gamepad.txt");
    let path = if external.exists() { external } else { data_dir.join("wie_gamepad.txt") };
    match crate::database::write_atomic(&path, text.as_bytes()) {
        Ok(()) => {
            log::info!("gamepad map written to {path:?} ({} line(s))", text.lines().filter(|l| !l.starts_with('#')).count());
            true
        }
        Err(e) => {
            log::warn!("failed to persist the gamepad map to {path:?}: {e}");
            false
        }
    }
}

fn blit_hle_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_blit_hle.txt")
}

/// Host sprite blitter (default ON). Writing `off` here makes the emulator hand every
/// blit back to the interpreted guest code — the exact behaviour before 26차.
///
/// ★ This exists for MEASUREMENT, not for players. A device A/B of the biggest
/// optimisation in the emulator is otherwise impossible: the only alternative is
/// installing the previous APK, and that is a downgrade, which means uninstall, which
/// means the save is gone. With this file the comparison is the same phone, the same
/// save, the same map, one restart apart. It is read once at worker start because the
/// binary patch is installed during boot.
pub fn read_blit_hle(data_dir: &Path) -> bool {
    // The internal data dir is unreachable over adb for a RELEASE build (`run-as` refuses a
    // non-debuggable package, and a debug build would measure a different binary), so the
    // app's own external files dir is checked first. `adb shell` is exempt from scoped
    // storage for `/sdcard/Android/data`, which makes this the only way to flip the switch
    // on the build that actually ships.
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_blit_hle.txt"),
        blit_hle_path(data_dir),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let on = s.trim() != "off"; // default (missing/other) = on
            log::warn!("blit HLE switch read from {path:?}: {}", if on { "on" } else { "off" });

            return on;
        }
    }

    true
}

fn frame_timer_cap_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_frame_timer_cap.txt")
}

/// The default ceiling on the guest's own frame timer, in milliseconds.
///
/// 50 is not a tuning knob picked by feel — it is the interval Hero3 itself asks for on
/// the field (`emu frame timing: min 50 ms`). Capping at the game's own fastest request
/// means the modal screens are woken no more often than the game already wakes itself
/// elsewhere, so nothing runs at a rate this build has never run at.
pub const FRAME_TIMER_CAP_DEFAULT_MS: u64 = 50;

/// Ceiling on `MC_knlSetTimer` (default 50 ms; `off`/`0` = honour the guest exactly).
///
/// ★ Why this switch exists. 37차 removed the host's two 16 ms roundings and the game
/// went 22 → 69 fps on the title. What survived was the game's OWN number: in the
/// modal/SLOT states — network warehouse, cash shop — it asks to be woken every
/// **244 ms**, which is 4 fps by design. 2026-08-14 device measurement showed nothing
/// else is left to blame there (page turn = one round trip, 24~49 ms; `recv polls/s` =
/// 0), so the only way further is to wake the guest earlier than it asked.
///
/// That is a behaviour change, not an optimisation, which is why it is a runtime switch
/// with the same external-files-dir escape hatch as [`read_blit_hle`]: a guest loop that
/// counts frame ticks instead of wall-clock now counts them ~5x faster in those screens.
/// Flip it off and restart to compare on the same phone and the same save:
/// `adb shell "echo off > /sdcard/Android/data/com.hero3remake.app/files/wie_frame_timer_cap.txt"`
pub fn read_frame_timer_cap(data_dir: &Path) -> u64 {
    for path in [
        PathBuf::from("/sdcard/Android/data/com.hero3remake.app/files/wie_frame_timer_cap.txt"),
        frame_timer_cap_path(data_dir),
    ] {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let s = s.trim();
            let ms = if s == "off" {
                0
            } else {
                s.parse::<u64>().unwrap_or(FRAME_TIMER_CAP_DEFAULT_MS)
            };
            log::warn!("frame timer cap read from {path:?}: {ms} ms (0 = off)");

            return ms;
        }
    }

    FRAME_TIMER_CAP_DEFAULT_MS
}

fn base_h_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_base_h.txt")
}

/// Immersive-mode logical content height. Smaller = more of the screen is
/// guaranteed to be filled by the map (Hero3 maps are 176..1120 px wide, median
/// 512) but chunkier pixels; larger = finer pixels but black side margins on
/// narrow maps. Cycled from the settings menu; applies at next start.
///
/// Retuned 2026-07-30: the derived width is now capped at
/// [`crate::POPUP_SAFE_LOGICAL_W`] (496) because the game's popup renderer wraps
/// its interior above 255 px. The old top choices (280/320) all clamped to that
/// same width on an ultra-wide phone, so the knob did nothing above 240; these
/// four stay distinct up to ~2.25:1. A previously persisted value outside the list
/// falls back to [`crate::LANDSCAPE_BASE_H`].
pub const BASE_H_CHOICES: [u32; 4] = [160, 190, 220, 240];

pub fn read_base_h(data_dir: &Path) -> u32 {
    let v = std::fs::read_to_string(base_h_path(data_dir))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(crate::LANDSCAPE_BASE_H);
    if BASE_H_CHOICES.contains(&v) { v } else { crate::LANDSCAPE_BASE_H }
}

pub fn write_base_h(data_dir: &Path, h: u32) {
    if let Err(e) = std::fs::write(base_h_path(data_dir), h.to_string()) {
        log::warn!("failed to persist base height: {e}");
    }
}

fn layout_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_layout.txt")
}

/// The player's custom control layout, one control per line:
///
/// ```text
/// # mode id centre-x centre-y size-percent
/// field ok 0.9120 0.8083 130
/// menu key1 0.4000 0.6000 100
/// ```
///
/// Position is a FRACTION of the window and size a PERCENT of the built-in size —
/// never pixels (a layout authored on one device would be nonsense on another, and
/// never guest px, which are frozen at boot). Parsing is per-line defensive: a line
/// that does not parse is dropped and everything else still loads, the same
/// discipline [`read_base_h`] uses for a single value.
pub fn read_layout(data_dir: &Path) -> crate::placement::Table {
    let mut table = crate::placement::Table::default();
    let Ok(text) = std::fs::read_to_string(layout_path(data_dir)) else {
        return table;
    };
    let (mut ok, mut bad, mut stale, mut folded) = (0u32, 0u32, 0u32, 0u32);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // ★ A `menu ...` line is SUPERSEDED, not corrupt: it belongs to the second seat
        // table, removed 2026-09-10 (`placement::Table`). Counted apart so the log says
        // "your old menu arrangement was dropped on purpose" instead of accusing the file
        // of being damaged - the menu controls follow the field arrangement now, and the
        // next write simply leaves these lines out.
        if line.starts_with(crate::placement::LEGACY_MENU_PREFIX) {
            stale += 1;
            continue;
        }
        // ★ 91차: a `field dpad …` line is the D-pad control that was folded into the
        // stick. It still parses (as the stick's seat) - counted so the log says the old
        // arrows' seat was ADOPTED, and so that "where did my dpad line go" has an answer.
        // It is written back as a `stick` line, so this fires once per migration.
        if crate::placement::is_legacy_dpad_line(line) {
            folded += 1;
        }
        match crate::placement::parse_line(line) {
            Some((id, place)) => {
                table.set(id, Some(place));
                ok += 1;
            }
            None => bad += 1,
        }
    }
    if bad > 0 {
        log::warn!("layout: {bad} unreadable line(s) dropped, {ok} loaded");
    }
    if stale > 0 {
        log::warn!("layout: {stale} superseded menu-mode line(s) dropped - one layout now, {ok} loaded");
    }
    if folded > 0 {
        log::warn!("layout: {folded} dpad line(s) adopted as the stick's seat - the stick and the arrows are one control now");
    }
    table
}

pub fn write_layout(data_dir: &Path, table: &crate::placement::Table) {
    let mut out = String::from("# WIE custom control layout — field id cx cy pct\n");
    out.push_str(&table.to_text());
    if let Err(e) = std::fs::write(layout_path(data_dir), out) {
        log::warn!("failed to persist the control layout: {e}");
    }
}

// ⛔⛔ THE PRIVATE-SERVER CONFIGURATION WAS HERE AND IS GONE (99차, 2026-09-16 — 100 %
// offline, user decision): `wie_server.txt` / `read_server_endpoint` /
// `build_time_endpoint` (`HERO3_SERVER_HOST` / `_PORT` env), `endpoint_report`,
// `wie_server_pin.txt` / `read_server_pin` (`HERO3_SERVER_PIN`), and `wie_account_id` /
// `read_or_create_account_id` / `account_id_hex` / `random_id` (the 16-byte bearer
// credential the login frame carried). `endpoint.rs`, `tls.rs`, `tls_buf.rs` and
// `hero3_link.rs` went with them. A device that still carries any of those files simply
// has files nothing reads; nothing is migrated because nothing is needed.

// ── offline warehouse / cash shop (60차) ─────────────────────────────────────

// ⛔ `NetMode` (AUTO / SERVER / OFFLINE, 60차) and `wie_net_mode.txt` WERE HERE and are
// gone (99차). There is exactly one mode now, and it needs no name: every warehouse and
// cash-shop session is the in-app engine (`network.rs`). The adb flip
// (`echo offline > …/wie_net_mode.txt`) is therefore also gone — there is nothing to flip.

/// The offline warehouse's contents. Plain text (see `offline::Store`), so a player
/// who loses items can be asked to paste it.
pub fn offline_store_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_offline_wh.txt")
}

pub fn read_offline_store(data_dir: &Path) -> crate::offline::Store {
    match std::fs::read_to_string(offline_store_path(data_dir)) {
        Ok(s) => crate::offline::Store::decode(&s),
        Err(_) => crate::offline::Store::new(),
    }
}

// ── 하늘성 / THE TOWER: three files ──────────────────────────────────────────
//
// The floor TABLE is BAKED INTO THE BINARY (122차) and an external copy is only an
// override; the CONTROL file lives on external storage so it can be authored with
// `adb` on a release build (same reason as `WH_PURGE_TRIGGER`); the PROGRESS file is
// the player's and lives in the internal data dir with the buffs.

/// ★★★ THE FLOOR TABLE, BUILT IN (122차, 2026-09-21). The same generated file the
/// selftest reads with `include_str!` (`guardian_selftest.rs`), so the bytes the `.so`
/// carries are the bytes `gen_guardian_floors.py --check` verified.
///
/// Why it moved into the binary: until 122차 the table was ONLY ever read from
/// [`GUARDIAN_FLOORS_FILE`], a file that had to be `adb push`ed after every install —
/// it is not in the APK. So every device that received nothing but the APK (a
/// hand-out, or a reinstall after an uninstall, which deletes that directory) had a
/// tower with no floors: the entrance NPC's hard-mode dialogue still ran, 「예」 still
/// set ENTER, and the run was refused in a log line nobody saw
/// (`refusing floor 1 — the floor table has 0 row(s)`). Reported 2026-09-21 as
/// "하드코어 모드 하늘성 입장 npc한테서 「예」를 눌러도 아무일도 안일어난다".
///
/// ⛔ The file is CRLF in a Windows checkout; `guardian::parse_table` trims every line,
///    so the line ending is not a format. The LF digest is what the device copies had.
pub const GUARDIAN_FLOORS_BUILTIN: &str = include_str!("../../wie_guardian_floors.txt");

/// An OVERRIDE for the floor table, if someone supplies one over `adb`.
///
/// ⛔ It is an override, not the source: with no file (the normal case since 122차) the
/// built-in table above is used. A file that is there but parses to ZERO rows is
/// treated as absent too — a broken override must not silence the tower the way the
/// missing file used to. ⚠ A STALE override wins over a newer built-in table (there is
/// no version mark in the format), so a device that still carries an old pushed copy
/// keeps that table until the file is deleted. Format and parser: `guardian::parse_table`.
pub const GUARDIAN_FLOORS_FILE: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_guardian_floors.txt";

/// Where a run is asked for, over `adb`. Read once and DELETED, like
/// [`REAVER_TEST_TRIGGER`], so a forgotten file cannot start a run on every launch.
pub const GUARDIAN_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_guardian.txt";

/// The PRE-138차 device-wide progress file (one `best N` for every save slot). Only read
/// to say, once, that it is ignored — see [`note_legacy_guardian_progress`].
fn legacy_guardian_progress_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_guardian_progress.txt")
}

/// ★ 138차: a SAVE SLOT's tower progress — beside the bag boxes (`<app root>/bagbox/`),
/// for the same reason: a host file keyed like a record store that must NOT live inside
/// one (a file under `db/GameSave_11_N/` would be vaulted as a save generation).
fn tower_progress_file(app_root: &Path, slot: u8) -> PathBuf {
    app_root.join("guardian").join(format!("slot{slot}.txt"))
}

/// The floor table: the external override if it exists and has at least one row,
/// otherwise the built-in [`GUARDIAN_FLOORS_BUILTIN`]. Bad lines are dropped and
/// REPORTED — the buff file's contract, and the reason `guardian::parse_table` returns
/// line numbers. ★ The boot line names the SOURCE (`from <path>` / `built in`), which
/// is the one thing a "예 does nothing" report needs read off the device.
pub fn read_guardian_floors() -> Vec<crate::guardian::Floor> {
    if let Ok(text) = std::fs::read_to_string(GUARDIAN_FLOORS_FILE) {
        let (rows, bad) = crate::guardian::parse_table(&text);
        if !bad.is_empty() {
            log::warn!("guardian: {GUARDIAN_FLOORS_FILE} line(s) {bad:?} did not parse and were skipped");
        }
        if rows.is_empty() {
            log::warn!("guardian: the override {GUARDIAN_FLOORS_FILE} has no usable row — ignoring it and using the built-in table");
        } else {
            log::warn!(
                "guardian: {} floor(s) loaded from {GUARDIAN_FLOORS_FILE} (an override of the built-in table)",
                rows.len()
            );
            return rows;
        }
    }
    let (rows, bad) = crate::guardian::parse_table(GUARDIAN_FLOORS_BUILTIN);
    if !bad.is_empty() {
        // Cannot happen for a table the selftest parses, but the contract is the same.
        log::warn!("guardian: built-in table line(s) {bad:?} did not parse and were skipped");
    }
    log::warn!(
        "guardian: {} floor(s) loaded (built in; {GUARDIAN_FLOORS_FILE} would override)",
        rows.len()
    );
    rows
}

/// Read a control file and delete it, so one file is one request.
pub fn read_guardian_command() -> Option<crate::guardian::Command> {
    let text = std::fs::read_to_string(GUARDIAN_TRIGGER).ok()?;
    let _ = std::fs::remove_file(GUARDIAN_TRIGGER);
    // ⛔ Before the command, and independent of it: `hard 1` on its own is a valid file
    // (it arms the entrance dialogue on a normal save for testing and asks for no run).
    let hard = crate::guardian::parse_hard_override(&text);
    if let Some(on) = hard {
        crate::guardian::set_hard_override(on);
        log::warn!("guardian: {GUARDIAN_TRIGGER} sets the hard-mode mirror to {on} (testing knob)");
    }
    let cmd = crate::guardian::parse_command(&text);
    match cmd {
        Some(c) => log::warn!("guardian: {GUARDIAN_TRIGGER} asks for {c:?}; file deleted"),
        None if hard.is_none() => {
            log::warn!("guardian: {GUARDIAN_TRIGGER} set nothing (expected `floor N`, optional `force 1`, `hard 1`, or `leave`); ignored")
        }
        None => {}
    }
    cmd
}

/// ★ 138차 (결정 ⑤): the old device-wide progress is IGNORED — every slot starts at 0,
/// because "reached floor N once, on some slot" is not "this character cleared boss N".
/// Said once at boot when the file is there; the file itself is left alone.
pub fn note_legacy_guardian_progress(data_dir: &Path) {
    if let Ok(text) = std::fs::read_to_string(legacy_guardian_progress_path(data_dir)) {
        let old = crate::guardian::parse_progress(&text);
        log::warn!(
            "guardian: the device-wide progress file (best {}) is ignored since 138차 — progress is per save slot now, and every slot starts at 0",
            old.best
        );
    }
}

/// One save slot's tower progress. A missing, unreadable or damaged file reads as
/// nothing cleared — the safe reading (it can only offer LESS of the tower).
pub fn read_tower_progress(data_dir: &Path, slot: u8) -> crate::guardian::Progress {
    let Some(root) = crate::vault::app_root(data_dir) else {
        return crate::guardian::Progress::default();
    };
    std::fs::read_to_string(tower_progress_file(&root, slot))
        .map(|t| crate::guardian::parse_progress(&t))
        .unwrap_or_default()
}

/// Persist a slot's progress — atomic (temp + rename), so a kill mid-write keeps the old
/// file whole. `false` when it could not be written (said in the log).
pub fn write_tower_progress(data_dir: &Path, slot: u8, p: &crate::guardian::Progress) -> bool {
    let Some(root) = crate::vault::app_root(data_dir) else {
        log::warn!("guardian: no app root yet — slot {slot}'s progress is not written");
        return false;
    };
    let path = tower_progress_file(&root, slot);
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("guardian: could not create {}: {e}", dir.display());
        return false;
    }
    match crate::database::write_atomic(&path, crate::guardian::write_progress(p).as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("guardian: could not persist progress to {}: {e}", path.display());
            false
        }
    }
}

/// 초기화 or a new character in `slot`: its tower progress goes with the old occupant —
/// the same two hooks as the bag boxes (`BagBoxHost::forget_slot` from the DB repository,
/// `forget_created_slot` on the creation edge). Static, so the repository can call it.
pub fn forget_tower_progress(app_root: &Path, slot: u8) {
    let path = tower_progress_file(app_root, slot);
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => log::warn!("guardian: slot {slot}'s tower progress is gone with its character"),
            Err(e) => log::warn!("guardian: could not remove {}: {e}", path.display()),
        }
    }
}

/// Where a maintenance purge is asked for, over `adb`.
const WH_PURGE_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_wh_purge.txt";

/// Drop rows from the offline warehouse, once, on request.
///
/// ── WHY THIS EXISTS ──────────────────────────────────────────────────────────
/// The warehouse file lives in the INTERNAL data dir, which `adb` cannot reach on a
/// release build (`run-as` refuses a non-debuggable package), and the game itself can
/// only discard one cell at a time. Testing a warehouse change against a store holding
/// a hundred rows of the same thing is otherwise a hundred taps. Same escape hatch as
/// [`read_blit_hle`] — `/sdcard/Android/data/<pkg>/files` is exempt from scoped storage
/// for `adb shell`.
///
/// The file is one rule per line, and is DELETED once applied so it cannot fire twice:
/// ```text
/// cat 10 idx 0f      # every row with that (cat, idx)
/// cat 10             # every row of that category
/// all                # everything
/// ```
/// Nothing here can touch a save record — this file is the warehouse and only the
/// warehouse. Called at startup, before the guest opens a socket.
pub fn apply_wh_purge(data_dir: &Path) -> Option<String> {
    let spec = std::fs::read_to_string(WH_PURGE_TRIGGER).ok()?;
    let _ = std::fs::remove_file(WH_PURGE_TRIGGER);
    let mut store = read_offline_store(data_dir);
    let before = store.items.len();
    for line in spec.lines() {
        let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        let want = |k: &str| {
            f.iter()
                .position(|t| *t == k)
                .and_then(|i| f.get(i + 1))
                .and_then(|v| u8::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        };
        match (f.first().copied(), want("cat"), want("idx")) {
            (Some("all"), _, _) => store.items.clear(),
            (_, Some(cat), Some(idx)) => store.items.retain(|r| !(r[8] == cat && r[9] == idx)),
            (_, Some(cat), None) => store.items.retain(|r| r[8] != cat),
            _ => {}
        }
    }
    if store.items.len() == before {
        return Some(format!("warehouse purge: nothing matched ({before} row(s) kept)"));
    }
    write_offline_store(data_dir, &store);
    Some(format!(
        "warehouse purge: {} row(s) removed, {} left",
        before - store.items.len(),
        store.items.len()
    ))
}

/// Where the 공격속도 TEST knob is asked for, over `adb` — the same escape hatch as
/// [`REAVER_TEST_TRIGGER`], for the same reason.
///
/// ```text
/// adb shell "echo 'pct 100' > /sdcard/Android/data/com.hero3remake.app/files/wie_attack_speed.txt"
/// ```
pub const ATTACK_SPEED_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_attack_speed.txt";

/// Read the 공격속도 override and DELETE the file — one app run, like the reaver test.
///
/// It exists because the stat has nowhere to come from yet: no shipped equipment row
/// carries the option (all 13 categories scanned, zero rows), so without this the only
/// way to see the hook work is to build an item first. `None` = no override, and then the
/// published percent is whatever the worn gear sums to (0, today).
pub fn read_attack_speed_test() -> Option<u32> {
    let text = std::fs::read_to_string(ATTACK_SPEED_TRIGGER).ok()?;
    let _ = std::fs::remove_file(ATTACK_SPEED_TRIGGER);
    match crate::atkspeed::parse_override(&text) {
        Some(pct) => {
            log::warn!("attack speed: armed for THIS RUN ONLY - {pct} %; {ATTACK_SPEED_TRIGGER} deleted");
            Some(pct)
        }
        None => {
            log::warn!("attack speed: {ATTACK_SPEED_TRIGGER} set nothing (expected `pct N`); ignored");
            None
        }
    }
}

/// Where the reaver-drop TEST knobs are asked for, over `adb`. Same escape hatch as
/// [`WH_PURGE_TRIGGER`] and [`GRANT_TRIGGER`], and for the same reason: a release build's
/// internal data dir is unreachable from `adb`.
pub const REAVER_TEST_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_reaver_test.txt";

/// A reaver-drop test arming: what to override for exactly one app run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReaverTest {
    /// Percent chance per kill, replacing `reaverdrop::RATE_PCT` (clamped 1..=100).
    pub rate_pct: Option<u32>,
    /// Treat every killed enemy as this species, so an ordinary monster takes a boss's
    /// dice. `141` = 크레이지타이탄 (reaver + 타이탄의 파편), `138` = 다크니스이라
    /// (reaver + 검은 눈물).
    pub as_species: Option<u8>,
    /// Pretend the guest difficulty byte reads HARD. The drop is hard-mode only (78차-C), so
    /// without this a tester on a normal-mode save sees nothing however the other two are set.
    pub force_hard: bool,
}

/// Parse a reaver-test file. Decimal, `#` comments, one key per line:
/// ```text
/// rate 100      # percent per kill, 1..100
/// as 141        # pretend every kill is this species (0..255)
/// hard 1        # pretend the guest difficulty byte says HARD
/// ```
/// `None` when nothing was actually set - a file that says nothing arms nothing.
pub fn parse_reaver_test(text: &str) -> Option<ReaverTest> {
    let mut out = ReaverTest::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else { continue };
        match key.to_ascii_lowercase().as_str() {
            "rate" => out.rate_pct = val.parse::<u32>().ok().map(|n| n.clamp(1, 100)),
            // `as` is the species the dice are judged against; `species` is accepted as a
            // synonym because that is what the first draft of this file was called.
            "as" | "species" => out.as_species = val.parse::<u8>().ok(),
            // Anything but an explicit 0 / off / false means "force hard".
            "hard" => out.force_hard = !matches!(val.to_ascii_lowercase().as_str(), "0" | "off" | "false"),
            _ => {}
        }
    }
    (out.rate_pct.is_some() || out.as_species.is_some() || out.force_hard).then_some(out)
}

/// Read the reaver-test arming and DELETE the file, so the widening lasts exactly one app
/// run and reverts itself on the next launch.
///
/// The same read-then-remove lifetime as [`apply_wh_purge`], and for a stronger reason: a
/// forgotten 100 % drop rate is a content change, so it must not be able to survive a
/// restart. `None` = nothing armed, which leaves the shipped constants byte-identical.
pub fn read_reaver_test() -> Option<ReaverTest> {
    let text = std::fs::read_to_string(REAVER_TEST_TRIGGER).ok()?;
    let _ = std::fs::remove_file(REAVER_TEST_TRIGGER);
    let parsed = parse_reaver_test(&text);
    match parsed {
        Some(t) => log::warn!(
            "reaver test: armed for THIS RUN ONLY - rate {:?} %, treat every kill as species {:?}, force hard {}; {} deleted",
            t.rate_pct,
            t.as_species,
            t.force_hard,
            REAVER_TEST_TRIGGER
        ),
        None => log::warn!("reaver test: {REAVER_TEST_TRIGGER} set nothing (expected `rate N`, `as N` and/or `hard 1`); ignored"),
    }
    parsed
}

#[cfg(test)]
mod reaver_test_tests {
    use super::*;

    #[test]
    fn a_file_that_says_nothing_arms_nothing() {
        assert_eq!(parse_reaver_test(""), None);
        assert_eq!(parse_reaver_test("# just a comment\n\n"), None);
        assert_eq!(parse_reaver_test("rate\n"), None, "a key with no value");
        assert_eq!(parse_reaver_test("nonsense 5\n"), None, "an unknown key");
        assert_eq!(parse_reaver_test("rate abc\n"), None, "an unparsable value arms nothing");
    }

    #[test]
    fn both_keys_parse_and_the_rate_is_clamped_to_a_percent() {
        let bare = |r, a| ReaverTest {
            rate_pct: r,
            as_species: a,
            force_hard: false,
        };
        assert_eq!(parse_reaver_test("rate 100"), Some(bare(Some(100), None)));
        assert_eq!(parse_reaver_test("as 141"), Some(bare(None, Some(141))));
        assert_eq!(parse_reaver_test("rate 50\nas 138\n"), Some(bare(Some(50), Some(138))));
        // `species` is a synonym for `as`.
        assert_eq!(parse_reaver_test("species 29"), Some(bare(None, Some(29))));
        // Comments, blank lines, extra whitespace and case are all tolerated.
        assert_eq!(
            parse_reaver_test("  # test\n\n  RATE  100   # every kill\n  As 141\n"),
            Some(bare(Some(100), Some(141)))
        );
        // ★ 78차-C: `hard` arms on its own, and only an explicit 0/off/false disarms it.
        assert_eq!(
            parse_reaver_test("hard 1"),
            Some(ReaverTest {
                rate_pct: None,
                as_species: None,
                force_hard: true
            })
        );
        assert!(parse_reaver_test("hard yes").unwrap().force_hard);
        assert_eq!(parse_reaver_test("hard 0"), None, "an explicit off arms nothing at all");
        assert_eq!(parse_reaver_test("hard off"), None);
        assert!(parse_reaver_test("rate 100\nhard 1\n").unwrap().force_hard);
        // A rate outside 1..=100 is clamped, never dropped - 0 would silently disable the
        // very thing the tester armed, and >100 is just 100.
        assert_eq!(parse_reaver_test("rate 0").unwrap().rate_pct, Some(1));
        assert_eq!(parse_reaver_test("rate 9999").unwrap().rate_pct, Some(100));
        // A species is a u8; the guest range is 0..=160.
        assert_eq!(parse_reaver_test("as 256"), None, "out of a u8 - arms nothing");
        assert_eq!(parse_reaver_test("as 0").unwrap().as_species, Some(0));
        assert_eq!(parse_reaver_test("as 255").unwrap().as_species, Some(255));
    }

    /// The two species a tester will actually type, and what each one buys.
    #[test]
    fn the_documented_species_are_the_two_accessory_bosses() {
        // 141 크레이지타이탄 -> reaver + 타이탄의 파편; 138 다크니스이라 -> reaver + 검은 눈물.
        for s in [141u8, 138] {
            assert_eq!(parse_reaver_test(&format!("as {s}")).unwrap().as_species, Some(s));
        }
    }
}

/// Where a one-shot item grant is asked for, over `adb`. Same escape hatch as
/// [`WH_PURGE_TRIGGER`] and for the same reason.
pub const GRANT_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_grant.txt";

/// A maintenance grant: put these `(cat, idx)` rows into the bag of ONE save slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// 0-based save slot (`GameSave_11_<slot>`) — the grant waits until that slot is
    /// the live character (`vault::live_slot`), so it cannot land in the wrong bag.
    pub slot: u8,
    pub items: Vec<(u16, u16)>,
}

/// Parse a grant file. One rule per line, hex like the purge file, `#` comments:
/// ```text
/// slot 1             # 0-based; the SECOND save slot
/// cat 0c idx 28      # one bag cell each, in order
/// cat 04 idx 1e
/// ```
/// `None` when there is no `slot` line or no items — a file that says nothing
/// grants nothing.
pub fn parse_grant(text: &str) -> Option<Grant> {
    let mut slot = None;
    let mut items = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        let want = |k: &str| {
            f.iter()
                .position(|t| *t == k)
                .and_then(|i| f.get(i + 1))
                .and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        };
        match (f.first().copied(), want("slot"), want("cat"), want("idx")) {
            (Some("slot"), Some(s), _, _) => slot = u8::try_from(s).ok(),
            (Some("cat"), _, Some(cat), Some(idx)) => items.push((cat, idx)),
            _ => {}
        }
    }
    let slot = slot?;
    (!items.is_empty()).then_some(Grant { slot, items })
}

/// The pending grant, if a file asks for one. NOT deleted here: the worker deletes it
/// with [`clear_grant`] only once every row is in the bag and a save is armed, so a
/// grant interrupted by a crash or a wrong slot simply waits for the next boot.
pub fn read_grant() -> Option<Grant> {
    let text = std::fs::read_to_string(GRANT_TRIGGER).ok()?;
    let g = parse_grant(&text);
    if g.is_none() {
        log::warn!("grant: {GRANT_TRIGGER} exists but has no `slot` line or no `cat … idx …` rows; ignored");
    }
    g
}

pub fn clear_grant() {
    let _ = std::fs::remove_file(GRANT_TRIGGER);
}

/// ★ 142차-C (2026-09-30): put N units straight into a 3종 가방 BOX — what [`GRANT_TRIGGER`]
/// cannot do. The grant hands the game ONE unit per row and calls the row done when it
/// shows up in the BAG; a box category (재료 `0e` · 오브 `10` · 소비 `12`/`0d`) is taken into
/// its box by the intake hook and never shows up in the bag, so a grant row for one would
/// never finish (and the grant waits on a full bag anyway). Asked for: 닉스코어의 파편 99 x 5
/// for a 조합 test. Read by `BagBoxHost::frame` while the named slot is the live one, applied
/// with `grant_boxed` (immediately durable), then deleted.
pub const BOX_ADD_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_bagbox_add.txt";

/// A parsed [`BOX_ADD_TRIGGER`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxAdd {
    /// 0-based save slot, like the grant's.
    pub slot: u8,
    /// `(cat, idx, units)`.
    pub rows: Vec<(u16, u16, u32)>,
}

/// `slot 1` then `cat 0e idx 2e n 99` lines (cat / idx / slot hex like the grant file, `n`
/// decimal, default 1), `#` comments. `None` without a slot line or a row.
pub fn parse_box_add(text: &str) -> Option<BoxAdd> {
    let mut slot = None;
    let mut rows = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        let after = |k: &str| f.iter().position(|t| *t == k).and_then(|i| f.get(i + 1)).copied();
        let hex = |k: &str| after(k).and_then(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok());
        match f.first().copied() {
            Some("slot") => slot = hex("slot").and_then(|s| u8::try_from(s).ok()),
            Some("cat") => {
                if let (Some(cat), Some(idx)) = (hex("cat"), hex("idx")) {
                    let n = after("n").map_or(Some(1), |v| v.parse::<u32>().ok());
                    if let Some(n) = n.filter(|&n| n > 0) {
                        rows.push((cat, idx, n));
                    }
                }
            }
            _ => {}
        }
    }
    let slot = slot?;
    (!rows.is_empty()).then_some(BoxAdd { slot, rows })
}

pub fn read_box_add() -> Option<BoxAdd> {
    let text = std::fs::read_to_string(BOX_ADD_TRIGGER).ok()?;
    let b = parse_box_add(&text);
    if b.is_none() {
        log::warn!("bagbox add: {BOX_ADD_TRIGGER} exists but has no `slot` line or no `cat … idx … n …` rows; ignored");
    }
    b
}

pub fn clear_box_add() {
    let _ = std::fs::remove_file(BOX_ADD_TRIGGER);
}

/// ★ 176차 (2026-10-09): own every avatar set in the named save slots — a test tool asked for
/// as *"테스트용으로 s25의 첫번째 케이와 두번째 리츠 세이브 슬롯에는 모든 아바타 파츠 전부 넣어줘"*.
/// `slot N` lines (`avatar::parse_test_grant`: hex, 0-based). Read by `AvatarHost::test_grant`
/// while one of those slots is the live character on the field; a slot's line leaves the file
/// once the game's save is armed ([`drop_avatar_grant_slot`]), and the last one deletes it.
pub const AVATAR_GRANT_TRIGGER: &str = "/sdcard/Android/data/com.hero3remake.app/files/wie_avatar_grant.txt";

/// The slots still owed — empty when there is no file (the common case: one failed open). A file
/// that names no slot is deleted with a warning, so a periodic read does not repeat it.
pub fn read_avatar_grant() -> Vec<u8> {
    let Ok(text) = std::fs::read_to_string(AVATAR_GRANT_TRIGGER) else { return Vec::new() };
    let slots = crate::avatar::parse_test_grant(&text);
    if slots.is_empty() {
        log::warn!("avatar: {AVATAR_GRANT_TRIGGER} names no `slot N` line; deleted");
        let _ = std::fs::remove_file(AVATAR_GRANT_TRIGGER);
    }
    slots
}

/// Take `slot`'s line out of the file — temp-then-rename; no slot left deletes it.
pub fn drop_avatar_grant_slot(slot: u8) {
    let Ok(text) = std::fs::read_to_string(AVATAR_GRANT_TRIGGER) else { return };
    let Some(rest) = crate::avatar::test_grant_without(&text, slot) else { return };
    if crate::avatar::parse_test_grant(&rest).is_empty() {
        let _ = std::fs::remove_file(AVATAR_GRANT_TRIGGER);
        return;
    }
    let tmp = format!("{AVATAR_GRANT_TRIGGER}.tmp");
    if std::fs::write(&tmp, rest.as_bytes()).is_ok() && std::fs::rename(&tmp, AVATAR_GRANT_TRIGGER).is_err() {
        let _ = std::fs::write(AVATAR_GRANT_TRIGGER, rest.as_bytes());
    }
}

/// ★ 141차: a row that has LANDED in the bag leaves the file — the first `cat … idx …`
/// line naming it. The file is what is still owed, so a grant larger than the bag can
/// be paid out across several visits without handing a sold or stored row out again
/// (the worker's "is it in the bag?" test alone would). Temp-then-rename; an empty
/// remainder deletes the file.
pub fn drop_grant_row(cat: u16, idx: u16) {
    let Ok(text) = std::fs::read_to_string(GRANT_TRIGGER) else { return };
    let Some(rest) = grant_without_row(&text, cat, idx) else { return };
    if parse_grant(&rest).is_none() {
        clear_grant();
        return;
    }
    let tmp = format!("{GRANT_TRIGGER}.tmp");
    if std::fs::write(&tmp, rest.as_bytes()).is_ok() && std::fs::rename(&tmp, GRANT_TRIGGER).is_err() {
        let _ = std::fs::write(GRANT_TRIGGER, rest.as_bytes());
    }
}

/// The grant file text without the FIRST `cat … idx …` line naming `(cat, idx)`;
/// `None` when no line names it. Comments and the `slot` line survive untouched.
pub fn grant_without_row(text: &str, cat: u16, idx: u16) -> Option<String> {
    let mut dropped = false;
    let keep: Vec<&str> = text
        .lines()
        .filter(|line| {
            if dropped {
                return true;
            }
            let hit = parse_grant(&format!("slot 0
{line}
")).is_some_and(|g| g.items == [(cat, idx)]);
            dropped |= hit;
            !hit
        })
        .collect();
    dropped.then(|| keep.join("
") + "
")
}

/// Persist the offline warehouse.
///
/// Temp-then-rename, like the record store: this file IS the player's warehouse, and
/// a torn write would lose every item in it. The cost is one extra small write per
/// deposit, against the whole warehouse.
///
/// `true` when the file is in place. ★ 96차: a host box move debits the OTHER side only on
/// `true` — a deposit that never reached the disk must not cost the bag box its units.
pub fn write_offline_store(data_dir: &Path, store: &crate::offline::Store) -> bool {
    let path = offline_store_path(data_dir);
    let tmp = path.with_extension("tmp");
    let text = store.encode();
    if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
        log::warn!("offline: cannot write the warehouse: {e}");
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!("offline: cannot replace the warehouse file: {e}");
        return false;
    }
    true
}

#[cfg(test)]
mod grant_row_tests {
    use super::*;

    const FILE: &str = "# 141\nslot 1\ncat 00 idx 38   # a\ncat 0f idx 27   # b\ncat 0c idx 28\n";

    #[test]
    fn a_landed_row_leaves_the_file_and_the_rest_still_parses() {
        let rest = grant_without_row(FILE, 0x0f, 0x27).unwrap();
        assert_eq!(parse_grant(&rest).unwrap().items, vec![(0x00, 0x38), (0x0c, 0x28)]);
        assert!(rest.starts_with("# 141"), "comments and the slot line survive: {rest:?}");
        // A row the file does not name changes nothing.
        assert_eq!(grant_without_row(FILE, 0x0f, 0x28), None);
        // Striking the last row leaves a file that grants nothing — the caller deletes it.
        let one = "slot 1\ncat 0c idx 28\n";
        assert!(parse_grant(&grant_without_row(one, 0x0c, 0x28).unwrap()).is_none());
    }
}

#[cfg(test)]
mod cooldown_tests {
    use super::*;

    #[test]
    fn missing_file_keeps_only_positive_builtins() {
        let dir = std::env::temp_dir().join(format!("wie_cd_miss_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let map = read_skill_cooldowns(&dir);
        assert!(map.values().all(|&ms| ms > 0));
        for &(sheet, cell, ms) in BUILTIN_SKILL_COOLDOWNS {
            if ms > 0 {
                assert_eq!(map.get(&(sheet, cell)), Some(&ms));
            } else {
                assert_eq!(map.get(&(sheet, cell)), None);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_overrides_and_zero_clears_a_builtin() {
        let dir = std::env::temp_dir().join(format!("wie_cd_ov_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wie_cooldowns.txt"), "s8 5 8000\ns7 0 0\n").unwrap();
        let map = read_skill_cooldowns(&dir);
        assert_eq!(map.get(&(4, 5)), Some(&8000)); // s8 일심 overlaid
        assert_eq!(map.get(&(3, 0)), None); // s7 사격 written as 0 → no overlay
        assert_eq!(map.get(&(0, 2)), Some(&(80 * SKILL_CD_TICK_MS))); // s4 자격 untouched
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skill_detail_zero_has_no_overlay() {
        let dir = std::env::temp_dir().join(format!("wie_cd_zero_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let map = read_skill_cooldowns(&dir);
        assert_eq!(map.get(&(0, 0)), None); // s4 창술 — 쿨타임 없음
        assert_eq!(map.get(&(0, 1)), None); // s4 섬광 — 연속사용 / 0
        assert_eq!(map.get(&(0, 6)), None); // s4 태산 — 패시브
        assert_eq!(map.get(&(3, 0)), None); // s7 사격
        assert_eq!(map.get(&(4, 5)), Some(&(10 * SKILL_CD_TICK_MS))); // s8 일심
        assert_eq!(map.get(&(0, 2)), Some(&(80 * SKILL_CD_TICK_MS))); // s4 자격
        assert_eq!(map.len(), BUILTIN_SKILL_COOLDOWNS.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_skips_a_damaged_line() {
        let mut out = std::collections::HashMap::new();
        let n = apply_cooldown_lines(&mut out, "s4 0 1500\nnot a line\ns99 0 1\n4 9 100\n");
        assert_eq!(n, 1);
        assert_eq!(out.get(&(0, 0)), Some(&1500));
    }

    #[test]
    fn worn_gear_shortens_a_positive_base() {
        assert_eq!(apply_cd_reduce(8000, 0), 8000);
        assert_eq!(apply_cd_reduce(8000, 20), 6400); // 폭풍의 2홈 ≈ −20 %
        assert_eq!(apply_cd_reduce(8000, 90), 800); // 폭풍의 5홈
        assert_eq!(apply_cd_reduce(0, 20), 0); // no overlay stays none
        assert_eq!(apply_cd_reduce(1000, 100), 10); // 99 % cap, never 0
        assert_eq!(apply_cd_reduce(1000, 200), 10);
    }
}

#[cfg(test)]
mod stick_mode_tests {
    use super::*;
    use crate::StickMode;

    /// The word every pre-2026-09-22 십자키 device has on disk must still mean the
    /// arrows — reading it as the new 고정 스틱 would swap those players' control.
    #[test]
    fn the_legacy_fixed_token_is_still_the_dpad() {
        assert_eq!(parse_stick_mode("fixed"), StickMode::Dpad);
        assert_eq!(
            parse_stick_mode(
                "fixed
"
            ),
            StickMode::Dpad
        );
        assert_eq!(parse_stick_mode("dpad"), StickMode::Dpad);
        assert_eq!(parse_stick_mode("pinned"), StickMode::Pinned);
        assert_eq!(parse_stick_mode("float"), StickMode::Float);
        assert_eq!(parse_stick_mode(""), StickMode::Float);
        assert_eq!(parse_stick_mode("garbage"), StickMode::Float);
    }

    #[test]
    fn every_mode_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("wie_stick_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_stick_mode(&dir), StickMode::Float, "missing file = 자유 스틱");
        for m in [StickMode::Dpad, StickMode::Pinned, StickMode::Float] {
            write_stick_mode(&dir, m);
            assert_eq!(read_stick_mode(&dir), m);
        }
        // What is written is the new spelling, never the legacy one.
        write_stick_mode(&dir, StickMode::Dpad);
        assert_eq!(std::fs::read_to_string(dir.join("wie_touch_type.txt")).unwrap(), "dpad");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
