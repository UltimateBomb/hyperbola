//! Finds, versions, and updates the binaries Hyperbola drives.
//!
//! Two ways a dependency can be present: *managed* (Hyperbola downloaded it
//! into its own `bin` directory and knows exactly which build it is) or
//! *system* (already on `PATH`). Managed binaries are kept current; system
//! ones are used as they are and never touched — replacing a binary the user
//! installed themselves is not the app's business.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures_util::StreamExt;
use hyperbola_core::updates::{
    evaluate, latest_release, parse_releases, version_from_publish_date, Channel, Component,
    ComponentStatus, Release, UpdateReport, UpdateState,
};
use hyperbola_core::version::Version;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

pub const APP_REPO: (&str, &str) = ("UltimateBomb", "hyperbola");
pub const YTDLP_STABLE_REPO: (&str, &str) = ("yt-dlp", "yt-dlp");
pub const YTDLP_NIGHTLY_REPO: (&str, &str) = ("yt-dlp", "yt-dlp-nightly-builds");
pub const FFMPEG_REPO: (&str, &str) = ("BtbN", "FFmpeg-Builds");

const USER_AGENT: &str = concat!("Hyperbola/", env!("CARGO_PKG_VERSION"));

/// Versions of the binaries Hyperbola installed itself.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct InstalledMarkers {
    #[serde(rename = "yt-dlp")]
    ytdlp: Option<String>,
    ffmpeg: Option<String>,
}

pub struct Dependencies {
    bin_dir: PathBuf,
    http: reqwest::Client,
}

