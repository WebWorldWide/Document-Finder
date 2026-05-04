import { useEffect } from "react";
import { FindTab } from "./components/FindTab";
import { LibraryView } from "./components/LibraryView";
import { SettingsView } from "./components/SettingsView";
import { Sidebar } from "./components/Sidebar";
import { listenAll } from "./lib/events";
import { useRunStore } from "./stores/runStore";
import { useUI } from "./stores/uiStore";

export default function App() {
  const apply = useRunStore((s) => s.apply);
  const view = useUI((s) => s.view);

  useEffect(() => {
    let unsub: undefined | (() => void);
    listenAll(apply).then((u) => {
      unsub = u;
    });
    return () => {
      unsub?.();
    };
  }, [apply]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-[var(--color-background)] text-[var(--color-foreground)]">
      <Sidebar />
      <main className="flex flex-1 flex-col overflow-hidden">
        <div
          className="h-8 shrink-0 border-b border-[var(--color-border)]"
          data-tauri-drag-region
        />
        <div className="flex-1 overflow-auto p-6">
          {view === "discover" && <FindTab />}
          {view === "library" && <LibraryView />}
          {view === "settings" && <SettingsView />}
        </div>
      </main>
    </div>
  );
}
