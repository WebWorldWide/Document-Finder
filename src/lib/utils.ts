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

export function formatEta(seconds: number | null): string {
  if (seconds == null) return "—";
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export const SOURCE_LABELS: Record<string, string> = {
  arxiv: "arXiv",
  openalex: "OpenAlex",
  semantic_scholar: "Semantic Scholar",
  internet_archive: "Internet Archive",
  doaj: "DOAJ",
  gutenberg: "Gutenberg",
  web: "Web",
  searxng: "SearXNG",
};

/// One-line description shown in the SourcePanel under each source name.
/// Kept brief — the row is dense and these clip to one line on narrow widths.
export const SOURCE_DESCRIPTIONS: Record<string, string> = {
  arxiv: "Physics, CS, math preprints",
  openalex: "Open scholarly graph — 240M works",
  semantic_scholar: "AI-augmented paper index",
  internet_archive: "Books, papers, scans",
  doaj: "Directory of Open Access Journals",
  gutenberg: "Public-domain books",
  web: "Web — DuckDuckGo HTML",
  searxng: "Local SearXNG (embedded)",
};

export const ALL_SOURCES = [
  "arxiv",
  "openalex",
  "semantic_scholar",
  "internet_archive",
  "doaj",
  "gutenberg",
  "web",
  "searxng",
] as const;

export type SourceId = (typeof ALL_SOURCES)[number];

export function sourceColor(source: string): string {
  // Strip `meta_search/<engine>` prefix so the originating engine still
  // colors the badge.
  const key = source.startsWith("meta_search/")
    ? source.slice("meta_search/".length)
    : source;
  return `var(--src-${key.replace(/-/g, "_")}, var(--accent))`;
}
