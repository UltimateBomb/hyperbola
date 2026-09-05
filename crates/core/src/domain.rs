//! The vocabulary every layer shares: what a download is, what state it is
//! in, and what the user asked for. Nothing here touches the filesystem, the
//! network or a process — the shells (Windows, Android) map these types onto
//! their own platform primitives.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable identifier for one queued download, unique within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DownloadId(pub u64);

impl fmt::Display for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What the user wants out of a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Video plus audio, merged.
    Video,
    /// Audio only, extracted and re-encoded when the container demands it.
    Audio,
}

/// Container the finished file should end up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Webm,
    Mkv,
    Mp3,
    Opus,
    Flac,
    Wav,
    M4a,
    /// Whatever yt-dlp picked; no remux, no re-encode. Fastest and lossless.
    Source,
}

impl Container {
    /// The `--merge-output-format` / `--audio-format` value, or `None` when
    /// the source container should be kept as-is.
    pub fn ytdlp_name(self) -> Option<&'static str> {
        match self {
            Container::Mp4 => Some("mp4"),
            Container::Webm => Some("webm"),
            Container::Mkv => Some("mkv"),
            Container::Mp3 => Some("mp3"),
            Container::Opus => Some("opus"),
            Container::Flac => Some("flac"),
            Container::Wav => Some("wav"),
            Container::M4a => Some("m4a"),
            Container::Source => None,
        }
    }

    pub fn kind(self) -> MediaKind {
        match self {
            Container::Mp3 | Container::Opus | Container::Flac | Container::Wav | Container::M4a => {
                MediaKind::Audio
            }
            _ => MediaKind::Video,
        }
    }
}

/// One selectable stream reported by yt-dlp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Format {
    pub id: String,
    pub ext: String,
    /// `None` for audio-only streams.
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    /// Exact size when the server reported one, otherwise yt-dlp's estimate.
    pub filesize: Option<u64>,
    /// True when `filesize` is an estimate rather than a reported value.
    pub filesize_is_estimate: bool,
    /// Total bitrate in kbit/s.
    pub tbr: Option<f64>,
    pub note: Option<String>,
    pub protocol: Option<String>,
}

impl Format {
    pub fn has_video(&self) -> bool {
        !matches!(self.vcodec.as_deref(), None | Some("none") | Some(""))
    }

    pub fn has_audio(&self) -> bool {
        !matches!(self.acodec.as_deref(), None | Some("none") | Some(""))
    }

    /// A short human label: `1080p60 · mp4 · avc1 · 142 MB`.
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match (self.height, self.fps) {
            (Some(h), Some(fps)) if fps >= 50.0 => parts.push(format!("{h}p{}", fps.round() as u32)),
            (Some(h), _) => parts.push(format!("{h}p")),
            (None, _) => parts.push("audio".to_string()),
        }
        parts.push(self.ext.clone());
        if let Some(codec) = self.vcodec.as_deref().filter(|c| *c != "none" && !c.is_empty()) {
            parts.push(codec.split('.').next().unwrap_or(codec).to_string());
        } else if let Some(codec) = self.acodec.as_deref().filter(|c| *c != "none" && !c.is_empty()) {
            parts.push(codec.split('.').next().unwrap_or(codec).to_string());
        }
        if let Some(size) = self.filesize {
            parts.push(format!(
                "{}{}",
                if self.filesize_is_estimate { "~" } else { "" },
                human_bytes(size)
            ));
        }
        parts.join(" · ")
    }
}

/// A subtitle track offered for a media item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleTrack {
    pub language: String,
    pub name: Option<String>,
    pub is_automatic: bool,
}

/// One media item: a single video, or one entry of a playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    pub url: String,
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_secs: Option<f64>,
    pub thumbnail: Option<String>,
    pub is_live: bool,
    pub formats: Vec<Format>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Playlist index, when this item came from a playlist.
    pub playlist_index: Option<u32>,
}

