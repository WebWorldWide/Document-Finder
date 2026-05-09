import { createSignal, Show, For, createMemo } from "solid-js";
import {
  Search, X, FolderOpen, BookOpen, Loader2, AlertTriangle, Archive,
  ChevronUp, ChevronDown, Sliders,
} from "lucide-solid";
import LiveResultsView from "./LiveResultsView";
import ModelStatusBadge from "./ModelStatusBadge";
import { runStore } from "@/stores/run";
import { settings, toggleSource } from "@/stores/settings";
import { uiStore } from "@/stores/ui";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, SOURCE_LABELS } from "@/lib/utils";

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
  const [showOptions, setShowOptions] = createSignal(false);

  const rs = () => runStore.state;
  const hasRunResults = createMemo(
    () =>
      rs().running ||
      rs().candidates.length > 0 ||
      Object.keys(rs().inFlight).length > 0 ||
      rs().completed.length > 0
  );
  const issueCount = () =>
    rs().sourceIssues.length +
    rs().completed.filter((c) => c.status === "failed").length;
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
      {/* Header — collapses to a compact strip once a run is in progress */}
      <Show
        when={hasRunResults()}
        fallback={
          <FullHeader
            query={query()}
            setQuery={setQuery}
            onSearch={handleSearch}
            running={rs().running}
            showOptions={showOptions()}
            setShowOptions={setShowOptions}
          />
        }
      >
        <CompactHeader
          query={query()}
          setQuery={setQuery}
          onSearch={handleSearch}
          running={rs().running}
          showOptions={showOptions()}
          setShowOptions={setShowOptions}
        />
      </Show>

      {/* Optional collapsed-options panel */}
      <Show when={showOptions()}>
        <div class="border-b border-[var(--color-border)] bg-[var(--color-card)] px-4 py-3 space-y-2">
          <div class="flex flex-wrap items-center gap-1">
            <span class="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-muted-foreground)] mr-2">
              Sources
            </span>
            <For each={ALL_SOURCES}>
              {(src) => {
                const active = () => settings.selectedSources.includes(src);
                return (
                  <button
                    onClick={() => toggleSource(src)}
                    class="rounded-full border px-2.5 py-0.5 text-[10px] font-medium transition-colors"
                    classList={{
                      "border-transparent text-white": active(),
                      "border-[var(--color-border)] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)]":
                        !active(),
                    }}
                    style={
                      active()
                        ? { "background-color": `var(--color-source-${src})` }
                        : {}
                    }
                  >
                    {SOURCE_LABELS[src]}
                  </button>
                );
              }}
            </For>
          </div>
          <Show when={settings.selectedSources.length === 0}>
            <p class="text-[10px] text-[var(--color-destructive)]">
              Select at least one source to search.
            </p>
          </Show>
        </div>
      </Show>

      {/* Body — the live results view OR a friendly welcome */}
      <div class="flex-1 overflow-hidden">
        <Show
          when={hasRunResults()}
          fallback={
            <WelcomeBody
              onPickExample={(ex) => setQuery(ex)}
            />
          }
        >
          <div class="flex h-full flex-col">
            {/* Status row + post-run actions */}
            <div class="flex flex-wrap items-center gap-3 border-b border-[var(--color-border)] px-4 py-2 text-[12px]">
              <span class="text-[var(--color-muted-foreground)]">
                <strong class="text-[var(--color-foreground)]">{rs().found}</strong>{" "}
                found
              </span>
              <span class="text-[var(--color-muted-foreground)]">
                <strong class="text-[var(--color-success)]">{rs().done}</strong>{" "}
                saved
              </span>
              <Show when={rs().failed > 0}>
                <span class="text-[var(--color-muted-foreground)]">
                  <strong class="text-[var(--color-destructive)]">
                    {rs().failed}
                  </strong>{" "}
                  failed
                </span>
              </Show>
              <ModelStatusBadge />

              <Show when={rs().total > 0}>
                <span class="ml-auto text-[10px] font-mono text-[var(--color-muted-foreground)]">
                  {runStore.overallPct}%
                </span>
                <div class="h-1 w-32 overflow-hidden rounded-full bg-[var(--color-border)]">
                  <div
                    class="h-full rounded-full bg-[var(--color-primary)] transition-all duration-500"
                    style={{ width: `${runStore.overallPct}%` }}
                  />
                </div>
              </Show>

              <Show when={rs().folder && !rs().running}>
                <div class="flex items-center gap-1.5">
                  <button
                    onClick={handleExport}
                    disabled={exporting()}
                    class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2 py-1 text-[11px] font-medium hover:bg-[var(--color-accent)] transition-colors disabled:opacity-50"
                  >
                    <Show when={exporting()} fallback={<Archive size={11} />}>
                      <Loader2 size={11} class="animate-spin" />
                    </Show>
                    Export ZIP
                  </button>
                  <button
                    onClick={handleOpenLibrary}
                    class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2 py-1 text-[11px] font-medium hover:bg-[var(--color-accent)] transition-colors"
                  >
                    <BookOpen size={11} />
                    Library
                  </button>
                  <button
                    onClick={() => rs().folder && api.revealInFinder(rs().folder!)}
                    class="flex items-center gap-1 rounded-md border border-[var(--color-border)] px-2 py-1 text-[11px] font-medium hover:bg-[var(--color-accent)] transition-colors"
                  >
                    <FolderOpen size={11} />
                    Folder
                  </button>
                </div>
              </Show>
            </div>

            {/* Fatal error banner */}
            <Show when={rs().fatalError}>
              <div class="flex items-start justify-between gap-2 border-b border-[var(--color-destructive)]/30 bg-[var(--color-destructive)]/5 px-4 py-2 text-[12px] text-[var(--color-destructive)]">
                <span>
                  <strong>Error:</strong> {rs().fatalError}
                </span>
                <button
                  onClick={() => runStore.clearFatalError()}
                  aria-label="Dismiss"
                  class="shrink-0 rounded p-0.5 hover:bg-[var(--color-destructive)]/10"
                >
                  <X size={11} />
                </button>
              </div>
            </Show>

            {/* Export status banners */}
            <Show when={exportedTo()}>
              <div
                class="flex items-start justify-between gap-2 border-b px-4 py-2 text-[12px]"
                style={{
                  "border-color":
                    "color-mix(in oklch, var(--color-success) 30%, transparent)",
                  "background-color": "var(--color-success-bg)",
                  color: "var(--color-success-fg)",
                }}
              >
                <span>
                  Exported to <code class="text-[11px]">{exportedTo()}</code>
                </span>
                <button onClick={() => setExportedTo(null)} aria-label="Dismiss">
                  <X size={11} />
                </button>
              </div>
            </Show>

            {/* The actual results lanes */}
            <div class="flex-1 overflow-hidden">
              <LiveResultsView />
            </div>

            {/* Source-issue panel — collapsed by default */}
            <Show when={hasIssues()}>
              <div class="border-t border-[var(--color-border)]">
                <button
                  onClick={() => setShowIssues((v) => !v)}
                  aria-expanded={showIssues()}
                  class="flex w-full items-center justify-between px-4 py-2 text-[12px] font-medium"
                >
                  <span class="flex items-center gap-2">
                    <AlertTriangle size={12} class="text-amber-500" />
                    {issueCount() === 1 ? "1 issue" : `${issueCount()} issues`}
                  </span>
                  <span class="text-[var(--color-muted-foreground)]">
                    <Show when={showIssues()} fallback={<ChevronDown size={12} />}>
                      <ChevronUp size={12} />
                    </Show>
                  </span>
                </button>
                <Show when={showIssues()}>
                  <div class="space-y-1 border-t border-[var(--color-border)] px-4 pb-3 pt-2">
                    <For each={rs().sourceIssues}>
                      {(issue) => (
                        <div class="flex gap-2 text-[11px]">
                          <span class="shrink-0 font-medium text-amber-600">
                            {issue.source}
                          </span>
                          <span class="text-[var(--color-muted-foreground)]">
                            {issue.error}
                          </span>
                        </div>
                      )}
                    </For>
                    <For each={failedItems()}>
                      {(item) => (
                        <div class="flex gap-2 text-[11px]">
                          <span class="shrink-0 font-medium text-[var(--color-destructive)]">
                            {item.title.slice(0, 40)}
                          </span>
                          <span class="text-[var(--color-muted-foreground)]">
                            {item.error}
                          </span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
}

// ----- Sub-components -------------------------------------------------------

function FullHeader(props: {
  query: string;
  setQuery: (v: string) => void;
  onSearch: () => void;
  running: boolean;
  showOptions: boolean;
  setShowOptions: (v: boolean) => void;
}) {
  return (
    <div class="border-b border-[var(--color-border)] p-5 pt-10 space-y-4">
      <div class="relative">
        <textarea
          class="w-full resize-none rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] px-4 py-3 pr-12 text-sm leading-relaxed outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors placeholder:text-[var(--color-muted-foreground)]"
          placeholder="What are you looking for? (Ctrl+Enter to search)"
          rows={2}
          value={props.query}
          onInput={(e) => props.setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              props.onSearch();
            }
          }}
        />
        <div class="absolute right-3 top-3 text-[var(--color-muted-foreground)]">
          <Search size={16} />
        </div>
      </div>

      <div class="flex flex-wrap gap-1.5">
        <For each={EXAMPLES}>
          {(ex) => (
            <button
              onClick={() => props.setQuery(ex)}
              class="rounded-full border border-[var(--color-border)] px-3 py-1 text-[11px] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)] transition-colors"
            >
              {ex}
            </button>
          )}
        </For>
      </div>

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
                  "border-[var(--color-border)] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)]":
                    !active(),
                }}
                style={
                  active() ? { "background-color": `var(--color-source-${src})` } : {}
                }
              >
                {SOURCE_LABELS[src]}
              </button>
            );
          }}
        </For>
      </div>

      <Show when={settings.selectedSources.length === 0}>
        <p class="text-xs text-[var(--color-destructive)]">
          Select at least one source to search.
        </p>
      </Show>

      <div class="flex items-center gap-3">
        <button
          onClick={props.onSearch}
          disabled={
            !props.query.trim() ||
            settings.selectedSources.length === 0 ||
            props.running
          }
          class="flex items-center gap-2 rounded-lg bg-[var(--color-primary)] px-5 py-2 text-sm font-medium text-white hover:opacity-90 transition-opacity disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <Search size={14} />
          Find Documents
        </button>
      </div>
    </div>
  );
}

