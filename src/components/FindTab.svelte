<script lang="ts">
  import {
    AlertTriangle,
    CheckCheck,
    FileArchive,
    FolderOpen,
    Library,
    Loader2,
    Search,
    X,
  } from "lucide-svelte";
  import { runStore } from "@/stores/run.svelte";
  import { settingsStore } from "@/stores/settings.svelte";
  import { uiStore } from "@/stores/ui.svelte";
  import { api } from "@/lib/tauri";
  import { ALL_SOURCES, SOURCE_LABELS } from "@/lib/utils";
  import LiveDownloadStream from "./LiveDownloadStream.svelte";

  const EXAMPLES = [
    "transformer architectures and attention",
    "Christian bibles and patristic writings",
    "Jungian psychology and individuation",
    "climate change adaptation in agriculture",
    "early modern philosophy of mind",
  ];

  let query = $state("");
  let exporting = $state(false);
  let exportError = $state<string | null>(null);
  let exportedTo = $state<string | null>(null);

  async function start() {
    exportedTo = null;
    exportError = null;
    await runStore.startSearch(query, settingsStore);
  }

  async function exportZip() {
    exportError = null;
    exporting = true;
    try {
      const result = await runStore.exportZip();
      if (result) {
        exportedTo = result.dest;
      }
    } catch (e) {
      exportError = String(e);
    } finally {
      exporting = false;
    }
  }

  async function goLibrary() {
    if (!runStore.folder) return;
    try {
      const info = await api.openLibrary(runStore.folder);
      uiStore.setActiveLibrary(info);
    } catch {
      // ignore
    }
  }

  let failedItems = $derived(runStore.completed.filter((c) => c.status === "failed").slice(-10).reverse());
</script>

