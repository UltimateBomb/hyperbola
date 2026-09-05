//! Reads yt-dlp's stdout line by line and turns it into events.
//!
//! Screen-scraping yt-dlp's human progress bar is fragile, so Hyperbola asks
//! yt-dlp to print machine-readable lines instead (see
//! [`PROGRESS_TEMPLATE`]) and only falls back to text lines for the things
//! templates cannot express: the output path and postprocessor stages.

use std::path::PathBuf;

use crate::domain::{Progress, Stage};

/// Marker that opens every templated progress line. Deliberately unlikely to
/// appear in a video title.
pub const PROGRESS_MARKER: &str = "@HB@";

/// `--progress-template` for the download phase. Field order must match
/// [`parse_line`].
pub const PROGRESS_TEMPLATE: &str = concat!(
    "download:@HB@|%(progress.status)s|%(progress.downloaded_bytes)s",
    "|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s",
    "|%(progress.speed)s|%(progress.eta)s"
);

/// `--progress-template` for the postprocessing phase.
pub const POSTPROCESS_TEMPLATE: &str =
    "postprocess:@HB@PP|%(progress.status)s|%(progress.postprocessor)s";

/// One thing learned from a line of yt-dlp output.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A progress sample.
    Progress(Progress),
    /// The pipeline moved to a new stage.
    Stage(Stage),
    /// The file being written. Later events override earlier ones, so the
    /// last path seen is the finished file.
    Destination(PathBuf),
    /// yt-dlp found the file already complete on disk.
    AlreadyDownloaded(PathBuf),
    Warning(String),
    Error(String),
}

