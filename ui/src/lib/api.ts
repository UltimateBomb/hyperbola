// Typed view of the commands the Rust side exposes. Shapes mirror the serde
// representations in crates/core, so a change there breaks the build here.
import { invoke } from "@tauri-apps/api/core";

export type MediaKind = "video" | "audio";
export type Container =
  | "mp4" | "webm" | "mkv" | "mp3" | "opus" | "flac" | "wav" | "m4a" | "source";
export type Stage = "probing" | "downloading" | "postprocessing";
export type Component = "app" | "yt_dlp" | "ffmpeg";
export type Channel = "stable" | "nightly";

export interface Format {
  id: string;
  ext: string;
  height: number | null;
  width: number | null;
  fps: number | null;
  vcodec: string | null;
  acodec: string | null;
  filesize: number | null;
  filesize_is_estimate: boolean;
  tbr: number | null;
  note: string | null;
  protocol: string | null;
}

export interface SubtitleTrack {
  language: string;
  name: string | null;
  is_automatic: boolean;
}

export interface MediaItem {
  url: string;
  id: string;
  title: string;
  uploader: string | null;
  duration_secs: number | null;
  thumbnail: string | null;
  is_live: boolean;
  formats: Format[];
  subtitles: SubtitleTrack[];
  playlist_index: number | null;
}

export interface MediaProbe {
  source_url: string;
  playlist_title: string | null;
  items: MediaItem[];
}

export interface Progress {
  stage: Stage;
  downloaded_bytes: number;
  total_bytes: number | null;
  speed_bps: number | null;
  eta_secs: number | null;
}

export type DownloadState =
  | { state: "queued" }
  | ({ state: "running" } & Progress)
  | ({ state: "paused" } & Progress)
  | { state: "completed"; path: string }
  | { state: "failed"; message: string; retryable: boolean }
  | { state: "canceled" };

export interface Download {
  id: number;
  title: string;
  thumbnail: string | null;
  state: DownloadState;
  attempts: number;
  options: { url: string; kind: MediaKind; container: Container; output_dir: string };
}

export interface QueueStats {
  queued: number;
  running: number;
  paused: number;
  completed: number;
  failed: number;
  speed_bps: number;
}

export interface Snapshot {
  items: Download[];
  stats: QueueStats;
}

export type CookieSource =
  | { source: "none" }
  | { source: "browser"; value: string }
  | { source: "file"; value: string };

export interface Settings {
  download_dir: string;
  max_concurrent: number;
  concurrent_fragments: number;
  video_container: Container;
  audio_container: Container;
  embed_metadata: boolean;
  embed_thumbnail: boolean;
  embed_chapters: boolean;
  subtitle_languages: string[];
  embed_subtitles: boolean;
  remove_sponsor_segments: boolean;
  speed_limit_kbps: number | null;
  cookies: CookieSource;
  proxy: string | null;
  ytdlp_channel: Channel;
  auto_check_updates: boolean;
  auto_install_dependency_updates: boolean;
  watch_clipboard: boolean;
  prefer_compatible: boolean;
}

export type UpdateState =
  | { state: "up_to_date" }
  | { state: "update_available"; from: string; to: string }
  | { state: "missing"; to: string }
  | { state: "unknown"; reason: string };

export interface ComponentStatus {
  component: Component;
  installed: string | null;
  latest: string | null;
  state: UpdateState;
}

export interface UpdateReport {
  components: ComponentStatus[];
}

export interface AddRequest {
  url: string;
  title?: string | null;
  kind: MediaKind;
  format_id?: string | null;
  max_height?: number | null;
  filename?: string | null;
  time_frame?: { start_secs: number; end_secs: number } | null;
}

export interface DependencyPaths {
  ytdlp: string | null;
  ffmpeg: string | null;
  bin_dir: string;
}

export const api = {
  probe: (url: string) => invoke<MediaProbe>("probe_url", { url }),
  add: (requests: AddRequest[]) => invoke<number[]>("add_downloads", { requests }),
  snapshot: () => invoke<Snapshot>("queue_snapshot"),
  pause: (id: number) => invoke<void>("pause_download", { id }),
  resume: (id: number) => invoke<void>("resume_download", { id }),
  cancel: (id: number) => invoke<void>("cancel_download", { id }),
  retry: (id: number) => invoke<void>("retry_download", { id }),
  remove: (id: number) => invoke<void>("remove_download", { id }),
  clearFinished: () => invoke<void>("clear_finished"),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  dependencyPaths: () => invoke<DependencyPaths>("dependency_paths"),
  checkUpdates: () => invoke<UpdateReport>("check_updates"),
  installUpdate: (component: Component) => invoke<string>("install_update", { component }),
  appVersion: () => invoke<string>("app_version"),
  platform: () => invoke<string>("app_platform"),
  pickOutputFolder: () => invoke<string | null>("pick_output_folder"),
  openDownload: (id: number) => invoke<void>("open_download", { id }),
  shareDownload: (id: number) => invoke<void>("share_download", { id }),
};

export function humanBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(value)} B`;
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

export function humanDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return "—";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

export function formatLabel(format: Format): string {
  const parts: string[] = [];
  if (format.height) {
    parts.push(format.fps && format.fps >= 50 ? `${format.height}p${Math.round(format.fps)}` : `${format.height}p`);
  } else {
    parts.push("audio");
  }
  parts.push(format.ext);
  const codec = format.vcodec && format.vcodec !== "none" ? format.vcodec : format.acodec;
  if (codec && codec !== "none") parts.push(codec.split(".")[0]);
  if (format.filesize) parts.push(`${format.filesize_is_estimate ? "~" : ""}${humanBytes(format.filesize)}`);
  return parts.join(" · ");
}
