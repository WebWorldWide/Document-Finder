import { describe, expect, it } from "vitest";
import {
  ALL_SOURCES,
  DEFAULT_ENABLED_SOURCES,
  META_SEARCH_COVERED,
  SOURCE_LABELS,
  formatBytes,
  formatDuration,
  sourceColor,
} from "@/lib/utils";

describe("formatBytes", () => {
  it("returns em-dash for zero", () => {
    expect(formatBytes(0)).toBe("—");
  });

  it("formats bytes under 1 KB", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("scales to KB without decimals over 100", () => {
    expect(formatBytes(150 * 1024)).toBe("150 KB");
  });

  it("uses one decimal under 100 of any unit > B", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("scales up to GB", () => {
    expect(formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
  });
});

describe("formatDuration", () => {
  it("uses ms under 1s", () => {
    expect(formatDuration(450)).toBe("450ms");
  });

  it("uses seconds with one decimal under 60s", () => {
    expect(formatDuration(2_500)).toBe("2.5s");
  });

  it("uses m/s for >= 60s", () => {
    expect(formatDuration(125_000)).toBe("2m 5s");
  });
});

describe("source registry", () => {
  it("DEFAULT_ENABLED_SOURCES is a subset of ALL_SOURCES", () => {
    for (const id of DEFAULT_ENABLED_SOURCES) {
      expect(ALL_SOURCES).toContain(id);
    }
  });

  it("every source id has a human label", () => {
    for (const id of ALL_SOURCES) {
      expect(SOURCE_LABELS[id]).toBeDefined();
      expect(SOURCE_LABELS[id]).not.toBe("");
    }
  });

  it("META_SEARCH_COVERED is a subset of ALL_SOURCES and excludes meta_search itself", () => {
    for (const id of META_SEARCH_COVERED) {
      expect(ALL_SOURCES).toContain(id);
    }
    expect(META_SEARCH_COVERED).not.toContain("meta_search");
  });

  it("ships meta_search enabled by default", () => {
    expect(DEFAULT_ENABLED_SOURCES).toContain("meta_search");
  });
});

describe("sourceColor", () => {
  it("returns a CSS var per source", () => {
    expect(sourceColor("arxiv")).toContain("--color-source-arxiv");
  });

  it("strips meta_search/ prefix so candidate badges color by the engine", () => {
    expect(sourceColor("meta_search/brave")).toContain("--color-source-brave");
  });
});
