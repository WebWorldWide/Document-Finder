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

export const SOURCE_LABELS: Record<string, string> = {
  arxiv: "arXiv",
  openalex: "OpenAlex",
  semantic_scholar: "Semantic Scholar",
  internet_archive: "Internet Archive",
  doaj: "DOAJ",
  gutenberg: "Gutenberg",
  meta_search: "Web (built-in)",
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
  "internet_archive",
  "doaj",
  "gutenberg",
  "meta_search",
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
  "internet_archive",
  "doaj",
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
  return `var(--color-source-${key.replace(/-/g, "_")}, oklch(0.7 0.1 270))`;
}
