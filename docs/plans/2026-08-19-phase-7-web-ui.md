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
- [ ] Rewrite `crates/daemon/assets/kuadrat.css` as the design doc's system: primitive ramp →
  semantic tokens (dark-first, light remap) → components (shell, table, pill, chip, fact
  grid, console card, timeline, form, button, focus ring, motion + reduced-motion).
- [ ] Suite + `make check` (CSS is served by `assets.rs`; nothing should move).
- [ ] Commit — `feat(ui): the design system stylesheet`

### Task 2: The shell
- [ ] `layout()`: skip link, top bar (square mark + name link, version right as tertiary),
  `<main>`; `not_found` / `store_unavailable` bodies inherit the shell's tone.
- [ ] Suite green; commit — `feat(ui): the shell`

### Task 3: Index
- [ ] Status pills (dot + word), route chip, mono repo path, row hover; designed empty state;
  registration form inside `<details id="register-panel">`, `open` only when the list is
  empty. New test: the panel is `open` exactly when no apps exist.
- [ ] Suite; commit — `feat(ui): the fleet index`

### Task 4: App detail
- [ ] Header block (h1 + pill + Redeploy), fact grid, history table pills, console card
  (`.console`) wrapping tail/follow/empty/error states, follow toggle in the card header.
  New test: the log section renders inside `.console` in all four states.
- [ ] Suite; commit — `feat(ui): the app page`

### Task 5: Deploy detail
- [ ] Timeline markers per status, stage emphasized, detail secondary; `aria-live="polite"`
  and a "live" pill only while streaming. New test: `aria-live` present iff live.
- [ ] Suite; commit — `feat(ui): the deploy timeline`

### Task 6: The screenshot loop (mandatory before close)
- [ ] Seed: `kuadrat serve --root <tmp>` on a spare port; register 3 apps via the API (one
  routed, one long-named); trigger deploys that fail without podman to populate history and a
  timeline.
- [ ] Playwright: 1280px and 375px, dark and light, all three pages + empty-store index.
- [ ] Critique against the design doc (alignment, rhythm, contrast, hierarchy, states); fix;
  re-shoot until nothing on the list remains.
- [ ] Commit per fix round — `fix(ui): <what the screenshots showed>`

### Task 7: Record what landed
- [ ] README: replace the stale status blockquote (it still says "Phases 1 through 3", "no
  MCP surface yet") with the current truth; refresh the Web UI bullet.
- [ ] Design addendum only if the loop forced deviations from the design doc.
- [ ] Full gauntlet; tick this plan; commit — `docs: record the phase-7 UI`

## Completion checklist

- [ ] Suite green, counts unchanged or additive; `make check` clean
- [ ] Zero `PreEscaped`; zero adblock-bait names (audit every new class/id)
- [ ] Both themes pass contrast spot-checks (4.5:1 text / 3:1 UI)
- [ ] Screenshots at 375/1280 × dark/light reviewed for every page, including empty states
- [ ] Keyboard: skip link works, focus visible everywhere, one h1 per page
- [ ] `prefers-reduced-motion` honored
- [ ] No new dependencies; no store/API changes
