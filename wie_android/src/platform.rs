//! `Platform` implementation for Android (ported from `wie_cli`'s
//! `WieCliPlatform`). Screen paints route to the winit UI thread through an
//! `EventLoopProxy`; filesystem/database live in app-private scoped storage.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use winit::event_loop::EventLoopProxy;

use wie_backend::{
    AudioSink, DatabaseRepository, Filesystem, Instant, Network, Platform, Screen,
    canvas::Image,
};

use crate::{
    UserEvent,
    audio_sink::{AndroidAudioSink, MidiMsg, WaveMsg, spawn_audio_thread},
    database::AndroidDatabaseRepository,
    filesystem::AndroidFilesystem,
    network::AndroidNetwork,
};

pub struct AndroidPlatform {
    screen: AndroidScreen,
    filesystem: AndroidFilesystem,
    database: AndroidDatabaseRepository,
    network: AndroidNetwork,
    wave_tx: Sender<WaveMsg>,
    midi_tx: Sender<MidiMsg>,
}

impl AndroidPlatform {
    pub fn new(screen: AndroidScreen, data_dir: PathBuf, paused: Arc<AtomicBool>) -> Self {
        // Spawn the rodio output thread + MIDI synth here (worker thread, so
        // ndk_context is populated for cpal's AAudio backend). `paused` is the
        // shared background/lock gate: the synth emits silence while it is set.
        //
        // The synth's cost settings are read ONCE here, because the `Synth` is built
        // once — see `config::read_audio_lite`. Default (no file) = today's sound.
        // ★ 57차: HEAT SAVER implies the lite preset — reverb/chorus run even in
        // silence (~13 % of a core) and this mode exists for the device that
        // cannot afford that. `wie_audio_lite.txt` still works on its own.
        let lite = crate::config::read_audio_lite(&data_dir) || {
            let heat = crate::config::read_heat_saver(&data_dir);
            if heat {
                log::info!("heat saver on: audio synth runs the lite preset");
            }
            heat
        };
        let (wave_tx, midi_tx) = spawn_audio_thread(paused, lite);
        Self {
            screen,
            filesystem: AndroidFilesystem::new(data_dir.clone()),
            database: AndroidDatabaseRepository::new(data_dir.clone()),
            // `data_dir` too: the network layer re-reads BOTH the configured
            // server endpoint and the account identity when it uses them, so
            // either can change without a rebuild or a restart.
            network: AndroidNetwork::new(data_dir),
            wave_tx,
            midi_tx,
        }
    }
}

impl Platform for AndroidPlatform {
    fn screen(&self) -> &dyn Screen {
        &self.screen
    }

    fn now(&self) -> Instant {
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        Instant::from_epoch_millis(since_epoch.as_millis() as _)
    }

    fn database_repository(&self) -> &dyn DatabaseRepository {
        &self.database
    }

    fn filesystem(&self) -> &dyn Filesystem {
        &self.filesystem
    }

    fn network(&self) -> Option<&dyn Network> {
        Some(&self.network)
    }

    fn audio_sink(&self) -> Box<dyn AudioSink> {
        // rodio PCM (play_wave) + oxisynth SF3 MIDI BGM, mixed.
        Box::new(AndroidAudioSink::new(self.wave_tx.clone(), self.midi_tx.clone()))
    }

    fn write_stdout(&self, buf: &[u8]) {
        if let Ok(s) = std::str::from_utf8(buf) {
            log::info!("[guest stdout] {s}");
        }
    }

    fn write_stderr(&self, buf: &[u8]) {
        if let Ok(s) = std::str::from_utf8(buf) {
            log::warn!("[guest stderr] {s}");
        }
    }

    fn exit(&self) {
        // `MC_knlExit`. Only records the request — the worker loop picks it up, tries
        // one last save, and then asks the UI thread to tear the app down
        // (`crate::request_exit`). Doing any of that here would run it on whichever
        // thread the guest happened to be on, inside a WIPI call, with the guest's
        // stack still live.
        crate::request_exit();
    }

    fn vibrate(&self, duration_ms: u64, _intensity: u8) {
        // JNI Vibrator. `vibrate(long)` has no amplitude arg, so intensity is
        // ignored for v1 (the game's setting is effectively on/off anyway).
        crate::vibration::vibrate(duration_ms);
    }
}

