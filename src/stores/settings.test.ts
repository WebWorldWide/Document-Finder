import { describe, expect, it } from "vitest";
import { safeSources } from "@/stores/settings";
import { DEFAULT_ENABLED_SOURCES } from "@/lib/utils";

describe("safeSources", () => {
  it("falls back to defaults for junk", () => {
    expect(safeSources(null)).toEqual(DEFAULT_ENABLED_SOURCES);
    expect(safeSources(["nope"])).toEqual(DEFAULT_ENABLED_SOURCES);
  });
  it("keeps valid ids and drops unknown ones", () => {
    expect(safeSources(["arxiv", "bogus", "openalex"])).toEqual(["arxiv", "openalex"]);
  });
});
