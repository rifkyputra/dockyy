# Phase 6 · Push-to-Deploy and Scheduled Tasks

Two of the items the README's scope deliberately left out of v1 — "push-to-deploy webhooks" and
scheduled tasks — promoted by Rifky on 2026-08-18 after a feature-parity read against Coolify.
They are one phase because they share a spine: both make kuadrat act **without an operator
present**, and both must therefore fail legibly into the store's event timeline rather than into
a terminal nobody is watching.

## Goal

1. **Push-to-deploy:** a `git push` to GitHub or GitLab redeploys the app — the forge calls a
   webhook, kuadrat verifies it, updates the host repo to the pushed commit, and runs the same
   deploy pipeline the button and the CLI run.
2. **Scheduled tasks:** a spec can declare commands that run on a schedule in a container from
   the app's image — Coolify's "cron jobs", done the systemd-native way: a Quadlet oneshot
   container plus a systemd timer.

## The decisions this group makes

| Decision | Choice | Why |
|---|---|---|
| Where hooks land | **Routes on the existing loopback daemon** (`POST /hooks/github/:app`, `/hooks/gitlab/:app`); reaching them from the internet is the operator's ingress (a Caddy route, a tunnel) and is documented, not automated | The daemon stays loopback-only, so binding is unchanged. Caddy is already the TLS-fronting ingress for every app; one more `reverse_proxy` block is operator config, not new machinery |
| Hook authentication | **A shared secret** from `KUADRAT_HOOK_SECRET(_FILE)`, verified per provider: GitHub `X-Hub-Signature-256` (HMAC-SHA256 over the raw body), GitLab `X-Gitlab-Token` (equality). Both compared constant-time. **No secret configured = the routes answer 404** | The forge signs what it sends; verifying that signature IS the authentication for this surface, and it involves no session, no cookie, no browser — so the CSRF trigger in `known-gaps.md` is not tripped. Absent configuration means off, the same contract as the outbound webhook |
| The crypto | **Two new daemon-only dependencies: `sha2` + `hmac`** (RustCrypto, pure Rust) | The first genuine crypto need in the repo, and `std` has none. The shell-out alternative (`openssl dgst -hmac <secret>`) puts the secret on argv — readable in any process listing, the exact leak the `_FILE` pattern exists to prevent. Hand-rolling SHA-256 is the one kind of hand-rolling this repo does not do |
| App ↔ repo mapping | **The URL names the app** (`/hooks/github/:app`); the payload's branch must match the host repo's checked-out branch | Matching payload repo URLs against `git remote` output is normalization guesswork. The operator who configures the webhook types the app name into the forge's webhook URL once; the branch check is what stops a push to `dev` redeploying `main` |
| Updating the repo | **`git fetch origin` + `git reset --hard <sha>`** through the `Executor` seam, in `crates/daemon` | The payload names the exact commit; deploying anything else would lie. `reset --hard` handles force-pushes; a push-to-deploy working copy is a deployment surface, not a workspace, and the docs say so. Lives in the daemon because fetching is network I/O and `crates/daemon` is the only networked code |
| Concurrent pushes | **A push during an in-flight deploy is answered 200 with `{"ignored": "…"}` and an event in the timeline; no queue** | GitHub disables hooks that keep failing, so non-2xx is not a signalling channel. A queue is real machinery for a case the operator resolves by pushing again (or redelivering); recorded as a known gap |
| Task shape | **`tasks` on the spec**: `[{name, schedule, command}]`, `schedule` a systemd `OnCalendar` expression | The spec is the source of truth; tasks are part of the workload's definition and travel with `kuadrat.json`. `OnCalendar` because kuadrat is systemd-native — inventing a cron dialect to translate would be a second scheduler grammar |
| Task execution | **A fresh container from the app's image per run** (`kuadrat-<slug>-task-<task>.container`, `Type=oneshot`), never `exec` into the running app | A fresh container works whether the app is up, crashed, or mid-deploy; it gets the same image, env, and secrets, and its exit code is the unit's. `exec` couples a task's fate to the app's PID 1 |
| Where timers live | **`.container` in the Quadlet dir, `.timer` in a new `Paths.systemd_dir`** (`/etc/systemd/system`, rerooted like everything else) | Quadlet only processes its own extensions; systemd does not read `.timer` from the Quadlet dir. Both files carry the managed marker and the `kuadrat-` prefix, so ownership and cleanup rules are unchanged |
| Schedule validation | **`systemd-analyze calendar <expr>` at apply time**, through the seam | A malformed `OnCalendar` does not fail `systemctl enable` — the timer just never fires, silently. The preflight turns "silently never" into an error naming the field |

