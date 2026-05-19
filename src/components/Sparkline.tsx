/// Tiny SVG sparkline. Used by the run card's speed strip to visualize
/// the last ~32 throughput samples. Pure presentational — values are
/// expected to be smoothed upstream (run.ts ticker handles that).
export default function Sparkline(props: {
  values: number[];
  color?: string;
}) {
  const path = () => {
    const v = props.values;
    if (v.length < 2) return "";
    const max = Math.max(0.001, ...v);
    const w = 100;
    const h = 28;
    const step = w / Math.max(1, v.length - 1);
    return v
      .map((y, i) => {
        const cmd = i === 0 ? "M" : "L";
        return `${cmd} ${(i * step).toFixed(1)} ${(h - (y / max) * h).toFixed(1)}`;
      })
      .join(" ");
  };

  return (
    <svg
      class="df-spark-svg"
      viewBox="0 0 100 28"
      preserveAspectRatio="none"
      aria-hidden="true"
    >
      <path
        d={path()}
        fill="none"
        stroke={props.color ?? "var(--accent)"}
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  );
}
