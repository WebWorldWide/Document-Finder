# Document Finder — UI Review

**Audited:** 2026-05-05
**Baseline:** Abstract 6-pillar standards (no UI-SPEC.md present)
**Screenshots:** Not captured (dev server not running; code-only audit)
**Stack:** Solid.js (TSX) + Tailwind CSS v4 + lucide-solid

---

## Pillar Scores

| Pillar | Score | Key Finding |
|--------|-------|-------------|
| 1. Visual Correctness | 7/10 | `#root` height rule never applies; pt-8 vs pt-10 inconsistency across views |
| 2. Reactive Correctness | 6/10 | Unlisten fn discarded; LibraryView infinite spinner on fresh install; `map()` nav loses active-state reactivity path |
| 3. State Correctness | 5/10 | Library export error silently swallowed; no dismiss on persistent banners; fatalError survives across new runs |
| 4. Accessibility | 3/10 | Zero aria-labels anywhere; no focus rings on any button; Library Folder input has no label element |
| 5. Error Handling UX | 6/10 | Raw Rust error strings shown verbatim; SearXNG export error silent; no retry buttons |
| 6. Polish & Consistency | 7/10 | `emerald-*` hardcodes break light/dark parity; `web` and `searxng` share identical source color; success banners never auto-dismiss |

**Overall: 34/60**

---

## Top 3 Priority Fixes

1. **Zero accessibility labels on all buttons and the Library Folder input** — Keyboard and screen reader users cannot navigate the sidebar, source toggles, action row, or settings inputs at all. Add `aria-label` to every button that contains only an icon, and wrap the Library Folder `<input>` in a `<label>` or attach `id`/`for`. Score impact: Pillar 4 at 3/10.

2. **Event listener unlisten function is discarded — memory/event leak** — `main.tsx:7` calls `listenAll(...)` but does not `await` it and does not store the returned `UnlistenFn`. Every Tauri event channel is permanently attached with no ability to tear down. In a Tauri 2 app the window never reloads, so this is low-severity in practice, but if the component tree ever re-mounts (HMR, future multi-window) listeners stack. Fix: `const unlisten = await listenAll(...)` and call it in the appropriate cleanup.

3. **Library export errors are silently swallowed in LibraryView** — `handleExport` has a `try/finally` with no `catch`. If `api.exportLibraryZip` throws, the error is discarded, `setExportingPath(null)` is called, and the user sees nothing. Add an `exportError` signal and render a dismissible error banner inline on the card.

---

## Detailed Findings

### Pillar 1: Visual Correctness (7/10)

**WARNING — `#root` height rule never matches the DOM**
`src/styles/globals.css:38` sets `html, body, #root { height: 100%; }`. The app entry point (`index.html:10`) mounts to `<div id="app">`, not `#root`. The `#root` selector is dead. Because `App` uses `h-screen` this does not break layout today, but the intent is not being fulfilled, and a future refactor that removes `h-screen` will cause a full-page collapse.

Fix:
```css
/* src/styles/globals.css:38 */
html, body, #app {
  height: 100%;
  margin: 0;
}
```

**WARNING — pt-8 on Sidebar vs pt-10 on main-area views**
`Sidebar.tsx:15` uses `pt-8` (32 px) to clear the macOS traffic-light drag region. All main-area views (`FindTab.tsx:65`, `LibraryView.tsx:42`, `SettingsView.tsx:48`) use `pt-10` (40 px). The drag region is declared as `h-8` (32 px) in `App.tsx:12`. Sidebar top-edge content starts 8 px lower than it needs to; main views add 8 px of unnecessary additional clearance above their first visible element.

Fix: Standardise on `pt-8` everywhere, or introduce a CSS variable `--drag-region-height: 2rem` and apply it consistently.

**LOW — In-flight download title alignment**
`LiveDownloadStream.tsx:32` truncates the title text with `text-right` while the source badge is `shrink-0` on the left. On very short titles the badge and title appear separated by excessive dead space; on very long titles the truncation direction is counter-intuitive (left end of title is hidden, right end visible). Prefer `text-left` + `flex-1 truncate` with the badge staying left.

**LOW — Progress percentage label doubles down on completed count**
`FindTab.tsx:211` shows `{rs().done + rs().failed} / {rs().total} ({runStore.overallPct}%)`. `done + failed` and `overallPct` are derived from the same numerator; displaying both is redundant and visually noisy. Show only the fraction or only the percentage.

