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
  web: "DuckDuckGo",
  brave: "Brave",
  bing: "Bing",
  searxng: "SearXNG",
};

export const ALL_SOURCES = [
  "arxiv",
  "openalex",
  "semantic_scholar",
  "internet_archive",
  "doaj",
  "gutenberg",
  "web",
  "brave",
  "bing",
  "searxng",
] as const;

export type SourceId = (typeof ALL_SOURCES)[number];

export function sourceColor(source: string): string {
  return `var(--color-source-${source.replace(/-/g, "_")}, oklch(0.7 0.1 270))`;
}
