import { createSignal, Show, For } from "solid-js";
import {
  Search, X, Download, FolderOpen, BookOpen, Loader2, AlertTriangle, Archive,
  ChevronUp, ChevronDown
} from "lucide-solid";
import LiveDownloadStream from "./LiveDownloadStream";
import { runStore } from "@/stores/run";
import { settings, toggleSource, saveSettings } from "@/stores/settings";
import { uiStore } from "@/stores/ui";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, SOURCE_LABELS, formatBytes } from "@/lib/utils";

const EXAMPLES = [
  "machine learning survey 2024",
  "climate change adaptation policy",
  "CRISPR gene editing review",
  "quantum computing algorithms",
  "transformer architecture attention",
];

export default function FindTab() {
  const [query, setQuery] = createSignal("");
  const [exporting, setExporting] = createSignal(false);
  const [exportError, setExportError] = createSignal<string | null>(null);
  const [exportedTo, setExportedTo] = createSignal<string | null>(null);
  const [showIssues, setShowIssues] = createSignal(false);

  const rs = () => runStore.state;
  const issueCount = () => rs().sourceIssues.length + rs().completed.filter((c) => c.status === "failed").length;
  const hasIssues = () => issueCount() > 0;
  const failedItems = () => rs().completed.filter((c) => c.status === "failed");

  async function handleSearch() {
    if (!query().trim() || rs().running) return;
    setExportError(null);
    setExportedTo(null);
    await runStore.startSearch(query());
  }

  async function handleExport() {
    setExporting(true);
    setExportError(null);
    try {
      const result = await runStore.exportZip();
      if (result) setExportedTo(result.dest);
    } catch (e) {
      setExportError(String(e));
    } finally {
      setExporting(false);
    }
  }

  async function handleOpenLibrary() {
    if (!rs().folder) return;
    try {
      const info = await api.openLibrary(rs().folder!);
      uiStore.setActiveLibrary(info);
      uiStore.setView("library");
    } catch (e) {
      setExportError(String(e));
    }
  }

  return (
    <div class="flex h-full flex-col overflow-hidden">
      {/* Header area */}
      <div class="border-b border-[var(--color-border)] p-5 pt-10 space-y-4">
        {/* Query input */}
        <div class="relative">
          <textarea
            class="w-full resize-none rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] px-4 py-3 pr-12 text-sm leading-relaxed outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors placeholder:text-[var(--color-muted-foreground)]"
            placeholder="What are you looking for? (Ctrl+Enter to search)"
            rows={2}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                handleSearch();
              }
            }}
          />
          <div class="absolute right-3 top-3 text-[var(--color-muted-foreground)]">
            <Search size={16} />
          </div>
        </div>

        {/* Example queries */}
        <div class="flex flex-wrap gap-1.5">
          <For each={EXAMPLES}>
            {(ex) => (
              <button
                onClick={() => setQuery(ex)}
                class="rounded-full border border-[var(--color-border)] px-3 py-1 text-[11px] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)] transition-colors"
              >
                {ex}
              </button>
            )}
          </For>
        </div>

        {/* Source toggles */}
        <div class="flex flex-wrap gap-1.5">
          <For each={ALL_SOURCES}>
            {(src) => {
              const active = () => settings.selectedSources.includes(src);
              return (
                <button
                  onClick={() => toggleSource(src)}
                  class="rounded-full border px-3 py-1 text-[11px] font-medium transition-colors"
                  classList={{
                    "border-transparent text-white": active(),
                    "border-[var(--color-border)] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)]": !active(),
                  }}
                  style={active() ? { "background-color": `var(--color-source-${src})` } : {}}
                >
                  {SOURCE_LABELS[src]}
                </button>
              );
            }}
          </For>
        </div>

        {/* No-source warning */}
        <Show when={settings.selectedSources.length === 0}>
          <p class="text-xs text-[var(--color-destructive)]">Select at least one source to search.</p>
        </Show>

        {/* Action row */}
        <div class="flex items-center gap-3">
          <Show
            when={!rs().running}
            fallback={
              <button
                onClick={() => api.cancelRun()}
                class="flex items-center gap-2 rounded-lg border border-[var(--color-destructive)] px-4 py-2 text-sm font-medium text-[var(--color-destructive)] hover:bg-[var(--color-destructive)] hover:text-white transition-colors"
              >
                <X size={14} />
                Cancel
              </button>
            }
          >
            <button
              onClick={handleSearch}
              disabled={!query().trim() || settings.selectedSources.length === 0}
              class="flex items-center gap-2 rounded-lg bg-[var(--color-primary)] px-5 py-2 text-sm font-medium text-white hover:opacity-90 transition-opacity disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <Search size={14} />
              Find Documents
            </button>
          </Show>

          {/* Post-run actions */}
          <Show when={rs().folder && !rs().running}>
            <button
              onClick={handleExport}
              disabled={exporting()}
              class="flex items-center gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm font-medium hover:bg-[var(--color-accent)] transition-colors disabled:opacity-50"
            >
              <Show when={exporting()} fallback={<Archive size={14} />}>
                <Loader2 size={14} class="animate-spin" />
              </Show>
              Export ZIP
            </button>
            <button
              onClick={handleOpenLibrary}
              class="flex items-center gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm font-medium hover:bg-[var(--color-accent)] transition-colors"
            >
              <BookOpen size={14} />
              Open Library
            </button>
            <button
              onClick={() => rs().folder && api.revealInFinder(rs().folder!)}
              class="flex items-center gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm font-medium hover:bg-[var(--color-accent)] transition-colors"
            >
              <FolderOpen size={14} />
              Show Folder
            </button>
          </Show>
        </div>
      </div>

      {/* Body — scrollable */}
      <div class="flex-1 overflow-y-auto p-5 space-y-5">
        {/* Stats + progress */}
        <Show when={rs().found > 0 || rs().done > 0 || rs().running}>
          <div class="space-y-3">
            {/* Stat pills */}
            <div class="flex flex-wrap gap-3 text-sm">
              <span class="text-[var(--color-muted-foreground)]">
                <strong class="text-[var(--color-foreground)]">{rs().found}</strong> found
              </span>
              <span class="text-[var(--color-muted-foreground)]">
                <strong class="text-[var(--color-success)]">{rs().done}</strong> saved
              </span>
              <Show when={rs().failed > 0}>
                <span class="text-[var(--color-muted-foreground)]">
                  <strong class="text-[var(--color-destructive)]">{rs().failed}</strong> failed
                </span>
              </Show>
              <Show when={rs().filteredCount > 0}>
                <span class="text-[var(--color-muted-foreground)]">
                  <strong class="text-[var(--color-muted-foreground)]">{rs().filteredCount}</strong> off-topic
                </span>
              </Show>
            </div>

            {/* Progress bar */}
            <Show when={rs().total > 0}>
              <div class="space-y-1">
                <div class="h-1.5 w-full rounded-full bg-[var(--color-border)] overflow-hidden">
                  <div
                    class="h-full rounded-full bg-[var(--color-primary)] transition-all duration-500"
                    style={{ width: `${runStore.overallPct}%` }}
                  />
                </div>
                <p class="text-right text-[10px] text-[var(--color-muted-foreground)]">
                  {runStore.overallPct}%
                </p>
              </div>
            </Show>
          </div>
        </Show>

        {/* Fatal error */}
        <Show when={rs().fatalError}>
          <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-4 text-sm text-[var(--color-destructive)] flex items-start justify-between gap-2">
            <span><strong>Error:</strong> {rs().fatalError}</span>
            <button
              onClick={() => runStore.clearFatalError()}
              aria-label="Dismiss"
              class="shrink-0 rounded hover:bg-[var(--color-destructive)]/10 p-0.5 transition-colors"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        {/* Export success */}
        <Show when={exportedTo()}>
          <div class="rounded-lg border p-3 text-sm flex items-start justify-between gap-2"
            style={{ "border-color": "color-mix(in oklch, var(--color-success) 30%, transparent)", "background-color": "var(--color-success-bg)", "color": "var(--color-success-fg)" }}
          >
            <span>Exported to <code class="text-xs">{exportedTo()}</code></span>
            <button
              onClick={() => setExportedTo(null)}
              aria-label="Dismiss"
              class="shrink-0 rounded p-0.5 transition-colors hover:opacity-70"
            >
              <X size={12} />
            </button>
          </div>
        </Show>
        <Show when={exportError()}>
          <div class="rounded-lg border border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 p-3 text-sm text-[var(--color-destructive)] flex items-start justify-between gap-2">
            <span>Export failed: {exportError()}</span>
            <button
              onClick={() => setExportError(null)}
              aria-label="Dismiss"
              class="shrink-0 rounded hover:bg-[var(--color-destructive)]/10 p-0.5 transition-colors"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        {/* Live download stream */}
        <Show when={Object.keys(rs().inFlight).length > 0 || rs().completed.length > 0}>
          <LiveDownloadStream />
        </Show>

        {/* Issues */}
        <Show when={hasIssues()}>
          <div class="rounded-xl border border-[var(--color-border)]">
            <button
              onClick={() => setShowIssues((v) => !v)}
              aria-expanded={showIssues()}
              class="flex w-full items-center justify-between px-4 py-3 text-sm font-medium"
            >
              <span class="flex items-center gap-2">
                <AlertTriangle size={14} class="text-amber-500" />
                {issueCount() === 1 ? "1 issue" : `${issueCount()} issues`}
              </span>
              <span class="text-[var(--color-muted-foreground)]">
                <Show when={showIssues()} fallback={<ChevronDown size={14} />}>
                  <ChevronUp size={14} />
                </Show>
              </span>
            </button>
            <Show when={showIssues()}>
              <div class="border-t border-[var(--color-border)] px-4 pb-4 pt-3 space-y-2">
                <For each={rs().sourceIssues}>
                  {(issue) => (
                    <div class="flex gap-2 text-xs">
                      <span class="font-medium text-amber-600 shrink-0">{issue.source}</span>
                      <span class="text-[var(--color-muted-foreground)]">{issue.error}</span>
                    </div>
                  )}
                </For>
                <For each={failedItems()}>
                  {(item) => (
                    <div class="flex gap-2 text-xs">
                      <span class="font-medium text-[var(--color-destructive)] shrink-0">{item.title.slice(0, 40)}</span>
                      <span class="text-[var(--color-muted-foreground)]">{item.error}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}
