import { beforeEach, describe, expect, it } from "vitest";
import { __beginForTest, progressFraction, runStore, stages } from "@/stores/run";
import type { DfEvent } from "@/lib/events";

const found = (source: string, total: number): DfEvent => ({
  type: "found",
  payload: { title: "t", source, url: `https://x/${total}`, total },
});
const doc = (i: number) => ({
  url: `https://d/${i}`,
  title: `Doc ${i}`,
  source: "arxiv",
  authors: [],
});
const done = (i: number, d: number, f: number, total: number): DfEvent => ({
  type: "download_done",
  payload: {
    ...doc(i),
    local_path: `${i}.pdf`,
    absolute_path: `/lib/${i}.pdf`,
    text_path: `_text/${i}.md`,
    bytes: 1000,
    cached: false,
    done: d,
    failed: f,
    total,
  },
});

describe("run store", () => {
  beforeEach(() => runStore.reset());

  it("ignores events when idle", () => {
    runStore.apply(found("arxiv", 1));
    expect(runStore.state.found).toBe(0);
  });

  it("counts per-source discoveries and normalizes meta_search engines", () => {
    __beginForTest("q");
    runStore.apply(found("meta_search/web", 1));
    runStore.apply(found("meta_search/brave", 2));
    runStore.apply(found("arxiv", 3));
    expect(runStore.state.found).toBe(3);
    expect(runStore.state.sources["meta_search"].found).toBe(2);
    expect(runStore.state.sources["arxiv"].found).toBe(1);
  });

  it("files saved documents newest-first with catalog numbers", () => {
    __beginForTest("q");
    runStore.apply({ type: "plan", payload: { planned: 4 } });
    runStore.apply({
      type: "download_started",
      payload: { url: "https://d/1", title: "Doc 1", source: "arxiv" },
    });
    expect(runStore.state.inFlight).toHaveLength(1);
    runStore.apply(done(1, 1, 0, 4));
    runStore.apply(done(2, 2, 0, 4));
    expect(runStore.state.inFlight).toHaveLength(0);
    expect(runStore.state.saved.map((s) => s.n)).toEqual([2, 1]);
    expect(progressFraction()).toBe(0.5);
    expect(stages().download).toBe("active");
    expect(stages().read).toBe("active");
  });

  it("records failures and finishes", () => {
    __beginForTest("q");
    runStore.apply({
      type: "download_failed",
      payload: { ...doc(3), error: "HTTP 403", done: 0, failed: 1, total: 1 },
    });
    expect(runStore.state.failures).toHaveLength(1);
    runStore.apply({
      type: "complete",
      payload: { done: 0, failed: 1, total: 1, folder: "/lib", manifest: "" },
    });
    expect(runStore.state.status).toBe("done");
    expect(runStore.state.folder).toBe("/lib");
    expect(stages()).toEqual({ search: "done", download: "done", read: "done" });
    expect(progressFraction()).toBe(1);
  });

  it("planned never shrinks below what's already been processed", () => {
    __beginForTest("q");
    runStore.apply({ type: "plan", payload: { planned: 1 } });
    runStore.apply(done(1, 1, 0, 1));
    runStore.apply(done(2, 2, 0, 1));
    expect(runStore.state.planned).toBeGreaterThanOrEqual(2);
  });
});
