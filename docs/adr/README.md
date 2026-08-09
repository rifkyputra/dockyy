# Architecture decision records

One record per decision that is expensive to reverse. Records are immutable once accepted — to
change a decision, write a new ADR that supersedes the old one and mark the old one
`Superseded by ADR-NNNN`.

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-podman-quadlet-as-runtime-substrate.md) | Podman Quadlet as the runtime substrate, systemd as the supervisor | Accepted |
| [0002](0002-transport-agnostic-core.md) | A transport-agnostic core crate behind an `Executor` trait | Accepted |