impl MediaItem {
    /// Best video format by resolution, then bitrate. `None` for audio-only URLs.
    pub fn best_video(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.has_video())
            .max_by(|a, b| {
                a.height
                    .unwrap_or(0)
                    .cmp(&b.height.unwrap_or(0))
                    .then_with(|| a.tbr.unwrap_or(0.0).total_cmp(&b.tbr.unwrap_or(0.0)))
            })
    }

    /// Distinct video heights offered, highest first — what the resolution
    /// picker shows.
    pub fn resolutions(&self) -> Vec<u32> {
        let mut heights: Vec<u32> = self
            .formats
            .iter()
            .filter(|f| f.has_video())
            .filter_map(|f| f.height)
            .collect();
        heights.sort_unstable_by(|a, b| b.cmp(a));
        heights.dedup();
        heights
    }
}

/// The result of probing a URL: one item, or a playlist of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaProbe {
    pub source_url: String,
    /// Playlist title, when the URL resolved to a playlist.
    pub playlist_title: Option<String>,
    pub items: Vec<MediaItem>,
}

impl MediaProbe {
    pub fn is_playlist(&self) -> bool {
        self.playlist_title.is_some() || self.items.len() > 1
    }
}

/// Cut a slice out of the media instead of downloading all of it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeFrame {
    pub start_secs: f64,
    pub end_secs: f64,
}

impl TimeFrame {
    pub fn duration_secs(&self) -> f64 {
        (self.end_secs - self.start_secs).max(0.0)
    }
}

/// Where cookies come from, when a site needs a logged-in session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source", content = "value")]
pub enum CookieSource {
    None,
    /// Read live cookies from an installed browser profile (desktop only).
    Browser(String),
    /// A Netscape-format cookies file the user exported.
    File(PathBuf),
}

impl Default for CookieSource {
    fn default() -> Self {
        CookieSource::None
    }
}

/// Everything the user chose for one download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadOptions {
    pub url: String,
    pub kind: MediaKind,
    pub container: Container,
    /// Explicit yt-dlp format id, when the user picked a specific stream.
    pub format_id: Option<String>,
    /// Cap by height (1080, 720, …) when no explicit format is chosen.
    pub max_height: Option<u32>,
    pub output_dir: PathBuf,
    /// Filename without extension; `None` keeps yt-dlp's title-based name.
    pub filename: Option<String>,
    pub subtitle_languages: Vec<String>,
    pub embed_subtitles: bool,
    pub embed_metadata: bool,
    pub embed_thumbnail: bool,
    pub embed_chapters: bool,
    /// Strip sponsor segments via SponsorBlock.
    pub remove_sponsor_segments: bool,
    pub time_frame: Option<TimeFrame>,
    /// Bytes per second; `None` means unlimited.
    pub speed_limit: Option<u64>,
    pub cookies: CookieSource,
    pub proxy: Option<String>,
    /// Extra raw yt-dlp arguments, for users who know what they want.
    pub extra_args: Vec<String>,
}

impl DownloadOptions {
    /// A sane default: best video up to the source quality, into `output_dir`.
    pub fn video(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        DownloadOptions {
            url: url.into(),
            kind: MediaKind::Video,
            container: Container::Mp4,
            format_id: None,
            max_height: None,
            output_dir: output_dir.into(),
            filename: None,
            subtitle_languages: Vec::new(),
            embed_subtitles: false,
            embed_metadata: true,
            embed_thumbnail: true,
            embed_chapters: true,
            remove_sponsor_segments: false,
            time_frame: None,
            speed_limit: None,
            cookies: CookieSource::None,
            proxy: None,
            extra_args: Vec::new(),
        }
    }

    /// Audio-only variant of [`DownloadOptions::video`].
    pub fn audio(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        DownloadOptions {
            kind: MediaKind::Audio,
            container: Container::Mp3,
            embed_thumbnail: true,
            ..DownloadOptions::video(url, output_dir)
        }
    }
}

/// Which phase of the pipeline a running download is in. yt-dlp reports these
/// through its progress hooks and postprocessor lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Reading metadata and the format list.
    Probing,
    /// Transferring bytes.
    Downloading,
    /// Merging video and audio, extracting audio, embedding subtitles.
    Postprocessing,
}

/// A progress sample for a running download.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub stage: Stage,
    pub downloaded_bytes: u64,
    /// Total size; `None` while the server has not reported one.
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<f64>,
    pub eta_secs: Option<u64>,
}

