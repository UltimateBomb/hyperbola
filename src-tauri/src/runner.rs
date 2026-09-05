//! Runs yt-dlp and turns its output into queue updates.
//!
//! The engine decides *what* to run; this module is the only place that knows
//! how to start a process, and the only place that talks to the window.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use hyperbola_core::args::{build_download_args, build_probe_args, RunnerEnv};
use hyperbola_core::domain::{DownloadId, DownloadOptions, MediaProbe};
use hyperbola_core::probe::parse_probe;
use hyperbola_core::progress::{parse_line, Event};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::AppState;

/// How often progress is pushed to the window. yt-dlp reports far faster than
/// a human can read.
const EMIT_INTERVAL: Duration = Duration::from_millis(200);

#[cfg(not(target_os = "android"))]
fn command(program: &PathBuf) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::deps::CREATE_NO_WINDOW);
    }
    command
}

/// Reads a URL's metadata without downloading anything.
#[cfg(not(target_os = "android"))]
pub async fn probe(app: &AppHandle, url: &str) -> Result<MediaProbe, String> {
    let (ytdlp, args) = {
        let state = app.state::<AppState>();
        let ytdlp = state
            .deps
            .ytdlp_path()
            .ok_or("yt-dlp is not installed — open Updates and install it")?;
        let settings = state.settings.lock().unwrap();
        let env = state.runner_env(&settings);
        let args = build_probe_args(url, &settings.cookies, settings.proxy.as_deref(), &env);
        (ytdlp, args)
    };

    let output = command(&ytdlp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("could not run yt-dlp: {e}"))?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .unwrap_or_else(|| stderr.lines().last().unwrap_or("yt-dlp failed"));
        return Err(message.trim_start_matches("ERROR:").trim().to_string());
    }
    parse_probe(url, &String::from_utf8_lossy(&output.stdout)).map_err(|e| e.to_string())
}

/// Runs one download to completion, feeding every event back into the queue.
#[cfg(not(target_os = "android"))]
pub async fn run(app: AppHandle, id: DownloadId, options: DownloadOptions, cancel: oneshot::Receiver<()>) {
    let prepared = {
        let state = app.state::<AppState>();
        state.deps.ytdlp_path().map(|ytdlp| {
            let settings = state.settings.lock().unwrap();
            let env = state.runner_env(&settings);
            (ytdlp, build_download_args(&options, &env))
        })
    };
    let Some((ytdlp, args)) = prepared else {
        finish_failed(&app, id, "yt-dlp is not installed".into(), false);
        return;
    };

    let mut child = match command(&ytdlp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            finish_failed(&app, id, format!("could not run yt-dlp: {e}"), true);
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    let mut destination: Option<PathBuf> = None;
    let mut last_error: Option<String> = None;
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut cancel = cancel;

    loop {
        tokio::select! {
            // The user pressed cancel: kill the process and stop reporting.
            _ = &mut cancel => {
                let _ = child.kill().await;
                crate::emit_queue(&app);
                crate::pump(app.clone());
                return;
            }
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(event) = parse_line(&line) {
                            apply(&app, id, event, &mut destination, &mut last_error, &mut last_emit);
                        }
                    }
                    _ => break,
                }
            }
            line = stderr_lines.next_line() => {
                if let Ok(Some(line)) = line {
                    if let Some(event) = parse_line(&line) {
                        apply(&app, id, event, &mut destination, &mut last_error, &mut last_emit);
                    }
                }
            }
        }
    }

    // Drain whatever is left on stderr so the failure message is the real one.
    while let Ok(Some(line)) = stderr_lines.next_line().await {
        if let Some(Event::Error(message)) = parse_line(&line) {
            last_error = Some(message);
        }
    }

    let status = child.wait().await;
    let succeeded = matches!(&status, Ok(status) if status.success());
    if succeeded {
        let path = destination.unwrap_or_else(|| options.output_dir.clone());
        {
            let state = app.state::<AppState>();
            let mut queue = state.queue.lock().unwrap();
            queue.on_completed(id, path);
        }
        crate::emit_queue(&app);
    } else {
        let message = last_error.unwrap_or_else(|| match &status {
            Ok(status) => format!("yt-dlp exited with {status}"),
            Err(e) => e.to_string(),
        });
        let retryable = is_retryable(&message);
        finish_failed(&app, id, message, retryable);
    }
    crate::pump(app.clone());
}

