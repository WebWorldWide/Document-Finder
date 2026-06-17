import { beforeEach, describe, expect, it } from "vitest";
import { runStore } from "@/stores/run";
import type { DfEvent } from "@/lib/events";

// Helpers to build the events the reducer consumes, with minimal valid payloads.
const sourceStart = (source: string): DfEvent => ({
  type: "source_start",
  payload: { source, sub_query: "q" },
});
const found = (source: string, total: number): DfEvent => ({
  type: "found",
  payload: { title: "t", source, url: `https://x/${total}`, total },
});
const sourceDone = (source: string, count: number): DfEvent => ({
  type: "source_done",
  payload: { source, count },
});
const foundTotal = (count: number): DfEvent => ({ type: "found_total", payload: { count } });

describe("runStore live per-source discovery counts", () => {
  beforeEach(() => runStore.reset());

  it("accumulates per-source hits live from found events and normalizes the meta_search prefix", () => {
    runStore.apply(sourceStart("meta_search"));
    runStore.apply(sourceStart("arxiv"));

    // meta_search documents stream in under nested engine ids.
    runStore.apply(found("meta_search/web", 1));
    runStore.apply(found("meta_search/brave", 2));
    runStore.apply(found("meta_search/web", 3));
    // arxiv documents carry the bare source id.
    runStore.apply(found("arxiv", 4));
    runStore.apply(found("arxiv", 5));

    expect(runStore.state.found).toBe(5); // global cumulative dedup total
    expect(runStore.state.sourceStats["meta_search"].hits).toBe(3);
    expect(runStore.state.sourceStats["arxiv"].hits).toBe(2);
    // Live updates keep the source in the "querying" state with a count.
    expect(runStore.state.sourceStats["meta_search"].status).toBe("querying");
  });

  it("does not double-count hits when source_done arrives after live found events", () => {
    runStore.apply(sourceStart("meta_search"));
    runStore.apply(found("meta_search/web", 1));
    runStore.apply(found("meta_search/web", 2));
    runStore.apply(found("meta_search/web", 3));
    runStore.apply(sourceDone("meta_search", 3));

    // hits stay at the live count (3), NOT 3 + count(3) = 6.
    expect(runStore.state.sourceStats["meta_search"].hits).toBe(3);
    expect(runStore.state.sourceStats["meta_search"].status).toBe("done");
    expect(runStore.state.sourceStats["meta_search"].active).toBe(0);
  });

  it("stays 'querying' until the last sibling sub-query task finishes (active counter)", () => {
    runStore.apply(sourceStart("arxiv")); // task 1 (active 1)
    runStore.apply(sourceStart("arxiv")); // task 2 (active 2)
    runStore.apply(found("arxiv", 1));

    runStore.apply(sourceDone("arxiv", 1)); // task 1 done -> active 1
    expect(runStore.state.sourceStats["arxiv"].status).toBe("querying");

    runStore.apply(sourceDone("arxiv", 0)); // task 2 done -> active 0
    expect(runStore.state.sourceStats["arxiv"].status).toBe("done");
    expect(runStore.state.sourceStats["arxiv"].hits).toBe(1);
  });

  it("found_total sweeps any still-querying source to done", () => {
    runStore.apply(sourceStart("gutenberg"));
    runStore.apply(found("gutenberg", 1));
    expect(runStore.state.sourceStats["gutenberg"].status).toBe("querying");

    runStore.apply(foundTotal(1));
    expect(runStore.state.sourceStats["gutenberg"].status).toBe("done");
    expect(runStore.state.sourceStats["gutenberg"].active).toBe(0);
  });

  it("retains full doc metadata on a failed download so it can be retried", () => {
    runStore.apply({
      type: "download_failed",
      payload: {
        url: "https://example.org/paper",
        title: "A Paper",
        source: "arxiv",
        authors: ["Ada L.", "Alan T."],
        year: "2021",
        abstract: "an abstract",
        identifier: "10.1234/abc",
        error: "the server refused the file",
        done: 0,
        failed: 1,
        total: 1,
      },
    });
    const f = runStore.state.completed.find((c) => c.url === "https://example.org/paper");
    expect(f).toBeTruthy();
    expect(f!.status).toBe("failed");
    expect(f!.authors).toEqual(["Ada L.", "Alan T."]);
    expect(f!.year).toBe("2021");
    expect(f!.identifier).toBe("10.1234/abc");
  });

  it("keeps a source that only errored (no hits) flagged as error after its task finishes", () => {
    runStore.apply(sourceStart("zenodo"));
    runStore.apply({
      type: "source_error",
      payload: { source: "zenodo", error: "429 Too Many Requests", kind: "rate_limit" },
    });
    runStore.apply(sourceDone("zenodo", 0));
    expect(runStore.state.sourceStats["zenodo"].status).toBe("error");
    expect(runStore.state.sourceStats["zenodo"].hits).toBe(0);
  });
});