/// `Screen` impl. `request_redraw` flips an atomic the worker polls; `paint`
/// serialises the frame and hands it to the UI thread for blitting.
///
/// ★ 2026-08-03: this used to carry a `suppress` flag that DROPPED painted frames,
/// so the player would not see the guest's map window while the host drove it open
/// and shut to refresh the minimap. The host renders the minimap itself now
/// (`items::render_minimap`), so no guest window is ever opened on the player's
/// behalf and there is nothing to hide.
pub struct AndroidScreen {
    proxy: EventLoopProxy<UserEvent>,
    should_redraw: Arc<AtomicBool>,
    width: u32,
    height: u32,
    /// Hash of the last frame handed to the UI thread (57차). The guest repaints
    /// on its frame timer whether or not anything changed — the title screen
    /// paints ~69 identical frames a second — and every one of those used to ride
    /// the full convert + send + blit + present pipeline. When the raw framebuffer
    /// bytes hash the same, the frame is dropped HERE, before even the RGB565
    /// conversion, and the UI keeps showing its (identical) copy.
    last_paint_hash: std::sync::atomic::AtomicU64,
}

impl AndroidScreen {
    pub fn new(proxy: EventLoopProxy<UserEvent>, should_redraw: Arc<AtomicBool>, width: u32, height: u32) -> Self {
        Self {
            proxy,
            should_redraw,
            width,
            height,
            last_paint_hash: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

/// Paint outcomes since the last gauge read (57차): frames handed to the UI vs
/// frames dropped as identical. The emulator's 30 s throughput block prints the
/// rates (`emu paint: …`) — on the title screen `skipped` should carry nearly the
/// whole guest framerate, and `skipped 0` during a static screen means the hash
/// gate has stopped working.
static PAINT_SENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PAINT_SKIPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `(sent, skipped)` since the last call.
pub fn paint_stats_take() -> (u64, u64) {
    (PAINT_SENT.swap(0, Ordering::Relaxed), PAINT_SKIPPED.swap(0, Ordering::Relaxed))
}

/// FNV-1a over 8-byte gulps — not the canonical byte-wise FNV, just a fast,
/// well-mixed 64-bit content fingerprint (~30 k iterations for a 496x240 16bpp
/// frame). A collision shows one stale frame until the next real change; at
/// 64 bits over near-identical inputs that is not a case worth code.
fn frame_hash(bytes: &[u8]) -> u64 {
    const PRIME: u64 = 0x100_0000_01b3;
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ bytes.len() as u64;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        h = (h ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(PRIME);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rem.len()].copy_from_slice(rem);
        h = (h ^ u64::from_le_bytes(tail)).wrapping_mul(PRIME);
    }
    h
}

/// 5-bit / 6-bit channel expansion tables matching `Rgb565Pixel::to_color`
/// exactly (`(v * 255 + half) / max`), so the fast path below is pixel-identical
/// to the old `colors()` route.
const fn expand_table<const N: usize>(max: u32) -> [u32; N] {
    let mut t = [0u32; N];
    let mut i = 0;
    while i < N {
        t[i] = (i as u32 * 255 + max / 2) / max;
        i += 1;
    }
    t
}
const EXPAND5: [u32; 32] = expand_table::<32>(31);
const EXPAND6: [u32; 64] = expand_table::<64>(63);

/// How many retired frame buffers to keep. Two covers the only overlap that exists —
/// the worker filling frame N+1 while the UI thread still owns frame N — and no more:
/// a deeper pool would just hold megabytes hostage (476 KB per buffer at 496x240, and
/// more in immersive mode, where the guest framebuffer tracks the screen aspect).
const FRAME_POOL_MAX: usize = 2;

/// Frame buffers the UI thread has finished with, waiting to be refilled by [`paint`].
///
/// ★ A process global rather than a handle threaded through `AndroidPlatform` /
/// `AndroidScreen`: the two ends sit in different modules on different threads (`paint`
/// on the emulator worker, the hand-back on the winit UI thread) and the only channel
/// between them today is the `EventLoopProxy`, which carries a buffer one way only.
/// Passing an `Arc` down would have meant changing `emulator::spawn` and `lib.rs` too,
/// for no behavioural difference — the pool is process-wide either way, because there is
/// exactly one emulator worker and one UI thread for the life of the process.
///
/// `Mutex` (not a lock-free stack) because it is touched twice per frame at ~30 fps and
/// held for a `push`/`pop`; contention is not a thing that can happen here.
static FRAME_POOL: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());

/// Hand a frame buffer the UI thread is done with back for reuse. The LENGTH is dropped;
/// the capacity is the whole point. Over the cap the buffer is simply freed, so the pool
/// cannot grow without bound however the two threads happen to interleave.
pub fn recycle_frame(mut frame: Vec<u32>) {
    // A zero-capacity Vec is not worth a lock (and never comes from `paint`).
    if frame.capacity() == 0 {
        return;
    }
    // A poisoned lock means some thread panicked mid-push. Dropping the buffer is the
    // right answer — the next `paint` just allocates — so the error is swallowed rather
    // than unwrapped: this is the UI thread, and a panic here kills the app.
    if let Ok(mut pool) = FRAME_POOL.lock()
        && pool.len() < FRAME_POOL_MAX
    {
        frame.clear();
        pool.push(frame);
    }
}

/// Take a buffer with room for `len` pixels, or a fresh one when the pool has nothing
/// usable. Buffers that are too SMALL are dropped rather than grown: the only way to get
/// one is a change of guest framebuffer size, which happens at most once per process, and
/// a `reserve` would otherwise re-grow the same undersized buffer every single frame.
fn take_frame(len: usize) -> Vec<u32> {
    if let Ok(mut pool) = FRAME_POOL.lock() {
        while let Some(mut frame) = pool.pop() {
            if frame.capacity() >= len {
                frame.clear();
                return frame;
            }
        }
    }
    Vec::with_capacity(len)
}

impl Screen for AndroidScreen {
    fn request_redraw(&self) -> wie_util::Result<()> {
        self.should_redraw.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn paint(&self, image: &dyn Image) {
        // One call = one frame the GUEST finished. See `stats::GUEST_FRAMES`.
        wie_util::stats::add_frame();

        // ★ Fast path for the 16bpp guest framebuffer (which is what every WIPI
        // frame is): convert RGB565 -> 0xAARRGGBB straight from the raw bytes.
        // The old route built a Vec<Color> first — a 16-byte-per-pixel temporary
        // (~1.9 MB per frame at 496x240) allocated and thrown away every frame,
        // plus float-free but branchy per-channel math in `to_color`.
        //
        // ★ The destination comes from `take_frame`, not from `collect`: at 496x240 the
        // buffer is 476 KB and it is thrown away and re-allocated EVERY frame, ~25 times
        // a second, on the thread that also has to hit the guest's frame budget. Extending
        // a pooled buffer reuses that allocation. `extend` on a `Vec` with enough capacity
        // cannot reallocate, so the pooled path never touches the allocator at all.
        // ★ 57차: identical-content frames stop here. `swap` (not load+store) so
        // the check is one atomic op; the worker is the only writer.
        if image.bytes_per_pixel() == 2 {
            let hash = frame_hash(&image.raw());
            if self.last_paint_hash.swap(hash, Ordering::Relaxed) == hash {
                PAINT_SKIPPED.fetch_add(1, Ordering::Relaxed);
                return;
            }
        } else {
            // The (never-taken today) non-16bpp path is not hashed; make sure it
            // can never be skipped against a stale 16bpp hash.
            self.last_paint_hash.store(0, Ordering::Relaxed);
        }

        let pixel_count = (image.width() * image.height()) as usize;
        let mut data = take_frame(pixel_count);
        if image.bytes_per_pixel() == 2 {
            let raw = image.raw();
            data.extend(raw.chunks_exact(2).map(|c| {
                let p = u16::from_le_bytes([c[0], c[1]]) as usize;
                0xff00_0000 | (EXPAND5[(p >> 11) & 0x1f] << 16) | (EXPAND6[(p >> 5) & 0x3f] << 8) | EXPAND5[p & 0x1f]
            }));
        } else {
            data.extend(
                image
                    .colors()
                    .iter()
                    .map(|c| ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)),
            );
        }

        crate::game_options::observe(&data, image.width() as i32, image.height() as i32);

        // UI thread may be gone during teardown; a failed send is non-fatal.
        PAINT_SENT.fetch_add(1, Ordering::Relaxed);
        let _ = self.proxy.send_event(UserEvent::Paint(data));
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::frame_hash;

    #[test]
    fn equal_frames_hash_equal() {
        let a = vec![0x5au8; 496 * 240 * 2];
        assert_eq!(frame_hash(&a), frame_hash(&a.clone()));
    }

    #[test]
    fn a_single_changed_byte_changes_the_hash_wherever_it_is() {
        // Head, middle, tail-remainder territory — the tail fold is the branch a
        // lazy hash gets wrong, so a non-multiple-of-8 length is used on purpose.
        let base = vec![0u8; 1001];
        let h0 = frame_hash(&base);
        for pos in [0usize, 500, 995, 1000] {
            let mut b = base.clone();
            b[pos] = 1;
            assert_ne!(frame_hash(&b), h0, "byte {pos} did not affect the hash");
        }
    }

    #[test]
    fn length_is_part_of_the_hash() {
        // A frame of zeros must not collide with a longer frame of zeros: the
        // guest framebuffer size is fixed per boot, but the hash should not rely
        // on that to stay sound.
        assert_ne!(frame_hash(&[0u8; 8]), frame_hash(&[0u8; 16]));
    }
}
