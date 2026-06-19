export function formatBytes(bytes: number): string {
  if (!bytes) return "—";
  // Guard against Infinity/NaN from a bad rate/division: the loop below is
  // bounded by `i`, so this can't hang, but it would otherwise render the
  // literal "Infinity GB".
  if (!Number.isFinite(bytes)) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
}

/** Compare two library folder names by the run timestamp the backend embeds as
 *  a `{slug}-{secs}-{nanos}` suffix, newest first. Used for the "Recent" views,
 *  where the backend's reverse-name ordering is dominated by the slug text and
 *  doesn't reflect recency. Names without the suffix sort last. */
export function compareLibraryRecency(a: string, b: string): number {
  const ts = (name: string): [number, number] => {
    const m = /-(\d+)-(\d+)$/.exec(name);
    return m ? [Number(m[1]), Number(m[2])] : [0, 0];
  };
  const [as, an] = ts(a);
  const [bs, bn] = ts(b);
  return bs - as || bn - an;
}

/** Unix-seconds run timestamp embedded in a library folder name as the
 *  `-{secs}-{nanos}` suffix, or null if the name has no such suffix. */
export function libraryTimestamp(name: string): number | null {
  const m = /-(\d+)-(\d+)$/.exec(name);
  return m ? Number(m[1]) : null;
}

/** Compact relative age ("just now", "5m ago", "3h ago", "2d ago") for recent
 *  items, falling back to an absolute date past a week. Lets the Library distinguish
 *  multiple libraries created from the SAME query (re-searching a topic) — they'd
 *  otherwise show identical titles. `secs` is unix seconds; `nowMs` is injectable
 *  for tests. */
export function formatRelativeTime(secs: number, nowMs: number = Date.now()): string {
  const deltaMs = nowMs - secs * 1000;
  const min = deltaMs / 60000;
  if (min < 1) return "just now";
  if (min < 60) return `${Math.floor(min)}m ago`;
  const hr = min / 60;
  if (hr < 24) return `${Math.floor(hr)}h ago`;
  const day = hr / 24;
  if (day < 7) return `${Math.floor(day)}d ago`;
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export const SOURCE_LABELS: Record<string, string> = {
  arxiv: "arXiv",
  openalex: "OpenAlex",
  semantic_scholar: "Semantic Scholar",
  europe_pmc: "Europe PMC",
  internet_archive: "Internet Archive",
  doaj: "DOAJ",
  zenodo: "Zenodo",
  gutenberg: "Gutenberg",
  meta_search: "Web (built-in)",
  searxng: "SearXNG",
  web: "DuckDuckGo",
  brave: "Brave",
  bing: "Bing",
  mojeek: "Mojeek",
  marginalia: "Marginalia",
  startpage: "Startpage",
  // Internal fallback sources tagged by SearxngPoolSource (not user-selectable).
  searxng_local: "SearXNG (local)",
  searxng_pool: "SearXNG (pool)",
};

export const ALL_SOURCES = [
  "arxiv",
  "openalex",
  "semantic_scholar",
  "europe_pmc",
  "internet_archive",
  "doaj",
  "zenodo",
  "gutenberg",
  "meta_search",
  "searxng",
  "web",
  "brave",
  "bing",
  "mojeek",
  "marginalia",
  "startpage",
] as const;

export type SourceId = (typeof ALL_SOURCES)[number];

/// Sources we ship enabled by default on a fresh install. The built-in
/// meta-search aggregator (`meta_search`) is the default web backend and
/// internally falls back to the SearXNG pool, so the standalone `searxng`
/// source is intentionally NOT enabled by default — when both are on, the
/// orchestrator drops `searxng` as already-covered, which made it look
/// "enabled but doing nothing". It remains available to select manually.
export const DEFAULT_ENABLED_SOURCES: SourceId[] = [
  "arxiv",
  "openalex",
  "semantic_scholar",
  "europe_pmc",
  "internet_archive",
  "doaj",
  "zenodo",
  "gutenberg",
  "meta_search",
];

/// Web-engine ids that the meta_search aggregator covers. We keep these as
/// individually-selectable advanced options but hide them from the main
/// toggle group when meta_search is active.
export const META_SEARCH_COVERED: SourceId[] = [
  "web",
  "brave",
  "bing",
  "mojeek",
  "marginalia",
  "startpage",
];

export function sourceColor(source: string): string {
  // Strip the `meta_search/<engine>` prefix so candidate badges still color
  // by the originating engine.
  const key = source.startsWith("meta_search/") ? source.slice("meta_search/".length) : source;
  return `var(--color-source-${key.replace(/-/g, "_")}, #5f86b0)`;
}

/// Human label for a source id, stripping the `meta_search/<engine>` prefix the
/// backend stamps on web results (so a download row shows "DuckDuckGo", not the
/// raw "meta_search/web"). Mirrors sourceColor's prefix handling; SOURCE_LABELS
/// has entries for both the bare engine keys and the aggregator.
export function sourceLabel(source: string): string {
  const key = source.startsWith("meta_search/") ? source.slice("meta_search/".length) : source;
  return SOURCE_LABELS[key] ?? SOURCE_LABELS[source] ?? source;
}

/// One-line descriptions for the rich Sources panel on Discover.
export const SOURCE_DESC: Record<string, string> = {
  arxiv: "Preprints in CS, physics, math, biology",
  openalex: "~250M scholarly works · open-access filter",
  semantic_scholar: "~200M papers · semantic relevance ranking",
  europe_pmc: "Life-science papers & preprints · open full text",
  internet_archive: "Books, papers, scanned media · deep but slow",
  doaj: "Directory of Open Access Journals",
  zenodo: "CERN open repository · papers, preprints, theses",
  gutenberg: "70,000+ public-domain ebooks · EPUB",
  meta_search: "6 web engines in parallel · no setup",
  searxng: "Privacy metasearch · in-process, no Docker",
  web: "DuckDuckGo · open-web document discovery",
  brave: "Brave Search · open-web results",
  bing: "Bing · open-web results",
  mojeek: "Mojeek · independent crawler",
  marginalia: "Marginalia · indie & long-tail web",
  startpage: "Startpage · privacy-front results",
};

export function sourceDesc(source: string): string {
  return SOURCE_DESC[source] ?? "Open-access document source";
}

export type FileType = "pdf" | "epub" | "html" | "txt";

/// Infer a document's file type from its saved path or URL, for the file-type
/// breakdown chips. Returns null when it can't be determined.
export function ftypeFromPath(path?: string | null): FileType | null {
  if (!path) return null;
  const m = path.toLowerCase().match(/\.(pdf|epub|html?|txt)(?:[?#].*)?$/);
  if (!m) return null;
  return m[1].startsWith("htm") ? "html" : (m[1] as FileType);
}
