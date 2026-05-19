import { createStore } from "solid-js/store";
import { api } from "@/lib/tauri";
import { ALL_SOURCES, type SourceId } from "@/lib/utils";

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
  if (!Array.isArray(v)) return [...ALL_SOURCES] as SourceId[];
  const valid = v.filter((s): s is SourceId => ALL_SOURCES.includes(s as SourceId));
  return valid.length > 0 ? valid : ([...ALL_SOURCES] as SourceId[]);
}
function safeUrl(v: unknown, fallback: string): string {
  if (typeof v !== "string") return fallback;
  if (v.trim() === "") return "";
  try { const u = new URL(v); if (u.protocol === "http:" || u.protocol === "https:") return v; } catch {}
  return fallback;
}

function migrateSearxng(v: unknown): string {
  if (v === "http://localhost:8080") return "";
  return safeUrl(v, "");
}

export type Theme = "paper" | "slate" | "midnight";
export type Accent =
  | "sky" | "blue" | "ink" | "electric" | "teal"
  | "emerald" | "amber" | "crimson" | "plum";
export type Density = "compact" | "regular";
export type StreamLayout = "stacked" | "split";

export const THEMES: readonly Theme[] = ["paper", "slate", "midnight"] as const;
export const ACCENTS: readonly Accent[] = [
  "sky", "blue", "ink", "electric", "teal", "emerald", "amber", "crimson", "plum",
] as const;
export const DENSITIES: readonly Density[] = ["compact", "regular"] as const;
export const STREAM_LAYOUTS: readonly StreamLayout[] = ["stacked", "split"] as const;

function safeTheme(v: unknown): Theme {
  return THEMES.includes(v as Theme) ? (v as Theme) : "slate";
}
function safeAccent(v: unknown): Accent {
  return ACCENTS.includes(v as Accent) ? (v as Accent) : "sky";
}
function safeDensity(v: unknown): Density {
  return DENSITIES.includes(v as Density) ? (v as Density) : "regular";
}
function safeStreamLayout(v: unknown): StreamLayout {
  return STREAM_LAYOUTS.includes(v as StreamLayout) ? (v as StreamLayout) : "stacked";
}

export const [settings, setSettings] = createStore({
  libraryRoot:     safeStr(saved.libraryRoot, ""),
  perSource:       posInt(saved.perSource, 100),
  maxTotal:        posInt(saved.maxTotal, 500),
  concurrency:     posInt(saved.concurrency, 8),
  selectedSources: safeSources(saved.selectedSources),
  searxngUrl:      migrateSearxng(saved.searxngUrl),
  theme:           safeTheme(saved.theme),
  accent:          safeAccent(saved.accent),
  density:         safeDensity(saved.density),
  streamLayout:    safeStreamLayout(saved.streamLayout),
});

if (!settings.libraryRoot) {
  api.defaultLibraryDir()
    .then(({ library_root }) => setSettings("libraryRoot", library_root))
    .catch(() => {});
}

export function saveSettings() {
  localStorage.setItem(LS_KEY, JSON.stringify(settings));
}

/// Write theme/accent/density data-attrs to <body>. Called at app boot
/// (main.tsx) and again from ThemeAccentPicker so the UI updates instantly
/// on every change without a re-render of the whole tree.
export function applyAttrs() {
  document.body.setAttribute("data-theme", settings.theme);
  document.body.setAttribute("data-accent", settings.accent);
  document.body.setAttribute("data-density", settings.density);
}

export function setTheme(t: Theme) {
  setSettings("theme", t);
  applyAttrs();
  saveSettings();
}
export function setAccent(a: Accent) {
  setSettings("accent", a);
  applyAttrs();
  saveSettings();
}
export function setDensity(d: Density) {
  setSettings("density", d);
  applyAttrs();
  saveSettings();
}
export function setStreamLayout(l: StreamLayout) {
  setSettings("streamLayout", l);
  saveSettings();
}

export function toggleSource(id: SourceId) {
  setSettings("selectedSources", (prev) =>
    prev.includes(id) ? prev.filter((s) => s !== id) : [...prev, id]
  );
  saveSettings();
}
