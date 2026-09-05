<script lang="ts">
  import {
    api, formatLabel, humanDuration,
    type AddRequest, type MediaKind, type MediaProbe,
  } from "./api";

  let { probe, onqueued }: { probe: MediaProbe; onqueued: () => void } = $props();

  let kind: MediaKind = $state("video");
  let maxHeight: number | null = $state(null);
  let subtitles: string[] = $state([]);
  let selected: Set<string> = $state(new Set(probe.items.map((i) => i.id)));
  let adding = $state(false);

  const first = $derived(probe.items[0]);
  const resolutions = $derived.by(() => {
    const heights = new Set<number>();
    for (const item of probe.items) {
      for (const f of item.formats) if (f.height) heights.add(f.height);
    }
    return [...heights].sort((a, b) => b - a);
  });
  const subtitleLanguages = $derived.by(() => {
    const langs = new Map<string, string>();
    for (const track of first?.subtitles ?? []) {
      if (!langs.has(track.language)) {
        langs.set(track.language, track.name ?? track.language);
      }
    }
    return [...langs.entries()].slice(0, 12);
  });

  function toggle(id: string) {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    selected = next;
  }

  function toggleSubtitle(language: string) {
    subtitles = subtitles.includes(language)
      ? subtitles.filter((l) => l !== language)
      : [...subtitles, language];
  }

  async function queue() {
    adding = true;
    try {
      const requests: AddRequest[] = probe.items
        .filter((item) => selected.has(item.id))
        .map((item) => ({
          url: item.url,
          title: item.title,
          kind,
          max_height: kind === "video" ? maxHeight : null,
          format_id: null,
          filename: null,
          time_frame: null,
        }));
      if (requests.length > 0) await api.add(requests);
      onqueued();
    } finally {
      adding = false;
    }
  }
</script>

<section class="panel">
  <div class="media">
    {#if first?.thumbnail}
      <img class="thumb" src={first.thumbnail} alt="" referrerpolicy="no-referrer" />
    {/if}
    <div class="meta">
      <h2>{probe.playlist_title ?? first?.title ?? "Media"}</h2>
      <p class="muted">
        {#if probe.playlist_title}
          Playlist · {probe.items.length} items
        {:else}
          {first?.uploader ?? "Unknown"} · {humanDuration(first?.duration_secs)}
          {#if first?.is_live} · <span class="pill err">live</span>{/if}
        {/if}
      </p>
      {#if first && first.formats.length > 0}
        <p class="muted small mono">best: {formatLabel(first.formats.reduce((a, b) => ((b.height ?? 0) > (a.height ?? 0) ? b : a)))}</p>
      {/if}
    </div>
  </div>

  <div class="controls">
    <div class="control">
      <span class="label muted">Type</span>
      <div class="segmented">
        <button class:on={kind === "video"} onclick={() => (kind = "video")}>Video</button>
        <button class:on={kind === "audio"} onclick={() => (kind = "audio")}>Audio</button>
      </div>
    </div>

    {#if kind === "video"}
      <div class="control">
        <span class="label muted">Quality</span>
        <select bind:value={maxHeight}>
          <option value={null}>Best available</option>
          {#each resolutions as height}
            <option value={height}>{height}p and below</option>
          {/each}
        </select>
      </div>
    {/if}

    {#if subtitleLanguages.length > 0}
      <div class="control wide">
        <span class="label muted">Subtitles</span>
        <div class="chips">
          {#each subtitleLanguages as [code, name]}
            <button class="chip" class:on={subtitles.includes(code)} onclick={() => toggleSubtitle(code)} title={name}>
              {code}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </div>

  {#if probe.items.length > 1}
    <div class="items">
      <div class="items-head">
        <span class="muted">{selected.size} of {probe.items.length} selected</span>
        <span>
          <button class="ghost" onclick={() => (selected = new Set(probe.items.map((i) => i.id)))}>All</button>
          <button class="ghost" onclick={() => (selected = new Set())}>None</button>
        </span>
      </div>
      <ul>
        {#each probe.items as item (item.id)}
          <li>
            <label class="row">
              <input type="checkbox" checked={selected.has(item.id)} onchange={() => toggle(item.id)} />
              <span class="index muted">{item.playlist_index ?? ""}</span>
              <span class="name">{item.title}</span>
              <span class="muted small">{humanDuration(item.duration_secs)}</span>
            </label>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <div class="actions">
    <button class="primary" onclick={queue} disabled={adding || selected.size === 0}>
      {adding ? "Adding…" : `Download ${selected.size > 1 ? `${selected.size} items` : ""}`}
    </button>
  </div>
</section>

<style>
  .panel {
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .media { display: flex; gap: 14px; align-items: flex-start; }
  .thumb { width: 160px; aspect-ratio: 16/9; object-fit: cover; border-radius: 8px; flex: none; background: var(--bg-soft); }
  .meta h2 { margin: 0 0 6px; font-size: 17px; line-height: 1.3; }
  .meta p { margin: 0 0 4px; }
  .small { font-size: 12px; }
  .controls { display: flex; flex-wrap: wrap; gap: 14px; }
  .control { display: flex; flex-direction: column; gap: 6px; min-width: 180px; }
  .control.wide { flex: 1 1 260px; }
  .label { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; }
  .segmented { display: flex; gap: 0; }
  .segmented button { border-radius: 0; }
  .segmented button:first-child { border-radius: var(--radius) 0 0 var(--radius); }
  .segmented button:last-child { border-radius: 0 var(--radius) var(--radius) 0; border-left: none; }
  .segmented button.on { background: #24325e; border-color: #3a4b80; color: #fff; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { padding: 5px 10px; font-size: 12px; border-radius: 999px; }
  .chip.on { background: #24325e; border-color: var(--accent); }
  .items { border-top: 1px solid var(--border); padding-top: 12px; }
  .items-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
  .items ul { list-style: none; margin: 0; padding: 0; max-height: 220px; overflow-y: auto; }
  .items li { padding: 4px 0; }
  .index { width: 24px; text-align: right; font-size: 12px; flex: none; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; }
  .actions { display: flex; justify-content: flex-end; }
</style>
