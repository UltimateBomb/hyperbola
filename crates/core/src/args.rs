//! Builds the yt-dlp command line.
//!
//! Every yt-dlp invocation Hyperbola makes starts with `--ignore-config`: a
//! stray `~/.config/yt-dlp/config` on the user's machine must never change
//! what the app does, or the same options would produce different files on
//! two machines.

use crate::domain::{CookieSource, DownloadOptions, MediaKind};
use crate::progress::{FINAL_PATH_TEMPLATE, POSTPROCESS_TEMPLATE, PROGRESS_TEMPLATE};
use std::path::PathBuf;

/// Paths and tuning the shell supplies — everything about the machine yt-dlp
/// will run on, which the core itself has no way to discover.
#[derive(Debug, Clone)]
pub struct RunnerEnv {
    /// Directory for `.part` files and other scratch data.
    pub temp_dir: PathBuf,
    /// Explicit ffmpeg location; when `None`, yt-dlp searches `PATH`.
    pub ffmpeg_path: Option<PathBuf>,
    /// Directory of extra yt-dlp plugins, when one is shipped.
    pub plugin_dir: Option<PathBuf>,
    /// JavaScript runtime for extractors that need one (`deno` on desktop,
    /// `quickjs` on Android).
    pub js_runtime: Option<String>,
    /// Parallel fragments per download.
    pub concurrent_fragments: u8,
    /// Restrict filenames to characters Windows accepts.
    pub windows_filenames: bool,
}

impl Default for RunnerEnv {
    fn default() -> Self {
        RunnerEnv {
            temp_dir: PathBuf::from("."),
            ffmpeg_path: None,
            plugin_dir: None,
            js_runtime: None,
            concurrent_fragments: 4,
            windows_filenames: cfg!(windows),
        }
    }
}

/// Arguments for reading a URL's metadata without downloading anything.
pub fn build_probe_args(
    url: &str,
    cookies: &CookieSource,
    proxy: Option<&str>,
    env: &RunnerEnv,
) -> Vec<String> {
    let mut args = vec![
        url.to_string(),
        "--ignore-config".into(),
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--ignore-errors".into(),
        "--no-warnings".into(),
        "--no-colors".into(),
    ];
    push_common(&mut args, cookies, proxy, env);
    args
}

/// Arguments for the download itself.
pub fn build_download_args(options: &DownloadOptions, env: &RunnerEnv) -> Vec<String> {
    let mut args = vec![
        options.url.clone(),
        "--ignore-config".into(),
        "--newline".into(),
        "--no-colors".into(),
        "--progress".into(),
        "--progress-template".into(),
        PROGRESS_TEMPLATE.into(),
        "--progress-template".into(),
        POSTPROCESS_TEMPLATE.into(),
        // Report where the finished file ended up. --print implies --quiet,
        // which is why --progress above is explicit.
        "--print".into(),
        FINAL_PATH_TEMPLATE.into(),
        // A postprocessor that cannot do its job must not throw away a file
        // that already downloaded. Success is judged by whether a file was
        // produced, not by yt-dlp's exit code alone.
        "--ignore-errors".into(),
        "--retries".into(),
        "3".into(),
        "--fragment-retries".into(),
        "10".into(),
        "--concurrent-fragments".into(),
        env.concurrent_fragments.max(1).to_string(),
    ];

    args.push("--format".into());
    args.push(format_selector(options));

    match options.kind {
        MediaKind::Video => {
            if let Some(container) = options.container.ytdlp_name() {
                args.push("--merge-output-format".into());
                args.push(container.into());
            }
        }
        MediaKind::Audio => {
            args.push("--extract-audio".into());
            if let Some(container) = options.container.ytdlp_name() {
                args.push("--audio-format".into());
                args.push(container.into());
                // 0 is yt-dlp's best VBR setting for lossy targets and is
                // ignored for lossless ones.
                args.push("--audio-quality".into());
                args.push("0".into());
            }
        }
    }

    args.push("--paths".into());
    args.push(format!("home:{}", options.output_dir.display()));
    args.push("--paths".into());
    args.push(format!("temp:{}", env.temp_dir.display()));
    args.push("--output".into());
    args.push(match &options.filename {
        Some(name) => format!("{name}.%(ext)s"),
        None => "%(title)s.%(ext)s".to_string(),
    });

    if !options.subtitle_languages.is_empty() {
        args.push("--write-subs".into());
        args.push("--write-auto-subs".into());
        args.push("--sub-langs".into());
        args.push(options.subtitle_languages.join(","));
        if options.embed_subtitles {
            args.push("--embed-subs".into());
        }
    }
    if options.embed_metadata {
        args.push("--embed-metadata".into());
    }
    // Only ask for a cover where the container can hold one.
    if options.embed_thumbnail && options.container.supports_embedded_thumbnail() {
        args.push("--embed-thumbnail".into());
    }
    if options.embed_chapters {
        args.push("--embed-chapters".into());
    }
    if options.remove_sponsor_segments {
        args.push("--sponsorblock-remove".into());
        args.push("default".into());
    }
    if let Some(frame) = options.time_frame {
        // yt-dlp's own --download-sections re-encodes unpredictably and drops
        // audio on some extractors; cutting in the merge step with ffmpeg is
        // the reliable path.
        args.push("--postprocessor-args".into());
        args.push(format!(
            "Merger+ffmpeg_i:-ss {:.3} -t {:.3}",
            frame.start_secs,
            frame.duration_secs()
        ));
    }
    if let Some(limit) = options.speed_limit {
        args.push("--limit-rate".into());
        args.push(limit.to_string());
    }

    push_common(&mut args, &options.cookies, options.proxy.as_deref(), env);
    args.extend(options.extra_args.iter().cloned());
    args
}

