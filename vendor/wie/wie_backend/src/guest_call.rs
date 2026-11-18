use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Where a host-initiated guest call drops its return value.
///
/// [`crate::Emulator::call_guest_function`] cannot hand a value back directly: the
/// call does not run inline, it is queued as a cooperative task and executes on a
/// later `System::tick` (the same mechanism WIPI callbacks use), so by the time the
/// guest's `r0` exists the caller has long returned. A host that needs to CHAIN calls
/// — "serialise the save, then write the buffer that returned" — therefore needs a
/// place for the result to land, plus a way to know it has landed.
///
/// Shared by `Arc` between the host thread and the spawned task. Deliberately tiny and
/// lock-free: `wie_backend` is `no_std` and the task may run on any executor turn.
///
/// The three states are distinguished (rather than "value or nothing") because a host
/// state machine must be able to tell "not yet" from "it failed" — treating a failure
/// as "not yet" would hang the machine forever, and treating "not yet" as a failure
/// would abandon a call that was about to succeed.
#[derive(Debug)]
pub struct GuestCallResult {
    value: AtomicU32,
    state: AtomicU8,
}

const PENDING: u8 = 0;
const OK: u8 = 1;
const FAILED: u8 = 2;

impl Default for GuestCallResult {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestCallResult {
    pub const fn new() -> Self {
        Self {
            value: AtomicU32::new(0),
            state: AtomicU8::new(PENDING),
        }
    }

    /// `None` = still running. `Some(Ok(r0))` = returned. `Some(Err(()))` = the call
    /// could not be made or the guest faulted.
    pub fn poll(&self) -> Option<Result<u32, ()>> {
        // Acquire pairs with the Release in `finish`, so a visible state implies the
        // value written before it is visible too.
        match self.state.load(Ordering::Acquire) {
            PENDING => None,
            OK => Some(Ok(self.value.load(Ordering::Relaxed))),
            _ => Some(Err(())),
        }
    }

    /// Record success. Called by the backend from the spawned task.
    pub fn ok(&self, value: u32) {
        self.value.store(value, Ordering::Relaxed);
        self.state.store(OK, Ordering::Release);
    }

    /// Record failure. Also what the trait's DEFAULT implementation does, so a host
    /// waiting on a backend that does not support guest calls fails fast instead of
    /// waiting forever.
    pub fn fail(&self) {
        self.state.store(FAILED, Ordering::Release);
    }
}
