export default function ProgressBar(props: { pct: number; color?: string; shimmer?: boolean }) {
  const width = () => `${Math.max(0, Math.min(100, props.pct))}%`;
  return (
    <div class="df-progress-track">
      <div
        classList={{ "df-progress-fill": true, shimmer: !!props.shimmer }}
        style={{ width: width(), ...(props.color ? { background: props.color } : {}) }}
      />
    </div>
  );
}
