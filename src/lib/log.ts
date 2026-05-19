/**
 * Structured frontend logger.
 *
 * Why this exists:
 *   - Before this module, the frontend had zero `console.*` calls. Silent
 *     `catch {}` blocks meant errors disappeared into devtools and the
 *     user had no visibility into what went wrong.
 *   - The Rust backend's `tracing!` output lands in stderr, which is also
 *     invisible to a packaged app's user.
 *
 * What it does:
 *   - Append-only ring buffer (last 500 entries) in memory, accessible via
 *     `log.tail(n?)`. Survives view switches but cleared on app relaunch.
 *   - Color-coded `console.{debug,info,warn,error}` mirror so devtools is
 *     still useful when it's open.
 *   - `subscribe(fn)` for components that want to live-render the buffer
 *     (used by the LogsPanel in Settings).
 *   - `installGlobalHandlers()` wires `window.onerror` and
 *     `unhandledrejection` so uncaught crashes land in the buffer
 *     instead of being lost.
 */

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogArea =
  | "boot"
  | "run"
  | "settings"
  | "library"
  | "find"
  | "tauri"
  | "ui"
  | "backend";

export interface LogEntry {
  ts: number;
  level: LogLevel;
  area: LogArea;
  msg: string;
  data?: unknown;
}

const MAX_ENTRIES = 500;
const buffer: LogEntry[] = [];
const listeners = new Set<() => void>();

function notify() {
  for (const fn of listeners) fn();
}

function append(entry: LogEntry) {
  buffer.push(entry);
  if (buffer.length > MAX_ENTRIES) {
    buffer.splice(0, buffer.length - MAX_ENTRIES);
  }
  notify();
}

// Devtools color codes for the prefix; the message itself stays default.
const COLOR: Record<LogLevel, string> = {
  debug: "color:#6c7384",
  info:  "color:#2549c9",
  warn:  "color:#b4861e;font-weight:bold",
  error: "color:#a13a2a;font-weight:bold",
};

function emit(level: LogLevel, area: LogArea, msg: string, data?: unknown) {
  const entry: LogEntry = { ts: Date.now(), level, area, msg, data };
  append(entry);

  const tag = `[df:${area}]`;
  const args: unknown[] = [`%c${tag}%c ${msg}`, COLOR[level], "color:inherit"];
  if (data !== undefined) args.push(data);

  // console.debug doesn't exist on all hosts; fall back to log.
  const fn =
    level === "debug" ? (console.debug ?? console.log)
    : level === "info" ? console.info
    : level === "warn" ? console.warn
    : console.error;
  fn(...(args as []));
}

export const log = {
  debug: (area: LogArea, msg: string, data?: unknown) => emit("debug", area, msg, data),
  info:  (area: LogArea, msg: string, data?: unknown) => emit("info",  area, msg, data),
  warn:  (area: LogArea, msg: string, data?: unknown) => emit("warn",  area, msg, data),
  error: (area: LogArea, msg: string, data?: unknown) => emit("error", area, msg, data),

  /// Snapshot of the last N entries (most recent at end). Default: full
  /// buffer.
  tail(n?: number): LogEntry[] {
    if (n == null || n >= buffer.length) return buffer.slice();
    return buffer.slice(-n);
  },

  /// Drop everything. Used by the Logs panel's Clear button.
  clear() {
    buffer.length = 0;
    notify();
  },

  /// Subscribe to mutations of the buffer. Returns an unsubscribe fn.
  /// Components can call this in createEffect + onCleanup.
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  },
};

export const subscribe = log.subscribe;

/// Install global error catchers. Call once from main.tsx, before render.
/// These convert otherwise-invisible crashes into log entries the user can
/// inspect via Settings → Logs.
export function installGlobalHandlers() {
  if (typeof window === "undefined") return;
  window.addEventListener("error", (e) => {
    log.error("ui", `uncaught error: ${e.message}`, {
      filename: e.filename,
      lineno: e.lineno,
      colno: e.colno,
    });
  });
  window.addEventListener("unhandledrejection", (e) => {
    const reason = e.reason instanceof Error
      ? `${e.reason.name}: ${e.reason.message}`
      : String(e.reason);
    log.error("ui", `unhandled promise rejection: ${reason}`, e.reason);
  });
}

/// Pretty-format an entry for the Logs panel and clipboard copy.
export function formatEntry(e: LogEntry): string {
  const t = new Date(e.ts).toISOString().slice(11, 23); // HH:MM:SS.mmm
  const lvl = e.level.toUpperCase().padEnd(5);
  const data = e.data !== undefined ? ` ${safeJSON(e.data)}` : "";
  return `${t} ${lvl} [${e.area}] ${e.msg}${data}`;
}

function safeJSON(v: unknown): string {
  try {
    if (v instanceof Error) return `${v.name}: ${v.message}`;
    if (typeof v === "string") return v;
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}
