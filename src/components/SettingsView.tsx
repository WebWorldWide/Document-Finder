import { createSignal, Show } from "solid-js";
import { Server, Loader2, CheckCircle2, X } from "lucide-solid";
import { api } from "@/lib/tauri";
import { settings, setSettings, saveSettings } from "@/stores/settings";
import { log } from "@/lib/log";
import ThemeAccentPicker from "./ThemeAccentPicker";
import LogsPanel from "./LogsPanel";

export default function SettingsView() {
  const [settingUpSearx, setSettingUpSearx] = createSignal(false);
  const [searxResult, setSearxResult] = createSignal<string | null>(null);
  const [searxError, setSearxError] = createSignal<string | null>(null);

  async function handleSetupSearx() {
    setSettingUpSearx(true);
    setSearxResult(null);
    setSearxError(null);
    try {
      const output = await api.setupSearXNG();
      setSearxResult(output);
      log.info("settings", "verified local SearXNG server", output);
    } catch (e) {
      setSearxError(String(e));
      log.error("settings", "SearXNG verification failed", e);
    } finally {
      setSettingUpSearx(false);
    }
  }

  function numInput(field: "perSource" | "maxTotal" | "concurrency") {
    return (e: InputEvent & { currentTarget: HTMLInputElement }) => {
      const v = parseInt(e.currentTarget.value, 10);
      if (!isNaN(v) && v > 0) {
        setSettings(field, v);
        saveSettings();
      }
    };
  }

  return (
    <div class="df-canvas">
      <div class="df-canvas-head">
        <h1 class="df-canvas-title">Settings</h1>
      </div>

      <div class="df-canvas-body">
        <div class="df-settings-wrap">
          {/* Theme + Accent first — the most user-visible knob */}
          <ThemeAccentPicker />

          {/* Discovery */}
          <section class="df-section">
            <h2>Discovery</h2>
            <p class="hint">How aggressive the search fan-out should be.</p>
            <div class="df-field-row">
              <div class="df-field">
                <label>Per source</label>
                <input
                  type="number"
                  min="1"
                  value={settings.perSource}
                  onInput={numInput("perSource")}
                />
                <span class="help">Max docs per source per sub-query</span>
              </div>
              <div class="df-field">
                <label>Max total</label>
                <input
                  type="number"
                  min="1"
                  value={settings.maxTotal}
                  onInput={numInput("maxTotal")}
                />
                <span class="help">Hard cap across all sources</span>
              </div>
              <div class="df-field">
                <label>Parallel</label>
                <input
                  type="number"
                  min="1"
                  max="32"
                  value={settings.concurrency}
                  onInput={numInput("concurrency")}
                />
                <span class="help">More = faster, hits rate limits sooner</span>
              </div>
            </div>
          </section>

          {/* Library Folder */}
          <section class="df-section">
            <h2>Library Folder</h2>
            <p class="hint">Where downloaded documents and library DBs live.</p>
            <div class="df-folder-row">
              <div class="df-field" style={{ flex: 1 }}>
                <input
                  class="mono"
                  type="text"
                  value={settings.libraryRoot}
                  onInput={(e) => {
                    setSettings("libraryRoot", e.currentTarget.value);
                    saveSettings();
                  }}
                />
              </div>
            </div>
          </section>

          {/* Local SearXNG */}
          <section class="df-section">
            <h2>Local Search Engine</h2>
            <p class="hint">
              Document Finder ships its own SearXNG-compatible search server
              built into the app. No Docker, no Python, no setup. Verify it's
              running or override with a remote SearXNG instance.
            </p>
            <div style={{ display: "flex", "flex-direction": "column", gap: "var(--pad-4)" }}>
              <button
                class="df-btn"
                onClick={handleSetupSearx}
                disabled={settingUpSearx()}
              >
                <Show when={settingUpSearx()} fallback={<Server size={14} />}>
                  <Loader2 size={14} class="spin" />
                </Show>
                {settingUpSearx() ? "Verifying…" : "Verify Local Search Engine"}
              </button>

              <div class="df-field">
                <label>SearXNG endpoint</label>
                <input
                  class="mono"
                  type="text"
                  value={settings.searxngUrl}
                  placeholder="(auto — embedded server)"
                  onInput={(e) => {
                    setSettings("searxngUrl", e.currentTarget.value);
                    saveSettings();
                  }}
                />
                <span class="help">
                  Leave blank for the embedded server. Override only with a
                  remote SearXNG you control.
                </span>
              </div>

              <Show when={searxResult() !== null}>
                <div class="df-banner ok">
                  <CheckCircle2 size={14} />
                  <div class="df-banner-body">
                    <strong>Local SearXNG is running.</strong>
                    <Show when={searxResult()}>
                      <pre style={{
                        margin: "6px 0 0",
                        "max-height": "96px",
                        overflow: "auto",
                        "white-space": "pre-wrap",
                        "font-family": "var(--font-mono)",
                        "font-size": "10px",
                        opacity: 0.85,
                      }}>{searxResult()}</pre>
                    </Show>
                  </div>
                </div>
              </Show>

              <Show when={searxError()}>
                <div class="df-banner bad">
                  <X size={14} />
                  <div class="df-banner-body">
                    <strong>Verification failed.</strong> {searxError()}
                  </div>
                </div>
              </Show>
            </div>
          </section>

          {/* Logs (frontend buffer + backend runlog) */}
          <LogsPanel />
        </div>
      </div>
    </div>
  );
}
