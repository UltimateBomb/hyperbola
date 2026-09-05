//! Turns yt-dlp's `--dump-single-json` output into [`MediaProbe`].
//!
//! yt-dlp's JSON is wide, versioned loosely and full of nulls: every field is
//! optional here on purpose. A missing field must degrade the UI, never fail
//! the probe — a title we cannot read still downloads fine.

use serde::Deserialize;

use crate::domain::{Format, MediaItem, MediaProbe, SubtitleTrack};
use crate::Error;

/// Parses the single JSON object yt-dlp prints for a URL.
pub fn parse_probe(source_url: &str, json: &str) -> Result<MediaProbe, Error> {
    // With --ignore-errors yt-dlp prints a bare `null` when it could not read
    // the URL at all. Reporting that as a deserialisation failure tells the
    // user nothing they can act on.
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Err(Error::Probe(
            "nothing to download at this link — the page has no media, or the site refused it"
                .to_string(),
        ));
    }
    let raw: RawInfo = serde_json::from_str(trimmed).map_err(|e| Error::Probe(e.to_string()))?;
    Ok(build_probe(source_url, raw))
}

fn build_probe(source_url: &str, raw: RawInfo) -> MediaProbe {
    let is_playlist = raw.kind.as_deref() == Some("playlist") || !raw.entries.is_empty();
    if is_playlist {
        let items = raw
            .entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let position = entry.playlist_index.unwrap_or(index as u32 + 1);
                build_item(source_url, entry, Some(position))
            })
            .collect();
        MediaProbe {
            source_url: source_url.to_string(),
            playlist_title: Some(raw.title.unwrap_or_else(|| "Playlist".to_string())),
            items,
        }
    } else {
        MediaProbe {
            source_url: source_url.to_string(),
            playlist_title: None,
            items: vec![build_item(source_url, raw, None)],
        }
    }
}

fn build_item(source_url: &str, raw: RawInfo, playlist_index: Option<u32>) -> MediaItem {
    let formats = raw.formats.into_iter().map(build_format).collect();
    let mut subtitles: Vec<SubtitleTrack> = raw
        .subtitles
        .into_iter()
        .map(|(language, tracks)| SubtitleTrack {
            name: tracks.into_iter().find_map(|t| t.name),
            language,
            is_automatic: false,
        })
        .collect();
    subtitles.extend(raw.automatic_captions.into_iter().map(|(language, tracks)| SubtitleTrack {
        name: tracks.into_iter().find_map(|t| t.name),
        language,
        is_automatic: true,
    }));
    subtitles.sort_by(|a, b| {
        a.is_automatic
            .cmp(&b.is_automatic)
            .then_with(|| a.language.cmp(&b.language))
    });

    MediaItem {
        url: raw
            .webpage_url
            .or(raw.original_url)
            .unwrap_or_else(|| source_url.to_string()),
        id: raw.id.unwrap_or_default(),
        title: raw.title.unwrap_or_else(|| "Untitled".to_string()),
        uploader: raw.uploader.or(raw.channel),
        duration_secs: raw.duration,
        thumbnail: raw.thumbnail,
        // `live_status` is authoritative when present; `is_live` is the older field.
        is_live: raw.live_status.as_deref() == Some("is_live") || raw.is_live.unwrap_or(false),
        formats,
        subtitles,
        playlist_index,
    }
}

fn build_format(raw: RawFormat) -> Format {
    let (filesize, is_estimate) = match (raw.filesize, raw.filesize_approx) {
        (Some(exact), _) => (Some(exact), false),
        (None, Some(approx)) => (Some(approx), true),
        (None, None) => (None, false),
    };
    Format {
        id: raw.format_id.unwrap_or_default(),
        ext: raw.ext.unwrap_or_else(|| "mp4".to_string()),
        height: raw.height,
        width: raw.width,
        fps: raw.fps,
        vcodec: raw.vcodec,
        acodec: raw.acodec,
        filesize,
        filesize_is_estimate: is_estimate,
        tbr: raw.tbr,
        note: raw.format_note,
        protocol: raw.protocol,
    }
}

