//! Hyperbola's engine.
//!
//! Everything in this crate is platform-independent and free of I/O: it
//! builds yt-dlp command lines, reads yt-dlp's output, runs the download
//! queue and decides what needs updating. Spawning processes, writing files
//! and drawing windows belong to the shells — the Windows app and the Android
//! app — which are the only places that differ between platforms.

#![forbid(unsafe_code)]

pub mod args;
pub mod domain;
pub mod probe;
pub mod progress;
pub mod queue;
pub mod updates;
pub mod version;

pub use args::{build_download_args, build_probe_args, RunnerEnv};
pub use domain::{
    Container, CookieSource, Download, DownloadId, DownloadOptions, DownloadState, Format,
    MediaItem, MediaKind, MediaProbe, Progress, Stage, SubtitleTrack, TimeFrame,
};
pub use probe::parse_probe;
pub use progress::{parse_line, Event};
pub use queue::{FailureOutcome, Queue, QueueStats};
pub use updates::{Channel, Component, ComponentStatus, UpdateReport, UpdateState};
pub use version::Version;

/// Everything that can go wrong inside the engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// yt-dlp's metadata output could not be read.
    #[error("could not read media info: {0}")]
    Probe(String),
    /// A release feed could not be read.
    #[error("could not check for updates: {0}")]
    Update(String),
    /// yt-dlp itself reported a failure.
    #[error("yt-dlp failed: {0}")]
    Ytdlp(String),
}

/// The crate version, reported by the update center as the installed app
/// version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
