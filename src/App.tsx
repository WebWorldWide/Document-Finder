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
  // WelcomeDialog has mounted yet.
  onMount(() => {
    void modelsStore.refresh();
    void modelsStore.ensureSubscribed();
  });

  return (
    <div class="df-body">
      <Sidebar />
      <main id="main-content" tabindex="-1" class="df-main">
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
