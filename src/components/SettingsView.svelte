<script lang="ts">
  import {
    FileText,
    FolderOpen,
    Server,
    Loader2,
    CheckCircle2,
  } from "lucide-svelte";
  import { api, type LogInfo } from "@/lib/tauri";
  import { formatBytes } from "@/lib/utils";
  import { settingsStore } from "@/stores/settings.svelte";

  let logInfo = $state<LogInfo | null>(null);
  let settingUpSearx = $state(false);
  let searxResult = $state<string | null>(null);
  let searxError = $state<string | null>(null);

  $effect(() => {
    api.runLogInfo().then((info) => (logInfo = info)).catch(() => (logInfo = null));
  });

  async function handleSetupSearx() {
    settingUpSearx = true;
    searxResult = null;
    searxError = null;
    try {
      searxResult = await api.setupSearXNG();
    } catch (e) {
      searxError = String(e);
    } finally {
      settingUpSearx = false;
    }
  }

  async function refreshLog() {
    try {
      logInfo = await api.runLogInfo();
    } catch {
      logInfo = null;
    }
  }
</script>

<div class="space-y-5">
  <header>
    <h1 class="text-2xl font-semibold">Settings</h1>
  </header>

  <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
    <h2 class="mb-3 text-sm font-medium">Discovery</h2>
    <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
      <div>
        <label class="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">Per source</label>
        <input
          type="number"
          min="1"
          bind:value={settingsStore.perSource}
          onchange={() => settingsStore.save()}
          class="w-full rounded-md border border-[var(--color-border)] bg-transparent p-2 text-sm"
        />
        <p class="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
          Max documents per source per sub-query.
        </p>
      </div>
      <div>
        <label class="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">Max total</label>
        <input
          type="number"
          min="1"
          bind:value={settingsStore.maxTotal}
          onchange={() => settingsStore.save()}
          class="w-full rounded-md border border-[var(--color-border)] bg-transparent p-2 text-sm"
        />
        <p class="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
          Hard cap across all sources.
        </p>
      </div>
      <div>
        <label class="block text-xs font-medium text-[var(--color-muted-foreground)] mb-1">Parallel downloads</label>
        <input
          type="number"
          min="1"
          max="32"
          bind:value={settingsStore.concurrency}
          onchange={() => settingsStore.save()}
          class="w-full rounded-md border border-[var(--color-border)] bg-transparent p-2 text-sm"
        />
        <p class="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
          Higher = faster but more risk of rate limits.
        </p>
      </div>
    </div>
  </section>

  <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
    <h2 class="mb-3 text-sm font-medium">Library folder</h2>
    <input
      bind:value={settingsStore.libraryRoot}
      onchange={() => settingsStore.save()}
      class="w-full rounded-md border border-[var(--color-border)] bg-transparent p-2 text-xs font-mono"
    />
  </section>

  <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
    <h2 class="mb-1 text-sm font-medium">Search Infrastructure</h2>
    <p class="mb-3 text-[11px] text-[var(--color-muted-foreground)]">
      SearXNG provides privacy-respecting search across dozens of engines.
      If you have Docker installed, we can set up a local instance for you.
    </p>
    <div class="space-y-3">
      <button
        onclick={handleSetupSearx}
        disabled={settingUpSearx}
        class="px-4 py-2 rounded-lg border border-[var(--color-border)] text-sm font-medium hover:bg-[var(--color-accent)] inline-flex items-center gap-2"
      >
        {#if settingUpSearx}
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
          Setting up...
        {:else}
          <Server class="h-3.5 w-3.5" />
          Setup SearXNG with Docker
        {/if}
      </button>

      {#if searxResult}
        <div class="flex items-start gap-2 rounded-md border border-emerald-500/30 bg-emerald-50 p-3 text-[11px] text-emerald-800">
          <CheckCircle2 class="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div class="space-y-1">
            <p class="font-medium">Success!</p>
            <pre class="max-h-32 overflow-auto whitespace-pre-wrap font-mono text-[10px] opacity-80">
              {searxResult}
            </pre>
          </div>
        </div>
      {/if}

      {#if searxError}
        <div class="flex items-start gap-2 rounded-md border border-rose-500/30 bg-rose-50 p-3 text-[11px] text-rose-800">
          <CheckCircle2 class="mt-0.5 h-3.5 w-3.5 shrink-0 rotate-45" />
          <div class="space-y-1">
            <p class="font-medium">Setup Failed</p>
            <p class="opacity-80">{searxError}</p>
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
    <h2 class="mb-1 text-sm font-medium">Run log</h2>
    <p class="mb-3 text-[11px] text-[var(--color-muted-foreground)]">
      Every query, source error, and download outcome is appended here.
      Share this file when reporting issues — it's the easiest way to
      diagnose failed downloads.
    </p>
    {#if logInfo}
      <div class="space-y-2">
        <code class="block truncate rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] px-2.5 py-1.5 font-mono text-[11px]">
          {logInfo.path}
        </code>
        <div class="flex items-center gap-2 text-[11px] text-[var(--color-muted-foreground)]">
          <span>
            {logInfo.exists
              ? formatBytes(logInfo.size_bytes)
              : "not yet written"}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <button
            onclick={() => api.revealInFinder(logInfo!.path)}
            disabled={!logInfo.exists}
            class="px-3 py-1.5 rounded-lg border border-[var(--color-border)] text-xs font-medium hover:bg-[var(--color-accent)] inline-flex items-center gap-2 disabled:opacity-50"
          >
            <FolderOpen class="h-3.5 w-3.5" />
            Show log in Finder
          </button>
          <button
            onclick={refreshLog}
            class="px-3 py-1.5 rounded-lg border border-[var(--color-border)] text-xs font-medium hover:bg-[var(--color-accent)] inline-flex items-center gap-2"
          >
            <FileText class="h-3.5 w-3.5" />
            Refresh
          </button>
        </div>
      </div>
    {:else}
      <div class="text-xs text-[var(--color-muted-foreground)]">
        Resolving log path…
      </div>
    {/if}
  </section>
</div>
