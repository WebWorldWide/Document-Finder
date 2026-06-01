import { createSignal } from "solid-js";

// Editorial theming: a base theme (paper/slate/midnight), an accent color (9),
// UI density, and the live-stream layout. All but stream layout are applied as
// data-* attributes on <html>; stream layout only changes markup in Discover.
export type Theme = "paper" | "slate" | "midnight";
export type Accent =
  | "sky"
  | "blue"
  | "ink"
  | "electric"
  | "teal"
  | "emerald"
  | "amber"
  | "crimson"
  | "plum";
export type Density = "compact" | "regular";
export type StreamLayout = "stacked" | "split";

const LS = {
  theme: "df-theme-v2",
  accent: "df-accent-v2",
  density: "df-density-v2",
  stream: "df-stream-v2",
} as const;

const THEMES = ["paper", "slate", "midnight"] as const;
const ACCENTS = [
  "sky",
  "blue",
  "ink",
  "electric",
  "teal",
  "emerald",
  "amber",
  "crimson",
  "plum",
] as const;
const DENSITIES = ["compact", "regular"] as const;
const STREAMS = ["stacked", "split"] as const;

function load<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  const v = localStorage.getItem(key);
  return v && (allowed as readonly string[]).includes(v) ? (v as T) : fallback;
}

const initTheme = load<Theme>(LS.theme, THEMES, "slate");
const initAccent = load<Accent>(LS.accent, ACCENTS, "sky");
const initDensity = load<Density>(LS.density, DENSITIES, "regular");
const initStream = load<StreamLayout>(LS.stream, STREAMS, "split");

const root = document.documentElement;
root.dataset.theme = initTheme;
root.dataset.accent = initAccent;
root.dataset.density = initDensity;

const [theme, setThemeSignal] = createSignal<Theme>(initTheme);
const [accent, setAccentSignal] = createSignal<Accent>(initAccent);
const [density, setDensitySignal] = createSignal<Density>(initDensity);
const [streamLayout, setStreamSignal] = createSignal<StreamLayout>(initStream);

export { theme, accent, density, streamLayout };

export function setTheme(t: Theme) {
  root.dataset.theme = t;
  localStorage.setItem(LS.theme, t);
  setThemeSignal(t);
}
export function setAccent(a: Accent) {
  root.dataset.accent = a;
  localStorage.setItem(LS.accent, a);
  setAccentSignal(a);
}
export function setDensity(d: Density) {
  root.dataset.density = d;
  localStorage.setItem(LS.density, d);
  setDensitySignal(d);
}
export function setStreamLayout(s: StreamLayout) {
  localStorage.setItem(LS.stream, s);
  setStreamSignal(s);
}

export const THEME_META: { id: Theme; label: string; swatch: string }[] = [
  { id: "paper", label: "Paper", swatch: "#fbf8f1" },
  { id: "slate", label: "Slate", swatch: "#f7f8fa" },
  { id: "midnight", label: "Midnight", swatch: "#151820" },
];

export const ACCENT_META: { id: Accent; color: string }[] = [
  { id: "sky", color: "#3b82f6" },
  { id: "blue", color: "#2549c9" },
  { id: "ink", color: "#19245a" },
  { id: "electric", color: "#1a6cff" },
  { id: "teal", color: "#0f7d8f" },
  { id: "emerald", color: "#2f7a52" },
  { id: "amber", color: "#b4651e" },
  { id: "crimson", color: "#a83a55" },
  { id: "plum", color: "#5d2e7c" },
];
