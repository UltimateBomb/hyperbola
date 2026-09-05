//! Desktop stub. The desktop shell spawns yt-dlp itself, so nothing here does
//! any work — the type exists only so the app can be written once.

use serde::de::DeserializeOwned;
use tauri::plugin::PluginApi;
use tauri::{AppHandle, Runtime};

use crate::Result;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<Ytdlp<R>> {
    Ok(Ytdlp(app.clone()))
}

pub struct Ytdlp<R: Runtime>(AppHandle<R>);
