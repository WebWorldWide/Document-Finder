import { For, Show } from "solid-js";
import { settings, setSettings, saveSettings, toggleSource } from "@/stores/settings";
import { runStore } from "@/stores/run";
import {
  ALL_SOURCES, SOURCE_LABELS, SOURCE_DESCRIPTIONS, type SourceId,
} from "@/lib/utils";

/// 2-column rich-row source grid.
///
/// Each row shows: a custom checkbox (colored per-source when on), the
/// source's display name + monospace id, a one-line description, and on the
/// right either:
///   - the live status pill during a run (`scanning…` / `+N hits` / `error`)
///   - the source's enabled/disabled state when idle
///
/// The 7-run histogram bars from the design prototype are deferred until
/// run-history persistence lands; for v3.1 we leave that slot blank when
/// not live.
export default function SourcePanel() {
  const enabledCount = () => settings.selectedSources.length;
  const total = () => ALL_SOURCES.length;
  const isRunning = () => runStore.state.running;
  const statusFor = (id: SourceId) => runStore.state.sourceStatus[id];

  const enableAll = () => {
    setSettings("selectedSources", [...ALL_SOURCES] as SourceId[]);
    saveSettings();
  };
  const disableAll = () => {
    setSettings("selectedSources", []);
    saveSettings();
  };
  const invert = () => {
    setSettings("selectedSources",
      ALL_SOURCES.filter((s) => !settings.selectedSources.includes(s)) as SourceId[],
    );
    saveSettings();
  };

  return (
    <div class="df-srcpanel">
      <div class="df-srcpanel-head">
        <strong>{enabledCount()} of {total()}</strong>
        <span>enabled · toggle which open-access platforms to fan the query out to</span>
      </div>

      <div class="df-srcpanel-grid">
        <For each={ALL_SOURCES}>{(id) => {
          const isOn = () => settings.selectedSources.includes(id);
          const live = () => statusFor(id);
          return (
            <button
              class="df-srcrow"
              classList={{ on: isOn(), off: !isOn() }}
              style={{ "--src-color": `var(--src-${id})` } as Record<string, string>}
              onClick={(e) => {
                e.preventDefault();
                toggleSource(id);
              }}
              aria-pressed={isOn()}
            >
              <span class="df-srcrow-check">
                <Show when={isOn()}>
                  <svg width="10" height="10" viewBox="0 0 12 12" fill="none">
                    <path d="M2 6l3 3 5-6" stroke="currentColor" stroke-width="1.8"
                      stroke-linecap="round" stroke-linejoin="round" />
                  </svg>
                </Show>
              </span>
              <div class="df-srcrow-main">
                <div class="df-srcrow-name">
                  <span>{SOURCE_LABELS[id]}</span>
                  <span class="src-id">{id}</span>
                </div>
                <div class="df-srcrow-desc">{SOURCE_DESCRIPTIONS[id] ?? ""}</div>
              </div>
              <div class="df-srcrow-meta">
                <Show when={isRunning() && live()}>
                  <span
                    class="df-srcrow-status"
                    classList={{
                      live: live()?.phase === "querying",
                      ok: live()?.phase === "done",
                      err: live()?.phase === "error",
                    }}
                  >
                    <span class="df-srcrow-status-dot" />
                    <Show when={live()?.phase === "querying"}>scanning…</Show>
                    <Show when={live()?.phase === "done"}>
                      +{live()?.doneCount ?? 0} hits
                    </Show>
                    <Show when={live()?.phase === "error"}>error</Show>
                  </span>
                </Show>
              </div>
            </button>
          );
        }}</For>
      </div>

      <div class="df-srcactions">
        <button onClick={enableAll}>Enable all</button>
        <span class="sep" />
        <button onClick={disableAll}>Disable all</button>
        <span class="sep" />
        <button onClick={invert}>Invert</button>
      </div>
    </div>
  );
}
