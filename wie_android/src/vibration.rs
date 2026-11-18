//! Haptic feedback for `MC_mdaVibrator` (the game's vibration setting) ->
//! `Platform::vibrate`.
//!
//! ── WHY THIS IS A THREAD AND NOT A FUNCTION CALL ─────────────────────────────
//! Player-reported 2026-08-03: "게임 플레이 시 크리티컬이 뜨면 살짝 느려진다."
//! It does, and this file was the reason. Traced end to end in the guest:
//!
//!   attack routine (guest 0x1439cc) -> the crit branch at 0x143c32 takes
//!   0x143cdc, which plays `/snd/sd004_mf` AND calls 0x147a74 (the settings-gated
//!   vibrate wrapper: byte 3 of the option struct `*(sl+0x16c)` is the vibration
//!   toggle) -> 0x1983b8, a bare thunk on the WIPI-C media interface
//!   `*(sl+0x44c)` slot +0x40 = method 16 = **MC_mdaVibrator(100, 200)**.
//!
//! wie routes that to `Platform::vibrate` **synchronously, inside `emulator.tick()`,
//! on the emulator worker thread**. What used to happen there, per crit:
//!   * `JavaVM::attach_current_thread` — attach AND, on drop, detach again. ART's
//!     `Thread::Destroy` is not free and can wait on the runtime.
//!   * `env.new_string("vibrator")` — a Java allocation, i.e. GC-visible work.
//!   * `Context.getSystemService(String)` — a Java call with a string lookup.
//!   * `Vibrator.vibrate(long)` — a **synchronous binder transaction** to
//!     system_server. Sub-millisecond when the system is idle; several ms when it
//!     is not, and it is exactly during a fight that it is not.
//! All of that landed inside a 16 ms frame that already budgets up to 24 ms for
//! interpretation, so the frame overran and the game visibly hitched.
//!
//! Now: the emulator thread does one `try_send` on a bounded channel (a few
//! nanoseconds, and it *cannot block* — a full queue drops the request instead of
//! stalling the game). A dedicated thread owns the JNI side, stays attached for the
//! process lifetime, holds the `Vibrator` as a global ref, and makes the binder call
//! where nobody is waiting on it. Bursts are coalesced: three crits in one frame
//! buzz once, for the longest of them, rather than queueing three buzzes that
//! outlast the fight.
//!
//! Requires the VIBRATE permission (AndroidManifest). Failures are logged, never
//! fatal; if the JNI side cannot be set up the thread exits and later requests are
//! dropped silently.

#[cfg(target_os = "android")]
use std::sync::{
    OnceLock,
    mpsc::{Receiver, SyncSender, sync_channel},
};

/// Queue depth. Small on purpose: haptics are a "now" signal, so a backlog is worse
/// than a drop, and a shallow queue bounds how far behind the vibrator thread can be.
#[cfg(target_os = "android")]
const QUEUE: usize = 4;

/// Ceiling on one buzz, so a bogus guest value cannot vibrate forever.
#[cfg(target_os = "android")]
const MAX_MS: u64 = 5_000;

#[cfg(target_os = "android")]
static TX: OnceLock<Option<SyncSender<u64>>> = OnceLock::new();

/// Request a buzz. **Never blocks and never calls into the JVM** — see the module
/// docs; this runs on the emulator thread, inside the guest's `MC_mdaVibrator`.
#[cfg(target_os = "android")]
pub fn vibrate(duration_ms: u64) {
    if duration_ms == 0 {
        return;
    }
    let Some(tx) = TX.get_or_init(spawn) else { return };
    // `try_send`, not `send`: if the vibrator thread is still inside a binder call
    // the request is dropped. Dropping a buzz is invisible; blocking the emulator
    // is the bug this file exists to fix.
    let _ = tx.try_send(duration_ms.min(MAX_MS));
}

#[cfg(not(target_os = "android"))]
pub fn vibrate(_duration_ms: u64) {}

#[cfg(target_os = "android")]
fn spawn() -> Option<SyncSender<u64>> {
    let (tx, rx) = sync_channel::<u64>(QUEUE);
    match std::thread::Builder::new().name("wie-vibe".into()).spawn(move || worker(rx)) {
        Ok(_) => Some(tx),
        Err(e) => {
            log::warn!("vibration: could not spawn the vibrator thread: {e}");
            None
        }
    }
}

#[cfg(target_os = "android")]
fn worker(rx: Receiver<u64>) {
    use jni::objects::{JObject, JValue};

    // Attach ONCE, as a daemon, and never detach: this thread lives as long as the
    // process, and attach/detach per buzz is a large part of what made this slow.
    let ctx = ndk_context::android_context();
    let vm = match unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
        Ok(vm) => vm,
        Err(e) => {
            log::warn!("vibration: JavaVM::from_raw failed: {e:?}");
            return;
        }
    };
    let mut env = match vm.attach_current_thread_as_daemon() {
        Ok(env) => env,
        Err(e) => {
            log::warn!("vibration: attach failed: {e:?}");
            return;
        }
    };

    // Resolve the Vibrator once and keep it as a global ref. ndk_context hands
    // native code the Application context, which is a `Context` and so has
    // `getSystemService`; framework classes resolve fine from a native thread
    // (app classes would not — see orientation.rs).
    let vibrator = {
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let resolved = (|| -> Result<Option<jni::objects::GlobalRef>, jni::errors::Error> {
            let name = env.new_string("vibrator")?;
            let name_ref: &JObject = &name;
            let v = env
                .call_method(&context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(name_ref)])?
                .l()?;
            if v.is_null() {
                return Ok(None);
            }
            Ok(Some(env.new_global_ref(&v)?))
        })();
        match resolved {
            Ok(Some(v)) => v,
            Ok(None) => {
                log::info!("vibration: no vibrator service on this device; haptics disabled");
                return;
            }
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
                log::warn!("vibration: could not resolve the Vibrator service: {e:?}");
                return;
            }
        }
    };
    log::info!("vibration: worker ready (attached once, Vibrator cached)");

    while let Ok(first) = rx.recv() {
        // Coalesce whatever piled up while the previous binder call was in flight:
        // one buzz, the longest of them. Rapid crits should feel like one hit, and
        // three queued 200 ms buzzes would still be running after the fight.
        let mut ms = first;
        while let Ok(more) = rx.try_recv() {
            ms = ms.max(more);
        }
        // `vibrate(long)` is deprecated since API 26 but still works and internally
        // builds a default VibrationEffect — the simplest thing that is correct on
        // every API level this app supports.
        if env.call_method(&vibrator, "vibrate", "(J)V", &[JValue::Long(ms as i64)]).is_err() {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            log::warn!("vibration: vibrate({ms}ms) failed");
        }
    }
}
