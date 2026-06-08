import "./styles/globals.css";
import "./stores/theme"; // apply data-theme before first paint
import { render } from "solid-js/web";
import App from "./App";
import { listenAll } from "./lib/events";
import { runStore } from "./stores/run";

listenAll((ev) => runStore.apply(ev)).then((fn) => {
  (window as unknown as { __dfUnlisten: () => void }).__dfUnlisten = fn;
});
render(() => <App />, document.getElementById("app")!);
