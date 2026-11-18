//! winit UI-thread handler. Blits frames the emulator worker pushes, draws the
//! orientation-aware control overlay + settings gear, and forwards touch input.
//! The softbuffer surface is recreated across suspend/resume (Android destroys
//! the native window when backgrounded).

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};

use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    event::{Touch, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

use wie_backend::KeyCode;

use crate::{CONTENT_H, CONTENT_W, InputEvent, QuickSlot, Stats, UserEvent, config, orientation, overlay, quickslot_hotkey};
use std::sync::mpsc::Receiver;

pub struct App {
    input_tx: Sender<InputEvent>,
    data_dir: PathBuf,

    /// Shared background/lock gate (see lib.rs). Set while the app is not in the
    /// foreground so the emulator stops ticking and the synth emits silence.
    paused: Arc<AtomicBool>,

    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,

    win_w: u32,
    win_h: u32,

    layout: overlay::Layout,
    settings_items: Vec<overlay::SettingsItem>,
    show_settings: bool,

    /// Last content frame from the worker (CONTENT_W x CONTENT_H, 0xAARRGGBB).
    last_frame: Vec<u32>,

    /// Active touch id -> the key it currently maps to (game controls).
    touches: HashMap<u64, KeyCode>,
    /// Touch ids consumed by UI (gear/settings) — excluded from game input.
    ui_touches: HashSet<u64>,
    /// Touch ids that STARTED on a control. Only these may change which key they
    /// hold by sliding (`TouchPhase::Moved`).
    ///
    /// ★ 2026-07-30: without this, ANY touch that had not been adopted by the
    /// joystick — e.g. a second finger while the first one is steering — acquired
    /// whatever button it happened to slide over. With the minimap panel sitting in
    /// the top-right corner as a plain `KeyCode::STAR` button, that is precisely the
    /// player-reported "the minimap keeps getting clicked by itself".
    slide_touches: HashSet<u64>,
    /// In-flight deliberate tap on the minimap panel: `(id, down_x, down_y)`.
    /// The panel is no longer a Button, so it cannot fire on touch-down or by being
    /// slid over; it fires on RELEASE, and only for a real tap.
    mm_tap: Option<(u64, f64, f64)>,
    /// Keys currently held by any touch or the joystick (for highlight).
    pressed: HashSet<KeyCode>,
    /// Left control style: floating joystick (true) vs classic D-pad.
    joystick: bool,
    /// The active joystick drag, if any.
    joy: Option<Joy>,
    /// Field HUD vs game-window (menu) controls — driven by the guest state the
    /// worker reports. See `overlay::Mode`.
    mode: overlay::Mode,
    /// The pull-out full keypad (immersive mode) is showing.
    keypad_open: bool,
    /// The game's own map window is open -> paint a big copy of the map over it.
    map_open: bool,
    /// Out-of-world title/slot screen -> letterbox its 240-px art.
    shell: bool,
    /// Published to the emulator worker: how far down the game's own top HUD band
    /// must go so it sits below our gauges (guest px). See `overlay::hud_y_shift`.
    hud_y: crate::HudYShift,

    /// Live field quickbar (5 slots) for the action HUD (from the worker).
    quickbar: Vec<QuickSlot>,
    /// Live player gauges for the top-left HUD.
    stats: Stats,
    /// Equipped secondary weapon's icon, drawn on the key-'7' button (`None` =
    /// nothing equipped -> the button is dimmed, matching the game's own refusal).
    secondary: Option<crate::IconRef>,
    /// The game's minimap (top-right panel); `None` until it has been painted.
    minimap: Option<crate::Minimap>,
    /// Cached button rects for `quickbar`, recomputed on relayout / change.
    quick_rects: Vec<overlay::Rect>,
    /// The touch id + index currently pressing a quickbar button (for highlight).
    quick_press: Option<(u64, usize)>,

    /// Emulator boot is DEFERRED until the first `resumed`, because the guest
    /// framebuffer size is derived from the real window size (landscape
    /// immersive mode) and the WIPI screen framebuffer can only be allocated
    /// once. `None` after the worker has been spawned.
    pending_boot: Option<PendingBoot>,
}

/// Everything the emulator worker needs, held until the window size is known.
pub struct PendingBoot {
    pub proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    pub should_redraw: Arc<AtomicBool>,
    pub input_rx: Receiver<InputEvent>,
    pub game_zip: Vec<u8>,
    pub hud_y: crate::HudYShift,
}

/// Was `(x, y)` close enough to the touch-down point to count as a TAP rather than
/// a drag? Deliberately generous (fingers wobble) but far below the size of the
/// panel it guards, so a swipe that merely crosses the panel never counts.
const TAP_SLOP: f64 = 28.0;
fn is_tap(dx: f64, dy: f64, x: f64, y: f64) -> bool {
    (x - dx).abs() <= TAP_SLOP && (y - dy).abs() <= TAP_SLOP
}

/// One in-progress joystick drag: which touch owns it, the (trailing) base
/// centre, the current thumb position, and the single direction key it holds.
struct Joy {
    id: u64,
    cx: f64,
    cy: f64,
    tx: f64,
    ty: f64,
    dir: Option<KeyCode>,
}

impl App {
    pub fn new(
        input_tx: Sender<InputEvent>,
        data_dir: PathBuf,
        paused: Arc<AtomicBool>,
        hud_y: crate::HudYShift,
        pending_boot: PendingBoot,
    ) -> Self {
        let joystick = config::read_joystick(&data_dir);
        // Live control-size knobs are globals (overlay geometry reads them without a
        // data_dir), so seed them from disk before the first layout is built.
        crate::set_minimap_scale(config::read_minimap_scale(&data_dir));
        crate::set_action_scale(config::read_action_scale(&data_dir));
        Self {
            input_tx,
            data_dir,
            paused,
            window: None,
            context: None,
            surface: None,
            win_w: CONTENT_W,
            win_h: CONTENT_H,
            layout: overlay::layout(CONTENT_W, CONTENT_H, joystick, overlay::Mode::Field, false),
            settings_items: overlay::settings_items(CONTENT_W, CONTENT_H),
            show_settings: false,
            last_frame: vec![0xff00_0000u32; (CONTENT_W * CONTENT_H) as usize],
            touches: HashMap::new(),
            ui_touches: HashSet::new(),
            slide_touches: HashSet::new(),
            mm_tap: None,
            pressed: HashSet::new(),
            joystick,
            joy: None,
            // Start in MENU mode: at boot the game is on its title screen, not in
            // the field, so the field HUD must not be up. (Device-caught: with
            // `Field` as the initial value the gauges + minimap panel were drawn
            // over the title screen, because the worker's first sample matches its
            // own initial `last_field` and so used to publish nothing.)
            mode: overlay::Mode::Menu,
            keypad_open: false,
            map_open: false,
            shell: false,
            hud_y,
            quickbar: Vec::new(),
            stats: Stats::default(),
            secondary: None,
            minimap: None,
            quick_rects: Vec::new(),
            quick_press: None,
            pending_boot: Some(pending_boot),
        }
    }

    fn relayout(&mut self) {
        self.layout = overlay::layout(self.win_w, self.win_h, self.joystick, self.mode, self.keypad_open);
        self.settings_items = overlay::settings_items(self.win_w, self.win_h);
        self.relayout_quick();
        // Tell the worker where the game's own top band has to go, now that we know
        // how tall our gauge block is at this window size.
        // Packed for the worker: [31:24] target-box left shift, [23:16] band height,
        // [15:8] band x, [7:0] y origin.
        let packed = if crate::immersive_active() && self.win_w > self.win_h {
            let g = self.layout.game;
            (overlay::target_box_shift(g).min(255) << 24)
                | (overlay::band_h_shift(g).min(255) << 16)
                | (overlay::band_x_shift(g).min(255) << 8)
                | overlay::hud_y_shift(g).min(255)
        } else {
            (25 << 16) | (1 << 8) // the game's own defaults
        };
        self.hud_y.store(packed, Ordering::Relaxed);
    }

    /// Recompute quickbar-button rects for the current game rect + slot count.
    fn relayout_quick(&mut self) {
        self.quick_rects = overlay::quick_buttons(self.layout.game, self.layout.gear, self.quickbar.len());
    }

    /// Rebuild `pressed` (highlight set) from the held touch keys + joystick dir.
    fn refresh_pressed(&mut self) {
        let mut p: HashSet<KeyCode> = self.touches.values().copied().collect();
        if let Some(k) = self.joy.as_ref().and_then(|j| j.dir) {
            p.insert(k);
        }
        self.pressed = p;
    }

    /// Update the joystick from a new thumb position: let the base TRAIL the
    /// finger, resolve the single walk direction, and emit keyup(old) before
    /// keydown(new) so the guest re-latches on every turn. `active` = finger down.
    fn joy_update(&mut self, x: f64, y: f64, active: bool) {
        let radius = self
            .layout
            .joystick
            .as_ref()
            .map(overlay::joy_radius)
            .unwrap_or(1.0);
        let Some(joy) = self.joy.as_mut() else { return };
        joy.tx = x;
        joy.ty = y;
        let new_dir = if active {
            // Trailing centre first: a slide that leaves the base behind is what
            // turns the character (see overlay::joy_follow).
            let (cx, cy) = overlay::joy_follow(joy.cx, joy.cy, x, y, radius);
            joy.cx = cx;
            joy.cy = cy;
            overlay::joy_dir(x - cx, y - cy, radius, joy.dir)
        } else {
            None
        };
        if new_dir != joy.dir {
            // Release first, then press: the guest latches a walk direction on
            // keydown, so the new direction must arrive AFTER the old one is gone.
            if let Some(old) = joy.dir {
                let _ = self.input_tx.send(InputEvent::Key { code: old, down: false });
            }
            if let Some(new) = new_dir {
                let _ = self.input_tx.send(InputEvent::Key { code: new, down: true });
            }
            joy.dir = new_dir;
        }
        self.refresh_pressed();
    }

    /// Release the joystick entirely (held direction up).
    fn joy_end(&mut self) {
        if let Some(joy) = self.joy.take()
            && let Some(k) = joy.dir
        {
            let _ = self.input_tx.send(InputEvent::Key { code: k, down: false });
        }
        self.refresh_pressed();
    }

    fn configure_surface(&mut self) {
        let (Some(context), Some(window)) = (self.context.as_ref(), self.window.as_ref()) else {
            return;
        };
        if self.surface.is_none() {
            match Surface::new(context, window.clone()) {
                Ok(s) => self.surface = Some(s),
                Err(e) => {
                    log::error!("Surface::new failed: {e}");
                    return;
                }
            }
        }
        let (Some(w), Some(h)) = (NonZeroU32::new(self.win_w), NonZeroU32::new(self.win_h)) else {
            return;
        };
        if let Some(surface) = self.surface.as_mut()
            && let Err(e) = surface.resize(w, h)
        {
            log::error!("surface resize failed: {e}");
        }
    }

    /// Request a coalesced redraw instead of drawing inline. winit merges any
    /// number of requests between frames into a single `RedrawRequested`, so the
    /// joystick's high-rate touch-move stream (which on a 120/240 Hz-sampling
    /// panel would otherwise trigger one full-screen blit + `present()` per
    /// event — the cause of the drag lag) collapses to at most one draw per frame.
    fn request_redraw(&self) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let (win_w, win_h) = (self.win_w, self.win_h);
        let landscape = win_w > win_h;
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let mut buf = match surface.buffer_mut() {
            Ok(b) => b,
            Err(e) => {
                log::warn!("buffer_mut failed: {e}");
                return;
            }
        };
        if buf.len() != (win_w * win_h) as usize {
            return;
        }

        buf.fill(0xff00_0000);

        // Blit the game into its layout rect (nearest-neighbour, aspect-correct).
        let g = self.layout.game;
        let (lw, lh) = crate::logical();
        if self.last_frame.len() == (lw * lh) as usize && g.w > 0 && g.h > 0 {
            for dy in 0..g.h {
                let sy = (dy * lh) / g.h;
                let src_row = (sy * lw) as usize;
                let dst_row = ((g.y + dy) * win_w + g.x) as usize;
                for dx in 0..g.w {
                    let sx = (dx * lw) / g.w;
                    buf[dst_row + dx as usize] = self.last_frame[src_row + sx as usize];
                }
            }
        }

        // Joystick view (thumb clamped to the base radius for rendering).
        let joy_view = self.joy.as_ref().and_then(|j| {
            let zone = self.layout.joystick.as_ref()?;
            let r = overlay::joy_radius(zone);
            let (dx, dy) = (j.tx - j.cx, j.ty - j.cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let thumb = if dist > r && dist > 0.0 {
                (j.cx + dx / dist * r, j.cy + dy / dist * r)
            } else {
                (j.tx, j.ty)
            };
            Some(overlay::JoyView { center: (j.cx, j.cy), thumb })
        });
        // FIELD only: cover the game's own field HUD (bottom strip + top-left skill
        // box) so our custom HUD replaces it, then draw our HUD on top. When one of
        // the game's own windows is open, all of that must stand down — the mask
        // would black out part of the window, and the gauges/minimap panel would
        // cover it (that, plus a zeroed Stats, is the reported "HP/MP read 0 in the
        // menu" bug). The buttons alone stay, now as menu-navigation keys.
        let field = self.mode == overlay::Mode::Field;
        if field {
            overlay::mask_game_hud(&mut buf, win_w, win_h, self.layout.game);
        }
        // Title / slot screens are 240-px art that the game mirrors outwards to fill
        // a wide framebuffer; letterbox it so it reads as centred, not cropped.
        if self.shell {
            overlay::letterbox_shell(&mut buf, win_w, win_h, self.layout.game);
        }
        // The game's map window is drawn 1:1 by the guest and is tiny on a wide
        // framebuffer; paint our mirrored copy over it, big and centred.
        if self.map_open {
            overlay::draw_map_overlay(&mut buf, win_w, win_h, self.minimap.as_ref());
        }
        overlay::draw_controls(&mut buf, win_w, win_h, &self.layout, &self.pressed, joy_view, self.keypad_open);
        // Gauges + EXP first, skill plates ON TOP: where the arc meets the bottom
        // HUD strip the EXP bar would otherwise clip the last slot.
        if field {
            overlay::draw_stats(&mut buf, win_w, win_h, self.layout.game, &self.stats);
            overlay::draw_quickbar(&mut buf, win_w, win_h, &self.quick_rects, &self.quickbar, self.quick_press.map(|(_, i)| i));
            overlay::draw_minimap(&mut buf, win_w, win_h, self.layout.game, self.minimap.as_ref());
            if crate::immersive_active() && landscape {
                // Key-'7' secondary-weapon button (immersive only — the other
                // layouts already have a real keypad).
                let r = overlay::secondary_button(win_w, win_h);
                overlay::draw_secondary(&mut buf, win_w, win_h, r, self.secondary);
            }
        }
        if self.show_settings {
            overlay::draw_settings(
                &mut buf,
                win_w,
                win_h,
                &self.settings_items,
                landscape,
                self.joystick,
                config::read_immersive(&self.data_dir),
                config::read_base_h(&self.data_dir),
            );
        }

        if let Err(e) = buf.present() {
            log::warn!("present failed: {e}");
        }
    }

    fn update_touch(&mut self, id: u64, new_key: Option<KeyCode>) {
        let old_key = self.touches.get(&id).copied();
        if old_key == new_key {
            return;
        }
        match new_key {
            Some(k) => {
                self.touches.insert(id, k);
            }
            None => {
                self.touches.remove(&id);
            }
        }
        if let Some(ok) = old_key
            && !self.touches.values().any(|&v| v == ok)
        {
            let _ = self.input_tx.send(InputEvent::Key { code: ok, down: false });
        }
        if let Some(nk) = new_key
            && self.touches.values().filter(|&&v| v == nk).count() == 1
        {
            let _ = self.input_tx.send(InputEvent::Key { code: nk, down: true });
        }
        self.refresh_pressed();
        self.request_redraw();
    }

    /// Toggle the shared background/lock gate. Idempotent; logs only on change.
    /// While set, the emulator worker stops ticking and the MIDI synth emits
    /// silence, so no audio continues when the app is backgrounded or locked.
    fn set_paused(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::SeqCst) != paused {
            log::info!("{}", if paused { "app -> PAUSED (background/lock): audio+emu halted" } else { "app -> RESUMED (foreground): audio+emu running" });
        }
    }

    fn handle_settings(&mut self, action: overlay::SettingsAction) {
        match action {
            overlay::SettingsAction::SetPortrait => {
                orientation::set(false);
                config::write_landscape(&self.data_dir, false);
                self.show_settings = false;
            }
            overlay::SettingsAction::SetLandscape => {
                orientation::set(true);
                config::write_landscape(&self.data_dir, true);
                self.show_settings = false;
            }
            overlay::SettingsAction::SetDpad => {
                self.set_control_style(false);
            }
            overlay::SettingsAction::SetJoystick => {
                self.set_control_style(true);
            }
            overlay::SettingsAction::ToggleImmersive => {
                let now = !config::read_immersive(&self.data_dir);
                config::write_immersive(&self.data_dir, now);
                log::info!("immersive pref -> {now} (applies at next start)");
            }
            overlay::SettingsAction::CycleBaseH => {
                let cur = config::read_base_h(&self.data_dir);
                let i = config::BASE_H_CHOICES.iter().position(|h| *h == cur).unwrap_or(1);
                let next = config::BASE_H_CHOICES[(i + 1) % config::BASE_H_CHOICES.len()];
                config::write_base_h(&self.data_dir, next);
                log::info!("immersive base height -> {next} (applies at next start)");
            }
            // Live knobs: persist, apply, and relayout on the spot so the player can
            // see the size they are choosing (player request 2026-07-30).
            overlay::SettingsAction::CycleMinimapScale => {
                let next = config::next_ui_scale(crate::minimap_scale());
                crate::set_minimap_scale(next);
                config::write_minimap_scale(&self.data_dir, next);
                self.relayout();
                log::info!("minimap scale -> {next}%");
            }
            overlay::SettingsAction::CycleActionScale => {
                let next = config::next_ui_scale(crate::action_scale());
                crate::set_action_scale(next);
                config::write_action_scale(&self.data_dir, next);
                self.relayout();
                log::info!("action-disc scale -> {next}%");
            }
            overlay::SettingsAction::Close => {
                self.show_settings = false;
            }
        }
        self.request_redraw();
    }

    fn set_control_style(&mut self, joystick: bool) {
        if self.joystick != joystick {
            self.joy_end(); // release any held joystick dirs before switching
            self.joystick = joystick;
            config::write_joystick(&self.data_dir, joystick);
            self.relayout();
        }
        self.show_settings = false;
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // Foregrounded: resume emulation + audio (BGM continues from where it froze).
        self.set_paused(false);

        let window = match event_loop.create_window(Window::default_attributes().with_title("WIE")) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("create_window failed: {e}");
                return;
            }
        };
        let context = match Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                log::error!("Context::new failed: {e}");
                return;
            }
        };

        let size = window.inner_size();
        self.win_w = size.width.max(1);
        self.win_h = size.height.max(1);
        log::info!("resumed: window size {}x{}", self.win_w, self.win_h);

        self.window = Some(window);
        self.context = Some(context);
        self.surface = None;

        // First resume: now that the real window size is known, fix the guest
        // framebuffer size and start the emulator. Landscape immersive mode gives
        // the guest an aspect-matched wide framebuffer so the game frame fills the
        // screen with no letterbox and no distortion (see `crate::init_logical`).
        if let Some(boot) = self.pending_boot.take() {
            let immersive = config::read_immersive(&self.data_dir);
            let landscape_pref = config::read_landscape(&self.data_dir);
            let base_h = config::read_base_h(&self.data_dir);
            let (lw, lh) = crate::init_logical(self.win_w, self.win_h, immersive, landscape_pref, base_h);
            log::info!(
                "boot: window {}x{} landscape_pref={landscape_pref} -> guest framebuffer {lw}x{lh} (immersive={immersive}, base_h={base_h})",
                self.win_w,
                self.win_h
            );
            self.last_frame = vec![0xff00_0000u32; (lw * lh) as usize];
            crate::emulator::spawn(
                boot.proxy,
                boot.should_redraw,
                boot.input_rx,
                self.data_dir.clone(),
                boot.game_zip,
                self.paused.clone(),
                boot.hud_y,
            );
        }

        self.relayout();
        self.configure_surface();
        self.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        log::info!("suspended: dropping surface/context/window");
        // Backgrounded: halt emulation + audio so nothing keeps playing.
        self.set_paused(true);
        self.surface = None;
        self.context = None;
        self.window = None;
        for key in self.pressed.drain() {
            let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
        }
        self.touches.clear();
        self.ui_touches.clear();
        self.joy = None;
        self.quick_press = None;
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Paint(data) => {
                self.last_frame = data;
                self.request_redraw();
            }
            UserEvent::MinimapFrame(mm) => {
                if mm.is_some() {
                    self.minimap = mm;
                    self.request_redraw();
                }
            }
            UserEvent::Hud { field, map_open, shell, slots, stats, secondary } => {
                let mode = if field { overlay::Mode::Field } else { overlay::Mode::Menu };
                let mut changed = slots != self.quickbar
                    || stats != self.stats
                    || secondary != self.secondary
                    || map_open != self.map_open
                    || shell != self.shell;
                self.secondary = secondary;
                self.map_open = map_open;
                self.shell = shell;
                if self.mode != mode {
                    // Field <-> game-window: the whole control set changes.
                    self.mode = mode;
                    self.relayout();
                    changed = true;
                }
                self.stats = stats;
                if slots != self.quickbar {
                    self.quickbar = slots;
                    // A shrunk list can orphan a held press index; clear it.
                    if self.quick_press.is_some_and(|(_, i)| i >= self.quickbar.len()) {
                        self.quick_press = None;
                    }
                    self.relayout_quick();
                }
                if changed {
                    self.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(new_size) => {
                self.win_w = new_size.width.max(1);
                self.win_h = new_size.height.max(1);
                log::info!("resized: {}x{}", self.win_w, self.win_h);
                self.relayout();
                self.configure_surface();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            // Lost/gained foreground focus. This catches screen-lock (and other
            // focus-stealing overlays) even when the surface is NOT destroyed, so
            // it complements suspended()/resumed(): either signal halts audio.
            WindowEvent::Focused(focused) => {
                self.set_paused(!focused);
                if !focused {
                    // Also release any held game keys so nothing sticks on return.
                    for key in self.pressed.drain() {
                        let _ = self.input_tx.send(InputEvent::Key { code: key, down: false });
                    }
                    self.touches.clear();
                    self.ui_touches.clear();
                    self.slide_touches.clear();
                    self.mm_tap = None;
                    self.joy = None;
                    self.quick_press = None;
                }
            }
            WindowEvent::Touch(Touch { phase, location, id, .. }) => {
                let (x, y) = (location.x, location.y);
                match phase {
                    TouchPhase::Started => {
                        if self.map_open {
                            // Enlarged map is up: ANY tap closes it. The game's map
                            // window closes on any key (guest 0x124360), so one CLEAR
                            // tap dismisses both it and our overlay copy.
                            let _ = self.input_tx.send(InputEvent::Key { code: KeyCode::CLEAR, down: true });
                            let _ = self.input_tx.send(InputEvent::Key { code: KeyCode::CLEAR, down: false });
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.show_settings {
                            if let Some(action) = overlay::settings_hit(&self.settings_items, self.win_w, self.win_h, x, y) {
                                self.handle_settings(action);
                            }
                            self.ui_touches.insert(id);
                        } else if overlay::gear_hit(&self.layout.gear, x, y) {
                            self.show_settings = true;
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if overlay::rect_hit(&self.layout.keypad_toggle, x, y) {
                            // Pull-out full keypad: the only way to reach the keys
                            // immersive mode has no permanent seat for (2/4/5/6/8/0/
                            // */LSK/RSK).
                            self.keypad_open = !self.keypad_open;
                            self.relayout();
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if let Some(i) = overlay::quick_hit(&self.quick_rects, x, y).filter(|_| self.mode == overlay::Mode::Field) {
                            // Field only: the quickbar shares the ring seats with the
                            // menu-navigation keys, and the mode flag and the slot list
                            // arrive in the same message — without this a stale rect
                            // could swallow a nav-key tap for one frame.
                            // One-shot: send the slot's fixed hotkey (down+up) so the
                            // game natively activates it (skill cast / potion use).
                            // Pure key event — no memory injection or guest call.
                            if let Some(slot) = self.quickbar.get(i)
                                && let Some((code, _)) = quickslot_hotkey(slot.slot)
                            {
                                let _ = self.input_tx.send(InputEvent::Key { code, down: true });
                                let _ = self.input_tx.send(InputEvent::Key { code, down: false });
                            }
                            self.quick_press = Some((id, i));
                            self.ui_touches.insert(id);
                            self.request_redraw();
                        } else if self.mode == overlay::Mode::Field
                            && overlay::rect_hit(&overlay::minimap_panel(self.win_w, self.win_h), x, y)
                        {
                            // Minimap panel: a DELIBERATE TAP opens the map (that is
                            // also what makes the guest repaint the buffer we mirror).
                            // Nothing is sent yet — see TouchPhase::Ended.
                            self.mm_tap = Some((id, x, y));
                            self.ui_touches.insert(id);
                        } else if let Some(key) = overlay::hit(&self.layout.buttons, x, y) {
                            self.slide_touches.insert(id);
                            self.update_touch(id, Some(key));
                        } else if self.joy.is_none() && overlay::in_joystick_zone(&self.layout, x, y) {
                            // Spawn the floating joystick at the touch point.
                            self.joy = Some(Joy { id, cx: x, cy: y, tx: x, ty: y, dir: None });
                            self.request_redraw();
                        }
                    }
                    TouchPhase::Moved => {
                        if self.joy.as_ref().is_some_and(|j| j.id == id) {
                            self.joy_update(x, y, true);
                            self.request_redraw();
                        } else if self.mm_tap.is_some_and(|(tid, dx, dy)| tid == id && !is_tap(dx, dy, x, y)) {
                            // Smeared into a drag: not a tap, so it will not open the map.
                            self.mm_tap = None;
                        } else if self.slide_touches.contains(&id) {
                            // Sliding between adjacent keys is legitimate (the D-pad
                            // depends on it) — but ONLY for a touch that began on a
                            // control. A stray finger can no longer pick up keys.
                            let key = overlay::hit(&self.layout.buttons, x, y);
                            self.update_touch(id, key);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        self.slide_touches.remove(&id);
                        // A real tap that both started AND ended inside the panel opens
                        // the map. `Cancelled` never counts — the system took the touch
                        // away, which is not an intent to press anything.
                        if let Some((tid, dx, dy)) = self.mm_tap
                            && tid == id
                        {
                            self.mm_tap = None;
                            if phase == TouchPhase::Ended
                                && is_tap(dx, dy, x, y)
                                && self.mode == overlay::Mode::Field
                                && overlay::rect_hit(&overlay::minimap_panel(self.win_w, self.win_h), x, y)
                            {
                                let _ = self.input_tx.send(InputEvent::Key { code: KeyCode::STAR, down: true });
                                let _ = self.input_tx.send(InputEvent::Key { code: KeyCode::STAR, down: false });
                            }
                        }
                        let was_quick = self.quick_press.map(|(pid, _)| pid) == Some(id);
                        if was_quick {
                            self.quick_press = None;
                        }
                        if self.joy.as_ref().is_some_and(|j| j.id == id) {
                            self.joy_end();
                            self.request_redraw();
                        } else if !self.ui_touches.remove(&id) {
                            self.update_touch(id, None);
                        } else if was_quick {
                            self.request_redraw();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
