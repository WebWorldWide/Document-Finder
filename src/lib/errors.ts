import { SOURCE_LABELS } from "./utils";

/**
 * Translate a raw backend download-failure string into one plain sentence a
 * non-technical person can act on. The raw text is kept available by callers
 * (e.g. as a tooltip) for bug reports. The backend (task D) also emits friendlier
 * strings now; this is a belt-and-suspenders layer so the UI always reads well.
 */
export function humanizeDownloadError(raw: string | undefined | null): string {
  if (!raw) return "Didn’t download";
  const r = raw.toLowerCase();
  // The offline pre-check writes a deliberately friendly, offline-specific
  // message; pass it through verbatim instead of collapsing it into the generic
  // "couldn't reach the server" line (it contains the word "connect", which the
  // network branch below would otherwise match).
  if (r.includes("offline")) return raw;
  // Match an ACTUAL HTTP status (the backend formats these as "(HTTP 404)"), not
  // any stray 3-digit number — otherwise a message like "only 500 bytes — likely
  // an error page" gets misread as a 5xx server problem.
  const code = raw.match(/\bHTTP\s*(\d{3})\b/i)?.[1];
  if (code === "400") return "Link rejected";
  if (code === "401" || code === "403") return "Needs a subscription";
  if (code === "404" || code === "410") return "No longer available";
  if (code === "429") return "Source busy";
  if (code && code.startsWith("5")) return "Server error";
  if (r.includes("landing page") || r.includes("text/html") || r.includes("not a document"))
    return "No file at link";
  if (r.includes("too large")) return "Too large";
  if (r.includes("too small") || r.includes("empty response")) return "Empty response";
  if (r.includes("valid pdf") || r.includes("valid epub") || r.includes("signature"))
    return "Not a document";
  if (r.includes("no readable text") || r.includes("text is empty")) return "No readable text";
  if (r.includes("timed out") || r.includes("timeout")) return "Timed out";
  if (r.includes("network") || r.includes("dns") || r.includes("connect") || r.includes("reach"))
    return "Couldn’t connect";
  if (r.includes("cancel")) return "Cancelled.";
  return raw;
}

/** Friendly one-liner for a per-source discovery error, by its classified kind. */
export function humanizeSourceKind(kind: string, source?: string): string {
  const name = source ? (SOURCE_LABELS[source] ?? source) : "This source";
  switch (kind) {
    case "rate_limit":
      return `${name}: busy`;
    case "forbidden":
      return `${name}: blocked`;
    case "server_error":
      return `${name}: server error`;
    case "timeout":
      return `${name}: timed out`;
    case "parse_error":
      return `${name}: unreadable`;
    default:
      return `${name}: error`;
  }
}

/** Short tag for an issue kind, shown next to the source name. */
export function issueKindTag(kind: string): string {
  switch (kind) {
    case "rate_limit":
      return "rate limited";
    case "forbidden":
      return "blocked";
    case "server_error":
      return "server error";
    case "timeout":
      return "timed out";
    case "parse_error":
      return "unreadable";
    default:
      return "issue";
  }
}
