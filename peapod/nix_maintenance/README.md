# Nix maintenance pea

Inspect system generations and store-filesystem capacity. Optimise store contents, collect unreferenced paths or remove specific reviewed generation references as separate authenticated operations. Current and booted systems and a rollback generation are protected. No arbitrary profile paths, lockfile changes, upgrade commands or automatic cleanup.

## Contract

- `inspect_resources` accepts domain `nix_maintenance`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
