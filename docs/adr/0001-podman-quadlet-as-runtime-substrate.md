# ADR-0001: Podman Quadlet as the runtime substrate

- **Date:** 2026-08-10
- **Status:** Accepted

## Context

kuadrat deploys containerized workloads to a single Linux host. The runtime substrate — what
actually runs and supervises containers — is the decision every other part of the system rests
on, and it is expensive to reverse.

Three candidates:

**Docker (dockerd + containerd).** The default. Every PaaS in this space (Dokku, Kamal,
Coolify, CapRover) is built on it, so the ecosystem and documentation are unmatched.

**containerd + nerdctl.** The runtime Kubernetes nodes use. Lighter than Docker, and matching a
cluster's runtime is valuable when debugging nodes.

**Podman Quadlet.** Containers declared as systemd units, generated at boot by a systemd
generator. No long-running daemon.

Measured on a representative host (Ubuntu 24.04, 15 running containers): Docker's engine
occupied ~327 MB RSS — `dockerd` 63 MB, `containerd` 48 MB, shims 176 MB, `docker-proxy` 40 MB.

## Decision

Use **Podman Quadlet** as the runtime substrate, with **systemd as the supervisor**.

## Consequences

**What this buys**

- **One supervisor, not two.** A container daemon supervises, restarts, tracks health, and
  orders startup — duplicating systemd and occasionally disagreeing with it during boot and
  shutdown. Quadlet collapses them: workloads are ordinary units, and `After=`/`Requires=`
  order them against host services, which Compose cannot do.
- **The daemon's memory goes away.** ~111 MB of `dockerd` + `containerd` on the measured host.
  Real on a small VPS, though not the primary reason — the per-container supervisor cost is
  comparable either way.
- **Native systemd facilities** — `systemctl status`, `journalctl -u`, `MemoryMax=`,
  `Restart=`, boot ordering — with no reimplementation.
- **`podman auto-update`** already does pull → restart → healthcheck → automatic rollback.
  The hardest part of a deployment tool ships with the runtime.
- **Rootless is a first-class path**, not a bolt-on.

**What this costs**

- **A smaller ecosystem.** Testcontainers, CI runners, and most tutorials assume a Docker
  socket. Fewer people have hit any given bug.
- **Verbosity.** A five-service app is five unit files plus `After=`/`Requires=` wiring rather
  than one `compose.yml`. kuadrat exists partly to hide this.
- **Rootless networking has a real cost.** `pasta` is userspace, so throughput-heavy workloads
  pay CPU that rootful bridge networking does not. Rootful is the default for network-heavy
  services.
- **Podman 6 removed cgroups v1, CNI, `slirp4netns`, iptables, and BoltDB.** kuadrat requires
  cgroups v2 and netavark. Hosts on RHEL 8 defaults are unsupported.

**Rejected alternatives**

*Docker* — the ecosystem advantage is real, but it forces the second-supervisor problem kuadrat
exists to avoid, and every competitor already occupies that ground.

*containerd + nerdctl* — its advantage is matching a Kubernetes cluster's runtime. With no
cluster, that advantage disappears while the operational complexity (containerd + buildkitd +
CNI, configured separately) remains.
