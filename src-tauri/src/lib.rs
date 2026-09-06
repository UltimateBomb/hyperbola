//! The desktop shell: state, commands, and the loop that keeps the queue moving.

mod deps;
mod runner;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// Where the queue is kept between runs, so closing the app mid-download
    /// loses nothing.
    pub queue_path: PathBuf,
    pub temp_dir: PathBuf,
    /// Where downloads are written while they run. Android needs this to be
    /// app-private; the desktop writes straight into the user's folder.
    pub staging_dir: PathBuf,
    /// Android only: directory where the bundled ffmpeg is reachable under
    /// the name yt-dlp looks for. Empty until the engine reports it.
    pub engine_ffmpeg_dir: Mutex<Option<PathBuf>>,
    /// Last time the queue was written; progress alone must not hammer the disk.
    pub last_save: Mutex<Instant>,
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
    /// `staging` is where yt-dlp is allowed to write. On the desktop that is
    /// the user's chosen folder; on Android it is the app's private directory,
    /// because that is the only place a plain path can point to — the file is
    /// moved to the user's folder once it is finished.
    fn into_options(self, settings: &Settings, staging: &PathBuf) -> (DownloadOptions, String) {
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
            output_dir: staging.clone(),
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

/// How often the queue is written to disk while downloads are running.
const SAVE_INTERVAL: Duration = Duration::from_secs(2);

/// Pushes the current queue to the window, and writes it to disk now and then
/// so a crash or a power cut costs at most a couple of seconds of progress.
pub fn emit_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let queue = state.queue.lock().unwrap();
        Snapshot { items: queue.items().to_vec(), stats: queue.stats() }
    };
    let due = {
        let mut last = state.last_save.lock().unwrap();
        let due = last.elapsed() >= SAVE_INTERVAL;
        if due {
            *last = Instant::now();
        }
        due
    };
    if due {
        write_queue(&state.queue_path, &snapshot.items);
    }
    let _ = app.emit("queue-changed", snapshot);
}

/// Writes the queue immediately, for changes worth not losing: a download
/// added, finished, cancelled or removed.
pub fn save_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let items = { state.queue.lock().unwrap().items().to_vec() };
    *state.last_save.lock().unwrap() = Instant::now();
    write_queue(&state.queue_path, &items);
}

fn write_queue(path: &PathBuf, items: &[Download]) {
    if let Ok(text) = serde_json::to_string(items) {
        let _ = std::fs::write(path, text);
    }
}

fn read_queue(path: &PathBuf) -> Vec<Download> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
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
        #[cfg(target_os = "android")]
        let staging = state.staging_dir.clone();
        #[cfg(not(target_os = "android"))]
        let staging = settings.download_dir.clone();
        let mut queue = state.queue.lock().unwrap();
        queue.set_max_concurrent(settings.max_concurrent);
        requests
            .into_iter()
            .map(|request| {
                let (options, title) = request.into_options(&settings, &staging);
                queue.add(options, title)
            })
            .collect()
    };
    save_queue(&app);
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
    save_queue(&app);
    pump(app);
}

#[tauri::command]
fn resume_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().resume(id);
    }
    save_queue(&app);
    pump(app);
}

#[tauri::command]
fn cancel_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().cancel(id);
    }
    signal_cancel(&app, id);
    save_queue(&app);
    pump(app);
}

#[tauri::command]
fn retry_download(app: AppHandle, id: DownloadId) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().retry(id);
    }
    save_queue(&app);
    pump(app);
}

#[tauri::command]
fn remove_download(app: AppHandle, id: DownloadId) {
    signal_cancel(&app, id);
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().remove(id);
    }
    save_queue(&app);
    pump(app);
}

#[tauri::command]
fn clear_finished(app: AppHandle) {
    {
        let state = app.state::<AppState>();
        state.queue.lock().unwrap().clear_finished();
    }
    save_queue(&app);
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
    let report = build_report(&app).await;
    let _ = app.emit("updates-changed", report.clone());
    report
}

