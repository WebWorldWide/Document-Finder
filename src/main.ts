import "./styles/globals.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { listen } from "@tauri-apps/api/event";
import { runStore } from "./stores/run.svelte";
import type { DfEvent } from "./lib/events";

// Global Tauri event listener
listen<DfEvent>("df_event", (event) => {
  runStore.apply(event.payload);
});

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
