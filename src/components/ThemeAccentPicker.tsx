import { For } from "solid-js";
import {
  settings,
  setTheme,
  setAccent,
  THEMES,
  ACCENTS,
  type Theme,
  type Accent,
} from "@/stores/settings";

const THEME_LABELS: Record<Theme, string> = {
  paper: "Paper",
  slate: "Slate",
  midnight: "Dark",
};

// Hex swatches matching styles/globals.css [data-accent] rules.
const ACCENT_HEX: Record<Accent, string> = {
  sky: "#3b82f6",
  blue: "#2549c9",
  ink: "#19245a",
  electric: "#1a6cff",
  teal: "#0f7d8f",
  emerald: "#2f7a52",
  amber: "#b4651e",
  crimson: "#a83a55",
  plum: "#5d2e7c",
};

// Density and Stream layout are still in settings.ts (with defaults
// "regular" and "stacked") for backward compatibility — anyone who set
// them previously keeps their preference. They're just not exposed
// here anymore. Removing them was a deliberate design call: this panel
// is for visual identity, not behavior knobs.

export default function ThemeAccentPicker() {
  return (
    <section class="df-section">
      <h2>Theme & Accent</h2>
      <p class="hint">Pick a theme and accent. Choices persist across launches.</p>

      <div style={{ display: "flex", "flex-direction": "column", gap: "var(--pad-4)" }}>
        <div>
          <div
            style={{
              "margin-bottom": "8px",
              "font-size": "11px",
              "text-transform": "uppercase",
              "letter-spacing": "0.06em",
              color: "var(--ink-3)",
              "font-weight": "500",
            }}
          >
            Mode
          </div>
          <div class="df-theme-radio" role="radiogroup" aria-label="Theme">
            <For each={THEMES}>
              {(t) => (
                <button
                  aria-pressed={settings.theme === t}
                  onClick={() => setTheme(t)}
                  title={THEME_LABELS[t]}
                >
                  {THEME_LABELS[t]}
                </button>
              )}
            </For>
          </div>
        </div>

        <div>
          <div
            style={{
              "margin-bottom": "8px",
              "font-size": "11px",
              "text-transform": "uppercase",
              "letter-spacing": "0.06em",
              color: "var(--ink-3)",
              "font-weight": "500",
            }}
          >
            Accent
          </div>
          <div class="df-accent-grid" role="radiogroup" aria-label="Accent">
            <For each={ACCENTS}>
              {(a) => (
                <button
                  class="df-accent-chip"
                  aria-pressed={settings.accent === a}
                  aria-label={a}
                  title={a}
                  onClick={() => setAccent(a)}
                  style={{ "background-color": ACCENT_HEX[a] }}
                />
              )}
            </For>
          </div>
        </div>
      </div>
    </section>
  );
}
