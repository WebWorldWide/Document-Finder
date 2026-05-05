import { api } from "@/lib/tauri";
import { ALL_SOURCES, type SourceId } from "@/lib/utils";

export class SettingsStore {
  libraryRoot = $state("");
  perSource = $state(100);
  maxTotal = $state(500);
  concurrency = $state(8);
  selectedSources = $state<SourceId[]>([...ALL_SOURCES]);

  constructor() {
    this.load();
  }

  async load() {
    try {
      const saved = localStorage.getItem("df_settings");
      if (saved) {
        const parsed = JSON.parse(saved);
        this.libraryRoot = parsed.libraryRoot || "";
        this.perSource = parsed.perSource || 100;
        this.maxTotal = parsed.maxTotal || 500;
        this.concurrency = parsed.concurrency || 8;
        this.selectedSources = parsed.selectedSources || [...ALL_SOURCES];
      }
    } catch {
      // ignore
    }

    if (!this.libraryRoot) {
      try {
        const d = await api.defaultLibraryDir();
        this.libraryRoot = d.library_root;
      } catch {
        // ignore
      }
    }
  }

  save() {
    localStorage.setItem(
      "df_settings",
      JSON.stringify({
        libraryRoot: this.libraryRoot,
        perSource: this.perSource,
        maxTotal: this.maxTotal,
        concurrency: this.concurrency,
        selectedSources: $state.snapshot(this.selectedSources),
      })
    );
  }

  toggleSource(id: SourceId) {
    if (this.selectedSources.includes(id)) {
      this.selectedSources = this.selectedSources.filter((s) => s !== id);
    } else {
      this.selectedSources.push(id);
    }
    this.save();
  }

  set(updates: Partial<SettingsStore>) {
    Object.assign(this, updates);
    this.save();
  }
}

export const settingsStore = new SettingsStore();
