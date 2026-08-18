---
name: kuadrat-operate
description: Use when operating a host whose workloads are managed by kuadrat (the Podman Quadlet deployment daemon) — a kuadrat-deployed service is down or failing, a deploy ended RolledBack or Failed, a spec/unit/secret/route needs changing, the host crashed mid-deploy, or the kuadrat daemon itself needs a health check. Also for questions about kuadrat's unit naming, file ownership, safe dry-run testing, or the MCP agent surface (`kuadrat mcp`, its six tools, why remove/secrets are absent).
---

# Operating a kuadrat host

## Overview

kuadrat deploys container workloads as Podman Quadlet units. **systemd is the only supervisor**, and **the spec is the source of truth — unit files are derived artifacts kuadrat owns and overwrites.** Fixing a workload means fixing its spec and redeploying, never editing unit files.

Anything that writes (`deploy`, `apply`, `remove`, `secret set|rm`, `reconcile`) needs **root**; `list` and `status` do not. `secret ls` queries the invoking user's podman store, so use `sudo` to see what root-level deploys use.

## Naming — get this right first

Workload `api` becomes:

- systemd unit **`kuadrat-api.service`** — never bare `api.service`
- Quadlet file `kuadrat-api.container`
- image `localhost/kuadrat-api:<git-sha>` — tagged by the build

The `kuadrat-` prefix is both the collision guard and the ownership signal. `<slug>` is derived from the name: lowercased, alphanumerics kept, spaces/`-`/`_` collapsed to single dashes, everything else dropped — `My_App v2` → `my-app-v2`.

## Where things live

| Path | What |
|---|---|
| `/etc/containers/systemd/kuadrat-<slug>.container` | generated Quadlet unit |
| `/var/lib/kuadrat/kuadrat.db` | SQLite: specs, deploy history, stage, locks, events |
| `/etc/caddy/kuadrat.d/<slug>.caddy` | Caddy route fragment |

`kuadrat --root <dir> <subcommand>` (the flag goes **before** the subcommand) reroots **all three** — the supported way to test without touching the live host. Not `QUADLET_UNIT_DIRS`, not user units. With `--root` set, `deploy` also runs locally instead of handing off to a running daemon. `--root` reroots *paths*, not process execution — a user-writable scratch dir needs no `sudo` for the file side, but stages that shell out to `systemctl`/`podman` still invoke the real binaries.

## Commands

| Command | What |
|---|---|
| `deploy <app> <path> [--route domain:port]` | full loop: Detect → Build → Secrets → Apply → Route → Healthcheck |
| `build <path>` | build + tag image only |
| `apply <file.json>` | apply a spec directly — no build, no route |
| `remove <name>` / `status <name>` / `list` | manage applied workloads |
| `secret set\|ls\|rm <name>` | podman secrets; values via **stdin** |
| `reconcile` | roll back deploys left in flight by a crash |
| `serve [--listen addr]` | the HTTP daemon (API, web UI, events) |
| `mcp [--listen addr]` | MCP over stdio for an agent; requires a running daemon |

## Diagnosing a down workload

1. `kuadrat status <name>` → Running / Stopped / Failed / Not installed / Unknown
2. `systemctl status kuadrat-<name>` and `journalctl -u kuadrat-<name> -e` — app stdout/stderr and systemd events, one stream
3. Deploy history and failed stage: `journalctl -u kuadrat.service -e`, or the web UI / `GET /api/apps/<name>` on the daemon
4. Routed apps: does `/etc/caddy/kuadrat.d/<slug>.caddy` exist, does the Caddyfile have `import kuadrat.d/*.caddy`, is `caddy` active? "Down" from outside is often the proxy, not the app.

## Changing a workload

Spec source, in order: **`kuadrat.json` in the app repo → spec stored from the last deploy → error.** kuadrat **never clones** — the repo already sits on the host (e.g. `~/apps/<app>`). There is no push-to-deploy and no polling: edit `kuadrat.json` (or `git pull` the repo), then rerun `sudo kuadrat deploy <app> <path>`.

- Spec `image` is **ignored on deploy** — the build tags from the checked-out commit. "Wrong image" means wrong checkout, not a spec field.
- The `<app>` argument overrides spec `name`; `--route` overrides spec `route`.
- A routed spec **must** set `health_cmd` — `validate()` rejects it otherwise.

Spec fields (`kuadrat.json`): `name` (string), `image` (string; only `apply` uses it), `command` (string[]|null, argv), `env` ([key,value] pairs), `ports` / `volumes` (string[], `"host:container"`), `secrets` (string[], podman secret **names**), `memory_max` (string|null, e.g. `"256M"`), `health_cmd` (string|null), `restart_policy` (`"Always"`|`"OnFailure"`|`"No"`), `route` (`{domain, port}`|null).

## Ownership — before touching any unit file

