<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { api, humanBytes, type Component, type ComponentStatus, type DependencyPaths, type UpdateReport } from "./api";

  let {
    report,
    progress,
    onrefresh,
  }: {
    report: UpdateReport | null;
    progress: { component: Component; downloaded: number; total: number | null } | null;
    onrefresh: () => void;
  } = $props();

  let busy: Component | null = $state(null);
  let failures: Record<string, string> = $state({});
  let checking = $state(false);
  let paths: DependencyPaths | null = $state(null);
  let error: string | null = $state(null);

  $effect(() => {
    api.dependencyPaths().then((p) => (paths = p)).catch(() => {});
  });

  // A failed automatic install used to leave the app looking fine while it
  // could not merge a file. Now it says so.
  $effect(() => {
    const unlisten = listen<{ component: Component; message: string }>(
      "dependency-error",
      (event) => (failures = { ...failures, [event.payload.component]: event.payload.message }),
    );
    return () => { unlisten.then((off) => off()); };
  });

  const names: Record<Component, string> = { app: "Hyperbola", yt_dlp: "yt-dlp", ffmpeg: "ffmpeg" };
  const why: Record<Component, string> = {
    app: "The app itself.",
    yt_dlp: "The extractor. This is the one that goes stale when a site changes — keep it current.",
    ffmpeg: "Merging, audio extraction and cutting.",
  };

  const actionable = $derived(
    (report?.components ?? []).filter(
      (c) => c.state.state === "update_available" || c.state.state === "missing",
    ),
  );

  function describe(status: ComponentStatus): string {
    switch (status.state.state) {
      case "up_to_date":
        return status.installed ? `Up to date · ${status.installed}` : "Up to date";
      case "update_available":
        return `${status.state.from} → ${status.state.to}`;
      case "missing":
        return `Not installed · ${status.state.to} available`;
      case "unknown":
        return `Could not check: ${status.state.reason}`;
    }
  }

  async function install(component: Component) {
    error = null;
    busy = component;
    try {
      await api.installUpdate(component);
      const { [component]: _cleared, ...rest } = failures;
      failures = rest;
      onrefresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  // The app update restarts the program, so it stays a deliberate click
  // rather than something that happens in the middle of a batch.
  async function installAll() {
    for (const status of actionable) {
      if (status.component === "app") continue;
      await install(status.component);
    }
  }

  async function check() {
    checking = true;
    error = null;
    try {
      onrefresh();
      await api.checkUpdates();
    } catch (e) {
      error = String(e);
    } finally {
      checking = false;
    }
  }
</script>

<div class="updates">
  <header>
    <h3>Updates</h3>
    <span class="head-actions">
      <button class="ghost" onclick={check} disabled={checking}>{checking ? "Checking…" : "Check now"}</button>
      <button class="primary" onclick={installAll} disabled={busy !== null || actionable.filter((s) => s.component !== "app").length === 0}>
        Update everything
      </button>
    </span>
  </header>

  {#if error}<p class="pill err">{error}</p>{/if}

  {#each report?.components ?? [] as status (status.component)}
    <section class="row">
      <div class="left">
        <div class="name">
          {names[status.component]}
          {#if status.state.state === "update_available"}<span class="pill warn">update</span>{/if}
          {#if status.state.state === "missing"}<span class="pill err">missing</span>{/if}
          {#if status.state.state === "up_to_date"}<span class="pill ok">ok</span>{/if}
          {#if status.state.state === "unknown"}<span class="pill">unknown</span>{/if}
        </div>
        <div class="muted small">{describe(status)}</div>
        <div class="muted small why">{why[status.component]}</div>
        {#if failures[status.component]}
          <div class="failure small">{failures[status.component]}</div>
        {/if}
      </div>
      <div class="right">
        {#if status.component === "app"}
          {#if status.state.state === "update_available"}
            <button onclick={() => install("app")} disabled={busy !== null}>
              {busy === "app" ? "Downloading…" : "Update and restart"}
            </button>
          {/if}
        {:else if status.state.state === "update_available" || status.state.state === "missing"}
          <button onclick={() => install(status.component)} disabled={busy !== null}>
            {busy === status.component ? "Installing…" : status.state.state === "missing" ? "Install" : "Update"}
          </button>
        {/if}
      </div>
      {#if busy === status.component && progress && progress.component === status.component}
        <div class="bar">
          <div
            class="fill"
            style={progress.total ? `width:${((progress.downloaded / progress.total) * 100).toFixed(1)}%` : "width:35%"}
          ></div>
        </div>
        <div class="muted small">{humanBytes(progress.downloaded)}{progress.total ? ` of ${humanBytes(progress.total)}` : ""}</div>
      {/if}
    </section>
  {:else}
    <p class="muted">Checking…</p>
  {/each}

  {#if paths}
    <footer class="muted small mono">
      <div>yt-dlp: {paths.ytdlp ?? "not found"}</div>
      <div>ffmpeg: {paths.ffmpeg ?? "not found"}</div>
      <div>managed binaries: {paths.bin_dir}</div>
    </footer>
  {/if}
</div>

<style>
  .updates { display: flex; flex-direction: column; gap: 12px; }
  header { display: flex; justify-content: space-between; align-items: center; }
  h3 { margin: 0; font-size: 15px; }
  .head-actions { display: flex; gap: 8px; }
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px 12px;
    align-items: center;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px 14px;
  }
  .name { display: flex; align-items: center; gap: 8px; font-weight: 600; }
  .small { font-size: 12px; }
  .why { opacity: 0.75; }
  .bar { grid-column: 1 / -1; height: 5px; background: var(--bg-soft); border-radius: 999px; overflow: hidden; }
  .fill { height: 100%; background: linear-gradient(90deg, #22d3ee, #a78bfa); transition: width 150ms linear; }
  .link { color: var(--accent); font-size: 13px; }
  .failure { color: var(--err); }
  footer { display: flex; flex-direction: column; gap: 2px; padding-top: 4px; word-break: break-all; }
</style>
