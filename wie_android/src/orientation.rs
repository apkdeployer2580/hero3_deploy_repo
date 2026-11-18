//! Runtime portrait/landscape toggle via `Activity.setRequestedOrientation`.
//! android-activity populates `ndk_context` with the JavaVM + the GameActivity
//! (an Activity) before `android_main`, so we can reach the method over JNI.
//!
//! NOTE: some Activity methods want the UI thread; `setRequestedOrientation`
//! is normally safe off-thread, but this path is verified on-device separately.
//! All failures are logged, never fatal.

/// `ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE`.
///
/// ★ 2026-08-08, player: "핸드폰을 거꾸로 돌려도 화면이 180도 안돌아간다". It was
/// `SCREEN_ORIENTATION_LANDSCAPE` (0), which pins the activity to ONE of the two landscape
/// directions — turning the phone end-for-end did nothing, so the charging cable and the
/// camera bump were stuck on whichever side Android happened to pick.
///
/// `SENSOR_LANDSCAPE` (6) keeps the app landscape but lets the sensor choose WHICH landscape,
/// so a 180° flip follows. Deliberately the sensor variant and not `USER_LANDSCAPE` (11): the
/// latter obeys the system auto-rotate lock, and a player with auto-rotate off — which is most
/// of them, because this is a game you hold — would see no change at all and report the same
/// bug again. The cost is that it also flips when the phone is inverted for other reasons, but
/// within landscape that only ever means the 180° the player asked for.
#[cfg(target_os = "android")]
const LANDSCAPE: i32 = 6;
/// `ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT`, the same choice for the portrait layout.
#[cfg(target_os = "android")]
const PORTRAIT: i32 = 7;

#[cfg(target_os = "android")]
pub fn set(landscape: bool) {
    let value = if landscape { LANDSCAPE } else { PORTRAIT };
    match try_set(value) {
        Ok(()) => log::info!("setRequestedOrientation(sensor {}) ok", if landscape { "landscape" } else { "portrait" }),
        Err(e) => log::error!("setRequestedOrientation failed: {e:?}"),
    }
}

#[cfg(not(target_os = "android"))]
pub fn set(_landscape: bool) {}

#[cfg(target_os = "android")]
fn try_set(value: i32) -> Result<(), jni::errors::Error> {
    use jni::objects::{JClass, JObject, JValue};

    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // ndk_context gives native code the *Application* (verified on-device), not
    // the Activity — so reach MainActivity's static entry through the app
    // classloader (a native-thread FindClass can't see app classes).
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
        env.call_static_method(&cls, "requestOrientation", "(I)V", &[JValue::Int(value)])?;
        Ok(())
    })();

    if result.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result
}
