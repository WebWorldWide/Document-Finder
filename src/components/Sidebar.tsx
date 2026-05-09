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
    <nav class="flex h-full w-56 flex-col bg-[var(--color-canvas)] pt-8 pb-4 px-3 shrink-0">
      {/* Header — raised brand panel; emerald to match the logo gradient */}
      <div class="surface-raised-sm mx-1 mb-5 flex items-center gap-2.5 px-3 py-2.5">
        <div
          class="flex h-7 w-7 items-center justify-center rounded-lg text-white shrink-0"
          style={{
            background:
              "linear-gradient(135deg, var(--color-accent-warm) 0%, var(--color-primary) 60%, var(--color-accent-cool) 100%)",
            "box-shadow": "var(--shadow-raised-xs)",
          }}
        >
          <Sparkles size={14} />
        </div>
        <span class="text-[13px] font-semibold tracking-tight">Document Finder</span>
      </div>

      {/* Active library — floating tile inside the canvas */}
      <Show when={uiStore.activeLibrary}>
        {(lib) => (
          <div class="surface-raised-sm mx-1 mb-5 p-3">
            <p class="text-[10px] font-medium uppercase tracking-wider text-[var(--color-foreground-muted)] mb-1">
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
      <div class="surface-raised-sm mx-1 p-1.5 flex flex-col gap-1">
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
    </nav>
  );
}
