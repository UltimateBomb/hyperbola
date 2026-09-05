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
| `hyperbola-core` — probing, arguments, progress, queue, updates | working, 69 tests |
| Windows shell (Tauri + WebView2) | in progress |
| Android shell (Tauri + youtubedl-android) | in progress |
| CI → signed Windows installer and APK in Releases | planned |

## Building

```bash
cargo test          # the engine
```

Shell build instructions land with the shells.

## Licence

GPL-3.0-or-later. See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and
for what each donor project contributed.
