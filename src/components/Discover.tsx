import { For, Show, createMemo, createSignal } from "solid-js";
import {
  ArrowLeft,
  ArrowRight,
  ChevronRight,
  ClipboardCopy,
  FileDown,
  FolderOpen,
  Library,
  Loader2,
  RotateCcw,
  Square,
} from "lucide-solid";
import { save } from "@tauri-apps/plugin-dialog";
import Note from "@/components/Note";
import { api } from "@/lib/tauri";
import { humanizeDownloadError, humanizeSourceKind } from "@/lib/errors";
import {
  DEFAULT_ENABLED_SOURCES,
  formatBytes,
  formatDuration,
  sourceColor,
  sourceDesc,
  sourceLabel,
  type SourceId,
} from "@/lib/utils";
import { progressFraction, runStore, stages, type StageId } from "@/stores/run";
import { setSources, settings, toggleSource } from "@/stores/settings";
import { uiStore } from "@/stores/ui";

const EXAMPLES = [
  "CRISPR off-target effects in human cells",
  "Roman roads and trade networks",
  "Urban heat islands and street trees",
];

/** Sources a user can pick (the individual web engines live inside "Web"). */
const PICKABLE: SourceId[] = DEFAULT_ENABLED_SOURCES;

export async function exportLibrary(folder: string, query: string) {
  const safe =
    query
      .replace(/[^\w\s-]/g, "")
      .trim()
      .replace(/\s+/g, "-")
      .slice(0, 60) || "library";
  const dest = await save({
    title: "Export for AI",
    defaultPath: `${safe}-ai-pack.zip`,
    filters: [{ name: "Zip archive", extensions: ["zip"] }],
  });
  if (!dest) return;
  try {
    const r = await api.exportLibraryZip(folder, dest, { budget_tokens: 150_000 });
    uiStore.showToast(`Exported ${r.files} files (${formatBytes(r.size_bytes)})`);
    void api.revealInFinder(r.dest).catch(() => undefined);
  } catch (e) {
    uiStore.showToast(`Export failed: ${String(e)}`);
  }
}

export async function copyLibraryForAi(folder: string, ids?: number[]) {
  try {
    const text = await api.copyForAi(folder, ids ?? null, 150_000);
    await navigator.clipboard.writeText(text);
    const tokens = Math.round(text.length / 4 / 1000);
    uiStore.showToast(`Copied ~${tokens}k tokens — paste into any AI chat`);
  } catch (e) {
    uiStore.showToast(`Couldn't copy: ${String(e)}`);
  }
}

function SourcePicker() {
  const [open, setOpen] = createSignal(false);
  const selected = () => settings.selectedSources.filter((s) => PICKABLE.includes(s));
  return (
    <div class="df-sources">
      <div class="df-sources-line">
        <span class="df-dots" aria-hidden="true">
          <For each={selected()}>
            {(s) => <i class="df-dot" style={{ background: sourceColor(s) }} />}
          </For>
        </span>
        <span>
          Searching <b class="num">{selected().length}</b> of {PICKABLE.length} open-access sources
        </span>
        <button class="df-link-btn" aria-expanded={open()} onClick={() => setOpen(!open())}>
          {open() ? "Done" : "Choose"}
        </button>
      </div>
      <Show when={open()}>
        <div class="df-source-grid" role="group" aria-label="Sources">
          <For each={PICKABLE}>
            {(s) => (
              <label class="df-source-row">
                <input
                  type="checkbox"
                  checked={settings.selectedSources.includes(s)}
                  disabled={runStore.live}
                  onChange={() => toggleSource(s)}
                  style={{ "accent-color": "var(--accent)", "margin-top": "3px" }}
                />
                <i class="df-dot" style={{ background: sourceColor(s), "margin-top": "6px" }} />
                <div>
                  <b>{sourceLabel(s)}</b>
                  <span>{sourceDesc(s)}</span>
                </div>
              </label>
            )}
          </For>
        </div>
        <div class="df-row" style={{ "margin-top": "10px" }}>
          <button class="df-btn df-btn-quiet" onClick={() => setSources([...PICKABLE])}>
            Select all
          </button>
        </div>
      </Show>
    </div>
  );
}

/** The query draft, shared so example chips can fill it. */
const [query, setQuery] = createSignal("");

