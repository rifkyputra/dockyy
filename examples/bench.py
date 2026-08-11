#!/usr/bin/env python3
"""Compare the runtime cost of the same app under Docker and under kuadrat/Quadlet.

Both examples build the identical app.py into the identical base image, so any
difference measured here is the runtime's, not the application's.

    python3 examples/bench.py

Requires examples/hello-py deployed via kuadrat (port 7654) and
examples/hello-py-docker up via compose (port 7655).

Three things are measured, and they are reported separately because they scale
differently:

  1. Fixed cost   — daemons that run whether or not you have any containers.
                    Amortised across every container on the host.
  2. Marginal cost— supervisor processes created per container. This is what
                    grows as you add apps, and what matters on a small host.
  3. Behaviour    — cold-start latency and request throughput.

Memory is reported as PSS (proportional set size) where readable, which counts
a shared page once, divided among the processes sharing it. RSS double-counts
shared pages and would flatter whichever runtime has more processes. Root-owned
processes may not expose smaps_rollup to a normal user; those fall back to RSS
and are marked, because an unmarked fallback is a silently wrong number.
"""

import json
import re
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import deque

KUADRAT_URL = "http://127.0.0.1:7654/"
DOCKER_URL = "http://127.0.0.1:7655/"
DOCKER_CONTAINER = "hello-docker"
KUADRAT_UNIT = "kuadrat-hello"

REQUESTS = 2000
CONCURRENCY = 16


# ---------------------------------------------------------------- process cost


def read_proc(pid, name):
    try:
        with open(f"/proc/{pid}/{name}") as fh:
            return fh.read()
    except (OSError, PermissionError):
        return ""


def mem_kb(pid):
    """(kilobytes, metric) for one pid. PSS when readable, else RSS."""
    rollup = read_proc(pid, "smaps_rollup")
    m = re.search(r"^Pss:\s+(\d+) kB", rollup, re.M)
    if m:
        return int(m.group(1)), "PSS"
    status = read_proc(pid, "status")
    m = re.search(r"^VmRSS:\s+(\d+) kB", status, re.M)
    if m:
        return int(m.group(1)), "RSS"
    return 0, "n/a"


def processes():
    """[(pid, comm, args)] for every process we can see."""
    out = subprocess.run(
        ["ps", "-eo", "pid=,comm=,args="], capture_output=True, text=True
    ).stdout
    rows = []
    for line in out.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) == 3:
            rows.append((int(parts[0]), parts[1], parts[2]))
    return rows


def container_id():
    out = subprocess.run(
        ["docker", "inspect", "-f", "{{.Id}}", DOCKER_CONTAINER],
        capture_output=True,
        text=True,
    )
    return out.stdout.strip()


def classify(procs, cid):
    """Split processes into the buckets we care about.

    Matching is by container id and by published port rather than by command
    name, so a host already running other containers (this one runs ~16) does
    not contaminate the marginal figures.
    """
    docker_fixed, docker_marginal, kuadrat_marginal = [], [], []

    for pid, comm, args in procs:
        if comm == "dockerd" or (comm == "containerd" and "shim" not in args):
            docker_fixed.append((pid, comm))
        elif "containerd-shim" in comm and cid and cid in args:
            docker_marginal.append((pid, "containerd-shim"))
        elif comm == "docker-proxy" and "7655" in args:
            docker_marginal.append((pid, "docker-proxy"))
        elif comm == "conmon" and KUADRAT_UNIT in args:
            kuadrat_marginal.append((pid, "conmon"))

    return docker_fixed, docker_marginal, kuadrat_marginal


def total(bucket):
    kb, metrics = 0, set()
    detail = []
    for pid, name in bucket:
        v, metric = mem_kb(pid)
        kb += v
        metrics.add(metric)
        detail.append((name, pid, v, metric))
    return kb, metrics, detail


# --------------------------------------------------------------------- latency


def get(url, timeout=5):
    start = time.perf_counter()
    with urllib.request.urlopen(url, timeout=timeout) as r:
        r.read()
    return (time.perf_counter() - start) * 1000


def wait_until_up(url, budget=60.0):
    """Seconds from now until the first successful response."""
    start = time.perf_counter()
    while time.perf_counter() - start < budget:
        try:
            get(url, timeout=2)
            return time.perf_counter() - start
        except (urllib.error.URLError, OSError):
            time.sleep(0.02)
    return float("nan")


def cold_start(kind):
    """Restart the workload and time until it serves again."""
    if kind == "docker":
        subprocess.run(
            ["docker", "restart", DOCKER_CONTAINER],
            capture_output=True,
            check=False,
        )
        return wait_until_up(DOCKER_URL)
    rc = subprocess.run(
        ["systemctl", "restart", KUADRAT_UNIT], capture_output=True, check=False
    )
    if rc.returncode != 0:
        return float("nan")  # needs root; reported as skipped
    return wait_until_up(KUADRAT_URL)


