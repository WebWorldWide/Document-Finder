import { For } from "solid-js";
import {
  theme,
  accent,
  density,
  streamLayout,
  setTheme,
  setAccent,
  setDensity,
  setStreamLayout,
  THEME_META,
  ACCENT_META,
} from "@/stores/theme";

/** Editorial theming controls: base theme, accent, density, and stream layout. */
export default function ThemePicker() {
  return (
    <div style={{ display: "flex", "flex-direction": "column", gap: "16px" }}>
      <div class="df-field" style={{ gap: "8px" }}>
        <label>Theme</label>
        <div style={{ display: "flex", gap: "8px" }}>
          <For each={THEME_META}>
            {(t) => (
              <button
                class="df-btn sm"
                aria-pressed={theme() === t.id}
                onClick={() => setTheme(t.id)}
                style={{
                  flex: 1,
                  "flex-direction": "column",
                  "align-items": "stretch",
                  gap: "6px",
                  padding: "8px",
                  ...(theme() === t.id
                    ? { "border-color": "var(--accent)", color: "var(--accent)" }
                    : {}),
                }}
              >
                <span
                  style={{
                    display: "block",
                    width: "100%",
                    height: "22px",
                    "border-radius": "4px",
                    background: t.swatch,
                    "box-shadow": "inset 0 0 0 0.5px var(--line-2)",
                  }}
                />
                {t.label}
              </button>
            )}
          </For>
        </div>
      </div>

      <div class="df-field" style={{ gap: "8px" }}>
        <label>Accent</label>
        {/* Plain buttons with aria-pressed (like the Theme buttons), NOT
            role=radiogroup/radio: the radio roles promise arrow-key roving
            navigation we don't implement, so announced semantics would mismatch
            behavior. A labelled group keeps them associated. */}
        <div class="df-swatches" role="group" aria-label="Accent color">
          <For each={ACCENT_META}>
            {(a) => (
              <button
                class="df-swatch"
                aria-pressed={accent() === a.id}
                aria-label={a.id}
                title={a.id}
                style={{ background: a.color }}
                onClick={() => setAccent(a.id)}
              />
            )}
          </For>
        </div>
      </div>

      <div style={{ display: "flex", gap: "24px", "flex-wrap": "wrap" }}>
        <div class="df-field" style={{ gap: "8px" }}>
          <label>Density</label>
          <div class="df-seg">
            <button
              class={density() === "compact" ? "on" : ""}
              aria-pressed={density() === "compact"}
              onClick={() => setDensity("compact")}
            >
              Compact
            </button>
            <button
              class={density() === "regular" ? "on" : ""}
              aria-pressed={density() === "regular"}
              onClick={() => setDensity("regular")}
            >
              Regular
            </button>
          </div>
        </div>
        <div class="df-field" style={{ gap: "8px" }}>
          <label>Stream layout</label>
          <div class="df-seg">
            <button
              class={streamLayout() === "stacked" ? "on" : ""}
              aria-pressed={streamLayout() === "stacked"}
              onClick={() => setStreamLayout("stacked")}
            >
              Stacked
            </button>
            <button
              class={streamLayout() === "split" ? "on" : ""}
              aria-pressed={streamLayout() === "split"}
              onClick={() => setStreamLayout("split")}
            >
              Split
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
