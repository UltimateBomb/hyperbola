//! The desktop shell: state, commands, and the loop that keeps the queue moving.

mod deps;
mod runner;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use hyperbola_core::domain::{
    Container, Download, DownloadId, DownloadOptions, MediaKind, MediaProbe, TimeFrame,
};
use hyperbola_core::queue::{Queue, QueueStats};
use hyperbola_core::updates::{Component, UpdateReport};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use deps::Dependencies;
use settings::Settings;

pub struct AppState {
    pub queue: Mutex<Queue>,
    pub settings: Mutex<Settings>,
    pub deps: Dependencies,
    /// Cancel signals for downloads that are currently running.
    pub cancels: Mutex<HashMap<DownloadId, oneshot::Sender<()>>>,
    pub config_path: PathBuf,
    pub temp_dir: PathBuf,
}

/// What the window renders: the queue and its totals in one payload, so the
/// list can never disagree with the header.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub items: Vec<Download>,
    pub stats: QueueStats,
}

/// A download the user asked for. Everything not listed here comes from
/// settings, so the rules live in one place.
#[derive(Debug, Clone, Deserialize)]
pub struct AddRequest {
    pub url: String,
    pub title: Option<String>,
    pub kind: MediaKind,
    pub format_id: Option<String>,
    pub max_height: Option<u32>,
    pub filename: Option<String>,
    pub time_frame: Option<TimeFrame>,
}

impl AddRequest {
    fn into_options(self, settings: &Settings) -> (DownloadOptions, String) {
        let container = match self.kind {
            MediaKind::Video => settings.video_container,
            MediaKind::Audio => settings.audio_container,
        };
        let title = self.title.clone().unwrap_or_else(|| self.url.clone());
        let options = DownloadOptions {
            url: self.url,
            kind: self.kind,
            container,
            format_id: self.format_id,
            max_height: self.max_height,
            output_dir: settings.download_dir.clone(),
            filename: self.filename,
            subtitle_languages: settings.subtitle_languages.clone(),
            embed_subtitles: settings.embed_subtitles,
            embed_metadata: settings.embed_metadata,
            embed_thumbnail: settings.embed_thumbnail && container != Container::Webm,
            embed_chapters: settings.embed_chapters,
            remove_sponsor_segments: settings.remove_sponsor_segments,
            time_frame: self.time_frame,
            speed_limit: settings.speed_limit_bps(),
            cookies: settings.cookies.clone(),
            proxy: settings.proxy.clone(),
            extra_args: Vec::new(),
        };
        (options, title)
    }
}

/// Pushes the current queue to the window.
pub fn emit_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let queue = state.queue.lock().unwrap();
        Snapshot { items: queue.items().to_vec(), stats: queue.stats() }
    };
    let _ = app.emit("queue-changed", snapshot);
}

/// Starts as many queued downloads as the concurrency limit allows.
pub fn pump(app: AppHandle) {
    loop {
        let next = {
            let state = app.state::<AppState>();
            let mut queue = state.queue.lock().unwrap();
            match queue.start_next() {
                Some(id) => queue.get(id).map(|d| (id, d.options.clone())),
                None => None,
            }
        };
        let Some((id, options)) = next else { break };

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let state = app.state::<AppState>();
            state.cancels.lock().unwrap().insert(id, cancel_tx);
        }
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            runner::run(handle, id, options, cancel_rx).await;
        });
    }
    emit_queue(&app);
}

/// Stops the process behind a download, if one is running.
fn signal_cancel(app: &AppHandle, id: DownloadId) {
    let state = app.state::<AppState>();
    let sender = state.cancels.lock().unwrap().remove(&id);
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
}

#[tauri::command]
async fn probe_url(app: AppHandle, url: String) -> Result<MediaProbe, String> {
    runner::probe(&app, url.trim()).await
}

#[tauri::command]
fn add_downloads(app: AppHandle, requests: Vec<AddRequest>) -> Vec<DownloadId> {
    let ids = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap().clone();
        let mut queue = state.queue.lock().unwrap();
        queue.set_max_concurrent(settings.max_concurrent);
        requests
            .into_iter()
            .map(|request| {
                let (options, title) = request.into_options(&settings);
                queue.add(options, title)
            })
            .collect()
    };
    pump(app);
    ids
}

#[tauri::command]
fn queue_snapshot(app: AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let queue = state.queue.lock().unwrap();
    Snapshot { items: queue.items().to_vec(), stats: queue.stats() }
}

#[tauri::command]
fn pause_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().pause(id);
    }
    signal_cancel(&app, id);
    pump(app);
}

#[tauri::command]
fn resume_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().resume(id);
    }
    pump(app);
}

