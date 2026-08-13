# kuadrat

**Podman Quadlet deployment daemon for a single host.**

Take a git repo to a running service with TLS — on systemd, without a container daemon.

> Status: **pre-alpha**. Phases 1 through 3 are merged — the CLI can build, deploy, route, hold
> secrets, roll back, and recover from a crash, and `kuadrat serve` runs a daemon with a web UI for
> status, live deploy progress, logs, and an optional webhook on terminal outcomes and stage
> failures. `packaging/kuadrat.service` runs it under systemd. The daemon binds loopback only, with
> no authentication — reach it remotely over an SSH tunnel or a VPN. There is no MCP surface yet.
> See the [Guide](#guide) to use it today.

## Why

Deploying an app to one Linux server means choosing between a Kubernetes-shaped stack that
costs more RAM than the app, or a Docker-based PaaS that runs a daemon and a second supervisor
alongside systemd.

Podman Quadlet removes the daemon — containers become native systemd units — but nothing wraps
it in a deployment loop. There is no "repo to running service with TLS" path on Quadlet, no
single place to see status and logs, and no agent-operable interface.

kuadrat is that layer. systemd stays the supervisor; kuadrat is the thing that puts workloads
in front of it.

### Measured

The daemon claim above is testable, so [`examples/`](examples/) ships the same app twice — once
as a kuadrat deployment, once as a Docker Compose service, with a **byte-identical** `app.py` and
base image — plus [`examples/bench.py`](examples/bench.py) to measure both.

Supervisor processes only, not the app itself. Ubuntu 24.04, 2 cores, Docker 29.1.3 vs
Podman 4.9.3:

| | Docker | kuadrat |
|---|---|---|
| **Fixed** — before any container exists | `dockerd` 88.8 MB + `containerd` 51.5 MB = **140.3 MB** | **0** — systemd is already running |
| **Marginal** — per container | `containerd-shim` 10.4 + 2× `docker-proxy` 8.9 = **19.3 MB** | `conmon` **2.5 MB** |
| **Ten containers** | **334 MB** | **25 MB** |

**7.8× per container**, and Docker pays the 140 MB before the first one starts. On a 1 GB VPS
that fixed cost alone is 14% of the machine. Docker runs *two* `docker-proxy` processes per
published port — one per address family — while `conmon` is the only per-container process on the
Quadlet side, because systemd is the supervisor and it is running regardless.

**Throughput showed no difference, and that is the honest result.** One run suggested kuadrat was
faster; it did not reproduce, and bypassing `docker-proxy` by hitting the container IP directly was
no faster than going through it — ruling out the obvious explanation. Both runtimes start the same
process under the same kernel. Anyone quoting a throughput win here is quoting noise.

Two caveats before citing these figures: memory is **RSS, not PSS** (`smaps_rollup` is unreadable
for root-owned processes without `sudo`), which double-counts shared pages and therefore *inflates*
the multi-process side — the direction is not in doubt but the ratio would shrink. And the host was
already running ~16 other Docker containers, so `dockerd`'s working set is fatter than a clean
host's; the marginal figures are matched by container id and port and are unaffected. Full method
and caveats in [`examples/hello-py-docker/README.md`](examples/hello-py-docker/README.md).

## What it does

```
kuadrat deploy pbrain
  Detect → Build → Secrets → Apply → Route → Healthcheck → Done
                                                   └── failure → RolledBack
```

- **Deploy loop** — detect stack, build, render a `.container` unit, route through Caddy with
  automatic TLS, healthcheck, roll back on failure
- **Secrets** — `podman secret` management; specs carry names, never values
- **Logs** — journald reads scoped to a unit, streamable live as JSON for an API client
- **Web UI** — status, live deploy progress, logs, live log following
- **MCP surface** — an agent can diagnose failures, deploy, and author config. Advisory only:
  it proposes, a human approves
- **Events** — typed and subscribable; kuadrat emits, subscribers deliver

## Guide

Everything below works against the current code. Commands that write units, secrets, or Caddy
fragments touch `/etc` and `/var/lib`, so they need **root**.

### Install

```bash
git clone git@github.com:rifkyputra/kuadrat.git && cd kuadrat
cargo build --release          # binary at target/release/kuadrat
sudo install -m755 target/release/kuadrat /usr/local/bin/
```

Needs Podman 4.4+, systemd on cgroups v2, and `git`. Caddy is only required if you route an app
(see [Routing](#routing-an-app)).

### Your first deploy

An app is a **local repo with a Containerfile** (or Dockerfile) plus a `kuadrat.json` describing
how it should run. kuadrat never clones — you or CI put the code on the host.

```jsonc
// ~/apps/worker/kuadrat.json
{
  "name": "worker",              // overwritten by the app argument; keep them the same
  "image": "",                   // ignored on deploy — the build fills it in
  "command": null,
  "env": [["LOG_LEVEL", "info"]],
  "ports": [],
  "volumes": [],
  "secrets": [],
  "memory_max": "256M",
  "health_cmd": null,
  "restart_policy": "Always",
  "route": null
}
```

```bash
sudo kuadrat deploy worker ~/apps/worker
```

That runs Detect → Build → Secrets → Apply → Route → Healthcheck. It prints the outcome and
**exits non-zero on anything but `Done`**, so CI can gate on it. With `route: null` the Route stage
is a no-op and Caddy is never called; the healthcheck falls back to `systemctl is-active`.

Then:

```bash
kuadrat list                      # kuadrat-managed workloads
kuadrat status worker             # Running / Stopped / Failed / Not installed / Unknown
journalctl -u kuadrat-worker -f   # logs — it's a normal systemd unit
sudo kuadrat remove worker
```

Every artefact carries the `kuadrat-` prefix, so `worker` becomes the unit `kuadrat-worker` and
can never collide with a hand-written `worker.container` or the host's own `worker.service`.

**Where the spec comes from**, in order: `kuadrat.json` in the repo → the spec stored from the app's
last deploy → error. The `app` argument always wins over the spec's `name` field, and `--route`
always wins over the spec's `route`. So a redeploy after an edit is just `kuadrat deploy worker
~/apps/worker` again; the image is rebuilt and tagged `localhost/kuadrat-worker:<git-sha>`.

### Routing an app

A route is a domain reverse-proxied to a container port, served by Caddy with automatic TLS.

```bash
sudo kuadrat deploy web ~/apps/web --route example.com:3000
```

Two prerequisites, both one-time:

1. **Caddy is installed and running.** kuadrat writes `/etc/caddy/kuadrat.d/<slug>.caddy` and runs
   `systemctl reload caddy`; it does not manage Caddy's lifecycle.
2. **Your Caddyfile imports the fragments** — add `import kuadrat.d/*.caddy`. Without that line the
   fragment lands on disk and serves nothing.

A routed spec **must** set `health_cmd`; `validate()` rejects a route without one. Public traffic
must not reach something with no readiness signal:

```jsonc
"health_cmd": "curl -fsS http://localhost:3000/health",
"route": { "domain": "example.com", "port": 3000 }
```

### Secrets

Specs carry secret **names**; values live in `podman secret` and never appear in a spec, a unit
file, or argv. Values are read from stdin only — argv is world-readable via `ps`.

```bash
printf '%s' "$TOKEN" | sudo kuadrat secret set api-token
kuadrat secret ls
sudo kuadrat secret rm api-token
```

Reference it by name in the spec (`"secrets": ["api-token"]`) and the Secrets stage fails the
deploy up front if a named secret is missing — so a deploy fails safe rather than serving
half-configured.

### When a deploy fails

Failure triggers compensation in reverse from the stage that failed, and the outcome is
`RolledBack`. A failure at Detect, Build, or Secrets touched nothing on the host — the old version
is still serving. A failure at Apply re-applies the previously deployed spec, or removes the unit
if this was the app's first deploy. A failure at Route or Healthcheck unwinds the route first, then
the unit. The outcome is `Failed` only when compensation *itself* fails — that one wants a look.

If the host dies mid-deploy, the app is left locked and `in_progress` in the store. Recover it:

```bash
sudo kuadrat reconcile
```

It rolls back anything still in flight and releases the lock. Idempotent — safe to run on every
boot, and the natural thing to wire into a `kuadrat-reconcile.service` with
`After=network-online.target`.

### Where things live

| Path | What | Override |
|---|---|---|
| `/etc/containers/systemd/kuadrat-<slug>.container` | the generated Quadlet unit | `--root <dir>` |
| `/var/lib/kuadrat/kuadrat.db` | SQLite: specs, deploy history, stage, locks, events | `--root <dir>` |
| `/etc/caddy/kuadrat.d/<slug>.caddy` | the Caddy fragment | `--root <dir>` |

`--root <dir>` relocates all three under one directory — for dry runs and testing without touching
the real host. kuadrat only ever overwrites files it owns: units carry a `# kuadrat-managed: true`
marker, and a foreign file at a target path is refused, never clobbered.

### Spec reference

| Field | Type | Notes |
|---|---|---|
| `name` | string | overwritten by the `app` argument on deploy |
| `image` | string | ignored on deploy (the build sets it); used by `kuadrat apply` |
| `command` | string[] \| null | argv; each element is one argument |
| `env` | [string, string][] | rendered as `Environment=` |
| `ports` | string[] | `"host:container"` |
| `volumes` | string[] | `"host:container"` |
| `secrets` | string[] | podman secret **names** only |
| `memory_max` | string \| null | e.g. `"256M"` |
| `health_cmd` | string \| null | **required** when `route` is set |
| `restart_policy` | `"Always"` \| `"OnFailure"` \| `"No"` | |
| `route` | `{domain, port}` \| null | needs Caddy |

Newlines and carriage returns are rejected in every rendered field — a `\n` in an env value would
otherwise inject directives (`Secret=`, `User=`) nobody wrote. `%` is escaped so systemd does not
expand it as a specifier.

### Commands

| Command | What |
|---|---|
| `deploy <app> <path> [--route domain:port]` | the full loop: build from a local repo and run it |
| `build <path>` | build and tag the image only; prints the reference |
| `apply <file.json>` | apply a spec directly — no build, no route |
| `remove <name>` / `status <name>` / `list` | manage applied workloads |
| `secret set\|ls\|rm <name>` | podman secrets; values via stdin |
| `reconcile` | roll back deploys left in flight by a crash |
| `serve [--listen addr]` | run the HTTP daemon: API, web UI, event stream. Loopback only |

### The webhook

`kuadrat serve` can POST a JSON message to a webhook whenever a deploy reaches a terminal outcome
(`Done`, `RolledBack`, `Failed`) or a stage fails — not every event, just the ones worth a line in
chat.

Configure it with one of:

| Variable | Value |
|---|---|
| `KUADRAT_WEBHOOK_URL` | the URL directly |
| `KUADRAT_WEBHOOK_URL_FILE` | path to a file containing the URL |

A webhook URL carries its token in its path, so prefer `KUADRAT_WEBHOOK_URL_FILE`: a systemd
`Environment=` line is readable by anyone who can run `systemctl show`, but a file loaded through
`LoadCredential=`, or via `EnvironmentFile=` as in `packaging/kuadrat.service`, is not. Neither
variable, nor the URL itself, ever reaches argv — see `crates/daemon/src/webhook.rs`.

## Design principles

- **systemd is the orchestrator.** kuadrat does not supervise, restart, or schedule — systemd
  already does. Adding a second supervisor is the problem, not the solution.
- **The spec is the source of truth.** Unit files are derived artifacts kuadrat owns and may
  overwrite. Rollback is re-applying a previous spec, not diffing files.
- **The core never touches the network.** `kuadrat-core` manipulates the local filesystem,
  systemd, and podman, and knows nothing about hosts. Everything network-facing lives in the
  daemon. See [ADR-0002](docs/adr/0002-transport-agnostic-core.md).
- **Emit events, don't deliver them.** No notification providers, no chat integrations.
  Dedupe and delivery belong to the subscriber.

## Scope

**v1:** deploy loop, secrets, logs, web UI, MCP surface, event stream.

**Not in v1:** multi-host orchestration, blue/green deploys, push-to-deploy webhooks,
notification delivery, metrics, backups, autonomous agent action.

## Requirements

**Podman 4.4+** (when Quadlet landed), systemd with **cgroups v2**, and Caddy. Validated on
Podman 4.9.3 / Ubuntu 24.04. Podman 6 removed cgroups v1, CNI, `slirp4netns`, and BoltDB — kuadrat
targets the modern stack, so hosts still on cgroups v1 defaults are unsupported.

The daemon binds loopback only (`kuadrat serve --listen 127.0.0.1:7457` by default) — reaching the
UI from elsewhere is the operator's job via SSH tunnel or VPN.

## Documentation

| Document | What |
|---|---|
| [Phase 1 design](docs/design/2026-08-10-design.md) | Architecture, components, data flow, error handling, testing |
| [Phase 2 design](docs/design/2026-08-10-phase-2-deploy-loop.md) | The deploy loop: state machine, gateway, secrets, store, events |
| [Phase 3 design](docs/design/2026-08-11-phase-3-daemon-and-surfaces.md) | The daemon: HTTP API, SSE, htmx UI, logs, webhook |
| [Phase 4 design](docs/design/2026-08-11-phase-4-live-logs.md) | The streaming seam and live log tailing |
| [Examples](examples/) | A runnable app, its Docker equivalent, and the runtime benchmark |
| [Plans](docs/plans/) | Per-gate implementation plans, task by task |
| [Known gaps](docs/known-gaps.md) | Deferred findings, acceptance records, what to re-read before which phase |
| [ADRs](docs/adr/) | Decisions and their reasoning |

## Prior art

[quadit](https://crates.io/crates/quadit) (GitOps for Quadlet),
[quadletman](https://github.com/mikkovihonen/quadletman) (web UI),
[podlet](https://github.com/containers/podlet) (one-shot compose conversion),
Cockpit (Quadlet management in a web console), and the Docker-only PaaS family
(Dokku, Kamal, Coolify, CapRover).

The unoccupied combination is deploy loop + gateway/TLS + agent surface on Quadlet.

## License

Apache-2.0
