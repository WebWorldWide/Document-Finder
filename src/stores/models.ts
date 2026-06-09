import { createStore, produce } from "solid-js/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type ModelInfo } from "@/lib/tauri";
import type { ModelProgressPayload, ModelStatusPayload } from "@/lib/events";
import { reconcileLlmModel } from "@/stores/settings";

interface ModelsState {
  models: ModelInfo[];
  loading: boolean;
  error: string | null;
  /// True once the embedding model (managed by fastembed itself, not by our
  /// registry) has been initialized in-process. Polled lazily from the
  /// `is_embedding_loaded` Tauri command. Used by the AI Models UI to show
  /// "Auto-managed — ready" vs "Auto-managed — downloads on first search".
  embeddingLoaded: boolean;
  /// True if the embedding model is cached on disk (loads without a network
  /// fetch). Polled from the `embedding_downloaded` command. Distinct from
  /// `embeddingLoaded`, which is in-memory readiness for this session.
  embeddingDownloaded: boolean;
  /// Set when the out-of-process embedding worker fails/crashes (the
  /// `embedding_failed` status event). Sticky until a retry or reset, so the
  /// Settings row can show a clear "couldn't load" state instead of the app
  /// just having vanished.
  embeddingError: string | null;
  // Per-model bytes/sec for the UI ETA, keyed by model_id.
  bytesPerSec: Record<string, number>;
  // Last activity status for the model (e.g. "embedding 23/100", "llm_warming").
  // Distinct from `models[*].status` which is the disk-availability state;
  // this one tracks what the *running pipeline* is currently doing with
  // each model.
  activity: Record<string, { status: string; detail: string | null }>;
}

const [state, setState] = createStore<ModelsState>({
  models: [],
  loading: false,
  error: null,
  embeddingLoaded: false,
  embeddingDownloaded: false,
  embeddingError: null,
  bytesPerSec: {},
  activity: {},
});

let unsubProgress: UnlistenFn | null = null;
let unsubStatus: UnlistenFn | null = null;
// In-flight subscription promise. App.onMount + WelcomeDialog.onMount (+ Settings)
// all call ensureSubscribed() in the same tick; without this, each passes the
// synchronous `unsubProgress && unsubStatus` guard BEFORE the first `await listen`
// resolves, so every model event ends up with 2-3 live listeners (leaked) and
// fires multiple times. Sharing one promise collapses them to a single subscribe.
let subscribing: Promise<void> | null = null;

async function refresh() {
  setState("loading", true);
  setState("error", null);
  try {
    const list = await api.listModels();
    setState("models", list);
    // Drop a persisted LLM selection that's no longer in the catalog.
    reconcileLlmModel(list.map((m) => m.id));
    // Embedding readiness is decoupled from the registry list — fastembed
    // owns its own model cache. Best-effort poll; ignore errors.
    api
      .isEmbeddingLoaded()
      .then((loaded) => setState("embeddingLoaded", loaded))
      .catch(() => {});
    api
      .embeddingDownloaded()
      .then((dl) => setState("embeddingDownloaded", dl))
      .catch(() => {});
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.error("listModels failed", e);
    setState("error", msg);
  } finally {
    setState("loading", false);
  }
}

/// Patches a single model's status field (in `state.models`). Used by event
/// handlers to update the on-disk status in place without re-fetching.
function patchStatus(modelId: string, mut: (m: ModelInfo) => void) {
  setState(
    "models",
    produce((arr) => {
      const m = arr.find((x) => x.id === modelId);
      if (m) mut(m);
    }),
  );
}

async function ensureSubscribed() {
  if (unsubProgress && unsubStatus) return;
  if (!subscribing) {
    subscribing = subscribeNow().catch((e) => {
      subscribing = null; // allow a later retry if the listen() calls failed
      throw e;
    });
  }
  return subscribing;
}

