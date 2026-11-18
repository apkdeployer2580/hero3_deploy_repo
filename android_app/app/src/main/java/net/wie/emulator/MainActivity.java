package net.wie.emulator;

import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.media.AudioManager;
import android.os.Build;
import android.os.Bundle;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.widget.Toast;


import com.google.androidgamesdk.GameActivity;

/**
 * Thin GameActivity subclass. All app logic lives in the Rust cdylib (libmain.so),
 * entered via android_main(). The static loadLibrary + the manifest
 * android.app.lib_name meta-data both point at "main".
 */
public class MainActivity extends GameActivity {
    static {
        System.loadLibrary("main");
    }

    // ndk_context hands native code the *Application*, not the Activity, so we
    // stash the Activity instance here and expose a static entry the Rust side
    // reaches via the app classloader.
    private static MainActivity instance;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        instance = this;
        super.onCreate(savedInstanceState);
        // Volume rocker -> MEDIA volume, always. Without this the keys control
        // whichever stream Android decides is "active", which for a game that is
        // often silent (menus, dialogue, between songs) is the RINGER — so the
        // player presses volume-up, watches a ring-volume slider go to maximum, and
        // reports that the game is quiet at full volume (2026-08-16).
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        letContentUseTheCutout();
        hideSystemBars();
    }

    /**
     * Let the game surface extend UNDER the display cutout (notch / punch-hole),
     * so it fills the whole physical screen instead of being letterboxed beside it
     * (player request 2026-08-19: "스마트폰 전체화면에 게임화면이 꽉차게").
     *
     * Why this is the fix: GameActivity's surface is sized from the WINDOW, and the
     * emulator paints the full surface (`window.inner_size()` -> win_w/win_h in
     * app.rs). With the default cutout policy the window stops at the cutout's safe
     * area, so on any phone with a notch/punch-hole a black bar is left along that
     * edge — the "화면이 꽉 안 참" the player saw on modern devices (the S25's
     * centred punch-hole becomes an edge bar once the app is landscape). Opting the
     * window into the cutout region makes the window — and therefore the surface and
     * `inner_size()` — cover the entire display. The bars are already hidden; the
     * small camera hole overlapping game pixels is the expected, wanted trade for a
     * gapless picture.
     *
     * A window attribute (set once here), not part of `hideSystemBars` which the
     * focus listener re-runs: the cutout mode is not cleared by the system the way
     * the legacy systemUiVisibility flags are. Guarded by API level — the constants
     * and cutouts themselves only exist from API 28 (Android 9), and this app's
     * minSdk is 26. ALWAYS (API 30+) fills into cutouts on every edge/orientation;
     * SHORT_EDGES is the same guarantee the older API can express.
     */
    private void letContentUseTheCutout() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) {
            return; // no display cutouts before API 28
        }
        final Window w = getWindow();
        WindowManager.LayoutParams lp = w.getAttributes();
        lp.layoutInDisplayCutoutMode = (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R)
                ? WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                : WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
        w.setAttributes(lp);
    }

    /**
     * Fully hide the system bars — most importantly the NAVIGATION bar, which the
     * AppTheme's windowFullscreen does not touch (player request 2026-08-07:
     * "게임이 실행되면 핸드폰의 네비게이션바가 숨김처리 되게").
     *
     * Sticky/transient behaviour on purpose: a swipe from the edge reveals the
     * bars over the game for a moment (so the player can still leave the app),
     * then they hide again by themselves. Re-asserted on every focus gain because
     * the legacy systemUiVisibility flags are CLEARED by the system whenever the
     * bars are shown (dialogs such as ours do exactly that).
     */
    private void hideSystemBars() {
        final Window w = getWindow();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            w.setDecorFitsSystemWindows(false);
            WindowInsetsController c = w.getInsetsController();
            if (c != null) {
                c.hide(WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(
                        WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            w.getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                            | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                            | View.SYSTEM_UI_FLAG_FULLSCREEN);
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    // ── Gamepad (58차) ──────────────────────────────────────────────────────────
    //
    // Captured HERE, at the very top of the dispatch chain, and not left to the
    // GameActivity native input path, because that path was read end-to-end and it
    // cannot carry a gamepad: android_native_app_glue's default_motion_filter drops
    // every non-touchscreen MotionEvent (so joystick axes never reach Rust), and
    // winit would misinterpret any that did as touches. Buttons take the same JNI
    // route so ALL pad input has one owner. Sources are matched with the full-mask
    // idiom — a plain `!= 0` would also catch keyboards via the shared BUTTON class
    // bit. Non-gamepad events fall through to super untouched, so touch, volume,
    // back key and TV-remote D-pads (SOURCE_DPAD only) behave exactly as before.

    private static boolean fromSource(int source, int mask) {
        return (source & mask) == mask;
    }

    private static boolean isGamepadSource(int source) {
        return fromSource(source, InputDevice.SOURCE_GAMEPAD)
                || fromSource(source, InputDevice.SOURCE_JOYSTICK);
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent e) {
        if (isGamepadSource(e.getSource())) {
            int action = e.getAction();
            if (action == KeyEvent.ACTION_DOWN || action == KeyEvent.ACTION_UP) {
                nativeGamepadButton(e.getKeyCode(), action == KeyEvent.ACTION_DOWN,
                        e.getRepeatCount() > 0);
                return true;
            }
        }
        return super.dispatchKeyEvent(e);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent e) {
        if (fromSource(e.getSource(), InputDevice.SOURCE_JOYSTICK)
                && e.getAction() == MotionEvent.ACTION_MOVE) {
            // Left stick + hat (the d-pad on most pads reports as HAT axes, not
            // DPAD key events) + triggers (which many Bluetooth pads report ONLY
            // as axes — LTRIGGER/RTRIGGER or BRAKE/GAS depending on the pad, so
            // take whichever is larger). The Rust side sums and quantises;
            // historical samples inside the event are skipped — the latest
            // position is all a 4-way latch can use.
            nativeGamepadAxes(
                    e.getAxisValue(MotionEvent.AXIS_X), e.getAxisValue(MotionEvent.AXIS_Y),
                    e.getAxisValue(MotionEvent.AXIS_HAT_X), e.getAxisValue(MotionEvent.AXIS_HAT_Y),
                    Math.max(e.getAxisValue(MotionEvent.AXIS_LTRIGGER),
                            e.getAxisValue(MotionEvent.AXIS_BRAKE)),
                    Math.max(e.getAxisValue(MotionEvent.AXIS_RTRIGGER),
                            e.getAxisValue(MotionEvent.AXIS_GAS)));
            return true;
        }
        return super.dispatchGenericMotionEvent(e);
    }

    /** Implemented in Rust — lib.rs `gamepad_jni` (resolved by JNI symbol name). */
    private static native void nativeGamepadButton(int keyCode, boolean down, boolean repeat);
    private static native void nativeGamepadAxes(
            float x, float y, float hatX, float hatY, float lt, float rt);

    /**
     * Change screen orientation from native code. setRequestedOrientation must
     * run on the UI thread. `orientation` = ActivityInfo.SCREEN_ORIENTATION_*
     * (0 landscape, 1 portrait).
     */
    public static void requestOrientation(final int orientation) {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                a.setRequestedOrientation(orientation);
            }
        });
    }

    /**
     * Quit the app, from native code (the game's own 종료 menu -> MC_knlExit ->
     * `platform::exit` -> the worker's exit block -> `UserEvent::ExitApp`).
     *
     * Player report 2026-08-20: "종료 버튼을 눌러도 종료가 되지 않아 강제로 홈으로
     * 나간 다음 직접 밀어서 종료해야하고". The host side had never implemented the
     * exit callback at all.
     *
     * finishAndRemoveTask() rather than finish(): the game is over, so leaving a
     * task card that resumes into a dead emulator is worse than no card. The
     * System.exit(0) after it is deliberate — this process holds an emulator
     * worker thread, an audio thread and a JVM the guest ran on, and Android's
     * process reuse would hand a relaunch that same state.
     */
    public static void quitApp() {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                a.finishAndRemoveTask();
                System.exit(0);
            }
        });
    }

    /**
     * Show on-device diagnostics (settings -> DIAG). Exists because a remote
     * tester cannot run adb: "소리가 안 나요" from another phone is undebuggable
     * unless the key init lines are readable — and copyable — on the device
     * itself. The Rust side passes its half (audio pipeline status, window /
     * logical sizes, mode flags); this side prepends what only Java knows
     * (app version, Android version, device model). 복사 puts the whole text on
     * the clipboard so the tester can paste it into a chat message.
     */
    public static void showDiagnostics(final String rustInfo) {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                String ver;
                try {
                    ver = a.getPackageManager().getPackageInfo(a.getPackageName(), 0).versionName;
                } catch (Exception e) {
                    ver = "?";
                }
                final String text = "app " + ver
                        + "\nandroid " + Build.VERSION.RELEASE + " (API " + Build.VERSION.SDK_INT + ")"
                        + "\ndevice " + Build.MANUFACTURER + " " + Build.MODEL
                        + "\n" + rustInfo;
                new AlertDialog.Builder(a)
                        .setTitle("진단 정보")
                        .setMessage(text + "\n\n소리가 안 나는 등 문제가 있으면 [복사]를 눌러\n개발자에게 붙여넣어 보내주세요.")
                        .setPositiveButton("복사", (dialog, which) -> {
                            ClipboardManager cm = (ClipboardManager) a.getSystemService(CLIPBOARD_SERVICE);
                            if (cm != null) {
                                cm.setPrimaryClip(ClipData.newPlainText("wie diagnostics", text));
                                toast(a, "복사되었습니다");
                            }
                        })
                        .setNegativeButton("닫기", null)
                        .setNeutralButton("세이브 복원", (dialog, which) -> showSaveRestore())
                        .show();
            }
        });
    }

    /** Implemented in Rust — lib.rs `vault_jni`. */
    private static native String nativeVaultList();
    private static native boolean nativeVaultRestore(String key);

    /**
     * Offer the save vault's generations and schedule the one the player picks.
     *
     * ── WHY THIS DIALOG EXISTS ─────────────────────────────────────────────
     * 2026-08-20: two players reported a character turning into 빈 공간, and
     * nothing on the device kept a second copy of a save. The vault
     * (wie_android/src/vault.rs) now keeps every version of a character record
     * that is about to be destroyed; this is how a player gets one back.
     *
     * It lives behind DIAG rather than on the settings panel deliberately —
     * restoring is destructive, and a row a player can hit by accident next to
     * HEAT SAVER would cause the very loss it undoes. The list itself comes from
     * Rust at the moment the dialog opens, because every save rotates the vault.
     *
     * The restore is APPLIED ON THE NEXT LAUNCH: the record may be open, with its
     * contents mirrored in a guest-heap buffer that gets flushed back to disk on
     * the next write, so swapping the file underneath a running game would be
     * undone (or half-undone) seconds later. So the app closes itself after
     * scheduling, and the native side applies it before the emulator starts.
     */
    private static void showSaveRestore() {
        final MainActivity a = instance;
        if (a == null) {
            return;
        }
        String raw;
        try {
            raw = nativeVaultList();
        } catch (Throwable t) {
            raw = null;
        }
        if (raw == null || raw.trim().isEmpty()) {
            new AlertDialog.Builder(a)
                    .setTitle("세이브 복원")
                    .setMessage("보관된 세이브 백업이 없습니다.\n\n백업은 게임이 세이브를 덮어쓸 때마다 자동으로 쌓입니다.")
                    .setPositiveButton("닫기", null)
                    .show();
            return;
        }
        final String[] keys = raw.split("\n");
        final String[] labels = new String[keys.length];
        final java.text.SimpleDateFormat fmt =
                new java.text.SimpleDateFormat("MM-dd HH:mm:ss", java.util.Locale.KOREA);
        for (int i = 0; i < keys.length; i++) {
            String[] f = keys[i].split("\\|");
            String slot = f.length > 0 ? f[0] : "?";
            long ms = 0;
            try {
                ms = f.length > 1 ? Long.parseLong(f[1]) : 0;
            } catch (NumberFormatException ignored) {
            }
            String len = f.length > 2 ? f[2] : "?";
            // "슬롯 1 · 08-20 01:32:11 · 1424B" — the slot number is the trailing
            // number of the record name (GameSave_11_<slot>, 0-based on disk) shown
            // 1-BASED, the way the game's own list labels it (SLOT 1 = GameSave_11_0).
            // User, 2026-09-23: "세이브복원에서 현재 슬롯0 시작하는데 이거 슬롯 1부터".
            String n = slot.substring(slot.lastIndexOf('_') + 1);
            try {
                n = Integer.toString(Integer.parseInt(n) + 1);
            } catch (NumberFormatException ignored) {
            }
            labels[i] = "슬롯 " + n + " · " + fmt.format(new java.util.Date(ms)) + " · " + len + "B";
        }
        new AlertDialog.Builder(a)
                .setTitle("복원할 백업 선택")
                .setItems(labels, (dialog, which) -> {
                    boolean ok;
                    try {
                        ok = nativeVaultRestore(keys[which]);
                    } catch (Throwable t) {
                        ok = false;
                    }
                    if (!ok) {
                        toast(a, "복원 예약 실패");
                        return;
                    }
                    new AlertDialog.Builder(a)
                            .setTitle("복원 예약됨")
                            .setMessage(labels[which] + "\n\n앱을 닫고 다시 실행하면 복원됩니다.\n지금 닫을까요?")
                            .setPositiveButton("닫기", (d2, w2) -> quitApp())
                            .setNegativeButton("나중에", null)
                            .show();
                })
                .setNegativeButton("취소", null)
                .show();
    }

    // ⛔ `promptServerAddress` / `promptAccountCode` (+ SERVER_FILE / ACCOUNT_FILE and
    // their writers) WERE HERE and are gone (99차, 2026-09-16): the app is 100 % offline
    // and has no server address and no account code to edit.

    private static void toast(MainActivity a, String msg) {
        Toast.makeText(a, msg, Toast.LENGTH_LONG).show();
    }
}
