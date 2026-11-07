import { For, Show, createResource, createSignal, onMount } from "solid-js";
import { Check, ClipboardCopy, FolderOpen, Plug } from "lucide-solid";
import { open } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";
import Logo from "@/components/Logo";
import Note from "@/components/Note";
import { api } from "@/lib/tauri";
import { formatBytes } from "@/lib/utils";
import { runStore } from "@/stores/run";
import { settings, syncLibraryRoot } from "@/stores/settings";
import { setThemePref, themePref, type ThemePref } from "@/stores/theme";
import { uiStore } from "@/stores/ui";

const isLinux =
  typeof navigator !== "undefined" && /Linux/.test(navigator.platform || navigator.userAgent || "");

async function copy(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    uiStore.showToast(`${what} copied`);
  } catch {
    uiStore.showToast("Couldn’t access the clipboard");
  }
}

function Setting(props: { title: string; desc?: string; children: unknown }) {
  return (
    <section class="df-setting">
      <div>
        <h3>{props.title}</h3>
        {props.desc ? <p>{props.desc}</p> : null}
      </div>
      <div class="df-setting-body">{props.children as never}</div>
    </section>
  );
}

function ConnectAi() {
  const [info, { refetch }] = createResource(() => api.mcpInfo().catch(() => null));
  const [busy, setBusy] = createSignal(false);
  const install = async () => {
    setBusy(true);
    try {
      const msg = await api.installMcpClaudeDesktop();
      uiStore.showToast(msg);
      void refetch();
    } catch (e) {
      uiStore.showToast(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Setting title="Connect your AI" desc="Let Claude and other MCP apps read your libraries.">
      <Show when={info()} fallback={<p class="df-muted">Loading…</p>}>
        {(i) => (
          <>
            <div class="df-row">
              <Show
                when={!i().claude_desktop_installed}
                fallback={
                  <span class="df-row" style={{ color: "var(--ok)" }}>
                    <Check size={15} aria-hidden="true" /> Connected to Claude Desktop
                  </span>
                }
              >
                <button
                  class="df-btn df-btn-primary"
                  disabled={busy()}
                  onClick={() => void install()}
                >
                  <Plug size={15} aria-hidden="true" /> Add to Claude Desktop
                </button>
              </Show>
            </div>
            <div>
              <div class="df-label" style={{ "margin-bottom": "6px" }}>
                Claude Code
              </div>
              <div class="df-row" style={{ "flex-wrap": "nowrap" }}>
                <div class="df-code" style={{ flex: 1 }}>
                  {i().claude_code_command}
                </div>
                <button
                  class="df-btn df-btn-icon"
                  aria-label="Copy Claude Code command"
                  onClick={() => void copy(i().claude_code_command, "Command")}
                >
                  <ClipboardCopy size={15} />
                </button>
              </div>
            </div>
            <details class="df-details">
              <summary>Other MCP apps (JSON config)</summary>
              <div class="df-details-body">
                <div class="df-code">{i().config_json}</div>
                <div class="df-row" style={{ "margin-top": "8px" }}>
                  <button class="df-btn" onClick={() => void copy(i().config_json, "Config")}>
                    <ClipboardCopy size={14} aria-hidden="true" /> Copy JSON
                  </button>
                </div>
              </div>
            </details>
          </>
        )}
      </Show>
    </Setting>
  );
}

export default function SettingsView() {
  const [rootError, setRootError] = createSignal<string | null>(null);
  const [version] = createResource(() => getVersion().catch(() => ""));
  const [log] = createResource(() => api.runLogInfo().catch(() => null));
  const [safeRender, setSafeRender] = createSignal(false);
  const [purgeLibrary, setPurgeLibrary] = createSignal(false);
  const [purgeMsg, setPurgeMsg] = createSignal<string | null>(null);
  const [purgeOk, setPurgeOk] = createSignal(false);

  onMount(async () => {
    if (isLinux) setSafeRender(await api.getSafeRenderMode().catch(() => false));
  });

  const browse = async () => {
    try {
      const picked = await open({
        directory: true,
        defaultPath: settings.libraryRoot || undefined,
      });
      if (typeof picked === "string") {
        await syncLibraryRoot(picked);
        setRootError(null);
        uiStore.showToast("Library folder changed");
      }
    } catch (e) {
      setRootError(String(e));
    }
  };

  const purge = async () => {
    const all = purgeLibrary();
    const ok = await uiStore.confirm({
      title: all ? "Erase everything?" : "Erase app data?",
      body: all
        ? "Settings, caches, logs and every library. This can’t be undone."
        : "Settings, caches and logs. Libraries are kept.",
      confirmLabel: all ? "Erase everything" : "Erase app data",
      danger: true,
    });
    if (!ok) return;
    try {
      const r = await api.purgeAllData(all);
      try {
        localStorage.clear();
      } catch {
        /* ignore */
      }
      if (all) {
        uiStore.setKnownLibraries([]);
        runStore.reset();
      }
      const failed = r.failed.length;
      setPurgeOk(failed === 0);
      setPurgeMsg(
        failed === 0
          ? `Erased ${r.removed.length} location${r.removed.length === 1 ? "" : "s"}. Restart to finish.`
          : `Erased ${r.removed.length}, but ${failed} couldn’t be removed — quit the app and try again.`,
      );
    } catch (e) {
      setPurgeMsg(`Couldn’t erase data: ${String(e)}`);
    }
  };

  return (
    <div class="df-page">
      <header class="df-page-head">
        <div>
          <h1 class="df-title" style={{ "font-size": "44px" }}>
            Settings
          </h1>
        </div>
      </header>

      <Setting title="Appearance">
        <div class="df-seg" role="group" aria-label="Appearance">
          <For
            each={
              [
                ["system", "System"],
                ["light", "Light"],
                ["dark", "Dark"],
              ] as [ThemePref, string][]
            }
          >
            {([id, label]) => (
              <button aria-pressed={themePref() === id} onClick={() => setThemePref(id)}>
                {label}
              </button>
            )}
          </For>
        </div>
      </Setting>

      <Setting title="Library folder" desc="Where searches are saved.">
        <div class="df-code">{settings.libraryRoot || "—"}</div>
        <div class="df-row">
          <button class="df-btn" onClick={() => void browse()} disabled={runStore.live}>
            Change…
          </button>
          <button
            class="df-btn df-btn-quiet"
            onClick={() =>
              settings.libraryRoot &&
              void api
                .revealInFinder(settings.libraryRoot)
                .catch((e) => uiStore.showToast(String(e)))
            }
          >
            <FolderOpen size={14} aria-hidden="true" /> Show
          </button>
        </div>
        <Show when={rootError()}>
          <Note tone="bad">{rootError()}</Note>
        </Show>
      </Setting>

      <ConnectAi />

      <Setting title="Diagnostics" desc="For bug reports.">
        <Show when={log()} fallback={<p class="df-muted">No log yet.</p>}>
          {(l) => (
            <>
              <div class="df-code">{l().path}</div>
              <div class="df-row">
                <span class="df-muted num">
                  {l().exists ? formatBytes(l().size_bytes) : "empty"}
                </span>
                <button
                  class="df-btn df-btn-quiet"
                  onClick={() =>
                    void api.revealInFinder(l().path).catch(() => uiStore.showToast("No log yet."))
                  }
                >
                  <FolderOpen size={14} aria-hidden="true" /> Show in folder
                </button>
              </div>
            </>
          )}
        </Show>
        <Show when={isLinux}>
          <label class="df-check">
            <input
              type="checkbox"
              checked={safeRender()}
              onChange={async (e) => {
                const on = e.currentTarget.checked;
                setSafeRender(on);
                await api.setSafeRenderMode(on).catch(() => setSafeRender(!on));
              }}
            />
            <span>
              Safe rendering mode
              <span class="df-muted" style={{ display: "block", "font-size": "12px" }}>
                For blank windows. Needs a restart.
              </span>
            </span>
          </label>
        </Show>
      </Setting>

      <Setting title="Erase data" desc="For a clean uninstall.">
        <label class="df-check">
          <input
            type="checkbox"
            checked={purgeLibrary()}
            onChange={(e) => setPurgeLibrary(e.currentTarget.checked)}
          />
          <span>Also delete my downloaded libraries</span>
        </label>
        <div class="df-row">
          <button
            class="df-btn df-btn-danger"
            onClick={() => void purge()}
            disabled={runStore.live}
          >
            {purgeLibrary() ? "Erase everything" : "Erase app data"}
          </button>
          <Show when={purgeOk()}>
            <button class="df-btn" onClick={() => void api.restartApp()}>
              Restart now
            </button>
          </Show>
        </div>
        <Show when={purgeMsg()}>
          <p class="df-muted" role="status">
            {purgeMsg()}
          </p>
        </Show>
      </Setting>

      <section class="df-setting" style={{ "border-bottom": 0 }}>
        <div class="df-row" style={{ gap: "12px" }}>
          <Logo size={36} />
          <div>
            <div class="df-brand-name">Document Finder</div>
            <div class="df-muted num" style={{ "font-size": "12px", "margin-top": "4px" }}>
              Version {version() || "—"} · AGPL-3.0
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