async function subscribeNow() {
  unsubProgress = await listen<ModelProgressPayload>("df:model_progress", (ev) => {
    const { model_id, downloaded, total, bytes_per_sec } = ev.payload;
    setState("bytesPerSec", model_id, bytes_per_sec);
    patchStatus(model_id, (m) => {
      m.status = { kind: "downloading", downloaded, total };
    });
  });
  unsubStatus = await listen<ModelStatusPayload>("df:model_status", (ev) => {
    const { model_id, status, detail } = ev.payload;
    // The out-of-process embedding worker failed/crashed. Record a sticky error
    // so the Settings row shows "couldn't load (see logs)" with a retry, rather
    // than the app appearing to have done nothing (or, pre-fix, having crashed).
    if (status === "embedding_failed") {
      setState("embeddingError", detail ?? "embedding model unavailable");
      setState("embeddingLoaded", false);
      return;
    }
    // Track non-disk activity events (embedding/llm_warming/etc) separately.
    if (
      status === "embedding" ||
      status === "llm_warming" ||
      status === "llm_expanding" ||
      status === "llm_filtering"
    ) {
      // The first "embedding" event implies fastembed has finished loading.
      if (status === "embedding") {
        setState("embeddingLoaded", true);
        // Loaded into memory implies it's also cached on disk now.
        setState("embeddingDownloaded", true);
        // Clear any prior failure now that it's working.
        setState("embeddingError", null);
      }
      setState("activity", model_id, { status, detail });
      // Auto-clear activity after 5s of silence.
      setTimeout(() => {
        const cur = state.activity[model_id];
        if (cur?.status === status && cur?.detail === detail) {
          setState(
            "activity",
            produce((a) => {
              delete a[model_id];
            }),
          );
        }
      }, 5000);
      return;
    }

    patchStatus(model_id, (m) => {
      switch (status) {
        case "ready":
          m.status = { kind: "ready" };
          break;
        case "verifying":
          m.status = { kind: "verifying" };
          break;
        case "downloading":
          // bytes will arrive via the progress event; init w/ zeros.
          m.status = { kind: "downloading", downloaded: 0, total: m.approx_bytes };
          break;
        case "failed":
          m.status = { kind: "failed", msg: detail ?? "unknown" };
          break;
        case "cancelled":
          m.status = { kind: "cancelled" };
          break;
      }
    });
  });
}

async function download(modelId: string) {
  await ensureSubscribed();
  // Clear any speed left over from a prior (cancelled/failed) attempt so a fresh
  // download never briefly shows a stale ETA before the first progress event.
  setState("bytesPerSec", modelId, 0);
  patchStatus(modelId, (m) => {
    m.status = { kind: "downloading", downloaded: 0, total: m.approx_bytes };
  });
  try {
    await api.downloadModel(modelId);
  } catch (e) {
    patchStatus(modelId, (m) => {
      m.status = { kind: "failed", msg: String(e) };
    });
  }
}

async function cancel(modelId: string) {
  try {
    await api.cancelModelDownload(modelId);
  } catch (e) {
    console.error("cancel failed", e);
  }
}

async function remove(modelId: string) {
  try {
    await api.deleteModel(modelId);
    setState("bytesPerSec", modelId, 0);
    patchStatus(modelId, (m) => {
      m.status = { kind: "not_downloaded" };
      m.on_disk_bytes = 0;
    });
  } catch (e) {
    console.error("delete failed", e);
  }
}

/// Kick off a background download + load of the embedding model (used by the
/// Settings "Download now" button). The df:model_status "embedding" event flips
/// embeddingLoaded when ready. Returns the invoke promise so the caller can
/// show a spinner and surface errors.
function warmEmbedding() {
  // Clear any prior failure so the row reflects this fresh attempt.
  setState("embeddingError", null);
  return api.warmEmbedding();
}

export const modelsStore = {
  get state() {
    return state;
  },
  refresh,
  ensureSubscribed,
  download,
  cancel,
  remove,
  warmEmbedding,
  /// Selectable for "Balanced": ready if loaded in memory or already cached on
  /// disk (it loads lazily on first use, and also auto-downloads on first
  /// search). The Settings row lets the user warm it explicitly.
  get embeddingReady() {
    return state.embeddingLoaded || state.embeddingDownloaded;
  },
  /// True if the embedding model is cached on disk.
  get embeddingDownloaded() {
    return state.embeddingDownloaded;
  },
  /// Coarse state for the Settings row: failed > in-memory > on-disk > absent.
  get embeddingState(): "loaded" | "downloaded" | "absent" | "failed" {
    if (state.embeddingError) return "failed";
    if (state.embeddingLoaded) return "loaded";
    if (state.embeddingDownloaded) return "downloaded";
    return "absent";
  },
  /// The last embedding-worker failure message, if any (for the Settings row).
  get embeddingError() {
    return state.embeddingError;
  },
  /// Convenience: any LLM model in Ready status?
  get llmReady() {
    return state.models.some((m) => m.kind === "llm" && m.status.kind === "ready");
  },
  /// Total disk usage across all downloaded models.
  get totalDiskBytes() {
    return state.models.reduce((acc, m) => acc + m.on_disk_bytes, 0);
  },
};
