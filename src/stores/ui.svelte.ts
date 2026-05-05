import type { LibraryInfo } from "@/lib/tauri";

export type View = "find" | "library" | "settings";

export class UIStore {
  view = $state<View>("find");
  activeLibrary = $state<LibraryInfo | null>(null);

  setView(v: View) {
    this.view = v;
  }

  setActiveLibrary(lib: LibraryInfo | null) {
    this.activeLibrary = lib;
    if (lib) {
        this.view = "library";
    }
  }
}

export const uiStore = new UIStore();
