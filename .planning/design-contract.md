# Document Finder 0.3 — design contract

Per the `design-ui` skill (AdamNolle/my-skills). App = **repeated task**; site = **brand encounter**.

- **Purpose.** App: type a question and get a trustworthy, readable library with no decisions. Site: understand in one screen what it does (finds open-access research → clean text for your AI) and download it.
- **Art direction.** *A quiet reading room.* Warm paper / deep ink fields, hairline rules, catalog numerals (№ 014), condensed index labels. Documents are *catalog slips* that get filed into a ruled ledger as they arrive. Signature: the filing motion — a slip slides into the ledger and its catalog number ticks up.
- **Reference roles.**
  - Raycast supplies the focused command surface: the query and Library search, with a clear selected-item inspector.
  - Locomotive supplies the editorial hierarchy: ruled lists, quiet navigation, large serif titles against small labels.
  - Remy supplies the framing language: tick marks for the active stage, and one active item with quieter neighbours.
  - Original: the catalog-slip motif, the ledger, and the stage ticks.
- **Typography.**
  - Instrument Serif 400 (roman and italic) for page titles, library names, the hero prompt and empty states only. Never bold, never under 24px.
  - Archivo variable for everything else. Width stops: 80 (labels, uppercase, +0.06em tracking), 100 (UI and reading), 112 (rare emphasis).
  - Numbers use `tabular-nums`. Mono only for file paths.
  - Both fonts are bundled as WOFF2: the app CSP blocks CDNs, and the site self-hosts them too.
- **Type scale (app).** 11 (labels), 13 (UI), 15 (reading), 28 / 44 (serif titles), 64 (hero prompt). Five sizes.
- **Surface system.**
  - Themes: Light, Dark, System.
  - One accent, blue-green from the logo: `#1E7F6E` light / `#5CC8A8` dark.
  - Status colors: ok / warn / bad.
  - Radii: 4 (controls), 8 (panels), 999 (pills). Hairline 1px rules carry structure; shadows only for overlays (dialog, toast).
  - Icons: lucide only, 16px, 1.75 stroke.
- **Assets.** Real product screenshots captured from the app, for the site. The existing app icon stays (it's the user's artwork). The site logo is the existing SVG.
- **Signature interaction.**
  - A new saved document slides 6px up and fades in at the top of the ledger. Its № increments, and the stage tick advances.
  - With reduced motion, items simply appear.
- **Small screens.**
  - App minimum is 960px: the sidebar collapses to icons below 1100px.
  - Site: the nav becomes a menu button at ≤ 860px. The hero stacks headline over screenshot, and sections go single-column.
