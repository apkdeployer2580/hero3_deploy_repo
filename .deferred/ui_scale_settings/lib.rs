//! wie_android — native aarch64 Android host for the wie WIPI/J2ME emulator.
//!
//! Ports `wie_cli` (the desktop winit+softbuffer host) to Android GameActivity.
//! Architecture (see NATIVE_REHOST_HANDOFF.md):
//!   * The emulator core (`wie_backend`/`wie_ktf`/… + vendored `arm32_cpu`) is a
//!     pure software interpreter and runs unchanged.
//!   * A dedicated worker thread with a large stack owns the `Emulator` + our
//!     `Platform` impl and drives the tick/redraw/input loop (the core-compat
//!     audit requires this — the GameActivity/JNI thread has a small stack and
//!     must not host the 512 KB `EmulatedMemory` construction + interpreter).
//!   * The winit UI thread owns the softbuffer surface, blits frames the worker
//!     pushes via `EventLoopProxy`, and forwards touch input to the worker.

mod app;
mod atlas;
mod audio_sink;
mod config;
mod database;
mod emulator;
mod filesystem;
mod items;
mod network;
mod orientation;
mod overlay;
mod platform;
mod vibration;

use wie_backend::KeyCode;

/// What a field quickbar slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Empty,
    Skill,
    Item,
}

/// Which decoded-atlas icon a slot draws: an item_bm frame, or a skill sheet
/// cell `(sheet 0..6 = s4..s10, cell)`. `None` = no icon (empty / unresolved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconRef {
    Item(u16),
    Skill(u8, u8),
}

/// One field-quickbar slot (0..4) for the on-screen action HUD. Its activation
/// key + label come from `quickslot_hotkey`. `count` = item stack (0 for skills).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickSlot {
    pub slot: u8,
    pub kind: SlotKind,
    pub count: u16,
    pub icon: Option<IconRef>,
}

/// Live player gauges for the HUD (from CHAR RAM — see `items.rs` for the offsets
/// and their provenance). HP is real points against a computed max; SP is the
/// game's 0..1000 scale (its max IS the constant 1000).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub hp: u16,
    pub hp_max: u16,
    pub sp: u16,
    pub sp_max: u16,
    pub exp: u32,
    pub exp_next: u32,
    pub level: u16,
}

/// Fixed quickbar slot -> (activation key, display label). This mapping is
/// STATIC in client.bin: the field action dispatcher 0x143048's command jump
/// table routes number keys to a fixed selector slot — key '1'->slot0, '3'->1,
/// '9'->2, '0'->3, '#'->4 (disasm-confirmed). Tapping a HUD button therefore
/// just sends the corresponding key (fully native; no memory injection).
pub fn quickslot_hotkey(slot: u8) -> Option<(KeyCode, char)> {
    Some(match slot {
        0 => (KeyCode::NUM1, '1'),
        1 => (KeyCode::NUM3, '3'),
        2 => (KeyCode::NUM9, '9'),
        3 => (KeyCode::NUM0, '0'),
        4 => (KeyCode::HASH, '#'),
        _ => return None,
    })
}

/// The game's own minimap, read straight out of the offscreen framebuffer it
/// paints (trace-confirmed: `MC_grpCreateOffScreenFrameBuffer(w,h)` once, then
/// `w*h` 1x1 `MC_grpFillRect` calls, then `MC_grpCopyFrameBuffer` to the screen).
/// We re-blit it enlarged in the HUD, so the panel mirrors whatever the game last
/// rendered — no map data or tile-colour guessing on our side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Minimap {
    pub w: u16,
    pub h: u16,
    /// `w*h` pixels, row-major, `0xAARRGGBB`.
    pub px: Vec<u32>,
}

