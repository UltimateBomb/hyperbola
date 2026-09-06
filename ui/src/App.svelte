<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import Queue from "./lib/Queue.svelte";
  import ProbePanel from "./lib/ProbePanel.svelte";
  import UpdatesPanel from "./lib/UpdatesPanel.svelte";
  import SettingsPanel from "./lib/SettingsPanel.svelte";
  import {
    api, humanBytes,
    type Component, type MediaProbe, type Settings, type Snapshot, type UpdateReport,
  } from "./lib/api";

  let url = $state("");
  let probing = $state(false);
  let probe: MediaProbe | null = $state(null);
  let error: string | null = $state(null);
  let snapshot: Snapshot = $state({
    items: [],
    stats: { queued: 0, running: 0, paused: 0, completed: 0, failed: 0, speed_bps: 0 },
  });
  let settings: Settings | null = $state(null);
  let updates: UpdateReport | null = $state(null);
  let dependencyProgress: { component: Component; downloaded: number; total: number | null } | null =
    $state(null);
  let panel: "none" | "updates" | "settings" = $state("none");
  let version = $state("");
  let platform = $state("");
  let clipboardSuggestion: string | null = $state(null);
  let lastClipboard = "";

  const pendingUpdates = $derived(
    (updates?.components ?? []).filter(
      (c) => c.state.state === "update_available" || c.state.state === "missing",
    ),
  );
  const ytdlpMissing = $derived(
    (updates?.components ?? []).some((c) => c.component === "yt_dlp" && c.state.state === "missing"),
  );

  onMount(async () => {
    snapshot = await api.snapshot();
    settings = await api.getSettings();
    version = await api.appVersion();
    platform = await api.platform().catch(() => "");

    await listen<Snapshot>("queue-changed", (event) => (snapshot = event.payload));
    await listen<UpdateReport>("updates-changed", (event) => (updates = event.payload));
    await listen<{ component: Component; downloaded: number; total: number | null }>(
      "dependency-progress",
      (event) => (dependencyProgress = event.payload),
    );

    api.checkUpdates().then((report) => (updates = report)).catch(() => {});
    setInterval(pollClipboard, 1200);
  });

  async function pollClipboard() {
    if (!settings?.watch_clipboard) return;
    // Android refuses clipboard reads to anything not in focus, and asking
    // every second only fills the system log with denials. Sharing a link to
    // the app is the platform's own answer to this.
    if (platform === "android") return;
    if (typeof document !== "undefined" && !document.hasFocus()) return;
    try {
      const text = (await readText())?.trim() ?? "";
      if (!text || text === lastClipboard) return;
      lastClipboard = text;
      if (/^https?:\/\/\S+$/i.test(text) && text !== url) clipboardSuggestion = text;
    } catch {
      // No clipboard access on this platform; the feature simply stays quiet.
    }
  }

  async function analyze(target = url) {
    const trimmed = target.trim();
    if (!trimmed) return;
    url = trimmed;
    clipboardSuggestion = null;
    probing = true;
    error = null;
    probe = null;
    try {
      probe = await api.probe(trimmed);
    } catch (e) {
      error = String(e);
    } finally {
      probing = false;
    }
  }

  function queued() {
    probe = null;
    url = "";
  }
</script>

