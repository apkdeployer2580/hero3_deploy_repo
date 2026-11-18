# Build the wie_android APK (Rust cdylib via cargo-ndk -> Gradle GameActivity APK).
# Captures every host-toolchain quirk discovered while bringing this up on Windows.
#
#   .\build_android.ps1                 # debug (opt-level 3, fast, ~20 MB APK)
#   .\build_android.ps1 -Release        # release (LTO) + versionCode bump + signing
#   .\build_android.ps1 -Install        # also adb-install + launch on the device
#   .\build_android.ps1 -NewKeystore    # create the release keystore, once, then exit
#   .\build_android.ps1 -Release -NoBump # rebuild the SAME versionCode (re-sign / retry)
#
# ── RELEASE SIGNING (read this before handing the APK to anyone) ──────────────
# An unsigned APK cannot be installed. Debug builds are auto-signed with a
# per-machine throwaway key, which is fine for your own phone and useless for
# distribution. `-Release` signs with YOUR key, which the build reads from outside
# this repo:
#     HERO3_KEYSTORE, HERO3_KEYSTORE_PASSWORD, HERO3_KEY_ALIAS, HERO3_KEY_PASSWORD
# (or hero3.keystore / hero3.keystorePassword / hero3.keyAlias / hero3.keyPassword
# in %USERPROFILE%\.gradle\gradle.properties, which is the persistent option).
#
# ★ THE KEY IS THE APP'S IDENTITY. Android will only update an installed app with
#   an APK signed by the SAME key. Lose it and every existing install is a dead end:
#   users must uninstall to take a new build, and uninstalling deletes their saves
#   (app-private storage). Back the .jks up somewhere that survives this machine.
param(
    [switch]$Release,
    [switch]$Install,
    [switch]$NewKeystore,
    [switch]$NoBump
)
# ⛔ `-LocalServer` and the whole private-server destination machinery (§0b / §2b:
# HERO3_SERVER_HOST / _PIN / _PORT, gradle.properties hero3.server*) WERE HERE and are
# gone (99차, 2026-09-16): the app is 100 % offline and bakes in no address at all.
$ErrorActionPreference = 'Stop'

# --- tool locations ---------------------------------------------------------
# ★ Each is a CANDIDATE LIST, not an absolute path (2026-08-05). These were three
# absolute paths into one user profile, so a second machine could not run this script
# at all — the SDK, the JDK and adb were all "not found" before the first line of work.
# The original values stay FIRST so nothing changes on the machine they were written
# for; the fallbacks are the standard per-user locations.
$Sdk = @(
    $env:ANDROID_HOME
    "$env:LOCALAPPDATA\Android\Sdk"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $Sdk) { throw 'Android SDK not found. Set $env:ANDROID_HOME.' }
