import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  on,
  onCleanup,
} from "solid-js";
import {
  ClipboardCopy,
  FileDown,
  FileText,
  FolderOpen,
  Quote,
  Search,
  Trash2,
  X,
} from "lucide-solid";
import { copyLibraryForAi, exportLibrary, exportMarkdown } from "@/components/Discover";
import { refreshLibraries } from "@/components/Sidebar";
import { api, type LibraryDoc, type LibraryInfo, type SearchHit } from "@/lib/tauri";
import {
  compareLibraryRecency,
  formatBytes,
  formatRelativeTime,
  libraryTimestamp,
  sourceColor,
  sourceLabel,
} from "@/lib/utils";
import { runStore } from "@/stores/run";
import { uiStore } from "@/stores/ui";

type Sort = "best" | "title" | "year";

const openFile = (path: string) =>
  void api.openPath(path).catch((e) => uiStore.showToast(`Couldn’t open the file: ${String(e)}`));
const reveal = (path: string) =>
  void api
    .revealInFinder(path)
    .catch((e) => uiStore.showToast(`Couldn’t show the folder: ${String(e)}`));

function citation(d: LibraryDoc): string {
  const a = d.authors.slice(0, 3).join(", ") + (d.authors.length > 3 ? " et al." : "");
  return [
    a && `${a}${d.year ? ` (${d.year})` : ""}.`,
    !a && d.year ? `(${d.year}).` : "",
    `${d.title.replace(/\.$/, "")}.`,
    d.venue ? `${d.venue}.` : "",
    d.doi ? `https://doi.org/${d.doi}` : d.url,
  ]
    .filter(Boolean)
    .join(" ");
}

/** `[term]` markers from FTS snippets → <mark> spans, safely (no innerHTML). */
function Snippet(props: { text: string }) {
  const parts = () => props.text.split(/(\[[^\]]*\])/g);
  return (
    <p>
      <For each={parts()}>
        {(p) => (p.startsWith("[") && p.endsWith("]") ? <mark>{p.slice(1, -1)}</mark> : <>{p}</>)}
      </For>
    </p>
  );
}

