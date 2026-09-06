//! The Android side of the plugin: a thin typed wrapper over the Kotlin
//! implementation. Every call blocks until Kotlin resolves it, so long ones
//! belong on a blocking task.

use serde::de::DeserializeOwned;
use tauri::plugin::{PluginApi, PluginHandle};
use tauri::{AppHandle, Runtime};

use crate::models::*;
use crate::Result;

/// Must match the Kotlin package name.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "app.hyperbola.ytdlp";

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<Ytdlp<R>> {
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "YtdlpPlugin")?;
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_ytdlp)?;
    Ok(Ytdlp(handle))
}

pub struct Ytdlp<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> Ytdlp<R> {
    /// Reads a URL's metadata. Returns yt-dlp's JSON for the engine to parse.
    pub fn probe(&self, payload: ProbeRequest) -> Result<ProbeResponse> {
        self.0.run_mobile_plugin("probe", payload).map_err(Into::into)
    }

    /// Runs a download to completion. Blocks; poll [`Ytdlp::poll_output`] from
    /// another task to follow progress while it runs.
    pub fn download(&self, payload: DownloadRequest) -> Result<DownloadResponse> {
        self.0.run_mobile_plugin("download", payload).map_err(Into::into)
    }

    /// Collects the output lines produced since the previous call.
    pub fn poll_output(&self, id: &str) -> Result<OutputLines> {
        self.0
            .run_mobile_plugin("pollOutput", ProcessId { id: id.to_string() })
            .map_err(Into::into)
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        self.0
            .run_mobile_plugin::<serde_json::Value>("cancel", ProcessId { id: id.to_string() })
            .map(|_| ())
            .map_err(Into::into)
    }

    /// Where ffmpeg can be found under the name yt-dlp expects.
    pub fn engine_paths(&self) -> Result<EnginePaths> {
        self.0.run_mobile_plugin("enginePaths", ()).map_err(Into::into)
    }

    pub fn engine_version(&self) -> Result<EngineVersion> {
        self.0
            .run_mobile_plugin("engineVersion", ())
            .map_err(Into::into)
    }

    /// Updates the bundled yt-dlp. This is the Android equivalent of
    /// downloading a new yt-dlp binary on the desktop.
    pub fn update_engine(&self, channel: &str) -> Result<UpdateResult> {
        self.0
            .run_mobile_plugin("updateEngine", UpdateRequest { channel: channel.to_string() })
            .map_err(Into::into)
    }

    /// Asks the user for a folder to keep downloads in (Storage Access
    /// Framework). Remembered by the system across restarts.
    pub fn pick_output_folder(&self) -> Result<FolderSelection> {
        self.0
            .run_mobile_plugin("pickOutputFolder", ())
            .map_err(Into::into)
    }

    /// Moves a finished file out of the app's private directory into the
    /// user's chosen folder, or into Downloads when none was chosen.
    pub fn publish(&self, payload: PublishRequest) -> Result<PublishResult> {
        self.0.run_mobile_plugin("publish", payload).map_err(Into::into)
    }
}
