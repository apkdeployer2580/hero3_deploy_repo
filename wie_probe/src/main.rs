//! Headless probe runner for the landscape (full-screen) track.
//!
//! Boots the game on the host with no window, no audio and an arbitrary
//! framebuffer size, then dumps frames as raw RGBA + the full WIPI draw-call
//! trace. This is the measurement tool `LANDSCAPE_FEASIBILITY.md` asks for:
//! "정적 disasm보다 런타임 WIPI draw-call 트레이스가 훨씬 빠름".
//!
//! usage:
//!   wie_probe --zip d:/tmp/hero3_fixed.zip --w 240 --h 320 --frames 600 \
//!             --out d:/tmp/probe --dump 60,120,300,600 \
//!             --keys 60:OK,90:OK,150:DOWN,200:OK
//!
//!   RUST_LOG=wie_wipi_c=debug wie_probe ... 2> trace.log
//!
//! ⚠ That draw-call trace needs a build with default features OFF. The default
//! `quiet_release` feature compiles every `debug!`/`trace!` in the whole graph out,
//! so that the probe measures the same code the APK runs (and so its
//! instructions/second number is comparable to the device's):
//!     cargo build --release -p wie_probe --no-default-features
//!
//! `--keys` is `frame:KEY[,frame:KEY...]`; a key is pressed on that frame and
//! released 6 frames later. Dumps land in `<out>/frame_<n>_<w>x<h>.rgba`
//! (convert with tools_re/probe_png.py).

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use wie_backend::{
    AudioSink, Database, DatabaseRepository, Emulator, Event, Filesystem, Instant, KeyCode, Options, Platform, RecordId,
    Screen, canvas::Image, extract_zip,
};
use wie_ktf::KtfEmulator;
use wie_lgt::LgtEmulator;

// ---------------------------------------------------------------- screen ----

/// Captures the last painted frame instead of showing it.
struct ProbeScreen {
    w: u32,
    h: u32,
    last: Arc<Mutex<Vec<u32>>>,
    redraw: Arc<Mutex<bool>>,
}

impl Screen for ProbeScreen {
    fn request_redraw(&self) -> wie_util::Result<()> {
        *self.redraw.lock().unwrap() = true;
        Ok(())
    }

    fn paint(&self, image: &dyn Image) {
        let data = image
            .colors()
            .iter()
            .map(|c| ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32))
            .collect::<Vec<_>>();
        *self.last.lock().unwrap() = data;
    }

    fn width(&self) -> u32 {
        self.w
    }

    fn height(&self) -> u32 {
        self.h
    }
}

// ------------------------------------------------------------ filesystem ----

#[derive(Default)]
struct MemFs {
    files: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemFs {
    fn key(aid: &str, path: &str) -> String {
        format!("{aid}:{path}")
    }
}

#[async_trait::async_trait]
impl Filesystem for MemFs {
    async fn exists(&self, aid: &str, path: &str) -> bool {
        self.files.lock().unwrap().contains_key(&Self::key(aid, path))
    }

    async fn size(&self, aid: &str, path: &str) -> Option<usize> {
        self.files.lock().unwrap().get(&Self::key(aid, path)).map(|v| v.len())
    }

    async fn read(&self, aid: &str, path: &str, offset: usize, count: usize, buf: &mut [u8]) -> Option<usize> {
        let files = self.files.lock().unwrap();
        let data = files.get(&Self::key(aid, path))?;
        if offset >= data.len() {
            return Some(0);
        }
        let n = count.min(data.len() - offset);
        buf[..n].copy_from_slice(&data[offset..offset + n]);
        Some(n)
    }

    async fn write(&self, aid: &str, path: &str, offset: usize, data: &[u8]) -> usize {
        let mut files = self.files.lock().unwrap();
        let f = files.entry(Self::key(aid, path)).or_default();
        if f.len() < offset + data.len() {
            f.resize(offset + data.len(), 0);
        }
        f[offset..offset + data.len()].copy_from_slice(data);
        data.len()
    }

    async fn truncate(&self, aid: &str, path: &str, len: usize) {
        let mut files = self.files.lock().unwrap();
        files.entry(Self::key(aid, path)).or_default().resize(len, 0);
    }
}

// -------------------------------------------------------------- database ----

type Store = HashMap<(String, String), HashMap<RecordId, Vec<u8>>>;

#[derive(Default)]
struct MemDbRepo {
    store: Arc<Mutex<Store>>,
}

struct MemDb {
    store: Arc<Mutex<Store>>,
    key: (String, String),
}

#[async_trait::async_trait]
impl DatabaseRepository for MemDbRepo {
    async fn open(&self, name: &str, app_id: &str) -> Box<dyn Database> {
        let key = (app_id.to_string(), name.to_string());
        self.store.lock().unwrap().entry(key.clone()).or_default();
        Box::new(MemDb {
            store: self.store.clone(),
            key,
        })
    }

    async fn exists(&self, name: &str, app_id: &str) -> bool {
        self.store.lock().unwrap().contains_key(&(app_id.to_string(), name.to_string()))
    }

    async fn delete(&self, name: &str, app_id: &str) -> bool {
        self.store.lock().unwrap().remove(&(app_id.to_string(), name.to_string())).is_some()
    }
}

#[async_trait::async_trait]
impl Database for MemDb {
    async fn next_id(&self) -> RecordId {
        let store = self.store.lock().unwrap();
        store[&self.key].keys().max().map(|m| m + 1).unwrap_or(1)
    }

    async fn add(&mut self, data: &[u8]) -> RecordId {
        let mut store = self.store.lock().unwrap();
        let recs = store.get_mut(&self.key).unwrap();
        let id = recs.keys().max().map(|m| m + 1).unwrap_or(1);
        recs.insert(id, data.to_vec());
        id
    }

    async fn get(&self, id: RecordId) -> Option<Vec<u8>> {
        self.store.lock().unwrap()[&self.key].get(&id).cloned()
    }

    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool {
        let mut store = self.store.lock().unwrap();
        store.get_mut(&self.key).unwrap().insert(id, data.to_vec());
        true
    }

    async fn delete(&mut self, id: RecordId) -> bool {
        let mut store = self.store.lock().unwrap();
        store.get_mut(&self.key).unwrap().remove(&id).is_some()
    }

    async fn get_record_ids(&self) -> Vec<RecordId> {
        let mut ids: Vec<RecordId> = self.store.lock().unwrap()[&self.key].keys().copied().collect();
        ids.sort();
        ids
    }
}

// ------------------------------------------------------ filesystem-backed DB ----

/// `<root>/<app_id>/db/<name>/<record_id>` — byte-identical layout to
/// `wie_android::database`, so a device backup can be dropped in as-is.
struct FsDbRepo {
    root: PathBuf,
}

struct FsDb {
    dir: PathBuf,
}

impl FsDbRepo {
    fn dir_for(&self, name: &str, app_id: &str) -> PathBuf {
        self.root.join(app_id).join("db").join(name.trim_start_matches('/'))
    }
}

#[async_trait::async_trait]
impl DatabaseRepository for FsDbRepo {
    async fn open(&self, name: &str, app_id: &str) -> Box<dyn Database> {
        let dir = self.dir_for(name, app_id);
        eprintln!("[probe] DB open name={name:?} app_id={app_id:?} -> {dir:?} (exists={})", dir.is_dir());
        let _ = fs::create_dir_all(&dir);
        Box::new(FsDb { dir })
    }

    async fn exists(&self, name: &str, app_id: &str) -> bool {
        let e = self.dir_for(name, app_id).is_dir();
        eprintln!("[probe] DB exists name={name:?} app_id={app_id:?} -> {e}");
        e
    }

    async fn delete(&self, name: &str, app_id: &str) -> bool {
        fs::remove_dir_all(self.dir_for(name, app_id)).is_ok()
    }
}

impl FsDb {
    fn ids(&self) -> Vec<RecordId> {
        let mut v: Vec<RecordId> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_string_lossy().parse().ok())
            .collect();
        v.sort();
        v
    }
}

#[async_trait::async_trait]
impl Database for FsDb {
    async fn next_id(&self) -> RecordId {
        self.ids().last().map(|m| m + 1).unwrap_or(1)
    }

    async fn add(&mut self, data: &[u8]) -> RecordId {
        let id = self.ids().last().map(|m| m + 1).unwrap_or(1);
        let _ = fs::write(self.dir.join(id.to_string()), data);
        id
    }

    async fn get(&self, id: RecordId) -> Option<Vec<u8>> {
        fs::read(self.dir.join(id.to_string())).ok()
    }

    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool {
        fs::write(self.dir.join(id.to_string()), data).is_ok()
    }

    async fn delete(&mut self, id: RecordId) -> bool {
        fs::remove_file(self.dir.join(id.to_string())).is_ok()
    }

    async fn get_record_ids(&self) -> Vec<RecordId> {
        self.ids()
    }
}

// -------------------------------------------------------------- platform ----

struct NoAudio;

impl AudioSink for NoAudio {
    fn play_wave(&self, _c: u8, _r: u32, _d: &[i16]) {}
    fn midi_note_on(&self, _c: u8, _n: u8, _v: u8) {}
    fn midi_note_off(&self, _c: u8, _n: u8, _v: u8) {}
    fn midi_program_change(&self, _c: u8, _p: u8) {}
    fn midi_control_change(&self, _c: u8, _ctl: u8, _v: u8) {}
    fn midi_pitch_bend(&self, _c: u8, _v: u16) {}
    fn midi_sysex(&self, _d: &[u8]) {}
}

struct ProbePlatform {
    screen: ProbeScreen,
    fs: MemFs,
    db: Box<dyn DatabaseRepository + Send + Sync>,
    /// Virtual clock (ms). The guest arms 50 ms WIPI timers and then idles, so a
    /// wall-clock `now()` would never fire them in a headless run that ticks as
    /// fast as it can. Driving time from the frame counter also makes runs
    /// deterministic.
    clock: Arc<AtomicU64>,
    /// `--net-offline`: the app's in-app server behind the guest's sockets, or `None`
    /// (every net slot fails cleanly — the probe's default).
    net: Option<probe_net::ProbeNet>,
}

impl Platform for ProbePlatform {
    fn network(&self) -> Option<&dyn wie_backend::Network> {
        self.net.as_ref().map(|n| n as &dyn wie_backend::Network)
    }

    fn screen(&self) -> &dyn Screen {
        &self.screen
    }

    fn now(&self) -> Instant {
        Instant::from_epoch_millis(self.clock.load(Ordering::SeqCst))
    }

    fn database_repository(&self) -> &dyn DatabaseRepository {
        self.db.as_ref()
    }

    fn filesystem(&self) -> &dyn Filesystem {
        &self.fs
    }

    fn audio_sink(&self) -> Box<dyn AudioSink> {
        Box::new(NoAudio)
    }

    fn write_stdout(&self, buf: &[u8]) {
        print!("{}", String::from_utf8_lossy(buf));
    }

    fn write_stderr(&self, buf: &[u8]) {
        eprint!("{}", String::from_utf8_lossy(buf));
    }

    fn exit(&self) {
        eprintln!("[probe] guest called exit()");
    }

    fn vibrate(&self, _duration_ms: u64, _intensity: u8) {}
}

// ------------------------------------------------------ shipping minimap ----
//
// `--map-check` diffs the app's OWN minimap renderer against the framebuffer the
// guest paints, so the source is included rather than mirrored: a check that
// exercises a copy proves nothing about what ships.
include!("../../wie_android/src/minimap_render.rs");

// The app's REAL portrait rules (141차): where the capture window goes and which rows
// are the head. `--portrait` arms the same window the worker arms and dumps what the
// host blitter put in it.
#[allow(dead_code)]
mod portrait {
    include!("../../wie_android/src/portrait.rs");
}

// ------------------------------------------------------- shipping anchors ----
//
// Same reasoning, for the per-build guest anchors (CHAR base, state offset, the
// handle-vs-direct pointer convention). The probe used to hard-code KTF's
// `0x1b3084` / `0x1b7b6c` / `+0x7c` in five places, so every introspection flag
// read unrelated memory on an LGT boot (MIGRATION_LGT.md §3.4).
// `Skt` is never constructed here — the probe only boots KTF/LGT archives — but the
// table is shared source, so the variant has to exist.
#[allow(dead_code)]
mod guest_build {
    include!("../../wie_android/src/guest_anchors.rs");
}
use guest_build::{CharAnchor, GuestBuild};

// The app's REAL save-chain step machine (`Job`), not a copy — same contract as the
// other `include!`s above. `--host-save` drives it through `call_guest_function_out`,
// which is the device-free end-to-end of MIGRATION_LGT.md §4-3: the probe issues the
// same four calls the app's quick-save issues, and the game itself is the judge (the
// record must land in `--db` and load on the next boot).
mod host_save {
    include!("../../wie_android/src/save.rs");
}

// The app's REAL held-attack-key rule (`AttackHold`, 132차-B), not a copy. `--hold-key`
// drives it against the live game the way the worker's loop step 3b does, which is the
// only end-to-end the hold has: the worker itself cannot run here.
#[allow(dead_code)]
mod atkspeed {
    include!("../../wie_android/src/atkspeed.rs");
}

// The app's REAL in-app server (`offline.rs`, 60차/99차) behind the guest's sockets —
// `--net-offline`. `offline.rs` resolves its catalog as `crate::cash_catalog`, which is
// why the generated table sits at this crate's root the same way it does in wie_android.
// ★ 171차: the avatar catalog, for the own-head mask the app publishes at boot.
#[allow(dead_code)]
mod avatar_catalog {
    include!("../../wie_android/src/avatar_catalog.rs");
}
#[allow(dead_code)]
mod cash_catalog {
    include!("../../wie_android/src/cash_catalog.rs");
}
#[allow(dead_code)]
mod offline {
    include!("../../wie_android/src/offline.rs");
}
mod probe_net;

/// CHAR base for the build that booted, or 0 when this build has no known anchor.
///
/// Deliberately WITHOUT `items::resolve_char`'s "is a character alive yet" content
/// test: that test is built from CHAR-record offsets that live in `items.rs`, and a
/// diagnostic tool wants the raw base even when the record reads dead — that is the
/// state one is usually debugging.
fn char_base(build: GuestBuild, read: &impl Fn(u32, usize) -> Vec<u8>) -> u32 {
    let u32at = |a: u32| -> u32 {
        let b = read(a, 4);
        if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
    };
    match build.char_anchor() {
        Some(CharAnchor::Pointer { global, fallback }) => {
            let p = u32at(global);
            if (0x10_0000..0x8000_0000).contains(&p) { p } else { fallback }
        }
        Some(CharAnchor::Static(base)) => base,
        None => 0,
    }
}

