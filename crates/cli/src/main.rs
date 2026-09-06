//! A window-less way to run the engine.
//!
//! Same core, same runner, same parser as the desktop app — only the queue
//! and the window are missing. Useful for checking on a real machine that a
//! change works end to end, and for reporting a problem without screenshots.
//!
//!   hyperbola probe <url>
//!   hyperbola get <url> [--audio] [--max-height 1080] [--out DIR]

use std::path::PathBuf;
use std::process::ExitCode;

use hyperbola_core::args::RunnerEnv;
use hyperbola_core::domain::{human_bytes, CookieSource, DownloadOptions, MediaKind};
use hyperbola_core::progress::Event;
use hyperbola_runner::Runner;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(usage());
    };
    let ytdlp = find_ytdlp()?;
    let env = RunnerEnv {
        temp_dir: std::env::temp_dir(),
        ffmpeg_path: which("ffmpeg"),
        plugin_dir: None,
        js_runtime: None,
        concurrent_fragments: 4,
        windows_filenames: cfg!(windows),
    };
    let runner = Runner::new(ytdlp, env);

    match command {
        "probe" => {
            let url = args.get(1).ok_or_else(usage)?;
            let probe = runner.probe(url, &CookieSource::None, None).await?;
            if let Some(playlist) = &probe.playlist_title {
                println!("playlist: {playlist} ({} items)", probe.items.len());
            }
            for item in &probe.items {
                println!("\n{}", item.title);
                println!(
                    "  by       {}",
                    item.uploader.as_deref().unwrap_or("unknown")
                );
                println!("  duration {:.0}s", item.duration_secs.unwrap_or(0.0));
                println!("  formats  {}", item.formats.len());
                let heights: Vec<String> =
                    item.resolutions().iter().map(|h| format!("{h}p")).collect();
                println!("  quality  {}", heights.join(", "));
                if let Some(best) = item.best_video() {
                    println!("  best     {}", best.label());
                }
                let subtitles: Vec<&str> = item
                    .subtitles
                    .iter()
                    .filter(|s| !s.is_automatic)
                    .map(|s| s.language.as_str())
                    .collect();
                if !subtitles.is_empty() {
                    println!("  subs     {}", subtitles.join(", "));
                }
            }
            Ok(())
        }
        "get" => {
            let url = args.get(1).ok_or_else(usage)?;
            let out = flag(&args, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let audio = args.iter().any(|a| a == "--audio");
            let mut options = if audio {
                DownloadOptions::audio(url.clone(), out.clone())
            } else {
                DownloadOptions::video(url.clone(), out.clone())
            };
            options.max_height = flag(&args, "--max-height").and_then(|v| v.parse().ok());
            options.embed_thumbnail = false;

            println!("saving to {}", out.display());
            let mut last_percent = -1i64;
            let outcome = runner
                .download(
                    &options,
                    |event| match event {
                        Event::Progress(progress) => {
                            let percent = progress
                                .fraction()
                                .map(|f| (f * 100.0) as i64)
                                .unwrap_or(-1);
                            if percent != last_percent {
                                last_percent = percent;
                                println!(
                                    "  {percent:>3}%  {} of {}  {}/s",
                                    human_bytes(progress.downloaded_bytes),
                                    human_bytes(progress.total_bytes.unwrap_or(0)),
                                    human_bytes(progress.speed_bps.unwrap_or(0.0) as u64),
                                );
                            }
                        }
                        Event::Stage(stage) => println!("  stage: {stage:?}"),
                        Event::Destination(path) => println!("  writing {}", path.display()),
                        Event::Error(message) => eprintln!("  {message}"),
                        _ => {}
                    },
                    None,
                )
                .await?;

            match (outcome.success, outcome.destination) {
                (true, Some(path)) => {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    println!("done: {} ({})", path.display(), human_bytes(size));
                    Ok(())
                }
                (true, None) => {
                    println!("done");
                    Ok(())
                }
                (false, _) => Err(outcome.error.unwrap_or_else(|| "download failed".into())),
            }
        }
        _ => Err(usage()),
    }
}

fn usage() -> String {
    "usage:\n  hyperbola probe <url>\n  hyperbola get <url> [--audio] [--max-height N] [--out DIR]"
        .to_string()
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn find_ytdlp() -> Result<PathBuf, String> {
    std::env::var_os("YTDLP")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| which("yt-dlp"))
        .ok_or_else(|| "yt-dlp not found on PATH (set YTDLP to point at it)".to_string())
}

fn which(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(&name);
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Audio downloads exist so this import is used on every platform.
#[allow(dead_code)]
fn _kinds() -> [MediaKind; 2] {
    [MediaKind::Video, MediaKind::Audio]
}
