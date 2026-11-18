#![no_std]
extern crate alloc;

mod audio_sink;
pub mod canvas;
mod database;
mod executor;
mod guest_call;
mod platform;
mod screen;
mod system;
mod task;
mod task_runner;
mod time;

pub use self::{
    audio_sink::{AudioSink, volume_gain},
    database::{Database, DatabaseRepository, RecordId},
    executor::{AsyncCallable, AsyncCallableResult},
    guest_call::GuestCallResult,
    platform::{Filesystem, Network, Platform},
    screen::Screen,
    system::{Event, FilesystemOverlay, KeyCode, System, TimerKey},
    task_runner::{DefaultTaskRunner, TaskRunner},
    time::Instant,
};

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use wie_util::{Result, WieError};

/// Host observability: the last framebuffer the guest copied to the screen that
/// was NOT the screen itself, packed as `(handle << 32) | (w << 16) | h`.
///
/// Written by the WIPI-C `MC_grpCopyFrameBuffer` implementation, read by hosts
/// that mirror guest-rendered images (wie_android's HUD uses it to locate the
/// game's per-map minimap buffer, which is otherwise indistinguishable from the
/// stale buffers of previously visited maps). Zero = nothing seen yet.
pub static LAST_OFFSCREEN_COPY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Record an offscreen -> screen blit for [`LAST_OFFSCREEN_COPY`].
pub fn note_offscreen_copy(handle: u32, w: u32, h: u32) {
    let packed = (u64::from(handle) << 32) | (u64::from(w & 0xffff) << 16) | u64::from(h & 0xffff);
    LAST_OFFSCREEN_COPY.store(packed, core::sync::atomic::Ordering::Relaxed);
}

/// `(handle, w, h)` of the last offscreen -> screen blit, or `None`.
pub fn last_offscreen_copy() -> Option<(u32, u32, u32)> {
    let v = LAST_OFFSCREEN_COPY.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        return None;
    }
    Some(((v >> 32) as u32, ((v >> 16) & 0xffff) as u32, (v & 0xffff) as u32))
}

pub trait Emulator {
    fn handle_event(&mut self, event: Event);
    fn tick(&mut self) -> Result<()>;

    /// Milliseconds until the guest has work again — `Some(0)` = right now, `None` =
    /// unknown (the default), which callers must read as "use your own pacing".
    /// See [`System::next_wakeup_ms`]; the host worker loop uses it so it does not
    /// sleep past the guest's next frame timer.
    fn next_wakeup_ms(&self) -> Option<u64> {
        None
    }

    /// Read `len` bytes of guest memory at `address` (host-side introspection,
    /// e.g. reading live character state for overlay HUD). Default = unsupported
    /// (empty); backends that own an ARM core override it. Never faults the guest.
    fn read_memory(&self, _address: u32, _len: usize) -> Vec<u8> {
        Vec::new()
    }

    /// Write `data` into guest memory at `address` (host-side poke, e.g. forcing a
    /// game flag the host UI owns). Returns `true` on success. Default =
    /// unsupported (`false`); backends that own an ARM core override it.
    ///
    /// Only for bytes whose full writer set is known — a host write races the
    /// guest, so the caller owns the contract that nothing else writes them at the
    /// same time.
    fn write_memory(&mut self, _address: u32, _data: &[u8]) -> bool {
        false
    }

    /// Schedule a call to the guest function at `address` with `args` in r0..r3
    /// (spill to stack beyond 4). The call runs on its OWN cooperative task /
    /// ARM thread context on the next `tick()`, so it does not clobber the parked
    /// main-loop task — this is exactly how the runtime invokes WIPI callbacks.
    /// Host-initiated (e.g. an item-use HUD button driving the game's own
    /// `useItemEffect(tab,slot)` routine). `address` must include the Thumb bit
    /// where the target is Thumb. Default = unsupported (no-op); backends that own
    /// an ARM core override it. Only inject self-contained, non-blocking guest
    /// functions at a valid game state — the caller owns that contract.
    fn call_guest_function(&mut self, _address: u32, _args: &[u32]) {}

    /// Like [`Emulator::call_guest_function`], but the guest's return value (`r0`) is
    /// delivered into `out` when the call completes, so the host can CHAIN calls whose
    /// arguments come from an earlier call's result.
    ///
    /// Needed because the call is queued, not inline: the host issues it, returns, and
    /// picks the result up on a later poll. The save path is the motivating case —
    /// `serialiseSave(&len)` hands back a heap buffer that the very next call has to be
    /// given, and there is no way to express that without seeing the value.
    ///
    /// Same contract as `call_guest_function` otherwise: `address` must carry the Thumb
    /// bit where the target is Thumb, the target must be self-contained and
    /// non-blocking, and the caller owns "is the guest in a state where this is legal".
    ///
    /// Default = unsupported, and it FAILS the result rather than leaving it pending, so
    /// a host state machine on a backend without an ARM core does not hang.
    fn call_guest_function_out(&mut self, _address: u32, _args: &[u32], out: alloc::sync::Arc<GuestCallResult>) {
        out.fail();
    }
}

pub struct ProfileSample {
    /// Leaf-first call stack: [pc, lr, lr_prev, ...].
    pub stack: Vec<u32>,
    pub count: u64,
}

/// Called periodically during execution with a batch of samples that the
/// profiler accumulated since the previous flush. The callback also fires once
/// more when the runtime shuts down to drain anything still in the buffer.
pub type ProfileCallback = Box<dyn FnMut(Vec<ProfileSample>) + Send + Sync>;

pub struct Options {
    pub enable_gdbserver: bool,
    pub profile: Option<ProfileCallback>,
}

pub fn extract_zip(zip: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    extern crate std; // XXX

    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let mut archive = ZipArchive::new(Cursor::new(zip)).map_err(|x| WieError::FatalError(format!("Invalid zip archive: {x}")))?;

    (0..archive.len())
        .filter_map(|x| {
            let mut file = match archive.by_index(x) {
                Ok(file) => file,
                Err(err) => return Some(Err(WieError::FatalError(format!("Failed to read zip entry {x}: {err}")))),
            };
            if !file.is_file() {
                return None;
            }

            let mut data = Vec::new();
            if let Err(err) = file.read_to_end(&mut data) {
                return Some(Err(WieError::FatalError(format!("Failed to read zip entry {}: {err}", file.name()))));
            }

            Some(Ok((file.name().to_string(), data)))
        })
        .collect::<Result<_>>()
}
