<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import {
    ArrowRight,
    CheckCheck,
    FileArchive,
    FolderOpen,
    Library as LibraryIcon,
    Loader2,
  } from "lucide-svelte";
  import { api, type LibraryInfo } from "@/lib/tauri";
  import { formatBytes } from "@/lib/utils";
  import { settingsStore } from "@/stores/settings.svelte";
  import { uiStore } from "@/stores/ui.svelte";

  let libraries = $state<LibraryInfo[]>([]);
  let loading = $state(false);
  let exportingPath = $state<string | null>(null);
  let justExported = $state<string | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    if (!settingsStore.libraryRoot) {
      api.defaultLibraryDir().then((d) => settingsStore.set({ libraryRoot: d.library_root }));
      return;
    }
    loadLibraries();
  });

  async function loadLibraries() {
    loading = true;
    try {
      libraries = await api.listLibraries(settingsStore.libraryRoot);
    } catch {
      libraries = [];
    } finally {
      loading = false;
    }
  }

  async function exportZip(l: LibraryInfo) {
    error = null;
    const dest = await save({
      defaultPath: `${l.name}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!dest) return;
    exportingPath = l.path;
    try {
      const result = await api.exportLibraryZip(l.path, dest);
      justExported = result.dest;
      setTimeout(() => (justExported = null), 4000);
      await api.revealInFinder(result.dest);
    } catch (e) {
      error = String(e);
    } finally {
      exportingPath = null;
    }
  }
</script>

<div class="space-y-5">
  <header>
    <h1 class="text-2xl font-semibold">Library</h1>
  </header>

  {#if error}
    <div class="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
      {error}
    </div>
  {/if}

  {#if justExported}
    <div class="flex items-center gap-2 rounded-md border border-emerald-500/40 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
      <CheckCheck class="h-4 w-4" />
      Exported to {justExported}
    </div>
  {/if}

  {#if loading}
    <div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-8 text-center text-sm text-[var(--color-muted-foreground)]">
      Loading…
    </div>
  {:else if libraries.length === 0}
    <div class="rounded-xl border border-dashed border-[var(--color-border)] bg-[var(--color-card)] p-10 text-center">
      <LibraryIcon class="mx-auto mb-3 h-8 w-8 text-[var(--color-muted-foreground)]" />
      <h2 class="text-base font-medium">No libraries yet</h2>
      <p class="mx-auto mt-1 max-w-md text-sm text-[var(--color-muted-foreground)]">
        Run a search in Discover and we'll save the downloaded documents here.
      </p>
      <button
        class="mt-4 px-6 py-2 rounded-lg bg-[var(--color-primary)] text-white font-medium hover:opacity-90 inline-flex items-center gap-2"
        onclick={() => uiStore.setView("find")}
      >
        Go to Discover
        <ArrowRight class="h-4 w-4" />
      </button>
    </div>
  {:else}
    <div class="grid grid-cols-1 gap-2 md:grid-cols-2">
      {#each libraries as l (l.path)}
        {@const isActive = uiStore.activeLibrary?.path === l.path}
        {@const isExporting = exportingPath === l.path}
        <div
          class="flex flex-col gap-3 rounded-xl border p-4 text-left transition-colors {isActive ? 'border-[var(--color-primary)]/60 bg-[var(--color-primary)]/5' : 'border-[var(--color-border)] bg-[var(--color-card)]'}"
        >
          <button
            type="button"
            onclick={() => uiStore.setActiveLibrary(l)}
            class="text-left group"
          >
            <div class="truncate text-sm font-medium group-hover:text-[var(--color-primary)]" title={l.query}>
              {l.query}
            </div>
            <div class="mt-1 flex items-center gap-2 text-xs text-[var(--color-muted-foreground)]">
              <span>{l.n_docs} document{l.n_docs === 1 ? "" : "s"}</span>
              {#if l.size_bytes > 0}
                <span>·</span>
                <span>{formatBytes(l.size_bytes)}</span>
              {/if}
            </div>
          </button>

          <div class="flex flex-wrap items-center gap-2">
            <button
              class="px-3 py-1.5 rounded-lg bg-[var(--color-primary)] text-white text-xs font-medium hover:opacity-90 disabled:opacity-50 inline-flex items-center gap-2"
              onclick={() => exportZip(l)}
              disabled={isExporting}
            >
              {#if isExporting}
                <Loader2 class="h-3.5 w-3.5 animate-spin" />
                Zipping…
              {:else}
                <FileArchive class="h-3.5 w-3.5" />
                Export ZIP
              {/if}
            </button>
            <button
              class="px-3 py-1.5 rounded-lg border border-[var(--color-border)] text-xs font-medium hover:bg-[var(--color-accent)] inline-flex items-center gap-2"
              onclick={() => api.revealInFinder(l.path)}
            >
              <FolderOpen class="h-3.5 w-3.5" />
              Show
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
