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
    <div class="flex h-screen w-screen overflow-hidden bg-pinstripe-light text-[var(--color-foreground)]">
      {/* macOS traffic light drag region */}
      <div class="fixed inset-x-0 top-0 h-8 z-50 pointer-events-none" data-tauri-drag-region aria-hidden="true" />
      <Sidebar />
      <main id="main-content" tabindex="-1" class="flex-1 overflow-hidden outline-none">
        <Switch>
          <Match when={uiStore.view === "find"}><FindTab /></Match>
          <Match when={uiStore.view === "library"}><LibraryView /></Match>
          <Match when={uiStore.view === "settings"}><SettingsView /></Match>
        </Switch>
      </main>
      <WelcomeDialog />
    </div>
  );
}