$Ndk = "$Sdk\ndk\25.1.8937393"
$NdkBin = "$Ndk\toolchains\llvm\prebuilt\windows-x86_64\bin"
if (-not (Test-Path $Ndk)) { throw "NDK 25.1.8937393 not found under $Sdk\ndk (gradle's ndkVersion pins this exact revision)." }
$env:ANDROID_HOME = $Sdk
$env:ANDROID_NDK_HOME = $Ndk
# Android Studio's bundled JBR is a real JDK 17+ and is present wherever Studio is, so
# it is the fallback rather than a second download.
$env:JAVA_HOME = @(
    $env:JAVA_HOME
    'C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot'
    'C:\Program Files\Android\Android Studio\jbr'
) | Where-Object { $_ -and (Test-Path (Join-Path $_ 'bin\java.exe')) } | Select-Object -First 1
if (-not $env:JAVA_HOME) { throw 'No JDK found. Set $env:JAVA_HOME.' }
$Adb = @(
    $env:HERO3_ADB
    'D:\tmp\platform-tools\adb.exe'
    "$Sdk\platform-tools\adb.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($Install -and -not $Adb) { throw 'adb not found. Set $env:HERO3_ADB.' }

# --- dlltool shim -----------------------------------------------------------
# Building for android on this windows-gnu host pulls host build-deps
# (cc -> windows-sys, getrandom) whose raw-dylib import libs need `dlltool.exe`.
# The bundled GNU dlltool needs a separate `as` the toolchain doesn't ship, so
# shim the default name to llvm-dlltool (self-contained). target.<host>.rustflags
# does NOT reach build-deps, hence a PATH shim.
#
# ★ 2026-08-05: copying llvm-dlltool to `dlltool.exe` is no longer enough, on ANY
#   toolchain. rustc chooses the GNU flag set from that file name and one of those
#   flags (`--temp-prefix`) makes llvm-dlltool print its usage and exit 1 — no import
#   lib, and the build dies with a misleading "failed to add native library
#   ...dll_imports.lib: file not found". Newer rustup also stopped shipping
#   llvm-dlltool here at all (`self-contained` now has the GNU one, still without the
#   `as` it needs). So the shim is a real WRAPPER, built from `tools/dlltool_shim.rs`,
#   which drops the GNU-only flags and forwards. See that file for the measurement.
$Shim = Join-Path $env:TEMP 'wie-dlltool'
New-Item -ItemType Directory -Force $Shim | Out-Null
$RealDlltool = Join-Path $Shim 'real-llvm-dlltool.exe'
$llvmDlltool = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-dlltool.exe"
if (Test-Path $llvmDlltool) {
    Copy-Item $llvmDlltool $RealDlltool -Force
} elseif (Test-Path "$NdkBin\llvm-ar.exe") {
    # llvm-ar/llvm-lib/llvm-ranlib/llvm-dlltool are ONE multicall binary: it picks its
    # personality from argv[0], so a copy named llvm-dlltool IS llvm-dlltool. It loads
    # its DLLs from the NDK bin dir, which is why that goes on PATH below.
    Copy-Item "$NdkBin\llvm-ar.exe" $RealDlltool -Force
} else {
    throw "No llvm-dlltool available (looked in the rustup gnu toolchain and $NdkBin)."
}
& "$env:USERPROFILE\.cargo\bin\rustc.exe" -O --edition 2021 "$PSScriptRoot\tools\dlltool_shim.rs" -o (Join-Path $Shim 'dlltool.exe')
if ($LASTEXITCODE -ne 0) { throw "could not build the dlltool shim ($LASTEXITCODE)" }
$env:WIE_REAL_DLLTOOL = $RealDlltool
# $NdkBin goes LAST: it is there so the multicall llvm binary above can find its own
# DLLs, and putting it first would let the NDK's cross tools shadow host ones.
$env:PATH = "$Shim;$env:USERPROFILE\.cargo\bin;$env:JAVA_HOME\bin;$env:PATH;$NdkBin"

$JniLibs = "$PSScriptRoot\android_app\app\src\main\jniLibs"
$VersionFile = "$PSScriptRoot\android_app\version.properties"
$GameAsset = "$PSScriptRoot\android_app\app\src\main\assets\hero3_fixed.zip"

# --- The bundled game archive, checked BEFORE anything expensive -------------
# Without the archive Gradle still packages happily, and the APK then
# builds, SIGNS, INSTALLS - and dies on the emulator's first line:
#
#     E/wie: main: bundled game asset hero3_fixed.zip not found; aborting
#
# ★ 2026-08-08: exactly that shipped to the player on a second machine. The build said
# "BUILD SUCCESSFUL" and apksigner verified it, so nothing upstream of the phone noticed. An
# APK that cannot start is worse than no APK, so it is a hard failure rather than a warning.
if (-not (Test-Path $GameAsset)) {
    Write-Warning "The game archive is missing from this checkout."
    Write-Warning "Copy hero3_fixed.zip to: $GameAsset"
    throw "missing bundled game archive: $GameAsset"
}
Write-Output ("==> game asset: hero3_fixed.zip ({0:N0} bytes)" -f (Get-Item $GameAsset).Length)

# --- -NewKeystore: create the release key, once -----------------------------
# Deliberately a separate switch and not something a build does implicitly: this
# generates a secret whose loss is unrecoverable (see the header). Default location
# is OUTSIDE the repo, under the user profile, so no clone or zip of this project
# can ever carry it.
if ($NewKeystore) {
    $ksDir = "$env:USERPROFILE\.android\keystores"
    $ks = if ($env:HERO3_KEYSTORE) { $env:HERO3_KEYSTORE } else { "$ksDir\hero3-release.jks" }
    if (Test-Path $ks) { throw "keystore already exists: $ks (refusing to overwrite - that would orphan every install)" }
    if (-not $env:HERO3_KEYSTORE_PASSWORD -or -not $env:HERO3_KEY_ALIAS -or -not $env:HERO3_KEY_PASSWORD) {
        throw @"
Set the three secrets first, then re-run:
  `$env:HERO3_KEYSTORE_PASSWORD = '<store password>'
  `$env:HERO3_KEY_ALIAS         = 'hero3'
  `$env:HERO3_KEY_PASSWORD      = '<key password>'   # may equal the store password
  .\build_android.ps1 -NewKeystore
"@
    }
    New-Item -ItemType Directory -Force $ksDir | Out-Null
    Write-Output "==> creating release keystore: $ks"
    # PKCS12, not JKS: JKS is Java's proprietary legacy format and keytool nags to
    # migrate off it on every use. Android/apksigner read PKCS12 natively.
    # 10000 days ~= 27 years. Play rejects keys that expire before 2033-10-22, and a
    # key that expires is a key you cannot ship updates with afterwards.
    & "$env:JAVA_HOME\bin\keytool.exe" -genkeypair -v `
        -keystore $ks -storetype PKCS12 -storepass $env:HERO3_KEYSTORE_PASSWORD `
        -alias $env:HERO3_KEY_ALIAS -keypass $env:HERO3_KEY_PASSWORD `
        -keyalg RSA -keysize 4096 -validity 10000 `
        -dname "CN=Hero3 WIE, OU=side project, O=self, C=KR"
    if ($LASTEXITCODE -ne 0) { throw "keytool failed ($LASTEXITCODE)" }
    Write-Output ''
    Write-Output "==> DONE. Now make it permanent (this file is outside the repo):"
    Write-Output "    %USERPROFILE%\.gradle\gradle.properties"
    Write-Output "      hero3.keystore=$($ks -replace '\\','/')"
    Write-Output "      hero3.keystorePassword=<store password>"
    Write-Output "      hero3.keyAlias=$env:HERO3_KEY_ALIAS"
    Write-Output "      hero3.keyPassword=<key password>"
    Write-Output ''
    Write-Output "==> AND BACK UP $ks somewhere off this machine."
    Write-Output "    Without it you can never update an installed copy - users would have to"
    Write-Output "    uninstall, which deletes their saves."
    return
}

# --- 0a) versionCode bump ---------------------------------------------------
# Only on -Release, so debug builds and IDE syncs leave it alone. Android refuses an
# APK whose versionCode is not higher than the installed one, and Play refuses one
# that was already uploaded - so a hand-edited constant is a trap you hit at the
# worst moment (handing someone a build that "won't install").
if ($Release -and -not $NoBump) {
    if (-not (Test-Path $VersionFile)) { throw "missing $VersionFile" }
    $lines = [System.IO.File]::ReadAllLines($VersionFile)
    $codeLine = $lines | Where-Object { $_ -match '^\s*versionCode\s*=' }
    if (-not $codeLine) { throw "no versionCode= line in $VersionFile" }
    $cur = [int]($codeLine -replace '.*=\s*', '')
    $next = $cur + 1
    $vname = (($lines | Where-Object { $_ -match '^\s*versionName\s*=' }) -replace '.*=\s*', '')
    # ★ NOT `Set-Content -Encoding utf8`: on Windows PowerShell 5.1 that writes a BOM
    # and re-encodes the file through the console codepage, which mangles anything
    # non-ASCII in the comments (caught doing exactly that). Write the bytes back the
    # way they came in.
    [System.IO.File]::WriteAllLines(
        $VersionFile,
        ($lines -replace '^\s*versionCode\s*=.*', "versionCode=$next"),
        (New-Object System.Text.UTF8Encoding($false)))
    Write-Output "==> versionCode $cur -> $next  (versionName $vname)"
}

# --- 0) no server destination (99차) ----------------------------------------
# The app is 100 % offline: the warehouse and the cash shop are served in-app and no
# socket is ever opened, so there is nothing to bake in and nothing to verify in the
# binary. (Until 2026-09-16 this section resolved HERO3_SERVER_HOST/_PIN/_PORT and
# refused a release without them.)
Write-Output '==> 100% offline build: no server destination (network features are in-app)'

# --- 1) Rust cdylib -> jniLibs ---------------------------------------------
# ★ 77차-B: vanilla APK keeps the same item tables as main; only the reaver
# drop hook is off. flavor.properties `flavor=vanilla` → rustc `--cfg hero3_vanilla`
# (hook.rs skips ReaverDropLgt). Do not set this on main.
$Flavor = ''
$FlavorFile = Join-Path $PSScriptRoot 'android_app\flavor.properties'
if (Test-Path $FlavorFile) {
    foreach ($line in (Get-Content $FlavorFile)) {
        if ($line -match '^\s*flavor\s*=\s*(\S*)\s*$') { $Flavor = $Matches[1]; break }
    }
}
if ($Flavor -eq 'vanilla') {
    $env:RUSTFLAGS = (("$env:RUSTFLAGS --cfg hero3_vanilla").Trim())
    Write-Output "==> flavor vanilla: rustc --cfg hero3_vanilla (drop hook off, tables stay)"
}
Write-Output "==> cargo-ndk build ($(if ($Release) {'release'} else {'debug'}))"
Push-Location "$PSScriptRoot\wie_android"
try {
    $ndkArgs = @('-t', 'arm64-v8a', '--platform', '26', '-o', $JniLibs, 'build')
    if ($Release) { $ndkArgs += '--release' }
    & cargo ndk @ndkArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed ($LASTEXITCODE)" }
} finally { Pop-Location }

# --- 2) strip (debug keeps debuginfo -> ~266 MB otherwise) ------------------
Write-Output '==> strip libmain.so'
& "$NdkBin\llvm-strip.exe" --strip-all "$JniLibs\arm64-v8a\libmain.so"

# --- 3) Gradle APK ----------------------------------------------------------
$task = if ($Release) { ':app:assembleRelease' } else { ':app:assembleDebug' }
Write-Output "==> gradle $task"
Push-Location "$PSScriptRoot\android_app"
try {
    & .\gradlew.bat $task --no-daemon
    if ($LASTEXITCODE -ne 0) { throw "gradle failed ($LASTEXITCODE)" }
} finally { Pop-Location }

# ★ The release output is named `app-release.apk` when it is SIGNED and
# `app-release-unsigned.apk` when it is not, so the filename itself is the answer to
# "can anyone install this?". Take whichever exists and say which one it was.
$apk = if ($Release) {
    $signed = "$PSScriptRoot\android_app\app\build\outputs\apk\release\app-release.apk"
    $unsigned = "$PSScriptRoot\android_app\app\build\outputs\apk\release\app-release-unsigned.apk"
    if (Test-Path $signed) { $signed } else { $unsigned }
} else {
    "$PSScriptRoot\android_app\app\build\outputs\apk\debug\app-debug.apk"
}
Write-Output "==> APK: $apk"
Get-Item $apk | Select-Object Length, LastWriteTime

# --- 3b) prove it is signed, and with WHICH key -----------------------------
# Not decoration: the failure mode is silent (an APK that simply refuses to install
# on someone else's phone, or that Android rejects as an update because the key
# changed). The certificate digest below is the app's identity - it must be the SAME
# string on every build you ever hand out.
$bt = Get-ChildItem "$Sdk\build-tools" -Directory -ErrorAction SilentlyContinue |
    Sort-Object Name -Descending | Select-Object -First 1
if ($Release) {
    $apksigner = if ($bt) { Join-Path $bt.FullName 'apksigner.bat' } else { $null }
    if ($apksigner -and (Test-Path $apksigner)) {
        Write-Output '==> apksigner verify'
        & $apksigner verify --print-certs $apk 2>&1 |
            Select-String -Pattern 'SHA-256 digest|DOES NOT VERIFY|Signer #1 certificate DN' |
            ForEach-Object { "    $_" }
        if ($LASTEXITCODE -ne 0) {
            Write-Warning '    APK IS NOT SIGNED - it cannot be installed. See the header of this script.'
        }
    } else {
        Write-Warning "==> apksigner not found under $Sdk\build-tools; signature unverified"
    }
    if ($apk -like '*unsigned*') {
        Write-Warning '==> This APK is UNSIGNED. Do not hand it out; it will not install.'
    }
}

# --- 4) optional install + launch ------------------------------------------
if ($Install) {
    Write-Output '==> adb install + launch'
    # Pick the target device. A second, UNAUTHORIZED phone is often also attached, so
    # this cannot just let adb choose ("more than one device"). It used to be the one
    # hardcoded serial below, which meant `-Install` did nothing but print
    # "device not found" on any other machine or after swapping phones — the same
    # single-machine assumption the tool paths above were fixed for. Order:
    #   1. $env:ANDROID_SERIAL, if the caller set one (adb reads this itself)
    #   2. the only device in `device` state — the ordinary one-phone case
    #   3. otherwise adb's own default
    if (-not $env:ANDROID_SERIAL) {
        $ready = @(& $Adb devices | Select-String -Pattern '^(\S+)\s+device$' | ForEach-Object { $_.Matches.Groups[1].Value })
        if ($ready.Count -eq 1) {
            $env:ANDROID_SERIAL = $ready[0]
        } elseif ($ready.Count -gt 1) {
            $env:ANDROID_SERIAL = $ready[0]
            Write-Warning "    $($ready.Count) devices ready ($($ready -join ', ')); using $($ready[0]). Set `$env:ANDROID_SERIAL to choose."
        } else {
            Write-Warning '    no device in `device` state (unauthorized? accept the USB debugging prompt).'
        }
    }
    Write-Output "    device $env:ANDROID_SERIAL"
    & $Adb install -r $apk
    & $Adb shell input keyevent KEYCODE_HOME
    # ★ Component DERIVED from the APK, not hardcoded. `applicationId` and the Java
    # package are different now (com.hero3remake.app vs net.wie.emulator), so
    # `am start -n <applicationId>/.MainActivity` — which is what this used to do —
    # names a class that does not exist. aapt2 reports what actually shipped.
    $badging = if ($bt) { & (Join-Path $bt.FullName 'aapt2.exe') dump badging $apk 2>$null } else { $null }
    $pkg = ($badging | Select-String -Pattern "^package: name='([^']+)'").Matches.Groups[1].Value
    $act = ($badging | Select-String -Pattern "^launchable-activity: name='([^']+)'").Matches.Groups[1].Value
    if ($pkg -and $act) {
        Write-Output "    launching $pkg/$act"
        & $Adb shell am start -n "$pkg/$act"
    } else {
        Write-Warning '    could not read the launch component from the APK; start it by hand'
    }
}