function QueryBox() {
  const canRun = () =>
    query().trim().length > 1 && settings.selectedSources.length > 0 && !runStore.live;
  const run = () => {
    if (!canRun()) return;
    void runStore.start(query().trim());
  };
  return (
    <form
      class="df-query"
      onSubmit={(e) => {
        e.preventDefault();
        run();
      }}
    >
      <label for="df-q" class="sr-only">
        What are you researching?
      </label>
      <textarea
        id="df-q"
        rows={2}
        placeholder="Describe a topic in plain language — e.g. “effects of sleep on memory in teenagers”"
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey || !e.shiftKey)) {
            e.preventDefault();
            run();
          }
        }}
      />
      <div class="df-query-bar">
        <span class="df-muted">
          {settings.selectedSources.length === 0
            ? "Choose at least one source below."
            : "Press Enter to search. We’ll find, download and read the best open-access matches."}
        </span>
        <button class="df-btn df-btn-primary df-btn-lg" type="submit" disabled={!canRun()}>
          Find documents <ArrowRight size={16} aria-hidden="true" />
        </button>
      </div>
    </form>
  );
}

function Stage(props: { id: StageId; name: string; figure: string; sub: string }) {
  const state = () => stages()[props.id];
  return (
    <div class="df-stage" data-state={state()}>
      <div class="df-stage-name">
        <Show when={state() === "active"}>
          <Loader2 size={13} class="df-spin" aria-hidden="true" />
        </Show>
        {props.name}
      </div>
      <div class="df-stage-figure num">{props.figure}</div>
      <div class="df-stage-sub">{props.sub}</div>
    </div>
  );
}

function statusLine(): string {
  const s = runStore.state;
  const elapsed = (s.endedAt || Date.now()) - s.startedAt;
  switch (s.status) {
    case "running":
      if (s.retry) return `Retrying ${s.planned} documents…`;
      if (s.done === 0 && s.inFlight.length === 0)
        return `Searching ${settings.selectedSources.length} sources…`;
      return `Saved ${s.done} of ${Math.max(s.planned, s.done + s.failed)} planned so far`;
    case "stopping":
      return "Stopping — finishing the files already in progress…";
    case "done":
      return `Finished in ${formatDuration(elapsed)} — ${s.done} documents saved${s.failed ? `, ${s.failed} couldn’t be downloaded` : ""}.`;
    case "cancelled":
      return `Stopped — ${s.done} documents saved.`;
    case "error":
      return "The search couldn’t run.";
    default:
      return "";
  }
}

