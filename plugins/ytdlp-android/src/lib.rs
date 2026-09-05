//! Runs yt-dlp on Android.
//!
//! Android will not execute a downloaded binary, so the desktop approach —
//! fetch `yt-dlp.exe` and spawn it — cannot work there. This plugin wraps
//! [youtubedl-android], which ships yt-dlp, Python, ffmpeg and QuickJS as
//! native libraries inside the APK, and exposes it to Rust with the same
//! shape the desktop runner uses: build arguments, run, read lines back.
//!
//! [youtubedl-android]: https://github.com/yausername/youtubedl-android

mod error;
mod models;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};
pub use models::*;

#[cfg(desktop)]
pub use desktop::Ytdlp;
#[cfg(mobile)]
pub use mobile::Ytdlp;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

pub trait YtdlpExt<R: Runtime> {
    fn ytdlp(&self) -> &Ytdlp<R>;
}

impl<R: Runtime, T: Manager<R>> YtdlpExt<R> for T {
    fn ytdlp(&self) -> &Ytdlp<R> {
        self.state::<Ytdlp<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("ytdlp")
        .setup(|app, api| {
            #[cfg(mobile)]
            let ytdlp = mobile::init(app, api)?;
            #[cfg(desktop)]
            let ytdlp = desktop::init(app, api)?;
            app.manage(ytdlp);
            Ok(())
        })
        .build()
}
