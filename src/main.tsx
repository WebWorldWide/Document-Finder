import "./styles/globals.css";
import "./stores/theme"; // apply data-theme before first paint
import { render } from "solid-js/web";
import App from "./App";
import { listenAll } from "./lib/events";
import { runStore } from "./stores/run";
import { uiStore } from "./stores/ui";

listenAll((ev) => runStore.apply(ev))
  .then((fn) => {
    (window as unknown as { __dfUnlisten: () => void }).__dfUnlisten = fn;
  })
  .catch((e) => {
    // If registering the Tauri event listeners fails, the entire live UI would
    // silently never update (no progress, no results). Surface it instead of
    // leaving an unhandled rejection and a frozen-looking app.
    console.error("Failed to register Tauri event listeners — live updates disabled:", e);
    uiStore.setListenersReady(false);
  });
render(() => <App />, document.getElementById("app")!);
