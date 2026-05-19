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
  perSource: posInt(saved.perSource, 100),
  maxTotal: posInt(saved.maxTotal, 500),
  concurrency: posInt(saved.concurrency, 8),
  selectedSources: safeSources(saved.selectedSources),
  useCitationGraph: safeBool(saved.useCitationGraph, false),
  /// Search quality preset — replaces the previous useSemanticRerank /
  /// useLlmExpansion / useLlmFilter trio. See `qualityToFlags` for the
  /// concrete flag mapping.
  quality: migrateQuality(saved),
  llmModelId: safeStr(saved.llmModelId, "qwen2.5-3b-instruct-q4_k_m"),
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
      return { use_semantic_rerank: true, use_llm_expansion: false, use_llm_filter: false };
    case "thorough":
      return { use_semantic_rerank: true, use_llm_expansion: true, use_llm_filter: true };
  }
}

if (!settings.libraryRoot) {
  api
    .defaultLibraryDir()
    .then(({ library_root }) => setSettings("libraryRoot", library_root))
    .catch(() => {});
}

export function saveSettings() {
  localStorage.setItem(LS_KEY, JSON.stringify(settings));
}

export function toggleSource(id: SourceId) {
  setSettings("selectedSources", (prev) =>
    prev.includes(id) ? prev.filter((s) => s !== id) : [...prev, id],
  );
  saveSettings();
}
