# Phase 3 · H7 — Serve

The group that closes phase 3: the webhook sender, `kuadrat serve` and its systemd unit, `kuadrat
deploy` as a client of the daemon, and the acceptance script that exercises the whole surface on a
real host.

Parent design: [`2026-08-11-phase-3-daemon-and-surfaces.md`](./2026-08-11-phase-3-daemon-and-surfaces.md)
— §"The webhook", §"Configuration", and the two open questions it defers to this group.
Sibling: [`2026-08-11-phase-3-h6-pages.md`](./2026-08-11-phase-3-h6-pages.md).

## Goal

An operator installs the unit, starts the service, and deploys from either the CLI or the browser.
When a deploy ends — or a stage fails — a message arrives in their chat. `scripts/serve-acceptance.sh`
proves it on a real host rather than in a test harness.

## The four decisions this group makes

| Decision | Choice | Why |
|---|---|---|
| `kuadrat deploy` and the daemon | **Use the daemon when it answers; run in-process when it does not** | The five existing acceptance scripts call the CLI with no daemon running. API-only would break all of them and stop `kuadrat` being a standalone tool |
| systemd unit | **Plain `Type=simple`, no socket activation** | Socket activation moves the listen address into the `.socket` file, so `Config::validate`'s loopback guard — a security boundary with eight tests — would stop being what decides it |
| HTTP client | **`curl` through the `Executor` seam** | No new dependency, and it is how this codebase already reaches the host. Testable through `FakeExecutor` with no fake HTTP server |
| Webhook payload | **A readable `text` plus structured fields beside it** | Slack, Mattermost and Rocket.Chat render `text` and ignore the rest; a script or a generic receiver uses the structured fields. One shape serves both |

### Why the fallback, and what it costs

The daemon path and the in-process path are not identical, and the difference should be stated
rather than discovered: the **global one-at-a-time semaphore only exists in the daemon**. Two
concurrent `kuadrat deploy` invocations with no daemon running are not serialised by it.

