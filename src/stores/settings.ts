import { createStore } from "solid-js/store";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, DEFAULT_ENABLED_SOURCES, type SourceId } from "@/lib/utils";

// The only persisted settings: where the library lives and which sources to
// search. Everything about *how much* to search and download is automatic.
const LS_KEY = "df-settings-v2";

function loadSaved(): Record<string, unknown> {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

export function safeSources(v: unknown): SourceId[] {
  if (!Array.isArray(v)) return [...DEFAULT_ENABLED_SOURCES];
  const valid = v.filter((s): s is SourceId => ALL_SOURCES.includes(s as SourceId));
  return valid.length > 0 ? valid : [...DEFAULT_ENABLED_SOURCES];
}

const saved = loadSaved();

export const [settings, setSettings] = createStore({
  libraryRoot: typeof saved.libraryRoot === "string" ? saved.libraryRoot : "",
  selectedSources: safeSources(saved.selectedSources),
});

export function saveSettings() {
  try {
    localStorage.setItem(
      LS_KEY,
      JSON.stringify({
        libraryRoot: settings.libraryRoot,
        selectedSources: settings.selectedSources,
      }),
    );
  } catch (e) {
    console.error("saveSettings failed (settings not persisted):", e);
  }
}

/** Tell the backend which root every path command is confined to. */
export async function syncLibraryRoot(root: string): Promise<string> {
  const canonical = await api.setLibraryRoot(root);
  if (canonical && canonical !== settings.libraryRoot) {
    setSettings("libraryRoot", canonical);
    saveSettings();
  }
  return canonical;
}

export function initLibraryRoot() {
  const start = settings.libraryRoot
    ? Promise.resolve(settings.libraryRoot)
    : api.defaultLibraryDir().then((r) => r?.library_root ?? "");
  return start
    .then((root) => (root ? syncLibraryRoot(root) : ""))
    .catch((e) => {
      console.error("set_library_root failed:", e);
      return "";
    });
}

export function toggleSource(id: SourceId) {
  setSettings("selectedSources", (prev) =>
    prev.includes(id) ? prev.filter((s) => s !== id) : [...prev, id],
  );
  saveSettings();
}

export function setSources(ids: SourceId[]) {
  setSettings("selectedSources", ids);
  saveSettings();
}
