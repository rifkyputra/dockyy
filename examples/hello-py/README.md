# hello-py — a minimal kuadrat example

A single Python file serving JSON on **port 7654**, standard library only. No
dependencies, so the build stage is fast and cannot fail on a network hiccup.

```
app.py         the service — GET / and GET /healthz
Containerfile  python:3.12-alpine, copies app.py, runs it
kuadrat.json   the spec kuadrat reads (ports, env, memory limit)
```

## Deploy it

kuadrat writes **system** Quadlet units and calls `systemctl daemon-reload`, so
deploying needs root:

```bash
PATH=$HOME/.cargo/bin:$PATH cargo build --release      # as your normal user
sudo ./target/release/kuadrat deploy hello examples/hello-py
```

That runs the full loop — Detect → Build → Secrets → Apply → Route →
Healthcheck — and rolls back if any stage fails.

Then:

```bash
curl localhost:7654
curl localhost:7654/healthz
systemctl status kuadrat-hello
journalctl -u kuadrat-hello -n 50
```

Remove it:

```bash
sudo ./target/release/kuadrat remove hello
```

## What each file is doing, and why

**`PYTHONUNBUFFERED=1`** in the Containerfile. Without it, Python buffers stderr
and `journalctl -u kuadrat-hello` shows nothing until the process exits — which
is indistinguishable from a hung container when you are debugging a deploy.

**The SIGTERM handler** in `app.py`. `BaseHTTPRequestHandler` does not stop on
SIGTERM by default, so podman would wait out its full 10-second kill timeout on
every restart. With the handler the container stops in well under a second,
measured at 0.29 s.

**`"image": ""`** in `kuadrat.json`. The deploy loop overwrites it with the
image it just built, tagged by commit (`run.rs` sets `spec.image` after the
Build stage). Anything you put there is discarded, so the empty string states
that plainly.

**`"health_cmd": null`.** With no health command the Healthcheck stage falls
back to `systemctl is-active`. Adding one makes the stage poll podman's own
healthcheck instead — needed if you add a `route`, which kuadrat requires a
health command for.

**No `route`.** The app is published on 7654 directly. Add one to put it behind
Caddy with automatic TLS:

```bash
sudo ./target/release/kuadrat deploy hello examples/hello-py --route example.com:7654
```

## A naming wrinkle worth knowing

`kuadrat build examples/hello-py` tags the image from the **directory** name
(`kuadrat-hello-py`), while `kuadrat deploy hello …` tags from the **app** name
(`kuadrat-hello`). Both work; they just produce different tags for the same
source. `build` is a convenience for checking that a repo builds at all, and
nothing consumes its tag.