fn apply(
    app: &AppHandle,
    id: DownloadId,
    event: Event,
    destination: &mut Option<PathBuf>,
    last_error: &mut Option<String>,
    last_emit: &mut Instant,
) {
    match event {
        Event::Progress(progress) => {
            let state = app.state::<AppState>();
            state.queue.lock().unwrap().on_progress(id, progress);
            if last_emit.elapsed() >= EMIT_INTERVAL {
                *last_emit = Instant::now();
                crate::emit_queue(app);
            }
            return;
        }
        Event::Stage(stage) => {
            let state = app.state::<AppState>();
            let mut queue = state.queue.lock().unwrap();
            let current = queue.get(id).and_then(|download| match &download.state {
                hyperbola_core::domain::DownloadState::Running(progress) => Some(*progress),
                _ => None,
            });
            if let Some(mut progress) = current {
                progress.stage = stage;
                queue.on_progress(id, progress);
            }
        }
        Event::Destination(path) | Event::AlreadyDownloaded(path) => {
            *destination = Some(path);
        }
        Event::Error(message) => *last_error = Some(message),
        Event::Warning(_) => {}
    }
    crate::emit_queue(app);
    *last_emit = Instant::now();
}

fn finish_failed(app: &AppHandle, id: DownloadId, message: String, retryable: bool) {
    {
        let state = app.state::<AppState>();
        let mut queue = state.queue.lock().unwrap();
        queue.on_failed(id, message, retryable);
    }
    crate::emit_queue(app);
    crate::pump(app.clone());
}

/// Whether a failure is worth another attempt. Network trouble is; a video
/// that no longer exists is not — retrying that only wastes the user's time
/// and hides the real reason behind three identical errors.
pub fn is_retryable(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    const PERMANENT: [&str; 9] = [
        "video unavailable",
        "private video",
        "removed by the uploader",
        "has been terminated",
        "members-only",
        "is not available in your country",
        "requested format is not available",
        "unsupported url",
        "404",
    ];
    !PERMANENT.iter().any(|needle| text.contains(needle))
}