---

### Pillar 2: Reactive Correctness (6/10)

**HIGH — `listenAll` return value (UnlistenFn) is discarded**
`src/main.tsx:7`:
```ts
listenAll((ev) => runStore.apply(ev));
```
`listenAll` is `async` and returns `Promise<UnlistenFn>`. The promise is not awaited and the resolved value is never stored. The Tauri `listen()` calls succeed — events work — but there is no cleanup path. If this module is ever evaluated twice (HMR, future architecture changes) a duplicate set of listeners will silently double-process every event, causing double state mutations.

Fix:
```ts
// src/main.tsx
let unlisten: (() => void) | null = null;
listenAll((ev) => runStore.apply(ev)).then((fn) => { unlisten = fn; });
// If a teardown hook is ever needed: unlisten?.()
```

**HIGH — LibraryView shows infinite spinner on fresh install when `libraryRoot` is empty**
`settings.ts:35` initialises `libraryRoot` to `""` when no saved value exists, then fires an async `api.defaultLibraryDir()` call on line 44 to populate it. `LibraryView.tsx:11` initialises `loading` to `true`. The `createEffect` on line 15–23 guards with `if (!root) return` — so when the component mounts before `defaultLibraryDir` resolves, the effect exits early, `setLoading(false)` is never called, and the user sees a perpetual spinner.

Fix: Initialise `loading` to `false` and set it to `true` only inside the effect after the root guard passes, or explicitly handle the `!root` branch:
```ts
// LibraryView.tsx
createEffect(() => {
  const root = settings.libraryRoot;
  if (!root) {
    setLoading(false);   // <-- show empty/configure state, not spinner
    return;
  }
  setLoading(true);
  ...
});
```

**MEDIUM — `navItems.map()` in Sidebar is a static render; reactive active-state depends on closure over `uiStore.view`**
`Sidebar.tsx:51`: `navItems.map((item) => (...))` runs once at component mount (correct for Solid — static arrays produce stable DOM). The `classList` expression accesses `uiStore.view === item.id` which calls `view()` signal, so reactivity is tracked correctly. This is a latent confusion risk — if `navItems` were ever replaced with a reactive source the `.map()` must be changed to `<For>`. Document or replace now.

Fix: Replace with `<For each={navItems}>` for consistency with every other list in the codebase, even though the current behavior is correct.

**LOW — `createEffect` in LibraryView can race if `listLibraries` takes longer than the next reactive update**
The effect does not abort the in-flight promise when `libraryRoot` changes again before the previous call resolves. A rapid settings change could apply library results from an old root to the new state. Add an abort flag or use `onCleanup`:
```ts
createEffect(() => {
  const root = settings.libraryRoot;
  if (!root) return;
  let cancelled = false;
  setLoading(true);
  api.listLibraries(root)
    .then((libs) => { if (!cancelled) { setLibraries(libs); setLoading(false); } })
    .catch((e) => { if (!cancelled) { setError(String(e)); setLoading(false); } });
  onCleanup(() => { cancelled = true; });
});
```

---

### Pillar 3: State Correctness (5/10)

**HIGH — `fatalError` from a previous run persists into the next run**
`runStore.ts:82–100` `reset()` correctly zeroes all counters but does set `fatalError: null`. However, `startSearch` calls `reset()` before `setState("running", true)`, so `fatalError` should clear. On re-inspection `reset()` on line 98 does include `fatalError: null` — this is not a bug. However, if the user presses "Find Documents" immediately after a fatal error, the error banner is still visible for the instant between render and the first reactive flush. Minor.

**CRITICAL — Library export error silently swallowed in `LibraryView.handleExport`**
`LibraryView.tsx:25–38`:
```ts
try {
  const result = await api.exportLibraryZip(lib.path, dest);
  await api.revealInFinder(result.dest);
} finally {
  setExportingPath(null);
}
```
There is no `catch`. Any error from `exportLibraryZip` or `revealInFinder` is silently consumed. The button returns to its normal state and the user has no idea the export failed.

Fix:
```ts
const [exportError, setExportError] = createSignal<string | null>(null);

async function handleExport(lib: LibraryInfo) {
  const dest = await save({ ... });
  if (!dest) return;
  setExportingPath(lib.path);
  setExportError(null);
  try {
    const result = await api.exportLibraryZip(lib.path, dest);
    await api.revealInFinder(result.dest);
  } catch (e) {
    setExportError(String(e));
  } finally {
    setExportingPath(null);
  }
}
```
Then render `<Show when={exportError()}>` with a dismissible banner.

