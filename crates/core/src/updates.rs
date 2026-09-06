//! One update center for everything that can go out of date: the app itself,
//! yt-dlp and ffmpeg.
//!
//! Both donor projects update these separately and inconsistently — Parabolic
//! keeps yt-dlp current but ships an ffmpeg that is frozen at install time,
//! Open Video Downloader updates ffmpeg but only silently at startup. Here
//! every component reports the same [`ComponentStatus`], so the UI can show
//! one list and one "update everything" button.
//!
//! This module is pure: it decides *what* needs updating and *which asset* to
//! fetch. The actual HTTP and file replacement happen in the shell, which
//! knows about the platform's file locking and permissions.

use serde::{Deserialize, Serialize};

use crate::version::Version;
use crate::Error;

/// A thing Hyperbola keeps up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    /// Hyperbola itself.
    App,
    /// The extractor — the component that actually goes stale, sometimes
    /// within days, when a site changes.
    YtDlp,
    /// Muxing, audio extraction and cutting.
    // Without this the derived name is "f_fmpeg", which no caller expects and
    // which left the component nameless in the update panel.
    #[serde(rename = "ffmpeg")]
    FFmpeg,
}

impl Component {
    pub fn display_name(self) -> &'static str {
        match self {
            Component::App => "Hyperbola",
            Component::YtDlp => "yt-dlp",
            Component::FFmpeg => "ffmpeg",
        }
    }

    /// Whether a stale version of this component breaks downloads outright.
    /// The UI nags about yt-dlp; it merely mentions the rest.
    pub fn is_critical(self) -> bool {
        matches!(self, Component::YtDlp)
    }
}

/// Which release stream to follow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    #[default]
    Stable,
    /// yt-dlp nightlies carry extractor fixes days before a stable release.
    Nightly,
}

/// What the app knows about one component right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub component: Component,
    /// `None` when the component is not installed at all.
    pub installed: Option<Version>,
    /// `None` when the last check failed or has not run.
    pub latest: Option<Version>,
    pub state: UpdateState,
}

/// The verdict for one component.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum UpdateState {
    UpToDate,
    UpdateAvailable {
        from: Version,
        to: Version,
    },
    /// Not installed yet — a first run, or a dependency the user deleted.
    Missing {
        to: Version,
    },
    /// The check itself failed; the app keeps working with what it has.
    Unknown {
        reason: String,
    },
}

impl UpdateState {
    /// True when there is something to install.
    pub fn is_actionable(&self) -> bool {
        matches!(
            self,
            UpdateState::UpdateAvailable { .. } | UpdateState::Missing { .. }
        )
    }
}

/// Decides the state of one component. `installed` is what is on disk,
/// `latest` is what the release feed offered (`None` when the check failed).
pub fn evaluate(
    component: Component,
    installed: Option<Version>,
    latest: Option<Version>,
    failure: Option<String>,
) -> ComponentStatus {
    let state = match (&installed, &latest) {
        (_, None) => UpdateState::Unknown {
            reason: failure.unwrap_or_else(|| "update check did not run".to_string()),
        },
        (None, Some(remote)) => UpdateState::Missing { to: remote.clone() },
        (Some(local), Some(remote)) if local.is_older_than(remote) => {
            UpdateState::UpdateAvailable {
                from: local.clone(),
                to: remote.clone(),
            }
        }
        (Some(_), Some(_)) => UpdateState::UpToDate,
    };
    ComponentStatus {
        component,
        installed,
        latest,
        state,
    }
}

/// The whole update picture, as the update center shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateReport {
    pub components: Vec<ComponentStatus>,
}

impl UpdateReport {
    pub fn new(components: Vec<ComponentStatus>) -> Self {
        UpdateReport { components }
    }

    /// Components with something to install, critical ones first — the order
    /// the "update everything" button works through.
    pub fn actionable(&self) -> Vec<&ComponentStatus> {
        let mut items: Vec<&ComponentStatus> = self
            .components
            .iter()
            .filter(|c| c.state.is_actionable())
            .collect();
        items.sort_by_key(|c| (!c.component.is_critical(), c.component));
        items
    }

