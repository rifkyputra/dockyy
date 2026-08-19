# Phase 7 · The Web UI, As Good As It Can Be

Rifky's brief (2026-08-19): "bikin web UI sebagus mungkin" — make the web UI as good as
possible. Phases 3–4 built a UI that is structurally right (semantic HTML, one layout, SSE
fragments, maud escaping everywhere) and visually bare: 124 lines of CSS, default tables, no
navigation, no designed states. This phase makes it look and feel like a real operator console
without changing what it is.

## What does not change

- **The architecture.** Server-rendered maud + htmx + SSE + one plain CSS file. No build step,
  no preprocessor, no framework, no JavaScript beyond the vendored htmx/sse extension. The
  stylesheet stays readable top to bottom.
- **Zero new dependencies.** This phase is markup and CSS.
- **`maud::PreEscaped` appears nowhere.** Journald content still flows through these pages.
- **Every existing test id and class keeps its name** (`#apps`, `#app-facts`,
  `#deploy-history`, `#redeploy`, `#register-*`, `#log-tail`, `#log-empty`, `#log-error`,
  `#store-error`, `#timeline`, `.log-line`, `.log-follow`, `.log-tail`, `.deploy-event`,
  `.event-stage`, `.event-status`). New names are additive.
- **The adblock-bait rule**, with two traps found while naming: **`badge`** and **`loading`**
  both contain the substring `ad`. This phase uses `pill` and `busy`. Every new id/class is
  checked against the banned list before it ships.
- **No store or API changes.** Deploy rows carry no timestamp; the history table therefore
  shows no "when" column this phase — recorded below as the one thing the UI wants from the
  store later.

## The direction, named

**Operator console, Linear/Vercel school, dark-first.** Calm and dense; near-monochrome
neutrals tinted toward the accent hue; ONE accent used semantically; depth from luminance
steps and 1px low-opacity borders, never drop shadows in dark; machine values (names, images,
paths, ids, logs) in the monospace stack; hierarchy from weight and size, not boxes. The
explicit negations: no gradients, no emoji as icons, no centered card grids, no invented
spacing, no pure #000/#fff.

kuadrat means "squared" — the mark is a filled square (`▪` drawn as a CSS square, not an
emoji), and the radius family stays small and square-ish: 4px controls, 6px cards, 0 on the
log console.

## The system (the tokens the stylesheet defines)

- **Type.** System UI stack (an operator tool on someone's own host has no business shipping a
  webfont) + `ui-monospace` for machine values. Ramp at 1.125 from a 14px functional body:
  12 / 13 / 14 / 16 / 18 / 21 — six sizes, two weights (400/600), `tabular-nums` on anything
  columnar. Line-height 1.5 body, 1.25 headings.
- **Spacing.** 4 / 8 / 12 / 16 / 24 / 32 / 48 — nothing off-scale. Rhythm rule: within-group
  < between-group < between-section (8 / 16–24 / 48).
- **Color.** Dark-first. Primitive ramp: ~9 graphite steps tinted toward the accent hue; light
  theme is the remap, not the source. Semantic tier only in components: `--surface` /
  `--surface-raised` / `--surface-sunken`, `--text-primary` / `--text-secondary` /
  `--text-tertiary`, `--border-subtle` / `--border-strong`, `--accent` (+ `-emphasis`).
  Accent: **teal-cyan** (phosphor heritage; visibly not the red/green/amber the states own).
  State colors: green Running/Done, red Failed, amber RolledBack/attention, gray
  Stopped/absent — always paired with the word, never color alone. Dark accents desaturated
  ~20%; contrast held at 4.5:1 text / 3:1 UI in both themes.
- **Focus.** 2px ring, 2px offset, accent at ≥3:1, `:focus-visible` only.
- **Motion.** 150ms ease-out on hover/pressed; new live rows (SSE) enter with an
  opacity-only fade; `prefers-reduced-motion` collapses all of it to instant.

## The pages

- **Shell.** Slim top bar: square mark + "kuadrat" home link, and the daemon's version
  (`CARGO_PKG_VERSION`) right-aligned as tertiary text. One `<h1>` per page. `<main>`
  max-width 1200px. A skip-to-content link for keyboards.
- **Index.** The fleet at a glance: the apps table with status **pills** (dot + word), route
  as a link-styled chip, repo paths in mono secondary, whole-row hover, name links. Empty
  state designed: the square mark, "No apps yet", one line of how, and the form. The
  registration form lives in a `<details>` panel ("Register an app") — progressive
  disclosure; open by default when the list is empty, closed when it is not.
- **App detail.** A header block: name (h1) + status pill on one line, repo/route/image as a
  labeled fact grid (mono values), Redeploy as the one primary button. Deploy history: table
  with stage text and status pills, id links mono. Log: a **console card** — dark surface in
  both themes (a terminal is dark), mono 13px, max-height with its own scroll, the
  Follow/Stop-following control styled as a toggle in the card's header row.
- **Deploy detail.** The timeline as a real timeline: each event a row with a status marker
  (square, per-status color), stage name emphasized, detail as secondary text; the list is an
  `aria-live="polite"` region while live, with a small "live" pill in the header that exists
  only while streaming.
- **Errors and edges.** 404 / store-unavailable / unreadable-journal pages get the same shell
  and designed, plain-language bodies. `#log-empty` ("No output yet.") renders inside the
  console card, muted — an empty console is still a console.

## Verification

- The existing suite stays green unchanged — the ids/classes above are load-bearing.
- New structural assertions only where behavior changed markup (the `details` form panel, the
  timeline markers, the console card).
- **The screenshot loop is mandatory**: a `--root` daemon on sumo seeded with registered apps
  (and failed deploy attempts, which need no podman), screenshotted with Playwright at 375px
  and 1280px, dark and light, and critiqued against this document — alignment, spacing rhythm,
  contrast, states — before the phase closes. The UI is not done from code alone.
- Contrast spot-checks on the final palette values (4.5:1 / 3:1) in both themes.

## Later, not now

- A deploy timestamp in `DeployRow` (the history table wants a "when" column).
- A favicon (nice; not the phase).
- Auth, and anything it unlocks — unchanged trigger.