/// Emulator worker thread -> winit UI thread (via `EventLoopProxy`).
#[derive(Debug)]
pub enum UserEvent {
    /// A freshly rendered content frame (`0xAARRGGBB`, `CONTENT_W * CONTENT_H`).
    Paint(Vec<u32>),
    /// The live field HUD state (quickbar slots + player gauges), refreshed from
    /// guest RAM while the game runs. `field` = real-time field free-roam has
    /// control (vs one of the game's own windows being open), which selects the
    /// whole overlay control set; `slots` is empty when the quickbar is not usable.
    /// `stats` stays at the LAST GOOD reading rather than zeroing, so a gauge never
    /// flashes empty on a transient failed read.
    /// `secondary` = the equipped secondary weapon's icon for the key-'7' button
    /// (`None` = nothing equipped, so the button is drawn dimmed).
    /// `map_open` = the game's own map window is up, so the overlay enlarges it.
    /// `shell` = the out-of-world title/slot screen, which is 240-px art on a wide
    /// framebuffer -> letterbox it.
    Hud { field: bool, map_open: bool, shell: bool, slots: Vec<QuickSlot>, stats: Stats, secondary: Option<IconRef> },
    /// The game's minimap image (or `None` until it has been painted once).
    MinimapFrame(Option<Minimap>),
}

/// winit UI thread -> emulator worker thread (via `mpsc`).
pub enum InputEvent {
    Key { code: KeyCode, down: bool },
}

/// Where the game's own top HUD band should sit, packed as
/// `(band_x_guest_px << 16) | y_origin_guest_px`. The UI thread owns the layout, the
/// emulator worker owns guest memory, so it crosses on an atomic (see
/// `items::hud_y_origin_to_set` / `items::band_x_patch`).
pub type HudYShift = std::sync::Arc<std::sync::atomic::AtomicU32>;

/// Original WIPI game content resolution (Hero3/Hero4 = 240x320 portrait).
pub const CONTENT_W: u32 = 240;
pub const CONTENT_H: u32 = 320;

/// Landscape immersive mode: logical content height. The width is derived from
/// the window aspect so the game frame fills the screen with **no letterbox and
/// no distortion** (`w = h * win_w / win_h`).
///
/// Why this works at all (measured 2026-07-28 with `wie_probe`, the headless
/// host runner): Hero3 is **resolution-adaptive**. Given any framebuffer size it
/// draws as much of the map as fits, centres the title art / minimap / HUD and
/// stretches the dialogue bar to the full width. The old
/// `LANDSCAPE_FEASIBILITY.md` verdict ("game never reads screen w/h → content
/// pinned top-left → needs guest patching") is refuted by that experiment.
///
/// The only limit is the **map's own pixel extent**: a map narrower than the
/// logical width shows black at the sides (the world simply ends). Hero3 map
/// widths are 176..1120 px, median 512 (134 maps) — so a logical width around
/// 500 fills most maps, and a smaller base height buys more fill at the cost of
/// chunkier pixels. Hence the base height is a tunable, not a constant.
pub const LANDSCAPE_BASE_H: u32 = 240;
/// Guard rails for the derived logical width.
pub const LOGICAL_W_MIN: u32 = 240;
pub const LOGICAL_W_MAX: u32 = 1024;

/// Hard cap on the immersive logical WIDTH, forced by a guest bug.
///
/// The game's context-popup renderer (`draw_popup`, guest 0x12e450 — the little
/// "포인트투자 / 1·3·7·9 에 등록" list you get by tapping a skill or item cell)
/// truncates the popup's INTERIOR origin to 8 bits: `text_x = (u8)(x + 6)`
/// (`strb`, file 0x2e47c), read back with `ldrb` by the row backgrounds, the
/// selection highlight, the option text and the inner border, while the outer
/// panel keeps the full 16-bit x. So the moment a popup lands at x > 249 its whole
/// interior wraps to `x - 256` — i.e. to the FAR LEFT EDGE of the screen. That is
/// the player-reported "the register list appears at the very left instead of
/// under the cell" (2026-07-30). At the original 240 px width the bug is
/// unreachable (max interior x = 116), which is why the game shipped with it.
///
/// The placement itself is fine and screen-width-adaptive: the menu window is
/// centred as `(screen_w - 176) / 2`, the popup sits at `cell_x + cell_w + 2` and
/// flips to `cell_x - popup_w - 2` when that would cross the screen midpoint. The
/// unflipped branch is what binds: it can put the popup just under `screen_w / 2`,
/// so `interior <= screen_w/2 + 7 <= 255` gives **`screen_w <= 496`** — and that
/// holds for EVERY popup call site (skill grid, inventory grid, entry 3) whatever
/// its cell positions, which the per-site formulas do not. The popup's y is safe
/// for every `BASE_H_CHOICES` value.
///
/// Fixing it in the guest instead would mean widening two adjacent stack bytes
/// plus ~8 non-size-neutral rewrites in a code cave; this costs ~5 % of horizontal
/// view on aspect ratios wider than 2.07:1 (nothing at all on 16:9/16:10) and no
/// binary patching. RE: workflow wf_6d270fb0 Q2, CONFIRMED by independent
/// byte-level re-derivation.
pub const POPUP_SAFE_LOGICAL_W: u32 = 496;