/// Parses one line of yt-dlp output. Returns `None` for lines Hyperbola does
/// not care about — most of yt-dlp's chatter.
pub fn parse_line(line: &str) -> Option<Event> {
    let line = line.trim_end_matches(['\r', '\n']);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix(&format!("{PROGRESS_MARKER}PP|")) {
        return parse_postprocess(rest);
    }
    if let Some(rest) = trimmed.strip_prefix(&format!("{PROGRESS_MARKER}|")) {
        return parse_progress(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("ERROR:") {
        return Some(Event::Error(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("WARNING:") {
        return Some(Event::Warning(rest.trim().to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("[download] Destination:") {
        return Some(Event::Destination(PathBuf::from(rest.trim())));
    }
    if let Some(rest) = trimmed.strip_prefix("[ExtractAudio] Destination:") {
        return Some(Event::Destination(PathBuf::from(rest.trim())));
    }
    if let Some(rest) = trimmed.strip_prefix("[Merger] Merging formats into") {
        let path = rest.trim().trim_matches('"');
        return Some(Event::Destination(PathBuf::from(path)));
    }
    if trimmed.starts_with("[download]") && trimmed.contains("has already been downloaded") {
        let path = trimmed
            .trim_start_matches("[download]")
            .trim()
            .trim_end_matches("has already been downloaded")
            .trim();
        return Some(Event::AlreadyDownloaded(PathBuf::from(path)));
    }
    None
}

fn parse_progress(fields: &str) -> Option<Event> {
    let mut parts = fields.split('|');
    let status = parts.next()?;
    let downloaded = parse_u64(parts.next());
    let total = parse_u64(parts.next());
    let total_estimate = parse_u64(parts.next());
    let speed = parse_f64(parts.next());
    let eta = parse_u64(parts.next());

    // "finished" arrives once per stream; the merge that follows is
    // postprocessing, so do not report it as a completed download here.
    let stage = match status {
        "downloading" | "finished" => Stage::Downloading,
        _ => Stage::Downloading,
    };
    Some(Event::Progress(Progress {
        stage,
        downloaded_bytes: downloaded.unwrap_or(0),
        total_bytes: total.or(total_estimate),
        speed_bps: speed,
        eta_secs: eta,
    }))
}

fn parse_postprocess(fields: &str) -> Option<Event> {
    let mut parts = fields.split('|');
    let status = parts.next()?;
    match status {
        "started" | "processing" => Some(Event::Stage(Stage::Postprocessing)),
        _ => None,
    }
}

/// yt-dlp prints `NA` for fields it has no value for, and floats for byte
/// counts often enough that integer parsing alone drops samples.
fn parse_u64(field: Option<&str>) -> Option<u64> {
    let raw = field?.trim();
    if raw.is_empty() || raw == "NA" || raw == "None" {
        return None;
    }
    raw.parse::<u64>()
        .ok()
        .or_else(|| raw.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64))
}

fn parse_f64(field: Option<&str>) -> Option<f64> {
    let raw = field?.trim();
    if raw.is_empty() || raw == "NA" || raw == "None" {
        return None;
    }
    raw.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_download_sample() {
        let line = "@HB@|downloading|1048576|10485760|NA|524288.0|18";
        match parse_line(line).unwrap() {
            Event::Progress(p) => {
                assert_eq!(p.stage, Stage::Downloading);
                assert_eq!(p.downloaded_bytes, 1_048_576);
                assert_eq!(p.total_bytes, Some(10_485_760));
                assert_eq!(p.speed_bps, Some(524_288.0));
                assert_eq!(p.eta_secs, Some(18));
                assert_eq!(p.fraction(), Some(0.1));
            }
            other => panic!("expected progress, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_the_estimated_total() {
        let line = "@HB@|downloading|500|NA|2000|1000.0|NA";
        let Event::Progress(p) = parse_line(line).unwrap() else { panic!("expected progress") };
        assert_eq!(p.total_bytes, Some(2000));
        assert_eq!(p.eta_secs, None);
    }

    #[test]
    fn accepts_float_byte_counts() {
        let line = "@HB@|downloading|1048576.0|10485760.0|NA|NA|NA";
        let Event::Progress(p) = parse_line(line).unwrap() else { panic!("expected progress") };
        assert_eq!(p.downloaded_bytes, 1_048_576);
        assert_eq!(p.total_bytes, Some(10_485_760));
        assert_eq!(p.speed_bps, None);
    }

    #[test]
    fn postprocessing_start_switches_stage() {
        assert_eq!(
            parse_line("@HB@PP|started|Merger").unwrap(),
            Event::Stage(Stage::Postprocessing)
        );
        assert_eq!(parse_line("@HB@PP|finished|Merger"), None);
    }

    #[test]
    fn reads_destination_paths() {
        assert_eq!(
            parse_line("[download] Destination: /home/u/Videos/clip.f137.mp4").unwrap(),
            Event::Destination(PathBuf::from("/home/u/Videos/clip.f137.mp4"))
        );
        assert_eq!(
            parse_line(r#"[Merger] Merging formats into "/home/u/Videos/clip.mp4""#).unwrap(),
            Event::Destination(PathBuf::from("/home/u/Videos/clip.mp4"))
        );
        assert_eq!(
            parse_line("[ExtractAudio] Destination: /home/u/Music/song.mp3").unwrap(),
            Event::Destination(PathBuf::from("/home/u/Music/song.mp3"))
        );
    }

    #[test]
    fn recognises_an_already_finished_file() {
        let line = "[download] /home/u/Videos/clip.mp4 has already been downloaded";
        assert_eq!(
            parse_line(line).unwrap(),
            Event::AlreadyDownloaded(PathBuf::from("/home/u/Videos/clip.mp4"))
        );
    }

    #[test]
    fn separates_errors_from_warnings() {
        assert_eq!(
            parse_line("ERROR: [youtube] abc: Video unavailable").unwrap(),
            Event::Error("[youtube] abc: Video unavailable".to_string())
        );
        assert_eq!(
            parse_line("WARNING: Falling back to generic extractor").unwrap(),
            Event::Warning("Falling back to generic extractor".to_string())
        );
    }

    #[test]
    fn ignores_noise() {
        assert_eq!(parse_line("[youtube] Extracting URL: https://…"), None);
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
    }
}