// ---------------------------------------------------------------------------
// Android
//
// No process is spawned here: the engine lives inside the APK as native
// libraries and is driven through the plugin. Everything above the process
// boundary — the arguments, the parser, the queue — is the same code the
// desktop uses.
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
pub async fn probe(app: &AppHandle, url: &str) -> Result<MediaProbe, String> {
    use tauri_plugin_ytdlp::{ProbeRequest, YtdlpExt};

    let args = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        let env = state.runner_env(&settings);
        build_probe_args(url, &settings.cookies, settings.proxy.as_deref(), &env)
    };
    let handle = app.clone();
    let url_owned = url.to_string();
    let response = tauri::async_runtime::spawn_blocking(move || {
        handle.ytdlp().probe(ProbeRequest {
            url: url_owned.clone(),
            // The engine puts the URL first; the plugin passes it separately.
            args: args.into_iter().skip(1).collect(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    parse_probe(url, &response.json).map_err(|e| e.to_string())
}

#[cfg(target_os = "android")]
pub async fn run(
    app: AppHandle,
    id: DownloadId,
    options: DownloadOptions,
    cancel: oneshot::Receiver<()>,
) {
    use tauri_plugin_ytdlp::{DownloadRequest, PublishRequest, YtdlpExt};

    let process_id = format!("hyperbola-{}", id.0);
    let (args, tree_uri) = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        let env = state.runner_env(&settings);
        (build_download_args(&options, &env), settings.android_tree_uri.clone())
    };

    let request = DownloadRequest {
        id: process_id.clone(),
        url: options.url.clone(),
        args: args.into_iter().skip(1).collect(),
        output_dir: options.output_dir.display().to_string(),
    };
    let engine = app.clone();
    let mut download = tauri::async_runtime::spawn_blocking(move || engine.ytdlp().download(request));

    let mut destination: Option<PathBuf> = None;
    let mut last_error: Option<String> = None;
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut cancel = cancel;
    let mut poll = tokio::time::interval(Duration::from_millis(300));

    let outcome = loop {
        tokio::select! {
            _ = &mut cancel => {
                let handle = app.clone();
                let id = process_id.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || handle.ytdlp().cancel(&id)).await;
                crate::emit_queue(&app);
                crate::pump(app.clone());
                return;
            }
            _ = poll.tick() => {
                let handle = app.clone();
                let pid = process_id.clone();
                let polled = tauri::async_runtime::spawn_blocking(move || handle.ytdlp().poll_output(&pid)).await;
                if let Ok(Ok(output)) = polled {
                    for line in output.lines {
                        if let Some(event) = parse_line(&line) {
                            apply(&app, id, event, &mut destination, &mut last_error, &mut last_emit);
                        }
                    }
                }
            }
            result = &mut download => break result,
        }
    };

    let failed = match outcome {
        Ok(Ok(response)) if response.exit_code == 0 => None,
        Ok(Ok(response)) => Some(
            last_error
                .clone()
                .or_else(|| response.stderr.lines().last().map(str::to_string))
                .unwrap_or_else(|| format!("the engine exited with {}", response.exit_code)),
        ),
        Ok(Err(e)) => Some(e.to_string()),
        Err(e) => Some(e.to_string()),
    };

    if let Some(message) = failed {
        let retryable = is_retryable(&message);
        finish_failed(&app, id, message, retryable);
        crate::pump(app.clone());
        return;
    }

    // yt-dlp can only write inside the app's own directory; hand the finished
    // file to the folder the user picked, or to Downloads.
    let source = destination.unwrap_or_else(|| options.output_dir.clone());
    let handle = app.clone();
    let published = tauri::async_runtime::spawn_blocking(move || {
        handle.ytdlp().publish(PublishRequest {
            source_path: source.display().to_string(),
            tree_uri,
        })
    })
    .await;

    match published {
        Ok(Ok(result)) => {
            let state = app.state::<AppState>();
            state.queue.lock().unwrap().on_completed(id, PathBuf::from(result.display_path));
        }
        Ok(Err(e)) => {
            finish_failed(&app, id, e.to_string(), false);
            crate::pump(app.clone());
            return;
        }
        Err(e) => {
            finish_failed(&app, id, e.to_string(), false);
            crate::pump(app.clone());
            return;
        }
    }
    crate::emit_queue(&app);
    crate::pump(app.clone());
}

/// Builds the environment yt-dlp runs in from the current settings.
impl AppState {
    pub fn runner_env(&self, settings: &crate::settings::Settings) -> RunnerEnv {
        RunnerEnv {
            temp_dir: self.temp_dir.clone(),
            // On Android ffmpeg and the JS runtime come from the engine
            // library itself; pointing yt-dlp at a path would break it.
            #[cfg(target_os = "android")]
            ffmpeg_path: None,
            #[cfg(not(target_os = "android"))]
            ffmpeg_path: self.deps.ffmpeg_path(),
            plugin_dir: None,
            js_runtime: None,
            concurrent_fragments: settings.concurrent_fragments,
            windows_filenames: cfg!(windows),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_retryable;

    #[test]
    fn network_failures_are_retried() {
        assert!(is_retryable("[download] Connection reset by peer"));
        assert!(is_retryable("Unable to download webpage: The read operation timed out"));
        assert!(is_retryable("HTTP Error 503: Service Unavailable"));
    }

    #[test]
    fn gone_or_impossible_downloads_are_not_retried() {
        assert!(!is_retryable("[youtube] abc: Video unavailable"));
        assert!(!is_retryable("ERROR: Private video. Sign in if you've been granted access"));
        assert!(!is_retryable("Requested format is not available"));
        assert!(!is_retryable("HTTP Error 404: Not Found"));
    }
}
