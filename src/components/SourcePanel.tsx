import { For, Show, Switch, Match } from "solid-js";
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
  return (
    <div
      class={`df-srcrow ${props.on ? "on" : "off"}`}
      style={{ "--src-color": color() }}
      role="button"
      tabindex={0}
      aria-pressed={props.on}
      aria-label={`${SOURCE_LABELS[props.id] ?? props.id} — ${props.on ? "enabled" : "disabled"}`}
      onClick={() => props.onToggle()}
      onKeyDown={(e) => {
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
              <Match when={props.status!.status === "querying"}>scanning…</Match>
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
  const totalHits = () => props.sources.reduce((n, s) => n + (props.stats[s]?.hits ?? 0), 0);
  return (
    <div class="df-srcpanel">
      <div class="df-srcpanel-head">
        <strong>{enabledCount()}</strong>
        <span style={{ color: "var(--ink-3)" }}>of {props.sources.length} sources enabled</span>
        <span style={{ flex: 1 }} />
        <Show when={totalHits() > 0}>
          <span style={{ color: "var(--ink-3)" }}>
            last run · <strong>{totalHits()}</strong> hits across all sources
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
        <button onClick={() => props.onEnableAll()}>Enable all</button>
        <span class="sep" />
        <button onClick={() => props.onDisableAll()}>Disable all</button>
        <span class="sep" />
        <button onClick={() => props.onInvert()}>Invert</button>
        <span style={{ flex: 1 }} />
        <span style={{ color: "var(--ink-3)" }}>Per-source cap &amp; weights in Settings</span>
      </div>
    </div>
  );
}
