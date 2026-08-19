# kuadrat Phase 7 · Web UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (this phase is
> one session with an empirical screenshot loop; subagent-per-task would lose the visual
> context between tasks). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The operator UI looks and feels like a real console — the design doc's
Linear/Vercel-school system — with zero architectural change: maud + htmx + one plain CSS
file, no build step, no new dependencies.

**Spec:** [`docs/design/2026-08-19-phase-7-web-ui.md`](../design/2026-08-19-phase-7-web-ui.md)
— the token system, page-by-page treatment, naming traps (`badge`/`loading` contain `ad`),
and the preserved id/class contract live there and are not repeated here.

## Global Constraints

- Existing suite stays green **unchanged**; every listed id/class keeps its name.
- `maud::PreEscaped` appears nowhere; every new id/class checked against the adblock list.
- `make check` clean; `PATH=$HOME/.cargo/bin:$PATH`.
- Baselines, to re-measure at start: cli 30, core 216, daemon 106, mcp 22 (per the card at
  `646b162`).
- Commit after every task, Conventional Commits.

---

### Task 1: The stylesheet — tokens and components
- [x] Rewrite `crates/daemon/assets/kuadrat.css` as the design doc's system: primitive ramp →
  semantic tokens (dark-first, light remap) → components (shell, table, pill, chip, fact
  grid, console card, timeline, form, button, focus ring, motion + reduced-motion).
- [x] Suite + `make check` (CSS is served by `assets.rs`; nothing should move).
- [x] Commit — `feat(ui): the design system stylesheet`

### Task 2: The shell
- [x] `layout()`: skip link, top bar (square mark + name link, version right as tertiary),
  `<main>`; `not_found` / `store_unavailable` bodies inherit the shell's tone.
- [x] Suite green; commit — `feat(ui): the shell`

### Task 3: Index
- [x] Status pills (dot + word), route chip, mono repo path, row hover; designed empty state;
  registration form inside `<details id="register-panel">`, `open` only when the list is
  empty. New test: the panel is `open` exactly when no apps exist.
- [x] Suite; commit — `feat(ui): the fleet index`

### Task 4: App detail
- [x] Header block (h1 + pill + Redeploy), fact grid, history table pills, console card
  (`.console`) wrapping tail/follow/empty/error states, follow toggle in the card header.
  New test: the log section renders inside `.console` in all four states.
- [x] Suite; commit — `feat(ui): the app page`

### Task 5: Deploy detail
- [x] Timeline markers per status, stage emphasized, detail secondary; `aria-live="polite"`
  and a "live" pill only while streaming. New test: `aria-live` present iff live.
- [x] Suite; commit — `feat(ui): the deploy timeline`

### Task 6: The screenshot loop (mandatory before close)
- [x] Seed: `kuadrat serve --root <tmp>` on a spare port; register 3 apps via the API (one
  routed, one long-named); trigger deploys that fail without podman to populate history and a
  timeline.
- [x] Playwright: 1280px and 375px, dark and light, all three pages + empty-store index.
- [x] Critique against the design doc (alignment, rhythm, contrast, hierarchy, states); fix;
  re-shoot until nothing on the list remains.
- [x] Commit per fix round — `fix(ui): <what the screenshots showed>`

### Task 7: Record what landed
- [x] README: replace the stale status blockquote (it still says "Phases 1 through 3", "no
  MCP surface yet") with the current truth; refresh the Web UI bullet.
- [x] Design addendum only if the loop forced deviations from the design doc.
- [x] Full gauntlet; tick this plan; commit — `docs: record the phase-7 UI`

## Completion checklist

> Closed 2026-08-19, verified on sumo. Measured: cli 30, core 216, daemon **109** (106 + the
> three structural tests), mcp 22 — **377 total, 0 failed**; `make check` clean. The screenshot
> loop ran against two seeded `--root` daemons (three apps incl. a routed and a long-named one,
> rolled-back deploys with real failure detail, plus an empty store) via the Playwright CLI at
> 1280px and 375px in both color schemes. It caught four real defects: tables overflowing the
> phone viewport (now they scroll inside themselves), the empty-state mark collapsing to
> nothing (inline span), and four WCAG contrast failures (tertiary text, console-muted, light
> button text, and input borders — inputs now carry a dedicated 3:1 `--border-input` per SC
> 1.4.11). One deviation: index and app page landed as one commit — they share the pill
> renderers that replaced `status_class`. And the mechanical adblock audit caught the auditor:
> `page-head` and `console-head` end in the banned `ad` substring (h-e-**a-d**) and shipped as
> `page-bar` / `console-top`.

- [x] Suite green, counts unchanged or additive; `make check` clean
- [x] Zero `PreEscaped`; zero adblock-bait names (audit every new class/id)
- [x] Both themes pass contrast spot-checks (4.5:1 text / 3:1 UI)
- [x] Screenshots at 375/1280 × dark/light reviewed for every page, including empty states
- [x] Keyboard: skip link works, focus visible everywhere, one h1 per page
- [x] `prefers-reduced-motion` honored
- [x] No new dependencies; no store/API changes
