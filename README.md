# Hyperbola

A yt-dlp front end for **Windows** and **Android**, built from one engine.

Descended from [Parabolic](https://github.com/NickvisionApps/Parabolic) (its
core/shell architecture and yt-dlp depth) and
[Open Video Downloader](https://github.com/StefanLobbenmeier/youtube-dl-gui)
(its habit of keeping yt-dlp *and* ffmpeg current at runtime), with the piece
neither has: **one update center** — app, yt-dlp and ffmpeg in a single list,
with one button that brings all of them up to date.

## Status

Early. The engine is written and tested; the shells are being built.

| Piece | State |
|---|---|
| `hyperbola-core` — probing, arguments, progress, queue, updates | 77 tests |
| `hyperbola-runner` + CLI harness | real downloads verified on macOS and Windows |
| Windows app | installs, runs, and fetches yt-dlp and ffmpeg by itself on a clean machine |
| Update centre | app, yt-dlp and ffmpeg in one list; failures shown, not swallowed |
| Android app | verified on a phone: engine runs, updates itself, downloads land in Downloads |
| CI | Windows installer and one APK per ABI on every run |

Verified end to end on Windows: a fresh install downloaded yt-dlp (17 MB) and
ffmpeg (170 MB archive) unattended, and the same engine then pulled a 59 MB
video down to the exact path it reported.

Verified end to end on Android (Galaxy S7 edge, LineageOS 20): the engine
unpacked from the APK, updated its own yt-dlp from 2025.11.12 to 2026.08.19,
and a 332 MB download finished in the phone's Downloads folder.

## Building

```bash
cargo test                       # the engine and the runner

# Run the engine without a window — needs yt-dlp on PATH
cargo run -p hyperbola-cli -- probe <url>
cargo run -p hyperbola-cli -- get <url> --max-height 1080 --out ~/Downloads

# The desktop app
npm ci && npm --prefix ui ci
npm run dev                      # or: npm run build
```

## Licence

GPL-3.0-or-later. See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and
for what each donor project contributed.