<main>
  <header class="top">
    <div class="brand">
      <span class="mark"></span>
      <span class="name">Hyperbola</span>
      <span class="muted small">{version}</span>
    </div>
    <div class="top-actions">
      {#if pendingUpdates.length > 0}
        <button class="pill warn" onclick={() => (panel = "updates")}>
          {pendingUpdates.length} update{pendingUpdates.length > 1 ? "s" : ""}
        </button>
      {/if}
      <button class="ghost" onclick={() => (panel = panel === "updates" ? "none" : "updates")}>Updates</button>
      <button class="ghost" onclick={() => (panel = panel === "settings" ? "none" : "settings")}>Settings</button>
    </div>
  </header>

  {#if ytdlpMissing}
    <p class="banner">
      yt-dlp is not installed yet — nothing can download until it is.
      <button class="ghost" onclick={() => (panel = "updates")}>Install it</button>
    </p>
  {/if}

  <section class="add">
    <input
      type="text"
      placeholder="Paste a video or playlist link"
      bind:value={url}
      onkeydown={(e) => e.key === "Enter" && analyze()}
    />
    <button class="primary" onclick={() => analyze()} disabled={probing || !url.trim()}>
      {probing ? "Reading…" : "Analyze"}
    </button>
  </section>

  {#if clipboardSuggestion}
    <p class="suggestion">
      Link in clipboard: <span class="mono">{clipboardSuggestion.slice(0, 72)}</span>
      <button class="ghost" onclick={() => analyze(clipboardSuggestion!)}>Use it</button>
      <button class="ghost" onclick={() => (clipboardSuggestion = null)}>Dismiss</button>
    </p>
  {/if}

  {#if error}<p class="pill err wide">{error}</p>{/if}

  {#if probe}
    <ProbePanel {probe} onqueued={queued} />
  {/if}

  <section class="queue-head">
    <h2>Queue</h2>
    <span class="muted small">
      {snapshot.stats.running} running · {snapshot.stats.queued} waiting
      {#if snapshot.stats.speed_bps > 0}· {humanBytes(snapshot.stats.speed_bps)}/s{/if}
    </span>
    <button
      class="ghost"
      onclick={() => api.clearFinished()}
      disabled={snapshot.stats.completed === 0}
    >
      Clear finished
    </button>
  </section>

  <Queue {snapshot} />
</main>

{#if panel !== "none"}
  <aside class="drawer">
    <button class="close ghost icon" onclick={() => (panel = "none")}>✕</button>
    {#if panel === "updates"}
      <UpdatesPanel
        report={updates}
        progress={dependencyProgress}
        onrefresh={() => api.checkUpdates().then((r) => (updates = r)).catch(() => {})}
      />
    {:else if panel === "settings" && settings}
      <SettingsPanel {settings} onchange={(s) => (settings = s)} />
    {/if}
  </aside>
{/if}

<style>
  main {
    max-width: 900px;
    margin: 0 auto;
    /* Phones put a status bar over the top of the page. */
    padding: calc(22px + env(safe-area-inset-top)) 24px calc(60px + env(safe-area-inset-bottom));
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  @media (max-width: 640px) {
    main { padding-left: 14px; padding-right: 14px; gap: 12px; }
    .add { flex-direction: column; }
    .add button { width: 100%; }
    .top-actions { gap: 6px; }
  }
  .top { display: flex; justify-content: space-between; align-items: center; }
  .brand { display: flex; align-items: baseline; gap: 10px; }
  .mark {
    width: 14px; height: 14px; border-radius: 4px; align-self: center;
    background: linear-gradient(120deg, #22d3ee, #3b82f6 55%, #a78bfa);
  }
  .name { font-weight: 700; letter-spacing: 0.01em; font-size: 16px; }
  .small { font-size: 12px; }
  .top-actions { display: flex; gap: 8px; align-items: center; }
  .banner {
    margin: 0; padding: 10px 14px; border-radius: var(--radius);
    background: #201a08; border: 1px solid #4a3a12; color: var(--warn);
    display: flex; align-items: center; gap: 10px;
  }
  .add { display: flex; gap: 10px; }
  .add input { flex: 1; }
  .add button { flex: none; }
  .suggestion {
    margin: -6px 0 0; font-size: 13px; color: var(--muted);
    display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  }
  .wide { display: block; }
  .queue-head { display: flex; align-items: center; gap: 12px; margin-top: 6px; }
  .queue-head h2 { margin: 0; font-size: 15px; }
  .queue-head button { margin-left: auto; }
  .drawer {
    position: fixed; top: 0; right: 0; bottom: 0; width: min(560px, 92vw);
    background: var(--bg-soft); border-left: 1px solid var(--border);
    padding: 22px; overflow-y: auto; box-shadow: -20px 0 60px rgba(0, 0, 0, 0.45);
  }
  .close { position: absolute; top: 14px; right: 16px; }
</style>
