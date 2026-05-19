import { For } from "solid-js";
import { theme, applyTheme, THEME_META, type Theme } from "@/stores/theme";

const THEMES: Theme[] = ["warm-light", "warm-dark", "apple-light", "apple-dark"];

const SWATCHES: Record<Theme, { bg: string; surface: string; text: string; accent: string }> = {
  "warm-light": { bg: "#ede9e4", surface: "#e8e3de", text: "#2a2520", accent: "#3b5fd6" },
  "warm-dark": { bg: "#2a2520", surface: "#332e28", text: "#ece7e1", accent: "#7a96f0" },
  "apple-light": { bg: "#f2f2f7", surface: "#ffffff", text: "#1c1c1e", accent: "#007aff" },
  "apple-dark": { bg: "#1c1c1e", surface: "#2c2c2e", text: "#f2f2f7", accent: "#0a84ff" },
};

export default function ThemePicker() {
  return (
    <div class="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Color theme">
      <For each={THEMES}>
        {(t) => {
          const meta = THEME_META[t];
          const sw = SWATCHES[t];
          const active = () => theme() === t;
          return (
            <button
              role="radio"
              aria-checked={active()}
              aria-label={`${meta.label} ${meta.palette}`}
              onClick={() => applyTheme(t)}
              class="relative flex flex-col items-start gap-1.5 rounded-lg border p-2.5 text-left transition-colors"
              style={{
                background: sw.bg,
                "border-color": active() ? sw.accent : "transparent",
                "box-shadow": active()
                  ? `0 0 0 2px ${sw.accent}`
                  : "0 1px 3px rgba(0,0,0,0.12), 0 0 0 1px rgba(0,0,0,0.06)",
              }}
            >
              {/* Mini preview card */}
              <div
                class="w-full rounded"
                style={{
                  background: sw.surface,
                  height: "28px",
                  "box-shadow": "0 1px 2px rgba(0,0,0,0.10)",
                }}
              >
                <div
                  class="m-1.5 rounded"
                  style={{
                    background: sw.accent,
                    height: "6px",
                    width: "40%",
                    opacity: "0.85",
                  }}
                />
              </div>
              <div>
                <div class="text-[11px] leading-tight font-semibold" style={{ color: sw.text }}>
                  {meta.label}
                </div>
                <div class="text-[10px] leading-tight opacity-70" style={{ color: sw.text }}>
                  {meta.palette}
                </div>
              </div>
            </button>
          );
        }}
      </For>
    </div>
  );
}
