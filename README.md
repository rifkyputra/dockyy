# kuadrat

**Podman Quadlet deployment daemon for a single host.**

Take a git repo to a running service with TLS — on systemd, without a container daemon.

> Status: **pre-alpha**. Design and phase-1 plan are written; no code yet.

## Why

Deploying an app to one Linux server means choosing between a Kubernetes-shaped stack that
costs more RAM than the app, or a Docker-based PaaS that runs a daemon and a second supervisor
alongside systemd.

Podman Quadlet removes the daemon — containers become native systemd units — but nothing wraps
it in a deployment loop. There is no "repo to running service with TLS" path on Quadlet, no
single place to see status and logs, and no agent-operable interface.

kuadrat is that layer. systemd stays the supervisor; kuadrat is the thing that puts workloads
in front of it.

## What it does

```
kuadrat deploy pbrain
  Detect → Build → Secrets → Apply → Route → Healthcheck → Done
                                                   └── failure → RolledBack
```

- **Deploy loop** — detect stack, build, render a `.container` unit, route through Caddy with
  automatic TLS, healthcheck, roll back on failure
- **Secrets** — `podman secret` management; specs carry names, never values
- **Logs** — journald reads scoped to a unit
- **Web UI** — status, live deploy progress, logs
- **MCP surface** — an agent can diagnose failures, deploy, and author config. Advisory only:
  it proposes, a human approves
- **Events** — typed and subscribable; kuadrat emits, subscribers deliver

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

Podman 5+, systemd with cgroups v2, Caddy. The daemon binds a unix socket and loopback only —
reaching the UI from elsewhere is the operator's job via SSH tunnel or VPN.

## Documentation

| Document | What |
|---|---|
| [Design](docs/design/2026-08-10-design.md) | Architecture, components, data flow, error handling, testing |
| [Phase 1 plan](docs/plans/2026-08-10-phase-1-core-foundation.md) | Core foundation, task by task |
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