**HIGH — Fatal error, export success, and export error banners have no dismiss button**
`FindTab.tsx:219–235`: All three persistent state banners — `fatalError`, `exportedTo`, `exportError` — render with no way to dismiss. `exportedTo` in particular stays permanently after a successful export, cluttering the UI for the entire session.

Fix: Add an `X` button to each banner:
```tsx
<Show when={exportedTo()}>
  <div class="flex items-center justify-between rounded-lg border border-emerald-500/30 bg-emerald-50 p-3 text-sm text-emerald-800">
    <span>Exported to <code class="text-xs">{exportedTo()}</code></span>
    <button onClick={() => setExportedTo(null)} aria-label="Dismiss" class="ml-3 opacity-60 hover:opacity-100">
      <X size={12} />
    </button>
  </div>
</Show>
```

**MEDIUM — No retry path on LibraryView load error**
`LibraryView.tsx:56–59` shows an error message but no retry button. If `listLibraries` fails (permission issue, stale path), the user is stuck. The only recovery is navigating away and back.

Fix: Add a Retry button that calls `setError(null); setLoading(true);` and re-triggers the effect, or extract the load function and call it directly.

**MEDIUM — Source toggle `selectedSources` can reach zero with no UI feedback on disabled search button**
When the user deselects all sources, `FindTab.tsx:138` disables the search button via `disabled={!query().trim() || settings.selectedSources.length === 0}`. The button correctly goes dim but there is no tooltip, helper text, or inline message explaining why. Users may not understand why the button is unresponsive.

Fix: Add a conditional hint below the source toggles:
```tsx
<Show when={settings.selectedSources.length === 0}>
  <p class="text-xs text-[var(--color-destructive)]">Select at least one source to search.</p>
</Show>
```

**LOW — SettingsView `logInfo` shows "Loading…" forever if `runLogInfo` fails**
`SettingsView.tsx:13–15`: `onMount` calls `api.runLogInfo().catch(() => null)`. If it throws, `setLogInfo(null)` is called (via `.catch(() => null)`), but the fallback content at line 174 shows "Loading…" — because the `Show` fallback text is `"Loading…"` which is appropriate only during initial load, not after a failure. The `null` return path is handled, but the message is wrong.

Fix: Use a separate error signal or change the fallback text to "Unavailable" when logInfo is definitively null.

---

### Pillar 4: Accessibility (3/10)

**CRITICAL — No aria-label on any button in the entire application**
Grepping `aria-label` across all `*.tsx` files returns zero results. Every icon-containing button is inaccessible to screen readers. Critical instances:

- `Sidebar.tsx:39` — "Show in Finder" button: has text label but icon has no `aria-hidden`
- `Sidebar.tsx:29–36` — active library button: no `aria-label` describing action
- `FindTab.tsx:127–133` — Cancel button: has text, but icon `<X>` needs `aria-hidden={true}`
- `FindTab.tsx:153` — `<Loader2>` spinner in Export ZIP button: needs `aria-hidden={true}` and button needs `aria-busy={exporting()}`
- `FindTab.tsx:246–253` — Issues accordion toggle button: no `aria-expanded={showIssues()}` attribute
- `LibraryView.tsx:101–118` — Export and Show buttons inside cards: no labels

Minimum fix for accordion:
```tsx
<button
  onClick={() => setShowIssues((v) => !v)}
  aria-expanded={showIssues()}
  aria-controls="issues-panel"
  ...
>
```

Fix for all icon buttons: add `aria-hidden="true"` to every `lucide-solid` icon inside a button that already has text, and add `aria-label="..."` to any button whose only child is an icon.

**CRITICAL — Library Folder `<input>` at `SettingsView.tsx:95` has no `<label>`, no `aria-label`, no `aria-labelledby`**
The section heading (`<h2>Library Folder</h2>`) provides visual context but is not associated with the input. A screen reader user gets only the `font-mono` placeholder-less input.

Fix:
```tsx
<label>
  <span class="sr-only">Library folder path</span>
  <input ... />
</label>
```

