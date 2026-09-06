//! Runs yt-dlp as a child process.
//!
//! This is the only place in the desktop path that knows about processes. It
//! owns no state and draws nothing: it takes options, runs yt-dlp, and hands
//! back every event the engine's parser recognised. The Tauri app uses it to
//! drive the queue; the command-line harness uses it to prove the same
//! pipeline works without a window.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use hyperbola_core::args::{build_download_args, build_probe_args, RunnerEnv};
use hyperbola_core::domain::{CookieSource, DownloadOptions, MediaProbe};
use hyperbola_core::probe::parse_probe;
use hyperbola_core::progress::{parse_line, Event};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

pub use hyperbola_core::retry::is_retryable;

/// Windows only: keep child processes from flashing a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How a download ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub success: bool,
    /// The file yt-dlp last wrote, when it said.
    pub destination: Option<PathBuf>,
    /// The last error yt-dlp printed, when it failed.
    pub error: Option<String>,
    /// True when the caller cancelled the download.
    pub canceled: bool,
}

pub struct Runner {
    ytdlp: PathBuf,
    env: RunnerEnv,
}

impl Runner {
    pub fn new(ytdlp: impl Into<PathBuf>, env: RunnerEnv) -> Self {
        Runner { ytdlp: ytdlp.into(), env }
    }

    pub fn ytdlp_path(&self) -> &Path {
        &self.ytdlp
    }

    fn command(&self) -> Command {
        #[allow(unused_mut)]
        let mut command = Command::new(&self.ytdlp);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    /// Reads a URL's metadata without downloading anything.
    pub async fn probe(
        &self,
        url: &str,
        cookies: &CookieSource,
        proxy: Option<&str>,
    ) -> Result<MediaProbe, String> {
        let args = build_probe_args(url, cookies, proxy, &self.env);
        let output = self
            .command()
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("could not run yt-dlp: {e}"))?;

        if !output.status.success() && output.stdout.is_empty() {
            return Err(last_error_line(&String::from_utf8_lossy(&output.stderr)));
        }
        parse_probe(url, &String::from_utf8_lossy(&output.stdout)).map_err(|e| e.to_string())
    }

    /// Runs a download, calling `on_event` for everything yt-dlp reports.
    ///
    /// `cancel` kills the process when it fires; the partial file is left on
    /// disk so the next attempt continues instead of starting over.
    pub async fn download<F>(
        &self,
        options: &DownloadOptions,
        mut on_event: F,
        cancel: Option<oneshot::Receiver<()>>,
    ) -> Result<Outcome, String>
    where
        F: FnMut(Event),
    {
        let args = build_download_args(options, &self.env);
        let mut child = self
            .command()
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run yt-dlp: {e}"))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdout_lines = BufReader::new(stdout).lines();
        let mut stderr_lines = BufReader::new(stderr).lines();

        let mut destination = None;
        let mut error = None;
        // A receiver that never fires stands in for "no cancellation".
        let (_keepalive, idle) = oneshot::channel::<()>();
        let mut cancel = cancel.unwrap_or(idle);

        loop {
            tokio::select! {
                _ = &mut cancel => {
                    let _ = child.kill().await;
                    return Ok(Outcome { success: false, destination, error, canceled: true });
                }
                line = stdout_lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(event) = parse_line(&line) {
                                record(&event, &mut destination, &mut error);
                                on_event(event);
                            }
                        }
                        _ => break,
                    }
                }
                line = stderr_lines.next_line() => {
                    if let Ok(Some(line)) = line {
                        if let Some(event) = parse_line(&line) {
                            record(&event, &mut destination, &mut error);
                            on_event(event);
                        }
                    }
                }
            }
        }

        // Drain whatever is left on stderr so the reported failure is the real one.
        while let Ok(Some(line)) = stderr_lines.next_line().await {
            if let Some(event) = parse_line(&line) {
                record(&event, &mut destination, &mut error);
                on_event(event);
            }
        }

        let status = child.wait().await.map_err(|e| e.to_string())?;
        // With errors ignored so a postprocessor cannot discard a finished
        // file, the exit code alone can no longer be trusted: the proof of a
        // download is a file on disk.
        let produced_file = destination.is_some();
        if !status.success() && error.is_none() {
            error = Some(format!("yt-dlp exited with {status}"));
        }
        if status.success() && !produced_file && error.is_none() {
            error = Some("yt-dlp finished without writing a file".to_string());
        }
        Ok(Outcome {
            success: status.success() && produced_file,
            destination,
            error,
            canceled: false,
        })
    }
}

fn record(event: &Event, destination: &mut Option<PathBuf>, error: &mut Option<String>) {
    match event {
        Event::Destination(path) | Event::AlreadyDownloaded(path) => {
            *destination = Some(path.clone())
        }
        Event::Error(message) => *error = Some(message.clone()),
        _ => {}
    }
}

fn last_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find(|line| line.starts_with("ERROR:"))
        .map(|line| line.trim_start_matches("ERROR:").trim().to_string())
        .or_else(|| stderr.lines().last().map(|l| l.trim().to_string()))
        .unwrap_or_else(|| "yt-dlp failed".to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_error_line_wins() {
        let stderr = "WARNING: something\nERROR: [youtube] abc: Video unavailable\n";
        assert_eq!(last_error_line(stderr), "[youtube] abc: Video unavailable");
    }

    #[test]
    fn output_without_an_error_line_still_reports_something() {
        assert_eq!(last_error_line("could not resolve host\n"), "could not resolve host");
        assert_eq!(last_error_line(""), "yt-dlp failed");
    }
}
