import { Show, For, onMount } from "solid-js";
import { Sparkles, FileSearch, BookOpen, Settings, FolderOpen, Library } from "lucide-solid";
import { uiStore } from "@/stores/ui";
import { settings } from "@/stores/settings";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";

const navItems = [
  { id: "find" as const, label: "Discover", icon: FileSearch },
  { id: "library" as const, label: "Library", icon: BookOpen },
  { id: "settings" as const, label: "Settings", icon: Settings },
];

export default function Sidebar() {
  // Lazy-prime the stats tile when the sidebar mounts so the first paint
  // shows real numbers if any libraries already exist on disk.
  onMount(() => {
    if (uiStore.knownLibraries.length === 0 && settings.libraryRoot) {
      api.listLibraries(settings.libraryRoot)
        .then((libs) => uiStore.setKnownLibraries(libs))
        .catch(() => {});
    }
  });

  const stats = () => uiStore.lifetimeStats;

  return (
    <nav class="material-aluminum flex h-full w-56 flex-col pt-8 pb-4 px-3 shrink-0" style={{ "border-radius": 0 }}>
      {/* Header — embossed brand chip on brushed-metal canvas */}
      <div class="material-paper border-stitched mx-1 mb-5 flex items-center gap-2.5 px-3 py-2.5">
        <div
          class="surface-glossy flex h-7 w-7 items-center justify-center rounded-lg text-white shrink-0"
          style={{
            background:
              "linear-gradient(135deg, var(--color-accent-warm) 0%, var(--color-primary) 60%, var(--color-accent-cool) 100%)",
            "box-shadow": "var(--shadow-raised-xs), inset 0 1px 0 oklch(1 0 0 / 0.6), inset 0 -1px 0 oklch(0 0 0 / 0.15)",
          }}
        >
          <Sparkles size={14} />
        </div>
        <span class="text-[13px] font-semibold tracking-tight text-embossed">Document Finder</span>
      </div>

      {/* Active library — floating tile inside the canvas */}
      <Show when={uiStore.activeLibrary}>
        {(lib) => (
          <div class="material-paper border-stitched mx-1 mb-5 p-3">
            <p class="text-[10px] font-medium uppercase tracking-wider text-[var(--color-foreground-muted)] mb-1 text-embossed">
              Active Library
            </p>
            <button
              class="w-full text-left focus-ring-inset rounded-md"
              aria-label={`Open library: ${lib().query ?? lib().name}`}
              onClick={() => {
                uiStore.setView("library");
                requestAnimationFrame(() =>
                  (document.getElementById("main-content") as HTMLElement)?.focus()
                );
              }}
            >
              <p class="truncate text-xs font-medium">{lib().query ?? lib().name}</p>
              <p class="text-[10px] text-[var(--color-foreground-muted)] mt-0.5">
                {lib().n_docs} docs · {formatBytes(lib().size_bytes)}
              </p>
            </button>
            <button
              onClick={() => api.revealInFinder(lib().path)}
              aria-label={`Reveal ${lib().query ?? lib().name} in Finder`}
              class="btn-tactile mt-2 flex w-full items-center justify-center gap-1.5 px-2 py-1 text-[11px] text-[var(--color-foreground-muted)]"
            >
              <FolderOpen size={11} />
              Show in Finder
            </button>
          </div>
        )}
      </Show>

      {/* Nav — single raised container, flat buttons inside.
       * Eliminates the per-button shadow blot between siblings. */}
      <div class="material-linen mx-1 p-1.5 flex flex-col gap-1">
        <For each={navItems}>
          {(item) => {
            const active = () => uiStore.view === item.id;
            return (
              <button
                onClick={() => {
                  uiStore.setView(item.id);
                  requestAnimationFrame(() =>
                    (document.getElementById("main-content") as HTMLElement)?.focus()
                  );
                }}
                class="flex items-center gap-3 px-3 py-2.5 text-sm font-medium w-full text-left transition-colors duration-100 focus-ring-inset"
                classList={{
                  "bg-[var(--color-primary)]/12": active(),
                  "hover:bg-[var(--color-foreground)]/4": !active(),
                  "text-[var(--color-primary)]": active(),
                  "text-[var(--color-foreground)]": !active(),
                }}
                style={{
                  "border-radius": "var(--radius-sm)",
                  ...(active()
                    ? {
                        "box-shadow": `inset 3px 0 0 0 var(--color-primary)`,
                      }
                    : {}),
                }}
              >
                <item.icon size={16} />
                {item.label}
              </button>
            );
          }}
        </For>
      </div>

      {/* Footer stats tile — paper-textured, sits at the bottom of the sidebar
       * so the otherwise-empty space below the nav carries useful info. */}
      <div class="mt-auto">
        <div class="material-paper border-stitched mx-1 px-3 py-2.5">
          <div class="flex items-center gap-2 mb-1.5">
            <Library size={12} class="text-[var(--color-foreground-muted)]" />
            <p class="text-[9px] font-semibold uppercase tracking-wider text-[var(--color-foreground-muted)] text-embossed">
              Your collections
            </p>
          </div>
          <Show
            when={stats().count > 0}
            fallback={
              <p class="text-[10px] text-[var(--color-foreground-muted)] leading-snug">
                No libraries yet — run a search to build one.
              </p>
            }
          >
            <div class="flex items-baseline gap-2">
              <span class="font-mono text-[15px] font-semibold tabular-nums">
                {stats().count}
              </span>
              <span class="text-[10px] text-[var(--color-foreground-muted)]">
                {stats().count === 1 ? "library" : "libraries"}
              </span>
            </div>
            <p class="mt-0.5 text-[10px] text-[var(--color-foreground-muted)]">
              {stats().totalDocs} docs · {formatBytes(stats().totalBytes)}
            </p>
          </Show>
        </div>
      </div>
    </nav>
  );
}