/// The subset of yt-dlp's info JSON Hyperbola reads.
#[derive(Debug, Default, Deserialize)]
struct RawInfo {
    #[serde(rename = "_type")]
    kind: Option<String>,
    id: Option<String>,
    title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    webpage_url: Option<String>,
    original_url: Option<String>,
    is_live: Option<bool>,
    live_status: Option<String>,
    playlist_index: Option<u32>,
    #[serde(default, deserialize_with = "null_as_default")]
    formats: Vec<RawFormat>,
    #[serde(default, deserialize_with = "null_as_default")]
    entries: Vec<RawInfo>,
    #[serde(default, deserialize_with = "null_as_default")]
    subtitles: std::collections::BTreeMap<String, Vec<RawSubtitle>>,
    #[serde(default, deserialize_with = "null_as_default")]
    automatic_captions: std::collections::BTreeMap<String, Vec<RawSubtitle>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawFormat {
    format_id: Option<String>,
    ext: Option<String>,
    height: Option<u32>,
    width: Option<u32>,
    fps: Option<f64>,
    vcodec: Option<String>,
    acodec: Option<String>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    tbr: Option<f64>,
    format_note: Option<String>,
    protocol: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSubtitle {
    name: Option<String>,
}

/// yt-dlp writes `null` where it means "empty" for several list and map
/// fields; serde's `default` alone does not cover an explicit null.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINGLE_VIDEO: &str = r#"{
        "id": "dQw4w9WgXcQ",
        "title": "Never Gonna Give You Up",
        "uploader": "Rick Astley",
        "channel": "Rick Astley",
        "duration": 212.0,
        "thumbnail": "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxres.jpg",
        "webpage_url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        "live_status": "not_live",
        "subtitles": {"en": [{"ext": "vtt", "name": "English"}]},
        "automatic_captions": {"ru": [{"ext": "vtt", "name": "Russian (auto)"}]},
        "formats": [
            {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2",
             "filesize": 3438339, "tbr": 129.5, "protocol": "https"},
            {"format_id": "137", "ext": "mp4", "height": 1080, "width": 1920, "fps": 30,
             "vcodec": "avc1.640028", "acodec": "none", "filesize_approx": 148000000,
             "tbr": 4000.0, "format_note": "1080p", "protocol": "https"},
            {"format_id": "18", "ext": "mp4", "height": 360, "fps": 30,
             "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "filesize": 12000000,
             "tbr": 700.0, "protocol": "https"}
        ]
    }"#;

    const PLAYLIST: &str = r#"{
        "_type": "playlist",
        "id": "PL123",
        "title": "Album",
        "entries": [
            {"id": "a", "title": "Track A", "playlist_index": 1, "formats": [],
             "subtitles": null, "automatic_captions": null},
            {"id": "b", "title": "Track B", "formats": null}
        ]
    }"#;

    #[test]
    fn parses_a_single_video() {
        let probe = parse_probe("https://youtu.be/dQw4w9WgXcQ", SINGLE_VIDEO).unwrap();
        assert!(!probe.is_playlist());
        let item = &probe.items[0];
        assert_eq!(item.title, "Never Gonna Give You Up");
        assert_eq!(item.uploader.as_deref(), Some("Rick Astley"));
        assert_eq!(item.duration_secs, Some(212.0));
        assert!(!item.is_live);
        assert_eq!(item.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(item.formats.len(), 3);
        assert_eq!(item.resolutions(), vec![1080, 360]);
    }

    #[test]
    fn marks_approximate_sizes_as_estimates() {
        let probe = parse_probe("u", SINGLE_VIDEO).unwrap();
        let f = probe.items[0].formats.iter().find(|f| f.id == "137").unwrap();
        assert_eq!(f.filesize, Some(148_000_000));
        assert!(f.filesize_is_estimate);

        let exact = probe.items[0].formats.iter().find(|f| f.id == "18").unwrap();
        assert!(!exact.filesize_is_estimate);
    }

    #[test]
    fn lists_manual_subtitles_before_automatic_ones() {
        let probe = parse_probe("u", SINGLE_VIDEO).unwrap();
        let subs = &probe.items[0].subtitles;
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].language, "en");
        assert!(!subs[0].is_automatic);
        assert_eq!(subs[1].language, "ru");
        assert!(subs[1].is_automatic);
    }

    #[test]
    fn parses_a_playlist_and_numbers_entries() {
        let probe = parse_probe("https://example.com/list", PLAYLIST).unwrap();
        assert!(probe.is_playlist());
        assert_eq!(probe.playlist_title.as_deref(), Some("Album"));
        assert_eq!(probe.items.len(), 2);
        assert_eq!(probe.items[0].playlist_index, Some(1));
        // Entry without an explicit index falls back to its position.
        assert_eq!(probe.items[1].playlist_index, Some(2));
    }

    #[test]
    fn survives_nulls_and_missing_fields() {
        let probe = parse_probe("u", r#"{"id": "x", "formats": null, "subtitles": null}"#).unwrap();
        let item = &probe.items[0];
        assert_eq!(item.title, "Untitled");
        assert!(item.formats.is_empty());
        assert_eq!(item.url, "u");
    }

    #[test]
    fn detects_live_streams() {
        let probe = parse_probe("u", r#"{"id": "x", "live_status": "is_live"}"#).unwrap();
        assert!(probe.items[0].is_live);
    }

    #[test]
    fn reports_broken_json_as_a_probe_error() {
        let err = parse_probe("u", "not json").unwrap_err();
        assert!(matches!(err, Error::Probe(_)));
    }

    #[test]
    fn a_bare_null_reads_as_nothing_found_not_as_a_parser_failure() {
        // yt-dlp prints `null` under --ignore-errors when it could not read
        // the URL. The message must say that, not mention structs.
        for output in ["null", "null\n", "  ", ""] {
            let err = parse_probe("https://example.com/x", output).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("nothing to download"), "unhelpful message: {message}");
            assert!(!message.contains("RawInfo"), "leaked parser detail: {message}");
        }
    }
}
