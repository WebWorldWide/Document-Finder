import { Switch, Match, onMount } from "solid-js";
import Sidebar from "@/components/Sidebar";
import FindTab from "@/components/FindTab";
import LibraryView from "@/components/LibraryView";
import SettingsView from "@/components/SettingsView";
import WelcomeDialog from "@/components/WelcomeDialog";
import { uiStore } from "@/stores/ui";
import { modelsStore } from "@/stores/models";

export default function App() {
  // Kick off the models registry load at app start so the AI Models card in
  // Settings doesn't get stuck on "Loading…" if neither SettingsView nor
  // FirstRunModelDialog has mounted yet.
  onMount(() => {
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  return (
    <div class="bg-pinstripe-light flex h-screen w-screen overflow-hidden text-[var(--color-foreground)]">
      {/* No custom drag region — Tauri's native macOS title bar (decorations:
       * true in tauri.conf.json) handles dragging on its own. The previous
       * fixed transparent drag region overlaid the pinstripe canvas at the
       * top of the window and showed up as a striped artifact strip. */}
      <Sidebar />
      <main id="main-content" tabindex="-1" class="flex-1 overflow-hidden outline-none">
        <Switch>
          <Match when={uiStore.view === "find"}>
            <FindTab />
          </Match>
          <Match when={uiStore.view === "library"}>
            <LibraryView />
          </Match>
          <Match when={uiStore.view === "settings"}>
            <SettingsView />
          </Match>
        </Switch>
      </main>
      <WelcomeDialog />
    </div>
  );
}
