#!/bin/bash
# Drive Hero3 (wie_android) from cold start to the SLOT screen and press '#' (복구).
# The slot screen's bottom menu shows "#: 복구" = server-side slot backup/restore,
# the other half of Track C alongside 네트워크창고.
set -e
cd /d/tmp
A="./platform-tools/adb.exe -s ${ANDROID_SERIAL:?set ANDROID_SERIAL to the phone serial}"
run() { MSYS_NO_PATHCONV=1 $A "$@"; }

OK="229 1872"; HASH="962 2200"
tap() { run shell input tap $1; }

run logcat -c
run shell am force-stop net.wie.emulator
sleep 1
run shell monkey -p net.wie.emulator -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
sleep 6                       # boot + title

tap "$OK";   sleep 2          # PRESS ANY KEY -> main menu (게임시작)
tap "$OK";   sleep 3          # 게임시작 -> slot screen
run exec-out screencap -p > netcap/r1.png
tap "$HASH"; sleep 3          # '#' -> 복구
run exec-out screencap -p > netcap/r2.png
tap "$OK";   sleep 6          # confirm whatever dialog appeared
run exec-out screencap -p > netcap/r3.png

echo "=== NETTRACE ==="
run logcat -d | grep -a "NETTRACE" || echo "(none)"
echo "=== EMULATOR ERRORS ==="
run logcat -d | grep -a "emulator tick error" | head -3 || echo "(none)"
