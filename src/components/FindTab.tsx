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
import { ALL_SOURCES, META_SEARCH_COVERED, SOURCE_LABELS, type SourceId } from "@/lib/utils";

// Top-level toggle row hides the six engines covered by `meta_search` to
// avoid clutter — they're still selectable from the "Individual engines"
// expander when meta_search is off or the user wants surgical control.
const PRIMARY_SOURCES: readonly SourceId[] = ALL_SOURCES.filter(
  (s) => !META_SEARCH_COVERED.includes(s)
);

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
        <div class="surface-pressed-sm mx-4 mb-2 px-4 py-3 space-y-2">
          <div class="flex flex-wrap items-center gap-1.5">
            <span class="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-foreground-muted)] mr-2">
              Sources
            </span>
            <For each={PRIMARY_SOURCES}>
              {(src) => {
                const active = () => settings.selectedSources.includes(src);
                return (
                  <button
                    onClick={() => toggleSource(src)}
                    class="tag-pill px-2.5 py-0.5 text-[10px] font-medium"
                    classList={{ "is-active": active() }}
                    style={
                      active()
                        ? {
                            "background-color": `var(--color-source-${src})`,
                            color: "white",
                          }
                        : { color: "var(--color-foreground-muted)" }
                    }
                  >
                    {SOURCE_LABELS[src]}
                  </button>
                );
              }}
            </For>
          </div>
          <details>
            <summary class="cursor-pointer text-[10px] font-medium text-[var(--color-foreground-muted)] hover:text-[var(--color-foreground)]">
              Individual web engines (advanced)
            </summary>
            <div class="mt-2 flex flex-wrap items-center gap-1.5">
              <For each={META_SEARCH_COVERED}>
                {(src) => {
                  const active = () => settings.selectedSources.includes(src);
                  return (
                    <button
                      onClick={() => toggleSource(src)}
                      class="tag-pill px-2.5 py-0.5 text-[10px] font-medium"
                      classList={{ "is-active": active() }}
                      style={
                        active()
                          ? {
                              "background-color": `var(--color-source-${src})`,
                              color: "white",
                            }
                          : { color: "var(--color-foreground-muted)" }
                      }
                    >
                      {SOURCE_LABELS[src]}
                    </button>
                  );
                }}
              </For>
            </div>
          </details>
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
            {/* Status row + post-run actions — raised pills on canvas */}
            <div class="flex flex-wrap items-center gap-2 px-4 py-3 text-[12px]">
              <span class="surface-raised-xs px-2.5 py-1 text-[var(--color-foreground-muted)]" style={{ "border-radius": "var(--radius-pill)" }}>
                <strong class="text-[var(--color-foreground)]">{rs().found}</strong>{" "}
                found
              </span>
              <span class="surface-raised-xs px-2.5 py-1 text-[var(--color-foreground-muted)]" style={{ "border-radius": "var(--radius-pill)" }}>
                <strong style={{ color: "var(--color-success)" }}>{rs().done}</strong>{" "}
                saved
              </span>
              <Show when={rs().failed > 0}>
                <span class="surface-raised-xs px-2.5 py-1 text-[var(--color-foreground-muted)]" style={{ "border-radius": "var(--radius-pill)" }}>
                  <strong class="text-[var(--color-destructive)]">
                    {rs().failed}
                  </strong>{" "}
                  failed
                </span>
              </Show>
              <ModelStatusBadge />

              <Show when={rs().total > 0}>
                <span class="ml-auto text-[10px] font-mono text-[var(--color-foreground-muted)]">
                  {runStore.overallPct}%
                </span>
                <div class="surface-pressed-sm h-1.5 w-32 overflow-hidden">
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
                    class="btn-tactile flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium"
                  >
                    <Show when={exporting()} fallback={<Archive size={11} />}>
                      <Loader2 size={11} class="animate-spin" />
                    </Show>
                    Export ZIP
                  </button>
                  <button
                    onClick={handleOpenLibrary}
                    class="btn-tactile flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium"
                  >
                    <BookOpen size={11} />
                    Library
                  </button>
                  <button
                    onClick={() => rs().folder && api.revealInFinder(rs().folder!)}
                    class="btn-tactile flex items-center gap-1 px-2.5 py-1 text-[11px] font-medium"
                  >
                    <FolderOpen size={11} />
                    Folder
                  </button>
                </div>
              </Show>
            </div>

            {/* Fatal error banner */}
            <Show when={rs().fatalError}>
              <div class="surface-raised-sm mx-4 mb-2 flex items-start justify-between gap-2 px-3 py-2 text-[12px] text-[var(--color-destructive)]">
                <span>
                  <strong>Error:</strong> {rs().fatalError}
                </span>
                <button
                  onClick={() => runStore.clearFatalError()}
                  aria-label="Dismiss"
                  class="btn-tactile shrink-0 p-1"
                >
                  <X size={11} />
                </button>
              </div>
            </Show>

            {/* Export status banners */}
            <Show when={exportedTo()}>
              <div
                class="surface-raised-sm mx-4 mb-2 flex items-start justify-between gap-2 px-3 py-2 text-[12px]"
                style={{
                  color: "var(--color-success-fg)",
                  "background-color": "var(--color-success-bg)",
                }}
              >
                <span>
                  Exported to <code class="text-[11px]">{exportedTo()}</code>
                </span>
                <button
                  onClick={() => setExportedTo(null)}
                  aria-label="Dismiss"
                  class="btn-tactile shrink-0 p-1"
                >
                  <X size={11} />
                </button>
              </div>
            </Show>

            {/* The actual results lanes */}
            <div class="flex-1 overflow-hidden">
              <LiveResultsView />
            </div>

            {/* Source-issue panel — pressed pill at the bottom of the canvas */}
            <Show when={hasIssues()}>
              <div class="surface-pressed-sm mx-4 mb-3 px-3">
                <button
                  onClick={() => setShowIssues((v) => !v)}
                  aria-expanded={showIssues()}
                  class="flex w-full items-center justify-between py-2 text-[12px] font-medium"
                >
                  <span class="flex items-center gap-2">
                    <AlertTriangle size={12} class="text-amber-500" />
                    {issueCount() === 1 ? "1 issue" : `${issueCount()} issues`}
                  </span>
                  <span class="text-[var(--color-foreground-muted)]">
                    <Show when={showIssues()} fallback={<ChevronDown size={12} />}>
                      <ChevronUp size={12} />
                    </Show>
                  </span>
                </button>
                <Show when={showIssues()}>
                  <div class="space-y-1 pb-3 pt-1">
                    <For each={rs().sourceIssues}>
                      {(issue) => (
                        <div class="flex gap-2 text-[11px]">
                          <span class="shrink-0 font-medium text-amber-600">
                            {issue.source}
                          </span>
                          <span class="text-[var(--color-foreground-muted)]">
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
                          <span class="text-[var(--color-foreground-muted)]">
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
    <div class="p-6 pt-10 space-y-5">
      {/* Inset query input */}
      <div class="relative">
        <textarea
          class="surface-input w-full resize-none px-4 py-3 pr-12 text-sm leading-relaxed outline-none placeholder:text-[var(--color-foreground-muted)]"
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
        <div class="absolute right-3 top-3 text-[var(--color-foreground-muted)]">
          <Search size={16} />
        </div>
      </div>

      {/* Example pills */}
      <div class="flex flex-wrap gap-1.5">
        <For each={EXAMPLES}>
          {(ex) => (
            <button
              onClick={() => props.setQuery(ex)}
              class="pill-toggle px-3 py-1 text-[11px] text-[var(--color-foreground-muted)] hover:text-[var(--color-primary)]"
            >
              {ex}
            </button>
          )}
        </For>
      </div>

      {/* Source toggles — flat outlined tags; active fills with source color */}
      <div class="space-y-2">
        <div class="flex flex-wrap gap-2">
          <For each={PRIMARY_SOURCES}>
            {(src) => {
              const active = () => settings.selectedSources.includes(src);
              return (
                <button
                  onClick={() => toggleSource(src)}
                  class="tag-pill px-3 py-1 text-[11px] font-medium"
                  classList={{ "is-active": active() }}
                  style={
                    active()
                      ? {
                          "background-color": `var(--color-source-${src})`,
                          color: "white",
                        }
                      : { color: "var(--color-foreground-muted)" }
                  }
                >
                  {SOURCE_LABELS[src]}
                </button>
              );
            }}
          </For>
        </div>
        <details>
          <summary class="cursor-pointer text-[11px] font-medium text-[var(--color-foreground-muted)] hover:text-[var(--color-foreground)]">
            Individual web engines (advanced)
          </summary>
          <div class="mt-2 flex flex-wrap gap-2">
            <For each={META_SEARCH_COVERED}>
              {(src) => {
                const active = () => settings.selectedSources.includes(src);
                return (
                  <button
                    onClick={() => toggleSource(src)}
                    class="tag-pill px-3 py-1 text-[11px] font-medium"
                    classList={{ "is-active": active() }}
                    style={
                      active()
                        ? {
                            "background-color": `var(--color-source-${src})`,
                            color: "white",
                          }
                        : { color: "var(--color-foreground-muted)" }
                    }
                  >
                    {SOURCE_LABELS[src]}
                  </button>
                );
              }}
            </For>
          </div>
        </details>
      </div>

      <Show when={settings.selectedSources.length === 0}>
        <p class="text-xs text-[var(--color-destructive)]">
          Select at least one source to search.
        </p>
      </Show>

      {/* Primary action — raised, depresses on click */}
      <div class="flex items-center gap-3">
        <button
          onClick={props.onSearch}
          disabled={
            !props.query.trim() ||
            settings.selectedSources.length === 0 ||
            props.running
          }
          class="btn-tactile flex items-center gap-2 px-6 py-2.5 text-sm font-semibold"
          style={{
            background: "var(--color-primary)",
            color: "white",
          }}
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
    <div class="flex items-center gap-2 px-4 py-3">
      <div class="relative flex-1">
        <input
          type="text"
          class="surface-input w-full px-3 py-1.5 pr-8 text-[13px] outline-none"
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
        <div class="absolute right-2.5 top-1/2 -translate-y-1/2 text-[var(--color-foreground-muted)]">
          <Search size={12} />
        </div>
      </div>

      <button
        onClick={() => props.setShowOptions(!props.showOptions)}
        aria-expanded={props.showOptions}
        title="Sources & options"
        class="btn-tactile p-1.5 text-[var(--color-foreground-muted)]"
        classList={{ "is-active": props.showOptions }}
      >
        <Sliders size={12} />
      </button>

      <Show
        when={!props.running}
        fallback={
          <button
            onClick={() => api.cancelRun()}
            class="btn-tactile flex items-center gap-1 px-2.5 py-1.5 text-[12px] font-medium text-[var(--color-destructive)]"
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
          class="btn-tactile flex items-center gap-1 px-3 py-1.5 text-[12px] font-semibold"
          style={{ background: "var(--color-primary)", color: "white" }}
        >
          <Search size={11} />
          New Search
        </button>
      </Show>
    </div>
  );
}

const PIPELINE_STAGES: { label: string; detail: string }[] = [
  { label: "Discover", detail: "Search 13 academic + web sources in parallel" },
  { label: "Rank", detail: "Cross-source dedup · TF-IDF · RRF · authority" },
  { label: "Filter", detail: "Optional semantic + LLM borderline judging" },
  { label: "Download", detail: "Concurrent fetch · resume · text extraction" },
];

function WelcomeBody(props: { onPickExample: (ex: string) => void }) {
  return (
    <div class="flex h-full items-center justify-center overflow-y-auto p-6 scroll-inset">
      <div class="material-linen border-stitched max-w-2xl w-full p-8 space-y-6">
        <div class="text-center space-y-2">
          <h2 class="text-lg font-semibold text-embossed">Find documents anywhere</h2>
          <p class="text-sm text-[var(--color-foreground-muted)] leading-relaxed">
            Pick a source set, type a query, and watch results stream in.
            <br />Built-in meta-search hits 6 web engines in parallel — no setup.
          </p>
        </div>

        <div class="flex flex-wrap justify-center gap-1.5">
          <For each={EXAMPLES}>
            {(ex) => (
              <button
                onClick={() => props.onPickExample(ex)}
                class="pill-toggle px-3 py-1 text-[11px] text-[var(--color-foreground-muted)] hover:text-[var(--color-primary)]"
              >
                {ex}
              </button>
            )}
          </For>
        </div>

        <div class="grid grid-cols-1 sm:grid-cols-4 gap-2">
          <For each={PIPELINE_STAGES}>
            {(stage, i) => (
              <div class="surface-raised-subtle surface-bevel-sm p-3 text-center">
                <div class="mb-1 flex items-center justify-center gap-1">
                  <span class="font-mono text-[10px] text-[var(--color-foreground-muted)]">
                    {String(i() + 1).padStart(2, "0")}
                  </span>
                  <span class="text-[11px] font-semibold text-embossed">
                    {stage.label}
                  </span>
                </div>
                <p class="text-[9.5px] leading-snug text-[var(--color-foreground-muted)]">
                  {stage.detail}
                </p>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
