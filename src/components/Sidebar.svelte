<script lang="ts">
  import { FileSearch, FolderOpen, Library, Settings, Sparkles } from "lucide-svelte";
  import { api } from "@/lib/tauri";
  import { formatBytes } from "@/lib/utils";
  import { uiStore, type View } from "@/stores/ui.svelte";

  const NAV_ITEMS: { id: View; label: string; icon: any }[] = [
    { id: "find", label: "Discover", icon: FileSearch },
    { id: "library", label: "Library", icon: Library },
    { id: "settings", label: "Settings", icon: Settings },
  ];
</script>

<aside class="flex h-full w-60 shrink-0 flex-col border-r border-[var(--color-border)] bg-[var(--color-muted)]">
  <div
    class="flex items-center gap-2 px-4 py-3"
    data-tauri-drag-region
  >
    <div class="flex h-7 w-7 items-center justify-center rounded-md bg-[var(--color-primary)]/15">
      <Sparkles class="h-4 w-4 text-[var(--color-primary)]" />
    </div>
    <div class="text-sm font-semibold leading-none">Document Finder</div>
  </div>

  <div class="px-3 pb-3">
    <div class="mb-1 px-2 text-[10px] font-semibold uppercase tracking-widest text-[var(--color-muted-foreground)]">
      Active library
    </div>
    {#if uiStore.activeLibrary}
      <div class="rounded-md border border-[var(--color-border)] bg-[var(--color-card)] p-2.5">
        <div class="truncate text-sm font-medium" title={uiStore.activeLibrary.query}>
          {uiStore.activeLibrary.query}
        </div>
        <div class="mt-0.5 flex items-center gap-2 text-[11px] text-[var(--color-muted-foreground)]">
          <span>{uiStore.activeLibrary.n_docs} docs</span>
          {#if uiStore.activeLibrary.size_bytes > 0}
            <span>·</span>
            <span>{formatBytes(uiStore.activeLibrary.size_bytes)}</span>
          {/if}
        </div>
        <button
          type="button"
          onclick={() => api.revealInFinder(uiStore.activeLibrary!.path)}
          class="mt-2 flex items-center gap-1.5 text-[11px] text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
        >
          <FolderOpen class="h-3 w-3" />
          Show in Finder
        </button>
      </div>
    {:else}
      <button
        type="button"
        onclick={() => uiStore.setView("library")}
        class="w-full rounded-md border border-dashed border-[var(--color-border)] bg-transparent p-2.5 text-left text-xs text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)]/40 hover:text-[var(--color-foreground)]"
      >
        No library selected — pick one →
      </button>
    {/if}
  </div>

  <nav class="flex flex-col gap-0.5 px-2">
    {#each NAV_ITEMS as item}
      <button
        type="button"
        onclick={() => uiStore.setView(item.id)}
        class="flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors {uiStore.view === item.id ? 'bg-[var(--color-primary)]/10 text-[var(--color-primary)]' : 'text-[var(--color-foreground)]/85 hover:bg-[var(--color-accent)]'}"
      >
        <svelte:component this={item.icon} class="h-4 w-4" />
        <span class="flex-1">{item.label}</span>
      </button>
    {/each}
  </nav>
</aside>
