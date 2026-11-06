import { createSignal } from "solid-js";

/** Appearance preference. "system" follows the OS light/dark setting. */
export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const KEY = "df-theme-v3";
const PREFS: readonly ThemePref[] = ["system", "light", "dark"];

function load(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v && (PREFS as readonly string[]).includes(v) ? (v as ThemePref) : "system";
  } catch {
    return "system";
  }
}

const media =
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

function resolve(p: ThemePref): ResolvedTheme {
  if (p === "system") return media?.matches ? "dark" : "light";
  return p;
}

const [pref, setPrefSignal] = createSignal<ThemePref>(load());
const [resolved, setResolved] = createSignal<ResolvedTheme>(resolve(pref()));

function apply() {
  const r = resolve(pref());
  setResolved(r);
  document.documentElement.dataset.theme = r;
}
apply();
media?.addEventListener?.("change", () => {
  if (pref() === "system") apply();
});

export const themePref = pref;
export const resolvedTheme = resolved;

export function setThemePref(p: ThemePref) {
  setPrefSignal(p);
  try {
    localStorage.setItem(KEY, p);
  } catch {
    /* storage disabled: preference just won't persist */
  }
  apply();
}
