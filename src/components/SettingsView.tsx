import { FileText, FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { api, type LogInfo } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import { useSettings } from "@/stores/settingsStore";

export function SettingsView() {
  const settings = useSettings();
  const [logInfo, setLogInfo] = useState<LogInfo | null>(null);

  useEffect(() => {
    api.runLogInfo().then(setLogInfo).catch(() => setLogInfo(null));
  }, []);
  return (
    <div className="space-y-5">
      <header>
        <h1 className="text-2xl font-semibold">Settings</h1>
      </header>

      <section className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
        <h2 className="mb-3 text-sm font-medium">Discovery</h2>
        <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
          <div>
            <Label>Per source</Label>
            <Input
              type="number"
              min={1}
              value={settings.perSource}
              onChange={(e) => settings.set({ perSource: Math.max(1, Number(e.target.value)) })}
              className="mt-1"
            />
            <p className="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
              Max documents per source per sub-query.
            </p>
          </div>
          <div>
            <Label>Max total</Label>
            <Input
              type="number"
              min={1}
              value={settings.maxTotal}
              onChange={(e) => settings.set({ maxTotal: Math.max(1, Number(e.target.value)) })}
              className="mt-1"
            />
            <p className="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
              Hard cap across all sources.
            </p>
          </div>
          <div>
            <Label>Parallel downloads</Label>
            <Input
              type="number"
              min={1}
              max={32}
              value={settings.concurrency}
              onChange={(e) =>
                settings.set({ concurrency: Math.max(1, Math.min(32, Number(e.target.value))) })
              }
              className="mt-1"
            />
            <p className="mt-1 text-[11px] text-[var(--color-muted-foreground)]">
              Higher = faster but more risk of rate limits.
            </p>
          </div>
        </div>
      </section>

      <section className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
        <h2 className="mb-3 text-sm font-medium">Library folder</h2>
        <Input
          value={settings.libraryRoot}
          onChange={(e) => settings.set({ libraryRoot: e.target.value })}
          className="font-mono text-xs"
        />
      </section>

      <section className="rounded-xl border border-[var(--color-border)] bg-[var(--color-card)] p-5">
        <h2 className="mb-1 text-sm font-medium">Run log</h2>
        <p className="mb-3 text-[11px] text-[var(--color-muted-foreground)]">
          Every query, source error, and download outcome is appended here. Share this file
          when reporting issues — it's the easiest way to diagnose failed downloads.
        </p>
        {logInfo ? (
          <div className="space-y-2">
            <code className="block truncate rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] px-2.5 py-1.5 font-mono text-[11px]">
              {logInfo.path}
            </code>
            <div className="flex items-center gap-2 text-[11px] text-[var(--color-muted-foreground)]">
              <span>
                {logInfo.exists ? formatBytes(logInfo.size_bytes) : "not yet written"}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => api.revealInFinder(logInfo.path)}
                disabled={!logInfo.exists}
              >
                <FolderOpen className="h-3.5 w-3.5" />
                Show log in Finder
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={async () => {
                  const fresh = await api.runLogInfo();
                  setLogInfo(fresh);
                }}
              >
                <FileText className="h-3.5 w-3.5" />
                Refresh
              </Button>
            </div>
          </div>
        ) : (
          <div className="text-xs text-[var(--color-muted-foreground)]">Resolving log path…</div>
        )}
      </section>
    </div>
  );
}
