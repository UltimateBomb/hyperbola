# Changelog

## 0.1.2 — 2026-09-07

- **The update notice appears on its own.** The app asked once, at startup,
  so a release published while it was open stayed invisible — which is what
  happened to 0.1.1. It now looks again every hour and whenever you come
  back to the window, and the Updates panel says when it last looked.
- **A failed check no longer looks like good news.** No network, a VPN in
  the way, GitHub unreachable — all of it used to be swallowed, leaving a
  screen that said nothing was available. The header now says "check
  failed" and "Check now" reports the reason. It also stopped asking
  twice per press.
- **The right build for your phone.** A 64-bit phone could end up with the
  32-bit APK and, once there, keep pulling 32-bit builds forever. The app
  now asks the device which architecture it wants, so a wrongly installed
  copy repairs itself on the next update.

## 0.1.1 — 2026-09-07

- **Playlists open.** A playlist used to fail with "could not read media
  info" and a column number: the app asked for the full details of every
  video in it, which takes minutes and dies on the first unavailable one. A
  playlist is now listed in about a second and survives broken entries.
- **Files that play.** YouTube serves AV1 and Opus as "best"; a phone more
  than a few years old decodes neither, so downloads went through and would
  not open. H.264 and AAC now come first, with a setting for the newer codecs.
- **Play and send work on Android.** The buttons handed the system a text
  path, which no other app on a phone can read; they now use the handle the
  system gave the file. Sending offers Bluetooth and every messenger, or a
  Wi-Fi link — which is how a film reaches a car head unit in half a minute
  instead of forty.
- **The app installs its own updates** on Android, with a switch in Settings.
  Every build is signed with one key from now on, so a new version installs
  over the old one and keeps the queue, settings and chosen folder.
- **Failures say what to do.** "HTTP Error 403" now comes with "the site
  refused it — updating yt-dlp usually fixes this", and seven more.
- Windows error messages are no longer lost when they are not UTF-8.
- Reading a link is cached for ten minutes, so the same link is instant.

## 0.1.0 — 2026-09-07

First release. Windows installer and one APK per Android architecture.

- One engine for both platforms: arguments, output parsing, queue, updates.
- Windows fetches yt-dlp and ffmpeg on first run and keeps them current.
- Android carries the engine inside the APK, because it cannot execute a
  downloaded binary, and updates its own extractor from the same feed.
- One update centre for the app, yt-dlp and ffmpeg.
- A queue that survives a restart with partial files intact.
- Downloads continue on a phone with the screen off.
