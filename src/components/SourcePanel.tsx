import { For, Show, Switch, Match, createMemo } from "solid-js";
import { Check } from "lucide-solid";
import { SOURCE_LABELS, sourceColor, sourceDesc } from "@/lib/utils";
import type { SourceStat } from "@/stores/run";

function SourceRow(props: {
  id: string;
  on: boolean;
  running: boolean;
  status?: SourceStat;
  onToggle: () => void;
}) {
  const color = () => sourceColor(props.id);
  const showLive = () => props.running && props.status != null;
  // While a run is active the source set is fixed (the backend already has its
  // snapshot), so toggling here would only desync the live lanes/graph from what
  // the search is actually querying. Lock the row during a run.
  const locked = () => props.running;
  return (
    <div
      class={`df-srcrow ${props.on ? "on" : "off"}`}
      classList={{ locked: locked() }}
      style={{ "--src-color": color(), ...(locked() ? { cursor: "default" } : {}) }}
      role="button"
      tabindex={locked() ? -1 : 0}
      aria-pressed={props.on}
      aria-disabled={locked()}
      aria-label={`${SOURCE_LABELS[props.id] ?? props.id} — ${props.on ? "enabled" : "disabled"}`}
      onClick={() => {
        if (locked()) return;
        props.onToggle();
      }}
      onKeyDown={(e) => {
        if (locked()) return;
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          props.onToggle();
        }
      }}
    >
      <div class="df-srcrow-check">
        <Show when={props.on}>
          <Check size={11} />
        </Show>
      </div>
      <div class="df-srcrow-main">
        <div class="df-srcrow-name">
          <span>{SOURCE_LABELS[props.id] ?? props.id}</span>
          <span class="src-id">{props.id}</span>
        </div>
        <div class="df-srcrow-desc">{sourceDesc(props.id)}</div>
      </div>
      <div class="df-srcrow-meta">
        <Show
          when={showLive()}
          fallback={
            <div class="df-srcrow-stat">
              <span class="df-srcrow-stat-num">{props.status?.hits ?? "—"}</span>
              <span class="df-srcrow-stat-label">last run</span>
            </div>
          }
        >
          <div
            class={`df-srcrow-status ${
              props.status!.status === "querying"
                ? "live"
                : props.status!.status === "error"
                  ? "err"
                  : "ok"
            }`}
          >
            <span class="df-srcrow-status-dot" />
            <Switch>
              <Match when={props.status!.status === "querying"}>
                {props.status!.hits > 0 ? `${props.status!.hits} found…` : "scanning…"}
              </Match>
              <Match when={props.status!.status === "done"}>+{props.status!.hits} hits</Match>
              <Match when={props.status!.status === "error"}>error</Match>
            </Switch>
          </div>
        </Show>
      </div>
    </div>
  );
}

/** The rich 2-column Sources panel on Discover. */
export default function SourcePanel(props: {
  sources: string[];
  enabled: string[];
  running: boolean;
  stats: Record<string, SourceStat>;
  onToggle: (id: string) => void;
  onEnableAll: () => void;
  onDisableAll: () => void;
  onInvert: () => void;
}) {
  const enabledCount = () => props.sources.filter((s) => props.enabled.includes(s)).length;
  // Web engines toggled under "Advanced" aren't among props.sources (the primary
  // set shown here), so an enabled advanced engine would otherwise read as a bare,
  // contradictory "0 of N enabled" even though the search runs. Surface the extra.
  const extraEnabled = () => props.enabled.filter((s) => !props.sources.includes(s)).length;
  const totalHits = createMemo(() => {
    let sum = 0;
    for (const s of props.sources) {
      sum += props.stats[s]?.hits ?? 0;
    }
    return sum;
  });
  return (
    <div class="df-srcpanel">
      <div class="df-srcpanel-head">
        <strong>{enabledCount()}</strong>
        <span style={{ color: "var(--ink-3)" }}>
          of {props.sources.length} sources enabled
          <Show when={extraEnabled() > 0}> · +{extraEnabled()} advanced web</Show>
        </span>
        <span style={{ flex: 1 }} />
        <Show when={totalHits() > 0}>
          <span style={{ color: "var(--ink-3)" }}>
            {props.running ? "this run" : "last run"} · <strong>{totalHits()}</strong> hits across
            all sources
          </span>
        </Show>
      </div>

      <div class="df-srcpanel-grid">
        <For each={props.sources}>
          {(id) => (
            <SourceRow
              id={id}
              on={props.enabled.includes(id)}
              running={props.running}
              status={props.stats[id]}
              onToggle={() => props.onToggle(id)}
            />
          )}
        </For>
      </div>

      <div class="df-srcactions">
        <button onClick={() => props.onEnableAll()} disabled={props.running}>
          Enable all
        </button>
        <span class="sep" />
        <button onClick={() => props.onDisableAll()} disabled={props.running}>
          Disable all
        </button>
        <span class="sep" />
        <button onClick={() => props.onInvert()} disabled={props.running}>
          Invert
        </button>
        <span style={{ flex: 1 }} />
        <span style={{ color: "var(--ink-3)" }}>Per-source cap &amp; weights in Settings</span>
      </div>
    </div>
  );
}