#[tauri::command]
fn cancel_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().cancel(id);
    }
    signal_cancel(&app, id);
    pump(app);
}

#[tauri::command]
fn retry_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().retry(id);
    }
    pump(app);
}

#[tauri::command]
fn remove_download(app: AppHandle, id: DownloadId) {
    signal_cancel(&app, id);
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().remove(id);
    }
    pump(app);
}

#[tauri::command]
fn clear_finished(app: AppHandle) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().clear_finished();
    }
    emit_queue(&app);
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    app.state::<AppState>().settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let mut queue = state.queue.lock().unwrap();
        queue.set_max_concurrent(settings.max_concurrent);
    }
    settings.save(&state.config_path).map_err(|e| e.to_string())?;
    *state.settings.lock().unwrap() = settings;
    pump(app.clone());
    Ok(())
}

/// Where the binaries Hyperbola uses actually live — shown in the update
/// panel so "which yt-dlp is this?" is never a guess.
#[derive(Debug, Serialize)]
struct DependencyPaths {
    ytdlp: Option<String>,
    ffmpeg: Option<String>,
    bin_dir: String,
}

#[tauri::command]
fn dependency_paths(app: AppHandle) -> DependencyPaths {
    let state = app.state::<AppState>();
    DependencyPaths {
        ytdlp: state.deps.ytdlp_path().map(|p| p.display().to_string()),
        ffmpeg: state.deps.ffmpeg_path().map(|p| p.display().to_string()),
        bin_dir: state.deps.bin_dir().display().to_string(),
    }
}

#[tauri::command]
async fn check_updates(app: AppHandle) -> UpdateReport {
    let channel = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        settings.ytdlp_channel
    };
    let report = {
        let state = app.state::<AppState>();
        state.deps.check(channel, env!("CARGO_PKG_VERSION")).await
    };
    let _ = app.emit("updates-changed", report.clone());
    report
}

/// Progress of a dependency download, pushed to the window while it runs.
#[derive(Debug, Clone, Serialize)]
struct DependencyProgress {
    component: Component,
    downloaded: u64,
    total: Option<u64>,
}

#[tauri::command]
async fn install_update(app: AppHandle, component: Component) -> Result<String, String> {
    let channel = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        settings.ytdlp_channel
    };
    let handle = app.clone();
    let progress = move |downloaded: u64, total: Option<u64>| {
        let _ = handle.emit(
            "dependency-progress",
            DependencyProgress { component, downloaded, total },
        );
    };
    let version = {
        let state = app.state::<AppState>();
        state.deps.install(component, channel, &progress).await?
    };
    let _ = check_updates(app).await;
    Ok(version.as_str().to_string())
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_ytdlp::init())
        .setup(|app| {
            let handle = app.handle();
            let config_dir = handle.path().app_config_dir()?;
            let data_dir = handle.path().app_data_dir()?;
            let temp_dir = handle.path().app_cache_dir()?.join("partials");
            std::fs::create_dir_all(&config_dir).ok();
            std::fs::create_dir_all(&temp_dir).ok();

            let config_path = config_dir.join("settings.json");
            let settings = Settings::load(&config_path);
            let queue = Queue::new(settings.max_concurrent);

            app.manage(AppState {
                queue: Mutex::new(queue),
                settings: Mutex::new(settings),
                deps: Dependencies::new(data_dir.join("bin")),
                cancels: Mutex::new(HashMap::new()),
                config_path,
                temp_dir,
            });

            // First run, or a dependency the user deleted: fetch what is
            // missing before the user hits a confusing failure.
            let startup = handle.clone();
            tauri::async_runtime::spawn(async move {
                let (auto_check, auto_install, channel) = {
                    let state = startup.state::<AppState>();
                    let settings = state.settings.lock().unwrap();
                    (
                        settings.auto_check_updates,
                        settings.auto_install_dependency_updates,
                        settings.ytdlp_channel,
                    )
                };
                if !auto_check {
                    return;
                }
                let report = {
                    let state = startup.state::<AppState>();
                    state.deps.check(channel, env!("CARGO_PKG_VERSION")).await
                };
                let _ = startup.emit("updates-changed", report.clone());
                if !auto_install {
                    return;
                }
                for status in report.actionable() {
                    if status.component == Component::App {
                        continue;
                    }
                    let handle = startup.clone();
                    let component = status.component;
                    let _ = install_update(handle, component).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_url,
            add_downloads,
            queue_snapshot,
            pause_download,
            resume_download,
            cancel_download,
            retry_download,
            remove_download,
            clear_finished,
            get_settings,
            set_settings,
            dependency_paths,
            check_updates,
            install_update,
            app_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hyperbola");
}