    pub fn has_updates(&self) -> bool {
        self.components.iter().any(|c| c.state.is_actionable())
    }

    /// True when a component that downloads cannot work without is missing.
    pub fn is_blocked(&self) -> bool {
        self.components
            .iter()
            .any(|c| c.component.is_critical() && matches!(c.state, UpdateState::Missing { .. }))
    }

    pub fn status_of(&self, component: Component) -> Option<&ComponentStatus> {
        self.components.iter().find(|c| c.component == component)
    }

    /// One line for the UI badge.
    ///
    /// A check that failed must not read as health: "everything is up to
    /// date" is only true when everything was actually checked.
    pub fn summary(&self) -> String {
        let actionable = self.actionable();
        if !actionable.is_empty() {
            let names: Vec<&str> = actionable
                .iter()
                .map(|c| c.component.display_name())
                .collect();
            return format!("Update available: {}", names.join(", "));
        }
        let unchecked: Vec<&str> = self
            .components
            .iter()
            .filter(|c| matches!(c.state, UpdateState::Unknown { .. }))
            .map(|c| c.component.display_name())
            .collect();
        if !unchecked.is_empty() {
            return format!("Could not check: {}", unchecked.join(", "));
        }
        "Everything is up to date".to_string()
    }
}

/// One downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A release as published by a GitHub repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub prerelease: bool,
    pub assets: Vec<ReleaseAsset>,
    pub notes: Option<String>,
    /// ISO-8601 publication time. Rolling releases — ffmpeg's nightly builds,
    /// for instance — reuse one tag forever, so the date is the only thing
    /// that tells two of them apart.
    pub published_at: Option<String>,
}

impl Release {
    /// First asset whose name contains every one of `must_contain` and none of
    /// `must_not_contain`, matched case-insensitively.
    pub fn find_asset(
        &self,
        must_contain: &[&str],
        must_not_contain: &[&str],
    ) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|asset| {
            let name = asset.name.to_ascii_lowercase();
            must_contain
                .iter()
                .all(|needle| name.contains(&needle.to_ascii_lowercase()))
                && must_not_contain
                    .iter()
                    .all(|needle| !name.contains(&needle.to_ascii_lowercase()))
        })
    }
}

/// Turns an ISO-8601 timestamp (`2026-09-05T12:30:00Z`) into a comparable
/// version (`2026.09.05`). Used for release feeds that publish under one
/// rolling tag, where the date is the only version there is.
pub fn version_from_publish_date(published_at: &str) -> Option<Version> {
    let date = published_at.split('T').next()?;
    let mut parts = date.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    Some(Version::parse(&format!("{y}.{m}.{d}")))
}

/// Parses the JSON array returned by `GET /repos/{owner}/{repo}/releases`.
pub fn parse_releases(json: &str) -> Result<Vec<Release>, Error> {
    let raw: Vec<RawRelease> =
        serde_json::from_str(json).map_err(|e| Error::Update(e.to_string()))?;
    Ok(raw
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| Release {
            version: Version::parse(r.tag_name.as_deref().unwrap_or_default()),
            tag: r.tag_name.unwrap_or_default(),
            prerelease: r.prerelease,
            notes: r.body,
            published_at: r.published_at,
            assets: r
                .assets
                .into_iter()
                .map(|a| ReleaseAsset {
                    name: a.name.unwrap_or_default(),
                    url: a.browser_download_url.unwrap_or_default(),
                    size: a.size.unwrap_or(0),
                })
                .collect(),
        })
        .collect())
}

/// Newest release on `channel`. Stable ignores prereleases; nightly takes the
/// newest release of either kind, because yt-dlp publishes nightlies as
/// ordinary releases in a separate repository.
pub fn latest_release(releases: &[Release], channel: Channel) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| match channel {
            Channel::Stable => !r.prerelease,
            Channel::Nightly => true,
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

#[derive(Debug, Deserialize)]
struct RawRelease {
    tag_name: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    body: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Debug, Deserialize)]