def load(url, n=REQUESTS, c=CONCURRENCY):
    """Fire n requests across c threads; return latencies in ms and wall time."""
    pending = deque(range(n))
    lock = threading.Lock()
    latencies, errors = [], [0]

    def worker():
        local = []
        while True:
            with lock:
                if not pending:
                    break
                pending.popleft()
            try:
                local.append(get(url))
            except Exception:
                errors[0] += 1
        with lock:
            latencies.extend(local)

    threads = [threading.Thread(target=worker) for _ in range(c)]
    t0 = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return latencies, time.perf_counter() - t0, errors[0]


def pct(values, p):
    if not values:
        return float("nan")
    ordered = sorted(values)
    k = min(int(len(ordered) * p / 100), len(ordered) - 1)
    return ordered[k]


# ------------------------------------------------------------------------ main


def main():
    for name, url in (("kuadrat", KUADRAT_URL), ("docker", DOCKER_URL)):
        try:
            get(url)
        except Exception as exc:
            sys.exit(f"FATAL: {name} not reachable at {url}: {exc}")

    cid = container_id()
    fixed, dmarg, kmarg = classify(processes(), cid)

    print("=" * 62)
    print("MEMORY — supervisor processes only (not the app itself)")
    print("=" * 62)

    fkb, fmetric, fdetail = total(fixed)
    dkb, dmetric, ddetail = total(dmarg)
    kkb, kmetric, kdetail = total(kmarg)

    print("\nDocker fixed cost — paid once, amortised over all containers:")
    for name, pid, v, metric in sorted(fdetail, key=lambda r: -r[2]):
        print(f"    {name:<20} pid {pid:<8} {v/1024:8.1f} MB  {metric}")
    print(f"    {'TOTAL':<20} {'':<12} {fkb/1024:8.1f} MB")

    print("\nDocker marginal cost — per container:")
    for name, pid, v, metric in sorted(ddetail, key=lambda r: -r[2]):
        print(f"    {name:<20} pid {pid:<8} {v/1024:8.1f} MB  {metric}")
    print(f"    {'TOTAL':<20} {'':<12} {dkb/1024:8.1f} MB")

    print("\nkuadrat marginal cost — per container:")
    for name, pid, v, metric in sorted(kdetail, key=lambda r: -r[2]):
        print(f"    {name:<20} pid {pid:<8} {v/1024:8.1f} MB  {metric}")
    print(f"    {'TOTAL':<20} {'':<12} {kkb/1024:8.1f} MB")
    print("    (no daemon: Quadlet units are supervised by systemd, already running)")

    if kkb:
        print(f"\n  marginal ratio: docker is {dkb/kkb:.1f}x kuadrat per container")
    print(f"  10 containers:  docker {(fkb+10*dkb)/1024:.0f} MB "
          f"vs kuadrat {(10*kkb)/1024:.0f} MB")

    metrics = fmetric | dmetric | kmetric
    if "RSS" in metrics:
        print("\n  NOTE: some processes fell back to RSS (root-owned, smaps_rollup")
        print("        unreadable). RSS double-counts shared pages, which flatters")
        print("        neither side consistently — rerun with sudo for pure PSS.")

    print()
    print("=" * 62)
    print("COLD START — restart to first successful response")
    print("=" * 62)
    for kind, label in (("docker", "docker"), ("kuadrat", "kuadrat")):
        t = cold_start(kind)
        if t != t:  # NaN
            print(f"  {label:<10} skipped (needs root)")
        else:
            print(f"  {label:<10} {t*1000:8.0f} ms")

    print()
    print("=" * 62)
    print(f"THROUGHPUT — {REQUESTS} requests, {CONCURRENCY} concurrent")
    print("=" * 62)
    results = {}
    for label, url in (("kuadrat", KUADRAT_URL), ("docker", DOCKER_URL)):
        wait_until_up(url)
        load(url, n=200, c=CONCURRENCY)  # warm up; discard
        lat, wall, errs = load(url)
        results[label] = (lat, wall, errs)
        print(
            f"  {label:<10} {len(lat)/wall:7.0f} req/s   "
            f"p50 {statistics.median(lat):5.1f} ms   "
            f"p95 {pct(lat,95):5.1f} ms   "
            f"p99 {pct(lat,99):5.1f} ms   errors {errs}"
        )

    print()
    print("  Expect these to be equivalent: both run the same process in the")
    print("  same kernel, and measurement on 2026-08-11 confirmed it — a gap")
    print("  seen in one run did not reproduce, and bypassing docker-proxy")
    print("  entirely was no faster than going through it.")
    print()
    print("  Treat a throughput difference here as noise unless it survives")
    print("  several runs. The load generator is Python and shares the host")
    print("  with both workloads, so it is the likely bottleneck, not either")
    print("  runtime. The memory figures above are the durable result.")


if __name__ == "__main__":
    main()
