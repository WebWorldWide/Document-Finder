import "./styles/globals.css";
import "./stores/theme"; // applies data-theme before first paint
import { render } from "solid-js/web";
import App from "./App";
import { listenAll } from "./lib/events";
import { runStore } from "./stores/run";
import { initLibraryRoot } from "./stores/settings";
import { uiStore } from "./stores/ui";

async function boot() {
  // In a plain browser (vite dev without Tauri), install a mock backend so the
  // UI can be designed and reviewed. Never included in production builds.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installMockBackend } = await import("./dev/mock");
    installMockBackend();
  }
  try {
    const unlisten = await listenAll((ev) => runStore.apply(ev));
    (window as unknown as { __dfUnlisten: () => void }).__dfUnlisten = unlisten;
  } catch (e) {
    console.error("Failed to register Tauri event listeners — live updates disabled:", e);
    uiStore.setListenersReady(false);
  }
  void initLibraryRoot();
  render(() => <App />, document.getElementById("app")!);
}

void boot();
