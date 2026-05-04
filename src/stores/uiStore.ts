import { create } from "zustand";
import type { LibraryInfo } from "@/lib/tauri";

export type ViewId = "discover" | "library" | "settings";

interface UIState {
  view: ViewId;
  activeLibrary: LibraryInfo | null;

  setView: (v: ViewId) => void;
  setActiveLibrary: (l: LibraryInfo | null) => void;
}

export const useUI = create<UIState>((set) => ({
  view: "discover",
  activeLibrary: null,
  setView: (view) => set({ view }),
  setActiveLibrary: (activeLibrary) => set({ activeLibrary }),
}));
