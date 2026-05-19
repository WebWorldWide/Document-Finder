import { createSignal } from "solid-js";

export type Theme = "warm-light" | "warm-dark" | "apple-light" | "apple-dark";

const LS_KEY = "df-theme-v1";

function loadTheme(): Theme {
  const saved = localStorage.getItem(LS_KEY);
  if (
    saved === "warm-light" ||
    saved === "warm-dark" ||
    saved === "apple-light" ||
    saved === "apple-dark"
  ) {
    return saved;
  }
  return "warm-light";
}

const initial = loadTheme();
document.documentElement.dataset.theme = initial;

const [theme, setThemeSignal] = createSignal<Theme>(initial);

export { theme };

export function applyTheme(t: Theme) {
  document.documentElement.dataset.theme = t;
  localStorage.setItem(LS_KEY, t);
  setThemeSignal(t);
}

export const THEME_META: Record<Theme, { label: string; palette: string }> = {
  "warm-light": { label: "Warm", palette: "Light" },
  "warm-dark": { label: "Warm", palette: "Dark" },
  "apple-light": { label: "Apple HIG", palette: "Light" },
  "apple-dark": { label: "Apple HIG", palette: "Dark" },
};
