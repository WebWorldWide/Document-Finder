import { save } from "@tauri-apps/plugin-dialog";
import {
  ArrowRight,
  CheckCheck,
  FileArchive,
  FolderOpen,
  Library as LibraryIcon,
  Loader2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "./ui/button";
import { api, type LibraryInfo } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import { useSettings } from "@/stores/settingsStore";
import { useUI } from "@/stores/uiStore";

export function LibraryView() {
  const libraryRoot = useSettings((s) => s.libraryRoot);
  const setSettings = useSettings((s) => s.set);
  const setActiveLibrary = useUI((s) => s.setActiveLibrary);
  const setView = useUI((s) => s.setView);
  const activeLibrary = useUI((s) => s.activeLibrary);

  const [libraries, setLibraries] = useState<LibraryInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [exportingPath, setExportingPath] = useState<string | null>(null);
  const [justExported, setJustExported] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!libraryRoot) {
      api.defaultLibraryDir().then((d) => setSettings({ libraryRoot: d.library_root }));
      return;
    }
    setLoading(true);
    api
      .listLibraries(libraryRoot)
      .then((rows) => setLibraries(rows))
      .catch(() => setLibraries([]))
      .finally(() => setLoading(false));
  }, [libraryRoot, setSettings]);

  const exportZip = async (l: LibraryInfo) => {
    setError(null);
    const dest = await save({
      defaultPath: `${l.name}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
    });
    if (!dest) return;
    setExportingPath(l.path);
    try {
      const result = await api.exportLibraryZip(l.path, dest);
      setJustExported(result.dest);
      setTimeout(() => setJustExported(null), 4000);
      await api.revealInFinder(result.dest);
    } catch (e) {
      setError(String(e));
    } finally {
      setExportingPath(null);
    }
  };

  return (
    <div className="space-y-5">
      <header>
        <h1 className="text-2xl font-semibold">Library</h1>
      </header>

      {error && (
        <div className="rounded-md border border-rose-500/40 bg-rose-50 px-3 py-2 text-sm text-rose-700">
          {error}
        </div>
      )}

      {justExported && (
        <div className="flex items-center gap-2 rounded-md border border-emerald-500/40 bg-emerald-50 px-3 py-2 text-sm text-emerald-700">
          <CheckCheck className="h-4 w-4" />
          Exported to {justExported}
        </div>
      )}

      {loading ? (
        <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-8 text-center text-sm text-[var(--color-muted-foreground)]">
          Loading…
        </div>
      ) : libraries.length === 0 ? (
        <div className="rounded-xl border border-dashed border-[var(--color-border)] bg-[var(--color-card)] p-10 text-center">
          <LibraryIcon className="mx-auto mb-3 h-8 w-8 text-[var(--color-muted-foreground)]" />
          <h2 className="text-base font-medium">No libraries yet</h2>
          <p className="mx-auto mt-1 max-w-md text-sm text-[var(--color-muted-foreground)]">
            Run a search in Discover and we'll save the downloaded documents here.
          </p>
          <Button className="mt-4" onClick={() => setView("discover")}>
            Go to Discover
            <ArrowRight className="h-4 w-4" />
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
          {libraries.map((l) => {
            const isActive = activeLibrary?.path === l.path;
            const isExporting = exportingPath === l.path;
            return (
              <div
                key={l.path}
                className={[
                  "flex flex-col gap-3 rounded-xl border p-4 text-left",
                  isActive
                    ? "border-[var(--color-primary)]/60 bg-[var(--color-primary)]/5"
                    : "border-[var(--color-border)] bg-[var(--color-card)]",
                ].join(" ")}
              >
                <button
                  type="button"
                  onClick={() => setActiveLibrary(l)}
                  className="text-left"
                >
                  <div className="truncate text-sm font-medium" title={l.query}>
                    {l.query}
                  </div>
                  <div className="mt-1 flex items-center gap-2 text-xs text-[var(--color-muted-foreground)]">
                    <span>{l.n_docs} document{l.n_docs === 1 ? "" : "s"}</span>
                    {l.size_bytes > 0 && (
                      <>
                        <span>·</span>
                        <span>{formatBytes(l.size_bytes)}</span>
                      </>
                    )}
                  </div>
                </button>

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    size="sm"
                    onClick={() => exportZip(l)}
                    disabled={isExporting}
                  >
                    {isExporting ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <FileArchive className="h-3.5 w-3.5" />
                    )}
                    {isExporting ? "Zipping…" : "Export ZIP"}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => api.revealInFinder(l.path)}
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                    Show
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