function RunView() {
  const s = runStore.state;
  const shownSaved = () => s.saved.slice(0, 80);
  const failureGroups = createMemo(() => {
    const m = new Map<string, number>();
    for (const f of s.failures) {
      const k = humanizeDownloadError(f.error);
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });
  const sourceErrors = () =>
    Object.entries(s.sources).filter(([, v]) => v.error && v.error !== "parse_error");
  const retryable = () => s.failures.filter((f) => !/Same file as/.test(f.error)).length;
  const ended = () => s.status === "done" || s.status === "cancelled";
  const speed = () => {
    const secs = ((s.endedAt || Date.now()) - s.startedAt) / 1000;
    return secs > 0 ? s.bytes / secs : 0;
  };

  return (
    <section aria-labelledby="df-run-title">
      <div class="df-run-head">
        <div style={{ "min-width": 0 }}>
          <div class="df-label">{runStore.live ? "Searching" : "Search"}</div>
          <h1 id="df-run-title" class="df-title" style={{ "margin-top": "6px" }}>
            {s.query}
          </h1>
          <p class="df-status" aria-live="polite">
            {statusLine()}
          </p>
        </div>
        <div class="df-run-actions">
          <Show when={runStore.live}>
            <button
              class="df-btn"
              onClick={() => void runStore.stop()}
              disabled={s.status === "stopping"}
            >
              <Square size={13} aria-hidden="true" />{" "}
              {s.status === "stopping" ? "Stopping…" : "Stop"}
            </button>
          </Show>
          <Show when={!runStore.live}>
            <button class="df-btn df-btn-quiet" onClick={() => runStore.reset()}>
              <ArrowLeft size={15} aria-hidden="true" /> New search
            </button>
          </Show>
          <Show when={ended() && s.folder && s.done > 0}>
            <button class="df-btn df-btn-primary" onClick={() => void copyLibraryForAi(s.folder)}>
              <ClipboardCopy size={15} aria-hidden="true" /> Copy for AI
            </button>
            <button class="df-btn" onClick={() => void exportLibrary(s.folder, s.query)}>
              <FileDown size={15} aria-hidden="true" /> Export
            </button>
            <button class="df-btn" onClick={() => uiStore.showLibrary(s.folder)}>
              <Library size={15} aria-hidden="true" /> Open in Library
            </button>
          </Show>
        </div>
      </div>

      <div class="df-stages">
        <Stage
          id="search"
          name="Search"
          figure={`${s.found}`}
          sub={
            s.retry
              ? "skipped (retry)"
              : `candidates from ${Object.keys(s.sources).length || settings.selectedSources.length} sources`
          }
        />
        <Stage
          id="download"
          name="Download"
          figure={`${s.done}${s.planned ? ` / ${Math.max(s.planned, s.done + s.failed)}` : ""}`}
          sub={`${s.inFlight.length} in progress · ${formatBytes(s.bytes)}${speed() > 0 ? ` · ${formatBytes(speed())}/s` : ""}`}
        />
        <Stage
          id="read"
          name="Read"
          figure={`${s.saved.filter((d) => d.textPath).length}`}
          sub="turned into clean, searchable text"
        />
      </div>
      <div
        class="df-progress"
        data-indeterminate={runStore.live && s.planned === 0 ? "true" : "false"}
        role="progressbar"
        aria-label="Overall progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(progressFraction() * 100)}
      >
        <div style={{ width: `${progressFraction() * 100}%` }} />
      </div>

      <Show when={s.status === "error"}>
        <Note tone="bad">{s.error || "Something went wrong."}</Note>
      </Show>
      <Show when={ended() && s.done === 0 && s.found === 0}>
        <Note tone="warn">
          Nothing matched. Try fewer or more general words, or check your internet connection.
        </Note>
      </Show>
      <Show when={ended() && s.done === 0 && s.found > 0}>
        <Note tone="warn">
          We found {s.found} candidates but none could be downloaded as open access. Try a broader
          search or add the Web source.
        </Note>
      </Show>

      <Show when={s.inFlight.length > 0}>
        <div class="df-section">
          <div class="df-section-head">
            <span class="df-label">In progress</span>
            <span class="df-label num">{s.inFlight.length}</span>
          </div>
          <div class="df-inflight">
            <For each={s.inFlight.slice(0, 8)}>
              {(f) => (
                <div class="df-inflight-item">
                  <div title={f.title}>{f.title}</div>
                  <div class="df-bar">
                    <i
                      style={{
                        width:
                          f.total > 0 ? `${Math.min(100, (f.downloaded / f.total) * 100)}%` : "12%",
                      }}
                    />
                  </div>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>

      <Show when={s.saved.length > 0}>
        <div class="df-section">
          <div class="df-section-head">
            <span class="df-label">Filed</span>
            <span class="df-label num">{s.done} documents</span>
          </div>
          <ol class="df-ledger" aria-label="Saved documents, newest first">
            <For each={shownSaved()}>
              {(d) => (
                <li class="df-slip">
                  <span class="df-slip-n num">№ {String(d.n).padStart(3, "0")}</span>
                  <div style={{ "min-width": 0 }}>
                    <button
                      class="df-slip-title"
                      onClick={() =>
                        void api.openPath(d.path).catch((e) => uiStore.showToast(String(e)))
                      }
                      title="Open the original file"
                    >
                      {d.doc.title}
                    </button>
                    <div class="df-slip-meta">
                      <i class="df-dot" style={{ background: sourceColor(d.doc.source) }} />
                      <span>{sourceLabel(d.doc.source)}</span>
                      <Show when={d.doc.year}>
                        <span>· {d.doc.year}</span>
                      </Show>
                      <Show when={d.doc.authors.length > 0}>
                        <span>
                          · {d.doc.authors.slice(0, 2).join(", ")}
                          {d.doc.authors.length > 2 ? " et al." : ""}
                        </span>
                      </Show>
                      <Show when={d.extractError}>
                        <span class="df-tag df-tag-warn" title={d.extractError}>
                          no text
                        </span>
                      </Show>
                      <Show when={d.cached}>
                        <span class="df-tag" title="Reused a copy already in your library">
                          reused
                        </span>
                      </Show>
                    </div>
                  </div>
                  <span class="df-slip-side num">{formatBytes(d.bytes)}</span>
                </li>
              )}
            </For>
          </ol>
          <Show when={s.saved.length > shownSaved().length}>
            <p class="df-muted" style={{ "margin-top": "12px" }}>
              And {s.done - shownSaved().length} more —{" "}
              <button class="df-link-btn" onClick={() => uiStore.showLibrary(s.folder)}>
                see them all in the Library
              </button>
              .
            </p>
          </Show>
        </div>
      </Show>

      <div class="df-section">
        <Show when={s.failures.length > 0}>
          <details class="df-details">
            <summary>
              <ChevronRight size={14} aria-hidden="true" />
              {s.failures.length} couldn’t be downloaded
              <Show when={ended() && retryable() > 0}>
                <button
                  class="df-btn df-btn-quiet"
                  style={{ "margin-left": "auto", height: "26px" }}
                  onClick={(e) => {
                    e.preventDefault();
                    void runStore.retryFailed();
                  }}
                >
                  <RotateCcw size={13} aria-hidden="true" /> Try again
                </button>
              </Show>
            </summary>
            <div class="df-details-body">
              <dl class="df-kv">
                <For each={failureGroups()}>
                  {([reason, n]) => (
                    <>
                      <dt class="num">{n} documents</dt>
                      <dd>{reason}</dd>
                    </>
                  )}
                </For>
              </dl>
            </div>
          </details>
        </Show>
        <Show when={s.offTopic > 0}>
          <details class="df-details">
            <summary>
              <ChevronRight size={14} aria-hidden="true" />
              {s.offTopic} off-topic results skipped
            </summary>
            <div class="df-details-body df-muted">
              These matched too little of your search to be worth downloading. Make your search more
              specific to tighten results, or more general to widen them.
            </div>
          </details>
        </Show>
        <details class="df-details">
          <summary>
            <ChevronRight size={14} aria-hidden="true" />
            Details
          </summary>
          <div class="df-details-body">
            <dl class="df-kv">
              <dt>Searches run</dt>
              <dd>
                <For each={s.subQueries}>
                  {(q, i) => (
                    <>
                      {i() > 0 ? " · " : ""}
                      {q}
                    </>
                  )}
                </For>
              </dd>
              <dt>Found per source</dt>
              <dd>
                <For each={Object.entries(s.sources).sort((a, b) => b[1].found - a[1].found)}>
                  {([src, v], i) => (
                    <>
                      {i() > 0 ? " · " : ""}
                      {sourceLabel(src)} <span class="num">{v.found}</span>
                    </>
                  )}
                </For>
              </dd>
              <Show when={sourceErrors().length > 0}>
                <dt>Source issues</dt>
                <dd>
                  <For each={sourceErrors()}>
                    {([src, v]) => <div>{humanizeSourceKind(v.error ?? "", src)}</div>}
                  </For>
                </dd>
              </Show>
              <Show when={s.folder}>
                <dt>Folder</dt>
                <dd>
                  <button class="df-link-btn" onClick={() => void api.revealInFinder(s.folder)}>
                    <FolderOpen size={12} aria-hidden="true" /> Show in folder
                  </button>
                </dd>
              </Show>
            </dl>
          </div>
        </details>
      </div>
    </section>
  );
}

export default function Discover() {
  const hasRun = () => runStore.state.status !== "idle";
  const useExample = (q: string) => {
    setQuery(q);
    document.getElementById("df-q")?.focus();
  };

  return (
    <div class="df-page">
      <Show
        when={hasRun()}
        fallback={
          <div class="df-hero">
            <div class="df-label">Discover</div>
            <h1 class="df-display" style={{ "margin-top": "14px" }}>
              What are you <em>researching?</em>
            </h1>
            <p class="df-hero-sub df-read">
              Ask in plain language. Document Finder searches open-access libraries in parallel,
              downloads the best matches, and turns them into clean text your AI can read.
            </p>
            <QueryBox />
            <div class="df-examples">
              <span class="df-label">Try</span>
              <For each={EXAMPLES}>
                {(q) => (
                  <button class="df-chip" onClick={() => useExample(q)}>
                    {q}
                  </button>
                )}
              </For>
            </div>
            <SourcePicker />
          </div>
        }
      >
        <RunView />
      </Show>
    </div>
  );
}