## Push-to-deploy — the flow

```
forge POST /hooks/github/:app
  → secret configured?            no  → 404
  → signature valid?              no  → 401 (constant-time compare)
  → app registered?               no  → 404
  → payload branch == repo HEAD?  no  → 200 {"ignored": "push to <ref>, deploying <branch>"}
  → app already deploying?        yes → 200 {"ignored": "deploy in progress"}
  → git fetch origin && git reset --hard <after-sha>   (Executor; failure → 500, no deploy)
  → the same reserve-and-spawn the API deploy handler runs
  → 200 {"deploy_id": N}
```

- GitHub payload: branch from `ref` (`refs/heads/<branch>`), commit from `after`. GitLab: same
  two fields. A zero SHA (branch deletion) is ignored with a reason.
- The deploy handler's guts (busy-check → spec load → validate → reserve → spawn) are extracted
  into one function both routes call, so the hook cannot drift from the button.
- The GitLab token comparison and the GitHub signature comparison both go through HMAC before
  comparing (compare `HMAC(k, a) == HMAC(k, b)`), which makes equality constant-time without a
  dedicated constant-time-compare dependency.
- **Exposure documentation, not automation:** the README shows the Caddy block
  (`handle /hooks/* { reverse_proxy 127.0.0.1:7457 }`) and names the secret env pair. kuadrat
  rendering its own ingress route for hooks is future work that would need the auth
  conversation; a signed webhook endpoint does not.

## Scheduled tasks — the shape

In `kuadrat.json`:

```json
{
  "tasks": [
    { "name": "cleanup", "schedule": "daily", "command": ["sh", "-c", "rm -rf /tmp/cache/*"] }
  ]
}
```

Rendered per task, both files marked `# kuadrat-managed: true`:

- `<quadlet_dir>/kuadrat-<slug>-task-<task>.container` — the app's image, env, and secrets; the
  task's `Exec`; `[Service] Type=oneshot`; no ports, no route, no healthcheck.
- `<systemd_dir>/kuadrat-<slug>-task-<task>.timer` — `OnCalendar=<schedule>`,
  `Persistent=true`, `[Install] WantedBy=timers.target`.

Apply: validate schedules (`systemd-analyze calendar`), write both files, `daemon-reload`,
`enable --now` each timer, and **remove task units the spec no longer names** (scan for the
`kuadrat-<slug>-task-` prefix; the marker guards what may be deleted). Remove: stop and disable
timers, delete both files, `daemon-reload` — folded into the existing `remove`.

Validation added to `spec.validate()`: task names slug non-empty and unique per spec,
`schedule` and every command word single-line, `command` non-empty. Same injection reasoning as
every other field: a `\n` in a schedule is a directive nobody wrote.

## Error handling

- **Hook failures are events.** A failed fetch/reset or a refused deploy lands in the response
  *and* in the app's timeline where an operator will actually look; the forge's delivery log
  shows the same body.
- **Task failures are systemd's.** A failed run is a failed oneshot service — visible in
  `systemctl list-timers`, the unit's journal, and therefore `tail_logs`. kuadrat does not
  build a second result channel for them in this phase.
- **A missing secret is 404, not 500** — the surface does not exist until configured, and the
  response does not confirm the feature is merely unconfigured.

## Testing (worth pinning specifically)

- A GitHub delivery with a valid signature deploys; a tampered body or wrong secret is 401 and
  **no git command runs**.
- A GitLab delivery with the right token deploys; the wrong token is 401.
- No secret in the environment → 404, and **no signature work happens**.
- A push to a non-deployed branch is 200 + ignored, and no git command runs.
- The reset uses the payload's SHA, not `origin/HEAD`.
- A spec with tasks renders both units (golden tests); the timer lands in `systemd_dir`.
- Applying a spec with a removed task deletes that task's units; a foreign (unmarked) file with
  a colliding name is refused, not deleted.
- An invalid `OnCalendar` fails apply with an error naming the task, before any file is written.
- `remove` cleans up timers and task containers with the app.

## Not in this group

- **A hook delivery queue / debounce** — a push during a deploy is ignored with a reason
  (known gap; redeliver or push again).
- **Bitbucket / Gitea** — Gitea speaks the GitHub shape and may Just Work; neither is tested
  nor claimed.
- **kuadrat-rendered ingress for the hook routes** — documented Caddy config instead; revisit
  with authentication.
- **Per-app hook secrets** — one daemon, one secret; the URL scopes the blast radius to one
  app's redeploy.
- **Task output capture beyond the journal** — systemd owns the run; `tail_logs` reads it.
- **PR/preview deployments, DB backups, the fleet driver** — the other parity gaps, unchanged.