**HIGH — No focus rings on any `<button>` element**
All buttons use `outline-none` or the Tailwind reset's default outline removal. `<textarea>` and `<input>` elements correctly apply `focus:border-primary focus:ring-2`, but none of the 17 `<button>` elements have any `focus-visible:ring-*` class. Keyboard users cannot see which button is focused.

Fix: Add to globals.css or apply per-button:
```css
button:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}
```

**HIGH — `<Switch>/<Match>` view transitions have no focus management**
When the user navigates between Find, Library, and Settings via `uiStore.setView()`, focus remains on the sidebar button that was just clicked. There is no `autofocus` on the main content area or first interactive element. Users relying on keyboard navigation must tab through the sidebar again to reach the main content.

Fix: After `setView`, move focus to the main content region:
```ts
// uiStore.ts — expose a setView that also moves focus
function setViewAndFocus(v: View) {
  setView(v);
  requestAnimationFrame(() => {
    (document.querySelector('main [tabindex="-1"], main h1, main textarea') as HTMLElement)?.focus();
  });
}
```
Or add `tabindex="-1"` to `<main>` in `App.tsx` and call `.focus()` on it.

**MEDIUM — `data-tauri-drag-region` div is `fixed` + `z-50` + `pointer-events-none` but not `aria-hidden`**
`App.tsx:12` — the drag region overlay should be invisible to the accessibility tree:
```tsx
<div aria-hidden="true" class="fixed inset-x-0 top-0 h-8 z-50 pointer-events-none" data-tauri-drag-region />
```

---

### Pillar 5: Error Handling UX (6/10)

**HIGH — Raw Rust/system error strings shown directly to users**
Multiple locations render `String(e)` or the error payload directly:
- `FindTab.tsx:221`: `{rs().fatalError}` — Tauri backend error strings like `"error running command start_run: tauri::Error::..."` can be shown
- `FindTab.tsx:263`: `{issue.error}` — raw source errors (HTTP codes, network stack messages)
- `FindTab.tsx:271`: `{item.error}` — raw download errors
- `LibraryView.tsx:58`: `{error()}` — raw `listLibraries` error
- `SettingsView.tsx:159`: `{searxError()}` with `opacity-80` label

For fatal errors and source issues, apply a sanitisation layer that strips Tauri boilerplate:
```ts
function friendlyError(e: unknown): string {
  const s = String(e);
  // Strip Tauri command prefix
  const match = s.match(/error running command [^:]+: (.*)/s);
  return match ? match[1].trim() : s;
}
```

**HIGH — SearXNG setup success message hardcodes `http://localhost:8080`**
`SettingsView.tsx:146`:
```tsx
<p>SearXNG is running at http://localhost:8080</p>
```
This string is static. The `match` on line 24 extracts the actual URL from the output and stores it in `settings.searxngUrl`, but the success banner ignores it. If the user configured a different port, the success message lies.

Fix:
```tsx
<p>SearXNG is running at {settings.searxngUrl}</p>
```

**MEDIUM — No way to dismiss or clear the fatalError banner without starting a new search**
`FindTab.tsx:219–223`: The fatal error persists until the next `handleSearch` call, which calls `reset()`. If the user edited the query to fix the cause of the error, the error banner still occupies space. Add an explicit dismiss.

**MEDIUM — SettingsView SearXNG error and result banners never clear when re-attempting setup**
`SettingsView.tsx:19–20`: `setSearxResult(null)` and `setSearxError(null)` are correctly called at the start of `handleSetupSearx`. This is fine. However, the success banner `Show when={searxResult() !== null}` would show even for an empty string result `""` (since `null !== null` is false but `"" !== null` is true). If the backend returns an empty string for a successful no-output setup, the `<pre>` block inside the banner would be empty. Minor.

**LOW — LibraryView export error is silent (covered in Pillar 3)**
Repeated here as an error-handling deficiency: see Pillar 3 CRITICAL finding.

---

### Pillar 6: Polish & Consistency (7/10)

**WARNING — `emerald-*` Tailwind color classes break the design token system and dark mode**
8 occurrences across 3 files use hardcoded `emerald-*` classes instead of CSS variables:
- `FindTab.tsx:187`: `text-emerald-600` for "saved" count
- `FindTab.tsx:227`: `border-emerald-500/30 bg-emerald-50 text-emerald-800` export success banner
- `LiveDownloadStream.tsx:75`: `text-emerald-500` for CheckCircle2
- `SettingsView.tsx:143,144,146,148`: `bg-emerald-50 dark:bg-emerald-950/20 text-emerald-800 dark:text-emerald-300 text-emerald-700 dark:text-emerald-400`