kuadrat-owned files carry a `# kuadrat-managed: true` marker plus the `kuadrat-` filename prefix. **The marker is the discriminator:** a marked file is kuadrat's and gets overwritten on the next deploy (hand-edits and behavior-changing drop-ins included — change the spec and redeploy instead); an unmarked file at a kuadrat target path is refused, never clobbered.

Runtime state is a different story from files: kuadrat-owned units are normal systemd units, so plain `systemctl start|stop|restart kuadrat-<slug>` and `journalctl -u kuadrat-<slug>` are fine.

## When a deploy fails

Failure compensates in reverse from the failed stage:

- **RolledBack** — auto-unwound; the previous version is still serving (or the unit was removed if this was the first deploy). Normal, not an emergency.
- **Failed** — compensation itself failed; the host may be inconsistent. No automated repair exists (`reconcile` only recovers *in-flight* deploys, not a terminal `Failed`): inspect the unit, the Caddy fragment, and the daemon journal by hand, then redeploy or `remove` once the cause is clear.
- Host died mid-deploy → app left locked/`in_progress` → `sudo kuadrat reconcile`. Idempotent, safe on every boot.
- The CLI exits non-zero on anything but `Done`, so scripts can gate on it. Deploy history lives in the daemon (web UI, `GET /api/apps/<name>`, `GET /api/deploys/<id>`) and its journal — there is no history CLI, and don't query `kuadrat.db` directly on a live host.

## Secrets

Specs carry secret **names** only; values live in `podman secret` and pass via stdin — never argv (world-readable in `ps`):

```bash
printf '%s' "$TOKEN" | sudo kuadrat secret set api-token
```

A missing named secret fails the deploy up front, before anything touches the host. Secrets resolve at container creation, so a changed value takes effect on the next `systemctl restart kuadrat-<slug>` or redeploy. To detach a secret from one workload, remove it from that spec's `secrets` and redeploy — `secret rm` deletes the value for every workload using it.

## The kuadrat daemon itself

`kuadrat.service` (`packaging/kuadrat.service` in the repo, installed to `/etc/systemd/system/`) runs `kuadrat serve --listen 127.0.0.1:7457` — **loopback only, no auth**; reach it remotely via SSH tunnel (`ssh -L 7457:127.0.0.1:7457 <host>`, then open `http://127.0.0.1:7457/`) or VPN, never by widening the bind. The web UI is at `http://127.0.0.1:7457/`. Health: `systemctl status kuadrat`, `journalctl -u kuadrat -e`, and `curl -fsS http://127.0.0.1:7457/api/apps` as a liveness probe (there is no `/healthz`). Webhook config goes through `KUADRAT_WEBHOOK_URL_FILE` via `EnvironmentFile=`, not `KUADRAT_WEBHOOK_URL` in an `Environment=` line — the URL carries its token and `systemctl show` exposes `Environment=` to anyone.

## The MCP surface (agent access)

`kuadrat mcp` is how an agent operates the host: the MCP client spawns it (stdio, not a daemon)
and gets **six tools** — `list_apps`, `get_app`, `deploy`, `get_deploy`, `tail_logs`,
`reconcile`. Register with `claude mcp add kuadrat -- kuadrat mcp`.

- **It refuses to start without a running `kuadrat serve`** — it talks to the daemon over
  loopback (`--listen addr` if the daemon isn't on the default `127.0.0.1:7457`), so no agent
  deploy can bypass the daemon's timeline. "MCP won't start" → check the daemon first. The
  `mcp` process itself needs no root — it is an HTTP client of the daemon.
- **`deploy` is async**: it returns a `deploy_id` immediately — poll `get_deploy` for the
  outcome; don't treat the fast return as `Done`.
- **Deliberately absent, not missing**: `remove` (the one irreversible op — a human runs it),
  the secret commands (values are stdin-only by construction; a JSON tool call can't provide
  that), and live log *following* (`tail_logs` is a bounded snapshot readable in one turn —
  follow live from a shell with `journalctl -u kuadrat-<slug> -f`).
- Tool parameter schemas are not duplicated here — MCP tools self-describe; the client sees
  the schemas at connect time.
- Speaks both current MCP clients (per-request versioning, `server/discover`) and older
  handshake-based ones (`initialize`).

## Common mistakes

| Mistake | Reality |
|---|---|
| `systemctl status api` | the unit is `kuadrat-api.service` |
| editing the `.container` file or adding drop-ins | derived artifact — reverted next deploy; edit the spec, redeploy |
| commit + push expecting a deploy | kuadrat never clones; run `kuadrat deploy` on the host |
| setting spec `image` to change the tag | ignored on deploy; the build tags from the git sha |
| `QUADLET_UNIT_DIRS` or user units for dry runs | use `--root <dir>` |
| curling the daemon from another machine | loopback only; SSH tunnel |
| treating `RolledBack` as the emergency | rollback worked; `Failed` is the one needing a human |
| debugging why MCP has no `remove`/secrets | omitted by design — those go through a human shell |
| `kuadrat mcp` exits at startup | it requires a running `kuadrat serve`; start the daemon |
