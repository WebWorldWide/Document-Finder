import { createStore } from "solid-js/store";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, DEFAULT_ENABLED_SOURCES, type SourceId } from "@/lib/utils";

const LS_KEY = "df-settings-v2";

function loadSaved(): Record<string, unknown> {
  try {
    return JSON.parse(localStorage.getItem(LS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

const saved = loadSaved();

function posInt(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.floor(v) : fallback;
}
function safeStr(v: unknown, fallback: string): string {
  return typeof v === "string" && v.length > 0 ? v : fallback;
}
function safeSources(v: unknown): SourceId[] {
  if (!Array.isArray(v)) return [...DEFAULT_ENABLED_SOURCES];
  const valid = v.filter((s): s is SourceId => ALL_SOURCES.includes(s as SourceId));
  return valid.length > 0 ? valid : [...DEFAULT_ENABLED_SOURCES];
}
function safeBool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}

/// One of three search-quality presets. The orchestrator's three AI booleans
/// (semantic rerank, LLM expansion, LLM filter) are derived from this in
/// `qualityToFlags` below — single source of truth for the user.
export type Quality = "fast" | "balanced" | "thorough";

function safeQuality(v: unknown, fallback: Quality): Quality {
  return v === "fast" || v === "balanced" || v === "thorough" ? v : fallback;
}

/// Best-effort migration from the old per-flag settings (round-3) to the
/// new quality enum. Only runs once on first load if the new key is absent.
function migrateQuality(saved: Record<string, unknown>): Quality {
  if (saved.quality !== undefined) return safeQuality(saved.quality, "balanced");
  const sem = !!saved.useSemanticRerank;
  const exp = !!saved.useLlmExpansion;
  const fil = !!saved.useLlmFilter;
  if (exp || fil) return "thorough";
  if (sem) return "balanced";
  // If the user had explicitly turned everything off, respect that.
  if (saved.useSemanticRerank === false) return "fast";
  return "balanced";
}

export const [settings, setSettings] = createStore({
  libraryRoot: safeStr(saved.libraryRoot, ""),
  perSource: posInt(saved.perSource, 150),
  maxTotal: posInt(saved.maxTotal, 900),
  concurrency: posInt(saved.concurrency, 8),
  selectedSources: safeSources(saved.selectedSources),
  useCitationGraph: safeBool(saved.useCitationGraph, false),
  /// Search quality preset — replaces the previous useSemanticRerank /
  /// useLlmExpansion / useLlmFilter trio. See `qualityToFlags` for the
  /// concrete flag mapping.
  quality: migrateQuality(saved),
  // Empty string = "use the backend's default LLM" (resolved by the registry).
  // A previously-persisted id is reconciled against the live catalog on load
  // (see `reconcileLlmModel`) so a removed model never leaves a dangling id.
  llmModelId: safeStr(saved.llmModelId, ""),
  // Whether to dismiss the first-run AI download prompt (sticky once dismissed)
  aiOnboardingDismissed: safeBool(saved.aiOnboardingDismissed, false),
});

/// Maps the user-facing `quality` enum to the three booleans the
/// orchestrator's `RunRequest` still consumes. Kept here so the
/// translation lives next to the enum definition.
export function qualityToFlags(q: Quality): {
  use_semantic_rerank: boolean;
  use_llm_expansion: boolean;
  use_llm_filter: boolean;
} {
  switch (q) {
    case "fast":
      return { use_semantic_rerank: false, use_llm_expansion: false, use_llm_filter: false };
    case "balanced":
      // Expansion is enabled here so the DEFAULT preset fans a search out into
      // many related sub-queries (broad reach out-of-the-box). It runs
      // concurrently with discovery and is a silent no-op when the local LLM
      // model isn't installed, so Balanced still only *requires* the embedding
      // model. The LLM relevance filter stays off (that's Thorough's extra).
      return { use_semantic_rerank: true, use_llm_expansion: true, use_llm_filter: false };
    case "thorough":
      return { use_semantic_rerank: true, use_llm_expansion: true, use_llm_filter: true };
  }
}

// eslint-disable-next-line solid/reactivity
if (!settings.libraryRoot) {
  api
    .defaultLibraryDir()
    .then((resp) => {
      // Guard the destructure: outside a Tauri runtime (e.g. unit tests) the
      // invoke can resolve to undefined, which shouldn't log a scary error.
      const root = resp?.library_root;
      if (!root) return undefined;
      setSettings("libraryRoot", root);
      return syncLibraryRoot(root);
    })
    .catch((e) => console.error("set_library_root failed:", e));
} else {
  // Tell the backend our configured root so open/export/delete confine to it
  // (otherwise a custom location outside ~/Documents/Document Finder fails).
  // One-time module init, not a reactive scope — the read is intentional.
  // eslint-disable-next-line solid/reactivity
  syncLibraryRoot(settings.libraryRoot).catch((e) => console.error("set_library_root failed:", e));
}

export function saveSettings() {
  localStorage.setItem(LS_KEY, JSON.stringify(settings));
}

/// Clears a persisted `llmModelId` that's no longer in the model catalog
/// (e.g. a model removed between app versions). An empty id makes the backend
/// fall back to the registry default. Called once the model list is loaded.
export function reconcileLlmModel(validIds: string[]) {
  if (settings.llmModelId && !validIds.includes(settings.llmModelId)) {
    setSettings("llmModelId", "");
    saveSettings();
  }
}

export function toggleSource(id: SourceId) {
  setSettings("selectedSources", (prev) =>
    prev.includes(id) ? prev.filter((s) => s !== id) : [...prev, id],
  );
  saveSettings();
}

// --- Download intensity ----------------------------------------------------
//
// A single, friendly control that sets the three raw discovery numbers
// (per-source / max-total / parallel downloads) together, so non-technical
// users don't have to reason about each number. The raw numbers remain editable
// under "Advanced"; editing them directly moves the slider to "Custom".

export type DownloadIntensity = "light" | "balanced" | "deep" | "exhaustive";

export interface IntensityPreset {
  perSource: number;
  maxTotal: number;
  concurrency: number;
  label: string;
  blurb: string;
}

/// Ordered light→exhaustive. `balanced` matches the historical defaults so the
/// behavior of existing installs is unchanged.
export const INTENSITY_ORDER: DownloadIntensity[] = ["light", "balanced", "deep", "exhaustive"];

export const INTENSITY_PRESETS: Record<DownloadIntensity, IntensityPreset> = {
  light: {
    perSource: 40,
    maxTotal: 150,
    concurrency: 4,
    label: "Light",
    blurb: "Quick skim — fewer results, gentle on sources.",
  },
  balanced: {
    perSource: 150,
    maxTotal: 900,
    concurrency: 8,
    label: "Balanced",
    blurb: "Good for most searches — ~150/source, 900 max, 8 at a time.",
  },
  deep: {
    perSource: 350,
    maxTotal: 2500,
    concurrency: 12,
    label: "Deep",
    blurb:
      "More thorough — wider per-source pull, longer source deadlines, and (with the LLM) more sub-queries. Slower, more rate-limits.",
  },
  exhaustive: {
    perSource: 600,
    maxTotal: 6000,
    concurrency: 16,
    label: "Exhaustive",
    blurb:
      "Widest possible reach — biggest caps, longest deadlines, most sub-queries and the largest semantic-rerank pool. Slowest, heaviest on sources.",
  },
};

/// The preset matching the current raw numbers, or null when they've been
/// hand-tuned in Advanced ("Custom").
export function currentIntensity(): DownloadIntensity | null {
  for (const level of INTENSITY_ORDER) {
    const p = INTENSITY_PRESETS[level];
    if (
      settings.perSource === p.perSource &&
      settings.maxTotal === p.maxTotal &&
      settings.concurrency === p.concurrency
    ) {
      return level;
    }
  }
  return null;
}

/// Apply a preset to the three raw numbers and persist.
export function setDownloadIntensity(level: DownloadIntensity) {
  const p = INTENSITY_PRESETS[level];
  setSettings("perSource", p.perSource);
  setSettings("maxTotal", p.maxTotal);
  setSettings("concurrency", p.concurrency);
  saveSettings();
}

// --- Library root ----------------------------------------------------------

/// Push the configured library root to the backend so path commands
/// (open/export/delete) confine to it. The returned promise REJECTS on failure
/// (e.g. a path the backend can't create/resolve) so interactive callers can
/// surface it; fire-and-forget callers should attach their own `.catch`.
export function syncLibraryRoot(path: string): Promise<unknown> {
  if (!path) return Promise.resolve();
  return api.setLibraryRoot(path);
}

/// Update the library root setting, persist it, and sync it to the backend.
/// Returns the sync promise so the UI can show an error if it fails.
export function setLibraryRoot(path: string): Promise<unknown> {
  setSettings("libraryRoot", path);
  saveSettings();
  return syncLibraryRoot(path);
}
