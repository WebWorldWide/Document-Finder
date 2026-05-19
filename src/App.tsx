import { Switch, Match, onMount, createEffect } from "solid-js";
import Sidebar from "@/components/Sidebar";
import FindTab from "@/components/FindTab";
import LibraryView from "@/components/LibraryView";
import SettingsView from "@/components/SettingsView";
import { uiStore } from "@/stores/ui";
import { runStore } from "@/stores/run";
import { settings } from "@/stores/settings";
import { api } from "@/lib/tauri";

export default function App() {
  // Populate uiStore.knownLibraries so the sidebar's Recent section + count
  // badge have data on first paint, and refresh whenever a run completes.
  async function refreshLibraries() {
    if (!settings.libraryRoot) return;
    try {
      const libs = await api.listLibraries(settings.libraryRoot);
      uiStore.setKnownLibraries(libs);
    } catch {
      // silent — empty libraries dir is normal on first launch
    }
  }
  onMount(refreshLibraries);
  // After every run finishes, refresh so the newly-created library appears.
  createEffect(() => {
    if (!runStore.state.running && runStore.state.folder) refreshLibraries();
  });

  return (
    <div class="df-app">
      <Sidebar />
      <main id="main-content" tabindex="-1" class="df-canvas">
        <Switch>
          <Match when={uiStore.view === "find"}><FindTab /></Match>
          <Match when={uiStore.view === "library"}><LibraryView /></Match>
          <Match when={uiStore.view === "settings"}><SettingsView /></Match>
        </Switch>
      </main>
    </div>
  );
}
