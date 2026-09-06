<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api, humanBytes, humanDuration, type Download, type Snapshot } from "./api";

  let { snapshot, platform = "" }: { snapshot: Snapshot; platform?: string } = $props();

  let problems: Record<number, string> = $state({});
  let advice: Record<number, string> = $state({});

  // A failure the user can do nothing with is half a failure. The engine's
  // own words stay; the sentence next to them says what to try.
  $effect(() => {
    for (const item of snapshot.items) {
      if (item.state.state !== "failed" || advice[item.id] !== undefined) continue;
      const message = item.state.message;
      api.failureAdvice(message).then((hint) => {
        if (hint) advice = { ...advice, [item.id]: hint };
      }).catch(() => {});
    }
  });

  // Opening and sending go through the shell: on a phone a finished file is
  // reached by the handle the system gave it, not by a path other apps
  // cannot read.
  async function open(id: number) {
    try {
      await api.openDownload(id);
      const { [id]: _gone, ...rest } = problems;
      problems = rest;
    } catch (e) {
      problems = { ...problems, [id]: String(e) };
    }
  }

  async function share(id: number) {
    try {
      await api.shareDownload(id);
    } catch (e) {
      problems = { ...problems, [id]: String(e) };
    }
  }

  function fraction(d: Download): number | null {
    const s = d.state;
    if ((s.state === "running" || s.state === "paused") && s.total_bytes) {
      return Math.min(1, s.downloaded_bytes / s.total_bytes);
    }
    return null;
  }

  function line(d: Download): string {
    const s = d.state;
    switch (s.state) {
      case "queued":
        return "Waiting";
      case "running":
        if (s.stage === "probing") return "Reading media info…";
        if (s.stage === "postprocessing") return "Merging and tagging…";
        return [
          `${humanBytes(s.downloaded_bytes)} of ${humanBytes(s.total_bytes)}`,
          s.speed_bps ? `${humanBytes(s.speed_bps)}/s` : null,
          s.eta_secs !== null ? `${humanDuration(s.eta_secs)} left` : null,
        ].filter(Boolean).join(" · ");
      case "paused":
        return `Paused at ${humanBytes(s.downloaded_bytes)}`;
      case "completed":
        return s.path;
      case "failed":
        return s.message;
      case "canceled":
        return "Cancelled";
    }
  }
</script>

<div class="queue">
  {#each snapshot.items as item (item.id)}
    {@const f = fraction(item)}
    <article class="item" class:failed={item.state.state === "failed"}>
      <div class="head">
        <span class="title" title={item.title}>{item.title}</span>
        <span class="badge {item.state.state}">{item.state.state}</span>
      </div>

      <div class="bar" class:indeterminate={item.state.state === "running" && f === null}>
        <div class="fill" style={f !== null ? `width:${(f * 100).toFixed(1)}%` : ""}></div>
      </div>

      {#if item.state.state === "failed" && advice[item.id]}
        <div class="advice">{advice[item.id]}</div>
      {/if}

      {#if problems[item.id]}
        <div class="problem">{problems[item.id]}</div>
      {/if}

      <div class="foot">
        <span class="detail mono" title={line(item)}>{line(item)}</span>
        <span class="actions">
          {#if item.state.state === "running"}
            <button class="icon ghost" onclick={() => api.pause(item.id)} title="Pause">❚❚</button>
          {/if}
          {#if item.state.state === "paused"}
            <button class="icon ghost" onclick={() => api.resume(item.id)} title="Resume">▶</button>
          {/if}
          {#if item.state.state === "failed" || item.state.state === "canceled"}
            <button class="icon ghost" onclick={() => api.retry(item.id)} title="Try again">↻</button>
          {/if}
          {#if item.state.state === "completed"}
            <button class="icon ghost" onclick={() => open(item.id)} title="Play">▶</button>
            {#if platform === "android"}
              <button class="icon ghost" onclick={() => share(item.id)} title="Send to another device">⤴</button>
            {:else}
              <button class="icon ghost" onclick={() => revealItemInDir(item.state.path)} title="Show in folder">📁</button>
            {/if}
          {/if}
          {#if item.state.state === "running" || item.state.state === "queued" || item.state.state === "paused"}
            <button class="icon ghost danger" onclick={() => api.cancel(item.id)} title="Cancel">✕</button>
          {:else}
            <button class="icon ghost danger" onclick={() => api.remove(item.id)} title="Remove">✕</button>
          {/if}
        </span>
      </div>
    </article>
  {:else}
    <p class="empty muted">Nothing in the queue. Paste a link above.</p>
  {/each}
</div>

<style>
  .queue { display: flex; flex-direction: column; gap: 10px; }
  .item {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px 14px;
  }
  .item.failed { border-color: #43202a; }
  .head { display: flex; align-items: center; gap: 10px; justify-content: space-between; }
  .title { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge {
    font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em;
    color: var(--muted); border: 1px solid var(--border); border-radius: 999px; padding: 2px 8px;
    flex: none;
  }
  .badge.running { color: var(--accent); border-color: #1d3f5e; }
  .badge.completed { color: var(--ok); border-color: #14432f; }
  .badge.failed { color: var(--err); border-color: #4a1c1c; }
  .bar {
    height: 6px; border-radius: 999px; background: var(--bg-soft);
    margin: 10px 0 8px; overflow: hidden;
  }
  .fill {
    height: 100%; width: 0;
    background: linear-gradient(90deg, #22d3ee, #3b82f6 60%, #a78bfa);
    transition: width 180ms linear;
  }
  .bar.indeterminate .fill { width: 35%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { 0% { margin-left: -35%; } 100% { margin-left: 100%; } }
  .foot { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .detail { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; gap: 4px; flex: none; }
  .empty { text-align: center; padding: 28px 0; }
  .problem { color: var(--err); font-size: 12px; margin-bottom: 6px; }
  .advice { color: var(--warn); font-size: 12px; margin: 4px 0 2px; }
</style>
