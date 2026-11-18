//! Tiny persisted settings (currently just the orientation preference), stored
//! as a one-line file in the app-private data dir so it survives restarts.

use std::path::{Path, PathBuf};

fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_config.txt")
}

/// Returns the saved landscape preference (default = portrait/false).
pub fn read_landscape(data_dir: &Path) -> bool {
    match std::fs::read_to_string(config_path(data_dir)) {
        Ok(s) => s.trim() == "landscape",
        Err(_) => false,
    }
}

pub fn write_landscape(data_dir: &Path, landscape: bool) {
    let value = if landscape { "landscape" } else { "portrait" };
    if let Err(e) = std::fs::write(config_path(data_dir), value) {
        log::warn!("failed to persist orientation pref: {e}");
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

fn ui_scale_path(data_dir: &Path, what: &str) -> PathBuf {
    data_dir.join(format!("wie_scale_{what}.txt"))
}

/// User-adjustable control sizes, as a PERCENT of the built-in size. Cycled from the
/// settings modal (player request 2026-07-30: "let me size the minimap and the OK
/// button from settings"). Applied immediately — unlike the base height, nothing
/// here touches the guest framebuffer.
pub const UI_SCALE_CHOICES: [u32; 5] = [70, 85, 100, 120, 140];
const UI_SCALE_DEFAULT: u32 = 100;

fn read_ui_scale(data_dir: &Path, what: &str) -> u32 {
    let v = std::fs::read_to_string(ui_scale_path(data_dir, what))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(UI_SCALE_DEFAULT);
    if UI_SCALE_CHOICES.contains(&v) { v } else { UI_SCALE_DEFAULT }
}

fn write_ui_scale(data_dir: &Path, what: &str, v: u32) {
    if let Err(e) = std::fs::write(ui_scale_path(data_dir, what), v.to_string()) {
        log::warn!("failed to persist {what} scale: {e}");
    }
}

pub fn read_minimap_scale(data_dir: &Path) -> u32 {
    read_ui_scale(data_dir, "minimap")
}
pub fn write_minimap_scale(data_dir: &Path, v: u32) {
    write_ui_scale(data_dir, "minimap", v);
}
pub fn read_action_scale(data_dir: &Path) -> u32 {
    read_ui_scale(data_dir, "action")
}
pub fn write_action_scale(data_dir: &Path, v: u32) {
    write_ui_scale(data_dir, "action", v);
}

/// Next value in [`UI_SCALE_CHOICES`], wrapping.
pub fn next_ui_scale(cur: u32) -> u32 {
    let i = UI_SCALE_CHOICES.iter().position(|&c| c == cur).unwrap_or(2);
    UI_SCALE_CHOICES[(i + 1) % UI_SCALE_CHOICES.len()]
}

fn account_id_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_account_id")
}

/// The 16-byte per-device account identity injected into the game's `01 3d`
/// login packet (see `network::AndroidNetwork::send`).
///
/// The vanilla Hero3 binary hardcodes this field to the constant `"0123456789"`
/// (its `getProperty("PHONENUMBER")` is a stub), so without this every player
/// would share ONE server account. We give each install a stable, unique id so
/// the private server keys each user's warehouse / recovery backups separately.
///
/// ★ ACCOUNT MODEL (user decision, PROTOCOL.md §6): this is the **device-auto-id**
/// model — the id is generated once and persisted in the app-private dir, so it
/// is stable across restarts but NOT across reinstall / device change. That is
/// enough to separate concurrent users, but it does NOT give cross-device
/// "복구" (recover my character on a new phone). For that, swap this for a
/// **user login (ID/PW)** model: derive the 16 bytes from a user-entered account
/// token (needs a small login UI). Left as device-auto until the user chooses.
pub fn read_or_create_account_id(data_dir: &Path) -> [u8; 16] {
    let path = account_id_path(data_dir);
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 16 {
            let mut id = [0u8; 16];
            id.copy_from_slice(&bytes);
            return id;
        }
    }
    // First run: derive a stable id from the current time. It is persisted
    // forever after, so low first-run entropy is acceptable — it only has to be
    // unique per install. splitmix64 spreads the nanosecond bits across 16 bytes.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut id = [0u8; 16];
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    for chunk in id.chunks_mut(8) {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let bytes = z.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    if let Err(e) = std::fs::write(&path, id) {
        log::warn!("failed to persist account id: {e}");
    }
    log::info!("net: generated a new device account id");
    id
}