/// `items::attack_hold_view`, for `--hold-key`. The CHAR offsets are mirrored from
/// items.rs (not include!d here — the same convention as `--verify-offsets`):
/// window id `+0xa0c0` == 3, app state == 4 (the field), menu overlay `+0x9c6c` == 0,
/// choice flag `+0xa224`, the modal word, and the player's state byte `+0x28+0x9b8e`.
fn hold_view(build: GuestBuild, read: &impl Fn(u32, usize) -> Vec<u8>) -> atkspeed::HoldView {
    let c = char_base(build, read);
    let u8at = |a: u32| read(a, 1).first().copied();
    let u32at = |a: u32| {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let field = c != 0
        && u8at(c + 0xa0c0) == Some(3)
        && u32at(c + build.state_off()) == Some(4)
        && u8at(c + 0x9c6c) == Some(0);
    let boxed = (c != 0 && u8at(c + 0xa224).unwrap_or(0) != 0)
        || build
            .net_disconnect_modal()
            .and_then(|m| u32at(m.state_word))
            .is_some_and(|m| m != 0);
    let state = u8at(c + 0x28 + 0x9b8e);
    let idle = field && !boxed && state == Some(0);
    // 132차-C: state 2 with a live step `+0x24f` whose next slot in the script `+0x23f[8]`
    // is still the -1 terminator = an OK now would chain the combo.
    let combo_open = if field && !boxed && state == Some(2) {
        let step = u8at(c + 0x24f).map(|b| b as i8);
        let script = read(c + 0x23f, 8);
        match step {
            Some(s) if s >= 0 && (s as usize) + 1 < 8 && script.len() == 8 => (script[s as usize + 1] as i8 == -1).then_some(s),
            _ => None,
        }
    } else {
        None
    };
    atkspeed::HoldView { idle, ui: !field || boxed, combo_open }
}

/// The map chain for this build, assembled the same way `items::map_chain` does.
fn map_chain_of(build: GuestBuild) -> MapChain {
    MapChain {
        data_off: build.container_data_off(),
        noblend_global: build.minimap_noblend_global(),
    }
}

/// Where the app's worker arms the portrait window this frame (`items::portrait_arm_window`,
/// 141차 · 153차): the player's draw corner (`obj+0x16`/`+0x18`, world px) plus the field
/// scroll — the camera the 「화면 이동」 option picks (`0x150e985 == 1` → the smoothed
/// `0x150d91c`, else the target `0x150d914`), clamped and a small map centred by
/// `portrait::field_scroll`. `(window origin, shadow anchor, a diagnostic line)`.
fn probe_portrait_window(
    build: GuestBuild,
    chain: &MapChain,
    rd: &impl Fn(u32, usize) -> Vec<u8>,
    screen: (i32, i32),
) -> Option<((i32, i32), (i32, i32), String)> {
    let u32at = |a: u32| -> Option<u32> {
        let b = rd(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let i16at = |a: u32| -> Option<i16> {
        let b = rd(a, 2);
        (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
    };
    let hop = |p: u32| -> Option<u32> {
        let first = u32at(p).filter(|v| *v != 0)?;
        match chain.data_off {
            Some(off) => u32at(first).map(|v| v.wrapping_add(off)),
            None => Some(first),
        }
    };
    let cp = char_base(build, rd);
    let obj = hop(cp + PLAYER_OBJ_HANDLE_OFF)?;
    let (dx, dy) = (i16at(obj + 0x16)?, i16at(obj + 0x18)?);
    let (px, py) = (i16at(obj + 0x10)?, i16at(obj + 0x12)?);
    let smooth = rd(0x150e985, 1).first().map(|b| *b as i8 == 1)?;
    let cam = if smooth {
        (u32at(0x150d91c)? as i32, u32at(0x150d920)? as i32)
    } else {
        (u32at(0x150d914)? as i32, u32at(0x150d918)? as i32)
    };
    // The map in px, through the same chain as the object (`MAP_HANDLE_OFF` → w/h at +0x48/+0x4c).
    let map = hop(cp + MAP_HANDLE_OFF)?;
    let map_px = (u32at(map + 0x48)? as i32 * 16, u32at(map + 0x4c)? as i32 * 16);
    let scroll = portrait::field_scroll(cam, map_px, screen);
    let (sdx, sdy) = (i32::from(dx) + scroll.0, i32::from(dy) + scroll.1);
    let origin = portrait::portrait_origin(sdx, sdy);
    let anchor = portrait::shadow_anchor(sdx, sdy);
    let line = format!(
        "obj={obj:#x} pos=({px},{py}) draw=({dx},{dy}) cam={cam:?} ({}) map={map_px:?} scroll={scroll:?} -> draw on screen ({sdx},{sdy}) window origin {origin:?} shadow anchor {anchor:?}",
        if smooth { "smoothed" } else { "target" }
    );
    Some((origin, anchor, line))
}

/// Where this build keeps its statics — the regions to sweep when hunting for a
/// global that points at something. KTF is RWPI (everything sits around
/// `sl = 0x1b2c40`); LGT is absolute, with `.data` and `.bss` at fixed addresses and
/// the whole CHAR record living inside `.bss`.
fn statics_ranges(build: GuestBuild) -> &'static [(u32, usize)] {
    match build {
        GuestBuild::Ktf => &[(0x1b_0000, 0x1_8000)],
        _ => &[(0x140_0000, 0x604), (0x150_0000, 0x1_2958)],
    }
}

// ------------------------------------------------------- stack high-water ----
//
// ★ The one gauge that cannot miss the leak (2026-08-06). The device crash is a
// native stack overflow on `wie-emu` whose backtrace is 500+ frames of two functions
// alternating, and every in-process counter we tried measures the WRONG layer:
// `ArmCoreThreadWrapper` nesting is 1 by construction, `run_function` nesting tops
// out at 21, and the executor's task count stays under 8. So measure the thing that
// actually overflows — the stack itself — from outside the code that uses it.
//
// Windows commits stack pages lazily and NEVER decommits them, so the still-RESERVED
// region at the base of the stack allocation only ever shrinks, and its size is an
// exact high-water mark for the whole run. A healthy emulator settles; a leaking one
// eats the reserve at a steady rate, and the rate says how long the device had.
#[cfg(windows)]
mod stack_gauge {
    use core::ffi::c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition_id: u16,
        _pad0: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        type_: u32,
        _pad1: u32,
    }

    impl MemoryBasicInformation {
        fn zeroed() -> Self {
            unsafe { core::mem::zeroed() }
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(address: *const c_void, buffer: *mut MemoryBasicInformation, length: usize) -> usize;
    }

    /// Bytes of this thread's stack still RESERVED (never touched). Monotonically
    /// non-increasing; `initial - current` is the deepest the stack has ever gone.
    pub fn reserved_bytes() -> Option<usize> {
        let probe = 0u8;
        let size = size_of::<MemoryBasicInformation>();
        let mut here = MemoryBasicInformation::zeroed();
        if unsafe { VirtualQuery(&probe as *const u8 as *const c_void, &mut here, size) } == 0 {
            return None;
        }
        let mut base = MemoryBasicInformation::zeroed();
        if unsafe { VirtualQuery(here.allocation_base as *const c_void, &mut base, size) } == 0 {
            return None;
        }
        // MEM_RESERVE = 0x2000. Anything else means the base region is already
        // committed, i.e. the whole stack has been touched.
        (base.state == 0x2000).then_some(base.region_size)
    }
}

#[cfg(not(windows))]
mod stack_gauge {
    pub fn reserved_bytes() -> Option<usize> {
        None
    }
}

// ------------------------------------------------------------------ main ----

fn key_of(name: &str) -> Option<KeyCode> {
    Some(match name.to_ascii_uppercase().as_str() {
        "UP" => KeyCode::UP,
        "DOWN" => KeyCode::DOWN,
        "LEFT" => KeyCode::LEFT,
        "RIGHT" => KeyCode::RIGHT,
        "OK" => KeyCode::OK,
        "LSK" => KeyCode::LEFT_SOFT_KEY,
        "RSK" => KeyCode::RIGHT_SOFT_KEY,
        "CLEAR" | "CLR" => KeyCode::CLEAR,
        "NUM0" | "0" => KeyCode::NUM0,
        "NUM1" | "1" => KeyCode::NUM1,
        "NUM2" | "2" => KeyCode::NUM2,
        "NUM3" | "3" => KeyCode::NUM3,
        "NUM4" | "4" => KeyCode::NUM4,
        "NUM5" | "5" => KeyCode::NUM5,
        "NUM6" | "6" => KeyCode::NUM6,
        "NUM7" | "7" => KeyCode::NUM7,
        "NUM8" | "8" => KeyCode::NUM8,
        "NUM9" | "9" => KeyCode::NUM9,
        "HASH" | "#" => KeyCode::HASH,
        "STAR" | "*" => KeyCode::STAR,
        _ => return None,
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    // --- args ---
    let mut zip = PathBuf::from("d:/tmp/hero3_fixed.zip");
    let mut out = PathBuf::from("d:/tmp/probe");
    let (mut w, mut h, mut frames) = (240u32, 320u32, 400u64);
    let mut ms_per_frame = 20u64;
    let mut db_root: Option<PathBuf> = None;
    let mut find_fb = false;
    let mut map_check = false;
    // --map-hunt: locate the per-map object and its tile arrays on a build whose map
    // chain has not been reverse engineered yet. Structural, not address-based: it
    // walks every static/CHAR word, tries both pointer conventions, and reports the
    // ones that land on something map-SHAPED (plausible tile w/h at +0x48/+0x4c with
    // two readable w*h byte arrays at +0x60/+0x68). See MIGRATION_LGT.md §4 item 1.
    let mut map_hunt = false;
    let mut profile = false;
    let mut hle = true;
    let mut char_dump: Option<PathBuf> = None;
    let mut trace_state = false;
    // `--anim <from>:<to>` — one line per frame of the PLAYER's animation clock, the
    // instrument the attack-speed work needs (TODO `DT`). The field state machine
    // `0x5089c` bumps the pose frame counter `base+0x9ba0` once per tick and each arm
    // ends its step when that counter equals `0x4aee0(...)` (= the pose's frame count
    // minus one), so an action's length in ticks is only visible as a per-frame trace.
    // Read-only.
    // `--atk-speed <pct>` — arm the 공격속도 knob the `atk_speed_lgt` hook reads, exactly
    // as the worker does from worn equipment or `wie_attack_speed.txt`. `0` = the
    // original timing, and then the hook hands the guest its own number back.
    let mut atk_speed_pct: u32 = 0;
    // `--set-bonus <frame>:<hexmask>` (142차) — publish the 85제 set mask the way the app's
    // worker does (`wie_util::stats::set_set_bonus`) at that frame. The play clock follows
    // the virtual clock, so 수호의 맹세's 5 s / 100 s run in probe time.
    let mut set_bonus_at: Vec<(u64, u32)> = vec![];
    // ★ 156차 — `--pierce <frame>:<pct>[:spec]`: publish the 방패무시 total the way the app's
    // worker does (`wie_util::stats::set_pierce`) at that frame; `spec` = the main weapon
    // resolves against the special defence (categories 7+). The `dmg_apply_lgt` hook then
    // gives every real hit back `def × pct / 100` of the record's defence.
    let mut pierce_at: Vec<(u64, u32, bool)> = vec![];
    // ★ 160차 — `--stat-wide <frame>:<slot>:<value>`: publish one of the four wide stats the
    // way the `statwide` getter hook does (slot 0 ATT1 · 1 ATT2 · 2 물리방어 · 3 특수방어) and
    // write the halfword the game would hold for it (saturated). The two combat readers then
    // take the true value back; with the halfword alone a hit starts from at most 32767.
    let mut stat_wide_at: Vec<(u64, usize, i32)> = vec![];
    // ★ 150차 — the reworked accessories' third options.
    // `--acc-effects <frame>:<taken>:<dealt>:<kill>:<hit>[,…]` — publish the worn accessories'
    // effect word at that frame (`stats::set_acc_effects`), the one the app's worker sums off the
    // nine worn slots: 피해감소 % (이안의맹세), 주는 피해 % (검은 눈물), 처치 / 피격 시 20초간
    // 최대 HP % (레아의목걸이 / 굴베이그의탈리스만). The damage hooks read it on every hit.
    let mut acc_effects_at: Vec<(u64, wie_util::stats::AccEffects)> = vec![];
    // `--third-opts 0|1` — the `acc_third_stat_lgt` A/B switch (`stats::THIRD_OPTS_OFF`): 0 = the
    // wear recompute adds no `Stat` third, so a run pair isolates exactly the third's share.
    let mut third_opts = true;
    // `--regen <frame>:<pct>` — 레아의목걸이's 「5초당 HP회복 +N%」 the way the host runs it: from
    // that frame, every `acc_options::REGEN_PERIOD_MS` of the play clock (`stats::regen_tick`),
    // when the true HP (`stats::player_hp_view` of the gauge) is below the true max, call the
    // game's own `addHP 0x4b9c1(STAT_BLOCK, stats::regen_amount(hp, max, pct))` — a host-made
    // call, so the replaced `addHP` takes the delta in TRUE HP (not a relative caller). Unlike
    // the app it does not wait for the field (the run decides what is on screen); the dead and
    // the full are `regen_amount`'s own 0.
    let mut regen_at: Option<(u64, u32)> = None;
    let mut anim_window: Option<(u64, u64)> = None;
    // `--hold-key <down>:<up>[:KEY],…` — hold KEY (default OK) from frame `down` to `up`
    // the way a thumb does on the device: ONE keydown at `down`, ONE keyup at `up`, and in
    // between whatever `atkspeed::AttackHold` decides — the worker's step 3b, emulated one
    // probe frame per worker iteration (the probe frame IS the virtual time step). The view
    // is read with the same offsets `items::attack_hold_view` uses. Prints `[hold]` lines.
    let mut hold_keys: Vec<(u64, u64, KeyCode)> = vec![];
    // `--hold-key-legacy 1` — run vc401's rule instead (re-press whenever the field is idle,
    // every 33 ms, from the iteration of the press on, no UI memory), to show what 132차-B
    // changed in the same scenario.
    let mut hold_legacy = false;
    // --walk-cal 1 — 탭-투-무브 calibration series (61차): every 4 frames, one line of
    // camera/scroll globals + the player's tile, so a scripted walk turns into a
    // fittable (tile -> camera) table. LGT-only: the camera globals are the LGT
    // statics from MIGRATION_LGT.md §2.7 (.bss 0x150d914/18, second pair +8), found
    // by single-word run isolation — no KTF equivalent is known.
    let mut walk_cal = false;
    let mut walk_cal_map = (0i32, 0i32);
    // --verify-offsets 1 — on every --dump frame, print the MIGRATION_LGT.md §2.11
    // matrix in one shot: the VERIFIED reference fields (gauges / window / state, so
    // the base can be confirmed against the on-screen status numbers) and then the five
    // 🔶 "silent-bug watch" offsets (quickbar cells, badges, secondary weapon, save
    // byte, skill-cast) decoded exactly as items.rs reads them. This is the whole point
    // of the audit made runnable: a local session opens the status screen and compares
    // these labelled values to the screen, closing all five 🔶 items in one command
    // instead of doing offset arithmetic by hand on a raw --char-dump (which is itself
    // a silent-bug risk). Reads are build-aware via the shared guest_build table.
    let mut verify_offsets = false;
    // --dump-mem base:len[,base:len...] — snapshot raw guest memory at every --dump
    // frame into <out>/mem_<frame>_<base>.bin. The RE workhorse: diff two frames'
    // snapshots to find the word that changed (HP dropped, cursor moved, …) with no
    // device. Ranges are hex; a good default for the LGT ELF is its statics:
    //   --dump-mem 0x1400000:0x604,0x1500000:0x12958
    let mut dump_mem: Vec<(u32, usize)> = vec![];
    let mut dumps: Vec<u64> = vec![];
    // --poke frame:addr:hexbytes[,…] — write guest memory at the start of a frame.
    //
    // The bench for a candidate guest CODE patch before anything ships it: a probe run
    // is deterministic, so poking a patch in and diffing the frames (and the CHAR
    // record) against an unpoked run says exactly what the patch does and what else it
    // disturbs. Used to qualify the LGT HUD-skip branch (MIGRATION_LGT.md §4 item 2);
    // the app's own patch functions verify the ORIGINAL bytes before writing, and this
    // prints them so that check can be written down from measurement.
    let mut pokes: Vec<(u64, u32, Vec<u8>)> = vec![];
    // --avatar-preview <frame>:<slot>:<set|->[,...] — 167차: arm (or drop, `-`) one avatar
    // slot's try-on knob at that frame, the way the AVATAR page does while its cursor moves.
    let mut avatar_previews: Vec<(u64, usize, Option<u8>)> = vec![];
    // --host-save <frame> — at that frame, start the app's own four-step save chain
    // (serialise → write GameSave → summary → write SlotSave) with the per-build
    // `guest_anchors::save_chain()` addresses, driven exactly like `emulator.rs` does.
    // Run it from a FIELD frame (no window open) — the same precondition the app's
    // `save_gate` enforces.
    let mut host_save_frame: Option<u64> = None;
    // --host-levelup <frame> — at that frame, deliver one 천상의 영약 grant exactly the
    // way `emulator.rs` does: exp = expToNext + 1, then the game's own checkLevelUp
    // (`guest_anchors::level_up_fn`). Run it from a FIELD frame. The frames after it
    // watch the level word and print the before/after plus the stat/skill point
    // deltas, so one deterministic run verifies the whole mechanism headlessly.
    let mut host_levelup_frame: Option<u64> = None;
    let mut host_scene: Option<(u64, u32)> = None;
    let mut host_scene_tail: Vec<u32> = vec![];
    let mut host_warp: Vec<(u64, u32, Option<u8>)> = vec![];
    // `--creatures <frame>[,<frame>...]` — at each of those frames, dereference the
    // live creature array (`*(CHAR + SceneWorld::creatures_off)`) and hexdump one
    // `creature_stride` record per spawned monster, with the fields that are already
    // confirmed decoded next to the raw bytes.
    //
    // This is the RE workhorse for `GUARDIAN_CASTLE_FEASIBILITY.md` item 3: two of
    // these frames around a hit are a field-level diff of the record with no device,
    // and `--host-warp` into a one-monster boss scene makes the setup a single run.
    // The record count comes from the MAP object (`map_spawn_count_off`), which is
    // what the loader actually spawned, so a scene with none prints nothing rather
    // than reading whatever follows the array.
    let mut creature_frames: Vec<u64> = vec![];
    // `--blit-log <frame>[,<frame>...]` — print every host-blitter call of those frames
    // (141차: how the portrait window was sized).
    let mut blit_log_frames: Vec<u64> = vec![];
    // `--portrait <frame>[,<frame>...]` — arm the portrait window from the player object
    // and the camera before that frame's tick, dump the capture after it (141차).
    let mut portrait_frames: Vec<u64> = vec![];
    // `--portrait-keep <frame>` (153차) — from that frame on, run the worker's ring logic
    // every frame: arm the window the way `items::portrait_arm_window` does, and after the
    // tick offer the face square of every canonical frame (idle · facing the viewer · the
    // idle loop's first frame · a run the player was drawn in · nothing blended) to the
    // app's own `PortraitKeeper`. One `[keep]` line per change of the ring's picture (its
    // face square dumped as `keep_<frame>_24x24.rgba`) and a summary at the end — the
    // headless proof that standing, walking and re-dressing move it exactly when they should.
    let mut portrait_keep_from: Option<u64> = None;
    // `--find-word <frame>:<hex>[,<hex>...]` — scan the guest .bss for 32-bit words equal to any of
    // the values after that frame's tick and print where they sit (141차: locating the scroll word).
    let mut find_words: Vec<(u64, Vec<u32>)> = vec![];
    // `--player <frame>[,<frame>...]` — the PLAYER OBJECT, which is what decides
    // where a warp lands. `0x56134` (the placement routine the warp handler calls
    // after `loadScene`) reads `(i16)[obj+0x10]` / `(i16)[obj+0x12]` and only NUDGES
    // them by the entry point, so a warp does NOT move the player: they keep the
    // coordinates they had in the previous map. This lever prints those two words
    // next to the tile bytes the map window draws its marker from (`+0x27`/`+0x28`)
    // and the destination map's size, which is the whole of that claim in one line.
    let mut player_frames: Vec<u64> = vec![];
    // `--host-place <frame>:<tx>:<ty>` — write the player onto a tile, the way a fix
    // would have to. Posted BEFORE the warp request so the game's own placement runs
    // on the new values; run it at the same frame as `--host-warp` to test that.
    let mut host_place: Vec<(u64, i32, i32)> = vec![];
    let mut crea_sets: Vec<(u64, Option<i32>, u32, Vec<u8>)> = vec![];
    // `--crea-hit <frame>:<idx>:<delta>` — run the game's OWN damage path on a live
    // record: `0x38620(rec, delta)` adds `delta` to `hp_cur`, clamps at 0, and a kill
    // goes through AI state 7 and the loot roll. Writing 0 into HP does NOT kill
    // (CLAUDE.md), so this is the only way a headless run can produce a corpse — and
    // therefore the only way to look at what a dead boss leaves on screen.
    let mut crea_hits: Vec<(u64, i32, i32)> = vec![];
    // `--crea-hurt <frame>:<idx>:<dmg>` — the game's OWN `hitMonster 0x3863c(rec, 0, dmg)`,
    // one level above `--crea-hit`: the number display, the `dmg_apply_lgt` hook (i16
    // clamp + the record's virtual HP word) and then `0x38620`. `dmg` is the 32-bit value
    // the patched resolver would have produced, so this is how a > 32767 hit is tested
    // without waiting for the player to land one.
    let mut crea_hurts: Vec<(u64, i32, u32)> = vec![];
    // `(frame due, idx, record, dmg)` — a `--crea-hurt` call whose record is read back on
    // the frame after the call (the guest call is async, see the lever).
    let mut hurt_pending: Vec<(u64, i32, u32, u32)> = vec![];
    // `--crea-loot <frame>:<idx>` — call the game's OWN loot routine `0x38e70(enemy)` on a live
    // record, bypassing the death animation. This is the routine the `reaver_drop_lgt` hook sits
    // in, so it exercises the whole hook chain (frame check, species read, difficulty gate, dice,
    // append, spawn) on any monster, without needing the AI to reach state 7.
    let mut crea_loots: Vec<(u64, i32)> = vec![];
    // `--host-give <frame>:<cat>:<idx>[:n[:announce]]` — the game's own
    // `giveItem 0x52b14(cat, idx, -1, 0)`, n times: the same primitive the app's grants
    // use. With `--bagbox <hexmask>` the `bagbox_give_lgt` entry hook is armed for those
    // categories (room 48 per box), so the give lands in the host's ring instead of a bag
    // cell — `--bag <frames>` prints both.
    //   `announce` is the third argument, decimal, default -1: the ANNOUNCE FLAG (103차).
    // -1 is what the host's grants pass and what the game passes for a scripted give; any
    // other value is what the drop the player walks over passes (`0x54a5a` hands it the
    // loot's index), and it makes `giveItem` append the item's name to the floating notice
    // queue `--bag` prints. That A/B is how the queue's offsets were checked against the
    // real game rather than against the disassembly alone.
    let mut host_gives: Vec<(u64, u16, u16, u32, u32)> = vec![];
    let mut bagbox_mask: u32 = 0;
    let mut bagbox_counts: Vec<(u32, u32, u32)> = vec![];
    let mut vm_items: Vec<(u64, u32, u32, u32, u32)> = vec![];
    let mut restages: Vec<(u64, u8, u8, u16, u16, bool)> = vec![];
    let mut use_items: Vec<(u64, u32, u32)> = vec![];
    let mut guest_calls: Vec<(u64, u32, Vec<u32>)> = vec![];
    let mut bag_frames: Vec<u64> = vec![];
    // `--loot a:b` — the field's dropped-item records, printed whenever the SET changes
    // (count / which items), with every record's lifetime counter (162차 measurement).
    let mut loot_watch: Option<(u64, u64)> = None;
    let mut loot_last: Option<String> = None;
    let mut loot_prev: Vec<String> = vec![];
    // `--net-offline 1` — the app's in-app server behind the guest's sockets, every frame
    // logged (`probe_net.rs`). Off = no network, every net slot fails (the default).
    let mut net_offline = false;
    let mut keys: Vec<(u64, KeyCode, u64)> = vec![];
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i + 1 < argv.len() {
        let (k, v) = (argv[i].as_str(), argv[i + 1].as_str());
        match k {
            "--zip" => zip = PathBuf::from(v),
            "--out" => out = PathBuf::from(v),
            "--w" => w = v.parse().unwrap(),
            "--h" => h = v.parse().unwrap(),
            "--frames" => frames = v.parse().unwrap(),
            "--ms" => ms_per_frame = v.parse().unwrap(),
            "--db" => db_root = Some(PathBuf::from(v)),
            "--find-fb" => find_fb = v == "1" || v == "true",
            "--map-check" => map_check = v == "1" || v == "true",
            "--map-hunt" => map_hunt = v == "1" || v == "true",
            "--profile" => profile = v == "1" || v == "true",
            "--hle" => hle = v == "1" || v == "true",
            "--char-dump" => char_dump = Some(PathBuf::from(v)),
            "--trace-state" => trace_state = v == "1",
            "--atk-speed" => atk_speed_pct = v.parse().unwrap_or(0),
            "--set-bonus" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let f = it.next().and_then(|s| s.trim().parse::<u64>().ok());
                    let m = it.next().and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok());
                    match (f, m) {
                        (Some(f), Some(m)) => set_bonus_at.push((f, m)),
                        _ => eprintln!("[probe] bad --set-bonus {spec:?} (want frame:hexmask)"),
                    }
                }
            }
            "--pierce" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let f = it.next().and_then(|s| s.trim().parse::<u64>().ok());
                    let pct = it.next().and_then(|s| s.trim().parse::<u32>().ok());
                    let special = it.next().map(|s| s.trim() == "spec").unwrap_or(false);
                    match (f, pct) {
                        (Some(f), Some(pct)) => pierce_at.push((f, pct, special)),
                        _ => eprintln!("[probe] bad --pierce {spec:?} (want frame:pct[:spec])"),
                    }
                }
            }
            "--stat-wide" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let f = it.next().and_then(|s| s.trim().parse::<u64>().ok());
                    let slot = it.next().and_then(|s| s.trim().parse::<usize>().ok()).filter(|s| *s < 4);
                    let value = it.next().and_then(|s| s.trim().parse::<i32>().ok());
                    match (f, slot, value) {
                        (Some(f), Some(slot), Some(value)) => stat_wide_at.push((f, slot, value)),
                        _ => eprintln!("[probe] bad --stat-wide {spec:?} (want frame:slot(0..3):value)"),
                    }
                }
            }
            "--acc-effects" => {
                for spec in v.split(',') {
                    let n: Vec<Option<u64>> = spec.split(':').map(|s| s.trim().parse::<u64>().ok()).collect();
                    match n.as_slice() {
                        [Some(f), Some(t), Some(d), Some(k), Some(h)] => acc_effects_at.push((
                            *f,
                            wie_util::stats::AccEffects {
                                taken_pct: (*t).min(255) as u8,
                                dealt_pct: (*d).min(255) as u8,
                                kill_max_hp_pct: (*k).min(255) as u8,
                                hit_max_hp_pct: (*h).min(255) as u8,
                            },
                        )),
                        _ => eprintln!("[probe] bad --acc-effects {spec:?} (want frame:taken:dealt:kill:hit)"),
                    }
                }
            }
            "--third-opts" => third_opts = !(v == "0" || v == "false"),
            "--regen" => {
                let mut it = v.split(':');
                match (it.next().and_then(|s| s.trim().parse::<u64>().ok()), it.next().and_then(|s| s.trim().parse::<u32>().ok())) {
                    (Some(f), Some(p)) => regen_at = Some((f, p)),
                    _ => eprintln!("[probe] bad --regen {v:?} (want frame:pct)"),
                }
            }
            "--anim" => {
                let mut it = v.split(':');
                if let (Some(a), Some(b)) = (it.next(), it.next()) {
                    if let (Ok(a), Ok(b)) = (a.parse::<u64>(), b.parse::<u64>()) {
                        anim_window = Some((a, b));
                    }
                }
            }
            "--hold-key-legacy" => hold_legacy = v == "1",
            "--hold-key" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(a), Some(b)) = (it.next(), it.next()) else { continue };
                    let kc = it.next().map_or(Some(KeyCode::OK), key_of);
                    match (a.parse::<u64>(), b.parse::<u64>(), kc) {
                        (Ok(a), Ok(b), Some(kc)) if a < b => hold_keys.push((a, b, kc)),
                        _ => eprintln!("[probe] bad --hold-key {spec:?} (want down:up[:KEY], down < up)"),
                    }
                }
            }
            "--walk-cal" => walk_cal = v == "1" || v == "true",
            "--verify-offsets" => verify_offsets = v == "1" || v == "true",
            "--host-save" => host_save_frame = v.parse().ok(),
            "--host-levelup" => host_levelup_frame = v.parse().ok(),
            // `--host-warp <frame>:<scene>` — post a warp REQUEST the way the game's
            // own warp tiles do (two bytes into the scene control block) and let the
            // handler at 0x3f1d4 run. Unlike `--host-scene` this leaves the world
            // drawable, because everything around the load is the game's own code.
            "--host-warp" => {
              for spec in v.split(',') {
                let mut it = spec.split(':');
                let f = it.next().and_then(|s| s.parse::<u64>().ok());
                let s = it.next().and_then(|s| s.parse::<u32>().ok());
                // Optional third field = the ENTRY POINT byte
                // (`SceneWorld::warp_entry_off`). Omit it and the run leaves that byte
                // exactly as the game left it, which is what the shipped warps do.
                let e = it.next().and_then(|s| s.parse::<u8>().ok());
                if let (Some(f), Some(s)) = (f, s) {
                    host_warp.push((f, s, e));
                }
              }
            }
            // `--host-scene <frame>:<scene>` — call the game's own loadScene and watch
            // the map id change. This is the runtime half of the claim in
            // `guest_anchors::SceneWorld`: static disassembly says one halfword moves
            // the player to another place, and nothing but a run proves it.
            // Optional trailing list of NO-ARGUMENT guest functions to call right
            // after, hex, comma-separated: the warp handler does not stop at
            // loadScene, and finding the shortest tail that makes the world drawable
            // is an experiment, so it is a command-line knob rather than a constant.
            "--host-scene" => {
                let mut it = v.split(':');
                let f = it.next().and_then(|s| s.parse::<u64>().ok());
                let s = it.next().and_then(|s| s.parse::<u32>().ok());
                let tail: Vec<u32> = it
                    .next()
                    .map(|t| {
                        t.split(',')
                            .filter_map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok())
                            .collect()
                    })
                    .unwrap_or_default();
                if let (Some(f), Some(s)) = (f, s) {
                    host_scene = Some((f, s));
                    host_scene_tail = tail;
                }
            }
            "--dump-mem" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(b), Some(l)) = (it.next(), it.next()) else { continue };
                    let parse = |s: &str| -> Option<u64> {
                        let s = s.trim();
                        let s = s.strip_prefix("0x").unwrap_or(s);
                        u64::from_str_radix(s, 16).ok()
                    };
                    if let (Some(b), Some(l)) = (parse(b), parse(l)) {
                        dump_mem.push((b as u32, l as usize));
                    }
                }
            }
            // `--tower-species <scene>:<s0>,<s1>,<s2>` — arm the host's species
            // override for ONE scene before the run starts, exactly as the worker does
            // before it posts a floor's warp (`wie_util::stats::TOWER_SPECIES`, applied
            // by the `tower_species_lgt` hook as that scene's header is parsed). `-`
            // `--reaver-rate <pct>` — arm the reaver/hell drop-rate knob for THIS RUN,
            // exactly what `wie_reaver_test.txt` does on a device (`rate N`). 100 makes
            // every eligible kill drop, which is how the 헬 드랍 rolls (133차, ladder 152차)
            // are proven deterministically: poke the difficulty byte to 2, `--crea-set` a
            // record's level (+0x70) — 79..85 rolls a 조합서, 85+ a new item, 79+ a
            // 닉스코어파편, 68..78 nothing of hell's — `--crea-loot` it, and read the hook's warn.
            "--reaver-rate" => {
                if let Ok(pct) = v.parse::<u32>() {
                    wie_util::stats::set_reaver_test(Some(pct), None, false);
                    println!("reaver/hell rate armed: {pct}% (this run only)");
                }
            }
            // leaves a slot empty (0xff). Pair it with `--host-warp` into that scene and
            // `--creatures` to see the triple the spawn loop actually used.
            "--tower-species" => {
                let mut it = v.split(':');
                let scene = it.next().and_then(|s| s.parse::<u8>().ok());
                let trio: Vec<u8> = it
                    .next()
                    .map(|t| t.split(',').map(|x| x.trim().parse::<u8>().unwrap_or(0xff)).collect())
                    .unwrap_or_default();
                if let (Some(scene), 3) = (scene, trio.len()) {
                    wie_util::stats::set_tower_species(Some((scene, [trio[0], trio[1], trio[2]])));
                    eprintln!("[probe] tower species armed: scene {scene} -> {trio:?}");
                } else {
                    eprintln!("[probe] bad --tower-species {v:?} (want scene:s0,s1,s2)");
                }
            }
            // `--tower-cap <scene>:<n>` — arm the 145차 spawn cap for ONE scene, exactly as
            // the worker does next to the species triple before it posts a floor's warp
            // (`wie_util::stats::TOWER_SPAWN_CAP`, applied by the `tower_spawn_cap_lgt`
            // hook in the spawn loop). Pair it with `--host-warp` + `--creatures`: the scene
            // should hold exactly `n` records, all alive.
            "--tower-cap" => {
                let mut it = v.split(':');
                let scene = it.next().and_then(|s| s.parse::<u8>().ok());
                let cap = it.next().and_then(|s| s.parse::<u8>().ok());
                if let (Some(scene), Some(cap)) = (scene, cap) {
                    wie_util::stats::set_tower_spawn_cap(Some((scene, cap)));
                    eprintln!("[probe] tower spawn cap armed: scene {scene} -> {cap}");
                } else {
                    eprintln!("[probe] bad --tower-cap {v:?} (want scene:n)");
                }
            }
            "--host-give" => {
                for spec in v.split(',') {
                    let parts: Vec<&str> = spec.split(':').collect();
                    if parts.len() < 3 {
                        eprintln!("[probe] bad --host-give {spec:?} (want frame:cat:idx[:n[:announce]])");
                        continue;
                    }
                    let f = parts[0].parse::<u64>();
                    let cat = u16::from_str_radix(parts[1].trim_start_matches("0x"), 16);
                    let idx = u16::from_str_radix(parts[2].trim_start_matches("0x"), 16);
                    let n = parts.get(3).map_or(Ok(1), |x| x.parse::<u32>());
                    let ann = parts.get(4).map_or(Ok(-1), |x| x.parse::<i32>());
                    match (f, cat, idx, n, ann) {
                        (Ok(f), Ok(c), Ok(i), Ok(n), Ok(a)) => host_gives.push((f, c, i, n, a as u32)),
                        _ => eprintln!("[probe] bad --host-give {spec:?} (want frame:cat:idx[:n[:announce]], hex cat/idx)"),
                    }
                }
            }
            "--bagbox" => bagbox_mask = u32::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0),
            // `--bagbox-count <cat>:<idx>:<n>` — seed the box MIRROR the two quest-VM
            // hooks read (hex cat/idx), as the app's worker publishes it every frame.
            "--bagbox-count" => {
                let p: Vec<&str> = v.split(':').collect();
                match p.as_slice() {
                    [c, i, n] => bagbox_counts.push((
                        u32::from_str_radix(c.trim_start_matches("0x"), 16).unwrap_or(0),
                        u32::from_str_radix(i.trim_start_matches("0x"), 16).unwrap_or(0),
                        n.parse().unwrap_or(0),
                    )),
                    _ => eprintln!("[probe] bad --bagbox-count {v:?} (want cat:idx:n)"),
                }
            }
            // `--vm-item <frame>:<cat>:<idx>:<n>:<mode>` — call the quest VM's own item
            // helper `0x3bcf0(cat, idx, out, mode)` directly, with `out` = the save
            // chain's scratch word. `mode 1` counts (and prints what it answered),
            // `mode 0` takes `n`. This is how the `bagbox_count_lgt` / `bagbox_take_lgt`
            // hooks are exercised without a quest: the helper is the ONE thing a script
            // asks, so driving it is driving the hand-in.
            "--vm-item" => {
                let p: Vec<&str> = v.split(':').collect();
                match p.as_slice() {
                    [f, c, i, n, m] => vm_items.push((
                        f.parse().unwrap_or(0),
                        u32::from_str_radix(c.trim_start_matches("0x"), 16).unwrap_or(0),
                        u32::from_str_radix(i.trim_start_matches("0x"), 16).unwrap_or(0),
                        n.parse().unwrap_or(0),
                        m.parse().unwrap_or(1),
                    )),
                    _ => eprintln!("[probe] bad --vm-item {v:?} (want frame:cat:idx:n:mode)"),
                }
            }
            // `--bagbox-restage <frame>:<tab>:<slot>:<cat>:<idx>` — do what the app's
            // `BagBoxHost::stage_unit` does (127차): snapshot the record at (tab, slot),
            // overwrite it with ONE unit of (hex cat, idx) in `giveItem`'s new-cell shape,
            // and arm the restage knob with the snapshot. A `--vm-item <f+n>:<cat>:<idx>:1:0`
            // (take mode, boxes off) then makes the guest's own bag walk call
            // `removeItem 0x5340c` on that cell — where the `bagbox_restage_lgt` hook must
            // put the snapshot back instead of clearing it. `--bag` afterwards shows which.
            // `--use-item <frame>:<tab>:<slot>` — call the game's own `useItem 0x5505c(tab,
            // slot)` on a bag cell: the same function the inventory popup's 사용하기 and the
            // field hotkey reach, arms and consume tail included (`0x55450`).
            "--use-item" => {
                let p: Vec<&str> = v.split(':').collect();
                match p.as_slice() {
                    [f, t, s] => use_items.push((f.parse().unwrap_or(0), t.parse().unwrap_or(0), s.parse().unwrap_or(0))),
                    _ => eprintln!("[probe] bad --use-item {v:?} (want frame:tab:slot)"),
                }
            }
            // `--call <frame>:<fn hex>[:<arg hex>]*` (comma-separated for several) — call ANY
            // guest function with up to four register arguments after this frame's host code.
            // The generic lever: 139차 ran the paper doll's layer setter `0x517bc(slot, cat,
            // idx, equip)` through it to see a new look drawn. ⛔ ASYNC like every guest call
            // here — read its effect on a later frame.
            "--call" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let f = it.next().and_then(|s| s.trim().parse::<u64>().ok());
                    let hex = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok();
                    let func = it.next().and_then(hex);
                    let args: Option<Vec<u32>> = it.map(hex).collect();
                    match (f, func, args) {
                        (Some(f), Some(func), Some(args)) if args.len() <= 4 => guest_calls.push((f, func, args)),
                        _ => eprintln!("[probe] bad --call {spec:?} (want frame:fnhex[:arghex]*, at most four args)"),
                    }
                }
            }
            // A sixth field `off` stages the cell the same way but leaves the knob OFF —
            // the control half of an A/B (the game's own removal then clears the cell).
            "--bagbox-restage" => {
                let p: Vec<&str> = v.split(':').collect();
                match p.as_slice() {
                    [f, t, s, c, i] | [f, t, s, c, i, _] => restages.push((
                        f.parse().unwrap_or(0),
                        t.parse().unwrap_or(0),
                        s.parse().unwrap_or(0),
                        u16::from_str_radix(c.trim_start_matches("0x"), 16).unwrap_or(0),
                        u16::from_str_radix(i.trim_start_matches("0x"), 16).unwrap_or(0),
                        p.len() < 6,
                    )),
                    _ => eprintln!("[probe] bad --bagbox-restage {v:?} (want frame:tab:slot:cat:idx[:off])"),
                }
            }
            // `--bagbox-bypass <cat>:<idx>:<n>` — arm the materialise bypass once (hex cat/idx).
            "--bagbox-bypass" => {
                let parts: Vec<&str> = v.split(':').collect();
                if let (Some(c), Some(i), Some(n)) = (parts.first(), parts.get(1), parts.get(2))
                    && let (Ok(c), Ok(i), Ok(n)) = (u16::from_str_radix(c.trim_start_matches("0x"), 16), u16::from_str_radix(i.trim_start_matches("0x"), 16), n.parse::<u8>())
                {
                    wie_util::stats::set_bagbox_bypass(c, i, n);
                } else {
                    eprintln!("[probe] bad --bagbox-bypass {v:?} (want cat:idx:n)");
                }
            }
            "--bag" => bag_frames = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--loot" => {
                let mut it = v.split([':', '-']);
                if let (Some(a), Some(b)) = (it.next(), it.next()) {
                    if let (Ok(a), Ok(b)) = (a.parse::<u64>(), b.parse::<u64>()) {
                        loot_watch = Some((a, b));
                    } else {
                        eprintln!("[probe] bad --loot {v:?} (want from:to)");
                    }
                }
            }
            "--net-offline" => net_offline = v == "1" || v == "true",
            "--crea-loot" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(i)) = (it.next(), it.next()) else { continue };
                    if let (Ok(f), Ok(i)) = (f.parse::<u64>(), i.parse::<i32>()) {
                        crea_loots.push((f, i));
                    } else {
                        eprintln!("[probe] bad --crea-loot {spec:?} (want frame:idx)");
                    }
                }
            }
            "--crea-hurt" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(i), Some(d)) = (it.next(), it.next(), it.next()) else { continue };
                    if let (Ok(f), Ok(i), Ok(d)) = (f.parse::<u64>(), i.parse::<i32>(), d.parse::<u32>()) {
                        crea_hurts.push((f, i, d));
                    } else {
                        eprintln!("[probe] bad --crea-hurt {spec:?} (want frame:idx:dmg)");
                    }
                }
            }
            "--crea-hit" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(i), Some(d)) = (it.next(), it.next(), it.next()) else { continue };
                    if let (Ok(f), Ok(i), Ok(d)) = (f.parse::<u64>(), i.parse::<i32>(), d.parse::<i32>()) {
                        crea_hits.push((f, i, d));
                    } else {
                        eprintln!("[probe] bad --crea-hit {spec:?} (want frame:idx:delta)");
                    }
                }
            }
            "--creatures" => creature_frames = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--blit-log" => blit_log_frames = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--portrait" => portrait_frames = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--portrait-keep" => portrait_keep_from = v.trim().parse().ok(),
            "--find-word" => {
                let mut it = v.split(':');
                if let (Some(f), Some(vals)) = (it.next().and_then(|s| s.parse::<u64>().ok()), it.next()) {
                    let vals: Vec<u32> = vals.split(',').filter_map(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok()).collect();
                    find_words.push((f, vals));
                }
            }
            "--player" => player_frames = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--host-place" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(x), Some(y)) = (it.next(), it.next(), it.next()) else { continue };
                    if let (Ok(f), Ok(x), Ok(y)) = (f.parse::<u64>(), x.parse::<i32>(), y.parse::<i32>()) {
                        host_place.push((f, x, y));
                    } else {
                        eprintln!("[probe] bad --host-place {spec:?} (want frame:tx:ty)");
                    }
                }
            }
            // `--crea-set <frame>:<idx|all>:<offhex>:<hexbytes>` — write into a live
            // creature record. The tower's whole "live stat fork" (level, HP, defence)
            // is this write and nothing else, so this is the lever that qualifies a
            // fork before any of it ships: run once without it and once with it, and
            // the frames plus `--creatures` say what the game did with the value.
            "--crea-set" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(idx), Some(off), Some(hex)) =
                        (it.next(), it.next(), it.next(), it.next())
                    else {
                        continue;
                    };
                    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
                    let bytes: Option<Vec<u8>> = (hex.len() % 2 == 0)
                        .then(|| {
                            (0..hex.len())
                                .step_by(2)
                                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                                .collect()
                        })
                        .flatten();
                    let off = u32::from_str_radix(off.trim().trim_start_matches("0x"), 16).ok();
                    let idx = if idx.eq_ignore_ascii_case("all") { Some(None) } else { idx.parse::<i32>().ok().map(Some) };
                    if let (Ok(f), Some(idx), Some(off), Some(bytes)) = (f.parse::<u64>(), idx, off, bytes) {
                        crea_sets.push((f, idx, off, bytes));
                    } else {
                        eprintln!("[probe] bad --crea-set {spec:?} (want frame:idx|all:offhex:hexbytes)");
                    }
                }
            }
            "--dump" => dumps = v.split(',').filter_map(|s| s.parse().ok()).collect(),
            "--poke" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(a), Some(hex)) = (it.next(), it.next(), it.next()) else { continue };
                    let hex: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
                    let bytes: Option<Vec<u8>> = (hex.len() % 2 == 0)
                        .then(|| (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect())
                        .flatten();
                    let addr = u32::from_str_radix(a.trim().trim_start_matches("0x"), 16).ok();
                    if let (Ok(f), Some(addr), Some(bytes)) = (f.parse::<u64>(), addr, bytes) {
                        pokes.push((f, addr, bytes));
                    } else {
                        eprintln!("[probe] bad --poke spec {spec:?} (want frame:hexaddr:hexbytes)");
                    }
                }
            }
            "--avatar-preview" => {
                for spec in v.split(',') {
                    let p: Vec<&str> = spec.split(':').collect();
                    match p.as_slice() {
                        [f, slot, set] => avatar_previews.push((
                            f.parse().unwrap_or(0),
                            slot.parse().unwrap_or(usize::MAX),
                            set.parse().ok(),
                        )),
                        _ => eprintln!("[probe] bad --avatar-preview {spec:?} (want frame:slot:set|-)"),
                    }
                }
            }
            "--keys" => {
                for spec in v.split(',') {
                    let mut it = spec.split(':');
                    let (Some(f), Some(name)) = (it.next(), it.next()) else { continue };
                    let hold: u64 = it.next().and_then(|h| h.parse().ok()).unwrap_or(6);
                    if let (Ok(f), Some(kc)) = (f.parse::<u64>(), key_of(name)) {
                        keys.push((f, kc, hold));
                    }
                }
            }
            _ => {}
        }
        i += 2;
    }
    fs::create_dir_all(&out).unwrap();
    eprintln!("[probe] zip={zip:?} screen={w}x{h} frames={frames} ms/frame={ms_per_frame} dumps={dumps:?} keys={keys:?}");

    // --- boot ---
    let last = Arc::new(Mutex::new(vec![0u32; (w * h) as usize]));
    let redraw = Arc::new(Mutex::new(false));
    let clock = Arc::new(AtomicU64::new(1_000));
    let platform = Box::new(ProbePlatform {
        screen: ProbeScreen {
            w,
            h,
            last: last.clone(),
            redraw: redraw.clone(),
        },
        fs: MemFs::default(),
        db: match &db_root {
            Some(root) => Box::new(FsDbRepo { root: root.clone() }),
            None => Box::new(MemDbRepo::default()),
        },
        clock: clock.clone(),
        net: net_offline.then(|| probe_net::ProbeNet::new(clock.clone())),
    });

    // Host sprite blitter on/off. Same binary, same deterministic input, one bool -
    // that is the whole A/B frame-hash check (see binary_patches/blit565.rs).
    wie_util::stats::set_blit_hle(hle);
    wie_util::stats::THIRD_OPTS_OFF.store(!third_opts, Ordering::Relaxed);
    if !third_opts {
        eprintln!("[probe] accessory third options: OFF (acc_third_stat_lgt adds nothing)");
    }
    wie_util::stats::set_attack_speed_pct(atk_speed_pct);
    if atk_speed_pct > 0 {
        eprintln!("[probe] attack speed armed: {atk_speed_pct} %");
    }
    eprintln!("[probe] host blitter (HLE) {}", if hle { "ON" } else { "OFF" });

    let bytes = fs::read(&zip).expect("game zip");
    let files = extract_zip(&bytes).expect("extract zip");
    // ★ 167차 아바타 옷장: the costume sheets the jar carries, the way the app publishes them
    // at boot — the two paper-doll hooks never name a sheet that is not in this list.
    if let Some(jar) = files.iter().find(|(k, _)| k.ends_with(".jar")).map(|(_, v)| v) {
        let masks = wie_util::avatar::sheets_in_zip(jar);
        wie_util::stats::set_avatar_sheets(masks);
        let n: u32 = masks.iter().map(|m| m.count_ones()).sum();
        eprintln!("[probe] avatar: the jar carries {n} costume sheet(s) {masks:x?}");
        // ★ 171차: the sets whose 투구 sheet draws the whole head — as `emulator.rs` publishes it.
        let own_head = avatar_catalog::SETS.iter().filter(|s| s.own_head).fold(0u64, |m, s| m | 1 << (s.id - 1));
        wie_util::stats::set_avatar_own_head(own_head);
        eprintln!("[probe] avatar: own-head sets {own_head:#x}");
    }
    let is_ktf = KtfEmulator::loadable_archive(&files);
    let is_lgt = !is_ktf && LgtEmulator::loadable_archive(&files);
    if !is_ktf && !is_lgt {
        eprintln!("[probe] not a KTF/LGT archive");
        return;
    }
    // TEMPORARY (2026-08-06): sampled guest profile. `ArmCore` already samples the
    // guest PC + R7 frame chain once per 1000-instruction batch; nothing ever turned
    // it on. Leaf PCs tell us WHICH guest routine burns the 2.5 M instructions a
    // frame costs, which is the only way to aim a host-side reimplementation.
    let prof_leaf: Arc<Mutex<HashMap<u32, u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let prof_caller: Arc<Mutex<HashMap<(u32, u32), u64>>> = Arc::new(Mutex::new(HashMap::new()));
    let options = Options {
        enable_gdbserver: false,
        profile: if profile {
            let leaf = prof_leaf.clone();
            let caller = prof_caller.clone();
            Some(Box::new(move |batch: Vec<wie_backend::ProfileSample>| {
                let mut leaf = leaf.lock().unwrap();
                let mut caller = caller.lock().unwrap();
                for s in batch {
                    if let Some(&pc) = s.stack.first() {
                        *leaf.entry(pc).or_insert(0) += s.count;
                        if let Some(&lr) = s.stack.get(1) {
                            *caller.entry((pc, lr & !1)).or_insert(0) += s.count;
                        }
                    }
                }
            }))
        } else {
            None
        },
    };
    let build = if is_ktf { GuestBuild::Ktf } else { GuestBuild::Lgt };
    let chain = map_chain_of(build);
    // `--portrait-keep` state (153차): the app's own keeper, the run count it last saw,
    // and the tallies the summary prints.
    let mut keep: portrait::PortraitKeeper<Vec<u32>> = portrait::PortraitKeeper::new();
    let mut keep_seq_seen: u32 = 0;
    let (mut keep_canonical, mut keep_offers, mut keep_changes) = (0u32, 0u32, 0u32);
    let mut emu: Box<dyn Emulator> = if is_ktf {
        Box::new(KtfEmulator::from_archive(platform, files, options).expect("KTF load"))
    } else {
        Box::new(LgtEmulator::from_archive(platform, files, options).expect("LGT load"))
    };
    eprintln!(
        "[probe] loaded ({}) char_anchor={} container_data_off={:?}",
        build.name(),
        match build.char_anchor() {
            Some(CharAnchor::Pointer { global, fallback }) => format!("*{global:#x} (fallback {fallback:#x})"),
            Some(CharAnchor::Static(b)) => format!("static {b:#x}"),
            None => "none".to_string(),
        },
        chain.data_off,
    );

    // --- tick loop (deterministic: 1 tick per frame, no wall-clock budget) ---
    let wall_start = std::time::Instant::now();
    let stack_reserve_start = stack_gauge::reserved_bytes().unwrap_or(0);
    let mut stack_reserve_min = stack_reserve_start;
    eprintln!("[stack] initial reserve {} KB", stack_reserve_start / 1024);
    let mut released: Vec<(u64, KeyCode)> = vec![];
    // One host save in flight at most, same as the app.
    let mut hs_job: Option<(host_save::Job, Arc<wie_backend::GuestCallResult>)> = None;
    // --host-levelup watch: (level, stat points, skill points) before the call.
    let mut lu_watch: Option<(u16, u16, u8)> = None;
    // One live `AttackHold` (and its re-press count) per `--hold-key` spec.
    let mut hold_state: Vec<Option<(atkspeed::AttackHold, u32)>> = vec![None; hold_keys.len()];
    let mut legacy_next: Vec<u64> = vec![0; hold_keys.len()];
    let mut ending_runs = wie_util::stats::ending_transition().0;
    // --regen: the tick's next due time (0 = arm one period out) and the pending read-back.
    let mut regen_due: u64 = 0;
    let mut regen_pending: Option<(u32, u32)> = None;
    for f in 0..=frames {
        clock.fetch_add(ms_per_frame, Ordering::SeqCst);
        wie_util::stats::set_play_clock_ms(f * ms_per_frame);
        for &(sf, m) in &set_bonus_at {
            if sf == f {
                wie_util::stats::set_set_bonus(m);
                eprintln!("[probe] f={f} set bonus mask -> {m:#x}");
            }
        }
        for &(pf, pct, special) in &pierce_at {
            if pf == f {
                wie_util::stats::set_pierce(pct, special);
                eprintln!("[probe] f={f} 방패무시 -> {pct} % ({} defence)", if special { "special" } else { "physical" });
            }
        }
        for &(wf, slot, value) in &stat_wide_at {
            if wf == f {
                // stat block (CHAR+0xb4) + 0x4a / 0x4c / 0x52 / 0x56 — `statwide.rs`'s four.
                let addr = 0x0150_3a88 + 0xb4 + [0x4a, 0x4c, 0x52, 0x56][slot];
                let held = wie_util::stats::sat_i16(value) as i16;
                wie_util::stats::set_player_wide(slot, value);
                let ok = emu.write_memory(addr, &held.to_le_bytes());
                eprintln!("[probe] f={f} wide stat {slot} -> {value} (halfword {held} at {addr:#x}, write {})", if ok { "ok" } else { "REFUSED" });
            }
        }
        for &(af, e) in &acc_effects_at {
            if af == f {
                wie_util::stats::set_acc_effects(e);
                eprintln!(
                    "[probe] f={f} accessory effects -> taken -{} % · dealt +{} % · kill max HP +{} % · hit max HP +{} %",
                    e.taken_pct, e.dealt_pct, e.kill_max_hp_pct, e.hit_max_hp_pct
                );
            }
        }
        for &(af, slot, set) in &avatar_previews {
            if af == f {
                wie_util::stats::set_avatar_preview(slot, set);
                eprintln!("[probe] f={f} avatar try-on slot {slot} -> {set:?}");
            }
        }
        for (pf, addr, bytes) in &pokes {
            if *pf != f {
                continue;
            }
            let before = emu.read_memory(*addr, bytes.len());
            let ok = emu.write_memory(*addr, bytes);
            let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            eprintln!("[poke] f={f} {addr:#x}: {} -> {} ({})", hex(&before), hex(bytes), if ok { "ok" } else { "REFUSED" });
        }
        if *redraw.lock().unwrap() {
            *redraw.lock().unwrap() = false;
            emu.handle_event(Event::Redraw);
        }
        for (kf, kc, hold) in &keys {
            if *kf == f {
                emu.handle_event(Event::Keydown(*kc));
                released.push((f + *hold, *kc));
            } else if *kf < f && f < *kf + *hold && (f - *kf) % 6 == 0 {
                emu.handle_event(Event::Keyrepeat(*kc)); // hold => auto-repeat
            }
        }
        released.retain(|(rf, kc)| {
            if *rf == f {
                emu.handle_event(Event::Keyup(*kc));
                false
            } else {
                true
            }
        });
        // --hold-key: the worker's step 3b, one probe frame per worker iteration.
        for (i, &(down, up, kc)) in hold_keys.iter().enumerate() {
            let now_ms = f * ms_per_frame;
            if f == down {
                emu.handle_event(Event::Keydown(kc));
                hold_state[i] = Some((atkspeed::AttackHold::new(now_ms, f), 0));
                eprintln!("[hold] f={f} {kc:?} down");
                if hold_legacy {
                    // vc401 ran its step in the SAME iteration as the press, with no delay.
                    legacy_next[i] = now_ms;
                    let view = hold_view(build, &|a: u32, n: usize| emu.read_memory(a, n));
                    if view.idle {
                        emu.handle_event(Event::Keydown(kc));
                        if let Some((_, sent)) = hold_state[i].as_mut() {
                            *sent += 1;
                        }
                        eprintln!("[hold] f={f} {kc:?} re-press #1 (vc401 rule, same iteration as the press)");
                    }
                    legacy_next[i] = now_ms + atkspeed::ATTACK_REPEAT_MS;
                } else {
                    // ★ 135차 §2 ⑧: the worker's 3b runs `step` in the PRESS iteration too, on
                    //   the view from before this tick. Rule 2 keeps it from pressing there,
                    //   but rule 1 has to see the UI the press landed on — a dialogue's last
                    //   page is exactly that case, and skipping this call (as the probe did
                    //   until 135차-B) only ever showed the hold the view AFTER the press.
                    let view = hold_view(build, &|a: u32, n: usize| emu.read_memory(a, n));
                    if let Some((hold, sent)) = hold_state[i].as_mut() {
                        if hold.step(now_ms, f, view) {
                            emu.handle_event(Event::Keydown(kc));
                            *sent += 1;
                            eprintln!("[hold] f={f} {kc:?} re-press #{sent} (in the press iteration — rule 2 should forbid this)");
                        } else if hold.spoiled() {
                            eprintln!("[hold] f={f} {kc:?} SPOILED at the press (a window/box/choice owned the key)");
                        }
                    }
                }
            } else if f == up {
                emu.handle_event(Event::Keyup(kc));
                let (sent, spoiled) = hold_state[i].map_or((0, false), |(h, n)| (n, h.spoiled()));
                eprintln!("[hold] f={f} {kc:?} up - {sent} re-press(es), spoiled={spoiled}");
                hold_state[i] = None;
            } else if down < f && f < up {
                let view = hold_view(build, &|a: u32, n: usize| emu.read_memory(a, n));
                if hold_legacy {
                    // vc401: `attack_next` gate, then `walk_idle_on_field`, nothing else.
                    if let Some((_, sent)) = hold_state[i].as_mut() {
                        if legacy_next[i] <= now_ms {
                            if view.idle {
                                emu.handle_event(Event::Keydown(kc));
                                *sent += 1;
                                eprintln!("[hold] f={f} {kc:?} re-press #{sent} (vc401 rule)");
                            }
                            legacy_next[i] = now_ms + atkspeed::ATTACK_REPEAT_MS;
                        }
                    }
                } else if let Some((hold, sent)) = hold_state[i].as_mut() {
                    let was = hold.spoiled();
                    if hold.step(now_ms, f, view) {
                        emu.handle_event(Event::Keydown(kc));
                        *sent += 1;
                        eprintln!("[hold] f={f} {kc:?} re-press #{sent}");
                    } else if !was && hold.spoiled() {
                        eprintln!("[hold] f={f} {kc:?} SPOILED (a window/box/choice owns the key) after {sent} re-press(es)");
                    }
                }
            }
        }
        if blit_log_frames.contains(&f) {
            wie_util::stats::blit_log_arm(true);
        }
        let keep_on = portrait_keep_from.is_some_and(|k| f >= k);
        if portrait_frames.contains(&f) || keep_on {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            match probe_portrait_window(build, &chain, &rd, (w as i32, h as i32)) {
                Some((origin, anchor, line)) => {
                    if portrait_frames.contains(&f) {
                        eprintln!("[portrait] f={f} {line}");
                    }
                    wie_util::stats::set_portrait_window(Some((origin, anchor)));
                }
                None => {
                    if portrait_frames.contains(&f) {
                        eprintln!("[portrait] f={f} no player object");
                    }
                    wie_util::stats::set_portrait_window(None);
                }
            }
        }
        if let Err(e) = emu.tick() {
            eprintln!("[probe] tick error at frame {f}: {e:?}");
            break;
        }
        for (ff, vals) in &find_words {
            if *ff != f {
                continue;
            }
            let (lo, hi) = (0x1500000u32, 0x1513000u32);
            let mem = emu.read_memory(lo, (hi - lo) as usize);
            for v in vals {
                let hits: Vec<String> = mem
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, c)| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) == *v)
                    .map(|(i, _)| format!("{:#x}", lo + i as u32 * 4))
                    .take(60)
                    .collect();
                eprintln!("[find] f={f} word {v:#x} ({}) at {} place(s): {}", *v as i32, hits.len(), hits.join(" "));
            }
        }
        if portrait_frames.contains(&f) {
            let px = wie_util::stats::portrait_snapshot();
            let (w, h) = (wie_util::stats::PORTRAIT_W, wie_util::stats::PORTRAIT_H);
            let drawn = px.iter().filter(|p| **p >> 24 != 0).count();
            let drawn_box = portrait::drawn_box(&px, w, h);
            let face = portrait::face_crop(&px, w, h).map(|(_, _, c)| c.iter().filter(|p| **p >> 24 != 0).count());
            let (seq, run, blended) = wie_util::stats::portrait_run_state();
            eprintln!(
                "[portrait] f={f} blits taken={} drawn px={drawn} box={drawn_box:?} face px={face:?} run#{seq} left={run} blended={blended}",
                wie_util::stats::PORTRAIT_BLITS.load(std::sync::atomic::Ordering::Relaxed)
            );
            let path = out.join(format!("portrait_{f:05}_{w}x{h}.rgba"));
            let mut raw = Vec::with_capacity(px.len() * 4);
            for p in &px {
                raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]);
            }
            fs::write(&path, &raw).unwrap();
        }
        // `--portrait-keep` (153차): the worker's after-tick step, with the probe's own reads of
        // the offsets `items::portrait_canonical` / `portrait_look` / `portrait_spot` use.
        if keep_on {
            let (seq, run, blended) = wie_util::stats::portrait_run_state();
            // The worker's HUD poll (every fifth frame): the look on the field, whichever way
            // the player faces — a remembered look goes up from here.
            if f % 5 == 0 {
                let rd = |a: u32, n: usize| emu.read_memory(a, n);
                let cp = char_base(build, &rd);
                let i16at = |a: u32| -> Option<i16> {
                    let b = rd(a, 2);
                    (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
                };
                let worn: Option<Vec<(i16, i16)>> =
                    (0..9u32).map(|i| Some((i16at(cp + 0x28 + 0x364 + i * 0x3c + 0x22)?, i16at(cp + 0x28 + 0x364 + i * 0x3c + 0x24)?))).collect();
                if let (Some(class), Some(worn)) = (i16at(cp + 0xb6), worn) {
                    let look = portrait::look_key(class as u16, &worn);
                    let what = keep.observe(look);
                    if what != portrait::Observed::Same && what != portrait::Observed::Waiting {
                        eprintln!("[keep] f={f} observe {look:#018x}: {what:?}");
                    }
                }
            }
            if seq != keep_seq_seen {
                keep_seq_seen = seq;
                let rd = |a: u32, n: usize| emu.read_memory(a, n);
                let cp = char_base(build, &rd);
                let u8at = |a: u32| rd(a, 1).first().copied();
                let i16at = |a: u32| -> Option<i16> {
                    let b = rd(a, 2);
                    (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
                };
                let canonical = match (u8at(cp + 0x28 + 0x9b8e), u8at(cp + 0x28 + 0x9b8c), i16at(cp + 0x28 + 0x9ba0)) {
                    (Some(state), Some(face), Some(tick)) => portrait::canonical_frame(state, face, tick as u16),
                    _ => false,
                };
                if canonical {
                    keep_canonical += 1;
                    let class = i16at(cp + 0xb6).map(|v| v as u16);
                    let worn: Option<Vec<(i16, i16)>> = (0..9u32)
                        .map(|i| {
                            let rec = cp + 0x28 + 0x364 + i * 0x3c;
                            Some((i16at(rec + 0x22)?, i16at(rec + 0x24)?))
                        })
                        .collect();
                    let spot = probe_portrait_window(build, &chain, &rd, (w as i32, h as i32)).and_then(|_| {
                        let u32at = |a: u32| -> Option<u32> {
                            let b = rd(a, 4);
                            (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        };
                        let first = u32at(cp + PLAYER_OBJ_HANDLE_OFF)?;
                        let obj = match chain.data_off {
                            Some(off) => u32at(first)?.wrapping_add(off),
                            None => first,
                        };
                        Some((i32::from(i16at(obj + 0x10)?), i32::from(i16at(obj + 0x12)?)))
                    });
                    let on_screen =
                        wie_util::stats::portrait_origin().is_some_and(|o| portrait::face_on_screen(o, w as i32, h as i32));
                    if let (Some(class), Some(worn), Some(spot)) = (class, worn, spot) {
                        let look = portrait::look_key(class, &worn);
                        if keep.wants(look) && run == 0 && !blended && on_screen {
                            let px = wie_util::stats::portrait_snapshot();
                            if let Some((_, _, face)) =
                                portrait::face_crop(&px, wie_util::stats::PORTRAIT_W, wie_util::stats::PORTRAIT_H)
                            {
                                keep_offers += 1;
                                if keep.offer(look, spot, face.clone()) {
                                    keep_changes += 1;
                                    let hash = face.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| (h ^ u64::from(*p)).wrapping_mul(0x100_0000_01b3));
                                    eprintln!("[keep] f={f} the ring took look {look:#018x} at {spot:?} — picture {hash:016x} (change #{keep_changes}, final {})", keep.is_final(look));
                                    let mut raw = Vec::with_capacity(face.len() * 4);
                                    for p in &face {
                                        raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]);
                                    }
                                    fs::write(out.join(format!("keep_{f:05}_24x24.rgba")), &raw).unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
        if blit_log_frames.contains(&f) {
            let log = wie_util::stats::blit_log_take();
            wie_util::stats::blit_log_arm(false);
            eprintln!("[blitlog] f={f} {} blit(s)", log.len());
            for (i, (x, y, w, h, spr, sx, sy, lr)) in log.iter().enumerate() {
                eprintln!("[blitlog]  #{i:03} dst=({x},{y}) {w}x{h} spr={spr:#x} src=({sx},{sy}) lr={lr:#x}");
            }
        }
        // The 2회차 transition latch (`ending_transition_lgt`, 135차-B): one line per run of
        // `0x3af2c`, with the difficulty byte it found — how a mashed ending is checked for a
        // missed transition without a device.
        let (runs, before) = wie_util::stats::ending_transition();
        if runs != ending_runs {
            ending_runs = runs;
            eprintln!("[ending] f={f} the 2회차 transition ran (#{runs}) — difficulty byte was {before:?}");
        }
        if host_levelup_frame == Some(f) {
            // CHAR offsets mirrored from items.rs (LEVEL/EXP/EXP_NEXT/points) — items.rs
            // is not include!d here, same convention as the --verify-offsets block.
            const LV: u32 = 0xe2;
            const EXP: u32 = 0xe4;
            const NEXT: u32 = 0xe8;
            match build.level_up_fn() {
                None => eprintln!("[levelup] f={f} refused: checkLevelUp not located on {}", build.name()),
                Some(fun) => {
                    let c = {
                        let read = |a: u32, l: usize| emu.read_memory(a, l);
                        char_base(build, &read)
                    };
                    let b = emu.read_memory(c + LV, 2);
                    let level = if b.len() == 2 { u16::from_le_bytes([b[0], b[1]]) } else { 0 };
                    let b = emu.read_memory(c + EXP, 4);
                    let exp = if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 };
                    let b = emu.read_memory(c + NEXT, 4);
                    let next = if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 };
                    let b = emu.read_memory(c + 0x18a, 2);
                    let sp0 = if b.len() == 2 { u16::from_le_bytes([b[0], b[1]]) } else { 0 };
                    let kp0 = emu.read_memory(c + 0x18c, 1).first().copied().unwrap_or(0);
                    let wrote = emu.write_memory(c + EXP, &(next.saturating_add(1)).to_le_bytes());
                    emu.call_guest_function(fun, &[]);
                    eprintln!(
                        "[levelup] f={f} issued: level={level} exp={exp}/{next} -> exp={} (write ok={wrote}), checkLevelUp={fun:#x}",
                        next.saturating_add(1)
                    );
                    lu_watch = Some((level, sp0, kp0));
                }
            }
        }
        // --- --player / --host-place: the object a warp does NOT move ------------
        if player_frames.contains(&f) || host_place.iter().any(|(pf, ..)| *pf == f) {
            let plan = {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let cp = char_base(build, &rd);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                // One hop on LGT, two on KTF - the same switch every other chain uses.
                let deref = |p: u32| -> Option<u32> {
                    let h = u32at(p).filter(|v| (0x4000_0000..0x5000_0000).contains(v))?;
                    match build.container_data_off() {
                        Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (0x4000_0000..0x5000_0000).contains(v)),
                        None => Some(h),
                    }
                };
                let obj = deref(cp + 0x9bd0);
                let map = deref(cp + 0x9c90);
                let i16at = |a: u32| -> Option<i16> {
                    let b = rd(a, 2);
                    (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
                };
                let u8at = |a: u32| -> Option<u8> { rd(a, 1).first().copied() };
                let (px, py) = match obj {
                    Some(o) => (i16at(o + 0x10), i16at(o + 0x12)),
                    None => (None, None),
                };
                let (tx, ty) = match obj {
                    Some(o) => (u8at(o + 0x27), u8at(o + 0x28)),
                    None => (None, None),
                };
                let (mw, mh) = match map {
                    Some(m) => (u32at(m + 0x48), u32at(m + 0x4c)),
                    None => (None, None),
                };
                // HP next to the position: `(i16)CHAR+0xf2` over `(i32)CHAR+0xec` — the
                // same two words the HUD gauge reads. A monster that cannot attack is a
                // claim about THIS number staying put, so the lever prints it.
                let hp = i16at(cp + 0xf2);
                let hp_max = u32at(cp + 0xec);
                eprintln!(
                    "[player] f={f} obj={:?} pos=({px:?},{py:?}) tile=({tx:?},{ty:?}) map={:?} {mw:?}x{mh:?} hp={hp:?}/{hp_max:?}",
                    obj.map(|v| format!("{v:#x}")),
                    map.map(|v| format!("{v:#x}")),
                );
                obj
            };
            for (pf, tx, ty) in host_place.iter().filter(|(pf, ..)| *pf == f) {
                let Some(obj) = plan else {
                    eprintln!("[player] f={pf} host-place refused: no player object");
                    continue;
                };
                // MEASURED convention, from a probe run in the starting town and again
                // after a warp: `x = tile_x * 16 + 8` (column centre) and `y = tile_y *
                // 16` (the feet, not the centre). Both samples agree on both axes.
                let (x, y) = ((tx * 16 + 8) as i16, (ty * 16) as i16);
                let a = emu.write_memory(obj + 0x10, &x.to_le_bytes());
                let b = emu.write_memory(obj + 0x12, &y.to_le_bytes());
                // The tile bytes too: they are not derived on the fly (a write to the
                // pixel pair alone leaves them stale), and the map window's marker and
                // the field's own collision both read THEM.
                let c = emu.write_memory(obj + 0x27, &[*tx as u8]);
                let d = emu.write_memory(obj + 0x28, &[*ty as u8]);
                eprintln!("[player] f={f} host-place tile ({tx},{ty}) -> pos ({x},{y}) ok={a}/{b}/{c}/{d}");
            }
        }
        // --- --host-warp: post the request, then watch what the game does with it -
        for (wf, scene, entry) in host_warp.clone() {
            if f >= wf && f <= wf + 4 {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let cp = char_base(build, &rd);
                match build.scene_world() {
                    None => {
                        eprintln!("[warp] f={f} refused: no scene table on {}", build.name());
                    }
                    Some(sw) => {
                        // Where the player stands, so a run says whether the entry
                        // byte moved the landing spot. These two words track the walk
                        // (the same pair `--walk-cal` prints), so they are readable
                        // before the map window has ever been opened.
                        let pos = |a: u32| -> i32 {
                            let b = rd(a, 4);
                            if b.len() == 4 { i32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { -1 }
                        };
                        let at = || (pos(cp + 0x9e94), pos(cp + 0x9e98));
                        if f == wf {
                            // Read before writing: `rd` borrows the emulator immutably
                            // and `write_memory` wants it mutably.
                            let was = at();
                            // Entry point BEFORE the destination, destination before the
                            // request: the request byte is what arms it, so writing it
                            // last means the handler can never see a stale pair even if
                            // it ran between the writes.
                            let e = entry.map(|e| emu.write_memory(cp + sw.warp_entry_off, &[e]));
                            let d = emu.write_memory(cp + sw.warp_dest_off, &[scene as u8]);
                            let r = emu.write_memory(cp + sw.warp_request_off, &[1u8]);
                            eprintln!(
                                "[warp] f={f} requested scene {scene} entry={entry:?} \
                                 (entry write {e:?}) dest@{:#x} ok={d} \
                                 request@{:#x} ok={r}; the player was at {was:?}",
                                cp + sw.warp_dest_off,
                                cp + sw.warp_request_off,
                            );
                        } else {
                            let u32at = |a: u32| -> Option<u32> {
                                let b = rd(a, 4);
                                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                            };
                            let req = rd(cp + sw.warp_request_off, 1).first().copied();
                            let scene_now = {
                                let b = rd(cp + sw.scene_id_off, 2);
                                (b.len() == 2).then(|| u16::from_le_bytes([b[0], b[1]]))
                            };
                            eprintln!(
                                "[warp] f={f} request={req:?} (0 = consumed) \
                                 scene={scene_now:?} map={:?} player at {:?}",
                                u32at(cp + sw.map_id_off),
                                at()
                            );
                        }
                    }
                }
            }
        }
        // --- --crea-hurt: the game's own hitMonster on a record --------------------
        // ⛔ `call_guest_function` is ASYNC — it runs after this frame's host code — so the
        // record is read back on the NEXT frame (117차; the same trap `--vm-item` documents).
        // The old lever printed the PRE-hit values right after the call.
        for (_, idx, rec, dmg) in hurt_pending.iter().filter(|(due, ..)| *due == f).copied().collect::<Vec<_>>() {
            let b = emu.read_memory(rec + 0x7e, 2);
            let hp = (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]));
            let w = |o: u32| {
                let b = emu.read_memory(rec + o, 4);
                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            eprintln!(
                "[hurt] f={f} record #{idx} at {rec:#x} one frame after 0x3863c(rec, 0, {dmg}): hp_cur {hp:?} disp32 {:?} vhp {:?}/{:?}",
                w(0xfc),
                w(0x100),
                w(0x104)
            );
        }
        hurt_pending.retain(|(due, ..)| *due > f);
        for (_, idx, dmg) in crea_hurts.iter().filter(|(hf, ..)| *hf == f).copied().collect::<Vec<_>>() {
            let rec = {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                let cp = char_base(build, &rd);
                build.scene_world().and_then(|sw| {
                    let arr = u32at(cp + sw.creatures_off)?;
                    (arr != 0).then(|| arr.wrapping_add(idx as u32 * sw.creature_stride))
                })
            };
            match rec {
                None => eprintln!("[hurt] f={f} refused: no creature array"),
                Some(rec) => {
                    emu.call_guest_function(0x3863c | 1, &[rec, 0, dmg]);
                    hurt_pending.push((f + 1, idx, rec, dmg));
                    eprintln!("[hurt] f={f} record #{idx} at {rec:#x}: 0x3863c(rec, 0, {dmg}) called — read back on f={}", f + 1);
                }
            }
        }
        // --- --crea-hit: the game's own damage routine on a record ----------------
        for (_, idx, delta) in crea_hits.iter().filter(|(hf, ..)| *hf == f).copied().collect::<Vec<_>>() {
            let rec = {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                let cp = char_base(build, &rd);
                build.scene_world().and_then(|sw| {
                    let arr = u32at(cp + sw.creatures_off)?;
                    (arr != 0).then(|| arr.wrapping_add(idx as u32 * sw.creature_stride))
                })
            };
            match rec {
                None => eprintln!("[hit] f={f} refused: no creature array"),
                Some(rec) => {
                    emu.call_guest_function(0x38620 | 1, &[rec, delta as u32]);
                    let b = emu.read_memory(rec + 0x7e, 2);
                    let hp = (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]));
                    eprintln!("[hit] f={f} record #{idx} at {rec:#x}: 0x38620(rec, {delta}) -> hp_cur {hp:?}");
                }
            }
        }
        // --- --bagbox / --host-give / --bag ---------------------------------------
        for (cat, idx, n) in bagbox_counts.iter().copied() {
            wie_util::stats::set_bagbox_count(cat, idx, n);
        }
        for (_, cat, idx, n, mode) in vm_items.iter().filter(|(vf, ..)| *vf == f).copied().collect::<Vec<_>>() {
            match build.save_chain().map(|c| c.scratch) {
                None => eprintln!("[vm] f={f} refused: no scratch word on {}", build.name()),
                Some(scratch) => {
                    // ⛔ `call_guest_function` is ASYNC — it runs after this frame's host
                    // code, so the answer to THIS call can only be read on a later one.
                    // That is why the previous call's `*out` is printed here rather than
                    // this one's (a first read of 0 is the seed, not an answer).
                    let was = emu.read_memory(scratch, 2);
                    let was = if was.len() == 2 { u16::from_le_bytes([was[0], was[1]]) } else { 0 };
                    // Take mode reads `n` from `*out`; count mode writes its answer there.
                    emu.write_memory(scratch, &(n as u16).to_le_bytes());
                    emu.call_guest_function(0x3bcf0 | 1, &[cat, idx, scratch, mode]);
                    eprintln!("[vm] f={f} *out was {was}; now 0x3bcf0(cat {cat:#x}, idx {idx:#x}, out={n}, mode {mode})");
                }
            }
        }
        for (_, tab, slot) in use_items.iter().filter(|(uf, ..)| *uf == f).copied().collect::<Vec<_>>() {
            emu.call_guest_function(0x5505c | 1, &[tab, slot, 0, 0]);
            eprintln!("[use] f={f} useItem 0x5505c(tab {tab}, slot {slot}) called");
        }
        for (_, func, args) in guest_calls.iter().filter(|(cf, ..)| *cf == f).cloned().collect::<Vec<_>>() {
            let mut regs = [0u32; 4];
            regs[..args.len()].copy_from_slice(&args);
            emu.call_guest_function(func | 1, &regs);
            eprintln!("[call] f={f} {func:#x}({})", args.iter().map(|a| format!("{a:#x}")).collect::<Vec<_>>().join(", "));
        }
        // --- --regen: the host's 5-second heal (150차 레아의목걸이 option 2) --------------
        if let Some((rf, pct)) = regen_at
            && f >= rf
        {
            let hp_view = |emu: &dyn Emulator| {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let cp = char_base(build, &rd);
                let g = rd(cp + 0xf2, 2);
                let m = rd(cp + 0xec, 4);
                (g.len() == 2 && m.len() == 4).then(|| {
                    let gauge = i16::from_le_bytes([g[0], g[1]]).max(0) as u32;
                    let gmax = i32::from_le_bytes([m[0], m[1], m[2], m[3]]).max(0) as u32;
                    wie_util::stats::player_hp_view(gauge, gmax)
                })
            };
            if let Some(before) = regen_pending.take() {
                eprintln!("[regen] f={f} one frame after the heal: true HP {before:?} -> {:?}", hp_view(emu.as_ref()));
            }
            let now = f * ms_per_frame;
            let (heal, next) = wie_util::stats::regen_tick(now, regen_due, wie_util::acc_options::REGEN_PERIOD_MS);
            regen_due = next;
            if heal {
                match hp_view(emu.as_ref()) {
                    Some((hp, max)) => {
                        let n = wie_util::stats::regen_amount(hp, max, pct);
                        if n > 0 {
                            emu.call_guest_function(0x4b9c0 | 1, &[0x0150_3a88 + 0xb4, n]);
                            regen_pending = Some((hp, max));
                            eprintln!("[regen] f={f} t={now} ms: HP {hp}/{max} -> addHP(block, {n}) ({pct} % of the max)");
                        } else {
                            eprintln!("[regen] f={f} t={now} ms: HP {hp}/{max} — nothing to heal");
                        }
                    }
                    None => eprintln!("[regen] f={f} t={now} ms: no character to read"),
                }
            }
        }
        for (_, tab, slot, cat, idx, arm) in restages.iter().filter(|(rf, ..)| *rf == f).copied().collect::<Vec<_>>() {
            let rd = |a: u32, l: usize| emu.read_memory(a, l);
            let cp = char_base(build, &rd);
            let grid = {
                let b = rd(cp + 0x9e28, 4);
                if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
            };
            let rec = grid + 0x394 + u32::from(tab) * 0x3c4 + u32::from(slot) * 0x3c;
            let snap = rd(rec, 0x3c);
            if grid == 0 || snap.len() != 0x3c || snap[0x1f] == 0 {
                eprintln!("[restage] f={f} refused: ({tab},{slot}) is not an occupied cell (grid {grid:#x})");
                continue;
            }
            let mut bytes = [0u8; 0x3c];
            bytes.copy_from_slice(&snap);
            // The shape `giveItem`'s non-equipment block writes (`bagbox::stage_record`):
            // name kept as a stand-in, `+0x1e` -1, occupied, `+0x20` 0, `+0x21` -1, cat, idx, 1.
            let mut staged = bytes;
            staged[0x1e] = 0xff;
            staged[0x1f] = 1;
            staged[0x20] = 0;
            staged[0x21] = 0xff;
            staged[0x22..0x24].copy_from_slice(&cat.to_le_bytes());
            staged[0x24..0x26].copy_from_slice(&idx.to_le_bytes());
            staged[0x26..0x28].copy_from_slice(&1u16.to_le_bytes());
            let ok = emu.write_memory(rec, &staged[..0x28]);
            if arm {
                wie_util::stats::set_bagbox_restage(&wie_util::stats::BagboxRestage {
                    tab,
                    slot,
                    cat: cat as u8,
                    idx: idx as u8,
                    rec,
                    bytes,
                });
            }
            eprintln!(
                "[restage] f={f} ({tab},{slot}) rec {rec:#x}: was cat {:#x} idx {:#x} x{} — now cat {cat:#x} idx {idx:#x} x1 staged (write ok {ok}), knob {}",
                u16::from_le_bytes([snap[0x22], snap[0x23]]),
                u16::from_le_bytes([snap[0x24], snap[0x25]]),
                u16::from_le_bytes([snap[0x26], snap[0x27]]),
                if arm { "armed" } else { "OFF (control)" }
            );
        }
        if bagbox_mask != 0 {
            // Re-armed every frame the way the app's worker does it.
            // The three bag-item cells (`bagbox::keep_idx_mask`).
            wie_util::stats::set_bagbox_accept(bagbox_mask, (1 << 0x16) | (1 << 0x17) | (1 << 0x19));
            for b in 0..3 {
                wie_util::stats::set_bagbox_room(b, 48);
            }
        }
        for (_, cat, idx, n, ann) in host_gives.iter().filter(|(hf, ..)| *hf == f).copied().collect::<Vec<_>>() {
            match build.give_item_fn() {
                None => eprintln!("[give] f={f} refused: giveItem not located on {}", build.name()),
                Some(fun) => {
                    for u in 0..n {
                        // The drop loop passes each unit's ordinal, so a loud give does too.
                        let a = if ann == u32::MAX { u32::MAX } else { ann.wrapping_add(u) };
                        emu.call_guest_function(fun, &[u32::from(cat), u32::from(idx), a, 0]);
                    }
                    eprintln!("[give] f={f} 0x52b14(cat {cat:#x}, idx {idx:#x}, announce {}) x{n} called", ann as i32);
                }
            }
        }
        // --- --loot: the dropped-item records on the field (162차) --------------------
        // `grid = *(CHAR+0x9e28)`; records at `grid+0x60 + i*0x14` (20 B: +0 x, +1 y, +2 idx,
        // +3 cat, +4 group, +8 u64 spawn time, +0x10 u16 lifetime counter, +0x12 active),
        // count (i8) at `grid+0x2b8`, 30 slots. The game's loot tick (`0x54914`, once per
        // world tick) bumps the counter and removes the record past its lifetime compare;
        // the painter (`0x54c00`) blinks it past its own. Printed on every change of the SET
        // (count or members) so the exact vanish frame is one line, counters included.
        if let Some((a, b)) = loot_watch {
            if (a..=b).contains(&f) {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let cp = char_base(build, &rd);
                let grid = {
                    let b = rd(cp + 0x9e28, 4);
                    if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
                };
                let cnt = rd(grid + 0x2b8, 1).first().copied().unwrap_or(0) as i8;
                let mut rows = vec![];
                let mut key = vec![];
                for i in 0..cnt.clamp(0, 30) as u32 {
                    let r = rd(grid + 0x60 + i * 0x14, 0x14);
                    if r.len() != 0x14 {
                        continue;
                    }
                    let ctr = u16::from_le_bytes([r[0x10], r[0x11]]);
                    let head = format!("#{i} cat {:#x} idx {:#x} @({},{}) b4 {} act {}", r[3], r[2], r[0] as i8, r[1] as i8, r[4], r[0x12]);
                    rows.push(format!("{head} ctr {ctr}"));
                    key.push(head);
                }
                let key = format!("count {cnt}: {}", key.join(" · "));
                if loot_last.as_deref() != Some(key.as_str()) {
                    if !loot_prev.is_empty() {
                        eprintln!("[loot] f={f} (previous set, last counters) {}", loot_prev.join(" · "));
                    }
                    eprintln!("[loot] f={f} grid {grid:#x} count {cnt}: {}", rows.join(" · "));
                    loot_last = Some(key);
                }
                loot_prev = rows;
            }
        }
        if bag_frames.contains(&f) {
            let rd = |a: u32, l: usize| emu.read_memory(a, l);
            let cp = char_base(build, &rd);
            let grid = {
                let b = rd(cp + 0x9e28, 4);
                if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
            };
            let bags = rd(grid + 0x1668, 1).first().copied().unwrap_or(0) as i8;
            let mut cells = vec![];
            for tab in 0..bags.clamp(0, 5) as u32 {
                for slot in 0..16u32 {
                    let rec = grid + 0x394 + tab * 0x3c4 + slot * 0x3c;
                    let r = rd(rec, 0x3c);
                    if r.len() != 0x3c || r[0x1f] == 0 {
                        continue;
                    }
                    let cat = u16::from_le_bytes([r[0x22], r[0x23]]);
                    let idx = u16::from_le_bytes([r[0x24], r[0x25]]);
                    let cnt = u16::from_le_bytes([r[0x26], r[0x27]]);
                    cells.push(format!("({tab},{slot}) cat {cat:#x} idx {idx:#x} x{cnt}"));
                    // The record's own bytes past the name field — what a host write of a
                    // cell has to reproduce (3단계 materialise reads its shape from here).
                    eprintln!("[bag] f={f} rec ({tab},{slot}) name {:?} +0x1e..: {}", String::from_utf8_lossy(&r[..0x1e]), r[0x1e..].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
                }
            }
            eprintln!("[bag] f={f} grid {grid:#x} bags {bags}: {} cell(s) — {}", cells.len(), cells.join(" · "));
            eprintln!(
                "[bag] f={f} restage knob {} · restaged {} time(s)",
                wie_util::stats::bagbox_restage().map_or("off".to_string(), |k| format!("armed for ({},{}) cat {:#x} idx {:#x} rec {:#x}", k.tab, k.slot, k.cat, k.idx, k.rec)),
                wie_util::stats::bagbox_restaged()
            );
            let mut ring = vec![];
            while let Some((cat, idx, n, src)) = wie_util::stats::bagbox_pop() {
                ring.push(format!("cat {cat:#x} idx {idx:#x} x{n} src {src}"));
            }
            eprintln!("[bag] f={f} bagbox ring: {} entr{} — {}", ring.len(), if ring.len() == 1 { "y" } else { "ies" }, ring.join(" · "));
            // The game's floating ACQUISITION NOTICE queue (103차): count at `grid+0x2b9`,
            // five `0x1e`-byte EUC-KR texts at `grid+0x2be`, frame timer at `grid+0x2bc`.
            // The renderer (`0x54d28`) draws entries 0..count and pops the front every
            // 0x1f frames. Printed raw so a stale slot past `count` is visible too.
            let cnt = rd(grid + 0x2b9, 1).first().copied().unwrap_or(0) as i8;
            let timer = {
                let b = rd(grid + 0x2bc, 2);
                if b.len() == 2 { u16::from_le_bytes([b[0], b[1]]) } else { 0 }
            };
            let texts: Vec<String> = (0..5u32)
                .map(|i| {
                    let b = rd(grid + 0x2be + i * 0x1e, 0x1e);
                    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
                    format!("[{i}] {:?}", String::from_utf8_lossy(&b[..end]))
                })
                .collect();
            // The two gates on the call to that renderer, from window 3's paint arm
            // (`0x3b984`): the scene block's `+0x30` byte must be non-zero and no menu
            // overlay may own the panel (`CHAR+0x9c6c`). A queue that fills while the
            // timer stays 0 means the renderer never ran, and these say which gate shut it.
            let g_scene = rd(cp + 0x9e60, 1).first().copied().unwrap_or(0);
            let g_overlay = rd(cp + 0x9c6c, 1).first().copied().unwrap_or(0);
            eprintln!(
                "[bag] f={f} notice queue: count {cnt} timer {timer} gate(scene {g_scene:#x}, overlay {g_overlay:#x}) — {}",
                texts.join(" · ")
            );
        }
        // --- --crea-loot: the game's own loot routine on a record ------------------
        for (_, idx) in crea_loots.iter().filter(|(hf, ..)| *hf == f).copied().collect::<Vec<_>>() {
            let rec = {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                let cp = char_base(build, &rd);
                build.scene_world().and_then(|sw| {
                    let arr = u32at(cp + sw.creatures_off)?;
                    (arr != 0).then(|| arr.wrapping_add(idx as u32 * sw.creature_stride))
                })
            };
            match rec {
                None => eprintln!("[loot] f={f} refused: no creature array"),
                Some(rec) => {
                    emu.call_guest_function(0x38e70 | 1, &[rec]);
                    eprintln!("[loot] f={f} record #{idx} at {rec:#x}: 0x38e70(rec) called");
                }
            }
        }
        // --- --crea-set: fork a live creature record ------------------------------
        if crea_sets.iter().any(|(cf, ..)| *cf == f) {
            // Every read first, then the writes: the reading closures borrow the
            // emulator immutably and `write_memory` needs it mutably, so the plan is
            // built and only then applied.
            let plan: Vec<(i32, u32, u32, Vec<u8>, Vec<u8>)> = {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                let cp = char_base(build, &rd);
                match build.scene_world() {
                    None => vec![],
                    Some(sw) => {
                        let hop = |field: u32| -> Option<u32> {
                            let first = u32at(field)?;
                            match chain.data_off {
                                Some(off) => u32at(first).map(|v| v.wrapping_add(off)),
                                None => Some(first),
                            }
                        };
                        let arr = u32at(cp + sw.creatures_off).unwrap_or(0);
                        let n = hop(cp + MAP_HANDLE_OFF)
                            .and_then(|m| u32at(m + sw.map_spawn_count_off))
                            .unwrap_or(0) as i32;
                        let mut plan = vec![];
                        for (_, idx, off, bytes) in crea_sets.iter().filter(|(cf, ..)| *cf == f) {
                            for i in 0..n.max(0) {
                                if idx.is_some_and(|want| want != i) {
                                    continue;
                                }
                                let a = arr.wrapping_add(i as u32 * sw.creature_stride + off);
                                plan.push((i, *off, a, rd(a, bytes.len()), bytes.clone()));
                            }
                        }
                        plan
                    }
                }
            };
            for (i, off, a, was, bytes) in plan {
                let ok = emu.write_memory(a, &bytes);
                eprintln!(
                    "[crea] f={f} set #{i} +{off:#x} @{a:#x}: {} -> {} (ok={ok})",
                    was.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(""),
                    bytes.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(""),
                );
            }
        }
        // --- --creatures: the live monster records, raw and decoded ---------------
        if creature_frames.contains(&f) {
            let rd = |a: u32, l: usize| emu.read_memory(a, l);
            let u32at = |a: u32| -> Option<u32> {
                let b = rd(a, 4);
                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            let cp = char_base(build, &rd);
            match build.scene_world() {
                None => eprintln!("[crea] f={f} refused: no scene table on {}", build.name()),
                Some(sw) => {
                    let hop = |field: u32| -> Option<u32> {
                        let first = u32at(field)?;
                        match chain.data_off {
                            Some(off) => u32at(first).map(|v| v.wrapping_add(off)),
                            None => Some(first),
                        }
                    };
                    let arr = u32at(cp + sw.creatures_off).unwrap_or(0);
                    let n = hop(cp + MAP_HANDLE_OFF)
                        .and_then(|m| u32at(m + sw.map_spawn_count_off))
                        .unwrap_or(0) as i32;
                    let scene = {
                        let b = rd(cp + sw.scene_id_off, 2);
                        (b.len() == 2).then(|| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
                    };
                    let species: Vec<Option<u32>> =
                        (0..3).map(|i| u32at(cp + sw.spawn_species_off + i * 4)).collect();
                    eprintln!(
                        "[crea] f={f} scene={scene} array={arr:#x} stride={:#x} spawned={n} species={species:?}",
                        sw.creature_stride
                    );
                    for i in 0..n.max(0) {
                        let rec = arr.wrapping_add(i as u32 * sw.creature_stride);
                        let b = rd(rec, sw.creature_stride as usize);
                        if b.len() != sw.creature_stride as usize {
                            eprintln!("[crea]  #{i} at {rec:#x}: unreadable");
                            continue;
                        }
                        let i16at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
                        let elem0 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        let obj = u32at(elem0 + 0x1c).unwrap_or(0);
                        eprintln!(
                            "[crea]  #{i} at {rec:#x} lv={} hp={}/{} elem0={elem0:#x} obj={obj:#x}",
                            b[0x70] as i8,
                            i16at(0x7e),
                            i16at(0x7c),
                        );
                        for row in 0..(sw.creature_stride as usize + 15) / 16 {
                            let o = row * 16;
                            let end = (o + 16).min(b.len());
                            let hex: Vec<String> = b[o..end].iter().map(|x| format!("{x:02x}")).collect();
                            eprintln!("[crea]   +{o:03x}  {}", hex.join(" "));
                        }
                    }
                }
            }
        }
        // --- --host-scene: the game's own loadScene, and what it changed ---------
        if let Some((sf, scene)) = host_scene {
            // Read the world words on every frame from the arm-frame onwards, so the
            // "before" line is a real observation and not an assumption, and so a
            // change that lands a frame or two late is still attributed.
            if f + 1 >= sf {
                let rd = |a: u32, l: usize| emu.read_memory(a, l);
                let u32at = |a: u32| -> Option<u32> {
                    let b = rd(a, 4);
                    (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                };
                let cp = char_base(build, &rd);
                match build.scene_world() {
                    None => {
                        if f == sf {
                            eprintln!("[scene] f={f} refused: no scene table on {}", build.name());
                            host_scene = None;
                        }
                    }
                    Some(sw) => {
                        let hop = |field: u32| -> Option<u32> {
                            let first = u32at(field)?;
                            match chain.data_off {
                                Some(off) => u32at(first).map(|v| v.wrapping_add(off)),
                                None => Some(first),
                            }
                        };
                        let scene_now = {
                            let b = rd(cp + sw.scene_id_off, 2);
                            (b.len() == 2).then(|| u16::from_le_bytes([b[0], b[1]]))
                        };
                        let map = u32at(cp + sw.map_id_off);
                        let species: Vec<Option<u32>> =
                            (0..3).map(|i| u32at(cp + sw.spawn_species_off + i * 4)).collect();
                        let m = hop(cp + MAP_HANDLE_OFF);
                        let dims = m.map(|m| (u32at(m + 0x48).unwrap_or(0), u32at(m + 0x4c).unwrap_or(0)));
                        let spawned = m.and_then(|m| u32at(m + sw.map_spawn_count_off));
                        let creatures = u32at(cp + sw.creatures_off);
                        eprintln!(
                            "[scene] f={f} scene={scene_now:?} map={map:?} dims={dims:?} \
                             spawned={spawned:?} species={species:?} creatures={creatures:?}"
                        );
                        if f == sf {
                            emu.call_guest_function(sw.fn_load_scene, &[scene]);
                            eprintln!(
                                "[scene] f={f} issued loadScene({scene}) at {:#x}",
                                sw.fn_load_scene
                            );
                            for fun in &host_scene_tail {
                                emu.call_guest_function(fun | 1, &[]);
                                eprintln!("[scene] f={f} issued tail call {:#x}", fun | 1);
                            }
                        }
                        if f >= sf + 3 {
                            host_scene = None;
                        }
                    }
                }
            }
        }
        if let Some((lv0, sp0, kp0)) = lu_watch {
            let read = |a: u32, l: usize| emu.read_memory(a, l);
            let c = char_base(build, &read);
            let b = emu.read_memory(c + 0xe2, 2);
            let lv = if b.len() == 2 { u16::from_le_bytes([b[0], b[1]]) } else { 0 };
            if lv != lv0 {
                let sp = {
                    let b = emu.read_memory(c + 0x18a, 2);
                    if b.len() == 2 { u16::from_le_bytes([b[0], b[1]]) } else { 0 }
                };
                let kp = emu.read_memory(c + 0x18c, 1).first().copied().unwrap_or(0);
                let (hp, hpmax, spv) = {
                    let h = emu.read_memory(c + 0xf2, 2);
                    let hm = emu.read_memory(c + 0xec, 2);
                    let s = emu.read_memory(c + 0xf8, 4);
                    (
                        if h.len() == 2 { u16::from_le_bytes([h[0], h[1]]) } else { 0 },
                        if hm.len() == 2 { u16::from_le_bytes([hm[0], hm[1]]) } else { 0 },
                        if s.len() == 4 { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { 0 },
                    )
                };
                eprintln!(
                    "[levelup] f={f} LEVELLED: {lv0} -> {lv}, stat points {sp0} -> {sp} (+{}), skill points {kp0} -> {kp} (+{}), hp={hp}/{hpmax} sp={spv}",
                    sp.wrapping_sub(sp0),
                    kp.wrapping_sub(kp0)
                );
                lu_watch = None;
            }
        }
        if host_save_frame == Some(f) {
            match build.save_chain() {
                None => eprintln!("[hostsave] f={f} refused: no save chain on {}", build.name()),
                Some(sc) => {
                    let u32at = |a: u32| {
                        let b = emu.read_memory(a, 4);
                        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    };
                    let names = match sc.names {
                        guest_build::SaveNames::SlPtrs { game, slot } => u32at(game).zip(u32at(slot)),
                        guest_build::SaveNames::Direct { game, slot } => Some((game, slot)),
                    };
                    let chain = host_save::Chain {
                        fn_serialise: sc.fn_serialise,
                        fn_slot_summary: sc.fn_slot_summary,
                        fn_write_record: sc.fn_write_record,
                        scratch: sc.scratch,
                    };
                    // Same order as `emulator.rs::save_begin`: clear the out-param word
                    // FIRST so a stale length cannot be mistaken for this save's.
                    let cleared = emu.write_memory(sc.scratch, &0u32.to_le_bytes());
                    let job = names.and_then(|(ng, ns)| host_save::Job::new(host_save::Reason::Manual, chain, ng, ns));
                    match (cleared, job) {
                        (true, Some(job)) => {
                            if let host_save::Next::Call { addr, args, argc, .. } = job.first_call() {
                                let res = Arc::new(wie_backend::GuestCallResult::new());
                                emu.call_guest_function_out(addr, &args[..argc], res.clone());
                                eprintln!(
                                    "[hostsave] f={f} serialise issued: fn={addr:#x} scratch={:#x} names=({:#x},{:#x})",
                                    sc.scratch, job.name_game, job.name_slot
                                );
                                hs_job = Some((job, res));
                            }
                        }
                        (false, _) => eprintln!("[hostsave] f={f} refused: scratch {:#x} not writable", sc.scratch),
                        (_, None) => eprintln!("[hostsave] f={f} refused: record names did not resolve"),
                    }
                }
            }
        }
        let mut hs_finished = false;
        if let Some((job, res)) = hs_job.as_mut() {
            let scratch_val = {
                let b = emu.read_memory(job.chain.scratch, 4);
                if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
            };
            match job.advance(res.poll(), scratch_val) {
                host_save::Next::Wait => {}
                host_save::Next::Call { addr, args, argc, step } => {
                    *res = Arc::new(wie_backend::GuestCallResult::new());
                    emu.call_guest_function_out(addr, &args[..argc], res.clone());
                    eprintln!("[hostsave] f={f} step {step:?} issued: fn={addr:#x} buf={:#x} len={}", job.buf, job.len);
                }
                host_save::Next::Done => {
                    eprintln!("[hostsave] f={f} DONE: payload {} B + slot summary written", job.len);
                    hs_finished = true;
                }
                host_save::Next::Failed(why) => {
                    eprintln!("[hostsave] f={f} FAILED: {why}");
                    hs_finished = true;
                }
            }
        }
        if hs_finished {
            hs_job = None;
        }
        // TEMPORARY (2026-08-06): the per-frame guest workload. The game rasterises
        // its world from guest code (the WIPI trace shows no DrawImage at all), so
        // this is the number that decides whether the lag is the interpreter or our
        // graphics layer.
        if f % 100 == 0 {
            let (b, fmt, tf, refused) = wie_util::stats::blit_snapshot();
            eprintln!(
                "[blit] f={f} calls={} hle={} px={} px_hle={} | fmt 4bpp={} 8bpp={} other={} | t0={} t1={} t2={} t3={} t4+={} | refused fmt={} tf={} geom={} mem={} null={}",
                b[0], b[1], b[2], b[3],
                fmt[0], fmt[1], fmt[2],
                tf[0], tf[1], tf[2], tf[3], tf[4] + tf[5] + tf[6] + tf[7] + tf[8] + tf[9],
                refused[0], refused[1], refused[2], refused[3], refused[4],
            );
            let (a, fr, walk, bh) = wie_util::stats::heap_snapshot();
            eprintln!(
                "[heap] f={f} allocs={a} frees={fr} LIVE={} list_walk_steps={walk} bucket_header_bytes={:.1} MB",
                a as i64 - fr as i64,
                bh as f64 / 1e6
            );
            // The guest SOUND path, which runs on THIS thread inside the guest's
            // `MC_mdaPlay`. `parses` is the only expensive part; with the parse cache it
            // should settle at the number of distinct sound files (Hero3 has 33) while
            // `plays` keeps climbing. `parses` tracking `plays` means the cache is not
            // being hit and every sound effect is paying for a full SMAF parse + sort
            // inside a frame. `handles` is the bookkeeping that used to grow forever.
            let au = wie_util::stats::audio_snapshot();
            eprintln!(
                "[snd]  f={f} loads={} plays={} parses={} ({:.0} KB) cache={} handles={} catchup={}",
                au[0], au[1], au[2], au[3] as f64 / 1024.0, au[4], au[5], au[6],
            );
            let (instr, svc, batches) = wie_util::stats::snapshot();
            let secs = wall_start.elapsed().as_secs_f64().max(1e-9);
            eprintln!(
                "[work] f={f} instr={instr} ({:.1} M/frame) svc={svc} batches={batches} | {:.1} M instr/s wall, virt {:.2}x",
                instr as f64 / (f.max(1) as f64) / 1e6,
                instr as f64 / secs / 1e6,
                (f as f64 * ms_per_frame as f64 / 1000.0) / secs,
            );
        }
        // Stack high-water, every 500 frames (= 10 s of virtual time at --ms 20).
        // Flat after warm-up = healthy; a steady decline = the leak that overflows
        // the 8 MB `wie-emu` stack on the device.
        if f % 500 == 0 {
            if let Some(reserved) = stack_gauge::reserved_bytes() {
                if stack_reserve_min > reserved {
                    stack_reserve_min = reserved;
                    eprintln!("[stack] f={f} reserve left {} KB (used {} KB)", reserved / 1024, (stack_reserve_start - reserved) / 1024);
                }
            }
        }
        // --find-fb: after the scripted keys have run, hunt for the offscreen
        // framebuffer the game paints its MINIMAP into (measured: created once via
        // MC_grpCreateOffScreenFrameBuffer, painted pixel-by-pixel, then
        // CopyFrameBuffer'd to the screen). Report which CHAR field holds it.
        if find_fb && (f == frames || dumps.contains(&f)) {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let u32at = |a: u32| -> u32 {
                let b = rd(a, 4);
                if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0 }
            };
            let char_ptr = char_base(build, &rd);
            eprintln!("[probe] CHAR = {char_ptr:#x}");
            let mut found = 0;
            for off in (0..0xa800u32).step_by(4) {
                let cand = u32at(char_ptr + off);
                if !(0x4000_0000..0x5000_0000).contains(&cand) {
                    continue;
                }
                // WIPICFramebuffer { w, h, bpl, bpp, buf } - plausible small image?
                let (w, h, bpl, bpp) = (u32at(cand), u32at(cand + 4), u32at(cand + 8), u32at(cand + 12));
                if (8..=512).contains(&w) && (8..=512).contains(&h) && bpl == w * 2 && bpp == 16 {
                    eprintln!("[probe] CHAR+{off:#x} -> fb {cand:#x} {w}x{h} bpl={bpl} bpp={bpp} buf={:#x}", u32at(cand + 16));
                    found += 1;
                }
            }
            eprintln!("[probe] framebuffer-like CHAR fields: {found}");
            // Wider hunt: scan the guest heap for a WIPICFramebuffer-shaped record,
            // then find every static/CHAR word that points at it.
            let mut fbs: Vec<(u32, u32, u32)> = Vec::new();
            for base in (0x4900_0000u32..0x4930_0000).step_by(0x10000) {
                let blk = rd(base, 0x10000);
                if blk.len() < 0x10000 {
                    continue;
                }
                for i in (0..blk.len() - 20).step_by(4) {
                    let g = |k: usize| u32::from_le_bytes([blk[i + k], blk[i + k + 1], blk[i + k + 2], blk[i + k + 3]]);
                    let (w, h, bpl, bpp) = (g(0), g(4), g(8), g(12));
                    if (8..=512).contains(&w) && (8..=512).contains(&h) && bpl == w * 2 && bpp == 16 {
                        fbs.push((base + i as u32, w, h));
                    }
                }
            }
            eprintln!("[probe] heap framebuffer records: {}", fbs.len());
            for (a, w, h) in fbs.iter().take(8) {
                eprintln!("    fb {a:#x} {w}x{h}");
            }
            for (a, w, h) in fbs.iter().take(4) {
                let mut refs = 0;
                for (rbase, rlen) in statics_ranges(build).iter().copied().chain([(char_ptr, 0xb000usize)]) {
                    let blk = rd(rbase, rlen);
                    for i in (0..blk.len().saturating_sub(4)).step_by(4) {
                        let v = u32::from_le_bytes([blk[i], blk[i + 1], blk[i + 2], blk[i + 3]]);
                        if v == *a {
                            let addr = rbase + i as u32;
                            eprintln!(
                                "    -> fb {a:#x} ({w}x{h}) referenced at {addr:#x} (CHAR+{:#x})",
                                addr.wrapping_sub(char_ptr),
                            );
                            refs += 1;
                        }
                    }
                }
                if refs == 0 {
                    eprintln!("    -> fb {a:#x} ({w}x{h}) no static/CHAR reference found");
                }
            }
        }
        // --map-check: prove that the host can render the game's minimap ITSELF, from
        // the map arrays in RAM, without ever opening the guest's map window.
        //
        // Reimplements guest 0x17ba44 (the map window's ENTER handler) exactly:
        //   MAP  = *(*(CHAR+0x9c90)) + 8      (KTF indirect handle, +8 data offset)
        //   w,h  = i32 @MAP+0x48 / +0x4c      (tiles); the buffer is 2w x 2h
        //   attr = *(*(MAP+0x60)) + 8         u8[w*h], bit 7 = blocked   (guest 0x17a6f8)
        //   evt  = *(*(MAP+0x68)) + 8         i8[w*h], >0 = event tile   (guest 0x17a6b4)
        // per tile (guest 0x17b874): blocked ? 0x5b5b5b : 0xd9d4be, then event -> 0x225acc
        // (except at tile 0,0); per pixel (guest 0x17b8e4) a corner blend with the two
        // neighbours in that quadrant, averaged per channel, skipped when the base
        // matches either neighbour.
        // The run then compares that against the buffer the guest itself painted, so a
        // mismatch is a bug in the reimplementation and not a matter of opinion.
        if map_check && dumps.contains(&f) {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let u32at = |a: u32| -> Option<u32> {
                let b = rd(a, 4);
                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            let cp = char_base(build, &rd);
            // ★ The SHIPPING renderer, included from wie_android — not a copy of it.
            let render = render_minimap_rgb(&rd, cp, chain).map(|(w, h, px)| (u32::from(w), u32::from(h), px));
            if let Some((w, h, _)) = &render {
                // The marker is drawn to the SCREEN, not into the buffer, so it is
                // reported rather than stamped: the diff below must stay like-for-like.
                match player_tile(&rd, cp, (*w / 2) as i32, (*h / 2) as i32, chain) {
                    Some((tx, ty)) => {
                        // The attribute byte under the marker, for context. NOT an
                        // assertion: the tile grid is a coarse quantisation of a
                        // pixel-space position, so standing against a wall can round
                        // onto it — and the game's own marker lands there too, which
                        // is the only standard that matters here.
                        let mapw = (*w / 2) as i32;
                        // Same hop count as the renderer, whichever build this is.
                        let hop = |field: u32| -> u32 {
                            let first = u32at(field).unwrap_or(0);
                            match chain.data_off {
                                Some(off) => u32at(first).unwrap_or(0).wrapping_add(off),
                                None => first,
                            }
                        };
                        let attr_p = hop(hop(cp + MAP_HANDLE_OFF) + 0x60);
                        let a = rd(attr_p + (ty * mapw + tx) as u32, 1).first().copied().unwrap_or(0xff);
                        eprintln!("[map] f={f}: player tile ({tx},{ty}) of {}x{}  attr={a:#04x}", w / 2, h / 2);
                    }
                    None => eprintln!("[map] f={f}: player tile UNRESOLVED"),
                }
            }
            let Some((fw, fh, ours)) = render else {
                eprintln!("[map] f={f}: could not resolve the map arrays");
                continue;
            };
            // The guest's own buffer, for comparison (only valid once its map window
            // has painted at least once).
            let theirs = (|| -> Option<(u32, u32, Vec<u32>)> {
                let (handle, gw, gh) = wie_backend::last_offscreen_copy()?;
                // `WIPICIndirectPtr` -> payload, per carrier runtime: `*(h)+8` on KTF,
                // the identity on LGT. Both the framebuffer record and its pixel
                // buffer are reached that way.
                let payload = |p: u32| -> Option<u32> {
                    match chain.data_off {
                        Some(off) => u32at(p).map(|v| v.wrapping_add(off)),
                        None => Some(p),
                    }
                };
                let sp = payload(handle)?;
                let hdr = rd(sp, 20);
                if hdr.len() < 20 {
                    return None;
                }
                let g = |i: usize| u32::from_le_bytes([hdr[i], hdr[i + 1], hdr[i + 2], hdr[i + 3]]);
                if g(0) != gw || g(4) != gh || g(12) != 16 {
                    return None;
                }
                let data = payload(g(16))?;
                let raw = rd(data, (gw * gh * 2) as usize);
                if raw.len() < (gw * gh * 2) as usize {
                    return None;
                }
                let px = (0..(gw * gh) as usize)
                    .map(|i| {
                        let v = u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
                        let (r, gg, b) = ((v >> 11) & 0x1f, (v >> 5) & 0x3f, v & 0x1f);
                        ((u32::from(r) * 255 / 31) << 16) | ((u32::from(gg) * 255 / 63) << 8) | (u32::from(b) * 255 / 31)
                    })
                    .collect();
                Some((gw, gh, px))
            })();
            match theirs {
                None => eprintln!("[map] f={f}: ours {fw}x{fh} (guest buffer not available yet)"),
                Some((gw, gh, _)) if (gw, gh) != (fw, fh) => {
                    eprintln!("[map] f={f}: SIZE MISMATCH ours {fw}x{fh} vs guest {gw}x{gh}");
                }
                Some((gw, gh, theirs)) => {
                    // RGB565 is lossy, so compare after quantising ours the same way.
                    let q = |c: u32| {
                        let (r, g, b) = ((c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff);
                        let v = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3);
                        let (r, g, b) = ((v >> 11) & 0x1f, (v >> 5) & 0x3f, v & 0x1f);
                        ((r * 255 / 31) << 16) | ((g * 255 / 63) << 8) | (b * 255 / 31)
                    };
                    let mut bad = 0usize;
                    let mut samples = Vec::new();
                    for i in 0..ours.len() {
                        if q(ours[i]) != theirs[i] {
                            bad += 1;
                            if samples.len() < 8 {
                                samples.push((i as u32 % gw, i as u32 / gw, q(ours[i]), theirs[i]));
                            }
                        }
                    }
                    eprintln!(
                        "[map] f={f}: {gw}x{gh}  mismatch {bad}/{} ({:.3}%)",
                        ours.len(),
                        bad as f64 * 100.0 / ours.len() as f64
                    );
                    for (x, y, a, b) in samples {
                        eprintln!("        ({x},{y}) ours {a:#08x} guest {b:#08x}");
                    }
                }
            }
        }
        // --map-hunt: find the per-map object on a build whose chain is not known yet.
        //
        // Structural rather than address-based, because a KTF address means nothing in
        // the LGT compile (MIGRATION_LGT.md §1) while the STRUCT LAYOUT does carry
        // over. So: sweep every word of the statics and of the CHAR record, try both
        // pointer conventions on it, and keep the ones that land on something
        // map-shaped — plausible tile dimensions at +0x48/+0x4c with two readable
        // `w*h` byte arrays at +0x60/+0x68. Reported with the blocked/event tile
        // counts, which is what identifies the right candidate against the picture the
        // game's own map window paints.
        if map_hunt && dumps.contains(&f) {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let u32at = |a: u32| -> Option<u32> {
                let b = rd(a, 4);
                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            let cp = char_base(build, &rd);
            let heap = |v: u32| (0x4000_0000..0x5000_0000).contains(&v);
            // Both conventions, tried side by side: `Some(8)` = KTF handle, `None` =
            // LGT direct. The hunt does not assume the build's own answer is right,
            // because "which convention" is exactly one of the things being confirmed.
            let convs: [(&str, Option<u32>); 2] = [("direct", None), ("handle+8", Some(8))];
            let mut hits = 0usize;
            // Windows to sweep: the statics (which on LGT contain the CHAR record) and
            // the CHAR record itself (which on KTF is on the heap).
            let mut windows: Vec<(u32, usize)> = statics_ranges(build).to_vec();
            if heap(cp) {
                windows.push((cp, 0xb000));
            }
            for (wbase, wlen) in windows {
                let blk = rd(wbase, wlen);
                for i in (0..blk.len().saturating_sub(4)).step_by(4) {
                    let field = wbase + i as u32;
                    let raw = u32::from_le_bytes([blk[i], blk[i + 1], blk[i + 2], blk[i + 3]]);
                    if !heap(raw) {
                        continue;
                    }
                    for (mname, madj) in convs {
                        let Some(map) = (match madj {
                            None => Some(raw),
                            Some(off) => u32at(raw).map(|v| v.wrapping_add(off)).filter(|v| heap(*v)),
                        }) else {
                            continue;
                        };
                        let (Some(mw), Some(mh)) = (u32at(map + 0x48), u32at(map + 0x4c)) else { continue };
                        let (mw, mh) = (mw as i32, mh as i32);
                        if !(1..=200).contains(&mw) || !(1..=200).contains(&mh) {
                            continue;
                        }
                        let n = (mw * mh) as usize;
                        for (aname, aadj) in convs {
                            let arr = |off: u32| -> Option<Vec<u8>> {
                                let h = u32at(map + off).filter(|v| heap(*v))?;
                                let p = match aadj {
                                    None => h,
                                    Some(a) => u32at(h).map(|v| v.wrapping_add(a)).filter(|v| heap(*v))?,
                                };
                                let b = rd(p, n);
                                (b.len() == n).then_some(b)
                            };
                            let (Some(attr), Some(event)) = (arr(0x60), arr(0x68)) else { continue };
                            let blocked = attr.iter().filter(|b| **b & 0x80 != 0).count();
                            let events = event.iter().filter(|b| (**b as i8) > 0).count();
                            // An all-zero or all-blocked "attribute" array is what a
                            // random heap block looks like; a real map has some of both.
                            let plausible = blocked > 0 && blocked < n;
                            eprintln!(
                                "[hunt] f={f} field {field:#x} (CHAR+{:#x}) {mname} -> map {map:#x} {mw}x{mh} arrays {aname}: blocked {blocked}/{n} events {events}{}",
                                field.wrapping_sub(cp),
                                if plausible { "  <= PLAUSIBLE" } else { "" }
                            );
                            // The event tiles, listed: these are the map's warps, so
                            // they are the coordinates to walk a probe run TOWARDS when
                            // the next thing to verify is a second map.
                            //
                            // ⚠ Printed per CANDIDATE, and the `{aname}` convention is
                            // part of the line, because the wrong convention can still
                            // produce a "plausible" array: on KTF the `direct` reading
                            // of the same map object yields 818 blocked / 21 events and
                            // the `handle+8` one 828 / 12, and only the latter is real
                            // (it is the one whose render matches the guest's own
                            // buffer). The pixel diff is the arbiter, not this list.
                            if plausible {
                                let evs: Vec<String> = (0..n)
                                    .filter(|i| (event[*i] as i8) > 0)
                                    .take(32)
                                    .map(|i| format!("({},{})={}", i as i32 % mw, i as i32 / mw, event[i] as i8))
                                    .collect();
                                eprintln!(
                                    "[hunt]   arrays {aname}: player {:?} event tiles: {}",
                                    player_tile(&rd, cp, mw, mh, chain),
                                    evs.join(" ")
                                );
                            }
                            hits += 1;
                        }
                    }
                }
            }
            eprintln!("[hunt] f={f}: {hits} map-shaped candidates (CHAR {cp:#x})");
        }
        if trace_state && dumps.contains(&f) {
            // Map signature (CHAR+0x9bcc, walk-invariant / map-sensitive) + the
            // framebuffer the guest last copied to screen (the HUD's minimap source).
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let cp = char_base(build, &rd);
            let sig = rd(cp + 0x9bcc, 8);
            eprintln!(
                "[mm] f={f} sig={} copy={:x?}",
                sig.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                wie_backend::last_offscreen_copy()
            );
        }
        if walk_cal && f % 4 == 0 {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let i32at = |a: u32| -> i32 {
                let b = rd(a, 4);
                if b.len() == 4 { i32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { i32::MIN }
            };
            let cp = char_base(build, &rd);
            // Map dims through the same chain the renderer uses, so the tile is
            // validated against the real grid.
            let u32at = |a: u32| -> Option<u32> {
                let b = rd(a, 4);
                (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            let hop = |field: u32| -> Option<u32> {
                let first = u32at(field)?;
                match chain.data_off {
                    Some(off) => u32at(first).map(|v| v.wrapping_add(off)),
                    None => Some(first),
                }
            };
            let dims = hop(cp + MAP_HANDLE_OFF).map(|m| (u32at(m + 0x48).unwrap_or(0) as i32, u32at(m + 0x4c).unwrap_or(0) as i32));
            let (mw, mh) = dims.unwrap_or((0, 0));
            let tile = player_tile(&rd, cp, mw, mh, chain);
            eprintln!(
                "[walkcal] f={f} cam=({},{}) cam2=({},{}) char9e8c=({},{}) tile={tile:?} map=({mw},{mh})",
                i32at(0x150d914), i32at(0x150d918), i32at(0x150d91c), i32at(0x150d920),
                i32at(cp + 0x9e8c), i32at(cp + 0x9e90),
            );
            // The player OBJECT's leading words, to locate its pixel-position fields
            // (the tile bytes at +0x27/+0x28 are a coarse quantisation of something).
            if let Some(first) = u32at(cp + 0x9bd0).filter(|v| (0x4000_0000..0x5000_0000).contains(v)) {
                let obj = match chain.data_off {
                    Some(off) => u32at(first).map(|v| v.wrapping_add(off)).unwrap_or(0),
                    None => first,
                };
                let words: Vec<String> = (0..24).map(|i| format!("{:x}", u32at(obj + i * 4).unwrap_or(0))).collect();
                eprintln!("[walkobj] f={f} obj={obj:#x} w={}", words.join(","));
            }
            // Once per map: the collision + event grids, hex row per tile row, so the
            // fitting script can align them against a frame dump.
            if walk_cal_map != (mw, mh) && mw > 0 {
                walk_cal_map = (mw, mh);
                let dump_grid = |name: &str, off: u32| {
                    if let Some(p) = hop(cp + MAP_HANDLE_OFF).and_then(|m| hop(m + off)) {
                        let b = rd(p, (mw * mh) as usize);
                        for (row, chunk) in b.chunks(mw as usize).enumerate() {
                            let hex: String = chunk.iter().map(|v| format!("{v:02x}")).collect();
                            eprintln!("[walkgrid] f={f} {name} row={row} {hex}");
                        }
                    }
                };
                dump_grid("attr", 0x60);
                dump_grid("event", 0x68);
            }
        }
        // --- --anim: the player's animation clock, one line per frame ----------
        if let Some((a, b)) = anim_window {
            if f >= a && f <= b {
                let rd = |ad: u32, n: usize| emu.read_memory(ad, n);
                let cp = char_base(build, &rd);
                let u8at = |o: u32| -> i32 { rd(cp + o, 1).first().map(|v| *v as i32).unwrap_or(-1) };
                let i8at = |o: u32| -> i32 { rd(cp + o, 1).first().map(|v| *v as i8 as i32).unwrap_or(-999) };
                let i16at = |o: u32| -> i32 {
                    let x = rd(cp + o, 2);
                    if x.len() == 2 { i16::from_le_bytes([x[0], x[1]]) as i32 } else { -999 }
                };
                let arr = |o: u32, n: usize| -> String {
                    rd(cp + o, n).iter().map(|v| format!("{:3}", *v as i8)).collect::<Vec<_>>().join(",")
                };
                eprintln!(
                    "[anim] f={f} state={} pose={} tick={} face={} step={} hit={} mode={} rec={} script=[{}] alt=[{}] win={}",
                    u8at(0x9bb6),
                    i8at(0x9bb5),
                    i16at(0x9bc8),
                    u8at(0x9bb4),
                    i8at(0x24f),
                    i8at(0x250),
                    i16at(0x128),      // state 4's sub-mode (`[r6,#0x74]`)
                    i8at(0x251),       // state 4's recovery countdown (`r6+0x19d`)
                    arr(0x23f, 8),
                    arr(0x247, 8),
                    u8at(0xa0c0),
                );
            }
        }
        if trace_state && f % 20 == 0 {
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let u32at = |a: u32| -> u32 {
                let b = rd(a, 4);
                if b.len() == 4 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { 0xffff_ffff }
            };
            let cp = char_base(build, &rd);
            // state byte moved between builds (KTF +0x7c / LGT +0x78); the window id
            // did not. `window` is the one that says WHICH of the game's own windows
            // owns the screen (3 = none, 4 = script, 8 = map), so it is what tells us
            // whether a key press opened the map at all.
            let st = u32at(cp + build.state_off());
            let sel = rd(cp + 0x9c6c, 1).first().copied().unwrap_or(0xff);
            let win = rd(cp + 0xa0c0, 1).first().copied().unwrap_or(0xff);
            eprintln!("[state] f={f} state={st} selflag={sel} window={win}");
        }
        if let Some(dir) = &char_dump {
            if dumps.contains(&f) {
                let rd = |a: u32, n: usize| emu.read_memory(a, n);
                let cp = char_base(build, &rd);
                let blob = rd(cp, 0xb000);
                let _ = fs::create_dir_all(dir);
                let path = dir.join(format!("char_{f:05}.bin"));
                fs::write(&path, &blob).unwrap();
                // The static/global data area too: a current-map id is far more likely
                // to be a global than a CHAR field. (KTF keeps its RWPI statics around
                // sl = 0x1b2c40; LGT has absolute `.data` + `.bss`.)
                for (gbase, glen) in statics_ranges(build).iter().copied() {
                    let g = rd(gbase, glen);
                    fs::write(dir.join(format!("glob_{f:05}_{gbase:#010x}.bin")), &g).unwrap();
                }
                eprintln!("[probe] CHAR dump {path:?} base={cp:#x} len={}", blob.len());
            }
        }
        if verify_offsets && dumps.contains(&f) {
            // MIGRATION_LGT.md §2.11 in one line. Offsets MIRROR items.rs (that file is
            // not include!d here — only guest_anchors is), so if items.rs ever moves one
            // this must follow; the reference block exists precisely so a wrong mirror
            // shows up as garbage against the on-screen status numbers.
            let rd = |a: u32, n: usize| emu.read_memory(a, n);
            let cp = char_base(build, &rd);
            let u8at = |a: u32| rd(a, 1).first().copied();
            let i8at = |a: u32| u8at(a).map(|b| b as i8);
            let u16at = |a: u32| { let b = rd(a, 2); (b.len() == 2).then(|| u16::from_le_bytes([b[0], b[1]])) };
            let u32at = |a: u32| { let b = rd(a, 4); (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
            // VERIFIED reference (§2.7): if these don't match the status screen the base
            // is wrong and nothing below means anything.
            eprintln!(
                "[verify] f={f} CHAR={cp:#x} | REF lv={:?} exp={:?}/{:?} hp={:?}/{:?} sp={:?} win={:?} state={:?} sel={:?}",
                u16at(cp + 0xe2), u32at(cp + 0xe4), u32at(cp + 0xe8),
                u16at(cp + 0xf2), u32at(cp + 0xec), u32at(cp + 0xf8),
                u8at(cp + 0xa0c0), u32at(cp + build.state_off()), u8at(cp + 0x9c6c),
            );
            // 🔶 #6 badges (0x18a u16 read as i16 / 0x18c u8), #18 skill-cast (0x9bb5,0x2c9)
            eprintln!(
                "[verify]   #6 badge stat={:?} skill={:?} | #18 cast mode={:?} cat={:?}",
                u16at(cp + 0x18a).map(|v| v as i16), u8at(cp + 0x18c),
                u8at(cp + 0x9bb5), u8at(cp + 0x2c9),
            );
            // 🔶 #5 quickbar: type/itemref/skillref/skillidx per slot (0x252/0x257/0x25c/0x26c)
            for s in 0..5u32 {
                eprintln!(
                    "[verify]   #5 qb[{s}] type={:?} itemref={:?} skillref={:?} cell={:?}",
                    u8at(cp + 0x252 + s), u8at(cp + 0x257 + s), u8at(cp + 0x25c + s), i8at(cp + 0x26c + s),
                );
            }
            // 🔶 #5b on-field skill sheet ids (0xd1+i) — the byte `items::skill_icon`
            // gates the atlas on (valid 4..=10, atlas index = value-4). §2.18-(2)
            // suspected the offset itself on LGT, but static RE closed that: the LGT
            // sheet loader (fn 0x4957c, template "res/skill/s4_bm" at 0x5e2e0) reads
            // (CHAR+0x28)+0x8c+0x1d+i = CHAR+0xd1+i — the SAME offset as KTF. What is
            // left to verify live is the VALUE (populated on party load, 0 before).
            eprintln!(
                "[verify]   #5b sheet[0]={:?} sheet[1]={:?} (want 4..=10; atlas=v-4)",
                u8at(cp + 0xd1), u8at(cp + 0xd2),
            );
            // 🔶 #7 secondary weapon (0x524 present / 0x513 tab / 0x514 slot, all i8)
            eprintln!(
                "[verify]   #7 equip present={:?} tab={:?} slot={:?}",
                i8at(cp + 0x524), i8at(cp + 0x513), i8at(cp + 0x514),
            );
            // 🔶 #8 save byte: area = deref(CHAR+0x9c90) [+0x11d]. Build-aware hop.
            let area = u32at(cp + 0x9c90).and_then(|first| match build.container_data_off() {
                None => Some(first),
                Some(off) => u32at(first).map(|blk| blk + off),
            });
            eprintln!("[verify]   #8 area={area:#x?} can_save={:?} (0=refuse)", area.and_then(|a| i8at(a + 0x11d)));
        }
        if dumps.contains(&f) {
            // force a paint of the current guest framebuffer
            emu.handle_event(Event::Redraw);
            let _ = emu.tick();
            let px = last.lock().unwrap().clone();
            let path = out.join(format!("frame_{f:05}_{w}x{h}.rgba"));
            let mut raw = Vec::with_capacity(px.len() * 4);
            for p in &px {
                raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]);
            }
            fs::write(&path, &raw).unwrap();
            let opaque = px.iter().filter(|p| (*p >> 24) != 0).count();
            let nonblack = px.iter().filter(|p| (*p & 0x00ff_ffff) != 0).count();
            eprintln!("[probe] frame {f}: wrote {path:?} ({} px, opaque {opaque}, non-black {nonblack})", px.len());
        }
        if dumps.contains(&f) {
            for (base, len) in &dump_mem {
                let bytes = emu.read_memory(*base, *len);
                let path = out.join(format!("mem_{f:05}_{base:#010x}.bin"));
                fs::write(&path, &bytes).unwrap();
                let nz = bytes.iter().filter(|b| **b != 0).count();
                eprintln!("[probe] frame {f}: mem {base:#x} len {} -> {path:?} ({} read, {nz} non-zero)", len, bytes.len());
            }
        }
    }
    if profile {
        let leaf = prof_leaf.lock().unwrap();
        let total: u64 = leaf.values().sum();
        let mut v: Vec<(u32, u64)> = leaf.iter().map(|(a, c)| (*a, *c)).collect();
        v.sort_by_key(|(_, c)| core::cmp::Reverse(*c));
        eprintln!("[prof] {total} samples, {} distinct leaf PCs", v.len());
        for (pc, c) in v.iter().take(30) {
            // Guest address -> client.bin file offset (see CLAUDE.md: guest = fo + 0x100000).
            eprintln!("[prof] {:6.2}%  pc={pc:#010x}  fo={:#x}  ({c})", *c as f64 * 100.0 / total as f64, pc.wrapping_sub(0x100000));
        }
        let caller = prof_caller.lock().unwrap();
        let mut cv: Vec<((u32, u32), u64)> = caller.iter().map(|(k, c)| (*k, *c)).collect();
        cv.sort_by_key(|(_, c)| core::cmp::Reverse(*c));
        eprintln!("[prof] top pc<-caller pairs:");
        for ((pc, lr), c) in cv.iter().take(15) {
            eprintln!("[prof]   {:6.2}%  pc={pc:#010x} <- lr={lr:#010x}", *c as f64 * 100.0 / total as f64);
        }
    }
    if let Some(k) = portrait_keep_from {
        eprintln!(
            "[keep] summary from frame {k}: {keep_canonical} canonical frame(s), {keep_offers} picture(s) offered, the ring changed {keep_changes} time(s), shown look {:?} final {}",
            keep.look().map(|l| format!("{l:#018x}")),
            keep.look().is_some_and(|l| keep.is_final(l))
        );
    }
    eprintln!("[probe] done");
}