/// The update picture for this platform.
///
/// The desktop keeps its own binaries and knows their versions from disk.
/// Android has no binaries to keep: yt-dlp, Python and ffmpeg are inside the
/// APK, so the engine reports its own version and updates itself in place.
/// The release feed is the same in both cases — it is the same yt-dlp.
async fn build_report(app: &AppHandle) -> UpdateReport {
    let channel = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        settings.ytdlp_channel
    };

    #[cfg(not(target_os = "android"))]
    {
        let state = app.state::<AppState>();
        state.deps.check(channel, env!("CARGO_PKG_VERSION")).await
    }

    #[cfg(target_os = "android")]
    {
        use hyperbola_core::updates::{evaluate, ComponentStatus, UpdateState};
        use hyperbola_core::version::Version;
        use tauri_plugin_ytdlp::YtdlpExt;

        let handle = app.clone();
        let installed = tauri::async_runtime::spawn_blocking(move || handle.ytdlp().engine_version())
            .await
            .ok()
            .and_then(|result| result.ok())
            .and_then(|v| v.version)
            .map(|v| Version::parse(&v));

        let latest = {
            let state = app.state::<AppState>();
            state.deps.latest_ytdlp_version(channel).await
        };
        let ytdlp = match latest {
            Ok(latest) => evaluate(Component::YtDlp, installed, Some(latest), None),
            Err(reason) => evaluate(Component::YtDlp, installed, None, Some(reason)),
        };

        // ffmpeg ships inside the APK and moves with the app, so there is
        // nothing here for the user to install or keep current.
        let ffmpeg = ComponentStatus {
            component: Component::FFmpeg,
            installed: Some(Version::parse("bundled")),
            latest: None,
            state: UpdateState::UpToDate,
        };

        let app_status = {
            let state = app.state::<AppState>();
            match state.deps.latest_app_version().await {
                Ok(latest) => evaluate(
                    Component::App,
                    Some(Version::parse(env!("CARGO_PKG_VERSION"))),
                    Some(latest),
                    None,
                ),
                Err(reason) => evaluate(
                    Component::App,
                    Some(Version::parse(env!("CARGO_PKG_VERSION"))),
                    None,
                    Some(reason),
                ),
            }
        };

        UpdateReport::new(vec![ytdlp, ffmpeg, app_status])
    }
}

/// A dependency that could not be installed, pushed to the window so a
/// failure during the automatic startup install is visible rather than silent.
#[derive(Debug, Clone, Serialize)]
struct DependencyError {
    component: Component,
    message: String,
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
    if component == Component::App {
        return install_app_update(app, &progress).await;
    }

    #[cfg(target_os = "android")]
    {
        use tauri_plugin_ytdlp::YtdlpExt;
        if component == Component::FFmpeg {
            return Err("ffmpeg is part of the app on Android and updates with it".to_string());
        }
        let channel_name = match channel {
            hyperbola_core::updates::Channel::Nightly => "nightly",
            hyperbola_core::updates::Channel::Stable => "stable",
        };
        let handle = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            handle.ytdlp().update_engine(channel_name)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        log::info!("engine update: {} {:?}", result.status, result.version);
        let _ = check_updates(app).await;
        return Ok(result.version.unwrap_or(result.status));
    }

    #[cfg(not(target_os = "android"))]
    let installed = {
        let state = app.state::<AppState>();
        state.deps.install(component, channel, &progress).await
    };
    #[cfg(not(target_os = "android"))]
    let version = match installed {
        Ok(version) => version,
        Err(message) => {
            log::error!("installing {} failed: {message}", component.display_name());
            let _ = app.emit(
                "dependency-error",
                DependencyError { component, message: message.clone() },
            );
            return Err(message);
        }
    };
    log::info!("installed {} {version}", component.display_name());
    let _ = check_updates(app).await;
    Ok(version.as_str().to_string())
}

/// Updating the app itself is not a file swap: the running program cannot
/// replace its own files on Windows, so the installer is downloaded, started,
/// and the app steps aside for it.
async fn install_app_update(
    app: AppHandle,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<String, String> {
    let installer = {
        let state = app.state::<AppState>();
        let into = state.temp_dir.join("updates");
        state.deps.download_app_installer(&into, progress).await?
    };
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(&installer)
            .spawn()
            .map_err(|e| format!("could not start the installer: {e}"))?;
        let handle = app.clone();
        // Give the installer a moment to appear before the window disappears.
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            handle.exit(0);
        });
    }
    Ok(installer.display().to_string())
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The window needs to know which shell it is running in: the folder picker,
/// and what "where files go" means, differ between the two.
#[tauri::command]
fn app_platform() -> &'static str {
    std::env::consts::OS
}