The `dark:` classes in SettingsView only work if the app supports dark mode via a `.dark` class on a parent (defined in `globals.css:3` as `@custom-variant dark (&:is(.dark *))`). There is no dark mode toggle in the app and no system preference detection. The `dark:` variants are dead code.

Define a semantic success token instead:
```css
/* globals.css */
--color-success: oklch(0.55 0.15 145);
--color-success-fg: oklch(0.25 0.08 145);
--color-success-bg: oklch(0.97 0.03 145);
```
Then replace all `emerald-*` references with these variables.

**WARNING — `web` and `searxng` sources share identical color `oklch(0.5 0.18 250)`**
`globals.css:32–33`: Both `--color-source-web` and `--color-source-searxng` resolve to the same hue value 250. In the download stream and source toggles, web and searxng results are visually indistinguishable. Assign `searxng` a distinct hue (e.g. `oklch(0.5 0.18 290)`, which is violet vs blue).

**MEDIUM — Source toggle active state has no border when active, relies solely on background fill**
`FindTab.tsx:109–112`: Active source toggles use `border-transparent` to hide the border. If the source color has low contrast against the white card background, the toggle boundary is invisible. Keeping the border as `border-current` or the source color at reduced opacity would maintain the button's interactive affordance:
```tsx
classList={{
  "border-current/50 text-white": active(),
  ...
}}
```

**MEDIUM — Issues accordion uses plain ▲/▼ Unicode characters for expand/collapse indicator**
`FindTab.tsx:254`: `{showIssues() ? "▲ Hide" : "▼ Show"}` — these are not icons from lucide-solid and have inconsistent sizing and weight compared to the rest of the icon system. Replace with `<ChevronUp>` / `<ChevronDown>` from lucide-solid.

**MEDIUM — `"issue(s)"` uses programmer-style parenthesized plural**
`FindTab.tsx:251`: `{count} issue(s)` — this is a code smell in UI copy. Use proper plural logic:
```tsx
{count === 1 ? "1 issue" : `${count} issues`}
```

**LOW — LibraryView card click area vs button click area conflict**
`LibraryView.tsx:88,101`: The outer `<div>` has `onClick={() => uiStore.setActiveLibrary(lib)}` (select library). The inner button row has `onClick={(e) => e.stopPropagation()}` to prevent bubbling. This `stopPropagation` blocks event bubbling but does not prevent the card-level click from firing on any click outside the button row. The pattern is correct but the card div should use `role="button"` or `tabindex="0"` with keyboard support (currently absent), since it has an `onClick` handler.

**LOW — Log path in SettingsView is truncated but title tooltip not present**
`SettingsView.tsx:178`: `<code class="block truncate ...">`. The `truncate` class clips long paths without any tooltip. The full path is only discoverable via "Show in Finder". Add `title={info().path}` to the code element.

**LOW — `formatBytes(0)` returns `"—"` but `lib.n_docs` of 0 renders as `"0 documents"`**
Inconsistent zero handling: bytes show an em-dash for zero, doc count shows `0`. Both are fine individually but the mixed convention looks unpolished on empty libraries. Consider rendering `"—"` for 0 documents too, or rendering bytes as `"0 B"` instead.

---

## Files Audited

| File | Lines |
|------|-------|
| `src/main.tsx` | 8 |
| `src/App.tsx` | 23 |
| `src/components/Sidebar.tsx` | 67 |
| `src/components/FindTab.tsx` | 282 |
| `src/components/LiveDownloadStream.tsx` | 97 |
| `src/components/LibraryView.tsx` | 129 |
| `src/components/SettingsView.tsx` | 208 |
| `src/stores/run.ts` | 294 |
| `src/stores/settings.ts` | 58 |
| `src/stores/ui.ts` | 14 |
| `src/lib/tauri.ts` | 71 |
| `src/lib/events.ts` | 146 |
| `src/lib/utils.ts` | 46 |
| `src/styles/globals.css` | 106 |
| `index.html` | 13 |
| `vite.config.ts` | 19 |
| `package.json` | 27 |

**Registry audit:** shadcn (`components.json`) not present — registry audit skipped.
