import { createSignal } from "solid-js";
import type { LibraryInfo } from "@/lib/tauri";

export type View = "find" | "library" | "settings";

const [view, setView] = createSignal<View>("find");
const [activeLibrary, setActiveLibrary] = createSignal<LibraryInfo | null>(null);
// LibraryView populates this whenever it loads; the Sidebar reads it for the
// stats tile so we don't double-fetch.
const [knownLibraries, setKnownLibraries] = createSignal<LibraryInfo[]>([]);
// False if the Tauri event listeners failed to register at startup — the live UI
// (progress, results) can't update in that state, so the Sidebar surfaces it
// instead of claiming "Backend ready". Set from main.tsx.
const [listenersReady, setListenersReady] = createSignal(true);

export const uiStore = {
  get view() {
    return view();
  },
  setView,
  get activeLibrary() {
    return activeLibrary();
  },
  setActiveLibrary,
  get knownLibraries() {
    return knownLibraries();
  },
  setKnownLibraries,
  get listenersReady() {
    return listenersReady();
  },
  setListenersReady,
  /// Aggregate stats across all loaded libraries — count + total bytes + total docs.
  get lifetimeStats() {
    const libs = knownLibraries();
    return {
      count: libs.length,
      totalBytes: libs.reduce((acc, l) => acc + l.size_bytes, 0),
      totalDocs: libs.reduce((acc, l) => acc + l.n_docs, 0),
    };
  },
};
