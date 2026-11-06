import { For, Show, createMemo } from "solid-js";
import { formatBytes, sourceColor, sourceLabel } from "@/lib/utils";
import { runStore } from "@/stores/run";

/** Cap on tiles drawn (one per planned document). */
const MAX_TILES = 300;

/**
 * Tile map: one square per planned download, in the order they finished.
 * Saved = the source's color, failed = red with a slash, in progress =
 * pulsing accent, still queued = an empty outline.
 */
export function TileMap() {
  const s = runStore.state;
  const tiles = createMemo(() => {
    const out: { kind: "ok" | "fail" | "live" | "wait"; color?: string; title: string }[] = [];
    // Only saved documents get tiles: an unavailable result is replaced by
    // the next-best match, so it never leaves a hole in the map.
    for (const o of s.outcomes) {
      if (!o.ok) continue;
      out.push({
        kind: o.ok ? "ok" : "fail",
        color: o.ok ? sourceColor(o.source) : undefined,
        title: `${o.ok ? "Saved" : "Couldn’t download"} · ${sourceLabel(o.source)} · ${o.title}`,
      });
      if (out.length >= MAX_TILES) return out;
    }
    for (const f of s.inFlight) {
      out.push({ kind: "live", title: `Downloading · ${f.title}` });
      if (out.length >= MAX_TILES) return out;
    }
    const planned = Math.min(MAX_TILES, Math.max(s.planned, out.length));
    while (out.length < planned) out.push({ kind: "wait", title: "Queued" });
    return out;
  });
  return (
    <Show when={tiles().length > 0}>
      <div
        class="df-tiles"
        role="img"
        aria-label={`${s.done} saved, ${s.inFlight.length} downloading, of ${Math.max(s.planned, s.done)} planned`}
      >
        <For each={tiles()}>
          {(t) => (
            <i
              class="df-tile"
              data-kind={t.kind}
              style={t.color ? { background: t.color } : undefined}
              title={t.title}
            />
          )}
        </For>
      </div>
    </Show>
  );
}

/** Stacked bar + legend: saved documents by source. */
export function SourceMix() {
  const s = runStore.state;
  const mix = createMemo(() => {
    const m = new Map<string, number>();
    for (const o of s.outcomes) if (o.ok) m.set(o.source, (m.get(o.source) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });
  const total = () => mix().reduce((a, [, n]) => a + n, 0);
  return (
    <Show when={total() > 0}>
      <div class="df-mix">
        <div class="df-mix-bar" aria-hidden="true">
          <For each={mix()}>
            {([src, n]) => (
              <span
                style={{ background: sourceColor(src), width: `${(n / total()) * 100}%` }}
                title={`${sourceLabel(src)}: ${n}`}
              />
            )}
          </For>
        </div>
        <ul class="df-mix-legend" aria-label="Saved documents by source">
          <For each={mix()}>
            {([src, n]) => (
              <li>
                <i class="df-dot" style={{ background: sourceColor(src) }} />
                {sourceLabel(src)} <b class="num">{n}</b>
              </li>
            )}
          </For>
        </ul>
      </div>
    </Show>
  );
}

/** Area chart of download speed over the last two minutes. */
export function SpeedChart() {
  const s = runStore.state;
  const W = 320;
  const H = 64;
  const geom = createMemo(() => {
    const pts = s.speedSeries;
    if (pts.length < 2) return null;
    const max = Math.max(...pts, 1);
    const step = W / (pts.length - 1);
    const xy = pts.map((v, i) => [i * step, H - 4 - (v / max) * (H - 10)] as const);
    const line = xy.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join("");
    const area = `${line}L${W},${H}L0,${H}Z`;
    const avg = pts.reduce((a, b) => a + b, 0) / pts.length;
    return { line, area, max, now: pts[pts.length - 1], avg };
  });
  return (
    <Show when={geom()}>
      {(g) => (
        <div class="df-speed">
          <div class="df-speed-figs">
            <div>
              <span class="df-label">Now</span>
              <b class="num">{runStore.live ? `${formatBytes(g().now)}/s` : "—"}</b>
            </div>
            <div>
              <span class="df-label">Average</span>
              <b class="num">{formatBytes(g().avg)}/s</b>
            </div>
            <div>
              <span class="df-label">Peak</span>
              <b class="num">{formatBytes(g().max)}/s</b>
            </div>
          </div>
          <svg
            viewBox={`0 0 ${W} ${H}`}
            preserveAspectRatio="none"
            class="df-speed-svg"
            role="img"
            aria-label={`Download speed, currently ${formatBytes(g().now)} per second`}
          >
            <defs>
              <linearGradient id="df-speed-fill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.35" />
                <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
              </linearGradient>
            </defs>
            <path d={g().area} fill="url(#df-speed-fill)" />
            <path
              d={g().line}
              fill="none"
              stroke="var(--accent)"
              stroke-width="1.75"
              vector-effect="non-scaling-stroke"
            />
          </svg>
        </div>
      )}
    </Show>
  );
}

const FTYPE_LABEL: Record<string, string> = { pdf: "PDF", epub: "EPUB", html: "HTML", txt: "TXT" };

/** Colored file-type badge for a saved document. */
export function FileTypeBadge(props: { type: string | null }) {
  return (
    <Show when={props.type}>
      {(t) => (
        <span class="df-ftype" data-type={t()}>
          {FTYPE_LABEL[t()] ?? t().toUpperCase()}
        </span>
      )}
    </Show>
  );
}
