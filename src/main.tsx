import "./styles/globals.css";
import { render } from "solid-js/web";
import App from "./App";
import { listenAll } from "./lib/events";
import { runStore } from "./stores/run";
import { applyAttrs } from "./stores/settings";

// Stamp persisted theme + accent + density onto <body> before mount so the
// very first paint already has the right colors and the user never sees a
// flash of the default. data-attrs are seeded in index.html; this overrides
// from localStorage if the user has switched.
applyAttrs();

listenAll((ev) => runStore.apply(ev)).then((fn) => {
  (window as any).__dfUnlisten = fn;
});
render(() => <App />, document.getElementById("app")!);