function CompactHeader(props: {
  query: string;
  setQuery: (v: string) => void;
  onSearch: () => void;
  running: boolean;
  showOptions: boolean;
  setShowOptions: (v: boolean) => void;
}) {
  return (
    <div class="flex items-center gap-2 border-b border-[var(--color-border)] bg-[var(--color-card)] px-4 py-2">
      <div class="relative flex-1">
        <input
          type="text"
          class="w-full rounded-md border border-[var(--color-border)] bg-transparent px-3 py-1.5 pr-8 text-[13px] outline-none focus:border-[var(--color-primary)] focus:ring-2 focus:ring-[var(--color-primary)]/20 transition-colors"
          placeholder="Refine query…"
          value={props.query}
          onInput={(e) => props.setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              props.onSearch();
            }
          }}
        />
        <div class="absolute right-2.5 top-1/2 -translate-y-1/2 text-[var(--color-muted-foreground)]">
          <Search size={12} />
        </div>
      </div>

      <button
        onClick={() => props.setShowOptions(!props.showOptions)}
        aria-expanded={props.showOptions}
        title="Sources & options"
        class="rounded-md border border-[var(--color-border)] p-1.5 text-[var(--color-muted-foreground)] hover:bg-[var(--color-accent)] transition-colors"
      >
        <Sliders size={12} />
      </button>

      <Show
        when={!props.running}
        fallback={
          <button
            onClick={() => api.cancelRun()}
            class="flex items-center gap-1 rounded-md border border-[var(--color-destructive)] px-2.5 py-1.5 text-[12px] font-medium text-[var(--color-destructive)] hover:bg-[var(--color-destructive)] hover:text-white transition-colors"
          >
            <X size={11} />
            Cancel
          </button>
        }
      >
        <button
          onClick={props.onSearch}
          disabled={
            !props.query.trim() || settings.selectedSources.length === 0
          }
          class="flex items-center gap-1 rounded-md bg-[var(--color-primary)] px-3 py-1.5 text-[12px] font-medium text-white hover:opacity-90 transition-opacity disabled:opacity-40"
        >
          <Search size={11} />
          New Search
        </button>
      </Show>
    </div>
  );
}

function WelcomeBody(props: { onPickExample: (ex: string) => void }) {
  return (
    <div class="flex h-full items-center justify-center p-6">
      <div class="max-w-md text-center space-y-4">
        <p class="text-sm text-[var(--color-muted-foreground)]">
          Pick a source set above and run a search. Live results, ranking, and
          downloads will fill this view.
        </p>
        <div class="flex flex-wrap justify-center gap-1.5">
          <For each={EXAMPLES.slice(0, 3)}>
            {(ex) => (
              <button
                onClick={() => props.onPickExample(ex)}
                class="rounded-full border border-[var(--color-border)] px-3 py-1 text-[11px] text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)] hover:text-[var(--color-primary)]"
              >
                {ex}
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
