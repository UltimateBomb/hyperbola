# Hyperbola — architecture

Hyperbola is a yt-dlp front end for Windows and Android built from one
engine. It descends from two projects and takes a different shape from both:

- **[Parabolic](https://github.com/NickvisionApps/Parabolic)** (MIT) —
  contributed the idea of a platform-independent core with thin per-platform
  shells, and the depth of its yt-dlp coverage: recovery of interrupted
  downloads, cookies, subtitles, postprocessors, cutting with ffmpeg instead
  of `--download-sections`.
- **[Open Video Downloader](https://github.com/StefanLobbenmeier/youtube-dl-gui)**
  (AGPL-3.0) — contributed the dependency-update thinking: yt-dlp *and*
  ffmpeg kept current at runtime, not frozen at install time, plus clipboard
  watching and post-queue actions.

No code is copied from either. Both are credited as design donors; Hyperbola
is GPL-3.0-or-later because it ships ffmpeg alongside the app.

## The one rule

**The engine knows no platform. The shells know no rules.**

`hyperbola-core` builds command lines, parses output, runs the queue and
decides what is out of date. It performs no I/O: no process spawning, no
sockets, no files. Everything platform-specific — spawning yt-dlp on Windows,
calling the bundled Python on Android, writing through the Storage Access
Framework, drawing a window — lives in a shell and is reached through a small
port trait.

That is what makes Windows and Android the same product rather than two
lookalike apps: a rule fixed in the engine is fixed on both, in the same
release, with the same tests behind it.

## Layout

```
crates/core/            hyperbola-core — the engine (no I/O, 100% unit-tested)
  version.rs            one comparison rule for app / yt-dlp / ffmpeg versions
  domain.rs             Download, Format, MediaItem, Progress, DownloadState
  probe.rs              yt-dlp --dump-single-json  ->  MediaProbe
  args.rs               DownloadOptions            ->  yt-dlp argv
  progress.rs           yt-dlp stdout line         ->  Event
  queue.rs              the queue state machine (concurrency, retry, pause)
  updates.rs            the update center: what is stale, which asset to fetch

crates/runner-desktop/  spawns yt-dlp.exe and ffmpeg.exe, installs updates
plugins/ytdlp-android/  Tauri plugin: Kotlin wrapper over youtubedl-android
ui/                     one web UI, shared by both shells
src-tauri/              the Tauri application (Windows + Android targets)
.github/workflows/      builds Windows installer and Android APK -> Releases
```

## Data flow, one download

```
URL
 └─ core::args::build_probe_args ──► shell runs yt-dlp ──► JSON
     └─ core::probe::parse_probe ──► MediaProbe (formats, subtitles, playlist)
         └─ user picks quality / container / subtitles  ──► DownloadOptions
             └─ core::args::build_download_args ──► shell runs yt-dlp
                 └─ each stdout line ──► core::progress::parse_line ──► Event
                     └─ core::queue::Queue::on_progress / on_completed / on_failed
```

The queue never runs anything itself. The shell loop is: ask
`Queue::start_next()` for an id, run it, feed events back, repeat. A download
that fails for a retryable reason is re-queued by the engine until its attempt
budget runs out; a cancelled download stays cancelled even if its dying
process reports an error afterwards.

## The update center

The feature neither donor has: **one panel, one button.** Every component
reports the same `ComponentStatus`, so the UI shows a single list.

| Component | Windows source | Android source |
|---|---|---|
| **yt-dlp** | GitHub Releases `yt-dlp/yt-dlp` (stable) or `yt-dlp-nightly-builds` — `yt-dlp.exe` / `yt-dlp_arm64.exe` | `YoutubeDL.updateYoutubeDL()` with the same two channels |
| **ffmpeg** | GitHub Releases `BtbN/FFmpeg-Builds`, `win64-gpl` archive | bundled with the engine module; updated with the app |
| **Hyperbola** | GitHub Releases of this repo, NSIS/MSI installer | GitHub Releases APK, installed via the system installer |

Rules the engine enforces:

- A failed check is `Unknown`, never `UpToDate`. Silence must not read as health.
- A locally newer version (a nightly ahead of stable) is *not* a downgrade prompt.
- Missing is distinct from outdated: a missing yt-dlp blocks downloads and the
  UI says so; a missing ffmpeg only limits merging.
- yt-dlp is the critical component and sorts first in "update everything" —
  it is the one that goes stale in days when a site changes.

## Android

Android has no ability to run a yt-dlp binary the way a desktop does: since
Android 10 an app may only execute code from its own native library
directory. The engine therefore does not spawn anything there. Instead:

- **[youtubedl-android](https://github.com/yausername/youtubedl-android)**
  (`io.github.junkfood02.youtubedl-android`, 0.18.1) ships yt-dlp, a Python
  runtime, ffmpeg, aria2c and QuickJS packaged as native libraries. It is the
  same engine Seal uses.
- A **Tauri plugin** wraps it in Kotlin and exposes probe / download / cancel
  / update to the Rust core. The core builds the same argument vector as on
  Windows and hands it over; the plugin returns the same stdout lines, so
  `progress.rs` parses identical events on both platforms.
- Downloads run in a **foreground service** with a notification, because
  Android kills background work aggressively.
- Files are written through the **Storage Access Framework** to a folder the
  user picks once, so downloads survive uninstall and are visible to other
  apps.
- The app updates itself from GitHub Releases: check, download the APK, hand
  it to the system installer via `REQUEST_INSTALL_PACKAGES`. There is no Play
  Store in this path.

Two Android risks are tracked from day one: 16 KB memory pages on Android 15+
(native libraries must be aligned) and battery optimisation killing long
downloads. Both are verified on a real device, not an emulator.

## Testing

- The engine is unit-tested end to end: every parser against real yt-dlp
  output shapes, every queue rule against its edge case, every update rule
  against a real GitHub release feed.
- The shells are tested by running the actual app: a Windows build on the
  Windows machine, an APK on a physical Android device.
- CI builds both targets on every tag and publishes them to Releases, which is
  also the update feed the app reads.
