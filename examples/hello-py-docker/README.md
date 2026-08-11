# hello-py-docker — the same app, on Docker

A Docker Compose version of [`../hello-py`](../hello-py), existing so the two
runtimes can be measured against each other. `app.py` is **byte-identical** to
the kuadrat example's (same md5), and the Dockerfile differs from the
Containerfile only in filename — any difference measured is the runtime's, not
the application's.

## Run it

```bash
cd examples/hello-py-docker
docker compose up -d --build
curl localhost:7655
```

Published on **7655** so it can run beside the kuadrat deployment on 7654. The
in-container port is 7654 in both.

```bash
docker compose down          # stop it
```

## Benchmark

With both running:

```bash
python3 examples/bench.py
```

## Results — 2026-08-11, Ubuntu 24.04, 2 cores, 7.6 GB RAM

Docker 29.1.3 (16 other containers already running) vs podman 4.9.3 + Quadlet.

### Memory — supervisor processes only, not the app

| | Docker | kuadrat |
|---|---|---|
| **Fixed** (any number of containers) | `dockerd` 88.8 MB + `containerd` 51.5 MB = **140.3 MB** | **0** — systemd is already running |
| **Marginal** (per container) | `containerd-shim` 10.4 + 2× `docker-proxy` 8.9 = **19.3 MB** | `conmon` **2.5 MB** |
| **10 containers** | **334 MB** | **25 MB** |

The per-container ratio is **7.8×**, and Docker additionally pays 140 MB before
the first container starts. On a 1 GB VPS that fixed cost alone is 14% of the
machine.

Two details behind the marginal number: Docker starts **two** `docker-proxy`
processes per published port (IPv4 and IPv6), and the shim is per container.
Quadlet's `conmon` is the only per-container process — there is no daemon
because systemd, which is running regardless, is the supervisor.

### Cold start

| | |
|---|---|
| Docker restart → first 200 | **70 ms** |
| kuadrat restart → first 200 | not measured (needs root) |

### Throughput — and why there is no result here

Both runtimes served ~300–350 req/s at 16 concurrent, and **the difference was
noise**. One run showed kuadrat ahead (529 vs 382 req/s); it did not reproduce.
Bypassing `docker-proxy` by hitting the container IP directly was *no faster*
than going through it (307 vs 320 req/s), which rules out the obvious
explanation for the first run's gap.

This is the expected outcome. Both runtimes start the same process under the
same kernel; once the container is running, neither is in the data path in a
way that shows up at this scale. The load generator is Python, shares two cores
with both workloads, and is the likely bottleneck.

**Memory is the real difference. Throughput is not.** Anyone quoting a
throughput win for either runtime from this benchmark is quoting noise.

## Caveats worth reading before citing these numbers

- Memory is **RSS, not PSS** — `smaps_rollup` on root-owned processes is
  unreadable to a normal user. RSS double-counts shared pages, which inflates
  the multi-process side (Docker). Rerun with `sudo` for exact figures; the
  direction is not in doubt but the ratio may be smaller.
- The host already ran ~16 Docker containers, so `dockerd`'s working set is
  larger than a clean host's. The **marginal** figures are unaffected — they are
  matched to this container by id and published port, not by process name.
- Two cores. Concurrency results on a larger host will differ.
- `mem_limit` in compose and `MemoryMax` in the kuadrat spec are both 128 MB, so
  neither container had more headroom than the other.
