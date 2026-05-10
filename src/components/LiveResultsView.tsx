import { For, Show, createMemo, createSignal } from "solid-js";
import { CheckCircle2, XCircle, ExternalLink, Download, AlertCircle } from "lucide-solid";
import {
  runStore,
  type Candidate,
  type CompletedItem,
  type InFlight,
} from "@/stores/run";
import { api } from "@/lib/tauri";
import { formatBytes, SOURCE_LABELS, sourceColor } from "@/lib/utils";
import PipelineStrip from "./PipelineStrip";

type Lane = "found" | "downloading" | "completed" | "failed";

const MAX_VISIBLE_ROWS = 250;

function SourceBadge(props: { source: string; size?: "sm" | "xs" }) {
  return (
    <span
      class="shrink-0 rounded-full px-1.5 py-0.5 font-medium text-white"
      classList={{
        "text-[10px]": props.size !== "xs",
        "text-[9px]": props.size === "xs",
      }}
      style={{ "background-color": sourceColor(props.source) }}
    >
      {SOURCE_LABELS[props.source] ?? props.source}
    </span>
  );
}

function ScoreBreakdown(props: { c: Candidate }) {
  return (
    <span
      class="shrink-0 font-mono text-[9px] text-[var(--color-muted-foreground)]"
      title={`tfidf=${props.c.tfidf.toFixed(2)}  rrf=${props.c.rrf.toFixed(3)}  authority=${props.c.authority.toFixed(2)}  ⇒ score=${props.c.score.toFixed(3)}`}
    >
      score {props.c.score.toFixed(2)}
    </span>
  );
}

export default function LiveResultsView() {
  const [lane, setLane] = createSignal<Lane>("found");

  const allCandidates = createMemo(() => runStore.state.candidates);
  const inFlight = createMemo(() => Object.values(runStore.state.inFlight));
  const completedItems = createMemo(() =>
    runStore.state.completed.filter((c) => c.status === "done").slice().reverse()
  );
  const failedItems = createMemo(() =>
    runStore.state.completed.filter((c) => c.status === "failed").slice().reverse()
  );

  const counts = createMemo(() => ({
    found: allCandidates().length,
    downloading: inFlight().length,
    completed: completedItems().length,
    failed: failedItems().length,
  }));

  // Bulk control: re-queue rejected candidates as downloads.
  // For now just emit a UX note since the backend doesn't yet have a
  // re-queue endpoint — wired here so the button is in place.
  const [showOverrideHint, setShowOverrideHint] = createSignal(false);

  async function cancelAll() {
    try {
      await api.cancelRun();
    } catch (e) {
      console.error("cancel failed", e);
    }
  }

  return (
    <div class="flex h-full flex-col">
      {/* Pipeline progress strip — every stage of the run, all in one glance */}
      <PipelineStrip />

      {/* Header — counters + bulk actions */}
      <div class="flex items-center justify-between gap-3 px-4 py-3">
        <div class="surface-raised-sm surface-bevel-sm flex items-center gap-1 p-1">
          <LaneTab
            id="found"
            label="All Found"
            count={counts().found}
            active={lane() === "found"}
            onSelect={setLane}
          />
          <LaneTab
            id="downloading"
            label="Downloading"
            count={counts().downloading}
            active={lane() === "downloading"}
            onSelect={setLane}
          />
          <LaneTab
            id="completed"
            label="Completed"
            count={counts().completed}
            active={lane() === "completed"}
            onSelect={setLane}
          />
          <LaneTab
            id="failed"
            label="Failed"
            count={counts().failed}
            active={lane() === "failed"}
            onSelect={setLane}
          />
        </div>

        <div class="flex items-center gap-2">
          <Show when={runStore.state.rankingDone}>
            <span class="text-[10px] text-[var(--color-foreground-muted)]">
              {runStore.state.rankingKept} kept · {runStore.state.rankingRejected} rejected
            </span>
          </Show>
          <Show when={runStore.state.running}>
            <button
              onClick={cancelAll}
              class="btn-tactile px-3 py-1.5 text-[11px] font-medium text-[var(--color-destructive)]"
            >
              Cancel All
            </button>
          </Show>
        </div>
      </div>

      {/* Lane content */}
      <div class="flex-1 overflow-y-auto px-4 pb-4 pt-1">
        <Show when={lane() === "found"}>
          <FoundLane
            candidates={allCandidates()}
            onOverrideHint={() => setShowOverrideHint(true)}
          />
          <Show when={showOverrideHint()}>
            <p class="mt-3 rounded-md border border-[var(--color-border)] bg-[var(--color-muted)] p-2 text-[10px] text-[var(--color-muted-foreground)]">
              Override-download for individual rejected candidates is on the
              roadmap. For now, drop a query that targets the specific paper
              into the Find tab.
            </p>
          </Show>
        </Show>

        <Show when={lane() === "downloading"}>
          <DownloadingLane items={inFlight()} />
        </Show>

        <Show when={lane() === "completed"}>
          <CompletedLane items={completedItems()} />
        </Show>

        <Show when={lane() === "failed"}>
          <FailedLane items={failedItems()} />
        </Show>
      </div>
    </div>
  );
}