/// Android's folder picker. The system remembers the grant, so this is asked
/// once and the answer survives restarts.
#[tauri::command]
async fn pick_output_folder(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_ytdlp::YtdlpExt;
        let handle = app.clone();
        let selection = tauri::async_runtime::spawn_blocking(move || handle.ytdlp().pick_output_folder())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let Some(uri) = selection.uri.clone() else {
            return Ok(None);
        };
        let label = selection.label.clone().unwrap_or_else(|| "Selected folder".to_string());
        {
            let state = app.state::<AppState>();
            let mut settings = state.settings.lock().unwrap();
            settings.android_tree_uri = Some(uri);
            settings.download_dir = PathBuf::from(&label);
            let _ = settings.save(&state.config_path);
        }
        Ok(Some(label))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("this platform uses the system file dialog".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A log file is the difference between "the update did not happen"
        // and knowing why. It lives next to the app's data.
        .plugin(
            tauri_plugin_log::Builder::new()
                // targets() replaces the defaults; adding to them would write
                // every line to the log file twice.
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("hyperbola".into()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_ytdlp::init())
        .setup(|app| {
            let handle = app.handle();
            let config_dir = handle.path().app_config_dir()?;
            let data_dir = handle.path().app_data_dir()?;
            let temp_dir = handle.path().app_cache_dir()?.join("partials");
            let staging_dir = data_dir.join("downloads");
            std::fs::create_dir_all(&config_dir).ok();
            std::fs::create_dir_all(&temp_dir).ok();
            std::fs::create_dir_all(&staging_dir).ok();

            let config_path = config_dir.join("settings.json");
            let queue_path = config_dir.join("queue.json");
            let settings = Settings::load(&config_path);
            let mut queue = Queue::new(settings.max_concurrent);
            queue.restore(read_queue(&queue_path));

            app.manage(AppState {
                queue: Mutex::new(queue),
                settings: Mutex::new(settings),
                deps: Dependencies::new(data_dir.join("bin")),
                cancels: Mutex::new(HashMap::new()),
                config_path,
                queue_path,
                staging_dir,
                engine_ffmpeg_dir: Mutex::new(None),
                temp_dir,
                last_save: Mutex::new(Instant::now()),
            });

            // Android: find out where the bundled ffmpeg is before anything
            // is queued, or the first merge fails after a full download.
            #[cfg(target_os = "android")]
            {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_ytdlp::YtdlpExt;
                    let engine = handle.clone();
                    let paths = tauri::async_runtime::spawn_blocking(move || engine.ytdlp().engine_paths())
                        .await;
                    match paths {
                        Ok(Ok(paths)) => {
                            log::info!("engine ffmpeg dir: {:?}", paths.ffmpeg_dir);
                            if let Some(dir) = paths.ffmpeg_dir {
                                let state = handle.state::<AppState>();
                                *state.engine_ffmpeg_dir.lock().unwrap() = Some(PathBuf::from(dir));
                            }
                        }
                        Ok(Err(e)) => log::error!("could not locate the bundled ffmpeg: {e}"),
                        Err(e) => log::error!("could not locate the bundled ffmpeg: {e}"),
                    }
                });
            }

            // First run, or a dependency the user deleted: fetch what is
            // missing before the user hits a confusing failure.
            let startup = handle.clone();
            tauri::async_runtime::spawn(async move {
                let (auto_check, auto_install) = {
                    let state = startup.state::<AppState>();
                    let settings = state.settings.lock().unwrap();
                    (
                        settings.auto_check_updates,
                        settings.auto_install_dependency_updates,
                    )
                };
                if !auto_check {
                    return;
                }
                let report = build_report(&startup).await;
                log::info!("update check: {}", report.summary());
                let _ = startup.emit("updates-changed", report.clone());
                if !auto_install {
                    return;
                }
                for status in report.actionable() {
                    if status.component == Component::App {
                        continue;
                    }
                    let component = status.component;
                    log::info!("installing {} automatically", component.display_name());
                    if let Err(message) = install_update(startup.clone(), component).await {
                        // Silence here used to mean the user saw a working app
                        // that could not merge a file, with nothing to read.
                        log::error!("could not install {}: {message}", component.display_name());
                        let _ = startup.emit(
                            "dependency-error",
                            DependencyError { component, message },
                        );
                    }
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
            app_platform,
            pick_output_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hyperbola");
}
