<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, type Settings } from "./api";

  let { settings, onchange }: { settings: Settings; onchange: (s: Settings) => void } = $props();

  let draft: Settings = $state({ ...settings, subtitle_languages: [...settings.subtitle_languages] });
  let saved = $state(false);

  const browsers = ["", "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi"];
  let cookieBrowser = $state(draft.cookies.source === "browser" ? draft.cookies.value : "");
  let subtitleText = $state(draft.subtitle_languages.join(", "));

  let platform = $state("");
  $effect(() => {
    api.platform().then((p) => (platform = p)).catch(() => {});
  });

  async function pickFolder() {
    // Android has no file paths to browse: the system hands back a folder
    // grant instead, and the app remembers it.
    if (platform === "android") {
      const label = await api.pickOutputFolder();
      if (label) draft.download_dir = label;
      return;
    }
    const chosen = await open({ directory: true, defaultPath: draft.download_dir });
    if (typeof chosen === "string") draft.download_dir = chosen;
  }

  async function save() {
    draft.cookies = cookieBrowser ? { source: "browser", value: cookieBrowser } : { source: "none" };
    draft.subtitle_languages = subtitleText
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    await api.setSettings($state.snapshot(draft));
    onchange($state.snapshot(draft));
    saved = true;
    setTimeout(() => (saved = false), 1600);
  }
</script>

<div class="settings">
  <header><h3>Settings</h3><button class="primary" onclick={save}>{saved ? "Saved" : "Save"}</button></header>

  <div class="field">
    <span class="label muted">Save downloads to</span>
    <div class="inline">
      <input type="text" bind:value={draft.download_dir} readonly={platform === "android"} />
      <button onclick={pickFolder}>Browse…</button>
    </div>
  </div>

  <div class="grid">
    <label class="field">
      <span class="label muted">Video format</span>
      <select bind:value={draft.video_container}>
        <option value="mp4">MP4</option><option value="mkv">MKV</option>
        <option value="webm">WebM</option><option value="source">Keep source</option>
      </select>
    </label>
    <label class="field">
      <span class="label muted">Audio format</span>
      <select bind:value={draft.audio_container}>
        <option value="mp3">MP3</option><option value="m4a">M4A</option>
        <option value="opus">Opus</option><option value="flac">FLAC</option><option value="wav">WAV</option>
      </select>
    </label>
    <label class="field">
      <span class="label muted">Downloads at once</span>
      <input type="number" min="1" max="10" bind:value={draft.max_concurrent} />
    </label>
    <label class="field">
      <span class="label muted">Speed limit (KB/s, empty = off)</span>
      <input type="number" min="0" bind:value={draft.speed_limit_kbps} />
    </label>
    <label class="field">
      <span class="label muted">Subtitle languages</span>
      <input type="text" bind:value={subtitleText} placeholder="en, ru" />
    </label>
    <label class="field">
      <span class="label muted">Cookies from browser</span>
      <select bind:value={cookieBrowser}>
        {#each browsers as b}<option value={b}>{b || "none"}</option>{/each}
      </select>
    </label>
    <label class="field">
      <span class="label muted">Proxy</span>
      <input type="text" bind:value={draft.proxy} placeholder="socks5://127.0.0.1:1080" />
    </label>
    <label class="field">
      <span class="label muted">yt-dlp channel</span>
      <select bind:value={draft.ytdlp_channel}>
        <option value="stable">Stable</option>
        <option value="nightly">Nightly (extractor fixes first)</option>
      </select>
    </label>
  </div>

  <div class="toggles">
    <label class="row">
      <input type="checkbox" bind:checked={draft.prefer_compatible} />
      Prefer formats that play everywhere (H.264/AAC)
    </label>
    <label class="row"><input type="checkbox" bind:checked={draft.embed_metadata} /> Embed metadata</label>
    <label class="row"><input type="checkbox" bind:checked={draft.embed_thumbnail} /> Embed thumbnail</label>
    <label class="row"><input type="checkbox" bind:checked={draft.embed_chapters} /> Embed chapters</label>
    <label class="row"><input type="checkbox" bind:checked={draft.embed_subtitles} /> Embed subtitles in the file</label>
    <label class="row"><input type="checkbox" bind:checked={draft.remove_sponsor_segments} /> Remove sponsor segments</label>
    <label class="row"><input type="checkbox" bind:checked={draft.watch_clipboard} /> Offer links copied to the clipboard</label>
    <label class="row"><input type="checkbox" bind:checked={draft.auto_check_updates} /> Check for updates on start</label>
    <label class="row"><input type="checkbox" bind:checked={draft.auto_install_dependency_updates} /> Install yt-dlp and ffmpeg updates automatically</label>
  </div>
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: 16px; }
  header { display: flex; justify-content: space-between; align-items: center; }
  h3 { margin: 0; font-size: 15px; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .label { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; }
  .inline { display: flex; gap: 8px; }
  .inline button { flex: none; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 12px; }
  .toggles { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 8px; }
</style>