function LaneTab(props: {
  id: Lane;
  label: string;
  count: number;
  active: boolean;
  onSelect: (id: Lane) => void;
}) {
  return (
    <button
      onClick={() => props.onSelect(props.id)}
      class="px-3 py-1.5 text-[12px] font-medium transition-all duration-150"
      classList={{
        "surface-pressed-sm": props.active,
        "btn-tactile": !props.active,
      }}
      style={{
        color: props.active ? "var(--color-primary)" : "var(--color-foreground-muted)",
      }}
    >
      {props.label}
      <span
        class="ml-1.5 rounded-full px-1.5 py-0.5 text-[10px] font-mono"
        style={{
          background: props.active
            ? "color-mix(in oklch, var(--color-primary) 18%, transparent)"
            : "var(--color-surface-deep)",
          color: props.active
            ? "var(--color-primary)"
            : "var(--color-foreground-muted)",
        }}
      >
        {props.count}
      </span>
    </button>
  );
}

function EmptyState(props: { msg: string }) {
  return (
    <div class="flex h-full items-center justify-center">
      <p class="text-xs text-[var(--color-muted-foreground)]">{props.msg}</p>
    </div>
  );
}

function FoundLane(props: {
  candidates: Candidate[];
  onOverrideHint: () => void;
}) {
  // Sort: kept (by final_rank), then rejected (by score).
  const sorted = createMemo(() => {
    const kept = props.candidates.filter((c) => c.status === "kept");
    const rejected = props.candidates.filter((c) => c.status !== "kept");
    kept.sort((a, b) => (a.final_rank ?? 999) - (b.final_rank ?? 999));
    rejected.sort((a, b) => b.score - a.score);
    return [...kept, ...rejected];
  });
  const visible = createMemo(() => sorted().slice(0, MAX_VISIBLE_ROWS));

  return (
    <Show when={sorted().length > 0} fallback={<EmptyState msg="Waiting for results…" />}>
      {/* Single raised container — rows inside are flat with accent strips,
        * eliminating the per-row shadow halo that was making the list look
        * blotchy at high density. */}
      <div class="surface-raised divide-y divide-[var(--color-border)]/50 overflow-hidden">
        <For each={visible()}>
          {(c) => <CandidateRow c={c} onOverrideHint={props.onOverrideHint} />}
        </For>
      </div>
      <Show when={sorted().length > visible().length}>
        <p class="mt-3 text-center text-[10px] text-[var(--color-foreground-muted)]">
          Showing {visible().length} of {sorted().length} candidates. Filter or
          narrow the query to see more.
        </p>
      </Show>
    </Show>
  );
}

function CandidateRow(props: {
  c: Candidate;
  onOverrideHint: () => void;
}) {
  const isRejected = () => props.c.status !== "kept";
  return (
    <li
      class="animate-slide-in flex items-start gap-2 p-2.5 transition-colors duration-100"
      classList={{
        "hover:bg-[var(--color-foreground)]/3": !isRejected(),
        "opacity-55 bg-[var(--color-foreground)]/2": isRejected(),
      }}
      style={{
        "border-left": isRejected()
          ? "3px solid var(--color-foreground-muted)"
          : "3px solid var(--color-primary)",
      }}
      title={
        props.c.reject_reason ??
        `Rank #${props.c.final_rank} — score ${props.c.score.toFixed(3)}`
      }
    >
      <div class="flex shrink-0 flex-col items-center gap-1 pt-0.5">
        <Show
          when={isRejected()}
          fallback={
            <span class="font-mono text-[10px] font-medium text-[var(--color-primary)]">
              #{props.c.final_rank}
            </span>
          }
        >
          <AlertCircle
            size={12}
            class="text-[var(--color-muted-foreground)]"
          />
        </Show>
      </div>

      <div class="min-w-0 flex-1">
        <div class="flex items-start gap-2">
          <a
            href={props.c.url}
            target="_blank"
            rel="noreferrer"
            class="line-clamp-2 flex-1 text-[12px] font-medium hover:underline"
          >
            {props.c.title}
          </a>
          <a
            href={props.c.url}
            target="_blank"
            rel="noreferrer"
            class="shrink-0 text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)]"
          >
            <ExternalLink size={11} />
          </a>
        </div>

        <div class="mt-1 flex flex-wrap items-center gap-1.5">
          <For each={props.c.sources}>
            {(s) => <SourceBadge source={s} size="xs" />}
          </For>

          <Show when={props.c.year}>
            <span class="text-[10px] text-[var(--color-muted-foreground)]">
              {props.c.year}
            </span>
          </Show>

          <Show when={props.c.authors.length > 0}>
            <span class="truncate text-[10px] text-[var(--color-muted-foreground)] max-w-[200px]">
              {props.c.authors.slice(0, 3).join(", ")}
              {props.c.authors.length > 3 ? " et al." : ""}
            </span>
          </Show>

          <ScoreBreakdown c={props.c} />

          <Show when={props.c.reject_reason}>
            <span class="text-[10px] italic text-[var(--color-muted-foreground)]">
              · {props.c.reject_reason}
            </span>
          </Show>
        </div>
      </div>

      <Show when={isRejected()}>
        <button
          onClick={props.onOverrideHint}
          class="btn-tactile shrink-0 p-1.5 text-[var(--color-foreground-muted)]"
          title="Force download (rejected by ranker)"
        >
          <Download size={11} />
        </button>
      </Show>
    </li>
  );
}

