//! User settings, persisted as JSON next to the app's config.

use std::path::{Path, PathBuf};

use hyperbola_core::domain::{Container, CookieSource};
use hyperbola_core::updates::Channel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub max_concurrent: usize,
    pub concurrent_fragments: u8,
    pub video_container: Container,
    pub audio_container: Container,
    pub embed_metadata: bool,
    pub embed_thumbnail: bool,
    pub embed_chapters: bool,
    pub subtitle_languages: Vec<String>,
    pub embed_subtitles: bool,
    pub remove_sponsor_segments: bool,
    /// Kilobytes per second; `None` means unlimited.
    pub speed_limit_kbps: Option<u64>,
    pub cookies: CookieSource,
    pub proxy: Option<String>,
    /// Which yt-dlp stream to follow. Nightly carries extractor fixes days
    /// before stable, which matters when a site breaks.
    pub ytdlp_channel: Channel,
    pub auto_check_updates: bool,
    /// Install dependency updates without asking. The app itself always asks.
    pub auto_install_dependency_updates: bool,
    pub watch_clipboard: bool,
    /// Prefer H.264 and AAC, which play on any phone, over AV1 and Opus,
    /// which are smaller and do not.
    pub prefer_compatible: bool,
    /// Android only: the folder the user granted access to, as a SAF tree
    /// URI. Finished files are moved there; without it they go to Downloads.
    pub android_tree_uri: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            download_dir: dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from(".")),
            max_concurrent: 3,
            concurrent_fragments: 4,
            video_container: Container::Mp4,
            audio_container: Container::Mp3,
            embed_metadata: true,
            embed_thumbnail: true,
            embed_chapters: true,
            subtitle_languages: Vec::new(),
            embed_subtitles: false,
            remove_sponsor_segments: false,
            speed_limit_kbps: None,
            cookies: CookieSource::None,
            proxy: None,
            ytdlp_channel: Channel::Stable,
            auto_check_updates: true,
            auto_install_dependency_updates: true,
            watch_clipboard: true,
            prefer_compatible: true,
            android_tree_uri: None,
        }
    }
}

impl Settings {
    /// Reads settings, falling back to defaults for a missing or unreadable
    /// file — a corrupt config must not stop the app from starting.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
    }

    /// Speed limit in bytes per second, as yt-dlp wants it.
    pub fn speed_limit_bps(&self) -> Option<u64> {
        self.speed_limit_kbps.filter(|v| *v > 0).map(|v| v * 1024)
    }
}