<div class="space-y-5">
  <header>
    <h1 class="text-2xl font-semibold">Discover</h1>
    <p class="text-sm text-[var(--color-muted-foreground)]">
      Describe what you're researching. We'll search {ALL_SOURCES.length} open sources,
      download what's relevant, and pack it as a ZIP for any AI tool.
    </p>
  </header>

  <div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5 shadow-sm">
    <textarea
      bind:value={query}
      rows="2"
      placeholder="e.g. attention mechanisms in transformers"
      disabled={runStore.running}
      class="w-full rounded-md border border-[var(--color-border)] bg-transparent p-3 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--color-primary)] disabled:opacity-50"
      onkeydown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          start();
        }
      }}
    ></textarea>

    <div class="mt-3 flex flex-wrap gap-1.5">
      {#each EXAMPLES as ex}
        <button
          type="button"
          onclick={() => (query = ex)}
          disabled={runStore.running}
          class="rounded-full border border-[var(--color-border)] px-2.5 py-0.5 text-xs text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)]/40 hover:bg-[var(--color-accent)] hover:text-[var(--color-foreground)] disabled:opacity-50"
        >
          {ex}
        </button>
      {/each}
    </div>

    <div class="mt-5">
      <div class="mb-1.5 flex items-center justify-between text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
        <span>Sources</span>
        {#if settingsStore.selectedSources.length === 0}
          <span class="normal-case tracking-normal text-rose-600">
            Pick at least one
          </span>
        {/if}
      </div>
      <div class="flex flex-wrap gap-1.5">
        {#each ALL_SOURCES as s}
          <button
            type="button"
            onclick={() => settingsStore.toggleSource(s)}
            disabled={runStore.running}
            class="px-3 py-1 rounded-md border text-xs transition-colors {settingsStore.selectedSources.includes(s) ? 'bg-[var(--color-primary)] text-white border-[var(--color-primary)]' : 'bg-transparent border-[var(--color-border)] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)]/40'}"
          >
            {SOURCE_LABELS[s]}
          </button>
        {/each}
      </div>
    </div>

    <div class="mt-5 flex flex-wrap items-center gap-2">
      {#if !runStore.running}
        <button
          onclick={start}
          disabled={!query.trim() || settingsStore.selectedSources.length === 0}
          class="flex items-center gap-2 px-6 py-2.5 rounded-lg bg-[var(--color-primary)] text-white font-medium hover:opacity-90 disabled:opacity-50"
        >
          <Search class="h-4 w-4" />
          Find documents
        </button>
      {:else}
        <button
          onclick={() => api.cancelRun()}
          class="flex items-center gap-2 px-6 py-2.5 rounded-lg bg-rose-600 text-white font-medium hover:opacity-90"
        >
          <X class="h-4 w-4" />
          Cancel
        </button>
      {/if}

      {#if runStore.folder && !runStore.running}
        <button
          onclick={exportZip}
          disabled={exporting}
          class="flex items-center gap-2 px-6 py-2.5 rounded-lg bg-[var(--color-secondary)] text-white font-medium hover:opacity-90 disabled:opacity-50"
        >
          {#if exporting}
            <Loader2 class="h-4 w-4 animate-spin" />
            Zipping…
          {:else}
            <FileArchive class="h-4 w-4" />
            Export ZIP for AI
          {/if}
        </button>
        <button
          onclick={goLibrary}
          class="flex items-center gap-2 px-6 py-2.5 rounded-lg border border-[var(--color-border)] font-medium hover:bg-[var(--color-accent)]"
        >
          <Library class="h-4 w-4" />
          Open in Library
        </button>
        <button
          onclick={() => api.revealInFinder(runStore.folder!)}
          class="flex items-center gap-2 px-6 py-2.5 rounded-lg text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
        >
          <FolderOpen class="h-4 w-4" />
          Show folder
        </button>
      {/if}
    </div>
  </div>

  {#if runStore.fatalError}
    <div class="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
      {runStore.fatalError}
    </div>
  {/if}

  {#if exportError}
    <div class="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
      Export failed: {exportError}
    </div>
  {/if}

  {#if exportedTo}
    <div class="flex items-center gap-2 rounded-md border border-emerald-500/40 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
      <CheckCheck class="h-4 w-4" />
      Saved to {exportedTo}
    </div>
  {/if}

  {#if runStore.running || runStore.total > 0}
    <div class="space-y-3">
      <div class="flex items-center justify-between gap-3 text-sm tabular-nums">
        <div class="flex items-center gap-3 text-[var(--color-muted-foreground)]">
          <span>
            <span class="font-semibold text-[var(--color-foreground)]">{runStore.found}</span> found
          </span>
          <span>·</span>
          <span>
            <span class="font-semibold text-emerald-700">{runStore.done}</span> done
          </span>
          {#if runStore.failed > 0}
            <span>·</span>
            <span>
              <span class="font-semibold text-rose-700">{runStore.failed}</span> failed
            </span>
          {/if}
          {#if runStore.filteredCount > 0}
            <span>·</span>
            <span title="Dropped because the title and abstract didn't mention any of your query keywords">
              <span class="font-semibold text-amber-700">{runStore.filteredCount}</span> off-topic
            </span>
          {/if}
        </div>
        {#if runStore.running}
          <div class="flex items-center gap-2 text-[var(--color-muted-foreground)]">
            <Loader2 class="h-4 w-4 animate-spin" />
            {runStore.total === 0 ? "Discovering…" : `${runStore.overallPct}%`}
          </div>
        {/if}
      </div>

      {#if runStore.total > 0}
        <div class="h-2 w-full bg-[var(--color-muted)] rounded-full overflow-hidden">
            <div class="h-full bg-[var(--color-primary)] transition-[width] duration-300" style="width: {runStore.overallPct}%"></div>
        </div>
      {/if}

      <LiveDownloadStream />
    </div>
  {/if}

  {#if runStore.sourceIssues.length > 0 || failedItems.length > 0}
    <details class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-4">
      <summary class="flex cursor-pointer list-none items-center gap-2 text-sm">
        <AlertTriangle class="h-4 w-4 text-amber-600" />
        <span class="font-medium">Issues</span>
        <span class="text-xs text-[var(--color-muted-foreground)]">
          {runStore.sourceIssues.length} source error{runStore.sourceIssues.length === 1 ? "" : "s"}
          {#if failedItems.length > 0}
            · {runStore.failed} failed download{runStore.failed === 1 ? "" : "s"}
          {/if}
        </span>
      </summary>

      {#if runStore.sourceIssues.length > 0}
        <div class="mt-3 space-y-1.5">
          <div class="text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
            Source errors
          </div>
          {#each runStore.sourceIssues as issue (issue.ts)}
            <div class="rounded-md border border-rose-500/30 bg-rose-50 px-2.5 py-1.5 text-xs">
              <span class="font-mono text-rose-700">
                {SOURCE_LABELS[issue.source] ?? issue.source}
              </span>{" "}
              <span class="text-[var(--color-foreground)]/80">{issue.error}</span>
            </div>
          {/each}
        </div>
      {/if}

      {#if failedItems.length > 0}
        <div class="mt-3 space-y-1.5">
          <div class="text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
            Recent failed downloads
          </div>
          {#each failedItems as c (c.url)}
            <div class="rounded-md border border-[var(--color-border)] bg-[var(--color-card)] px-2.5 py-1.5 text-xs" title={c.url}>
              <div class="truncate text-[var(--color-foreground)]">{c.title}</div>
              {#if c.error}
                <div class="truncate text-[10px] text-rose-700/80">{c.error}</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </details>
  {/if}
</div>