function DownloadingLane(props: { items: InFlight[] }) {
  return (
    <Show
      when={props.items.length > 0}
      fallback={<EmptyState msg="Nothing downloading." />}
    >
      <div class="surface-raised divide-y divide-[var(--color-border)]/50 overflow-hidden">
        <For each={props.items}>
          {(item) => {
            const pct = () =>
              item.total > 0 ? Math.round((item.downloaded / item.total) * 100) : 0;
            return (
              <div class="p-3" style={{ "border-left": "3px solid var(--color-primary)" }}>
                <div class="mb-2 flex items-center gap-2">
                  <SourceBadge source={item.source} />
                  <span class="line-clamp-1 flex-1 text-[12px]">{item.title}</span>
                </div>
                <div class="flex items-center gap-2">
                  <div class="surface-pressed-sm h-1.5 flex-1 overflow-hidden">
                    <div
                      class="h-full rounded-full transition-all duration-300"
                      classList={{
                        "progress-shimmer": item.total === 0,
                        "bg-[var(--color-primary)]": item.total > 0,
                      }}
                      style={{
                        width: item.total === 0 ? "100%" : `${pct()}%`,
                      }}
                    />
                  </div>
                  <span class="shrink-0 font-mono text-[10px] text-[var(--color-foreground-muted)]">
                    {item.total > 0
                      ? `${formatBytes(item.downloaded)} / ${formatBytes(item.total)}`
                      : formatBytes(item.downloaded)}
                  </span>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </Show>
  );
}

function CompletedLane(props: { items: CompletedItem[] }) {
  return (
    <Show
      when={props.items.length > 0}
      fallback={<EmptyState msg="No completed downloads yet." />}
    >
      <div class="surface-raised divide-y divide-[var(--color-border)]/50 overflow-hidden">
        <For each={props.items.slice(0, MAX_VISIBLE_ROWS)}>
          {(item) => (
            <div class="flex items-center gap-2 px-3 py-2 hover:bg-[var(--color-foreground)]/3" style={{ "border-left": "3px solid var(--color-success)" }}>
              <CheckCircle2
                size={13}
                class="shrink-0"
                style={{ color: "var(--color-success)" }}
              />
              <SourceBadge source={item.source} size="xs" />
              <span class="line-clamp-1 flex-1 text-[11px]">{item.title}</span>
              <Show when={item.absolute_path}>
                <button
                  onClick={() => api.revealInFinder(item.absolute_path!)}
                  class="shrink-0 text-[10px] text-[var(--color-foreground-muted)] hover:text-[var(--color-primary)]"
                >
                  reveal
                </button>
              </Show>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

function FailedLane(props: { items: CompletedItem[] }) {
  return (
    <Show
      when={props.items.length > 0}
      fallback={<EmptyState msg="No failures." />}
    >
      <div class="surface-raised divide-y divide-[var(--color-border)]/50 overflow-hidden">
        <For each={props.items.slice(0, MAX_VISIBLE_ROWS)}>
          {(item) => (
            <div class="flex items-start gap-2 px-3 py-2" style={{ "border-left": "3px solid var(--color-destructive)" }}>
              <XCircle
                size={13}
                class="mt-0.5 shrink-0 text-[var(--color-destructive)]"
              />
              <SourceBadge source={item.source} size="xs" />
              <div class="min-w-0 flex-1">
                <p class="line-clamp-1 text-[11px]">{item.title}</p>
                <Show when={item.error}>
                  <p class="line-clamp-1 text-[10px] text-[var(--color-destructive)]">
                    {item.error}
                  </p>
                </Show>
              </div>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
