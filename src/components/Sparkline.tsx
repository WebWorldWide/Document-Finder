import { Show } from "solid-js";

/** A tiny filled-area sparkline (SVG) for the throughput strip. */
export default function Sparkline(props: { values: number[]; color?: string; max?: number }) {
  const color = () => props.color ?? "var(--accent)";
  const W = 100;
  const H = 28;
  const geom = () => {
    const values = props.values;
    if (!values || values.length < 2) return null;
    const m = props.max ?? Math.max(1, ...values);
    const stepX = W / Math.max(1, values.length - 1);
    const pts = values.map((v, i) => [i * stepX, H - (v / m) * (H - 4) - 2] as const);
    const d = pts
      .map((p, i) => `${i === 0 ? "M" : "L"}${p[0].toFixed(1)},${p[1].toFixed(1)}`)
      .join(" ");
    return { d, area: `${d} L${W},${H} L0,${H} Z` };
  };
  return (
    <Show when={geom()}>
      {(g) => (
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
          <path d={g().area} fill={color()} opacity="0.12" />
          <path
            d={g().d}
            fill="none"
            stroke={color()}
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      )}
    </Show>
  );
}