static LOGICAL: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
static IMMERSIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// User control-size knobs, as a PERCENT (see `config::UI_SCALE_CHOICES`). Unlike
/// the guest framebuffer size these are LIVE — the settings modal changes them and
/// the next relayout picks them up — so they are atomics, not `OnceLock`s.
static MINIMAP_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);
static ACTION_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);

pub fn minimap_scale() -> u32 {
    MINIMAP_SCALE.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn set_minimap_scale(v: u32) {
    MINIMAP_SCALE.store(v, std::sync::atomic::Ordering::Relaxed);
}
pub fn action_scale() -> u32 {
    ACTION_SCALE.load(std::sync::atomic::Ordering::Relaxed)
}
pub fn set_action_scale(v: u32) {
    ACTION_SCALE.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// Apply `pct` percent to `v`, never returning 0.
pub fn scaled(v: u32, pct: u32) -> u32 {
    ((u64::from(v) * u64::from(pct.clamp(50, 200)) / 100) as u32).max(1)
}

/// Resolve the guest framebuffer size ONCE, from the window the game will run
/// in. `None` (or a portrait window) keeps the original 240x320.
///
/// Called before the emulator thread starts; the WIPI screen framebuffer is
/// allocated once at `MC_grpGetScreenFrameBuffer` and its pointer is cached by
/// the guest, so this cannot change while the game runs.
/// `landscape_pref` = the persisted orientation choice, NOT `win_w > win_h`.
/// Device-confirmed race (2026-07-28): `orientation::set(true)` is issued before
/// the event loop, so the FIRST `resumed` can still report the pre-rotation
/// PORTRAIT size (seen: launch A 2340x1080, launch B 1080x2340 on the same
/// device with the same config). Sizing off that transient value silently
/// dropped immersive mode every other launch — so derive the aspect from the
/// window's long/short sides plus the preference the activity will settle into.
pub fn init_logical(win_w: u32, win_h: u32, landscape_immersive: bool, landscape_pref: bool, base_h: u32) -> (u32, u32) {
    let (long, short) = (win_w.max(win_h), win_w.min(win_h));
    let size = if landscape_immersive && landscape_pref && short > 0 {
        let base = base_h.max(120);
        // even width keeps the 2-byte-per-pixel guest stride tidy
        let w = ((u64::from(base) * u64::from(long) / u64::from(short)) as u32)
            .clamp(LOGICAL_W_MIN, LOGICAL_W_MAX.min(POPUP_SAFE_LOGICAL_W))
            & !1;
        // Derive the height BACK from the (possibly clamped) width so the frame
        // still fills the window 1:1 — a clamp applied to the width alone would
        // stretch the picture. On aspects up to 2.07:1 nothing is clamped and this
        // returns `base` unchanged (±1 px of integer rounding).
        let h = ((u64::from(w) * u64::from(short) / u64::from(long)) as u32).max(120);
        (w, h)
    } else {
        (CONTENT_W, CONTENT_H)
    };
    let _ = LOGICAL.set(size);
    let _ = IMMERSIVE.set(size != (CONTENT_W, CONTENT_H));
    logical()
}

/// The guest framebuffer size in use (240x320 until [`init_logical`] runs).
pub fn logical() -> (u32, u32) {
    *LOGICAL.get().unwrap_or(&(CONTENT_W, CONTENT_H))
}

/// True when the guest runs at an aspect-matched wide framebuffer, i.e. the
/// window is filled 1:1 by the game and the controls are translucent overlays.
pub fn immersive_active() -> bool {
    *IMMERSIVE.get().unwrap_or(&false)
}

/// For v1 the game archive is bundled as an APK asset (no file picker yet).
pub const BUNDLED_GAME_ASSET: &str = "hero3_fixed.zip";

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use std::sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::channel,
    };

    use winit::{event_loop::EventLoop, platform::android::EventLoopBuilderExtAndroid};

    // Save-load reject-of-valid bug FIXED 2026-07-22 (DB slot 15 MC_dbUnk15
    // was an unimplemented crash-stub in the loader FUN_94d70's success path;
    // now a benign no-op + select_record_ktf mode-2 size gate). Back to Info —
    // Debug is chatty (~17 timer/s + allocator buckets). Bump to Debug locally
    // if the WIPI-C DB tracing::debug! events are needed again.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("wie"),
    );
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));
    log::info!("android_main: starting wie_android");

    // Read the bundled game + resolve app-private storage BEFORE the AndroidApp
    // is moved into the event loop builder.
    let game_zip = match read_asset(&app, BUNDLED_GAME_ASSET) {
        Some(bytes) => {
            log::info!("loaded asset {BUNDLED_GAME_ASSET} ({} bytes)", bytes.len());
            bytes
        }
        None => {
            log::error!("bundled game asset {BUNDLED_GAME_ASSET} not found; aborting");
            return;
        }
    };
    let data_dir = match app.internal_data_path() {
        Some(p) => p,
        None => {
            log::error!("no internal_data_path; aborting");
            return;
        }
    };
    log::info!("internal data dir = {data_dir:?}");

    // Restore the saved orientation preference before the window appears.
    if config::read_landscape(&data_dir) {
        orientation::set(true);
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .with_android_app(app)
        .build()
        .expect("failed to build android event loop");
    let proxy = event_loop.create_proxy();

    let (input_tx, input_rx) = channel::<InputEvent>();
    let should_redraw = Arc::new(AtomicBool::new(true));
    // Shared background/lock gate. The UI thread sets it on suspend / focus-loss;
    // while set, the emulator worker stops ticking (no new sound events, saves
    // battery) and the MIDI synth emits silence — so audio does NOT keep playing
    // when the app is backgrounded or the screen is locked. Cleared on resume /
    // focus-gain (BGM resumes seamlessly).
    let paused = Arc::new(AtomicBool::new(false));
    // UI thread -> worker: how far to push the game's own top HUD band down.
    let hud_y: HudYShift = Arc::new(std::sync::atomic::AtomicU32::new(0));

    // The emulator boot is DEFERRED to the first `resumed` so the guest
    // framebuffer can be sized from the real window (landscape immersive mode).
    let pending_boot = app::PendingBoot {
        proxy,
        should_redraw,
        input_rx,
        game_zip,
        hud_y: hud_y.clone(),
    };
    let mut handler = app::App::new(input_tx, data_dir, paused, hud_y, pending_boot);
    if let Err(e) = event_loop.run_app(&mut handler) {
        log::error!("event loop exited with error: {e}");
    }
}

/// Read a file bundled in the APK's `assets/` via the NDK AssetManager.
#[cfg(target_os = "android")]
fn read_asset(app: &winit::platform::android::activity::AndroidApp, name: &str) -> Option<Vec<u8>> {
    use std::io::Read;

    let manager = app.asset_manager();
    let cname = std::ffi::CString::new(name).ok()?;
    let mut asset = manager.open(&cname)?;
    let mut buf = Vec::new();
    asset.read_to_end(&mut buf).ok()?;
    Some(buf)
}
