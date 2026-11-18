//! Platform (Java) dialogs and the quit hook, reached over JNI.
//!
//! The in-game overlay draws with the game's own bitmap fonts and has no text-input or
//! scrolling-dialog machinery, so the two things that need a real Android widget go to
//! `MainActivity` — the same classloader dance [`crate::orientation`] uses:
//!
//! * [`show_diagnostics`] — settings → DIAG: the Rust status text, prepended by the Java
//!   side with app / Android / device versions, with a 복사 button and the 세이브 복원
//!   neutral button (`MainActivity.showSaveRestore` → `vault_jni`).
//! * [`quit_app`] — the guest's own 종료: `finishAndRemoveTask()` + `System.exit(0)`.
//!
//! ★ 99차 (2026-09-16): this file was `server_prompt.rs`, and its other two dialogs —
//! the private-server address prompt and the account-code prompt — went with the
//! online features (100 % offline). Nothing here writes a file any more.
//!
//! All failures are logged, never fatal.

/// Show the on-device diagnostics dialog (settings -> DIAG). `info` is the Rust
/// half of the text (audio status, sizes, mode flags); the Java side prepends
/// app version / Android version / device model and offers a 복사 button — the
/// whole point is that a REMOTE tester can read and paste it without adb.
#[cfg(target_os = "android")]
pub fn show_diagnostics(info: &str) {
    match call_with_string("showDiagnostics", info) {
        Ok(()) => log::info!("diagnostics dialog opened"),
        Err(e) => log::error!("showDiagnostics failed: {e:?}"),
    }
}

#[cfg(not(target_os = "android"))]
pub fn show_diagnostics(_info: &str) {}

/// Close the app (`MainActivity.quitApp`), for the guest's own 종료 menu.
///
/// Not a dialog, but the same classloader dance and the same "log, never fatal"
/// contract. If it fails the app simply stays open, which is the behaviour this
/// fixes, not a regression on top of it.
#[cfg(target_os = "android")]
pub fn quit_app() {
    match call_void("quitApp") {
        Ok(()) => log::warn!("exit: quitApp() dispatched"),
        Err(e) => log::error!("quitApp failed: {e:?}"),
    }
}

#[cfg(not(target_os = "android"))]
pub fn quit_app() {}

/// Call `MainActivity.<method>()`.
#[cfg(target_os = "android")]
fn call_void(method: &str) -> Result<(), jni::errors::Error> {
    use jni::objects::{JClass, JObject, JValue};

    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };

    let result: Result<(), jni::errors::Error> = (|| {
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let cls_name = env.new_string("net.wie.emulator.MainActivity")?;
        let cls_name_ref: &JObject = &cls_name;
        let cls_obj = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(cls_name_ref)],
            )?
            .l()?;
        let cls = JClass::from(cls_obj);
        env.call_static_method(&cls, method, "()V", &[])?;
        Ok(())
    })();

    if result.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result
}

/// Call `MainActivity.<method>(String)`.
#[cfg(target_os = "android")]
fn call_with_string(method: &str, arg: &str) -> Result<(), jni::errors::Error> {
    use jni::objects::{JClass, JObject, JValue};

    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // ndk_context hands native code the *Application*, not the Activity, so the
    // app class has to come from the context's classloader — a FindClass on this
    // thread cannot see app classes.
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };

    let result: Result<(), jni::errors::Error> = (|| {
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let cls_name = env.new_string("net.wie.emulator.MainActivity")?;
        let cls_name_ref: &JObject = &cls_name;
        let cls_obj = env
            .call_method(
                &loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(cls_name_ref)],
            )?
            .l()?;
        let cls = JClass::from(cls_obj);
        let jarg = env.new_string(arg)?;
        let arg_ref: &JObject = &jarg;
        env.call_static_method(&cls, method, "(Ljava/lang/String;)V", &[JValue::Object(arg_ref)])?;
        Ok(())
    })();

    if result.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result
}
