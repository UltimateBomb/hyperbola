//! Version comparison for every component Hyperbola keeps up to date.
//!
//! Four independent things carry versions: the app (semver, `0.1.0`), yt-dlp
//! (date-based, `2025.11.12`, nightlies add a fourth segment), ffmpeg
//! (`7.1.1`, sometimes with a vendor suffix) and the Android engine. One rule
//! covers all of them: compare numeric segments left to right, a missing
//! segment counts as zero, and a pre-release suffix sorts before the plain
//! release it belongs to.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A parsed component version.
#[derive(Debug, Clone)]
pub struct Version {
    raw: String,
    segments: Vec<u64>,
    /// Everything after the numeric part: `-beta.1`, `-Debian-6ubuntu1`, …
    suffix: Option<String>,
}

impl Version {
    /// Parses a version string. Leading `v` and surrounding whitespace are
    /// ignored, so `v2025.11.12` and `2025.11.12` are the same version.
    ///
    /// Never fails: a string with no leading number parses to the zero
    /// version carrying the whole string as its suffix, which sorts below
    /// every real release. That keeps an unreadable `--version` output from
    /// blocking an update check.
    pub fn parse(input: &str) -> Version {
        let raw = input.trim().to_string();
        let body = raw.strip_prefix('v').or_else(|| raw.strip_prefix('V')).unwrap_or(&raw);
        let numeric_end = body
            .char_indices()
            .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
            .map(|(i, _)| i)
            .unwrap_or(body.len());
        let (numeric, rest) = body.split_at(numeric_end);
        let segments: Vec<u64> = numeric
            .split('.')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();
        let suffix = {
            let trimmed = rest.trim_start_matches(['-', '_', '+', '.']).trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
        };
        Version { raw, segments, suffix }
    }

    /// The string as it was reported by the component, for display.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// True when this version is strictly older than `other`.
    pub fn is_older_than(&self, other: &Version) -> bool {
        self < other
    }

    fn segment(&self, index: usize) -> u64 {
        self.segments.get(index).copied().unwrap_or(0)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.segments.len().max(other.segments.len());
        for i in 0..width {
            match self.segment(i).cmp(&other.segment(i)) {
                Ordering::Equal => continue,
                non_equal => return non_equal,
            }
        }
        match (&self.suffix, &other.suffix) {
            (None, None) => Ordering::Equal,
            // A plain release outranks any suffixed build of the same numbers.
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for Version {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Version::parse(s))
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Version::parse(&raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_leading_v_and_whitespace() {
        assert_eq!(Version::parse(" v1.2.0 "), Version::parse("1.2.0"));
    }

    #[test]
    fn compares_ytdlp_date_versions() {
        assert!(Version::parse("2025.11.12") < Version::parse("2026.03.17"));
        assert!(Version::parse("2026.03.17") < Version::parse("2026.03.18"));
    }

    #[test]
    fn nightly_outranks_the_stable_it_builds_on() {
        // yt-dlp nightlies append a build segment to the stable version.
        assert!(Version::parse("2025.11.12") < Version::parse("2025.11.12.232351"));
    }

    #[test]
    fn missing_segments_count_as_zero() {
        assert_eq!(Version::parse("7.1"), Version::parse("7.1.0"));
        assert!(Version::parse("7.1") < Version::parse("7.1.1"));
    }

    #[test]
    fn prerelease_sorts_below_its_release() {
        assert!(Version::parse("1.2.0-beta.1") < Version::parse("1.2.0"));
        assert!(Version::parse("1.2.0-beta.1") < Version::parse("1.2.0-beta.2"));
    }

    #[test]
    fn vendor_suffix_does_not_make_ffmpeg_look_newer() {
        // Debian/Ubuntu ffmpeg reports e.g. "7.1.1-1ubuntu1".
        assert!(Version::parse("7.1.1-1ubuntu1") < Version::parse("7.1.1"));
        assert!(Version::parse("7.1.1-1ubuntu1") < Version::parse("7.2"));
    }

    #[test]
    fn unparsable_input_sorts_below_everything() {
        let unknown = Version::parse("unknown");
        assert!(unknown < Version::parse("0.0.1"));
        assert_eq!(unknown.as_str(), "unknown");
    }

    #[test]
    fn round_trips_through_json() {
        let v = Version::parse("2026.03.17");
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"2026.03.17\"");
        assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), v);
    }
}
