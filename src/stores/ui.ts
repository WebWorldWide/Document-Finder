import { createSignal } from "solid-js";
import type { LibraryInfo } from "@/lib/tauri";

export type View = "find" | "library" | "settings";

const [view, setView] = createSignal<View>("find");
const [activeLibrary, setActiveLibrary] = createSignal<LibraryInfo | null>(null);

export const uiStore = {
  get view() { return view(); },
  setView,
  get activeLibrary() { return activeLibrary(); },
  setActiveLibrary,
};
