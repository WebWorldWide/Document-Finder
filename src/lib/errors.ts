import { SOURCE_LABELS } from "./utils";

/**
 * Translate a raw backend download-failure string into one plain sentence a
 * non-technical person can act on. The raw text is kept available by callers
 * (e.g. as a tooltip) for bug reports. The backend (task D) also emits friendlier
 * strings now; this is a belt-and-suspenders layer so the UI always reads well.
 */
export function humanizeDownloadError(raw: string | undefined | null): string {
  if (!raw) return "The download didn't complete.";
  const r = raw.toLowerCase();
  const code = raw.match(/\b(4\d\d|5\d\d)\b/)?.[1];
  if (code === "400")
    return "The download link was rejected by the server — it may have expired or need a login. Skipped.";
  if (code === "401" || code === "403")
    return "This source blocked the download — it may require a subscription or sign-in.";
  if (code === "404" || code === "410") return "The document is no longer available at this link.";
  if (code === "429") return "The source was busy and rate-limited us — we backed off and retried.";
  if (code && code.startsWith("5")) return "The source's server had a temporary problem.";
  if (r.includes("landing page") || r.includes("text/html") || r.includes("not a document"))
    return "This link opened a web page, not a downloadable file — skipped.";
  if (r.includes("too large")) return "The file was larger than the download size limit.";
  if (r.includes("too small") || r.includes("empty response"))
    return "The server returned an empty or tiny response — likely an error page.";
  if (r.includes("valid pdf") || r.includes("valid epub") || r.includes("signature"))
    return "The downloaded file was corrupted or wasn't really a document.";
  if (r.includes("no readable text") || r.includes("text is empty"))
    return "Downloaded, but no readable text could be pulled out (it may be a scanned image).";
  if (r.includes("timed out") || r.includes("timeout")) return "The download timed out.";
  if (r.includes("network") || r.includes("dns") || r.includes("connect") || r.includes("reach"))
    return "Couldn't reach the server — check your internet connection.";
  if (r.includes("cancel")) return "Cancelled.";
  return raw;
}

/** Friendly one-liner for a per-source discovery error, by its classified kind. */
export function humanizeSourceKind(kind: string, source?: string): string {
  const name = source ? (SOURCE_LABELS[source] ?? source) : "This source";
  switch (kind) {
    case "rate_limit":
      return `${name} is rate-limiting us right now — we'll back off and keep going.`;
    case "forbidden":
      return `${name} blocked the request (it may need a sign-in or be region-locked).`;
    case "server_error":
      return `${name} had a temporary server error.`;
    case "timeout":
      return `${name} took too long to respond.`;
    case "parse_error":
      return `Couldn't read ${name}'s response.`;
    default:
      return `${name} reported a problem.`;
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