impl Progress {
    /// Completion in `0.0..=1.0`, or `None` when the total is unknown.
    pub fn fraction(&self) -> Option<f64> {
        match self.total_bytes {
            Some(total) if total > 0 => {
                Some((self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            stage: Stage::Probing,
            downloaded_bytes: 0,
            total_bytes: None,
            speed_bps: None,
            eta_secs: None,
        }
    }
}

/// Lifecycle of one download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DownloadState {
    /// Waiting for a free slot.
    Queued,
    Running(Progress),
    /// Stopped by the user; the partial file is kept so it can resume.
    Paused(Progress),
    Completed { path: PathBuf },
    Failed { message: String, retryable: bool },
    Canceled,
}

impl DownloadState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DownloadState::Completed { .. } | DownloadState::Failed { .. } | DownloadState::Canceled
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(self, DownloadState::Running(_))
    }
}

/// One entry in the queue: what was asked for, and where it got to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub id: DownloadId,
    pub options: DownloadOptions,
    pub title: String,
    pub thumbnail: Option<String>,
    pub state: DownloadState,
    /// How many times this download has been retried after a failure.
    pub attempts: u32,
    /// Free-form fields the shells attach (notification ids, SAF uris, …).
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Formats a byte count the way the UI shows it: `1.4 GB`, `142 MB`, `800 KB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(id: &str, height: Option<u32>, vcodec: &str, acodec: &str) -> Format {
        Format {
            id: id.to_string(),
            ext: "mp4".to_string(),
            height,
            width: None,
            fps: Some(30.0),
            vcodec: Some(vcodec.to_string()),
            acodec: Some(acodec.to_string()),
            filesize: Some(148_000_000),
            filesize_is_estimate: false,
            tbr: Some(2500.0),
            note: None,
            protocol: Some("https".to_string()),
        }
    }

    #[test]
    fn video_and_audio_presence_reads_codec_none() {
        let video_only = format("137", Some(1080), "avc1.640028", "none");
        assert!(video_only.has_video());
        assert!(!video_only.has_audio());

        let audio_only = format("140", None, "none", "mp4a.40.2");
        assert!(!audio_only.has_video());
        assert!(audio_only.has_audio());
    }

    #[test]
    fn best_video_prefers_height_then_bitrate() {
        let mut low = format("18", Some(360), "avc1", "mp4a");
        low.tbr = Some(700.0);
        let mut high = format("137", Some(1080), "avc1", "none");
        high.tbr = Some(2500.0);
        let mut high_but_thin = format("136", Some(1080), "avc1", "none");
        high_but_thin.tbr = Some(1200.0);

        let item = MediaItem {
            url: "u".into(),
            id: "id".into(),
            title: "t".into(),
            uploader: None,
            duration_secs: None,
            thumbnail: None,
            is_live: false,
            formats: vec![low, high_but_thin, high.clone()],
            subtitles: vec![],
            playlist_index: None,
        };
        assert_eq!(item.best_video().unwrap().id, "137");
        assert_eq!(item.resolutions(), vec![1080, 360]);
    }

    #[test]
    fn format_label_is_readable() {
        let mut f = format("137", Some(1080), "avc1.640028", "none");
        f.fps = Some(60.0);
        assert_eq!(f.label(), "1080p60 · mp4 · avc1 · 141 MB");
    }

    #[test]
    fn estimated_sizes_are_marked() {
        let mut f = format("137", Some(720), "avc1", "none");
        f.filesize_is_estimate = true;
        f.filesize = Some(10_000_000);
        assert!(f.label().contains("~9.5 MB"));
    }

    #[test]
    fn audio_containers_imply_audio_kind() {
        assert_eq!(Container::Mp3.kind(), MediaKind::Audio);
        assert_eq!(Container::Mkv.kind(), MediaKind::Video);
        assert_eq!(Container::Source.ytdlp_name(), None);
    }

    #[test]
    fn progress_fraction_needs_a_total() {
        let mut p = Progress { stage: Stage::Downloading, downloaded_bytes: 50, ..Default::default() };
        assert_eq!(p.fraction(), None);
        p.total_bytes = Some(200);
        assert_eq!(p.fraction(), Some(0.25));
    }

    #[test]
    fn human_bytes_matches_ui_expectations() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(148_000_000), "141 MB");
        assert_eq!(human_bytes(3_000_000_000), "2.8 GB");
    }
}
