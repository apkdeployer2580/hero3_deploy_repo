#!/bin/bash
# Drive Hero3 (wie_android) from cold start to 네트워크창고 and dump the NETTRACE.
#
# Overlay button centres for the S25 portrait layout (1080x2340), derived from
# wie_android/src/overlay.rs::layout and confirmed against a screencap.
set -e
cd /d/tmp
A="./platform-tools/adb.exe -s ${ANDROID_SERIAL:?set ANDROID_SERIAL to the phone serial}"
run() { MSYS_NO_PATHCONV=1 $A "$@"; }

OK=(229 1872); RIGHT=(377 1872); LEFT=(76 1872)
tap() { run shell input tap "$1" "$2"; }

run logcat -c
run shell am force-stop net.wie.emulator
sleep 1
run shell monkey -p net.wie.emulator -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1
sleep 6                       # boot + title

tap "${OK[@]}";    sleep 2    # PRESS ANY KEY -> main menu (게임시작)
tap "${RIGHT[@]}"; sleep 1    # -> 네트워크창고
tap "${OK[@]}";    sleep 2    # -> "네트워크 접속시 통화료..." dialog (예 선ected)
tap "${OK[@]}";    sleep 8    # 예 -> network bring-up + socket connect

run exec-out screencap -p > netcap/w1.png
echo "=== NETTRACE ==="
run logcat -d | grep -a "NETTRACE" || echo "(none)"
echo "=== EMULATOR ERRORS ==="
run logcat -d | grep -a "emulator tick error" | head -3 || echo "(none)"
