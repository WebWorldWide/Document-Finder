import { createSignal } from "solid-js";
import type { LibraryInfo } from "@/lib/tauri";

export type View = "find" | "library" | "settings";

const [view, setView] = createSignal<View>("find");
const [activeLibrary, setActiveLibrary] = createSignal<LibraryInfo | null>(null);
const [knownLibraries, setKnownLibraries] = createSignal<LibraryInfo[]>([]);

export const uiStore = {
  get view() { return view(); },
  setView,
  get activeLibrary() { return activeLibrary(); },
  setActiveLibrary,
  /// Cached list of libraries on disk, populated by App.tsx on mount and
  /// refreshed by LibraryView. Read by Sidebar for the Recent section and
  /// the live "Library N" count badge.
  get knownLibraries() { return knownLibraries(); },
  setKnownLibraries,
};
