import { Match, Switch } from "solid-js";
import Sidebar from "@/components/Sidebar";
import Discover from "@/components/Discover";
import LibraryView from "@/components/LibraryView";
import SettingsView from "@/components/SettingsView";
import Overlays from "@/components/Overlays";
import { uiStore } from "@/stores/ui";

export default function App() {
  return (
    <div class="df-shell">
      <Sidebar />
      <main id="main-content" tabindex="-1" class="df-main">
        <Switch>
          <Match when={uiStore.view === "discover"}>
            <Discover />
          </Match>
          <Match when={uiStore.view === "library"}>
            <LibraryView />
          </Match>
          <Match when={uiStore.view === "settings"}>
            <SettingsView />
          </Match>
        </Switch>
      </main>
      <Overlays />
      <div role="status" aria-live="polite" class="sr-only">
        {uiStore.announcement}
      </div>
    </div>
  );
}
