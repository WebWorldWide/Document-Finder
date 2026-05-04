import { FileSearch, FolderOpen, Library, Settings, Sparkles } from "lucide-react";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import { useUI, type ViewId } from "@/stores/uiStore";

interface NavItem {
  id: ViewId;
  label: string;
  icon: typeof FileSearch;
}

const NAV_ITEMS: NavItem[] = [
  { id: "discover", label: "Discover", icon: FileSearch },
  { id: "library", label: "Library", icon: Library },
  { id: "settings", label: "Settings", icon: Settings },
];

export function Sidebar() {
  const view = useUI((s) => s.view);
  const setView = useUI((s) => s.setView);
  const activeLibrary = useUI((s) => s.activeLibrary);

  return (
    <aside className="flex h-full w-60 shrink-0 flex-col border-r border-[var(--color-border)] bg-[var(--color-muted)]">
      <div
        className="flex items-center gap-2 px-4 py-3"
        data-tauri-drag-region
      >
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-[var(--color-primary)]/15">
          <Sparkles className="h-4 w-4 text-[var(--color-primary)]" />
        </div>
        <div className="text-sm font-semibold leading-none">Document Finder</div>
      </div>

      <div className="px-3 pb-3">
        <div className="mb-1 px-2 text-[10px] font-semibold uppercase tracking-widest text-[var(--color-muted-foreground)]">
          Active library
        </div>
        {activeLibrary ? (
          <div className="rounded-md border border-[var(--color-border)] bg-[var(--color-card)] p-2.5">
            <div className="truncate text-sm font-medium" title={activeLibrary.query}>
              {activeLibrary.query}
            </div>
            <div className="mt-0.5 flex items-center gap-2 text-[11px] text-[var(--color-muted-foreground)]">
              <span>{activeLibrary.n_docs} docs</span>
              {activeLibrary.size_bytes > 0 && (
                <>
                  <span>·</span>
                  <span>{formatBytes(activeLibrary.size_bytes)}</span>
                </>
              )}
            </div>
            <button
              type="button"
              onClick={() => api.revealInFinder(activeLibrary.path)}
              className="mt-2 flex items-center gap-1.5 text-[11px] text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
            >
              <FolderOpen className="h-3 w-3" />
              Show in Finder
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setView("library")}
            className="w-full rounded-md border border-dashed border-[var(--color-border)] bg-transparent p-2.5 text-left text-xs text-[var(--color-muted-foreground)] hover:border-[var(--color-primary)]/40 hover:text-[var(--color-foreground)]"
          >
            No library selected — pick one →
          </button>
        )}
      </div>

      <nav className="flex flex-col gap-0.5 px-2">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const isActive = view === item.id;
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => setView(item.id)}
              className={[
                "flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors",
                isActive
                  ? "bg-[var(--color-primary)]/10 text-[var(--color-primary)]"
                  : "text-[var(--color-foreground)]/85 hover:bg-[var(--color-accent)]",
              ].join(" ")}
            >
              <Icon className="h-4 w-4" />
              <span className="flex-1">{item.label}</span>
            </button>
          );
        })}
      </nav>
    </aside>
  );
}
