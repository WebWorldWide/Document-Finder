import { createSignal, createEffect, onCleanup, For, Show } from "solid-js";
import { Copy, Trash2, FolderOpen, RefreshCw } from "lucide-solid";
import {
  log, formatEntry,
  type LogEntry, type LogLevel,
} from "@/lib/log";
import { api, type LogInfo } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

const LEVEL_FILTERS: readonly { id: LogLevel | "all"; label: string }[] = [
  { id: "all",   label: "All" },
  { id: "info",  label: "Info" },
  { id: "warn",  label: "Warn" },
  { id: "error", label: "Errors" },
] as const;

const LEVEL_RANK: Record<LogLevel, number> = {
  debug: 0, info: 1, warn: 2, error: 3,
};

/// Live-updating view of the frontend log ring buffer plus access to the
/// Rust-side runlog file. Subscribes to log mutations via log.subscribe()
/// and re-renders on every new entry. Filter by minimum level; clear /
/// copy the buffer to clipboard for bug reports.
export default function LogsPanel() {
  const [entries, setEntries] = createSignal<LogEntry[]>(log.tail());
  const [filter, setFilter] = createSignal<LogLevel | "all">("all");
  const [logInfo, setLogInfo] = createSignal<LogInfo | null>(null);
  const [copied, setCopied] = createSignal(false);

  createEffect(() => {
    const unsub = log.subscribe(() => setEntries(log.tail()));
    onCleanup(unsub);
  });

  async function refreshLogInfo() {
    try {
      setLogInfo(await api.runLogInfo());
    } catch (e) {
      log.error("settings", "runLogInfo failed", e);
    }
  }
  refreshLogInfo();

  const filtered = () => {
    const f = filter();
    if (f === "all") return entries();
    const min = LEVEL_RANK[f];
    return entries().filter((e) => LEVEL_RANK[e.level] >= min);
  };

  async function copyAll() {
    const text = entries().map(formatEntry).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
      log.info("ui", `copied ${entries().length} log lines to clipboard`);
    } catch (e) {
      log.error("ui", "clipboard write failed", e);
    }
  }

  return (
    <section class="df-section">
      <h2>Logs</h2>
      <p class="hint">
        Live frontend log + Rust backend runlog path. Filter by level, copy
        for bug reports, or open the runlog in Finder.
      </p>

      <div style={{
        display: "flex", "align-items": "center",
        gap: "var(--pad-3)",
        "margin-bottom": "var(--pad-3)",
      }}>
        <div class="df-theme-radio" role="radiogroup" aria-label="Log level filter">
          <For each={LEVEL_FILTERS}>{(f) => (
            <button
              aria-pressed={filter() === f.id}
              onClick={() => setFilter(f.id)}
            >
              {f.label}
            </button>
          )}</For>
        </div>
        <span style={{ flex: 1 }} />
        <button
          class="df-btn sm"
          onClick={copyAll}
          disabled={entries().length === 0}
          title="Copy filtered log to clipboard"
        >
          <Copy size={12} /> {copied() ? "Copied" : "Copy"}
        </button>
        <button
          class="df-btn sm"
          onClick={() => log.clear()}
          disabled={entries().length === 0}
          title="Clear in-memory buffer"
        >
          <Trash2 size={12} /> Clear
        </button>
      </div>

      <div
        style={{
          "max-height": "320px",
          overflow: "auto",
          background: "var(--card-2)",
          border: "0.5px solid var(--line)",
          "border-radius": "var(--r-2)",
          padding: "8px 10px",
          "font-family": "var(--font-mono)",
          "font-size": "10.5px",
          "line-height": "1.55",
          color: "var(--ink-2)",
        }}
        ref={(el) => {
          // Pin to bottom on update — this is the streaming behavior users
          // expect from a log view. Solid runs the ref after every render,
          // and we set scrollTop unconditionally because there's no smarter
          // signal that says "new entry arrived".
          createEffect(() => {
            void filtered();
            if (el) el.scrollTop = el.scrollHeight;
          });
        }}
      >
        <Show
          when={filtered().length > 0}
          fallback={
            <div style={{
              color: "var(--ink-3)",
              padding: "var(--pad-3) 0",
              "text-align": "center",
            }}>
              No log entries at this level yet. Run a search to populate.
            </div>
          }
        >
          <For each={filtered()}>{(e) => (
            <div
              style={{
                "white-space": "pre-wrap",
                "word-break": "break-word",
                color:
                  e.level === "error" ? "var(--bad)"
                  : e.level === "warn" ? "var(--warn)"
                  : e.level === "debug" ? "var(--ink-3)"
                  : "var(--ink-2)",
              }}
            >
              {formatEntry(e)}
            </div>
          )}</For>
        </Show>
      </div>

      {/* Rust runlog file */}
      <div style={{ "margin-top": "var(--pad-4)" }}>
        <div style={{
          "font-size": "11px",
          "text-transform": "uppercase",
          "letter-spacing": "0.06em",
          color: "var(--ink-3)",
          "font-weight": 500,
          "margin-bottom": "8px",
        }}>
          Backend runlog
        </div>
        <Show
          when={logInfo()}
          fallback={
            <p class="hint" style={{ margin: 0 }}>Unavailable.</p>
          }
        >
          {(info) => (
            <div style={{ display: "flex", "flex-direction": "column", gap: "var(--pad-3)" }}>
              <code
                title={info().path}
                style={{
                  display: "block",
                  "white-space": "nowrap",
                  overflow: "hidden",
                  "text-overflow": "ellipsis",
                  background: "var(--card-2)",
                  border: "0.5px solid var(--line)",
                  "border-radius": "var(--r-2)",
                  padding: "8px 12px",
                  "font-family": "var(--font-mono)",
                  "font-size": "11px",
                  color: "var(--ink-2)",
                }}
              >
                {info().path}
              </code>
              <p class="hint" style={{ margin: 0 }}>
                {info().exists ? formatBytes(info().size_bytes) : "Not yet written"}
              </p>
              <div style={{ display: "flex", gap: "8px" }}>
                <button
                  class="df-btn sm"
                  onClick={() => api.revealInFinder(info().path)}
                  disabled={!info().exists}
                >
                  <FolderOpen size={12} /> Show in Finder
                </button>
                <button class="df-btn sm" onClick={refreshLogInfo}>
                  <RefreshCw size={12} /> Refresh
                </button>
              </div>
            </div>
          )}
        </Show>
      </div>
    </section>
  );
}
