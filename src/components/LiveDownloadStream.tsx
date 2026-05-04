import { Check, Download, X } from "lucide-react";
import { useMemo } from "react";
import { useShallow } from "zustand/shallow";
import { Badge } from "./ui/badge";
import { formatBytes, SOURCE_LABELS } from "@/lib/utils";
import { useRunStore } from "@/stores/runStore";

export function LiveDownloadStream() {
  const inFlightMap = useRunStore(useShallow((s) => s.inFlight));
  const completedRaw = useRunStore(useShallow((s) => s.completed));
  const running = useRunStore((s) => s.running);

  const inFlight = useMemo(() => Object.values(inFlightMap), [inFlightMap]);
  const completed = useMemo(() => completedRaw.slice(-50).reverse(), [completedRaw]);

  if (!running && inFlight.length === 0 && completed.length === 0) return null;

  return (
    <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)]">
      {inFlight.length > 0 && (
        <div className="border-b border-[var(--color-border)]">
          <div className="flex items-center gap-2 px-4 py-2 text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
            <Download className="h-3 w-3" />
            <span>Downloading ({inFlight.length})</span>
          </div>
          <ul className="divide-y divide-[var(--color-border)]">
            {inFlight.map((d) => {
              const pct =
                d.total > 0 ? Math.min(100, (d.downloaded / d.total) * 100) : 0;
              return (
                <li
                  key={d.url}
                  className="flex items-center gap-3 px-4 py-2 text-sm"
                >
                  <Badge variant="source" source={d.source} className="shrink-0">
                    {SOURCE_LABELS[d.source] ?? d.source}
                  </Badge>
                  <span
                    className="flex-1 truncate text-[var(--color-foreground)]"
                    title={d.title}
                  >
                    {d.title || "Untitled"}
                  </span>
                  <span className="w-32 shrink-0 font-mono text-xs tabular-nums text-[var(--color-muted-foreground)]">
                    {formatBytes(d.downloaded)}
                    {d.total > 0 && ` / ${formatBytes(d.total)}`}
                  </span>
                  <div className="h-1 w-24 shrink-0 overflow-hidden rounded-full bg-[var(--color-muted)]">
                    <div
                      className="h-full rounded-full bg-[var(--color-primary)] transition-[width] duration-150"
                      style={{ width: d.total > 0 ? `${pct}%` : "30%" }}
                    />
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {completed.length > 0 && (
        <div>
          <div className="px-4 py-2 text-[11px] uppercase tracking-wider text-[var(--color-muted-foreground)]">
            Recent ({completed.length})
          </div>
          <ul className="max-h-80 divide-y divide-[var(--color-border)] overflow-auto">
            {completed.map((c) => (
              <li
                key={c.url}
                className="flex items-center gap-3 px-4 py-1.5 text-sm"
              >
                {c.status === "done" ? (
                  <Check className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
                ) : (
                  <X className="h-3.5 w-3.5 shrink-0 text-rose-600" />
                )}
                <Badge variant="source" source={c.source} className="shrink-0">
                  {SOURCE_LABELS[c.source] ?? c.source}
                </Badge>
                <span
                  className={
                    "flex-1 truncate " +
                    (c.status === "failed"
                      ? "text-[var(--color-muted-foreground)]"
                      : "text-[var(--color-foreground)]")
                  }
                  title={c.title}
                >
                  {c.title || "Untitled"}
                </span>
                {c.status === "failed" && c.error && (
                  <span
                    className="max-w-[40%] shrink-0 truncate text-xs text-rose-600/80"
                    title={c.error}
                  >
                    {c.error}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
