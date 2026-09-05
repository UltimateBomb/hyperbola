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
| `hyperbola-core` — probing, arguments, progress, queue, updates | working, 74 tests |
| `hyperbola-runner` + CLI harness | working; real downloads verified |
| Windows shell (Tauri + WebView2) | builds; update centre wired |
| Android shell (Tauri + youtubedl-android) | written, not yet run on a device |
| CI → Windows installer and APK | building both |

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
