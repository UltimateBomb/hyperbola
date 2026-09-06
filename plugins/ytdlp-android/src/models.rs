//! Payloads exchanged with the Kotlin side. Field names are what the Kotlin
//! `@InvokeArg` classes expect, so renaming one means renaming both.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    pub url: String,
    /// yt-dlp arguments, already built by the engine. The URL itself is not
    /// repeated here — youtubedl-android takes it separately.
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResponse {
    /// Raw `--dump-single-json` output, parsed by the engine.
    pub json: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    /// Process id, used to cancel and to collect output.
    pub id: String,
    pub url: String,
    pub args: Vec<String>,
    /// Directory the finished file is written to.
    pub output_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResponse {
    pub exit_code: i32,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessId {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputLines {
    /// Lines produced since the last poll, oldest first.
    pub lines: Vec<String>,
    pub finished: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    /// `stable` or `nightly`.
    pub channel: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineVersion {
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub status: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSelection {
    /// A SAF tree URI, or `null` when the user backed out.
    pub uri: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    /// File in the app's private directory that should be handed to the user.
    pub source_path: String,
    /// SAF tree URI chosen earlier, or `null` to use the system Downloads folder.
    pub tree_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    /// Where the file ended up, as shown to the user.
    pub display_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePaths {
    /// Directory holding `ffmpeg` and `ffprobe` under the names yt-dlp looks
    /// for, or `null` when they could not be made reachable.
    pub ffmpeg_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRequest {
    /// What the notification says while downloads run.
    pub text: String,
}
