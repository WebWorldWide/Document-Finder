import { describe, expect, it } from "vitest";
import { qualityToFlags } from "@/stores/settings";

describe("qualityToFlags", () => {
  it("fast disables every AI flag", () => {
    expect(qualityToFlags("fast")).toEqual({
      use_semantic_rerank: false,
      use_llm_expansion: false,
      use_llm_filter: false,
    });
  });

  it("balanced enables semantic rerank and broad LLM expansion (no filter)", () => {
    expect(qualityToFlags("balanced")).toEqual({
      use_semantic_rerank: true,
      use_llm_expansion: true,
      use_llm_filter: false,
    });
  });

  it("thorough enables every AI flag", () => {
    expect(qualityToFlags("thorough")).toEqual({
      use_semantic_rerank: true,
      use_llm_expansion: true,
      use_llm_filter: true,
    });
  });
});
