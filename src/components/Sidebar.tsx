import { Show, For } from "solid-js";
import { Sparkles, FileSearch, BookOpen, Settings, FolderOpen } from "lucide-solid";
import { uiStore } from "@/stores/ui";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

const navItems = [
  { id: "find" as const, label: "Discover", icon: FileSearch },
  { id: "library" as const, label: "Library", icon: BookOpen },
  { id: "settings" as const, label: "Settings", icon: Settings },
];

export default function Sidebar() {
  return (
    <nav class="flex h-full w-52 flex-col border-r border-[var(--color-border)] bg-[var(--color-card)] pt-8 shrink-0">
      {/* Header */}
      <div class="flex items-center gap-2.5 px-4 pb-5">
        <div class="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--color-primary)] text-white shrink-0">
          <Sparkles size={14} />
        </div>
        <span class="text-sm font-semibold tracking-tight">Document Finder</span>
      </div>

      {/* Active library */}
      <Show when={uiStore.activeLibrary}>
        {(lib) => (
          <div class="mx-3 mb-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-muted)] p-3">
            <p class="text-[10px] font-medium uppercase tracking-wider text-[var(--color-muted-foreground)] mb-1">Active Library</p>
            <button
              class="w-full text-left"
              aria-label={`Open library: ${lib().query ?? lib().name}`}
              onClick={() => {
                uiStore.setView("library");
                requestAnimationFrame(() =>
                  (document.getElementById("main-content") as HTMLElement)?.focus()
                );
              }}
            >
              <p class="truncate text-xs font-medium">{lib().query ?? lib().name}</p>
              <p class="text-[10px] text-[var(--color-muted-foreground)] mt-0.5">
                {lib().n_docs} docs · {formatBytes(lib().size_bytes)}
              </p>
            </button>
            <button
              onClick={() => api.revealInFinder(lib().path)}
              aria-label={`Reveal ${lib().query ?? lib().name} in Finder`}
              class="mt-2 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-[11px] text-[var(--color-muted-foreground)] hover:bg-[var(--color-accent)] transition-colors"
            >
              <FolderOpen size={11} />
              Show in Finder
            </button>
          </div>
        )}
      </Show>

      {/* Nav */}
      <div class="flex flex-col gap-0.5 px-2 flex-1">
        <For each={navItems}>
          {(item) => (
            <button
              onClick={() => {
                uiStore.setView(item.id);
                requestAnimationFrame(() =>
                  (document.getElementById("main-content") as HTMLElement)?.focus()
                );
              }}
              classList={{
                "flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium transition-colors w-full text-left": true,
                "bg-[var(--color-primary)] text-white": uiStore.view === item.id,
                "text-[var(--color-foreground)] hover:bg-[var(--color-accent)]": uiStore.view !== item.id,
              }}
            >
              <item.icon size={16} />
              {item.label}
            </button>
          )}
        </For>
      </div>
    </nav>
  );
}
