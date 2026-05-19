import "./styles/globals.css";
import { render } from "solid-js/web";
import App from "./App";
import { listenAll } from "./lib/events";
import { runStore } from "./stores/run";
import { applyAttrs, settings } from "./stores/settings";
import { installGlobalHandlers, log } from "./lib/log";

// Catch unhandled errors and promise rejections before they vanish into
// devtools. Must come first so any error during early boot also surfaces.
installGlobalHandlers();
log.info("boot", "Document Finder starting", {
  theme: settings.theme,
  accent: settings.accent,
  density: settings.density,
});

// Stamp persisted theme + accent + density onto <body> before mount so the
// very first paint already has the right colors. data-attrs are seeded in
// index.html; applyAttrs overrides from localStorage if the user has
// switched.
applyAttrs();

// Tauri events fire from the Rust orchestrator. We log every non-firehose
// event at debug level so the Logs panel can surface what the backend is
// doing without drowning in download_progress noise.
const NOISY = new Set(["download_progress", "found"]);
listenAll((ev) => {
  if (!NOISY.has(ev.type)) {
    const lvl =
      ev.type === "error" ? "error"
      : ev.type === "source_error" ? "warn"
      : "debug";
    log[lvl]("backend", `event ${ev.type}`, ev.payload);
  }
  runStore.apply(ev);
})
  .then((fn) => {
    (window as { __dfUnlisten?: () => void }).__dfUnlisten = fn;
    log.info("boot", "tauri event bridge ready");
  })
  .catch((e) => log.error("boot", "tauri event bridge failed", e));

render(() => <App />, document.getElementById("app")!);
