<script lang="ts">
  import { Check, Download, X } from "lucide-svelte";
  import { runStore } from "@/stores/run.svelte";
  import { formatBytes, SOURCE_LABELS } from "@/lib/utils";

  let inFlight = $derived(Object.values(runStore.inFlight));
  let completed = $derived(runStore.completed.slice(-50).reverse());
</script>

{#if runStore.running || inFlight.length > 0 || completed.length > 0}
  <div class="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] overflow-hidden">
    {#if inFlight.length > 0}
      <div class="border-b border-[var(--color-border)]">
        <div class="flex items-center gap-2 px-4 py-2 text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
          <Download class="h-3 w-3" />
          <span>Downloading ({inFlight.length})</span>
        </div>
        <ul class="divide-y divide-[var(--color-border)]">
          {#each inFlight as d (d.url)}
            {@const pct = d.total > 0 ? Math.min(100, (d.downloaded / d.total) * 100) : 0}
            <li class="flex items-center gap-3 px-4 py-2 text-sm">
              <span class="px-1.5 py-0.5 rounded text-[10px] bg-[var(--color-primary)]/10 text-[var(--color-primary)] shrink-0">
                {SOURCE_LABELS[d.source] ?? d.source}
              </span>
              <span class="flex-1 truncate text-[var(--color-foreground)]" title={d.title}>
                {d.title || "Untitled"}
              </span>
              <span class="w-32 shrink-0 font-mono text-xs tabular-nums text-[var(--color-muted-foreground)]">
                {formatBytes(d.downloaded)}
                {#if d.total > 0}
                  / {formatBytes(d.total)}
                {/if}
              </span>
              <div class="h-1 w-24 shrink-0 overflow-hidden rounded-full bg-[var(--color-muted)]">
                <div
                  class="h-full rounded-full bg-[var(--color-primary)] transition-[width] duration-150"
                  style="width: {d.total > 0 ? pct + '%' : '30%'}"
                ></div>
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if completed.length > 0}
      <div>
        <div class="px-4 py-2 text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
          Recent ({completed.length})
        </div>
        <ul class="max-h-80 divide-y divide-[var(--color-border)] overflow-auto">
          {#each completed as c (c.url)}
            <li class="flex items-center gap-3 px-4 py-1.5 text-sm">
              {#if c.status === "done"}
                <Check class="h-3.5 w-3.5 shrink-0 text-emerald-600" />
              {:else}
                <X class="h-3.5 w-3.5 shrink-0 text-rose-600" />
              {/if}
              <span class="px-1.5 py-0.5 rounded text-[10px] bg-[var(--color-primary)]/10 text-[var(--color-primary)] shrink-0">
                {SOURCE_LABELS[c.source] ?? c.source}
              </span>
              <span class="flex-1 truncate {c.status === 'failed' ? 'text-[var(--color-muted-foreground)]' : 'text-[var(--color-foreground)]'}" title={c.title}>
                {c.title || "Untitled"}
              </span>
              {#if c.status === "failed" && c.error}
                <span class="max-w-[40%] shrink-0 truncate text-xs text-rose-600/80" title={c.error}>
                  {c.error}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
{/if}
