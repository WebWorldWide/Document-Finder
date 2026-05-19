import { createSignal, Show, createMemo, For } from "solid-js";
import { Search, X, Sparkles, AlertTriangle } from "lucide-solid";
import RunCard from "./RunCard";
import SourcePanel from "./SourcePanel";
import { runStore } from "@/stores/run";
import { settings } from "@/stores/settings";
import { uiStore } from "@/stores/ui";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

/// Discover view — the hero of the app.
///
/// Layout (top to bottom):
///   - .df-canvas-head: <h1>Discover</h1> + head-stats (libraries / docs / on-disk)
///   - hero query textarea (.df-query-wrap) with a bottom bar (sparkle hint,
///     ⌘↵ kbd chips, Find/Stop button)
///   - SourcePanel (rich 2-col grid)
///   - RunCard (stats, progress, speed sparkline, telemetry, stream)
///   - issues disclosure when source_errors fire
export default function FindTab() {
  const [query, setQuery] = createSignal("");
  const [showIssues, setShowIssues] = createSignal(false);

  const rs = () => runStore.state;
  const running = () => rs().running;
  const hasResults = () =>
    running() ||
    rs().completed.length > 0 ||
    Object.keys(rs().inFlight).length > 0;

  // Head stats — read from uiStore.knownLibraries which App refreshes.
  const libCount = createMemo(() => uiStore.knownLibraries.length);
  const docCount = createMemo(() =>
    uiStore.knownLibraries.reduce((s, l) => s + l.n_docs, 0),
  );
  const totalBytes = createMemo(() =>
    uiStore.knownLibraries.reduce((s, l) => s + l.size_bytes, 0),
  );

  const issueCount = () =>
    rs().sourceIssues.length +
    rs().completed.filter((c) => c.status === "failed").length;
  const failedItems = () =>
    rs().completed.filter((c) => c.status === "failed");

  async function handleSearch() {
    if (!query().trim() || running()) return;
    if (settings.selectedSources.length === 0) return;
    await runStore.startSearch(query());
  }

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <h1 class="df-canvas-title">Discover</h1>
        <div class="df-headstats">
          <div class="df-headstat">
            <span class="df-headstat-num">{libCount()}</span>
            <span class="df-headstat-label">libraries</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{docCount()}</span>
            <span class="df-headstat-label">docs saved</span>
          </div>
          <div class="df-headstat">
            <span class="df-headstat-num">{formatBytes(totalBytes())}</span>
            <span class="df-headstat-label">on disk</span>
          </div>
        </div>
      </div>

      <div class="df-canvas-body">
        {/* Hero query input */}
        <div class="df-query-wrap">
          <textarea
            class="df-query-input"
            rows={2}
            placeholder="What are you researching?"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                handleSearch();
              }
            }}
          />
          <div class="df-query-bar">
            <span class="df-query-hint">
              <Sparkles size={12} /> Natural language — we&rsquo;ll split it into sub-queries
            </span>
            <span style={{ flex: 1 }} />
            <span class="df-query-hint">
              <span class="df-kbd">⌘</span>
              <span class="df-kbd">↵</span> to search
            </span>
            <Show
              when={!running()}
              fallback={
                <button
                  class="df-btn danger"
                  onClick={() => api.cancelRun()}
                  title="Stop the running search"
                >
                  <X size={12} /> Stop
                </button>
              }
            >
              <button
                class="df-btn accent"
                onClick={handleSearch}
                disabled={!query().trim() || settings.selectedSources.length === 0}
              >
                <Search size={13} /> Find documents
              </button>
            </Show>
          </div>
        </div>

        {/* Sources panel */}
        <div class="df-section-head">
          <span class="df-section-label">Sources</span>
          <span class="df-section-meta">
            Open-access platforms — fan out concurrent metasearch.
          </span>
        </div>
        <SourcePanel />
        <Show when={settings.selectedSources.length === 0}>
          <p style={{
            "margin-top": "10px",
            "font-size": "12px",
            color: "var(--bad)",
          }}>
            Enable at least one source to start a search.
          </p>
        </Show>

        {/* Fatal error banner */}
        <Show when={rs().fatalError}>
          <div class="df-banner bad" style={{ "margin-top": "var(--pad-4)" }}>
            <AlertTriangle size={14} />
            <div class="df-banner-body">
              <strong>Search error.</strong> {rs().fatalError}
            </div>
            <button
              class="df-banner-x"
              onClick={() => runStore.clearFatalError()}
              aria-label="Dismiss"
            >
              <X size={12} />
            </button>
          </div>
        </Show>

        {/* Run telemetry */}
        <Show when={hasResults()}>
          <RunCard />
        </Show>

        {/* Issues disclosure */}
        <Show when={issueCount() > 0}>
          <details
            class="df-issues"
            open={showIssues()}
            onToggle={(e) => setShowIssues(e.currentTarget.open)}
          >
            <summary>
              <AlertTriangle size={13} />
              {issueCount() === 1 ? "1 issue" : `${issueCount()} issues`}
            </summary>
            <ul>
              <For each={rs().sourceIssues}>{(issue) => (
                <li>
                  <code
                    style={{
                      "--src-color": `var(--src-${issue.source.replace(/-/g, "_")})`,
                    } as Record<string, string>}
                  >
                    {issue.source}
                  </code>
                  <span>{issue.error}</span>
                </li>
              )}</For>
              <For each={failedItems()}>{(item) => (
                <li>
                  <code style={{ color: "var(--bad)" }}>
                    {item.title.slice(0, 40)}
                  </code>
                  <span>{item.error ?? "download failed"}</span>
                </li>
              )}</For>
            </ul>
          </details>
        </Show>
      </div>
    </div>
  );
}
