import { save } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  CheckCheck,
  FileArchive,
  FolderOpen,
  Library,
  Loader2,
  Search,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/shallow";
import { Button } from "./ui/button";
import { Progress } from "./ui/progress";
import { Textarea } from "./ui/textarea";
import { Toggle } from "./ui/toggle";
import { LiveDownloadStream } from "./LiveDownloadStream";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, SOURCE_LABELS } from "@/lib/utils";
import { useRunStore } from "@/stores/runStore";
import { useSettings } from "@/stores/settingsStore";
import { useUI } from "@/stores/uiStore";

const EXAMPLES = [
  "transformer architectures and attention",
  "Christian bibles and patristic writings",
  "Jungian psychology and individuation",
  "climate change adaptation in agriculture",
  "early modern philosophy of mind",
];

export function FindTab() {
  const [query, setQuery] = useState("");
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const [exportedTo, setExportedTo] = useState<string | null>(null);

  const settings = useSettings();
  const setSettings = useSettings((s) => s.set);
  const libraryRoot = useSettings((s) => s.libraryRoot);

  const reset = useRunStore((s) => s.reset);
  const setRunning = useRunStore((s) => s.setRunning);
  const apply = useRunStore((s) => s.apply);
  const cancelRun = () => api.cancelRun();

  const {
    running,
    total,
    found,
    done,
    failed,
    filteredCount,
    folder,
    sourceIssues,
    completed,
    fatalError,
  } = useRunStore(
    useShallow((s) => ({
      running: s.running,
      total: s.total,
      found: s.found,
      done: s.done,
      failed: s.failed,
      filteredCount: s.filteredCount,
      folder: s.folder,
      sourceIssues: s.sourceIssues,
      completed: s.completed,
      fatalError: s.fatalError,
    })),
  );

  const setActiveLibrary = useUI((s) => s.setActiveLibrary);
  const setView = useUI((s) => s.setView);

  useEffect(() => {
    if (!libraryRoot) {
      api.defaultLibraryDir().then((d) => setSettings({ libraryRoot: d.library_root }));
    }
  }, [libraryRoot, setSettings]);

  const start = async () => {
    if (!query.trim() || running) return;
    if (settings.selectedSources.length === 0) return;
    reset(query.trim());
    setRunning(true);
    setExportedTo(null);
    setExportError(null);
    try {
      await api.startRun({
        query: query.trim(),
        sources: settings.selectedSources,
        out_dir: settings.libraryRoot,
        per_source: settings.perSource,
        max_total: settings.maxTotal,
        concurrency: settings.concurrency,
        extract: true,
        source_options: {},
      });
    } catch (e) {
      setRunning(false);
      apply({ type: "error", payload: { message: String(e) } });
    }
  };

  const overallPct = total > 0 ? Math.round(((done + failed) / total) * 100) : 0;

  const goLibrary = async () => {
    if (!folder) return;
    try {
      const info = await api.openLibrary(folder);
      setActiveLibrary(info);
    } catch {
      // ignore
    }
    setView("library");
  };

  const exportZip = async () => {
    if (!folder) return;
    setExportError(null);
    const slug = folder.split("/").pop() ?? "library";
    const dest = await save({
      defaultPath: `${slug}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!dest) return;
    setExporting(true);
    try {
      const result = await api.exportLibraryZip(folder, dest);
      setExportedTo(result.dest);
      await api.revealInFinder(result.dest);
    } catch (e) {
      setExportError(String(e));
    } finally {
      setExporting(false);
    }
  };

  const failedItems = useMemo(
    () => completed.filter((c) => c.status === "failed").slice(-10).reverse(),
    [completed],
  );

  return (
    <div className="space-y-5">
      <header>
        <h1 className="text-2xl font-semibold">Discover</h1>
        <p className="text-sm text-[var(--color-muted-foreground)]">
          Describe what you're researching. We'll search 7 open sources, download what's
          relevant, and pack it as a ZIP for any AI tool.
        </p>
      </header>

      <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5 shadow-sm">
        <Textarea
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          rows={2}
          placeholder="e.g. attention mechanisms in transformers"
          disabled={running}
          autoFocus
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              start();
            }
          }}
        />

        <div className="mt-3 flex flex-wrap gap-1.5">
          {EXAMPLES.map((ex) => (
            <button
              key={ex}
              type="button"
              onClick={() => setQuery(ex)}
              disabled={running}
              className="rounded-full border border-[var(--color-border)] px-2.5 py-0.5 text-xs text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)]/40 hover:bg-[var(--color-accent)] hover:text-[var(--color-foreground)] disabled:opacity-50"
            >
              {ex}
            </button>
          ))}
        </div>

        <div className="mt-5">
          <div className="mb-1.5 flex items-center justify-between text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
            <span>Sources</span>
            {settings.selectedSources.length === 0 && (
              <span className="normal-case tracking-normal text-rose-600">
                Pick at least one
              </span>
            )}
          </div>
          <div className="flex flex-wrap gap-1.5">
            {ALL_SOURCES.map((s) => (
              <Toggle
                key={s}
                pressed={settings.selectedSources.includes(s)}
                onPressedChange={() => settings.toggleSource(s)}
                disabled={running}
              >
                {SOURCE_LABELS[s]}
              </Toggle>
            ))}
          </div>
        </div>

        <div className="mt-5 flex flex-wrap items-center gap-2">
          {!running ? (
            <Button
              onClick={start}
              size="lg"
              disabled={!query.trim() || settings.selectedSources.length === 0}
            >
              <Search className="h-4 w-4" />
              Find documents
            </Button>
          ) : (
            <Button onClick={cancelRun} size="lg" variant="destructive">
              <X className="h-4 w-4" />
              Cancel
            </Button>
          )}
          {folder && !running && (
            <>
              <Button size="lg" onClick={exportZip} disabled={exporting} variant="secondary">
                {exporting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <FileArchive className="h-4 w-4" />
                )}
                {exporting ? "Zipping…" : "Export ZIP for AI"}
              </Button>
              <Button size="lg" variant="outline" onClick={goLibrary}>
                <Library className="h-4 w-4" />
                Open in Library
              </Button>
              <Button
                size="lg"
                variant="ghost"
                onClick={() => api.revealInFinder(folder)}
              >
                <FolderOpen className="h-4 w-4" />
                Show folder
              </Button>
            </>
          )}
        </div>
      </div>

      {fatalError && (
        <div className="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
          {fatalError}
        </div>
      )}

      {exportError && (
        <div className="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
          Export failed: {exportError}
        </div>
      )}

      {exportedTo && (
        <div className="flex items-center gap-2 rounded-md border border-emerald-500/40 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
          <CheckCheck className="h-4 w-4" />
          Saved to {exportedTo}
        </div>
      )}

      {(running || total > 0) && (
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-3 text-sm tabular-nums">
            <div className="flex items-center gap-3 text-[var(--color-muted-foreground)]">
              <span>
                <span className="font-semibold text-[var(--color-foreground)]">{found}</span>{" "}
                found
              </span>
              <span>·</span>
              <span>
                <span className="font-semibold text-emerald-700">{done}</span> done
              </span>
              {failed > 0 && (
                <>
                  <span>·</span>
                  <span>
                    <span className="font-semibold text-rose-700">{failed}</span> failed
                  </span>
                </>
              )}
              {filteredCount > 0 && (
                <>
                  <span>·</span>
                  <span title="Dropped because the title and abstract didn't mention any of your query keywords">
                    <span className="font-semibold text-amber-700">{filteredCount}</span>{" "}
                    off-topic
                  </span>
                </>
              )}
            </div>
            {running && (
              <div className="flex items-center gap-2 text-[var(--color-muted-foreground)]">
                <Loader2 className="h-4 w-4 animate-spin" />
                {total === 0 ? "Discovering…" : `${overallPct}%`}
              </div>
            )}
          </div>
          {total > 0 && <Progress value={overallPct} />}
          <LiveDownloadStream />
        </div>
      )}

      {(sourceIssues.length > 0 || failedItems.length > 0) && (
        <details className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-4">
          <summary className="flex cursor-pointer list-none items-center gap-2 text-sm">
            <AlertTriangle className="h-4 w-4 text-amber-600" />
            <span className="font-medium">Issues</span>
            <span className="text-xs text-[var(--color-muted-foreground)]">
              {sourceIssues.length} source error{sourceIssues.length === 1 ? "" : "s"}
              {failedItems.length > 0 &&
                ` · ${failed} failed download${failed === 1 ? "" : "s"}`}
            </span>
          </summary>
          {sourceIssues.length > 0 && (
            <div className="mt-3 space-y-1.5">
              <div className="text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
                Source errors
              </div>
              {sourceIssues.map((issue, i) => (
                <div
                  key={`${issue.ts}-${i}`}
                  className="rounded-md border border-rose-500/30 bg-rose-50 px-2.5 py-1.5 text-xs"
                >
                  <span className="font-mono text-rose-700">
                    {SOURCE_LABELS[issue.source] ?? issue.source}
                  </span>{" "}
                  <span className="text-[var(--color-foreground)]/80">{issue.error}</span>
                </div>
              ))}
            </div>
          )}
          {failedItems.length > 0 && (
            <div className="mt-3 space-y-1.5">
              <div className="text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
                Recent failed downloads
              </div>
              {failedItems.map((c) => (
                <div
                  key={c.url}
                  className="rounded-md border border-[var(--color-border)] bg-[var(--color-card)] px-2.5 py-1.5 text-xs"
                  title={c.url}
                >
                  <div className="truncate text-[var(--color-foreground)]">{c.title}</div>
                  {c.error && (
                    <div className="truncate text-[10px] text-rose-700/80">{c.error}</div>
                  )}
                </div>
              ))}
            </div>
          )}
        </details>
      )}
    </div>
  );
}