impl Dependencies {
    pub fn new(bin_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&bin_dir);
        Dependencies {
            bin_dir,
            http: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    fn managed_path(&self, stem: &str) -> PathBuf {
        self.bin_dir.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX))
    }

    fn markers_path(&self) -> PathBuf {
        self.bin_dir.join("installed.json")
    }

    fn markers(&self) -> InstalledMarkers {
        std::fs::read_to_string(self.markers_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write_marker(&self, component: Component, version: &Version) {
        let mut markers = self.markers();
        match component {
            Component::YtDlp => markers.ytdlp = Some(version.as_str().to_string()),
            Component::FFmpeg => markers.ffmpeg = Some(version.as_str().to_string()),
            Component::App => return,
        }
        if let Ok(text) = serde_json::to_string_pretty(&markers) {
            let _ = std::fs::write(self.markers_path(), text);
        }
    }

    /// Managed yt-dlp if present, otherwise one found on `PATH`.
    pub fn ytdlp_path(&self) -> Option<PathBuf> {
        let managed = self.managed_path("yt-dlp");
        if managed.is_file() {
            return Some(managed);
        }
        which("yt-dlp")
    }

    pub fn ffmpeg_path(&self) -> Option<PathBuf> {
        let managed = self.managed_path("ffmpeg");
        if managed.is_file() {
            return Some(managed);
        }
        which("ffmpeg")
    }

    fn ffmpeg_is_managed(&self) -> bool {
        self.managed_path("ffmpeg").is_file()
    }

    pub async fn ytdlp_version(&self) -> Option<Version> {
        let path = self.ytdlp_path()?;
        let out = run_capture(&path, &["--version"]).await?;
        Some(Version::parse(out.trim()))
    }

    /// Reads ffmpeg's own reported version, e.g. `ffmpeg version 7.1.1-full`.
    pub async fn ffmpeg_version(&self) -> Option<Version> {
        let path = self.ffmpeg_path()?;
        let out = run_capture(&path, &["-version"]).await?;
        let first = out.lines().next()?;
        let token = first.split_whitespace().nth(2)?;
        Some(Version::parse(token))
    }

    async fn releases(&self, repo: (&str, &str)) -> Result<Vec<Release>, String> {
        let url = format!("https://api.github.com/repos/{}/{}/releases?per_page=15", repo.0, repo.1);
        let response = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("{} returned {}", repo.1, response.status()));
        }
        let body = response.text().await.map_err(|e| e.to_string())?;
        parse_releases(&body).map_err(|e| e.to_string())
    }

    fn ytdlp_repo(channel: Channel) -> (&'static str, &'static str) {
        match channel {
            Channel::Stable => YTDLP_STABLE_REPO,
            Channel::Nightly => YTDLP_NIGHTLY_REPO,
        }
    }

    /// Newest yt-dlp version on `channel`, straight from the release feed.
    /// Android reuses this: the engine there is the same yt-dlp, only
    /// installed through the bundled Python instead of as a binary.
    pub async fn latest_ytdlp_version(&self, channel: Channel) -> Result<Version, String> {
        let releases = self.releases(Self::ytdlp_repo(channel)).await?;
        latest_release(&releases, channel)
            .map(|r| r.version.clone())
            .ok_or_else(|| "no yt-dlp release found".to_string())
    }

    /// Newest published version of Hyperbola itself.
    pub async fn latest_app_version(&self) -> Result<Version, String> {
        let releases = self.releases(APP_REPO).await?;
        latest_release(&releases, Channel::Stable)
            .map(|r| r.version.clone())
            .ok_or_else(|| "no release found".to_string())
    }

    /// Checks every component and returns one report for the update center.
    pub async fn check(&self, channel: Channel, app_version: &str) -> UpdateReport {
        let installed_ytdlp = self.ytdlp_version().await;
        let installed_ffmpeg = self.ffmpeg_version().await;

        // yt-dlp
        let ytdlp_status = match self.latest_ytdlp_version(channel).await {
            Ok(latest) => evaluate(Component::YtDlp, installed_ytdlp.clone(), Some(latest), None),
            Err(reason) => evaluate(Component::YtDlp, installed_ytdlp.clone(), None, Some(reason)),
        };

        // ffmpeg: only builds Hyperbola installed itself are tracked for updates.
        let ffmpeg_status = if !self.ffmpeg_is_managed() {
            match installed_ffmpeg.clone() {
                Some(version) => ComponentStatus {
                    component: Component::FFmpeg,
                    installed: Some(version),
                    latest: None,
                    // A system ffmpeg is the user's to manage; saying
                    // "up to date" here means "nothing for Hyperbola to do".
                    state: UpdateState::UpToDate,
                },
                None => match self.latest_ffmpeg_build().await {
                    Ok(Some(latest)) => evaluate(Component::FFmpeg, None, Some(latest), None),
                    Ok(None) => evaluate(
                        Component::FFmpeg,
                        None,
                        None,
                        Some("no ffmpeg build for this platform".into()),
                    ),
                    Err(reason) => evaluate(Component::FFmpeg, None, None, Some(reason)),
                },
            }
        } else {
            let installed = Version::parse(self.markers().ffmpeg.as_deref().unwrap_or("0"));
            match self.latest_ffmpeg_build().await {
                Ok(Some(latest)) => evaluate(Component::FFmpeg, Some(installed), Some(latest), None),
                Ok(None) => evaluate(Component::FFmpeg, Some(installed), None, Some("no build".into())),
                Err(reason) => evaluate(Component::FFmpeg, Some(installed), None, Some(reason)),
            }
        };

        // Hyperbola itself
        let app_status = match self.latest_app_version().await {
            Ok(latest) => evaluate(Component::App, Some(Version::parse(app_version)), Some(latest), None),
            Err(reason) => evaluate(
                Component::App,
                Some(Version::parse(app_version)),
                None,
                Some(reason),
            ),
        };

        UpdateReport::new(vec![ytdlp_status, ffmpeg_status, app_status])
    }

    /// ffmpeg builds are published under one rolling tag, so the publication
    /// date is the version.
    async fn latest_ffmpeg_build(&self) -> Result<Option<Version>, String> {
        if ffmpeg_asset_patterns().is_none() {
            return Ok(None);
        }
        let releases = self.releases(FFMPEG_REPO).await?;
        Ok(releases
            .iter()
            .filter_map(|r| r.published_at.as_deref().and_then(version_from_publish_date))
            .max())
    }

    /// Downloads and installs one component. `progress` is called with
    /// (downloaded, total) as bytes arrive.
    pub async fn install(
        &self,
        component: Component,
        channel: Channel,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<Version, String> {
        match component {
            Component::YtDlp => self.install_ytdlp(channel, progress).await,
            Component::FFmpeg => self.install_ffmpeg(progress).await,
            Component::App => Err("the app installs its own updates".into()),
        }
    }

    async fn install_ytdlp(
        &self,
        channel: Channel,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<Version, String> {
        let releases = self.releases(Self::ytdlp_repo(channel)).await?;
        let release = latest_release(&releases, channel).ok_or("no yt-dlp release found")?;
        let (must, must_not) = ytdlp_asset_patterns();
        let asset = release
            .find_asset(&must, &must_not)
            .ok_or("no yt-dlp build for this platform")?;

        let target = self.managed_path("yt-dlp");
        let temp = target.with_extension("part");
        self.download(&asset.url, &temp, progress).await?;
        make_executable(&temp)?;
        std::fs::rename(&temp, &target).map_err(|e| e.to_string())?;
        self.write_marker(Component::YtDlp, &release.version);
        Ok(release.version.clone())
    }

    async fn install_ffmpeg(
        &self,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<Version, String> {
        let (must, must_not) = ffmpeg_asset_patterns().ok_or("no ffmpeg build for this platform")?;
        let releases = self.releases(FFMPEG_REPO).await?;
        let release = releases
            .iter()
            .filter(|r| r.find_asset(&must, &must_not).is_some())
            .max_by_key(|r| r.published_at.clone())
            .ok_or("no ffmpeg build for this platform")?;
        let asset = release.find_asset(&must, &must_not).expect("filtered above");
        let version = release
            .published_at
            .as_deref()
            .and_then(version_from_publish_date)
            .unwrap_or_else(|| Version::parse("0"));

        let archive = self.bin_dir.join("ffmpeg-download.zip");
        self.download(&asset.url, &archive, progress).await?;
        let extracted = extract_ffmpeg(&archive, &self.bin_dir)?;
        let _ = std::fs::remove_file(&archive);
        if extracted.is_empty() {
            return Err("archive contained no ffmpeg binary".into());
        }
        self.write_marker(Component::FFmpeg, &version);
        Ok(version)
    }

    /// Fetches the installer for the newest release of Hyperbola itself and
    /// returns where it landed. Running it is the shell's business: on
    /// Windows the app has to exit before its own files can be replaced.
    pub async fn download_app_installer(
        &self,
        into: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<PathBuf, String> {
        let (must, must_not) = app_asset_patterns().ok_or("no installer for this platform")?;
        let releases = self.releases(APP_REPO).await?;
        let release = latest_release(&releases, Channel::Stable).ok_or("no release found")?;
        let asset = release
            .find_asset(&must, &must_not)
            .ok_or("this release has no build for this platform")?;
        let _ = std::fs::create_dir_all(into);
        let target = into.join(&asset.name);
        self.download(&asset.url, &target, progress).await?;
        Ok(target)
    }

    async fn download(
        &self,
        url: &str,
        target: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<(), String> {
        let response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("download returned {}", response.status()));
        }
        let total = response.content_length();
        let mut file = tokio::fs::File::create(target).await.map_err(|e| e.to_string())?;
        let mut downloaded = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            downloaded += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            progress(downloaded, total);
        }
        file.flush().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Asset name patterns for yt-dlp on this platform: (must contain, must not contain).
fn ytdlp_asset_patterns() -> (Vec<&'static str>, Vec<&'static str>) {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            (vec!["yt-dlp_arm64.exe"], vec![])
        } else {
            (vec!["yt-dlp.exe"], vec!["arm64"])
        }
    } else if cfg!(target_os = "macos") {
        (vec!["yt-dlp_macos"], vec!["legacy", ".zip"])
    } else if cfg!(target_arch = "aarch64") {
        (vec!["yt-dlp_linux_aarch64"], vec![])
    } else {
        (vec!["yt-dlp_linux"], vec!["aarch64", "armv7"])
    }
}

/// How Hyperbola's own release assets are named per platform.
fn app_asset_patterns() -> Option<(Vec<&'static str>, Vec<&'static str>)> {
    if cfg!(target_os = "windows") {
        Some((vec!["setup.exe"], vec![]))
    } else if cfg!(target_os = "android") {
        Some((vec![".apk"], vec![]))
    } else {
        None
    }
}

/// ffmpeg builds exist for Windows and Linux; macOS users bring their own.
fn ffmpeg_asset_patterns() -> Option<(Vec<&'static str>, Vec<&'static str>)> {
    if cfg!(target_os = "windows") {
        Some((vec!["win64-gpl", ".zip"], vec!["shared"]))
    } else if cfg!(target_os = "linux") {
        Some((vec!["linux64-gpl", ".tar.xz"], vec!["shared"]))
    } else {
        None
    }
}

/// Pulls `ffmpeg` and `ffprobe` out of a BtbN archive into `bin_dir`.
fn extract_ffmpeg(archive: &Path, bin_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let wanted = [
        format!("ffmpeg{}", std::env::consts::EXE_SUFFIX),
        format!("ffprobe{}", std::env::consts::EXE_SUFFIX),
    ];
    let mut written = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().rsplit('/').next().unwrap_or("").to_string();
        if !wanted.contains(&name) {
            continue;
        }
        let target = bin_dir.join(&name);
        let mut out = std::fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        drop(out);
        make_executable(&target)?;
        written.push(target);
    }
    Ok(written)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Runs a binary and returns its stdout, or `None` if it could not run.
async fn run_capture(program: &Path, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().await.ok()?;
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Windows only: keep child processes from flashing a console window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Looks up an executable on `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(&name);
            candidate.is_file().then_some(candidate)
        })
    })
}
