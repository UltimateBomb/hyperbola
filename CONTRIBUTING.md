# Contributing

## The one rule

The engine (`crates/core`) knows no platform and does no I/O — no processes,
no sockets, no files. If a change needs any of those, it belongs in a shell:
`crates/runner` for the desktop, `plugins/ytdlp-android` for the phone.

Rules live in the engine so both platforms get them at once. Everything in
there is unit-tested, and a change without a test that fails before it is not
finished.

## Before opening a pull request

```bash
cargo fmt --all
cargo clippy --package hyperbola-core --all-targets -- -D warnings
cargo test
npm --prefix ui run build
```

## Testing what CI cannot

CI proves it compiles. It does not prove a download works, and several of the
faults in this project's history were invisible to it: a scrambled argument
vector, an error message lost to a code page, a file that downloads perfectly
and will not play. Use the harness on a real machine:

```bash
cargo run -p hyperbola-cli -- probe <url>
cargo run -p hyperbola-cli -- get <url> --out ~/Downloads
cargo run -p hyperbola-cli -- parse dump.json   # a saved --dump-single-json
```

For a phone, `docs/ANDROID.md` lists what to check first and in what order.

## Reporting a download that fails

The message the app shows is the useful part — it comes from yt-dlp. Better
still, attach the output of:

```bash
yt-dlp --dump-single-json --flat-playlist <url> > dump.json
```

That one file is usually the whole diagnosis.
