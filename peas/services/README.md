# Services pea

Inspect loaded systemd services and exit/result status. Use exact discovered unit names for reviewed start, stop or restart. Protected infrastructure units cannot be controlled. Persistent service enablement uses supported NixOS options; disable withdraws Peasy contributions only. Never use systemctl enable or edit units.

## Contract

- `inspect_resources` accepts domain `services`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
