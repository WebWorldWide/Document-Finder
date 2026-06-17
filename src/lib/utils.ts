export function formatBytes(bytes: number): string {
  if (!bytes) return "—";
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

export const SOURCE_LABELS: Record<string, string> = {
  arxiv: "arXiv",
  openalex: "OpenAlex",
  semantic_scholar: "Semantic Scholar",
  europe_pmc: "Europe PMC",
  internet_archive: "Internet Archive",
  doaj: "DOAJ",
  zenodo: "Zenodo",
  core: "CORE",
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
  "core",
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
/// meta-search aggregator (`meta_search`) replaces SearXNG as the default
/// web backend so the app works zero-config.
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
  "searxng",
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

/// One-line descriptions for the rich Sources panel on Discover.
export const SOURCE_DESC: Record<string, string> = {
  arxiv: "Preprints in CS, physics, math, biology",
  openalex: "~250M scholarly works · open-access filter",
  semantic_scholar: "~200M papers · semantic relevance ranking",
  europe_pmc: "Life-science papers & preprints · open full text",
  internet_archive: "Books, papers, scanned media · deep but slow",
  doaj: "Directory of Open Access Journals",
  zenodo: "CERN open repository · papers, preprints, theses",
  core: "300M+ open-access works · needs a free API key",
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
