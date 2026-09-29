# Storage pea

Inspect disks, filesystem UUIDs and mount state. Mount, unmount or explicitly format only a discovered writable removable leaf filesystem whose entire disk is non-system. Formatting erases data and requires explicit user intent and review. Persistent mounts use discovered UUIDs under /mnt/peasy-NAME. No partition editing, internal-disk mutations, LUKS operations or secret input.

## Contract

- `inspect_resources` accepts domain `storage`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
