import { createStore } from "solid-js/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PipelineStage, PipelineState, PipelineStagePayload } from "@/lib/events";

export interface StageEntry {
  state: PipelineState | "idle";
  count?: number;
  total?: number;
  message?: string;
}

const ALL_STAGES: PipelineStage[] = [
  "llm_expand",
  "discovery",
  "rank",
  "semantic_rerank",
  "llm_filter",
  "citation_enrich",
  "download",
  "extract",
];

function emptyState(): Record<PipelineStage, StageEntry> {
  const out = {} as Record<PipelineStage, StageEntry>;
  for (const s of ALL_STAGES) out[s] = { state: "idle" };
  return out;
}

const [state, setState] = createStore<{ stages: Record<PipelineStage, StageEntry> }>({
  stages: emptyState(),
});

let unsub: UnlistenFn | null = null;

async function ensureSubscribed() {
  if (unsub) return;
  unsub = await listen<PipelineStagePayload>("df:pipeline_stage", (ev) => {
    const { stage, state: s, count, total, message } = ev.payload;
    setState("stages", stage, {
      state: s,
      count,
      total,
      message,
    });
  });
}

function reset() {
  setState("stages", emptyState());
}

export const pipelineStore = {
  get state() {
    return state;
  },
  get stages() {
    return state.stages;
  },
  /// Stages in display order — left-to-right rail.
  get ordered(): readonly PipelineStage[] {
    return ALL_STAGES;
  },
  ensureSubscribed,
  reset,
};