That is acceptable because the semaphore was never the correctness mechanism. The store's per-app
lock is, and it applies on both paths — the parent design says so explicitly ("the semaphore is a
resource policy, not a correctness mechanism, and removing it must not make anything unsafe"). What
the local path loses is the RAM protection, not safety.

### Why not socket activation

The parent design left this open, weighing zero-RAM idling against the reconcile-before-bind
ordering. The ordering turns out not to be the problem: a connection would wait in the kernel accept
queue while `reconcile` runs, which is exactly the property we want.

The problem is elsewhere. With socket activation the listen address lives in the `.socket` unit, so
`Config::validate` — which refuses any non-loopback address and has eight tests pinning it — stops
being what decides where the daemon is reachable. The parent design put that guard in code
deliberately, "enforced in code rather than documented as advice". Socket activation would undo
that for a few megabytes.

Recorded as a known gap rather than a closed door: if RAM pressure on a real host ever justifies it,
the same guard has to be re-established some other way first.

## The webhook

It lives in **`crates/daemon`**, not `core`. `crates/daemon/src/lib.rs` opens with "The daemon: the
only networked code in kuadrat", and that sentence should stay true.

It subscribes to the broadcast hub and does its work in its own task — the shape H1 anticipated when
it made `EventSink::emit` synchronous and infallible: "a sink needing async work — the H7 webhook —
subscribes to the broadcast channel and does that work in its own task."

**What is sent.** Every `EventKind::Finished`, and every `EventKind::Stage` whose status is `Failed`.
That is one to three POSTs per deploy, matching the parent design's estimate. Stage `Started` and
`Succeeded` events are not sent — the receiver wants warnings, not a trace.

**The payload.**

```json
{
  "text": "kuadrat: web deploy #12 rolled back at apply — systemctl daemon-reload failed: ...",
  "app": "web",
  "deploy_id": 12,
  "stage": "apply",
  "status": "rolled_back",
  "detail": "systemctl daemon-reload failed: ..."
}
```

`stage` is the same projection the database and the JSON API use — `EventKind::columns()` — so a
deploy-level event spells it `"deploy"` here too, and the three surfaces cannot drift.

**Delivery is best-effort with a bounded retry: three attempts, one second apart, then log and
drop.** Fixed rather than exponential — the whole budget is three seconds, so a backoff curve would
be arithmetic without a decision behind it. Three seconds is also the ceiling on how long a
subscriber task lags behind the hub, which matters because a lagging subscriber is the failure mode
the broadcast channel reports and the stream has to recover from.

A webhook can never affect a deploy: `emit` returns nothing and cannot await, so the sink hands off
to a channel and the retry happens in the subscriber's task, where no deploy is waiting on it.

**The URL is a secret and is handled as one.** Read once at startup from `KUADRAT_WEBHOOK_URL` or
from a file named by `KUADRAT_WEBHOOK_URL_FILE`; never logged in full. It reaches `curl` through
`Executor::run_with_stdin` as a `curl --config -` document, **never as an argv element** — argv is
world-readable through `ps`. That seam already exists for exactly this reason: `secrets::set` pipes
a secret value to podman rather than passing it, with the same justification in its doc comment.

Absent configuration disables the sender. No URL is not an error, and the daemon must not warn about
it on every start.

## `kuadrat serve`

A new CLI subcommand wrapping the `serve()` the daemon already exposes:

```
kuadrat serve --listen 127.0.0.1:7457 --root /etc/containers/systemd
```

`--listen` defaults to `127.0.0.1:7457`; a non-loopback value is refused by the existing guard, with
the error that names the SSH tunnel. `--root` defaults to the real host paths.

`crates/daemon/examples/serve.rs` exists today as a development affordance and is **superseded by
this**; H7 removes it.

**`Config.socket` is removed.** The field exists, the parent design's configuration example shows a
`--socket` flag, and `serve()` ignores it entirely — it is dead. Loopback TCP plus an SSH tunnel or
`tailscale serve` covers remote access, so a unix socket would add a listener and a file-permission
question without closing a need that is still open. Removing it is smaller than wiring it, and the
parent design's example is corrected in the same change.

## `kuadrat deploy` against the daemon

`deploy` tries the daemon first and falls back:

1. `POST /api/apps/:name/deploy` with `Accept: application/json`, via `curl` through the `Executor`.
2. Connection refused, or no daemon configured → run in-process exactly as today.
3. Any other failure — a 404, a 409, a 400 — is the daemon's answer and is reported as such, **not**
   retried locally. A 409 means a deploy of that app is already running; starting a second one
   locally is the one thing that must not happen.

That third rule is the whole risk of a fallback design. "Cannot reach the daemon" and "the daemon
said no" have to stay distinguishable, or the fallback becomes a way to bypass the lock.

Which one happened is visible to the operator: the CLI says whether it ran the deploy itself or
handed it to the daemon, and prints the `/deploy/:id` URL in the second case.

`kuadrat status` and `kuadrat list` are **not** changed to prefer the daemon. The parent design
leaves that open and notes it is not needed for phase 3's criterion; reading the host directly stays
correct and simpler.

## The systemd unit

`packaging/kuadrat.service`, `Type=simple`, running `kuadrat serve`, `Restart=on-failure`, and
`After=network.target podman.socket`.

Hardening is deliberately minimal, because this service writes Quadlet units into
`/etc/containers/systemd`, runs `podman`, and calls `systemctl daemon-reload` — most of what a
hardening template would switch on breaks at least one of those. It ships with `NoNewPrivileges=yes`
and `ProtectHome=yes`, both of which are compatible with all three, and nothing else.

**A hardening directive that was never run is an assumption, not a protection**, so the acceptance
script must complete a real deploy with the unit as shipped. Anything that cannot be demonstrated
that way does not go in the file.

The unit is shipped and documented, not installed by the build.

## Acceptance

`scripts/serve-acceptance.sh`, alongside the five that exist. It starts the daemon, registers and
deploys a real repo over the socket, and asserts:

- the stream carried six stages
- the three pages render
- the deployed app answers
- the daemon **refuses** a non-loopback `--listen` — the parent design names this specifically, since
  an untested guard is an assumed one
- `kuadrat deploy` reaches the daemon when it is running, and runs locally when it is not
- a webhook POST is attempted on the terminal event, against a local receiver rather than a real
  chat service

## Error handling

| Condition | Behaviour |
|---|---|
| no webhook URL configured | sender disabled, silently |
| webhook POST fails | retried a bounded number of times, then logged and dropped |
| daemon unreachable from the CLI | fall back to in-process, and say so |
| daemon returns 4xx | report it; do not fall back |
| non-loopback `--listen` | refuse to start, naming the tunnel |

## Testing

Unit tests through `FakeExecutor` for the webhook: the payload shape, that the URL never appears in
argv, that a failure retries and then gives up, and that an absent URL sends nothing. The subscriber
task's event selection — `Finished` and failed stages only — is testable against a `FakeSink`-shaped
harness without a network.

The CLI's daemon-first path is tested the same way: a `FakeExecutor` scripting `curl` proves the
fallback triggers on connection refusal and does **not** trigger on a 409.

The rest is the acceptance script, because a systemd unit and a real deploy cannot be proven in a
unit test.

## Not in H7

- **Live log tailing** and the fleet driver — phase 4.
- **Authentication, and the CSRF defence** the form routes lack. Recorded in `known-gaps.md` with
  its trigger: fix it in the same change that adds auth or non-loopback reachability.
- **`kuadrat status`/`list` preferring the daemon** — left open as the parent design leaves it.
- **Socket activation** — recorded as a known gap, with the guard problem that has to be solved
  first.
