# System diagnostics pea

Inspect bounded CPU load, available memory, filesystem capacity, I/O pressure, failed units and routing state. Diagnose from evidence; distinguish unavailable observations from healthy state. Read-only: never propose a repair through this pea.

## Contract

- `inspect_resources` accepts domain `diagnostics`; only service inspection accepts a target.
- This pea has no mutation permission.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
