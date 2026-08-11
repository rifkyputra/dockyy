# Vendored assets

This is the only third-party code in the repository. It is vendored rather than fetched because the
daemon binds loopback on a host that may have no outbound network — a CDN reference would break the
UI exactly where kuadrat is meant to run.

| File | Upstream | Version | SHA-256 |
|---|---|---|---|
| `htmx.min.js` | https://unpkg.com/htmx.org@2.0.10/dist/htmx.min.js | 2.0.10 | `71ea67185bfa8c98c39d31717c6fce5d852370fcdfd129db4543774d3145c0de` |
| `sse.min.js` | https://unpkg.com/htmx-ext-sse@2.2.4/dist/sse.min.js | 2.2.4 | `98a46496de0c3605fbffdce9167ba427bdd9553184f83f149c261891a92c0136` |

Retrieved 2026-08-11. htmx 2 ships SSE support as a separate extension package, which is why this is
two files. To update: fetch the new version, record its hash here in the same edit, and re-run the
asset tests.

`kuadrat.css` is ours; it has no upstream.