/** Minimal, safe Markdown rendering for the preview (headings, quotes, paragraphs). */
function Preview(props: { markdown: string }) {
  const [limit, setLimit] = createSignal(80);
  const blocks = createMemo(() => {
    let md = props.markdown;
    if (md.startsWith("---\n")) {
      const end = md.indexOf("\n---\n", 4);
      if (end > 0) md = md.slice(end + 5);
    }
    return (
      md
        .split(/\n{2,}/)
        .map((b) => b.trim())
        // The title and abstract are already shown above the preview.
        .filter((b) => b && !b.startsWith("# ") && !b.startsWith("> **Abstract.**"))
    );
  });
  return (
    <div class="df-preview">
      <For each={blocks().slice(0, limit())}>
        {(b) => {
          const page = /^<!-- page (\d+) -->$/.exec(b);
          if (page)
            return (
              <div class="df-label" style={{ margin: "14px 0 6px" }}>
                Page {page[1]}
              </div>
            );
          const h = /^(#{2,4})\s+(.*)$/.exec(b);
          if (h) {
            return h[1].length === 2 ? (
              <h2>{h[2]}</h2>
            ) : h[1].length === 3 ? (
              <h3>{h[2]}</h3>
            ) : (
              <h4>{h[2]}</h4>
            );
          }
          if (b.startsWith(">"))
            return <blockquote>{b.replace(/^>\s?/gm, "").replace(/\*\*/g, "")}</blockquote>;
          return <p>{b}</p>;
        }}
      </For>
      <Show when={blocks().length > limit()}>
        <button class="df-btn df-btn-quiet" onClick={() => setLimit(limit() + 120)}>
          Show more
        </button>
      </Show>
    </div>
  );
}

function Inspector(props: { doc: LibraryDoc; folder: string; onClose: () => void }) {
  const [md] = createResource(
    () => props.doc.md_path ?? null,
    (p) => (p ? api.readDocMarkdown(p).catch(() => "") : Promise.resolve("")),
  );
  const d = () => props.doc;
  const copyCitation = async () => {
    try {
      await navigator.clipboard.writeText(citation(d()));
      uiStore.showToast("Citation copied");
    } catch {
      uiStore.showToast("Couldn’t access the clipboard");
    }
  };
  return (
    <aside class="df-inspector" aria-label="Document details">
      <div class="df-row" style={{ "justify-content": "space-between" }}>
        <span class="df-label">
          <i
            class="df-dot"
            style={{ background: sourceColor(d().source), "margin-right": "6px" }}
          />
          {d().sources.length > 1
            ? d().sources.map(sourceLabel).join(" · ")
            : sourceLabel(d().source)}
        </span>
        <button
          class="df-btn df-btn-quiet df-btn-icon"
          aria-label="Close details"
          onClick={props.onClose}
        >
          <X size={15} />
        </button>
      </div>
      <h2 class="df-title">{d().title}</h2>
      <dl class="df-kv">
        <Show when={d().authors.length}>
          <dt>Authors</dt>
          <dd>{d().authors.join(", ")}</dd>
        </Show>
        <Show when={d().year}>
          <dt>Year</dt>
          <dd class="num">{d().year}</dd>
        </Show>
        <Show when={d().venue}>
          <dt>Published in</dt>
          <dd>{d().venue}</dd>
        </Show>
        <Show when={d().doi}>
          <dt>DOI</dt>
          <dd class="mono">{d().doi}</dd>
        </Show>
        <Show when={d().pages || d().words}>
          <dt>Length</dt>
          <dd class="num">
            {d().pages ? `${d().pages} pages · ` : ""}
            {d().words ? `${d().words?.toLocaleString()} words · ` : ""}
            {d().tokens ? `~${Math.round((d().tokens ?? 0) / 1000)}k tokens` : ""}
          </dd>
        </Show>
        <Show when={d().cited_by}>
          <dt>Cited by</dt>
          <dd class="num">{d().cited_by?.toLocaleString()}</dd>
        </Show>
        <Show when={d().license}>
          <dt>License</dt>
          <dd>{d().license}</dd>
        </Show>
      </dl>
      <div class="df-inspector-actions">
        <button class="df-btn df-btn-primary" onClick={() => openFile(d().path)}>
          <FileText size={14} aria-hidden="true" /> Open
        </button>
        <button class="df-btn" onClick={() => void copyLibraryForAi(props.folder, [d().id])}>
          <ClipboardCopy size={14} aria-hidden="true" /> Copy for AI
        </button>
        <button class="df-btn" onClick={() => void copyCitation()}>
          <Quote size={14} aria-hidden="true" /> Cite
        </button>
        <button
          class="df-btn"
          title="Save this document as a Markdown file"
          onClick={() => void exportMarkdown(props.folder, d().title, [d().id])}
        >
          <FileDown size={14} aria-hidden="true" /> .md
        </button>
        <button
          class="df-btn df-btn-quiet df-btn-icon"
          aria-label="Show in folder"
          title="Show in folder"
          onClick={() => reveal(d().path)}
        >
          <FolderOpen size={15} />
        </button>
      </div>
      <Show when={d().extract_error}>
        <p class="df-muted">No text could be extracted: {d().extract_error}</p>
      </Show>
      <Show when={d().abstract}>
        <div class="df-label" style={{ "margin-top": "8px" }}>
          Abstract
        </div>
        <p class="df-read" style={{ "font-size": "13.5px", "margin-top": "6px" }}>
          {d().abstract}
        </p>
      </Show>
      <Show when={md()}>{(text) => <Preview markdown={text()} />}</Show>
    </aside>
  );
}

function LibraryDetail(props: { lib: LibraryInfo; onDeleted: () => void }) {
  const [docs, { refetch }] = createResource(
    () => props.lib.path,
    (p) => api.listLibraryDocs(p),
  );
  /** Doc list, never throwing (a failed read shows the error message). */
  const docList = (): LibraryDoc[] => (docs.error ? [] : (docs.latest ?? []));
  const [filter, setFilter] = createSignal("");
  const [sort, setSort] = createSignal<Sort>("best");
  const [selected, setSelected] = createSignal<number | null>(null);
  const [hits, setHits] = createSignal<SearchHit[]>([]);

  // Refresh when a run that wrote into this library finishes.
  createEffect(
    on(
      () => runStore.state.status,
      (st) => {
        if ((st === "done" || st === "cancelled") && runStore.state.folder === props.lib.path)
          void refetch();
      },
      { defer: true },
    ),
  );

  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  createEffect(
    on(filter, (q) => {
      clearTimeout(timer);
      const mine = ++seq;
      if (q.trim().length < 2) {
        setHits([]);
        return;
      }
      timer = setTimeout(() => {
        api
          .searchLibrary(q, props.lib.path, 30)
          .then((h) => mine === seq && setHits(h))
          .catch(() => mine === seq && setHits([]));
      }, 220);
    }),
  );
  onCleanup(() => clearTimeout(timer));

  const visible = createMemo(() => {
    const q = filter().trim().toLowerCase();
    let list = docList().filter(
      (d) =>
        !q ||
        d.title.toLowerCase().includes(q) ||
        d.authors.some((a) => a.toLowerCase().includes(q)),
    );
    if (sort() === "title") list = [...list].sort((a, b) => a.title.localeCompare(b.title));
    if (sort() === "year")
      list = [...list].sort((a, b) => (b.year ?? "").localeCompare(a.year ?? ""));
    return list;
  });
  const selectedDoc = () => docList().find((d) => d.id === selected()) ?? null;
  const ts = () => libraryTimestamp(props.lib.name);

  const onListKey = (e: KeyboardEvent) => {
    const list = visible();
    if (!list.length) return;
    const i = list.findIndex((d) => d.id === selected());
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = e.key === "ArrowDown" ? Math.min(list.length - 1, i + 1) : Math.max(0, i - 1);
      setSelected(list[next].id);
      document.getElementById(`df-doc-${list[next].id}`)?.focus();
    } else if (e.key === "Enter") {
      // Open the row that has focus (not just the selected one).
      const id = Number((e.target as HTMLElement).id?.replace("df-doc-", ""));
      const doc = list.find((d) => d.id === id) ?? list[i];
      if (doc) openFile(doc.path);
    }
  };

  const remove = async () => {
    const ok = await uiStore.confirm({
      title: "Delete this library?",
      body: `“${props.lib.query}” and its ${props.lib.n_docs} downloaded files will be removed from your computer. This can’t be undone.`,
      confirmLabel: "Delete library",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteLibrary(props.lib.path);
      uiStore.showToast("Library deleted");
      props.onDeleted();
    } catch (e) {
      uiStore.showToast(`Couldn’t delete: ${String(e)}`);
    }
  };

  return (
    <div class="df-lib-body" data-inspect={selectedDoc() ? "true" : "false"}>
      <div class="df-lib-docs">
        <div class="df-page-head" style={{ "margin-bottom": 0 }}>
          <div style={{ "min-width": 0 }}>
            <div class="df-label">Library</div>
            <h1 class="df-title" style={{ "font-size": "38px" }}>
              {props.lib.query}
            </h1>
            <div class="df-muted num" style={{ "margin-top": "6px" }}>
              {props.lib.n_docs} documents · {formatBytes(props.lib.size_bytes)}
              {ts() ? ` · ${formatRelativeTime(ts()!)}` : ""}
            </div>
          </div>
          <div class="df-run-actions">
            <button
              class="df-btn df-btn-primary"
              onClick={() => void copyLibraryForAi(props.lib.path)}
            >
              <ClipboardCopy size={15} aria-hidden="true" /> Copy for AI
            </button>
            <button
              class="df-btn"
              title="One Markdown file with every document's full text — for RAG and AI tools"
              onClick={() => void exportMarkdown(props.lib.path, props.lib.query)}
            >
              <FileText size={15} aria-hidden="true" /> Export .md
            </button>
            <button
              class="df-btn"
              title="Zip with Markdown per document, chunks.jsonl, references.bib and corpus.md"
              onClick={() => void exportLibrary(props.lib.path, props.lib.query)}
            >
              <FileDown size={15} aria-hidden="true" /> Export pack
            </button>
            <button
              class="df-btn df-btn-quiet df-btn-icon"
              aria-label="Show in folder"
              title="Show in folder"
              onClick={() => reveal(props.lib.path)}
            >
              <FolderOpen size={15} />
            </button>
            <button
              class="df-btn df-btn-quiet df-btn-icon"
              aria-label="Delete library"
              title="Delete library"
              onClick={() => void remove()}
            >
              <Trash2 size={15} />
            </button>
          </div>
        </div>

        <div class="df-lib-toolbar">
          <div class="df-search">
            <Search size={14} aria-hidden="true" />
            <input
              class="df-input"
              type="search"
              placeholder="Search"
              aria-label="Search this library"
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
            />
          </div>
          <div class="df-seg" role="group" aria-label="Sort">
            <For
              each={
                [
                  ["best", "Best match"],
                  ["title", "Title"],
                  ["year", "Newest"],
                ] as [Sort, string][]
              }
            >
              {([id, label]) => (
                <button aria-pressed={sort() === id} onClick={() => setSort(id)}>
                  {label}
                </button>
              )}
            </For>
          </div>
        </div>

        <Show when={hits().length > 0}>
          <div class="df-section" style={{ "margin-top": "var(--sp-4)" }}>
            <div class="df-section-head">
              <span class="df-label">Passages</span>
              <span class="df-label num">{hits().length}</span>
            </div>
            <For each={hits().slice(0, 12)}>
              {(h) => (
                <div class="df-hit">
                  <button class="df-slip-title" onClick={() => setSelected(h.doc_id)}>
                    {h.title}
                  </button>
                  <div class="df-slip-meta">
                    {h.heading ? <span>{h.heading}</span> : null}
                    {h.page_start ? <span>· p. {h.page_start}</span> : null}
                  </div>
                  <Snippet text={h.snippet} />
                </div>
              )}
            </For>
          </div>
        </Show>

        <div class="df-section" style={{ "margin-top": "var(--sp-4)" }}>
          <div class="df-section-head">
            <span class="df-label">Documents</span>
            <span class="df-label num">{visible().length}</span>
          </div>
          <Show when={!docs.loading} fallback={<p class="df-muted">Loading…</p>}>
            <Show
              when={!docs.error}
              fallback={<p class="df-muted">Couldn’t read this library: {String(docs.error)}</p>}
            >
              <div role="listbox" aria-label="Documents" onKeyDown={onListKey}>
                <For each={visible()}>
                  {(d) => (
                    <button
                      id={`df-doc-${d.id}`}
                      role="option"
                      class="df-doc-row"
                      aria-selected={selected() === d.id}
                      onClick={() => setSelected(d.id)}
                      onDblClick={() => openFile(d.path)}
                    >
                      <div style={{ "min-width": 0 }}>
                        <div class="df-slip-title">{d.title}</div>
                        <div class="df-slip-meta">
                          <i class="df-dot" style={{ background: sourceColor(d.source) }} />
                          <span>{sourceLabel(d.source)}</span>
                          {d.year ? <span>· {d.year}</span> : null}
                          {d.authors.length ? (
                            <span>
                              · {d.authors.slice(0, 2).join(", ")}
                              {d.authors.length > 2 ? " et al." : ""}
                            </span>
                          ) : null}
                          {d.extract_error ? <span class="df-tag df-tag-warn">no text</span> : null}
                        </div>
                      </div>
                      <span class="df-slip-side num">
                        {d.pages ? `${d.pages} pp · ` : ""}
                        {formatBytes(d.size_bytes ?? 0)}
                      </span>
                    </button>
                  )}
                </For>
              </div>
              <Show when={visible().length === 0}>
                <p class="df-muted">No documents match “{filter()}”.</p>
              </Show>
            </Show>
          </Show>
        </div>
      </div>
      <Show when={selectedDoc()} keyed>
        {(d) => <Inspector doc={d} folder={props.lib.path} onClose={() => setSelected(null)} />}
      </Show>
    </div>
  );
}

export default function LibraryView() {
  const libs = () =>
    [...uiStore.knownLibraries].sort((a, b) => compareLibraryRecency(a.name, b.name));
  const [globalQ, setGlobalQ] = createSignal("");
  const [globalHits, setGlobalHits] = createSignal<SearchHit[]>([]);
  const current = () =>
    libs().find((l) => l.path === uiStore.openLibraryPath) ??
    (uiStore.openLibraryPath ? null : (libs()[0] ?? null));

  void refreshLibraries();

  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  createEffect(
    on(globalQ, (q) => {
      clearTimeout(timer);
      const mine = ++seq;
      if (q.trim().length < 2) return setGlobalHits([]);
      timer = setTimeout(() => {
        api
          .searchLibrary(q, null, 40)
          .then((h) => mine === seq && setGlobalHits(h))
          .catch(() => mine === seq && setGlobalHits([]));
      }, 250);
    }),
  );
  onCleanup(() => clearTimeout(timer));

  return (
    <Show
      when={libs().length > 0}
      fallback={
        <div class="df-page">
          <div class="df-empty">
            <h1 class="df-title" style={{ "font-size": "44px" }}>
              Your library is <em>empty</em>
            </h1>
            <button
              class="df-btn df-btn-primary df-btn-lg"
              onClick={() => uiStore.setView("discover")}
            >
              Start a search
            </button>
          </div>
        </div>
      }
    >
      <div class="df-lib">
        <div class="df-lib-list">
          <div class="df-lib-list-head">
            <div class="df-label">Libraries</div>
          </div>
          <div class="df-search">
            <Search size={14} aria-hidden="true" />
            <input
              class="df-input"
              type="search"
              placeholder="Search all"
              aria-label="Search the full text of every library"
              value={globalQ()}
              onInput={(e) => setGlobalQ(e.currentTarget.value)}
            />
          </div>
          <Show
            when={globalQ().trim().length >= 2}
            fallback={
              <div>
                <For each={libs()}>
                  {(l) => {
                    const ts = libraryTimestamp(l.name);
                    return (
                      <button
                        class="df-lib-item"
                        aria-current={current()?.path === l.path}
                        onClick={() => uiStore.showLibrary(l.path)}
                      >
                        <b>{l.query}</b>
                        <span class="num">
                          {l.n_docs} docs · {formatBytes(l.size_bytes)}
                          {ts ? ` · ${formatRelativeTime(ts)}` : ""}
                        </span>
                      </button>
                    );
                  }}
                </For>
              </div>
            }
          >
            <div>
              <Show when={globalHits().length === 0}>
                <p class="df-muted" style={{ padding: "0 8px" }}>
                  No passages match.
                </p>
              </Show>
              <For each={globalHits()}>
                {(h) => (
                  <button
                    class="df-lib-item"
                    onClick={() => uiStore.showLibrary(h.library)}
                    style={{ "padding-bottom": "12px" }}
                  >
                    <span>{h.library_query}</span>
                    <div style={{ "font-weight": 560, margin: "2px 0" }}>{h.title}</div>
                    <span>
                      <Snippet text={h.snippet} />
                    </span>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
        <Show
          when={current()?.path}
          keyed
          fallback={
            <div class="df-page">
              <p class="df-muted">Library not found.</p>
            </div>
          }
        >
          {(path) => (
            <LibraryDetail
              lib={libs().find((l) => l.path === path)!}
              onDeleted={() => {
                uiStore.showLibrary(null);
                void refreshLibraries();
              }}
            />
          )}
        </Show>
      </div>
    </Show>
  );
}
