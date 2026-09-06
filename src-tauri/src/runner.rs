//! Connects the queue to whatever actually runs yt-dlp.
//!
//! On the desktop that is `hyperbola-runner`, which spawns the binary. On
//! Android it is the plugin, which drives the engine bundled in the APK.
//! Both hand back the same events, so everything below this line — the queue,
//! the retry rule, what the window is told — is one implementation.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hyperbola_core::args::RunnerEnv;
use hyperbola_core::domain::{DownloadId, DownloadOptions, MediaProbe};
use hyperbola_core::progress::Event;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

#[cfg(target_os = "android")]
use hyperbola_core::args::{build_download_args, build_probe_args};
#[cfg(target_os = "android")]
use hyperbola_core::probe::parse_probe;
#[cfg(target_os = "android")]
use hyperbola_core::progress::parse_line;
#[cfg(not(target_os = "android"))]
use hyperbola_runner::Runner;

use crate::AppState;

/// How often progress is pushed to the window. yt-dlp reports far faster than
/// a human can read.
const EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// Builds a runner from the current settings, along with the cookie source
/// and proxy those settings ask for.
#[cfg(not(target_os = "android"))]
fn build_runner(
    app: &AppHandle,
) -> Result<(Runner, hyperbola_core::domain::CookieSource, Option<String>), String> {
    let state = app.state::<AppState>();
    let ytdlp = state
        .deps
        .ytdlp_path()
        .ok_or("yt-dlp is not installed — open Updates and install it")?;
    let settings = state.settings.lock().unwrap();
    let env = state.runner_env(&settings);
    Ok((
        Runner::new(ytdlp, env),
        settings.cookies.clone(),
        settings.proxy.clone(),
    ))
}

/// Reads a URL's metadata without downloading anything.
#[cfg(not(target_os = "android"))]
pub async fn probe(app: &AppHandle, url: &str) -> Result<MediaProbe, String> {
    let (runner, cookies, proxy) = build_runner(app)?;
    runner.probe(url, &cookies, proxy.as_deref()).await
}

/// Runs one download to completion, feeding every event back into the queue.
#[cfg(not(target_os = "android"))]
pub async fn run(
    app: AppHandle,
    id: DownloadId,
    options: DownloadOptions,
    cancel: oneshot::Receiver<()>,
) {
    let runner = match build_runner(&app) {
        Ok((runner, _, _)) => runner,
        Err(message) => {
            finish_failed(&app, id, message, false);
            return;
        }
    };

    let events = app.clone();
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut destination: Option<PathBuf> = None;
    let mut last_error: Option<String> = None;
    let outcome = runner
        .download(
            &options,
            |event| {
                apply(
                    &events,
                    id,
                    event,
                    &mut destination,
                    &mut last_error,
                    &mut last_emit,
                )
            },
            Some(cancel),
        )
        .await;

    match outcome {
        Ok(outcome) if outcome.canceled => {
            crate::emit_queue(&app);
        }
        Ok(outcome) if outcome.success => {
            let path = outcome
                .destination
                .or(destination)
                .unwrap_or_else(|| options.output_dir.clone());
            {
                let state = app.state::<AppState>();
                state.queue.lock().unwrap().on_completed(id, path);
            }
            crate::save_queue(&app);
            crate::emit_queue(&app);
        }
        Ok(outcome) => {
            let message = outcome
                .error
                .or(last_error)
                .unwrap_or_else(|| "yt-dlp failed".to_string());
            let retryable = hyperbola_runner::is_retryable(&message);
            finish_failed(&app, id, message, retryable);
        }
        Err(message) => {
            let retryable = hyperbola_runner::is_retryable(&message);
            finish_failed(&app, id, message, retryable);
        }
    }
    crate::pump(app.clone());
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
        (
            build_download_args(&options, &env),
            settings.android_tree_uri.clone(),
        )
    };

    let request = DownloadRequest {
        id: process_id.clone(),
        url: options.url.clone(),
        args: args.into_iter().skip(1).collect(),
        output_dir: options.output_dir.display().to_string(),
    };
    let engine = app.clone();
    let mut download =
        tauri::async_runtime::spawn_blocking(move || engine.ytdlp().download(request));

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

    // The engine can finish between two polls, and the last lines it wrote
    // are the ones that matter: the final path arrives at the very end.
    let handle = app.clone();
    let pid = process_id.clone();
    if let Ok(Ok(output)) =
        tauri::async_runtime::spawn_blocking(move || handle.ytdlp().poll_output(&pid)).await
    {
        for line in output.lines {
            if let Some(event) = parse_line(&line) {
                apply(
                    &app,
                    id,
                    event,
                    &mut destination,
                    &mut last_error,
                    &mut last_emit,
                );
            }
        }
    }

    let failed = match outcome {
        Ok(Ok(response)) => {
            log::info!(
                "download {id}: exit {}, file {:?}, {} bytes of output",
                response.exit_code,
                destination,
                response.stderr.len()
            );
            // The file is the proof, not the exit code: the engine exits
            // non-zero after an ignored postprocessing error, with the
            // finished file already written.
            if destination.is_some() {
                if response.exit_code != 0 {
                    log::warn!(
                        "download {id} finished with code {} but produced a file",
                        response.exit_code
                    );
                }
                None
            } else {
                Some(hyperbola_runner::describe_failure(
                    last_error.clone(),
                    Some(response.exit_code),
                    &response.stderr,
                ))
            }
        }
        Ok(Err(e)) => Some(hyperbola_runner::describe_failure(
            Some(e.to_string()),
            None,
            "",
        )),
        Err(e) => Some(hyperbola_runner::describe_failure(
            Some(e.to_string()),
            None,
            "",
        )),
    };

    if let Some(message) = failed {
        let retryable = hyperbola_core::retry::is_retryable(&message);
        finish_failed(&app, id, message, retryable);
        crate::pump(app.clone());
        return;
    }

    // yt-dlp can only write inside the app's own directory; hand the finished
    // file to the folder the user picked, or to Downloads.
    let Some(source) = destination else {
        log::error!("download {id} produced no file path to publish");
        finish_failed(
            &app,
            id,
            "the engine finished without saying where it put the file".to_string(),
            true,
        );
        crate::pump(app.clone());
        return;
    };
    log::info!("download {id}: publishing {}", source.display());
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
            state
                .queue
                .lock()
                .unwrap()
                .on_completed(id, PathBuf::from(result.display_path));
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

/// Applies one event to the queue and, at most a few times a second, tells
/// the window about it.
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
    crate::save_queue(app);
    crate::emit_queue(app);
}

/// Builds the environment yt-dlp runs in from the current settings.
impl AppState {
    pub fn runner_env(&self, settings: &crate::settings::Settings) -> RunnerEnv {
        RunnerEnv {
            temp_dir: self.temp_dir.clone(),
            // On Android ffmpeg lives in the native library directory under a
            // name yt-dlp does not recognise; the engine reports a directory
            // where it is reachable as plain `ffmpeg`.
            #[cfg(target_os = "android")]
            ffmpeg_path: self.engine_ffmpeg_dir.lock().unwrap().clone(),
            #[cfg(not(target_os = "android"))]
            ffmpeg_path: self.deps.ffmpeg_path(),
            plugin_dir: None,
            js_runtime: None,
            concurrent_fragments: settings.concurrent_fragments,
            windows_filenames: cfg!(windows),
        }
    }
}