/// The `--format` expression for these options.
pub fn format_selector(options: &DownloadOptions) -> String {
    if let Some(id) = &options.format_id {
        // Pair an explicit video-only stream with the best audio, and fall
        // back to the stream alone if the site has no separate audio.
        return match options.kind {
            MediaKind::Video => format!("{id}+bestaudio/{id}"),
            MediaKind::Audio => id.clone(),
        };
    }
    match options.kind {
        MediaKind::Audio => "bestaudio/best".to_string(),
        MediaKind::Video => match options.max_height {
            Some(h) => format!(
                "bestvideo[height<={h}]+bestaudio/best[height<={h}]/bestvideo+bestaudio/best"
            ),
            None => "bestvideo+bestaudio/best".to_string(),
        },
    }
}

/// Flags shared by probing and downloading.
fn push_common(args: &mut Vec<String>, cookies: &CookieSource, proxy: Option<&str>, env: &RunnerEnv) {
    if let Some(ffmpeg) = &env.ffmpeg_path {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg.display().to_string());
    }
    if let Some(plugins) = &env.plugin_dir {
        args.push("--plugin-dirs".into());
        args.push(plugins.display().to_string());
    }
    if let Some(runtime) = &env.js_runtime {
        args.push("--js-runtimes".into());
        args.push(runtime.clone());
    }
    if env.windows_filenames {
        args.push("--windows-filenames".into());
    }
    match cookies {
        CookieSource::None => {}
        CookieSource::Browser(browser) => {
            args.push("--cookies-from-browser".into());
            args.push(browser.clone());
        }
        CookieSource::File(path) => {
            args.push("--cookies".into());
            args.push(path.display().to_string());
        }
    }
    if let Some(proxy) = proxy {
        if !proxy.is_empty() {
            args.push("--proxy".into());
            args.push(proxy.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Container, TimeFrame};

    fn env() -> RunnerEnv {
        RunnerEnv {
            temp_dir: PathBuf::from("/tmp/hb"),
            ffmpeg_path: Some(PathBuf::from("/opt/ffmpeg")),
            plugin_dir: None,
            js_runtime: Some("deno:/opt/deno".into()),
            concurrent_fragments: 4,
            windows_filenames: false,
        }
    }

    /// True when `flag` is present and immediately followed by `value`.
    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn every_invocation_ignores_the_users_global_config() {
        let options = DownloadOptions::video("https://x/y", "/out");
        assert!(build_download_args(&options, &env()).contains(&"--ignore-config".to_string()));
        assert!(build_probe_args("https://x/y", &CookieSource::None, None, &env())
            .contains(&"--ignore-config".to_string()));
    }

    #[test]
    fn video_selector_respects_a_height_cap() {
        let mut options = DownloadOptions::video("u", "/out");
        options.max_height = Some(1080);
        assert_eq!(
            format_selector(&options),
            "bestvideo[height<=1080]+bestaudio/best[height<=1080]/bestvideo+bestaudio/best"
        );
    }

    #[test]
    fn explicit_video_format_gets_audio_attached() {
        let mut options = DownloadOptions::video("u", "/out");
        options.format_id = Some("137".into());
        assert_eq!(format_selector(&options), "137+bestaudio/137");
    }

    #[test]
    fn audio_download_extracts_and_converts() {
        let options = DownloadOptions::audio("u", "/out");
        let args = build_download_args(&options, &env());
        assert!(args.contains(&"--extract-audio".to_string()));
        assert!(has_pair(&args, "--audio-format", "mp3"));
        assert!(has_pair(&args, "--audio-quality", "0"));
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn a_cover_is_only_requested_where_it_fits() {
        let mut options = DownloadOptions::video("u", "/out");
        options.embed_thumbnail = true;

        options.container = Container::Mp4;
        assert!(build_download_args(&options, &env()).contains(&"--embed-thumbnail".to_string()));

        // Embedding into webm fails at the very end, after the whole file has
        // been downloaded — so it is never asked for.
        options.container = Container::Webm;
        assert!(!build_download_args(&options, &env()).contains(&"--embed-thumbnail".to_string()));

        options.container = Container::Source;
        assert!(!build_download_args(&options, &env()).contains(&"--embed-thumbnail".to_string()));
    }

    #[test]
    fn postprocessing_errors_do_not_discard_the_file() {
        let args = build_download_args(&DownloadOptions::video("u", "/out"), &env());
        assert!(args.contains(&"--ignore-errors".to_string()));
    }

    #[test]
    fn source_container_skips_remuxing() {
        let mut options = DownloadOptions::video("u", "/out");
        options.container = Container::Source;
        let args = build_download_args(&options, &env());
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn paths_and_output_template_are_separate() {
        let mut options = DownloadOptions::video("u", "/out/videos");
        options.filename = Some("my clip".into());
        let args = build_download_args(&options, &env());
        assert!(has_pair(&args, "--paths", "home:/out/videos"));
        assert!(has_pair(&args, "--paths", "temp:/tmp/hb"));
        assert!(has_pair(&args, "--output", "my clip.%(ext)s"));
    }

    #[test]
    fn default_output_template_uses_the_title() {
        let options = DownloadOptions::video("u", "/out");
        assert!(has_pair(&build_download_args(&options, &env()), "--output", "%(title)s.%(ext)s"));
    }

    #[test]
    fn subtitles_are_written_and_embedded_on_request() {
        let mut options = DownloadOptions::video("u", "/out");
        options.subtitle_languages = vec!["en".into(), "ru".into()];
        options.embed_subtitles = true;
        let args = build_download_args(&options, &env());
        assert!(has_pair(&args, "--sub-langs", "en,ru"));
        assert!(args.contains(&"--write-subs".to_string()));
        assert!(args.contains(&"--embed-subs".to_string()));
    }

    #[test]
    fn no_subtitle_flags_when_none_selected() {
        let args = build_download_args(&DownloadOptions::video("u", "/out"), &env());
        assert!(!args.iter().any(|a| a.starts_with("--sub-langs")));
        assert!(!args.contains(&"--write-subs".to_string()));
    }

    #[test]
    fn time_frame_cuts_with_ffmpeg_not_download_sections() {
        let mut options = DownloadOptions::video("u", "/out");
        options.time_frame = Some(TimeFrame { start_secs: 30.0, end_secs: 95.5 });
        let args = build_download_args(&options, &env());
        assert!(!args.contains(&"--download-sections".to_string()));
        assert!(has_pair(&args, "--postprocessor-args", "Merger+ffmpeg_i:-ss 30.000 -t 65.500"));
    }

    #[test]
    fn cookies_from_browser_and_from_file_are_distinct() {
        let mut options = DownloadOptions::video("u", "/out");
        options.cookies = CookieSource::Browser("firefox".into());
        assert!(has_pair(&build_download_args(&options, &env()), "--cookies-from-browser", "firefox"));

        options.cookies = CookieSource::File(PathBuf::from("/home/u/cookies.txt"));
        let args = build_download_args(&options, &env());
        assert!(has_pair(&args, "--cookies", "/home/u/cookies.txt"));
        assert!(!args.contains(&"--cookies-from-browser".to_string()));
    }

    #[test]
    fn speed_limit_and_proxy_are_passed_through() {
        let mut options = DownloadOptions::video("u", "/out");
        options.speed_limit = Some(500_000);
        options.proxy = Some("socks5://127.0.0.1:1080".into());
        let args = build_download_args(&options, &env());
        assert!(has_pair(&args, "--limit-rate", "500000"));
        assert!(has_pair(&args, "--proxy", "socks5://127.0.0.1:1080"));
    }

    #[test]
    fn empty_proxy_string_is_not_passed() {
        let mut options = DownloadOptions::video("u", "/out");
        options.proxy = Some(String::new());
        assert!(!build_download_args(&options, &env()).contains(&"--proxy".to_string()));
    }

    #[test]
    fn extra_arguments_come_last_so_they_win() {
        let mut options = DownloadOptions::video("u", "/out");
        options.extra_args = vec!["--force-ipv4".into()];
        let args = build_download_args(&options, &env());
        assert_eq!(args.last().unwrap(), "--force-ipv4");
    }

    #[test]
    fn progress_templates_are_always_installed() {
        let args = build_download_args(&DownloadOptions::video("u", "/out"), &env());
        assert!(args.contains(&PROGRESS_TEMPLATE.to_string()));
        assert!(args.contains(&POSTPROCESS_TEMPLATE.to_string()));
        assert!(args.contains(&"--newline".to_string()));
    }

    #[test]
    fn the_final_path_is_always_printed() {
        let args = build_download_args(&DownloadOptions::video("u", "/out"), &env());
        assert!(args.contains(&FINAL_PATH_TEMPLATE.to_string()));
        // --print implies --quiet, so progress has to be asked for explicitly
        // or the download would report nothing at all.
        assert!(args.contains(&"--progress".to_string()));
    }

    #[test]
    fn url_is_the_first_argument() {
        let args = build_download_args(&DownloadOptions::video("https://x/y", "/out"), &env());
        assert_eq!(args[0], "https://x/y");
    }
}