struct RawAsset {
    name: Option<String>,
    browser_download_url: Option<String>,
    size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s)
    }

    #[test]
    fn components_serialise_under_the_names_the_ui_uses() {
        assert_eq!(
            serde_json::to_string(&Component::FFmpeg).unwrap(),
            "\"ffmpeg\""
        );
        assert_eq!(
            serde_json::to_string(&Component::YtDlp).unwrap(),
            "\"yt_dlp\""
        );
        assert_eq!(serde_json::to_string(&Component::App).unwrap(), "\"app\"");
        assert_eq!(
            serde_json::from_str::<Component>("\"ffmpeg\"").unwrap(),
            Component::FFmpeg
        );
    }

    #[test]
    fn an_older_install_is_an_update() {
        let status = evaluate(
            Component::YtDlp,
            Some(v("2025.11.12")),
            Some(v("2026.03.17")),
            None,
        );
        assert_eq!(
            status.state,
            UpdateState::UpdateAvailable {
                from: v("2025.11.12"),
                to: v("2026.03.17")
            }
        );
        assert!(status.state.is_actionable());
    }

    #[test]
    fn a_newer_install_is_not_a_downgrade_prompt() {
        // Happens with nightlies: local build is ahead of the stable feed.
        let status = evaluate(
            Component::YtDlp,
            Some(v("2026.04.01")),
            Some(v("2026.03.17")),
            None,
        );
        assert_eq!(status.state, UpdateState::UpToDate);
    }

    #[test]
    fn a_missing_dependency_is_reported_separately_from_an_update() {
        let status = evaluate(Component::FFmpeg, None, Some(v("7.1.1")), None);
        assert_eq!(status.state, UpdateState::Missing { to: v("7.1.1") });
    }

    #[test]
    fn a_failed_check_never_looks_up_to_date() {
        let status = evaluate(
            Component::YtDlp,
            Some(v("2026.03.17")),
            None,
            Some("network unreachable".into()),
        );
        match status.state {
            UpdateState::Unknown { reason } => assert_eq!(reason, "network unreachable"),
            other => panic!("expected unknown, got {other:?}"),
        }
    }

    #[test]
    fn report_orders_critical_components_first() {
        let report = UpdateReport::new(vec![
            evaluate(Component::App, Some(v("0.1.0")), Some(v("0.2.0")), None),
            evaluate(Component::FFmpeg, Some(v("7.1")), Some(v("7.1.1")), None),
            evaluate(
                Component::YtDlp,
                Some(v("2025.11.12")),
                Some(v("2026.03.17")),
                None,
            ),
        ]);
        let names: Vec<&str> = report
            .actionable()
            .iter()
            .map(|c| c.component.display_name())
            .collect();
        assert_eq!(names, vec!["yt-dlp", "Hyperbola", "ffmpeg"]);
        assert!(report.has_updates());
        assert_eq!(
            report.summary(),
            "Update available: yt-dlp, Hyperbola, ffmpeg"
        );
    }

    #[test]
    fn report_is_blocked_only_when_a_critical_component_is_absent() {
        let missing_ffmpeg = UpdateReport::new(vec![
            evaluate(
                Component::YtDlp,
                Some(v("2026.03.17")),
                Some(v("2026.03.17")),
                None,
            ),
            evaluate(Component::FFmpeg, None, Some(v("7.1.1")), None),
        ]);
        assert!(!missing_ffmpeg.is_blocked());

        let missing_ytdlp = UpdateReport::new(vec![evaluate(
            Component::YtDlp,
            None,
            Some(v("2026.03.17")),
            None,
        )]);
        assert!(missing_ytdlp.is_blocked());
    }

    #[test]
    fn a_failed_check_does_not_read_as_health() {
        let report = UpdateReport::new(vec![
            evaluate(
                Component::YtDlp,
                Some(v("2026.03.17")),
                Some(v("2026.03.17")),
                None,
            ),
            evaluate(
                Component::App,
                Some(v("0.1.0")),
                None,
                Some("offline".into()),
            ),
        ]);
        assert!(!report.has_updates());
        assert_eq!(report.summary(), "Could not check: Hyperbola");
    }

    #[test]
    fn quiet_report_says_so() {
        let report = UpdateReport::new(vec![evaluate(
            Component::YtDlp,
            Some(v("2026.03.17")),
            Some(v("2026.03.17")),
            None,
        )]);
        assert!(!report.has_updates());
        assert_eq!(report.summary(), "Everything is up to date");
    }

    const RELEASES_JSON: &str = r#"[
        {"tag_name": "2026.03.17", "prerelease": false, "draft": false, "body": "stable",
         "published_at": "2026-03-17T09:15:00Z",
         "assets": [
            {"name": "yt-dlp.exe", "browser_download_url": "https://x/yt-dlp.exe", "size": 17000000},
            {"name": "yt-dlp_arm64.exe", "browser_download_url": "https://x/arm.exe", "size": 16000000},
            {"name": "yt-dlp_linux", "browser_download_url": "https://x/linux", "size": 3000000}
         ]},
        {"tag_name": "2026.04.01.120000", "prerelease": true, "draft": false, "body": "nightly",
         "assets": [{"name": "yt-dlp.exe", "browser_download_url": "https://x/n.exe", "size": 17000001}]},
        {"tag_name": "2026.05.01", "prerelease": false, "draft": true,
         "assets": []},
        {"tag_name": "2025.11.12", "prerelease": false, "draft": false,
         "assets": []}
    ]"#;

    #[test]
    fn parses_releases_and_drops_drafts() {
        let releases = parse_releases(RELEASES_JSON).unwrap();
        assert_eq!(releases.len(), 3);
        assert!(!releases.iter().any(|r| r.tag == "2026.05.01"));
    }

    #[test]
    fn stable_channel_ignores_prereleases() {
        let releases = parse_releases(RELEASES_JSON).unwrap();
        assert_eq!(
            latest_release(&releases, Channel::Stable).unwrap().tag,
            "2026.03.17"
        );
        assert_eq!(
            latest_release(&releases, Channel::Nightly).unwrap().tag,
            "2026.04.01.120000"
        );
    }

    #[test]
    fn asset_matching_distinguishes_architectures() {
        let releases = parse_releases(RELEASES_JSON).unwrap();
        let stable = latest_release(&releases, Channel::Stable).unwrap();
        assert_eq!(
            stable.find_asset(&["yt-dlp.exe"], &[]).unwrap().url,
            "https://x/yt-dlp.exe"
        );
        assert_eq!(
            stable.find_asset(&["arm64", ".exe"], &[]).unwrap().url,
            "https://x/arm.exe"
        );
        // The plain x64 build must not be matched by an arm64 request, and
        // vice versa.
        assert_eq!(
            stable.find_asset(&[".exe"], &["arm64"]).unwrap().name,
            "yt-dlp.exe"
        );
        assert!(stable.find_asset(&["macos"], &[]).is_none());
    }

    #[test]
    fn publication_date_becomes_a_version_for_rolling_tags() {
        let releases = parse_releases(RELEASES_JSON).unwrap();
        let stable = latest_release(&releases, Channel::Stable).unwrap();
        let published = stable.published_at.as_deref().unwrap();
        assert_eq!(
            version_from_publish_date(published).unwrap(),
            v("2026.03.17")
        );
        assert!(version_from_publish_date("garbage").is_none());
        assert!(version_from_publish_date("2026-3-5T00:00:00Z").is_none());
    }

    #[test]
    fn broken_feed_is_an_update_error() {
        assert!(matches!(
            parse_releases("{}").unwrap_err(),
            Error::Update(_)
        ));
    }
}
