import { createStore, produce } from "solid-js/store";
import { api, type ModelInfo } from "@/lib/tauri";
import type { DfEvent, ModelProgressPayload, ModelStatusPayload } from "@/lib/events";
import { log } from "@/lib/log";

/**
 * AI model lifecycle store.
 *
 * The Rust backend owns the actual model files (under ~/.cache/document-finder/models)
 * and the streaming download / SHA256 verify pipeline. This store is the
 * thin frontend mirror: it queries `list_models` on demand, dispatches
 * download/cancel/delete commands, and patches local rows reactively
 * when `df:model_progress` and `df:model_status` events arrive.
 *
 * Two flavors of state per row matter:
 *   - `models[*].status`  → disk-availability (downloaded? verified?)
 *   - `activity[id]`      → what the running pipeline is currently doing
 *                           with the model (embedding / llm_warming / etc).
 *     Distinct because a model can be "ready" on disk and simultaneously
 *     "embedding 23/100" in the active run.
 */

interface ModelsState {
  models: ModelInfo[];
  loading: boolean;
  error: string | null;
  /// True once the embedding model has loaded into process memory.
  /// fastembed manages its own download separately from our registry, so
  /// this is decoupled from `models[*].status`.
  embeddingLoaded: boolean;
  /// Bytes-per-second observed in the last model_progress event, keyed
  /// by model_id. Used by the UI for an instantaneous-rate label and ETA.
  bytesPerSec: Record<string, number>;
  /// Non-disk runtime activity (embedding, llm_warming, etc), auto-cleared
  /// 5s after the last event for that model.
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

async function refresh() {
  setState("loading", true);
  setState("error", null);
  try {
    const list = await api.listModels();
    setState("models", list);
    log.info("settings", `loaded ${list.length} AI models`);
    api
      .isEmbeddingLoaded()
      .then((loaded) => setState("embeddingLoaded", loaded))
      .catch(() => {
        /* best-effort */
      });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setState("error", msg);
    log.error("settings", "list_models failed", e);
  } finally {
    setState("loading", false);
  }
}

function patchStatus(modelId: string, mut: (m: ModelInfo) => void) {
  setState(
    "models",
    produce((arr) => {
      const m = arr.find((x) => x.id === modelId);
      if (m) mut(m);
    }),
  );
}

/// Apply a Tauri model_progress / model_status event. Called by main.tsx's
/// listener bridge so the panel updates in real time.
export function applyModelEvent(ev: DfEvent) {
  if (ev.type === "model_progress") {
    const p = ev.payload as ModelProgressPayload;
    setState("bytesPerSec", p.model_id, p.bytes_per_sec);
    patchStatus(p.model_id, (m) => {
      m.status = { kind: "downloading", downloaded: p.downloaded, total: p.total };
    });
    return;
  }
  if (ev.type === "model_status") {
    const p = ev.payload as ModelStatusPayload;
    // Runtime activity (embedding / llm_*) is shown separately from
    // disk status. The first "embedding" event implies fastembed
    // finished loading into memory.
    if (
      p.status === "embedding" ||
      p.status === "llm_warming" ||
      p.status === "llm_expanding" ||
      p.status === "llm_filtering"
    ) {
      if (p.status === "embedding") setState("embeddingLoaded", true);
      setState("activity", p.model_id, { status: p.status, detail: p.detail });
      // Auto-clear if no further event for this model arrives in 5s.
      const snapshot = { status: p.status, detail: p.detail };
      setTimeout(() => {
        const cur = state.activity[p.model_id];
        if (cur?.status === snapshot.status && cur?.detail === snapshot.detail) {
          setState(
            "activity",
            produce((a) => {
              delete a[p.model_id];
            }),
          );
        }
      }, 5000);
      return;
    }
    patchStatus(p.model_id, (m) => {
      switch (p.status) {
        case "ready":
          m.status = { kind: "ready" };
          break;
        case "verifying":
          m.status = { kind: "verifying" };
          break;
        case "downloading":
          m.status = { kind: "downloading", downloaded: 0, total: m.approx_bytes };
          break;
        case "failed":
          m.status = { kind: "failed", msg: p.detail ?? "unknown" };
          break;
        case "cancelled":
          m.status = { kind: "cancelled" };
          break;
      }
    });
  }
}

async function download(modelId: string) {
  patchStatus(modelId, (m) => {
    m.status = { kind: "downloading", downloaded: 0, total: m.approx_bytes };
  });
  log.info("settings", `starting model download: ${modelId}`);
  try {
    await api.downloadModel(modelId);
  } catch (e) {
    patchStatus(modelId, (m) => {
      m.status = { kind: "failed", msg: String(e) };
    });
    log.error("settings", `model download ${modelId} failed`, e);
  }
}

async function cancel(modelId: string) {
  try {
    await api.cancelModelDownload(modelId);
    log.info("settings", `cancelled model download: ${modelId}`);
  } catch (e) {
    log.error("settings", `cancel model ${modelId} failed`, e);
  }
}

async function remove(modelId: string) {
  try {
    await api.deleteModel(modelId);
    patchStatus(modelId, (m) => {
      m.status = { kind: "not_downloaded" };
      m.on_disk_bytes = 0;
    });
    log.info("settings", `deleted model: ${modelId}`);
  } catch (e) {
    log.error("settings", `delete model ${modelId} failed`, e);
  }
}

export const modelsStore = {
  get state() {
    return state;
  },
  refresh,
  download,
  cancel,
  remove,
  /// True once the embedding model has loaded into process memory.
  get embeddingReady() {
    return state.embeddingLoaded;
  },
  /// Convenience: any LLM model currently in Ready status?
  get llmReady() {
    return state.models.some((m) => m.kind === "llm" && m.status.kind === "ready");
  },
  /// Total disk usage across all downloaded models. Used in the panel
  /// summary line.
  get totalDiskBytes() {
    return state.models.reduce((acc, m) => acc + m.on_disk_bytes, 0);
  },
};
