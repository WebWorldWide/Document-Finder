import { createStore, produce } from "solid-js/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type ModelInfo } from "@/lib/tauri";
import type { ModelProgressPayload, ModelStatusPayload } from "@/lib/events";

interface ModelsState {
  models: ModelInfo[];
  loading: boolean;
  error: string | null;
  /// True once the embedding model (managed by fastembed itself, not by our
  /// registry) has been initialized in-process. Polled lazily from the
  /// `is_embedding_loaded` Tauri command. Used by the AI Models UI to show
  /// "Auto-managed — ready" vs "Auto-managed — downloads on first search".
  embeddingLoaded: boolean;
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
  bytesPerSec: {},
  activity: {},
});

let unsubProgress: UnlistenFn | null = null;
let unsubStatus: UnlistenFn | null = null;

async function refresh() {
  setState("loading", true);
  setState("error", null);
  try {
    const list = await api.listModels();
    setState("models", list);
    // Embedding readiness is decoupled from the registry list — fastembed
    // owns its own model cache. Best-effort poll; ignore errors.
    api
      .isEmbeddingLoaded()
      .then((loaded) => setState("embeddingLoaded", loaded))
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
  unsubProgress = await listen<ModelProgressPayload>("df:model_progress", (ev) => {
    const { model_id, downloaded, total, bytes_per_sec } = ev.payload;
    setState("bytesPerSec", model_id, bytes_per_sec);
    patchStatus(model_id, (m) => {
      m.status = { kind: "downloading", downloaded, total };
    });
  });
  unsubStatus = await listen<ModelStatusPayload>("df:model_status", (ev) => {
    const { model_id, status, detail } = ev.payload;
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
    patchStatus(modelId, (m) => {
      m.status = { kind: "not_downloaded" };
      m.on_disk_bytes = 0;
    });
  } catch (e) {
    console.error("delete failed", e);
  }
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
  /// True once the embedding model (managed by fastembed) has loaded into
  /// process memory. The first semantic-rerank kicks the download/load.
  get embeddingReady() {
    return state.embeddingLoaded;
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
