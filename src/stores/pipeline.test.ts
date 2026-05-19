import { describe, expect, it } from "vitest";
import { pipelineStore } from "@/stores/pipeline";

describe("pipelineStore", () => {
  it("exposes all stages in display order", () => {
    expect(pipelineStore.ordered).toEqual([
      "llm_expand",
      "discovery",
      "rank",
      "semantic_rerank",
      "llm_filter",
      "citation_enrich",
      "download",
      "extract",
    ]);
  });

  it("seeds every stage as idle", () => {
    pipelineStore.reset();
    for (const stage of pipelineStore.ordered) {
      expect(pipelineStore.stages[stage].state).toBe("idle");
    }
  });

  it("reset re-initializes every stage to idle", () => {
    pipelineStore.reset();
    const allIdle = pipelineStore.ordered.every((s) => pipelineStore.stages[s].state === "idle");
    expect(allIdle).toBe(true);
  });
});
