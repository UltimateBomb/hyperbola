# Running Hyperbola on Android

The Android build is not a port: it is the same engine, the same queue and the
same window, with the download engine reached through a plugin instead of a
spawned binary. What follows is how to get it onto a phone and what to check
first.

## Getting the APK

Every run of the `release` workflow builds one. Take it from the run's
artifacts (`hyperbola-android-apk`), or build locally with the Android SDK,
NDK and a JDK installed:

```bash
npm ci && npm --prefix ui ci
npx tauri android init
node scripts/android-prepare.mjs
npx tauri android build --debug --apk
```

The APK is debug-signed, which is what makes it installable without a
published keystore. It is not for distribution.

## Installing

```bash
adb install -r <apk>
```

Over Wi-Fi, when the phone is not plugged in:

```bash
# on the phone: Developer options -> Wireless debugging -> pair
adb pair <ip>:<pair-port>
adb connect <ip>:<port>
adb install -r <apk>
```

## What to check first, in this order

1. **It starts.** The engine unpacks Python and ffmpeg on first launch, which
   takes a few seconds. `adb logcat | grep -i ytdlp` shows what it is doing.
2. **16 KB pages.** Android 15 and later can run with 16 KB memory pages, and
   native libraries built for 4 KB pages fail to load there. If the app dies
   at startup with a linker error in logcat, this is why, and the fix belongs
   in the engine library, not here.
3. **A folder is granted.** Settings → the folder button opens the system
   picker. Without a grant, finished files land in Downloads.
4. **A real download.** Anything small. Watch for three things: progress
   moves, the file appears in the chosen folder, and the queue survives
   leaving and reopening the app.
5. **The update centre.** yt-dlp reports a version and updates on demand —
   this is the Android counterpart of downloading a new binary on the
   desktop.

## Verified on a device

Samsung Galaxy S7 edge, LineageOS 20 (Android 13), arm64, 4 KB pages:

- the engine unpacks Python 3.12, ffmpeg 7.1.1 and QuickJS from the APK on
  first launch and runs
- yt-dlp updates itself in place through the same GitHub feed the desktop
  reads (2025.11.12 → 2026.08.19)
- a 332 MB download completes and the file lands in the phone's Downloads
  folder, visible to every other app

Three things that only a device could show, all fixed:

- the engine library keys options by flag name, so a flat argument vector
  fed one token at a time arrives scrambled and yt-dlp reads the values as
  URLs; arguments go through `addCommands` instead
- ffmpeg is installed as `libffmpeg.so` in the native library directory,
  which is the only place Android will execute from and not a name yt-dlp
  looks for; the app links it under the expected name and points yt-dlp there
- the library throws whenever yt-dlp exits non-zero, which it does after an
  ignored postprocessing error — with the finished file already written. The
  file is the proof of a download, not the exit code

## Size

One APK per architecture, about **56–60 MB**. Nearly all of it is the engine
and it cannot be otherwise: Android only executes code from an app's native
library directory, so ffmpeg (35 MB), the Python runtime (14 MB) and yt-dlp
(3 MB) must ship inside the package — there is no "download it on first run"
the way the desktop does it.

The app's own library is 6.4 MB. It was 178 MB until the build was switched
from debug to release, which is where a 110 MB APK came from.

Two things that are deliberately not done:

- **dex shrinking is off.** R8 renames the classes the engine uses to unpack
  itself, and the release build then installed and died on first launch. It
  saved about 8 MB in an app whose weight is native libraries.
- **ffmpeg is the full build.** A trimmed one would save 15–20 MB and put us
  on the hook for every codec yt-dlp meets on hundreds of sites, for four
  architectures, forever. The failure would land on a user's video, not in
  CI.

## Known limits

- Downloads stop when Android kills the app; a foreground service to hold them
  open is not wired yet.
- The app cannot install its own update yet: the update panel points at the
  release instead of handing the APK to the system installer.
- Battery optimisation will interrupt long downloads until the service exists.
- Every CI run signs with a fresh debug key, so a new APK will not install
  over an old one — uninstall first. A real signing key removes this.
